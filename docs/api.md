# API Reference

curvekit is a client library. Instantiate `Curvekit` once and call flat async
endpoint methods. No server process. No sockets. No chrono import needed.

## Date inputs

All methods that take a date accept `impl IntoDate`:

```rust
client.treasury_curve("2020-03-20").await?           // ISO string
client.treasury_curve("2020/03/20").await?           // slashed
client.treasury_curve(20200320u32).await?            // YYYYMMDD integer
client.treasury_curve((2020i32, 3u32, 20u32)).await?  // YMD tuple
client.treasury_curve(Date::today_et()).await?       // Date newtype
client.treasury_curve(naive_date).await?             // NaiveDate (compat)
```

## Types

### `Date`

```rust
pub struct Date(NaiveDate);
```

Ergonomic date wrapper. Constructed from strings, integers, or tuples; no
`chrono` import required.

**Constructors:**

| Constructor | Input |
|---|---|
| `Date::from_ymd(y, m, d)` | Returns `Result<Date, DateError>` |
| `Date::from_yyyymmdd(v)` | Returns `Result<Date, DateError>` |
| `Date::today_et()` | System local (ET-oriented) date |
| `Date::today_utc()` | UTC date |
| `"2020-03-20".parse::<Date>()` | ISO / slashed / compact string |

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
| `get(impl Into<Tenor>) → Option<f64>` | Exact lookup or linear interpolation |
| `yield_at(impl Into<Tenor>) → Option<f64>` | Alias for `get` |
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

`rate_for(impl Into<Tenor>) → Option<f64>` inserts SOFR at the 1-day point
before interpolating, giving a continuous term structure from overnight to 30Y.

### `Tenor`

Semantic type for yield-curve maturities. Stored internally as days.

**Named constants (Treasury calendar knots):**

```rust
Tenor::ON  // 1d   overnight
Tenor::W1  // 7d
Tenor::M1  // 30d
Tenor::M2  // 60d
Tenor::M3  // 91d  (not 3×30=90 — Treasury knot)
Tenor::M6  // 182d (not 6×30=180 — Treasury knot)
Tenor::Y1  // 365d
Tenor::Y2  // 730d
Tenor::Y3  // 1095d
Tenor::Y5  // 1826d (not 5×365=1825 — Treasury knot)
Tenor::Y7  // 2555d
Tenor::Y10 // 3650d
Tenor::Y20 // 7300d
Tenor::Y30 // 10950d
```

**Constructors:**

```rust
Tenor::days(45)       // exact days
Tenor::weeks(2)       // 2 × 7 = 14d
Tenor::months(3)      // 3 × 30 = 90d
Tenor::years(10)      // 10 × 365 = 3650d
```

