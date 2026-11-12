use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::StatusCode;
use serde_json::{Value, json};

use super::{Llm, LlmError, LlmRequest, Part, RateLimiter};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const MAX_RETRIES: u32 = 3;
const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(120);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(15);

pub struct GeminiClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    limiter: Arc<RateLimiter>,
}

impl fmt::Debug for GeminiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeminiClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl GeminiClient {
    pub fn new(api_key: String, rpm: u32) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            http,
            api_key,
            base_url: DEFAULT_BASE_URL.to_owned(),
            limiter: Arc::new(RateLimiter::new(rpm)),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url.trim_end_matches('/').to_owned();
        self
    }

    /// Cheapest authenticated call: lists one model. Skips the rate limiter.
    pub async fn verify_key(&self) -> Result<(), LlmError> {
        let resp = self
            .http
            .get(format!("{}/v1beta/models?pageSize=1", self.base_url))
            .header("x-goog-api-key", &self.api_key)
            .timeout(VERIFY_TIMEOUT)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.without_url().to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(LlmError::Auth),
            StatusCode::BAD_REQUEST if text.contains("API_KEY_INVALID") => Err(LlmError::Auth),
            s => Err(LlmError::Http(format!("{s}: {}", error_message(&text)))),
        }
    }

    fn build_body(req: &LlmRequest) -> Value {
        let parts: Vec<Value> = req
            .parts
            .iter()
            .map(|p| match p {
                Part::Text(text) => json!({ "text": text }),
                Part::Image { mime, data } => json!({
                    "inline_data": { "mime_type": mime, "data": STANDARD.encode(data) }
                }),
            })
            .collect();
        json!({
            "systemInstruction": { "parts": [{ "text": req.system }] },
            "contents": [{ "role": "user", "parts": parts }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": req.schema,
            },
        })
    }
}

impl Llm for GeminiClient {
    async fn generate_json(&self, req: LlmRequest) -> Result<Value, LlmError> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.base_url, req.model
        );
        let body = Self::build_body(&req);
        let report = |d: Duration| {
            if let Some(tx) = &req.wait_tx {
                let _ = tx.send(d);
            }
        };

        let mut retries = 0;
        loop {
            let waited = self.limiter.acquire().await;
            if !waited.is_zero() {
                report(waited);
            }

            let resp = self
                .http
                .post(&url)
                .header("x-goog-api-key", &self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Http(e.without_url().to_string()))?;
            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| LlmError::Http(e.without_url().to_string()))?;

            match status {
                s if s.is_success() => return extract_json(&text),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(LlmError::Auth),
                StatusCode::TOO_MANY_REQUESTS => {
                    if retries >= MAX_RETRIES {
                        return Err(LlmError::RateLimited);
                    }
                    retries += 1;
                    let delay = retry_delay(&text)
                        .unwrap_or(DEFAULT_RETRY_DELAY)
                        .min(MAX_RETRY_DELAY);
                    report(delay);
                    tokio::time::sleep(delay).await;
                }
                StatusCode::BAD_REQUEST if text.contains("API_KEY_INVALID") => {
                    return Err(LlmError::Auth);
                }
                s => return Err(LlmError::Http(format!("{s}: {}", error_message(&text)))),
            }
        }
    }
}

fn extract_json(body: &str) -> Result<Value, LlmError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| LlmError::InvalidOutput(format!("response is not JSON: {e}")))?;
    let text = v
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::InvalidOutput("response has no candidate text".into()))?;
    serde_json::from_str(text)
        .map_err(|e| LlmError::InvalidOutput(format!("candidate text is not JSON: {e}")))
}

fn error_message(body: &str) -> String {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.to_owned());
    msg.chars().take(200).collect()
}

