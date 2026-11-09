//! Commit statuses: CI results per revision. Branch protections consume
//! them as merge gates (phase 4).

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use vcsstar_db::models::Identity;
use vcsstar_db::schema::{identities, statuses};
use vcsstar_types::{
    CommitStatus, EventPayload, ListStatusRequest, ListStatusResponse, SetStatusRequest,
    StatusPayload,
};

use crate::authz::{self, RepoRole};
use crate::outbox::{self, Emit};
use crate::principal::{identity_to_wire, ReqCtx};
use crate::{to_ms, vcs, App, Error};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = statuses)]
pub struct StatusRow {
    pub id: uuid::Uuid,
    pub repo_id: uuid::Uuid,
    pub sha: String,
    pub context: String,
    pub state: String,
    pub target_url: Option<String>,
    pub description: Option<String>,
    pub reporter: Option<uuid::Uuid>,
    pub created_at: DateTime<Utc>,
}

fn to_wire(conn: &mut PgConnection, s: &StatusRow) -> Result<CommitStatus, Error> {
    let reporter = match s.reporter {
        Some(id) => identities::table
            .find(id)
            .select(Identity::as_select())
            .first(conn)
            .optional()?
            .map(|i| identity_to_wire(&i)),
        None => None,
    };
    Ok(CommitStatus {
        sha: s.sha.clone(),
        context: s.context.clone(),
        state: s.state.clone(),
        target_url: s.target_url.clone(),
        description: s.description.clone(),
        reporter: reporter.unwrap_or(vcsstar_types::Identity {
            user_id: String::new(),
            domain: String::new(),
            handle: None,
            display_name: None,
        }),
        created_at: to_ms(s.created_at),
    })
}

fn check_sha(sha: &str) -> Result<(), Error> {
    if sha.len() < 7 || sha.len() > 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::invalid("sha must be a hex revision id"));
    }
    Ok(())
}

pub fn set(app: &App, ctx: &ReqCtx, req: SetStatusRequest) -> Result<CommitStatus, Error> {
    let p = ctx.require()?;
    p.require_scope("statuses:write")?;
    check_sha(&req.sha)?;
    if !["pending", "success", "failure", "error"].contains(&req.state.as_str()) {
        return Err(Error::invalid(
            "state must be pending, success, failure, or error",
        ));
    }
    if req.context.is_empty() || req.context.len() > 200 {
        return Err(Error::invalid("context is required (max 200 chars)"));
    }
    let mut conn = app.conn()?;
    let (org, repo, role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    authz::require_repo_role(role, RepoRole::Write)?;
    let row = conn.transaction::<StatusRow, Error, _>(|conn| {
        let row: StatusRow = diesel::insert_into(statuses::table)
            .values((
                statuses::id.eq(vcsstar_db::new_id()),
                statuses::repo_id.eq(repo.id),
                statuses::sha.eq(&req.sha),
                statuses::context.eq(&req.context),
                statuses::state.eq(&req.state),
                statuses::target_url.eq(req.target_url.as_deref()),
                statuses::description.eq(req.description.as_deref()),
                statuses::reporter.eq(p.identity.id),
            ))
            .on_conflict((statuses::repo_id, statuses::sha, statuses::context))
            .do_update()
            .set((
                statuses::state.eq(&req.state),
                statuses::target_url.eq(req.target_url.as_deref()),
                statuses::description.eq(req.description.as_deref()),
                statuses::reporter.eq(p.identity.id),
                statuses::created_at.eq(Utc::now()),
            ))
            .returning(StatusRow::as_returning())
            .get_result(conn)?;
        outbox::emit(
            conn,
            Emit {
                lane: outbox::lane_for_repo(&org.name, &repo.name),
                kind: "status_set",
                repo_id: Some(repo.id),
                org_id: Some(org.id),
                actor_id: Some(p.identity.id),
                actor: Some(p.to_wire()),
                repo: Some(vcs::repo_ref(&org, &repo)),
                org: Some(org.name.clone()),
                subject: Some(req.sha.clone()),
                payload: EventPayload::Variant6(StatusPayload {
                    sha: req.sha.clone(),
                    context: req.context.clone(),
                    state: req.state.clone(),
                }),
            },
        )?;
        Ok(row)
    })?;
    to_wire(&mut conn, &row)
}

pub fn list(app: &App, ctx: &ReqCtx, req: ListStatusRequest) -> Result<ListStatusResponse, Error> {
    check_sha(&req.sha)?;
    let mut conn = app.conn()?;
    let (_org, repo, _role) =
        authz::load_repo_visible(app, &mut conn, ctx, &req.repo.org, &req.repo.name)?;
    let rows = for_sha(&mut conn, repo.id, &req.sha)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(to_wire(&mut conn, r)?);
    }
    Ok(ListStatusResponse { statuses: out })
}

pub fn for_sha(
    conn: &mut PgConnection,
    repo_id: uuid::Uuid,
    sha: &str,
) -> Result<Vec<StatusRow>, Error> {
    Ok(statuses::table
        .filter(statuses::repo_id.eq(repo_id))
        .filter(statuses::sha.eq(sha))
        .order(statuses::context.asc())
        .select(StatusRow::as_select())
        .load(conn)?)
}
