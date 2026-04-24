//! Stateful `Curvekit` client — flat async endpoint methods.
//!
//! Fetches parquet files from GitHub raw (or a configurable origin) with an
//! XDG-compliant local cache + ETag revalidation. Falls back to stale cache on
//! network errors so existing workflows survive transient outages.
//!
//! # Example
//!
//! ```no_run
//! use curvekit::{Curvekit, Tenor};
//!
//! #[tokio::main]
//! async fn main() -> curvekit::Result<()> {
//!     let client = Curvekit::new();   // infallible
//!
//!     // Any date form works — no chrono import needed
//!     let curve = client.treasury_curve("2020-03-20").await?;
//!     println!("10Y: {:.4}%", curve.get(Tenor::Y10).unwrap_or(0.0) * 100.0);
//!
//!     let sofr = client.sofr_latest().await?;
//!     println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
//!     Ok(())
//! }
//! ```

use chrono::Datelike;
use futures::future::try_join_all;
use std::path::PathBuf;

use crate::curve::{SofrDay, YieldCurve};
use crate::date::{Date, IntoDate};
use crate::error::{Error, Result};
use crate::fetcher::{default_cache_dir, resolved_base_url, CachedFetcher};
use crate::sources::parquet_io::{read_sofr_year, read_treasury_year};
use crate::tenor::Tenor;

/// Stateful curvekit client.
///
/// Wraps an ETag-aware cached fetcher and exposes flat endpoint methods.
/// Create once and reuse across calls; the internal reqwest client is kept
/// alive for connection pooling.
///
/// # Infallible construction
///
/// ```no_run
/// use curvekit::Curvekit;
///
/// let client = Curvekit::new();   // never fails
/// ```
///
/// # Builder pattern
///
/// ```no_run
/// use curvekit::Curvekit;
/// use std::path::PathBuf;
///
/// let client = Curvekit::new()
///     .with_base_url("https://my-mirror.example.com/curvekit")
///     .with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
/// ```
pub struct Curvekit {
    fetcher: CachedFetcher,
}

