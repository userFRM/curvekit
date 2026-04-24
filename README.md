# curvekit

Risk-free rate service for Rust — US Treasury yield curve + SOFR overnight
rate, served from bundled parquet with runtime GitHub fetch and local cache.
No API keys. Offline after first query.

## Install

```toml
# Cargo.toml
[dependencies]
curvekit = { git = "https://github.com/userFRM/curvekit" }
```

Once published to crates.io: `cargo add curvekit`

## Quick start — one-off scripts

```rust
use curvekit::Tenor;

#[tokio::main]
async fn main() -> curvekit::Result<()> {
    // Free functions — no client setup, no chrono import
    let curve = curvekit::treasury_curve_for("2020-03-20").await?;
    let r     = curvekit::treasury_rate_at("2020-03-20", Tenor::Y10).await?;
    let today = curvekit::treasury_today().await?;
    let sofr  = curvekit::sofr_today().await?;

    println!("10Y on 2020-03-20: {r:.6}");
    println!("Latest Treasury:   {}", today.date);
    println!("Latest SOFR:       {:.4}%", sofr.rate * 100.0);
    Ok(())
}
```

## Client pattern — connection pool + cache reuse

```rust
use curvekit::{Curvekit, Date, Tenor};

#[tokio::main]
async fn main() -> curvekit::Result<()> {
    let client = Curvekit::new();   // infallible, no ?

    // Any date form — no chrono import needed
    let curve = client.treasury_curve("2020-03-20").await?;
    let curve = client.treasury_curve(20200320u32).await?;
    let curve = client.treasury_curve((2020i32, 3u32, 20u32)).await?;
    let curve = client.treasury_curve(Date::today_et()).await?;

    // Named tenors
    let r_10y = curve.get(Tenor::Y10).unwrap_or(0.0);
    let r_3m  = curve.get(Tenor::M3).unwrap_or(0.0);
    println!("2020-03-20  10Y: {r_10y:.4}  3M: {r_3m:.4}");

    // Ad-hoc tenors (linear interpolation between knots)
    let r_45d = curve.get(Tenor::days(45));
    let r_18m = curve.get(Tenor::months(18));

    // Interpolated rate in one call
    let r = client.treasury_rate("2020-03-20", Tenor::Y10).await?;
    println!("10Y interpolated: {r:.6}");

    // Latest SOFR observation
    let sofr = client.sofr_latest().await?;
    println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);

    // Blocking from sync code — no async runtime needed
    let curve = client.treasury_curve_blocking(20200320u32)?;
    let r     = client.treasury_rate_blocking("2020-03-20", Tenor::Y10)?;

    Ok(())
}
```

## CLI

```bash
# Print Treasury curve for a date
curvekit-cli get treasury --date 2026-04-14

# Print SOFR rate for a date
curvekit-cli get sofr --date 2026-04-14

# Backfill full history (run once or via CI)
curvekit-cli backfill

# Append yesterday's data (used by nightly CI)
curvekit-cli append-today
```

## API surface

### Free functions (one-off scripts)

| Function | Returns |
|---|---|
| `treasury_today()` | `Result<YieldCurve>` — latest par curve |
| `treasury_curve_for(date)` | `Result<YieldCurve>` — par curve |
| `treasury_rate_at(date, tenor)` | `Result<f64>` — interpolated cc rate |
| `sofr_today()` | `Result<SofrDay>` — latest SOFR observation |

### Client methods — Treasury

| Method | Returns |
|---|---|
| `treasury_par_curve(date)` | `Result<YieldCurve>` — par yields |
| `treasury_zero_curve(date)` | `Result<YieldCurve>` — bootstrapped spot rates |
| `treasury_range(start, end)` | `Result<Vec<YieldCurve>>` — par, chronological |
| `treasury_rate(date, tenor)` | `Result<f64>` — interpolated cc rate |
| `treasury_rate_with_convention(date, tenor, dc)` | `Result<f64>` — with ISDA day-count |
| `treasury_latest()` | `Result<YieldCurve>` |
| `treasury_earliest_date()` | `Result<NaiveDate>` |
| `treasury_curve(date)` | `Result<YieldCurve>` — **deprecated** since 1.0.0; use `treasury_par_curve` |

### Client methods — overnight rates

| Method | Returns |
|---|---|
| `sofr(date)` | `Result<f64>` — cc overnight rate |
| `sofr_range(start, end)` | `Result<Vec<SofrDay>>` |
| `sofr_latest()` | `Result<SofrDay>` |
| `sofr_earliest_date()` | `Result<NaiveDate>` |
| `effr(date)` | `Result<f64>` — Effective Federal Funds Rate (cc) |
| `effr_range(start, end)` | `Result<Vec<EffrDay>>` |
| `effr_latest()` | `Result<EffrDay>` |
| `effr_earliest_date()` | `Result<NaiveDate>` |
| `obfr(date)` | `Result<f64>` — Overnight Bank Funding Rate (cc) |
| `obfr_range(start, end)` | `Result<Vec<ObfrDay>>` |
| `obfr_latest()` | `Result<ObfrDay>` |
| `obfr_earliest_date()` | `Result<NaiveDate>` |

