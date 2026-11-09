//! The dispatcher role: fan each new outbox event out to the work it
//! implies (hook deliveries now, mirror actions in phase 3), submit one
//! keyed corndogs task per unit of work, then mark the event dispatched.
//! A periodic reconcile pass resubmits wake-ups for deliveries that are
//! still pending, so a lost submission self-heals.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use vcsstar_core::mirror::{self, EventRef};
use vcsstar_core::services::hooks::EndpointRow;
use vcsstar_core::App;
use vcsstar_db::schema::{events, hook_deliveries, hook_endpoints, mirror_state, orgs, repos};
use vcsstar_types::decode_event;

use crate::queue::Queue;

/// Wake-up for the hook worker: process the endpoint's pending deliveries.
#[derive(Debug, Serialize, Deserialize)]
pub struct HookTask {
    pub endpoint_id: String,
}

pub fn hook_queue(endpoint_id: uuid::Uuid) -> String {
    format!("hooks:{endpoint_id}")
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = events)]
struct EventRow {
    lane: String,
    seq: i64,
    repo_id: Option<uuid::Uuid>,
    org_id: Option<uuid::Uuid>,
    kind: String,
    payload_cbor: Vec<u8>,
    created_at: DateTime<Utc>,
}

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    let mut last_reconcile = Instant::now() - Duration::from_secs(3600);
    while !stop.load(Ordering::SeqCst) {
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => {
                    tracing::info!("dispatcher connected to corndogs");
                    queue = Some(q);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "dispatcher cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let mut drop_connection = false;
        {
            let q = queue.as_ref().expect("connected");
            match tick(&app, q) {
                Ok(n) if n > 0 => {
                    tracing::debug!(events = n, "dispatched");
                    continue; // more may be waiting
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "dispatch tick failed");
                    drop_connection = e.to_string().contains("corndogs");
                }
            }
            if last_reconcile.elapsed() > Duration::from_secs(60) {
                if let Err(e) = reconcile(&app, q) {
                    tracing::warn!(error = %e, "reconcile failed");
                    drop_connection |= e.to_string().contains("corndogs");
                }
                last_reconcile = Instant::now();
            }
        }
        if drop_connection {
            queue = None;
        }
        crate::pause(&stop, Duration::from_secs(1));
    }
    Ok(())
}

/// Dispatch up to one batch of undispatched events. Returns the count.
pub fn tick(app: &App, q: &Queue) -> anyhow::Result<usize> {
    let mut conn = app.conn()?;
    let rows: Vec<EventRow> = events::table
        .filter(events::dispatched_at.is_null())
        .order((
            events::created_at.asc(),
            events::lane.asc(),
            events::seq.asc(),
        ))
        .limit(200)
        .select(EventRow::as_select())
        .load(&mut conn)?;
    if rows.is_empty() {
        return Ok(0);
    }
    // Active endpoints, loaded once per batch.
    let endpoints: Vec<EndpointRow> = hook_endpoints::table
        .filter(hook_endpoints::active.eq(true))
        .select(EndpointRow::as_select())
        .load(&mut conn)?;
    let n = rows.len();
    for ev in rows {
        dispatch_one(app, &mut conn, q, &endpoints, &ev)?;
        diesel::update(
            events::table
                .filter(events::lane.eq(&ev.lane))
                .filter(events::seq.eq(ev.seq))
                .filter(events::created_at.eq(ev.created_at)),
        )
        .set(events::dispatched_at.eq(Utc::now()))
        .execute(&mut conn)?;
    }
    Ok(n)
}

