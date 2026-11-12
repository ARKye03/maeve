import json
import re
from urllib.parse import parse_qs, urlparse

from scrapling.parser import Selector

from ..fetch import resolve_redirect, stealth
from ..parse import clean, parse_int, parse_price, parse_rating
from ..protocol import ScrapeError, build_product
from ..router import validate_url

MAX_REVIEWS = 10
_RAW_DATA = re.compile(r"window\.rawData\s*=\s*")


def _store(html: str) -> dict:
    match = _RAW_DATA.search(html)
    if not match:
        return {}
    try:
        data, _ = json.JSONDecoder().raw_decode(html, match.end())
    except ValueError:
        return {}
    store = data.get("store") if isinstance(data, dict) else None
    return store if isinstance(store, dict) else {}


def _dict(value: object) -> dict:
    return value if isinstance(value, dict) else {}


def _selected_sku(url: str) -> str | None:
    query = parse_qs(urlparse(url).query)
    for key in ("sku_id", "skuId"):
        if query.get(key):
            return query[key][0]
    return None


def _cents(text: object, minor: object, currency: str) -> tuple[int, str] | None:
    """Price from the display string ("$ 230"); the integer minor-unit field is the fallback."""
    if isinstance(text, str) and re.search(r"\d", text):
        try:
            return parse_price(text)
        except ValueError:
            pass
    if isinstance(minor, (int, float)) and not isinstance(minor, bool):
        return int(minor), currency
    return None


def _price(store: dict, url: str) -> tuple[int, str, bool] | None:
    goods = _dict(store.get("goods"))
    currency = str(_dict(store.get("localInfo")).get("currency") or "UYU").upper()

    sku = _selected_sku(url)
    if sku:
        for item in store.get("sku") or []:
            if isinstance(item, dict) and str(item.get("skuId")) == sku:
                found = _cents(item.get("salePriceStr"), item.get("salePrice"), currency)
                if found:
                    return found[0], found[1], False

    low = _cents(goods.get("minOnSalePriceStr"), goods.get("minOnSalePrice"), currency)
    if low is None:
        return None
    high = _cents(goods.get("maxOnSalePriceStr"), goods.get("maxOnSalePrice"), currency)
    return low[0], low[1], high is not None and high[0] != low[0]


def _specs(store: dict) -> list[tuple[str, str]]:
    props = _dict(store.get("goods")).get("goodsProperty") or store.get("goodsProperty") or []
    specs: list[tuple[str, str]] = []
    seen: set[str] = set()
    for item in props:
        if not isinstance(item, dict):
            continue
        values = item.get("values")
        key = clean(str(item.get("key") or ""))
        value = clean(" ".join(map(str, values)) if isinstance(values, list) else "")
        if key and value and key not in seen:
            seen.add(key)
            specs.append((key, value))
    return specs


def _reviews(store: dict) -> list[str]:
    items = _dict(_dict(store.get("review")).get("reviewData")).get("reviewInfoList")
    if not items:
        items = _dict(store.get("reviewStore")).get("commentList")
    texts = [clean(str(i.get("comment") or "")) for i in items or [] if isinstance(i, dict)]
    return [t for t in texts if t][:MAX_REVIEWS]


def parse(html: str, url: str) -> dict:
    sel = Selector(html)
    store = _store(html)
    goods = _dict(store.get("goods"))
    review_data = _dict(_dict(store.get("review")).get("reviewData"))

    title = clean(str(goods.get("goodsName") or ""))
    title = title or clean(sel.css('meta[property="og:title"]::attr(content)').get())

    price = _price(store, url)

    if not title or price is None:
        if "captcha" in html[:5000].lower():
            raise ScrapeError("blocked", "TEMU served a captcha")
        raise ScrapeError("layout", "could not find title and price on the page")

    cents, currency, is_range = price
    image = goods.get("hdThumbUrl") or goods.get("thumbUrl") or sel.css(
        'meta[property="og:image"]::attr(content)'
    ).get()
    return build_product(
        site="temu",
        url=url,
        title=title,
        cents=cents,
        currency=currency,
        image_url=image if isinstance(image, str) and image else None,
        price_is_range=is_range,
        rating=parse_rating(review_data.get("goodsScore")),
        review_count=parse_int(review_data.get("reviewNum")),
        specs=_specs(store),
        reviews=_reviews(store),
        reviews_optional=True,
    )


async def scrape(url: str) -> dict:
    if urlparse(url).hostname == "share.temu.com":
        site, url = validate_url(await resolve_redirect(url))
        if site != "temu":
            raise ScrapeError("unsupported", "redirected to unsupported site")
    return parse(await stealth.fetch(url), url)
