//! The hook worker role: deliver pending HTTPS webhooks for an endpoint
//! in order, with HMAC-SHA256 signatures and exponential backoff. A task
//! is a wake-up for one endpoint; the desired state is the delivery rows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use vcsstar_core::events_json::event_to_json;
use vcsstar_core::services::hooks::{DeliveryRow, EndpointRow};
use vcsstar_core::App;
use vcsstar_db::schema::{hook_deliveries, hook_endpoints};
use vcsstar_types::decode_event;

use crate::dispatcher::{hook_queue, HookTask};
use crate::queue::{Claimed, Queue};

pub const MAX_ATTEMPTS: i32 = 20;
const LEASE: Duration = Duration::from_secs(120);

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    let mut queues: Vec<String> = Vec::new();
    let mut refreshed = Instant::now() - Duration::from_secs(3600);
    while !stop.load(Ordering::SeqCst) {
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => {
                    tracing::info!("hook worker connected to corndogs");
                    queue = Some(q);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "hook worker cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        if refreshed.elapsed() > Duration::from_secs(10) {
            match active_queues(&app) {
                Ok(qs) => queues = qs,
                Err(e) => tracing::warn!(error = %e, "endpoint list failed"),
            }
            refreshed = Instant::now();
        }
        let q = queue.as_ref().expect("connected");
        match q.claim(&queues, LEASE) {
            Ok(Some(claimed)) => {
                if let Err(e) = handle(&app, q, &claimed) {
                    tracing::warn!(error = %e, task = %claimed.uuid, "hook task failed; left for the lease to expire");
                }
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "claim failed; reconnecting");
                queue = None;
            }
        }
        crate::pause(&stop, Duration::from_secs(2));
    }
    Ok(())
}

fn active_queues(app: &App) -> anyhow::Result<Vec<String>> {
    let mut conn = app.conn()?;
    let ids: Vec<uuid::Uuid> = hook_endpoints::table
        .filter(hook_endpoints::active.eq(true))
        .filter(hook_endpoints::kind.eq("https"))
        .select(hook_endpoints::id)
        .load(&mut conn)?;
    Ok(ids.into_iter().map(hook_queue).collect())
}

/// Process one wake-up: deliver every due pending delivery of the endpoint
/// in order. Park the task when work remains, complete it otherwise.
pub fn handle(app: &App, q: &Queue, claimed: &Claimed) -> anyhow::Result<()> {
    let Some(task): Option<HookTask> = crate::decode(&claimed.payload) else {
        q.complete(claimed)?;
        return Ok(());
    };
    let endpoint_id = uuid::Uuid::parse_str(&task.endpoint_id)?;
    let mut conn = app.conn()?;
    let Some(ep): Option<EndpointRow> = hook_endpoints::table
        .find(endpoint_id)
        .select(EndpointRow::as_select())
        .first(&mut conn)
        .optional()?
    else {
        q.complete(claimed)?;
        return Ok(());
    };
    let outcome = process_endpoint(&mut conn, &ep)?;
    match outcome {
        Outcome::Done => {
            q.complete(claimed)?;
        }
        Outcome::RetryAfter(delay) => {
            q.park(claimed, delay)?;
        }
    }
    Ok(())
}

pub enum Outcome {
    Done,
    RetryAfter(Duration),
}

/// Deliver due rows. Returns when nothing is pending (Done) or when the
/// next pending row is not due yet (RetryAfter).
pub fn process_endpoint(conn: &mut PgConnection, ep: &EndpointRow) -> anyhow::Result<Outcome> {
    if !ep.active {
        return Ok(Outcome::Done);
    }
    loop {
        let pending: Vec<DeliveryRow> = hook_deliveries::table
            .filter(hook_deliveries::endpoint_id.eq(ep.id))
            .filter(hook_deliveries::state.eq("pending"))
            .order(hook_deliveries::created_at.asc())
            .limit(50)
            .select(DeliveryRow::as_select())
            .load(conn)?;
        if pending.is_empty() {
            return Ok(Outcome::Done);
        }
        let now = Utc::now();
        let mut progressed = false;
        let mut soonest: Option<DateTime<Utc>> = None;
        for d in &pending {
            if d.next_attempt_at > now {
                soonest = Some(soonest.map_or(d.next_attempt_at, |s| s.min(d.next_attempt_at)));
                if ep.unordered {
                    continue;
                }
                break;
            }
            let ok = attempt(conn, ep, d)?;
            progressed = true;
            if !ok && !ep.unordered {
                // Strict order: the failed one goes first next time.
                break;
            }
        }
        if !progressed {
            let wait = soonest
                .map(|s| (s - now).to_std().unwrap_or(Duration::from_secs(1)))
                .unwrap_or(Duration::from_secs(30));
            return Ok(Outcome::RetryAfter(wait.max(Duration::from_secs(1))));
        }
    }
}

