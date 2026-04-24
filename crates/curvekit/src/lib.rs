//! `curvekit` — US Treasury yield curve and SOFR overnight rate for Rust.
//!
//! Fetches parquet files on demand from GitHub raw, caches them locally with
//! ETag revalidation, and falls back to stale cache on network errors. No API
//! keys. Offline after the first successful fetch of each year file.
//!
//! # Quick start — one-off scripts
//!
//! ```no_run
//! use curvekit::Tenor;
//!
//! #[tokio::main]
//! async fn main() -> curvekit::Result<()> {
//!     // Free functions — no client setup needed
//!     let curve = curvekit::treasury_curve_for("2020-03-20").await?;
//!     let r     = curvekit::treasury_rate_at("2020-03-20", Tenor::Y10).await?;
//!     let today = curvekit::treasury_today().await?;
//!     let sofr  = curvekit::sofr_today().await?;
//!
//!     println!("10Y on 2020-03-20: {r:.6}");
//!     println!("Latest Treasury:   {}", today.date);
//!     println!("Latest SOFR:       {:.4}%", sofr.rate * 100.0);
//!     Ok(())
//! }
//! ```
//!
//! # Client pattern — connection pool + cache reuse
//!
//! ```no_run
//! use curvekit::{Curvekit, Date, Tenor};
//!
//! #[tokio::main]
//! async fn main() -> curvekit::Result<()> {
//!     let client = Curvekit::new();   // infallible, no ?
//!
//!     // Any date form — ISO string, compact u32, tuple, NaiveDate
//!     let curve = client.treasury_curve("2020-03-20").await?;
//!     let curve = client.treasury_curve(20200320u32).await?;
//!     let curve = client.treasury_curve((2020i32, 3u32, 20u32)).await?;
//!     let curve = client.treasury_curve(Date::today_et()).await?;
//!
//!     let r = client.treasury_rate("2020-03-20", Tenor::Y10).await?;
//!
//!     // Blocking from sync code — no async runtime needed
//!     let curve = client.treasury_curve_blocking(20200320u32)?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Major types
//!
//! - [`Curvekit`] — stateful client; create once, call many times.
//! - [`Date`] — ergonomic date wrapper; accepts strings, integers, tuples.
//! - [`YieldCurve`] — Treasury yield curve for a single date. All rates are
//!   continuously compounded.
//! - [`SofrDay`] — a single SOFR overnight observation.
//! - [`TermStructure`] — combined Treasury + SOFR view for a date.
//! - [`Tenor`] — typed constants for standard maturity labels (`Tenor::Y10`, etc.).
//! - [`Error`] — unified error type; match on this, never on sub-types.
//!
//! # Environment overrides
//!
//! | Variable | Effect |
//! |---|---|
//! | `CURVEKIT_BASE_URL` | Replace the GitHub raw origin URL |
//! | `CURVEKIT_CACHE_DIR` | Override `~/.cache/curvekit/` |
//!
//! # Modules
//!
//! - [`client`] — [`Curvekit`] async client with blocking wrappers.
//! - [`date`] — [`Date`] newtype for ergonomic date input.
//! - [`tenor`] — [`Tenor`] semantic type for maturities.
//! - [`curve`] — [`YieldCurve`], [`SofrDay`], [`SofrRate`], [`TermStructure`].
//! - [`sources::bundled`] — synchronous reader from local parquet (CLI `get`).
//! - [`sources::parquet_io`] — parquet writer (CLI `backfill` / `append-today`).
//! - [`sources::treasury`] — Treasury CSV fetcher (used by CLI).
//! - [`sources::sofr`] — NY Fed SOFR CSV fetcher (used by CLI).
//! - [`interpolation`] — linear interpolation between tenor knots.
//! - [`error`] — unified [`Error`] enum and [`Result`] alias.

pub mod client;
pub mod curve;
pub mod date;
pub mod error;
pub(crate) mod fetcher;
pub mod interpolation;
pub mod sources;
pub mod tenor;

// ── Top-level re-exports ──────────────────────────────────────────────────────

pub use client::Curvekit;
pub use curve::{SofrDay, SofrRate, TermStructure, YieldCurve, YieldCurveDay};
pub use date::{Date, DateError, IntoDate};
pub use error::{CurvekitError, Error, Result};
pub use sources::sofr::{parse_sofr_csv, HttpSofrFetcher, SofrFetcher};
pub use sources::treasury::{parse_treasury_csv, HttpTreasuryFetcher, TreasuryFetcher};
pub use tenor::Tenor;

// Legacy bundled API re-exports (kept for backward compat, deprecated at source).
#[allow(deprecated)]
pub use sources::bundled::{
    rate_for, rate_for_days, sofr, sofr_latest_date, treasury_curve, treasury_latest_date,
};

// ── Free-function shortcuts ───────────────────────────────────────────────────
//
// Each function internally uses a process-wide `Curvekit` instance so that
// multiple calls share one HTTP client and cache.

use std::sync::OnceLock;

fn global_client() -> &'static Curvekit {
    static CLIENT: OnceLock<Curvekit> = OnceLock::new();
    CLIENT.get_or_init(Curvekit::new)
}

/// Latest available Treasury yield curve (uses shared global client).
///
/// # Example
///
/// ```no_run
/// #[tokio::main]
/// async fn main() -> curvekit::Result<()> {
///     let curve = curvekit::treasury_today().await?;
///     println!("Latest Treasury: {}", curve.date);
///     Ok(())
/// }
/// ```
pub async fn treasury_today() -> Result<YieldCurve> {
    global_client().treasury_latest().await
}

/// Full Treasury yield curve for a specific date (uses shared global client).
///
/// Accepts any date form: ISO string, compact u32, tuple, or `NaiveDate`.
///
/// # Example
///
/// ```no_run
/// #[tokio::main]
/// async fn main() -> curvekit::Result<()> {
///     let curve = curvekit::treasury_curve_for("2020-03-20").await?;
///     println!("3M: {:.4}%", curve.get(curvekit::Tenor::M3).unwrap_or(0.0) * 100.0);
///     Ok(())
/// }
/// ```
pub async fn treasury_curve_for(date: impl IntoDate) -> Result<YieldCurve> {
    global_client().treasury_curve(date).await
}

/// Interpolated Treasury rate at a specific tenor for a date (uses shared global client).
///
/// # Example
///
/// ```no_run
/// use curvekit::Tenor;
///
/// #[tokio::main]
/// async fn main() -> curvekit::Result<()> {
///     let r = curvekit::treasury_rate_at("2020-03-20", Tenor::Y10).await?;
///     println!("10Y on 2020-03-20: {r:.6}");
///     Ok(())
/// }
/// ```
pub async fn treasury_rate_at(date: impl IntoDate, tenor: impl Into<Tenor>) -> Result<f64> {
    global_client().treasury_rate(date, tenor).await
}

/// Latest available SOFR overnight observation (uses shared global client).
///
/// # Example
///
/// ```no_run
/// #[tokio::main]
/// async fn main() -> curvekit::Result<()> {
///     let sofr = curvekit::sofr_today().await?;
///     println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
///     Ok(())
/// }
/// ```
pub async fn sofr_today() -> Result<SofrDay> {
    global_client().sofr_latest().await
}
