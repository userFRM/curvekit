//! Stateful `Curvekit` client — flat async endpoint methods.
//!
//! Fetches parquet files from GitHub raw (or a configurable origin) with an
//! XDG-compliant local cache + ETag revalidation. Falls back to stale cache on
//! network errors so existing workflows survive transient outages.
//!
//! # Example
//!
//! ```no_run
//! use curvekit::Curvekit;
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = Curvekit::new()?;
//!     let curve = client.treasury_latest().await?;
//!     println!("Latest treasury curve: {}", curve.date);
//!     let sofr = client.sofr_latest().await?;
//!     println!("Latest SOFR: {:.4}%", sofr.rate * 100.0);
//!     Ok(())
//! }
//! ```

use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};
use futures::future::try_join_all;
use std::path::PathBuf;

use crate::curve::{SofrDay, YieldCurve};
use crate::fetcher::{default_cache_dir, resolved_base_url, CachedFetcher};
use crate::sources::parquet_io::{read_sofr_year, read_treasury_year};
use crate::tenor::Tenor;

/// Stateful curvekit client.
///
/// Wraps an ETag-aware cached fetcher and exposes flat endpoint methods.
/// Create once and reuse across calls; the internal reqwest client is kept
/// alive for connection pooling.
///
/// # Builder
///
/// ```no_run
/// use curvekit::Curvekit;
/// use std::path::PathBuf;
///
/// let client = Curvekit::new()?
///     .with_base_url("https://my-mirror.example.com/curvekit")
///     .with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
/// # Ok::<(), anyhow::Error>(())
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
    /// # Errors
    ///
    /// Returns an error if the underlying reqwest client cannot be constructed
    /// (TLS init failure on unusual platforms; essentially never in practice).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    /// let client = Curvekit::new()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn new() -> Result<Self> {
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
    /// let client = Curvekit::new()?.with_base_url("https://my-mirror.example.com/curvekit");
    /// # Ok::<(), anyhow::Error>(())
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
    /// let client = Curvekit::new()?.with_cache_dir(PathBuf::from("/tmp/curvekit-test"));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.fetcher.cache_dir = dir;
        self
    }

    // ── Treasury endpoints ────────────────────────────────────────────────────

    /// Fetch the full US Treasury Par Yield Curve for a single date.
    ///
    /// Resolves the date to a year file (`treasury-{year}.parquet`), fetches
    /// and caches it (ETag revalidation), then filters to the requested date.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for the year.
    /// - `date` is not present in the year file (weekend, holiday, or outside
    ///   coverage 2002–present).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::{Curvekit, Tenor};
    /// # use chrono::NaiveDate;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let curve = client
    ///     .treasury_curve(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap())
    ///     .await?;
    /// println!("10Y: {:.4}%", curve.get(Tenor::Y10).unwrap_or(0.0) * 100.0);
    /// println!("3M:  {:.4}%", curve.get(Tenor::M3).unwrap_or(0.0) * 100.0);
    /// # Ok(()) }
    /// ```
    pub async fn treasury_curve(&self, date: NaiveDate) -> Result<YieldCurve> {
        let year = date.year();
        let curves = self.treasury_year(year).await?;
        curves
            .into_iter()
            .find(|c| c.date == date)
            .ok_or_else(|| anyhow!("no treasury curve for {date}"))
    }

    /// Fetch all Treasury yield curves in `[start, end]` (inclusive).
    ///
    /// Determines the year span, fetches each year file in parallel, then
    /// filters to the requested date range. Non-trading days are absent.
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
    /// # use chrono::NaiveDate;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let curves = client
    ///     .treasury_range(
    ///         NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
    ///         NaiveDate::from_ymd_opt(2020, 12, 31).unwrap(),
    ///     )
    ///     .await?;
    /// println!("Trading days in 2020: {}", curves.len());
    /// # Ok(()) }
    /// ```
    pub async fn treasury_range(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<YieldCurve>> {
        if start > end {
            return Err(anyhow!("treasury_range: start {start} > end {end}"));
        }
        let years: Vec<i32> = (start.year()..=end.year()).collect();
        let fetches = years
            .iter()
            .map(|&y| self.treasury_year(y))
            .collect::<Vec<_>>();
        let all_years = try_join_all(fetches).await?;
        let mut out: Vec<YieldCurve> = all_years
            .into_iter()
            .flatten()
            .filter(|c| c.date >= start && c.date <= end)
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
    /// Accepts any type that converts into [`Tenor`]: a named constant
    /// (`Tenor::Y10`), a constructed value (`Tenor::days(45)`), or a raw `u32`
    /// (backward-compatible).
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
    /// # use chrono::NaiveDate;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    ///
    /// // Named tenor
    /// let r_10y = client
    ///     .treasury_rate(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(), Tenor::Y10)
    ///     .await?;
    /// println!("10Y rate: {r_10y:.6}");
    ///
    /// // Ad-hoc tenor
    /// let r_45d = client
    ///     .treasury_rate(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(), Tenor::days(45))
    ///     .await?;
    /// println!("45d rate: {r_45d:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn treasury_rate(&self, date: NaiveDate, tenor: impl Into<Tenor>) -> Result<f64> {
        let tenor = tenor.into();
        let curve = self.treasury_curve(date).await?;
        curve
            .get(tenor)
            .ok_or_else(|| anyhow!("no treasury data for {date} at {}", tenor))
    }

    /// Latest available Treasury yield curve.
    ///
    /// Fetches the current calendar year; falls back to the previous year if
    /// no data is present yet (e.g. early January before the first trading day).
    ///
    /// # Errors
    ///
    /// - Network failure with no cached files for both the current and previous
    ///   year.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
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
        Err(anyhow!("no treasury data available"))
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
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let d = client.treasury_earliest_date().await?;
    /// println!("Earliest treasury: {d}");
    /// # Ok(()) }
    /// ```
    pub async fn treasury_earliest_date(&self) -> Result<NaiveDate> {
        // The repo starts from 2000; fetch that year and return the first date.
        let curves = self.treasury_year(2000).await?;
        curves
            .into_iter()
            .map(|c| c.date)
            .min()
            .ok_or_else(|| anyhow!("no data in treasury-2000.parquet"))
    }

    // ── SOFR endpoints ────────────────────────────────────────────────────────

    /// Fetch the SOFR overnight rate (continuously compounded) for a single date.
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
    /// # use chrono::NaiveDate;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let r = client.sofr(NaiveDate::from_ymd_opt(2026, 4, 14).unwrap()).await?;
    /// println!("SOFR: {r:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn sofr(&self, date: NaiveDate) -> Result<f64> {
        let year = date.year();
        let rates = self.sofr_year(year).await?;
        rates
            .into_iter()
            .find(|r| r.date == date)
            .map(|r| r.rate)
            .ok_or_else(|| anyhow!("no SOFR for {date}"))
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
    /// # use chrono::NaiveDate;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let rates = client
    ///     .sofr_range(
    ///         NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
    ///         NaiveDate::from_ymd_opt(2023, 12, 31).unwrap(),
    ///     )
    ///     .await?;
    /// println!("SOFR observations in 2023: {}", rates.len());
    /// # Ok(()) }
    /// ```
    pub async fn sofr_range(&self, start: NaiveDate, end: NaiveDate) -> Result<Vec<SofrDay>> {
        if start > end {
            return Err(anyhow!("sofr_range: start {start} > end {end}"));
        }
        let years: Vec<i32> = (start.year()..=end.year()).collect();
        let fetches = years.iter().map(|&y| self.sofr_year(y)).collect::<Vec<_>>();
        let all_years = try_join_all(fetches).await?;
        let mut out: Vec<SofrDay> = all_years
            .into_iter()
            .flatten()
            .filter(|r| r.date >= start && r.date <= end)
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
    /// - Network failure with no cached files for both the current and previous
    ///   year.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
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
        Err(anyhow!("no SOFR data available"))
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
    /// # async fn run() -> anyhow::Result<()> {
    /// let client = Curvekit::new()?;
    /// let d = client.sofr_earliest_date().await?;
    /// println!("SOFR inception: {d}");
    /// # Ok(()) }
    /// ```
    pub async fn sofr_earliest_date(&self) -> Result<NaiveDate> {
        let rates = self.sofr_year(2018).await?;
        rates
            .into_iter()
            .map(|r| r.date)
            .min()
            .ok_or_else(|| anyhow!("no data in sofr-2018.parquet"))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Fetch and decode one full treasury year file.
    async fn treasury_year(&self, year: i32) -> Result<Vec<YieldCurve>> {
        let key = format!("treasury-{year}");
        let bytes = self.fetcher.fetch(&key).await?;
        // write to a temp file for the parquet reader (which takes &Path)
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

/// Write bytes to a named temp file and return the [`tempfile::NamedTempFile`].
///
/// The file is kept open and deleted on drop. The parquet reader only needs the
/// path to be valid during the synchronous call, so this is safe.
fn tempfile_for_bytes(bytes: &bytes::Bytes, _hint: &str) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    Ok(tmp)
}
