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
const PROVIDER_TIMEOUT_SECS: u64 = 60;

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
                messages.push(json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": calls,
                }));
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
        "tools": tools,
        "tool_choice": "auto",
        "max_tokens": request.max_output_tokens,
    });
    match compatibility {
        ChatCompatibility::Openai => body["parallel_tool_calls"] = json!(false),
        ChatCompatibility::Deepseek => {
            body["thinking"] = json!({"type": "disabled"});
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
    let mut text = choice
        .message
        .content
        .filter(|value| !value.trim().is_empty());
    let refused = choice.message.refusal.is_some();
    if text.is_none() {
        text = choice.message.refusal.filter(|value| !value.is_empty());
    }
    let tool_calls: Vec<ProviderToolCall> = choice
        .message
        .tool_calls
        .into_iter()
        .map(|call| parse_tool_call(call.id, call.function.name, &call.function.arguments))
        .collect::<Result<_, _>>()
        .map_err(|error| error.with_usage(usage.clone()))?;
    if text.is_none() && tool_calls.is_empty() {
        return Err(ProviderError::invalid_response_protocol(
            "provider_response_empty",
            "provider response contained neither text nor tool calls",
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
        tool_calls,
        finish_reason,
        usage,
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
    let arguments: Value = parse_tool_arguments(arguments)?;
    if !arguments.is_object() {
        return Err(ProviderError::invalid_response_protocol(
            "provider_tool_arguments_invalid",
            "provider tool arguments were not a JSON object",
        ));
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
            if let Ok(unwrapped) = serde_json::from_str(inner) {
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
        .ok_or_else(|| ProviderError::invalid_response_protocol(
            "provider_tool_arguments_invalid",
            "provider returned invalid JSON tool arguments",
        ))
}

fn extract_first_json_object(value: &str) -> Option<&str> {
    let start = value.char_indices().find_map(|(index, character)| (character == '{').then_some(index))?;
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
    fn deepseek_chat_mode_disables_thinking_and_uses_supported_json_shape() {
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

        assert_eq!(body["thinking"]["type"], "disabled");
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
            parse_tool_arguments("<decision_summary>next</decision_summary>\n{}")
                .unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            parse_tool_arguments(r#""{\"candidates\":[]}""#).unwrap(),
            serde_json::json!({"candidates": []})
        );
    }
}
