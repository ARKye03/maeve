use serde::{Deserialize, Serialize};

use super::{Currency, Money, Product, Site};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSettings {
    pub iva_bp: u32,
    pub regimen_simplificado: bool,
    pub regimen_bp: u32,
    pub ml_free_shipping_threshold: Money,
}

impl Default for CostSettings {
    fn default() -> Self {
        Self {
            iva_bp: 2200,
            regimen_simplificado: false,
            regimen_bp: 6000,
            ml_free_shipping_threshold: Money {
                cents: 100_000,
                currency: Currency::Uyu,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxRate {
    pub uyu_per_usd: f64,
    pub date: jiff::civil::Date,
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxKind {
    Iva,
    RegimenSimplificado,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxLine {
    pub kind: TaxKind,
    pub bp: u32,
    pub amount: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShippingCost {
    Known(i64),
    Unknown,
}

/// All amounts are UYU cents. `total` excludes unknown shipping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub product: i64,
    pub shipping: ShippingCost,
    pub tax: Option<TaxLine>,
    pub total: i64,
    pub converted: bool,
}

fn to_uyu_cents(m: Money, fx: &FxRate) -> i64 {
    match m.currency {
        Currency::Uyu => m.cents,
        Currency::Usd => (m.cents as f64 * fx.uyu_per_usd + 0.5).floor() as i64,
    }
}

fn apply_bp(base: i64, bp: u32) -> i64 {
    (base * i64::from(bp) + 5_000).div_euclid(10_000)
}

pub fn landed_cost(
    p: &Product,
    shipping_override: Option<Money>,
    fx: &FxRate,
    s: &CostSettings,
) -> CostBreakdown {
    let product = to_uyu_cents(p.price, fx);

    let explicit = shipping_override.or(p.shipping);
    let shipping = match (explicit, p.site) {
        (Some(m), _) => ShippingCost::Known(to_uyu_cents(m, fx)),
        (None, Site::Temu) => ShippingCost::Known(0),
        (None, Site::MercadoLibre) => {
            if product >= to_uyu_cents(s.ml_free_shipping_threshold, fx) {
                ShippingCost::Known(0)
            } else {
                ShippingCost::Unknown
            }
        }
    };
    let shipping_cents = match shipping {
        ShippingCost::Known(c) => c,
        ShippingCost::Unknown => 0,
    };

    let tax = match p.site {
        Site::MercadoLibre => None,
        Site::Temu => {
            let (kind, bp) = if s.regimen_simplificado {
                (TaxKind::RegimenSimplificado, s.regimen_bp)
            } else {
                (TaxKind::Iva, s.iva_bp)
            };
            Some(TaxLine {
                kind,
                bp,
                amount: apply_bp(product + shipping_cents, bp),
            })
        }
    };

    let converted =
        p.price.currency == Currency::Usd || explicit.is_some_and(|m| m.currency == Currency::Usd);

    CostBreakdown {
        product,
        shipping,
        total: product + shipping_cents + tax.map_or(0, |t| t.amount),
        tax,
        converted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uyu(whole: i64) -> Money {
        Money {
            cents: whole * 100,
            currency: Currency::Uyu,
        }
    }

    fn usd(whole: i64) -> Money {
        Money {
            cents: whole * 100,
            currency: Currency::Usd,
        }
    }

    fn fx() -> FxRate {
        FxRate {
            uyu_per_usd: 40.0,
            date: jiff::civil::date(2026, 11, 12),
            manual: false,
        }
    }

    fn product(site: Site, price: Money, shipping: Option<Money>) -> Product {
        Product {
            site,
            url: "https://example.com/p".into(),
            title: "Item".into(),
            image_url: None,
            price,
            price_is_range: false,
            shipping,
            rating: None,
            review_count: None,
            specs: vec![],
            reviews: vec![],
            missing: vec![],
        }
    }

    #[test]
    fn temu_iva_with_override_shipping() {
        let p = product(Site::Temu, uyu(1200), None);
        let c = landed_cost(&p, Some(uyu(150)), &fx(), &CostSettings::default());
        assert_eq!(c.product, 120_000);
        assert_eq!(c.shipping, ShippingCost::Known(15_000));
        let tax = c.tax.unwrap();
        assert_eq!(tax.kind, TaxKind::Iva);
        assert_eq!(tax.bp, 2200);
        assert_eq!(tax.amount, 29_700);
        assert_eq!(c.total, 164_700);
        assert!(!c.converted);
    }

    #[test]
    fn temu_regimen_simplificado() {
        let p = product(Site::Temu, uyu(1200), None);
        let s = CostSettings {
            regimen_simplificado: true,
            ..CostSettings::default()
        };
        let c = landed_cost(&p, Some(uyu(150)), &fx(), &s);
        let tax = c.tax.unwrap();
        assert_eq!(tax.kind, TaxKind::RegimenSimplificado);
        assert_eq!(tax.bp, 6000);
        assert_eq!(tax.amount, 81_000);
        assert_eq!(c.total, 216_000);
    }

    #[test]
    fn temu_without_shipping_taxes_product_only() {
        let p = product(Site::Temu, uyu(1200), None);
        let c = landed_cost(&p, None, &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Known(0));
        assert_eq!(c.tax.unwrap().amount, 26_400);
        assert_eq!(c.total, 146_400);
    }

    #[test]
    fn ml_at_threshold_ships_free_without_tax() {
        let p = product(Site::MercadoLibre, uyu(1000), None);
        let c = landed_cost(&p, None, &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Known(0));
        assert_eq!(c.tax, None);
        assert_eq!(c.total, 100_000);
    }

    #[test]
    fn ml_below_threshold_shipping_unknown() {
        let p = product(Site::MercadoLibre, uyu(999), None);
        let c = landed_cost(&p, None, &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Unknown);
        assert_eq!(c.total, 99_900);
    }

    #[test]
    fn ml_scraped_shipping_is_known() {
        let p = product(Site::MercadoLibre, uyu(999), Some(uyu(200)));
        let c = landed_cost(&p, None, &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Known(20_000));
        assert_eq!(c.total, 119_900);
    }

    #[test]
    fn ml_usd_price_converts_and_compares_in_uyu() {
        let p = product(Site::MercadoLibre, usd(49), None);
        let c = landed_cost(&p, None, &fx(), &CostSettings::default());
        assert_eq!(c.product, 196_000);
        assert!(c.converted);
        assert_eq!(c.shipping, ShippingCost::Known(0));
        assert_eq!(c.total, 196_000);

        let cheap = product(Site::MercadoLibre, usd(20), None);
        let c = landed_cost(&cheap, None, &fx(), &CostSettings::default());
        assert_eq!(c.product, 80_000);
        assert_eq!(c.shipping, ShippingCost::Unknown);
    }

    #[test]
    fn override_wins_over_scraped_shipping() {
        let p = product(Site::MercadoLibre, uyu(999), Some(uyu(200)));
        let c = landed_cost(&p, Some(uyu(50)), &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Known(5_000));
    }

    #[test]
    fn usd_override_converts() {
        let p = product(Site::Temu, uyu(1200), None);
        let c = landed_cost(&p, Some(usd(5)), &fx(), &CostSettings::default());
        assert_eq!(c.shipping, ShippingCost::Known(20_000));
        assert!(c.converted);
    }

    #[test]
    fn tax_rounds_half_up() {
        let tiny = |cents| {
            product(
                Site::Temu,
                Money {
                    cents,
                    currency: Currency::Uyu,
                },
                None,
            )
        };
        let bp = |iva_bp| CostSettings {
            iva_bp,
            ..CostSettings::default()
        };
        assert_eq!(
            landed_cost(&tiny(25), None, &fx(), &bp(2000))
                .tax
                .unwrap()
                .amount,
            5
        );
        assert_eq!(
            landed_cost(&tiny(1), None, &fx(), &bp(5000))
                .tax
                .unwrap()
                .amount,
            1
        );
        assert_eq!(
            landed_cost(&tiny(1), None, &fx(), &bp(4999))
                .tax
                .unwrap()
                .amount,
            0
        );
    }
}
