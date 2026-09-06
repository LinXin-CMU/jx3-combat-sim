use super::{
    config::is_loopback_host,
    protocol::{
        valid_identifier, FinishReason, ModelMessage, ModelRequest, ModelResponse,
        ProviderToolCall, TokenUsage,
    },
    LlmProvider, ProviderError,
};
use async_trait::async_trait;
use reqwest::{redirect::Policy, Client, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;
const PROVIDER_TIMEOUT_SECS: u64 = 240;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatCompatibility {
    #[default]
    Openai,
    Deepseek,
}

struct OpenAiTransport {
    profile_id: String,
    model: String,
    base_url: String,
    api_key: String,
    client: Client,
}

impl OpenAiTransport {
    fn new(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, ProviderError> {
        Self::new_with_timeout(
            profile_id,
            model,
            base_url,
            api_key,
            Duration::from_secs(PROVIDER_TIMEOUT_SECS),
        )
    }

    fn new_with_timeout(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        if api_key.trim().is_empty() || api_key.chars().any(char::is_control) {
            return Err(ProviderError::configuration(
                "provider_key_invalid",
                "provider credential is invalid",
            ));
        }
        let parsed_base_url = Url::parse(&base_url).map_err(|_| {
            ProviderError::configuration(
                "provider_base_url_invalid",
                "provider base URL is invalid",
            )
        })?;
        if !matches!(parsed_base_url.scheme(), "http" | "https")
            || parsed_base_url.host_str().is_none()
            || !parsed_base_url.username().is_empty()
            || parsed_base_url.password().is_some()
            || parsed_base_url.query().is_some()
            || parsed_base_url.fragment().is_some()
        {
            return Err(ProviderError::configuration(
                "provider_base_url_invalid",
                "provider base URL is invalid",
            ));
        }
        if parsed_base_url.scheme() == "http"
            && !parsed_base_url.host_str().is_some_and(is_loopback_host)
        {
            return Err(ProviderError::configuration(
                "provider_insecure_remote_url",
                "remote provider base URL must use HTTPS",
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .redirect(Policy::none())
            .build()
            .map_err(|_| {
                ProviderError::configuration(
                    "provider_client_unavailable",
                    "provider HTTP client cannot be initialized",
                )
            })?;
        Ok(Self {
            profile_id,
            model,
            base_url: parsed_base_url.as_str().trim_end_matches('/').to_string(),
            api_key,
            client,
        })
    }

    fn endpoint(&self, suffix: &str) -> Result<Url, ProviderError> {
        Url::parse(&format!("{}/{}", self.base_url, suffix)).map_err(|_| {
            ProviderError::configuration(
                "provider_base_url_invalid",
                "provider base URL is invalid",
            )
        })
    }

    async fn post_json(&self, suffix: &str, body: &Value) -> Result<Vec<u8>, ProviderError> {
        let encoded = serde_json::to_vec(body).map_err(|_| ProviderError::invalid_request())?;
        let response = self
            .client
            .post(self.endpoint(suffix)?)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json; charset=utf-8")
            .body(encoded)
            .send()
            .await
            .map_err(|error| ProviderError::network(&error))?;

        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64)
        {
            return Err(ProviderError::response_too_large());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ProviderError::network(&error))?;
        if bytes.len() > MAX_PROVIDER_RESPONSE_BYTES {
            return Err(ProviderError::response_too_large());
        }
        if !status.is_success() {
            return Err(ProviderError::classified_upstream_response(
                status.as_u16(),
                &bytes,
            ));
        }
        Ok(bytes.to_vec())
    }
}

pub struct OpenAiResponsesProvider {
    transport: OpenAiTransport,
}

impl OpenAiResponsesProvider {
    pub fn new(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            transport: OpenAiTransport::new(profile_id, model, base_url, api_key)?,
        })
    }

    #[cfg(test)]
    fn new_with_timeout(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            transport: OpenAiTransport::new_with_timeout(
                profile_id, model, base_url, api_key, timeout,
            )?,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    fn profile_id(&self) -> &str {
        &self.transport.profile_id
    }

    fn model(&self) -> &str {
        &self.transport.model
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        request
            .validate()
            .map_err(|_| ProviderError::invalid_request())?;
        let body = responses_request(&self.transport.model, request)?;
        let bytes = self.transport.post_json("responses", &body).await?;
        let response = parse_responses_response(&bytes)?;
        response.validate().map_err(|error| {
            ProviderError::invalid_response_protocol(error.code, error.message)
                .with_usage(response.usage.clone())
        })?;
        Ok(response)
    }
}

pub struct OpenAiChatProvider {
    transport: OpenAiTransport,
    compatibility: ChatCompatibility,
}

impl OpenAiChatProvider {
    #[cfg(test)]
    pub fn new(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, ProviderError> {
        Self::new_with_compatibility(
            profile_id,
            model,
            base_url,
            api_key,
            ChatCompatibility::Openai,
        )
    }

    pub fn new_with_compatibility(
        profile_id: String,
        model: String,
        base_url: String,
        api_key: String,
        compatibility: ChatCompatibility,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            transport: OpenAiTransport::new(profile_id, model, base_url, api_key)?,
            compatibility,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiChatProvider {
    fn profile_id(&self) -> &str {
        &self.transport.profile_id
    }

    fn model(&self) -> &str {
        &self.transport.model
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        request
            .validate()
            .map_err(|_| ProviderError::invalid_request())?;
        let body =
            chat_request_with_compatibility(&self.transport.model, request, self.compatibility)?;
        let bytes = self.transport.post_json("chat/completions", &body).await?;
        let response = parse_chat_response(&bytes)?;
        response.validate().map_err(|error| {
            ProviderError::invalid_response_protocol(error.code, error.message)
                .with_usage(response.usage.clone())
        })?;
        Ok(response)
    }
}

fn responses_request(model: &str, request: &ModelRequest) -> Result<Value, ProviderError> {
    let mut input = Vec::new();
    for message in &request.messages {
        match message {
            ModelMessage::User { content } => input.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": content}],
            })),
            ModelMessage::Assistant {
                content,
                tool_calls,
                reasoning_content: _,
            } => {
                if let Some(content) = content {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": content}],
                    }));
                }
                for call in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.call_id,
                        "name": call.name,
                        "arguments": compact_json(&call.arguments)?,
                    }));
                }
            }
            ModelMessage::ToolResult { call_id, output } => input.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": compact_json(output)?,
            })),
        }
    }
    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": true,
            })
        })
        .collect();
    let mut body = json!({
        "model": model,
        "instructions": request.instructions,
        "input": input,
        "tools": tools,
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "max_output_tokens": request.max_output_tokens,
        "store": false,
    });
    if let Some(format) = &request.response_format {
        body["text"] = json!({
            "format": {
                "type": "json_schema",
                "name": format.name,
                "schema": format.schema,
                "strict": true
            }
        });
    }
    Ok(body)
}