fn dispatch_one(
    _app: &App,
    conn: &mut PgConnection,
    q: &Queue,
    endpoints: &[EndpointRow],
    ev: &EventRow,
) -> anyhow::Result<()> {
    let decoded = decode_event(&ev.payload_cbor).ok();
    for ep in endpoints.iter().filter(|e| {
        Some(e.org_id) == ev.org_id
            && (e.repo_id.is_none() || e.repo_id == ev.repo_id)
            && e.wants(&ev.kind)
    }) {
        // One delivery row per (endpoint, event); idempotent on re-dispatch.
        let exists: Option<uuid::Uuid> = hook_deliveries::table
            .filter(hook_deliveries::endpoint_id.eq(ep.id))
            .filter(hook_deliveries::event_lane.eq(&ev.lane))
            .filter(hook_deliveries::event_seq.eq(ev.seq))
            .select(hook_deliveries::id)
            .first(conn)
            .optional()?;
        if exists.is_some() {
            continue;
        }
        let delivery_id = vcsstar_db::new_id();
        match ep.kind.as_str() {
            "https" => {
                diesel::insert_into(hook_deliveries::table)
                    .values((
                        hook_deliveries::id.eq(delivery_id),
                        hook_deliveries::endpoint_id.eq(ep.id),
                        hook_deliveries::event_lane.eq(&ev.lane),
                        hook_deliveries::event_seq.eq(ev.seq),
                        hook_deliveries::event_kind.eq(&ev.kind),
                        hook_deliveries::state.eq("pending"),
                        hook_deliveries::payload_cbor.eq(&ev.payload_cbor),
                    ))
                    .execute(conn)?;
                q.submit(
                    &hook_queue(ep.id),
                    &format!("hooks:{}:{}:{}", ep.id, ev.lane, ev.seq),
                    crate::encode(&HookTask {
                        endpoint_id: ep.id.to_string(),
                    }),
                    0,
                )
                .map_err(|e| anyhow::anyhow!("corndogs submit: {e}"))?;
            }
            "corndogs" => {
                // Trusted consumer: the full event goes into its own queue.
                let queue_name = ep.corndogs_queue.clone().unwrap_or_default();
                q.submit(
                    &queue_name,
                    &format!("{}:{}", ev.lane, ev.seq),
                    ev.payload_cbor.clone(),
                    0,
                )
                .map_err(|e| anyhow::anyhow!("corndogs submit: {e}"))?;
                diesel::insert_into(hook_deliveries::table)
                    .values((
                        hook_deliveries::id.eq(delivery_id),
                        hook_deliveries::endpoint_id.eq(ep.id),
                        hook_deliveries::event_lane.eq(&ev.lane),
                        hook_deliveries::event_seq.eq(ev.seq),
                        hook_deliveries::event_kind.eq(&ev.kind),
                        hook_deliveries::state.eq("delivered"),
                        hook_deliveries::attempts.eq(1),
                        hook_deliveries::delivered_at.eq(Utc::now()),
                        hook_deliveries::payload_cbor.eq(&ev.payload_cbor),
                    ))
                    .execute(conn)?;
            }
            _ => {}
        }
    }
    let _ = decoded;
    // Mirror: one pending action per object key, newest desired state wins.
    if let Some(repo_id) = ev.repo_id {
        let mirrored: Option<(bool, bool, String, String)> = mirror_state::table
            .inner_join(repos::table.inner_join(orgs::table))
            .filter(mirror_state::repo_id.eq(repo_id))
            .select((
                repos::mirror_enabled,
                mirror_state::paused,
                orgs::name,
                repos::name,
            ))
            .first(conn)
            .optional()?;
        if let Some((enabled, paused, org, repo)) = mirrored {
            if enabled {
                let eref = EventRef {
                    repo_id,
                    lane: &ev.lane,
                    seq: ev.seq,
                    kind: &ev.kind,
                    payload_cbor: &ev.payload_cbor,
                };
                for spec in mirror::expand_event(&eref) {
                    let id = mirror::upsert_action(conn, &eref, &spec)?;
                    if !paused {
                        q.submit(
                            &mirror::queue_name(&org, &repo),
                            &format!("mirror:{id}:{}", ev.seq),
                            mirror::encode_task(&mirror::MirrorTask {
                                action_id: id.to_string(),
                            }),
                            0,
                        )
                        .map_err(|e| anyhow::anyhow!("corndogs submit: {e}"))?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resubmit a wake-up for every endpoint that still has a due delivery.
/// Keyed per minute so repeated passes within the receipt window dedup.
pub fn reconcile(app: &App, q: &Queue) -> anyhow::Result<usize> {
    let mut conn = app.conn()?;
    let due: Vec<uuid::Uuid> = hook_deliveries::table
        .inner_join(hook_endpoints::table)
        .filter(hook_deliveries::state.eq("pending"))
        .filter(hook_deliveries::next_attempt_at.le(Utc::now()))
        .filter(hook_endpoints::kind.eq("https"))
        .filter(hook_endpoints::active.eq(true))
        .select(hook_deliveries::endpoint_id)
        .distinct()
        .load(&mut conn)?;
    let minute = Utc::now().timestamp() / 60;
    for ep in &due {
        q.submit(
            &hook_queue(*ep),
            &format!("hooks:{ep}:reconcile:{minute}"),
            crate::encode(&HookTask {
                endpoint_id: ep.to_string(),
            }),
            0,
        )
        .map_err(|e| anyhow::anyhow!("corndogs submit: {e}"))?;
    }
    Ok(due.len())
}
