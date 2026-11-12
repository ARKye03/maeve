# maeve-scraper

Scrapling sidecar for Maeve. Speaks JSON lines on stdin/stdout.

- Start: `uv run python -m maeve_scraper` prints `{"ready": true}`
- Request: `{"id": 1, "url": "https://…"}`
- Response: `{"id": 1, "ok": true, "product": {…}}` or `{"id": 1, "ok": false, "error": {"kind": "blocked|layout|network|unsupported", "message": "…"}}`
- Sites: Mercado Libre UY, TEMU
- Test: `uv run pytest`
