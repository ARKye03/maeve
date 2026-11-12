pub mod cost;
pub mod money;

use serde::{Deserialize, Serialize};

pub use cost::{CostBreakdown, CostSettings, FxRate, ShippingCost, TaxKind, TaxLine, landed_cost};
pub use money::format_money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Site {
    MercadoLibre,
    Temu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Uyu,
    Usd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub cents: i64,
    pub currency: Currency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Price,
    Shipping,
    Rating,
    ReviewCount,
    Specs,
    Reviews,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Product {
    pub site: Site,
    pub url: String,
    pub title: String,
    pub image_url: Option<String>,
    pub price: Money,
    pub price_is_range: bool,
    pub shipping: Option<Money>,
    pub rating: Option<f32>,
    pub review_count: Option<u32>,
    pub specs: Vec<(String, String)>,
    pub reviews: Vec<String>,
    pub missing: Vec<Field>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_matches_sidecar_json() {
        let json = r#"{
            "site": "temu",
            "url": "https://www.temu.com/x-g-601099.html",
            "title": "Auriculares",
            "image_url": null,
            "price": {"cents": 120000, "currency": "UYU"},
            "price_is_range": false,
            "shipping": {"cents": 15000, "currency": "USD"},
            "rating": 4.5,
            "review_count": 120,
            "specs": [["Color", "Negro"]],
            "reviews": ["Bueno"],
            "missing": ["review_count", "specs"]
        }"#;
        let product: Product = serde_json::from_str(json).unwrap();
        assert_eq!(product.site, Site::Temu);
        assert_eq!(
            product.price,
            Money {
                cents: 120_000,
                currency: Currency::Uyu
            }
        );
        assert_eq!(product.shipping.unwrap().currency, Currency::Usd);
        assert_eq!(product.specs, vec![("Color".into(), "Negro".into())]);
        assert_eq!(product.missing, vec![Field::ReviewCount, Field::Specs]);

        let back = serde_json::to_value(&product).unwrap();
        assert_eq!(
            back,
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
        assert_eq!(serde_json::from_value::<Product>(back).unwrap(), product);
    }

    #[test]
    fn site_serde_names() {
        assert_eq!(
            serde_json::to_string(&Site::MercadoLibre).unwrap(),
            "\"mercadolibre\""
        );
    }
}
