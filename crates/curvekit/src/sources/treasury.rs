//! US Treasury yield curve fetcher.
//!
//! Pulls the daily Treasury yield curve from `home.treasury.gov`, parses the
//! CSV, and converts BEY percentages to continuously-compounded rates.
//!
//! # Data source
//!
//! `https://home.treasury.gov/resource-center/data-chart-center/interest-rates/
//!  daily-treasury-rates.csv/all/all?type=daily_treasury_yield_curve
//!  &field_tdr_date_value=YYYY&page&_format=csv`
//!
//! Returns one row per trading day. Columns include the 12 standard maturities:
//! 1 Mo, 2 Mo, 3 Mo, 6 Mo, 1 Yr, 2 Yr, 3 Yr, 5 Yr, 7 Yr, 10 Yr, 20 Yr, 30 Yr.
//!
//! # Refresh schedule
//!
//! The Treasury publishes at approximately **15:30 ET** on each business day.

use async_trait::async_trait;
use chrono::NaiveDate;
use std::collections::BTreeMap;
use tracing::warn;

use crate::curve::YieldCurve;
use crate::error::{CurvekitError, Result, TreasuryError};

/// Standard Treasury maturity column names as they appear in the CSV header.
const MATURITY_COLUMNS: [&str; 12] = [
    "1 Mo", "2 Mo", "3 Mo", "6 Mo", "1 Yr", "2 Yr", "3 Yr", "5 Yr", "7 Yr", "10 Yr", "20 Yr",
    "30 Yr",
];

/// Days-to-maturity approximations aligned to [`MATURITY_COLUMNS`].
const MATURITY_DAYS: [u32; 12] = [
    30, 60, 91, 182, 365, 730, 1095, 1825, 2555, 3650, 7300, 10950,
];

/// Trait for fetching Treasury yield curves. Implemented by [`HttpTreasuryFetcher`];
/// test stubs can provide alternative implementations.
#[async_trait]
pub trait TreasuryFetcher: Send + Sync {
    /// Fetch yield curves for every trading day in `[start, end]` (YYYYMMDD integers).
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<YieldCurve>>;
}

/// Parse a treasury.gov CSV response into [`YieldCurve`] values.
///
/// Missing maturity columns are simply omitted from the curve (no NaN padding).
/// Unparseable rows are skipped with a `tracing::warn!`.
pub fn parse_treasury_csv(csv: &str) -> Result<Vec<YieldCurve>> {
    let mut lines = csv.lines();
    let header = lines
        .next()
        .ok_or_else(|| CurvekitError::TreasuryFetch(TreasuryError::Parse("empty CSV".into())))?;

    let headers: Vec<&str> = split_csv_row(header);

    let date_idx = headers
        .iter()
        .position(|h| h.trim_matches('"').trim() == "Date")
        .ok_or_else(|| {
            CurvekitError::TreasuryFetch(TreasuryError::Parse("missing 'Date' column".into()))
        })?;

    // Map each standard maturity column to its position (None = absent).
    let col_indices: [Option<usize>; 12] = std::array::from_fn(|i| {
        headers
            .iter()
            .position(|col| col.trim_matches('"').trim() == MATURITY_COLUMNS[i])
    });

    let mut curves = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv_row(line);

        let date_str = match fields.get(date_idx) {
            Some(s) => s.trim_matches('"').trim().to_owned(),
            None => {
                warn!(row = %line, "treasury: row missing date field, skipping");
                continue;
            }
        };
        let date = match parse_treasury_date(&date_str) {
            Some(d) => d,
            None => {
                warn!(row = %line, date = %date_str, "treasury: unparseable date, skipping");
                continue;
            }
        };

        let mut points = BTreeMap::new();
        for (i, col_opt) in col_indices.iter().enumerate() {
            let Some(col_idx) = col_opt else { continue };
            let Some(val_str) = fields.get(*col_idx) else {
                continue;
            };
            let cleaned = val_str.trim_matches('"').trim();
            if cleaned.is_empty() {
                continue;
            }
            match cleaned.parse::<f64>() {
                Ok(bey_pct) => {
                    // BEY % → APY → continuous.
                    let bey = bey_pct / 100.0;
                    let apy = (1.0 + bey / 2.0).powi(2) - 1.0;
                    let cont = (1.0 + apy).ln();
                    points.insert(MATURITY_DAYS[i], cont);
                }
                Err(_) => {
                    warn!(
                        col = MATURITY_COLUMNS[i],
                        val = cleaned,
                        "treasury: unparseable yield, skipping column"
                    );
                }
            }
        }

        curves.push(YieldCurve { date, points });
    }

    Ok(curves)
}

