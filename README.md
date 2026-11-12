# Maeve

Desktop app (Rust, `iced`) that compares product listings from Mercado Libre UY and TEMU. It scrapes each link, computes landed cost in UYU (shipping, IVA or Régimen, FX), has Gemini argue for each product, and a judge picks a winner. Comparisons are saved locally.

## Setup

- Rust (edition 2024) and [`uv`](https://docs.astral.sh/uv/)
- Scraper sidecar (Python, Scrapling):
  ```sh
  cd scraper
  uv sync
  ```
  The sidecar downloads its Chromium build on first use (one time, about a minute).
- Gemini API key: paste it in Settings; it is stored in the macOS Keychain.

## Run

```sh
cargo run --release
```

## Browser extension

TEMU often shows the scraper a captcha. The extension in `extension/` (Brave, Chrome, Edge, Firefox) sends product pages you open in your own browser to Maeve, which fills in the failed row.

- Chromium browsers: `brave://extensions` (or `chrome://extensions`) → Developer mode → Load unpacked → `extension/`.
- Firefox: `about:debugging#/runtime/this-firefox` → Load Temporary Add-on → `extension/manifest.json` (removed on restart unless the add-on is signed).
- In Maeve, click **Abrir en el navegador** on a failed row; solve the captcha if TEMU asks. The toolbar icon shows ✓ once Maeve received the page.
- Maeve listens on `127.0.0.1:47821` and only accepts requests carrying the extension's header.

## Install

```sh
cargo install cargo-bundle
cargo bundle --release            # macOS: target/release/bundle/osx/Maeve.app
cargo bundle --release --format deb  # Linux: target/release/bundle/deb/*.deb
```

- macOS: drag `Maeve.app` into `/Applications`.
- The bundle ships the scraper; it still needs `uv` installed. Its venv lives in the user data dir (`maeve/scraper-venv`).
- Icon source: `assets/icon.svg`; PNGs and `maeve.icns` in `assets/icons/`.

## Develop

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- `cd scraper && uv run pytest`
- `scripts/snap.sh [compare|history|settings]` writes screenshots of every theme and language combination to `target/snaps/` using demo data

Design and plan: `docs/superpowers/`.
