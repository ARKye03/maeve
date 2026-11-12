import asyncio

import pytest

from maeve_scraper import router
from maeve_scraper.protocol import ScrapeError
from maeve_scraper.router import site_for, validate_url


@pytest.mark.parametrize(
    ("url", "site", "safe"),
    [
        (
            "https://www.mercadolibre.com.uy/x/p/MLU62906158",
            "mercadolibre",
            "https://www.mercadolibre.com.uy/x/p/MLU62906158",
        ),
        (
            "HTTP://Articulo.MercadoLibre.com.uy/MLU-1-x?a=1#f",
            "mercadolibre",
            "http://articulo.mercadolibre.com.uy/MLU-1-x?a=1#f",
        ),
        ("https://www.temu.com:443/uy/g-607586053128116.html", "temu", "https://www.temu.com/uy/g-607586053128116.html"),
        ("https://share.temu.com/abc", "temu", "https://share.temu.com/abc"),
        ("https://mercadolibre.com.uy/ñandú", "mercadolibre", "https://mercadolibre.com.uy/ñandú"),
    ],
)
def test_accepts_and_rebuilds(url, site, safe):
    assert validate_url(url) == (site, safe)


@pytest.mark.parametrize(
    "url",
    [
        "",
        "https://amazon.com/x",
        "https://articulo.mercadolibre.com.ar/MLA-1-x",
        "https://evilmercadolibre.com.uy/x",
        "https://mercadolibre.com.uy.evil.com/x",
        "https://temu.com.evil.com/x",
        "ftp://www.temu.com/x",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "//www.temu.com/x",
        "www.temu.com/x",
        "https://user@www.temu.com/x",
        "https://user:pw@www.temu.com/x",
        "https://evil.com@www.temu.com/x",
        "https://www.temu.com@evil.com/x",
        "https://evil.com\\@www.temu.com/x",
        "https://evil.com\\.temu.com/x",
        "https://www.temu.com\\x",
        "https://www.temu.com/x\n",
        "https://www.temu.com/\tx",
        "https://www.temu.com/x\r\nHost: evil.com",
        "https://www.temu.com/x\x00",
        "https://www.temu.com/x y",
        "https://www.temu.com/x​",
        "https://www.temu.com:8080/x",
        "https://www.temu.com:bad/x",
        "https://[::1/x",
    ],
)
def test_rejects(url):
    with pytest.raises(ScrapeError) as err:
        validate_url(url)
    assert err.value.kind == "unsupported"
    assert site_for(url) is None


def test_scrape_passes_rebuilt_url_to_site(monkeypatch):
    seen = []

    async def fake(url):
        seen.append(url)
        return {}

    from maeve_scraper.sites import mercadolibre

    monkeypatch.setattr(mercadolibre, "scrape", fake)
    asyncio.run(router.scrape("HTTPS://WWW.MercadoLibre.com.uy/x"))
    assert seen == ["https://www.mercadolibre.com.uy/x"]


def test_scrape_rejects_before_fetching(monkeypatch):
    from maeve_scraper.sites import mercadolibre, temu

    async def boom(url):
        raise AssertionError("fetcher must not run")

    monkeypatch.setattr(mercadolibre, "scrape", boom)
    monkeypatch.setattr(temu, "scrape", boom)
    with pytest.raises(ScrapeError):
        asyncio.run(router.scrape("https://www.temu.com@evil.com/x"))
