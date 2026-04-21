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

pub trait RateLimiter: Send + Sync {
    fn allow(&self, identifier: &str) -> impl std::future::Future<Output = bool> + Send;
    fn cleanup(&self) -> impl std::future::Future<Output = ()> + Send {
        std::future::ready(())
    }
    fn len(&self) -> impl std::future::Future<Output = usize> + Send;
    fn is_empty(&self) -> impl std::future::Future<Output = bool> + Send {
        async { self.len().await == 0 }
    }
}

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

fn try_extract_from_headers(req: &Request) -> Option<String> {
    req.extensions()
        .get::<ClientIp>()
        .map(|ip| ip.0.to_string())
}

fn try_extract_from_connect_info(req: &Request) -> Option<String> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.0.ip().to_string())
}
