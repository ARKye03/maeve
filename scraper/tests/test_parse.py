import pytest

from maeve_scraper.parse import number_to_cents, parse_price


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        ("$ 1.234", (123400, "UYU")),
        ("U$S 49,90", (4990, "USD")),
        ("US$ 1.299,00", (129900, "USD")),
        ("$1.234,5", (123450, "UYU")),
        ("UYU 15.999", (1599900, "UYU")),
        ("US$12.99", (1299, "USD")),
        ("$ 1.234.567", (123456700, "UYU")),
        ("$ 49,90", (4990, "UYU")),
    ],
)
def test_parse_price(text, expected):
    assert parse_price(text) == expected


def test_lone_dot_in_uyu_is_never_decimal():
    assert parse_price("$ 12.50") == (125000, "UYU")


@pytest.mark.parametrize("text", ["", "gratis", "$"])
def test_parse_price_rejects_garbage(text):
    with pytest.raises(ValueError):
        parse_price(text)


def test_number_to_cents_uses_dot_decimal():
    assert number_to_cents(3499) == 349900
    assert number_to_cents("1234.5") == 123450
