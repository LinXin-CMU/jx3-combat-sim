//! One custom profile per worker. Metadata is persisted; the key is never serialized.
use super::{
    custom_network,
    openai::{ChatCompatibility, OpenAiChatProvider, OpenAiResponsesProvider},
    LlmProvider, ModelMessage, ModelRequest, ProviderError, SafeProviderProfile, ToolDefinition,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const CUSTOM_PROFILE_ID: &str = "user-custom";
const MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomProtocol {
    ChatCompletions,
    Responses,
    Deepseek,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CustomMetadata {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub protocol: CustomProtocol,
}

// No Debug or Serialize: request credentials cannot enter debug logs or responses.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomInput {
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub protocol: CustomProtocol,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Default)]
struct CustomState {
    metadata: Option<CustomMetadata>,
    api_key: String,
    path: Option<PathBuf>,
    allow_loopback: bool,
}

#[derive(Clone)]
pub struct CustomProviderStore {
    state: Arc<Mutex<CustomState>>,
    probe: Arc<tokio::sync::Semaphore>,
}

impl Default for CustomProviderStore {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CustomState::default())),
            probe: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

impl std::fmt::Debug for CustomProviderStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomProviderStore")
            .finish_non_exhaustive()
    }
}

impl CustomMetadata {
    fn validate(mut self, allow_loopback: bool) -> Result<Self, ProviderError> {
        self.label = self.label.trim().to_string();
        self.model = self.model.trim().to_string();
        if self.label.is_empty() {
            self.label = "自定义接口".into();
        }
        if self.label.len() > 80
            || self.model.is_empty()
            || self.model.len() > 128
            || self
                .label
                .chars()
                .chain(self.model.chars())
                .any(char::is_control)
        {
            return Err(ProviderError::configuration(
                "custom_text_invalid",
                "请填写模型名；名称和模型名过长或含无效字符",
            ));
        }
        self.base_url = custom_network::normalize_url(&self.base_url, allow_loopback)?;
        Ok(self)
    }
}

