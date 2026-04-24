# Changelog

All notable changes to curvekit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-04-23

### Added

- **Par → zero bootstrap** (`YieldType` enum; `YieldCurve::bootstrap_zero()`).
  Treasury publishes BEY par yields; `bootstrap_zero()` produces continuously
  compounded spot rates via the standard semi-annual coupon iterative algorithm.
  New client methods: `treasury_par_curve()`, `treasury_zero_curve()`.
- **Day-count conventions** (`daycount` module): `DayCount { Act360,
  Act365Fixed, Thirty360, ActAct }` with `year_fraction(start, end)`.
  New client method: `treasury_rate_with_convention(date, tenor, convention)`.
- **Retry + exponential backoff**: up to 3 attempts, delays 250 ms → 750 ms →
  2 000 ms (capped). Retries on 5xx / 429 / connect / timeout. Honours
  `Retry-After` response header when present.
- **Single-flight per-key cache**: concurrent callers requesting the same date
  share one HTTP fetch via `Arc<OnceCell>` deduplication; no duplicate network
  round-trips.
- **CDN mirror fallback**: after primary URL retries are exhausted, the fetcher
  tries jsDelivr (`cdn.jsdelivr.net/gh/userFRM/curvekit@main/data`). Override
  with `CURVEKIT_MIRROR_URL`.
- **SHA-256 manifest verification**: `data/manifest.json` maps each parquet
  filename to its expected `sha256:<hex>` digest. Downloaded bytes are verified
  before being written to the local cache. New `Error::ChecksumMismatch`
  variant. New CLI sub-command `curvekit-cli manifest` regenerates the
  manifest from local `data/`.
- **EFFR data source**: Effective Federal Funds Rate fetcher backed by the NY
  Fed Markets API. Client methods: `effr(date)`, `effr_range(start, end)`,
  `effr_latest()`, `effr_earliest_date()`. Parquet schema identical to SOFR.
  CLI: `get effr --date`, `backfill effr`.
- **OBFR data source**: Overnight Bank Funding Rate fetcher. Client methods:
  `obfr(date)`, `obfr_range(start, end)`, `obfr_latest()`,
  `obfr_earliest_date()`. Coverage starts 2016-03-01.
  CLI: `get obfr --date`, `backfill obfr`.
- Release workflow (`.github/workflows/release.yml`): fires on `v*` tags,
  runs the full CI gate, then publishes `curvekit` then `curvekit-cli` to
  crates.io via `CARGO_REGISTRY_TOKEN`.

### Changed

- `treasury_curve()` is **deprecated** since 1.0.0. Use `treasury_par_curve()`
  (identical semantics) or `treasury_zero_curve()` (bootstrapped spot rates).
- `CachedFetcher` construction is now via `CachedFetcher::new()` (opaque
  constructor); direct struct-literal construction is no longer available.

### Deprecated

- `Curvekit::treasury_curve()` — replaced by `treasury_par_curve()` and
  `treasury_zero_curve()`.

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

[1.0.0]: https://github.com/userFRM/curvekit/releases/tag/v1.0.0
[0.2.0]: https://github.com/userFRM/curvekit/releases/tag/v0.2.0
[0.1.0]: https://github.com/userFRM/curvekit/releases/tag/v0.1.0
