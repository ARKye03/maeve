//! Fixture for the screenshot harness (`MAEVE_DEMO=1`); never hits the network.

use crate::domain::{
    CostSettings, Currency, Field, FxRate, Money, Product, Site, build_stat_table, landed_cost,
};
use crate::i18n::Lang;
use crate::llm::prompts::Preset;
use crate::llm::schema::{AdvocateOutput, AlignedCell, AlignedRow, Attack, Better, JudgeOutput};
use crate::pipeline::{CompareInput, ComparisonResult, ItemInput};
use crate::store::HistoryEntry;

const ML_URL: &str = "https://articulo.mercadolibre.com.uy/MLU-1234567890-mini-pc-ryzen-5";
const TEMU_URL: &str = "https://www.temu.com/uy/mini-pc-intel-n100-g-601099511223344.html";

fn specs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
}

fn spec_cell(key: &str, numeric: f64, unit: &str) -> AlignedCell {
    AlignedCell {
        spec_key: Some(key.into()),
        numeric: Some(numeric),
        unit: Some(unit.into()),
    }
}

fn products() -> Vec<Product> {
    vec![
        Product {
            site: Site::MercadoLibre,
            url: ML_URL.into(),
            title: "Mini PC AMD Ryzen 5 5600U 16 GB RAM 512 GB SSD".into(),
            image_url: None,
            price: Money {
                cents: 42_900,
                currency: Currency::Usd,
            },
            price_is_range: false,
            shipping: Some(Money {
                cents: 0,
                currency: Currency::Uyu,
            }),
            rating: Some(4.7),
            review_count: Some(312),
            specs: specs(&[
                ("RAM", "16 GB"),
                ("Almacenamiento", "512 GB SSD"),
                ("Procesador", "AMD Ryzen 5 5600U"),
                ("Garantía", "12 meses"),
            ]),
            reviews: vec![
                "Muy silenciosa y rápida, la uso para trabajar todo el día.".into(),
                "Llegó en 2 días, bien empaquetada.".into(),
            ],
            missing: vec![],
        },
        Product {
            site: Site::Temu,
            url: TEMU_URL.into(),
            title: "Mini PC Intel N100 12 GB RAM 256 GB SSD".into(),
            image_url: None,
            price: Money {
                cents: 1_149_000,
                currency: Currency::Uyu,
            },
            price_is_range: false,
            shipping: None,
            rating: Some(4.3),
            review_count: Some(1_840),
            specs: specs(&[
                ("RAM", "12 GB"),
                ("Almacenamiento", "256 GB SSD"),
                ("Procesador", "Intel N100"),
            ]),
            reviews: vec![
                "Anda bien para oficina, se calienta un poco.".into(),
                "Tardó tres semanas pero llegó completa.".into(),
            ],
            missing: vec![Field::Shipping],
        },
    ]
}

pub fn demo_comparison() -> (CompareInput, ComparisonResult) {
    let products = products();
    let cost = CostSettings::default();
    let fx = FxRate {
        uyu_per_usd: 40.2,
        date: jiff::civil::date(2026, 11, 12),
        manual: false,
    };
    let costs: Vec<_> = products
        .iter()
        .map(|p| landed_cost(p, None, &fx, &cost))
        .collect();

    let rows = vec![
        AlignedRow {
            label: "RAM".into(),
            better: Better::Higher,
            cells: vec![spec_cell("RAM", 16.0, "GB"), spec_cell("RAM", 12.0, "GB")],
        },
        AlignedRow {
            label: "Almacenamiento".into(),
            better: Better::Higher,
            cells: vec![
                spec_cell("Almacenamiento", 512.0, "GB"),
                spec_cell("Almacenamiento", 256.0, "GB"),
            ],
        },
    ];
    let stats = build_stat_table(&products, &costs, &rows);

    let advocates = vec![
        AdvocateOutput {
            product: 0,
            argument: "El Ryzen 5 5600U rinde bastante más que el N100 en tareas multihilo, \
                       trae el doble de almacenamiento y llega rápido con garantía local."
                .into(),
            strengths: vec![
                "16 GB de RAM y 512 GB de SSD".into(),
                "Valoración 4,7 con 312 opiniones".into(),
                "Garantía de 12 meses".into(),
            ],
            attacks: vec![Attack {
                target: 1,
                point: "Sin envío confirmado y con IVA sumado al total.".into(),
            }],
        },
        AdvocateOutput {
            product: 1,
            argument: "Es la opción más barata y alcanza de sobra para oficina y navegación; \
                       el N100 consume muy poco."
                .into(),
            strengths: vec![
                "Precio inicial mucho menor".into(),
                "Consumo y ruido mínimos".into(),
            ],
            attacks: vec![Attack {
                target: 0,
                point: "Paga de más por potencia que no va a usar.".into(),
            }],
        },
    ];

    let judge = JudgeOutput {
        winner: 0,
        scores: vec![8.4, 6.7],
        verdict: "La Mini PC Ryzen 5 gana: más RAM, más almacenamiento, mejor valoración y \
                  garantía local. La de Temu solo conviene si el presupuesto es lo único que \
                  importa y no hay apuro."
            .into(),
        rows,
    };

    let input = CompareInput {
        items: vec![
            ItemInput {
                url: ML_URL.into(),
                product: Some(products[0].clone()),
                shipping_override: None,
            },
            ItemInput {
                url: TEMU_URL.into(),
                product: Some(products[1].clone()),
                shipping_override: None,
            },
        ],
        preset: Preset::BestOverall,
        purpose: Some("Trabajo de oficina y programación".into()),
        lang: Lang::Es,
        cost,
        fx,
        advocate_model: "gemini-flash-latest".into(),
        judge_model: "gemini-flash-latest".into(),
    };

    (
        input,
        ComparisonResult {
            products,
            costs,
            advocates,
            judge,
            stats,
            excluded: Vec::new(),
        },
    )
}

pub fn demo_history() -> Vec<HistoryEntry> {
    let (_, result) = demo_comparison();
    let titles: Vec<String> = result.products.iter().map(|p| p.title.clone()).collect();
    let winner = titles[result.judge.winner].clone();
    let entry = |id: i64, created_at: &str, preset: &str| HistoryEntry {
        id,
        created_at: created_at.into(),
        titles: titles.clone(),
        winner: winner.clone(),
        preset: preset.into(),
    };
    vec![
        entry(3, "2026-11-12T14:05:00Z", "best_overall"),
        entry(2, "2026-11-10T19:30:00Z", "quality_price"),
        entry(1, "2026-11-03T09:12:00Z", "Para la oficina"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_consistent() {
        let (input, result) = demo_comparison();
        assert_eq!(input.items.len(), 2);
        assert_eq!(result.products.len(), 2);
        assert_eq!(result.costs.len(), 2);
        assert!(result.costs[0].converted);
        assert!(result.stats.iter().all(|r| r.cells.len() == 2));
        assert!(result.judge.winner < result.products.len());
    }
}
