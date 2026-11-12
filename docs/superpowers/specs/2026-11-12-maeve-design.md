# Maeve — Product Comparison App: Design

Date: 2026-11-12 · Status: draft, awaiting approval

## Intent

Personal desktop app. User compares Mercado Libre Uruguay and TEMU listings tens of times a month. Today: manual copy of page data into an AI model, because models can't fetch product pages. Maeve removes that step: paste URLs → scrape → AI debate → verdict + stats the user can verify.

Decision drivers (user's words): **specs, price with shipping, reviews**.

## Goals

- Compare 2+ products of the same category from URLs (Mercado Libre UY, TEMU; mixing allowed).
- Scrape specs, price, shipping, rating, review count, review text.
- Adversarial evaluation: one Gemini advocate per product, one Gemini judge.
- Presets + free-text purpose ("best for the price to do X").
- Plain-language verdict plus a stats table/charts from scraped data.
- Real landed cost in UYU, incl. TEMU import tax.
- Local history of comparisons, reopenable without re-scraping or re-calling Gemini.
- UI in Spanish and English; light / dark / system theme.
- Efficient: native Rust UI, async work off the UI thread, parallel scraping and advocates.

## Non-goals

- Multi-user, cloud sync, accounts, distribution/packaging for others.
- Other marketplaces or other Mercado Libre countries (design should not block adding them).
- Price tracking or alerts.
- Multi-round debates (one advocate round + judge is v1).

## Anti-goals

User named none. My call: the app must never present invented numbers as scraped data — every stat traces to scraped/pasted input or is visibly marked as estimated (import tax, converted currency).

## Constraints

- Rust, edition 2024, `iced` 0.14 (already in `Cargo.toml`). Follow the project's `rust-best-practices` and `rust-async-patterns` skills.
- Scraper: original Python **Scrapling** (D4Vinci/Scrapling). Local Python setup is acceptable. No Rust port (`scrapling-rs`, `rust_scrapling` evaluated: unmaintained / single-author, rejected).
- LLM: Gemini API.
- Storage: Turso, local embedded (`turso` crate, 0.8.x). No server.
- Platform: macOS (user's machine). Personal use only.
- `docs/theme.css` is reference only; delete it once the iced theme is ported. Token values are copied below so the doc stands alone.

## Architecture

```
iced app (Rust)
 ├─ Scraper client ──JSON lines over stdin/stdout──▶ Python sidecar (Scrapling)
 ├─ Gemini client (HTTP)
 ├─ Cost engine (pure Rust: shipping, IVA / régimen, FX)
 ├─ Store (turso, local file)
 └─ Settings + secrets (macOS Keychain for API key)
```

- **Python sidecar** (my call): long-lived process started on first compare, kept alive so the stealth browser is reused. Lives in `scraper/` with its own `pyproject.toml`, run via `uv`. Protocol: one JSON request per line `{id, url}` → one JSON response `{id, ok, product | error}`. Rust side owns timeouts and restarts it if it dies.
- Scraper sits behind a Rust trait so a site can later move to native Rust (`wreq`) without touching the rest.
- All I/O runs as `iced::Task`s on tokio; UI thread never blocks.

## Pipeline

1. User enters ≥2 URLs, picks preset, optional purpose, clicks **Compare**.
2. **Scrape** all URLs in parallel. Per product: title, image URL, price + currency, shipping (if found), rating, review count, specs (key/value), review text sample (Mercado Libre only; TEMU text optional, weighted lower).
3. **Cost**: landed cost per product (see Cost model).
4. **Advocates**: one Gemini call per product, in parallel. Input: all products' structured data + preset + purpose + UI language. Task: argue for own product, attack the others, using only provided data.
5. **Judge**: one Gemini call. Input: all data + all arguments. Output (structured JSON): winner, per-product score, spec-row alignment, plain-language verdict in UI language.
   - Spec alignment (my call, enforces the anti-goal): the judge only maps each product's scraped spec keys to a shared row label and may attach a parsed numeric value + unit for comparison. Table cells always show the raw scraped/pasted text. "Best value per row" and charts use the parsed numeric only when every product in the row has one; parsed/converted values are marked as AI-derived on hover. Missing specs show "—", never filled in.
6. Render results; save to history.

Progress shown per product: scraping → advocate → judge.

## Cost model (all shown in UYU)

- **Mercado Libre UY**: price assumed IVA-inclusive. Shipping box defaults to `$0` when price ≥ `$1.000` UYU (threshold in settings); otherwise scraped value, or empty + highlighted.
- **TEMU**: displayed prices exclude Uruguayan tax (user: "22% IVA always", added on our side). Scraper records the displayed currency; FX converts if not UYU. Shipping box defaults to `$0`.
  - Default: `product + shipping + IVA 22%`, shown as separate lines, e.g. `Producto $1.200 · Envío $150 · IVA $297 · Total $1.647`.
  - **Régimen Simplificado** toggle (global setting): replaces IVA with a 60% charge, same breakdown.
  - Both rates editable (defaults 22 / 60). My call: the rate applies to product + shipping.
- Shipping boxes are editable per product; edits recompute stats instantly; Gemini re-runs only on Compare.
- **FX**: USD (`U$S`) prices converted to UYU with a daily rate, cached for the day; converted values marked as converted. Rate source left to builder (free API); manual override in settings (my call).

## Scrape failure fallback

- Failed product shows an inline error (`TEMU blocked the request`, `Couldn't find price`); other products continue.
- Actions: **Retry** or **Paste instead**: text (clipboard) or screenshot (clipboard or file drop). Gemini extracts the same fields from it.
- Partial scrapes: only missing fields flagged; user can paste just for those.

### Run behavior with missing data (my call)

- Compare never blocks. Products that failed scraping are excluded from advocates/judge, with a banner naming them; fixing one (Retry/Paste) and clicking Compare re-runs.
- ML item under the free-shipping threshold with no scraped shipping: runs with shipping `unknown`; the judge is told it's unknown; the stats cell is highlighted.
- Any edit after a verdict (shipping, paste, settings that change costs) marks the verdict **stale** with a "Re-run" button. Stats update immediately regardless.
- Reopened history item: edits + re-run save as a new history entry; the original stays unchanged.

### Variants (my call)

Use the variant selected in the URL. If none is selected and prices differ, use the lowest price, show it as "desde $X", and tell the judge it's a range.

## Presets

Built-in: Best overall · Best quality-price · Cheapest decent · Custom (free-text criteria). Purpose field ("to do X") is independent of preset and goes to advocates and judge. Custom presets saved for reuse (my call).

## UI — Layout A (single scroll), chosen in mockup

- **Header**: app name · theme switch (light / dark / system) · language switch (ES/EN).
- **Input card**: URL list (min 2, "+ add URL", remove), preset chips, purpose field, Compare button, per-product status + shipping box + cost breakdown.
- **Results card** (below):
  - Verdict: winner highlighted, plain language, in UI language.
  - Stats table: landed cost breakdown, rating, review count, aligned spec rows; best value per row highlighted. Small bar charts for numeric rows (my call, `iced` canvas).
  - Debate: each advocate's argument, collapsed by default.
- **History**: list of past comparisons (date, products, winner, preset); opening one restores the full results view.
- **Settings**: Gemini API key, model choices, IVA %, régimen toggle + %, ML free-shipping threshold, FX override, default language, theme.

## Theme (port of tweakcn theme to an `iced` custom `Theme`)

Radius `0.5rem` (8px base; sm 4, md 6, lg 8, xl 12). Shadow: subtle `0 1px 3px rgb(0 0 0 / .1)`. Fonts: system sans / mono.

| Token | Light | Dark |
|---|---|---|
| background | `oklch(0.9821 0 0)` | `oklch(0.1776 0 0)` |
| foreground | `oklch(0.2435 0 0)` | `oklch(0.9491 0 0)` |
| card | `oklch(0.9911 0 0)` | `oklch(0.2134 0 0)` |
| primary | `oklch(0.4341 0.0392 41.9938)` | `oklch(0.9247 0.0524 66.1732)` |
| primary-foreground | `oklch(1 0 0)` | `oklch(0.2029 0.024 200.1962)` |
| secondary | `oklch(0.92 0.0651 74.3695)` | `oklch(0.3163 0.019 63.6992)` |
| secondary-foreground | `oklch(0.3499 0.0685 40.8288)` | `oklch(0.9247 0.0524 66.1732)` |
| muted | `oklch(0.9521 0 0)` | `oklch(0.252 0 0)` |
| muted-foreground | `oklch(0.5032 0 0)` | `oklch(0.7699 0 0)` |
| accent | `oklch(0.931 0 0)` | `oklch(0.285 0 0)` |
| destructive | `oklch(0.6271 0.1936 33.339)` | same |
| border | `oklch(0.8822 0 0)` | `oklch(0.2351 0.0115 91.7467)` |
| input | `oklch(0.8822 0 0)` | `oklch(0.4017 0 0)` |
| ring | = primary | = primary |
| chart-1…5 | `0.4341 0.0392 41.99`, `0.92 0.0651 74.37`, `0.931 0 0`, `0.9367 0.0523 75.50`, `0.4338 0.0437 41.67` | `0.9247 0.0524 66.17`, `0.3163 0.019 63.70`, `0.285 0 0`, `0.3481 0.0219 67.00`, `0.9245 0.0533 67.09` |

Convert OKLCH → sRGB at build time or as constants. "System" follows the OS appearance and updates live if feasible.

## i18n

All UI strings in ES and EN; switchable at runtime. Gemini output language follows UI language. Number/currency formatting Uruguayan style (`$1.647`, `U$S 49`).

## Gemini tier

User is on the **free tier** for now; paid may come later.

- Rate limiting is first-class: a shared client-side limiter (requests/minute per model, configurable in settings) queues advocate calls instead of firing all at once; 429s retry with backoff honoring the server's retry delay. Progress UI shows "waiting for rate limit" honestly.
- Defaults pick models with usable free-tier limits; judge model is configurable so it can be upgraded when a paid key arrives.
- Raising the limits in settings is all a paid key needs; no code changes.
- Free-tier prompts may be used by Google for training. Accepted: only public listing data and the user's purpose text are sent.

## Secrets & privacy

Gemini API key in macOS Keychain (my call, e.g. `keyring` crate), never in plain files or the DB. Data stays local except calls to the stores and Gemini.

## Left to the builder

- Crate choices beyond those named (HTTP client, i18n approach, keyring, charts), module layout, error types.
- Exact Gemini model IDs (defaults: fast model for advocates, strongest for judge; configurable).
- Prompt wording and JSON schemas for advocates/judge/paste-extraction.
- Scrapling selectors and fetcher mode per site; review sample size.
- DB schema and migrations.
- FX rate source.
- Testing strategy (pure cost engine and parsing are the obvious unit-test targets).
