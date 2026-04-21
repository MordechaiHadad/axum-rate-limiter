use std::{collections::HashMap, sync::Mutex};

use jiff::{Timestamp, ToSpan};
use tracing::{debug, trace};

use crate::RateLimiter;

const RATE_LIMIT_MAX: i32 = 5;

#[derive(Debug)]
struct LimitData {
    tries: i32,
    expiry: Timestamp,
}

pub struct InMemoryLimiter {
    data: Mutex<HashMap<String, LimitData>>,
}

impl InMemoryLimiter {
    fn new() -> InMemoryLimiter {
        trace!("Initializing in-memory rate limiter");
        debug!("InMemoryLimiter created with empty data store");
        InMemoryLimiter {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl RateLimiter for InMemoryLimiter {
    async fn allow(&self, identifier: &str) -> bool {
        trace!("allow called for identifier: {}", identifier);
        let mut lock_guard = self.data.lock().unwrap();
        let now = jiff::Timestamp::now();

        let data = lock_guard
            .entry(identifier.to_string())
            .or_insert(LimitData {
                tries: 0,
                expiry: now + 60.seconds(),
            });

        trace!(
            "state for {}: tries={}, expiry={:?}",
            identifier, data.tries, data.expiry
        );

        if now > data.expiry {
            data.expiry = now + 60.seconds();
            data.tries = 0;
            debug!("resetting limit for {}", identifier);
            return true;
        }

        if data.tries >= RATE_LIMIT_MAX {
            debug!("rate limit exceeded for {}", identifier);
            return false;
        }

        data.expiry = now + 60.seconds();
        data.tries += 1;
        debug!("allowing request for {} (tries={})", identifier, data.tries);
        true
    }

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

    async fn len(&self) -> usize {
        let lock = self.data.lock().unwrap();
        let l = lock.len();
        trace!("len called, returning {}", l);
        l
    }
}
