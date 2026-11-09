//! Domain logic: configuration, error registry, principals and authorization,
//! the outbox writer, service implementations, and the dispatch table that
//! maps every contract operation to its handler.

pub mod authz;
pub mod config;
pub mod dispatch;
pub mod errors;
pub mod linkkeys;
pub mod outbox;
pub mod principal;
pub mod services;
pub mod tokens;

use std::sync::Arc;

use vscstar_db::DbPool;

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
}

impl App {
    pub fn new(config: Config, db: DbPool) -> Self {
        let rp: Option<Arc<dyn linkkeys::RpCaller>> =
            match linkkeys::TcpRpCaller::from_config(&config.linkkeys) {
                Ok(c) => Some(Arc::new(c)),
                Err(e) => {
                    tracing::warn!(reason = %e.message, "LinkKeys login disabled");
                    None
                }
            };
        Self {
            config: Arc::new(config),
            db,
            started_at: chrono::Utc::now(),
            rp,
        }
    }

    /// Build with an explicit RP caller (tests, fakes).
    pub fn with_rp(config: Config, db: DbPool, rp: Arc<dyn linkkeys::RpCaller>) -> Self {
        Self {
            config: Arc::new(config),
            db,
            started_at: chrono::Utc::now(),
            rp: Some(rp),
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

    pub fn conn(&self) -> Result<vscstar_db::DbConn, Error> {
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
