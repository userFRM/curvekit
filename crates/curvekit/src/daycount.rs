//! Day-count conventions for yield calculations.
//!
//! Day-count conventions determine the year fraction between two dates, which
//! in turn affects how interest accrues and how rates are quoted.
//!
//! # Supported conventions
//!
//! | Convention | Description |
//! |---|---|
//! | [`Act360`][DayCount::Act360] | Actual days / 360 — US money markets, LIBOR |
//! | [`Act365Fixed`][DayCount::Act365Fixed] | Actual days / 365 — GBP, AUD, also curvekit default |
//! | [`Thirty360`][DayCount::Thirty360] | 30/360 US (ISDA) — US corporate bonds |
//! | [`ActAct`][DayCount::ActAct] | Actual/Actual ISDA — US Treasuries (the standard for UST) |
//!
//! # References
//!
//! - ISDA 2006 Definitions, §4.16
//! - ICMA Rule Book
//!
//! # Note on curvekit's internal rate storage
//!
//! Rates stored in curvekit's parquet files are indexed by an integer
//! **days-to-maturity** value which implicitly uses `Act365Fixed` (actual
//! days ÷ 365). All `YieldCurve::get` interpolation works in that space.
//! [`treasury_rate_with_convention`][crate::Curvekit::treasury_rate_with_convention]
//! scales the result to the requested convention.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Day-count convention for year-fraction calculations.
///
/// # Examples
///
/// ```
/// use curvekit::DayCount;
/// use chrono::NaiveDate;
///
/// let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
/// let end   = NaiveDate::from_ymd_opt(2020, 7, 1).unwrap();   // 182 actual days
///
/// let yf_act360    = DayCount::Act360.year_fraction(start, end);
/// let yf_act365    = DayCount::Act365Fixed.year_fraction(start, end);
/// let yf_thirty360 = DayCount::Thirty360.year_fraction(start, end);
/// let yf_actact    = DayCount::ActAct.year_fraction(start, end);
///
/// // Act/360: 182 / 360
/// assert!((yf_act360 - 182.0 / 360.0).abs() < 1e-10);
///
/// // Act/365: 182 / 365
/// assert!((yf_act365 - 182.0 / 365.0).abs() < 1e-10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DayCount {
    /// Actual/360 — actual calendar days divided by 360.
    ///
    /// Used for: USD LIBOR (historical), Fed Funds, many money-market instruments,
    /// EURIBOR.
    ///
    /// `year_fraction = actual_days / 360`
    Act360,

    /// Actual/365 Fixed — actual calendar days divided by 365.
    ///
    /// Used for: GBP, AUD, JPY markets; curvekit's internal rate indexing.
    ///
    /// `year_fraction = actual_days / 365`
    Act365Fixed,

    /// 30/360 US (ISDA 2006 §4.16(f)) — each month treated as 30 days,
    /// year as 360 days.
    ///
    /// Used for: US corporate bonds, US agency bonds, some USD swaps.
    ///
    /// Rules (ISDA):
    /// - If `D1 = 31`, set `D1 = 30`.
    /// - If `D2 = 31` and `D1 >= 30`, set `D2 = 30`.
    ///
    /// `year_fraction = (360*(Y2-Y1) + 30*(M2-M1) + (D2-D1)) / 360`
    Thirty360,

    /// Actual/Actual ISDA — actual days in each calendar year's portion,
    /// divided by the length of that year (365 or 366 in leap years).
    ///
    /// Used for: US Treasury bonds (ISDA standard), many government bonds.
    ///
    /// For a period spanning a single year: `actual_days / days_in_year`.
    /// For multi-year periods: sum contributions from each calendar year.
    ActAct,
}

impl DayCount {
    /// Compute the year fraction between `start` (inclusive) and `end`
    /// (exclusive) under this convention.
    ///
    /// Returns `0.0` if `end <= start`.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::DayCount;
    /// use chrono::NaiveDate;
    ///
    /// let start = NaiveDate::from_ymd_opt(2020, 3, 1).unwrap();
    /// let end   = NaiveDate::from_ymd_opt(2020, 9, 1).unwrap();
    ///
    /// // 184 actual days (2020 is leap)
    /// let yf = DayCount::Act360.year_fraction(start, end);
    /// assert!((yf - 184.0 / 360.0).abs() < 1e-10, "yf={yf}");
    /// ```
    pub fn year_fraction(&self, start: NaiveDate, end: NaiveDate) -> f64 {
        if end <= start {
            return 0.0;
        }
        match self {
            DayCount::Act360 => {
                let days = (end - start).num_days() as f64;
                days / 360.0
            }
            DayCount::Act365Fixed => {
                let days = (end - start).num_days() as f64;
                days / 365.0
            }
            DayCount::Thirty360 => thirty360_us(start, end),
            DayCount::ActAct => act_act_isda(start, end),
        }
    }
}

