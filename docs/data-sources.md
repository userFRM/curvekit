# Data Sources

curvekit pulls from two public, no-authentication-required endpoints.

## US Treasury Yield Curve

**Publisher:** US Department of the Treasury

**URL pattern:**
```
https://home.treasury.gov/resource-center/data-chart-center/interest-rates/
  daily-treasury-rates.csv/all/all
  ?type=daily_treasury_yield_curve
  &field_tdr_date_value={YYYY}
  &page&_format=csv
```

This returns all trading days for the requested year as a CSV.
One HTTP request per calendar year that the requested date range spans.

**Column format:** The response contains a `Date` column (MM/DD/YYYY) and
12 maturity columns: `1 Mo`, `2 Mo`, `3 Mo`, `6 Mo`, `1 Yr`, `2 Yr`, `3 Yr`,
`5 Yr`, `7 Yr`, `10 Yr`, `20 Yr`, `30 Yr`. Values are Bond Equivalent Yields
(BEY) in percent with semi-annual compounding. Missing maturities may be
absent on some dates (e.g. the 20Yr was discontinued for several years).

**Published:** Approximately **15:30 ET** on each business day.

**Rate conversion:** BEY % → APY → continuously compounded:
```
BEY  = column value / 100
APY  = (1 + BEY / 2)^2 − 1
r    = ln(1 + APY)
```

## SOFR (Secured Overnight Financing Rate)

**Publisher:** Federal Reserve Bank of New York

**URL pattern:**
```
https://markets.newyorkfed.org/api/rates/secured/sofr/search.csv
  ?startDate=MM/DD/YYYY
  &endDate=MM/DD/YYYY
```

Returns a CSV with one row per business day.

**Relevant columns:** `Effective Date` (MM/DD/YYYY) and `Rate (%)`.

**Published:** Approximately **08:00 ET** on each business day, reflecting the
previous business day's activity.

**Rate conversion:** Percent → continuously compounded:
```
r = ln(1 + rate_pct / 100)
```

SOFR is stored at the **1-day (overnight)** point in the term structure and
serves as the short-end anchor when `TermStructure::rate_for_days` is called.

## Refresh schedule summary

| Source | Published | curvekit refresh window |
|---|---|---|
| Treasury yield curve | ~15:30 ET, business days | 15:30 ET ± 1 min |
| SOFR | ~08:00 ET, business days | 08:00 ET ± 1 min |

curvekit checks every 60 seconds. On weekends (Saturday, Sunday) the loop
skips without fetching.
