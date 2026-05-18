//! `FakeClock` — virtual time. `sleep` records the request and returns
//! immediately. Tests advance time explicitly via `advance`.

use std::sync::Mutex;
use std::time::Duration;

use time::OffsetDateTime;

use crate::core::ports::Clock;

pub struct FakeClock {
    state: Mutex<State>,
}

struct State {
    now: OffsetDateTime,
    sleeps: Vec<Duration>,
}

impl FakeClock {
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            state: Mutex::new(State {
                now: start,
                sleeps: Vec::new(),
            }),
        }
    }

    pub fn epoch() -> Self {
        Self::new(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap())
    }

    pub fn advance(&self, d: Duration) {
        let mut s = self.state.lock().unwrap();
        s.now += time::Duration::new(
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            i32::try_from(d.subsec_nanos()).unwrap_or(0),
        );
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.state.lock().unwrap().sleeps.clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        self.state.lock().unwrap().now
    }

    fn sleep(&self, d: Duration) {
        let mut s = self.state.lock().unwrap();
        s.sleeps.push(d);
        s.now += time::Duration::new(
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            i32::try_from(d.subsec_nanos()).unwrap_or(0),
        );
    }
}
