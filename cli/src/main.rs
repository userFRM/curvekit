//! `curvekit-cli` — backfill, append, and inspect bundled parquet data.
//!
//! # Commands
//!
//! ```text
//! curvekit-cli backfill --years 25
//! curvekit-cli backfill --source treasury --year 2024
//! curvekit-cli append-today
//! curvekit-cli get treasury --date 2026-04-14
//! curvekit-cli get sofr --date 2026-04-14
//! ```

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

use curvekit::curve::SofrDay;
use curvekit::sources::parquet_io::{
    append_sofr_day, append_treasury_day, write_sofr_year, write_treasury_year,
};
use curvekit::sources::sofr::HttpSofrFetcher;
use curvekit::sources::treasury::HttpTreasuryFetcher;
use curvekit::SofrFetcher;
use curvekit::TreasuryFetcher;

// ── CLI definition ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "curvekit-cli",
    about = "Manage curvekit bundled-parquet data — Treasury yield curves + SOFR",
    version,
    propagate_version = true
)]
struct Cli {
    /// Path to the data directory (default: <repo-root>/data/).
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
    /// With no options, fetches treasury 2000-present + SOFR 2018-present.
    Backfill {
        /// Number of years back from current year (overrides --year).
        #[arg(long)]
        years: Option<u32>,

        /// Restrict to one source: "treasury" or "sofr".
        #[arg(long)]
        source: Option<String>,

        /// Single year to fetch (used with --source).
        #[arg(long)]
        year: Option<i32>,
    },

    /// Fetch yesterday's treasury curve + today's SOFR and append to current-year parquet.
    AppendToday,

    /// Read from bundled parquet and print to stdout.
    Get {
        #[command(subcommand)]
        source: GetSource,
    },
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
        // Relative to the CLI binary at target/release/curvekit-cli → ../../data
        // But CARGO_MANIFEST_DIR gives us the cli/ directory at compile time.
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
        },
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
            other => bail!("unknown source '{other}' — use 'treasury' or 'sofr'"),
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
                    tracing::error!(source = "treasury", year = y, error = %e, "fetch failed");
                    had_error = true;
                }
            }
        }
        Some("sofr") => {
            let start_year = (current_year - n + 1).max(2018);
            for y in start_year..=current_year {
                if let Err(e) = fetch_sofr_year(data_dir, y).await {
                    tracing::error!(source = "sofr", year = y, error = %e, "fetch failed");
                    had_error = true;
                }
            }
        }
        None => {
            // Both sources.
            let t_start = (current_year - n + 1).max(2000);
            for y in t_start..=current_year {
                if let Err(e) = fetch_treasury_year(data_dir, y).await {
                    tracing::error!(source = "treasury", year = y, error = %e, "fetch failed");
                    had_error = true;
                }
            }
            let s_start = (current_year - n + 1).max(2018);
            for y in s_start..=current_year {
                if let Err(e) = fetch_sofr_year(data_dir, y).await {
                    tracing::error!(source = "sofr", year = y, error = %e, "fetch failed");
                    had_error = true;
                }
            }
        }
        Some(other) => bail!("unknown source '{other}' — use 'treasury' or 'sofr'"),
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
    let start = year as u32 * 10000 + 101; // YYYYMMDD: Jan 1
    let end = year as u32 * 10000 + 1231; // YYYYMMDD: Dec 31
    let curves = fetcher
        .fetch(start, end)
        .await
        .with_context(|| format!("treasury fetch for {year}"))?;
    if curves.is_empty() {
        // Not an error — could be a year with no published data yet (e.g. current partial year
        // with no trading days yet fetched). Log and skip silently so the caller can decide.
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

// ── append-today ──────────────────────────────────────────────────────────────

async fn cmd_append_today(data_dir: &std::path::Path) -> Result<()> {
    let today = Utc::now().date_naive();
    let yesterday = today.pred_opt().unwrap_or(today);

    // Treasury: yesterday's close.
    let t_fetcher = HttpTreasuryFetcher::new()?;
    let yyyymmdd = |d: NaiveDate| d.year() as u32 * 10000 + d.month() * 100 + d.day();
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

    // SOFR: today's publication (or yesterday if not yet published).
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
                tracing::warn!("no SOFR data for {yesterday}–{today} (weekend/holiday?)");
            }
        }
        Err(e) => tracing::warn!("SOFR fetch failed: {e}"),
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .with_context(|| format!("invalid date '{s}' — expected YYYY-MM-DD"))
}

fn format_tenor(days: u32) -> String {
    match days {
        30 => "1M".into(),
        60 => "2M".into(),
        91 => "3M".into(),
        182 => "6M".into(),
        365 => "1Y".into(),
        730 => "2Y".into(),
        1095 => "3Y".into(),
        1825 => "5Y".into(),
        2555 => "7Y".into(),
        3650 => "10Y".into(),
        7300 => "20Y".into(),
        10950 => "30Y".into(),
        d => format!("{d}d"),
    }
}
