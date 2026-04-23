//! REST endpoint handlers — mirrors the JSON-RPC methods for `curl` users.
//!
//! # Routes
//!
//! | Method | Path | Description |
//! |---|---|---|
//! | GET | `/treasury/curve/:date` | Treasury yield curve for a date |
//! | GET | `/treasury/range` | Curves for date range (?start=&end=) |
//! | GET | `/sofr/:date` | SOFR rate for a date |
//! | GET | `/health` | Service health / cache row counts |
//! | GET | `/openapi.json` | OpenAPI 3.0 spec |

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_date(s: &str) -> Result<NaiveDate, (StatusCode, String)> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| (StatusCode::BAD_REQUEST, format!("Invalid date: {s}")))
}

fn cache_err(e: impl std::fmt::Display) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Cache error: {e}"),
    )
}

fn not_found(msg: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, msg.into())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub async fn get_treasury_curve(
    State(state): State<Arc<AppState>>,
    Path(date_str): Path<String>,
) -> impl IntoResponse {
    match parse_date(&date_str) {
        Err((code, msg)) => (code, msg).into_response(),
        Ok(date) => match state.cache.get_treasury(date) {
            Ok(Some(curve)) => Json(curve).into_response(),
            Ok(None) => not_found(format!("No treasury data for {date_str}")).into_response(),
            Err(e) => cache_err(e).into_response(),
        },
    }
}

#[derive(Deserialize)]
pub struct RangeParams {
    pub start: String,
    pub end: String,
}

pub async fn get_treasury_range(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RangeParams>,
) -> impl IntoResponse {
    let start = match parse_date(&params.start) {
        Ok(d) => d,
        Err((code, msg)) => return (code, msg).into_response(),
    };
    let end = match parse_date(&params.end) {
        Ok(d) => d,
        Err((code, msg)) => return (code, msg).into_response(),
    };
    match state.cache.get_treasury_range(start, end) {
        Ok(curves) => Json(curves).into_response(),
        Err(e) => cache_err(e).into_response(),
    }
}

pub async fn get_sofr(
    State(state): State<Arc<AppState>>,
    Path(date_str): Path<String>,
) -> impl IntoResponse {
    match parse_date(&date_str) {
        Err((code, msg)) => (code, msg).into_response(),
        Ok(date) => match state.cache.get_sofr(date) {
            Ok(Some(rate)) => Json(rate).into_response(),
            Ok(None) => not_found(format!("No SOFR data for {date_str}")).into_response(),
            Err(e) => cache_err(e).into_response(),
        },
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub treasury_rows: u64,
    pub sofr_rows: u64,
}

pub async fn get_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let treasury_rows = state.cache.treasury_row_count().unwrap_or(0);
    let sofr_rows = state.cache.sofr_row_count().unwrap_or(0);
    Json(HealthResponse {
        status: "ok",
        treasury_rows,
        sofr_rows,
    })
}

pub async fn get_openapi() -> impl IntoResponse {
    const SPEC: &str = include_str!("openapi.json");
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        SPEC,
    )
}
