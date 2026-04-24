//! `curvekit-cli` — backfill, append, inspect bundled parquet data, and
//! generate SHA-256 manifests.
//!
//! # Commands
//!
//! ```text
//! curvekit-cli backfill --years 25
//! curvekit-cli backfill --source treasury --year 2024
//! curvekit-cli backfill --source effr --year 2024
//! curvekit-cli backfill --source obfr --year 2024
//! curvekit-cli append-today
//! curvekit-cli get treasury --date 2026-04-14
//! curvekit-cli get sofr --date 2026-04-14
//! curvekit-cli manifest
//! ```

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

use curvekit::curve::SofrDay;
use curvekit::sources::effr::{EffrDay, HttpEffrFetcher};
use curvekit::sources::obfr::{HttpObfrFetcher, ObfrDay};
use curvekit::sources::parquet_io::{
    append_effr_day, append_obfr_day, append_sofr_day, append_treasury_day, write_effr_year,
    write_obfr_year, write_sofr_year, write_treasury_year,
};
use curvekit::sources::sofr::HttpSofrFetcher;
use curvekit::sources::treasury::HttpTreasuryFetcher;
use curvekit::EffrFetcher;
use curvekit::ObfrFetcher;
use curvekit::SofrFetcher;
use curvekit::Tenor;
use curvekit::TreasuryFetcher;

// ── CLI definition ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "curvekit-cli",
    about = "Manage curvekit bundled-parquet data — Treasury + SOFR + EFFR + OBFR",
    version,
    propagate_version = true
)]
struct Cli {
    /// Path to the data directory (default: `<repo-root>/data/`).
    /// Override with $CURVEKIT_DATA_DIR or this flag.
    #[arg(long, env = "CURVEKIT_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Backfill historical data from public sources.
    ///
    /// With no options, fetches treasury 2000-present + SOFR 2018-present
    /// + EFFR 2000-present + OBFR 2016-present.
    Backfill {
        /// Number of years back from current year (overrides --year).
        #[arg(long)]
        years: Option<u32>,

        /// Restrict to one source: "treasury", "sofr", "effr", "obfr".
        #[arg(long)]
        source: Option<String>,

        /// Single year to fetch (used with --source).
        #[arg(long)]
        year: Option<i32>,
    },

    /// Fetch yesterday's treasury curve + today's SOFR/EFFR/OBFR and append
    /// to current-year parquet.
    AppendToday,

    /// Read from bundled parquet and print to stdout.
    Get {
        #[command(subcommand)]
        source: GetSource,
    },

    /// Generate (or regenerate) `data/manifest.json` with SHA-256 digests for
    /// all parquet files in the data directory.
    ///
    /// The manifest is used by the curvekit client to verify integrity of
    /// fetched parquet files. Commit it alongside the data files.
    Manifest,
}

#[derive(Subcommand, Debug)]
enum GetSource {
    /// Print the US Treasury yield curve for a date.
    Treasury {
        /// Date in YYYY-MM-DD format.
        #[arg(long, short)]
        date: String,
    },
    /// Print the SOFR overnight rate for a date.
    Sofr {
        /// Date in YYYY-MM-DD format.
        #[arg(long, short)]
        date: String,
    },
    /// Print the EFFR overnight rate for a date.
    Effr {
        /// Date in YYYY-MM-DD format.
        #[arg(long, short)]
        date: String,
    },
    /// Print the OBFR overnight rate for a date.
    Obfr {
        /// Date in YYYY-MM-DD format.
        #[arg(long, short)]
        date: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Resolve data dir: CLI flag > env var (handled by clap) > repo default.
    let data_dir = cli.data_dir.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
    });
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;

    match cli.cmd {
        Command::Backfill {
            years,
            source,
            year,
        } => {
            cmd_backfill(&data_dir, years, source.as_deref(), year).await?;
        }
        Command::AppendToday => {
            cmd_append_today(&data_dir).await?;
        }
        Command::Get { source } => match source {
            GetSource::Treasury { date } => {
                let d = parse_date(&date)?;
                let curve = curvekit::treasury_curve(d)
                    .with_context(|| format!("reading treasury curve for {d}"))?;
                println!("US Treasury Yield Curve — {d}");
                println!("{:<10} {:>14}", "Tenor", "Rate (cont %)");
                println!("{}", "-".repeat(26));
                for (&days, &rate) in &curve.points {
                    println!("{:<10} {:>14.6}", format_tenor(days), rate * 100.0);
                }
            }
            GetSource::Sofr { date } => {
                let d = parse_date(&date)?;
                let rate = curvekit::sofr(d).with_context(|| format!("reading SOFR for {d}"))?;
                println!("SOFR — {d}: {:.6}% (continuously compounded)", rate * 100.0);
            }
            GetSource::Effr { date } => {
                let d = parse_date(&date)?;
                let rate = read_overnight_rate(&data_dir, "effr", d)?;
                println!("EFFR — {d}: {:.6}% (continuously compounded)", rate * 100.0);
            }
            GetSource::Obfr { date } => {
                let d = parse_date(&date)?;
                let rate = read_overnight_rate(&data_dir, "obfr", d)?;
                println!("OBFR — {d}: {:.6}% (continuously compounded)", rate * 100.0);
            }
        },
        Command::Manifest => {
            cmd_manifest(&data_dir)?;
        }
    }

    Ok(())
}

