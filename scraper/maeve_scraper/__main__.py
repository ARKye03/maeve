import asyncio
import sys
from collections.abc import Awaitable, Callable

from . import router
from .fetch import browser, stealth
from .protocol import ScrapeError, error_response, ok_response, parse_request

Scrape = Callable[[str], Awaitable[dict]]
ParsePage = Callable[[str, str], dict]


async def handle_line(
    line: str, scrape: Scrape = router.scrape, parse_page: ParsePage = router.parse_page
) -> str:
    try:
        req = parse_request(line)
    except ValueError as exc:
        return error_response(None, "layout", f"malformed request: {exc}")
    try:
        if req.html is not None:
            product = await asyncio.to_thread(parse_page, req.url, req.html)
        else:
            product = await scrape(req.url)
        return ok_response(req.id, product)
    except ScrapeError as exc:
        return error_response(req.id, exc.kind, exc.message)
    except Exception as exc:
        return error_response(req.id, "layout", f"unexpected error: {exc}")


def emit(line: str) -> None:
    sys.stdout.write(line.encode("utf-8", "replace").decode("utf-8") + "\n")
    sys.stdout.flush()


async def serve() -> None:
    emit('{"ready": true}')
    browser.start()
    pending: set[asyncio.Task] = set()

    async def respond(line: str) -> None:
        emit(await handle_line(line))

    while line := await asyncio.to_thread(sys.stdin.readline):
        if not line.strip():
            continue
        task = asyncio.create_task(respond(line))
        pending.add(task)
        task.add_done_callback(pending.discard)

    if pending:
        await asyncio.gather(*pending)
    await stealth.close()


def main() -> None:
    asyncio.run(serve())


if __name__ == "__main__":
    main()
