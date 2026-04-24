//! [`Tenor`] — semantic type for yield-curve maturities.
//!
//! Replaces raw `u32` day counts in the public API.  Internally stored as days;
//! Display/parse uses standard money-market notation.
//!
//! # Examples
//!
//! ```
//! use curvekit::Tenor;
//!
//! // Named constants — Treasury calendar knots
//! assert_eq!(Tenor::Y10.as_days(), 3650);
//! assert_eq!(Tenor::M3.as_days(), 91);   // Treasury 3M knot (not 3×30=90)
//!
//! // Constructors — mathematical approximations
//! assert_eq!(Tenor::months(6).as_days(), 180);  // 6 × 30
//! assert_eq!(Tenor::years(2).as_days(), 730);   // 2 × 365
//! assert_eq!(Tenor::weeks(1).as_days(), 7);
//! assert_eq!(Tenor::days(45).as_days(), 45);
//!
//! // u32 still converts (backward compat)
//! let t: Tenor = 3650_u32.into();
//! assert_eq!(t, Tenor::Y10);
//!
//! // Display round-trips through FromStr for constants whose day counts
//! // are exact multiples of the implied unit
//! let s = Tenor::Y10.to_string();
//! assert_eq!(s, "10Y");
//! let parsed: Tenor = s.parse().unwrap();
//! assert_eq!(parsed, Tenor::Y10);
//!
//! // Ad-hoc tenors fall back to days notation
//! assert_eq!(Tenor::days(45).to_string(), "45D");
//! let parsed_d: Tenor = "45D".parse().unwrap();
//! assert_eq!(parsed_d, Tenor::days(45));
//!
//! // Treasury knots display with their label but do NOT round-trip:
//! // M3=91 displays "3M", but "3M".parse() → months(3) = 90 ≠ 91
//! assert_eq!(Tenor::M3.to_string(), "3M");
//! assert_ne!("3M".parse::<Tenor>().unwrap(), Tenor::M3);
//! ```

use std::fmt;
use std::str::FromStr;

/// Market tenor — duration to maturity, stored internally as days.
///
/// Display/parse uses standard money-market notation: `1D`, `1W`, `1M`, `3M`,
/// `6M`, `1Y`, `2Y`, …, `30Y`.  The struct is `Copy` and totally ordered by
/// days so it sorts naturally from shortest to longest.
///
/// # Approximations
///
/// These follow the conventional rates-desk approximations (not ACT/360):
/// - 1 month = 30 days
/// - 1 year  = 365 days
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tenor(u32);

impl Tenor {
    // ── Standard market tenors ──────────────────────────────────────────────

    /// Overnight (1 day).
    pub const ON: Tenor = Tenor(1);
    /// 1 week (7 days).
    pub const W1: Tenor = Tenor(7);
    /// 1 month (30 days).
    pub const M1: Tenor = Tenor(30);
    /// 2 months (60 days).
    pub const M2: Tenor = Tenor(60);
    /// 3 months (91 days).
    pub const M3: Tenor = Tenor(91);
    /// 6 months (182 days).
    pub const M6: Tenor = Tenor(182);
    /// 1 year (365 days).
    pub const Y1: Tenor = Tenor(365);
    /// 2 years (730 days).
    pub const Y2: Tenor = Tenor(730);
    /// 3 years (1 095 days).
    pub const Y3: Tenor = Tenor(1095);
    /// 5 years (1 826 days).
    pub const Y5: Tenor = Tenor(1826);
    /// 7 years (2 555 days).
    pub const Y7: Tenor = Tenor(2555);
    /// 10 years (3 650 days).
    pub const Y10: Tenor = Tenor(3650);
    /// 20 years (7 300 days).
    pub const Y20: Tenor = Tenor(7300);
    /// 30 years (10 950 days).
    pub const Y30: Tenor = Tenor(10950);

    // ── Constructors ────────────────────────────────────────────────────────

    /// Construct from an exact number of days.
    #[inline]
    pub const fn days(d: u32) -> Self {
        Self(d)
    }

    /// Construct from weeks (1 week = 7 days).
    #[inline]
    pub const fn weeks(w: u32) -> Self {
        Self(w * 7)
    }

    /// Construct from months using the 30-day month approximation.
    ///
    /// This is the standard convention for yield-curve math; it does **not**
    /// follow actual calendar months.
    #[inline]
    pub const fn months(m: u32) -> Self {
        Self(m * 30)
    }

