//! Repository imports from GitHub (queue `import:github`), threads of the
//! `mirror-worker` role. Each task is one repository; the work is
//! `vcsstar_core::mirror_import::run`, which is idempotent and claims the
//! repository, so a task given out twice (lease expiry after a pod
//! restart) does the import once. A failed attempt parks the task with a
//! backoff (1, 2, 4, 8, 16 minutes); after `MAX_ATTEMPTS` the import is
//! "failed" and a new `import-repo` request queues it again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vcsstar_core::mirror_import::{self, ImportTask, Outcome};
use vcsstar_core::App;

use crate::queue::{Claimed, Queue};

/// Import threads per node. A fetch is mostly network wait.
pub const THREADS: usize = 2;
/// A large repository can take minutes to fetch.
const LEASE: Duration = Duration::from_secs(1800);
/// How often a node looks for imports whose task was never submitted.
const SWEEP_EVERY: Duration = Duration::from_secs(120);

pub fn run(app: App, stop: Arc<AtomicBool>, sweeper: bool) -> anyhow::Result<()> {
    let mut queue: Option<Queue> = None;
    let queues = vec![mirror_import::QUEUE.to_string()];
    let mut last_sweep = Instant::now() - SWEEP_EVERY;
    while !stop.load(Ordering::SeqCst) {
        if sweeper && last_sweep.elapsed() >= SWEEP_EVERY {
            last_sweep = Instant::now();
            match mirror_import::resubmit_lost(&app) {
                Ok(0) => {}
                Ok(n) => tracing::info!(n, "import tasks submitted again"),
                Err(e) => tracing::warn!(error = %e.message, "import sweep failed"),
            }
        }
        if queue.is_none() {
            match Queue::connect(&app.config) {
                Ok(q) => queue = Some(q),
                Err(e) => {
                    tracing::warn!(error = %e, "import worker cannot reach corndogs");
                    crate::pause(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let mut drop_connection = false;
        {
            let q = queue.as_ref().expect("connected");
            match q.claim(&queues, LEASE) {
                Ok(Some(claimed)) => {
                    if let Err(e) = handle(&app, q, &claimed) {
                        tracing::warn!(error = %e, "import task failed");
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "import claim failed; reconnecting");
                    drop_connection = true;
                }
            }
        }
        if drop_connection {
            queue = None;
        }
        crate::pause(&stop, Duration::from_secs(3));
    }
    Ok(())
}

fn handle(app: &App, q: &Queue, claimed: &Claimed) -> anyhow::Result<()> {
    let Some(task): Option<ImportTask> = crate::decode(&claimed.payload) else {
        q.complete(claimed)?;
        return Ok(());
    };
    let _: bool = match mirror_import::run(app, task.repo_id) {
        Ok(Outcome::Done { .. }) | Ok(Outcome::Gone) => q.complete(claimed)?,
        Ok(Outcome::Failed { final_: true, .. }) => q.complete(claimed)?,
        Ok(Outcome::Failed { .. }) => {
            let attempts = app
                .conn()
                .ok()
                .and_then(|mut c| {
                    vcsstar_core::mirror::state_for(&mut c, task.repo_id)
                        .ok()
                        .flatten()
                })
                .map(|s| s.import_attempts)
                .unwrap_or(1)
                .clamp(1, 5);
            q.park(claimed, Duration::from_secs(60 << (attempts - 1)))?
        }
        Ok(Outcome::Held(wait)) => q.park(claimed, wait.max(Duration::from_secs(30)))?,
        Ok(Outcome::Busy) => q.park(claimed, Duration::from_secs(120))?,
        Err(e) => {
            tracing::warn!(repo = %task.repo_id, error = %e.message, "import errored; retry");
            q.park(claimed, Duration::from_secs(60))?
        }
    };
    Ok(())
}
