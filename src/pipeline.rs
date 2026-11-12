use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::domain::{
    CostBreakdown, CostSettings, FxRate, Money, Product, StatRow, build_stat_table, landed_cost,
};
use crate::fx::validate_fx;
use crate::i18n::Lang;
use crate::llm::prompts::{EvalContext, Preset, advocate_request, judge_request};
use crate::llm::schema::{AdvocateOutput, JudgeOutput, parse_advocate, parse_judge};
use crate::llm::{Llm, LlmError, LlmRequest};
use crate::scrape::{ScrapeError, ScrapeErrorKind, Scraper, classify_url};

const EVENT_BUFFER: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInput {
    pub url: String,
    pub product: Option<Product>,
    pub shipping_override: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareInput {
    pub items: Vec<ItemInput>,
    pub preset: Preset,
    pub purpose: Option<String>,
    pub lang: Lang,
    pub cost: CostSettings,
    pub fx: FxRate,
    pub advocate_model: String,
    pub judge_model: String,
}

/// `excluded` indices refer to `CompareInput::items`. Everything else
/// (`products`, `costs`, `advocates`, `judge` winner/scores/rows, `stats`) is
/// indexed by position among the products that survived, i.e. the items not
/// listed in `excluded`, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub products: Vec<Product>,
    pub costs: Vec<CostBreakdown>,
    pub advocates: Vec<AdvocateOutput>,
    pub judge: JudgeOutput,
    pub stats: Vec<StatRow>,
    pub excluded: Vec<(usize, ScrapeError)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PipelineError {
    #[error("at least 2 products are needed to compare")]
    NotEnoughProducts,
    #[error("invalid exchange rate: {0}")]
    InvalidFx(String),
    #[error(transparent)]
    Llm(#[from] LlmError),
}

/// Item indices refer to `CompareInput::items`.
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Scraped {
        item: usize,
        result: Result<Product, ScrapeError>,
    },
    AdvocateStarted {
        item: usize,
    },
    AdvocateDone {
        item: usize,
    },
    Waiting(Duration),
    JudgeStarted,
    Finished(Result<ComparisonResult, PipelineError>),
}

/// Starts the work on the first poll, so building the stream needs no Tokio
/// runtime context (the UI creates it outside one).
struct PipelineStream {
    start: Option<Box<dyn FnOnce() + Send>>,
    rx: ReceiverStream<PipelineEvent>,
}

impl Stream for PipelineStream {
    type Item = PipelineEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(start) = self.start.take() {
            start();
        }
        Pin::new(&mut self.rx).poll_next(cx)
    }
}

pub fn run_comparison<S: Scraper, L: Llm>(
    scraper: Arc<S>,
    llm: Arc<L>,
    input: CompareInput,
) -> impl Stream<Item = PipelineEvent> + Send + 'static {
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let start = Box::new(move || {
        tokio::spawn(async move {
            // Dropping the stream cancels the run, aborting every in-flight task.
            tokio::select! {
                () = tx.closed() => {}
                () = drive(scraper, llm, input, tx.clone()) => {}
            }
        });
    });
    PipelineStream {
        start: Some(start),
        rx: ReceiverStream::new(rx),
    }
}

async fn drive<S: Scraper, L: Llm>(
    scraper: Arc<S>,
    llm: Arc<L>,
    input: CompareInput,
    tx: mpsc::Sender<PipelineEvent>,
) {
    let outcome = compare(scraper, llm, input, &tx).await;
    let _ = tx.send(PipelineEvent::Finished(outcome)).await;
}

