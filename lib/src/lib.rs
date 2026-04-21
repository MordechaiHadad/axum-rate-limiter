pub mod memory;
#[cfg(feature = "surrealdb")]
pub mod surrealdb;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use axum_client_ip::ClientIp;
use tracing::{debug, trace};

/// Trait representing a rate limiter backend.
///
/// Implementors decide whether an identifier (usually an IP) is allowed to make
/// a request, can optionally perform periodic cleanup, and report the number
/// of tracked entries.
pub trait RateLimiter: Send + Sync {
    /// Check whether the given identifier is allowed to proceed.
    fn allow(&self, identifier: &str) -> impl std::future::Future<Output = bool> + Send;
    /// Optional cleanup task for the limiter implementation.
    fn cleanup(&self) -> impl std::future::Future<Output = ()> + Send {
        std::future::ready(())
    }
    /// Return the number of tracked entries currently held by the limiter.
    fn len(&self) -> impl std::future::Future<Output = usize> + Send;
    /// Return true when there are no tracked entries.
    fn is_empty(&self) -> impl std::future::Future<Output = bool> + Send {
        async { self.len().await == 0 }
    }
}

/// Axum middleware that applies rate limiting using the provided `RateLimiter`.
///
/// The middleware extracts a client identifier (IP) from the request, consults
/// the limiter, and either forwards the request or returns 429 Too Many Requests.
pub async fn rate_limit_middleware<R>(
    State(limiter): State<Arc<R>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode>
where
    R: RateLimiter,
{
    let client_ip = extract_ip_with_fallback(&req);
    debug!("client_ip = {}", client_ip);

    if limiter.allow(&client_ip).await {
        trace!("allowed {}", client_ip);
        return Ok(next.run(req).await);
    }
    trace!("rate limited {}", client_ip);
    Err(StatusCode::TOO_MANY_REQUESTS)
}

/// Extract a client IP from the request using headers first, then connect info.
///
/// Returns the string "unknown" if no IP can be found.
fn extract_ip_with_fallback(req: &Request) -> String {
    if let Some(ip) = try_extract_from_headers(req) {
        trace!("extracted from headers {}", ip);
        return ip;
    }

    if let Some(ip) = try_extract_from_connect_info(req) {
        trace!("extracted from connect info {}", ip);
        return ip;
    }

    "unknown".to_string()
}

/// Try to extract a client IP from request extensions populated by `axum_client_ip`.
fn try_extract_from_headers(req: &Request) -> Option<String> {
    req.extensions()
        .get::<ClientIp>()
        .map(|ip| ip.0.to_string())
}

/// Try to extract a client IP from `ConnectInfo<SocketAddr>` in request extensions.
fn try_extract_from_connect_info(req: &Request) -> Option<String> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.0.ip().to_string())
}
