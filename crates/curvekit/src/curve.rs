use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::interpolation;
pub use crate::tenor::Tenor;

/// A complete US Treasury yield curve for a single trading day.
///
/// `points` is keyed by **days to maturity** (using the [`Tenor`] approximations).
/// All yields are **continuously compounded** (converted from the published
/// Bond Equivalent Yields via `r_cont = ln(1 + APY)` where
/// `APY = (1 + BEY/200)^2 - 1`).
///
/// Missing maturities are simply absent from the map (not filled with NaN).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YieldCurve {
    /// Trading date this curve represents.
    pub date: NaiveDate,
    /// Continuously-compounded yields keyed by days to maturity.
    /// Keys are the standard `Tenor` values but exposed as plain `u32`
    /// for JSON-friendliness and flexible consumer arithmetic.
    pub points: BTreeMap<u32, f64>,
}

impl YieldCurve {
    /// Create a new, empty yield curve for the given date.
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            points: BTreeMap::new(),
        }
    }

    /// Insert a yield point. `days` is days to maturity; `rate` is
    /// continuously compounded.
    pub fn insert(&mut self, days: u32, rate: f64) {
        self.points.insert(days, rate);
    }

    /// Look up the yield at a tenor, or linearly interpolate between bracketing
    /// points. Returns `None` if the curve is empty.
    ///
    /// Accepts any type that converts into [`Tenor`]: a named constant
    /// (`Tenor::Y10`), a constructed value (`Tenor::days(45)`), or a raw `u32`
    /// (backward-compatible).
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::{Tenor, YieldCurve};
    /// use chrono::NaiveDate;
    ///
    /// let mut curve = YieldCurve::new(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    /// curve.insert(91, 0.015);
    /// curve.insert(3650, 0.025);
    ///
    /// // Named tenor
    /// assert!(curve.get(Tenor::Y10).is_some());
    ///
    /// // Ad-hoc tenor
    /// assert!(curve.get(Tenor::days(45)).is_some());
    ///
    /// // Raw u32 (backward-compatible)
    /// assert!(curve.get(3650_u32).is_some());
    /// ```
    pub fn get(&self, tenor: impl Into<Tenor>) -> Option<f64> {
        if self.points.is_empty() {
            return None;
        }
        interpolation::linear(&self.points, tenor.into().as_days())
    }

    /// Get the yield at a named tenor. Alias for [`get`][Self::get].
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::{Tenor, YieldCurve};
    /// use chrono::NaiveDate;
    ///
    /// let mut curve = YieldCurve::new(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    /// curve.insert(3650, 0.025);
    ///
    /// let r = curve.yield_at(Tenor::Y10).unwrap();
    /// assert!((r - 0.025).abs() < 1e-12);
    /// ```
    pub fn yield_at(&self, tenor: impl Into<Tenor>) -> Option<f64> {
        self.get(tenor)
    }

    /// Convenience: number of points in this curve.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Return a `HashMap<days, continuous_rate>` suitable for Kairos's
    /// `kairos_common::rates::FullRates::continuous_curve`.
    ///
    /// The rates stored in `points` are already continuously compounded
    /// (converted from BEY during CSV parse / parquet read), so this is
    /// a simple copy into the HashMap type Kairos expects.
    pub fn to_continuous_map(&self) -> HashMap<u32, f64> {
        self.points.iter().map(|(&k, &v)| (k, v)).collect()
    }
}

/// A single SOFR observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SofrRate {
    /// Date of the observation.
    pub date: NaiveDate,
    /// Continuously-compounded overnight rate (converted from the published
    /// percentage via `r = ln(1 + rate_pct / 100)`).
    pub rate: f64,
}

/// Combined view of the risk-free term structure: the Treasury yield curve
/// plus the SOFR overnight anchor.
///
/// SOFR is stored at the 1-day point (overnight) and exposed separately
/// so callers can distinguish source provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermStructure {
    pub date: NaiveDate,
    /// Treasury curve for this date (may be empty on non-trading days).
    pub treasury: YieldCurve,
    /// SOFR overnight rate (absent on non-business days).
    pub sofr: Option<SofrRate>,
}

