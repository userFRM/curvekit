# Changelog

All notable changes to curvekit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — 2026-04-23

### Added

- `curvekit-core`: Treasury yield curve fetcher (home.treasury.gov CSV), SOFR
  fetcher (NY Fed markets API), `YieldCurve` / `SofrRate` / `TermStructure`
  types, linear + monotone cubic-spline interpolation, SQLite cache via rusqlite.
- `curvekit-rpc`: JSON-RPC 2.0 handler (`rates.treasury_curve`,
  `rates.sofr`, `rates.treasury_range`, `rates.health`) plus REST mirrors
  (`GET /treasury/curve/:date`, `/sofr/:date`, `/health`).
- `curvekit-server`: Binary that serves the JSON-RPC + REST API on port 8080,
  with background refresh at 08:00 ET (SOFR) and 15:30 ET (Treasury).
- `curvekit-sdk` (Rust): `CurvekitClient` with `treasury_curve`, `sofr`,
  `rate_for_days`, `treasury_range`, `health` methods.
- `curvekit` (CLI): `get treasury`, `get sofr`, `refresh`, `health` sub-commands.
- OpenAPI 3.0 spec served at `GET /openapi.json`.
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`.

[0.1.0]: https://github.com/userFRM/curvekit/releases/tag/v0.1.0
