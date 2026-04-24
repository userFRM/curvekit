//! OBFR (Overnight Bank Funding Rate) fetcher.
//!
//! The Overnight Bank Funding Rate is a measure of wholesale, unsecured,
//! overnight bank funding. Published daily by the Federal Reserve Bank of New
//! York. Coverage starts 2016-03-01.
//!
//! # Data source
//!
//! NY Fed Markets API (JSON):
//! ```text
//! https://markets.newyorkfed.org/api/rates/unsecured/obfr/search.json
//!   ?startDate=YYYY-MM-DD&endDate=YYYY-MM-DD
//! ```
//!
//! # Rate conversion
//!
//! The NY Fed publishes percentage rates. curvekit converts to continuously
//! compounded: `r = ln(1 + rate_pct / 100)`.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use tracing::warn;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single OBFR observation.
#[derive(Debug, Clone, PartialEq)]
pub struct ObfrDay {
    pub date: NaiveDate,
    /// Continuously-compounded rate (converted from the published percentage).
    pub rate: f64,
}

// ---------------------------------------------------------------------------
// Fetcher trait
// ---------------------------------------------------------------------------

/// Trait for fetching OBFR observations.
#[async_trait]
pub trait ObfrFetcher: Send + Sync {
    /// Fetch OBFR observations for every business day in `[start, end]`
    /// (YYYYMMDD integers).
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<ObfrDay>>;
}

// ---------------------------------------------------------------------------
// JSON response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct NyFedResponse {
    #[serde(rename = "refRates")]
    ref_rates: Vec<NyFedRateEntry>,
}

#[derive(Deserialize)]
struct NyFedRateEntry {
    #[serde(rename = "effectiveDate")]
    effective_date: String,
    #[serde(rename = "percentRate")]
    percent_rate: f64,
}

// ---------------------------------------------------------------------------
// Parse helper
// ---------------------------------------------------------------------------

/// Parse the NY Fed JSON response body into [`ObfrDay`] values.
pub fn parse_obfr_json(json: &str) -> Result<Vec<ObfrDay>> {
    let resp: NyFedResponse = serde_json::from_str(json)
        .map_err(|e| Error::Other(format!("OBFR JSON parse error: {e}")))?;

    let mut days = Vec::with_capacity(resp.ref_rates.len());
    for entry in resp.ref_rates {
        match parse_iso_date(&entry.effective_date) {
            Some(date) => {
                let rate = (1.0 + entry.percent_rate / 100.0).ln();
                days.push(ObfrDay { date, rate });
            }
            None => {
                warn!(date = %entry.effective_date, "OBFR: unparseable date, skipping");
            }
        }
    }
    Ok(days)
}

fn parse_iso_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

fn yyyymmdd_to_iso(v: u32) -> String {
    let y = v / 10000;
    let m = (v / 100) % 100;
    let d = v % 100;
    format!("{y:04}-{m:02}-{d:02}")
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

/// Fetches OBFR rates from the NY Fed public API.
pub struct HttpObfrFetcher {
    client: reqwest::Client,
}

impl HttpObfrFetcher {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("curvekit/1.0 (+https://github.com/userFRM/curvekit)")
            .build()?;
        Ok(Self { client })
    }

    fn url(start: u32, end: u32) -> String {
        format!(
            "https://markets.newyorkfed.org/api/rates/unsecured/obfr/search.json\
             ?startDate={}&endDate={}",
            yyyymmdd_to_iso(start),
            yyyymmdd_to_iso(end)
        )
    }
}

#[async_trait]
impl ObfrFetcher for HttpObfrFetcher {
    async fn fetch(&self, start: u32, end: u32) -> Result<Vec<ObfrDay>> {
        if start > end {
            return Err(Error::Other(format!(
                "OBFR: invalid date range: start {start} > end {end}"
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
        parse_obfr_json(&body)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON: &str = r#"{
        "refRates": [
            {"effectiveDate": "2026-04-14", "percentRate": 4.33},
            {"effectiveDate": "2026-04-15", "percentRate": 4.34}
        ]
    }"#;

    #[test]
    fn parse_happy_returns_two_rates() {
        let days = parse_obfr_json(SAMPLE_JSON).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[1].date, NaiveDate::from_ymd_opt(2026, 4, 15).unwrap());
        let expected = (1.0_f64 + 0.0434).ln();
        assert!(
            (days[1].rate - expected).abs() < 1e-9,
            "rate={}",
            days[1].rate
        );
    }

    #[test]
    fn parse_empty_refrates() {
        let json = r#"{"refRates": []}"#;
        let days = parse_obfr_json(json).unwrap();
        assert!(days.is_empty());
    }

    #[test]
    fn parse_bad_date_skipped() {
        let json = r#"{"refRates": [{"effectiveDate": "bad", "percentRate": 4.33}]}"#;
        let days = parse_obfr_json(json).unwrap();
        assert!(days.is_empty());
    }

    #[test]
    fn url_format() {
        let url = HttpObfrFetcher::url(20260101, 20261231);
        assert!(url.contains("startDate=2026-01-01"), "url={url}");
        assert!(url.contains("endDate=2026-12-31"), "url={url}");
        assert!(
            url.contains("obfr"),
            "url should target OBFR endpoint: {url}"
        );
    }
}