    /// Construct from years using the 365-day year approximation.
    ///
    /// This is the standard convention for yield-curve math; it does **not**
    /// account for leap years.
    #[inline]
    pub const fn years(y: u32) -> Self {
        Self(y * 365)
    }

    // ── Accessor ─────────────────────────────────────────────────────────────

    /// Return the underlying day count.
    #[inline]
    pub const fn as_days(self) -> u32 {
        self.0
    }
}

// ── From<u32> — backward-compatible conversion ───────────────────────────────

impl From<u32> for Tenor {
    /// Convert a raw day count into a `Tenor`.
    ///
    /// Allows callers that still pass `u32` literals to compile unchanged
    /// wherever `impl Into<Tenor>` is accepted.
    #[inline]
    fn from(days: u32) -> Self {
        Self(days)
    }
}

// ── Display ───────────────────────────────────────────────────────────────────

/// Display uses the smallest clean unit:
///
/// - If the day count equals a whole number of years (365-day year) → `{n}Y`
/// - Else if the day count equals a whole number of months (30-day month) → `{n}M`
/// - Else if the day count equals a whole number of weeks (7-day week) → `{n}W`
/// - Otherwise → `{n}D`
///
/// This means `Tenor::M3` (91 days) displays as `"3M"` because `91 = 3 × 30 + 1`
/// is **not** a clean multiple of 30, so … wait — 91 is a named constant whose
/// display the brief explicitly pins to `"3M"`.  The named constants use an
/// exact-match table; only unnamed values fall through to the unit logic.
///
/// Named-constant table (exact day counts):
///
/// | Days  | Display |
/// |-------|---------|
/// | 1     | `ON`    |
/// | 7     | `1W`    |
/// | 30    | `1M`    |
/// | 60    | `2M`    |
/// | 91    | `3M`    |
/// | 182   | `6M`    |
/// | 365   | `1Y`    |
/// | 730   | `2Y`    |
/// | 1095  | `3Y`    |
/// | 1826  | `5Y`    |
/// | 2555  | `7Y`    |
/// | 3650  | `10Y`   |
/// | 7300  | `20Y`   |
/// | 10950 | `30Y`   |
///
/// All other values: smallest clean unit, fallback `{n}D`.
impl fmt::Display for Tenor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            1 => write!(f, "ON"),
            7 => write!(f, "1W"),
            30 => write!(f, "1M"),
            60 => write!(f, "2M"),
            91 => write!(f, "3M"),
            182 => write!(f, "6M"),
            365 => write!(f, "1Y"),
            730 => write!(f, "2Y"),
            1095 => write!(f, "3Y"),
            1826 => write!(f, "5Y"),
            2555 => write!(f, "7Y"),
            3650 => write!(f, "10Y"),
            7300 => write!(f, "20Y"),
            10950 => write!(f, "30Y"),
            d => {
                // Fall through: smallest clean unit
                if d % 365 == 0 {
                    write!(f, "{}Y", d / 365)
                } else if d % 30 == 0 {
                    write!(f, "{}M", d / 30)
                } else if d % 7 == 0 {
                    write!(f, "{}W", d / 7)
                } else {
                    write!(f, "{}D", d)
                }
            }
        }
    }
}

// ── FromStr ───────────────────────────────────────────────────────────────────

/// Error returned when a tenor string cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenorParseError(String);

impl fmt::Display for TenorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid tenor '{}': expected <n>D, <n>W, <n>M, or <n>Y (e.g. 10Y, 3M, 45D)",
            self.0
        )
    }
}

impl std::error::Error for TenorParseError {}

