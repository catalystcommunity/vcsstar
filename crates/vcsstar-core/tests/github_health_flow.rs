//! The GitHub lane breaker and rate limit table live in Postgres when an
//! `App` exists, so every node shares them. Runs only with DATABASE_URL.

use diesel::prelude::*;
use vcsstar_core::github_health::{self as health, Lane};
use vcsstar_core::{App, Config};
use vcsstar_db::schema::{github_lanes, github_rate_limits};

fn app() -> Option<App> {
    let url = std::env::var("DATABASE_URL").ok()?;
    std::env::set_var("VCSSTAR__DATABASE_URL", &url);
    let config = Config::from_env().ok()?;
    let db = vcsstar_db::pool(&url, 2).ok()?;
    vcsstar_db::run_migrations(&mut *db.get().ok()?).ok()?;
    Some(App::new(config, db))
}

#[test]
fn lane_and_rate_state_are_shared_rows() {
    let Some(app) = app() else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let mut conn = app.conn().unwrap();
    // Start from a closed git lane whatever an earlier run left.
    health::record_success(Lane::Git);
    assert!(!health::is_open(Lane::Git));

    for _ in 0..3 {
        health::record_failure(Lane::Git, "fatal: unable to access: 503");
    }
    assert!(
        health::is_open(Lane::Git),
        "three transient failures open it"
    );
    let (failures, open_until): (i32, Option<chrono::DateTime<chrono::Utc>>) = github_lanes::table
        .find("git")
        .select((github_lanes::consecutive_failures, github_lanes::open_until))
        .first(&mut conn)
        .unwrap();
    assert_eq!(failures, 3);
    assert!(open_until.is_some(), "the row carries the hold");
    let wait = health::hold(Lane::Git).expect("held");
    assert!(wait.as_secs() <= 30);
    assert!(health::longest_hold().is_some());

    health::record_success(Lane::Git);
    assert!(!health::is_open(Lane::Git));
    assert!(health::hold(Lane::Git).is_none());
    let (failures, open_until, recovered): (
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = github_lanes::table
        .find("git")
        .select((
            github_lanes::consecutive_failures,
            github_lanes::open_until,
            github_lanes::recovered_at,
        ))
        .first(&mut conn)
        .unwrap();
    assert_eq!(failures, 0);
    assert!(open_until.is_none());
    assert!(
        recovered.is_some(),
        "recovery is recorded for the schedulers"
    );
    assert_eq!(health::recovered_at(Lane::Git), recovered);

    let token = format!("test:{}", uuid::Uuid::now_v7().simple());
    let reset = chrono::Utc::now() + chrono::Duration::minutes(5);
    health::note_rate_limit(&token, 10, 5000, reset);
    let remaining: i64 = github_rate_limits::table
        .find(&token)
        .select(github_rate_limits::remaining)
        .first(&mut conn)
        .unwrap();
    assert_eq!(remaining, 10);
    assert!(health::rate_hold(&token).is_some(), "below the reserve");
    let (_, rates) = health::snapshot();
    assert!(rates.iter().any(|r| r.token == token && r.held));
    diesel::delete(github_rate_limits::table.find(&token))
        .execute(&mut conn)
        .unwrap();
}
