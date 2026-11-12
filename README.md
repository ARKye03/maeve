# Maeve

Maeve compares product listings from Mercado Libre Uruguay and TEMU and tells you which one to buy. It reads each link and works out what the product really costs delivered in Uruguay: shipping, IVA or the simplified import regime, and the exchange rate. Then Gemini argues for each product and a judge model picks the winner. Comparisons are saved on your computer.

## Setup

You need Rust (edition 2024), [`uv`](https://docs.astral.sh/uv/) and a Gemini API key.

1. Install the scraper sidecar (Python, Scrapling):
   ```sh
   cd scraper
   uv sync
   ```
   The sidecar downloads its own Chromium build on first use, which takes about a minute.
2. Get a Gemini API key ([how](docs/gemini-api-key.md)) and paste it in **Ajustes**. Maeve checks the key with Gemini and stores it in the macOS Keychain.

## Run

```sh
cargo run --release
```

## Browser extension

TEMU often answers the scraper with a captcha. The extension in `extension/` gets around it: it sends the product pages you open in your own browser to Maeve, which fills in the failed row. It works in Brave, Chrome, Edge and Firefox.

- Chromium browsers: `brave://extensions` (or `chrome://extensions`) → Developer mode → Load unpacked → `extension/`.
- Firefox: `about:debugging#/runtime/this-firefox` → Load Temporary Add-on → `extension/manifest.json`. Firefox removes it on restart unless the add-on is signed.

Then click **Abrir en el navegador** on a failed row and solve the captcha if TEMU asks. The toolbar icon shows ✓ once Maeve has the page.

Maeve listens only on `127.0.0.1:47821` and rejects any request without the extension's header, so ordinary web pages can't post to it.

## Install

```sh
cargo install cargo-bundle
cargo bundle --release            # macOS: target/release/bundle/osx/Maeve.app
cargo bundle --release --format deb  # Linux: target/release/bundle/deb/*.deb
```

On macOS, drag `Maeve.app` into `/Applications`. The bundle ships the scraper but still needs `uv` installed; the scraper's venv lives in your user data folder (`maeve/scraper-venv`). The icon source is `assets/icon.svg`, with the PNGs and `maeve.icns` in `assets/icons/`.

## Develop

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- `cd scraper && uv run pytest`
- `scripts/snap.sh [compare|history|settings]` writes screenshots of every theme and language combination to `target/snaps/` using demo data.

## License

MIT. See [LICENSE](LICENSE).