impl CustomProviderStore {
    pub fn configure(&self, path: PathBuf, allow_loopback: bool) {
        let mut state = self.state.lock().unwrap();
        state.allow_loopback = allow_loopback;
        // Only bounded, non-secret metadata can be restored after a restart.
        state.metadata = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= MAX_BODY_BYTES as u64)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|b| serde_json::from_slice::<CustomMetadata>(&b).ok())
            .and_then(|m| m.validate(allow_loopback).ok());
        state.api_key.clear();
        state.path = Some(path);
    }

    pub fn safe_profile(&self) -> Option<SafeProviderProfile> {
        let state = self.state.lock().unwrap();
        state.metadata.as_ref().map(|m| SafeProviderProfile {
            id: CUSTOM_PROFILE_ID.into(),
            label: m.label.clone(),
            model: m.model.clone(),
            available: !state.api_key.is_empty(),
        })
    }

    pub fn view(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap();
        json!({"profile_id": CUSTOM_PROFILE_ID, "config": state.metadata,
            "has_key": !state.api_key.is_empty(), "key_storage": "worker_memory"})
    }

    fn prepare(
        input: CustomInput,
        state: &CustomState,
    ) -> Result<(CustomMetadata, String), ProviderError> {
        let metadata = CustomMetadata {
            label: input.label,
            base_url: input.base_url,
            model: input.model,
            protocol: input.protocol,
        }
        .validate(state.allow_loopback)?;
        let key = if input.api_key.trim().is_empty() {
            // Never forward an existing key to a newly entered endpoint.
            if !state
                .metadata
                .as_ref()
                .is_some_and(|m| m.base_url == metadata.base_url)
            {
                return Err(ProviderError::configuration(
                    "custom_key_required",
                    "请为这个 API 地址填写 API Key",
                ));
            }
            state.api_key.clone()
        } else {
            input.api_key.trim().to_string()
        };
        if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ProviderError::configuration(
                "custom_key_invalid",
                "请填写有效的 API Key",
            ));
        }
        Ok((metadata, key))
    }

    pub fn save(&self, input: CustomInput) -> Result<(), ProviderError> {
        let mut state = self.state.lock().unwrap();
        let (metadata, key) = Self::prepare(input, &state)?;
        if let Some(path) = &state.path {
            let write = || -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, serde_json::to_vec_pretty(&metadata)?)?;
                std::fs::rename(tmp, path)
            };
            write().map_err(|_| {
                ProviderError::configuration("custom_save_failed", "接口配置保存失败，请重试")
            })?;
        }
        state.metadata = Some(metadata);
        state.api_key = key;
        Ok(())
    }

    pub fn remove(&self) -> Result<(), ProviderError> {
        let mut state = self.state.lock().unwrap();
        if let Some(path) = &state.path {
            match std::fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => {
                    return Err(ProviderError::configuration(
                        "custom_delete_failed",
                        "接口配置移除失败，请重试",
                    ))
                }
            }
        }
        state.metadata = None;
        state.api_key.clear();
        Ok(())
    }

    fn build(
        metadata: CustomMetadata,
        key: String,
        allow_loopback: bool,
    ) -> Result<Box<dyn LlmProvider>, ProviderError> {
        let client = custom_network::client(allow_loopback)?;
        let credential = key.clone();
        let inner: Box<dyn LlmProvider> = match metadata.protocol {
            CustomProtocol::Responses => Box::new(
                OpenAiResponsesProvider::new(
                    CUSTOM_PROFILE_ID.into(),
                    metadata.model,
                    metadata.base_url,
                    key,
                )?
                .with_client(client),
            ),
            protocol => Box::new(
                OpenAiChatProvider::new_with_compatibility(
                    CUSTOM_PROFILE_ID.into(),
                    metadata.model,
                    metadata.base_url,
                    key,
                    if protocol == CustomProtocol::Deepseek {
                        ChatCompatibility::Deepseek
                    } else {
                        ChatCompatibility::Openai
                    },
                )?
                .with_client(client),
            ),
        };
        Ok(Box::new(CustomProvider { inner, credential }))
    }

    pub fn create_provider(&self) -> Result<Box<dyn LlmProvider>, ProviderError> {
        let state = self.state.lock().unwrap();
        let metadata = state.metadata.clone().ok_or_else(|| {
            ProviderError::configuration("provider_not_found", "请先配置自定义接口")
        })?;
        if state.api_key.is_empty() {
            return Err(ProviderError::configuration(
                "provider_key_unavailable",
                "服务已重启，请重新填写自定义接口的 API Key",
            ));
        }
        Self::build(metadata, state.api_key.clone(), state.allow_loopback)
    }

    fn draft_provider(&self, input: CustomInput) -> Result<Box<dyn LlmProvider>, ProviderError> {
        let state = self.state.lock().unwrap();
        let (metadata, key) = Self::prepare(input, &state)?;
        Self::build(metadata, key, state.allow_loopback)
    }
}

struct CustomProvider {
    inner: Box<dyn LlmProvider>,
    credential: String,
}

