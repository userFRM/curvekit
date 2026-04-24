//! Parquet reader/writer for bundled Treasury + SOFR data.
//!
//! # File layout
//!
//! ```text
//! data/
//! ├── treasury-{year}.parquet   (Date32, UInt32 tenor_days, UInt32 yield_bps)
//! └── sofr-{year}.parquet       (Date32, UInt32 rate_bps)
//! ```
//!
//! All rates are stored in **basis points** (`round(rate * 10_000)`) as `UInt32`
//! to avoid floating-point drift across parquet round-trips. The reader converts
//! back to f64 on the way out.
//!
//! Compression: ZSTD level 3. Row group size: 10 000 rows.

use arrow::array::{Date32Array, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::curve::{SofrDay, YieldCurve, YieldCurveDay};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ROW_GROUP_SIZE: usize = 10_000;
const SCALE: f64 = 10_000.0; // bps storage scale

// ---------------------------------------------------------------------------
// Schema helpers
// ---------------------------------------------------------------------------

fn treasury_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("date", DataType::Date32, false),
        Field::new("tenor_days", DataType::UInt32, false),
        Field::new("yield_bps", DataType::UInt32, false),
    ]))
}

fn sofr_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("date", DataType::Date32, false),
        Field::new("rate_bps", DataType::UInt32, false),
    ]))
}

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

/// Convert NaiveDate to days since Unix epoch (1970-01-01) — Arrow Date32.
fn to_date32(d: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    (d - epoch).num_days() as i32
}

/// Convert Arrow Date32 (days since epoch) to NaiveDate.
fn from_date32(days: i32) -> Option<NaiveDate> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    epoch.checked_add_signed(chrono::Duration::days(days as i64))
}

// ---------------------------------------------------------------------------
// Writer API
// ---------------------------------------------------------------------------

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            ZstdLevel::try_new(3).expect("valid zstd level"),
        ))
        .set_max_row_group_size(ROW_GROUP_SIZE)
        .build()
}

