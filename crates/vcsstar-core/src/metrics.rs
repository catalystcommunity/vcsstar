//! Process-wide Prometheus metrics. One registry per process; `vcsstard`
//! serves it at `GET /metrics` on the `api` role. Counters are created
//! once and looked up by name, so a call site never fails on a duplicate
//! registration.

use std::sync::OnceLock;

use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder};

struct Metrics {
    registry: Registry,
    rate_limited: IntCounterVec,
}

fn metrics() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(|| {
        let registry = Registry::new();
        let rate_limited = IntCounterVec::new(
            Opts::new(
                "vcsstar_rate_limited_total",
                "Requests refused by a login rate limit, by entry point",
            ),
            &["entry"],
        )
        .expect("static metric definition");
        registry
            .register(Box::new(rate_limited.clone()))
            .expect("fresh registry");
        Metrics {
            registry,
            rate_limited,
        }
    })
}

/// One refusal by the limiter guarding `entry` (`login`, `complete`,
/// `device_begin`, `device_poll`, `device_approve`).
pub fn rate_limited(entry: &str) {
    metrics().rate_limited.with_label_values(&[entry]).inc();
}

/// The registry in Prometheus text exposition format.
pub fn render() -> String {
    let families = metrics().registry.gather();
    let mut out = Vec::new();
    if TextEncoder::new().encode(&families, &mut out).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_shows_up_in_the_rendering() {
        rate_limited("test_entry");
        rate_limited("test_entry");
        let text = render();
        assert!(
            text.contains("vcsstar_rate_limited_total{entry=\"test_entry\"} 2"),
            "{text}"
        );
    }
}