### Client methods — blocking (sync)

| Method | Returns |
|---|---|
| `treasury_curve_blocking(date)` | `Result<YieldCurve>` |
| `treasury_range_blocking(start, end)` | `Result<Vec<YieldCurve>>` |
| `treasury_rate_blocking(date, tenor)` | `Result<f64>` |
| `treasury_latest_blocking()` | `Result<YieldCurve>` |
| `sofr_blocking(date)` | `Result<f64>` |
| `sofr_range_blocking(start, end)` | `Result<Vec<SofrDay>>` |
| `sofr_latest_blocking()` | `Result<SofrDay>` |

### Day-count conventions

```rust
use curvekit::{DayCount, Tenor};

let r = client
    .treasury_rate_with_convention("2024-01-15", Tenor::Y10, DayCount::Act360)
    .await?;
```

Available conventions: `DayCount::Act360`, `DayCount::Act365Fixed`,
`DayCount::Thirty360`, `DayCount::ActAct` (Act/Act ISDA).

All rates are continuously compounded internally; `year_fraction` is applied
when converting for a specific day-count basis.

### Date inputs

Every method that takes a date accepts any of these forms via `impl IntoDate`:

```rust
client.treasury_curve("2020-03-20").await?          // ISO string
client.treasury_curve("2020/03/20").await?          // slashed string
client.treasury_curve(20200320u32).await?           // YYYYMMDD integer
client.treasury_curve((2020i32, 3u32, 20u32)).await? // YMD tuple
client.treasury_curve(Date::today_et()).await?      // Date newtype
client.treasury_curve(naive_date).await?            // NaiveDate (compat)
```

### Tenor

`YieldCurve::get` and `YieldCurve::yield_at` both accept `impl Into<Tenor>` —
pass `Tenor::Y10`, `Tenor::days(45)`, or a raw `u32` (backward-compatible).

`Tenor` constants: `ON`, `W1`, `M1`, `M2`, `M3`, `M6`, `Y1`, `Y2`, `Y3`,
`Y5`, `Y7`, `Y10`, `Y20`, `Y30`.  Constructors: `Tenor::days(n)`,
`Tenor::weeks(n)`, `Tenor::months(n)`, `Tenor::years(n)`.
Parse from string (`"10Y"`, `"3M"`, `"45D"`, `"2W"`, `"ON"`) via
`"10Y".parse::<Tenor>()`.

All rates are continuously compounded. See [`docs/api.md`](docs/api.md) for
full method signatures, parameters, and error conditions.

## Data

| Source | Coverage | Published |
|---|---|---|
| US Treasury Par Yield Curve | 2002 – present | ~15:30 ET, business days |
| NY Fed SOFR | 2018-04-02 – present | ~08:00 ET, business days |
| NY Fed EFFR | 1954 – present | ~08:00 ET, business days |
| NY Fed OBFR | 2016-03-01 – present | ~08:00 ET, business days |

Parquet files live in `data/` and are updated by two GitHub Actions workflows:

- **nightly.yml** — cron `0 3 * * 1-5` (03:00 UTC, Mon–Fri): appends
  yesterday's Treasury curve and latest SOFR/EFFR/OBFR to the current-year files.
- **backfill.yml** — `workflow_dispatch`: full historical fetch (all sources).

## v1.0 — stability guarantees

- **Semver**: public API follows [Semantic Versioning](https://semver.org/).
  Breaking changes require a major version bump. Deprecations are announced
  one minor version before removal.
- **MSRV**: Rust stable ≥ 1.75 (edition 2021). MSRV changes are a minor bump.
- **Re-export policy**: every type and function re-exported from `curvekit::*`
  is considered public API. Internal modules (`sources`, `fetcher`) are
  `pub(crate)` and not covered.
- **Error stability**: `curvekit::Error` variants are stable; adding new
  variants is a minor bump, removing them is a major bump.

## Cache

On first use, `Curvekit` downloads each year file from
`raw.githubusercontent.com/userFRM/curvekit/main/data/` and writes it to
`~/.cache/curvekit/` (XDG-compliant via the `directories` crate). Subsequent
calls check the SHA-256 digest listed in `data/manifest.json`; an unmodified
cached file is returned immediately. On network failure the stale cached file
is returned so existing workflows survive transient outages.

Each file's SHA-256 digest is verified against `manifest.json` before being
written to cache. A `ChecksumMismatch` error is returned if verification
fails.

**Env overrides:**

| Variable | Effect |
|---|---|
| `CURVEKIT_BASE_URL` | Replace the GitHub raw origin |
| `CURVEKIT_CACHE_DIR` | Override the cache directory |
| `CURVEKIT_MIRROR_URL` | CDN fallback URL (default: jsDelivr) |

See [`docs/architecture.md`](docs/architecture.md) for the full data-flow
diagram and [`docs/data-sources.md`](docs/data-sources.md) for upstream URL
details.

## Crates

| Crate | Description |
|---|---|
| `curvekit` | Library — fetcher, cache, types, interpolation |
| `curvekit-cli` | Binary — backfill, append-today, get |

## License

Apache-2.0 — see [`LICENSE`](LICENSE).

Copyright 2026 userFRM
