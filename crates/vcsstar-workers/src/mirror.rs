//! The mirror worker role: claim one action wake-up at a time from the
//! `mirror:{org}/{repo}` queues, converge it, and schedule the periodic
//! reconcile pass (oldest `last_full_reconcile_at` first, a few per tick).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_core::mirror::{self, Exec};
use vcsstar_core::App;
use vcsstar_db::schema::{mirror_state, orgs, repos};

use crate::queue::{Claimed, Queue};

const LEASE: Duration = Duration::from_secs(600);
const RECONCILE_EVERY: chrono::Duration = chrono::Duration::minutes(15);

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    let mut queues: Vec<String> = Vec::new();
    let mut refreshed = Instant::now() - Duration::from_secs(3600);
    let mut last_reconcile = Instant::now() - Duration::from_secs(3600);
    while !stop.load(Ordering::SeqCst) {
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => {
                    tracing::info!("mirror worker connected to corndogs");
                    queue = Some(q);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mirror worker cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        if refreshed.elapsed() > Duration::from_secs(15) {
            match mirror_queues(&app) {
                Ok(qs) => queues = qs,
                Err(e) => tracing::warn!(error = %e, "mirror queue list failed"),
            }
            refreshed = Instant::now();
        }
        let mut drop_connection = false;
        {
            let q = queue.as_ref().expect("connected");
            match q.claim(&queues, LEASE) {
                Ok(Some(claimed)) => {
                    if let Err(e) = handle(&app, q, &claimed) {
                        tracing::warn!(error = %e, task = %claimed.uuid, "mirror task failed; lease will expire");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "mirror claim failed; reconnecting");
                    drop_connection = true;
                }
            }
            if last_reconcile.elapsed() > Duration::from_secs(60) {
                if let Err(e) = reconcile_pass(&app) {
                    tracing::warn!(error = %e, "reconcile pass failed");
                }
                last_reconcile = Instant::now();
            }
        }
        if drop_connection {
            queue = None;
        }
        crate::pause(&stop, Duration::from_secs(2));
    }
    Ok(())
}

fn mirror_queues(app: &App) -> anyhow::Result<Vec<String>> {
    let mut conn = app.conn()?;
    let rows: Vec<(String, String)> = mirror_state::table
        .inner_join(repos::table.inner_join(orgs::table))
        .filter(repos::mirror_enabled.eq(true))
        .select((orgs::name, repos::name))
        .load(&mut conn)?;
    Ok(rows.iter().map(|(o, r)| mirror::queue_name(o, r)).collect())
}

fn handle(app: &App, q: &Queue, claimed: &Claimed) -> anyhow::Result<()> {
    let Some(task) = mirror::decode_task(&claimed.payload) else {
        q.complete(claimed)?;
        return Ok(());
    };
    let action_id = uuid::Uuid::parse_str(&task.action_id)?;
    let mut conn = app.conn()?;
    match mirror::execute(app, &mut conn, action_id) {
        Ok(Exec::Applied) => {
            tracing::info!(action = %action_id, "mirror action applied");
            q.complete(claimed)?;
        }
        Ok(Exec::Skipped(why)) => {
            tracing::debug!(action = %action_id, why, "mirror action skipped");
            q.complete(claimed)?;
        }
        Ok(Exec::Blocked(reason)) => {
            tracing::warn!(action = %action_id, reason, "mirror action blocked");
            q.complete(claimed)?;
        }
        Ok(Exec::Transient(reason)) => {
            let attempts: i32 = vcsstar_db::schema::mirror_actions::table
                .find(action_id)
                .select(vcsstar_db::schema::mirror_actions::attempts)
                .first(&mut conn)
                .unwrap_or(1);
            let delay = mirror::backoff(attempts);
            tracing::info!(action = %action_id, reason, retry_in = ?delay, "mirror action parked");
            q.park(claimed, delay)?;
        }
        Err(e) if e.code == vcsstar_core::Code::NotFound => {
            // The action (or its repository) is gone: nothing to converge.
            tracing::info!(action = %action_id, "mirror action vanished; task completed");
            q.complete(claimed)?;
        }
        Err(e) => {
            tracing::error!(action = %action_id, error = %e.message, "mirror action errored");
            q.park(claimed, Duration::from_secs(60))?;
        }
    }
    Ok(())
}

/// Reconcile the repos whose last full pass is oldest, up to three per
/// minute. Webhook-verified refs make each pass cheap. Also resubmit a
/// wake-up for pending actions nothing touched for two minutes: a lost
/// submission or a deduplicated replay self-heals here.
fn reconcile_pass(app: &App) -> anyhow::Result<()> {
    let mut conn = app.conn()?;
    {
        use vcsstar_db::schema::mirror_actions;
        let stale: Vec<(uuid::Uuid, String, String)> = mirror_actions::table
            .inner_join(repos::table.inner_join(orgs::table))
            .filter(mirror_actions::state.eq("pending"))
            .filter(mirror_actions::updated_at.lt(Utc::now() - chrono::Duration::minutes(2)))
            .filter(repos::mirror_enabled.eq(true))
            .select((mirror_actions::id, orgs::name, repos::name))
            .limit(100)
            .load(&mut conn)?;
        let minute = Utc::now().timestamp() / 60;
        for (id, org, repo) in stale {
            if let Err(e) = mirror::submit_action(app, &org, &repo, id, minute) {
                tracing::warn!(error = %e.message, "stale action resubmit failed");
            }
            diesel::update(mirror_actions::table.find(id))
                .set(mirror_actions::updated_at.eq(Utc::now()))
                .execute(&mut conn)?;
        }
    }
    let due: Vec<(uuid::Uuid, String, String)> = mirror_state::table
        .inner_join(repos::table.inner_join(orgs::table))
        .filter(repos::mirror_enabled.eq(true))
        .filter(mirror_state::paused.eq(false))
        .filter(
            mirror_state::last_full_reconcile_at
                .is_null()
                .or(mirror_state::last_full_reconcile_at.lt(Utc::now() - RECONCILE_EVERY)),
        )
        .order(mirror_state::last_full_reconcile_at.asc().nulls_first())
        .limit(3)
        .select((repos::id, orgs::name, repos::name))
        .load(&mut conn)?;
    for (repo_id, org, repo) in due {
        match mirror::reconcile(app, &mut conn, repo_id) {
            Ok(o) => {
                for id in &o.scheduled {
                    if let Err(e) = mirror::submit_action(app, &org, &repo, *id, 0) {
                        tracing::warn!(error = %e.message, "reconcile submit failed");
                    }
                }
                tracing::info!(repo = %format!("{org}/{repo}"), checked = o.refs_checked, scheduled = o.scheduled.len(), drift = o.drift.len(), "reconciled");
            }
            Err(e) => {
                tracing::warn!(repo = %format!("{org}/{repo}"), error = %e.message, "reconcile failed");
                // Stamp anyway so one broken repo does not starve the others.
                diesel::update(mirror_state::table.find(repo_id))
                    .set((
                        mirror_state::last_full_reconcile_at.eq(Utc::now()),
                        mirror_state::last_error.eq(&e.message),
                    ))
                    .execute(&mut conn)?;
            }
        }
    }
    Ok(())
}
