//! Repositories: metadata, grants, protections. VCS reads (log, tree, blob,
//! diff) arrive with the git backend in phase 1 and are not implemented yet.

use diesel::prelude::*;
use vcsstar_db::models::{Identity, NewRepo, Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::{branch_protections, identities, repo_grants, repos, teams};
use vcsstar_db::DbConn;
use vcsstar_types::{
    BranchProtection, CreateRepoRequest, DeleteRepoRequest, EventPayload, GetRepoRequest, Grant,
    ListGrantsResponse, ListProtectionsResponse, ListReposRequest, Ok as WireOk,
    RemoveGrantRequest, RemoveProtectionRequest, Repo, RepoPage, SetGrantRequest,
    SetProtectionRequest, SettingsPayload, UpdateRepoRequest,
};

use crate::authz::{self, OrgRole, RepoRole};
use crate::outbox::{self, Emit};
use crate::principal::ReqCtx;
use crate::services::{check_name, parse_identity_key};
use crate::{to_ms, App, Error};

pub const VCS_TYPES: &[&str] = &["git"];
pub const VISIBILITIES: &[&str] = &["public", "org", "private"];

pub fn to_wire(app: &App, org: &OrgRow, r: &RepoRow) -> Repo {
    Repo {
        org: org.name.clone(),
        name: r.name.clone(),
        vcs_type: r.vcs_type.clone(),
        visibility: r.visibility.clone(),
        default_branch: r.default_branch.clone(),
        description: r.description.clone(),
        fork_of: None,
        mirror_enabled: r.mirror_enabled,
        archived: r.archived,
        created_at: to_ms(r.created_at),
        https_url: format!(
            "{}/{}/{}.git",
            app.config.public_url.trim_end_matches('/'),
            org.name,
            r.name
        ),
        ssh_url: crate::vcs::ssh_url(app, org, r),
    }
}

fn settings_event<'a>(
    org: &OrgRow,
    repo: &RepoRow,
    ctx: &ReqCtx,
    kind: &'a str,
    changed: Vec<String>,
) -> Emit<'a> {
    Emit {
        lane: outbox::lane_for_repo(&org.name, &repo.name),
        kind,
        repo_id: Some(repo.id),
        org_id: Some(org.id),
        actor_id: ctx.actor_id(),
        actor: ctx.principal.as_ref().map(|p| p.to_wire()),
        repo: Some(vcsstar_types::RepoRef {
            instance: None,
            org: org.name.clone(),
            name: repo.name.clone(),
        }),
        org: Some(org.name.clone()),
        subject: None,
        payload: EventPayload::Variant5(SettingsPayload { changed }),
    }
}

pub fn create(app: &App, ctx: &ReqCtx, req: CreateRepoRequest) -> Result<Repo, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    check_name("repository", &req.name)?;
    let vcs_type = req.vcs_type.as_deref().unwrap_or("git");
    if !VCS_TYPES.contains(&vcs_type) {
        return Err(Error::invalid(format!(
            "vcs_type must be one of {VCS_TYPES:?}"
        )));
    }
    let visibility = req.visibility.as_deref().unwrap_or("org");
    if !VISIBILITIES.contains(&visibility) {
        return Err(Error::invalid(format!(
            "visibility must be one of {VISIBILITIES:?}"
        )));
    }
    let default_branch = req.default_branch.as_deref().unwrap_or("main");
    if default_branch.is_empty()
        || default_branch.contains(char::is_whitespace)
        || default_branch.starts_with("refs/")
    {
        return Err(Error::invalid("default_branch must be a short branch name"));
    }
    let mut conn = app.conn()?;
    let org = authz::load_org(&mut conn, &req.org)?;
    authz::require_org_role(app, &mut conn, &org, p, OrgRole::Admin)?;
    let repo = conn.transaction::<RepoRow, Error, _>(|conn| {
        let row = NewRepo {
            id: vcsstar_db::new_id(),
            org_id: org.id,
            name: &req.name,
            visibility,
            vcs_type,
            default_branch,
            description: req.description.as_deref(),
        };
        let repo: RepoRow = diesel::insert_into(repos::table)
            .values(&row)
            .returning(RepoRow::as_returning())
            .get_result(conn)?;
        outbox::emit(
            conn,
            settings_event(&org, &repo, ctx, "repo_created", vec!["created".into()]),
        )?;
        Ok(repo)
    })?;
    app.store.ensure(repo.id)?;
    Ok(to_wire(app, &org, &repo))
}

