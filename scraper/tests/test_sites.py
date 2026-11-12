import json
import re
from pathlib import Path

import pytest

from maeve_scraper.protocol import ScrapeError
from maeve_scraper.sites import mercadolibre, temu

FIXTURES = Path(__file__).parent / "fixtures"
ML_URL = "https://www.mercadolibre.com.uy/pack-x2-ventiladores-3-en-1-goldtech-5-aspas-metalicas/p/MLU62906158"
TEMU_URL = "https://www.temu.com/uy/juego-de-3-piezas-para-dosificar-g-607586053128116.html"


def load(name: str) -> str:
    return (FIXTURES / name).read_text(encoding="utf-8")


def ml_page(offers: dict | list) -> str:
    ld = {"@type": "Product", "name": "Producto", "offers": offers}
    return f'<script type="application/ld+json">{json.dumps(ld)}</script>'


def temu_page(goods: dict, **store) -> str:
    return "<script>window.rawData=" + json.dumps({"store": {"goods": goods, **store}}) + ";</script>"


def test_mercadolibre_catalog_page():
    product = mercadolibre.parse(load("ml_item.html"), ML_URL)
    assert product["site"] == "mercadolibre"
    assert product["title"] == "Pack X2 Ventiladores 3 En 1 Goldtech 5 Aspas Metálicas"
    assert product["price"] == {"cents": 225900, "currency": "UYU"}
    assert product["price_is_range"] is False
    assert product["rating"] == 4.7
    assert product["review_count"] == 294
    assert product["image_url"] == "https://http2.mlstatic.com/D_NQ_NP_841678-MLA100035710948_122025-O.webp"
    assert product["specs"][:2] == [["Marca", "Goldtech"], ["Línea", "Ventiladores"]]
    assert ["Voltaje", "220V"] in product["specs"]
    assert ["Cantidad de aspas", "5"] in product["specs"]
    assert ["Material de las aspas", "Metal"] in product["specs"]
    assert len(product["specs"]) == 34
    assert product["reviews"][1] == "La verdad muy bien. Recomendados."
    assert len(product["reviews"]) == 5
    assert product["missing"] == ["shipping"]


def test_mercadolibre_falls_back_to_page_markup_without_json_ld():
    html = re.sub(r'<script type="application/ld\+json">.*?</script>', "", load("ml_item.html"), flags=re.S)
    product = mercadolibre.parse(html, ML_URL)
    assert product["title"] == "Pack X2 Ventiladores 3 En 1 Goldtech 5 Aspas Metálicas"
    assert product["price"] == {"cents": 225900, "currency": "UYU"}


def test_mercadolibre_caps_reviews_at_20():
    comments = "".join(
        f'<p class="ui-review-capability-comments__comment__content">r{i}</p>' for i in range(30)
    )
    html = load("ml_item.html").replace("</body>", comments + "</body>")
    reviews = mercadolibre.parse(html, ML_URL)["reviews"]
    assert len(reviews) == 20


def test_mercadolibre_range_uses_lowest_price():
    html = ml_page(
        [
            {"@type": "Offer", "price": 1500, "priceCurrency": "UYU"},
            {"@type": "Offer", "price": 1200, "priceCurrency": "UYU"},
        ]
    )
    product = mercadolibre.parse(html, ML_URL)
    assert product["price"]["cents"] == 120000
    assert product["price_is_range"] is True


def test_mercadolibre_variant_in_url_picks_its_price():
    html = ml_page(
        [
            {"@type": "Offer", "price": 1500, "sku": "V1", "url": "https://x/p?variation=V1"},
            {"@type": "Offer", "price": 1200, "sku": "V2", "url": "https://x/p?variation=V2"},
        ]
    )
    product = mercadolibre.parse(html, ML_URL + "?variation=V1")
    assert product["price"]["cents"] == 150000
    assert product["price_is_range"] is False


def test_mercadolibre_no_price_is_layout_error():
    with pytest.raises(ScrapeError) as err:
        mercadolibre.parse("<html><body><h1>nada</h1></body></html>", ML_URL)
    assert err.value.kind == "layout"


def test_temu_page():
    product = temu.parse(load("temu_item.html"), TEMU_URL)
    assert product["site"] == "temu"
    assert product["title"].startswith("Juego de 3 piezas para dosificar granos de café")
    assert product["price"] == {"cents": 23000, "currency": "UYU"}
    assert product["price_is_range"] is False
    assert product["rating"] == 4.9
    assert product["review_count"] == 28
    assert product["image_url"] == "https://img.kwcdn.com/product/fancy/46453d40-da9b-421e-982e-0d099de637c4.jpg"
    assert product["specs"] == [
        ["Material", "Silicona"],
        ["Fuente de alimentación", "Uso sin electricidad"],
        ["Puede usarse para contacto con alimentos", "Sí"],
        ["ID del artículo", "22M6762KWV"],
        ["Origen", "Guangdong, China"],
    ]
    assert product["reviews"][-1] == "Excelente calidad y resistencia"
    assert len(product["reviews"]) == 4
    assert product["missing"] == ["shipping"]


def test_temu_without_reviews_does_not_flag_them_missing():
    html = load("temu_item.html").replace('"reviewInfoList"', '"other"').replace('"commentList"', '"other2"')
    product = temu.parse(html, TEMU_URL)
    assert product["reviews"] == []
    assert "reviews" not in product["missing"]


def test_temu_caps_reviews_at_10():
    infos = [{"comment": f"c{i}"} for i in range(15)]
    html = temu_page(
        {"goodsName": "X", "minOnSalePriceStr": "$ 10", "minOnSalePrice": 1000},
        review={"reviewData": {"reviewInfoList": infos}},
    )
    assert len(temu.parse(html, TEMU_URL)["reviews"]) == 10


def test_temu_price_range_flagged():
    html = temu_page(
        {"goodsName": "X", "minOnSalePriceStr": "$ 10", "maxOnSalePriceStr": "$ 15"},
    )
    product = temu.parse(html, TEMU_URL)
    assert product["price"]["cents"] == 1000
    assert product["price_is_range"] is True


def test_temu_selected_sku_in_url_picks_its_price():
    html = temu_page(
        {"goodsName": "X", "minOnSalePriceStr": "$ 10", "maxOnSalePriceStr": "$ 15"},
        sku=[{"skuId": 1, "salePrice": 1000}, {"skuId": 2, "salePrice": 1500}],
        localInfo={"currency": "UYU"},
    )
    product = temu.parse(html, TEMU_URL + "?sku_id=2")
    assert product["price"] == {"cents": 1500, "currency": "UYU"}
    assert product["price_is_range"] is False


def test_temu_missing_price_is_layout_error():
    with pytest.raises(ScrapeError) as err:
        temu.parse("<html><body>nothing</body></html>", TEMU_URL)
    assert err.value.kind == "layout"
