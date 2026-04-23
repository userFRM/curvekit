# API Reference

curvekit is a library — there is no HTTP server or RPC interface.
Consumers link against the `curvekit` crate and call synchronous functions.

## Reader API (`curvekit::`)

```rust
/// Full Treasury yield curve for a date from bundled parquet.
pub fn treasury_curve(date: NaiveDate) -> Result<YieldCurve>;

/// SOFR overnight rate for a date (continuously compounded).
pub fn sofr(date: NaiveDate) -> Result<f64>;

/// Interpolated continuously-compounded rate at arbitrary tenor.
pub fn rate_for_days(date: NaiveDate, days: u32) -> Result<f64>;

/// Latest date for which Treasury data is bundled.
pub fn treasury_latest_date() -> NaiveDate;

/// Latest date for which SOFR data is bundled.
pub fn sofr_latest_date() -> NaiveDate;
```

## Writer API (`curvekit::sources::parquet_io::`)

Used by the CLI. Consumers that only read data do not need this.

```rust
/// Write one year of Treasury curves to data_dir/treasury-{year}.parquet.
pub fn write_treasury_year(data_dir: &Path, year: i32, curves: &[YieldCurveDay]) -> Result<()>;

/// Write one year of SOFR rates to data_dir/sofr-{year}.parquet.
pub fn write_sofr_year(data_dir: &Path, year: i32, rates: &[SofrDay]) -> Result<()>;

/// Append one day's Treasury curve (reads existing file, merges, rewrites).
pub fn append_treasury_day(data_dir: &Path, date: NaiveDate, curve: &YieldCurve) -> Result<()>;

/// Append one day's SOFR rate.
pub fn append_sofr_day(data_dir: &Path, date: NaiveDate, rate: f64) -> Result<()>;
```

## CLI (`curvekit-cli`)

```
curvekit-cli backfill --years 25
curvekit-cli backfill --source treasury --year 2024
curvekit-cli append-today
curvekit-cli get treasury --date 2026-04-14
curvekit-cli get sofr --date 2026-04-14
```

The `--data-dir` flag (or `$CURVEKIT_DATA_DIR` env var) overrides the default
`<repo-root>/data/` directory.