pub fn get(app: &App, ctx: &ReqCtx, req: GetRepoRequest) -> Result<Repo, Error> {
    let mut conn = app.conn()?;
    let (org, repo, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    Ok(to_wire(app, &org, &repo))
}

pub fn list(app: &App, ctx: &ReqCtx, req: ListReposRequest) -> Result<RepoPage, Error> {
    let mut conn = app.conn()?;
    let limit = req
        .page
        .as_ref()
        .and_then(|p| p.limit)
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let mut q = repos::table
        .inner_join(vcsstar_db::schema::orgs::table)
        .into_boxed();
    if let Some(org) = &req.org {
        q = q.filter(vcsstar_db::schema::orgs::name.eq(org));
    }
    if let Some(cursor) = req.page.as_ref().and_then(|p| p.cursor.clone()) {
        // Cursor is "org/name" of the last row returned.
        if let Some((o, n)) = cursor.split_once('/') {
            let (o, n) = (o.to_string(), n.to_string());
            q = q.filter(
                vcsstar_db::schema::orgs::name
                    .gt(o.clone())
                    .or(vcsstar_db::schema::orgs::name.eq(o).and(repos::name.gt(n))),
            );
        }
    }
    let rows: Vec<(RepoRow, OrgRow)> = q
        .order((vcsstar_db::schema::orgs::name.asc(), repos::name.asc()))
        .select((RepoRow::as_select(), OrgRow::as_select()))
        .load(&mut conn)?;
    // Visibility filtering happens per row; the page fills from visible rows.
    let mut out = Vec::new();
    let mut next_cursor = None;
    for (repo, org) in rows {
        if authz::repo_role(app, &mut conn, &org, &repo, ctx.principal.as_ref())?.is_none() {
            continue;
        }
        if out.len() == limit {
            next_cursor = out.last().map(|r: &Repo| format!("{}/{}", r.org, r.name));
            break;
        }
        out.push(to_wire(app, &org, &repo));
    }
    Ok(RepoPage {
        repos: out,
        next_cursor,
    })
}

pub fn update(app: &App, ctx: &ReqCtx, req: UpdateRepoRequest) -> Result<Repo, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    if let Some(v) = &req.visibility {
        if !VISIBILITIES.contains(&v.as_str()) {
            return Err(Error::invalid(format!(
                "visibility must be one of {VISIBILITIES:?}"
            )));
        }
    }
    if let Some(n) = &req.new_name {
        check_name("repository", n)?;
    }
    let mut changed = Vec::new();
    for (f, present) in [
        ("description", req.description.is_some()),
        ("default_branch", req.default_branch.is_some()),
        ("visibility", req.visibility.is_some()),
        ("archived", req.archived.is_some()),
        ("name", req.new_name.is_some()),
    ] {
        if present {
            changed.push(f.to_string());
        }
    }
    let updated = conn.transaction::<RepoRow, Error, _>(|conn| {
        let updated: RepoRow = diesel::update(repos::table.find(repo.id))
            .set((
                req.description.as_ref().map(|v| repos::description.eq(v)),
                req.default_branch
                    .as_ref()
                    .map(|v| repos::default_branch.eq(v)),
                req.visibility.as_ref().map(|v| repos::visibility.eq(v)),
                req.archived.map(|v| repos::archived.eq(v)),
                req.new_name.as_ref().map(|v| repos::name.eq(v)),
            ))
            .returning(RepoRow::as_returning())
            .get_result(conn)?;
        let kind = if req.new_name.is_some() {
            "repo_renamed"
        } else if req.archived == Some(true) {
            "repo_archived"
        } else {
            "repo_settings_changed"
        };
        outbox::emit(conn, settings_event(&org, &updated, ctx, kind, changed))?;
        Ok(updated)
    })?;
    Ok(to_wire(app, &org, &updated))
}

pub fn delete(app: &App, ctx: &ReqCtx, req: DeleteRepoRequest) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    if req.confirm_name != req.repo.name {
        return Err(Error::invalid(
            "confirm_name must equal the repository name",
        ));
    }
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    conn.transaction::<(), Error, _>(|conn| {
        outbox::emit(
            conn,
            settings_event(&org, &repo, ctx, "repo_deleted", vec!["deleted".into()]),
        )?;
        diesel::delete(repos::table.find(repo.id)).execute(conn)?;
        Ok(())
    })?;
    if let Err(e) = app.store.remove(repo.id) {
        tracing::warn!(error = %e, "repo cache dir removal failed");
    }
    Ok(WireOk { ok: true })
}

fn grantee_id(
    conn: &mut DbConn,
    org_id: uuid::Uuid,
    kind: &str,
    grantee: &str,
) -> Result<uuid::Uuid, Error> {
    match kind {
        "identity" => {
            let (user_id, domain) = parse_identity_key(grantee)?;
            identities::table
                .filter(identities::lk_user_id.eq(user_id))
                .filter(identities::lk_domain.eq(&domain))
                .select(identities::id)
                .first(conn)
                .optional()?
                .ok_or_else(|| Error::not_found("identity (the user must log in once first)"))
        }
        "team" => teams::table
            .filter(teams::org_id.eq(org_id))
            .filter(teams::name.eq(grantee))
            .select(teams::id)
            .first(conn)
            .optional()?
            .ok_or_else(|| Error::not_found("team")),
        _ => Err(Error::invalid("grantee_kind must be identity or team")),
    }
}

