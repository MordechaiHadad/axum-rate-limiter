use surrealdb::Surreal;
use surrealdb::engine::local::Db;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::{debug, trace};

use crate::{RATE_LIMIT_MAX, RateLimiter};

pub struct SurrealLimiter {
    db: Surreal<Db>,
}

#[derive(Debug, SurrealValue, Clone)]
struct SurrealLimitEntry {
    id: RecordId,
    tries: i32,
    expiry: surrealdb::types::Datetime,
}

#[derive(Debug, SurrealValue, Clone)]
struct SurrealCount {
    count: i64,
}

impl SurrealLimiter {
    /// Create and initialize a `SurrealRateLimiter`.
    ///
    /// This executes any required migration/initialization queries to ensure the
    /// `rate_limits` table and its fields exist, then returns a limiter backed by
    /// the provided SurrealDB instance.
    pub async fn new(db: Surreal<Db>) -> Self {
        db.query(
            r"
            BEGIN TRANSACTION;
            DEFINE TABLE IF NOT EXISTS rate_limits SCHEMAFULL;
            DEFINE FIELD tries ON rate_limits TYPE int;
            DEFINE FIELD expiry ON rate_limits TYPE datetime;
            DEFINE INDEX limits_count ON rate_limits COUNT;
            COMMIT TRANSACTION;
            ",
        )
        .await
        .unwrap();
        trace!("Initialized SurrealDB rate limiter");
        SurrealLimiter { db }
    }
}

impl RateLimiter for SurrealLimiter {
    /// Determine whether the given identifier (usually an IP address) is allowed.
    ///
    /// Returns `true` when the identifier is within the configured rate limits,
    /// and `false` when the rate limit has been exceeded. This function will
    /// create a record for the identifier if one does not yet exist and will
    /// reset the counter when the expiry has passed.
    async fn allow(&self, identifier: &str) -> bool {
        trace!("allow check started for {}", identifier);
        let id = RecordId::new("rate_limits", identifier.to_string());
        let mut response = self
            .db
            .query(
                r"
                {
                LET $now = time::now();
                UPSERT $id
                SET
                    tries = IF expiry < $now { 1 } ELSE { tries + 1 },
                    expiry = IF expiry < $now { time::now() + 60s } ELSE { expiry }
                RETURN AFTER;
                };
                ",
            )
            .bind(("id", id))
            .await
            .unwrap();

        let result: Option<SurrealLimitEntry> = match response.take(0) {
            Ok(v) => v,
            Err(e) => {
                debug!("error taking query result for {}: {:?}", identifier, e);
                return false;
            }
        };

        let entry = result.unwrap();
        debug!("loaded entry for {}: {:?}", identifier, entry);

        if entry.tries > RATE_LIMIT_MAX {
            debug!(
                "rate limit exceeded for {} (tries={})",
                identifier, entry.tries
            );
            return false;
        }

        true
    }

    /// Remove expired entries from the rate limits table.
    ///
    /// This performs a delete query to remove records whose expiry time is in the past.
    async fn cleanup(&self) {
        trace!("running surrealdb cleanup");
        self.db
            .query("DELETE FROM rate_limits WHERE expiry < time::now()")
            .await
            .unwrap();
        debug!("surrealdb cleanup completed");
    }

    /// Return the number of stored rate limit entries.
    ///
    /// Queries the database for the total count of records in the `rate_limits`
    /// table and returns that value as a `usize`.
    async fn len(&self) -> usize {
        trace!("counting rate limits");
        let mut response = self
            .db
            .query("SELECT COUNT() AS count FROM rate_limits GROUP ALL")
            .await
            .unwrap();
        let count = response.take::<Option<SurrealCount>>(0).unwrap();
        debug!("count query result: {:?}", count);
        count.map(|c| c.count as usize).unwrap_or(0)
    }
}
