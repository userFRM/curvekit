//! SQLite-backed cache for Treasury curves and SOFR rates.
//!
//! The cache is a **convenience layer**, not the source of truth. On restart,
//! the server repopulates from live sources. The cache reduces latency for
//! repeated queries within a session and avoids hammering public APIs.
//!
//! # Schema
//!
//! ```sql
//! CREATE TABLE treasury_curves (
//!     date       TEXT NOT NULL,        -- ISO 8601 (YYYY-MM-DD)
//!     tenor_days INTEGER NOT NULL,
//!     rate_cont  REAL NOT NULL,        -- continuously compounded
//!     PRIMARY KEY (date, tenor_days)
//! );
//!
//! CREATE TABLE sofr_rates (
//!     date      TEXT NOT NULL PRIMARY KEY,  -- ISO 8601
//!     rate_cont REAL NOT NULL               -- continuously compounded
//! );
//! ```

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use crate::curve::{SofrRate, YieldCurve};
use crate::error::{CacheError, Result};

/// SQLite-backed rate cache. Thread-safe via an internal `Mutex`.
pub struct RateCache {
    conn: Mutex<Connection>,
}

impl RateCache {
    /// Open (or create) a cache database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(CacheError::Sqlite)?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.create_schema()?;
        Ok(cache)
    }

    /// Open an in-memory cache (useful for tests and ephemeral sessions).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(CacheError::Sqlite)?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.create_schema()?;
        Ok(cache)
    }

    fn create_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous  = NORMAL;

            CREATE TABLE IF NOT EXISTS treasury_curves (
                date       TEXT    NOT NULL,
                tenor_days INTEGER NOT NULL,
                rate_cont  REAL    NOT NULL,
                PRIMARY KEY (date, tenor_days)
            );

            CREATE TABLE IF NOT EXISTS sofr_rates (
                date      TEXT NOT NULL PRIMARY KEY,
                rate_cont REAL NOT NULL
            );
            ",
        )
        .map_err(CacheError::Sqlite)?;
        Ok(())
    }

    // ── Treasury ───────────────────────────────────────────────────────────

    /// Store (or replace) a Treasury yield curve in the cache.
    pub fn store_treasury(&self, curve: &YieldCurve) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let date_str = curve.date.format("%Y-%m-%d").to_string();
        let tx = conn.unchecked_transaction().map_err(CacheError::Sqlite)?;
        for (&tenor_days, &rate) in &curve.points {
            tx.execute(
                "INSERT OR REPLACE INTO treasury_curves (date, tenor_days, rate_cont)
                 VALUES (?1, ?2, ?3)",
                params![date_str, tenor_days, rate],
            )
            .map_err(CacheError::Sqlite)?;
        }
        tx.commit().map_err(CacheError::Sqlite)?;
        Ok(())
    }

    /// Retrieve a Treasury yield curve for the given date.
    /// Returns `None` if the date is not cached.
    pub fn get_treasury(&self, date: NaiveDate) -> Result<Option<YieldCurve>> {
        let conn = self.conn.lock().unwrap();
        let date_str = date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT tenor_days, rate_cont FROM treasury_curves WHERE date = ?1 ORDER BY tenor_days",
            )
            .map_err(CacheError::Sqlite)?;
        let rows: Vec<(u32, f64)> = stmt
            .query_map(params![date_str], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(CacheError::Sqlite)?
            .collect::<std::result::Result<_, _>>()
            .map_err(CacheError::Sqlite)?;

        if rows.is_empty() {
            return Ok(None);
        }
        let points: BTreeMap<u32, f64> = rows.into_iter().collect();
        Ok(Some(YieldCurve { date, points }))
    }

    /// Retrieve Treasury curves for all cached dates in `[start, end]`.
    pub fn get_treasury_range(&self, start: NaiveDate, end: NaiveDate) -> Result<Vec<YieldCurve>> {
        let conn = self.conn.lock().unwrap();
        let start_str = start.format("%Y-%m-%d").to_string();
        let end_str = end.format("%Y-%m-%d").to_string();

        let mut stmt = conn
            .prepare(
                "SELECT date, tenor_days, rate_cont
                 FROM treasury_curves
                 WHERE date >= ?1 AND date <= ?2
                 ORDER BY date, tenor_days",
            )
            .map_err(CacheError::Sqlite)?;

        let rows: Vec<(String, u32, f64)> = stmt
            .query_map(params![start_str, end_str], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(CacheError::Sqlite)?
            .collect::<std::result::Result<_, _>>()
            .map_err(CacheError::Sqlite)?;

        // Group by date.
        let mut map: std::collections::BTreeMap<String, BTreeMap<u32, f64>> =
            std::collections::BTreeMap::new();
        for (date_str, tenor, rate) in rows {
            map.entry(date_str).or_default().insert(tenor, rate);
        }

        let curves = map
            .into_iter()
            .filter_map(|(date_str, points)| {
                let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").ok()?;
                Some(YieldCurve { date, points })
            })
            .collect();

        Ok(curves)
    }

    // ── SOFR ───────────────────────────────────────────────────────────────

    /// Store (or replace) a SOFR rate in the cache.
    pub fn store_sofr(&self, rate: &SofrRate) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let date_str = rate.date.format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT OR REPLACE INTO sofr_rates (date, rate_cont) VALUES (?1, ?2)",
            params![date_str, rate.rate],
        )
        .map_err(CacheError::Sqlite)?;
        Ok(())
    }

    /// Retrieve the SOFR rate for the given date. Returns `None` if not cached.
    pub fn get_sofr(&self, date: NaiveDate) -> Result<Option<SofrRate>> {
        let conn = self.conn.lock().unwrap();
        let date_str = date.format("%Y-%m-%d").to_string();
        let rate_opt: Option<f64> = conn
            .query_row(
                "SELECT rate_cont FROM sofr_rates WHERE date = ?1",
                params![date_str],
                |row| row.get(0),
            )
            .optional()
            .map_err(CacheError::Sqlite)?;
        Ok(rate_opt.map(|rate| SofrRate { date, rate }))
    }

    // ── Diagnostics ────────────────────────────────────────────────────────

    /// Number of (date, tenor) rows in the Treasury table.
    pub fn treasury_row_count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM treasury_curves", [], |r| r.get(0))
            .map_err(CacheError::Sqlite)?;
        Ok(n as u64)
    }

    /// Number of rows in the SOFR table.
    pub fn sofr_row_count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM sofr_rates", [], |r| r.get(0))
            .map_err(CacheError::Sqlite)?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_curve(d: NaiveDate) -> YieldCurve {
        let mut c = YieldCurve::new(d);
        c.insert(365, 0.04);
        c.insert(3650, 0.05);
        c
    }

    #[test]
    fn treasury_store_and_retrieve() {
        let cache = RateCache::in_memory().unwrap();
        let d = date(2026, 4, 15);
        let curve = make_curve(d);
        cache.store_treasury(&curve).unwrap();
        let retrieved = cache.get_treasury(d).unwrap().unwrap();
        assert_eq!(retrieved.date, d);
        assert_eq!(retrieved.points.len(), 2);
        assert!((retrieved.points[&365] - 0.04).abs() < 1e-12);
    }

    #[test]
    fn treasury_missing_returns_none() {
        let cache = RateCache::in_memory().unwrap();
        let result = cache.get_treasury(date(2000, 1, 1)).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn treasury_replace_on_duplicate() {
        let cache = RateCache::in_memory().unwrap();
        let d = date(2026, 4, 15);
        let mut c1 = YieldCurve::new(d);
        c1.insert(365, 0.04);
        cache.store_treasury(&c1).unwrap();
        let mut c2 = YieldCurve::new(d);
        c2.insert(365, 0.05);
        cache.store_treasury(&c2).unwrap();
        let r = cache.get_treasury(d).unwrap().unwrap();
        assert!((r.points[&365] - 0.05).abs() < 1e-12);
    }

    #[test]
    fn sofr_store_and_retrieve() {
        let cache = RateCache::in_memory().unwrap();
        let sr = SofrRate {
            date: date(2026, 4, 15),
            rate: 0.043,
        };
        cache.store_sofr(&sr).unwrap();
        let out = cache.get_sofr(date(2026, 4, 15)).unwrap().unwrap();
        assert!((out.rate - 0.043).abs() < 1e-12);
    }

    #[test]
    fn sofr_missing_returns_none() {
        let cache = RateCache::in_memory().unwrap();
        assert!(cache.get_sofr(date(1990, 1, 1)).unwrap().is_none());
    }

    #[test]
    fn treasury_range_query() {
        let cache = RateCache::in_memory().unwrap();
        for d in [date(2026, 4, 14), date(2026, 4, 15), date(2026, 4, 16)] {
            cache.store_treasury(&make_curve(d)).unwrap();
        }
        let range = cache
            .get_treasury_range(date(2026, 4, 14), date(2026, 4, 15))
            .unwrap();
        assert_eq!(range.len(), 2);
    }

    #[test]
    fn row_counts_increment() {
        let cache = RateCache::in_memory().unwrap();
        assert_eq!(cache.treasury_row_count().unwrap(), 0);
        assert_eq!(cache.sofr_row_count().unwrap(), 0);
        let d = date(2026, 4, 15);
        cache.store_treasury(&make_curve(d)).unwrap();
        cache
            .store_sofr(&SofrRate {
                date: d,
                rate: 0.04,
            })
            .unwrap();
        assert_eq!(cache.treasury_row_count().unwrap(), 2); // 2 tenor points
        assert_eq!(cache.sofr_row_count().unwrap(), 1);
    }
}