/// Write (or overwrite) one year of Treasury curves to `{data_dir}/treasury-{year}.parquet`.
///
/// Rows are sorted by (date, tenor_days) ascending.
pub fn write_treasury_year(data_dir: &Path, year: i32, curves: &[YieldCurveDay]) -> Result<()> {
    // Flatten and sort.
    let mut rows: Vec<(NaiveDate, u32, u32)> = Vec::new();
    for curve in curves {
        for (&tenor_days, &rate) in &curve.points {
            let yield_bps = (rate * SCALE).round() as u32;
            rows.push((curve.date, tenor_days, yield_bps));
        }
    }
    rows.sort_by_key(|&(d, t, _)| (d, t));

    let dates: Date32Array = rows.iter().map(|(d, _, _)| Some(to_date32(*d))).collect();
    let tenors: UInt32Array = rows.iter().map(|(_, t, _)| Some(*t)).collect();
    let yields: UInt32Array = rows.iter().map(|(_, _, y)| Some(*y)).collect();

    let batch = RecordBatch::try_new(
        treasury_schema(),
        vec![Arc::new(dates), Arc::new(tenors), Arc::new(yields)],
    )?;

    let path = data_dir.join(format!("treasury-{year}.parquet"));
    let file = fs::File::create(&path)?;
    let mut writer = ArrowWriter::try_new(file, treasury_schema(), Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;

    tracing::info!("wrote {} rows → {}", rows.len(), path.display());
    Ok(())
}

/// Write (or overwrite) one year of SOFR rates to `{data_dir}/sofr-{year}.parquet`.
pub fn write_sofr_year(data_dir: &Path, year: i32, rates: &[SofrDay]) -> Result<()> {
    let mut sorted = rates.to_vec();
    sorted.sort_by_key(|r| r.date);

    let dates: Date32Array = sorted.iter().map(|r| Some(to_date32(r.date))).collect();
    let bps: UInt32Array = sorted
        .iter()
        .map(|r| Some((r.rate * SCALE).round() as u32))
        .collect();

    let batch = RecordBatch::try_new(sofr_schema(), vec![Arc::new(dates), Arc::new(bps)])?;

    let path = data_dir.join(format!("sofr-{year}.parquet"));
    let file = fs::File::create(&path)?;
    let mut writer = ArrowWriter::try_new(file, sofr_schema(), Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;

    tracing::info!("wrote {} rows → {}", sorted.len(), path.display());
    Ok(())
}

/// Append one day's Treasury curve to the appropriate year file.
///
/// Reads the existing file (if present), merges the new day's rows (replacing
/// any existing rows for that date), then rewrites the whole year file.
pub fn append_treasury_day(data_dir: &Path, date: NaiveDate, curve: &YieldCurve) -> Result<()> {
    let year = date.format("%Y").to_string().parse::<i32>().unwrap();
    let path = data_dir.join(format!("treasury-{year}.parquet"));

    let mut existing = if path.exists() {
        read_treasury_year_raw(&path)?
    } else {
        Vec::new()
    };

    // Remove any existing rows for this date.
    existing.retain(|(d, _, _)| *d != date);

    // Add new rows.
    for (&tenor_days, &rate) in &curve.points {
        existing.push((date, tenor_days, (rate * SCALE).round() as u32));
    }

    // Reconstruct as YieldCurveDay vec and rewrite.
    let curves = rows_to_curves(existing);
    write_treasury_year(data_dir, year, &curves)
}

/// Append one day's SOFR rate to the appropriate year file.
pub fn append_sofr_day(data_dir: &Path, date: NaiveDate, rate: f64) -> Result<()> {
    let year = date.format("%Y").to_string().parse::<i32>().unwrap();
    let path = data_dir.join(format!("sofr-{year}.parquet"));

    let mut existing = if path.exists() {
        read_sofr_year_raw(&path)?
    } else {
        Vec::new()
    };

    existing.retain(|(d, _)| *d != date);
    existing.push((date, (rate * SCALE).round() as u32));

    let days: Vec<SofrDay> = existing
        .into_iter()
        .map(|(d, bps)| SofrDay {
            date: d,
            rate: bps as f64 / SCALE,
        })
        .collect();
    write_sofr_year(data_dir, year, &days)
}

// ---------------------------------------------------------------------------
// Reader API (internal helpers — public reader is in bundled.rs)
// ---------------------------------------------------------------------------

type TreasuryRow = (NaiveDate, u32, u32); // (date, tenor_days, yield_bps)
type SofrRow = (NaiveDate, u32); // (date, rate_bps)

fn read_treasury_year_raw(path: &Path) -> Result<Vec<TreasuryRow>> {
    let file = fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let dates = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .ok_or_else(|| Error::Parquet("date column type mismatch".into()))?;
        let tenors = batch
            .column(1)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .ok_or_else(|| Error::Parquet("tenor_days column type mismatch".into()))?;
        let yields = batch
            .column(2)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .ok_or_else(|| Error::Parquet("yield_bps column type mismatch".into()))?;

        for i in 0..batch.num_rows() {
            if let Some(d) = from_date32(dates.value(i)) {
                rows.push((d, tenors.value(i), yields.value(i)));
            }
        }
    }
    Ok(rows)
}

fn read_sofr_year_raw(path: &Path) -> Result<Vec<SofrRow>> {
    let file = fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let dates = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .ok_or_else(|| Error::Parquet("date column type mismatch".into()))?;
        let bps_col = batch
            .column(1)
            .as_any()
            .downcast_ref::<UInt32Array>()
            .ok_or_else(|| Error::Parquet("rate_bps column type mismatch".into()))?;

        for i in 0..batch.num_rows() {
            if let Some(d) = from_date32(dates.value(i)) {
                rows.push((d, bps_col.value(i)));
            }
        }
    }
    Ok(rows)
}