async fn compare<S: Scraper, L: Llm>(
    scraper: Arc<S>,
    llm: Arc<L>,
    input: CompareInput,
    tx: &mpsc::Sender<PipelineEvent>,
) -> Result<ComparisonResult, PipelineError> {
    validate_fx(&input.fx).map_err(PipelineError::InvalidFx)?;
    let CompareInput {
        items,
        preset,
        purpose,
        lang,
        cost,
        fx,
        advocate_model,
        judge_model,
    } = input;

    let (slots, excluded) = scrape_missing(&scraper, &items, tx).await;
    let (item_of, products): (Vec<usize>, Vec<Product>) = slots
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .unzip();
    let n = products.len();
    if n < 2 {
        return Err(PipelineError::NotEnoughProducts);
    }

    let costs: Vec<CostBreakdown> = products
        .iter()
        .zip(&item_of)
        .map(|(p, &i)| landed_cost(p, items[i].shipping_override, &fx, &cost))
        .collect();
    let ctx = EvalContext {
        products: &products,
        costs: &costs,
        preset: &preset,
        purpose: purpose.as_deref(),
        lang,
    };

    let (wait_tx, mut wait_rx) = mpsc::unbounded_channel::<Duration>();

    let mut advocate_set = JoinSet::new();
    for (idx, &item) in item_of.iter().enumerate() {
        let mut req = advocate_request(&ctx, idx, &advocate_model);
        req.wait_tx = Some(wait_tx.clone());
        let llm = Arc::clone(&llm);
        let tx = tx.clone();
        advocate_set.spawn(async move {
            let _ = tx.send(PipelineEvent::AdvocateStarted { item }).await;
            let out = call_with_retry(&*llm, req, |v| parse_advocate(v, n, idx)).await?;
            let _ = tx.send(PipelineEvent::AdvocateDone { item }).await;
            Ok::<_, LlmError>((idx, out))
        });
    }
    let collect = async {
        let mut outs: Vec<Option<AdvocateOutput>> = vec![None; n];
        while let Some(joined) = advocate_set.join_next().await {
            match joined {
                Ok(Ok((idx, out))) => outs[idx] = Some(out),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(LlmError::Http(format!("advocate task failed: {e}"))),
            }
        }
        Ok(outs)
    };
    let advocates: Vec<AdvocateOutput> = with_waits(collect, &mut wait_rx, tx)
        .await?
        .into_iter()
        .flatten()
        .collect();

    let _ = tx.send(PipelineEvent::JudgeStarted).await;
    let mut req = judge_request(&ctx, &advocates, &judge_model);
    req.wait_tx = Some(wait_tx);
    let judge = with_waits(
        call_with_retry(&*llm, req, |v| parse_judge(v, &products)),
        &mut wait_rx,
        tx,
    )
    .await?;

    let stats = build_stat_table(&products, &costs, &judge.rows);
    Ok(ComparisonResult {
        products,
        costs,
        advocates,
        judge,
        stats,
        excluded,
    })
}

/// Scrapes the items that have no product yet, in parallel. Pre-filled items
/// are neither scraped nor reported.
async fn scrape_missing<S: Scraper>(
    scraper: &Arc<S>,
    items: &[ItemInput],
    tx: &mpsc::Sender<PipelineEvent>,
) -> (Vec<Option<Product>>, Vec<(usize, ScrapeError)>) {
    let mut slots: Vec<Option<Product>> = items.iter().map(|i| i.product.clone()).collect();
    let mut excluded = Vec::new();
    let mut set = JoinSet::new();
    let mut item_by_task = HashMap::new();

    for (idx, item) in items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.product.is_none())
    {
        let scraper = Arc::clone(scraper);
        let raw = item.url.clone();
        let handle = set.spawn(async move {
            let (_, url) = classify_url(&raw)?;
            scraper.scrape(url).await
        });
        item_by_task.insert(handle.id(), idx);
    }

    while let Some(joined) = set.join_next_with_id().await {
        let (id, result) = match joined {
            Ok((id, result)) => (id, result),
            Err(e) => (
                e.id(),
                Err(ScrapeError::new(
                    ScrapeErrorKind::Network,
                    format!("scrape task failed: {e}"),
                )),
            ),
        };
        let item = item_by_task[&id];
        let _ = tx
            .send(PipelineEvent::Scraped {
                item,
                result: result.clone(),
            })
            .await;
        match result {
            Ok(product) => slots[item] = Some(product),
            Err(e) => excluded.push((item, e)),
        }
    }
    excluded.sort_by_key(|(i, _)| *i);
    (slots, excluded)
}

/// Invalid model output is retried once; every other error is returned as is.
async fn call_with_retry<L: Llm, T>(
    llm: &L,
    req: LlmRequest,
    parse: impl Fn(serde_json::Value) -> Result<T, LlmError>,
) -> Result<T, LlmError> {
    let mut retried = false;
    loop {
        match llm.generate_json(req.clone()).await.and_then(&parse) {
            Err(LlmError::InvalidOutput(_)) if !retried => retried = true,
            other => return other,
        }
    }
}

