//! Process-wide Prometheus metrics. One registry per process; `vcsstard`
//! serves it at `GET /metrics` on the `api` role. Counters are created
//! once and looked up by name, so a call site never fails on a duplicate
//! registration.

use std::sync::OnceLock;

use prometheus::{Encoder, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder};

struct Metrics {
    registry: Registry,
    rate_limited: IntCounterVec,
    mirror_actions: IntCounterVec,
    github_webhooks: IntCounterVec,
    github_ingest: IntCounterVec,
    github_lane_open: IntGaugeVec,
    github_lane_transitions: IntCounterVec,
    github_rate_limit_remaining: IntGaugeVec,
    github_pull_sync: IntCounterVec,
    hook_deliveries: IntCounterVec,
    repack: IntCounterVec,
    cache_warm: IntCounterVec,
    mirror_pending: prometheus::IntGauge,
    mirror_blocked: prometheus::IntGauge,
    mirror_lag: prometheus::IntGauge,
}

fn plain_gauge(registry: &Registry, name: &str, help: &str) -> prometheus::IntGauge {
    let g = prometheus::IntGauge::new(name, help).expect("static metric definition");
    registry
        .register(Box::new(g.clone()))
        .expect("fresh registry");
    g
}

fn gauge(registry: &Registry, name: &str, help: &str, labels: &[&str]) -> IntGaugeVec {
    let g = IntGaugeVec::new(Opts::new(name, help), labels).expect("static metric definition");
    registry
        .register(Box::new(g.clone()))
        .expect("fresh registry");
    g
}

fn counter(registry: &Registry, name: &str, help: &str, labels: &[&str]) -> IntCounterVec {
    let c = IntCounterVec::new(Opts::new(name, help), labels).expect("static metric definition");
    registry
        .register(Box::new(c.clone()))
        .expect("fresh registry");
    c
}

fn metrics() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(|| {
        let registry = Registry::new();
        let rate_limited = counter(
            &registry,
            "vcsstar_rate_limited_total",
            "Requests refused by a login rate limit, by entry point",
            &["entry"],
        );
        let mirror_actions = counter(
            &registry,
            "vcsstar_mirror_actions_total",
            "Mirror actions executed, by object kind and outcome (applied, skipped, blocked, transient, error)",
            &["kind", "outcome"],
        );
        let github_webhooks = counter(
            &registry,
            "vcsstar_github_webhooks_total",
            "GitHub webhook deliveries at POST /hooks/github, by event and outcome (captured, duplicate, unauthorized, unknown_repo, rejected)",
            &["event", "outcome"],
        );
        let github_ingest = counter(
            &registry,
            "vcsstar_github_ingest_total",
            "GitHub webhook deliveries processed by the ingest worker, by event and outcome (applied, ignored, retry)",
            &["event", "outcome"],
        );
        let github_lane_open = gauge(
            &registry,
            "vcsstar_github_lane_open",
            "1 while this node holds calls on a GitHub lane (api, git) after repeated transient failures",
            &["lane"],
        );
        let github_lane_transitions = counter(
            &registry,
            "vcsstar_github_lane_transitions_total",
            "Times a GitHub lane opened or closed on this node, by lane and new state",
            &["lane", "to"],
        );
        let github_rate_limit_remaining = gauge(
            &registry,
            "vcsstar_github_rate_limit_remaining",
            "Remaining GitHub API calls in the current window, by token label",
            &["token"],
        );
        let github_pull_sync = counter(
            &registry,
            "vcsstar_github_pull_sync_total",
            "Pull-based GitHub sync passes, by outcome (complete, partial, held, error)",
            &["outcome"],
        );
        let hook_deliveries = counter(
            &registry,
            "vcsstar_hook_deliveries_total",
            "Outbound webhook delivery attempts, by outcome (delivered, retry, dead)",
            &["outcome"],
        );
        let repack = counter(
            &registry,
            "vcsstar_repack_total",
            "Repack passes that looked at a repository, by outcome (repacked, unchanged)",
            &["outcome"],
        );
        let cache_warm = counter(
            &registry,
            "vcsstar_cache_warm_total",
            "Repositories the background cache warm handled on this node, by outcome (hydrated, current, error)",
            &["outcome"],
        );
        let mirror_pending = plain_gauge(
            &registry,
            "vcsstar_mirror_pending_actions",
            "Mirror actions waiting to converge on GitHub, across every repository (read at scrape time)",
        );
        let mirror_blocked = plain_gauge(
            &registry,
            "vcsstar_mirror_blocked_actions",
            "Mirror actions blocked on a terminal GitHub refusal, across every repository (read at scrape time)",
        );
        let mirror_lag = plain_gauge(
            &registry,
            "vcsstar_mirror_lag_seconds",
            "Age of the oldest pending mirror action in seconds; 0 when nothing is pending (read at scrape time)",
        );
        Metrics {
            registry,
            rate_limited,
            mirror_actions,
            github_webhooks,
            github_ingest,
            github_lane_open,
            github_lane_transitions,
            github_rate_limit_remaining,
            github_pull_sync,
            hook_deliveries,
            repack,
            cache_warm,
            mirror_pending,
            mirror_blocked,
            mirror_lag,
        }
    })
}

