//! Domain logic: configuration, error registry, principals and authorization,
//! the outbox writer, service implementations, and the dispatch table that
//! maps every contract operation to its handler.

pub mod authz;
pub mod config;
pub mod dispatch;
pub mod errors;
pub mod events_json;
pub mod github;
pub mod github_app;
pub mod github_health;
pub mod github_link;
pub mod github_sync;
pub mod instance_keys;
pub mod linkkeys;
pub mod metrics;
pub mod mirror;
pub mod mirror_import;
pub mod outbox;
pub mod principal;
pub mod push;
pub mod queue;
pub mod ratelimit;
pub mod services;
pub mod tokens;
pub mod vcs;

use std::sync::Arc;

use vcsstar_db::DbPool;

pub use config::Config;
pub use errors::{Code, Error};

/// Everything a request handler needs. Cheap to clone; shared by every role.
#[derive(Clone)]
pub struct App {
    pub config: Arc<Config>,
    pub db: DbPool,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Delegated RP client. `None` when LinkKeys login is not configured;
    /// token-based access still works.
    pub rp: Option<Arc<dyn linkkeys::RpCaller>>,
    /// Repository cache directories and the VCS backend.
    pub store: Arc<vcsstar_store::LocalStore>,
    /// Pushes in flight on this node, keyed by push id for the hook.
    pub pushes: Arc<push::PushRegistry>,
    /// Lazily dialed corndogs connection for request-cycle submissions
    /// (ingest-first receivers). Workers hold their own connections.
    pub queue: Arc<std::sync::Mutex<Option<queue::Queue>>>,
    /// Per-node token buckets for the login entry points.
    pub limiter: Arc<ratelimit::Limiter>,
    /// GitHub App installation tokens minted on this node.
    pub github_app_tokens: Arc<github_app::TokenCache>,
}

impl App {
    pub fn new(config: Config, db: DbPool) -> Self {
        github_health::attach(db.clone());
        let rp: Option<Arc<dyn linkkeys::RpCaller>> =
            match linkkeys::TcpRpCaller::from_config(&config.linkkeys) {
                Ok(c) => Some(Arc::new(c)),
                Err(e) => {
                    tracing::warn!(reason = %e.message, "LinkKeys login disabled");
                    None
                }
            };
        Self::build(config, db, rp)
    }

    /// Build with an explicit RP caller (tests, fakes).
    pub fn with_rp(config: Config, db: DbPool, rp: Arc<dyn linkkeys::RpCaller>) -> Self {
        Self::build(config, db, Some(rp))
    }

    fn build(config: Config, db: DbPool, rp: Option<Arc<dyn linkkeys::RpCaller>>) -> Self {
        let mut local = vcsstar_store::LocalStore::new(
            &config.repo_cache_dir,
            Arc::new(vcsstar_vcs::GitBackend::new(Some(
                config.repo_cache_dir.join("hooks"),
            ))),
        );
        if let Some(os) = &config.object_store {
            match vcsstar_store::segments::S3Store::new(
                &os.endpoint,
                &os.bucket,
                &os.region,
                &os.access_key,
                &os.secret_key,
            ) {
                Ok(s3) => {
                    tracing::info!(bucket = %os.bucket, endpoint = %os.endpoint, "history segments in object store");
                    local = local.with_segments(Arc::new(s3));
                }
                Err(e) => {
                    tracing::error!(error = %e, "object store misconfigured; history stays on local disk")
                }
            }
        } else {
            tracing::warn!("no object store configured; history lives on this node's disk only");
        }
        let store = Arc::new(local);
        Self {
            config: Arc::new(config),
            db,
            started_at: chrono::Utc::now(),
            rp,
            store,
            pushes: Arc::new(push::PushRegistry::default()),
            queue: Arc::new(std::sync::Mutex::new(None)),
            limiter: Arc::new(ratelimit::Limiter::new()),
            github_app_tokens: Arc::new(github_app::TokenCache::default()),
        }
    }

    /// Submit one keyed task from a request cycle. Dials on first use and
    /// drops the connection on a transport error so the next call re-dials.
    pub fn submit_task(
        &self,
        queue_name: &str,
        key: &str,
        payload: Vec<u8>,
        priority: i64,
    ) -> Result<(), Error> {
        let mut guard = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(
                queue::Queue::connect(&self.config)
                    .map_err(|e| Error::new(Code::Unavailable, format!("corndogs: {e}")))?,
            );
        }
        let q = guard.as_ref().expect("connected");
        match q.submit(queue_name, key, payload, priority) {
            Ok(_) => Ok(()),
            // Code 4: the key was submitted before with a different payload.
            // The work is already captured; the connection is fine.
            Err(queue::QueueError::Client(corndogs::ClientError::Service { code: 4, message })) => {
                Err(Error::new(
                    Code::AlreadyExists,
                    format!("corndogs: a task with key {key} is already captured: {message}"),
                ))
            }
            Err(e) => {
                *guard = None;
                Err(Error::new(Code::Unavailable, format!("corndogs: {e}")))
            }
        }
    }

    pub fn rp(&self) -> Result<linkkeys::Rp<'_>, Error> {
        self.rp
            .as_deref()
            .map(|caller| linkkeys::Rp { caller })
            .ok_or_else(|| {
                Error::new(
                    Code::Unavailable,
                    "LinkKeys login is not configured on this instance",
                )
            })
    }

    pub fn conn(&self) -> Result<vcsstar_db::DbConn, Error> {
        self.db
            .get()
            .map_err(|e| Error::new(Code::Unavailable, format!("database: {e}")))
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn to_ms(t: chrono::DateTime<chrono::Utc>) -> i64 {
    t.timestamp_millis()
}

pub fn from_ms(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms).unwrap_or_else(chrono::Utc::now)
}
