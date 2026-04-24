use axum::{Router, middleware, routing::get};
use axum_rate_limiter::{RateLimiter, rate_limit_middleware};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tracing::info;

cfg_select! {
    feature = "surrealdb" => {
        use surrealdb::engine::local::Mem;

        type LimiterImpl = axum_rate_limiter::surrealdb::SurrealLimiter;

        async fn build_limiter() -> Arc<LimiterImpl> {
            let db = surrealdb::Surreal::new::<Mem>(()).await.unwrap();
            db.use_ns("test").use_db("test").await.unwrap();
            Arc::new(axum_rate_limiter::surrealdb::SurrealLimiter::new(db).await)
        }
    }

    feature = "redis" => {
        type LimiterImpl = axum_rate_limiter::redis::RedisLimiter;

        async fn build_limiter() -> Arc<LimiterImpl> {
            let client = redis::Client::open("redis://127.0.0.1/").unwrap();
            let con = client.get_multiplexed_async_connection().await.unwrap();
            Arc::new(axum_rate_limiter::redis::RedisLimiter::new(con))
        }
    }

    _ => {
        type LimiterImpl = axum_rate_limiter::hashmap::InMemoryLimiter;

        async fn build_limiter() -> Arc<LimiterImpl> {
            Arc::new(axum_rate_limiter::hashmap::InMemoryLimiter::new())
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let limiter = build_limiter().await;
    let limiter_cleanup = limiter.clone();

    tokio::spawn(async move {
        let interval = Duration::from_secs(60);
        loop {
            tokio::time::sleep(interval).await;
            info!(
                "rate limiting storage size: {}",
                limiter_cleanup.len().await
            );
            limiter_cleanup.cleanup().await;
        }
    });

    let app = Router::new()
        .route("/", get(|| async { "Hello, world!" }))
        .layer(middleware::from_fn_with_state(
            limiter,
            rate_limit_middleware,
        ))
        .with_state(());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("starting at {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap()
}