// ── Backfill ──────────────────────────────────────────────────────────────────

async fn cmd_backfill(
    data_dir: &std::path::Path,
    years: Option<u32>,
    source: Option<&str>,
    year: Option<i32>,
) -> Result<()> {
    let current_year = Utc::now().year();

    // Single-year mode (--source + --year).
    if let Some(y) = year {
        let src = source.unwrap_or("treasury");
        match src {
            "treasury" => {
                fetch_treasury_year(data_dir, y).await?;
            }
            "sofr" => {
                if y < 2018 {
                    bail!("SOFR data only available from 2018 onwards");
                }
                fetch_sofr_year(data_dir, y).await?;
            }
            "effr" => {
                fetch_effr_year(data_dir, y).await?;
            }
            "obfr" => {
                if y < 2016 {
                    bail!("OBFR data only available from 2016 onwards");
                }
                fetch_obfr_year(data_dir, y).await?;
            }
            other => bail!("unknown source '{other}' — use 'treasury', 'sofr', 'effr', or 'obfr'"),
        }
        return Ok(());
    }

    // Multi-year mode.
    let n = years.unwrap_or(25) as i32;
    let mut had_error = false;

    match source {
        Some("treasury") => {
            let start_year = (current_year - n + 1).max(2000);
            for y in start_year..=current_year {
                if let Err(e) = fetch_treasury_year(data_dir, y).await {
                    tracing::error!(source = "treasury", year = y, error = %e);
                    had_error = true;
                }
            }
        }
        Some("sofr") => {
            let start_year = (current_year - n + 1).max(2018);
            for y in start_year..=current_year {
                if let Err(e) = fetch_sofr_year(data_dir, y).await {
                    tracing::error!(source = "sofr", year = y, error = %e);
                    had_error = true;
                }
            }
        }
        Some("effr") => {
            let start_year = (current_year - n + 1).max(2000);
            for y in start_year..=current_year {
                if let Err(e) = fetch_effr_year(data_dir, y).await {
                    tracing::error!(source = "effr", year = y, error = %e);
                    had_error = true;
                }
            }
        }
        Some("obfr") => {
            let start_year = (current_year - n + 1).max(2016);
            for y in start_year..=current_year {
                if let Err(e) = fetch_obfr_year(data_dir, y).await {
                    tracing::error!(source = "obfr", year = y, error = %e);
                    had_error = true;
                }
            }
        }
        None => {
            // All sources.
            let t_start = (current_year - n + 1).max(2000);
            for y in t_start..=current_year {
                if let Err(e) = fetch_treasury_year(data_dir, y).await {
                    tracing::error!(source = "treasury", year = y, error = %e);
                    had_error = true;
                }
            }
            let s_start = (current_year - n + 1).max(2018);
            for y in s_start..=current_year {
                if let Err(e) = fetch_sofr_year(data_dir, y).await {
                    tracing::error!(source = "sofr", year = y, error = %e);
                    had_error = true;
                }
            }
            let e_start = (current_year - n + 1).max(2000);
            for y in e_start..=current_year {
                if let Err(e) = fetch_effr_year(data_dir, y).await {
                    tracing::error!(source = "effr", year = y, error = %e);
                    had_error = true;
                }
            }
            let o_start = (current_year - n + 1).max(2016);
            for y in o_start..=current_year {
                if let Err(e) = fetch_obfr_year(data_dir, y).await {
                    tracing::error!(source = "obfr", year = y, error = %e);
                    had_error = true;
                }
            }
        }
        Some(other) => {
            bail!("unknown source '{other}' — use 'treasury', 'sofr', 'effr', or 'obfr'")
        }
    }

    if had_error {
        bail!("backfill completed with one or more fetch errors (see above)");
    }
    println!("Backfill complete. Data written to {}", data_dir.display());
    Ok(())
}

