# API Reference

The curvekit server exposes two equivalent interfaces on port 8080:

- **JSON-RPC 2.0** at `POST /rpc` — structured, batch-capable.
- **REST** at `GET /treasury/...`, `GET /sofr/...`, `GET /health` — `curl`-friendly.

An OpenAPI 3.0 spec is available at `GET /openapi.json`.

## JSON-RPC

All calls are `POST /rpc` with `Content-Type: application/json`.

### `rates.treasury_curve`

Fetch the Treasury yield curve for a single date.

```json
{
  "jsonrpc": "2.0", "id": 1,
  "method": "rates.treasury_curve",
  "params": { "date": "2026-04-15" }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0", "id": 1,
  "result": {
    "date": "2026-04-15",
    "points": {
      "30":    0.04187,
      "60":    0.04163,
      "91":    0.04115,
      "182":   0.04041,
      "365":   0.03957,
      "730":   0.03912,
      "1095":  0.03867,
      "1825":  0.03823,
      "2555":  0.03780,
      "3650":  0.03739,
      "7300":  0.03960,
      "10950": 0.03912
    }
  }
}
```

Keys are **days to maturity** (integers). Values are **continuously compounded**
rates (not percentages).

### `rates.sofr`

Fetch the SOFR overnight rate for a single date.

```json
{
  "jsonrpc": "2.0", "id": 1,
  "method": "rates.sofr",
  "params": { "date": "2026-04-15" }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0", "id": 1,
  "result": { "date": "2026-04-15", "rate": 0.042371 }
}
```

### `rates.treasury_range`

Fetch Treasury curves for a date range (returns all cached dates in the range).

```json
{
  "jsonrpc": "2.0", "id": 1,
  "method": "rates.treasury_range",
  "params": { "start": "2026-04-01", "end": "2026-04-15" }
}
```

**Response:** array of `YieldCurve` objects.

### `rates.health`

Service health and cache statistics.

```json
{ "jsonrpc": "2.0", "id": 1, "method": "rates.health", "params": {} }
```

**Response:**
```json
{
  "jsonrpc": "2.0", "id": 1,
  "result": { "status": "ok", "treasury_rows": 240, "sofr_rows": 20 }
}
```

## REST

### `GET /treasury/curve/:date`

```bash
curl http://localhost:8080/treasury/curve/2026-04-15 | jq
```

Returns a `YieldCurve` JSON object. 404 if the date is not in cache.

### `GET /treasury/range?start=YYYY-MM-DD&end=YYYY-MM-DD`

```bash
curl "http://localhost:8080/treasury/range?start=2026-04-01&end=2026-04-15" | jq
```

Returns an array of `YieldCurve` objects.

### `GET /sofr/:date`

```bash
curl http://localhost:8080/sofr/2026-04-15 | jq
```

Returns a `SofrRate` JSON object. 404 if not in cache.

### `GET /health`

```bash
curl http://localhost:8080/health | jq
```

Returns `{ status, treasury_rows, sofr_rows }`.

### `GET /openapi.json`

Machine-readable OpenAPI 3.0 specification.