/// Group flat `(date, tenor_days, yield_bps)` rows into per-date YieldCurves.
pub(crate) fn rows_to_curves(mut rows: Vec<TreasuryRow>) -> Vec<YieldCurveDay> {
    rows.sort_by_key(|&(d, t, _)| (d, t));
    let mut map: BTreeMap<NaiveDate, BTreeMap<u32, f64>> = BTreeMap::new();
    for (date, tenor_days, yield_bps) in rows {
        map.entry(date)
            .or_default()
            .insert(tenor_days, yield_bps as f64 / SCALE);
    }
    map.into_iter()
        .map(|(date, points)| YieldCurve { date, points })
        .collect()
}

// ---------------------------------------------------------------------------
// Public read helpers (used by bundled.rs)
// ---------------------------------------------------------------------------

/// Read all Treasury curves from one year's parquet file. Returns sorted vec.
pub fn read_treasury_year(path: &Path) -> Result<Vec<YieldCurveDay>> {
    let rows = read_treasury_year_raw(path)?;
    Ok(rows_to_curves(rows))
}

/// Read all SOFR rates from one year's parquet file. Returns sorted vec.
pub fn read_sofr_year(path: &Path) -> Result<Vec<SofrDay>> {
    let mut rows = read_sofr_year_raw(path)?;
    rows.sort_by_key(|(d, _)| *d);
    Ok(rows
        .into_iter()
        .map(|(date, bps)| SofrDay {
            date,
            rate: bps as f64 / SCALE,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn make_curve(date: NaiveDate, points: &[(u32, f64)]) -> YieldCurve {
        YieldCurve {
            date,
            points: points.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn date32_roundtrip() {
        let d = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
        let d32 = to_date32(d);
        let back = from_date32(d32).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn write_read_treasury_year() {
        let dir = tempdir().unwrap();
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let curve = make_curve(date, &[(365, 0.04321), (3650, 0.04872)]);
        write_treasury_year(dir.path(), 2025, &[curve]).unwrap();

        let curves = read_treasury_year(&dir.path().join("treasury-2025.parquet")).unwrap();
        assert_eq!(curves.len(), 1);
        let c = &curves[0];
        assert_eq!(c.date, date);
        assert!((c.points[&365] - 0.04321).abs() < 1e-4);
        assert!((c.points[&3650] - 0.04872).abs() < 1e-4);
    }

    #[test]
    fn write_read_sofr_year() {
        let dir = tempdir().unwrap();
        let date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        write_sofr_year(
            dir.path(),
            2025,
            &[SofrDay {
                date,
                rate: 0.04330,
            }],
        )
        .unwrap();

        let rates = read_sofr_year(&dir.path().join("sofr-2025.parquet")).unwrap();
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].date, date);
        assert!((rates[0].rate - 0.04330).abs() < 1e-4);
    }

    #[test]
    fn append_treasury_day_merges() {
        let dir = tempdir().unwrap();
        let d1 = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 1, 3).unwrap();

        let c1 = make_curve(d1, &[(365, 0.041)]);
        append_treasury_day(dir.path(), d1, &c1).unwrap();

        let c2 = make_curve(d2, &[(365, 0.042)]);
        append_treasury_day(dir.path(), d2, &c2).unwrap();

        let curves = read_treasury_year(&dir.path().join("treasury-2025.parquet")).unwrap();
        assert_eq!(curves.len(), 2);
    }

    #[test]
    fn append_sofr_day_merges() {
        let dir = tempdir().unwrap();
        let d1 = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 1, 3).unwrap();

        append_sofr_day(dir.path(), d1, 0.0430).unwrap();
        append_sofr_day(dir.path(), d2, 0.0431).unwrap();

        let rates = read_sofr_year(&dir.path().join("sofr-2025.parquet")).unwrap();
        assert_eq!(rates.len(), 2);
    }
}
