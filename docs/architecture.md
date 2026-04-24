# Architecture

## Data flow

```
┌──────────────┐      ┌────────────────────────────┐
│  Data source │      │   GitHub Actions (runner)  │
│ treasury.gov │──────│  backfill.yml (manual)     │
│   NY Fed     │      │  nightly.yml (cron weekday │
└──────────────┘      │                03:00 UTC)  │
                      └──────────┬─────────────────┘
                                 │ commit data/*.parquet
                                 ▼
                 ┌───────────────────────────┐
                 │  userFRM/curvekit repo    │
                 │  data/treasury-YYYY.parquet│
                 │  data/sofr-YYYY.parquet    │
                 └───────────┬───────────────┘
                             │ raw.githubusercontent.com/.../data/
                             ▼
┌──────────────────────────────────────────────────┐
│  curvekit::Curvekit (client lib)                 │
│   fetch → ETag check → parquet parse            │
│   cache: ~/.cache/curvekit/                     │
└──────────────────────────────────────────────────┘
                             ▲
                             │ flat async API
         ┌───────────────────┴──────────────────┐
         │  user app (e.g. Kairos)              │
         │  client.treasury_curve(date).await?  │
         └──────────────────────────────────────┘
```

## Crates

### curvekit (lib)

Single library crate. Contains:

- `client` — `Curvekit` struct with flat async endpoint methods.
- `fetcher` — `CachedFetcher`: ETag-aware HTTP fetch + disk cache.
- `curve` — `YieldCurve`, `SofrDay`, `SofrRate`, `TermStructure`, `Tenor`.
- `sources::bundled` — synchronous reader from local parquet (used by CLI `get`).
- `sources::parquet_io` — parquet writer (used by CLI `backfill` / `append-today`).
- `sources::treasury` — Treasury CSV fetcher (used by CLI only).
- `sources::sofr` — NY Fed SOFR CSV fetcher (used by CLI only).
- `interpolation` — linear interpolation between tenor knots.
- `error` — typed error enums.

### curvekit-cli (binary)

All network I/O for data ingestion lives here. Consumes the `curvekit` lib.

## Cache semantics

Cache directory: `~/.cache/curvekit/` (XDG via the `directories` crate).
Override: `$CURVEKIT_CACHE_DIR`.

```
~/.cache/curvekit/
├── treasury-2024.parquet
├── treasury-2024.parquet.etag
├── sofr-2024.parquet
└── sofr-2024.parquet.etag
```

On each `Curvekit` method call the internal `CachedFetcher::fetch` runs this
logic for the relevant year file:

1. If the file is cached and an ETag is stored, send `If-None-Match`.
2. `304 Not Modified` → return cached bytes, no download.
3. `2xx` → write new body + new ETag, return bytes.
4. Non-2xx (and not 304) → return error.
5. Network error + cache present → log warning, return stale cache.
6. Network error + no cache → return error.

**Stale fallback** means existing workflows survive transient outages or
offline operation after the cache is warm.

**Base URL override:** `$CURVEKIT_BASE_URL` replaces
`https://raw.githubusercontent.com/userFRM/curvekit/main/data` — useful for
pointing at a fork or a self-hosted mirror.

## Data format

### Treasury parquet schema

Long format: one row per (date, tenor). 12 tenors × ~250 trading days/year ≈
3 000 rows/year.

| Column | Arrow type | Description |
|---|---|---|
| `date` | `Date32` | Days since Unix epoch (1970-01-01) |
| `tenor_days` | `UInt32` | Days to maturity (30/60/91/182/365/730/…) |
| `yield_bps` | `UInt32` | Continuously-compounded rate × 10 000 |

### SOFR parquet schema

| Column | Arrow type | Description |
|---|---|---|
| `date` | `Date32` | Days since Unix epoch |
| `rate_bps` | `UInt32` | Continuously-compounded rate × 10 000 |

Rates are stored as integer basis-point counts to avoid floating-point drift.

## Rate conversion

Treasury publishes Bond Equivalent Yields (BEY, semi-annual). curvekit
converts on ingest to continuously compounded:

```
BEY  = column_value / 100
APY  = (1 + BEY / 2)^2 − 1
r_cc = ln(1 + APY)
```

SOFR is published as a percentage; conversion: `r_cc = ln(1 + rate_pct / 100)`.

## Refresh schedule

| Workflow | Trigger | Action |
|---|---|---|
| `nightly.yml` | `0 3 * * 1-5` (03:00 UTC, Mon–Fri) | `append-today` — yesterday's Treasury + latest SOFR |
| `backfill.yml` | `workflow_dispatch` (manual) | `backfill` — full historical fetch (25 years) |

## Interpolation

`YieldCurve::get(impl Into<Tenor>)` uses piecewise linear interpolation between
bracketing tenor knots via `curvekit::interpolation::linear`. The function does
flat extrapolation at the boundaries (clamps to the shortest/longest available
tenor). `TermStructure::rate_for` inserts SOFR at the 1-day point before
interpolating, providing a short-end anchor.
