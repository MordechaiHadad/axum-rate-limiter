use redis::aio::MultiplexedConnection;
use tracing::{debug, trace};

use crate::{RATE_LIMIT_MAX, RateLimiter};

/// Rate limiter backed by Redis using a per-identifier counter with expiry.
///
/// This limiter uses Redis INCR to track the number of attempts for a given
/// identifier (typically an IP). When the counter is first created it is given
/// an expiry so counts automatically reset after the configured window.
pub struct RedisLimiter {
    connection: MultiplexedConnection,
}

impl RedisLimiter {
    /// Create a new `RedisLimiter` from an existing multiplexed Redis connection.
    ///
    /// The provided connection is cloned for command execution where necessary.
    pub fn new(connection: MultiplexedConnection) -> Self {
        trace!("Initializing Redis rate limiter");
        RedisLimiter { connection }
    }
}

impl RateLimiter for RedisLimiter {
    /// Check whether the provided `identifier` is allowed to proceed.
    ///
    /// The implementation executes a small Redis script that increments a key
    /// and sets an expiry when the key is first created. If the incremented
    /// value exceeds the configured `RATE_LIMIT_MAX`, this returns `false`.
    async fn allow(&self, identifier: &str) -> bool {
        trace!("allow called for identifier: {identifier}");
        let id = format!("rate_limit:{identifier}");

        let script = redis::Script::new(
            r"
                local current = redis.call('INCR', KEYS[1])
                if current == 1 then
                    redis.call('EXPIRE', KEYS[1], ARGV[1])
                end
                return current
            ",
        );

        let tries: i32 = script
            .key(id)
            .arg(60)
            .invoke_async(&mut self.connection.clone())
            .await
            .unwrap();

        if tries > RATE_LIMIT_MAX {
            debug!("rate limit exceeded for {identifier} (tries={tries})");
            return false;
        }

        debug!("allowing request for {identifier} (tries={tries})");
        true
    }
}
