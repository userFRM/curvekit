//! Offline-first reader for bundled parquet files.
//!
//! # Data directory resolution (in priority order)
//!
//! 1. `$CURVEKIT_DATA_DIR` environment variable (absolute path).
//! 2. `<crate_source_root>/../../data/` — works for:
//!    - `cargo test` inside the workspace
//!    - git deps (cargo fetches the full repo, so `data/` is present)
//!    - `cargo install --path .`
//!
//! The `data/` directory lives at the repo root:
//! ```text
//! curvekit/
//!   crates/curvekit/   ← CARGO_MANIFEST_DIR points here
//!   data/              ← ../../data from manifest dir
//! ```
//!
//! # No network calls
//!
//! All functions in this module read from disk only. If a parquet file is
//! missing (e.g. on first clone before backfill), the functions return
//! `CurvekitError::DateNotFound`.

use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{CurvekitError, Result};
use crate::sources::parquet_io::{read_sofr_year, read_treasury_year};

// ---------------------------------------------------------------------------
// Data directory resolution
// ---------------------------------------------------------------------------

/// Resolve the `data/` directory.
///
/// Returns a `PathBuf` — but the directory may not exist yet (e.g. before first
/// backfill). Callers that need the dir to exist should check and surface a
/// friendly error.
fn data_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("CURVEKIT_DATA_DIR") {
        return PathBuf::from(env_dir);
    }
    // CARGO_MANIFEST_DIR is set by cargo at compile time for the crate that
    // defines the constant. At runtime for test binaries and examples it is
    // also available; for regular binaries built with `cargo build` the env
    // var is not set, so we fall back to the relative path anchored at the
    // executable. We bake the path at compile time for library consumers.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

// ---------------------------------------------------------------------------
// Reader API
// ---------------------------------------------------------------------------

/// Read the full yield curve for `date` from the bundled parquet.
///
/// Returns `CurvekitError::DateNotFound` if no curve exists for that date.
pub fn treasury_curve(date: NaiveDate) -> Result<crate::YieldCurve> {
    let year = date.year();
    let path = data_dir().join(format!("treasury-{year}.parquet"));

    if !path.exists() {
        return Err(CurvekitError::DateNotFound(format!(
            "treasury-{year}.parquet not found (run: curvekit backfill)"
        )));
    }

    let curves = read_treasury_year(&path)?;
    curves
        .into_iter()
        .find(|c| c.date == date)
        .ok_or_else(|| CurvekitError::DateNotFound(format!("no treasury curve for {date}")))
}

/// Read the SOFR rate for `date` from the bundled parquet.
///
/// Returns the continuously-compounded rate as `f64`.
pub fn sofr(date: NaiveDate) -> Result<f64> {
    let year = date.year();
    let path = data_dir().join(format!("sofr-{year}.parquet"));

    if !path.exists() {
        return Err(CurvekitError::DateNotFound(format!(
            "sofr-{year}.parquet not found (run: curvekit backfill)"
        )));
    }

    let rates = read_sofr_year(&path)?;
    rates
        .into_iter()
        .find(|r| r.date == date)
        .map(|r| r.rate)
        .ok_or_else(|| CurvekitError::DateNotFound(format!("no SOFR for {date}")))
}

/// Interpolated continuously-compounded rate at arbitrary `days` to maturity.
///
/// Reads the Treasury curve for `date` and linearly interpolates. Includes
/// flat extrapolation at the boundaries (same as [`crate::interpolation::linear`]).
pub fn rate_for_days(date: NaiveDate, days: u32) -> Result<f64> {
    let curve = treasury_curve(date)?;
    curve
        .get(days)
        .ok_or_else(|| CurvekitError::Interpolation(format!("empty curve for {date}")))
}

/// Latest date for which treasury data is bundled.
///
/// Scans all `treasury-{year}.parquet` files in the data dir and returns
/// the maximum date found across all files.
pub fn treasury_latest_date() -> NaiveDate {
    latest_date_for("treasury").unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 3).unwrap())
}

/// Latest date for which SOFR data is bundled.
pub fn sofr_latest_date() -> NaiveDate {
    latest_date_for("sofr").unwrap_or_else(|| NaiveDate::from_ymd_opt(2018, 4, 2).unwrap())
}

fn latest_date_for(prefix: &str) -> Option<NaiveDate> {
    let dir = data_dir();
    if !dir.exists() {
        return None;
    }

    let mut max_date: Option<NaiveDate> = None;

    let read_dir = std::fs::read_dir(&dir).ok()?;
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(prefix) || !name_str.ends_with(".parquet") {
            continue;
        }
        // Extract year from filename: "{prefix}-{year}.parquet"
        let year_str = name_str
            .strip_prefix(&format!("{prefix}-"))
            .and_then(|s| s.strip_suffix(".parquet"))?;
        let year: i32 = year_str.parse().ok()?;
        let path = dir.join(name.clone());

        let date = if prefix == "treasury" {
            read_treasury_year(&path)
                .ok()
                .and_then(|v| v.into_iter().map(|c| c.date).max())
        } else {
            read_sofr_year(&path)
                .ok()
                .and_then(|v| v.into_iter().map(|r| r.date).max())
        };

        // Prefer reading from file, but at minimum trust the year.
        let candidate = date.unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(year, 12, 31)
                .unwrap_or(NaiveDate::from_ymd_opt(year, 1, 1).unwrap())
        });
        max_date = Some(match max_date {
            Some(cur) if cur >= candidate => cur,
            _ => candidate,
        });
    }

    max_date
}

// ---------------------------------------------------------------------------
// `HashMap<u32,f64>` accessor for Kairos integration
// ---------------------------------------------------------------------------

/// Return the full continuous yield curve as `HashMap<days, rate>`.
///
/// Convenience wrapper over [`treasury_curve`] + [`crate::curve::YieldCurve::to_continuous_map`].
pub fn treasury_continuous_map(date: NaiveDate) -> Result<HashMap<u32, f64>> {
    Ok(treasury_curve(date)?.to_continuous_map())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The data directory may or may not exist in CI / dev without backfill.
    /// These tests are integration-level and pass when data is present.
    #[test]
    fn treasury_latest_date_does_not_panic() {
        let _ = treasury_latest_date();
    }

    #[test]
    fn sofr_latest_date_does_not_panic() {
        let _ = sofr_latest_date();
    }

    #[test]
    fn treasury_curve_missing_year_returns_err() {
        // Year 1800 will never have a parquet file.
        let date = NaiveDate::from_ymd_opt(1800, 1, 1).unwrap();
        let result = treasury_curve(date);
        assert!(result.is_err());
    }

    #[test]
    fn sofr_missing_year_returns_err() {
        let date = NaiveDate::from_ymd_opt(1800, 1, 1).unwrap();
        let result = sofr(date);
        assert!(result.is_err());
    }
}
