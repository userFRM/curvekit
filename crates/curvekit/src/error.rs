use thiserror::Error;

/// Errors produced by curvekit operations.
#[derive(Debug, Error)]
pub enum CurvekitError {
    #[error("Treasury fetch failed: {0}")]
    TreasuryFetch(#[from] TreasuryError),

    #[error("SOFR fetch failed: {0}")]
    SofrFetch(#[from] SofrError),

    #[error("Parquet I/O error: {0}")]
    Parquet(#[from] ParquetError),

    #[error("Interpolation error: {0}")]
    Interpolation(String),

    #[error("No data for date {0}")]
    DateNotFound(String),
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

/// Errors from parquet file operations.
#[derive(Debug, Error)]
pub enum ParquetError {
    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    #[error("Parquet file error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Schema error: {0}")]
    Schema(String),
}

pub type Result<T, E = CurvekitError> = std::result::Result<T, E>;