impl TermStructure {
    /// Interpolate the continuously-compounded risk-free rate at `tenor`.
    /// SOFR (1-day) is included as an anchor point.
    ///
    /// Accepts any type that converts into [`Tenor`]: `Tenor::Y10`,
    /// `Tenor::days(45)`, or a raw `u32` (backward-compatible).
    ///
    /// Returns `None` if both the Treasury curve and SOFR are absent.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::{Tenor, YieldCurve, curve::{SofrRate, TermStructure}};
    /// use chrono::NaiveDate;
    ///
    /// let date = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
    /// let mut treasury = YieldCurve::new(date);
    /// treasury.insert(365, 0.04);
    /// let ts = TermStructure { date, treasury, sofr: None };
    ///
    /// let r = ts.rate_for(Tenor::Y1).unwrap();
    /// assert!((r - 0.04).abs() < 1e-12);
    /// ```
    pub fn rate_for(&self, tenor: impl Into<Tenor>) -> Option<f64> {
        let days = tenor.into().as_days();
        let mut points: BTreeMap<u32, f64> = self.treasury.points.clone();
        if let Some(sofr) = &self.sofr {
            points.insert(1, sofr.rate);
        }
        if points.is_empty() {
            return None;
        }
        interpolation::linear(&points, days)
    }

    /// Interpolate the continuously-compounded risk-free rate for `days`
    /// to maturity.
    ///
    /// # Deprecated
    ///
    /// Use [`rate_for`][Self::rate_for] with a [`Tenor`] value instead.
    /// This shim will be removed in the next major release.
    #[deprecated(
        since = "0.2.0",
        note = "use `rate_for(Tenor::days(d))` or `rate_for(Tenor::Y10)` instead"
    )]
    pub fn rate_for_days(&self, days: u32) -> Option<f64> {
        self.rate_for(days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
    }

    #[test]
    fn yield_curve_exact_lookup() {
        let mut c = YieldCurve::new(date());
        c.insert(365, 0.04);
        assert!((c.get(365).unwrap() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn yield_curve_interpolates() {
        let mut c = YieldCurve::new(date());
        c.insert(30, 0.04);
        c.insert(60, 0.05);
        // Midpoint
        let mid = c.get(45).unwrap();
        assert!((mid - 0.045).abs() < 1e-12);
    }

    #[test]
    fn yield_curve_empty_returns_none() {
        let c = YieldCurve::new(date());
        assert!(c.get(30).is_none());
    }

    #[test]
    fn term_structure_uses_sofr_anchor() {
        let date = date();
        let mut treasury = YieldCurve::new(date);
        treasury.insert(365, 0.04);
        let sofr = Some(SofrRate { date, rate: 0.05 });
        let ts = TermStructure {
            date,
            treasury,
            sofr,
        };
        // At 1 day → SOFR
        let r1 = ts.rate_for(Tenor::ON).unwrap();
        assert!((r1 - 0.05).abs() < 1e-12);
        // At 365 days → treasury
        let r365 = ts.rate_for(Tenor::Y1).unwrap();
        assert!((r365 - 0.04).abs() < 1e-12);
    }

    #[test]
    fn to_continuous_map_roundtrip() {
        let d = date();
        let mut c = YieldCurve::new(d);
        c.insert(365, 0.042);
        c.insert(3650, 0.048);
        let map = c.to_continuous_map();
        assert_eq!(map.len(), 2);
        assert!((map[&365] - 0.042).abs() < 1e-12);
        assert!((map[&3650] - 0.048).abs() < 1e-12);
    }
}

// ── Bulk types used by the parquet writer API ─────────────────────────────────

/// One day of Treasury curve data: a full [`YieldCurve`].
///
/// This is a type alias used by the bulk writer API for clarity.
pub type YieldCurveDay = YieldCurve;

/// One SOFR day record — used by the parquet writer API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SofrDay {
    pub date: NaiveDate,
    /// Continuously-compounded overnight rate.
    pub rate: f64,
}
