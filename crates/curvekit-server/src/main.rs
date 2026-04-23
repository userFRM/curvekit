//! `curvekit-server` — serves Treasury yield curve + SOFR data via JSON-RPC 2.0 and REST.
//!
//! # Usage
//!
//! ```text
//! curvekit-server --port 8080 --db ./curvekit.db
//! ```
//!
//! On startup:
//! 1. Opens (or creates) the SQLite cache.
//! 2. Fetches today's Treasury curve and SOFR rate from public sources.
//! 3. Starts the HTTP server.
//! 4. Spawns a background task that refreshes data on schedule:
//!    - Treasury at 15:30 ET (published after market close)
//!    - SOFR at 08:00 ET (published in the morning)

use anyhow::{Context, Result};
use chrono::Utc;
use chrono_tz::America::New_York;
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use curvekit_core::{
    HttpSofrFetcher, HttpTreasuryFetcher, RateCache, SofrFetcher, TreasuryFetcher,
};
use curvekit_rpc::{build_router, AppState};

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "curvekit-server",
    about = "Risk-free rate service: US Treasury yield curve + SOFR",
    version
)]
struct Cli {
    /// TCP port to listen on.
    #[arg(long, short, default_value = "8080", env = "CURVEKIT_PORT")]
    port: u16,

    /// Path to the SQLite cache database.
    #[arg(
        long,
        default_value = "curvekit.db",
        env = "CURVEKIT_DB",
        value_name = "PATH"
    )]
    db: PathBuf,

    /// Skip the initial data fetch on startup (useful in offline/test mode).
    #[arg(long, default_value = "false")]
    no_bootstrap: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("curvekit=info".parse().expect("valid directive")),
        )
        .init();

    let cli = Cli::parse();

    info!(db = %cli.db.display(), port = cli.port, "starting curvekit-server");

    // ── Open cache ────────────────────────────────────────────────────────
    let cache = RateCache::open(&cli.db)
        .with_context(|| format!("opening cache at {}", cli.db.display()))?;
    let state = Arc::new(AppState::new(cache));

    // ── Initial bootstrap ─────────────────────────────────────────────────
    if !cli.no_bootstrap {
        let s = Arc::clone(&state);
        if let Err(e) = bootstrap_once(&s).await {
            warn!("initial bootstrap failed (continuing with empty cache): {e:#}");
        }
    }

    // ── Background refresh task ───────────────────────────────────────────
    {
        let s = Arc::clone(&state);
        tokio::spawn(async move {
            refresh_loop(s).await;
        });
    }

    // ── HTTP server ───────────────────────────────────────────────────────
    let router = build_router(Arc::clone(&state));
    let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding to port {}", cli.port))?;

    info!(addr = %addr, "curvekit-server listening");
    info!("JSON-RPC: POST http://localhost:{}/rpc", cli.port);
    info!(
        "REST:     GET  http://localhost:{}/treasury/curve/YYYY-MM-DD",
        cli.port
    );
    info!(
        "REST:     GET  http://localhost:{}/sofr/YYYY-MM-DD",
        cli.port
    );
    info!("Health:   GET  http://localhost:{}/health", cli.port);

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server error")?;

    info!("curvekit-server stopped");
    Ok(())
}

// ── Bootstrap helpers ─────────────────────────────────────────────────────────

async fn bootstrap_once(state: &Arc<AppState>) -> Result<()> {
    let today = Utc::now().date_naive();
    let today_int = {
        let s = today.format("%Y%m%d").to_string();
        s.parse::<u32>().unwrap_or(0)
    };

    info!("bootstrapping data for {today}");

    let treasury = HttpTreasuryFetcher::new()?;
    match treasury.fetch(today_int, today_int).await {
        Ok(curves) => {
            for curve in &curves {
                if let Err(e) = state.cache.store_treasury(curve) {
                    warn!("cache write (treasury): {e}");
                }
            }
            info!(count = curves.len(), "treasury bootstrap complete");
            *state.last_treasury_refresh.lock().unwrap() = Some(Utc::now());
        }
        Err(e) => warn!("treasury fetch failed: {e:#}"),
    }

    let sofr = HttpSofrFetcher::new()?;
    match sofr.fetch(today_int, today_int).await {
        Ok(rates) => {
            for rate in &rates {
                if let Err(e) = state.cache.store_sofr(rate) {
                    warn!("cache write (SOFR): {e}");
                }
            }
            info!(count = rates.len(), "SOFR bootstrap complete");
            *state.last_sofr_refresh.lock().unwrap() = Some(Utc::now());
        }
        Err(e) => warn!("SOFR fetch failed: {e:#}"),
    }

    Ok(())
}

// ── Refresh loop ──────────────────────────────────────────────────────────────

/// Checks once per minute whether a scheduled refresh is due.
///
/// - Treasury: 15:30 ET on each business day.
/// - SOFR: 08:00 ET on each business day.
async fn refresh_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        let now_et = Utc::now().with_timezone(&New_York);
        let hour = now_et.hour();
        let minute = now_et.minute();
        let weekday = now_et.weekday();

        use chrono::Weekday::{Fri, Mon, Thu, Tue, Wed};
        let is_weekday = matches!(weekday, Mon | Tue | Wed | Thu | Fri);

        if !is_weekday {
            continue;
        }

        // Treasury refresh window: 15:30–15:31 ET
        if hour == 15 && minute == 30 {
            let s = Arc::clone(&state);
            tokio::spawn(async move {
                if let Err(e) = bootstrap_once(&s).await {
                    warn!("scheduled treasury refresh failed: {e:#}");
                }
            });
        }

        // SOFR refresh window: 08:00–08:01 ET
        if hour == 8 && minute == 0 {
            let s = Arc::clone(&state);
            tokio::spawn(async move {
                if let Err(e) = bootstrap_once(&s).await {
                    warn!("scheduled SOFR refresh failed: {e:#}");
                }
            });
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    info!("shutdown signal received");
}

// Bring in Weekday + time accessors.
use chrono::{Datelike, Timelike};
