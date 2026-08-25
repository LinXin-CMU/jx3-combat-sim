use super::{
    protocol::{
        FinishReason, ModelMessage, ModelRequest, ModelResponse, ProviderToolCall, TokenUsage,
    },
    LlmProvider, ProviderError,
};
use async_trait::async_trait;
use serde_json::{json, Value};

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

        let last_tool_result = request
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                ModelMessage::ToolResult { output, .. } => Some(output),
                _ => None,
            });
        if request.tools.is_empty() {
            let response = ModelResponse {
                assistant_text: Some(refusal_report()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            };
            response
                .validate_against(request)
                .map_err(|_| ProviderError::invalid_response())?;
            return Ok(response);
        }

        if let Some(output) = last_tool_result {
            if output.get("tool_name").and_then(Value::as_str) == Some("get_current_scenario")
                && request
                    .tools
                    .iter()
                    .any(|tool| tool.name == "simulate_scenario")
            {
                return validated(
                    request,
                    ModelResponse {
                        assistant_text: None,
                        tool_calls: vec![ProviderToolCall {
                            call_id: "fake-call-2".to_string(),
                            name: "simulate_scenario".to_string(),
                            arguments: json!({}),
                        }],
                        finish_reason: FinishReason::ToolCalls,
                        usage: TokenUsage::default(),
                    },
                );
            }

            let report = simulation_report(output).unwrap_or_else(refusal_report);
            return validated(
                request,
                ModelResponse {
                    assistant_text: Some(report),
                    tool_calls: Vec::new(),
                    finish_reason: FinishReason::Stop,
                    usage: TokenUsage::default(),
                },
            );
        }

        let response = ModelResponse {
            assistant_text: None,
            tool_calls: vec![ProviderToolCall {
                call_id: "fake-call-1".to_string(),
                name: request
                    .tools
                    .iter()
                    .find(|tool| tool.name == "get_current_scenario")
                    .unwrap_or(&request.tools[0])
                    .name
                    .clone(),
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

fn validated(
    request: &ModelRequest,
    response: ModelResponse,
) -> Result<ModelResponse, ProviderError> {
    response
        .validate_against(request)
        .map_err(|_| ProviderError::invalid_response())?;
    Ok(response)
}

fn simulation_report(output: &Value) -> Option<String> {
    let evidence =
        output.get("evidence")?.as_array()?.iter().find(|item| {
            item.get("tool_name").and_then(Value::as_str) == Some("simulate_scenario")
        })?;
    let evidence_id = evidence.get("evidence_id")?.as_str()?;
    let dps = evidence.pointer("/result/dps")?.as_f64()?;
    serde_json::to_string(&json!({
        "schema_version": "agent-report-content/v1",
        "summary": "离线基线分析已完成。",
        "findings": [{
            "title": "当前输出基线",
            "explanation": "数值来自确定性模拟器证据。",
            "evidence_ids": [evidence_id],
            "metrics": [{
                "label": "DPS",
                "value": dps,
                "unit": "damage_per_second",
                "evidence_id": evidence_id,
                "json_pointer": "/result/dps"
            }]
        }],
        "recommendations": [],
        "limitations": ["离线供应商只验证基线工具闭环。"],
        "refusal_reason": null
    }))
    .ok()
}

fn refusal_report() -> String {
    serde_json::to_string(&json!({
        "schema_version": "agent-report-content/v1",
        "summary": "离线供应商没有获得可验证证据。",
        "findings": [],
        "recommendations": [],
        "limitations": ["需要先完成只读模拟。"],
        "refusal_reason": "证据不足。"
    }))
    .expect("static fake report must serialize")
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
            response_format: None,
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
