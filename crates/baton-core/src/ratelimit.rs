// SPDX-License-Identifier: Apache-2.0

//! Token-bucket rate limiting keyed by anything hashable, with bounded memory.
//!
//! Two places use it: new connection attempts per source address, and handoff
//! items per paired device. Both see input from the network, so the table of
//! keys has a hard size limit and evicts instead of growing.

use std::{
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Allows a burst of events per key and refills evenly over a window.
#[derive(Debug)]
pub(crate) struct RateLimiter<K> {
    burst: f64,
    refill_per_sec: f64,
    max_keys: usize,
    buckets: HashMap<K, Bucket>,
}

impl<K: Hash + Eq + Clone> RateLimiter<K> {
    /// A limiter that allows `burst` events at once and `burst` more per `window`,
    /// remembering at most `max_keys` keys.
    pub(crate) fn new(burst: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            burst: f64::from(burst),
            refill_per_sec: f64::from(burst) / window.as_secs_f64().max(f64::EPSILON),
            max_keys: max_keys.max(1),
            buckets: HashMap::new(),
        }
    }

    /// Records an event for `key` at `now` and says whether it is within the limit.
    pub(crate) fn allow(&mut self, key: &K, now: Instant) -> bool {
        if !self.buckets.contains_key(key) && self.buckets.len() >= self.max_keys {
            self.make_room(now);
        }
        let bucket = self.buckets.entry(key.clone()).or_insert(Bucket {
            tokens: self.burst,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = elapsed
            .mul_add(self.refill_per_sec, bucket.tokens)
            .min(self.burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Forgets everything about `key`.
    pub(crate) fn forget(&mut self, key: &K) {
        self.buckets.remove(key);
    }

    fn make_room(&mut self, now: Instant) {
        // A bucket that has refilled completely says nothing a new one would not.
        let (burst, rate) = (self.burst, self.refill_per_sec);
        self.buckets.retain(|_, bucket| {
            let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
            elapsed.mul_add(rate, bucket.tokens) < burst
        });
        if self.buckets.len() >= self.max_keys {
            let oldest = self
                .buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.updated)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.buckets.remove(&oldest);
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_burst_is_allowed_and_then_refused() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(3, 10 * SECOND, 16);
        assert!(limiter.allow(&"a", start));
        assert!(limiter.allow(&"a", start));
        assert!(limiter.allow(&"a", start));
        assert!(!limiter.allow(&"a", start));
    }

    #[test]
    fn tokens_come_back_over_the_window() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(2, 10 * SECOND, 16);
        assert!(limiter.allow(&1, start));
        assert!(limiter.allow(&1, start));
        assert!(!limiter.allow(&1, start));
        // One token every five seconds.
        assert!(!limiter.allow(&1, start + 4 * SECOND));
        assert!(limiter.allow(&1, start + 6 * SECOND));
        assert!(!limiter.allow(&1, start + 6 * SECOND));
        // Waiting longer than the window never banks more than the burst.
        let later = start + 100 * SECOND;
        assert!(limiter.allow(&1, later));
        assert!(limiter.allow(&1, later));
        assert!(!limiter.allow(&1, later));
    }

    #[test]
    fn keys_are_independent() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(1, 10 * SECOND, 16);
        assert!(limiter.allow(&"a", start));
        assert!(!limiter.allow(&"a", start));
        assert!(limiter.allow(&"b", start));
    }

    #[test]
    fn forgetting_a_key_gives_it_a_fresh_bucket() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(1, 10 * SECOND, 16);
        assert!(limiter.allow(&"a", start));
        assert!(!limiter.allow(&"a", start));
        limiter.forget(&"a");
        assert!(limiter.allow(&"a", start));
    }

    #[test]
    fn memory_stays_bounded_whatever_the_keys() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(1, 10 * SECOND, 8);
        for key in 0..1_000u32 {
            limiter.allow(&key, start + Duration::from_millis(u64::from(key)));
            assert!(limiter.len() <= 8);
        }
    }

    #[test]
    fn eviction_prefers_buckets_that_have_fully_refilled() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(1, 10 * SECOND, 2);
        assert!(limiter.allow(&"idle", start));
        // "busy" is still empty after the window starts to refill "idle".
        assert!(limiter.allow(&"busy", start + 9 * SECOND));
        assert!(!limiter.allow(&"busy", start + 9 * SECOND));
        // "idle" has refilled completely, so it is the one dropped for "new".
        assert!(limiter.allow(&"new", start + 10 * SECOND));
        assert!(
            !limiter.allow(&"busy", start + 10 * SECOND),
            "the busy key must not have been forgotten"
        );
    }
}