// ---------------------------------------------------------------------------
// 30/360 US (ISDA 2006 §4.16(f))
// ---------------------------------------------------------------------------

fn thirty360_us(start: NaiveDate, end: NaiveDate) -> f64 {
    use chrono::Datelike;

    let y1 = start.year();
    let m1 = start.month() as i32;
    let mut d1 = start.day() as i32;

    let y2 = end.year();
    let m2 = end.month() as i32;
    let mut d2 = end.day() as i32;

    // ISDA rule: if D1 = 31, set D1 = 30
    if d1 == 31 {
        d1 = 30;
    }
    // ISDA rule: if D2 = 31 and D1 >= 30, set D2 = 30
    if d2 == 31 && d1 >= 30 {
        d2 = 30;
    }

    let numerator = 360 * (y2 - y1) + 30 * (m2 - m1) + (d2 - d1);
    numerator as f64 / 360.0
}

// ---------------------------------------------------------------------------
// Actual/Actual ISDA
// ---------------------------------------------------------------------------

fn act_act_isda(start: NaiveDate, end: NaiveDate) -> f64 {
    use chrono::Datelike;

    // Handle periods within the same calendar year.
    if start.year() == end.year() {
        let days = (end - start).num_days() as f64;
        let days_in_year = if is_leap_year(start.year()) {
            366.0
        } else {
            365.0
        };
        return days / days_in_year;
    }

    // Multi-year: split at each year boundary.
    let mut total = 0.0_f64;

    // Contribution from start year: start → Jan 1 of next year.
    let start_year_end = NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).expect("year+1 is valid");
    let days_in_start_year = if is_leap_year(start.year()) {
        366.0
    } else {
        365.0
    };
    total += (start_year_end - start).num_days() as f64 / days_in_start_year;

    // Contribution from complete middle years.
    let mut y = start.year() + 1;
    while y < end.year() {
        let days_in_y = if is_leap_year(y) { 366.0 } else { 365.0 };
        let _ = days_in_y; // computed only for symmetry; full year always contributes 1.0
        total += 1.0; // one complete calendar year
        y += 1;
    }

    // Contribution from end year: Jan 1 of end year → end.
    let end_year_start = NaiveDate::from_ymd_opt(end.year(), 1, 1).expect("end year is valid");
    let days_in_end_year = if is_leap_year(end.year()) {
        366.0
    } else {
        365.0
    };
    total += (end - end_year_start).num_days() as f64 / days_in_end_year;

    total
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // ── Act/360 ──────────────────────────────────────────────────────────────

    #[test]
    fn act360_non_leap_half_year() {
        // 2019-01-01 to 2019-07-01: 181 actual days (Jan=31, Feb=28, Mar=31, Apr=30, May=31, Jun=30)
        let yf = DayCount::Act360.year_fraction(d(2019, 1, 1), d(2019, 7, 1));
        assert!((yf - 181.0 / 360.0).abs() < 1e-10, "yf={yf}");
    }

    #[test]
    fn act360_leap_year() {
        // 2020-01-01 to 2020-07-01: 182 actual days (includes Feb 29)
        let yf = DayCount::Act360.year_fraction(d(2020, 1, 1), d(2020, 7, 1));
        assert!((yf - 182.0 / 360.0).abs() < 1e-10, "yf={yf}");
    }

    #[test]
    fn act360_zero_when_same_date() {
        let yf = DayCount::Act360.year_fraction(d(2020, 6, 1), d(2020, 6, 1));
        assert_eq!(yf, 0.0);
    }

    // ── Act/365 Fixed ─────────────────────────────────────────────────────────

    #[test]
    fn act365_full_year_non_leap() {
        // 2019-01-01 to 2020-01-01: 365 days → 1.0
        let yf = DayCount::Act365Fixed.year_fraction(d(2019, 1, 1), d(2020, 1, 1));
        assert!((yf - 1.0).abs() < 1e-10, "yf={yf}");
    }

    #[test]
    fn act365_leap_year_is_not_1() {
        // 2020-01-01 to 2021-01-01: 366 days / 365 > 1.0
        let yf = DayCount::Act365Fixed.year_fraction(d(2020, 1, 1), d(2021, 1, 1));
        assert!((yf - 366.0 / 365.0).abs() < 1e-10, "yf={yf}");
    }

    #[test]
    fn act365_182_days() {
        // From doc example: 2020-01-01 to 2020-07-01 = 182 days
        let yf = DayCount::Act365Fixed.year_fraction(d(2020, 1, 1), d(2020, 7, 1));
        assert!((yf - 182.0 / 365.0).abs() < 1e-10, "yf={yf}");
    }

    // ── 30/360 US ────────────────────────────────────────────────────────────

    /// ISDA 2006 §4.16(f) example: start=2007-01-15, end=2007-07-15 → 6 months
    #[test]
    fn thirty360_half_year_exact() {
        let yf = DayCount::Thirty360.year_fraction(d(2007, 1, 15), d(2007, 7, 15));
        // 0*360 + 6*30 + 0 = 180 / 360 = 0.5
        assert!((yf - 0.5).abs() < 1e-10, "yf={yf}");
    }

    /// D1=31 → D1 becomes 30; D2=31 and D1≥30 → D2 becomes 30.
    #[test]
    fn thirty360_both_31() {
        // 2020-01-31 to 2020-07-31
        // D1=31 → 30; D2=31 and D1=30 → D2=30
        // 0*360 + 6*30 + (30-30) = 180 / 360 = 0.5
        let yf = DayCount::Thirty360.year_fraction(d(2020, 1, 31), d(2020, 7, 31));
        assert!((yf - 0.5).abs() < 1e-10, "yf={yf}");
    }

    /// D2=31 but D1=28 → D2 stays 31 (ISDA rule: only apply if D1≥30).
    #[test]
    fn thirty360_d2_31_d1_28() {
        // 2020-02-28 to 2020-03-31
        // D1=28 (< 30) so D2 stays 31.
        // 0*360 + 1*30 + (31-28) = 33 / 360
        let yf = DayCount::Thirty360.year_fraction(d(2020, 2, 28), d(2020, 3, 31));
        let expected = 33.0 / 360.0;
        assert!(
            (yf - expected).abs() < 1e-10,
            "yf={yf}, expected={expected}"
        );
    }

    #[test]
    fn thirty360_full_year() {
        // 2020-01-01 to 2021-01-01: 360*1 + 30*0 + 0 = 360/360 = 1.0
        let yf = DayCount::Thirty360.year_fraction(d(2020, 1, 1), d(2021, 1, 1));
        assert!((yf - 1.0).abs() < 1e-10, "yf={yf}");
    }

    // ── Act/Act ISDA ─────────────────────────────────────────────────────────

    /// Within a non-leap year: actual days / 365.
    #[test]
    fn actact_within_non_leap_year() {
        // 2019-03-01 to 2019-09-01: 184 days in a 365-day year
        let yf = DayCount::ActAct.year_fraction(d(2019, 3, 1), d(2019, 9, 1));
        assert!((yf - 184.0 / 365.0).abs() < 1e-10, "yf={yf}");
    }

    /// Within a leap year: actual days / 366.
    #[test]
    fn actact_within_leap_year() {
        // 2020-01-01 to 2020-07-01: 182 days in a 366-day year
        let yf = DayCount::ActAct.year_fraction(d(2020, 1, 1), d(2020, 7, 1));
        assert!((yf - 182.0 / 366.0).abs() < 1e-10, "yf={yf}");
    }

    /// Spanning a year boundary: 2019-07-01 to 2020-07-01.
    /// Contribution from 2019: (366-182)/365 days of 2019; contribution from 2020: 182/366.
    /// But: 2019-07-01 to 2020-01-01 = 184 days / 365; 2020-01-01 to 2020-07-01 = 182 / 366.
    #[test]
    fn actact_spanning_year_boundary() {
        let start = d(2019, 7, 1);
        let end = d(2020, 7, 1);
        let yf = DayCount::ActAct.year_fraction(start, end);
        // 2019 portion: 2019-07-01 to 2020-01-01 = 184 days / 365
        // 2020 portion: 2020-01-01 to 2020-07-01 = 182 days / 366
        let expected = 184.0 / 365.0 + 182.0 / 366.0;
        assert!(
            (yf - expected).abs() < 1e-10,
            "yf={yf}, expected={expected}"
        );
    }

    /// Full non-leap year is exactly 1.0.
    #[test]
    fn actact_full_non_leap_year() {
        let yf = DayCount::ActAct.year_fraction(d(2019, 1, 1), d(2020, 1, 1));
        assert!((yf - 1.0).abs() < 1e-10, "yf={yf}");
    }

    /// Full leap year is exactly 1.0 (ActAct: 366/366).
    #[test]
    fn actact_full_leap_year() {
        let yf = DayCount::ActAct.year_fraction(d(2020, 1, 1), d(2021, 1, 1));
        assert!((yf - 1.0).abs() < 1e-10, "yf={yf}");
    }

    /// Zero returns 0.0 regardless of convention.
    #[test]
    fn zero_year_fraction() {
        let dt = d(2020, 6, 15);
        for conv in &[
            DayCount::Act360,
            DayCount::Act365Fixed,
            DayCount::Thirty360,
            DayCount::ActAct,
        ] {
            assert_eq!(conv.year_fraction(dt, dt), 0.0, "conv={conv:?}");
        }
    }

    /// Leap-year check.
    #[test]
    fn leap_year_detection() {
        assert!(is_leap_year(2020));
        assert!(!is_leap_year(2019));
        assert!(!is_leap_year(1900)); // divisible by 100 but not 400
        assert!(is_leap_year(2000)); // divisible by 400
    }
}
