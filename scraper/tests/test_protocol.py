import asyncio
import json

from maeve_scraper.__main__ import handle_line
from maeve_scraper.protocol import ScrapeError


def run(line, scrape=None):
    coro = handle_line(line, scrape) if scrape else handle_line(line)
    return json.loads(asyncio.run(coro))


def test_unsupported_host():
    out = run('{"id": 7, "url": "https://amazon.com/x"}')
    assert out["id"] == 7
    assert out["ok"] is False
    assert out["error"]["kind"] == "unsupported"


def test_other_mercadolibre_country_is_unsupported():
    out = run('{"id": 1, "url": "https://articulo.mercadolibre.com.ar/MLA-1-x"}')
    assert out["error"]["kind"] == "unsupported"


def test_malformed_line_answers_with_null_id():
    for line in ("not json", "[]", '{"id": 1}', '{"url": "https://x"}'):
        out = run(line)
        assert out["id"] is None
        assert out["ok"] is False


def test_success_envelope():
    async def scrape(url):
        return {"title": url}

    assert run('{"id": 3, "url": "https://x"}', scrape) == {
        "id": 3,
        "ok": True,
        "product": {"title": "https://x"},
    }


def test_scrape_error_maps_kind_and_message():
    async def scrape(url):
        raise ScrapeError("blocked", "captcha")

    out = run('{"id": 4, "url": "https://x"}', scrape)
    assert out["error"] == {"kind": "blocked", "message": "captcha"}


def test_loop_survives_malformed_line_and_answers_concurrently():
    async def scrape(url):
        await asyncio.sleep(0.05 if url.endswith("slow") else 0)
        return {"url": url}

    async def go():
        lines = ["garbage", '{"id": 1, "url": "https://a/slow"}', '{"id": 2, "url": "https://a/fast"}']
        tasks = [asyncio.create_task(handle_line(line, scrape)) for line in lines]
        order = []
        for done in asyncio.as_completed(tasks):
            order.append(json.loads(await done)["id"])
        return order

    assert asyncio.run(go()) == [None, 2, 1]
