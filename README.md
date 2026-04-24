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

## Quick start

```rust
use curvekit::Curvekit;
use chrono::NaiveDate;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Curvekit::new()?;

    // Full Treasury yield curve for a date
    let curve = client.treasury_curve(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap()).await?;
    println!("2020-03-20 10Y: {:.4}", curve.get(3650).unwrap_or(0.0));

    // Interpolated rate at an arbitrary tenor
    let r_30d = client.treasury_rate(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(), 30).await?;
    println!("30d interp: {r_30d:.4}");

    // Latest SOFR observation
    let sofr = client.sofr_latest().await?;
    println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);

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

| Method | Returns |
|---|---|
| `treasury_curve(date)` | `Result<YieldCurve>` |
| `treasury_range(start, end)` | `Result<Vec<YieldCurve>>` |
| `treasury_rate(date, days)` | `Result<f64>` — interpolated cc rate |
| `treasury_latest()` | `Result<YieldCurve>` |
| `treasury_earliest_date()` | `Result<NaiveDate>` |
| `sofr(date)` | `Result<f64>` — cc overnight rate |
| `sofr_range(start, end)` | `Result<Vec<SofrDay>>` |
| `sofr_latest()` | `Result<SofrDay>` |
| `sofr_earliest_date()` | `Result<NaiveDate>` |

All rates are continuously compounded. See [`docs/api.md`](docs/api.md) for
full method signatures, parameters, and error conditions.

## Data

| Source | Coverage | Published |
|---|---|---|
| US Treasury Par Yield Curve | 2002 – present | ~15:30 ET, business days |
| NY Fed SOFR | 2018-04-02 – present | ~08:00 ET, business days |

Parquet files live in `data/` and are updated by two GitHub Actions workflows:

- **nightly.yml** — cron `0 3 * * 1-5` (03:00 UTC, Mon–Fri): appends
  yesterday's Treasury curve and latest SOFR to the current-year file.
- **backfill.yml** — `workflow_dispatch`: full historical fetch (25 years).

## Cache

On first use, `Curvekit` downloads each year file from
`raw.githubusercontent.com/userFRM/curvekit/main/data/` and writes it to
`~/.cache/curvekit/` (XDG-compliant via the `directories` crate). Subsequent
calls send `If-None-Match` with the stored ETag; a `304 Not Modified` skips
re-download. On network failure the stale cached file is returned so existing
workflows survive transient outages.

**Env overrides:**

| Variable | Effect |
|---|---|
| `CURVEKIT_BASE_URL` | Replace the GitHub raw origin |
| `CURVEKIT_CACHE_DIR` | Override the cache directory |

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

Copyright 2026 Theta Gamma Systems
