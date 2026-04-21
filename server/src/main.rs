use axum::{Router, middleware, routing::get};
use axum_rate_limiter::{RateLimiter, rate_limit_middleware};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use surrealdb::engine::local::Mem;

#[tokio::main]
async fn main() {
    let db = surrealdb::Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    let limiter = Arc::new(axum_rate_limiter::surrealdb::SurrealRateLimiter::new(db).await);
    let limiter_cleanup = limiter.clone();

    tokio::spawn(async move {
        let interval = Duration::from_secs(60);
        loop {
            tokio::time::sleep(interval).await;
            println!(
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
    println!("starting at {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap()
}
