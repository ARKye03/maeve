import asyncio
import sys
import urllib.request

from scrapling.fetchers import AsyncFetcher, AsyncStealthySession

from .protocol import ScrapeError
from .router import validate_url

BLOCK_MARKERS = ("captcha", "account-verification", "px-captcha", "access denied")


def looks_blocked(status: int, html: str) -> bool:
    if status in (403, 429):
        return True
    head = html[:5000].lower()
    return any(marker in head for marker in BLOCK_MARKERS)


async def fetch_plain(url: str) -> str:
    try:
        response = await AsyncFetcher.get(url, timeout=30)
    except Exception as exc:
        raise ScrapeError("network", f"request failed: {exc}") from exc
    html = response.html_content
    if looks_blocked(response.status, html):
        raise ScrapeError("blocked", f"blocked (HTTP {response.status})")
    return html


INSTALL_WAIT_S = 40


class BrowserInstall:
    """Downloads the Chromium build patchright needs (Scrapling's own `install`
    fetches Playwright's build instead). A no-op once it is present."""

    def __init__(self) -> None:
        self._task: asyncio.Task | None = None

    def start(self) -> asyncio.Task:
        if self._task is None:
            self._task = asyncio.create_task(self._run())
        return self._task

    async def _run(self) -> None:
        proc = await asyncio.create_subprocess_exec(
            sys.executable, "-m", "patchright", "install", "chromium",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.PIPE,
        )
        _, err = await proc.communicate()
        if proc.returncode != 0:
            lines = err.decode(errors="replace").strip().splitlines()
            raise RuntimeError(lines[-1] if lines else f"exit code {proc.returncode}")

    async def ready(self) -> None:
        task = self.start()
        try:
            await asyncio.wait_for(asyncio.shield(task), INSTALL_WAIT_S)
        except TimeoutError:
            raise ScrapeError(
                "browser_setup", "still downloading the scraper browser; retry in a minute"
            ) from None
        except Exception as exc:
            self._task = None
            raise ScrapeError(
                "browser_setup", f"could not install the scraper browser: {exc}"
            ) from exc


browser = BrowserInstall()


class StealthPool:
    """One lazily started stealth browser, reused for every request."""

    def __init__(self) -> None:
        self._session: AsyncStealthySession | None = None
        self._lock = asyncio.Lock()

    async def _get(self) -> AsyncStealthySession:
        async with self._lock:
            if self._session is None:
                await browser.ready()
                session = AsyncStealthySession(headless=True, network_idle=True, max_pages=5)
                await session.start()
                self._session = session
            return self._session

    async def fetch(self, url: str) -> str:
        try:
            session = await self._get()
            response = await session.fetch(url)
        except ScrapeError:
            raise
        except Exception as exc:
            raise ScrapeError("network", f"browser fetch failed: {exc}") from exc
        html = response.html_content
        if looks_blocked(response.status, html):
            raise ScrapeError("blocked", f"blocked (HTTP {response.status})")
        return html

    async def close(self) -> None:
        if self._session is not None:
            await self._session.close()
            self._session = None


stealth = StealthPool()


class _AllowedRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _resolve_sync(url: str) -> str:
    opener = urllib.request.build_opener(_AllowedRedirects)
    request = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "Mozilla/5.0"})
    with opener.open(request, timeout=20) as response:
        return response.geturl()


async def resolve_redirect(url: str) -> str:
    try:
        return await asyncio.to_thread(_resolve_sync, url)
    except ScrapeError:
        raise
    except Exception as exc:
        raise ScrapeError("network", f"could not resolve {url}: {exc}") from exc
