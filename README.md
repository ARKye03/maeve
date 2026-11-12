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

## Install

```sh
cargo install cargo-bundle
cargo bundle --release            # macOS: target/release/bundle/osx/Maeve.app
cargo bundle --release --format deb  # Linux: target/release/bundle/deb/*.deb
```

- macOS: drag `Maeve.app` into `/Applications`.
- The bundle ships the scraper; it still needs `uv` installed and `uv run scrapling install` run once. Its venv lives in the user data dir (`maeve/scraper-venv`).
- Icon source: `assets/icon.svg`; PNGs and `maeve.icns` in `assets/icons/`.

## Develop

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- `cd scraper && uv run pytest`
- `scripts/snap.sh [compare|history|settings]` writes screenshots of every theme and language combination to `target/snaps/` using demo data

Design and plan: `docs/superpowers/`.
