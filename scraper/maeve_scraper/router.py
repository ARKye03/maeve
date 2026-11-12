import unicodedata
from urllib.parse import urlsplit, urlunsplit

from .protocol import ScrapeError

_BAD_CATEGORIES = {"Cc", "Cf", "Zs", "Zl", "Zp"}
_SITES = {"mercadolibre.com.uy": "mercadolibre", "temu.com": "temu"}
_DEFAULT_PORTS = {None, 80, 443}


def _unsupported(url: str) -> ScrapeError:
    return ScrapeError("unsupported", f"unsupported url: {url[:80]!r}")


def validate_url(url: str) -> tuple[str, str]:
    """Return (site, rebuilt url) or raise ScrapeError("unsupported").

    Checks the raw string before parsing, since urlsplit silently strips tabs and
    newlines and tolerates backslashes that browsers treat as slashes.
    """
    if not url or "\\" in url or any(unicodedata.category(c) in _BAD_CATEGORIES for c in url):
        raise _unsupported(url)
    try:
        parts = urlsplit(url)
        host = (parts.hostname or "").lower()
        port = parts.port
    except ValueError as exc:
        raise _unsupported(url) from exc
    if parts.scheme.lower() not in ("http", "https") or "@" in parts.netloc or port not in _DEFAULT_PORTS:
        raise _unsupported(url)
    site = next((s for d, s in _SITES.items() if host == d or host.endswith("." + d)), None)
    if site is None:
        raise _unsupported(url)
    safe = urlunsplit((parts.scheme.lower(), host, parts.path, parts.query, parts.fragment))
    return site, safe


def site_for(url: str) -> str | None:
    try:
        return validate_url(url)[0]
    except ScrapeError:
        return None


async def scrape(url: str) -> dict:
    site, safe_url = validate_url(url)
    if site == "mercadolibre":
        from .sites import mercadolibre

        return await mercadolibre.scrape(safe_url)
    from .sites import temu

    return await temu.scrape(safe_url)
