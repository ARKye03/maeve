import re
from decimal import Decimal, InvalidOperation

_USD_MARKERS = re.compile(r"U\$S|US\$|USD", re.IGNORECASE)
_UYU_MARKERS = re.compile(r"UYU|\$U|\$", re.IGNORECASE)
_NUMBER = re.compile(r"\d[\d.,]*")


def _to_cents(value: Decimal) -> int:
    return int((value * 100).to_integral_value())


def parse_price(text: str) -> tuple[int, str]:
    """Parse a Uruguayan-formatted price into (cents, currency).

    "." groups thousands and "," marks decimals. A lone "." is only read as a
    decimal point for USD with 1-2 trailing digits (TEMU's "US$12.99"), never for UYU.
    """
    currency = "USD" if _USD_MARKERS.search(text) else "UYU"
    cleaned = _UYU_MARKERS.sub(" ", _USD_MARKERS.sub(" ", text))
    match = _NUMBER.search(cleaned)
    if not match:
        raise ValueError(f"unparseable price: {text!r}")
    raw = match.group().rstrip(".,")

    last_dot, last_comma = raw.rfind("."), raw.rfind(",")
    if last_dot >= 0 and last_comma >= 0:
        dec_pos = max(last_dot, last_comma)
    elif last_comma >= 0:
        grouping = currency == "USD" and len(raw) - last_comma - 1 == 3
        dec_pos = last_comma if raw.count(",") == 1 and not grouping else -1
    elif (
        last_dot >= 0
        and currency == "USD"
        and raw.count(".") == 1
        and len(raw) - last_dot - 1 <= 2
    ):
        dec_pos = last_dot
    else:
        dec_pos = -1

    if dec_pos >= 0:
        integer = re.sub(r"[.,]", "", raw[:dec_pos])
        fraction = raw[dec_pos + 1 :]
        digits = f"{integer or '0'}.{fraction}"
    else:
        digits = re.sub(r"[.,]", "", raw)
    try:
        return _to_cents(Decimal(digits)), currency
    except InvalidOperation as exc:
        raise ValueError(f"unparseable price: {text!r}") from exc


def number_to_cents(value: object) -> int:
    """Convert a machine-readable decimal (JSON-LD, meta tags) where "." is the decimal point."""
    try:
        return _to_cents(Decimal(str(value).strip()))
    except InvalidOperation as exc:
        raise ValueError(f"unparseable number: {value!r}") from exc


def parse_int(value: object) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return int(value)
    digits = re.sub(r"\D", "", str(value or ""))
    return int(digits) if digits else None


def parse_rating(value: object) -> float | None:
    try:
        rating = float(str(value).replace(",", "."))
    except ValueError:
        return None
    return rating if 0 < rating <= 5 else None


def clean(text: str | None) -> str:
    return re.sub(r"\s+", " ", text or "").strip()
