use redis::aio::MultiplexedConnection;
use serde::Deserialize;
use tiktoken_rs::{bpe_for_tokenizer, tokenizer::Tokenizer};
use tracing::{debug, trace};

use crate::{RATE_LIMIT_MAX, RateLimiter};

const DEFAULT_WINDOW_SECONDS: i32 = 60;
const AI_WINDOW_SECONDS: i32 = 600;
const QUOTA_UNITS_PER_DOLLAR: i64 = 500;
const TOTAL_AI_QUOTA_UNITS: i64 = QUOTA_UNITS_PER_DOLLAR * 10;
const GPT_4_COST_UNITS_PER_TOKEN: i64 = 1;
const GPT_5_COST_UNITS_PER_TOKEN: i64 = 25;

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

        if tries > i64::from(RATE_LIMIT_MAX) {
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
    query: String,
}

#[derive(Debug)]
pub struct AiUsage {
    pub model: String,
    pub query_tokens: i64,
    pub charge_units: i64,
}

#[derive(Debug)]
pub enum AiRequestError {
    InvalidJson(serde_json::Error),
    UnsupportedModel(String),
    ChargeOverflow,
    Tokenizer(String),
}

#[derive(Clone, Copy)]
enum ModelFamily {
    Gpt4,
    Gpt5,
}

impl ModelFamily {
    fn parse(model: &str) -> Option<Self> {
        if model == "gpt-4" || model.starts_with("gpt-4.") {
            return Some(Self::Gpt4);
        }

        if model == "gpt-5" || model.starts_with("gpt-5.") {
            return Some(Self::Gpt5);
        }

        None
    }

    fn tokenizer(self) -> Tokenizer {
        match self {
            Self::Gpt4 => Tokenizer::Cl100kBase,
            Self::Gpt5 => Tokenizer::O200kBase,
        }
    }

    fn cost_units_per_token(self) -> i64 {
        match self {
            Self::Gpt4 => GPT_4_COST_UNITS_PER_TOKEN,
            Self::Gpt5 => GPT_5_COST_UNITS_PER_TOKEN,
        }
    }
}

impl RedisAiLimiter {
    pub fn new(connection: MultiplexedConnection) -> Self {
        trace!("Initializing Redis AI rate limiter");
        RedisAiLimiter { connection }
    }

    pub async fn allow(&self, identifier: &str, usage: &AiUsage) -> bool {
        if usage.charge_units == 0 {
            debug!(
                "allowing {} on {} with zero-token query",
                identifier, usage.model
            );
            return true;
        }

        trace!(
            "allow called for identifier: {identifier}, model: {}, charge_units: {}",
            usage.model, usage.charge_units
        );
        let id = format!("rate_limit:ai:{identifier}");

        let total =
            increment_with_expiry_by(&self.connection, &id, AI_WINDOW_SECONDS, usage.charge_units)
                .await;

        if total > TOTAL_AI_QUOTA_UNITS {
            debug!(
                "rate limit exceeded for {} on {} (tokens={}, charge_units={}, total_units={}, quota_units={})",
                identifier,
                usage.model,
                usage.query_tokens,
                usage.charge_units,
                total,
                TOTAL_AI_QUOTA_UNITS
            );
            return false;
        }

        debug!(
            "allowing request for {} on {} (tokens={}, charge_units={}, total_units={}, quota_units={})",
            identifier,
            usage.model,
            usage.query_tokens,
            usage.charge_units,
            total,
            TOTAL_AI_QUOTA_UNITS
        );
        true
    }

    pub async fn cleanup(&self) {}

    pub async fn len(&self) -> usize {
        0
    }
}

pub fn extract_ai_usage(body: &[u8]) -> Result<AiUsage, AiRequestError> {
    let payload =
        serde_json::from_slice::<AiRequestPayload>(body).map_err(AiRequestError::InvalidJson)?;
    let family = ModelFamily::parse(&payload.model)
        .ok_or_else(|| AiRequestError::UnsupportedModel(payload.model.clone()))?;
    let query_tokens = count_query_tokens(&payload.query, family)?;
    let query_tokens = i64::try_from(query_tokens).map_err(|_| AiRequestError::ChargeOverflow)?;
    let charge_units = query_tokens
        .checked_mul(family.cost_units_per_token())
        .ok_or(AiRequestError::ChargeOverflow)?;

    Ok(AiUsage {
        model: payload.model,
        query_tokens,
        charge_units,
    })
}

async fn increment_with_expiry(
    connection: &MultiplexedConnection,
    key: &str,
    expiry_seconds: i32,
) -> i64 {
    increment_with_expiry_by(connection, key, expiry_seconds, 1).await
}

async fn increment_with_expiry_by(
    connection: &MultiplexedConnection,
    key: &str,
    expiry_seconds: i32,
    increment: i64,
) -> i64 {
    let script = redis::Script::new(
        r"
            local current = redis.call('INCRBY', KEYS[1], ARGV[2])
            if current == tonumber(ARGV[2]) then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return current
        ",
    );

    script
        .key(key)
        .arg(expiry_seconds)
        .arg(increment)
        .invoke_async(&mut connection.clone())
        .await
        .unwrap()
}

fn count_query_tokens(query: &str, model_family: ModelFamily) -> Result<usize, AiRequestError> {
    let bpe = bpe_for_tokenizer(model_family.tokenizer())
        .map_err(|error| AiRequestError::Tokenizer(error.to_string()))?;
    Ok(bpe.count_ordinary(query))
}

#[cfg(test)]
mod tests {
    use super::{AiRequestError, ModelFamily, extract_ai_usage};

    #[test]
    fn extracts_usage_from_json_payload() {
        let usage = extract_ai_usage(br#"{"model":"gpt-5","query":"hello world"}"#).unwrap();
        assert_eq!(usage.model, "gpt-5");
        assert!(usage.query_tokens > 0);
        assert!(usage.charge_units >= usage.query_tokens);
    }

    #[test]
    fn accepts_only_supported_model_families() {
        assert!(matches!(
            ModelFamily::parse("gpt-4"),
            Some(ModelFamily::Gpt4)
        ));
        assert!(matches!(
            ModelFamily::parse("gpt-4.1"),
            Some(ModelFamily::Gpt4)
        ));
        assert!(matches!(
            ModelFamily::parse("gpt-5"),
            Some(ModelFamily::Gpt5)
        ));
        assert!(matches!(
            ModelFamily::parse("gpt-5.4"),
            Some(ModelFamily::Gpt5)
        ));
        assert!(ModelFamily::parse("gpt-4o").is_none());
        assert!(ModelFamily::parse("gpt-5-mini").is_none());
        assert!(ModelFamily::parse("o3").is_none());
    }

    #[test]
    fn rejects_unsupported_models() {
        let error = extract_ai_usage(br#"{"model":"gpt-4o","query":"hello world"}"#).unwrap_err();
        assert!(matches!(error, AiRequestError::UnsupportedModel(model) if model == "gpt-4o"));
    }
}
