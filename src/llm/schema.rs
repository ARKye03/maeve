use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::LlmError;
use crate::domain::{Currency, Field, Money, Product, Site};

const MAX_SCORE: f32 = 10.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attack {
    pub target: usize,
    pub point: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvocateOutput {
    pub product: usize,
    pub argument: String,
    pub strengths: Vec<String>,
    pub attacks: Vec<Attack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Better {
    Higher,
    Lower,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedCell {
    pub spec_key: Option<String>,
    pub numeric: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedRow {
    pub label: String,
    pub better: Better,
    pub cells: Vec<AlignedCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeOutput {
    pub winner: usize,
    pub scores: Vec<f32>,
    pub verdict: String,
    pub rows: Vec<AlignedRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedSpec {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub title: String,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub price: Option<Money>,
    #[serde(default)]
    pub price_is_range: bool,
    #[serde(default)]
    pub shipping: Option<Money>,
    #[serde(default)]
    pub rating: Option<f32>,
    #[serde(default)]
    pub review_count: Option<u32>,
    #[serde(default)]
    pub specs: Vec<ExtractedSpec>,
    #[serde(default)]
    pub reviews: Vec<String>,
}

pub(super) fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn invalid(e: impl std::fmt::Display) -> LlmError {
    LlmError::InvalidOutput(e.to_string())
}

fn string() -> Value {
    json!({ "type": "STRING" })
}

fn nullable(mut schema: Value) -> Value {
    schema["nullable"] = json!(true);
    schema
}

fn money_schema() -> Value {
    nullable(json!({
        "type": "OBJECT",
        "properties": {
            "cents": { "type": "INTEGER" },
            "currency": { "type": "STRING", "enum": ["UYU", "USD"] },
        },
        "required": ["cents", "currency"],
    }))
}

pub fn advocate_schema() -> Value {
    json!({
        "type": "OBJECT",
        "properties": {
            "product": { "type": "INTEGER" },
            "argument": string(),
            "strengths": { "type": "ARRAY", "items": string() },
            "attacks": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": { "target": { "type": "INTEGER" }, "point": string() },
                    "required": ["target", "point"],
                },
            },
        },
        "required": ["product", "argument", "strengths", "attacks"],
    })
}

pub fn judge_schema(n: usize) -> Value {
    json!({
        "type": "OBJECT",
        "properties": {
            "winner": { "type": "INTEGER" },
            "scores": {
                "type": "ARRAY",
                "items": { "type": "NUMBER" },
                "minItems": n,
                "maxItems": n,
            },
            "verdict": string(),
            "rows": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "label": string(),
                        "better": { "type": "STRING", "enum": ["higher", "lower", "none"] },
                        "cells": {
                            "type": "ARRAY",
                            "items": {
                                "type": "OBJECT",
                                "properties": {
                                    "spec_key": nullable(string()),
                                    "numeric": nullable(json!({ "type": "NUMBER" })),
                                    "unit": nullable(string()),
                                },
                                "required": ["spec_key", "numeric", "unit"],
                            },
                            "minItems": n,
                            "maxItems": n,
                        },
                    },
                    "required": ["label", "better", "cells"],
                },
            },
        },
        "required": ["winner", "scores", "verdict", "rows"],
    })
}

pub fn extraction_schema() -> Value {
    json!({
        "type": "OBJECT",
        "properties": {
            "title": string(),
            "image_url": nullable(string()),
            "price": money_schema(),
            "price_is_range": { "type": "BOOLEAN" },
            "shipping": money_schema(),
            "rating": nullable(json!({ "type": "NUMBER" })),
            "review_count": nullable(json!({ "type": "INTEGER" })),
            "specs": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": { "key": string(), "value": string() },
                    "required": ["key", "value"],
                },
            },
            "reviews": { "type": "ARRAY", "items": string() },
        },
        "required": ["title", "price_is_range", "specs", "reviews"],
    })
}

/// `n` is the product count; `idx` is the advocate's own product index.
pub fn parse_advocate(v: Value, n: usize, idx: usize) -> Result<AdvocateOutput, LlmError> {
    let mut out: AdvocateOutput = serde_json::from_value(v).map_err(invalid)?;
    if out.product != idx {
        return Err(invalid(format!(
            "advocate argued for product {} but was assigned {idx}",
            out.product
        )));
    }
    out.attacks.retain(|a| a.target < n && a.target != idx);
    Ok(out)
}

