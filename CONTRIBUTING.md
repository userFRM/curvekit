# Contributing to curvekit

Thank you for your interest in curvekit!

## Local setup

```bash
git clone https://github.com/userFRM/curvekit
cd curvekit
cargo build --workspace
cargo test --workspace
```

Rust stable (1.77+) is required. No system libraries are needed — all
dependencies are pure Rust or bundled.

## Before submitting a PR

Run the full local CI check:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three must pass. Live-network tests (fetching from treasury.gov / NY Fed
/ GitHub raw) are gated behind `#[ignore]` and are not required for CI.

## Pull request conventions

- One logical change per PR.
- Include a one-sentence "why" in the PR description.
- Update `CHANGELOG.md` under `[Unreleased]`.
- No external API keys — all data sources are public.

## Data source changes

If you change the Treasury or SOFR source URL or CSV schema, update
`docs/data-sources.md` to match and add a fixture CSV to `tests/fixtures/`.

If you change the parquet schema (`data/*.parquet`), update
`docs/architecture.md` and the schema tables in `crates/curvekit/src/`
accordingly.

## License

By contributing, you agree your contributions will be licensed under
the Apache-2.0 License.
