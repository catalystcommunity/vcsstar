//! Import of a GitHub or GitLab repository by a mirror worker from a corndogs
//! task (queue [`QUEUE`]). `Mirror/import-repo` and `Mirror/import-all`
//! only make the row (paused, mirror not enabled) and submit the task, so
//! a pod restart or a slow fetch never loses an import: corndogs gives the
//! task out again after its lease, and [`resubmit_lost`] submits it again
//! when the submission itself was lost.
//!
//! The import fetches every branch and tag in batches, captures them as a
//! history segment, records the refs and mirrored shas, reads the remote
//! default branch and description, and enables the mirror. Events carry
//! the provider as their origin. Only GitHub imports start a pull request
//! history job.

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
use vcsstar_db::models::{Org as OrgRow, Repo as RepoRow};
use vcsstar_db::schema::{mirror_refs, mirror_state, orgs, repos};
use vcsstar_store::refs::{self, RefUpdate};
use vcsstar_vcs::RevId;

use crate::github::Failure;
use crate::mirror::{self, cred, StateRow};
use crate::{outbox, vcs, App, Code, Error};

/// The shared import queue. Keep its legacy name for tasks already queued.
pub const QUEUE: &str = "import:github";
/// After this many failed attempts the import stops retrying ("failed").
pub const MAX_ATTEMPTS: i32 = 6;
/// A claim older than this belongs to a worker that died.
const CLAIM_STALE: time::Duration = time::Duration::minutes(30);

/// The task payload: a pointer; the worker reads the rest from Postgres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportTask {
    pub repo_id: Uuid,
}

/// "queued", "failed", or "done"; `None` for a mirror configured on an
/// existing repository (no import).
pub fn state(repo: &RepoRow, s: &StateRow) -> Option<&'static str> {
    if repo.mirror_enabled || s.import_done_at.is_some() {
        Some("done")
    } else if s.import_requested_at.is_none() {
        None
    } else if s.import_attempts >= MAX_ATTEMPTS {
        Some("failed")
    } else {
        Some("queued")
    }
}

