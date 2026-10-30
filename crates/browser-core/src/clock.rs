use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Monotonic time source. Lifecycle and network policy depend on elapsed time
/// only through this trait so they can be tested deterministically.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that only moves when told to.
#[derive(Clone)]
pub struct ManualClock {
    now: Arc<Mutex<Instant>>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self { now: Arc::new(Mutex::new(Instant::now())) }
    }
}

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        *self.now.lock() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock()
    }
}
