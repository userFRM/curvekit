//! NY Fed SOFR fetcher.
//!
//! Pulls the Secured Overnight Financing Rate from the NY Fed's public API:
//!
//! `https://markets.newyorkfed.org/api/rates/secured/sofr/search.csv
//!  ?startDate=MM/DD/YYYY&endDate=MM/DD/YYYY`
//!
//! The published rate is a percentage; curvekit converts to continuously
//! compounded: `r = ln(1 + rate_pct / 100)`.
//!
//! # Refresh schedule
//!
//! The NY Fed publishes SOFR at approximately **08:00 ET** on each business day.

use async_trait::async_trait;
use chrono::NaiveDate;
use tracing::warn;

use crate::curve::SofrRate;
use crate::error::{Error, Result};

/// Trait for fetching SOFR observations. Implemented by [`HttpSofrFetcher`];
/// test stubs can provide alternative implementations.
#[async_trait]
pub trait SofrFetcher: Send + Sync {
    /// Fetch SOFR observations for every business day in `[start, end]`
    /// (YYYYMMDD integers).
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<SofrRate>>;
}

/// Parse a NY Fed SOFR CSV response into [`SofrRate`] values.
///
/// Expected header columns include at least:
/// - `Effective Date` — format `MM/DD/YYYY`
/// - `Rate (%)` — the published SOFR percentage
///
/// Unparseable rows are skipped with a `tracing::warn!`.
pub fn parse_sofr_csv(csv: &str) -> Result<Vec<SofrRate>> {
    let mut lines = csv.lines();
    let header = lines
        .next()
        .ok_or_else(|| Error::Sofr("empty SOFR CSV".into()))?;

    let headers: Vec<&str> = header.split(',').map(str::trim).collect();

    let date_idx = headers
        .iter()
        .position(|h| *h == "Effective Date")
        .ok_or_else(|| Error::Sofr("SOFR CSV missing 'Effective Date' column".into()))?;

    let rate_idx = headers
        .iter()
        .position(|h| *h == "Rate (%)")
        .ok_or_else(|| Error::Sofr("SOFR CSV missing 'Rate (%)' column".into()))?;

    let mut rates = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();

        let date_str = match fields.get(date_idx) {
            Some(s) => *s,
            None => {
                warn!(row = %line, "SOFR: row missing date field, skipping");
                continue;
            }
        };
        let date = match parse_ny_fed_date(date_str) {
            Some(d) => d,
            None => {
                warn!(row = %line, date = %date_str, "SOFR: unparseable date, skipping");
                continue;
            }
        };

        let rate_str = match fields.get(rate_idx) {
            Some(s) => *s,
            None => {
                warn!(row = %line, "SOFR: row missing rate field, skipping");
                continue;
            }
        };
        let rate_pct: f64 = match rate_str.parse() {
            Ok(v) => v,
            Err(_) => {
                warn!(row = %line, rate = %rate_str, "SOFR: unparseable rate, skipping");
                continue;
            }
        };

        // SOFR is published as a percentage; convert to continuously compounded.
        let rate = (1.0 + rate_pct / 100.0).ln();

        rates.push(SofrRate { date, rate });
    }

    Ok(rates)
}

fn parse_ny_fed_date(s: &str) -> Option<NaiveDate> {
    // Format: "MM/DD/YYYY"
    let mut parts = s.split('/');
    let mm: u32 = parts.next()?.parse().ok()?;
    let dd: u32 = parts.next()?.parse().ok()?;
    let yyyy: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // Extra components → reject
    }
    NaiveDate::from_ymd_opt(yyyy, mm, dd)
}

fn yyyymmdd_to_slash(yyyymmdd: u32) -> String {
    let y = yyyymmdd / 10000;
    let m = (yyyymmdd / 100) % 100;
    let d = yyyymmdd % 100;
    format!("{m:02}/{d:02}/{y:04}")
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

/// Fetches SOFR rates from the NY Fed's public CSV endpoint.
pub struct HttpSofrFetcher {
    client: reqwest::Client,
}

impl HttpSofrFetcher {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("curvekit/0.1 (github.com/userFRM/curvekit)")
            .build()?;
        Ok(Self { client })
    }

    fn url(start: u32, end: u32) -> String {
        let s = yyyymmdd_to_slash(start);
        let e = yyyymmdd_to_slash(end);
        format!(
            "https://markets.newyorkfed.org/api/rates/secured/sofr/search.csv\
             ?startDate={s}&endDate={e}"
        )
    }
}

#[async_trait]
impl SofrFetcher for HttpSofrFetcher {
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<SofrRate>> {
        if start > end {
            return Err(Error::Sofr(format!(
                "invalid date range: start {start} > end {end}"
            )));
        }
        let url = Self::url(start, end);
        let body = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_sofr_csv(&body)
    }
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
    fn parse_happy_returns_two_rates() {
        let rates = parse_sofr_csv(&fixture("sofr_happy.csv")).unwrap();
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0].date, NaiveDate::from_ymd_opt(2026, 4, 14).unwrap());
        assert_eq!(rates[1].date, NaiveDate::from_ymd_opt(2026, 4, 15).unwrap());
        // Rate (%) = 4.33 → continuous = ln(1 + 0.0433)
        let expected = (1.0_f64 + 0.0433).ln();
        assert!((rates[1].rate - expected).abs() < 1e-9);
    }

    #[test]
    fn parse_empty_returns_empty_vec() {
        let rates = parse_sofr_csv(&fixture("sofr_empty.csv")).unwrap();
        assert!(rates.is_empty());
    }

    #[test]
    fn slash_format_roundtrip() {
        assert_eq!(yyyymmdd_to_slash(20260415), "04/15/2026");
    }
}