async fn fetch_treasury_year(data_dir: &std::path::Path, year: i32) -> Result<()> {
    tracing::info!("fetching treasury {year}...");
    let fetcher = HttpTreasuryFetcher::new()?;
    let start = year as u32 * 10000 + 101;
    let end = year as u32 * 10000 + 1231;
    let curves = fetcher
        .fetch(start, end)
        .await
        .with_context(|| format!("treasury fetch for {year}"))?;
    if curves.is_empty() {
        tracing::warn!("treasury {year}: upstream returned 0 rows — skipping write");
        return Ok(());
    }
    write_treasury_year(data_dir, year, &curves)
        .with_context(|| format!("writing treasury {year}"))?;
    tracing::info!("wrote treasury-{year}.parquet: {} rows", curves.len());
    Ok(())
}

async fn fetch_sofr_year(data_dir: &std::path::Path, year: i32) -> Result<()> {
    tracing::info!("fetching sofr {year}...");
    let fetcher = HttpSofrFetcher::new()?;
    let start = year as u32 * 10000 + 101;
    let end = year as u32 * 10000 + 1231;
    let rates = fetcher
        .fetch(start, end)
        .await
        .with_context(|| format!("sofr fetch for {year}"))?;
    if rates.is_empty() {
        tracing::warn!("sofr {year}: upstream returned 0 rows — skipping write");
        return Ok(());
    }
    let n = rates.len();
    let days: Vec<SofrDay> = rates
        .into_iter()
        .map(|r| SofrDay {
            date: r.date,
            rate: r.rate,
        })
        .collect();
    write_sofr_year(data_dir, year, &days).with_context(|| format!("writing sofr {year}"))?;
    tracing::info!("wrote sofr-{year}.parquet: {n} rows");
    Ok(())
}

async fn fetch_effr_year(data_dir: &std::path::Path, year: i32) -> Result<()> {
    tracing::info!("fetching effr {year}...");
    let fetcher = HttpEffrFetcher::new()?;
    let start = year as u32 * 10000 + 101;
    let end = year as u32 * 10000 + 1231;
    let rates = fetcher
        .fetch(start, end)
        .await
        .with_context(|| format!("effr fetch for {year}"))?;
    if rates.is_empty() {
        tracing::warn!("effr {year}: upstream returned 0 rows — skipping write");
        return Ok(());
    }
    let n = rates.len();
    let days: Vec<EffrDay> = rates;
    write_effr_year(data_dir, year, &days).with_context(|| format!("writing effr {year}"))?;
    tracing::info!("wrote effr-{year}.parquet: {n} rows");
    Ok(())
}

async fn fetch_obfr_year(data_dir: &std::path::Path, year: i32) -> Result<()> {
    tracing::info!("fetching obfr {year}...");
    let fetcher = HttpObfrFetcher::new()?;
    let start = year as u32 * 10000 + 101;
    let end = year as u32 * 10000 + 1231;
    let rates = fetcher
        .fetch(start, end)
        .await
        .with_context(|| format!("obfr fetch for {year}"))?;
    if rates.is_empty() {
        tracing::warn!("obfr {year}: upstream returned 0 rows — skipping write");
        return Ok(());
    }
    let n = rates.len();
    let days: Vec<ObfrDay> = rates;
    write_obfr_year(data_dir, year, &days).with_context(|| format!("writing obfr {year}"))?;
    tracing::info!("wrote obfr-{year}.parquet: {n} rows");
    Ok(())
}

// ── append-today ──────────────────────────────────────────────────────────────

