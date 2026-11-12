use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use turso::{Builder, Connection, Database};

use crate::domain::FxRate;
use crate::llm::prompts::Preset;
use crate::pipeline::{CompareInput, ComparisonResult};
use crate::settings::Settings;

const SETTINGS_KEY: &str = "settings";
const FX_KEY: &str = "fx";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS comparisons (
    id INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    titles TEXT NOT NULL,
    winner TEXT NOT NULL,
    preset TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS presets (name TEXT PRIMARY KEY, criteria TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] turso::Error),
    #[error("stored data is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot create database directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("database path is not valid UTF-8: {0}")]
    InvalidPath(PathBuf),
    #[error("comparison {0} not found")]
    NotFound(i64),
    #[error("only custom presets can be stored")]
    NotCustomPreset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonRecord {
    pub input: CompareInput,
    pub result: ComparisonResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub created_at: String,
    pub titles: Vec<String>,
    pub winner: String,
    pub preset: String,
}

pub struct Store {
    _db: Database,
    conn: Connection,
}

pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("maeve")
        .join("maeve.db")
}

fn preset_label(p: &Preset) -> String {
    match p {
        Preset::BestOverall => "best_overall".into(),
        Preset::QualityPrice => "quality_price".into(),
        Preset::CheapestDecent => "cheapest_decent".into(),
        Preset::Custom { name, .. } => name.clone(),
    }
}

