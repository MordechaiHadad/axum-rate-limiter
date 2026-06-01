use redis::aio::MultiplexedConnection;
use serde::Deserialize;
use tracing::{debug, trace};

use crate::{RATE_LIMIT_MAX, RateLimiter};

const DEFAULT_WINDOW_SECONDS: i32 = 60;
const GPT_5_LIMIT: i32 = 5;
const GPT_4_LIMIT: i32 = 10;
const AI_WINDOW_SECONDS: i32 = 600;

/// Rate limiter backed by Redis using a per-identifier counter with expiry.
///
/// This limiter uses Redis INCR to track the number of attempts for a given
/// identifier (typically an IP). When the counter is first created it is given
/// an expiry so counts automatically reset after the configured window.
pub struct RedisLimiter {
    connection: MultiplexedConnection,
}

pub struct RedisAiLimiter {
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
        let tries = increment_with_expiry(&self.connection, &id, DEFAULT_WINDOW_SECONDS).await;

        if tries > RATE_LIMIT_MAX {
            debug!("rate limit exceeded for {identifier} (tries={tries})");
            return false;
        }

        debug!("allowing request for {identifier} (tries={tries})");
        true
    }
}

#[derive(Deserialize)]
struct AiRequestPayload {
    model: String,
}

impl RedisAiLimiter {
    pub fn new(connection: MultiplexedConnection) -> Self {
        trace!("Initializing Redis AI rate limiter");
        RedisAiLimiter { connection }
    }

    pub async fn allow(&self, identifier: &str, model: &str) -> bool {
        let Some(limit) = model_limit(model) else {
            debug!("allowing {} with unrecognized model {}", identifier, model);
            return true;
        };

        trace!("allow called for identifier: {identifier}, model: {model}");
        let id = format!("rate_limit:ai:{model}:{identifier}");

        let tries = increment_with_expiry(&self.connection, &id, AI_WINDOW_SECONDS).await;

        if tries > limit {
            debug!(
                "rate limit exceeded for {} on {} (tries={}, limit={})",
                identifier, model, tries, limit
            );
            return false;
        }

        debug!(
            "allowing request for {} on {} (tries={}, limit={})",
            identifier, model, tries, limit
        );
        true
    }
}

pub fn extract_model(body: &[u8]) -> Result<String, serde_json::Error> {
    serde_json::from_slice::<AiRequestPayload>(body).map(|payload| payload.model)
}

async fn increment_with_expiry(
    connection: &MultiplexedConnection,
    key: &str,
    expiry_seconds: i32,
) -> i32 {
    let script = redis::Script::new(
        r"
            local current = redis.call('INCR', KEYS[1])
            if current == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return current
        ",
    );

    script
        .key(key)
        .arg(expiry_seconds)
        .invoke_async(&mut connection.clone())
        .await
        .unwrap()
}

fn model_limit(model: &str) -> Option<i32> {
    if model.starts_with("gpt-5") {
        return Some(GPT_5_LIMIT);
    }

    if model.starts_with("gpt-4") {
        return Some(GPT_4_LIMIT);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{extract_model, model_limit};

    #[test]
    fn extracts_model_from_json_payload() {
        let model = extract_model(br#"{"model":"gpt-5"}"#).unwrap();
        assert_eq!(model, "gpt-5");
    }

    #[test]
    fn maps_model_limits() {
        assert_eq!(model_limit("gpt-5"), Some(5));
        assert_eq!(model_limit("gpt-5-mini"), Some(5));
        assert_eq!(model_limit("gpt-4"), Some(10));
        assert_eq!(model_limit("gpt-4.1"), Some(10));
        assert_eq!(model_limit("o3"), None);
    }
}
