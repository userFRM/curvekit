# Changelog

All notable changes to curvekit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-04-24

### Changed
- Switched from JSON-RPC server to client-library-only architecture.
- Client fetches parquet files on-demand from raw.githubusercontent.com
  with ETag revalidation and XDG-compliant local cache.

### Removed
- `curvekit-rpc` crate (JSON-RPC handlers)
- `curvekit-server` crate (HTTP binary)
- `curvekit-sdk` crate (folded into `curvekit`)
- SQLite cache layer

### Added
- Flat endpoint client: `Curvekit::treasury_curve` / `sofr` / etc.
- Runtime GitHub raw fetch
- ETag revalidation with stale-cache fallback
- `CURVEKIT_BASE_URL` / `CURVEKIT_CACHE_DIR` environment overrides

### Fixed
- Treasury 403 (swapped `/all/all` URL path for `/{year}/all`)
- Silent error-swallowing in CLI backfill (now exits 1 on any fetch failure)

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
