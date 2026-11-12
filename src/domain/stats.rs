use serde::{Deserialize, Serialize};

use super::cost::ShippingCost;
use super::{CostBreakdown, Currency, Field, Money, Product, format_money};
use crate::llm::schema::{AlignedCell, AlignedRow, Better};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowKind {
    Total,
    Rating,
    Reviews,
    Spec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatCell {
    pub raw: Option<String>,
    pub numeric: Option<f64>,
    pub ai_derived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatRow {
    pub kind: RowKind,
    pub label: String,
    pub cells: Vec<StatCell>,
    pub best: Vec<usize>,
}

impl StatCell {
    fn missing(ai_derived: bool) -> Self {
        Self {
            raw: None,
            numeric: None,
            ai_derived,
        }
    }

    fn exact(raw: String, numeric: f64) -> Self {
        Self {
            raw: Some(raw),
            numeric: Some(numeric),
            ai_derived: false,
        }
    }
}

fn best_indices(cells: &[StatCell], better: Better) -> Vec<usize> {
    if better == Better::None {
        return Vec::new();
    }
    let Some(values) = cells
        .iter()
        .map(|c| c.numeric.filter(|n| n.is_finite()))
        .collect::<Option<Vec<f64>>>()
    else {
        return Vec::new();
    };
    let target = match better {
        Better::Higher => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Better::Lower => values.iter().copied().fold(f64::INFINITY, f64::min),
        Better::None => return Vec::new(),
    };
    values
        .iter()
        .enumerate()
        .filter(|(_, v)| **v == target)
        .map(|(i, _)| i)
        .collect()
}

fn row(kind: RowKind, label: &str, cells: Vec<StatCell>, better: Better) -> StatRow {
    let best = best_indices(&cells, better);
    StatRow {
        kind,
        label: label.to_string(),
        cells,
        best,
    }
}

pub fn build_stat_table(
    products: &[Product],
    costs: &[CostBreakdown],
    rows: &[AlignedRow],
) -> Vec<StatRow> {
    let total = products
        .iter()
        .enumerate()
        .map(|(i, p)| match costs.get(i) {
            Some(c) if !p.missing.contains(&Field::Price) => StatCell {
                raw: Some(format_money(Money {
                    cents: c.total,
                    currency: Currency::Uyu,
                })),
                numeric: (c.shipping != ShippingCost::Unknown).then_some(c.total as f64),
                ai_derived: false,
            },
            _ => StatCell::missing(false),
        })
        .collect();

    let rating = products
        .iter()
        .map(|p| match p.rating {
            Some(r) => StatCell::exact(format!("{r:.1}"), f64::from(r)),
            None => StatCell::missing(false),
        })
        .collect();

    let reviews = products
        .iter()
        .map(|p| match p.review_count {
            Some(n) => StatCell::exact(n.to_string(), f64::from(n)),
            None => StatCell::missing(false),
        })
        .collect();

    let mut table = vec![
        row(RowKind::Total, "Total", total, Better::Lower),
        row(RowKind::Rating, "Rating", rating, Better::Higher),
        row(RowKind::Reviews, "Reviews", reviews, Better::Higher),
    ];

    table.extend(rows.iter().map(|r| {
        let cells = products
            .iter()
            .enumerate()
            .map(|(i, p)| spec_cell(p, r.cells.get(i)))
            .collect();
        let mut stat_row = row(RowKind::Spec, &r.label, cells, r.better);
        if !units_agree(&r.cells) {
            stat_row.best.clear();
        }
        stat_row
    }));
    table
}

/// Cells with differing units (e.g. "2 kg" vs "500 g") are not comparable, so no best is marked.
pub(crate) fn units_agree(cells: &[AlignedCell]) -> bool {
    let norm = |c: &AlignedCell| c.unit.as_deref().map(|u| u.trim().to_lowercase());
    cells.windows(2).all(|w| norm(&w[0]) == norm(&w[1]))
}

/// `numeric` is trusted only when `spec_key` resolves to a real spec of the product.
fn spec_cell(product: &Product, cell: Option<&AlignedCell>) -> StatCell {
    let Some(cell) = cell else {
        return StatCell::missing(true);
    };
    let raw = cell.spec_key.as_deref().and_then(|key| {
        product
            .specs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    });
    match raw {
        Some(raw) => StatCell {
            raw: Some(raw),
            numeric: cell.numeric,
            ai_derived: true,
        },
        None => StatCell::missing(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Site, cost::ShippingCost};

    fn product(rating: Option<f32>, review_count: Option<u32>, specs: &[(&str, &str)]) -> Product {
        Product {
            site: Site::MercadoLibre,
            url: "https://example.com/p".into(),
            title: "P".into(),
            image_url: None,
            price: Money {
                cents: 100_000,
                currency: Currency::Uyu,
            },
            price_is_range: false,
            shipping: None,
            rating,
            review_count,
            specs: specs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            reviews: vec![],
            missing: vec![],
        }
    }

    fn cost(total: i64) -> CostBreakdown {
        CostBreakdown {
            product: total,
            shipping: ShippingCost::Known(0),
            tax: None,
            total,
            converted: false,
        }
    }

    fn cell(key: Option<&str>, numeric: Option<f64>) -> AlignedCell {
        AlignedCell {
            spec_key: key.map(Into::into),
            numeric,
            unit: None,
        }
    }

    fn find(table: &[StatRow], kind: RowKind) -> &StatRow {
        table.iter().find(|r| r.kind == kind).unwrap()
    }

    #[test]
    fn lowest_total_wins() {
        let products = [product(None, None, &[]), product(None, None, &[])];
        let table = build_stat_table(&products, &[cost(164_700), cost(120_000)], &[]);
        let total = find(&table, RowKind::Total);
        assert_eq!(total.best, vec![1]);
        assert_eq!(total.cells[0].raw.as_deref(), Some("$1.647"));
        assert!(!total.cells[0].ai_derived);
        assert_eq!(table[0].kind, RowKind::Total);
        assert_eq!(table[1].kind, RowKind::Rating);
        assert_eq!(table[2].kind, RowKind::Reviews);
    }

    #[test]
    fn rating_tie_gives_both() {
        let products = [
            product(Some(4.7), Some(10), &[]),
            product(Some(4.7), Some(20), &[]),
        ];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &[]);
        assert_eq!(find(&table, RowKind::Rating).best, vec![0, 1]);
        assert_eq!(find(&table, RowKind::Reviews).best, vec![1]);
    }

    #[test]
    fn missing_rating_empties_best() {
        let products = [product(Some(4.7), None, &[]), product(None, None, &[])];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &[]);
        let rating = find(&table, RowKind::Rating);
        assert!(rating.best.is_empty());
        assert_eq!(rating.cells[1].raw, None);
        assert_eq!(rating.cells[1].numeric, None);
    }

    #[test]
    fn invented_spec_key_has_no_raw() {
        let products = [
            product(None, None, &[("RAM", "8 GB")]),
            product(None, None, &[("RAM", "16 GB")]),
        ];
        let rows = [AlignedRow {
            label: "RAM".into(),
            better: Better::Higher,
            cells: vec![
                cell(Some("RAM"), Some(8.0)),
                cell(Some("Memory"), Some(16.0)),
            ],
        }];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &rows);
        let spec = find(&table, RowKind::Spec);
        assert_eq!(spec.cells[0].raw.as_deref(), Some("8 GB"));
        assert!(spec.cells[0].ai_derived);
        assert_eq!(spec.cells[1].raw, None);
        assert!(spec.best.is_empty());
    }

    #[test]
    fn better_none_has_no_best() {
        let products = [
            product(None, None, &[("Color", "Negro")]),
            product(None, None, &[("Color", "Rojo")]),
        ];
        let rows = [AlignedRow {
            label: "Color".into(),
            better: Better::None,
            cells: vec![
                cell(Some("Color"), Some(1.0)),
                cell(Some("Color"), Some(2.0)),
            ],
        }];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &rows);
        assert!(find(&table, RowKind::Spec).best.is_empty());
    }

    #[test]
    fn unknown_shipping_total_is_not_comparable() {
        let products = [product(None, None, &[]), product(None, None, &[])];
        let mut unknown = cost(90_000);
        unknown.shipping = ShippingCost::Unknown;
        let table = build_stat_table(&products, &[unknown, cost(95_000)], &[]);
        let total = find(&table, RowKind::Total);
        assert!(total.best.is_empty());
        assert_eq!(total.cells[0].raw.as_deref(), Some("$900"));
    }

    #[test]
    fn mixed_units_have_no_best() {
        let products = [
            product(None, None, &[("Peso", "2 kg")]),
            product(None, None, &[("Peso", "500 g")]),
        ];
        let unit = |u: &str| AlignedCell {
            spec_key: Some("Peso".into()),
            numeric: Some(if u == "kg" { 2.0 } else { 500.0 }),
            unit: Some(u.into()),
        };
        let rows = [AlignedRow {
            label: "Peso".into(),
            better: Better::Lower,
            cells: vec![unit("kg"), unit("g")],
        }];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &rows);
        assert!(find(&table, RowKind::Spec).best.is_empty());
    }

    #[test]
    fn lower_is_better_for_spec_rows() {
        let products = [
            product(None, None, &[("Peso", "200 g")]),
            product(None, None, &[("Peso", "150 g")]),
        ];
        let rows = [AlignedRow {
            label: "Peso".into(),
            better: Better::Lower,
            cells: vec![
                cell(Some("Peso"), Some(200.0)),
                cell(Some("Peso"), Some(150.0)),
            ],
        }];
        let table = build_stat_table(&products, &[cost(1), cost(2)], &rows);
        assert_eq!(find(&table, RowKind::Spec).best, vec![1]);
    }
}
