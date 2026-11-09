//! SIGTERM/SIGINT handling. A process that is PID 1 gets no default signal
//! action, so the binary installs its own handler and drains in order: fail
//! readiness, stop listeners, wait for in-flight work, exit 0.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

#[derive(Clone)]
pub struct State {
    flag: Arc<AtomicBool>,
}

impl State {
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }
    pub fn stopping(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    /// Request a stop from code (tests, admin ops).
    #[allow(dead_code)]
    pub fn request_stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
        STOP.store(true, Ordering::SeqCst);
    }
    /// Block until a stop is requested.
    pub fn wait(&self) {
        while !self.stopping() {
            if STOP.load(Ordering::SeqCst) {
                self.flag.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

pub fn install() -> State {
    // SAFETY: installing a signal handler that only stores an atomic flag.
    #[allow(unsafe_code)]
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    State {
        flag: Arc::new(AtomicBool::new(false)),
    }
}
