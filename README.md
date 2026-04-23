# curvekit

**Risk-free rate service for Rust** — fetches, caches, interpolates, and
serves the US Treasury yield curve and SOFR overnight rate via JSON-RPC 2.0
and REST.

No API keys. No paid data subscriptions. Just the public endpoints that
treasury.gov and the NY Fed have provided for decades, wrapped in a
production-ready Rust service.

## Why

The Rust ecosystem has excellent fixed-income math libraries, but none that
handle the operational layer: scheduled fetching from public sources, local
caching, and a typed client SDK. Every team that needs a risk-free rate ends
up copy-pasting the same treasury.gov CSV parser. curvekit does it once,
correctly, with tests.

## Quick start

```bash
# Install and start the server
cargo install curvekit-server
curvekit-server --port 8080 --db ./curvekit.db

# Query via curl
curl http://localhost:8080/treasury/curve/2026-04-15 | jq
curl http://localhost:8080/sofr/2026-04-15           | jq
curl http://localhost:8080/health                    | jq

# Or via JSON-RPC
curl -s http://localhost:8080/rpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"rates.treasury_curve","params":{"date":"2026-04-15"}}' \
  | jq
```

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                   curvekit-server (binary)                   │
│                                                              │
│  ┌─────────────┐    ┌──────────────────────────────────────┐ │
│  │  scheduler  │    │           axum HTTP server           │ │
│  │ 08:00 SOFR  │    │  POST /rpc  ·  GET /treasury/...     │ │
│  │ 15:30 Tsy   │    │  GET /sofr/...  ·  GET /health       │ │
│  └──────┬──────┘    └───────────────────┬──────────────────┘ │
│         │                               │                    │
│  ┌──────▼───────────────────────────────▼──────────────────┐ │
│  │                   curvekit-rpc                           │ │
│  │         JSON-RPC dispatch · REST handlers               │ │
│  └──────────────────────────┬───────────────────────────────┘ │
│                             │                                │
│  ┌──────────────────────────▼───────────────────────────────┐ │
│  │                  curvekit-core                           │ │
│  │  sources/treasury ── sources/sofr   interpolation        │ │
│  │  YieldCurve · SofrRate · TermStructure                   │ │
│  │                    SQLite cache                          │ │
│  └──────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
                              ▲
          ┌───────────────────┤
          │                   │
  curvekit-sdk (Rust)    curvekit CLI
  CurvekitClient         get · refresh · health
```

## Crates

| Crate | Description |
|---|---|
| `curvekit-core` | Fetchers, parsers, curve types, interpolation, SQLite cache |
| `curvekit-rpc` | JSON-RPC 2.0 and REST handlers (axum) |
| `curvekit-server` | Binary — serves the API |
| `curvekit-sdk` | Rust HTTP client wrapping the server |
| `curvekit` (CLI) | Command-line interface |

## Data sources

| Source | URL | Refresh |
|---|---|---|
| US Treasury | `home.treasury.gov` daily CSV | 15:30 ET, business days |
| SOFR | `markets.newyorkfed.org` API | 08:00 ET, business days |

See [`docs/data-sources.md`](docs/data-sources.md) for details.

## Rust SDK

```rust
use curvekit_sdk::CurvekitClient;
use chrono::NaiveDate;

let client = CurvekitClient::new("http://localhost:8080");
let date   = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();

// Full yield curve
let curve = client.treasury_curve(date).await?;

// SOFR overnight rate (continuously compounded)
let sofr = client.sofr(date).await?;

// Interpolated risk-free rate for any DTE
let r_45d = client.rate_for_days(date, 45).await?;
```

## JSON-RPC methods

| Method | Params | Returns |
|---|---|---|
| `rates.treasury_curve` | `{"date": "YYYY-MM-DD"}` | `YieldCurve` |
| `rates.sofr` | `{"date": "YYYY-MM-DD"}` | `SofrRate` |
| `rates.treasury_range` | `{"start": "...", "end": "..."}` | `[YieldCurve]` |
| `rates.health` | `{}` | row counts + status |

OpenAPI spec: `GET /openapi.json`.

## Yield curve representation

All rates are **continuously compounded**, converted from the Treasury's
published Bond Equivalent Yields:

```
BEY → APY:  APY  = (1 + BEY/200)^2 − 1
APY → cont: r    = ln(1 + APY)
```

Curve points are keyed by **days to maturity** using these approximations:
`1M=30, 2M=60, 3M=91, 6M=182, 1Y=365, 2Y=730, 3Y=1095, 5Y=1825,
7Y=2555, 10Y=3650, 20Y=7300, 30Y=10950`.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).

Copyright 2026 Theta Gamma Systems
