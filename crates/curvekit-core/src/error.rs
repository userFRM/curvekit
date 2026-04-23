use thiserror::Error;

/// Errors produced by curvekit-core operations.
#[derive(Debug, Error)]
pub enum CurvekitError {
    #[error("Treasury fetch failed: {0}")]
    TreasuryFetch(#[from] TreasuryError),

    #[error("SOFR fetch failed: {0}")]
    SofrFetch(#[from] SofrError),

    #[error("Cache error: {0}")]
    Cache(#[from] CacheError),

    #[error("Interpolation error: {0}")]
    Interpolation(String),
}

/// Errors specific to the Treasury data source.
#[derive(Debug, Error)]
pub enum TreasuryError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("CSV parse error: {0}")]
    Parse(String),

    #[error("Empty response for date range {start}–{end}")]
    EmptyResponse { start: String, end: String },

    #[error("Invalid date range: start {start} > end {end}")]
    InvalidDateRange { start: u32, end: u32 },
}

/// Errors specific to the SOFR data source.
#[derive(Debug, Error)]
pub enum SofrError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("CSV parse error: {0}")]
    Parse(String),

    #[error("Empty response for date range {start}–{end}")]
    EmptyResponse { start: String, end: String },

    #[error("Invalid date range: start {start} > end {end}")]
    InvalidDateRange { start: u32, end: u32 },
}

/// Errors from SQLite cache operations.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T, E = CurvekitError> = std::result::Result<T, E>;
