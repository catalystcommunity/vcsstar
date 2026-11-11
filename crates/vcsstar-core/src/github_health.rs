//! The instance's shared view of GitHub's availability and of API rate
//! limits. Every node reads and writes the same rows (`github_lanes`,
//! `github_rate_limits`), so API and worker pods scale out without each
//! one discovering an outage on its own or spending a token's budget
//! twice. Without a database attached (unit tests) the same state
//! machine runs in process memory.
//!
//! Two lanes are tracked: `api` (REST) and `git` (smart HTTP). Each lane
//! is a circuit breaker. Transient failures in a row open it; while it is
//! open the mirror holds every call on that lane (parked tasks wait, the
//! reconcile passes skip the lane) instead of hammering an outage and
//! burning rate limit. When the open period ends one probe goes through
//! (the row lock makes it one across nodes); success closes the lane and
//! records `recovered_at`, which the reconcile passes read so every
//! mirrored repository is checked soon after GitHub comes back. Another
//! failure opens it for twice as long, up to ten minutes, so recovery lag
//! stays bounded.
//!
//! Rate limits are tracked per token label from the `X-RateLimit-*`
//! headers of every response. A token whose remaining budget is below
//! the reserve is held until its window resets; a `Retry-After`
//! (secondary limit) holds it for that long. Terminal refusals (401, 403
//! without a limit, 404, 422) are not failures of the lane: GitHub is up,
//! it just said no.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use vcsstar_db::schema::{github_lanes, github_rate_limits};
use vcsstar_db::DbPool;
use vcsstar_types::{LaneHealth, RateLimitInfo};

use crate::to_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    Api,
    Git,
}

impl Lane {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lane::Api => "api",
            Lane::Git => "git",
        }
    }
}

/// Transient failures in a row that open a lane.
const OPEN_AFTER: i32 = 3;
const OPEN_BASE: chrono::Duration = chrono::Duration::seconds(30);
const OPEN_MAX: chrono::Duration = chrono::Duration::minutes(10);
/// A probe that reports nothing for this long frees the slot for another.
const PROBE_TIMEOUT: chrono::Duration = chrono::Duration::seconds(90);
/// Calls per token kept in reserve for interactive use; a token below it
/// is held until its window resets.
const RESERVE_FLOOR: i64 = 50;
/// A healthy lane's `last_success_at` is refreshed at most this often, so
/// a busy mirror does not write a row per GitHub call.
const SUCCESS_STAMP_EVERY: chrono::Duration = chrono::Duration::seconds(60);

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = github_lanes, treat_none_as_null = true)]
struct LaneRow {
    lane: String,
    consecutive_failures: i32,
    /// Times the lane opened without a success in between (backoff step).
    opens: i32,
    open_until: Option<DateTime<Utc>>,
    probing_since: Option<DateTime<Utc>>,
    last_error: Option<String>,
    last_failure_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    recovered_at: Option<DateTime<Utc>>,
}

impl LaneRow {
    fn new(lane: Lane) -> Self {
        Self {
            lane: lane.as_str().into(),
            consecutive_failures: 0,
            opens: 0,
            open_until: None,
            probing_since: None,
            last_error: None,
            last_failure_at: None,
            last_success_at: None,
            recovered_at: None,
        }
    }

    fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.open_until.is_some_and(|t| t > now)
    }

    fn state_name(&self, now: DateTime<Utc>) -> &'static str {
        if self.is_open(now) {
            "open"
        } else if self.open_until.is_some() {
            "half_open"
        } else {
            "closed"
        }
    }

    /// A call succeeded. Returns true when the lane was down before.
    fn on_success(&mut self, now: DateTime<Utc>) -> bool {
        let was_down = self.open_until.is_some() || self.probing_since.is_some();
        if was_down {
            self.recovered_at = Some(now);
        }
        self.consecutive_failures = 0;
        self.opens = 0;
        self.open_until = None;
        self.probing_since = None;
        if self
            .last_success_at
            .is_none_or(|t| now - t >= SUCCESS_STAMP_EVERY)
        {
            self.last_success_at = Some(now);
        }
        was_down
    }

    /// A transient failure. Returns the hold when the lane (re)opened.
    fn on_failure(&mut self, message: &str, now: DateTime<Utc>) -> Option<chrono::Duration> {
        self.consecutive_failures += 1;
        self.last_failure_at = Some(now);
        self.last_error = Some(message.chars().take(300).collect());
        let probe_failed = self.probing_since.is_some();
        if probe_failed || self.consecutive_failures >= OPEN_AFTER {
            self.opens += 1;
            let step = (self.opens - 1).clamp(0, 8);
            let dur = std::cmp::min(OPEN_BASE * (1 << step), OPEN_MAX);
            self.open_until = Some(now + dur);
            self.probing_since = None;
            return Some(dur);
        }
        None
    }

    /// Whether a call must wait now; the first caller after the open
    /// period becomes the probe.
    fn hold(&mut self, now: DateTime<Utc>) -> Option<Duration> {
        match self.open_until {
            None => None,
            Some(t) if t > now => Some(to_std(t - now)),
            Some(_) => match self.probing_since {
                Some(since) if now - since < PROBE_TIMEOUT => Some(Duration::from_secs(5)),
                _ => {
                    self.probing_since = Some(now);
                    None
                }
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = github_rate_limits, treat_none_as_null = true)]
struct RateRow {
    token_label: String,
    remaining: i64,
    limit: i64,
    reset_at: DateTime<Utc>,
    /// Set by a `Retry-After` or an exhausted window.
    hold_until: Option<DateTime<Utc>>,
    seen_at: DateTime<Utc>,
}

impl RateRow {
    fn hold(&self, now: DateTime<Utc>) -> Option<Duration> {
        if let Some(t) = self.hold_until {
            if t > now {
                return Some(to_std(t - now));
            }
        }
        if self.limit > 0 && self.remaining < reserve(self.limit) && self.reset_at > now {
            return Some(to_std(self.reset_at - now));
        }
        None
    }
}

fn reserve(limit: i64) -> i64 {
    std::cmp::max(RESERVE_FLOOR, limit / 50)
}

fn to_std(d: chrono::Duration) -> Duration {
    d.to_std().unwrap_or(Duration::ZERO)
}

// ---------------------------------------------------------------------
// Storage: the shared tables, or process memory when none is attached
// ---------------------------------------------------------------------

static DB: OnceLock<DbPool> = OnceLock::new();

#[derive(Default)]
struct Mem {
    lanes: HashMap<String, LaneRow>,
    rates: HashMap<String, RateRow>,
}

fn mem() -> std::sync::MutexGuard<'static, Mem> {
    static M: OnceLock<Mutex<Mem>> = OnceLock::new();
    M.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Use the shared tables from now on. Called once by `App::new`; a second
/// call (another `App` in the same process) is ignored.
pub fn attach(db: DbPool) {
    let _ = DB.set(db);
}

type LaneFn<'a, R> = Box<dyn FnOnce(&mut LaneRow, DateTime<Utc>) -> R + 'a>;

/// Load the lane row, let `f` change it, and store it back, all under the
/// row lock so two nodes never race on a transition.
fn with_lane<R>(lane: Lane, f: impl FnOnce(&mut LaneRow, DateTime<Utc>) -> R) -> R {
    let now = Utc::now();
    let Some(db) = DB.get() else {
        let mut m = mem();
        let row = m
            .lanes
            .entry(lane.as_str().to_string())
            .or_insert_with(|| LaneRow::new(lane));
        return f(row, now);
    };
    let mut slot: Option<LaneFn<'_, R>> = Some(Box::new(f));
    let mut result: Option<R> = None;
    let attempt: Result<(), diesel::result::Error> = (|| {
        let mut conn = db
            .get()
            .map_err(|_| diesel::result::Error::BrokenTransactionManager)?;
        conn.transaction::<(), diesel::result::Error, _>(|conn| {
            let mut row: LaneRow = github_lanes::table
                .find(lane.as_str())
                .for_update()
                .select(LaneRow::as_select())
                .first(conn)
                .optional()?
                .unwrap_or_else(|| LaneRow::new(lane));
            let before = row.clone();
            let f = slot.take().expect("closure runs once");
            result = Some(f(&mut row, now));
            if row != before {
                diesel::insert_into(github_lanes::table)
                    .values(&row)
                    .on_conflict(github_lanes::lane)
                    .do_update()
                    .set(&row)
                    .execute(conn)?;
            }
            Ok(())
        })
    })();
    match (attempt, result, slot) {
        (Ok(()), Some(r), _) => r,
        (Err(e), Some(r), _) => {
            // The closure ran but the write did not commit: the lane keeps
            // its previous state, which is the safe direction.
            tracing::warn!(error = %e, lane = lane.as_str(), "github lane state not written");
            r
        }
        (Err(e), None, Some(f)) => {
            tracing::warn!(error = %e, lane = lane.as_str(), "github lane state unavailable; using a scratch row");
            f(&mut LaneRow::new(lane), now)
        }
        _ => unreachable!("closure neither ran nor remains"),
    }
}

fn lane_row(lane: Lane) -> LaneRow {
    if let Some(db) = DB.get() {
        if let Ok(mut conn) = db.get() {
            if let Ok(Some(row)) = github_lanes::table
                .find(lane.as_str())
                .select(LaneRow::as_select())
                .first::<LaneRow>(&mut conn)
                .optional()
            {
                return row;
            }
        }
        return LaneRow::new(lane);
    }
    mem()
        .lanes
        .get(lane.as_str())
        .cloned()
        .unwrap_or_else(|| LaneRow::new(lane))
}

fn put_rate(row: RateRow) {
    if let Some(db) = DB.get() {
        if let Ok(mut conn) = db.get() {
            if let Err(e) = diesel::insert_into(github_rate_limits::table)
                .values(&row)
                .on_conflict(github_rate_limits::token_label)
                .do_update()
                .set(&row)
                .execute(&mut conn)
            {
                tracing::warn!(error = %e, "github rate limit row not written");
            }
        }
        return;
    }
    mem().rates.insert(row.token_label.clone(), row);
}

fn rate_row(token: &str) -> Option<RateRow> {
    if let Some(db) = DB.get() {
        let mut conn = db.get().ok()?;
        return github_rate_limits::table
            .find(token)
            .select(RateRow::as_select())
            .first::<RateRow>(&mut conn)
            .optional()
            .ok()
            .flatten();
    }
    mem().rates.get(token).cloned()
}

fn rate_rows() -> Vec<RateRow> {
    if let Some(db) = DB.get() {
        let Ok(mut conn) = db.get() else {
            return vec![];
        };
        return github_rate_limits::table
            .select(RateRow::as_select())
            .load(&mut conn)
            .unwrap_or_default();
    }
    mem().rates.values().cloned().collect()
}

// ---------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------

/// A call on the lane succeeded. Closes an open or half-open lane.
pub fn record_success(lane: Lane) {
    let (was_down, failures) = with_lane(lane, |st, now| {
        let failures = st.consecutive_failures;
        (st.on_success(now), failures)
    });
    if was_down {
        tracing::info!(
            lane = lane.as_str(),
            after_failures = failures,
            "GitHub lane recovered"
        );
        crate::metrics::github_lane(lane.as_str(), false);
        crate::metrics::github_lane_transition(lane.as_str(), "closed");
    }
}

/// A transient failure (5xx, network, rate limit) on the lane.
pub fn record_failure(lane: Lane, message: &str) {
    let (opened, failures) = with_lane(lane, |st, now| {
        (st.on_failure(message, now), st.consecutive_failures)
    });
    if let Some(dur) = opened {
        tracing::warn!(lane = lane.as_str(), failures, hold_s = dur.num_seconds(), error = %message.chars().take(300).collect::<String>(), "GitHub lane opened; holding calls");
        crate::metrics::github_lane(lane.as_str(), true);
        crate::metrics::github_lane_transition(lane.as_str(), "open");
    }
}

/// Whether to hold a call on the lane now: `Some(wait)` while the lane is
/// open (or another probe is in flight), `None` when the call may go. The
/// first caller after the open period ends is the probe.
pub fn hold(lane: Lane) -> Option<Duration> {
    with_lane(lane, |st, now| st.hold(now))
}

/// True while the lane is open (a plain read; for schedulers).
pub fn is_open(lane: Lane) -> bool {
    lane_row(lane).is_open(Utc::now())
}

/// When the lane last came back after being open.
pub fn recovered_at(lane: Lane) -> Option<DateTime<Utc>> {
    lane_row(lane).recovered_at
}

// ---------------------------------------------------------------------
// Rate limits
// ---------------------------------------------------------------------

/// Rate limit headers of one response for `token` (a label, never the
/// secret). `reset_at` is the window end GitHub reported.
pub fn note_rate_limit(token: &str, remaining: u64, limit: u64, reset_at: DateTime<Utc>) {
    let now = Utc::now();
    let mut row = rate_row(token).unwrap_or(RateRow {
        token_label: token.to_string(),
        remaining: 0,
        limit: 0,
        reset_at,
        hold_until: None,
        seen_at: now,
    });
    row.remaining = remaining.min(i64::MAX as u64) as i64;
    row.limit = limit.min(i64::MAX as u64) as i64;
    row.reset_at = reset_at;
    row.seen_at = now;
    if remaining == 0 && reset_at > now {
        row.hold_until = Some(reset_at);
    }
    put_rate(row);
    crate::metrics::github_rate_limit_remaining(token, remaining);
}

/// GitHub asked for a pause on this token (`Retry-After`, secondary limit).
pub fn note_retry_after(token: &str, wait: Duration) {
    let now = Utc::now();
    let until = now + chrono::Duration::from_std(wait).unwrap_or(chrono::Duration::seconds(60));
    let mut row = rate_row(token).unwrap_or(RateRow {
        token_label: token.to_string(),
        remaining: 0,
        limit: 0,
        reset_at: until,
        hold_until: None,
        seen_at: now,
    });
    row.hold_until = Some(match row.hold_until {
        Some(t) if t > until => t,
        _ => until,
    });
    row.seen_at = now;
    put_rate(row);
}

/// Whether calls with `token` should wait: `Some(wait)` while a
/// `Retry-After` holds or the remaining budget is below the reserve and
/// the window has not reset.
pub fn rate_hold(token: &str) -> Option<Duration> {
    rate_row(token)?.hold(Utc::now())
}

/// The longest hold in force on any lane or token, for a worker that
/// parks a task after a transient failure.
pub fn longest_hold() -> Option<Duration> {
    let now = Utc::now();
    let mut best: Option<Duration> = None;
    for lane in [Lane::Api, Lane::Git] {
        if let Some(t) = lane_row(lane).open_until {
            if t > now {
                let d = to_std(t - now);
                best = Some(best.map_or(d, |b| b.max(d)));
            }
        }
    }
    for row in rate_rows() {
        if let Some(d) = row.hold(now) {
            best = Some(best.map_or(d, |b| b.max(d)));
        }
    }
    best
}

/// Lane and rate limit state for the `Mirror/health` op.
pub fn snapshot() -> (Vec<LaneHealth>, Vec<RateLimitInfo>) {
    let now = Utc::now();
    let lanes_wire: Vec<LaneHealth> = [Lane::Api, Lane::Git]
        .iter()
        .map(|lane| {
            let st = lane_row(*lane);
            LaneHealth {
                lane: lane.as_str().into(),
                state: st.state_name(now).into(),
                consecutive_failures: st.consecutive_failures.max(0) as u64,
                open_until: st.open_until.filter(|t| *t > now).map(to_ms),
                last_error: st.last_error.clone(),
                last_failure_at: st.last_failure_at.map(to_ms),
                last_success_at: st.last_success_at.map(to_ms),
                recovered_at: st.recovered_at.map(to_ms),
            }
        })
        .collect();
    let mut rate_wire: Vec<RateLimitInfo> = rate_rows()
        .into_iter()
        .filter(|r| now - r.seen_at < chrono::Duration::hours(2))
        .map(|r| RateLimitInfo {
            token: r.token_label.clone(),
            remaining: r.remaining.max(0) as u64,
            limit: r.limit.max(0) as u64,
            reset_at: to_ms(r.reset_at),
            held: r.hold(now).is_some(),
        })
        .collect();
    rate_wire.sort_by(|a, b| a.token.cmp(&b.token));
    (lanes_wire, rate_wire)
}

/// A short, stable label for a token value that has no better name.
pub fn token_label(value: &str) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(value.as_bytes());
    format!("tok:{:02x}{:02x}{:02x}{:02x}", d[0], d[1], d[2], d[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state machine, driven with explicit clocks on plain rows.
    #[test]
    fn three_failures_open_the_lane_and_a_probe_closes_it() {
        let t0 = Utc::now();
        let mut st = LaneRow::new(Lane::Git);
        assert!(st.hold(t0).is_none());
        assert!(st.on_failure("502", t0).is_none());
        assert!(
            st.on_failure("502", t0).is_none(),
            "two failures keep it closed"
        );
        let opened = st.on_failure("502", t0).expect("third opens");
        assert_eq!(opened, chrono::Duration::seconds(30));
        let wait = st.hold(t0).expect("open");
        assert!(wait >= Duration::from_secs(29) && wait <= Duration::from_secs(30));
        assert!(st.is_open(t0));
        assert_eq!(st.state_name(t0), "open");
        // After the window one probe goes; a second caller waits briefly.
        let t1 = t0 + chrono::Duration::seconds(31);
        assert_eq!(st.state_name(t1), "half_open");
        assert!(st.hold(t1).is_none(), "the probe");
        assert_eq!(st.hold(t1), Some(Duration::from_secs(5)));
        // The probe fails: open twice as long.
        let reopened = st.on_failure("502", t1).expect("reopened");
        assert_eq!(reopened, chrono::Duration::seconds(60));
        let t2 = t1 + chrono::Duration::seconds(61);
        assert!(st.hold(t2).is_none());
        assert!(st.on_success(t2), "was down");
        assert!(st.hold(t2).is_none());
        assert_eq!(st.recovered_at, Some(t2));
        assert_eq!(st.state_name(t2), "closed");
        assert_eq!(st.consecutive_failures, 0);
        assert!(!st.on_success(t2), "already up");
    }

    #[test]
    fn a_stuck_probe_frees_its_slot() {
        let t0 = Utc::now();
        let mut st = LaneRow::new(Lane::Api);
        for _ in 0..3 {
            st.on_failure("503", t0);
        }
        let t1 = t0 + chrono::Duration::seconds(31);
        assert!(st.hold(t1).is_none());
        assert!(st.hold(t1).is_some());
        let t2 = t1 + PROBE_TIMEOUT + chrono::Duration::seconds(1);
        assert!(st.hold(t2).is_none(), "a new probe after the timeout");
    }

    #[test]
    fn open_period_caps_at_ten_minutes_and_success_stamps_are_sparse() {
        let t0 = Utc::now();
        let mut st = LaneRow::new(Lane::Api);
        let mut last = chrono::Duration::zero();
        for i in 0..12 {
            let t = t0 + chrono::Duration::minutes(i * 20);
            if let Some(d) = st.on_failure("x", t) {
                last = d;
            }
            st.hold(t + chrono::Duration::minutes(11));
        }
        assert_eq!(last, OPEN_MAX);
        let mut up = LaneRow::new(Lane::Git);
        up.on_success(t0);
        let before = up.clone();
        up.on_success(t0 + chrono::Duration::seconds(10));
        assert_eq!(up, before, "no write within a minute of the last stamp");
    }

    #[test]
    fn rate_limits_hold_below_the_reserve_and_on_retry_after() {
        // Process memory: no database is attached in unit tests.
        let token = format!("user:{}", uuid::Uuid::now_v7().simple());
        let reset = Utc::now() + chrono::Duration::minutes(10);
        note_rate_limit(&token, 4000, 5000, reset);
        assert!(rate_hold(&token).is_none());
        note_rate_limit(&token, 99, 5000, reset);
        let wait = rate_hold(&token).expect("below the 2 % reserve");
        assert!(wait > Duration::from_secs(500));
        note_rate_limit(&token, 4000, 5000, reset);
        assert!(rate_hold(&token).is_none());
        note_retry_after(&token, Duration::from_secs(45));
        let wait = rate_hold(&token).expect("retry-after");
        assert!(wait <= Duration::from_secs(45) && wait > Duration::from_secs(40));
        assert!(longest_hold().is_some());
        let (_, rates) = snapshot();
        assert!(rates.iter().any(|r| r.token == token && r.held));
        assert!(rate_hold("unknown").is_none());
    }

    #[test]
    fn memory_backend_runs_the_lane_machine() {
        // Unit tests share the process; use the api lane only here and
        // leave it closed at the end.
        record_success(Lane::Api);
        assert!(hold(Lane::Api).is_none());
        assert!(!is_open(Lane::Api));
        record_failure(Lane::Api, "x");
        record_success(Lane::Api);
        assert_eq!(lane_row(Lane::Api).consecutive_failures, 0);
    }

    #[test]
    fn token_labels_hide_the_value() {
        let l = token_label("ghp_secret");
        assert!(l.starts_with("tok:") && l.len() == 12);
        assert!(!l.contains("secret"));
    }
}
