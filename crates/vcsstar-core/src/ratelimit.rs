//! In-process token buckets for the unauthenticated entry points (login
//! begin and complete, device begin, poll, and approve). One bucket per
//! key; keys name the peer address, the identity typed into the form, the
//! device code, or the approving principal. The limits are per node, which
//! is enough to stop online guessing of short device codes and to keep one
//! client from flooding the RP with sign-request calls.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::{Code, Error};

/// Buckets above this count trigger a sweep of full buckets.
const SWEEP_AT: usize = 20_000;

struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl Limiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one unit from bucket `key`. The bucket holds `per_minute` units,
    /// refills at that rate, and starts full. `Err` carries
    /// `ResourceExhausted` and names `what` for the caller.
    pub fn take(&self, key: &str, per_minute: u32, what: &str) -> Result<(), Error> {
        self.take_at(key, per_minute, what, Instant::now())
    }

    fn take_at(&self, key: &str, per_minute: u32, what: &str, now: Instant) -> Result<(), Error> {
        if per_minute == 0 {
            return Ok(());
        }
        let capacity = per_minute as f64;
        let per_second = capacity / 60.0;
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= SWEEP_AT {
            map.retain(|_, b| {
                let refilled = b.tokens + now.duration_since(b.updated).as_secs_f64() * per_second;
                refilled < capacity
            });
        }
        let bucket = map.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Ok(());
        }
        let retry_after = ((1.0 - bucket.tokens) / per_second).ceil().max(1.0) as u64;
        drop(map);
        crate::metrics::rate_limited(what);
        Err(exhausted(what, retry_after))
    }
}

/// The error a refused request receives. The message carries the wait so a
/// human reads it; `detail` carries it in machine form for clients that
/// back off (`retry_after_s=<seconds>`).
pub fn exhausted(what: &str, retry_after: u64) -> Error {
    Error::new(
        Code::ResourceExhausted,
        format!("too many {what} attempts; try again in {retry_after} s"),
    )
    .with_detail(format!("retry_after_s={retry_after}"))
}

/// Seconds a client should wait, from an error's `detail` or message.
pub fn retry_after_secs(message: &str, detail: Option<&str>) -> Option<u64> {
    if let Some(d) = detail {
        if let Some(v) = d.strip_prefix("retry_after_s=") {
            return v.trim().parse().ok();
        }
    }
    let rest = message.split("try again in ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// A stable, non-reversible bucket key for a secret such as a device code.
pub fn key_of_secret(prefix: &str, secret: &str) -> String {
    let digest = crate::tokens::hash(secret.trim());
    format!("{prefix}:{}", hex::encode(&digest[..8]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bucket_empties_then_refills() {
        let l = Limiter::new();
        let t0 = Instant::now();
        for _ in 0..6 {
            l.take_at("k", 6, "login", t0).expect("within burst");
        }
        let err = l.take_at("k", 6, "login", t0).expect_err("exhausted");
        assert_eq!(err.code, Code::ResourceExhausted);
        assert_eq!(err.detail.as_deref(), Some("retry_after_s=10"));
        assert!(
            err.message.ends_with("try again in 10 s"),
            "{}",
            err.message
        );
        assert_eq!(retry_after_secs(&err.message, None), Some(10));
        assert_eq!(retry_after_secs("x", err.detail.as_deref()), Some(10));
        // 6 per minute is one every 10 seconds.
        l.take_at("k", 6, "login", t0 + Duration::from_secs(10))
            .expect("one token back");
        l.take_at("k", 6, "login", t0 + Duration::from_secs(10))
            .expect_err("only one came back");
        l.take_at("other", 6, "login", t0)
            .expect("keys are independent");
    }

    #[test]
    fn zero_disables() {
        let l = Limiter::new();
        for _ in 0..100 {
            l.take("k", 0, "x").expect("disabled");
        }
    }

    #[test]
    fn secret_keys_do_not_leak_the_secret() {
        let k = key_of_secret("poll", "vsd_supersecret");
        assert!(k.starts_with("poll:"));
        assert!(!k.contains("supersecret"));
        assert_eq!(k, key_of_secret("poll", " vsd_supersecret "));
    }
}