pub fn set_grant(app: &App, ctx: &ReqCtx, req: SetGrantRequest) -> Result<Grant, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    RepoRole::parse(&req.grant.role)
        .ok_or_else(|| Error::invalid("role must be admin, maintain, write, or read"))?;
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    let gid = grantee_id(
        &mut conn,
        org.id,
        &req.grant.grantee_kind,
        &req.grant.grantee,
    )?;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(repo_grants::table)
            .values((
                repo_grants::repo_id.eq(repo.id),
                repo_grants::grantee_kind.eq(&req.grant.grantee_kind),
                repo_grants::grantee_id.eq(gid),
                repo_grants::role.eq(&req.grant.role),
            ))
            .on_conflict((
                repo_grants::repo_id,
                repo_grants::grantee_kind,
                repo_grants::grantee_id,
            ))
            .do_update()
            .set(repo_grants::role.eq(&req.grant.role))
            .execute(conn)?;
        outbox::emit(
            conn,
            settings_event(
                &org,
                &repo,
                ctx,
                "grant_changed",
                vec![format!("{}:{}", req.grant.grantee_kind, req.grant.grantee)],
            ),
        )?;
        Ok(())
    })?;
    Ok(req.grant)
}

pub fn remove_grant(app: &App, ctx: &ReqCtx, req: RemoveGrantRequest) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    let gid = grantee_id(&mut conn, org.id, &req.grantee_kind, &req.grantee)?;
    let n = conn.transaction::<usize, Error, _>(|conn| {
        let n = diesel::delete(
            repo_grants::table
                .filter(repo_grants::repo_id.eq(repo.id))
                .filter(repo_grants::grantee_kind.eq(&req.grantee_kind))
                .filter(repo_grants::grantee_id.eq(gid)),
        )
        .execute(conn)?;
        if n > 0 {
            outbox::emit(
                conn,
                settings_event(
                    &org,
                    &repo,
                    ctx,
                    "grant_changed",
                    vec![format!("{}:{}", req.grantee_kind, req.grantee)],
                ),
            )?;
        }
        Ok(n)
    })?;
    if n == 0 {
        return Err(Error::not_found("grant"));
    }
    Ok(WireOk { ok: true })
}

pub fn list_grants(
    app: &App,
    ctx: &ReqCtx,
    req: GetRepoRequest,
) -> Result<ListGrantsResponse, Error> {
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Maintain)?;
    let rows: Vec<(String, uuid::Uuid, String)> = repo_grants::table
        .filter(repo_grants::repo_id.eq(repo.id))
        .select((
            repo_grants::grantee_kind,
            repo_grants::grantee_id,
            repo_grants::role,
        ))
        .load(&mut conn)?;
    let mut grants = Vec::with_capacity(rows.len());
    for (kind, gid, role) in rows {
        let grantee = match kind.as_str() {
            "identity" => {
                let i: Option<Identity> = identities::table
                    .find(gid)
                    .select(Identity::as_select())
                    .first(&mut conn)
                    .optional()?;
                match i {
                    Some(i) => format!("{}@{}", i.lk_user_id, i.lk_domain),
                    None => continue,
                }
            }
            _ => {
                let t: Option<String> = teams::table
                    .filter(teams::id.eq(gid))
                    .filter(teams::org_id.eq(org.id))
                    .select(teams::name)
                    .first(&mut conn)
                    .optional()?;
                match t {
                    Some(t) => t,
                    None => continue,
                }
            }
        };
        grants.push(Grant {
            grantee_kind: kind,
            grantee,
            role,
        });
    }
    Ok(ListGrantsResponse { grants })
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = branch_protections)]
pub struct ProtectionRow {
    pub ref_pattern: String,
    pub required_approvals: i32,
    pub dismiss_stale: bool,
    pub required_signature_level: String,
    pub required_checks: Vec<Option<String>>,
    pub allowed_merge_kinds: Vec<Option<String>>,
    pub allow_force_push: bool,
    pub allow_delete: bool,
}

pub fn protections_for(
    conn: &mut DbConn,
    repo_id: uuid::Uuid,
) -> Result<Vec<ProtectionRow>, Error> {
    Ok(branch_protections::table
        .filter(branch_protections::repo_id.eq(repo_id))
        .select(ProtectionRow::as_select())
        .load(conn)?)
}

