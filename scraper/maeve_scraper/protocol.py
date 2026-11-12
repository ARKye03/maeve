import json
from dataclasses import dataclass
from typing import Any

KINDS = ("blocked", "layout", "network", "unsupported")


class ScrapeError(Exception):
    def __init__(self, kind: str, message: str):
        assert kind in KINDS
        super().__init__(message)
        self.kind = kind
        self.message = message


@dataclass
class Request:
    id: int
    url: str


def parse_request(line: str) -> Request:
    """Raises ValueError on malformed input; the caller answers with id null."""
    data = json.loads(line)
    if not isinstance(data, dict) or "id" not in data or not isinstance(data.get("url"), str):
        raise ValueError("request must be an object with id and url")
    return Request(id=data["id"], url=data["url"])


def ok_response(req_id: Any, product: dict) -> str:
    return json.dumps({"id": req_id, "ok": True, "product": product}, ensure_ascii=False)


def error_response(req_id: Any, kind: str, message: str) -> str:
    return json.dumps(
        {"id": req_id, "ok": False, "error": {"kind": kind, "message": message}},
        ensure_ascii=False,
    )


def build_product(
    *,
    site: str,
    url: str,
    title: str,
    cents: int,
    currency: str,
    image_url: str | None = None,
    price_is_range: bool = False,
    shipping: dict | None = None,
    rating: float | None = None,
    review_count: int | None = None,
    specs: list[tuple[str, str]] | None = None,
    reviews: list[str] | None = None,
    reviews_optional: bool = False,
) -> dict:
    """Product JSON, field-for-field the Rust `Product` serde shape."""
    if isinstance(currency, str):
        currency = currency.strip().upper()
    if currency not in ("UYU", "USD"):
        raise ScrapeError("layout", f"unsupported currency: {currency!r}")
    specs = specs or []
    reviews = reviews or []
    missing = []
    if shipping is None:
        missing.append("shipping")
    if rating is None:
        missing.append("rating")
    if review_count is None:
        missing.append("review_count")
    if not specs:
        missing.append("specs")
    if not reviews and not reviews_optional:
        missing.append("reviews")
    return {
        "site": site,
        "url": url,
        "title": title,
        "image_url": image_url,
        "price": {"cents": cents, "currency": currency},
        "price_is_range": price_is_range,
        "shipping": shipping,
        "rating": rating,
        "review_count": review_count,
        "specs": [list(pair) for pair in specs],
        "reviews": reviews,
        "missing": missing,
    }
