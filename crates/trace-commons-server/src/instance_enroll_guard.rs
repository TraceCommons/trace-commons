//! Process-local enrollment guards: nonce replay cache + per-instance token
//! bucket. Restart-resets (matching the allowlist DenialCounter posture); the
//! attestation `exp` and derived-tenant idempotency are the durable backstop.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ReplayCache {
    seen: Mutex<HashMap<String, Instant>>, // key -> expiry
}

impl ReplayCache {
    pub fn new() -> Self {
        Self { seen: Mutex::new(HashMap::new()) }
    }

    /// Returns true if `key` is fresh (and records it until `now + ttl`);
    /// false if it is a replay within its TTL. Evicts expired keys on each call.
    pub fn consume(&self, key: &str, ttl: Duration, now: Instant) -> bool {
        let mut seen = self.seen.lock().expect("ReplayCache poisoned");
        seen.retain(|_, &mut expiry| expiry > now);
        if seen.contains_key(key) {
            return false;
        }
        seen.insert(key.to_string(), now + ttl);
        true
    }

    /// Returns true if `key` was already recorded and its TTL has not expired
    /// (i.e. this would be a replay). Does NOT record or modify state. Evicts
    /// expired keys as a side effect.
    pub fn is_seen(&self, key: &str, now: Instant) -> bool {
        let mut seen = self.seen.lock().expect("ReplayCache poisoned");
        seen.retain(|_, &mut expiry| expiry > now);
        seen.contains_key(key)
    }

    /// Records `key` with expiry `now + ttl`. If `key` is already present its
    /// expiry is left unchanged (idempotent). Call this only after the
    /// operation that the nonce guards has succeeded.
    pub fn record(&self, key: &str, ttl: Duration, now: Instant) {
        let mut seen = self.seen.lock().expect("ReplayCache poisoned");
        seen.retain(|_, &mut expiry| expiry > now);
        seen.entry(key.to_string()).or_insert(now + ttl);
    }
}

impl Default for ReplayCache {
    fn default() -> Self { Self::new() }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct InstanceRateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl InstanceRateLimiter {
    pub fn new() -> Self {
        Self { buckets: Mutex::new(HashMap::new()) }
    }

    /// Token bucket: capacity = `rate_per_min`, refill = `rate_per_min`/60 per
    /// second. Returns true if a token was available and consumed.
    pub fn try_acquire(&self, subject: &str, rate_per_min: u32, now: Instant) -> bool {
        if rate_per_min == 0 {
            return false;
        }
        let cap = rate_per_min as f64;
        let refill_per_sec = cap / 60.0;
        let mut buckets = self.buckets.lock().expect("InstanceRateLimiter poisoned");
        let bucket = buckets.entry(subject.to_string()).or_insert(Bucket { tokens: cap, last: now });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(cap);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for InstanceRateLimiter {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn replay_cache_blocks_second_use_until_ttl() {
        let cache = ReplayCache::new();
        let t0 = Instant::now();
        assert!(cache.consume("inst|nonce", Duration::from_secs(300), t0));
        assert!(!cache.consume("inst|nonce", Duration::from_secs(300), t0));
        // After TTL the key is evicted and a (hypothetical) reuse is fresh again.
        let later = t0 + Duration::from_secs(301);
        assert!(cache.consume("inst|nonce", Duration::from_secs(300), later));
    }

    #[test]
    fn rate_limiter_refuses_past_budget() {
        let rl = InstanceRateLimiter::new();
        let t0 = Instant::now();
        // rate_per_min = 2 -> 2 tokens available at start.
        assert!(rl.try_acquire("inst", 2, t0));
        assert!(rl.try_acquire("inst", 2, t0));
        assert!(!rl.try_acquire("inst", 2, t0));
        // Half a minute later ~1 token refilled.
        assert!(rl.try_acquire("inst", 2, t0 + Duration::from_secs(31)));
    }
}
