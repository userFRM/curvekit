use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::interpolation;

/// Standard tenor labels aligned to US Treasury maturities.
///
/// Days are approximate calendar days (months × 30, years × 365).
/// This is the conventional mapping used for yield-curve math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tenor(pub u32);

impl Tenor {
    pub const M1: Tenor = Tenor(30);
    pub const M2: Tenor = Tenor(60);
    pub const M3: Tenor = Tenor(91);
    pub const M6: Tenor = Tenor(182);
    pub const Y1: Tenor = Tenor(365);
    pub const Y2: Tenor = Tenor(730);
    pub const Y3: Tenor = Tenor(1095);
    pub const Y5: Tenor = Tenor(1825);
    pub const Y7: Tenor = Tenor(2555);
    pub const Y10: Tenor = Tenor(3650);
    pub const Y20: Tenor = Tenor(7300);
    pub const Y30: Tenor = Tenor(10950);

    /// Days to maturity.
    #[inline]
    pub fn days(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for Tenor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            30 => write!(f, "1M"),
            60 => write!(f, "2M"),
            91 => write!(f, "3M"),
            182 => write!(f, "6M"),
            365 => write!(f, "1Y"),
            730 => write!(f, "2Y"),
            1095 => write!(f, "3Y"),
            1825 => write!(f, "5Y"),
            2555 => write!(f, "7Y"),
            3650 => write!(f, "10Y"),
            7300 => write!(f, "20Y"),
            10950 => write!(f, "30Y"),
            d => write!(f, "{}d", d),
        }
    }
}

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

    /// Look up the yield for an exact number of days, or linearly interpolate
    /// between bracketing points. Returns `None` if the curve is empty.
    pub fn get(&self, days: u32) -> Option<f64> {
        if self.points.is_empty() {
            return None;
        }
        interpolation::linear(&self.points, days)
    }

    /// Convenience: number of points in this curve.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
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
    /// Interpolate the continuously-compounded risk-free rate for `days`
    /// to maturity. SOFR (1-day) is included as an anchor point.
    ///
    /// Returns `None` if both the Treasury curve and SOFR are absent.
    pub fn rate_for_days(&self, days: u32) -> Option<f64> {
        let mut points: BTreeMap<u32, f64> = self.treasury.points.clone();
        if let Some(sofr) = &self.sofr {
            points.insert(1, sofr.rate);
        }
        if points.is_empty() {
            return None;
        }
        interpolation::linear(&points, days)
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
        let r1 = ts.rate_for_days(1).unwrap();
        assert!((r1 - 0.05).abs() < 1e-12);
        // At 365 days → treasury
        let r365 = ts.rate_for_days(365).unwrap();
        assert!((r365 - 0.04).abs() < 1e-12);
    }
}
