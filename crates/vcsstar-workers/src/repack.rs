//! The repack worker role: consolidate the history segments of busy
//! repositories into one pack with a reachability bitmap, so a cold node
//! downloads one object instead of hundreds and `upload-pack` searches one
//! index. Every push adds a segment; this undoes the accumulation.
//!
//! One repository at a time per node, claimed through `repos.repacked_at`
//! with a conditional update, so several nodes never repack the same
//! repository at once. Pushes that land during a repack are untouched: they
//! add segments after the ones the repack saw.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use diesel::prelude::*;
use vcsstar_core::App;
use vcsstar_db::schema::{repo_segments, repos};

/// A repository is looked at again this long after its last repack.
const REPACK_EVERY: chrono::Duration = chrono::Duration::hours(6);
const PASS_EVERY: Duration = Duration::from_secs(600);

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut next = std::time::Instant::now();
    while !stop.load(Ordering::SeqCst) {
        if std::time::Instant::now() >= next {
            match pass(&app) {
                Ok(n) if n > 0 => tracing::info!(repacked = n, "repack pass"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "repack pass failed"),
            }
            next = std::time::Instant::now() + PASS_EVERY;
        }
        crate::pause(&stop, Duration::from_secs(2));
    }
    Ok(())
}

/// Repack the repository with the most live segments above the threshold.
pub fn pass(app: &App) -> anyhow::Result<usize> {
    if !app.store.has_segment_store() {
        return Ok(0);
    }
    let mut conn = app.conn()?;
    let cutoff = Utc::now() - REPACK_EVERY;
    let candidates: Vec<(uuid::Uuid, i64)> = repo_segments::table
        .inner_join(repos::table)
        .filter(repo_segments::superseded_at.is_null())
        .filter(
            repos::repacked_at
                .is_null()
                .or(repos::repacked_at.lt(cutoff)),
        )
        .group_by(repos::id)
        .select((repos::id, diesel::dsl::count_star()))
        .having(diesel::dsl::count_star().ge(app.config.repack_min_segments))
        .order(diesel::dsl::count_star().desc())
        .limit(1)
        .load(&mut conn)?;
    let Some((repo_id, segments)) = candidates.first().copied() else {
        return Ok(0);
    };
    let claimed = diesel::update(
        repos::table.find(repo_id).filter(
            repos::repacked_at
                .is_null()
                .or(repos::repacked_at.lt(cutoff)),
        ),
    )
    .set(repos::repacked_at.eq(Utc::now()))
    .execute(&mut conn)?;
    if claimed == 0 {
        return Ok(0);
    }
    let repo: vcsstar_db::models::Repo = repos::table
        .find(repo_id)
        .select(vcsstar_db::models::Repo::as_select())
        .first(&mut conn)?;
    let seen = vcsstar_store::segments::manifest(&mut conn, repo_id)?;
    let dir = vcsstar_core::vcs::hydrate(app, &mut conn, &repo)?;
    match app.store.repack(&mut conn, repo_id, &dir, &seen)? {
        Some(o) => {
            vcsstar_core::metrics::repack("repacked");
            tracing::info!(repo = %repo_id, segments, replaced = o.replaced, bytes = o.size, bitmap = o.has_bitmap, "repository repacked");
            Ok(1)
        }
        None => {
            vcsstar_core::metrics::repack("unchanged");
            Ok(0)
        }
    }
}