/// Runs `fut` while relaying the limiter's wait notices as events.
async fn with_waits<T>(
    fut: impl Future<Output = T>,
    waits: &mut mpsc::UnboundedReceiver<Duration>,
    tx: &mpsc::Sender<PipelineEvent>,
) -> T {
    let mut fut = std::pin::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => {
                while let Ok(wait) = waits.try_recv() {
                    let _ = tx.send(PipelineEvent::Waiting(wait)).await;
                }
                return out;
            }
            Some(wait) = waits.recv() => {
                let _ = tx.send(PipelineEvent::Waiting(wait)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};
    use tokio_stream::StreamExt as _;

    use super::*;
    use crate::domain::{Currency, Site};

    const ML_A: &str = "https://articulo.mercadolibre.com.uy/MLU-1-a-_JM";
    const ML_B: &str = "https://articulo.mercadolibre.com.uy/MLU-2-b-_JM";
    const ML_C: &str = "https://articulo.mercadolibre.com.uy/MLU-3-c-_JM";

    fn product(url: &str, uyu: i64) -> Product {
        Product {
            site: Site::MercadoLibre,
            url: url.into(),
            title: format!("Item {url}"),
            image_url: None,
            price: Money {
                cents: uyu * 100,
                currency: Currency::Uyu,
            },
            price_is_range: false,
            shipping: None,
            rating: Some(4.5),
            review_count: Some(10),
            specs: vec![],
            reviews: vec![],
            missing: vec![],
        }
    }

    fn blocked() -> ScrapeError {
        ScrapeError::new(ScrapeErrorKind::Blocked, "blocked")
    }

    #[derive(Default)]
    struct FakeScraper {
        results: HashMap<String, Result<Product, ScrapeError>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeScraper {
        fn new(entries: Vec<(&str, Result<Product, ScrapeError>)>) -> Arc<Self> {
            Arc::new(Self {
                results: entries
                    .into_iter()
                    .map(|(u, r)| (u.to_owned(), r))
                    .collect(),
                calls: Mutex::default(),
            })
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Scraper for FakeScraper {
        async fn scrape(&self, url: url::Url) -> Result<Product, ScrapeError> {
            self.calls.lock().unwrap().push(url.to_string());
            self.results
                .get(url.as_str())
                .cloned()
                .unwrap_or_else(|| Err(ScrapeError::new(ScrapeErrorKind::Layout, "unknown url")))
        }
    }

    enum Reply {
        Json(Value),
        Fail(LlmError),
        Hang,
    }

    type Handler = Box<dyn Fn(&LlmRequest) -> Reply + Send + Sync>;

    struct FakeLlm {
        handler: Handler,
        requests: Mutex<Vec<LlmRequest>>,
    }

    impl FakeLlm {
        fn new(handler: impl Fn(&LlmRequest) -> Reply + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                handler: Box::new(handler),
                requests: Mutex::default(),
            })
        }

        fn requests(&self) -> Vec<LlmRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Llm for FakeLlm {
        async fn generate_json(&self, req: LlmRequest) -> Result<Value, LlmError> {
            let reply = (self.handler)(&req);
            self.requests.lock().unwrap().push(req);
            match reply {
                Reply::Json(v) => Ok(v),
                Reply::Fail(e) => Err(e),
                Reply::Hang => std::future::pending().await,
            }
        }
    }

    fn is_judge(req: &LlmRequest) -> bool {
        req.system.contains("impartial judge")
    }

    fn advocate_idx(req: &LlmRequest) -> usize {
        (0..10)
            .find(|i| req.system.contains(&format!("advocate for product {i} ")))
            .unwrap()
    }

    fn advocate_json(idx: usize) -> Value {
        json!({
            "product": idx,
            "argument": format!("arg {idx}"),
            "strengths": ["solid"],
            "attacks": [],
        })
    }

    fn judge_json(n: usize) -> Value {
        json!({
            "winner": 0,
            "scores": vec![7.5; n],
            "verdict": "first wins",
            "rows": [],
        })
    }

    fn happy_llm(n: usize) -> Arc<FakeLlm> {
        FakeLlm::new(move |req| {
            if is_judge(req) {
                Reply::Json(judge_json(n))
            } else {
                Reply::Json(advocate_json(advocate_idx(req)))
            }
        })
    }

    fn item(url: &str) -> ItemInput {
        ItemInput {
            url: url.into(),
            product: None,
            shipping_override: None,
        }
    }

    fn input(items: Vec<ItemInput>) -> CompareInput {
        CompareInput {
            items,
            preset: Preset::BestOverall,
            purpose: None,
            lang: Lang::En,
            cost: CostSettings::default(),
            fx: FxRate {
                uyu_per_usd: 40.0,
                date: jiff::civil::date(2026, 11, 12),
                manual: false,
            },
            advocate_model: "adv-model".into(),
            judge_model: "judge-model".into(),
        }
    }

    fn three_scraped() -> Arc<FakeScraper> {
        FakeScraper::new(vec![
            (ML_A, Ok(product(ML_A, 1500))),
            (ML_B, Ok(product(ML_B, 1200))),
            (ML_C, Ok(product(ML_C, 2000))),
        ])
    }

    async fn run(
        scraper: Arc<FakeScraper>,
        llm: Arc<FakeLlm>,
        input: CompareInput,
    ) -> Vec<PipelineEvent> {
        let stream = run_comparison(scraper, llm, input);
        tokio::time::timeout(Duration::from_secs(5), stream.collect::<Vec<_>>())
            .await
            .expect("pipeline must finish")
    }

    fn finished(events: &[PipelineEvent]) -> &Result<ComparisonResult, PipelineError> {
        match events.last() {
            Some(PipelineEvent::Finished(r)) => r,
            other => panic!("last event is not Finished: {other:?}"),
        }
    }

    fn position(events: &[PipelineEvent], pred: impl Fn(&PipelineEvent) -> bool) -> Vec<usize> {
        events
            .iter()
            .enumerate()
            .filter(|(_, e)| pred(e))
            .map(|(i, _)| i)
            .collect()
    }

    #[tokio::test]
    async fn happy_path_event_order() {
        let llm = happy_llm(3);
        let events = run(
            three_scraped(),
            llm.clone(),
            input(vec![item(ML_A), item(ML_B), item(ML_C)]),
        )
        .await;

        let scraped = position(&events, |e| matches!(e, PipelineEvent::Scraped { .. }));
        let started = position(&events, |e| {
            matches!(e, PipelineEvent::AdvocateStarted { .. })
        });
        let done = position(&events, |e| matches!(e, PipelineEvent::AdvocateDone { .. }));
        let judge = position(&events, |e| matches!(e, PipelineEvent::JudgeStarted));
        assert_eq!((scraped.len(), started.len(), done.len()), (3, 3, 3));
        assert_eq!(judge.len(), 1);
        assert!(scraped.iter().max() < started.iter().min());
        assert!(started.iter().min() < done.iter().min());
        assert!(done.iter().max() < judge.first());
        assert_eq!(judge[0] + 2, events.len());
        for i in 0..3 {
            let s = position(
                &events,
                |e| matches!(e, PipelineEvent::AdvocateStarted { item } if *item == i),
            );
            let d = position(
                &events,
                |e| matches!(e, PipelineEvent::AdvocateDone { item } if *item == i),
            );
            assert!(s[0] < d[0]);
        }

        let result = finished(&events).as_ref().unwrap();
        assert_eq!(result.products.len(), 3);
        assert_eq!(result.costs.len(), 3);
        assert_eq!(result.advocates.len(), 3);
        assert!(result.excluded.is_empty());
        assert_eq!(result.judge.winner, 0);
        assert!(!result.stats.is_empty());

        let judge_req = llm.requests().into_iter().find(is_judge).unwrap();
        let Some(crate::llm::Part::Text(text)) = judge_req.parts.first() else {
            panic!("judge request has no text part");
        };
        for i in 0..3 {
            assert!(text.contains(&format!("arg {i}")), "missing arg {i}");
        }
        assert_eq!(judge_req.model, "judge-model");
        assert_eq!(llm.requests().len(), 4);
    }

    #[tokio::test]
    async fn prefilled_products_are_not_scraped() {
        let scraper = FakeScraper::new(vec![(ML_B, Ok(product(ML_B, 1200)))]);
        let prefilled = ItemInput {
            product: Some(product(ML_A, 1500)),
            ..item(ML_A)
        };
        let events = run(
            scraper.clone(),
            happy_llm(2),
            input(vec![prefilled, item(ML_B)]),
        )
        .await;

        assert_eq!(scraper.calls(), vec![ML_B.to_owned()]);
        let scraped = position(&events, |e| {
            matches!(e, PipelineEvent::Scraped { item: 0, .. })
        });
        assert!(scraped.is_empty());
        assert_eq!(finished(&events).as_ref().unwrap().products.len(), 2);
    }

    #[tokio::test]
    async fn two_of_three_failures_stop_before_any_llm_call() {
        let scraper = FakeScraper::new(vec![
            (ML_A, Ok(product(ML_A, 1500))),
            (ML_B, Err(blocked())),
            (ML_C, Err(blocked())),
        ]);
        let llm = happy_llm(3);
        let events = run(
            scraper,
            llm.clone(),
            input(vec![item(ML_A), item(ML_B), item(ML_C)]),
        )
        .await;

        assert_eq!(
            finished(&events).as_ref().unwrap_err(),
            &PipelineError::NotEnoughProducts
        );
        assert!(llm.requests().is_empty());
        let failed = position(&events, |e| {
            matches!(e, PipelineEvent::Scraped { result: Err(_), .. })
        });
        assert_eq!(failed.len(), 2);
    }

    #[tokio::test]
    async fn unsupported_url_counts_as_failure() {
        let scraper = FakeScraper::new(vec![(ML_A, Ok(product(ML_A, 1500)))]);
        let llm = happy_llm(2);
        let events = run(
            scraper.clone(),
            llm.clone(),
            input(vec![item(ML_A), item("https://example.com/x")]),
        )
        .await;
        assert_eq!(
            finished(&events).as_ref().unwrap_err(),
            &PipelineError::NotEnoughProducts
        );
        assert_eq!(scraper.calls().len(), 1);
        assert!(llm.requests().is_empty());
    }

    #[tokio::test]
    async fn single_failure_is_excluded() {
        let scraper = FakeScraper::new(vec![
            (ML_A, Ok(product(ML_A, 1500))),
            (ML_B, Err(blocked())),
            (ML_C, Ok(product(ML_C, 2000))),
        ]);
        let events = run(
            scraper,
            happy_llm(2),
            input(vec![item(ML_A), item(ML_B), item(ML_C)]),
        )
        .await;

        let result = finished(&events).as_ref().unwrap();
        assert_eq!(result.products.len(), 2);
        assert_eq!(result.excluded.len(), 1);
        assert_eq!(result.excluded[0].0, 1);
        assert_eq!(result.products[1].url, ML_C);

        let started = position(&events, |e| {
            matches!(e, PipelineEvent::AdvocateStarted { item: 1 })
        });
        assert!(started.is_empty(), "failed item must not get an advocate");
        let started_c = position(&events, |e| {
            matches!(e, PipelineEvent::AdvocateStarted { item: 2 })
        });
        assert_eq!(started_c.len(), 1);
    }

    fn judge_after_invalid(invalid_count: usize) -> Arc<FakeLlm> {
        let judge_calls = AtomicUsize::new(0);
        FakeLlm::new(move |req| {
            if !is_judge(req) {
                return Reply::Json(advocate_json(advocate_idx(req)));
            }
            if judge_calls.fetch_add(1, Ordering::SeqCst) < invalid_count {
                Reply::Json(json!({"winner": 9, "scores": [1.0, 2.0], "verdict": "x", "rows": []}))
            } else {
                Reply::Json(judge_json(2))
            }
        })
    }

    fn two_items() -> (Arc<FakeScraper>, CompareInput) {
        let scraper = FakeScraper::new(vec![
            (ML_A, Ok(product(ML_A, 1500))),
            (ML_B, Ok(product(ML_B, 1200))),
        ]);
        (scraper, input(vec![item(ML_A), item(ML_B)]))
    }

    #[tokio::test]
    async fn invalid_then_valid_judge_succeeds() {
        let llm = judge_after_invalid(1);
        let (scraper, inp) = two_items();
        let events = run(scraper, llm.clone(), inp).await;
        assert!(finished(&events).is_ok());
        assert_eq!(llm.requests().iter().filter(|r| is_judge(r)).count(), 2);
    }

    #[tokio::test]
    async fn invalid_twice_judge_fails() {
        let llm = judge_after_invalid(2);
        let (scraper, inp) = two_items();
        let events = run(scraper, llm.clone(), inp).await;
        assert!(matches!(
            finished(&events),
            Err(PipelineError::Llm(LlmError::InvalidOutput(_)))
        ));
        assert_eq!(llm.requests().iter().filter(|r| is_judge(r)).count(), 2);
    }

    #[tokio::test]
    async fn invalid_advocate_is_retried_once() {
        let calls = AtomicUsize::new(0);
        let llm = FakeLlm::new(move |req| {
            if is_judge(req) {
                return Reply::Json(judge_json(2));
            }
            let idx = advocate_idx(req);
            if idx == 0 && calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Reply::Json(advocate_json(1))
            } else {
                Reply::Json(advocate_json(idx))
            }
        });
        let (scraper, inp) = two_items();
        let events = run(scraper, llm.clone(), inp).await;
        assert!(finished(&events).is_ok());
        assert_eq!(llm.requests().len(), 4);
    }

    #[tokio::test]
    async fn auth_error_aborts_promptly() {
        let llm = FakeLlm::new(|req| {
            if advocate_idx(req) == 0 {
                Reply::Fail(LlmError::Auth)
            } else {
                Reply::Hang
            }
        });
        let events = run(
            three_scraped(),
            llm.clone(),
            input(vec![item(ML_A), item(ML_B), item(ML_C)]),
        )
        .await;

        assert_eq!(
            finished(&events).as_ref().unwrap_err(),
            &PipelineError::Llm(LlmError::Auth)
        );
        assert!(!llm.requests().iter().any(is_judge));
        let judge = position(&events, |e| matches!(e, PipelineEvent::JudgeStarted));
        assert!(judge.is_empty());
    }

    #[tokio::test]
    async fn invalid_fx_stops_before_any_work() {
        for bad in [0.0, -40.0, f64::NAN, f64::INFINITY, 1e12] {
            let scraper = three_scraped();
            let llm = happy_llm(3);
            let mut inp = input(vec![item(ML_A), item(ML_B), item(ML_C)]);
            inp.fx.uyu_per_usd = bad;
            let events = run(scraper.clone(), llm.clone(), inp).await;
            assert!(
                matches!(finished(&events), Err(PipelineError::InvalidFx(_))),
                "{bad}"
            );
            assert_eq!(events.len(), 1);
            assert!(scraper.calls().is_empty());
            assert!(llm.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn limiter_waits_are_relayed() {
        let llm = FakeLlm::new(|req| {
            if is_judge(req) {
                if let Some(tx) = &req.wait_tx {
                    tx.send(Duration::from_secs(12)).unwrap();
                }
                Reply::Json(judge_json(2))
            } else {
                Reply::Json(advocate_json(advocate_idx(req)))
            }
        });
        let (scraper, inp) = two_items();
        let events = run(scraper, llm, inp).await;
        let waits = position(
            &events,
            |e| matches!(e, PipelineEvent::Waiting(d) if *d == Duration::from_secs(12)),
        );
        assert_eq!(waits.len(), 1);
        assert!(finished(&events).is_ok());
    }

    #[tokio::test]
    async fn usd_shipping_override_feeds_costs() {
        let (scraper, mut inp) = two_items();
        inp.items[0].shipping_override = Some(Money {
            cents: 500,
            currency: Currency::Usd,
        });
        let events = run(scraper, happy_llm(2), inp).await;
        let result = finished(&events).as_ref().unwrap();
        assert_eq!(
            result.costs[0].shipping,
            crate::domain::ShippingCost::Known(20_000)
        );
        assert_eq!(result.costs[0].total, 150_000 + 20_000);
    }

    #[test]
    fn stream_is_built_without_a_runtime() {
        let (scraper, inp) = two_items();
        let _stream = run_comparison(scraper, happy_llm(2), inp);
    }
}
