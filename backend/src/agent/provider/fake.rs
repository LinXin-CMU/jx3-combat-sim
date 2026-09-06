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
                ModelMessage::ToolResult { output, .. } => Some(output.clone()),
                _ => None,
            })
            .or_else(|| latest_handoff_tool_result(request));
        if let Some(output) = last_tool_result.as_ref() {
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
                            reasoning_content: None,
                            tool_calls: vec![ProviderToolCall {
                                call_id: "fake-call-knowledge".to_string(),
                                name: "search_knowledge_base".to_string(),
                                arguments: json!({
                                    "query": bounded_user_question(request),
                                    "version_scope": "current_only",
                                    "season": null,
                                    "category": null
                                }),
                            }],
                            finish_reason: FinishReason::ToolCalls,
                            usage: TokenUsage::default(),
                        },
                    );
                }
                let Some(baseline_tool) = baseline_tool_name(request) else {
                    return validated(
                        request,
                        ModelResponse {
                            assistant_text: Some(refusal_report()),
                            reasoning_content: None,
                            tool_calls: Vec::new(),
                            finish_reason: FinishReason::Stop,
                            usage: TokenUsage::default(),
                        },
                    );
                };
                return validated(
                    request,
                    ModelResponse {
                        assistant_text: Some(
                            "先取得当前循环的基线诊断，再判断是否需要候选实验。".to_string(),
                        ),
                        reasoning_content: None,
                        tool_calls: vec![ProviderToolCall {
                            call_id: "fake-call-2".to_string(),
                            name: baseline_tool.to_string(),
                            arguments: json!({}),
                        }],
                        finish_reason: FinishReason::ToolCalls,
                        usage: TokenUsage::default(),
                    },
                );
            }

            if output.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base") {
                // Domain-aware runs may receive a server-owned knowledge prefetch before the
                // provider's first turn.  A combat-baseline question still needs simulator
                // evidence; do not mistake the prefetched guide for a complete answer.
                let diagnostic_available = request
                    .tools
                    .iter()
                    .any(|tool| tool.name == "analyze_timeline");
                if let Some(baseline_tool) = baseline_tool_name(request)
                    .filter(|_| diagnostic_available || !should_search_knowledge(request))
                {
                    return validated(
                        request,
                        ModelResponse {
                            assistant_text: Some(
                                "攻略证据已取得；下一步运行基线时间轴，区分已确认优点与观察到的风险。"
                                    .to_string(),
                            ),
                            reasoning_content: None,
                            tool_calls: vec![ProviderToolCall {
                                call_id: "fake-call-2".to_string(),
                                name: baseline_tool.to_string(),
                                arguments: json!({}),
                            }],
                            finish_reason: FinishReason::ToolCalls,
                            usage: TokenUsage::default(),
                        },
                    );
                }
                return validated(
                    request,
                    ModelResponse {
                        assistant_text: Some(
                            knowledge_report(output).unwrap_or_else(refusal_report),
                        ),
                        reasoning_content: None,
                        tool_calls: Vec::new(),
                        finish_reason: FinishReason::Stop,
                        usage: TokenUsage::default(),
                    },
                );
            }

            let report = simulation_report(request, output).unwrap_or_else(refusal_report);
            return validated(
                request,
                ModelResponse {
                    assistant_text: Some(report),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    finish_reason: FinishReason::Stop,
                    usage: TokenUsage::default(),
                },
            );
        }

        if request.tools.is_empty() {
            let response = ModelResponse {
                assistant_text: Some(refusal_report()),
                reasoning_content: None,
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
            reasoning_content: None,
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

fn latest_handoff_tool_result(request: &ModelRequest) -> Option<Value> {
    request.messages.iter().rev().find_map(|message| {
        let ModelMessage::User { content } = message else {
            return None;
        };
        let body = content
            .strip_prefix(
                "<model_evidence server_generated=\"true\" schema=\"agent-model-evidence/v1\">\n",
            )?
            .strip_suffix("\n</model_evidence>")?;
        let payload: Value = serde_json::from_str(body).ok()?;
        let items = payload.get("items")?.as_array()?.clone();
        // Handoff items are ordered by relevance and capability coverage, not
        // execution chronology. Determine the fixture stage from completed
        // evidence so a trailing scenario or event page cannot restart it.
        let tool_name = if items.iter().any(|item| {
            item.get("tool_name").and_then(Value::as_str) == Some("simulate_scenario")
                && item.pointer("/result/dps").and_then(Value::as_f64).is_some()
        }) {
            "simulate_scenario"
        } else if items.iter().any(|item| item.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")) {
            "search_knowledge_base"
        } else if items.iter().any(|item| item.get("tool_name").and_then(Value::as_str) == Some("get_current_scenario")) {
            "get_current_scenario"
        } else {
            items.first()?.get("tool_name")?.as_str()?
        };
        Some(json!({
            "tool_name": tool_name,
            "evidence": items,
        }))
    })
}

fn baseline_tool_name(request: &ModelRequest) -> Option<&str> {
    request
        .tools
        .iter()
        .find(|tool| tool.name == "analyze_timeline")
        .or_else(|| {
            request
                .tools
                .iter()
                .find(|tool| tool.name == "simulate_scenario")
        })
        .map(|tool| tool.name.as_str())
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
            ModelMessage::User { content } if !content.starts_with('<') => {
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

fn simulation_report(request: &ModelRequest, output: &Value) -> Option<String> {
    let evidence = output.get("evidence")?.as_array()?;
    // A timeline dispatch registers both its diagnostic evidence and the
    // underlying simulation evidence.  Select by capability instead of list
    // order: only the simulation projection owns the top-level DPS field.
    let simulation = evidence.iter().find(|item| {
        item.get("tool_name").and_then(Value::as_str) == Some("simulate_scenario")
            && item.pointer("/result/dps").and_then(Value::as_f64).is_some()
    })?;
    let simulation_id = simulation.get("evidence_id")?.as_str()?;
    let dps = simulation.pointer("/result/dps")?.as_f64()?;
    let timeline = evidence
        .iter()
        .find(|item| item.get("tool_name").and_then(Value::as_str) == Some("analyze_timeline"));
    let handoff = latest_handoff_tool_result(request);
    let knowledge = request
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::ToolResult { output, .. } => output.get("evidence")?.as_array(),
            _ => None,
        })
        .flatten()
        .find(|item| {
            item.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")
                && item
                    .pointer("/result/results/0/fact_eligible")
                    .and_then(Value::as_bool)
                    == Some(true)
        })
        .or_else(|| {
            handoff
                .as_ref()
                .and_then(|value| value.get("evidence"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|item| {
                    item.get("tool_name").and_then(Value::as_str)
                        == Some("search_knowledge_base")
                        && item
                            .pointer("/result/results/0/fact_eligible")
                            .and_then(Value::as_bool)
                            == Some(true)
                })
        });
    let mut findings = vec![json!({
        "title": "当前输出基线",
        "explanation": "数值来自确定性模拟器证据。",
        "evidence_ids": [simulation_id],
        "metrics": [{
            "label": "DPS",
            "value": dps,
            "unit": "damage_per_second",
            "evidence_id": simulation_id,
            "json_pointer": "/result/dps"
        }]
    })];
    if let Some(timeline) = timeline {
        let timeline_id = timeline.get("evidence_id")?.as_str()?;
        findings.push(json!({
            "title": "循环诊断已建立",
            "explanation": "已生成循环输入类型、节奏、冷却、姿态、资源与操作跳过的基线诊断；现象不直接等同于因果。",
            "evidence_ids": [timeline_id],
            "metrics": []
        }));
    }
    if let Some(knowledge) = knowledge {
        let knowledge_id = knowledge.get("evidence_id")?.as_str()?;
        findings.push(json!({
            "title": "当前版本攻略依据",
            "explanation": "已取得与当前版本匹配且可用于事实判断的攻略正文。",
            "evidence_ids": [knowledge_id],
            "metrics": []
        }));
    }
    serde_json::to_string(&json!({
        "schema_version": "agent-report-content/v1",
        "summary": "离线循环基线与诊断已完成。",
        "findings": findings,
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
                    reasoning_content: None,
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
                reasoning_content: None,
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
            reasoning_content: None,
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

    #[tokio::test]
    async fn server_prefetched_knowledge_does_not_replace_required_simulation() {
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let request = ModelRequest {
            instructions: "Use tools.".to_string(),
            messages: vec![
                ModelMessage::User {
                    content: "分析当前循环的确定性输出基线，并说明证据边界。".to_string(),
                },
                ModelMessage::Assistant {
                    content: None,
                    tool_calls: vec![ProviderToolCall {
                        call_id: "server-prefetch-knowledge".to_string(),
                        name: "search_knowledge_base".to_string(),
                        arguments: json!({}),
                    }],
                    reasoning_content: None,
                },
                ModelMessage::ToolResult {
                    call_id: "server-prefetch-knowledge".to_string(),
                    output: json!({
                        "tool_name": "search_knowledge_base",
                        "evidence": []
                    }),
                },
            ],
            tools: vec![
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
            ],
            response_format: None,
            max_output_tokens: 512,
        };

        let response = provider.complete(&request).await.unwrap();
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "simulate_scenario");
    }

    #[test]
    fn compact_handoff_uses_simulation_evidence_even_when_timeline_precedes_it() {
        let simulation_id = "b".repeat(64);
        let timeline_id = "c".repeat(64);
        let payload = json!({
            "items": [
                {
                    "tool_name": "analyze_timeline",
                    "evidence_id": timeline_id,
                    "result": {"diagnostic_profile": {}}
                },
                {
                    "tool_name": "simulate_scenario",
                    "evidence_id": simulation_id,
                    "result": {"dps": 1234.5}
                }
            ]
        });
        let request = ModelRequest {
            instructions: "Use evidence.".to_string(),
            messages: vec![ModelMessage::User {
                content: format!(
                    "<model_evidence server_generated=\"true\" schema=\"agent-model-evidence/v1\">\n{}\n</model_evidence>",
                    serde_json::to_string(&payload).unwrap()
                ),
            }],
            tools: Vec::new(),
            response_format: None,
            max_output_tokens: 512,
        };

        let handoff = latest_handoff_tool_result(&request).expect("compacted evidence");
        let report = simulation_report(&request, &handoff).expect("simulation report");
        let report: Value = serde_json::from_str(&report).unwrap();
        assert_eq!(report["findings"][0]["evidence_ids"][0], "b".repeat(64));
        assert_eq!(report["findings"][0]["metrics"][0]["value"], 1234.5);
        assert_eq!(report["findings"][1]["evidence_ids"][0], "c".repeat(64));
    }

    #[tokio::test]
    async fn compact_handoff_completes_regardless_of_evidence_order_or_event_table() {
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let mut items = vec![
            json!({"tool_name":"simulate_scenario", "evidence_id":"a".repeat(64), "result":{"dps":1234.5}}),
            json!({"tool_name":"analyze_timeline", "evidence_id":"b".repeat(64), "result":{"diagnostic_profile":{}}}),
            json!({"tool_name":"inspect_timeline_events", "evidence_id":"c".repeat(64), "result":{"match_index_table":{"source_pointer":"/result/match_index", "columns":["/event_number"], "constants":{}, "source_indices":[0], "rows":[[12]]}}}),
            json!({"tool_name":"get_current_scenario", "evidence_id":"d".repeat(64), "result":{"rotation_input":{"mode":"manual"}}}),
        ];
        for _ in 0..items.len() {
            let mut request = request(vec![
                ModelMessage::User { content:"分析当前循环。".to_string() },
                ModelMessage::User { content:format!("<model_evidence server_generated=\"true\" schema=\"agent-model-evidence/v1\">\n{}\n</model_evidence>", json!({"items":items})) },
            ]);
            request.tools.push(ToolDefinition {
                name:"analyze_timeline".to_string(), description:"Run a baseline diagnosis.".to_string(), parameters:json!({"type":"object"}),
            });
            let response = provider.complete(&request).await.unwrap();
            assert!(response.tool_calls.is_empty());
            assert_eq!(response.finish_reason, FinishReason::Stop);
            let report: Value = serde_json::from_str(response.assistant_text.as_deref().unwrap()).unwrap();
            assert!(report["refusal_reason"].is_null());
            assert_eq!(report["findings"][0]["metrics"][0]["value"], 1234.5);
            assert_eq!(report["findings"][1]["evidence_ids"][0], "b".repeat(64));
            items.rotate_left(1);
        }
    }
}
