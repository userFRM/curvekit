# Changelog

All notable changes to curvekit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-04-23

### Changed

- Replaced server/RPC/SDK architecture with offline-first bundled-parquet design.
  The library now reads directly from `data/*.parquet` — no network calls, no
  daemon process, no HTTP dependency for consumers.
- Renamed `curvekit-core` → `curvekit` (single library crate).
- Renamed CLI binary from `curvekit` → `curvekit-cli`.

### Added

- `data/` directory with ZSTD-compressed parquet files (one per source per year).
  Treasury: `date` (Date32), `tenor_days` (UInt32), `yield_bps` (UInt32).
  SOFR: `date` (Date32), `rate_bps` (UInt32). Storage scale: 10 000 bps per unit.
- `sources::bundled` — reader API: `treasury_curve`, `sofr`, `rate_for_days`,
  `treasury_latest_date`, `sofr_latest_date`. All sync, no runtime required.
- `sources::parquet_io` — writer API: `write_treasury_year`, `write_sofr_year`,
  `append_treasury_day`, `append_sofr_day`. Used by CLI only.
- `YieldCurve::to_continuous_map()` — returns `HashMap<u32, f64>` for direct
  use in consumer crates that maintain a continuous yield curve map.
- `YieldCurveDay` type alias + `SofrDay` struct for the bulk writer API.
- CLI commands: `backfill --years N`, `backfill --source S --year Y`,
  `append-today`, `get treasury --date`, `get sofr --date`.
- `.github/workflows/backfill.yml` — `workflow_dispatch` bulk fetch.
- `.github/workflows/nightly.yml` — `0 3 * * 1-5` (03:00 UTC weekdays) appends
  yesterday's Treasury curve + latest SOFR.

### Removed

- `curvekit-rpc` crate (JSON-RPC 2.0 + REST handlers).
- `curvekit-server` crate (axum HTTP server).
- `curvekit-sdk` crate (HTTP client).
- `curvekit-core::cache` (SQLite persistence layer via rusqlite).
- OpenAPI spec.
- `axum`, `tower`, `tower-http`, `rusqlite` dependencies.

## [0.1.0] — 2026-04-23

### Added

- `curvekit-core`: Treasury yield curve fetcher (home.treasury.gov CSV), SOFR
  fetcher (NY Fed markets API), `YieldCurve` / `SofrRate` / `TermStructure`
  types, linear + monotone cubic-spline interpolation, SQLite cache via rusqlite.
- `curvekit-rpc`: JSON-RPC 2.0 handler with REST mirrors.
- `curvekit-server`: Binary serving JSON-RPC + REST API on port 8080.
- `curvekit-sdk` (Rust): `CurvekitClient`.
- `curvekit` (CLI): `get treasury`, `get sofr`, `refresh`, `health`.
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`.

[0.2.0]: https://github.com/userFRM/curvekit/releases/tag/v0.2.0
[0.1.0]: https://github.com/userFRM/curvekit/releases/tag/v0.1.0