#[cfg(test)]
fn chat_request(model: &str, request: &ModelRequest) -> Result<Value, ProviderError> {
    chat_request_with_compatibility(model, request, ChatCompatibility::Openai)
}

fn chat_request_with_compatibility(
    model: &str,
    request: &ModelRequest,
    compatibility: ChatCompatibility,
) -> Result<Value, ProviderError> {
    let mut messages = vec![json!({
        "role": "system",
        "content": request.instructions,
    })];
    for message in &request.messages {
        match message {
            ModelMessage::User { content } => {
                messages.push(json!({"role": "user", "content": content}));
            }
            ModelMessage::Assistant {
                content,
                tool_calls,
                reasoning_content,
            } => {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|call| {
                        Ok(json!({
                            "id": call.call_id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": compact_json(&call.arguments)?,
                            },
                        }))
                    })
                    .collect::<Result<_, ProviderError>>()?;
                let mut assistant = json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": calls,
                });
                if compatibility == ChatCompatibility::Deepseek && !tool_calls.is_empty() {
                    assistant["reasoning_content"] =
                        json!(reasoning_content.as_deref().unwrap_or_default());
                }
                messages.push(assistant);
            }
            ModelMessage::ToolResult { call_id, output } => messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": compact_json(output)?,
            })),
        }
    }
    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|tool| {
            let mut function = json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            });
            if compatibility == ChatCompatibility::Openai {
                function["strict"] = json!(true);
            }
            json!({
                "type": "function",
                "function": function,
            })
        })
        .collect();
    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_tokens": request.max_output_tokens,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }
    match compatibility {
        ChatCompatibility::Openai => body["parallel_tool_calls"] = json!(false),
        ChatCompatibility::Deepseek => {
            body["thinking"] = json!({"type": "enabled"});
            body["reasoning_effort"] = json!(if model.contains("flash") || request.tools.is_empty() {
                "low"
            } else {
                "high"
            });
            let setting = if request.tools.is_empty() {
                "JX3_AGENT_REPORT_REASONING_EFFORT"
            } else if model.contains("flash") {
                "JX3_AGENT_FLASH_REASONING_EFFORT"
            } else {
                "JX3_AGENT_PRO_REASONING_EFFORT"
            };
            if let Ok(effort) = std::env::var(setting) {
                if matches!(effort.as_str(), "low" | "high") {
                    body["reasoning_effort"] = json!(effort);
                }
            }
        }
    }
    if let Some(format) = &request.response_format {
        body["response_format"] = match compatibility {
            ChatCompatibility::Openai => json!({
                "type": "json_schema",
                "json_schema": {
                    "name": format.name,
                    "schema": format.schema,
                    "strict": true
                }
            }),
            ChatCompatibility::Deepseek if request.tools.is_empty() => {
                json!({"type": "json_object"})
            }
            ChatCompatibility::Deepseek => Value::Null,
        };
        if body["response_format"].is_null() {
            body.as_object_mut()
                .expect("Chat request body is an object")
                .remove("response_format");
        }
    }
    Ok(body)
}

