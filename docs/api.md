# API Reference

curvekit is a client library. Instantiate `Curvekit` once and call flat async
endpoint methods. No server process. No sockets.

## Types

### `YieldCurve`

```rust
pub struct YieldCurve {
    pub date: NaiveDate,
    /// Continuously-compounded yields keyed by days to maturity.
    pub points: BTreeMap<u32, f64>,
}
```

`points` keys are standard tenor approximations (days):
`30, 60, 91, 182, 365, 730, 1095, 1825, 2555, 3650, 7300, 10950`.

Missing maturities are absent from the map (not NaN). All rates are
continuously compounded (converted from Treasury BEY on ingest).

**Methods:**

| Method | Description |
|---|---|
| `get(days) → Option<f64>` | Exact lookup or linear interpolation |
| `insert(days, rate)` | Insert a yield point |
| `len() → usize` | Number of knots |
| `to_continuous_map() → HashMap<u32, f64>` | Copy into a plain HashMap |

### `SofrDay`

```rust
pub struct SofrDay {
    pub date: NaiveDate,
    pub rate: f64,   // continuously compounded
}
```

### `TermStructure`

Combined Treasury curve + SOFR anchor for a single date:

```rust
pub struct TermStructure {
    pub date: NaiveDate,
    pub treasury: YieldCurve,
    pub sofr: Option<SofrRate>,
}
```

`rate_for_days(days) → Option<f64>` inserts SOFR at the 1-day point before
interpolating, giving a continuous term structure from overnight to 30Y.

### `Tenor` constants

```rust
Tenor::M1  // 30d
Tenor::M2  // 60d
Tenor::M3  // 91d
Tenor::M6  // 182d
Tenor::Y1  // 365d
Tenor::Y2  // 730d
Tenor::Y3  // 1095d
Tenor::Y5  // 1825d
Tenor::Y7  // 2555d
Tenor::Y10 // 3650d
Tenor::Y20 // 7300d
Tenor::Y30 // 10950d
```

## `Curvekit` client

### `Curvekit::new`

```rust
pub fn new() -> Result<Self>
```

Creates a client with the default GitHub raw backend and XDG cache directory.
Reads `CURVEKIT_BASE_URL` and `CURVEKIT_CACHE_DIR` from the environment if set.

**Errors:** reqwest client construction failure (very unlikely in practice).

```rust
let client = Curvekit::new()?;
```

### Builder: `with_base_url`

```rust
pub fn with_base_url(self, url: impl Into<String>) -> Self
```

Override the origin URL. Default:
`https://raw.githubusercontent.com/userFRM/curvekit/main/data`

Useful for pointing at a fork or a self-hosted mirror.

```rust
let client = Curvekit::new()?
    .with_base_url("https://my-mirror.example.com/curvekit");
```

### Builder: `with_cache_dir`

```rust
pub fn with_cache_dir(self, dir: PathBuf) -> Self
```

Override the on-disk cache directory. Default: `~/.cache/curvekit/` (XDG).

```rust
let client = Curvekit::new()?
    .with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
```

## Treasury endpoints

### `treasury_curve`

```rust
pub async fn treasury_curve(&self, date: NaiveDate) -> Result<YieldCurve>
```

Fetch the full US Treasury Par Yield Curve for a single trading date.

**Parameters:**
- `date` — the trading date. Must fall within 2002–present.

**Returns:** `YieldCurve` with up to 12 tenor points.

**Errors:**
- Network error with no cached file.
- `date` not found in the year file (weekend, holiday, or before data coverage).

```rust
# use curvekit::Curvekit;
# use chrono::NaiveDate;
let client = Curvekit::new()?;
let curve = client
    .treasury_curve(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap())
    .await?;
println!("10Y: {:.4}%", curve.get(3650).unwrap_or(0.0) * 100.0);
```

### `treasury_range`

```rust
pub async fn treasury_range(
    &self,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<YieldCurve>>
```

Fetch all Treasury curves in `[start, end]` inclusive. Fetches each calendar
year in the span in parallel. Non-trading days are absent from the result.

**Parameters:**
- `start`, `end` — inclusive date range. `start` must be ≤ `end`.

**Returns:** `Vec<YieldCurve>` sorted ascending by date.

**Errors:**
- `start > end`.
- Network error for any year in the span with no cached file.

```rust
# use curvekit::Curvekit;
# use chrono::NaiveDate;
let client = Curvekit::new()?;
let curves = client
    .treasury_range(
        NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2020, 12, 31).unwrap(),
    )
    .await?;
println!("Trading days in 2020: {}", curves.len());
```

### `treasury_rate`

```rust
pub async fn treasury_rate(&self, date: NaiveDate, days: u32) -> Result<f64>
```

