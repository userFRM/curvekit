//! `curvekit-rpc` — JSON-RPC 2.0 + REST handlers for the curvekit rate service.
//!
//! This crate provides:
//!
//! - [`methods`] — JSON-RPC 2.0 dispatch (`POST /rpc`)
//! - [`rest`] — REST endpoint handlers (`GET /treasury/...`, `/sofr/...`, etc.)
//! - [`build_router`] — constructs the full axum [`Router`]
//! - [`AppState`] — shared state (cache + refresh timestamps)

pub mod methods;
pub mod rest;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use curvekit_core::RateCache;
use methods::{dispatch, RpcRequest};

/// Shared state injected into all axum handlers.
pub struct AppState {
    pub cache: RateCache,
    pub last_treasury_refresh: Mutex<Option<DateTime<Utc>>>,
    pub last_sofr_refresh: Mutex<Option<DateTime<Utc>>>,
}

impl AppState {
    pub fn new(cache: RateCache) -> Self {
        Self {
            cache,
            last_treasury_refresh: Mutex::new(None),
            last_sofr_refresh: Mutex::new(None),
        }
    }
}

/// Build the full axum router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // JSON-RPC 2.0 endpoint
        .route("/rpc", post(rpc_handler))
        // REST mirrors
        .route("/treasury/curve/:date", get(rest::get_treasury_curve))
        .route("/treasury/range", get(rest::get_treasury_range))
        .route("/sofr/:date", get(rest::get_sofr))
        .route("/health", get(rest::get_health))
        .route("/openapi.json", get(rest::get_openapi))
        .with_state(state)
}

async fn rpc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<Value>,
) -> (StatusCode, Json<Value>) {
    // Accept single request or batch array.
    if let Value::Array(reqs) = req {
        let mut responses = Vec::new();
        for item in reqs {
            let resp = dispatch_one(item, Arc::clone(&state)).await;
            responses.push(serde_json::to_value(resp).unwrap_or(Value::Null));
        }
        return (StatusCode::OK, Json(Value::Array(responses)));
    }

    let resp = dispatch_one(req, state).await;
    (
        StatusCode::OK,
        Json(serde_json::to_value(resp).unwrap_or(Value::Null)),
    )
}

async fn dispatch_one(raw: Value, state: Arc<AppState>) -> methods::RpcResponse {
    match serde_json::from_value::<RpcRequest>(raw) {
        Ok(req) => dispatch(req, state).await,
        Err(e) => methods::RpcResponse::err(None, -32700, format!("Parse error: {e}")),
    }
}