fn compact_json(value: &Value) -> Result<String, ProviderError> {
    serde_json::to_string(value).map_err(|_| ProviderError::invalid_request())
}

#[derive(Deserialize)]
struct ResponsesPayload {
    #[serde(default)]
    output: Vec<ResponsesOutput>,
    #[serde(default)]
    usage: ResponsesUsage,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    incomplete_details: Option<IncompleteDetails>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ResponsesOutput {
    #[serde(rename = "message")]
    Message {
        #[serde(default)]
        content: Vec<ResponsesContent>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ResponsesContent {
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
    #[serde(other)]
    Other,
}

#[derive(Default, Deserialize)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Deserialize)]
struct IncompleteDetails {
    #[serde(default)]
    reason: Option<String>,
}

fn parse_responses_response(bytes: &[u8]) -> Result<ModelResponse, ProviderError> {
    let payload: ResponsesPayload =
        serde_json::from_slice(bytes).map_err(|_| ProviderError::invalid_response())?;
    let mut text = Vec::new();
    let mut tool_calls = Vec::new();
    let mut refused = false;
    for output in payload.output {
        match output {
            ResponsesOutput::Message { content } => {
                for item in content {
                    match item {
                        ResponsesContent::OutputText { text: item } if !item.is_empty() => {
                            text.push(item)
                        }
                        ResponsesContent::Refusal { refusal } if !refusal.is_empty() => {
                            refused = true;
                            text.push(refusal);
                        }
                        _ => {}
                    }
                }
            }
            ResponsesOutput::FunctionCall {
                call_id,
                name,
                arguments,
            } => tool_calls.push(parse_tool_call(call_id, name, &arguments)?),
            ResponsesOutput::Other => {}
        }
    }
    if text.is_empty() && tool_calls.is_empty() {
        return Err(ProviderError::invalid_response());
    }
    let finish_reason = if !tool_calls.is_empty() {
        FinishReason::ToolCalls
    } else if refused {
        FinishReason::Refusal
    } else if payload.status.as_deref() == Some("incomplete") {
        match payload
            .incomplete_details
            .and_then(|details| details.reason)
            .as_deref()
        {
            Some("max_output_tokens") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Other,
        }
    } else {
        FinishReason::Stop
    };
    Ok(ModelResponse {
        assistant_text: (!text.is_empty()).then(|| text.join("\n")),
        reasoning_content: None,
        tool_calls,
        finish_reason,
        usage: TokenUsage {
            input_tokens: payload.usage.input_tokens,
            output_tokens: payload.usage.output_tokens,
            total_tokens: payload.usage.total_tokens,
        },
    })
}

#[derive(Deserialize)]
struct ChatPayload {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: ChatUsage,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Deserialize)]
struct ChatToolCall {
    id: String,
    function: ChatFunction,
}

#[derive(Deserialize)]
struct ChatFunction {
    name: String,
    arguments: String,
}

#[derive(Default, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

fn parse_chat_response(bytes: &[u8]) -> Result<ModelResponse, ProviderError> {
    let payload: ChatPayload = serde_json::from_slice(bytes).map_err(|_| {
        ProviderError::invalid_response_protocol(
            "provider_response_json_invalid",
            "provider response was not valid Chat Completions JSON",
        )
    })?;
    let usage = TokenUsage {
        input_tokens: payload.usage.prompt_tokens,
        output_tokens: payload.usage.completion_tokens,
        total_tokens: payload.usage.total_tokens,
    };
    let choice = payload.choices.into_iter().next().ok_or_else(|| {
        ProviderError::invalid_response_protocol(
            "provider_response_empty",
            "provider response did not contain a choice",
        )
        .with_usage(usage.clone())
    })?;
    let reasoning_content = choice
        .message
        .reasoning_content
        .filter(|value| !value.trim().is_empty());
    let mut text = choice
        .message
        .content
        .filter(|value| !value.trim().is_empty());
    let refused = choice.message.refusal.is_some();
    if text.is_none() {
        text = choice.message.refusal.filter(|value| !value.is_empty());
    }
    let mut tool_calls: Vec<ProviderToolCall> = choice
        .message
        .tool_calls
        .into_iter()
        .map(|call| parse_tool_call(call.id, call.function.name, &call.function.arguments))
        .collect::<Result<_, _>>()
        .map_err(|error| error.with_usage(usage.clone()))?;
    // Compatible providers occasionally emit a complete call in `content`
    // instead of `tool_calls`. Normalize only a standalone call object; the
    // usual exposed-tool and argument validation still runs before execution.
    if tool_calls.is_empty()
        && !refused
        && !matches!(choice.finish_reason.as_deref(), Some("length" | "content_filter"))
    {
        if let Some(call) = text.as_deref().and_then(parse_standalone_text_tool_call) {
            tool_calls.push(call);
            text = None;
        }
    }
    if text.is_none() && tool_calls.is_empty() {
        return Err(ProviderError::invalid_response_protocol(
            if choice.finish_reason.as_deref() == Some("length") {
                "provider_output_limit"
            } else { "provider_response_empty" },
            if choice.finish_reason.as_deref() == Some("length") {
                "provider exhausted the output allowance before returning an answer"
            } else { "provider response contained neither text nor tool calls" },
        )
        .with_usage(usage));
    }
    let finish_reason = if !tool_calls.is_empty() {
        FinishReason::ToolCalls
    } else if refused {
        FinishReason::Refusal
    } else {
        match choice.finish_reason.as_deref() {
            Some("stop") | None => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Other,
        }
    };
    Ok(ModelResponse {
        assistant_text: text,
        reasoning_content,
        tool_calls,
        finish_reason,
        usage,
    })
}

fn parse_standalone_text_tool_call(text: &str) -> Option<ProviderToolCall> {
    if text.len() > 64 * 1024 { return None; }
    let candidate = text.trim();
    let candidate = candidate.strip_prefix("<tool_call>").unwrap_or(candidate).trim();
    let candidate = candidate.strip_suffix("</tool_call>").unwrap_or(candidate).trim();
    let value: Value = serde_json::from_str(candidate).ok()?;
    let object = value.as_object()?;
    if object.len() != 2 { return None; }
    let name = object.get("name")?.as_str()?;
    let arguments = object.get("arguments")?;
    if !valid_identifier(name) || !arguments.is_object() { return None; }
    static NEXT_TEXT_CALL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT_TEXT_CALL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(ProviderToolCall {
        call_id: format!("text_call_{id}"),
        name: name.to_string(),
        arguments: arguments.clone(),
    })
}

fn parse_tool_call(
    call_id: String,
    name: String,
    arguments: &str,
) -> Result<ProviderToolCall, ProviderError> {
    if !valid_identifier(&call_id) || !valid_identifier(&name) {
        return Err(ProviderError::invalid_response_protocol(
            "provider_tool_call_invalid",
            "provider returned an invalid tool call id or name",
        ));
    }
    let private_argument_excerpt = || {
        let redacted = crate::agent::session::redact_sensitive_text(arguments);
        let excerpt = redacted.chars().take(1_024).collect::<String>();
        format!("tool={name}; arguments={excerpt}")
    };
    let mut arguments: Value = parse_tool_arguments(arguments)
        .map_err(|error| error.with_private_detail(private_argument_excerpt()))?;
    // A few compatible chat providers occasionally wrap a single function
    // argument object in an array. Unwrap only the unambiguous one-object
    // shape; the normal tool schema still validates every field afterwards.
    if let Value::Array(items) = &arguments {
        if items.len() == 1 && items[0].is_object() {
            arguments = items[0].clone();
        }
    }
    if !arguments.is_object() {
        return Err(ProviderError::invalid_response_protocol(
            "provider_tool_arguments_invalid",
            "provider tool arguments were not a JSON object",
        )
        .with_private_detail(private_argument_excerpt()));
    }
    Ok(ProviderToolCall {
        call_id,
        name,
        arguments,
    })
}

/// Some OpenAI-compatible providers occasionally put literal newlines inside a
/// JSON string or leave a trailing comma in function arguments. Repair only
/// those two transport-level defects; the closed tool schema and local typed
/// validation still reject invented fields or invalid simulator inputs.
fn parse_tool_arguments(arguments: &str) -> Result<Value, ProviderError> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    if let Ok(value) = serde_json::from_str(trimmed) {
        if let Value::String(inner) = &value {
            if let Ok(unwrapped) = parse_tool_arguments(inner) {
                return Ok(unwrapped);
            }
        }
        return Ok(value);
    }
    if arguments.len() > 64 * 1024 {
        return Err(ProviderError::invalid_response_protocol(
            "provider_tool_arguments_invalid",
            "provider returned invalid JSON tool arguments",
        ));
    }
    let repaired = repair_common_json_transport_defects(trimmed);
    serde_json::from_str(&repaired)
        .ok()
        .or_else(|| {
            extract_first_json_object(&repaired)
                .and_then(|candidate| serde_json::from_str(candidate).ok())
        })
        .or_else(|| {
            let pythonish = repair_pythonish_argument_object(&repaired);
            let bare_scalars = quote_bare_json_scalar_values(&pythonish);
            serde_json::from_str(&bare_scalars).ok().or_else(|| {
                extract_first_json_object(&bare_scalars)
                    .and_then(|candidate| serde_json::from_str(candidate).ok())
            })
        })
        .ok_or_else(|| {
            ProviderError::invalid_response_protocol(
                "provider_tool_arguments_invalid",
                "provider returned invalid JSON tool arguments",
            )
        })
}

/// Recover the common Python-dict spelling emitted by some compatible model
/// gateways (`'text'`, `None`, `True`, `False`). This runs only after strict
/// JSON parsing and the ordinary transport repair both fail.
fn repair_pythonish_argument_object(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if in_double {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if escaped {
                if character == '\'' {
                    output.push('\'');
                } else {
                    output.push('\\');
                    output.push(character);
                }
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '\'' {
                output.push('"');
                in_single = false;
            } else if character == '"' {
                output.push_str("\\\"");
            } else {
                output.push(character);
            }
            continue;
        }
        if character == '"' {
            in_double = true;
            output.push(character);
            continue;
        }
        if character == '\'' {
            in_single = true;
            output.push('"');
            continue;
        }
        if character.is_ascii_alphabetic() {
            let mut token = String::from(character);
            while chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic() || *next == '_')
            {
                token.push(chars.next().expect("peeked character exists"));
            }
            output.push_str(match token.as_str() {
                "None" => "null",
                "True" => "true",
                "False" => "false",
                _ => &token,
            });
            continue;
        }
        output.push(character);
    }
    output
}

