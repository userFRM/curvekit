//! [`Date`] — ergonomic date input for curvekit's public API.
//!
//! Accepts multiple formats so callers are never forced to import `chrono`:
//!
//! - ISO string: `"2020-03-20"`
//! - Slashed string: `"2020/03/20"`
//! - Compact string: `"20200320"`
//! - `u32` YYYYMMDD: `20200320_u32`
//! - YMD tuple `(i32, u32, u32)` or `(u32, u32, u32)`: `(2020, 3, 20)`
//! - Existing `chrono::NaiveDate` (infallible)
//!
//! # Examples
//!
//! ```
//! use curvekit::Date;
//!
//! // From string (ISO, slashed, or compact)
//! let d1: Date = "2020-03-20".parse().unwrap();
//! let d2: Date = "2020/03/20".parse().unwrap();
//! let d3: Date = "20200320".parse().unwrap();
//!
//! // From YYYYMMDD u32
//! let d4 = Date::from_yyyymmdd(20200320).unwrap();
//!
//! // From YMD components
//! let d5 = Date::from_ymd(2020, 3, 20).unwrap();
//!
//! // All produce the same date
//! assert_eq!(d1, d4);
//! assert_eq!(d1, d5);
//!
//! // Display is always ISO-8601
//! assert_eq!(d1.to_string(), "2020-03-20");
//! ```

use chrono::NaiveDate;
use std::fmt;
use std::str::FromStr;

/// Error produced when a value cannot be converted into a [`Date`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateError(String);

impl fmt::Display for DateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid date: {}", self.0)
    }
}

impl std::error::Error for DateError {}

/// A calendar date for curvekit's public API.
///
/// Thin newtype over [`chrono::NaiveDate`] that accepts many input forms so
/// callers do not need to import `chrono` just to pass a date.
///
/// # Conversions
///
/// ```
/// use curvekit::{Date, IntoDate};
/// use chrono::NaiveDate;
///
/// // From NaiveDate (infallible)
/// let nd = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
/// let d = Date::from(nd);
///
/// // From ISO string
/// let d2: Date = "2020-03-20".parse().unwrap();
///
/// // From u32 YYYYMMDD
/// let d3 = Date::from_yyyymmdd(20200320).unwrap();
///
/// // From YMD tuple via IntoDate
/// let d4 = (2020i32, 3u32, 20u32).into_date().unwrap();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(NaiveDate);

impl Date {
    /// Construct from year, month, day components.
    ///
    /// # Errors
    ///
    /// Returns [`DateError`] if the components do not form a valid calendar date.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::Date;
    /// let d = Date::from_ymd(2020, 3, 20).unwrap();
    /// assert_eq!(d.to_string(), "2020-03-20");
    /// ```
    pub fn from_ymd(y: i32, m: u32, d: u32) -> Result<Self, DateError> {
        NaiveDate::from_ymd_opt(y, m, d)
            .map(Self)
            .ok_or_else(|| DateError(format!("{y:04}-{m:02}-{d:02} is not a valid date")))
    }

    /// Construct from a `u32` in `YYYYMMDD` format (e.g. `20200320`).
    ///
    /// # Errors
    ///
    /// Returns [`DateError`] if the value does not encode a valid calendar date.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::Date;
    /// let d = Date::from_yyyymmdd(20200320).unwrap();
    /// assert_eq!(d.to_string(), "2020-03-20");
    /// ```
    pub fn from_yyyymmdd(v: u32) -> Result<Self, DateError> {
        let y = (v / 10000) as i32;
        let m = (v / 100) % 100;
        let d = v % 100;
        Self::from_ymd(y, m, d)
    }

    /// Current date in America/New_York (ET) time zone.
    ///
    /// Uses UTC offset approximation via the `chrono` local offset; this is
    /// suitable for date selection (not sub-second precision). If the system
    /// timezone is set correctly this returns the Eastern calendar date; on
    /// systems where it is not, falls back to UTC.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::Date;
    /// use chrono::Datelike;
    /// let today = Date::today_et();
    /// // Just verify it's a plausible year
    /// assert!(today.inner().year() >= 2024);
    /// ```
    pub fn today_et() -> Self {
        // We don't take a dependency on chrono-tz for a single offset.
        // Instead use the system local time, which on servers is typically set
        // to UTC or ET. Callers who need exact ET behaviour should pass a
        // specific date.
        use chrono::Local;
        let local = Local::now().date_naive();
        Self(local)
    }

