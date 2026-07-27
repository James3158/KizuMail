use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

pub const DEFAULT_AI_TIMEOUT: Duration = Duration::from_secs(45);
pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_GEMINI_GENERATION_MODEL: &str = "gemini-3.1-flash-lite";
pub const LEGACY_GEMINI_GENERATION_MODEL: &str = "gemini-2.5-flash-lite";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Gemini,
    Ollama,
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gemini => formatter.write_str("Gemini"),
            Self::Ollama => formatter.write_str("Ollama"),
        }
    }
}

#[derive(Clone)]
pub struct GeminiConfig {
    api_key: String,
    pub base_url: String,
    pub generation_model: String,
    pub embedding_model: String,
    pub timeout: Duration,
}

impl GeminiConfig {
    pub fn new(
        api_key: impl Into<String>,
        generation_model: impl Into<String>,
        embedding_model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_GEMINI_BASE_URL.to_string(),
            generation_model: generation_model.into(),
            embedding_model: embedding_model.into(),
            timeout: DEFAULT_AI_TIMEOUT,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }
}

impl fmt::Debug for GeminiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiConfig")
            .field("api_key", &"[redacted]")
            .field("base_url", &self.base_url)
            .field("generation_model", &self.generation_model)
            .field("embedding_model", &self.embedding_model)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct OllamaConfig {
    pub base_url: String,
    pub generation_model: String,
    pub embedding_model: String,
    pub timeout: Duration,
}

impl OllamaConfig {
    pub fn new(generation_model: impl Into<String>, embedding_model: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_OLLAMA_BASE_URL.to_string(),
            generation_model: generation_model.into(),
            embedding_model: embedding_model.into(),
            timeout: DEFAULT_AI_TIMEOUT,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Clone, Debug)]
pub enum ProviderConfig {
    Gemini(GeminiConfig),
    Ollama(OllamaConfig),
}

impl ProviderConfig {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Gemini(_) => ProviderKind::Gemini,
            Self::Ollama(_) => ProviderKind::Ollama,
        }
    }
}

/// Text originating outside KizuMail's trusted control plane, such as a mail body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UntrustedText {
    pub label: String,
    pub content: String,
}

impl UntrustedText {
    pub fn new(label: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationOptions {
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

/// `instruction` and `prompt` must be trusted application text. Mail-derived text
/// belongs exclusively in `untrusted_data`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationRequest {
    pub instruction: String,
    pub prompt: String,
    #[serde(default)]
    pub untrusted_data: Vec<UntrustedText>,
    #[serde(default)]
    pub options: GenerationOptions,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationResponse {
    pub provider: ProviderKind,
    pub model: String,
    pub text: String,
    pub finish_reason: Option<String>,
    pub usage: TokenUsage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingPurpose {
    RetrievalDocument,
    RetrievalQuery,
    SemanticSimilarity,
    Classification,
    Clustering,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRequest {
    pub input: UntrustedText,
    pub purpose: EmbeddingPurpose,
    pub output_dimension: Option<u32>,
    #[serde(default = "default_true")]
    pub truncate: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingResponse {
    pub provider: ProviderKind,
    pub model: String,
    pub vector: Vec<f32>,
    pub input_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationFlag(Arc<AtomicBool>);

impl CancellationFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), AiError> {
        if self.is_cancelled() {
            Err(AiError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AiError {
    Cancelled,
    InvalidConfiguration(String),
    InvalidRequest(String),
    Timeout {
        provider: ProviderKind,
    },
    Transport {
        provider: ProviderKind,
        detail: String,
    },
    Http {
        provider: ProviderKind,
        status: u16,
        code: Option<String>,
        message: String,
    },
    Rejected {
        provider: ProviderKind,
        reason: String,
    },
    InvalidResponse {
        provider: ProviderKind,
        detail: String,
    },
}

impl fmt::Display for AiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("AI request was cancelled"),
            Self::InvalidConfiguration(detail) => {
                write!(formatter, "invalid AI provider configuration: {detail}")
            }
            Self::InvalidRequest(detail) => write!(formatter, "invalid AI request: {detail}"),
            Self::Timeout { provider } => write!(formatter, "{provider} request timed out"),
            Self::Transport { provider, detail } => {
                write!(formatter, "{provider} transport failed: {detail}")
            }
            Self::Http {
                provider,
                status,
                code,
                message,
            } => {
                write!(formatter, "{provider} returned HTTP {status}")?;
                if let Some(code) = code {
                    write!(formatter, " ({code})")?;
                }
                write!(formatter, ": {message}")
            }
            Self::Rejected { provider, reason } => {
                write!(formatter, "{provider} rejected the request: {reason}")
            }
            Self::InvalidResponse { provider, detail } => {
                write!(formatter, "invalid {provider} response: {detail}")
            }
        }
    }
}

impl std::error::Error for AiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_debug_output_redacts_api_key() {
        let config = GeminiConfig::new("top-secret", "generate", "embed");
        let debug = format!("{config:?}");

        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("top-secret"));
    }

    #[test]
    fn cancellation_flag_is_shared_between_clones() {
        let flag = CancellationFlag::new();
        let clone = flag.clone();

        clone.cancel();

        assert!(flag.is_cancelled());
        assert_eq!(flag.check(), Err(AiError::Cancelled));
    }
}
