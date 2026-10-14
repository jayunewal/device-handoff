// SPDX-License-Identifier: Apache-2.0

//! The current time, behind a trait so tests can control it.
//!
//! There are two kinds of time and they are kept apart on purpose. The *wall clock*
//! says what date it is; a person, a network time service or a daylight saving
//! change can set it, forwards or backwards, at any moment. It is right for
//! timestamps that are shown or stored, such as when a device was paired. A
//! *monotonic clock* only ever moves forward at the pace of real time. It is right
//! for deadlines, because a deadline that follows the wall clock can be extended or
//! cut short by whoever sets it.

use std::{
    fmt,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A source of the current time.
pub trait Clock: fmt::Debug + Send + Sync + 'static {
    /// The wall clock, in seconds since the Unix epoch. It can jump.
    fn now_unix(&self) -> u64;

    /// A clock that never goes backwards, measured from some fixed moment that is the
    /// same on every call. Only the difference between two readings means anything.
    ///
    /// Whether it keeps counting while the machine sleeps depends on the operating
    /// system.
    fn monotonic(&self) -> Duration;
}

impl<C: Clock + ?Sized> Clock for std::sync::Arc<C> {
    fn now_unix(&self) -> u64 {
        (**self).now_unix()
    }

    fn monotonic(&self) -> Duration {
        (**self).monotonic()
    }
}

/// The operating system's clocks.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }

    fn monotonic(&self) -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
}

/// A clock that only moves when told to. For tests.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct ManualClock {
    wall: std::sync::atomic::AtomicU64,
    monotonic: std::sync::atomic::AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl ManualClock {
    /// A clock showing `start` seconds since the Unix epoch.
    pub const fn new(start: u64) -> Self {
        Self {
            wall: std::sync::atomic::AtomicU64::new(start),
            monotonic: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Lets time pass: both clocks move forward.
    pub fn advance(&self, seconds: u64) {
        use std::sync::atomic::Ordering::SeqCst;
        self.wall.fetch_add(seconds, SeqCst);
        self.monotonic.fetch_add(seconds, SeqCst);
    }

    /// Sets the wall clock to `unix` seconds without any time passing, the way a person
    /// or a network time service can. The monotonic clock does not notice.
    pub fn set_wall_clock(&self, unix: u64) {
        self.wall.store(unix, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Clock for ManualClock {
    fn now_unix(&self) -> u64 {
        self.wall.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn monotonic(&self) -> Duration {
        Duration::from_secs(self.monotonic.load(std::sync::atomic::Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_after_2025() {
        assert!(SystemClock.now_unix() > 1_735_689_600);
    }

    #[test]
    fn the_system_monotonic_clock_does_not_go_backwards() {
        let first = SystemClock.monotonic();
        let second = SystemClock.monotonic();
        assert!(second >= first);
    }

    #[test]
    fn the_manual_clock_moves_only_when_told() {
        let clock = ManualClock::new(100);
        assert_eq!(clock.now_unix(), 100);
        assert_eq!(clock.monotonic(), Duration::ZERO);
        clock.advance(5);
        assert_eq!(clock.now_unix(), 105);
        assert_eq!(clock.monotonic(), Duration::from_secs(5));
    }

    #[test]
    fn setting_the_wall_clock_leaves_the_monotonic_one_alone() {
        let clock = ManualClock::new(100);
        clock.advance(2);
        clock.set_wall_clock(5_000);
        assert_eq!(clock.now_unix(), 5_000);
        assert_eq!(clock.monotonic(), Duration::from_secs(2));
        clock.set_wall_clock(1);
        assert_eq!(clock.now_unix(), 1);
        assert_eq!(clock.monotonic(), Duration::from_secs(2));
    }
}