Interpolated continuously-compounded rate for `days` to maturity on `date`.
Calls `treasury_curve` internally then applies linear interpolation via
`YieldCurve::get`.

**Parameters:**
- `date` — the trading date.
- `days` — days to maturity. Any positive value; extrapolates flat at boundaries.

**Returns:** `f64` continuously-compounded rate.

**Errors:**
- `date` not found (see `treasury_curve`).
- Empty curve for the date (all tenors missing).

```rust
# use curvekit::Curvekit;
# use chrono::NaiveDate;
let client = Curvekit::new()?;
let r = client
    .treasury_rate(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(), 45)
    .await?;
println!("45d rate: {r:.6}");
```

### `treasury_latest`

```rust
pub async fn treasury_latest(&self) -> Result<YieldCurve>
```

Latest available Treasury yield curve. Fetches the current calendar year;
falls back to the previous year if no data is present yet (e.g. early January
before the first trading day).

**Returns:** Most recent `YieldCurve` by date.

**Errors:**
- Network error with no cached files for both the current and previous year.

```rust
# use curvekit::Curvekit;
let client = Curvekit::new()?;
let curve = client.treasury_latest().await?;
println!("Latest: {}", curve.date);
```

### `treasury_earliest_date`

```rust
pub async fn treasury_earliest_date(&self) -> Result<NaiveDate>
```

Earliest date for which Treasury data is available (fetches `treasury-2000.parquet`
from the remote). Coverage in practice starts 2002-01-02.

**Returns:** First `NaiveDate` in the earliest year file.

**Errors:**
- Network error with no cached file for year 2000.

```rust
# use curvekit::Curvekit;
let client = Curvekit::new()?;
let d = client.treasury_earliest_date().await?;
println!("Earliest treasury: {d}");
```

## SOFR endpoints

### `sofr`

```rust
pub async fn sofr(&self, date: NaiveDate) -> Result<f64>
```

SOFR overnight rate (continuously compounded) for a single date.

**Parameters:**
- `date` — the observation date. Must be a business day on or after 2018-04-02.

**Returns:** `f64` continuously-compounded overnight rate.

**Errors:**
- Network error with no cached file.
- `date` not found (weekend, holiday, or before SOFR inception).

```rust
# use curvekit::Curvekit;
# use chrono::NaiveDate;
let client = Curvekit::new()?;
let r = client.sofr(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap()).await?;
println!("SOFR: {r:.6}");
```

### `sofr_range`

```rust
pub async fn sofr_range(
    &self,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<SofrDay>>
```

All SOFR observations in `[start, end]` inclusive. Fetches each year in the
span in parallel. Non-business days are absent.

**Parameters:**
- `start`, `end` — inclusive date range. `start` must be ≤ `end`.

**Returns:** `Vec<SofrDay>` sorted ascending by date.

**Errors:**
- `start > end`.
- Network error for any year in the span with no cached file.

```rust
# use curvekit::Curvekit;
# use chrono::NaiveDate;
let client = Curvekit::new()?;
let rates = client
    .sofr_range(
        NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2023, 12, 31).unwrap(),
    )
    .await?;
println!("SOFR observations in 2023: {}", rates.len());
```

### `sofr_latest`

```rust
pub async fn sofr_latest(&self) -> Result<SofrDay>
```

Latest available SOFR observation. Fetches the current calendar year; falls
back to the previous year if no data is present yet.

**Returns:** Most recent `SofrDay` by date.

**Errors:**
- Network error with no cached files for both the current and previous year.

```rust
# use curvekit::Curvekit;
let client = Curvekit::new()?;
let sofr = client.sofr_latest().await?;
println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
```

### `sofr_earliest_date`

```rust
pub async fn sofr_earliest_date(&self) -> Result<NaiveDate>
```

Earliest date for which SOFR data is available (fetches `sofr-2018.parquet`).
SOFR began 2018-04-02.

**Returns:** First `NaiveDate` in `sofr-2018.parquet`.

**Errors:**
- Network error with no cached file for year 2018.

```rust
# use curvekit::Curvekit;
let client = Curvekit::new()?;
let d = client.sofr_earliest_date().await?;
println!("SOFR inception: {d}");
```

## CLI reference

```bash
# Read commands (from bundled/cached parquet)
curvekit-cli get treasury --date YYYY-MM-DD
curvekit-cli get sofr --date YYYY-MM-DD

# Write commands (fetch from upstream and write to data/)
curvekit-cli backfill                          # both sources, 25 years
curvekit-cli backfill --years 5               # last 5 years, both sources
curvekit-cli backfill --source treasury --year 2024
curvekit-cli backfill --source sofr --year 2023
curvekit-cli append-today                     # yesterday's close, used by nightly CI
```

Override the data directory: `--data-dir /path/to/data` or `$CURVEKIT_DATA_DIR`.