/// Quote an unquoted textual value after `:` while leaving JSON literals and
/// numbers untouched. Example: `{"skill_name": 绝刀}`. This recovery is
/// intentionally limited to value positions and runs only after strict parse
/// has already failed.
fn quote_bare_json_scalar_values(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < chars.len() {
        let character = chars[index];
        output.push(character);
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if character == '"' {
            in_string = true;
            index += 1;
            continue;
        }
        if character != ':' {
            index += 1;
            continue;
        }
        index += 1;
        while index < chars.len() && chars[index].is_whitespace() {
            output.push(chars[index]);
            index += 1;
        }
        let Some(next) = chars.get(index).copied() else {
            break;
        };
        let starts_json_value = matches!(next, '"' | '{' | '[' | '-' | '0'..='9')
            || chars[index..].starts_with(&['t', 'r', 'u', 'e'])
            || chars[index..].starts_with(&['f', 'a', 'l', 's', 'e'])
            || chars[index..].starts_with(&['n', 'u', 'l', 'l']);
        if starts_json_value {
            continue;
        }
        let start = index;
        while index < chars.len() && !matches!(chars[index], ',' | '}' | ']') {
            index += 1;
        }
        let bare = chars[start..index]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        output.push_str(&serde_json::to_string(&bare).unwrap_or_else(|_| "\"\"".to_string()));
    }
    output
}

