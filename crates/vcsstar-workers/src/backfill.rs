//! History import of GitHub pull requests (`github_sync::backfill_pulls`),
//! run by every node with the `mirror-worker` role on its own thread.
//!
//! One repository at a time per node, the one whose last import pass is
//! oldest, so every pending repository gets turns. The claim and the
//! schedule are `mirror_state.backfill_pass_at` (conditional update), so
//! any number of nodes share the work without running one repository
//! twice. The speed is set by GitHub: a pass stops at the bulk rate
//! reserve and the repository waits for the window to reset, and nothing
//! runs while the API lane is open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use diesel::prelude::*;
use time::OffsetDateTime;
use vcsstar_core::github_health::{self as health, Lane};
use vcsstar_core::github_sync;
use vcsstar_core::mirror;
use vcsstar_core::App;
use vcsstar_db::schema::{mirror_state, orgs, repos};

/// A pass holds its repository at most this long; a node that dies
/// mid-pass frees it after the lease.
const LEASE: time::Duration = time::Duration::minutes(10);
/// Wait when no repository has an import pending.
const IDLE: Duration = Duration::from_secs(30);
/// Wait after a failed pass of a repository before it is due again.
const AFTER_ERROR: time::Duration = time::Duration::minutes(2);

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    while !stop.load(Ordering::SeqCst) {
        let wait = match tick(&app) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "history import tick failed");
                IDLE
            }
        };
        crate::pause(&stop, wait);
    }
    Ok(())
}

/// Run one pass of one repository; returns how long to wait before the
/// next tick.
pub fn tick(app: &App) -> anyhow::Result<Duration> {
    // A plain read: `health::hold` would take the lane's one half-open
    // probe without making a call. Live work probes; the import follows.
    if health::is_open(Lane::Api) {
        return Ok(Duration::from_secs(10));
    }
    let mut conn = app.conn()?;
    let now = OffsetDateTime::now_utc();
    let due = || {
        mirror_state::backfill_pass_at
            .is_null()
            .or(mirror_state::backfill_pass_at.lt(now))
    };
    let next: Option<(uuid::Uuid, String)> = mirror_state::table
        .inner_join(repos::table.inner_join(orgs::table))
        .filter(repos::mirror_enabled.eq(true))
        .filter(mirror_state::paused.eq(false))
        .filter(diesel::dsl::sql::<diesel::sql_types::Bool>("NOT EXISTS (SELECT 1 FROM sync_connections sc WHERE sc.repo_id = mirror_state.repo_id)"))
        .filter(mirror_state::provider.eq("github"))
        .filter(mirror_state::backfill_done_at.is_null())
        .filter(due())
        .order(mirror_state::backfill_pass_at.asc().nulls_first())
        .select((repos::id, orgs::name))
        .first(&mut conn)
        .optional()?;
    let Some((repo_id, org_name)) = next else {
        return Ok(IDLE);
    };
    let claimed = diesel::update(mirror_state::table.find(repo_id).filter(due()))
        .set(mirror_state::backfill_pass_at.eq(Some(now + LEASE)))
        .execute(&mut conn)?;
    if claimed == 0 {
        // Another node took it; look again at once.
        return Ok(Duration::from_millis(200));
    }
    let org: vcsstar_db::models::Org = orgs::table
        .filter(orgs::name.eq(&org_name))
        .select(vcsstar_db::models::Org::as_select())
        .first(&mut conn)?;
    let repo: vcsstar_db::models::Repo = repos::table
        .find(repo_id)
        .select(vcsstar_db::models::Repo::as_select())
        .first(&mut conn)?;
    let Some(state) = mirror::state_for(&mut conn, repo_id)? else {
        return Ok(Duration::ZERO);
    };
    let name = format!("{org_name}/{}", repo.name);
    // When the repository is due again. Behind the others by default
    // (`now` sorts after every older stamp), later after a hold or error.
    let (again, wait) = match github_sync::backfill_pulls(
        app,
        &mut conn,
        &org,
        &repo,
        &state,
        github_sync::BACKFILL_BUDGET,
    ) {
        Ok(o) => {
            tracing::info!(repo = %name, pulls = o.pulls, changed = o.changed, skipped = o.skipped, calls = o.calls, done = o.done, "history import pass");
            match o.held {
                Some(h) => {
                    let h = time::Duration::try_from(h).unwrap_or(time::Duration::minutes(1));
                    // Other repositories on the same installation are held
                    // too; the tick finds them not due or held in turn.
                    (OffsetDateTime::now_utc() + h, Duration::from_secs(1))
                }
                None => (OffsetDateTime::now_utc(), Duration::from_millis(200)),
            }
        }
        Err(e) => {
            vcsstar_core::metrics::github_backfill("error");
            tracing::warn!(repo = %name, error = %e.message, "history import pass failed");
            (
                OffsetDateTime::now_utc() + AFTER_ERROR,
                Duration::from_secs(1),
            )
        }
    };
    diesel::update(mirror_state::table.find(repo_id))
        .set(mirror_state::backfill_pass_at.eq(Some(again)))
        .execute(&mut conn)?;
    Ok(wait)
}
