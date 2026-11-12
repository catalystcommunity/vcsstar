//! Authorization: (principal, resource) -> effective role. One code path for
//! RPC handlers and VCS hooks. Roles are vcsstar-local tables.

use diesel::prelude::*;
use vcsstar_db::models::{Org, Repo};
use vcsstar_db::schema::{org_members, orgs, repo_grants, repos, team_members, teams};
use vcsstar_db::DbConn;

use crate::principal::{InstanceRole, Principal, ReqCtx};
use crate::{App, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrgRole {
    Member,
    Admin,
    Owner,
}

impl OrgRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "member" => Some(OrgRole::Member),
            "admin" => Some(OrgRole::Admin),
            "owner" => Some(OrgRole::Owner),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            OrgRole::Member => "member",
            OrgRole::Admin => "admin",
            OrgRole::Owner => "owner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepoRole {
    Read,
    Write,
    Maintain,
    Admin,
}

impl RepoRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(RepoRole::Read),
            "write" => Some(RepoRole::Write),
            "maintain" => Some(RepoRole::Maintain),
            "admin" => Some(RepoRole::Admin),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            RepoRole::Read => "read",
            RepoRole::Write => "write",
            RepoRole::Maintain => "maintain",
            RepoRole::Admin => "admin",
        }
    }
}

/// Instance administrators may create organizations and manage anything:
/// the ones in `INSTANCE_ADMINS` and the `admin` grants of `instance_roles`.
pub fn is_instance_admin(app: &App, p: &Principal) -> bool {
    p.instance_role == Some(InstanceRole::Admin) || is_configured_admin(app, &p.key())
}

/// An admin from the configuration (cannot be removed through the API).
pub fn is_configured_admin(app: &App, key: &str) -> bool {
    app.config
        .instance_admins
        .iter()
        .any(|a| a.eq_ignore_ascii_case(key))
}

/// The caller's instance role, configuration included.
pub fn instance_role(app: &App, p: &Principal) -> Option<InstanceRole> {
    if is_instance_admin(app, p) {
        Some(InstanceRole::Admin)
    } else {
        p.instance_role
    }
}

pub fn org_role(conn: &mut DbConn, org: &Org, p: &Principal) -> Result<Option<OrgRole>, Error> {
    let role: Option<String> = org_members::table
        .filter(org_members::org_id.eq(org.id))
        .filter(org_members::identity_id.eq(p.identity.id))
        .select(org_members::role)
        .first(conn)
        .optional()?;
    Ok(role.as_deref().and_then(OrgRole::parse))
}

/// Effective repo role, or `None` when the caller may not see the repo.
pub fn repo_role(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    repo: &Repo,
    p: Option<&Principal>,
) -> Result<Option<RepoRole>, Error> {
    let Some(p) = p else {
        return Ok(if repo.visibility == "public" {
            Some(RepoRole::Read)
        } else {
            None
        });
    };
    if is_instance_admin(app, p) {
        return Ok(Some(RepoRole::Admin));
    }
    let mut best: Option<RepoRole> = None;
    let mut bump = |r: RepoRole| {
        if best.is_none_or(|b| r > b) {
            best = Some(r);
        }
    };
    // An instance viewer reads every repository.
    if repo.visibility == "public" || p.instance_role == Some(InstanceRole::Viewer) {
        bump(RepoRole::Read);
    }
    match org_role(conn, org, p)? {
        Some(OrgRole::Owner) | Some(OrgRole::Admin) => bump(RepoRole::Admin),
        Some(OrgRole::Member) if repo.visibility != "private" => bump(RepoRole::Read),
        Some(OrgRole::Member) | None => {}
    }
    let direct: Vec<String> = repo_grants::table
        .filter(repo_grants::repo_id.eq(repo.id))
        .filter(repo_grants::grantee_kind.eq("identity"))
        .filter(repo_grants::grantee_id.eq(p.identity.id))
        .select(repo_grants::role)
        .load(conn)?;
    let via_team: Vec<String> = repo_grants::table
        .inner_join(teams::table.on(teams::id.eq(repo_grants::grantee_id)))
        .inner_join(team_members::table.on(team_members::team_id.eq(teams::id)))
        .filter(repo_grants::repo_id.eq(repo.id))
        .filter(repo_grants::grantee_kind.eq("team"))
        .filter(team_members::identity_id.eq(p.identity.id))
        .select(repo_grants::role)
        .load(conn)?;
    for r in direct
        .iter()
        .chain(via_team.iter())
        .filter_map(|s| RepoRole::parse(s))
    {
        bump(r);
    }
    Ok(best)
}

/// Load an org by name or fail with not-found.
pub fn load_org(conn: &mut DbConn, name: &str) -> Result<Org, Error> {
    orgs::table
        .filter(orgs::name.eq(name))
        .select(Org::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("organization"))
}

/// Load a repo by (org, name). Returns not-found for repos the caller may
/// not see, so existence does not leak.
pub fn load_repo_visible(
    app: &App,
    conn: &mut DbConn,
    ctx: &ReqCtx,
    org: &str,
    name: &str,
) -> Result<(Org, Repo, RepoRole), Error> {
    let org_row = load_org(conn, org)?;
    let repo: Repo = repos::table
        .filter(repos::org_id.eq(org_row.id))
        .filter(repos::name.eq(name))
        .select(Repo::as_select())
        .first(conn)
        .optional()?
        .ok_or_else(|| Error::not_found("repository"))?;
    match repo_role(app, conn, &org_row, &repo, ctx.principal.as_ref())? {
        Some(role) => Ok((org_row, repo, role)),
        None => Err(Error::not_found("repository")),
    }
}

pub fn require_org_role(
    app: &App,
    conn: &mut DbConn,
    org: &Org,
    p: &Principal,
    min: OrgRole,
) -> Result<OrgRole, Error> {
    if is_instance_admin(app, p) {
        return Ok(OrgRole::Owner);
    }
    match org_role(conn, org, p)? {
        Some(r) if r >= min => Ok(r),
        _ => Err(Error::forbidden(format!(
            "requires org role {}",
            min.as_str()
        ))),
    }
}

pub fn require_repo_role(role: RepoRole, min: RepoRole) -> Result<(), Error> {
    if role >= min {
        Ok(())
    } else {
        Err(Error::forbidden(format!(
            "requires repo role {}",
            min.as_str()
        )))
    }
}