fn extract_first_json_object(value: &str) -> Option<&str> {
    let start = value
        .char_indices()
        .find_map(|(index, character)| (character == '{').then_some(index))?;
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in value[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&value[start..start + offset + character.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

fn repair_common_json_transport_defects(arguments: &str) -> String {
    let mut escaped_controls = String::with_capacity(arguments.len() + 16);
    let mut in_string = false;
    let mut escaped = false;
    for character in arguments.chars() {
        if in_string {
            if escaped {
                escaped_controls.push(character);
                escaped = false;
                continue;
            }
            match character {
                '\\' => {
                    escaped_controls.push(character);
                    escaped = true;
                }
                '"' => {
                    escaped_controls.push(character);
                    in_string = false;
                }
                '\n' => escaped_controls.push_str("\\n"),
                '\r' => escaped_controls.push_str("\\r"),
                '\t' => escaped_controls.push_str("\\t"),
                _ => escaped_controls.push(character),
            }
        } else {
            if character == '"' {
                in_string = true;
            }
            escaped_controls.push(character);
        }
    }

    let characters = escaped_controls.chars().collect::<Vec<_>>();
    let mut without_trailing_commas = String::with_capacity(escaped_controls.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in characters.iter().copied().enumerate() {
        if in_string {
            without_trailing_commas.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            without_trailing_commas.push(character);
            continue;
        }
        if character == ',' {
            let next = characters[index + 1..]
                .iter()
                .copied()
                .find(|next| !next.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        without_trailing_commas.push(character);
    }
    without_trailing_commas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::{StructuredOutputDefinition, ToolDefinition};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    fn request() -> ModelRequest {
        ModelRequest {
            instructions: "Use grounded tool evidence only.".to_string(),
            messages: vec![ModelMessage::User {
                content: "Inspect the current scenario.".to_string(),
            }],
            tools: vec![ToolDefinition {
                name: "get_current_scenario".to_string(),
                description: "Read the immutable scenario.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            }],
            response_format: None,
            max_output_tokens: 512,
        }
    }

    #[test]
    fn structured_output_schema_maps_to_both_adapter_shapes() {
        let mut request = request();
        request.response_format = Some(StructuredOutputDefinition {
            name: "agent_report_v1".to_string(),
            schema: json!({
                "type": "object",
                "properties": {"summary": {"type": "string"}},
                "required": ["summary"],
                "additionalProperties": false
            }),
        });

        let responses = responses_request("model", &request).unwrap();
        assert_eq!(responses["text"]["format"]["type"], "json_schema");
        assert_eq!(responses["text"]["format"]["strict"], true);
        assert_eq!(responses["text"]["format"]["name"], "agent_report_v1");

        let chat = chat_request("model", &request).unwrap();
        assert_eq!(chat["response_format"]["type"], "json_schema");
        assert_eq!(chat["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            chat["response_format"]["json_schema"]["name"],
            "agent_report_v1"
        );
    }

    #[test]
    fn deepseek_chat_mode_enables_high_effort_thinking_and_uses_supported_json_shape() {
        let mut request = request();
        request.response_format = Some(StructuredOutputDefinition {
            name: "agent_report_v1".to_string(),
            schema: json!({"type": "object"}),
        });

        let body = chat_request_with_compatibility(
            "deepseek-v4-pro",
            &request,
            ChatCompatibility::Deepseek,
        )
        .unwrap();

        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("response_format").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        assert!(body["tools"][0]["function"].get("strict").is_none());

        request.tools.clear();
        let repair_body = chat_request_with_compatibility(
            "deepseek-v4-pro",
            &request,
            ChatCompatibility::Deepseek,
        )
        .unwrap();
        assert_eq!(repair_body["response_format"]["type"], "json_object");
        assert!(repair_body.get("tools").is_none());
        assert!(repair_body.get("tool_choice").is_none());
        assert_eq!(repair_body["reasoning_effort"], "low");

        let flash_body = chat_request_with_compatibility(
            "deepseek-v4-flash",
            &request,
            ChatCompatibility::Deepseek,
        )
        .unwrap();
        assert_eq!(flash_body["thinking"]["type"], "enabled");
        assert_eq!(flash_body["reasoning_effort"], "low");
    }

    #[test]
    fn deepseek_tool_continuation_echoes_transient_reasoning_context() {
        let mut request = request();
        request.messages.push(ModelMessage::Assistant {
            content: Some("先读取时间轴。".to_string()),
            tool_calls: vec![ProviderToolCall {
                call_id: "call-1".to_string(),
                name: "get_current_scenario".to_string(),
                arguments: json!({}),
            }],
            reasoning_content: Some("private provider reasoning".to_string()),
        });
        request.messages.push(ModelMessage::ToolResult {
            call_id: "call-1".to_string(),
            output: json!({"ok": true}),
        });

        let body = chat_request_with_compatibility(
            "deepseek-v4-pro",
            &request,
            ChatCompatibility::Deepseek,
        )
        .unwrap();

        assert_eq!(
            body["messages"][2]["reasoning_content"],
            "private provider reasoning"
        );
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("private provider reasoning"));
    }

    async fn mock_server(status: u16, body: &'static str) -> (String, oneshot::Receiver<String>) {
        mock_server_with_delay(status, body, Duration::ZERO).await
    }

    async fn mock_server_with_delay(
        status: u16,
        body: &'static str,
        delay: Duration,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end;
            loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&chunk[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    header_end = position + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
            tokio::time::sleep(delay).await;
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
        (format!("http://{address}/v1"), receiver)
    }

    #[tokio::test]
    async fn responses_adapter_sends_stateless_serial_tool_request_and_parses_call() {
        let body = r#"{
            "status":"completed",
            "output":[{"type":"function_call","call_id":"call-1","name":"get_current_scenario","arguments":"{}"}],
            "usage":{"input_tokens":12,"output_tokens":7,"total_tokens":19}
        }"#;
        let (base_url, captured) = mock_server(200, body).await;
        let provider = OpenAiResponsesProvider::new(
            "openai".to_string(),
            "example-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();
        let response = provider.complete(&request()).await.unwrap();
        let captured = captured.await.unwrap();
        let (_, request_body) = captured.split_once("\r\n\r\n").unwrap();
        let request_body: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.tool_calls[0].name, "get_current_scenario");
        assert_eq!(response.usage.total_tokens, 19);
        assert_eq!(request_body["store"], false);
        assert_eq!(request_body["parallel_tool_calls"], false);
        assert_eq!(request_body["tools"][0]["strict"], true);
        assert!(captured.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(captured
            .to_ascii_lowercase()
            .contains("authorization: bearer test-secret-value"));
    }

    #[tokio::test]
    async fn chat_adapter_uses_compatible_tool_shape_and_parses_text() {
        let body = r#"{
            "choices":[{"message":{"content":"Grounded result.","tool_calls":[]},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":8,"completion_tokens":3,"total_tokens":11}
        }"#;
        let (base_url, captured) = mock_server(200, body).await;
        let provider = OpenAiChatProvider::new(
            "compatible".to_string(),
            "example-chat-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();
        let response = provider.complete(&request()).await.unwrap();
        let captured = captured.await.unwrap();
        let (_, request_body) = captured.split_once("\r\n\r\n").unwrap();
        let request_body: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(response.assistant_text.as_deref(), Some("Grounded result."));
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert_eq!(response.usage.total_tokens, 11);
        assert_eq!(request_body["tools"][0]["function"]["strict"], true);
        assert_eq!(request_body["parallel_tool_calls"], false);
        assert!(captured.starts_with("POST /v1/chat/completions HTTP/1.1"));
    }

    #[test]
    fn empty_chat_response_retains_usage_for_accounting() {
        let body = br#"{
            "choices":[{"message":{"content":"","tool_calls":[]},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":120,"completion_tokens":7,"total_tokens":127}
        }"#;
        let error = parse_chat_response(body).unwrap_err();

        assert_eq!(error.code, "provider_response_empty");
        assert_eq!(error.usage.input_tokens, 120);
        assert_eq!(error.usage.output_tokens, 7);
        assert_eq!(error.usage.total_tokens, 127);
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("total_tokens"));
    }

    #[test]
    fn exhausted_output_is_distinct_from_empty_response() {
        let body = br#"{"choices":[{"message":{"content":""},"finish_reason":"length"}],"usage":{"completion_tokens":16384}}"#;
        let error = parse_chat_response(body).unwrap_err();
        assert_eq!(error.code, "provider_output_limit");
        assert_eq!(error.usage.output_tokens, 16384);
    }

    #[test]
    fn standalone_text_call_is_a_tool_action_not_a_report() {
        let call = r#"{"name":"inspect_timeline_events","arguments":{"selector":"skill","skill_name":"绝刀","buff_names":["血怒·惊涌"],"context_radius":4,"limit":6}}"#;
        for text in [call.to_string(), format!("<tool_call>{call}</tool_call>"), format!("{call}\n</tool_call>")] {
            let body = json!({"choices":[{"message":{"content":text},"finish_reason":"stop"}]});
            let response = parse_chat_response(&serde_json::to_vec(&body).unwrap()).unwrap();
            assert_eq!(response.finish_reason, FinishReason::ToolCalls);
            assert!(response.assistant_text.is_none());
            assert_eq!(response.tool_calls.len(), 1);
            assert_eq!(response.tool_calls[0].arguments["buff_names"][0], "血怒·惊涌");
            assert_eq!(response.tool_calls[0].arguments["context_radius"], 4);
            assert_eq!(response.tool_calls[0].arguments["limit"], 6);
        }
    }

    #[test]
    fn textual_call_normalization_does_not_execute_examples_or_bypass_tool_visibility() {
        let call = r#"{"name":"shell","arguments":{}}"#;
        for text in [format!("示例：{call}"), format!("```json\n{call}\n```"),
            r#"{"summary":"工具示例","name":"shell","arguments":{}}"#.to_string()] {
            assert!(parse_standalone_text_tool_call(&text).is_none());
        }
        let body = json!({"choices":[{"message":{"content":call},"finish_reason":"stop"}]});
        let response = parse_chat_response(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(response.validate_against(&request()).unwrap_err().code, "unregistered_provider_tool");
        let body = json!({"choices":[{"message":{"content":call},"finish_reason":"length"}]});
        assert!(parse_chat_response(&serde_json::to_vec(&body).unwrap()).unwrap().tool_calls.is_empty());
    }

    #[tokio::test]
    async fn unknown_chat_tool_is_returned_for_orchestrator_recovery() {
        let body = r#"{
            "choices":[{"message":{"content":null,"tool_calls":[{
                "id":"call-shell",
                "type":"function",
                "function":{"name":"shell","arguments":"{}"}
            }]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":80,"completion_tokens":9,"total_tokens":89}
        }"#;
        let (base_url, _captured) = mock_server(200, body).await;
        let provider = OpenAiChatProvider::new(
            "compatible".to_string(),
            "example-chat-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();
        let response = provider.complete(&request()).await.unwrap();

        assert_eq!(response.tool_calls[0].name, "shell");
        assert_eq!(response.usage.total_tokens, 89);
    }

    #[tokio::test]
    async fn upstream_error_body_and_key_are_never_reflected() {
        let (base_url, _captured) =
            mock_server(429, r#"{"error":{"message":"leaked-upstream-body"}}"#).await;
        let provider = OpenAiResponsesProvider::new(
            "openai".to_string(),
            "example-model".to_string(),
            base_url,
            "super-secret-credential".to_string(),
        )
        .unwrap();
        let error = provider.complete(&request()).await.unwrap_err();
        let serialized = serde_json::to_string(&error).unwrap();

        assert_eq!(error.upstream_status, Some(429));
        assert!(error.retryable);
        assert!(!serialized.contains("super-secret-credential"));
        assert!(!serialized.contains("leaked-upstream-body"));
    }

    #[tokio::test]
    async fn upstream_5xx_is_retryable_and_body_is_not_reflected() {
        let (base_url, _captured) =
            mock_server(503, r#"{"error":{"message":"private-provider-detail"}}"#).await;
        let provider = OpenAiChatProvider::new(
            "compatible".to_string(),
            "example-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();
        let error = provider.complete(&request()).await.unwrap_err();
        let serialized = serde_json::to_string(&error).unwrap();

        assert_eq!(error.upstream_status, Some(503));
        assert!(error.retryable);
        assert!(!serialized.contains("private-provider-detail"));
    }

    #[tokio::test]
    async fn provider_timeout_has_a_fixed_safe_error() {
        let (base_url, _captured) = mock_server_with_delay(
            200,
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"late"}]}]}"#,
            Duration::from_millis(100),
        )
        .await;
        let provider = OpenAiResponsesProvider::new_with_timeout(
            "openai".to_string(),
            "example-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
            Duration::from_millis(10),
        )
        .unwrap();
        let error = provider.complete(&request()).await.unwrap_err();

        assert_eq!(error.code, "provider_timeout");
        assert!(error.retryable);
        assert!(!error.to_string().contains("test-secret-value"));
    }

    #[tokio::test]
    async fn dropping_a_timed_out_request_cancels_the_in_flight_future() {
        let (base_url, captured) = mock_server_with_delay(
            200,
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"late"}]}]}"#,
            Duration::from_millis(200),
        )
        .await;
        let provider = OpenAiResponsesProvider::new(
            "openai".to_string(),
            "example-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();

        let result =
            tokio::time::timeout(Duration::from_millis(50), provider.complete(&request())).await;
        assert!(result.is_err());
        assert!(captured.await.unwrap().starts_with("POST /v1/responses"));
    }

    #[tokio::test]
    async fn duplicate_tool_calls_from_upstream_are_rejected() {
        let body = r#"{
            "status":"completed",
            "output":[
                {"type":"function_call","call_id":"call-1","name":"get_current_scenario","arguments":"{}"},
                {"type":"function_call","call_id":"call-1","name":"get_current_scenario","arguments":"{}"}
            ]
        }"#;
        let (base_url, _captured) = mock_server(200, body).await;
        let provider = OpenAiResponsesProvider::new(
            "openai".to_string(),
            "example-model".to_string(),
            base_url,
            "test-secret-value".to_string(),
        )
        .unwrap();

        assert_eq!(
            provider.complete(&request()).await.unwrap_err().code,
            "duplicate_provider_call_id"
        );
    }

    #[test]
    fn malformed_tool_arguments_are_rejected() {
        let body = br#"{
            "status":"completed",
            "output":[{"type":"function_call","call_id":"call-1","name":"tool","arguments":"not-json"}]
        }"#;
        assert_eq!(
            parse_responses_response(body).unwrap_err().code,
            "provider_tool_arguments_invalid"
        );
    }

    #[test]
    fn literal_newlines_and_trailing_commas_in_tool_arguments_are_repaired() {
        let parsed = parse_tool_arguments(
            "{\"candidates\":[{\"patch\":{\"macro_text\":\"/cast 血怒\n/cast 绝刀\",},}],}",
        )
        .unwrap();

        assert_eq!(
            parsed.pointer("/candidates/0/patch/macro_text"),
            Some(&Value::String("/cast 血怒\n/cast 绝刀".to_string()))
        );
    }

    #[test]
    fn empty_wrapped_and_double_encoded_tool_arguments_are_repaired() {
        assert_eq!(parse_tool_arguments("  ").unwrap(), serde_json::json!({}));
        assert_eq!(
            parse_tool_arguments("<decision_summary>next</decision_summary>\n{}").unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            parse_tool_arguments(r#""{\"candidates\":[]}""#).unwrap(),
            serde_json::json!({"candidates": []})
        );
        assert_eq!(
            parse_tool_arguments(r#""{\"candidates\":[],}""#).unwrap(),
            serde_json::json!({"candidates": []})
        );
    }

    #[test]
    fn one_object_argument_array_is_unwrapped_at_the_transport_boundary() {
        let call = parse_tool_call(
            "call-1".to_string(),
            "inspect_timeline_events".to_string(),
            r#"[{"selector":"skill","skill_name":"绝刀"}]"#,
        )
        .unwrap();
        assert_eq!(call.arguments, serde_json::json!({"selector": "skill", "skill_name": "绝刀"}));
    }

    #[test]
    fn python_dict_style_tool_arguments_are_repaired() {
        let parsed = parse_tool_arguments(
            "{'selector': 'skill', 'skill_name': '绝刀', 'buff_names': ['血怒·惊涌'], 'enabled': True}",
        )
        .unwrap();
        assert_eq!(parsed["selector"], "skill");
        assert_eq!(parsed["skill_name"], "绝刀");
        assert_eq!(parsed["enabled"], true);
    }

    #[test]
    fn unquoted_textual_tool_argument_value_is_repaired() {
        let parsed = parse_tool_arguments(
            r#"{"selector":"skill","skill_name":绝刀,"time_seconds":0,"buff_names":["血怒"]}"#,
        )
        .unwrap();
        assert_eq!(parsed["skill_name"], "绝刀");
        assert_eq!(parsed["time_seconds"], 0);
    }
}
