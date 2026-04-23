//! `curvekit-core` — risk-free rate fetchers, curve types, interpolation, and cache.
//!
//! This crate is the foundation of curvekit. It contains:
//!
//! - [`curve`] — [`YieldCurve`], [`SofrRate`], and [`TermStructure`] types.
//! - [`sources::treasury`] — US Treasury yield curve fetcher.
//! - [`sources::sofr`] — NY Fed SOFR fetcher.
//! - [`interpolation`] — linear and cubic-spline interpolation on yield curves.
//! - [`cache`] — SQLite persistence layer.
//! - [`error`] — typed error enums.

pub mod cache;
pub mod curve;
pub mod error;
pub mod interpolation;
pub mod sources;

// Re-exports for convenience.
pub use cache::RateCache;
pub use curve::{SofrRate, Tenor, TermStructure, YieldCurve};
pub use error::{CurvekitError, Result};
pub use sources::sofr::{parse_sofr_csv, HttpSofrFetcher, SofrFetcher};
pub use sources::treasury::{parse_treasury_csv, HttpTreasuryFetcher, TreasuryFetcher};
