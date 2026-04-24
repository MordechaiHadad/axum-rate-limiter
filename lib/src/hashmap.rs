use std::{collections::HashMap, sync::Mutex};

use jiff::{Timestamp, ToSpan};
use tracing::{debug, trace};

use crate::{RATE_LIMIT_MAX, RateLimiter};

#[derive(Debug)]
struct LimitData {
    tries: i32,
    expiry: Timestamp,
}

#[derive(Debug, Default)]
pub struct InMemoryLimiter {
    data: Mutex<HashMap<String, LimitData>>,
}

impl InMemoryLimiter {
    /// Create a new `InMemoryLimiter`.
    ///
    /// This initializes the internal data store used to track per-identifier
    /// request counts and expiration timestamps.
    pub fn new() -> InMemoryLimiter {
        trace!("Initializing in-memory rate limiter");
        debug!("InMemoryLimiter created with empty data store");
        InMemoryLimiter {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl RateLimiter for InMemoryLimiter {
    /// Determine whether the given `identifier` is allowed to make a request.
    ///
    /// If the identifier has remaining allowance, this increments the stored
    /// attempt count and returns `true`. If the rate limit has been reached,
    /// returns `false`. Expired counters are reset automatically.
    async fn allow(&self, identifier: &str) -> bool {
        trace!("allow called for identifier: {}", identifier);
        let mut lock_guard = self.data.lock().unwrap();
        let now = jiff::Timestamp::now();

        let data = lock_guard
            .entry(identifier.to_string())
            .and_modify(|data| data.tries += 1)
            .or_insert(LimitData {
                tries: 1,
                expiry: now + 60.seconds(),
            });

        trace!(
            "state for {}: tries={}, expiry={:?}",
            identifier, data.tries, data.expiry
        );

        if now > data.expiry {
            data.tries = 1;
            data.expiry = now + 60.seconds();
            debug!("resetting limit for {}", identifier);
            return true;
        }

        if data.tries > RATE_LIMIT_MAX {
            debug!("rate limit exceeded for {}", identifier);
            return false;
        }

        debug!("allowing request for {identifier} (tries={})", data.tries);
        true
    }

    /// Remove expired entries from the in-memory store.
    ///
    /// Attempts to acquire a non-blocking lock and deletes entries whose expiry
    /// timestamps are before the current time. If the lock cannot be acquired,
    /// the cleanup is skipped.
    async fn cleanup(&self) {
        trace!("cleanup called");
        let now = jiff::Timestamp::now();
        if let Ok(mut data) = self.data.try_lock() {
            let before = data.len();
            data.retain(|_, v| now < v.expiry);
            let after = data.len();
            debug!("cleanup removed {} entries", before.saturating_sub(after));
        } else {
            trace!("cleanup could not acquire lock");
        }
    }

    /// Return the number of tracked identifiers currently stored.
    ///
    /// This acquires a lock to read the current number of entries in the
    /// internal data store.
    async fn len(&self) -> usize {
        let lock = self.data.lock().unwrap();
        let l = lock.len();
        trace!("len called, returning {}", l);
        l
    }
}
