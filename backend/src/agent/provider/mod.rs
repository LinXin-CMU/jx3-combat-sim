//! Model-provider boundary for the combat-analysis Agent.
//!
//! Provider adapters translate a normalized, versioned protocol into upstream
//! requests. They never receive simulator internals, filesystem access, or a
//! browser-supplied credential.

mod config;
mod fake;
pub(crate) mod openai;
pub mod protocol;

use async_trait::async_trait;
use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::SharedState;

pub use config::{
    ProviderAdapterKind, ProviderCatalog, ProviderListResponse, SafeProviderProfile,
    PROVIDER_LIST_SCHEMA_V1,
};
pub use fake::FakeProvider;
pub use protocol::{
    FinishReason, ModelMessage, ModelRequest, ModelResponse, ProviderToolCall,
    StructuredOutputDefinition, TokenUsage, ToolDefinition, PROVIDER_PROTOCOL_V1,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    #[serde(skip_serializing)]
    pub usage: TokenUsage,
    /// Local replay-only diagnostics. This field is never serialized into
    /// public API errors or shareable debug exports.
    #[serde(skip_serializing)]
    pub private_detail: Option<String>,
}

impl ProviderError {
    pub fn configuration(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            upstream_status: None,
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: "invalid_model_request",
            message: "model request failed local protocol validation",
            retryable: false,
            upstream_status: None,
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn network(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self {
                code: "provider_timeout",
                message: "provider request timed out",
                retryable: true,
                upstream_status: None,
                usage: TokenUsage::default(),
                private_detail: None,
            }
        } else {
            Self {
                code: "provider_network_error",
                message: "provider request failed",
                retryable: true,
                upstream_status: None,
                usage: TokenUsage::default(),
                private_detail: None,
            }
        }
    }

    pub fn upstream_status(status: u16) -> Self {
        let code = match status {
            400 => "provider_http_400",
            401 => "provider_http_401",
            402 => "provider_balance_insufficient",
            403 => "provider_http_403",
            404 => "provider_http_404",
            408 => "provider_http_408",
            409 => "provider_http_409",
            422 => "provider_http_422",
            429 => "provider_http_429",
            500..=599 => "provider_http_5xx",
            _ => "provider_http_error",
        };
        Self {
            code,
            message: "provider returned an unsuccessful status",
            retryable: status == 408 || status == 429 || status >= 500,
            upstream_status: Some(status),
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn classified_upstream_response(status: u16, body: &[u8]) -> Self {
        let normalized = String::from_utf8_lossy(body).to_ascii_lowercase();
        if status == 400 {
            let classified = [
                (
                    "reasoning_content",
                    "provider_reasoning_context_required",
                    "provider requires transient reasoning context for tool continuation",
                ),
                (
                    "tool_call_id",
                    "provider_tool_transcript_invalid",
                    "provider rejected the normalized tool transcript",
                ),
                (
                    "tool call",
                    "provider_tool_transcript_invalid",
                    "provider rejected the normalized tool transcript",
                ),
                (
                    "response_format",
                    "provider_response_format_unsupported",
                    "provider rejected the structured response format",
                ),
                (
                    "context length",
                    "provider_context_limit",
                    "provider context limit was exceeded",
                ),
                (
                    "maximum context",
                    "provider_context_limit",
                    "provider context limit was exceeded",
                ),
            ];
            if let Some((_, code, message)) = classified
                .into_iter()
                .find(|(needle, _, _)| normalized.contains(needle))
            {
                return Self {
                    code,
                    message,
                    retryable: false,
                    upstream_status: Some(status),
                    usage: TokenUsage::default(),
                    private_detail: None,
                };
            }
        }
        Self::upstream_status(status)
    }

    pub fn response_too_large() -> Self {
        Self {
            code: "provider_response_too_large",
            message: "provider response exceeded the configured limit",
            retryable: false,
            upstream_status: None,
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn invalid_response() -> Self {
        Self {
            code: "provider_response_invalid",
            message: "provider returned an invalid response",
            retryable: false,
            upstream_status: None,
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn invalid_response_protocol(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            upstream_status: None,
            usage: TokenUsage::default(),
            private_detail: None,
        }
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_private_detail(mut self, detail: String) -> Self {
        self.private_detail = Some(detail);
        self
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProviderError {}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn profile_id(&self) -> &str;
    fn model(&self) -> &str;
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError>;
}

pub async fn providers_handler(State(state): State<SharedState>) -> Response {
    let mut response = (StatusCode::OK, Json(state.agent_providers.safe_list())).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_errors_are_fixed_and_serializable() {
        let error = ProviderError::upstream_status(429);
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(error.code, "provider_http_429");
        assert!(error.retryable);
        assert!(json.contains("429"));
        assert!(!json.contains("Authorization"));
    }

    #[test]
    fn upstream_body_is_only_used_for_fixed_classification() {
        let error = ProviderError::classified_upstream_response(
            400,
            br#"{"error":{"message":"Missing reasoning_content; private detail"}}"#,
        );
        let json = serde_json::to_string(&error).unwrap();

        assert_eq!(error.code, "provider_reasoning_context_required");
        assert!(!json.contains("private detail"));
        assert!(!json.contains("Missing reasoning_content"));
    }

    #[test]
    fn payment_required_is_classified_without_exposing_upstream_body() {
        let error = ProviderError::classified_upstream_response(
            402,
            br#"{"error":{"message":"Insufficient Balance; private account detail"}}"#,
        );
        let json = serde_json::to_string(&error).unwrap();

        assert_eq!(error.code, "provider_balance_insufficient");
        assert!(!error.retryable);
        assert_eq!(error.upstream_status, Some(402));
        assert!(!json.contains("private account detail"));
        assert!(!json.contains("Insufficient Balance"));
    }
}
