//! Connection sync tasks share the mirror-worker role.
use crate::queue::Queue;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use vcsstar_core::{star::engine, App};

pub fn run(app: App, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let mut queue = None;
    let mut scheduled = Instant::now() - Duration::from_secs(60);
    while !stop.load(Ordering::SeqCst) {
        if scheduled.elapsed() >= Duration::from_secs(10) {
            if let Err(e) = engine::schedule(&app) {
                tracing::warn!(error=%e,"connection scheduling failed");
            }
            scheduled = Instant::now();
        }
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => queue = Some(q),
                Err(e) => {
                    tracing::warn!(error=%e,"connection queue unavailable");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let q = queue.as_ref().expect("connected");
        match q.claim(&[engine::QUEUE.into()], Duration::from_secs(1800)) {
            Ok(Some(claimed)) => {
                let result = match crate::decode::<engine::Task>(&claimed.payload) {
                    None => q.complete(&claimed),
                    Some(task) => match engine::run(&app, &task) {
                        Ok(true) => q.complete(&claimed),
                        Ok(false) => q.park(&claimed, Duration::from_secs(15)),
                        Err(e) => {
                            tracing::warn!(connection=%task.connection_id,error=%e,"connection sync failed");
                            if task.webhook.is_some() {
                                q.park(&claimed, Duration::from_secs(60))
                            } else {
                                q.complete(&claimed)
                            } // The connection row schedules the next poll.
                        }
                    },
                };
                if let Err(e) = result {
                    tracing::warn!(error=%e,"connection task acknowledgement failed; reconnecting");
                    queue = None;
                }
            }
            Ok(None) => crate::pause(&stop, Duration::from_secs(2)),
            Err(e) => {
                tracing::warn!(error=%e,"connection claim failed");
                queue = None;
                crate::pause(&stop, Duration::from_secs(3));
            }
        }
    }
    Ok(())
}