/// Submit the import task of a repository. The key carries the time of
/// the request, so a new request after a final failure is a new task and
/// a repeated submission of the same request is refused by corndogs (and
/// taken as success). Retries are the same task, parked with a backoff.
pub fn submit(app: &App, repo_id: Uuid, requested_at: OffsetDateTime) -> Result<(), Error> {
    let mut payload = Vec::new();
    ciborium::ser::into_writer(&ImportTask { repo_id }, &mut payload).map_err(Error::internal)?;
    match app.submit_task(
        QUEUE,
        &format!("import:{repo_id}:{}", crate::to_ms(requested_at)),
        payload,
        3,
    ) {
        Ok(()) => Ok(()),
        Err(e) if e.code == Code::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// What one run of an import task did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Refs recorded and the mirror enabled (or it was already).
    Done { refs: usize },
    /// Another worker holds the claim; try again later.
    Busy,
    /// GitHub is not reachable now (open lane, rate hold); no attempt
    /// counted. Try again after the hold.
    Held(std::time::Duration),
    /// The attempt failed; `final_` when it was the last one.
    Failed { message: String, final_: bool },
    /// The row is gone (repository deleted); nothing to do.
    Gone,
}

/// Run the import of one repository. Idempotent: a finished import
/// answers `Done` without touching anything.
pub fn run(app: &App, repo_id: Uuid) -> Result<Outcome, Error> {
    let mut conn = app.conn()?;
    let Some(state) = mirror::state_for(&mut conn, repo_id)? else {
        return Ok(Outcome::Gone);
    };
    let repo: RepoRow = repos::table
        .find(repo_id)
        .select(RepoRow::as_select())
        .first(&mut conn)?;
    if repo.mirror_enabled || state.import_done_at.is_some() {
        return Ok(Outcome::Done { refs: 0 });
    }
    if state.is_github() && crate::github_health::is_open(crate::github_health::Lane::Git) {
        return Ok(Outcome::Held(std::time::Duration::from_secs(60)));
    }
    // Claim: one worker at a time per repository.
    let now = OffsetDateTime::now_utc();
    let claimed = diesel::update(
        mirror_state::table
            .find(repo_id)
            .filter(mirror_state::import_done_at.is_null())
            .filter(
                mirror_state::import_claimed_at
                    .is_null()
                    .or(mirror_state::import_claimed_at.lt(now - CLAIM_STALE)),
            ),
    )
    .set(mirror_state::import_claimed_at.eq(now))
    .execute(&mut conn)?;
    if claimed == 0 {
        return Ok(Outcome::Busy);
    }
    let org: OrgRow = orgs::table
        .find(repo.org_id)
        .select(OrgRow::as_select())
        .first(&mut conn)?;
    match fetch_and_enable(app, &mut conn, &org, &repo, &state) {
        Ok(Ok(refs)) => {
            if state.is_github() {
                crate::metrics::github_import("done");
            }
            tracing::info!(repo = %state.github_repo, refs, "import complete");
            Ok(Outcome::Done { refs })
        }
        Ok(Err(wait)) => {
            release(&mut conn, repo_id, None)?;
            if state.is_github() {
                crate::metrics::github_import("held");
            }
            Ok(Outcome::Held(wait))
        }
        Err(e) => {
            let attempts = state.import_attempts + 1;
            let message = format!("import: {}", e.message);
            release(&mut conn, repo_id, Some((attempts, &message)))?;
            let final_ = attempts >= MAX_ATTEMPTS;
            if state.is_github() {
                crate::metrics::github_import(if final_ { "failed" } else { "retry" });
            }
            tracing::warn!(repo = %state.github_repo, attempts, error = %e.message, "import attempt failed");
            Ok(Outcome::Failed { message, final_ })
        }
    }
}

/// Drop the claim; on a failure count the attempt and keep the reason.
fn release(
    conn: &mut PgConnection,
    repo_id: Uuid,
    failure: Option<(i32, &str)>,
) -> Result<(), Error> {
    let target = mirror_state::table.find(repo_id);
    match failure {
        Some((attempts, message)) => diesel::update(target)
            .set((
                mirror_state::import_claimed_at.eq(None::<OffsetDateTime>),
                mirror_state::import_attempts.eq(attempts),
                mirror_state::last_error.eq(message),
            ))
            .execute(conn)?,
        None => diesel::update(target)
            .set(mirror_state::import_claimed_at.eq(None::<OffsetDateTime>))
            .execute(conn)?,
    };
    Ok(())
}

/// The import proper. `Ok(Err(wait))`: GitHub asked us to wait.
fn fetch_and_enable(
    app: &App,
    conn: &mut vcsstar_db::DbConn,
    org: &OrgRow,
    repo: &RepoRow,
    state: &StateRow,
) -> Result<Result<usize, std::time::Duration>, Error> {
    let full = &state.github_repo;
    let owner = full.split('/').next().unwrap_or_default();
    // The instance App's installation, the organization's credential set,
    // then the requester's own link; a public repository also reads
    // anonymously.
    let token = if state.is_github() {
        match cred::token_for_owner(app, conn, org.id, owner, state.import_requested_by) {
            Ok((t, _)) => Some(t),
            Err(Failure::Terminal(_)) => None,
            Err(Failure::Transient(m)) => return Err(Error::new(Code::Unavailable, m)),
        }
    } else {
        // No credential permits anonymous reads. A configured but invalid
        // credential must fail instead of falling back to anonymous access.
        let mut found = false;
        for purpose in ["import_fetch", "mirror_git_push", "mirror_api"] {
            if !cred::candidates(conn, org.id, repo.id, purpose)?.is_empty() {
                found = true;
                break;
            }
        }
        if found {
            Some(cred::token_for_actor(
                app,
                conn,
                org.id,
                repo.id,
                "import_fetch",
                None,
            )?)
        } else {
            None
        }
    };
    let (remote_default, remote_description) = if state.is_github() {
        let info = match &token {
            Some(t) => {
                let gh = crate::github::Client::new(&app.config.github_api_base, &t.value)
                    .with_label(&t.label());
                if let Some(wait) = gh.rate_hold() {
                    return Ok(Err(wait));
                }
                gh.get_repo(full).ok().flatten()
            }
            None => None,
        };
        info.map(|r| (r.default_branch, r.description))
            .unwrap_or_default()
    } else {
        let info =
            crate::gitlab::project(&state.host, full, token.as_ref().map(|t| t.value.as_str()))?;
        if repo.visibility == "public" && info.visibility != "public" {
            return Err(Error::invalid(
                "a non-public GitLab project must be imported with private visibility",
            ));
        }
        (info.default_branch, info.description)
    };
    let token = token.map(|t| t.value);
    let dir = app.store.ensure(repo.id)?;
    let url = state.git_url(app);
    let remote = mirror::ls_remote_for(&dir, &url, token.as_deref(), state.is_github())?;
    // One fetch (in batches) for every branch and tag, not one per ref.
    let specs: Vec<(String, String)> = remote.iter().map(|(n, _)| (n.clone(), n.clone())).collect();
    if let Err(e) = mirror::fetch_refs_for(&dir, &url, token.as_deref(), &specs, state.is_github())
    {
        return Err(Error::new(
            Code::Unavailable,
            format!("fetch from {}: {}", state.provider, e.message()),
        ));
    }
    let default_branch = remote_default
        .or_else(|| {
            remote
                .iter()
                .find(|(n, _)| n == "refs/heads/main" || n == "refs/heads/master")
                .map(|(n, _)| n.trim_start_matches("refs/heads/").to_string())
        })
        .unwrap_or_else(|| repo.default_branch.clone());
    let description = remote_description.filter(|d| !d.is_empty());
    let updates: Vec<RefUpdate> = remote
        .iter()
        .map(|(n, s)| RefUpdate {
            name: n.clone(),
            old: None,
            new: Some(RevId(s.clone())),
        })
        .collect();
    let want: Vec<RevId> = remote.iter().map(|(_, s)| RevId(s.clone())).collect();
    // Refs that an earlier attempt recorded stay (the transaction below is
    // all or nothing, so there are none unless someone pushed meanwhile).
    let existing = refs::load(conn, repo.id)?;
    let updates: Vec<RefUpdate> = updates
        .into_iter()
        .filter(|u| !existing.iter().any(|(n, _)| n == &u.name))
        .collect();
    outbox::with_origin(&state.provider, || {
        conn.transaction::<(), Error, _>(|conn| {
            vcs::capture(app, conn, repo.id, &dir, &[], &want)?;
            refs::apply(conn, repo.id, &updates, state.import_requested_by)?;
            for (n, s) in &remote {
                diesel::insert_into(mirror_refs::table)
                    .values((
                        mirror_refs::repo_id.eq(repo.id),
                        mirror_refs::ref_.eq(n),
                        mirror_refs::mirrored_sha.eq(s),
                        mirror_refs::last_verified_at.eq(OffsetDateTime::now_utc()),
                    ))
                    .on_conflict_do_nothing()
                    .execute(conn)?;
            }
            let now = OffsetDateTime::now_utc();
            diesel::update(mirror_state::table.find(repo.id))
                .set((
                    mirror_state::paused.eq(false),
                    mirror_state::last_full_reconcile_at.eq(now),
                    mirror_state::import_done_at.eq(now),
                    mirror_state::import_claimed_at.eq(None::<OffsetDateTime>),
                    mirror_state::last_error.eq(None::<String>),
                ))
                .execute(conn)?;
            diesel::update(repos::table.find(repo.id))
                .set((
                    repos::mirror_enabled.eq(true),
                    repos::default_branch.eq(&default_branch),
                    repos::description.eq(description.as_deref()),
                ))
                .execute(conn)?;
            Ok(())
        })
    })?;
    Ok(Ok(remote.len()))
}

/// Submit again the task of every import that never started and was
/// asked for more than two minutes ago. Covers a submission lost when
/// corndogs was down; a task still in the queue has the same key and is
/// not duplicated. A started import is parked in corndogs between its
/// attempts. Returns how many were submitted.
pub fn resubmit_lost(app: &App) -> Result<usize, Error> {
    let mut conn = app.conn()?;
    let now = OffsetDateTime::now_utc();
    let rows: Vec<(Uuid, Option<OffsetDateTime>)> = mirror_state::table
        .filter(mirror_state::import_done_at.is_null())
        .filter(mirror_state::import_requested_at.lt(now - time::Duration::minutes(2)))
        .filter(mirror_state::import_attempts.eq(0))
        .filter(mirror_state::import_claimed_at.is_null())
        .select((mirror_state::repo_id, mirror_state::import_requested_at))
        .limit(200)
        .load(&mut conn)?;
    let mut n = 0;
    for (repo_id, at) in rows {
        let Some(at) = at else { continue };
        if submit(app, repo_id, at).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}
