# Maeve

Desktop app (Rust, `iced`) that compares product listings from Mercado Libre UY and TEMU. It scrapes each link, computes landed cost in UYU (shipping, IVA or Régimen, FX), has Gemini argue for each product, and a judge picks a winner. Comparisons are saved locally.

## Setup

- Rust (edition 2024) and [`uv`](https://docs.astral.sh/uv/)
- Scraper sidecar (Python, Scrapling):
  ```sh
  cd scraper
  uv sync
  uv run scrapling install
  ```
- Gemini API key: paste it in Settings; it is stored in the macOS Keychain.

## Run

```sh
cargo run --release
```

## Develop

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- `cd scraper && uv run pytest`
- `scripts/snap.sh [compare|history|settings]` writes screenshots of every theme and language combination to `target/snaps/` using demo data

Design and plan: `docs/superpowers/`.