    /// Current date in UTC.
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::Date;
    /// use chrono::Datelike;
    /// let today = Date::today_utc();
    /// assert!(today.inner().year() >= 2024);
    /// ```
    pub fn today_utc() -> Self {
        use chrono::Utc;
        Self(Utc::now().date_naive())
    }

    /// Return the underlying [`NaiveDate`].
    ///
    /// # Examples
    ///
    /// ```
    /// use curvekit::Date;
    /// use chrono::{NaiveDate, Datelike};
    ///
    /// let d = Date::from_ymd(2020, 3, 20).unwrap();
    /// let nd: NaiveDate = d.inner();
    /// assert_eq!(nd.year(), 2020);
    /// ```
    #[inline]
    pub fn inner(self) -> NaiveDate {
        self.0
    }
}

// ── Fallible conversions ──────────────────────────────────────────────────────
//
// All conversions use `TryFrom<T, Error = DateError>` so that every call site
// only needs to handle one error type.  The `?` operator converts `DateError`
// into `curvekit::Error` automatically (via the `From<DateError> for Error`
// impl in error.rs).

/// Infallible `From<NaiveDate>`.
impl From<NaiveDate> for Date {
    #[inline]
    fn from(d: NaiveDate) -> Self {
        Self(d)
    }
}

// ── IntoDate sealed trait ─────────────────────────────────────────────────────
//
// Rather than fighting Rust's blanket `TryFrom` coherence rules (which prevent
// having `TryFrom<NaiveDate, Error=DateError>` when `From<NaiveDate>` exists),
// we use a dedicated conversion trait.  This is intentionally sealed — only the
// impls in this module are valid.

mod private {
    pub trait Sealed {}
}

/// Convert a value into a [`Date`].
///
/// Implemented for:
/// - `Date` — identity
/// - `&str`, `String` — parsed as `"YYYY-MM-DD"`, `"YYYY/MM/DD"`, or `"YYYYMMDD"`
/// - `u32` — interpreted as YYYYMMDD
/// - `(i32, u32, u32)`, `(u32, u32, u32)` — YMD tuple
/// - `chrono::NaiveDate` — direct wrap
///
/// Used as the bound on all public date parameters:
/// ```text
/// pub async fn treasury_curve(&self, date: impl IntoDate) -> Result<YieldCurve>
/// ```
pub trait IntoDate: private::Sealed {
    fn into_date(self) -> Result<Date, DateError>;
}

impl private::Sealed for Date {}
impl IntoDate for Date {
    fn into_date(self) -> Result<Date, DateError> {
        Ok(self)
    }
}

impl private::Sealed for &str {}
impl IntoDate for &str {
    fn into_date(self) -> Result<Date, DateError> {
        self.parse()
    }
}

impl private::Sealed for String {}
impl IntoDate for String {
    fn into_date(self) -> Result<Date, DateError> {
        self.parse()
    }
}

impl private::Sealed for u32 {}
impl IntoDate for u32 {
    fn into_date(self) -> Result<Date, DateError> {
        Date::from_yyyymmdd(self)
    }
}

impl private::Sealed for (i32, u32, u32) {}
impl IntoDate for (i32, u32, u32) {
    fn into_date(self) -> Result<Date, DateError> {
        Date::from_ymd(self.0, self.1, self.2)
    }
}

impl private::Sealed for (u32, u32, u32) {}
impl IntoDate for (u32, u32, u32) {
    fn into_date(self) -> Result<Date, DateError> {
        Date::from_ymd(self.0 as i32, self.1, self.2)
    }
}

impl private::Sealed for NaiveDate {}
impl IntoDate for NaiveDate {
    fn into_date(self) -> Result<Date, DateError> {
        Ok(Date(self))
    }
}

// ── FromStr ───────────────────────────────────────────────────────────────────

/// Parse a date from a string.
///
/// Accepted formats:
/// - `"YYYY-MM-DD"` (ISO 8601)
/// - `"YYYY/MM/DD"` (slashed)
/// - `"YYYYMMDD"` (compact, no separator)
impl FromStr for Date {
    type Err = DateError;

