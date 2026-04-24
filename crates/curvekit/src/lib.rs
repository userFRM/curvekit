//! `curvekit` — US Treasury yield curve and SOFR overnight rate for Rust.
//!
//! Fetches parquet files on demand from GitHub raw, caches them locally with
//! ETag revalidation, and falls back to stale cache on network errors. No API
//! keys. Offline after the first successful fetch of each year file.
//!
//! # Quick start
//!
//! ```no_run
//! use curvekit::Curvekit;
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = Curvekit::new()?;
//!
//!     let curve = client
//!         .treasury_curve(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap())
//!         .await?;
//!     println!("10Y: {:.4}%", curve.get(3650).unwrap_or(0.0) * 100.0);
//!
//!     let sofr = client.sofr_latest().await?;
//!     println!("SOFR {}: {:.4}%", sofr.date, sofr.rate * 100.0);
//!
//!     Ok(())
//! }
//! ```
//!
//! # Major types
//!
//! - [`Curvekit`] — stateful client; create once, call many times.
//! - [`YieldCurve`] — Treasury yield curve for a single date, keyed by days
//!   to maturity. All rates are continuously compounded.
//! - [`SofrDay`] — a single SOFR overnight observation.
//! - [`TermStructure`] — combined Treasury + SOFR view for a date.
//! - [`Tenor`] — typed constants for standard maturity labels (`Tenor::Y10`, etc.).
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
//! - [`client`] — [`Curvekit`] async client.
//! - [`curve`] — [`YieldCurve`], [`SofrDay`], [`SofrRate`], [`TermStructure`], [`Tenor`].
//! - [`sources::bundled`] — synchronous reader from local parquet (CLI `get`).
//! - [`sources::parquet_io`] — parquet writer (CLI `backfill` / `append-today`).
//! - [`sources::treasury`] — Treasury CSV fetcher (used by CLI).
//! - [`sources::sofr`] — NY Fed SOFR CSV fetcher (used by CLI).
//! - [`interpolation`] — linear interpolation between tenor knots.
//! - [`error`] — typed error enums.

pub mod client;
pub mod curve;
pub mod error;
pub(crate) mod fetcher;
pub mod interpolation;
pub mod sources;

// Top-level re-exports.
pub use client::Curvekit;
pub use curve::{SofrDay, SofrRate, Tenor, TermStructure, YieldCurve, YieldCurveDay};
pub use error::{CurvekitError, Result};
pub use sources::bundled::{
    rate_for_days, sofr, sofr_latest_date, treasury_curve, treasury_latest_date,
};
pub use sources::sofr::{parse_sofr_csv, HttpSofrFetcher, SofrFetcher};
pub use sources::treasury::{parse_treasury_csv, HttpTreasuryFetcher, TreasuryFetcher};
