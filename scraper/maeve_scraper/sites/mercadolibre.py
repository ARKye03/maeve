import json
from urllib.parse import parse_qs, urlparse

from scrapling.parser import Selector

from ..fetch import fetch_plain, stealth
from ..parse import clean, number_to_cents, parse_int, parse_price, parse_rating
from ..protocol import ScrapeError, build_product

MAX_REVIEWS = 20


def _ld_blocks(sel: Selector) -> list[dict]:
    blocks: list[dict] = []
    for script in sel.css('script[type="application/ld+json"]'):
        try:
            data = json.loads(script.text)
        except (ValueError, TypeError):
            continue
        stack = data if isinstance(data, list) else [data]
        for item in stack:
            if isinstance(item, dict):
                blocks.extend(item["@graph"] if "@graph" in item else [item])
    return [b for b in blocks if isinstance(b, dict)]


def _ld_product(sel: Selector) -> dict | None:
    for block in _ld_blocks(sel):
        kind = block.get("@type")
        kinds = kind if isinstance(kind, list) else [kind]
        if "Product" in kinds:
            return block
    return None


def _selected_variant(url: str) -> str | None:
    parsed = urlparse(url)
    for source in (parsed.query, parsed.fragment):
        values = parse_qs(source).get("variation")
        if values:
            return values[0]
    return None


def _offer_price(offers: object, url: str) -> tuple[int, str, bool] | None:
    """(cents, currency, price_is_range) from a JSON-LD `offers` value."""
    items = offers if isinstance(offers, list) else [offers]
    items = [o for o in items if isinstance(o, dict)]
    if not items:
        return None

    if len(items) == 1 and "lowPrice" in items[0]:
        offer = items[0]
        try:
            low = number_to_cents(offer["lowPrice"])
            high = number_to_cents(offer.get("highPrice", offer["lowPrice"]))
        except ValueError:
            return None
        return low, offer.get("priceCurrency", "UYU"), high != low

    priced = []
    for offer in items:
        raw = offer.get("price")
        if raw is None and isinstance(offer.get("priceSpecification"), dict):
            raw = offer["priceSpecification"].get("price")
        if raw is None:
            continue
        try:
            cents = number_to_cents(raw)
        except ValueError:
            continue
        priced.append((cents, offer.get("priceCurrency", "UYU"), offer))
    if not priced:
        return None

    variant = _selected_variant(url)
    if variant:
        for cents, currency, offer in priced:
            refs = [urlparse(str(offer.get(key, ""))) for key in ("url", "@id")]
            if variant in (str(offer.get("sku", "")), str(offer.get("@id", ""))) or any(
                variant in parse_qs(part).get("variation", []) for ref in refs for part in (ref.query, ref.fragment)
            ):
                return cents, currency, False
    cents, currency, _ = min(priced, key=lambda p: p[0])
    return cents, currency, len({p[0] for p in priced}) > 1


def _first(value: object) -> str | None:
    if isinstance(value, list):
        value = value[0] if value else None
    if isinstance(value, dict):
        value = value.get("url")
    return value if isinstance(value, str) and value else None


def _specs(sel: Selector) -> list[tuple[str, str]]:
    specs: list[tuple[str, str]] = []
    seen: set[str] = set()
    for row in sel.css("table.andes-table tr"):
        key = clean(row.css("th")[0].get_all_text()) if row.css("th") else ""
        value = clean(row.css("td")[0].get_all_text()) if row.css("td") else ""
        if key and value and key not in seen:
            seen.add(key)
            specs.append((key, value))
    return specs


def _reviews(sel: Selector, product: dict | None) -> list[str]:
    texts = [
        clean(node.get_all_text())
        for node in sel.css(
            '.ui-review-capability-comments__comment__content, [data-testid="comment-content-component"]'
        )
    ]
    if not any(texts) and product:
        reviews = product.get("review")
        for review in reviews if isinstance(reviews, list) else []:
            if isinstance(review, dict):
                texts.append(clean(review.get("reviewBody")))
    return [t for t in texts if t][:MAX_REVIEWS]


def parse(html: str, url: str) -> dict:
    sel = Selector(html)
    ld = _ld_product(sel)

    title = clean(ld.get("name")) if ld else ""
    if not title and sel.css("h1.ui-pdp-title"):
        title = clean(sel.css("h1.ui-pdp-title")[0].get_all_text())
    title = title or clean(sel.css('meta[property="og:title"]::attr(content)').get())

    price = _offer_price(ld.get("offers"), url) if ld else None
    if price is None:
        meta = sel.css('meta[itemprop="price"]::attr(content)').get()
        if meta:
            currency = sel.css('meta[itemprop="priceCurrency"]::attr(content)').get() or "UYU"
            try:
                price = (number_to_cents(meta), currency, False)
            except ValueError:
                price = None
    if price is None:
        text = sel.css(".ui-pdp-price__second-line .andes-money-amount")
        if text:
            try:
                cents, currency = parse_price(clean(text[0].get_all_text()))
                price = (cents, currency, False)
            except ValueError:
                price = None

    if not title or price is None:
        if "captcha" in html[:5000].lower():
            raise ScrapeError("blocked", "Mercado Libre served a captcha")
        raise ScrapeError("layout", "could not find title and price on the page")

    cents, currency, is_range = price
    rating_block = (ld or {}).get("aggregateRating") or {}
    return build_product(
        site="mercadolibre",
        url=url,
        title=title,
        cents=cents,
        currency=currency,
        image_url=_first((ld or {}).get("image"))
        or sel.css('meta[property="og:image"]::attr(content)').get()
        or None,
        price_is_range=is_range,
        rating=parse_rating(rating_block.get("ratingValue")),
        review_count=parse_int(rating_block.get("ratingCount") or rating_block.get("reviewCount")),
        specs=_specs(sel),
        reviews=_reviews(sel, ld),
    )


async def scrape(url: str) -> dict:
    try:
        return parse(await fetch_plain(url), url)
    except ScrapeError as first:
        if first.kind not in ("blocked", "layout", "network"):
            raise
    return parse(await stealth.fetch(url), url)
