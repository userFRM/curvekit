//! JSON-RPC 2.0 method dispatch.
//!
//! All methods are exposed via POST `/rpc`. Each method name follows the
//! `rates.<method>` convention.
//!
//! # Methods
//!
//! | Method | Params | Returns |
//! |---|---|---|
//! | `rates.treasury_curve` | `{"date": "YYYY-MM-DD"}` | `YieldCurve` |
//! | `rates.sofr` | `{"date": "YYYY-MM-DD"}` | `SofrRate` |
//! | `rates.treasury_range` | `{"start": "YYYY-MM-DD", "end": "YYYY-MM-DD"}` | `[YieldCurve]` |
//! | `rates.health` | `{}` | `HealthResponse` |

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use curvekit_core::RateCache;

use crate::AppState;

// ── JSON-RPC 2.0 envelope types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcResponse {
    pub fn ok(id: Option<Value>, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ── Health response ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub treasury_rows: u64,
    pub sofr_rows: u64,
}

// ── Method handler ──────────────────────────────────────────────────────────

pub async fn dispatch(req: RpcRequest, state: Arc<AppState>) -> RpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "rates.treasury_curve" => handle_treasury_curve(id, req.params, &state.cache).await,
        "rates.sofr" => handle_sofr(id, req.params, &state.cache).await,
        "rates.treasury_range" => handle_treasury_range(id, req.params, &state.cache).await,
        "rates.health" => handle_health(id, &state.cache).await,
        other => RpcResponse::err(id, -32601, format!("Method not found: {other}")),
    }
}

async fn handle_treasury_curve(id: Option<Value>, params: Value, cache: &RateCache) -> RpcResponse {
    #[derive(Deserialize)]
    struct P {
        date: String,
    }
    let p: P = match serde_json::from_value(params) {
        Ok(v) => v,
        Err(e) => return RpcResponse::err(id, -32602, format!("Invalid params: {e}")),
    };
    let date = match NaiveDate::parse_from_str(&p.date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return RpcResponse::err(id, -32602, format!("Invalid date: {}", p.date));
        }
    };
    match cache.get_treasury(date) {
        Ok(Some(curve)) => RpcResponse::ok(id, curve),
        Ok(None) => RpcResponse::err(id, -32000, format!("No data for {}", p.date)),
        Err(e) => RpcResponse::err(id, -32000, format!("Cache error: {e}")),
    }
}

async fn handle_sofr(id: Option<Value>, params: Value, cache: &RateCache) -> RpcResponse {
    #[derive(Deserialize)]
    struct P {
        date: String,
    }
    let p: P = match serde_json::from_value(params) {
        Ok(v) => v,
        Err(e) => return RpcResponse::err(id, -32602, format!("Invalid params: {e}")),
    };
    let date = match NaiveDate::parse_from_str(&p.date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return RpcResponse::err(id, -32602, format!("Invalid date: {}", p.date));
        }
    };
    match cache.get_sofr(date) {
        Ok(Some(rate)) => RpcResponse::ok(id, rate),
        Ok(None) => RpcResponse::err(id, -32000, format!("No SOFR data for {}", p.date)),
        Err(e) => RpcResponse::err(id, -32000, format!("Cache error: {e}")),
    }
}

async fn handle_treasury_range(id: Option<Value>, params: Value, cache: &RateCache) -> RpcResponse {
    #[derive(Deserialize)]
    struct P {
        start: String,
        end: String,
    }
    let p: P = match serde_json::from_value(params) {
        Ok(v) => v,
        Err(e) => return RpcResponse::err(id, -32602, format!("Invalid params: {e}")),
    };
    let start = match NaiveDate::parse_from_str(&p.start, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return RpcResponse::err(id, -32602, format!("Invalid start: {}", p.start)),
    };
    let end = match NaiveDate::parse_from_str(&p.end, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return RpcResponse::err(id, -32602, format!("Invalid end: {}", p.end)),
    };
    match cache.get_treasury_range(start, end) {
        Ok(curves) => RpcResponse::ok(id, curves),
        Err(e) => RpcResponse::err(id, -32000, format!("Cache error: {e}")),
    }
}

async fn handle_health(id: Option<Value>, cache: &RateCache) -> RpcResponse {
    let treasury_rows = cache.treasury_row_count().unwrap_or(0);
    let sofr_rows = cache.sofr_row_count().unwrap_or(0);
    RpcResponse::ok(
        id,
        HealthResponse {
            status: "ok".into(),
            treasury_rows,
            sofr_rows,
        },
    )
}
