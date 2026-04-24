use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::interpolation;
pub use crate::tenor::Tenor;

/// Whether a [`YieldCurve`] contains par yields or zero (spot) rates.
///
/// # Par yields
///
/// The yields published daily by the US Treasury are **par yields** — the
/// coupon rate that makes a theoretical bond priced at par (100). These are
/// quoted as Bond Equivalent Yields (BEY, semi-annual) and converted to
/// continuously compounded rates when stored in `points`.
///
/// # Zero rates
///
/// A zero-coupon (spot) rate is the yield on a single cash flow at time T.
/// For discount-factor arithmetic (`DF(T) = exp(-z(T) * T)`) you need zero
/// rates, not par rates. For flat curves the two are equal; on non-flat
/// curves they differ, sometimes materially.
///
/// Obtain a zero curve via [`YieldCurve::bootstrap_zero`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YieldType {
    /// Par yield — as published by the US Treasury (BEY, converted to
    /// continuously compounded on ingest). This is the default.
    Par,
    /// Zero-coupon (spot) rate — bootstrapped from par yields.
    Zero,
}

/// A complete US Treasury yield curve for a single trading day.
///
/// `points` is keyed by **days to maturity** (using the [`Tenor`] approximations).
/// All yields are **continuously compounded** (converted from the published
/// Bond Equivalent Yields via `r_cont = ln(1 + APY)` where
/// `APY = (1 + BEY/200)^2 - 1`).
///
/// Missing maturities are simply absent from the map (not filled with NaN).
///
/// # Par vs zero
///
/// By default curves loaded from parquet carry [`YieldType::Par`] rates —
/// exactly as published by Treasury. Call [`bootstrap_zero`][Self::bootstrap_zero]
/// to convert to spot rates. See [`YieldType`] for the conceptual difference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YieldCurve {
    /// Trading date this curve represents.
    pub date: NaiveDate,
    /// Whether `points` contain par yields or zero rates.
    /// Defaults to [`YieldType::Par`].
    pub yield_type: YieldType,
    /// Continuously-compounded yields keyed by days to maturity.
    /// Keys are the standard `Tenor` values but exposed as plain `u32`
    /// for JSON-friendliness and flexible consumer arithmetic.
    pub points: BTreeMap<u32, f64>,
}

impl YieldCurve {
    /// Create a new, empty par yield curve for the given date.
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            yield_type: YieldType::Par,
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