/// One refusal by the limiter guarding `entry` (`login`, `complete`,
/// `device_begin`, `device_poll`, `device_approve`).
pub fn rate_limited(entry: &str) {
    metrics().rate_limited.with_label_values(&[entry]).inc();
}

/// One mirror action finished with `outcome`; `kind` is the object kind
/// (`ref`, `repo-settings`, `pr`, `pr-comment`, `pr-review`, `status`).
pub fn mirror_action(kind: &str, outcome: &str) {
    metrics()
        .mirror_actions
        .with_label_values(&[kind, outcome])
        .inc();
}

/// One delivery reached the webhook receiver.
pub fn github_webhook(event: &str, outcome: &str) {
    metrics()
        .github_webhooks
        .with_label_values(&[event, outcome])
        .inc();
}

/// One delivery left the ingest worker.
pub fn github_ingest(event: &str, outcome: &str) {
    metrics()
        .github_ingest
        .with_label_values(&[event, outcome])
        .inc();
}

/// A GitHub lane opened (`true`) or closed on this node.
pub fn github_lane(lane: &str, open: bool) {
    metrics()
        .github_lane_open
        .with_label_values(&[lane])
        .set(open as i64);
}

pub fn github_lane_transition(lane: &str, to: &str) {
    metrics()
        .github_lane_transitions
        .with_label_values(&[lane, to])
        .inc();
}

pub fn github_rate_limit_remaining(token: &str, remaining: u64) {
    metrics()
        .github_rate_limit_remaining
        .with_label_values(&[token])
        .set(remaining.min(i64::MAX as u64) as i64);
}

/// One pull-based sync pass of a repository finished.
pub fn github_pull_sync(outcome: &str) {
    metrics()
        .github_pull_sync
        .with_label_values(&[outcome])
        .inc();
}

/// One outbound webhook delivery attempt finished.
pub fn hook_delivery(outcome: &str) {
    metrics()
        .hook_deliveries
        .with_label_values(&[outcome])
        .inc();
}

pub fn repack(outcome: &str) {
    metrics().repack.with_label_values(&[outcome]).inc();
}

pub fn cache_warm(outcome: &str) {
    metrics().cache_warm.with_label_values(&[outcome]).inc();
}

/// Backlog gauges from the database, for the scrape handler. Any node
/// can answer, since the backlog lives in Postgres and not in a worker.
pub fn refresh_backlog(conn: &mut diesel::PgConnection) -> Result<(), diesel::result::Error> {
    use diesel::prelude::*;
    use vcsstar_db::schema::mirror_actions;
    let pending: i64 = mirror_actions::table
        .filter(mirror_actions::state.eq("pending"))
        .count()
        .get_result(conn)?;
    let blocked: i64 = mirror_actions::table
        .filter(mirror_actions::state.eq("blocked"))
        .count()
        .get_result(conn)?;
    let oldest: Option<chrono::DateTime<chrono::Utc>> = mirror_actions::table
        .filter(mirror_actions::state.eq("pending"))
        .select(diesel::dsl::min(mirror_actions::created_at))
        .first(conn)?;
    let m = metrics();
    m.mirror_pending.set(pending);
    m.mirror_blocked.set(blocked);
    m.mirror_lag.set(
        oldest
            .map(|t| (chrono::Utc::now() - t).num_seconds().max(0))
            .unwrap_or(0),
    );
    Ok(())
}

/// The registry in Prometheus text exposition format.
pub fn render() -> String {
    let families = metrics().registry.gather();
    let mut out = Vec::new();
    if TextEncoder::new().encode(&families, &mut out).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_shows_up_in_the_rendering() {
        rate_limited("test_entry");
        rate_limited("test_entry");
        let text = render();
        assert!(
            text.contains("vcsstar_rate_limited_total{entry=\"test_entry\"} 2"),
            "{text}"
        );
    }
}
