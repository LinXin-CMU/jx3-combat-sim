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
    FinishReason, ModelMessage, ModelRequest, ModelResponse, ProviderToolCall, TokenUsage,
    ToolDefinition, PROVIDER_PROTOCOL_V1,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
}

impl ProviderError {
    pub fn configuration(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            upstream_status: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: "invalid_model_request",
            message: "model request failed local protocol validation",
            retryable: false,
            upstream_status: None,
        }
    }

    pub fn network(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self {
                code: "provider_timeout",
                message: "provider request timed out",
                retryable: true,
                upstream_status: None,
            }
        } else {
            Self {
                code: "provider_network_error",
                message: "provider request failed",
                retryable: true,
                upstream_status: None,
            }
        }
    }

    pub fn upstream_status(status: u16) -> Self {
        Self {
            code: "provider_http_error",
            message: "provider returned an unsuccessful status",
            retryable: status == 408 || status == 429 || status >= 500,
            upstream_status: Some(status),
        }
    }

    pub fn response_too_large() -> Self {
        Self {
            code: "provider_response_too_large",
            message: "provider response exceeded the configured limit",
            retryable: false,
            upstream_status: None,
        }
    }

    pub fn invalid_response() -> Self {
        Self {
            code: "provider_response_invalid",
            message: "provider returned an invalid response",
            retryable: false,
            upstream_status: None,
        }
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
        assert_eq!(error.code, "provider_http_error");
        assert!(error.retryable);
        assert!(json.contains("429"));
        assert!(!json.contains("Authorization"));
    }
}
