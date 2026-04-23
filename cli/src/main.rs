//! `curvekit` — CLI for the curvekit risk-free rate service.
//!
//! # Commands
//!
//! ```text
//! curvekit serve --port 8080 --db ./curvekit.db
//! curvekit get treasury --date 2026-04-14
//! curvekit get sofr --date 2026-04-14
//! curvekit refresh
//! ```

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

use curvekit_sdk::CurvekitClient;

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "curvekit",
    about = "Risk-free rate service CLI — Treasury yield curves + SOFR",
    version,
    propagate_version = true
)]
struct Cli {
    /// curvekit-server base URL.
    #[arg(
        long,
        default_value = "http://localhost:8080",
        env = "CURVEKIT_URL",
        global = true
    )]
    url: String,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Fetch and display rate data from the server.
    Get {
        #[command(subcommand)]
        source: GetSource,
    },

    /// Force the server to refresh all data from public sources.
    Refresh,

    /// Show server health and cache statistics.
    Health,
}

#[derive(Subcommand, Debug)]
enum GetSource {
    /// Fetch the US Treasury yield curve.
    Treasury {
        /// Date in YYYY-MM-DD format (defaults to today).
        #[arg(long, short)]
        date: Option<String>,
    },
    /// Fetch the SOFR overnight rate.
    Sofr {
        /// Date in YYYY-MM-DD format (defaults to today).
        #[arg(long, short)]
        date: Option<String>,
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
    let client = CurvekitClient::new(&cli.url);

    match cli.cmd {
        Command::Get { source } => match source {
            GetSource::Treasury { date } => {
                let d = resolve_date(date)?;
                let curve = client
                    .treasury_curve(d)
                    .await
                    .with_context(|| format!("fetching treasury curve for {d}"))?;
                println!("US Treasury Yield Curve — {d}");
                println!("{:<10} {:>12}", "Tenor", "Rate (%)");
                println!("{}", "-".repeat(24));
                for (&days, &rate) in &curve.points {
                    println!("{:<10} {:>12.4}", format_tenor(days), rate * 100.0);
                }
            }
            GetSource::Sofr { date } => {
                let d = resolve_date(date)?;
                let rate = client
                    .sofr(d)
                    .await
                    .with_context(|| format!("fetching SOFR for {d}"))?;
                println!("SOFR — {d}: {:.6}% (continuously compounded)", rate * 100.0);
            }
        },
        Command::Refresh => {
            // Refresh is triggered by health check + manual fetch
            // (the server's scheduled refresh runs autonomously;
            // this command just confirms the server is reachable).
            let h = client.health().await.context("server health check")?;
            println!(
                "Server reachable — status={} treasury_rows={} sofr_rows={}",
                h.status, h.treasury_rows, h.sofr_rows
            );
            println!(
                "Scheduled refresh runs automatically at 08:00 ET (SOFR) and 15:30 ET (Treasury)."
            );
        }
        Command::Health => {
            let h = client.health().await.context("health check failed")?;
            println!("status:        {}", h.status);
            println!("treasury_rows: {}", h.treasury_rows);
            println!("sofr_rows:     {}", h.sofr_rows);
        }
    }

    Ok(())
}

fn resolve_date(s: Option<String>) -> Result<NaiveDate> {
    match s {
        Some(ds) => NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
            .with_context(|| format!("invalid date '{ds}' — expected YYYY-MM-DD")),
        None => Ok(chrono::Local::now().date_naive()),
    }
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