    /// Bootstrap a zero-coupon (spot) curve from this par yield curve.
    ///
    /// # Background
    ///
    /// Treasury par yields are BEY-quoted coupon rates that price a bond at
    /// par. For discount-factor arithmetic you need zero rates `z(T)` such
    /// that `DF(T) = exp(-z(T) * T_years)`.
    ///
    /// # Algorithm
    ///
    /// **Short end (≤ 365 days):** money-market tenors carry no interim
    /// coupons, so par rate = zero rate exactly.
    ///
    /// **Long end (> 365 days):** semi-annual coupon bonds. At each tenor `T`
    /// with semi-annual par rate `c = par_bey / 2` and already-known discount
    /// factors `DF(t)` for all `t < T`:
    ///
    /// ```text
    ///   100 = sum_{i=1}^{2T-1} (c * 100) * DF(i/2)  +  (100 + c*100) * DF(T)
    /// ```
    ///
    /// Solving:
    ///
    /// ```text
    ///   DF(T) = (100 - c * sum_{i=1}^{2T-1} DF(i/2) * 100) / (100 + c*100)
    ///         = (1 - c * sum_{i=1}^{2T-1} DF(i/2)) / (1 + c)
    /// ```
    ///
    /// Intermediate coupon DFs are interpolated on the zero curve built so
    /// far (linear interpolation in `z × T` log space for the DF).
    ///
    /// Zero rate from DF: `z(T) = -ln(DF(T)) / T_years`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - The curve is already a zero curve (`yield_type == YieldType::Zero`).
    /// - A computed discount factor is not positive (degenerate / inverted
    ///   curve that makes the bootstrap numerically unstable).
    ///
    /// # Example
    ///
    /// ```
    /// use curvekit::YieldCurve;
    /// use chrono::NaiveDate;
    ///
    /// // Flat par curve at 5% continuously compounded
    /// let mut par = YieldCurve::new(NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    /// for &days in &[30u32, 91, 182, 365, 730, 1095, 1825, 2555, 3650] {
    ///     par.insert(days, 0.05);
    /// }
    ///
    /// let zero = par.bootstrap_zero().unwrap();
    /// // For a flat par curve the zero rates are approximately equal to the
    /// // par rates (within 1 bp rounding from BEY conversion).
    /// let z10 = zero.get(3650u32).unwrap();
    /// assert!((z10 - 0.05).abs() < 5e-4, "flat par≈zero: {z10:.6} ≠ 0.05");
    /// ```
    pub fn bootstrap_zero(&self) -> crate::error::Result<YieldCurve> {
        use crate::error::Error;

        if self.yield_type == YieldType::Zero {
            return Err(Error::Other(
                "bootstrap_zero called on an already-zero curve".into(),
            ));
        }

        if self.points.is_empty() {
            return Ok(YieldCurve {
                date: self.date,
                yield_type: YieldType::Zero,
                points: BTreeMap::new(),
            });
        }

        // We work in (tenor_days → continuously_compounded_zero_rate).
        // Build the result incrementally; short-end knots copy directly.
        let mut zero_points: BTreeMap<u32, f64> = BTreeMap::new();

        for (&days, &par_cont) in &self.points {
            let t_years = days as f64 / 365.0;

            if days <= 365 {
                // Short end: par rate == zero rate.
                zero_points.insert(days, par_cont);
            } else {
                // Long end: bootstrap.
                // par_cont is the continuously compounded par rate.
                // Convert back to semi-annual BEY for coupon calculation:
                //   BEY = 2 * (exp(par_cont/2) - 1)
                let par_cont_semi = par_cont / 2.0; // six-month continuous rate
                let bey_half = par_cont_semi.exp() - 1.0; // semi-annual yield (decimal)
                let c = bey_half; // coupon per $1 face per semi period

                // Number of full semi-annual periods.
                let n_semi = (t_years * 2.0).round() as u32;

                // Sum of DF(i/2) for i = 1 .. n_semi-1 (intermediate coupons).
                let mut coupon_df_sum = 0.0_f64;
                for i in 1..n_semi {
                    let coupon_days = (i as f64 / 2.0 * 365.0).round() as u32;
                    let z_coupon = df_from_zero_points(&zero_points, coupon_days);
                    coupon_df_sum += z_coupon;
                }

                // DF(T) = (1 - c * coupon_sum) / (1 + c)
                let numerator = 1.0 - c * coupon_df_sum;
                let denominator = 1.0 + c;

                if denominator <= 0.0 || numerator <= 0.0 {
                    return Err(Error::Other(format!(
                        "bootstrap_zero: non-positive discount factor at {days}d \
                         (numerator={numerator:.6}, denominator={denominator:.6}). \
                         Curve may be severely inverted or data is degenerate."
                    )));
                }

                let df_t = numerator / denominator;

                // Zero rate: z(T) = -ln(DF(T)) / T_years
                let z_t = -df_t.ln() / t_years;
                zero_points.insert(days, z_t);
            }
        }

        Ok(YieldCurve {
            date: self.date,
            yield_type: YieldType::Zero,
            points: zero_points,
        })
    }
}

