//! The sweeper role: `CleanUpTimedOut` on every queue every two seconds.
//! Retry, backoff, and crashed-worker recovery all stop when this stops,
//! so it runs alone and reconnects on any error.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use vcsstar_core::App;

use crate::queue::Queue;

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    while !stop.load(Ordering::SeqCst) {
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => {
                    tracing::info!("sweeper connected to corndogs");
                    queue = Some(q);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "sweeper cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let q = queue.as_ref().expect("connected");
        let result = q.queues().and_then(|names| {
            let mut total = 0;
            for name in names {
                total += q.sweep(&name)?;
            }
            Ok(total)
        });
        match result {
            Ok(n) if n > 0 => tracing::debug!(released = n, "sweep released tasks"),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "sweep failed; reconnecting");
                queue = None;
            }
        }
        crate::pause(&stop, Duration::from_secs(2));
    }
    Ok(())
}
