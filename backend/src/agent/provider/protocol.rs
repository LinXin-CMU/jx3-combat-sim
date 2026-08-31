use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const PROVIDER_PROTOCOL_V1: &str = "agent-provider-protocol/v1";
pub const MAX_INSTRUCTIONS_BYTES: usize = 32 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_TOOLS: usize = 16;
pub const MAX_PROVIDER_TOOL_CALLS: usize = 8;
pub const MAX_OUTPUT_TOKENS: u32 = 8192;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelMessage {
    User {
        content: String,
    },
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ProviderToolCall>,
    },
    ToolResult {
        call_id: String,
        output: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuredOutputDefinition {
    pub name: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    pub instructions: String,
    pub messages: Vec<ModelMessage>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<StructuredOutputDefinition>,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Refusal,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_text: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ProviderToolCall>,
    pub finish_reason: FinishReason,
    #[serde(default)]
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: &'static str,
    pub message: &'static str,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProtocolError {}

impl ModelRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.instructions.trim().is_empty() || self.instructions.len() > MAX_INSTRUCTIONS_BYTES {
            return Err(protocol_error(
                "invalid_instructions",
                "instructions must be 1..32768 bytes",
            ));
        }
        if self.messages.is_empty() {
            return Err(protocol_error(
                "missing_messages",
                "at least one model message is required",
            ));
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > MAX_OUTPUT_TOKENS {
            return Err(protocol_error(
                "invalid_output_budget",
                "max_output_tokens must be 1..8192",
            ));
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(protocol_error(
                "too_many_tools",
                "no more than 16 tools may be exposed",
            ));
        }
        if let Some(format) = &self.response_format {
            if !valid_identifier(&format.name) || !format.schema.is_object() {
                return Err(protocol_error(
                    "invalid_response_schema",
                    "structured output requires a valid name and object schema",
                ));
            }
        }

        let mut tool_names = HashSet::new();
        for tool in &self.tools {
            if !valid_identifier(&tool.name) {
                return Err(protocol_error(
                    "invalid_tool_name",
                    "tool names must match [A-Za-z0-9_-] and be 1..64 bytes",
                ));
            }
            if !tool_names.insert(tool.name.as_str()) {
                return Err(protocol_error(
                    "duplicate_tool_name",
                    "tool names must be unique",
                ));
            }
            if tool.description.trim().is_empty() || tool.description.len() > 1024 {
                return Err(protocol_error(
                    "invalid_tool_description",
                    "tool descriptions must be 1..1024 bytes",
                ));
            }
            if !tool.parameters.is_object() {
                return Err(protocol_error(
                    "invalid_tool_schema",
                    "tool parameters must be a JSON object schema",
                ));
            }
        }

        let mut known_calls = HashSet::new();
        for message in &self.messages {
            match message {
                ModelMessage::User { content } => validate_text(content)?,
                ModelMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    if content.as_deref().is_none_or(str::is_empty) && tool_calls.is_empty() {
                        return Err(protocol_error(
                            "empty_assistant_message",
                            "assistant messages require text or tool calls",
                        ));
                    }
                    if let Some(content) = content {
                        validate_text(content)?;
                    }
                    for call in tool_calls {
                        validate_call(call)?;
                        if !known_calls.insert(call.call_id.as_str()) {
                            return Err(protocol_error(
                                "duplicate_call_id",
                                "tool call ids must be unique",
                            ));
                        }
                    }
                }
                ModelMessage::ToolResult { call_id, output } => {
                    if !valid_identifier(call_id) || !known_calls.contains(call_id.as_str()) {
                        return Err(protocol_error(
                            "orphan_tool_result",
                            "tool results must reference an earlier tool call",
                        ));
                    }
                    let size = serde_json::to_vec(output)
                        .map(|value| value.len())
                        .unwrap_or(usize::MAX);
                    if size > MAX_TOOL_OUTPUT_BYTES {
                        return Err(protocol_error(
                            "tool_output_too_large",
                            "tool output exceeds 262144 bytes",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl ModelResponse {
    /// Validate provider-controlled response structure without applying the
    /// request-specific tool allow-list. Adapters use this check so the
    /// orchestrator can recover from a model selecting an unavailable tool
    /// instead of losing the whole grounded run at the transport boundary.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self
            .assistant_text
            .as_deref()
            .is_some_and(|content| validate_text(content).is_err())
        {
            return Err(protocol_error(
                "invalid_assistant_response",
                "assistant response text is invalid",
            ));
        }
        if self.assistant_text.is_none() && self.tool_calls.is_empty() {
            return Err(protocol_error(
                "empty_model_response",
                "model response requires text or tool calls",
            ));
        }
        if self.tool_calls.len() > MAX_PROVIDER_TOOL_CALLS {
            return Err(protocol_error(
                "too_many_provider_tool_calls",
                "provider returned more than 8 tool calls",
            ));
        }

        let mut call_ids = HashSet::new();
        for call in &self.tool_calls {
            validate_call(call)?;
            if !call_ids.insert(call.call_id.as_str()) {
                return Err(protocol_error(
                    "duplicate_provider_call_id",
                    "provider returned duplicate tool call ids",
                ));
            }
        }
        Ok(())
    }

    pub fn validate_against(&self, request: &ModelRequest) -> Result<(), ProtocolError> {
        self.validate()?;
        let allowed_tools: HashSet<&str> = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        if self
            .tool_calls
            .iter()
            .any(|call| !allowed_tools.contains(call.name.as_str()))
        {
            return Err(protocol_error(
                "unregistered_provider_tool",
                "provider returned a tool that was not exposed",
            ));
        }
        Ok(())
    }
}

pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn validate_call(call: &ProviderToolCall) -> Result<(), ProtocolError> {
    if !valid_identifier(&call.call_id) || !valid_identifier(&call.name) {
        return Err(protocol_error(
            "invalid_tool_call",
            "tool call ids and names must match [A-Za-z0-9_-] and be 1..64 bytes",
        ));
    }
    if !call.arguments.is_object() {
        return Err(protocol_error(
            "invalid_tool_arguments",
            "tool call arguments must be a JSON object",
        ));
    }
    Ok(())
}

fn validate_text(content: &str) -> Result<(), ProtocolError> {
    if content.trim().is_empty() || content.len() > MAX_MESSAGE_BYTES {
        Err(protocol_error(
            "invalid_message",
            "message text must be 1..65536 bytes",
        ))
    } else {
        Ok(())
    }
}

fn protocol_error(code: &'static str, message: &'static str) -> ProtocolError {
    ProtocolError { code, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ModelRequest {
        ModelRequest {
            instructions: "Use deterministic evidence.".to_string(),
            messages: vec![ModelMessage::User {
                content: "Compare two scenarios.".to_string(),
            }],
            tools: vec![ToolDefinition {
                name: "compare_scenarios".to_string(),
                description: "Run a typed A/B comparison.".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            response_format: None,
            max_output_tokens: 1024,
        }
    }

    #[test]
    fn valid_request_passes() {
        request().validate().unwrap();
    }

    #[test]
    fn duplicate_tools_and_orphan_results_are_rejected() {
        let mut duplicate = request();
        duplicate.tools.push(duplicate.tools[0].clone());
        assert_eq!(
            duplicate.validate().unwrap_err().code,
            "duplicate_tool_name"
        );

        let mut orphan = request();
        orphan.messages.push(ModelMessage::ToolResult {
            call_id: "call-1".to_string(),
            output: serde_json::json!({"ok": true}),
        });
        assert_eq!(orphan.validate().unwrap_err().code, "orphan_tool_result");
    }

    #[test]
    fn response_must_only_call_registered_tools_with_unique_ids() {
        let request = request();
        let unknown = ModelResponse {
            assistant_text: None,
            tool_calls: vec![ProviderToolCall {
                call_id: "call-1".to_string(),
                name: "write_files".to_string(),
                arguments: serde_json::json!({}),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: TokenUsage::default(),
        };
        assert_eq!(
            unknown.validate_against(&request).unwrap_err().code,
            "unregistered_provider_tool"
        );

        let call = ProviderToolCall {
            call_id: "call-1".to_string(),
            name: "compare_scenarios".to_string(),
            arguments: serde_json::json!({}),
        };
        let duplicate = ModelResponse {
            assistant_text: None,
            tool_calls: vec![call.clone(), call],
            finish_reason: FinishReason::ToolCalls,
            usage: TokenUsage::default(),
        };
        assert_eq!(
            duplicate.validate_against(&request).unwrap_err().code,
            "duplicate_provider_call_id"
        );
    }
}
