//! `FakeClock`: время стоит, пока тест не позовёт `advance`.

use crate::clock::Clock;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct FakeClock {
    start: Instant,
    utc_start: DateTime<Utc>,
    elapsed: Mutex<Duration>,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeClock {
    /// Начало отсчёта — 2026-01-01T00:00:00Z.
    pub fn new() -> Self {
        Self::at(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap())
    }

    pub fn at(utc_start: DateTime<Utc>) -> Self {
        Self {
            start: Instant::now(),
            utc_start,
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap_or_else(|e| e.into_inner()) += by;
    }

    pub fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start + self.elapsed()
    }

    fn utc_now(&self) -> DateTime<Utc> {
        self.utc_start + ChronoDuration::from_std(self.elapsed()).expect("разумная длительность")
    }
}
