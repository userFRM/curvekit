//! `curvekit` — risk-free rate library: Treasury yield curves + SOFR.
//!
//! # Two usage modes
//!
//! ## 1. Async client (recommended for applications)
//!
//! [`Curvekit`] fetches parquet files from GitHub raw on demand, caches them
//! locally with ETag revalidation, and falls back to stale cache on network
//! errors.
//!
//! ```no_run
//! use curvekit::Curvekit;
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = Curvekit::new()?;
//!     let curve = client.treasury_latest().await?;
//!     let sofr  = client.sofr_latest().await?;
//!     println!("{} — 10Y: {:.4}%  SOFR: {:.4}%",
//!              curve.date,
//!              curve.get(3650).unwrap_or(0.0) * 100.0,
//!              sofr.rate * 100.0);
//!     Ok(())
//! }
//! ```
//!
//! ## 2. Offline bundled reader (for CLI / data-pipeline tools)
//!
//! [`sources::bundled`] reads from local parquet files populated by
//! `curvekit-cli backfill`. No network calls; returns
//! `CurvekitError::DateNotFound` if the file is absent.
//!
//! # Modules
//!
//! - [`client`] — [`Curvekit`] async client with GitHub raw backend.
//! - [`fetcher`] — ETag-aware [`CachedFetcher`] internals (pub(crate)).
//! - [`curve`] — [`YieldCurve`], [`SofrRate`], [`TermStructure`], etc.
//! - [`sources::bundled`] — offline parquet reader.
//! - [`sources::parquet_io`] — parquet writer (used by CLI).
//! - [`sources::treasury`] — US Treasury CSV fetcher (used by CLI).
//! - [`sources::sofr`] — NY Fed SOFR CSV fetcher (used by CLI).
//! - [`interpolation`] — linear interpolation.
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
