//! ETag-aware HTTP fetcher for parquet files served from GitHub raw or any
//! compatible origin.
//!
//! # Cache layout
//!
//! ```text
//! $CURVEKIT_CACHE_DIR/          (default: XDG cache / curvekit)
//! ├── treasury-2020.parquet     ← cached body
//! ├── treasury-2020.parquet.etag
//! ├── sofr-2020.parquet
//! └── sofr-2020.parquet.etag
//! ```
//!
//! On each `fetch` call:
//!
//! 1. If cache file exists **and** a stored ETag is available, send
//!    `If-None-Match` with the stored ETag.
//! 2. `304 Not Modified` → return the cached bytes unchanged.
//! 3. `2xx` → write new body + new ETag to cache, return bytes.
//! 4. Non-2xx (not 304) → return `Err`.
//! 5. Network error and cache exists → warn + return stale cache.
//! 6. Network error and no cache → return `Err`.

use bytes::Bytes;
use reqwest::StatusCode;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// ETag-aware fetcher backed by a local disk cache.
pub(crate) struct CachedFetcher {
    pub http: reqwest::Client,
    pub base_url: String,
    pub cache_dir: PathBuf,
}

impl CachedFetcher {
    /// Fetch a parquet file by logical key (e.g. `"treasury-2020"`).
    ///
    /// The key is resolved to `{base_url}/{key}.parquet` on the wire and
    /// `{cache_dir}/{key}.parquet` on disk.
    pub async fn fetch(&self, key: &str) -> Result<Bytes> {
        let cache_path = self.cache_dir.join(format!("{key}.parquet"));
        let etag_path = self.cache_dir.join(format!("{key}.parquet.etag"));

        let mut req = self.http.get(format!("{}/{key}.parquet", self.base_url));

        if cache_path.exists() {
            if let Some(etag) = read_etag(&etag_path) {
                req = req.header("If-None-Match", etag);
            }
        }

        match req.send().await {
            Ok(resp) if resp.status() == StatusCode::NOT_MODIFIED => {
                // Cache is fresh.
                let bytes = tokio::fs::read(&cache_path).await?;
                Ok(bytes.into())
            }
            Ok(resp) if resp.status().is_success() => {
                let etag = resp
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                let bytes = resp.bytes().await?;
                // Write atomically: body first, then ETag.
                tokio::fs::create_dir_all(&self.cache_dir).await?;
                tokio::fs::write(&cache_path, &bytes).await?;
                if let Some(e) = etag {
                    tokio::fs::write(&etag_path, e).await?;
                }
                Ok(bytes)
            }
            Ok(resp) => Err(Error::Other(format!(
                "fetch {key}: HTTP {} {}",
                resp.status().as_u16(),
                resp.status().canonical_reason().unwrap_or("")
            ))),
            Err(e) if cache_path.exists() => {
                tracing::warn!(key, error = %e, "network fetch failed, using stale cache");
                let bytes = tokio::fs::read(&cache_path).await?;
                Ok(bytes.into())
            }
            Err(e) => Err(Error::Http(e)),
        }
    }
}

fn read_etag(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Resolve the cache directory.
///
/// Priority:
/// 1. `$CURVEKIT_CACHE_DIR` env var.
/// 2. XDG/platform cache dir for the `curvekit` application
///    (`directories::ProjectDirs`).
/// 3. Fallback: `~/.cache/curvekit` (or `%LOCALAPPDATA%\curvekit\cache` on Windows).
pub(crate) fn default_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CURVEKIT_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(proj) = directories::ProjectDirs::from("", "", "curvekit") {
        return proj.cache_dir().to_path_buf();
    }
    // Ultimate fallback.
    dirs_fallback()
}

fn dirs_fallback() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("curvekit").join("cache"))
            .unwrap_or_else(|_| PathBuf::from("curvekit-cache"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".cache").join("curvekit"))
            .unwrap_or_else(|_| PathBuf::from(".curvekit-cache"))
    }
}

/// Default base URL for GitHub raw content.
pub(crate) const DEFAULT_BASE_URL: &str =
    "https://raw.githubusercontent.com/userFRM/curvekit/main/data";

/// Resolve the base URL.
///
/// Checks `$CURVEKIT_BASE_URL` first (useful for tests or self-hosting).
pub(crate) fn resolved_base_url() -> String {
    std::env::var("CURVEKIT_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}