fn split_csv_row(row: &str) -> Vec<&str> {
    // treasury.gov quotes some headers. Track quote state to avoid splitting
    // on commas inside quoted fields.
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0usize;
    for (i, ch) in row.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                out.push(&row[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&row[start..]);
    out
}

fn parse_treasury_date(s: &str) -> Option<NaiveDate> {
    // Format: "MM/DD/YYYY"
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let mm: u32 = parts[0].parse().ok()?;
    let dd: u32 = parts[1].parse().ok()?;
    let yyyy: i32 = parts[2].parse().ok()?;
    NaiveDate::from_ymd_opt(yyyy, mm, dd)
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

/// Fetches US Treasury yield curves from `home.treasury.gov`.
pub struct HttpTreasuryFetcher {
    client: reqwest::Client,
}

impl HttpTreasuryFetcher {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("curvekit/0.1 (github.com/userFRM/curvekit)")
            .build()?;
        Ok(Self { client })
    }

    fn url_for_year(year: i32) -> String {
        format!(
            "https://home.treasury.gov/resource-center/data-chart-center/interest-rates/\
             daily-treasury-rates.csv/all/all\
             ?type=daily_treasury_yield_curve\
             &field_tdr_date_value={year}\
             &page&_format=csv"
        )
    }
}

#[async_trait]
impl TreasuryFetcher for HttpTreasuryFetcher {
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<YieldCurve>> {
        if start > end {
            return Err(CurvekitError::TreasuryFetch(
                TreasuryError::InvalidDateRange { start, end },
            ));
        }
        let start_year = (start / 10000) as i32;
        let end_year = (end / 10000) as i32;

        let mut all: Vec<YieldCurve> = Vec::new();
        for year in start_year..=end_year {
            let url = Self::url_for_year(year);
            let body = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(TreasuryError::Http)?
                .error_for_status()
                .map_err(TreasuryError::Http)?
                .text()
                .await
                .map_err(TreasuryError::Http)?;
            all.extend(parse_treasury_csv(&body)?);
        }

        // Filter to the requested date range.
        all.retain(|c| {
            let d = date_to_yyyymmdd(c.date);
            d >= start && d <= end
        });

        Ok(all)
    }
}

fn date_to_yyyymmdd(d: NaiveDate) -> u32 {
    (d.format("%Y%m%d").to_string()).parse().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        let path: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "..",
            "..",
            "tests",
            "fixtures",
            name,
        ]
        .iter()
        .collect();
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    #[test]
    fn parse_happy_returns_two_curves() {
        let curves = parse_treasury_csv(&fixture("treasury_happy.csv")).unwrap();
        assert_eq!(curves.len(), 2);
        assert_eq!(
            curves[0].date,
            NaiveDate::from_ymd_opt(2026, 4, 14).unwrap()
        );
        assert_eq!(
            curves[1].date,
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
        );
        // All 12 standard maturities should be present
        assert_eq!(curves[1].points.len(), 12);
    }

    #[test]
    fn parse_happy_ten_year_continuous_rate() {
        let curves = parse_treasury_csv(&fixture("treasury_happy.csv")).unwrap();
        let c = &curves[1]; // 04/15/2026: 10Y BEY = 3.87%
        let r = c.points[&3650];
        // BEY 3.87% → APY = (1 + 0.0387/2)^2 - 1, r_cont = ln(1+APY)
        let bey = 0.0387_f64;
        let apy = (1.0 + bey / 2.0).powi(2) - 1.0;
        let expected = (1.0 + apy).ln();
        assert!((r - expected).abs() < 1e-9, "r={r} expected={expected}");
    }

    #[test]
    fn parse_partial_fills_present_columns_only() {
        let curves = parse_treasury_csv(&fixture("treasury_partial.csv")).unwrap();
        assert_eq!(curves.len(), 1);
        // treasury_partial.csv has: 1Mo, 3Mo, 6Mo, 1Yr, 10Yr → days 30,91,182,365,3650
        let c = &curves[0];
        for &days in &[30u32, 91, 182, 365, 3650] {
            assert!(c.points.contains_key(&days), "missing {days}d");
        }
        // 2Mo, 2Yr, 3Yr, 5Yr, 7Yr, 20Yr, 30Yr → absent
        for &days in &[60u32, 730, 1095, 1825, 2555, 7300, 10950] {
            assert!(!c.points.contains_key(&days), "unexpected {days}d");
        }
    }

    #[test]
    fn parse_empty_returns_empty_vec() {
        let curves = parse_treasury_csv(&fixture("treasury_empty.csv")).unwrap();
        assert!(curves.is_empty());
    }
}