/// One HTTP attempt. Records the result on the row. Returns success.
fn attempt(conn: &mut PgConnection, ep: &EndpointRow, d: &DeliveryRow) -> anyhow::Result<bool> {
    let url = ep.url.clone().unwrap_or_default();
    let (body, content_type) = match ep.content_type.as_str() {
        "cbor" => (d.payload_cbor.clone(), "application/cbor"),
        _ => {
            let json = match decode_event(&d.payload_cbor) {
                Ok(ev) => event_to_json(&ev),
                Err(e) => serde_json::json!({"error": format!("undecodable event: {e}")}),
            };
            (serde_json::to_vec(&json)?, "application/json")
        }
    };
    let signature = sign(ep.secret.as_deref().unwrap_or(&[]), &body);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let result = agent
        .post(&url)
        .set("Content-Type", content_type)
        .set("User-Agent", "vcsstar-hooks/0.1")
        .set("X-Vcsstar-Event", &d.event_kind)
        .set("X-Vcsstar-Delivery", &d.id.to_string())
        .set("X-Vcsstar-Signature-256", &format!("sha256={signature}"))
        .send_bytes(&body);
    let attempts = d.attempts + 1;
    match result {
        Ok(resp) if (200..300).contains(&resp.status()) => {
            diesel::update(hook_deliveries::table.find(d.id))
                .set((
                    hook_deliveries::state.eq("delivered"),
                    hook_deliveries::attempts.eq(attempts),
                    hook_deliveries::last_status.eq(resp.status() as i32),
                    hook_deliveries::last_error.eq::<Option<String>>(None),
                    hook_deliveries::delivered_at.eq(Utc::now()),
                ))
                .execute(conn)?;
            Ok(true)
        }
        other => {
            let (status, error) = match other {
                Ok(resp) => (
                    Some(resp.status() as i32),
                    format!("HTTP {}", resp.status()),
                ),
                Err(ureq::Error::Status(code, resp)) => {
                    let text = resp.into_string().unwrap_or_default();
                    (
                        Some(code as i32),
                        format!(
                            "HTTP {code}: {}",
                            text.chars().take(500).collect::<String>()
                        ),
                    )
                }
                Err(e) => (None, e.to_string().chars().take(500).collect()),
            };
            let dead = attempts >= MAX_ATTEMPTS;
            diesel::update(hook_deliveries::table.find(d.id))
                .set((
                    hook_deliveries::state.eq(if dead { "dead" } else { "pending" }),
                    hook_deliveries::attempts.eq(attempts),
                    hook_deliveries::last_status.eq(status),
                    hook_deliveries::last_error.eq(Some(error.clone())),
                    hook_deliveries::next_attempt_at.eq(Utc::now() + backoff(attempts)),
                ))
                .execute(conn)?;
            tracing::info!(endpoint = %ep.id, delivery = %d.id, attempts, dead, error = %error, "hook delivery failed");
            Ok(false)
        }
    }
}

/// 30 s doubling to a 1 h cap: about 3 days over 20 attempts.
pub fn backoff(attempts: i32) -> chrono::Duration {
    let secs = 30i64.saturating_mul(1i64 << (attempts.clamp(1, 12) - 1));
    chrono::Duration::seconds(secs.min(3600))
}

pub fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff(1).num_seconds(), 30);
        assert_eq!(backoff(2).num_seconds(), 60);
        assert_eq!(backoff(7).num_seconds(), 1920);
        assert_eq!(backoff(8).num_seconds(), 3600);
        assert_eq!(backoff(20).num_seconds(), 3600);
    }

    #[test]
    fn signature_is_hmac_sha256_hex() {
        // RFC 4231 test case 2.
        let sig = sign(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            sig,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
