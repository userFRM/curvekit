# Architecture

## Data flow

```
Public internet                    curvekit-server process
───────────────     ───────────────────────────────────────────────────────
treasury.gov/CSV  → sources/treasury.rs → parse → YieldCurve
nyfed.org/CSV     → sources/sofr.rs    → parse → SofrRate
                                                     │
                                                     ▼
                                              SQLite cache
                                         (RateCache, WAL mode)
                                                     │
                                        ─────────────┴──────────────
                                        │                           │
                                   /rpc (JSON-RPC)            REST routes
                                   methods.rs                 rest.rs
                                        │                           │
                                   curvekit-rpc::build_router (axum)
                                                     │
                                              TCP :8080
                                                     │
                                        ─────────────┴──────────────
                                        │                           │
                                 curvekit-sdk                    curl
                                 CurvekitClient
```

## Crates

### curvekit-core

The foundation. No HTTP server logic. Depends on:
- `reqwest` for HTTP fetching
- `rusqlite` (bundled SQLite) for caching
- `chrono` / `chrono-tz` for dates and ET timezone
- `async-trait` for `TreasuryFetcher` / `SofrFetcher` traits

### curvekit-rpc

Depends on `curvekit-core` + `axum`. Provides:
- `AppState` — wraps `RateCache` + last-refresh timestamps
- `build_router()` — returns the full axum `Router`
- `methods.rs` — JSON-RPC 2.0 dispatch
- `rest.rs` — REST handler functions

### curvekit-server

Binary only. Depends on `curvekit-core` + `curvekit-rpc`. Handles:
- CLI argument parsing (clap)
- SQLite cache initialization
- Initial bootstrap (today's data on startup)
- Background refresh loop (60s tick, fires on schedule match)
- Graceful shutdown on CTRL+C

### curvekit-sdk

Thin HTTP client. Depends on `curvekit-core` (for types) + `reqwest`.
All calls go through `/rpc` (JSON-RPC 2.0). Types are re-exported so
consumers don't need `curvekit-core` in their `Cargo.toml`.

## Cache design

- SQLite with WAL mode and `SYNCHRONOUS = NORMAL` — survives crashes
  (WAL guarantees committed data), gives good write throughput.
- Schema is intentionally minimal: `treasury_curves(date, tenor_days, rate_cont)`
  and `sofr_rates(date, rate_cont)`.
- `INSERT OR REPLACE` semantics — re-fetching the same date is idempotent.
- On server restart, the cache persists but the server bootstraps from live
  sources anyway (in case the cache is stale over a holiday).

## Refresh schedule

| Source | Schedule | Implementation |
|---|---|---|
| SOFR | 08:00 ET weekdays | `refresh_loop` checks `hour==8 && minute==0` |
| Treasury | 15:30 ET weekdays | `refresh_loop` checks `hour==15 && minute==30` |

The loop runs every 60 seconds and does nothing on weekends or outside the
target windows. Each refresh spawns a new task so a slow fetch doesn't block
the tick.

## Interpolation

Two methods are provided in `curvekit_core::interpolation`:

- `linear` — piecewise linear between bracketing points; flat extrapolation
  at boundaries. O(log n) via `BTreeMap::range`.
- `cubic_spline` — Fritsch-Carlson monotone cubic (no oscillation), falls back
  to `linear` for < 3 points.

`YieldCurve::get(days)` uses `linear` by default. The RPC/REST layer exposes
raw `YieldCurve` points; consumers can apply either method locally.
