# axum-rate-limiter

Minimal rate-limiting middleware for Axum.

## Trait

The crate exposes a simple `RateLimiter` trait.

- `allow(&self, identifier: &str) -> Future<Output = bool>`  
  Returns `true` when the given identifier (typically an IP) is allowed to proceed, `false` when rate-limited.

- `cleanup(&self) -> Future<Output = ()>` (optional)  
  Optional periodic maintenance hook (no-op by default).

- `len(&self) -> Future<Output = usize>`  
  Return the current number of tracked entries (useful for monitoring).

- `is_empty(&self) -> Future<Output = bool>`  
  Convenience helper that resolves to `len().await == 0`.

Implementors can provide whatever storage/strategy they need (in-memory hashmap, Redis, SurrealDB, or custom algorithms such as token-bucket or sliding window).

## Quick usage

Run the demo server with one of the built-in backends:

- In-memory (default): `cargo run`  
- SurrealDB feature: `cargo run -F surrealdb`  
- Redis feature: `cargo run -F redis`

## Benchmarks

Command used: `oha -z 30s -c 100 http://localhost:3000/`

Key metrics (single 30s run)

| Backend (what it is)        | Req/s   | Avg ms | p50 ms | p95 ms | p99 ms | 200 | 429     | Errors |
|----------------------------:|--------:|-------:|-------:|-------:|-------:|----:|--------:|-------:|
| Hashmap (in-memory)         | 4042.66 | 24.73  | 21.08  | 43.45  | 139.61 | 3  | 121,191 | 98     |
| Redis (central store)       | 3892.17 | 25.69  | 21.63  | 34.50  | 177.82 | 3  | 116,683 | 99     |
| SurrealDB (local::Mem)      | 1361.00 | 73.52  | 67.96  | 125.22 | 308.93 | 3  | 40,734  | 98     |

Relative throughput (visual)

InMemory ██████████████████████████████████████████ 4042.7  
Redis    ████████████████████████████████████████ 3892.2  
Surreal  ████████████                           1361.0