**Parse from string:** `"10Y".parse::<Tenor>()`, `"3M"`, `"45D"`, `"2W"`, `"ON"`.
All string forms use the mathematical approximations (`"3M"` → 90d, not Treasury's 91d).

All methods accept `impl Into<Tenor>`, so raw `u32` day counts still compile.

### `Error`

Single unified error type. Users only need to match `curvekit::Error`:

```rust
pub enum Error {
    Date(DateError),
    Treasury(String),
    Sofr(String),
    Parquet(String),
    Interpolation(String),
    DateNotFound(String),
    Http(reqwest::Error),
    Io(std::io::Error),
    // ...
}
```

`curvekit::Result<T>` is `std::result::Result<T, curvekit::Error>`.

## Free functions

For one-off scripts; internally share a process-wide `Curvekit` instance.

```rust
pub async fn treasury_today() -> Result<YieldCurve>
pub async fn treasury_curve_for(date: impl IntoDate) -> Result<YieldCurve>
pub async fn treasury_rate_at(date: impl IntoDate, tenor: impl Into<Tenor>) -> Result<f64>
pub async fn sofr_today() -> Result<SofrDay>
```

**Example:**

```rust
use curvekit::Tenor;

#[tokio::main]
async fn main() -> curvekit::Result<()> {
    let r = curvekit::treasury_rate_at("2020-03-20", Tenor::Y10).await?;
    println!("10Y on 2020-03-20: {r:.6}");
    Ok(())
}
```

## `Curvekit` client

### `Curvekit::new`

```rust
pub fn new() -> Self   // infallible
```

Creates a client with the default GitHub raw backend and XDG cache directory.
Reads `CURVEKIT_BASE_URL` and `CURVEKIT_CACHE_DIR` from the environment if set.

```rust
let client = Curvekit::new();   // no ? needed
```

### `Curvekit::try_new`

```rust
pub fn try_new() -> Result<Self>
```

Like `new()` but surfaces HTTP client construction failures immediately.
Prefer `new()` for typical use.

```rust
let client = Curvekit::try_new()?;
```

### Builder: `with_base_url`

```rust
pub fn with_base_url(self, url: impl Into<String>) -> Self
```

Override the origin URL. Default:
`https://raw.githubusercontent.com/userFRM/curvekit/main/data`

```rust
let client = Curvekit::new().with_base_url("https://my-mirror.example.com/curvekit");
```

### Builder: `with_cache_dir`

```rust
pub fn with_cache_dir(self, dir: PathBuf) -> Self
```

Override the on-disk cache directory. Default: `~/.cache/curvekit/` (XDG).

```rust
let client = Curvekit::new().with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
```

## Treasury endpoints

### `treasury_curve`

```rust
pub async fn treasury_curve(&self, date: impl IntoDate) -> Result<YieldCurve>
```

Fetch the full US Treasury Par Yield Curve for a single trading date.

**Returns:** `YieldCurve` with up to 12 tenor points.

**Errors:**
- Network error with no cached file.
- `date` not found in the year file (weekend, holiday, or before data coverage).

```rust
let client = Curvekit::new();
let curve = client.treasury_curve("2020-03-20").await?;
println!("10Y: {:.4}%", curve.get(Tenor::Y10).unwrap_or(0.0) * 100.0);
```

### `treasury_range`

```rust
pub async fn treasury_range(
    &self,
    start: impl IntoDate,
    end: impl IntoDate,
) -> Result<Vec<YieldCurve>>
```

Fetch all Treasury curves in `[start, end]` inclusive. Fetches each calendar
year in the span in parallel. Non-trading days are absent from the result.

```rust
let client = Curvekit::new();
let curves = client.treasury_range("2020-01-01", "2020-12-31").await?;
println!("Trading days in 2020: {}", curves.len());
```

### `treasury_rate`

```rust
pub async fn treasury_rate(
    &self,
    date: impl IntoDate,
    tenor: impl Into<Tenor>,
) -> Result<f64>
```

Interpolated continuously-compounded rate for `tenor` on `date`.

```rust
let client = Curvekit::new();
let r = client.treasury_rate("2020-03-20", Tenor::Y10).await?;
println!("10Y rate: {r:.6}");
```

### `treasury_latest`

```rust
pub async fn treasury_latest(&self) -> Result<YieldCurve>
```

Latest available Treasury yield curve.

```rust
let client = Curvekit::new();
let curve = client.treasury_latest().await?;
println!("Latest: {}", curve.date);
```

### `treasury_earliest_date`

```rust
pub async fn treasury_earliest_date(&self) -> Result<NaiveDate>
```

Earliest date for which Treasury data is available (coverage starts 2002-01-02).

## SOFR endpoints

### `sofr`

```rust
pub async fn sofr(&self, date: impl IntoDate) -> Result<f64>
```

SOFR overnight rate (continuously compounded) for a single date.

```rust
let client = Curvekit::new();
let r = client.sofr("2020-03-20").await?;
println!("SOFR: {r:.6}");
```

### `sofr_range`

```rust
pub async fn sofr_range(
    &self,
    start: impl IntoDate,
    end: impl IntoDate,
) -> Result<Vec<SofrDay>>
```

All SOFR observations in `[start, end]` inclusive.

```rust
let client = Curvekit::new();
let rates = client.sofr_range("2023-01-01", "2023-12-31").await?;
println!("SOFR observations in 2023: {}", rates.len());
```

### `sofr_latest`

```rust
pub async fn sofr_latest(&self) -> Result<SofrDay>
```

Latest available SOFR observation.

```rust
let client = Curvekit::new();
let sofr = client.sofr_latest().await?;
println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
```

### `sofr_earliest_date`

```rust
pub async fn sofr_earliest_date(&self) -> Result<NaiveDate>
```

Earliest date for which SOFR data is available. SOFR began 2018-04-02.

## Blocking variants

Every async method has a `_blocking` counterpart that works from sync code.
Uses `block_in_place` inside an existing tokio runtime, or spins a minimal
single-thread runtime when called outside one.

```rust
let client = Curvekit::new();
let curve = client.treasury_curve_blocking("2020-03-20")?;
let rates = client.treasury_range_blocking("2020-01-01", "2020-12-31")?;
let r     = client.treasury_rate_blocking("2020-03-20", Tenor::Y10)?;
let curve = client.treasury_latest_blocking()?;
let r     = client.sofr_blocking("2020-03-20")?;
let rates = client.sofr_range_blocking("2020-01-01", "2020-12-31")?;
let sofr  = client.sofr_latest_blocking()?;
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