/// Interpolate a discount factor from a partially-built zero curve.
///
/// For tenors within the built range: linear interpolation in zero-rate space.
/// For tenors below the shortest knot: flat extrapolation (use shortest rate).
/// For tenors above the longest knot: flat extrapolation (use longest rate).
fn df_from_zero_points(zero_points: &BTreeMap<u32, f64>, days: u32) -> f64 {
    if zero_points.is_empty() {
        // No zero curve yet — use zero rate = 0 → DF = 1.
        return 1.0;
    }
    let z = interpolation::linear(zero_points, days).unwrap_or(0.0);
    let t_years = days as f64 / 365.0;
    (-z * t_years).exp()
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
    /// The curve carries [`YieldType::Par`] by default. Call
    /// [`YieldCurve::bootstrap_zero`] on the treasury field to convert.
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
    /// **Note:** this method returns the rate from the `treasury` curve as-is.
    /// If the treasury curve is a par curve (the default), the returned rate
    /// is a par yield, not a zero rate. For discount-factor math, first call
    /// `self.treasury.bootstrap_zero()` and build a `TermStructure` with the
    /// zero curve.
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
    fn yield_curve_defaults_to_par_type() {
        let c = YieldCurve::new(date());
        assert_eq!(c.yield_type, YieldType::Par);
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

    // ── Bootstrap zero curve tests ──────────────────────────────────────────

    fn flat_par_curve(rate: f64) -> YieldCurve {
        let d = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
        let mut c = YieldCurve::new(d);
        // Standard Treasury knots (days to maturity)
        for &days in &[
            30u32, 91, 182, 365, 730, 1095, 1825, 2555, 3650, 7300, 10950,
        ] {
            c.insert(days, rate);
        }
        c
    }

    fn steep_par_curve() -> YieldCurve {
        // Upward sloping: short end 0.5%, 30Y at 4.5%
        let d = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
        let mut c = YieldCurve::new(d);
        c.insert(30, 0.005);
        c.insert(91, 0.007);
        c.insert(182, 0.010);
        c.insert(365, 0.015);
        c.insert(730, 0.020);
        c.insert(1095, 0.025);
        c.insert(1825, 0.030);
        c.insert(2555, 0.035);
        c.insert(3650, 0.040);
        c.insert(7300, 0.043);
        c.insert(10950, 0.045);
        c
    }

    fn inverted_par_curve() -> YieldCurve {
        let d = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
        let mut c = YieldCurve::new(d);
        c.insert(30, 0.055);
        c.insert(91, 0.054);
        c.insert(182, 0.052);
        c.insert(365, 0.050);
        c.insert(730, 0.048);
        c.insert(1095, 0.046);
        c.insert(1825, 0.044);
        c.insert(2555, 0.042);
        c.insert(3650, 0.040);
        c.insert(7300, 0.038);
        c.insert(10950, 0.036);
        c
    }

    /// For a flat curve, zero rates ≈ par rates (within a small BEY ↔ continuous
    /// conversion rounding). Tolerance is generous at 5 bp.
    #[test]
    fn bootstrap_flat_curve_zero_approx_par() {
        let par = flat_par_curve(0.05);
        let zero = par.bootstrap_zero().unwrap();
        assert_eq!(zero.yield_type, YieldType::Zero);

        for (&days, &par_rate) in &par.points {
            let z = zero.get(days).unwrap();
            assert!(
                (z - par_rate).abs() < 5e-4,
                "flat: days={days}, par={par_rate:.6}, zero={z:.6}, diff={:.6}",
                (z - par_rate).abs()
            );
        }
    }

    /// For a steep upward-sloping curve, zero rates should be >= par rates
    /// at each tenor (the "bootstrapped zeros lie above par" property of
    /// normally-shaped curves).
    #[test]
    fn bootstrap_steep_curve_zeros_above_par() {
        let par = steep_par_curve();
        let zero = par.bootstrap_zero().unwrap();

        // Long end: zero > par (standard relationship for upward-sloping curves)
        for &days in &[730u32, 1095, 1825, 2555, 3650] {
            let p = par.points[&days];
            let z = zero.get(days).unwrap();
            assert!(
                z >= p - 1e-9,
                "steep: days={days}, zero={z:.6} should be ≥ par={p:.6}"
            );
        }
    }

    /// Inverted curve: zero rates should be <= par rates at long end.
    #[test]
    fn bootstrap_inverted_curve_zeros_below_par() {
        let par = inverted_par_curve();
        let zero = par.bootstrap_zero().unwrap();

        for &days in &[730u32, 1095, 1825, 2555, 3650] {
            let p = par.points[&days];
            let z = zero.get(days).unwrap();
            assert!(
                z <= p + 1e-9,
                "inverted: days={days}, zero={z:.6} should be ≤ par={p:.6}"
            );
        }
    }

    /// Short-end (≤ 365 days) knots must be identical: par = zero for
    /// money-market tenors.
    #[test]
    fn bootstrap_short_end_par_equals_zero() {
        let par = steep_par_curve();
        let zero = par.bootstrap_zero().unwrap();

        for &days in &[30u32, 91, 182, 365] {
            let p = par.points[&days];
            let z = zero.points[&days];
            assert!(
                (z - p).abs() < 1e-12,
                "short end: days={days}, par={p:.8}, zero={z:.8}"
            );
        }
    }

    /// Very short tenors — single knot at 30D — should succeed.
    #[test]
    fn bootstrap_single_knot_short_end() {
        let d = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
        let mut par = YieldCurve::new(d);
        par.insert(30, 0.04);
        let zero = par.bootstrap_zero().unwrap();
        assert_eq!(zero.points[&30], 0.04);
    }

    /// Empty par curve produces empty zero curve.
    #[test]
    fn bootstrap_empty_curve() {
        let d = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
        let par = YieldCurve::new(d);
        let zero = par.bootstrap_zero().unwrap();
        assert!(zero.points.is_empty());
        assert_eq!(zero.yield_type, YieldType::Zero);
    }

    /// Calling bootstrap_zero on a zero curve is an error.
    #[test]
    fn bootstrap_zero_on_zero_curve_is_err() {
        let d = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
        let mut z = YieldCurve::new(d);
        z.yield_type = YieldType::Zero;
        z.insert(365, 0.04);
        assert!(z.bootstrap_zero().is_err());
    }

    /// Only 2Y and 10Y provided — bootstrap should work even without contiguous
    /// tenors, using interpolation for intermediate coupon periods.
    #[test]
    fn bootstrap_sparse_curve() {
        let d = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
        let mut par = YieldCurve::new(d);
        par.insert(365, 0.02);
        par.insert(3650, 0.04);
        let zero = par.bootstrap_zero().unwrap();
        // Short end: zero == par
        assert!((zero.points[&365] - 0.02).abs() < 1e-12);
        // Long end: zero rate at 10Y should be > par rate (upward slope)
        let z10 = zero.points[&3650];
        assert!(
            z10 > 0.04 - 1e-6,
            "sparse 10Y zero={z10:.6} expected ≥ 0.04"
        );
    }

    /// Near-zero rate environment: flat at 0.1% (2020 pandemic levels).
    #[test]
    fn bootstrap_near_zero_rates() {
        let par = flat_par_curve(0.001);
        let zero = par.bootstrap_zero().unwrap();
        for (&days, &par_rate) in &par.points {
            let z = zero.get(days).unwrap();
            assert!(
                (z - par_rate).abs() < 5e-4,
                "near-zero: days={days}, par={par_rate:.6}, zero={z:.6}"
            );
        }
    }

    /// High-rate environment: flat at 8%.
    #[test]
    fn bootstrap_high_rate_environment() {
        let par = flat_par_curve(0.08);
        let zero = par.bootstrap_zero().unwrap();
        for &days in par.points.keys() {
            let z = zero.get(days).unwrap();
            // All bootstrapped zeros should be positive
            assert!(z > 0.0, "high-rate: negative zero at {days}d: {z}");
        }
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
