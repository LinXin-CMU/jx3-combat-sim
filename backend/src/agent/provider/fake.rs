use super::{
    protocol::{
        FinishReason, ModelMessage, ModelRequest, ModelResponse, ProviderToolCall, TokenUsage,
    },
    LlmProvider, ProviderError,
};
use async_trait::async_trait;

pub struct FakeProvider {
    profile_id: String,
    model: String,
}

impl FakeProvider {
    pub fn new(profile_id: String, model: String) -> Self {
        Self { profile_id, model }
    }
}

#[async_trait]
impl LlmProvider for FakeProvider {
    fn profile_id(&self) -> &str {
        &self.profile_id
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        request
            .validate()
            .map_err(|_| ProviderError::invalid_request())?;

        let has_tool_result = request
            .messages
            .iter()
            .any(|message| matches!(message, ModelMessage::ToolResult { .. }));
        if has_tool_result || request.tools.is_empty() {
            let response = ModelResponse {
                assistant_text: Some("离线 provider 已完成确定性响应。".to_string()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            };
            response
                .validate_against(request)
                .map_err(|_| ProviderError::invalid_response())?;
            return Ok(response);
        }

        let response = ModelResponse {
            assistant_text: None,
            tool_calls: vec![ProviderToolCall {
                call_id: "fake-call-1".to_string(),
                name: request.tools[0].name.clone(),
                arguments: serde_json::json!({}),
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: TokenUsage::default(),
        };
        response
            .validate_against(request)
            .map_err(|_| ProviderError::invalid_response())?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::{ModelMessage, ToolDefinition};

    fn request(messages: Vec<ModelMessage>) -> ModelRequest {
        ModelRequest {
            instructions: "Use tools.".to_string(),
            messages,
            tools: vec![ToolDefinition {
                name: "get_current_scenario".to_string(),
                description: "Read the immutable scenario.".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_output_tokens: 128,
        }
    }

    #[tokio::test]
    async fn fake_provider_is_deterministic_across_tool_round_trip() {
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let first = provider
            .complete(&request(vec![ModelMessage::User {
                content: "Inspect this scenario.".to_string(),
            }]))
            .await
            .unwrap();
        assert_eq!(first.finish_reason, FinishReason::ToolCalls);
        assert_eq!(first.tool_calls[0].call_id, "fake-call-1");

        let second = provider
            .complete(&request(vec![
                ModelMessage::User {
                    content: "Inspect this scenario.".to_string(),
                },
                ModelMessage::Assistant {
                    content: None,
                    tool_calls: first.tool_calls,
                },
                ModelMessage::ToolResult {
                    call_id: "fake-call-1".to_string(),
                    output: serde_json::json!({"scenario_hash": "fixture"}),
                },
            ]))
            .await
            .unwrap();
        assert_eq!(second.finish_reason, FinishReason::Stop);
        assert!(second.tool_calls.is_empty());
    }
}
