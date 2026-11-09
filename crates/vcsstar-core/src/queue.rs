//! corndogs adapter: keyed submissions and guarded claims only (0.8.0
//! `required` policies). One `Queue` is one connection; give each loop its
//! own. The transport never resends after an uncertain outcome, so callers
//! rebuild the same request (same key or operation id) and call again.

use std::time::Duration;

use crate::Config;
use corndogs::tls::TlsOptions;
use corndogs::transport::{ConnectOptions, Transport};
use corndogs::{
    ClaimGuardedTaskGroupRequest, CleanUpTimedOutRequest, ClientError, CompleteGuardedTaskRequest,
    CorndogsClient, GetQueuesRequest, GetServerInfoRequest, SubmitKeyedTaskRequest,
    UpdateGuardedTaskRequest,
};

pub const STATE_PENDING: &str = "pending";
pub const STATE_WORKING: &str = "working";
pub const STATE_BACKOFF: &str = "backoff";
pub const REQUIRED_FEATURES: [&str; 2] = ["submission-keys-v1", "task-guards-v1"];

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("corndogs is not configured (VCSSTAR__CORNDOGS_ADDR)")]
    NotConfigured,
    #[error("corndogs: {0}")]
    Client(#[from] ClientError),
    #[error("corndogs server lacks feature {0}")]
    MissingFeature(&'static str),
}

pub struct Queue {
    client: CorndogsClient<Transport>,
    pub receipt_retention: Duration,
}

/// One claimed task.
#[derive(Debug, Clone)]
pub struct Claimed {
    pub uuid: String,
    pub queue: String,
    pub payload: Vec<u8>,
    pub revision: i64,
    pub current_state: String,
}

impl Queue {
    pub fn connect(cfg: &Config) -> Result<Self, QueueError> {
        let addr = cfg.corndogs_addr.clone().ok_or(QueueError::NotConfigured)?;
        let mut opts = ConnectOptions::new()
            .connect_timeout(Duration::from_secs(5))
            .io_timeout(Duration::from_secs(30));
        if cfg.corndogs_tls {
            opts = opts.tls(TlsOptions::system_roots());
        }
        let transport = Transport::connect_options(addr, opts)?;
        let client = CorndogsClient::new(transport);
        let info = client.get_server_info(GetServerInfoRequest {})?;
        for f in REQUIRED_FEATURES {
            if !info.features.iter().any(|h| h == f) {
                return Err(QueueError::MissingFeature(f));
            }
        }
        Ok(Self {
            client,
            receipt_retention: Duration::from_secs(info.receipt_retention_seconds.max(60) as u64),
        })
    }

    /// Submit one unit of work. The key makes producer retries idempotent
    /// within the receipt retention window.
    pub fn submit(
        &self,
        queue: &str,
        key: &str,
        payload: Vec<u8>,
        priority: i64,
    ) -> Result<bool, QueueError> {
        let resp = self.client.submit_keyed_task(SubmitKeyedTaskRequest {
            submission_key: key.to_string(),
            guarded: true,
            queue: queue.to_string(),
            current_state: STATE_PENDING.into(),
            auto_target_state: STATE_WORKING.into(),
            timeout: -1,
            payload,
            priority,
        })?;
        Ok(!resp.replayed)
    }

    /// Claim the next pending task on any of `queues` with a lease.
    pub fn claim(&self, queues: &[String], lease: Duration) -> Result<Option<Claimed>, QueueError> {
        if queues.is_empty() {
            return Ok(None);
        }
        let resp = self
            .client
            .claim_guarded_task_group(ClaimGuardedTaskGroupRequest {
                operation_id: uuid::Uuid::now_v7().to_string(),
                queues: queues.to_vec(),
                current_state: STATE_PENDING.into(),
                override_timeout: lease.as_secs().max(1) as i64,
                override_current_state: String::new(),
                override_auto_target_state: STATE_PENDING.into(),
            })?;
        Ok(resp.delivery.map(|d| Claimed {
            uuid: d.task.task.uuid,
            queue: d.task.task.queue,
            payload: d.payload,
            revision: d.task.revision,
            current_state: d.task.task.current_state,
        }))
    }

    /// The work is done. `Ok(false)` means the claim was no longer ours.
    pub fn complete(&self, c: &Claimed) -> Result<bool, QueueError> {
        match self
            .client
            .complete_guarded_task(CompleteGuardedTaskRequest {
                operation_id: uuid::Uuid::now_v7().to_string(),
                uuid: c.uuid.clone(),
                queue: c.queue.clone(),
                expected_revision: c.revision,
                expected_state: Some(c.current_state.clone()),
            }) {
            Ok(_) => Ok(true),
            Err(e) if not_mine(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Park the task; the sweeper returns it to pending after `delay`.
    pub fn park(&self, c: &Claimed, delay: Duration) -> Result<bool, QueueError> {
        match self.client.update_guarded_task(UpdateGuardedTaskRequest {
            operation_id: uuid::Uuid::now_v7().to_string(),
            uuid: c.uuid.clone(),
            queue: c.queue.clone(),
            expected_revision: c.revision,
            expected_state: Some(c.current_state.clone()),
            new_state: STATE_BACKOFF.into(),
            auto_target_state: STATE_PENDING.into(),
            timeout: delay.as_secs().max(1) as i64,
            payload: None,
            priority: None,
        }) {
            Ok(_) => Ok(true),
            Err(e) if not_mine(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn queues(&self) -> Result<Vec<String>, QueueError> {
        Ok(self.client.get_queues(GetQueuesRequest {})?.queues)
    }

    /// Release expired leases and backoffs on one queue. corndogs evaluates
    /// timeouts only here.
    pub fn sweep(&self, queue: &str) -> Result<i64, QueueError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        Ok(self
            .client
            .clean_up_timed_out(CleanUpTimedOutRequest {
                at_time: now,
                queue: queue.to_string(),
            })?
            .timed_out)
    }
}

/// Guard codes 5 (stale revision), 7 (terminal), 8 (replayed claim lost):
/// the claim ended, which is not an outage.
fn not_mine(e: &ClientError) -> bool {
    matches!(e, ClientError::Service { code, .. } if matches!(code, 5 | 7 | 8))
}