async fn cmd_append_today(data_dir: &std::path::Path) -> Result<()> {
    let today = Utc::now().date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);
    let yyyymmdd = |d: NaiveDate| d.year() as u32 * 10000 + d.month() * 100 + d.day();

    // Treasury: yesterday's close.
    let t_fetcher = HttpTreasuryFetcher::new()?;
    let t_start = yyyymmdd(yesterday);
    let t_end = yyyymmdd(yesterday);

    match t_fetcher.fetch(t_start, t_end).await {
        Ok(curves) => {
            for curve in &curves {
                append_treasury_day(data_dir, curve.date, curve)
                    .with_context(|| format!("appending treasury {}", curve.date))?;
                tracing::info!("appended treasury {}", curve.date);
            }
            if curves.is_empty() {
                tracing::warn!("no treasury data for {yesterday} (market holiday?)");
            }
        }
        Err(e) => tracing::warn!("treasury fetch failed: {e}"),
    }

    // SOFR/EFFR/OBFR: today's or yesterday's publication.
    let s_fetcher = HttpSofrFetcher::new()?;
    let s_start = yyyymmdd(yesterday);
    let s_end = yyyymmdd(today);

    match s_fetcher.fetch(s_start, s_end).await {
        Ok(rates) => {
            for rate in &rates {
                append_sofr_day(data_dir, rate.date, rate.rate)
                    .with_context(|| format!("appending sofr {}", rate.date))?;
                tracing::info!("appended sofr {}", rate.date);
            }
            if rates.is_empty() {
                tracing::warn!("no SOFR data for {yesterday}–{today}");
            }
        }
        Err(e) => tracing::warn!("SOFR fetch failed: {e}"),
    }

    let e_fetcher = HttpEffrFetcher::new()?;
    match e_fetcher.fetch(s_start, s_end).await {
        Ok(rates) => {
            for rate in &rates {
                append_effr_day(data_dir, rate.date, rate.rate)
                    .with_context(|| format!("appending effr {}", rate.date))?;
                tracing::info!("appended effr {}", rate.date);
            }
            if rates.is_empty() {
                tracing::warn!("no EFFR data for {yesterday}–{today}");
            }
        }
        Err(e) => tracing::warn!("EFFR fetch failed: {e}"),
    }

    let o_fetcher = HttpObfrFetcher::new()?;
    match o_fetcher.fetch(s_start, s_end).await {
        Ok(rates) => {
            for rate in &rates {
                append_obfr_day(data_dir, rate.date, rate.rate)
                    .with_context(|| format!("appending obfr {}", rate.date))?;
                tracing::info!("appended obfr {}", rate.date);
            }
            if rates.is_empty() {
                tracing::warn!("no OBFR data for {yesterday}–{today}");
            }
        }
        Err(e) => tracing::warn!("OBFR fetch failed: {e}"),
    }

    Ok(())
}

// ── manifest ──────────────────────────────────────────────────────────────────

/// Generate `data/manifest.json` — SHA-256 digests for all parquet files.
///
/// The manifest format is:
/// ```json
/// {
///   "treasury-2020.parquet": "sha256:<hex>",
///   "sofr-2020.parquet":     "sha256:<hex>",
///   ...
/// }
/// ```
///
/// The curvekit client downloads this manifest at session start and verifies
/// each fetched parquet file against it. Regenerate after every backfill run.
fn cmd_manifest(data_dir: &std::path::Path) -> Result<()> {
    use std::collections::BTreeMap;

    let mut entries: BTreeMap<String, String> = BTreeMap::new();

    let read_dir = std::fs::read_dir(data_dir)
        .with_context(|| format!("reading data dir {}", data_dir.display()))?;

    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if !name_str.ends_with(".parquet") {
            continue;
        }
        let path = entry.path();
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        entries.insert(name_str, format!("sha256:{hex}"));
    }

    let manifest =
        serde_json::to_string_pretty(&entries).context("serializing manifest to JSON")?;
    let manifest_path = data_dir.join("manifest.json");
    std::fs::write(&manifest_path, manifest)
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!(
        "Wrote manifest with {} entries → {}",
        entries.len(),
        manifest_path.display()
    );
    Ok(())
}

// ── overnight-rate reader (for `get effr/obfr`) ───────────────────────────────

fn read_overnight_rate(data_dir: &std::path::Path, prefix: &str, date: NaiveDate) -> Result<f64> {
    use chrono::Datelike;
    let year = date.year();
    let path = data_dir.join(format!("{prefix}-{year}.parquet"));
    if !path.exists() {
        bail!("{prefix}-{year}.parquet not found — run: curvekit-cli backfill --source {prefix} --year {year}");
    }
    match prefix {
        "effr" => {
            let rates = curvekit::sources::parquet_io::read_effr_year(&path)?;
            rates
                .into_iter()
                .find(|r| r.date == date)
                .map(|r| r.rate)
                .ok_or_else(|| anyhow::anyhow!("no EFFR for {date}"))
        }
        "obfr" => {
            let rates = curvekit::sources::parquet_io::read_obfr_year(&path)?;
            rates
                .into_iter()
                .find(|r| r.date == date)
                .map(|r| r.rate)
                .ok_or_else(|| anyhow::anyhow!("no OBFR for {date}"))
        }
        _ => bail!("unknown prefix: {prefix}"),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .with_context(|| format!("invalid date '{s}' — expected YYYY-MM-DD"))
}

fn format_tenor(days: u32) -> String {
    Tenor::days(days).to_string()
}