fn retry_delay(body: &str) -> Option<Duration> {
    let v: Value = serde_json::from_str(body).ok()?;
    let raw = v
        .pointer("/error/details")?
        .as_array()?
        .iter()
        .find(|d| {
            d.get("@type")
                .and_then(Value::as_str)
                .is_some_and(|t| t.ends_with("RetryInfo"))
        })?
        .get("retryDelay")?
        .as_str()?;
    let secs: f64 = raw.strip_suffix('s')?.parse().ok()?;
    Duration::try_from_secs_f64(secs).ok()
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const KEY: &str = "sekrit-key-123";
    const PATH: &str = "/v1beta/models/gemini-test:generateContent";

    fn request(parts: Vec<Part>) -> LlmRequest {
        LlmRequest {
            model: "gemini-test".into(),
            system: "sys".into(),
            parts,
            schema: json!({ "type": "OBJECT" }),
            wait_tx: None,
        }
    }

    fn client(server: &MockServer) -> GeminiClient {
        GeminiClient::new(KEY.into(), 1000).with_base_url(server.uri())
    }

    fn ok_body(text: &str) -> Value {
        json!({ "candidates": [{ "content": { "parts": [{ "text": text }] } }] })
    }

    fn rate_limited(delay: &str) -> ResponseTemplate {
        ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "code": 429,
                "message": "quota",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.RetryInfo",
                    "retryDelay": delay,
                }],
            }
        }))
    }

    #[tokio::test]
    async fn returns_parsed_json_with_key_and_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(PATH))
            .and(header("x-goog-api-key", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(r#"{"a":1}"#)))
            .mount(&server)
            .await;

        let out = client(&server)
            .generate_json(request(vec![Part::Text("hi".into())]))
            .await
            .unwrap();
        assert_eq!(out, json!({ "a": 1 }));

        let sent: Value = server.received_requests().await.unwrap()[0]
            .body_json()
            .unwrap();
        assert_eq!(
            sent["generationConfig"]["responseSchema"],
            json!({ "type": "OBJECT" })
        );
        assert_eq!(
            sent["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(sent["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(sent["contents"][0]["parts"][0]["text"], "hi");
    }

    #[tokio::test]
    async fn auth_failures_make_exactly_one_request() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let err = client(&server)
                .generate_json(request(vec![]))
                .await
                .unwrap_err();
            assert_eq!(err, LlmError::Auth);
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn invalid_key_400_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "message": "API key not valid", "details": [{ "reason": "API_KEY_INVALID" }] }
            })))
            .mount(&server)
            .await;
        let err = client(&server)
            .generate_json(request(vec![]))
            .await
            .unwrap_err();
        assert_eq!(err, LlmError::Auth);
    }

    #[tokio::test]
    async fn retries_429_honouring_retry_delay_and_reports_wait() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(rate_limited("0.2s"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("{}")))
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut req = request(vec![]);
        req.wait_tx = Some(tx);
        let out = client(&server).generate_json(req).await.unwrap();
        assert_eq!(out, json!({}));
        assert_eq!(rx.recv().await, Some(Duration::from_millis(200)));
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn gives_up_after_three_retries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(rate_limited("0.01s"))
            .mount(&server)
            .await;
        let err = client(&server)
            .generate_json(request(vec![]))
            .await
            .unwrap_err();
        assert_eq!(err, LlmError::RateLimited);
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn retries_go_through_the_limiter() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(rate_limited("0s"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("{}")))
            .mount(&server)
            .await;

        let client = GeminiClient::new(KEY.into(), 1000).with_base_url(server.uri());
        client.generate_json(request(vec![])).await.unwrap();
        let stamps = client.limiter.in_window();
        assert_eq!(stamps, 2);
    }

    #[tokio::test]
    async fn non_json_text_is_invalid_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("not json")))
            .mount(&server)
            .await;
        let err = client(&server)
            .generate_json(request(vec![]))
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[tokio::test]
    async fn missing_candidates_is_invalid_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "candidates": [] })))
            .mount(&server)
            .await;
        let err = client(&server)
            .generate_json(request(vec![]))
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::InvalidOutput(_)));
    }

    #[tokio::test]
    async fn image_part_is_base64_inline_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body("{}")))
            .mount(&server)
            .await;
        let parts = vec![Part::Image {
            mime: "image/png".into(),
            data: vec![1, 2, 3, 250],
        }];
        client(&server).generate_json(request(parts)).await.unwrap();

        let sent: Value = server.received_requests().await.unwrap()[0]
            .body_json()
            .unwrap();
        let inline = &sent["contents"][0]["parts"][0]["inline_data"];
        assert_eq!(inline["mime_type"], "image/png");
        assert_eq!(inline["data"], STANDARD.encode([1u8, 2, 3, 250]));
    }

    #[tokio::test]
    async fn server_error_is_http_without_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let err = client(&server)
            .generate_json(request(vec![]))
            .await
            .unwrap_err();
        let LlmError::Http(msg) = &err else {
            panic!("expected Http, got {err:?}");
        };
        assert!(msg.contains("500"));
        assert!(!format!("{err} {err:?}").contains(KEY));
    }

    #[tokio::test]
    async fn connection_error_does_not_leak_key() {
        let dead = GeminiClient::new(KEY.into(), 1000).with_base_url("http://127.0.0.1:1".into());
        let err = dead.generate_json(request(vec![])).await.unwrap_err();
        assert!(matches!(err, LlmError::Http(_)));
        assert!(!format!("{err} {err:?}").contains(KEY));
    }

    #[test]
    fn debug_redacts_key() {
        let c = GeminiClient::new(KEY.into(), 10);
        assert!(!format!("{c:?}").contains(KEY));
    }

    #[test]
    fn parses_retry_delay() {
        let body = r#"{"error":{"details":[{"@type":"x/google.rpc.Help"},{"@type":"x/google.rpc.RetryInfo","retryDelay":"2s"}]}}"#;
        assert_eq!(retry_delay(body), Some(Duration::from_secs(2)));
        assert_eq!(retry_delay("{}"), None);
    }

    #[tokio::test]
    async fn verify_key_accepts_a_listed_model() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("x-goog-api-key", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
            .mount(&server)
            .await;
        assert!(client(&server).verify_key().await.is_ok());
    }

    #[tokio::test]
    async fn verify_key_maps_invalid_key_to_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": { "code": 400, "message": "API key not valid", "details": [{ "reason": "API_KEY_INVALID" }] }
            })))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server).verify_key().await,
            Err(LlmError::Auth)
        ));
    }

    #[tokio::test]
    async fn verify_key_reports_other_failures_as_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server).verify_key().await,
            Err(LlmError::Http(_))
        ));
    }
}