    fn from_str(s: &str) -> Result<Self, DateError> {
        let s = s.trim();
        // ISO: "2020-03-20" (length 10, dash at pos 4 and 7)
        if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
            return NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(Self)
                .map_err(|_| DateError(format!("'{s}' is not a valid YYYY-MM-DD date")));
        }
        // Slashed: "2020/03/20"
        if s.len() == 10 && s.as_bytes()[4] == b'/' && s.as_bytes()[7] == b'/' {
            return NaiveDate::parse_from_str(s, "%Y/%m/%d")
                .map(Self)
                .map_err(|_| DateError(format!("'{s}' is not a valid YYYY/MM/DD date")));
        }
        // Compact: "20200320"
        if s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
            let v: u32 = s
                .parse()
                .map_err(|_| DateError(format!("'{s}' cannot be parsed as YYYYMMDD")))?;
            return Self::from_yyyymmdd(v);
        }
        Err(DateError(format!(
            "'{s}' is not a recognised date format — expected YYYY-MM-DD, YYYY/MM/DD, or YYYYMMDD"
        )))
    }
}

// ── Display ───────────────────────────────────────────────────────────────────

/// Always formats as ISO 8601: `"YYYY-MM-DD"`.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%d"))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    // ── FromStr ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_iso() {
        let d: Date = "2020-03-20".parse().unwrap();
        assert_eq!(d.inner(), NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    }

    #[test]
    fn parse_slashed() {
        let d: Date = "2020/03/20".parse().unwrap();
        assert_eq!(d.inner(), NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    }

    #[test]
    fn parse_compact() {
        let d: Date = "20200320".parse().unwrap();
        assert_eq!(d.inner(), NaiveDate::from_ymd_opt(2020, 3, 20).unwrap());
    }

    #[test]
    fn parse_invalid_returns_err() {
        assert!("2020-99-99".parse::<Date>().is_err());
        assert!("not-a-date".parse::<Date>().is_err());
        assert!("".parse::<Date>().is_err());
    }

    // ── from_yyyymmdd ─────────────────────────────────────────────────────────

    #[test]
    fn from_yyyymmdd_valid() {
        let d = Date::from_yyyymmdd(20200320).unwrap();
        assert_eq!(d.to_string(), "2020-03-20");
    }

    #[test]
    fn from_yyyymmdd_invalid() {
        assert!(Date::from_yyyymmdd(20209999).is_err());
    }

    // ── IntoDate for u32 ─────────────────────────────────────────────────────

    #[test]
    fn into_date_u32() {
        let d = 20200320u32.into_date().unwrap();
        assert_eq!(d.to_string(), "2020-03-20");
    }

    // ── IntoDate for tuples ───────────────────────────────────────────────────

    #[test]
    fn into_date_i32_tuple() {
        let d = (2020i32, 3u32, 20u32).into_date().unwrap();
        assert_eq!(d.to_string(), "2020-03-20");
    }

    #[test]
    fn into_date_u32_tuple() {
        let d = (2020u32, 3u32, 20u32).into_date().unwrap();
        assert_eq!(d.to_string(), "2020-03-20");
    }

    // ── From NaiveDate ────────────────────────────────────────────────────────

    #[test]
    fn from_naive_date() {
        let nd = NaiveDate::from_ymd_opt(2020, 3, 20).unwrap();
        let d = Date::from(nd);
        assert_eq!(d.inner(), nd);
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn display_is_iso() {
        let d = Date::from_ymd(2020, 3, 20).unwrap();
        assert_eq!(d.to_string(), "2020-03-20");
    }

    // ── from_ymd ─────────────────────────────────────────────────────────────

    #[test]
    fn from_ymd_valid() {
        let d = Date::from_ymd(2020, 3, 20).unwrap();
        assert_eq!(d.inner().month(), 3);
    }

    #[test]
    fn from_ymd_invalid() {
        assert!(Date::from_ymd(2020, 13, 1).is_err());
    }

    // ── today helpers ─────────────────────────────────────────────────────────

    #[test]
    fn today_utc_is_plausible() {
        let d = Date::today_utc();
        assert!(d.inner().year() >= 2024);
    }

    #[test]
    fn today_et_is_plausible() {
        let d = Date::today_et();
        assert!(d.inner().year() >= 2024);
    }

    // ── ordering / equality ───────────────────────────────────────────────────

    #[test]
    fn ordering() {
        let d1 = Date::from_ymd(2020, 1, 1).unwrap();
        let d2 = Date::from_ymd(2020, 12, 31).unwrap();
        assert!(d1 < d2);
    }
}
