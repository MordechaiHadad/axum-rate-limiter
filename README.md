# axum-rate-limiter — Benchmark summary

Command used
```
oha -z 30s -c 100 http://localhost:3000/
```

Run the server
```bash
# default -> in-memory hashmap limiter
cargo run

# SurrealDB (embedded in-memory engine)
cargo run -F surrealdb

# Redis
cargo run -F redis
```

Key metrics (single 30s run)
| Backend (what it is)        | Req/s   | Avg ms | p50 ms | p95 ms | p99 ms | 200 | 429     | Errors |
|----------------------------:|--------:|-------:|-------:|-------:|-------:|----:|--------:|-------:|
| Hashmap (in-memory)         | 4042.66 | 24.73  | 21.08  | 43.45  | 139.61 | 3  | 121,191 | 98     |
| Redis (central store)       | 3892.17 | 25.69  | 21.63  | 34.50  | 177.82 | 3  | 116,683 | 99     |
| SurrealDB (local::Mem)      | 1361.00 | 73.52  | 67.96  | 125.22 | 308.93 | 3  | 40,734  | 98     |

Relative throughput (visual)
```
InMemory ██████████████████████████████████████████ 4042.7
Redis    ████████████████████████████████████████ 3892.2
Surreal  ████████████                           1361.0
```
