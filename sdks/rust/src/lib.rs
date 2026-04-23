//! `curvekit-sdk` — Rust client for the curvekit rate service.
//!
//! # Quick start
//!
//! ```no_run
//! use curvekit_sdk::CurvekitClient;
//! use chrono::NaiveDate;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = CurvekitClient::new("http://localhost:8080");
//!     let date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
//!     let curve = client.treasury_curve(date).await?;
//!     println!("{:?}", curve);
//!     Ok(())
//! }
//! ```

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

// Re-export core types so callers don't need a separate curvekit-core dep.
pub use curvekit_core::{SofrRate, YieldCurve};

/// Health information returned by the service.
#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub treasury_rows: u64,
    pub sofr_rows: u64,
}

/// HTTP client for the curvekit rate service.
///
/// All methods talk to the server via JSON-RPC 2.0 (`POST /rpc`).
#[derive(Clone)]
pub struct CurvekitClient {
    base_url: String,
    http: reqwest::Client,
}

impl CurvekitClient {
    /// Create a new client pointing at `base_url`
    /// (e.g. `"http://localhost:8080"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .user_agent("curvekit-sdk/0.1")
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Fetch the Treasury yield curve for `date`.
    pub async fn treasury_curve(&self, date: NaiveDate) -> Result<YieldCurve> {
        let result = self
            .rpc(
                "rates.treasury_curve",
                json!({"date": date.format("%Y-%m-%d").to_string()}),
            )
            .await?;
        serde_json::from_value(result).context("deserializing YieldCurve")
    }

    /// Fetch the SOFR rate for `date`.
    pub async fn sofr(&self, date: NaiveDate) -> Result<f64> {
        let result = self
            .rpc(
                "rates.sofr",
                json!({"date": date.format("%Y-%m-%d").to_string()}),
            )
            .await?;
        let rate: SofrRate = serde_json::from_value(result).context("deserializing SofrRate")?;
        Ok(rate.rate)
    }

    /// Fetch the continuously-compounded risk-free rate for `days` to maturity
    /// on `date`, interpolated from the Treasury curve (+ SOFR anchor at 1 day).
    pub async fn rate_for_days(&self, date: NaiveDate, days: u32) -> Result<f64> {
        let curve = self.treasury_curve(date).await?;
        let sofr = self.sofr(date).await.ok();

        // Build combined point set: SOFR at 1 day + treasury curve.
        use std::collections::BTreeMap;
        let mut pts: BTreeMap<u32, f64> = curve.points.clone();
        if let Some(r) = sofr {
            pts.insert(1, r);
        }

        curvekit_core::interpolation::linear(&pts, days)
            .context("interpolation failed: empty curve")
    }

    /// Fetch Treasury curves for all dates in `[start, end]`.
    pub async fn treasury_range(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<YieldCurve>> {
        let result = self
            .rpc(
                "rates.treasury_range",
                json!({
                    "start": start.format("%Y-%m-%d").to_string(),
                    "end":   end.format("%Y-%m-%d").to_string(),
                }),
            )
            .await?;
        serde_json::from_value(result).context("deserializing Vec<YieldCurve>")
    }

    /// Query service health and cache statistics.
    pub async fn health(&self) -> Result<HealthResponse> {
        let result = self.rpc("rates.health", json!({})).await?;
        serde_json::from_value(result).context("deserializing HealthResponse")
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp: Value = self
            .http
            .post(format!("{}/rpc", self.base_url))
            .json(&req)
            .send()
            .await
            .with_context(|| format!("POST /rpc ({method})"))?
            .error_for_status()
            .with_context(|| format!("non-2xx from /rpc ({method})"))?
            .json()
            .await
            .with_context(|| format!("reading /rpc body ({method})"))?;

        if let Some(err) = resp.get("error") {
            bail!(
                "RPC error for {method}: {}",
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            );
        }

        resp.get("result")
            .cloned()
            .context("missing 'result' in RPC response")
    }
}
