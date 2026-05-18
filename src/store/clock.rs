//! `SystemClock` — the real-world `Clock` impl.

use std::time::Duration;

use time::OffsetDateTime;

use crate::core::ports::Clock;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
