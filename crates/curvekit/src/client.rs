//! Stateful `Curvekit` client — flat ThetaDataDx-style endpoint methods.
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
//!     let rate = client.sofr_latest().await?;
//!     println!("Latest SOFR: {:.4}%", rate.rate * 100.0);
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

/// Stateful curvekit client.
///
/// Wraps an ETag-aware [`CachedFetcher`] and exposes flat endpoint methods
/// named after what they return (ThetaDataDx pattern).
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
    /// Env overrides: `CURVEKIT_BASE_URL`, `CURVEKIT_CACHE_DIR`.
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

    /// Override the origin URL (default: `https://raw.githubusercontent.com/…`).
    ///
    /// Useful for pointing at a fork or a self-hosted mirror.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.fetcher.base_url = url.into();
        self
    }

    /// Override the on-disk cache directory.
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.fetcher.cache_dir = dir;
        self
    }

    // ── Treasury endpoints ────────────────────────────────────────────────────

    /// Fetch the full Treasury yield curve for a single date.
    ///
    /// Resolves the date to a year file (`treasury-{year}.parquet`), fetches +
    /// caches it, then filters to the requested date.
    pub async fn treasury_curve(&self, date: NaiveDate) -> Result<YieldCurve> {
        let year = date.year();
        let curves = self.treasury_year(year).await?;
        curves
            .into_iter()
            .find(|c| c.date == date)
            .ok_or_else(|| anyhow!("no treasury curve for {date}"))
    }

    /// Fetch all Treasury curves in `[start, end]` (inclusive).
    ///
    /// Determines the year span, fetches each year file in parallel, concatenates
    /// and filters to the requested range.
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

    /// Interpolated continuously-compounded rate at `days` to maturity for `date`.
    pub async fn treasury_rate(&self, date: NaiveDate, days: u32) -> Result<f64> {
        let curve = self.treasury_curve(date).await?;
        curve
            .get(days)
            .ok_or_else(|| anyhow!("no treasury data for {date} at {days}d"))
    }

    /// Latest available Treasury yield curve across all cached/remote year files.
    ///
    /// Fetches the current year; if no data is present yet (e.g. early in a new
    /// year before the first trading day), falls back to the previous year.
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

    /// Earliest date for which treasury data is available remotely.
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

    /// Fetch the SOFR overnight rate for a single date.
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
    /// SOFR began 2018-04-02.
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
