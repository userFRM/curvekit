//! Unified error type for curvekit.
//!
//! All public methods return `curvekit::Result<T>` which is
//! `std::result::Result<T, curvekit::Error>`.

use thiserror::Error;

pub use crate::date::DateError;

/// The single unified error type for curvekit operations.
///
/// Match on this enum when you need to distinguish error kinds; otherwise
/// `?` propagates it through any `Result<_, curvekit::Error>` context.
#[derive(Debug, Error)]
pub enum Error {
    /// A date string or integer could not be converted to a valid calendar date.
    #[error("date error: {0}")]
    Date(#[from] DateError),

    /// Treasury data fetch or parse failed.
    #[error("treasury error: {0}")]
    Treasury(String),

    /// SOFR data fetch or parse failed.
    #[error("sofr error: {0}")]
    Sofr(String),

    /// Parquet file read or write failed.
    #[error("parquet I/O error: {0}")]
    Parquet(String),

    /// Linear or cubic-spline interpolation produced no result (empty curve).
    #[error("interpolation error: {0}")]
    Interpolation(String),

    /// The requested date is not present in the available data
    /// (weekend, holiday, or outside coverage).
    #[error("no data for date: {0}")]
    DateNotFound(String),

    /// Underlying HTTP transport error.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// I/O error (file system, tempfile, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Arrow columnar format error (from parquet reading).
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// Native parquet crate error.
    #[error("parquet error: {0}")]
    ParquetNative(#[from] parquet::errors::ParquetError),

    /// SHA-256 digest of a fetched file does not match the manifest entry.
    ///
    /// The corrupt bytes were NOT written to the on-disk cache.
    #[error("checksum mismatch for {file}: expected sha256:{expected} got sha256:{actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    /// Any other error not covered by the specific variants above.
    #[error("{0}")]
    Other(String),
}

/// `Result<T>` alias using [`enum@Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

// ── Legacy compatibility shims ────────────────────────────────────────────────

/// Alias for [`enum@Error`] kept for backward compatibility.
///
/// Code that referenced `curvekit::CurvekitError` continues to compile.
pub type CurvekitError = Error;

/// Legacy source-specific errors — kept as internal helpers for the sources
/// modules. New code should use `Error` variants directly.
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

// Conversions from legacy source errors into unified Error.

impl From<TreasuryError> for Error {
    fn from(e: TreasuryError) -> Self {
        match e {
            TreasuryError::Http(inner) => Error::Http(inner),
            other => Error::Treasury(other.to_string()),
        }
    }
}

impl From<SofrError> for Error {
    fn from(e: SofrError) -> Self {
        match e {
            SofrError::Http(inner) => Error::Http(inner),
            other => Error::Sofr(other.to_string()),
        }
    }
}

impl From<ParquetError> for Error {
    fn from(e: ParquetError) -> Self {
        match e {
            ParquetError::Arrow(inner) => Error::Arrow(inner),
            ParquetError::Parquet(inner) => Error::ParquetNative(inner),
            ParquetError::Io(inner) => Error::Io(inner),
            ParquetError::Schema(s) => Error::Parquet(s),
        }
    }
}

// Allow `anyhow::Error` context to be converted for legacy internal code
// that still uses `anyhow`.  This is a one-way conversion.
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Other(e.to_string())
    }
}
