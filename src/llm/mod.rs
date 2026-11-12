mod gemini;
mod limiter;
pub mod prompts;
pub mod schema;

use std::future::Future;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::mpsc;

pub use gemini::GeminiClient;
pub use limiter::RateLimiter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Image { mime: String, data: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system: String,
    pub parts: Vec<Part>,
    pub schema: serde_json::Value,
    pub wait_tx: Option<mpsc::UnboundedSender<Duration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    #[error("the API key was rejected")]
    Auth,
    #[error("rate limited by the API")]
    RateLimited,
    #[error("invalid model output: {0}")]
    InvalidOutput(String),
    #[error("http error: {0}")]
    Http(String),
}

pub trait Llm: Send + Sync + 'static {
    fn generate_json(
        &self,
        req: LlmRequest,
    ) -> impl Future<Output = Result<serde_json::Value, LlmError>> + Send;
}
