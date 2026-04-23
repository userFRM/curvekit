//! `curvekit` — offline-first bundled-parquet risk-free rate library.
//!
//! Parquet files ship inside the repo under `data/` and are read at runtime
//! without any network calls. The nightly GitHub Actions workflow appends
//! yesterday's Treasury curve and today's SOFR to the current-year file.
//!
//! # Modules
//!
//! - [`curve`] — [`YieldCurve`], [`SofrRate`], [`TermStructure`], [`YieldCurveDay`], [`SofrDay`].
//! - [`sources::bundled`] — parquet reader API (offline-first, no network).
//! - [`sources::parquet_io`] — parquet writer API (used by CLI backfill/append).
//! - [`sources::treasury`] — US Treasury CSV fetcher (used by CLI, not by lib consumers).
//! - [`sources::sofr`] — NY Fed SOFR CSV fetcher (used by CLI, not by lib consumers).
//! - [`interpolation`] — linear and monotone cubic-spline interpolation.
//! - [`error`] — typed error enums.

pub mod curve;
pub mod error;
pub mod interpolation;
pub mod sources;

// Top-level re-exports: the reader API is the primary public surface.
pub use curve::{SofrDay, SofrRate, Tenor, TermStructure, YieldCurve, YieldCurveDay};
pub use error::{CurvekitError, Result};
pub use sources::bundled::{
    rate_for_days, sofr, sofr_latest_date, treasury_curve, treasury_latest_date,
};
pub use sources::sofr::{parse_sofr_csv, HttpSofrFetcher, SofrFetcher};
pub use sources::treasury::{parse_treasury_csv, HttpTreasuryFetcher, TreasuryFetcher};
