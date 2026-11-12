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
    github_rate_limit_limit: IntGaugeVec,
    github_rate_limit_reset: IntGaugeVec,
    github_rate_limit_held: IntGaugeVec,
    github_pull_sync: IntCounterVec,
    github_backfill: IntCounterVec,
    github_import: IntCounterVec,
    github_backfill_pulls: prometheus::IntCounter,
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
        // GitHub rate limits, from the shared table at scrape time (every
        // node answers the same). `token` is the label the instance tracks
        // (`app:<app id>/<installation>`, `user:<login>`, a credential
        // purpose, `tok:<hash>`); `app_id` and `installation` are set for
        // GitHub App installation tokens and empty otherwise.
        let rate_labels = &["token", "app_id", "installation"];
        let github_rate_limit_remaining = gauge(
            &registry,
            "vcsstar_github_rate_limit_remaining",
            "GitHub API calls left in the current rate limit window, from the X-RateLimit-Remaining header",
            rate_labels,
        );
        let github_rate_limit_limit = gauge(
            &registry,
            "vcsstar_github_rate_limit_limit",
            "GitHub API calls allowed per rate limit window, from the X-RateLimit-Limit header",
            rate_labels,
        );
        let github_rate_limit_reset = gauge(
            &registry,
            "vcsstar_github_rate_limit_reset_seconds",
            "Seconds until the GitHub rate limit window resets (0 when it has reset), from X-RateLimit-Reset",
            rate_labels,
        );
        let github_rate_limit_held = gauge(
            &registry,
            "vcsstar_github_rate_limit_held",
            "1 while vcsstar holds calls on this token (reserve reached or Retry-After), else 0",
            rate_labels,
        );
        let github_pull_sync = counter(
            &registry,
            "vcsstar_github_pull_sync_total",
            "Pull-based GitHub sync passes, by outcome (complete, partial, held, error)",
            &["outcome"],
        );
        let github_backfill = counter(
            &registry,
            "vcsstar_github_backfill_total",
            "History import passes of GitHub pull requests, by outcome (done, partial, held, error)",
            &["outcome"],
        );
        let github_import = counter(
            &registry,
            "vcsstar_github_import_total",
            "Repository import attempts from GitHub, by outcome (done, retry, failed, held)",
            &["outcome"],
        );
        let github_backfill_pulls =
            prometheus::IntCounter::new("vcsstar_github_backfill_pulls_total", "Pull requests the history import went through")
                .expect("static metric definition");
        registry
            .register(Box::new(github_backfill_pulls.clone()))
            .expect("fresh registry");
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
            github_rate_limit_limit,
            github_rate_limit_reset,
            github_rate_limit_held,
            github_pull_sync,
            github_backfill,
            github_import,
            github_backfill_pulls,
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

/// The `app_id` and `installation` labels of a token label.
fn app_parts(token: &str) -> (&str, &str) {
    token
        .strip_prefix("app:")
        .and_then(|rest| rest.split_once('/'))
        .unwrap_or(("", ""))
}

/// The rate limit gauges, from the shared rate table (tokens seen in the
/// last two hours). Called by the scrape handler; the old label sets are
/// dropped first, so a token nobody uses any more leaves the output.
pub fn refresh_rate_limits() {
    let (_, rates) = crate::github_health::snapshot();
    let m = metrics();
    m.github_rate_limit_remaining.reset();
    m.github_rate_limit_limit.reset();
    m.github_rate_limit_reset.reset();
    m.github_rate_limit_held.reset();
    let now = crate::now_ms();
    for r in rates {
        let (app_id, installation) = app_parts(&r.token);
        let labels = [r.token.as_str(), app_id, installation];
        let clamp = |v: u64| v.min(i64::MAX as u64) as i64;
        m.github_rate_limit_remaining
            .with_label_values(&labels)
            .set(clamp(r.remaining));
        m.github_rate_limit_limit
            .with_label_values(&labels)
            .set(clamp(r.limit));
        m.github_rate_limit_reset
            .with_label_values(&labels)
            .set(((r.reset_at - now) / 1000).max(0));
        m.github_rate_limit_held
            .with_label_values(&labels)
            .set(r.held as i64);
    }
}

/// One pull-based sync pass of a repository finished.
pub fn github_pull_sync(outcome: &str) {
    metrics()
        .github_pull_sync
        .with_label_values(&[outcome])
        .inc();
}

/// One repository import attempt finished.
pub fn github_import(outcome: &str) {
    metrics().github_import.with_label_values(&[outcome]).inc();
}

/// One history import pass finished.
pub fn github_backfill(outcome: &str) {
    metrics()
        .github_backfill
        .with_label_values(&[outcome])
        .inc();
}

pub fn github_backfill_pulls(n: u64) {
    metrics().github_backfill_pulls.inc_by(n);
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
    fn app_labels_split_out_of_the_token_label() {
        assert_eq!(app_parts("app:5701582/998877"), ("5701582", "998877"));
        assert_eq!(app_parts("user:octo"), ("", ""));
        assert_eq!(app_parts("mirror_api"), ("", ""));
    }

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