impl Curvekit {
    /// Create a client with the default GitHub raw backend and XDG cache.
    ///
    /// Reads `CURVEKIT_BASE_URL` and `CURVEKIT_CACHE_DIR` from the environment
    /// if set, otherwise uses the GitHub raw origin and `~/.cache/curvekit/`.
    ///
    /// **This function never fails.** If the underlying HTTP client cannot be
    /// built (essentially only on exotic platforms with broken TLS), the error
    /// is deferred to the first fetch call. Use [`try_new`][Self::try_new] for
    /// early detection.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    /// let client = Curvekit::new();   // no ? needed
    /// ```
    pub fn new() -> Self {
        // If the client cannot be built we store a fallback that will fail on
        // the first actual fetch. In practice reqwest::Client::builder().build()
        // only fails on platforms that lack TLS support — essentially never.
        let http = reqwest::Client::builder()
            .user_agent("curvekit/0.1 (+https://github.com/userFRM/curvekit)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            fetcher: CachedFetcher {
                http,
                base_url: resolved_base_url(),
                cache_dir: default_cache_dir(),
            },
        }
    }

    /// Create a client with early failure detection.
    ///
    /// Like [`new`][Self::new] but returns an error immediately if the HTTP
    /// client cannot be constructed. Prefer [`new`][Self::new] for typical use;
    /// use this in contexts where surfacing TLS init failures immediately matters.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the underlying reqwest client cannot be constructed
    /// (TLS init failure — essentially never in practice).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    /// let client = Curvekit::try_new()?;
    /// # Ok::<(), curvekit::Error>(())
    /// ```
    pub fn try_new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("curvekit/0.1 (+https://github.com/userFRM/curvekit)")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            fetcher: CachedFetcher {
                http,
                base_url: resolved_base_url(),
                cache_dir: default_cache_dir(),
            },
        })
    }

    /// Override the origin URL.
    ///
    /// Default: `https://raw.githubusercontent.com/userFRM/curvekit/main/data`.
    /// Useful for pointing at a fork or a self-hosted mirror.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    /// let client = Curvekit::new().with_base_url("https://my-mirror.example.com/curvekit");
    /// ```
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.fetcher.base_url = url.into();
        self
    }

    /// Override the on-disk cache directory.
    ///
    /// Default: `~/.cache/curvekit/` (XDG via the `directories` crate).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    /// use std::path::PathBuf;
    /// let client = Curvekit::new().with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
    /// ```
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.fetcher.cache_dir = dir;
        self
    }

    // ── Treasury endpoints ────────────────────────────────────────────────────

    /// Fetch the full US Treasury Par Yield Curve for a single date.
    ///
    /// Accepts any date form — no `chrono` import required:
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    ///
    /// // ISO string
    /// let curve = client.treasury_curve("2020-03-20").await?;
    ///
    /// // Compact integer YYYYMMDD
    /// let curve = client.treasury_curve(20200320u32).await?;
    ///
    /// // YMD tuple (year as i32, month/day as u32)
    /// let curve = client.treasury_curve((2020i32, 3u32, 20u32)).await?;
    ///
    /// // Existing NaiveDate (still works — infallible From conversion)
    /// use chrono::NaiveDate;
    /// let nd = NaiveDate::from_ymd_opt(2020, 3, 20).expect("valid date");
    /// let curve = client.treasury_curve(nd).await?;
    /// # Ok(()) }
    /// ```
    ///
    /// Resolves the date to a year file (`treasury-{year}.parquet`), fetches
    /// and caches it (ETag revalidation), then filters to the requested date.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for the year.
    /// - `date` is not present in the year file (weekend, holiday, or outside
    ///   coverage 2002–present).
    /// - Invalid date string or YYYYMMDD value.
    pub async fn treasury_curve(&self, date: impl IntoDate) -> Result<YieldCurve> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let curves = self.treasury_year(year).await?;
        curves
            .into_iter()
            .find(|c| c.date == date.inner())
            .ok_or_else(|| Error::DateNotFound(format!("no treasury curve for {date}")))
    }

    /// Fetch all Treasury yield curves in `[start, end]` (inclusive).
    ///
    /// Determines the year span, fetches each year file in parallel, then
    /// filters to the requested date range. Non-trading days are absent.
    ///
    /// Both `start` and `end` accept any date form (string, u32, tuple, NaiveDate).
    ///
    /// # Errors
    ///
    /// - `start > end`.
    /// - Network failure for any year in the span with no cached file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let curves = client.treasury_range("2020-01-01", "2020-12-31").await?;
    /// println!("Trading days in 2020: {}", curves.len());
    /// # Ok(()) }
    /// ```
    pub async fn treasury_range(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<YieldCurve>> {
        let start = start.into_date()?;
        let end = end.into_date()?;
        if start > end {
            return Err(Error::Other(format!(
                "treasury_range: start {start} > end {end}"
            )));
        }
        let start_nd = start.inner();
        let end_nd = end.inner();
        let years: Vec<i32> = (start_nd.year()..=end_nd.year()).collect();
        let fetches = years
            .iter()
            .map(|&y| self.treasury_year(y))
            .collect::<Vec<_>>();
        let all_years = try_join_all(fetches).await?;
        let mut out: Vec<YieldCurve> = all_years
            .into_iter()
            .flatten()
            .filter(|c| c.date >= start_nd && c.date <= end_nd)
            .collect();
        out.sort_by_key(|c| c.date);
        Ok(out)
    }

    /// Interpolated continuously-compounded rate at `tenor` to maturity for `date`.
    ///
    /// Calls [`treasury_curve`][Self::treasury_curve] internally and applies
    /// linear interpolation via [`YieldCurve::get`]. Extrapolates flat at
    /// the shortest and longest available tenors.
    ///
    /// Accepts any date form and any type that converts into [`Tenor`].
    ///
    /// # Errors
    ///
    /// - Same as [`treasury_curve`][Self::treasury_curve].
    /// - The curve for `date` is empty (all tenors absent).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::{Curvekit, Tenor};
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    ///
    /// let r_10y = client.treasury_rate("2026-04-14", Tenor::Y10).await?;
    /// println!("10Y rate: {r_10y:.6}");
    ///
    /// let r_45d = client.treasury_rate(20260414u32, Tenor::days(45)).await?;
    /// println!("45d rate: {r_45d:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn treasury_rate(&self, date: impl IntoDate, tenor: impl Into<Tenor>) -> Result<f64> {
        let tenor = tenor.into();
        let curve = self.treasury_curve(date).await?;
        curve
            .get(tenor)
            .ok_or_else(|| Error::Interpolation(format!("no treasury data at {}", tenor)))
    }

    /// Latest available Treasury yield curve.
    ///
    /// Fetches the current calendar year; falls back to the previous year if
    /// no data is present yet (e.g. early January before the first trading day).
    ///
    /// # Errors
    ///
    /// - Network failure with no cached files for both the current and previous year.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let curve = client.treasury_latest().await?;
    /// println!("Latest: {}", curve.date);
    /// # Ok(()) }
    /// ```
    pub async fn treasury_latest(&self) -> Result<YieldCurve> {
        use chrono::Utc;
        let current_year = Utc::now().year();
        for year in [current_year, current_year - 1] {
            if let Ok(curves) = self.treasury_year(year).await {
                if let Some(latest) = curves.into_iter().max_by_key(|c| c.date) {
                    return Ok(latest);
                }
            }
        }
        Err(Error::DateNotFound("no treasury data available".into()))
    }

    /// Earliest date for which Treasury data is available remotely.
    ///
    /// Fetches `treasury-2000.parquet` and returns the minimum date found.
    /// Coverage in practice starts 2002-01-02.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for year 2000.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let d = client.treasury_earliest_date().await?;
    /// println!("Earliest treasury: {d}");
    /// # Ok(()) }
    /// ```
    pub async fn treasury_earliest_date(&self) -> Result<chrono::NaiveDate> {
        let curves = self.treasury_year(2000).await?;
        curves
            .into_iter()
            .map(|c| c.date)
            .min()
            .ok_or_else(|| Error::DateNotFound("no data in treasury-2000.parquet".into()))
    }

    // ── SOFR endpoints ────────────────────────────────────────────────────────

    /// Fetch the SOFR overnight rate (continuously compounded) for a single date.
    ///
    /// Accepts any date form — no `chrono` import required.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for the year.
    /// - `date` not found in the year file (weekend, holiday, or before
    ///   SOFR inception 2018-04-02).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let r = client.sofr("2026-04-14").await?;
    /// println!("SOFR: {r:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn sofr(&self, date: impl IntoDate) -> Result<f64> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let rates = self.sofr_year(year).await?;
        rates
            .into_iter()
            .find(|r| r.date == date.inner())
            .map(|r| r.rate)
            .ok_or_else(|| Error::DateNotFound(format!("no SOFR for {date}")))
    }

    /// Fetch all SOFR observations in `[start, end]` (inclusive).
    ///
    /// Fetches each calendar year in the span in parallel. Non-business days
    /// are absent from the result.
    ///
    /// # Errors
    ///
    /// - `start > end`.
    /// - Network failure for any year in the span with no cached file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let rates = client.sofr_range("2023-01-01", "2023-12-31").await?;
    /// println!("SOFR observations in 2023: {}", rates.len());
    /// # Ok(()) }
    /// ```
    pub async fn sofr_range(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<SofrDay>> {
        let start = start.into_date()?;
        let end = end.into_date()?;
        if start > end {
            return Err(Error::Other(format!(
                "sofr_range: start {start} > end {end}"
            )));
        }
        let start_nd = start.inner();
        let end_nd = end.inner();
        let years: Vec<i32> = (start_nd.year()..=end_nd.year()).collect();
        let fetches = years.iter().map(|&y| self.sofr_year(y)).collect::<Vec<_>>();
        let all_years = try_join_all(fetches).await?;
        let mut out: Vec<SofrDay> = all_years
            .into_iter()
            .flatten()
            .filter(|r| r.date >= start_nd && r.date <= end_nd)
            .collect();
        out.sort_by_key(|r| r.date);
        Ok(out)
    }

    /// Latest available SOFR observation.
    ///
    /// Fetches the current calendar year; falls back to the previous year if
    /// no data is present yet.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached files for both the current and previous year.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let sofr = client.sofr_latest().await?;
    /// println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
    /// # Ok(()) }
    /// ```
    pub async fn sofr_latest(&self) -> Result<SofrDay> {
        use chrono::Utc;
        let current_year = Utc::now().year();
        for year in [current_year, current_year - 1] {
            if let Ok(rates) = self.sofr_year(year).await {
                if let Some(latest) = rates.into_iter().max_by_key(|r| r.date) {
                    return Ok(latest);
                }
            }
        }
        Err(Error::DateNotFound("no SOFR data available".into()))
    }

    /// Earliest date for which SOFR data is available remotely.
    ///
    /// SOFR began 2018-04-02. Fetches `sofr-2018.parquet` and returns the
    /// minimum date found.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for year 2018.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let d = client.sofr_earliest_date().await?;
    /// println!("SOFR inception: {d}");
    /// # Ok(()) }
    /// ```
    pub async fn sofr_earliest_date(&self) -> Result<chrono::NaiveDate> {
        let rates = self.sofr_year(2018).await?;
        rates
            .into_iter()
            .map(|r| r.date)
            .min()
            .ok_or_else(|| Error::DateNotFound("no data in sofr-2018.parquet".into()))
    }

    // ── Blocking wrappers ─────────────────────────────────────────────────────

    /// Blocking variant of [`treasury_curve`][Self::treasury_curve].
    ///
    /// Works from both sync and async contexts:
    /// - Inside a tokio runtime: uses `block_in_place` + `Handle::block_on`.
    /// - Outside a runtime: spins up a single-threaded runtime for the call.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::{Curvekit, Tenor};
    ///
    /// // From synchronous code — no async needed
    /// let client = Curvekit::new();
    /// let curve = client.treasury_curve_blocking("2020-03-20")?;
    /// println!("10Y: {:.4}%", curve.get(Tenor::Y10).unwrap_or(0.0) * 100.0);
    /// # Ok::<(), curvekit::Error>(())
    /// ```
    pub fn treasury_curve_blocking(&self, date: impl IntoDate) -> Result<YieldCurve> {
        let date: Date = date.into_date()?;
        block(self.treasury_curve(date))
    }

    /// Blocking variant of [`treasury_range`][Self::treasury_range].
    pub fn treasury_range_blocking(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<YieldCurve>> {
        let start: Date = start.into_date()?;
        let end: Date = end.into_date()?;
        block(self.treasury_range(start, end))
    }

    /// Blocking variant of [`treasury_rate`][Self::treasury_rate].
    pub fn treasury_rate_blocking(
        &self,
        date: impl IntoDate,
        tenor: impl Into<Tenor>,
    ) -> Result<f64> {
        let date: Date = date.into_date()?;
        let tenor: Tenor = tenor.into();
        block(self.treasury_rate(date, tenor))
    }

    /// Blocking variant of [`treasury_latest`][Self::treasury_latest].
    pub fn treasury_latest_blocking(&self) -> Result<YieldCurve> {
        block(self.treasury_latest())
    }

    /// Blocking variant of [`sofr`][Self::sofr].
    pub fn sofr_blocking(&self, date: impl IntoDate) -> Result<f64> {
        let date: Date = date.into_date()?;
        block(self.sofr(date))
    }

    /// Blocking variant of [`sofr_range`][Self::sofr_range].
    pub fn sofr_range_blocking(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<SofrDay>> {
        let start: Date = start.into_date()?;
        let end: Date = end.into_date()?;
        block(self.sofr_range(start, end))
    }

    /// Blocking variant of [`sofr_latest`][Self::sofr_latest].
    pub fn sofr_latest_blocking(&self) -> Result<SofrDay> {
        block(self.sofr_latest())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Fetch and decode one full treasury year file.
    async fn treasury_year(&self, year: i32) -> Result<Vec<YieldCurve>> {
        let key = format!("treasury-{year}");
        let bytes = self.fetcher.fetch(&key).await?;
        let tmp = tempfile_for_bytes(&bytes, &format!("{key}.parquet"))?;
        let curves = read_treasury_year(tmp.path())?;
        Ok(curves)
    }

    /// Fetch and decode one full SOFR year file.
    async fn sofr_year(&self, year: i32) -> Result<Vec<SofrDay>> {
        let key = format!("sofr-{year}");
        let bytes = self.fetcher.fetch(&key).await?;
        let tmp = tempfile_for_bytes(&bytes, &format!("{key}.parquet"))?;
        let rates = read_sofr_year(tmp.path())?;
        Ok(rates)
    }
}

impl Default for Curvekit {
    fn default() -> Self {
        Self::new()
    }
}

// ── Date → TryInto wiring ─────────────────────────────────────────────────────
//
// All `TryFrom<T, Error = DateError> for Date` impls live in `date.rs`.
// The client methods use `impl IntoDate`, so any type
// that has `TryFrom<T, Error = DateError> for Date` works:
//   - `&str`, `String`  — parsed as ISO / slashed / compact
//   - `u32`             — interpreted as YYYYMMDD
//   - `(i32,u32,u32)`, `(u32,u32,u32)` — YMD components
//   - `NaiveDate`       — direct wrap
//   - `Date`            — identity
//
// `DateError` converts into `curvekit::Error` via `From<DateError> for Error`,
// so callers can use `?` freely in `-> Result<_, curvekit::Error>` functions.

// ── Blocking helper ───────────────────────────────────────────────────────────

/// Drive a future to completion from any context (sync or async).
///
/// - Inside a tokio multi-thread runtime: `block_in_place` + `Handle::block_on`
///   (avoids the "cannot call block_on inside async" panic).
/// - Outside any runtime: spin up a minimal current-thread runtime.
fn block<F: std::future::Future<Output = Result<T>>, T>(fut: F) -> Result<T> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(Error::Io)?;
            rt.block_on(fut)
        }
    }
}

/// Write bytes to a named temp file and return the [`tempfile::NamedTempFile`].
fn tempfile_for_bytes(bytes: &bytes::Bytes, _hint: &str) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    Ok(tmp)
}