/// A cell survives only if its `spec_key` exists in that product's scraped
/// specs, so a model-invented key never reaches the UI. `numeric` and `unit`
/// are model-interpreted and not checked against the spec value.
pub fn parse_judge(v: Value, products: &[Product]) -> Result<JudgeOutput, LlmError> {
    let n = products.len();
    let mut out: JudgeOutput = serde_json::from_value(v).map_err(invalid)?;
    if out.winner >= n {
        return Err(invalid(format!(
            "winner {} is out of range for {n} products",
            out.winner
        )));
    }
    if out.scores.len() != n {
        return Err(invalid(format!(
            "expected {n} scores, got {}",
            out.scores.len()
        )));
    }
    if out
        .scores
        .iter()
        .any(|s| !s.is_finite() || !(0.0..=MAX_SCORE).contains(s))
    {
        return Err(invalid(format!("scores must be within 0..={MAX_SCORE}")));
    }

    out.rows.retain(|r| r.cells.len() == n);
    for row in &mut out.rows {
        for (cell, product) in row.cells.iter_mut().zip(products) {
            // The prompt shows single-line keys, so accept that form and map back to the original.
            let matched = cell.spec_key.as_deref().and_then(|k| {
                product
                    .specs
                    .iter()
                    .find(|(sk, _)| sk == k || one_line(sk) == k)
                    .map(|(sk, _)| sk.clone())
            });
            let known = matched.is_some();
            cell.spec_key = matched.or(cell.spec_key.take());
            if !known {
                *cell = AlignedCell {
                    spec_key: None,
                    numeric: None,
                    unit: None,
                };
            } else if cell.numeric.is_some_and(|x| !x.is_finite()) {
                cell.numeric = None;
            }
        }
    }
    Ok(out)
}

pub fn parse_extraction(v: Value) -> Result<Extraction, LlmError> {
    let mut e: Extraction = serde_json::from_value(v).map_err(invalid)?;
    if [e.price, e.shipping]
        .into_iter()
        .flatten()
        .any(|m| m.cents < 0)
    {
        return Err(invalid("price and shipping must not be negative"));
    }
    if e.rating
        .is_some_and(|r| !r.is_finite() || !(0.0..=5.0).contains(&r))
    {
        return Err(invalid("rating must be within 0..=5"));
    }
    e.image_url = e.image_url.filter(|u| {
        let u = u.trim().to_ascii_lowercase();
        u.starts_with("https://") || u.starts_with("http://")
    });
    Ok(e)
}

