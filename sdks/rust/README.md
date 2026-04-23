# curvekit-sdk

Rust client for the [curvekit](https://github.com/userFRM/curvekit) rate service.

## Usage

Add to `Cargo.toml`:

```toml
curvekit-sdk = { git = "https://github.com/userFRM/curvekit", branch = "main" }
```

Then:

```rust
use curvekit_sdk::CurvekitClient;
use chrono::NaiveDate;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = CurvekitClient::new("http://localhost:8080");
    let date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();

    let curve = client.treasury_curve(date).await?;
    let sofr  = client.sofr(date).await?;
    let r_45d = client.rate_for_days(date, 45).await?;

    println!("45-day rate: {:.4}%", r_45d * 100.0);
    Ok(())
}
```

The server must be running (`curvekit-server --port 8080`).
All rates are continuously compounded.