/// Parse standard money-market tenor strings.
///
/// Accepted suffix letters (case-insensitive): `D`, `W`, `M`, `Y`.
/// Also accepts `ON` (case-insensitive) for overnight.
///
/// # Examples
///
/// ```
/// use curvekit::Tenor;
///
/// assert_eq!("10Y".parse::<Tenor>().unwrap(), Tenor::Y10);
/// // "3M" → months(3) = 90 days (NOT the Treasury M3 knot at 91 days)
/// assert_eq!("3M".parse::<Tenor>().unwrap(), Tenor::months(3));
/// assert_eq!("45D".parse::<Tenor>().unwrap(), Tenor::days(45));
/// assert_eq!("2W".parse::<Tenor>().unwrap(), Tenor::weeks(2));
/// assert_eq!("ON".parse::<Tenor>().unwrap(), Tenor::ON);
/// // Case-insensitive
/// assert_eq!("10y".parse::<Tenor>().unwrap(), Tenor::Y10);
/// assert_eq!("3m".parse::<Tenor>().unwrap(), Tenor::months(3));
/// ```
impl FromStr for Tenor {
    type Err = TenorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(TenorParseError(s.to_string()));
        }
        // Special case: overnight
        if s.eq_ignore_ascii_case("ON") {
            return Ok(Tenor::ON);
        }
        // Split into numeric prefix and unit suffix (one character)
        let bytes = s.as_bytes();
        let suffix = *bytes.last().unwrap(); // safe: len >= 1 after empty check
        let prefix = &s[..s.len() - 1];
        if prefix.is_empty() {
            return Err(TenorParseError(s.to_string()));
        }
        let n: u32 = prefix.parse().map_err(|_| TenorParseError(s.to_string()))?;
        match suffix.to_ascii_uppercase() {
            b'D' => Ok(Tenor::days(n)),
            b'W' => Ok(Tenor::weeks(n)),
            b'M' => Ok(Tenor::months(n)),
            b'Y' => Ok(Tenor::years(n)),
            _ => Err(TenorParseError(s.to_string())),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constants ────────────────────────────────────────────────────────────

    #[test]
    fn constant_on() {
        assert_eq!(Tenor::ON.as_days(), 1);
    }

    #[test]
    fn constant_w1() {
        assert_eq!(Tenor::W1.as_days(), 7);
    }

    #[test]
    fn constant_m3() {
        assert_eq!(Tenor::M3.as_days(), 91);
    }

    #[test]
    fn constant_y10() {
        assert_eq!(Tenor::Y10.as_days(), 3650);
    }

    #[test]
    fn constant_y30() {
        assert_eq!(Tenor::Y30.as_days(), 10950);
    }

    // ── Constructors ─────────────────────────────────────────────────────────

    #[test]
    fn constructor_days() {
        assert_eq!(Tenor::days(45).as_days(), 45);
    }

    #[test]
    fn constructor_weeks() {
        assert_eq!(Tenor::weeks(2).as_days(), 14);
    }

    #[test]
    fn constructor_months() {
        assert_eq!(Tenor::months(3).as_days(), 90);
        // Note: Tenor::M3 is pinned at 91; months(3) gives 90 (3×30).
        // They are deliberately different — M3 follows the Treasury knot.
        assert_ne!(Tenor::months(3), Tenor::M3);
    }

    #[test]
    fn constructor_years() {
        assert_eq!(Tenor::years(10).as_days(), 3650);
        assert_eq!(Tenor::years(10), Tenor::Y10);
    }

    // ── From<u32> ─────────────────────────────────────────────────────────────

    #[test]
    fn from_u32() {
        let t: Tenor = 3650_u32.into();
        assert_eq!(t, Tenor::Y10);
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn display_named_constants() {
        assert_eq!(Tenor::ON.to_string(), "ON");
        assert_eq!(Tenor::W1.to_string(), "1W");
        assert_eq!(Tenor::M3.to_string(), "3M");
        assert_eq!(Tenor::Y10.to_string(), "10Y");
        assert_eq!(Tenor::Y30.to_string(), "30Y");
    }

    #[test]
    fn display_ad_hoc_days() {
        // 45 is not divisible by 7, 30, or 365 → falls back to days
        assert_eq!(Tenor::days(45).to_string(), "45D");
    }

    #[test]
    fn display_ad_hoc_weeks() {
        // 14 = 2 × 7 and not divisible by 30 or 365
        assert_eq!(Tenor::days(14).to_string(), "2W");
    }

    #[test]
    fn display_ad_hoc_months() {
        // 120 = 4 × 30, not divisible by 365
        assert_eq!(Tenor::days(120).to_string(), "4M");
    }

    #[test]
    fn display_ad_hoc_years() {
        // 1460 = 4 × 365
        assert_eq!(Tenor::days(1460).to_string(), "4Y");
    }

    // ── FromStr ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_years() {
        assert_eq!("10Y".parse::<Tenor>().unwrap(), Tenor::Y10);
        assert_eq!("1Y".parse::<Tenor>().unwrap(), Tenor::Y1);
    }

    #[test]
    fn parse_months() {
        assert_eq!("6M".parse::<Tenor>().unwrap(), Tenor::months(6));
    }

    #[test]
    fn parse_days() {
        assert_eq!("45D".parse::<Tenor>().unwrap(), Tenor::days(45));
    }

    #[test]
    fn parse_weeks() {
        assert_eq!("2W".parse::<Tenor>().unwrap(), Tenor::weeks(2));
    }

    #[test]
    fn parse_overnight() {
        assert_eq!("ON".parse::<Tenor>().unwrap(), Tenor::ON);
        assert_eq!("on".parse::<Tenor>().unwrap(), Tenor::ON);
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!("10y".parse::<Tenor>().unwrap(), Tenor::Y10);
        assert_eq!("3m".parse::<Tenor>().unwrap(), Tenor::months(3));
        assert_eq!("45d".parse::<Tenor>().unwrap(), Tenor::days(45));
        assert_eq!("2w".parse::<Tenor>().unwrap(), Tenor::weeks(2));
    }

    #[test]
    fn parse_invalid_empty() {
        assert!("".parse::<Tenor>().is_err());
    }

    #[test]
    fn parse_invalid_no_suffix() {
        assert!("365".parse::<Tenor>().is_err());
    }

    #[test]
    fn parse_invalid_no_number() {
        assert!("Y".parse::<Tenor>().is_err());
    }

    #[test]
    fn parse_invalid_unknown_suffix() {
        assert!("10Q".parse::<Tenor>().is_err());
    }

    // ── Display / FromStr round-trip ──────────────────────────────────────────

    /// Constants that round-trip perfectly: their day count is an exact
    /// multiple of the unit implied by their Display string.
    ///
    /// M3 (91 days), M6 (182 days), and Y5 (1826 days) do NOT round-trip
    /// because they are Treasury-calendar knots whose day counts differ from
    /// the math approximations (3×30=90, 6×30=180, 5×365=1825).
    /// This asymmetry is intentional and tested in `constructor_months`.
    #[test]
    fn round_trip_named_constants() {
        for tenor in &[
            Tenor::ON, // 1 → "ON" → 1
            Tenor::W1, // 7 → "1W" → 7
            Tenor::M1, // 30 → "1M" → 30
            Tenor::M2, // 60 → "2M" → 60
            // M3 (91) and M6 (182) skipped: Treasury knots ≠ months(n)×30
            Tenor::Y1, // 365 → "1Y" → 365
            Tenor::Y2, // 730 → "2Y" → 730
            Tenor::Y3, // 1095 → "3Y" → 1095
            // Y5 (1826) skipped: not a multiple of 365
            Tenor::Y7,  // 2555 → "7Y" → 2555
            Tenor::Y10, // 3650 → "10Y" → 3650
            Tenor::Y20, // 7300 → "20Y" → 7300
            Tenor::Y30, // 10950 → "30Y" → 10950
        ] {
            let s = tenor.to_string();
            let parsed: Tenor = s
                .parse()
                .unwrap_or_else(|_| panic!("round-trip failed for {s}"));
            assert_eq!(parsed, *tenor, "round-trip mismatch for {s}");
        }
    }

    /// Verify the known asymmetry: M3/M6/Y5 are Treasury knots that display
    /// as their closest month/year label but do NOT round-trip through parse.
    #[test]
    fn treasury_knot_display_asymmetry() {
        // M3 = 91 days; display shows "3M" but parsing "3M" → months(3) = 90
        assert_eq!(Tenor::M3.to_string(), "3M");
        assert_ne!("3M".parse::<Tenor>().unwrap(), Tenor::M3);

        // M6 = 182 days; display shows "6M" but parsing "6M" → months(6) = 180
        assert_eq!(Tenor::M6.to_string(), "6M");
        assert_ne!("6M".parse::<Tenor>().unwrap(), Tenor::M6);

        // Y5 = 1826 days; display shows "5Y" but parsing "5Y" → years(5) = 1825
        assert_eq!(Tenor::Y5.to_string(), "5Y");
        assert_ne!("5Y".parse::<Tenor>().unwrap(), Tenor::Y5);
    }

    #[test]
    fn round_trip_ad_hoc() {
        let t = Tenor::days(45);
        let s = t.to_string(); // "45D"
        let parsed: Tenor = s.parse().unwrap();
        assert_eq!(parsed, t);
    }

    // ── Ordering ──────────────────────────────────────────────────────────────

    #[test]
    fn ordering() {
        assert!(Tenor::M1 < Tenor::M3);
        assert!(Tenor::Y1 < Tenor::Y10);
        assert!(Tenor::Y10 < Tenor::Y30);
    }
}
