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
use crate::daycount::DayCount;
use crate::error::{Error, Result};
use crate::fetcher::{default_cache_dir, resolved_base_url, CachedFetcher};
use crate::sources::effr::EffrDay;
use crate::sources::obfr::ObfrDay;
use crate::sources::parquet_io::{
    read_effr_year, read_obfr_year, read_sofr_year, read_treasury_year,
};
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
            .user_agent("curvekit/1.0 (+https://github.com/userFRM/curvekit)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            fetcher: CachedFetcher::new(http, resolved_base_url(), default_cache_dir()),
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
            .user_agent("curvekit/1.0 (+https://github.com/userFRM/curvekit)")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            fetcher: CachedFetcher::new(http, resolved_base_url(), default_cache_dir()),
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
        self.fetcher.set_base_url(url.into());
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
        self.fetcher.set_cache_dir(dir);
        self
    }

    /// Override the CDN mirror URL used when the primary fetch fails.
    ///
    /// Default: jsDelivr CDN mirror of this repo. The mirror is activated
    /// only when the primary GitHub raw fetch exhausts its retry budget.
    ///
    /// - `Some(url)` — use a custom mirror (self-hosted, R2, Fastly, …).
    /// - `None` — disable mirror fallback entirely. A primary failure
    ///   returns the error directly. Useful in tests where you want to
    ///   observe primary-path behavior without the fallback masking it.
    ///
    /// Equivalent to `CURVEKIT_MIRROR_URL` env var. Builder form takes
    /// precedence — the env var is read only if this method is not called.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use curvekit::Curvekit;
    ///
    /// // Disable mirror entirely (e.g. in tests)
    /// let client = Curvekit::new().with_mirror_url(None);
    ///
    /// // Use a custom self-hosted mirror
    /// let client = Curvekit::new().with_mirror_url(Some("https://mirror.example.com/curvekit".into()));
    /// ```
    pub fn with_mirror_url(mut self, url: Option<String>) -> Self {
        self.fetcher.set_mirror_url(url);
        self
    }

    // ── Treasury endpoints ────────────────────────────────────────────────────

    /// Fetch the full US Treasury Par Yield Curve for a single date.
    ///
    /// Returns the curve exactly as published by Treasury — **par yields**
    /// (Bond Equivalent Yield, converted to continuously compounded). For
    /// discount-factor arithmetic you usually want
    /// [`treasury_zero_curve`][Self::treasury_zero_curve]
    /// instead, which bootstraps zero rates from the par curve.
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
    ///
    /// # See also
    ///
    /// - [`treasury_par_curve`][Self::treasury_par_curve] — explicit alias (preferred for clarity)
    /// - [`treasury_zero_curve`][Self::treasury_zero_curve] — bootstrapped zero/spot rates
    #[deprecated(
        since = "1.0.0",
        note = "use `treasury_par_curve` or `treasury_zero_curve` for explicit yield type"
    )]
    pub async fn treasury_curve(&self, date: impl IntoDate) -> Result<YieldCurve> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let curves = self.treasury_year(year).await?;
        curves
            .into_iter()
            .find(|c| c.date == date.inner())
            .ok_or_else(|| Error::DateNotFound(format!("no treasury curve for {date}")))
    }

    /// Fetch the US Treasury **par yield** curve for a single date.
    ///
    /// Returns the curve exactly as published — par yields (BEY, converted to
    /// continuously compounded). `yield_type` is [`YieldType::Par`][crate::YieldType::Par].
    ///
    /// Use this when you need the raw published data or want to perform your
    /// own curve math. For discount-factor work, prefer
    /// [`treasury_zero_curve`][Self::treasury_zero_curve].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let par = client.treasury_par_curve("2020-03-20").await?;
    /// println!("Par type: {:?}", par.yield_type);
    /// # Ok(()) }
    /// ```
    pub async fn treasury_par_curve(&self, date: impl IntoDate) -> Result<YieldCurve> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let curves = self.treasury_year(year).await?;
        curves
            .into_iter()
            .find(|c| c.date == date.inner())
            .ok_or_else(|| Error::DateNotFound(format!("no treasury curve for {date}")))
    }

    /// Fetch the US Treasury **zero-coupon (spot)** curve for a single date.
    ///
    /// Fetches the par curve and bootstraps zero rates using the iterative
    /// semi-annual coupon bootstrap. See [`YieldCurve::bootstrap_zero`] for
    /// the full algorithm description.
    ///
    /// `yield_type` is [`YieldType::Zero`][crate::YieldType::Zero].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let zero = client.treasury_zero_curve("2020-03-20").await?;
    /// let df_10y = (-zero.get(3650u32).unwrap() * 10.0).exp();
    /// println!("10Y discount factor: {df_10y:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn treasury_zero_curve(&self, date: impl IntoDate) -> Result<YieldCurve> {
        let par = self.treasury_par_curve(date).await?;
        par.bootstrap_zero()
    }

    /// Interpolated continuously-compounded par rate at `tenor` for `date`,
    /// using [`DayCount::Act365Fixed`].
    ///
    /// For a rate with an explicit day-count convention see
    /// [`treasury_rate_with_convention`][Self::treasury_rate_with_convention].
    ///
    /// The returned rate is a **par yield** (continuously compounded). For
    /// zero rates use [`treasury_zero_curve`][Self::treasury_zero_curve].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::{Curvekit, Tenor};
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let r = client.treasury_rate("2026-04-14", Tenor::Y10).await?;
    /// # Ok(()) }
    /// ```
    pub async fn treasury_rate(&self, date: impl IntoDate, tenor: impl Into<Tenor>) -> Result<f64> {
        let tenor = tenor.into();
        let curve = self.treasury_par_curve(date).await?;
        curve
            .get(tenor)
            .ok_or_else(|| Error::Interpolation(format!("no treasury data at {}", tenor)))
    }

    /// Interpolated rate at `tenor` for `date` scaled to the given day-count
    /// convention's year fraction.
    ///
    /// The raw rate in the curve is continuously compounded on an Act/365 basis
    /// (matching the Treasury's days-to-maturity indexing). This method
    /// multiplies by the ratio `Act365Fixed / convention` so that the returned
    /// rate is expressed in the requested convention.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::{Curvekit, Tenor, DayCount};
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let r_act360 = client
    ///     .treasury_rate_with_convention("2026-04-14", Tenor::M3, DayCount::Act360)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn treasury_rate_with_convention(
        &self,
        date: impl IntoDate,
        tenor: impl Into<Tenor>,
        convention: DayCount,
    ) -> Result<f64> {
        let date_val = date.into_date()?;
        let tenor = tenor.into();
        let curve = self.treasury_par_curve(date_val).await?;
        let r = curve
            .get(tenor)
            .ok_or_else(|| Error::Interpolation(format!("no treasury data at {}", tenor)))?;
        // r is Act365Fixed continuously compounded. Convert to the target convention
        // by scaling the year fraction: r_conv = r * (T_act365 / T_conv).
        let maturity = date_val
            .inner()
            .checked_add_signed(chrono::Duration::days(tenor.as_days() as i64))
            .ok_or_else(|| Error::Other("tenor overflow computing maturity date".into()))?;
        let t_act365 = DayCount::Act365Fixed.year_fraction(date_val.inner(), maturity);
        let t_conv = convention.year_fraction(date_val.inner(), maturity);
        if t_conv == 0.0 {
            return Ok(r);
        }
        Ok(r * t_act365 / t_conv)
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

    // ── EFFR endpoints ────────────────────────────────────────────────────────

    /// Fetch the Effective Federal Funds Rate for a single date.
    ///
    /// Returns the continuously-compounded overnight rate.
    ///
    /// # Data source
    ///
    /// Parquet files `data/effr-{year}.parquet` served from the curvekit data
    /// repository. Populated by `curvekit-cli backfill --source effr`.
    ///
    /// # Errors
    ///
    /// - Network failure with no cached file for the year.
    /// - `date` not found (weekend/holiday, or before EFFR coverage).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let r = client.effr("2026-04-14").await?;
    /// println!("EFFR: {r:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn effr(&self, date: impl IntoDate) -> Result<f64> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let rates = self.effr_year(year).await?;
        rates
            .into_iter()
            .find(|r| r.date == date.inner())
            .map(|r| r.rate)
            .ok_or_else(|| Error::DateNotFound(format!("no EFFR for {date}")))
    }

    /// Fetch all EFFR observations in `[start, end]` (inclusive).
    pub async fn effr_range(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<EffrDay>> {
        let start = start.into_date()?;
        let end = end.into_date()?;
        if start > end {
            return Err(Error::Other(format!(
                "effr_range: start {start} > end {end}"
            )));
        }
        let start_nd = start.inner();
        let end_nd = end.inner();
        let years: Vec<i32> = (start_nd.year()..=end_nd.year()).collect();
        let fetches = years.iter().map(|&y| self.effr_year(y)).collect::<Vec<_>>();
        let all_years = futures::future::try_join_all(fetches).await?;
        let mut out: Vec<EffrDay> = all_years
            .into_iter()
            .flatten()
            .filter(|r| r.date >= start_nd && r.date <= end_nd)
            .collect();
        out.sort_by_key(|r| r.date);
        Ok(out)
    }

    /// Latest available EFFR observation.
    pub async fn effr_latest(&self) -> Result<EffrDay> {
        use chrono::Utc;
        let current_year = Utc::now().year();
        for year in [current_year, current_year - 1] {
            if let Ok(rates) = self.effr_year(year).await {
                if let Some(latest) = rates.into_iter().max_by_key(|r| r.date) {
                    return Ok(latest);
                }
            }
        }
        Err(Error::DateNotFound("no EFFR data available".into()))
    }

    /// Earliest date for which EFFR data is available (fetches oldest year file).
    pub async fn effr_earliest_date(&self) -> Result<chrono::NaiveDate> {
        // EFFR history goes back to 1954; curvekit typically backfills from 2000.
        let rates = self.effr_year(2000).await?;
        rates
            .into_iter()
            .map(|r| r.date)
            .min()
            .ok_or_else(|| Error::DateNotFound("no data in effr-2000.parquet".into()))
    }

    // ── OBFR endpoints ────────────────────────────────────────────────────────

    /// Fetch the Overnight Bank Funding Rate for a single date.
    ///
    /// Coverage starts 2016-03-01. Returns the continuously-compounded rate.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use curvekit::Curvekit;
    /// # async fn run() -> curvekit::Result<()> {
    /// let client = Curvekit::new();
    /// let r = client.obfr("2026-04-14").await?;
    /// println!("OBFR: {r:.6}");
    /// # Ok(()) }
    /// ```
    pub async fn obfr(&self, date: impl IntoDate) -> Result<f64> {
        let date = date.into_date()?;
        let year = date.inner().year();
        let rates = self.obfr_year(year).await?;
        rates
            .into_iter()
            .find(|r| r.date == date.inner())
            .map(|r| r.rate)
            .ok_or_else(|| Error::DateNotFound(format!("no OBFR for {date}")))
    }

    /// Fetch all OBFR observations in `[start, end]` (inclusive).
    pub async fn obfr_range(
        &self,
        start: impl IntoDate,
        end: impl IntoDate,
    ) -> Result<Vec<ObfrDay>> {
        let start = start.into_date()?;
        let end = end.into_date()?;
        if start > end {
            return Err(Error::Other(format!(
                "obfr_range: start {start} > end {end}"
            )));
        }
        let start_nd = start.inner();
        let end_nd = end.inner();
        let years: Vec<i32> = (start_nd.year()..=end_nd.year()).collect();
        let fetches = years.iter().map(|&y| self.obfr_year(y)).collect::<Vec<_>>();
        let all_years = futures::future::try_join_all(fetches).await?;
        let mut out: Vec<ObfrDay> = all_years
            .into_iter()
            .flatten()
            .filter(|r| r.date >= start_nd && r.date <= end_nd)
            .collect();
        out.sort_by_key(|r| r.date);
        Ok(out)
    }

    /// Latest available OBFR observation.
    pub async fn obfr_latest(&self) -> Result<ObfrDay> {
        use chrono::Utc;
        let current_year = Utc::now().year();
        for year in [current_year, current_year - 1] {
            if let Ok(rates) = self.obfr_year(year).await {
                if let Some(latest) = rates.into_iter().max_by_key(|r| r.date) {
                    return Ok(latest);
                }
            }
        }
        Err(Error::DateNotFound("no OBFR data available".into()))
    }

    /// Earliest date for which OBFR data is available (2016-03-01).
    pub async fn obfr_earliest_date(&self) -> Result<chrono::NaiveDate> {
        let rates = self.obfr_year(2016).await?;
        rates
            .into_iter()
            .map(|r| r.date)
            .min()
            .ok_or_else(|| Error::DateNotFound("no data in obfr-2016.parquet".into()))
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
        block(self.treasury_par_curve(date))
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

    /// Fetch and decode one full EFFR year file.
    async fn effr_year(&self, year: i32) -> Result<Vec<EffrDay>> {
        let key = format!("effr-{year}");
        let bytes = self.fetcher.fetch(&key).await?;
        let tmp = tempfile_for_bytes(&bytes, &format!("{key}.parquet"))?;
        let rates = read_effr_year(tmp.path())?;
        Ok(rates)
    }

    /// Fetch and decode one full OBFR year file.
    async fn obfr_year(&self, year: i32) -> Result<Vec<ObfrDay>> {
        let key = format!("obfr-{year}");
        let bytes = self.fetcher.fetch(&key).await?;
        let tmp = tempfile_for_bytes(&bytes, &format!("{key}.parquet"))?;
        let rates = read_obfr_year(tmp.path())?;
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
