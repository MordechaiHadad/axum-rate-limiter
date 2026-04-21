use surrealdb::Surreal;
use surrealdb::engine::local::Db;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::{debug, trace};

use crate::RateLimiter;

const RATE_LIMIT_MAX: i32 = 3;

pub struct SurrealRateLimiter {
    db: Surreal<Db>,
}

#[derive(Debug, SurrealValue, Clone)]
struct SurrealLimitEntry {
    id: RecordId,
    tries: i32,
    expiry: surrealdb::types::Datetime,
    ip: String,
}

#[derive(Debug, SurrealValue, Clone)]
struct SurrealCount {
    count: i64,
}

impl SurrealRateLimiter {
    /// Create and initialize a `SurrealRateLimiter`.
    ///
    /// This executes any required migration/initialization queries to ensure the
    /// `rate_limits` table and its fields exist, then returns a limiter backed by
    /// the provided SurrealDB instance.
    pub async fn new(db: Surreal<Db>) -> Self {
        db.query(
            "
            BEGIN TRANSACTION;
            DEFINE TABLE IF NOT EXISTS rate_limits SCHEMAFULL;
            DEFINE FIELD ip ON rate_limits TYPE string;
            DEFINE FIELD tries ON rate_limits TYPE int;
            DEFINE FIELD expiry ON rate_limits TYPE datetime;
            DEFINE INDEX limits_count ON rate_limits COUNT;
            COMMIT TRANSACTION;
            ",
        )
        .await
        .unwrap();
        trace!("Initialized SurrealDB rate limiter");
        SurrealRateLimiter { db }
    }
}

impl RateLimiter for SurrealRateLimiter {
    /// Determine whether the given identifier (usually an IP address) is allowed.
    ///
    /// Returns `true` when the identifier is within the configured rate limits,
    /// and `false` when the rate limit has been exceeded. This function will
    /// create a record for the identifier if one does not yet exist and will
    /// reset the counter when the expiry has passed.
    async fn allow(&self, identifier: &str) -> bool {
        trace!("allow check started for {}", identifier);
        let now = jiff::Timestamp::now();
        let mut response = self
            .db
            .query(
                "
                {
                LET $result = (SELECT * FROM rate_limits WHERE ip = $ip)[0];
                LET $new_result = IF $result IS NONE {
                    (CREATE rate_limits CONTENT { ip: $ip, tries: 0, expiry: time::now() + 60s })[0]
                } ELSE {
                    $result
                };
                RETURN $new_result;
                };
                ",
            )
            .bind(("ip", identifier.to_owned()))
            .await
            .unwrap();

        let result: Option<SurrealLimitEntry> = response.take(0).unwrap();

        let entry = result.unwrap();
        debug!("loaded entry for {}: {:?}", identifier, entry);
        let entry_expiry = jiff::Timestamp::from_second(entry.expiry.timestamp()).unwrap();
        if now > entry_expiry {
            debug!("entry expired for {}, resetting tries", identifier);
            self.db
                .query("UPDATE $id SET tries = 0, expiry = time::now() + 60s")
                .bind(("id", entry.id))
                .await
                .unwrap();
            return true;
        }

        if entry.tries >= RATE_LIMIT_MAX {
            debug!(
                "rate limit exceeded for {} (tries={})",
                identifier, entry.tries
            );
            return false;
        }

        debug!(
            "incrementing tries for {} (tries={})",
            identifier, entry.tries
        );
        self.db
            .query("UPDATE $id SET tries += 1, expiry = time::now() + 60s")
            .bind(("id", entry.id))
            .await
            .unwrap();

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