#[async_trait::async_trait]
impl LlmProvider for CustomProvider {
    fn profile_id(&self) -> &str {
        self.inner.profile_id()
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
    async fn complete(
        &self,
        request: &ModelRequest,
    ) -> Result<super::ModelResponse, ProviderError> {
        let result = self.inner.complete(request).await.map_err(|mut error| {
            // Arbitrary upstream diagnostics must not enter replay files.
            error.private_detail = None;
            error
        })?;
        if serde_json::to_string(&result)
            .unwrap_or_default()
            .contains(&self.credential)
            || result
                .reasoning_content
                .as_ref()
                .is_some_and(|s| s.contains(&self.credential))
        {
            return Err(ProviderError::configuration(
                "custom_credential_echo",
                "接口响应包含凭据内容，请检查服务商接口",
            ));
        }
        Ok(result)
    }
}

fn response(status: StatusCode, body: impl Serialize) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

fn error(error: ProviderError) -> Response {
    response(StatusCode::BAD_REQUEST, json!({"error": error}))
}

fn check_headers(headers: &HeaderMap) -> Result<(), ProviderError> {
    // This header is deliberately absent from the app's CORS allow_headers.
    // Cross-origin web pages cannot issue this request, including via the router.
    if headers
        .get("x-jx3-provider-settings")
        .and_then(|h| h.to_str().ok())
        != Some("1")
        || headers
            .get("sec-fetch-site")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|s| !matches!(s, "same-origin" | "none"))
    {
        return Err(ProviderError::configuration(
            "custom_same_origin_required",
            "请从模拟器页面打开接口设置",
        ));
    }
    if let Some(origin) = headers.get("origin") {
        let secure = origin
            .to_str()
            .ok()
            .and_then(|s| reqwest::Url::parse(s).ok())
            .is_some_and(|u| {
                u.scheme() == "https"
                    || u.scheme() == "http"
                        && u.host_str().is_some_and(super::config::is_loopback_host)
            });
        if !secure {
            return Err(ProviderError::configuration(
                "custom_secure_page_required",
                "请通过 HTTPS 或本机地址配置 API Key",
            ));
        }
    }
    Ok(())
}

fn parse_input(body: &[u8]) -> Result<CustomInput, ProviderError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ProviderError::configuration(
            "custom_body_too_large",
            "接口配置内容过长",
        ));
    }
    serde_json::from_slice(body)
        .map_err(|_| ProviderError::configuration("custom_input_invalid", "接口配置格式不正确"))
}

pub async fn get_handler(State(state): State<crate::SharedState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_headers(&headers) {
        return error(e);
    }
    response(StatusCode::OK, state.agent_providers.custom.view())
}

pub async fn save_handler(
    State(state): State<crate::SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = check_headers(&headers)
        .and_then(|_| parse_input(&body))
        .and_then(|input| state.agent_providers.custom.save(input));
    match result {
        Ok(()) => response(StatusCode::OK, state.agent_providers.custom.view()),
        Err(e) => error(e),
    }
}

pub async fn delete_handler(
    State(state): State<crate::SharedState>,
    headers: HeaderMap,
) -> Response {
    match check_headers(&headers).and_then(|_| state.agent_providers.custom.remove()) {
        Ok(()) => response(StatusCode::OK, json!({"ok": true})),
        Err(e) => error(e),
    }
}

fn probe_request() -> ModelRequest {
    ModelRequest {
        instructions: "This is a connection and function-calling test. Call connection_test with ok=true once. Do not answer in prose.".into(),
        messages: vec![ModelMessage::User { content: "Call connection_test now.".into() }],
        tools: vec![ToolDefinition { name: "connection_test".into(), description: "Verify function calling without side effects.".into(),
            parameters: json!({"type":"object", "properties":{"ok":{"type":"boolean"}}, "required":["ok"], "additionalProperties":false}) }],
        response_format: None, max_output_tokens: 512,
    }
}

pub async fn test_handler(
    State(state): State<crate::SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let provider = match check_headers(&headers)
        .and_then(|_| parse_input(&body))
        .and_then(|input| state.agent_providers.custom.draft_provider(input))
    {
        Ok(p) => p,
        Err(e) => return error(e),
    };
    let Ok(_permit) = state.agent_providers.custom.probe.try_acquire() else {
        return response(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error":{"code":"custom_test_busy", "message":"已有连接测试正在进行"}}),
        );
    };
    let start = Instant::now();
    match tokio::time::timeout(Duration::from_secs(40), provider.complete(&probe_request())).await {
        Ok(Ok(result)) => response(
            StatusCode::OK,
            json!({"ok":true, "latency_ms":start.elapsed().as_millis(),
            "tool_calling": result.tool_calls.iter().any(|c| c.name == "connection_test" && c.arguments == json!({"ok":true}))}),
        ),
        Ok(Err(e)) => error(e),
        Err(_) => error(ProviderError::configuration(
            "provider_timeout",
            "连接测试超时，请检查地址与模型",
        )),
    }
}

#[cfg(test)]
#[path = "../../../tests/agent/custom_provider.rs"]
mod tests;
