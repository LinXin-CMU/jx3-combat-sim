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
        if let Some(output) = last_tool_result {
            if output.get("tool_name").and_then(Value::as_str) == Some("get_current_scenario") {
                if request
                    .tools
                    .iter()
                    .any(|tool| tool.name == "search_knowledge_base")
                    && should_search_knowledge(request)
                {
                    return validated(
                        request,
                        ModelResponse {
                            assistant_text: None,
                            tool_calls: vec![ProviderToolCall {
                                call_id: "fake-call-knowledge".to_string(),
                                name: "search_knowledge_base".to_string(),
                                arguments: json!({
                                    "query": bounded_user_question(request),
                                    "version_scope": "current_only",
                                    "season": null,
                                    "category": null,
                                    "top_k": 3
                                }),
                            }],
                            finish_reason: FinishReason::ToolCalls,
                            usage: TokenUsage::default(),
                        },
                    );
                }
                if !request
                    .tools
                    .iter()
                    .any(|tool| tool.name == "simulate_scenario")
                {
                    return validated(
                        request,
                        ModelResponse {
                            assistant_text: Some(refusal_report()),
                            tool_calls: Vec::new(),
                            finish_reason: FinishReason::Stop,
                            usage: TokenUsage::default(),
                        },
                    );
                }
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

            if output.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base") {
                return validated(
                    request,
                    ModelResponse {
                        assistant_text: Some(
                            knowledge_report(output).unwrap_or_else(refusal_report),
                        ),
                        tool_calls: Vec::new(),
                        finish_reason: FinishReason::Stop,
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

fn should_search_knowledge(request: &ModelRequest) -> bool {
    let question = bounded_user_question(request);
    ["攻略", "版本资料", "白皮书", "配装", "一键宏", "玩法资料"]
        .iter()
        .any(|keyword| question.contains(keyword))
}

fn bounded_user_question(request: &ModelRequest) -> String {
    request
        .messages
        .iter()
        .find_map(|message| match message {
            ModelMessage::User { content } if !content.starts_with("<session_context") => {
                Some(content.chars().take(160).collect())
            }
            _ => None,
        })
        .unwrap_or_else(|| "当前版本玩法资料".to_string())
}

fn knowledge_report(output: &Value) -> Option<String> {
    let evidence = output.get("evidence")?.as_array()?.iter().find(|item| {
        item.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")
    })?;
    let evidence_id = evidence.get("evidence_id")?.as_str()?;
    let results = evidence.pointer("/result/results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    let has_grounded_body = results.iter().any(|result| {
        result
            .get("fact_eligible")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let (summary, explanation, limitations) = if has_grounded_body {
        (
            "已找到与当前场景版本匹配的玩法资料。",
            "资料正文可核验；具体赛季、可信状态与原始出处见下方来源卡。",
            Vec::<String>::new(),
        )
    } else {
        (
            "已找到当前版本的相关来源入口，但正文尚不足以支持玩法结论。",
            "当前只能确认来源存在，不能复述尚未取得的正文内容。",
            vec!["需要补齐正文后才能形成可核验的玩法说明。".to_string()],
        )
    };
    serde_json::to_string(&json!({
        "schema_version": "agent-report-content/v1",
        "summary": summary,
        "findings": [{
            "title": "当前版本资料检索结果",
            "explanation": explanation,
            "evidence_ids": [evidence_id],
            "metrics": []
        }],
        "recommendations": [],
        "limitations": limitations,
        "refusal_reason": null
    }))
    .ok()
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

    #[tokio::test]
    async fn fake_provider_can_demo_current_version_knowledge_without_network() {
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let tools = vec![
            ToolDefinition {
                name: "search_knowledge_base".to_string(),
                description: "Search current guides.".to_string(),
                parameters: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "simulate_scenario".to_string(),
                description: "Simulate.".to_string(),
                parameters: json!({"type": "object"}),
            },
        ];
        let initial_messages = vec![
            ModelMessage::User {
                content: "结合当前版本攻略说明循环思路。".to_string(),
            },
            ModelMessage::Assistant {
                content: None,
                tool_calls: vec![ProviderToolCall {
                    call_id: "prefetch".to_string(),
                    name: "get_current_scenario".to_string(),
                    arguments: json!({}),
                }],
            },
            ModelMessage::ToolResult {
                call_id: "prefetch".to_string(),
                output: json!({"tool_name": "get_current_scenario"}),
            },
        ];
        let first_request = ModelRequest {
            instructions: "Use tools.".to_string(),
            messages: initial_messages.clone(),
            tools: tools.clone(),
            response_format: None,
            max_output_tokens: 512,
        };
        let first = provider.complete(&first_request).await.unwrap();
        assert_eq!(first.tool_calls[0].name, "search_knowledge_base");
        assert_eq!(
            first.tool_calls[0].arguments["version_scope"],
            "current_only"
        );

        let evidence_id = "a".repeat(64);
        let mut final_messages = initial_messages;
        final_messages.push(ModelMessage::Assistant {
            content: None,
            tool_calls: first.tool_calls,
        });
        final_messages.push(ModelMessage::ToolResult {
            call_id: "fake-call-knowledge".to_string(),
            output: json!({
                "tool_name": "search_knowledge_base",
                "evidence": [{
                    "tool_name": "search_knowledge_base",
                    "evidence_id": evidence_id,
                    "result": {"results": [{"fact_eligible": true}]}
                }]
            }),
        });
        let final_request = ModelRequest {
            instructions: "Use tools.".to_string(),
            messages: final_messages,
            tools,
            response_format: None,
            max_output_tokens: 512,
        };
        let final_response = provider.complete(&final_request).await.unwrap();
        assert!(final_response.tool_calls.is_empty());
        let report: Value =
            serde_json::from_str(final_response.assistant_text.as_deref().unwrap()).unwrap();
        assert_eq!(report["findings"][0]["evidence_ids"][0], "a".repeat(64));
        assert!(report["summary"].as_str().unwrap().contains("当前场景版本"));
    }
}
