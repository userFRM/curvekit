# Architecture

## Design: offline-first bundled parquet

curvekit ships with historical rate data baked into the repository as compressed
parquet files under `data/`. Consumers link against the `curvekit` crate and call
synchronous functions — no daemon, no HTTP, no runtime required.

```
data/
├── treasury-{year}.parquet   (Date32, tenor_days UInt32, yield_bps UInt32)
├── ...
├── sofr-{year}.parquet       (Date32, rate_bps UInt32)
└── ...

          ┌──────────────────────────────────────┐
          │  curvekit (lib)                      │
          │  sources::bundled   ← read at call   │
          │  sources::parquet_io← write (CLI)    │
          │  sources::treasury  ← HTTP fetch     │
          │  sources::sofr      ← HTTP fetch     │
          │  curve / interpolation / error       │
          └──────────────────────────────────────┘
                   ▲ git dep / path dep
           consumer crates (e.g. kairos-engine)
```

## Crates

### curvekit (lib)

The single library crate. Depends on:
- `arrow` + `parquet` for parquet I/O
- `reqwest` for HTTP fetching (CLI / backfill only path)
- `async-trait` for `TreasuryFetcher` / `SofrFetcher` traits
- `chrono` for dates
- `thiserror` for typed errors

### curvekit-cli (binary)

The CLI binary. All network I/O lives here — the library API (`bundled.rs`) is
fully synchronous and disk-only.

## Data format

### Treasury

Long format: one row per (date, tenor). 12 tenors × ~250 trading days/year =
~3 000 rows/year.

| column      | type    | description                                  |
|-------------|---------|----------------------------------------------|
| date        | Date32  | days since Unix epoch (1970-01-01)           |
| tenor_days  | UInt32  | days to maturity (30/60/91/182/365/730/…)    |
| yield_bps   | UInt32  | continuously-compounded rate × 10 000        |

### SOFR

| column   | type    | description                              |
|----------|---------|------------------------------------------|
| date     | Date32  | days since Unix epoch                    |
| rate_bps | UInt32  | continuously-compounded rate × 10 000   |

Rates are stored as basis-point integers to avoid floating-point drift.

## Data directory resolution

At runtime the reader looks for `data/` in order:

1. `$CURVEKIT_DATA_DIR` env var (absolute path override).
2. `CARGO_MANIFEST_DIR/../../data/` — the compile-time path relative to
   `crates/curvekit/`. Works for `cargo test`, git deps, and `cargo install --path`.

## Refresh schedule

Data is updated by two GitHub Actions workflows:

| Workflow | Trigger | Action |
|---|---|---|
| `nightly.yml` | `0 3 * * 1-5` (03:00 UTC weekdays) | `append-today` — yesterday's Treasury + latest SOFR |
| `backfill.yml` | `workflow_dispatch` | `backfill --years 25` — full historical fetch |

## Interpolation

Two methods in `curvekit::interpolation`:

- `linear` — piecewise linear between bracketing points; flat extrapolation at
  boundaries. O(log n) via `BTreeMap::range`.
- `cubic_spline` — Fritsch-Carlson monotone cubic (no oscillation), falls back
  to `linear` for < 3 points.

`YieldCurve::get(days)` uses `linear` by default.