/// A missing price becomes zero and is flagged in `missing`, never guessed.
pub fn extraction_into_product(e: Extraction, site: Site, url: String) -> Product {
    let mut missing = Vec::new();
    let price = e.price.unwrap_or_else(|| {
        missing.push(Field::Price);
        Money {
            cents: 0,
            currency: Currency::Uyu,
        }
    });
    if e.shipping.is_none() {
        missing.push(Field::Shipping);
    }
    if e.rating.is_none() {
        missing.push(Field::Rating);
    }
    if e.review_count.is_none() {
        missing.push(Field::ReviewCount);
    }
    if e.specs.is_empty() {
        missing.push(Field::Specs);
    }
    if e.reviews.is_empty() {
        missing.push(Field::Reviews);
    }
    Product {
        site,
        url,
        title: e.title,
        image_url: e.image_url,
        price,
        price_is_range: e.price_is_range,
        shipping: e.shipping,
        rating: e.rating,
        review_count: e.review_count,
        specs: e.specs.into_iter().map(|s| (s.key, s.value)).collect(),
        reviews: e.reviews,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(specs: &[(&str, &str)]) -> Product {
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
            rating: None,
            review_count: None,
            specs: specs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            reviews: vec![],
            missing: vec![],
        }
    }

    fn cell(key: Option<&str>, numeric: Option<f64>) -> Value {
        json!({ "spec_key": key, "numeric": numeric, "unit": "h" })
    }

    fn judge(winner: usize, scores: Value, rows: Value) -> Value {
        json!({ "winner": winner, "scores": scores, "verdict": "v", "rows": rows })
    }

    fn two() -> Vec<Product> {
        vec![
            product(&[("Batería", "20 h")]),
            product(&[("Battery", "30 h")]),
        ]
    }

    #[test]
    fn judge_accepts_valid_output() {
        let rows = json!([{ "label": "Battery", "better": "higher",
            "cells": [cell(Some("Batería"), Some(20.0)), cell(Some("Battery"), Some(30.0))] }]);
        let out = parse_judge(judge(1, json!([6.5, 8.0]), rows), &two()).unwrap();
        assert_eq!(out.winner, 1);
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].better, Better::Higher);
        assert_eq!(out.rows[0].cells[1].numeric, Some(30.0));
    }

    #[test]
    fn judge_rejects_out_of_range_winner() {
        let err = parse_judge(judge(3, json!([5.0, 5.0]), json!([])), &two()).unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[test]
    fn judge_rejects_wrong_scores_length() {
        for scores in [json!([5.0]), json!([5.0, 5.0, 5.0])] {
            let err = parse_judge(judge(0, scores, json!([])), &two()).unwrap_err();
            assert!(matches!(err, LlmError::InvalidOutput(_)));
        }
    }

    #[test]
    fn judge_rejects_scores_outside_range() {
        let err = parse_judge(judge(0, json!([11.0, 5.0]), json!([])), &two()).unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[test]
    fn judge_rejects_malformed_shape() {
        let err = parse_judge(json!({ "winner": "first" }), &two()).unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
        let err = parse_judge(judge(0, json!([5.0, 5.0]), json!([])), &[]).unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[test]
    fn judge_nulls_spec_key_absent_from_that_products_specs() {
        let rows = json!([{ "label": "Battery", "better": "higher",
            "cells": [cell(Some("Batería"), Some(20.0)), cell(Some("Batería"), Some(99.0))] }]);
        let out = parse_judge(judge(0, json!([5.0, 5.0]), rows), &two()).unwrap();
        let cells = &out.rows[0].cells;
        assert_eq!(cells[0].spec_key.as_deref(), Some("Batería"));
        assert_eq!(cells[0].numeric, Some(20.0));
        assert_eq!(
            cells[1],
            AlignedCell {
                spec_key: None,
                numeric: None,
                unit: None
            }
        );
    }

    #[test]
    fn judge_drops_number_with_no_spec_key() {
        let rows = json!([{ "label": "X", "better": "none",
            "cells": [cell(None, Some(5.0)), cell(None, None)] }]);
        let out = parse_judge(judge(0, json!([5.0, 5.0]), rows), &two()).unwrap();
        assert_eq!(out.rows[0].cells[0].numeric, None);
        assert_eq!(out.rows[0].cells[0].unit, None);
    }

    #[test]
    fn judge_drops_rows_with_wrong_cell_count() {
        let rows = json!([
            { "label": "short", "better": "none", "cells": [cell(None, None)] },
            { "label": "ok", "better": "none", "cells": [cell(None, None), cell(None, None)] },
        ]);
        let out = parse_judge(judge(0, json!([5.0, 5.0]), rows), &two()).unwrap();
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].label, "ok");
    }

    fn advocate(product: usize, attacks: Value) -> Value {
        json!({ "product": product, "argument": "a", "strengths": ["s"], "attacks": attacks })
    }

    #[test]
    fn advocate_rejects_wrong_product_index() {
        let err = parse_advocate(advocate(1, json!([])), 3, 0).unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[test]
    fn advocate_drops_out_of_range_and_self_attacks() {
        let attacks = json!([
            { "target": 1, "point": "keep" },
            { "target": 5, "point": "out of range" },
            { "target": 0, "point": "self" },
        ]);
        let out = parse_advocate(advocate(0, attacks), 2, 0).unwrap();
        assert_eq!(
            out.attacks,
            vec![Attack {
                target: 1,
                point: "keep".into()
            }]
        );
    }

    #[test]
    fn extraction_parses_and_flags_missing() {
        let v = json!({
            "title": "Auriculares",
            "price": { "cents": 120000, "currency": "UYU" },
            "price_is_range": true,
            "specs": [{ "key": "Color", "value": "Negro" }],
            "reviews": [],
        });
        let e = parse_extraction(v).unwrap();
        let p = extraction_into_product(e, Site::Temu, "https://temu.com/x".into());
        assert_eq!(p.site, Site::Temu);
        assert_eq!(p.url, "https://temu.com/x");
        assert_eq!(p.price.cents, 120_000);
        assert!(p.price_is_range);
        assert_eq!(p.specs, vec![("Color".into(), "Negro".into())]);
        assert_eq!(
            p.missing,
            vec![
                Field::Shipping,
                Field::Rating,
                Field::ReviewCount,
                Field::Reviews
            ]
        );
    }

    #[test]
    fn extraction_without_price_is_flagged_not_guessed() {
        let e = parse_extraction(json!({ "title": "T" })).unwrap();
        let p = extraction_into_product(e, Site::MercadoLibre, "u".into());
        assert_eq!(p.price.cents, 0);
        assert!(p.missing.contains(&Field::Price));
    }

    #[test]
    fn extraction_rejects_out_of_range_values_and_bad_urls() {
        let neg = json!({ "title": "T", "price": { "cents": -5, "currency": "UYU" } });
        assert!(parse_extraction(neg).is_err());
        assert!(parse_extraction(json!({ "title": "T", "rating": 45.0 })).is_err());
        let e =
            parse_extraction(json!({ "title": "T", "image_url": "javascript:alert(1)" })).unwrap();
        assert_eq!(e.image_url, None);
        let e =
            parse_extraction(json!({ "title": "T", "image_url": "https://x.com/a.png" })).unwrap();
        assert!(e.image_url.is_some());
    }

    #[test]
    fn judge_maps_single_line_spec_key_back_to_original() {
        let products = vec![product(&[("Bat\nería", "20 h")]), product(&[("B", "1")])];
        let rows = json!([{ "label": "L", "better": "none",
            "cells": [cell(Some("Bat ería"), Some(20.0)), cell(None, None)] }]);
        let out = parse_judge(judge(0, json!([5.0, 5.0]), rows), &products).unwrap();
        assert_eq!(out.rows[0].cells[0].spec_key.as_deref(), Some("Bat\nería"));
        assert_eq!(out.rows[0].cells[0].numeric, Some(20.0));
    }

    #[test]
    fn extraction_requires_title() {
        assert!(parse_extraction(json!({ "price_is_range": false })).is_err());
    }

    #[test]
    fn schemas_pin_array_lengths() {
        let s = judge_schema(3);
        assert_eq!(s["properties"]["scores"]["minItems"], 3);
        assert_eq!(s["properties"]["scores"]["maxItems"], 3);
        assert_eq!(advocate_schema()["type"], "OBJECT");
        assert_eq!(extraction_schema()["required"][0], "title");
    }
}
