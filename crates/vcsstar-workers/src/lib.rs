//! Worker roles. corndogs is durable work capture only: one keyed task per
//! unit of pending work, guarded claims, idempotent workers that read the
//! desired state from Postgres at execution time.

pub mod dispatcher;
pub mod hooks;
pub mod ingest;
pub mod mirror;
pub mod repack;
pub mod sweeper;

pub use vcsstar_core::queue;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vcsstar_core::App;

/// Run one role until `stop` is set. Every loop body is fallible and the
/// loop logs and continues; a dead corndogs connection is re-dialed on the
/// next tick.
pub fn run_role(app: App, role: &str, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    match role {
        "dispatcher" => dispatcher::run(app, stop),
        "sweeper" => sweeper::run(app, stop),
        "hook-worker" => hooks::run(app, stop),
        "mirror-worker" => mirror::run(app, stop),
        "ingest-worker" => ingest::run(app, stop),
        "repack-worker" => repack::run(app, stop),
        other => anyhow::bail!("unknown role {other}"),
    }
}

/// Sleep in small steps so a stop request is noticed quickly.
pub(crate) fn pause(stop: &AtomicBool, d: Duration) {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Task payloads are small pointers into Postgres, encoded as CBOR.
pub(crate) fn encode<T: serde::Serialize>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(v, &mut out).expect("cbor encode");
    out
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(b: &[u8]) -> Option<T> {
    ciborium::de::from_reader(b).ok()
}