fn protection_to_wire(r: ProtectionRow) -> BranchProtection {
    BranchProtection {
        ref_pattern: r.ref_pattern,
        required_approvals: r.required_approvals.max(0) as u64,
        dismiss_stale: r.dismiss_stale,
        required_signature_level: r.required_signature_level,
        required_checks: r.required_checks.into_iter().flatten().collect(),
        allowed_merge_kinds: r.allowed_merge_kinds.into_iter().flatten().collect(),
        allow_force_push: r.allow_force_push,
        allow_delete: r.allow_delete,
    }
}

pub fn set_protection(
    app: &App,
    ctx: &ReqCtx,
    req: SetProtectionRequest,
) -> Result<BranchProtection, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let pr = &req.protection;
    if pr.ref_pattern.is_empty() {
        return Err(Error::invalid("ref_pattern is required"));
    }
    if !["none", "any", "attested"].contains(&pr.required_signature_level.as_str()) {
        return Err(Error::invalid(
            "required_signature_level must be none, any, or attested",
        ));
    }
    for k in &pr.allowed_merge_kinds {
        if !["merge", "squash", "rebase"].contains(&k.as_str()) {
            return Err(Error::invalid(
                "allowed_merge_kinds entries must be merge, squash, or rebase",
            ));
        }
    }
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    let checks: Vec<Option<String>> = pr.required_checks.iter().cloned().map(Some).collect();
    let kinds: Vec<Option<String>> = pr.allowed_merge_kinds.iter().cloned().map(Some).collect();
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(branch_protections::table)
            .values((
                branch_protections::id.eq(vcsstar_db::new_id()),
                branch_protections::repo_id.eq(repo.id),
                branch_protections::ref_pattern.eq(&pr.ref_pattern),
                branch_protections::required_approvals
                    .eq(pr.required_approvals.min(i32::MAX as u64) as i32),
                branch_protections::dismiss_stale.eq(pr.dismiss_stale),
                branch_protections::required_signature_level.eq(&pr.required_signature_level),
                branch_protections::required_checks.eq(&checks),
                branch_protections::allowed_merge_kinds.eq(&kinds),
                branch_protections::allow_force_push.eq(pr.allow_force_push),
                branch_protections::allow_delete.eq(pr.allow_delete),
            ))
            .on_conflict((branch_protections::repo_id, branch_protections::ref_pattern))
            .do_update()
            .set((
                branch_protections::required_approvals
                    .eq(pr.required_approvals.min(i32::MAX as u64) as i32),
                branch_protections::dismiss_stale.eq(pr.dismiss_stale),
                branch_protections::required_signature_level.eq(&pr.required_signature_level),
                branch_protections::required_checks.eq(&checks),
                branch_protections::allowed_merge_kinds.eq(&kinds),
                branch_protections::allow_force_push.eq(pr.allow_force_push),
                branch_protections::allow_delete.eq(pr.allow_delete),
            ))
            .execute(conn)?;
        outbox::emit(
            conn,
            settings_event(
                &org,
                &repo,
                ctx,
                "protection_changed",
                vec![pr.ref_pattern.clone()],
            ),
        )?;
        Ok(())
    })?;
    Ok(req.protection)
}

pub fn remove_protection(
    app: &App,
    ctx: &ReqCtx,
    req: RemoveProtectionRequest,
) -> Result<WireOk, Error> {
    let p = ctx.require()?;
    p.require_scope("repo:admin")?;
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Admin)?;
    let n = conn.transaction::<usize, Error, _>(|conn| {
        let n = diesel::delete(
            branch_protections::table
                .filter(branch_protections::repo_id.eq(repo.id))
                .filter(branch_protections::ref_pattern.eq(&req.ref_pattern)),
        )
        .execute(conn)?;
        if n > 0 {
            outbox::emit(
                conn,
                settings_event(
                    &org,
                    &repo,
                    ctx,
                    "protection_changed",
                    vec![req.ref_pattern.clone()],
                ),
            )?;
        }
        Ok(n)
    })?;
    if n == 0 {
        return Err(Error::not_found("protection"));
    }
    Ok(WireOk { ok: true })
}

pub fn list_protections(
    app: &App,
    ctx: &ReqCtx,
    req: GetRepoRequest,
) -> Result<ListProtectionsResponse, Error> {
    let mut conn = app.conn()?;
    let (_org, repo, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    let rows: Vec<ProtectionRow> = branch_protections::table
        .filter(branch_protections::repo_id.eq(repo.id))
        .order(branch_protections::ref_pattern.asc())
        .select(ProtectionRow::as_select())
        .load(&mut conn)?;
    Ok(ListProtectionsResponse {
        protections: rows.into_iter().map(protection_to_wire).collect(),
    })
}