fn text(row: &turso::Row, idx: usize) -> Result<String, StoreError> {
    Ok(row.get::<String>(idx)?)
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| StoreError::InvalidPath(path.to_owned()))?;
        let db = Builder::new_local(path_str).build().await?;
        let conn = db.connect()?;
        conn.execute_batch(SCHEMA).await?;
        Ok(Self { _db: db, conn })
    }

    pub async fn save_comparison(&self, r: &ComparisonRecord) -> Result<i64, StoreError> {
        let titles: Vec<&str> = r.result.products.iter().map(|p| p.title.as_str()).collect();
        let winner = r
            .result
            .products
            .get(r.result.judge.winner)
            .map(|p| p.title.as_str())
            .unwrap_or_default();
        let mut rows = self
            .conn
            .query(
                "INSERT INTO comparisons (created_at, titles, winner, preset, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
                (
                    jiff::Timestamp::now().to_string(),
                    serde_json::to_string(&titles)?,
                    winner.to_owned(),
                    preset_label(&r.input.preset),
                    serde_json::to_string(r)?,
                ),
            )
            .await?;
        let row = rows.next().await?.ok_or_else(|| {
            turso::Error::Error("INSERT ... RETURNING produced no row".to_owned())
        })?;
        Ok(row.get::<i64>(0)?)
    }

    pub async fn list_history(&self, limit: u32) -> Result<Vec<HistoryEntry>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, created_at, titles, winner, preset FROM comparisons \
                 ORDER BY id DESC LIMIT ?1",
                (i64::from(limit),),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(HistoryEntry {
                id: row.get::<i64>(0)?,
                created_at: text(&row, 1)?,
                titles: serde_json::from_str(&text(&row, 2)?)?,
                winner: text(&row, 3)?,
                preset: text(&row, 4)?,
            });
        }
        Ok(out)
    }

    pub async fn load_comparison(&self, id: i64) -> Result<ComparisonRecord, StoreError> {
        let mut rows = self
            .conn
            .query("SELECT payload FROM comparisons WHERE id = ?1", (id,))
            .await?;
        let row = rows.next().await?.ok_or(StoreError::NotFound(id))?;
        Ok(serde_json::from_str(&text(&row, 0)?)?)
    }

    pub async fn delete_comparison(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM comparisons WHERE id = ?1", (id,))
            .await?;
        Ok(())
    }

    pub async fn load_settings(&self) -> Result<Settings, StoreError> {
        Ok(self.kv_get(SETTINGS_KEY).await?.unwrap_or_default())
    }

    pub async fn save_settings(&self, s: &Settings) -> Result<(), StoreError> {
        self.kv_set(SETTINGS_KEY, s).await
    }

    pub async fn list_presets(&self) -> Result<Vec<Preset>, StoreError> {
        let mut rows = self
            .conn
            .query("SELECT name, criteria FROM presets ORDER BY name", ())
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Preset::Custom {
                name: text(&row, 0)?,
                criteria: text(&row, 1)?,
            });
        }
        Ok(out)
    }

    pub async fn save_preset(&self, p: &Preset) -> Result<(), StoreError> {
        let Preset::Custom { name, criteria } = p else {
            return Err(StoreError::NotCustomPreset);
        };
        self.conn
            .execute(
                "INSERT OR REPLACE INTO presets (name, criteria) VALUES (?1, ?2)",
                (name.clone(), criteria.clone()),
            )
            .await?;
        Ok(())
    }

    pub async fn cached_fx(&self) -> Result<Option<FxRate>, StoreError> {
        self.kv_get(FX_KEY).await
    }

    pub async fn save_fx(&self, r: &FxRate) -> Result<(), StoreError> {
        self.kv_set(FX_KEY, r).await
    }

    async fn kv_get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StoreError> {
        let mut rows = self
            .conn
            .query("SELECT value FROM kv WHERE key = ?1", (key.to_owned(),))
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some(serde_json::from_str(&text(&row, 0)?)?)),
            None => Ok(None),
        }
    }

    async fn kv_set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)",
                (key.to_owned(), serde_json::to_string(value)?),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use tempfile::TempDir;

    use super::*;
    use crate::domain::{
        CostBreakdown, CostSettings, Currency, Money, Product, ShippingCost, Site,
    };
    use crate::i18n::Lang;
    use crate::llm::schema::{AdvocateOutput, JudgeOutput};
    use crate::pipeline::ItemInput;
    use crate::scrape::{ScrapeError, ScrapeErrorKind};
    use crate::settings::ThemePref;

    fn product(title: &str, cents: i64) -> Product {
        Product {
            site: Site::MercadoLibre,
            url: format!("https://articulo.mercadolibre.com.uy/{title}"),
            title: title.into(),
            image_url: None,
            price: Money {
                cents,
                currency: Currency::Uyu,
            },
            price_is_range: false,
            shipping: None,
            rating: Some(4.5),
            review_count: Some(10),
            specs: vec![("RAM".into(), "16 GB".into())],
            reviews: vec!["Bueno".into()],
            missing: vec![],
        }
    }

    fn fx() -> FxRate {
        FxRate {
            uyu_per_usd: 40.12,
            date: date(2026, 11, 12),
            manual: false,
        }
    }

    fn record(preset: Preset) -> ComparisonRecord {
        let products = vec![product("A", 100_000), product("B", 90_000)];
        let costs = products
            .iter()
            .map(|p| CostBreakdown {
                product: p.price.cents,
                shipping: ShippingCost::Unknown,
                tax: None,
                total: p.price.cents,
                converted: false,
            })
            .collect();
        let input = CompareInput {
            items: vec![
                ItemInput {
                    url: products[0].url.clone(),
                    product: Some(products[0].clone()),
                    shipping_override: None,
                },
                ItemInput {
                    url: "https://www.temu.com/x-g-1.html".into(),
                    product: None,
                    shipping_override: Some(Money {
                        cents: 5_000,
                        currency: Currency::Usd,
                    }),
                },
            ],
            preset,
            purpose: Some("trabajo".into()),
            lang: Lang::Es,
            cost: CostSettings::default(),
            fx: fx(),
            advocate_model: "m".into(),
            judge_model: "m".into(),
        };
        let result = ComparisonResult {
            products,
            costs,
            advocates: vec![AdvocateOutput {
                product: 0,
                argument: "arg".into(),
                strengths: vec!["s".into()],
                attacks: vec![],
            }],
            judge: JudgeOutput {
                winner: 1,
                scores: vec![6.0, 8.0],
                verdict: "B gana".into(),
                rows: vec![],
            },
            stats: vec![],
            excluded: vec![(
                1,
                ScrapeError::new(ScrapeErrorKind::Blocked, "blocked by site"),
            )],
        };
        ComparisonRecord { input, result }
    }

    async fn open_temp() -> (TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("nested").join("maeve.db"))
            .await
            .unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn saved_comparison_is_listed_first_and_round_trips() {
        let (_dir, store) = open_temp().await;
        let first = record(Preset::BestOverall);
        let mut second = record(Preset::QualityPrice);
        second.result.judge.winner = 0;

        let id1 = store.save_comparison(&first).await.unwrap();
        let id2 = store.save_comparison(&second).await.unwrap();

        let list = store.list_history(10).await.unwrap();
        assert_eq!(list.iter().map(|e| e.id).collect::<Vec<_>>(), [id2, id1]);
        assert_eq!(list[0].titles, ["A", "B"]);
        assert_eq!(list[0].winner, "A");
        assert_eq!(list[0].preset, "quality_price");
        assert_eq!(list[1].winner, "B");
        assert!(!list[0].created_at.is_empty());
        assert_eq!(store.list_history(1).await.unwrap().len(), 1);

        assert_eq!(store.load_comparison(id1).await.unwrap(), first);
        assert_eq!(store.load_comparison(id2).await.unwrap(), second);
    }

    #[tokio::test]
    async fn resaving_a_reopened_record_adds_an_entry() {
        let (_dir, store) = open_temp().await;
        let id1 = store
            .save_comparison(&record(Preset::BestOverall))
            .await
            .unwrap();
        let reopened = store.load_comparison(id1).await.unwrap();
        let id2 = store.save_comparison(&reopened).await.unwrap();

        assert_ne!(id1, id2);
        let ids: Vec<_> = store
            .list_history(10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [id2, id1]);
        assert_eq!(store.load_comparison(id1).await.unwrap(), reopened);
    }

    #[tokio::test]
    async fn delete_and_missing_comparison() {
        let (_dir, store) = open_temp().await;
        let id = store
            .save_comparison(&record(Preset::BestOverall))
            .await
            .unwrap();
        store.delete_comparison(id).await.unwrap();
        assert!(store.list_history(10).await.unwrap().is_empty());
        assert!(matches!(
            store.load_comparison(id).await,
            Err(StoreError::NotFound(missing)) if missing == id
        ));
    }

    #[tokio::test]
    async fn data_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maeve.db");
        let rec = record(Preset::BestOverall);
        let id = {
            let store = Store::open(&path).await.unwrap();
            store.save_comparison(&rec).await.unwrap()
        };
        let store = Store::open(&path).await.unwrap();
        assert_eq!(store.load_comparison(id).await.unwrap(), rec);
    }

    #[tokio::test]
    async fn settings_default_when_empty_then_round_trip() {
        let (_dir, store) = open_temp().await;
        assert_eq!(store.load_settings().await.unwrap(), Settings::default());

        let custom = Settings {
            lang: Lang::En,
            theme: ThemePref::Dark,
            rpm: 4,
            fx_manual: Some(41.5),
            ..Settings::default()
        };
        store.save_settings(&custom).await.unwrap();
        assert_eq!(store.load_settings().await.unwrap(), custom);

        store.save_settings(&Settings::default()).await.unwrap();
        assert_eq!(store.load_settings().await.unwrap(), Settings::default());
    }

    #[tokio::test]
    async fn custom_preset_upserts_by_name() {
        let (_dir, store) = open_temp().await;
        assert!(store.list_presets().await.unwrap().is_empty());
        let custom = |name: &str, criteria: &str| Preset::Custom {
            name: name.into(),
            criteria: criteria.into(),
        };

        store.save_preset(&custom("gaming", "fps")).await.unwrap();
        store
            .save_preset(&custom("oficina", "quiet"))
            .await
            .unwrap();
        store
            .save_preset(&custom("gaming", "low latency"))
            .await
            .unwrap();

        assert_eq!(
            store.list_presets().await.unwrap(),
            [custom("gaming", "low latency"), custom("oficina", "quiet")]
        );
        assert!(matches!(
            store.save_preset(&Preset::BestOverall).await,
            Err(StoreError::NotCustomPreset)
        ));
    }

    #[tokio::test]
    async fn fx_cache_round_trip() {
        let (_dir, store) = open_temp().await;
        assert_eq!(store.cached_fx().await.unwrap(), None);
        store.save_fx(&fx()).await.unwrap();
        assert_eq!(store.cached_fx().await.unwrap(), Some(fx()));
        let newer = FxRate {
            uyu_per_usd: 41.0,
            date: date(2026, 11, 13),
            manual: false,
        };
        store.save_fx(&newer).await.unwrap();
        assert_eq!(store.cached_fx().await.unwrap(), Some(newer));
    }

    #[test]
    fn default_path_is_under_maeve_dir() {
        let p = default_db_path();
        assert!(p.ends_with("maeve/maeve.db"));
    }
}
