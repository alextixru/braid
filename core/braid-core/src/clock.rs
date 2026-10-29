//! Часы — шов для гистерезиса, окон проб, ротации журнала и интервалов
//! подписки: в тестах `FakeClock`, в приложении [`SystemClock`].

use chrono::{DateTime, Utc};
use std::time::Instant;

pub trait Clock: Send + Sync {
    /// Монотонное время — для интервалов и таймаутов.
    fn now(&self) -> Instant;
    /// Настенное время — для журнала и дат подписки.
    fn utc_now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn utc_now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
