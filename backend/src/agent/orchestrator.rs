use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[cfg(test)]
use super::domain::select_analysis_plan;
use super::domain::{
    build_evidence_pack, evidence_pack_model_context, knowledge_prefetch, plan_model_context,
    select_analysis_plan_with_history, trace_annotation, AnalysisPlanV1,
};
use super::evidence::validate_trace_id;
use super::prompt::agent_prompt_v17;
use super::provider::{
    FinishReason, LlmProvider, ModelMessage, ModelRequest, ProviderToolCall,
    StructuredOutputDefinition, TokenUsage,
};
use super::registry::{normalize_reference_query, AgentToolRegistry, MAX_KNOWLEDGE_SEARCHES};
use super::report::{
    cited_evidence_ids, cited_knowledge_sources, parse_and_salvage_report,
    parse_and_validate_report, report_content_json_schema, AgentFindingV1, AgentReportContentV1,
    AgentReportV1, AgentRunAccountingV1, EvidenceStore, AGENT_REPORT_CONTENT_SCHEMA_V1,
    AGENT_REPORT_SCHEMA_V1,
};
use super::{AgentRuntime, ScenarioSnapshotV1};

pub const AGENT_RUN_SCHEMA_V1: &str = "agent-run/v1";
const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_SESSION_CONTEXT_BYTES: usize = 16 * 1024;
const MAX_REPORT_REPAIRS: u32 = 1;
const MAX_EMPTY_RESPONSE_RETRIES: u32 = 1;
const MAX_TOOL_SELECTION_RETRIES: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
    PartiallyVerified,
    Refused,
    EvidenceInsufficient,
    Cancelled,
    BudgetExhausted,
    ProviderFailed,
    ProtocolFailed,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunLimits {
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_simulations: u32,
    pub max_output_tokens_per_turn: u32,
    pub wall_time_ms: u64,
}

impl Default for AgentRunLimits {
    fn default() -> Self {
        Self {
            max_model_turns: 6,
            max_tool_calls: 8,
            max_simulations: 8,
            max_output_tokens_per_turn: 2048,
            wall_time_ms: 120_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentRunInput {
    pub run_id: String,
    pub question: String,
    pub scenario: ScenarioSnapshotV1,
    /// Bounded, user-visible history reconstructed by the trusted session store.
    /// It is data, not an instruction, and never contains provider transcripts.
    pub session_context: Option<String>,
    /// The last server-selected playbook. This is trusted routing state, not model prose.
    pub session_playbook_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunErrorV1 {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentTraceEventV1 {
    pub sequence: u32,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playbook_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunResultV1 {
    pub schema_version: String,
    pub run_id: String,
    pub scenario_hash: String,
    pub prompt_version: String,
    pub prompt_sha256: String,
    pub provider_profile: String,
    pub model: String,
    pub status: AgentRunStatus,
    pub accounting: AgentRunAccountingV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<AgentReportV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentRunErrorV1>,
    pub trace: Vec<AgentTraceEventV1>,
}

pub type AgentTraceSink = Arc<dyn Fn(AgentTraceEventV1) + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentReplayEventV1 {
    pub kind: String,
    pub payload: serde_json::Value,
}

pub type AgentReplaySink = Arc<dyn Fn(AgentReplayEventV1) + Send + Sync>;

fn record_replay(sink: &Option<AgentReplaySink>, kind: &str, payload: serde_json::Value) {
    if let Some(sink) = sink {
        sink(AgentReplayEventV1 {
            kind: kind.to_string(),
            payload,
        });
    }
}

#[derive(Debug)]
struct AgentCancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Debug, Clone)]
pub struct AgentCancellation {
    inner: Arc<AgentCancellationInner>,
}

impl Default for AgentCancellation {
    fn default() -> Self {
        Self {
            inner: Arc::new(AgentCancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }
}

impl AgentCancellation {
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

struct TraceCollector {
    events: Vec<AgentTraceEventV1>,
    sink: Option<AgentTraceSink>,
    plan: AnalysisPlanV1,
}

impl TraceCollector {
    fn new(sink: Option<AgentTraceSink>, plan: AnalysisPlanV1) -> Self {
        Self {
            events: Vec::new(),
            sink,
            plan,
        }
    }

    fn push(
        &mut self,
        kind: &str,
        tool_name: Option<String>,
        evidence_ids: Vec<String>,
        code: Option<String>,
    ) {
        let annotation =
            trace_annotation(&self.plan, kind, tool_name.as_deref(), evidence_ids.len());
        let event = AgentTraceEventV1 {
            sequence: self.events.len() as u32 + 1,
            kind: kind.to_string(),
            tool_name,
            evidence_ids,
            code,
            stage_id: Some(annotation.stage_id),
            label: Some(annotation.label),
            overview: Some(annotation.overview),
            playbook_id: Some(self.plan.playbook.playbook_id.clone()),
        };
        if let Some(sink) = &self.sink {
            sink(event.clone());
        }
        self.events.push(event);
    }

    fn push_decision_summary(&mut self, summary: String) {
        let annotation = trace_annotation(&self.plan, "decision_checkpoint", None, 0);
        let event = AgentTraceEventV1 {
            sequence: self.events.len() as u32 + 1,
            kind: "decision_checkpoint".to_string(),
            tool_name: None,
            evidence_ids: Vec::new(),
            code: Some("public_decision_summary".to_string()),
            stage_id: Some(annotation.stage_id),
            label: Some("记录决策依据".to_string()),
            overview: Some(summary),
            playbook_id: Some(self.plan.playbook.playbook_id.clone()),
        };
        if let Some(sink) = &self.sink {
            sink(event.clone());
        }
        self.events.push(event);
    }

    fn into_events(self) -> Vec<AgentTraceEventV1> {
        self.events
    }
}

pub async fn run_agent(
    provider: &dyn LlmProvider,
    runtime: &AgentRuntime,
    input: AgentRunInput,
    limits: AgentRunLimits,
    cancellation: AgentCancellation,
) -> AgentRunResultV1 {
    run_agent_observed(provider, runtime, input, limits, cancellation, None).await
}

pub async fn run_agent_observed(
    provider: &dyn LlmProvider,
    runtime: &AgentRuntime,
    input: AgentRunInput,
    limits: AgentRunLimits,
    cancellation: AgentCancellation,
    event_sink: Option<AgentTraceSink>,
) -> AgentRunResultV1 {
    run_agent_recorded(
        provider,
        runtime,
        input,
        limits,
        cancellation,
        event_sink,
        None,
    )
    .await
}

pub async fn run_agent_recorded(
    provider: &dyn LlmProvider,
    runtime: &AgentRuntime,
    input: AgentRunInput,
    limits: AgentRunLimits,
    cancellation: AgentCancellation,
    event_sink: Option<AgentTraceSink>,
    replay_sink: Option<AgentReplaySink>,
) -> AgentRunResultV1 {
    let started = Instant::now();
    let prompt = agent_prompt_v17();
    let analysis_plan = select_analysis_plan_with_history(
        &input.question,
        input.session_playbook_id.as_deref(),
        &input.scenario,
    );
    let mut accounting = AgentRunAccountingV1::default();
    let mut trace = TraceCollector::new(event_sink, analysis_plan.clone());
    record_replay(
        &replay_sink,
        "run_input",
        serde_json::json!({
            "question": &input.question,
            "scenario": &input.scenario,
            "session_context": &input.session_context,
            "session_playbook_id": &input.session_playbook_id,
            "provider_profile": provider.profile_id(),
            "model": provider.model(),
            "limits": &limits,
            "prompt_version": prompt.version,
            "prompt_sha256": &prompt.sha256,
            "prompt_instructions": prompt.instructions,
        }),
    );
    record_replay(
        &replay_sink,
        "analysis_plan",
        serde_json::to_value(&analysis_plan).unwrap_or_else(|_| serde_json::json!({})),
    );

    if validate_input(&input, &limits).is_err() {
        return terminal(
            provider,
            &input,
            &prompt,
            AgentRunStatus::ProtocolFailed,
            accounting,
            None,
            Some(fixed_error(
                "invalid_run_input",
                "Agent run input is invalid",
            )),
            trace,
            started,
        );
    }

    let mut registry = AgentToolRegistry::new_with_knowledge(
        &input.scenario,
        runtime,
        limits.max_simulations,
        runtime.knowledge(),
    );
    registry.set_knowledge_question(&input.question);
    let definitions = if let Some(knowledge) = runtime.knowledge() {
        let seasons = knowledge.seasons().map(str::to_string).collect::<Vec<_>>();
        let categories = knowledge
            .categories()
            .map(str::to_string)
            .collect::<Vec<_>>();
        AgentToolRegistry::definitions_with_knowledge(&seasons, &categories)
    } else {
        AgentToolRegistry::definitions()
    };
    let knowledge_only_client_scope = requires_knowledge_only_client_scope(&input.question);
    let mut tools = definitions
        .into_iter()
        .filter(|tool| {
            !knowledge_only_client_scope
                || matches!(
                    tool.name.as_str(),
                    "get_current_scenario" | "search_knowledge_base"
                )
        })
        .filter(|tool| {
            tool.name == "get_current_scenario"
                || analysis_plan
                    .playbook
                    .preferred_tools
                    .iter()
                    .any(|preferred| preferred == &tool.name)
        })
        .collect::<Vec<_>>();
    tools.sort_by_key(|tool| {
        analysis_plan
            .playbook
            .preferred_tools
            .iter()
            .position(|preferred| preferred == &tool.name)
            .unwrap_or(usize::MAX)
    });
    let mut messages = Vec::new();
    if let Some(context) = &input.session_context {
        messages.push(ModelMessage::User {
            content: format!(
                "<session_context untrusted_data=\"true\">\n{context}\n</session_context>"
            ),
        });
    }
    messages.push(ModelMessage::User {
        content: input.question.clone(),
    });
    messages.push(ModelMessage::User {
        content: plan_model_context(&analysis_plan),
    });
    let mut repairs = 0;
    let mut empty_response_retries = 0;
    let mut tool_selection_retries = 0;
    let mut diagnosis_gap_reminders = 0_u8;
    let mut evidence_gap_reminders = 0_u8;
    let mut repair_message = None;
    let mut final_report_only = false;
    let adaptive_experiments = analysis_plan
        .playbook
        .preferred_tools
        .iter()
        .any(|tool| tool == "analyze_timeline")
        && analysis_plan
            .playbook
            .preferred_tools
            .iter()
            .any(|tool| tool == "compare_scenarios");
    let mut domain_experiment_completed = false;
    trace.push("planning", None, Vec::new(), None);
    trace.push(
        "analysis_plan_selected",
        None,
        Vec::new(),
        Some(analysis_plan.playbook.playbook_id.clone()),
    );
    if knowledge_only_client_scope {
        trace.push(
            "knowledge_only_client_scope",
            None,
            Vec::new(),
            Some("wujie_simulation_not_implemented".to_string()),
        );
    }

    if cancellation.is_cancelled() {
        return terminal_with_registry(
            provider,
            &input,
            &prompt,
            AgentRunStatus::Cancelled,
            accounting,
            None,
            Some(fixed_error("run_cancelled", "Agent run was cancelled")),
            trace,
            started,
            &registry,
        );
    }

    const PREFETCH_CALL_ID: &str = "server-prefetch-scenario";
    const PREFETCH_TOOL: &str = "get_current_scenario";
    trace.push(
        "tool_started",
        Some(PREFETCH_TOOL.to_string()),
        Vec::new(),
        Some("server_prefetch".to_string()),
    );
    let prefetched = registry.dispatch(&input.run_id, PREFETCH_TOOL, serde_json::json!({}));
    record_replay(
        &replay_sink,
        "tool_dispatch",
        serde_json::json!({
            "call_id": PREFETCH_CALL_ID,
            "tool_name": PREFETCH_TOOL,
            "arguments": {},
            "output": &prefetched.output,
            "evidence_ids": &prefetched.evidence_ids,
            "budget_exhausted": prefetched.budget_exhausted,
            "server_initiated": true,
        }),
    );
    accounting.tool_calls = 1;
    trace.push(
        "tool_finished",
        Some(PREFETCH_TOOL.to_string()),
        prefetched.evidence_ids.clone(),
        prefetched
            .output
            .pointer("/error/code")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    );
    if prefetched.evidence_ids.is_empty() {
        return terminal_with_registry(
            provider,
            &input,
            &prompt,
            AgentRunStatus::ProtocolFailed,
            accounting,
            None,
            Some(fixed_error(
                "scenario_prefetch_failed",
                "Trusted scenario prefetch failed",
            )),
            trace,
            started,
            &registry,
        );
    }
    messages.push(ModelMessage::Assistant {
        content: None,
        tool_calls: vec![ProviderToolCall {
            call_id: PREFETCH_CALL_ID.to_string(),
            name: PREFETCH_TOOL.to_string(),
            arguments: serde_json::json!({}),
        }],
    });
    messages.push(ModelMessage::ToolResult {
        call_id: PREFETCH_CALL_ID.to_string(),
        output: prefetched.output,
    });

    const KNOWLEDGE_PREFETCH_CALL_ID: &str = "server-prefetch-knowledge";
    const KNOWLEDGE_PREFETCH_TOOL: &str = "search_knowledge_base";
    if runtime.knowledge().is_some() && accounting.tool_calls < limits.max_tool_calls {
        if let Some(query) = knowledge_prefetch(&analysis_plan, &input.question) {
            trace.push(
                "tool_started",
                Some(KNOWLEDGE_PREFETCH_TOOL.to_string()),
                Vec::new(),
                Some("server_domain_prefetch".to_string()),
            );
            let arguments = serde_json::to_value(&query).unwrap_or_else(|_| serde_json::json!({}));
            let prefetched_knowledge =
                registry.dispatch(&input.run_id, KNOWLEDGE_PREFETCH_TOOL, arguments.clone());
            record_replay(
                &replay_sink,
                "tool_dispatch",
                serde_json::json!({
                    "call_id": KNOWLEDGE_PREFETCH_CALL_ID,
                    "tool_name": KNOWLEDGE_PREFETCH_TOOL,
                    "arguments": &arguments,
                    "output": &prefetched_knowledge.output,
                    "evidence_ids": &prefetched_knowledge.evidence_ids,
                    "budget_exhausted": prefetched_knowledge.budget_exhausted,
                    "server_initiated": true,
                }),
            );
            accounting.tool_calls += 1;
            trace.push(
                "tool_finished",
                Some(KNOWLEDGE_PREFETCH_TOOL.to_string()),
                prefetched_knowledge.evidence_ids.clone(),
                prefetched_knowledge
                    .output
                    .pointer("/error/code")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            );
            messages.push(ModelMessage::Assistant {
                content: None,
                tool_calls: vec![ProviderToolCall {
                    call_id: KNOWLEDGE_PREFETCH_CALL_ID.to_string(),
                    name: KNOWLEDGE_PREFETCH_TOOL.to_string(),
                    arguments,
                }],
            });
            messages.push(ModelMessage::ToolResult {
                call_id: KNOWLEDGE_PREFETCH_CALL_ID.to_string(),
                output: prefetched_knowledge.output,
            });
        }
    }
    let initial_evidence_pack = build_evidence_pack(&analysis_plan, registry.evidence());
    trace.push(
        "evidence_coverage_checked",
        None,
        initial_evidence_pack.evidence_ids.clone(),
        Some(
            initial_evidence_pack
                .coverage
                .sufficiency
                .as_str()
                .to_string(),
        ),
    );
    messages.push(ModelMessage::User {
        content: evidence_pack_model_context(&initial_evidence_pack),
    });

    loop {
        if cancellation.is_cancelled() {
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::Cancelled,
                accounting,
                None,
                Some(fixed_error("run_cancelled", "Agent run was cancelled")),
                trace,
                started,
                &registry,
            );
        }
        if started.elapsed() >= Duration::from_millis(limits.wall_time_ms) {
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::TimedOut,
                accounting,
                None,
                Some(fixed_error(
                    "run_timeout",
                    "Agent run exceeded its wall-time limit",
                )),
                trace,
                started,
                &registry,
            );
        }
        if accounting.model_turns >= limits.max_model_turns {
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::BudgetExhausted,
                accounting,
                None,
                Some(fixed_error(
                    "model_turn_budget",
                    "Model turn budget is exhausted",
                )),
                trace,
                started,
                &registry,
            );
        }

        let is_repair = repair_message.is_some();
        let tools_available = !is_repair && !final_report_only;
        let request = ModelRequest {
            instructions: prompt.instructions.to_string(),
            messages: repair_message
                .take()
                .map(|content| vec![ModelMessage::User { content }])
                .unwrap_or_else(|| messages.clone()),
            tools: if tools_available {
                // Keep the plan-scoped tool catalog stable across model turns.
                // Budget and diagnosis rules are enforced as recoverable tool
                // results below; shrinking the catalog made compatible models
                // repeat a previously visible tool and abort the whole run.
                tools.clone()
            } else {
                Vec::new()
            },
            response_format: Some(StructuredOutputDefinition {
                name: "agent_report_content_v1".to_string(),
                schema: report_content_json_schema(),
            }),
            max_output_tokens: limits.max_output_tokens_per_turn,
        };
        if request.validate().is_err() {
            record_replay(
                &replay_sink,
                "local_protocol_error",
                serde_json::json!({"code": "invalid_model_transcript", "request": &request}),
            );
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::ProtocolFailed,
                accounting,
                None,
                Some(fixed_error(
                    "invalid_model_transcript",
                    "Model transcript failed local validation",
                )),
                trace,
                started,
                &registry,
            );
        }

        trace.push(
            "model_started",
            None,
            Vec::new(),
            Some(
                if is_repair {
                    "report_repair"
                } else if final_report_only {
                    "final_report"
                } else {
                    "tool_selection"
                }
                .to_string(),
            ),
        );
        record_replay(
            &replay_sink,
            "model_request",
            serde_json::to_value(&request).unwrap_or_else(|_| serde_json::json!({})),
        );
        accounting.model_turns += 1;
        let remaining =
            Duration::from_millis(limits.wall_time_ms).saturating_sub(started.elapsed());
        let provider_result = tokio::select! {
            result = tokio::time::timeout(remaining, provider.complete(&request)) => Some(result),
            _ = cancellation.cancelled() => None,
        };
        let response = match provider_result {
            None => {
                record_replay(
                    &replay_sink,
                    "provider_cancelled",
                    serde_json::json!({"model_turn": accounting.model_turns}),
                );
                return terminal_with_registry(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::Cancelled,
                    accounting,
                    None,
                    Some(fixed_error("run_cancelled", "Agent run was cancelled")),
                    trace,
                    started,
                    &registry,
                );
            }
            Some(Ok(Ok(response))) => response,
            Some(Ok(Err(error))) => {
                record_replay(
                    &replay_sink,
                    "provider_error",
                    serde_json::json!({
                        "code": error.code,
                        "message": error.message,
                        "retryable": error.retryable,
                        "upstream_status": error.upstream_status,
                        "usage": &error.usage,
                    }),
                );
                add_usage(&mut accounting, &error.usage);
                if error.code == "provider_response_empty" {
                    if empty_response_retries < MAX_EMPTY_RESPONSE_RETRIES
                        && !registry.evidence().is_empty()
                        && accounting.model_turns < limits.max_model_turns
                    {
                        empty_response_retries += 1;
                        final_report_only = true;
                        messages.push(ModelMessage::User {
                            content: "The previous provider response was empty. Using only the tool evidence already present in this transcript, return one complete AgentReportContentV1 JSON object now. Do not call more tools and do not add unsupported claims.".to_string(),
                        });
                        trace.push(
                            "provider_empty_retry",
                            None,
                            Vec::new(),
                            Some("bounded_final_report_retry".to_string()),
                        );
                        continue;
                    }
                    if let Some(content) = evidence_preserving_provider_fallback(
                        &analysis_plan,
                        registry.evidence(),
                        "模型响应为空，未形成完整解释。",
                    ) {
                        trace.push(
                            "provider_empty_evidence_preserved",
                            None,
                            cited_evidence_ids(&content),
                            Some(error.code.to_string()),
                        );
                        return terminal_with_report(
                            provider,
                            &input,
                            &prompt,
                            AgentRunStatus::PartiallyVerified,
                            accounting,
                            content,
                            Some(fixed_error(error.code, error.message)),
                            trace,
                            started,
                            &registry,
                        );
                    }
                }
                return terminal_with_registry(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::ProviderFailed,
                    accounting,
                    None,
                    Some(fixed_error(error.code, error.message)),
                    trace,
                    started,
                    &registry,
                );
            }
            Some(Err(_)) => {
                record_replay(
                    &replay_sink,
                    "provider_timeout",
                    serde_json::json!({"model_turn": accounting.model_turns}),
                );
                return terminal_with_registry(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::TimedOut,
                    accounting,
                    None,
                    Some(fixed_error("provider_timeout", "Provider call timed out")),
                    trace,
                    started,
                    &registry,
                );
            }
        };
        record_replay(
            &replay_sink,
            "model_response",
            serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({})),
        );
        add_usage(&mut accounting, &response.usage);
        if cancellation.is_cancelled() {
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::Cancelled,
                accounting,
                None,
                Some(fixed_error("run_cancelled", "Agent run was cancelled")),
                trace,
                started,
                &registry,
            );
        }
        if let Err(protocol_error) = response.validate_against(&request) {
            if protocol_error.code == "unregistered_provider_tool"
                && !is_repair
                && tool_selection_retries < MAX_TOOL_SELECTION_RETRIES
                && accounting.model_turns < limits.max_model_turns
            {
                tool_selection_retries += 1;
                let available = request
                    .tools
                    .iter()
                    .map(|tool| tool.name.clone())
                    .collect::<Vec<_>>();
                let unavailable = response
                    .tool_calls
                    .iter()
                    .filter(|call| !available.contains(&call.name))
                    .map(|call| call.name.clone())
                    .collect::<Vec<_>>();
                record_replay(
                    &replay_sink,
                    "tool_selection_rejected",
                    serde_json::json!({
                        "code": protocol_error.code,
                        "requested_tools": &unavailable,
                        "available_tools": &available,
                    }),
                );
                let correction = if available.is_empty() {
                    "The previous action selected a tool, but this is a report-only turn and nothing was executed. Return the final AgentReportContentV1 JSON object now using only the existing evidence.".to_string()
                } else {
                    format!(
                        "The previous action selected an unavailable tool and nothing was executed. Continue from the existing evidence. Select only one of these tools: {}. You may instead return the final AgentReportContentV1 JSON object.",
                        available.join(", ")
                    )
                };
                messages.push(ModelMessage::User {
                    content: correction,
                });
                trace.push(
                    "tool_selection_recovered",
                    None,
                    Vec::new(),
                    Some("unregistered_provider_tool".to_string()),
                );
                continue;
            }
            if protocol_error.code == "unregistered_provider_tool" {
                if let Some(content) = evidence_preserving_provider_fallback(
                    &analysis_plan,
                    registry.evidence(),
                    "模型连续选择了不可用工具；已保留此前取得的可验证证据。",
                ) {
                    trace.push(
                        "tool_selection_evidence_preserved",
                        None,
                        cited_evidence_ids(&content),
                        Some("unregistered_provider_tool".to_string()),
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::PartiallyVerified,
                        accounting,
                        content,
                        Some(fixed_error(protocol_error.code, protocol_error.message)),
                        trace,
                        started,
                        &registry,
                    );
                }
            }
            record_replay(
                &replay_sink,
                "local_protocol_error",
                serde_json::json!({
                    "code": "invalid_provider_response",
                    "request": &request,
                    "response": &response,
                }),
            );
            return terminal_with_registry(
                provider,
                &input,
                &prompt,
                AgentRunStatus::ProtocolFailed,
                accounting,
                None,
                Some(fixed_error(
                    "invalid_provider_response",
                    "Provider response failed local validation",
                )),
                trace,
                started,
                &registry,
            );
        }

        trace.push("model_finished", None, Vec::new(), None);

        if !response.tool_calls.is_empty() {
            trace.push_decision_summary(public_decision_summary(
                response.assistant_text.as_deref(),
                &response.tool_calls,
            ));
            let requested_knowledge_calls = response
                .tool_calls
                .iter()
                .filter(|call| call.name == "search_knowledge_base")
                .count() as u32;
            let reference_lookup_requested =
                response.tool_calls.iter().any(is_reference_lookup_call);
            let mut available_knowledge_calls =
                MAX_KNOWLEDGE_SEARCHES.saturating_sub(registry.used_knowledge_searches());
            if reference_lookup_requested {
                // Identity/source lookups are point queries. One bounded retrieval is enough;
                // allowing a second query encourages associative drift from the matched alias
                // to nearby names instead of answering from the direct passage.
                available_knowledge_calls = available_knowledge_calls.min(1);
            }
            let coalesced_knowledge_calls =
                requested_knowledge_calls.saturating_sub(available_knowledge_calls);
            let effective_tool_calls =
                (response.tool_calls.len() as u32).saturating_sub(coalesced_knowledge_calls);
            if accounting.tool_calls.saturating_add(effective_tool_calls) > limits.max_tool_calls {
                return terminal_with_registry(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::BudgetExhausted,
                    accounting,
                    None,
                    Some(fixed_error(
                        "tool_call_budget",
                        "Tool call budget is exhausted",
                    )),
                    trace,
                    started,
                    &registry,
                );
            }
            let domain_experiment_requested = response
                .tool_calls
                .iter()
                .any(|call| is_domain_experiment(&call.name));
            domain_experiment_completed |= domain_experiment_requested;
            messages.push(ModelMessage::Assistant {
                content: response.assistant_text,
                tool_calls: response.tool_calls.clone(),
            });
            let mut knowledge_calls_processed = 0_u32;
            let mut knowledge_calls_coalesced = 0_u32;
            for mut call in response.tool_calls {
                if cancellation.is_cancelled() {
                    return terminal_with_registry(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::Cancelled,
                        accounting,
                        None,
                        Some(fixed_error("run_cancelled", "Agent run was cancelled")),
                        trace,
                        started,
                        &registry,
                    );
                }
                if call.name == "search_knowledge_base"
                    && knowledge_calls_processed >= available_knowledge_calls
                {
                    knowledge_calls_coalesced += 1;
                    let coalesced = AgentToolRegistry::coalesced_knowledge_search();
                    record_replay(
                        &replay_sink,
                        "tool_dispatch",
                        serde_json::json!({
                            "call_id": &call.call_id,
                            "tool_name": &call.name,
                            "arguments": &call.arguments,
                            "output": &coalesced.output,
                            "evidence_ids": &coalesced.evidence_ids,
                            "budget_exhausted": coalesced.budget_exhausted,
                            "coalesced": true,
                        }),
                    );
                    messages.push(ModelMessage::ToolResult {
                        call_id: call.call_id,
                        output: coalesced.output,
                    });
                    continue;
                }
                let reference_lookup_call = is_reference_lookup_call(&call);
                if call.name == "search_knowledge_base" {
                    knowledge_calls_processed += 1;
                }
                if reference_lookup_call {
                    let planned_query = normalize_reference_query(&input.question);
                    if let Some(arguments) = call.arguments.as_object_mut() {
                        arguments.insert("query".to_string(), serde_json::json!(planned_query));
                    }
                }
                accounting.tool_calls += 1;
                let diagnosis_deferred = rotation_tool_requires_diagnosis(
                    &analysis_plan,
                    registry.evidence(),
                    &call.name,
                );
                trace.push(
                    if diagnosis_deferred {
                        "tool_deferred"
                    } else {
                        "tool_started"
                    },
                    Some(call.name.clone()),
                    Vec::new(),
                    diagnosis_deferred.then(|| "rotation_diagnosis_required".to_string()),
                );
                let arguments = call.arguments.clone();
                let outcome = if diagnosis_deferred {
                    AgentToolRegistry::rotation_diagnosis_required(&call.name)
                } else {
                    registry.dispatch(&input.run_id, &call.name, call.arguments)
                };
                record_replay(
                    &replay_sink,
                    "tool_dispatch",
                    serde_json::json!({
                        "call_id": &call.call_id,
                        "tool_name": &call.name,
                        "arguments": &arguments,
                        "output": &outcome.output,
                        "evidence_ids": &outcome.evidence_ids,
                        "budget_exhausted": outcome.budget_exhausted,
                        "coalesced": false,
                    }),
                );
                trace.push(
                    "tool_finished",
                    Some(call.name.clone()),
                    outcome.evidence_ids.clone(),
                    outcome
                        .output
                        .pointer("/error/code")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                );
                messages.push(ModelMessage::ToolResult {
                    call_id: call.call_id,
                    output: outcome.output,
                });
                if outcome.budget_exhausted {
                    let knowledge_budget = call.name == "search_knowledge_base";
                    final_report_only = true;
                    trace.push(
                        "budget_limit_reached",
                        Some(call.name.clone()),
                        Vec::new(),
                        Some(
                            if knowledge_budget {
                                "knowledge_search_budget"
                            } else {
                                "simulation_budget"
                            }
                            .to_string(),
                        ),
                    );
                    messages.push(ModelMessage::User {
                        content: "The requested tool exceeded its bounded budget. Do not call more tools. Return a report using the evidence already registered, clearly marking the unrun experiment as a limitation instead of treating the whole conversation as failed.".to_string(),
                    });
                    break;
                }
            }
            if knowledge_calls_coalesced > 0 {
                trace.push(
                    "knowledge_searches_coalesced",
                    None,
                    Vec::new(),
                    Some("redundant_searches_suppressed".to_string()),
                );
            }
            let evidence_pack = build_evidence_pack(&analysis_plan, registry.evidence());
            trace.push(
                "evidence_coverage_checked",
                None,
                evidence_pack.evidence_ids.clone(),
                Some(evidence_pack.coverage.sufficiency.as_str().to_string()),
            );
            messages.push(ModelMessage::User {
                content: evidence_pack_model_context(&evidence_pack),
            });
            let needs_knowledge_followup = runtime.knowledge().is_some()
                && evidence_pack
                    .coverage
                    .missing_dimensions
                    .iter()
                    .any(|dimension| {
                        matches!(
                            dimension.as_str(),
                            "versioned_knowledge" | "implementation_boundary"
                        )
                    })
                && registry.used_knowledge_searches() < MAX_KNOWLEDGE_SEARCHES;
            // Rotation playbooks with both diagnosis and comparison tools remain open
            // for a bounded diagnose -> candidate -> A/B loop. Other playbooks preserve
            // their cheap one-experiment or one-point-lookup termination behavior.
            if !adaptive_experiments {
                final_report_only = domain_experiment_completed && !needs_knowledge_followup;
                if registry.used_knowledge_searches() >= MAX_KNOWLEDGE_SEARCHES
                    || knowledge_calls_coalesced > 0
                    || reference_lookup_requested
                {
                    final_report_only = true;
                }
            }
            continue;
        }

        if matches!(response.finish_reason, FinishReason::Refusal) {
            let content = refusal_content("模型拒绝了当前请求。", "当前请求没有形成可验证结论。");
            return completed_report(
                provider,
                &input,
                &prompt,
                AgentRunStatus::Refused,
                accounting,
                content,
                trace,
                started,
                &registry,
            );
        }

        let evidence_pack = build_evidence_pack(&analysis_plan, registry.evidence());
        let explicit_refusal = response
            .assistant_text
            .as_deref()
            .and_then(|raw| parse_and_validate_report(raw, registry.evidence()).ok())
            .is_some_and(|validated| validated.content.refusal_reason.is_some());
        let rotation_diagnosis_missing = evidence_pack
            .coverage
            .missing_dimensions
            .iter()
            .any(|dimension| dimension == "rotation_diagnosis");
        let diagnosis_available = tools.iter().any(|tool| tool.name == "analyze_timeline")
            && limits
                .max_simulations
                .saturating_sub(registry.used_simulations())
                >= 1;
        // A rotation report is premature until the server has produced the
        // deterministic diagnostic profile. This gate applies equally to manual
        // sequences and macros, and comes before any candidate experiment.
        if !is_repair
            && !final_report_only
            && !explicit_refusal
            && rotation_diagnosis_missing
            && diagnosis_available
            && diagnosis_gap_reminders == 0
            && accounting.model_turns < limits.max_model_turns
        {
            diagnosis_gap_reminders += 1;
            messages.push(ModelMessage::Assistant {
                content: response.assistant_text,
                tool_calls: Vec::new(),
            });
            messages.push(ModelMessage::User {
                content: "The server evidence contract still lacks the baseline rotation diagnosis. Do not propose or verify a modification yet. Call analyze_timeline, then distinguish supported strengths from observed risks and state the interpretation boundary before deciding whether a candidate experiment is warranted.".to_string(),
            });
            trace.push(
                "evidence_gap_requires_tool",
                Some("analyze_timeline".to_string()),
                evidence_pack.evidence_ids,
                Some("rotation_diagnosis_missing".to_string()),
            );
            continue;
        }
        let candidate_comparison_missing = evidence_pack
            .coverage
            .missing_dimensions
            .iter()
            .any(|dimension| dimension == "candidate_comparison");
        let comparison_available = has_rotation_diagnosis(registry.evidence())
            && tools.iter().any(|tool| tool.name == "compare_scenarios")
            && limits
                .max_simulations
                .saturating_sub(registry.used_simulations())
                >= 2;
        // A final report is premature when the server-selected task contract says
        // the user explicitly asked for a tested candidate. Give the planner one
        // bounded chance to fill that semantic gap; if it still declines, validate
        // and publish only what the evidence supports instead of dead-ending.
        if !is_repair
            && !final_report_only
            && candidate_comparison_missing
            && comparison_available
            && evidence_gap_reminders == 0
            && accounting.model_turns < limits.max_model_turns
        {
            evidence_gap_reminders += 1;
            messages.push(ModelMessage::Assistant {
                content: response.assistant_text,
                tool_calls: Vec::new(),
            });
            messages.push(ModelMessage::User {
                content: "The server evidence contract still lacks the explicitly requested same-scenario candidate comparison. Do not publish a verified modification yet. Use the current scenario, guide, and timeline evidence to formulate one conservative single-variable candidate and call compare_scenarios. If no grounded candidate exists, preserve that as a limitation on the following turn rather than inventing one.".to_string(),
            });
            trace.push(
                "evidence_gap_requires_tool",
                Some("compare_scenarios".to_string()),
                evidence_pack.evidence_ids,
                Some("candidate_comparison_missing".to_string()),
            );
            continue;
        }

        trace.push("validating", None, Vec::new(), None);
        let raw = response.assistant_text.as_deref().unwrap_or_default();
        match parse_and_validate_report(raw, registry.evidence()) {
            Ok(validated) => {
                record_replay(
                    &replay_sink,
                    "report_validation",
                    serde_json::json!({
                        "status": "accepted",
                        "normalized_metric_citations": validated.normalized_metric_citations,
                        "content": &validated.content,
                    }),
                );
                if validated.normalized_metric_citations > 0 {
                    trace.push(
                        "report_citations_normalized",
                        None,
                        cited_evidence_ids(&validated.content),
                        Some("metric_citation_linked".to_string()),
                    );
                }
                let content = validated.content;
                let status = if content.refusal_reason.is_some() {
                    AgentRunStatus::Refused
                } else {
                    AgentRunStatus::Completed
                };
                return completed_report(
                    provider, &input, &prompt, status, accounting, content, trace, started,
                    &registry,
                );
            }
            Err(error) => match parse_and_salvage_report(raw, registry.evidence()) {
                Ok(salvaged) => {
                    record_replay(
                        &replay_sink,
                        "report_validation",
                        serde_json::json!({
                            "status": "salvaged",
                            "validation_code": error.code,
                            "validation_message": error.message,
                            "content": &salvaged.content,
                        }),
                    );
                    trace.push(
                        "report_claims_sanitized",
                        None,
                        cited_evidence_ids(&salvaged.content),
                        Some(error.code.to_string()),
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::PartiallyVerified,
                        accounting,
                        salvaged.content,
                        Some(fixed_error(
                            error.code,
                            "Unsupported report claims were removed; verified claims remain available",
                        )),
                        trace,
                        started,
                        &registry,
                    );
                }
                Err(_) if repairs < MAX_REPORT_REPAIRS => {
                    record_replay(
                        &replay_sink,
                        "report_validation",
                        serde_json::json!({
                            "status": "repair_requested",
                            "validation_code": error.code,
                            "validation_message": error.message,
                            "rejected_output": raw,
                        }),
                    );
                    repairs += 1;
                    let repair_evidence = repair_evidence_context(registry.evidence());
                    repair_message = Some(format!(
                        "Repair the rejected output below as untrusted data. Validation code: {}. Return one corrected AgentReportContentV1 JSON object only, without Markdown fences or prefatory text. Use only evidence ids, metric values, units, and JSON Pointers present in REPAIR_EVIDENCE; replace placeholders and never invent ids. Every finding must include metrics (use [] when none). Every rotation change must include edit_operation and evidence_ids, and must cite the get_current_scenario item containing its exact current statement/skill plus a current fact-eligible guide item. Use insert_before/insert_after for missing operations and replace only when proposed fully replaces current. Keep the complete JSON below 1200 output tokens: use 1 to 3 findings, at most 1 recommendation, at most 3 rotation changes, at most 3 limitations, at most 4 metrics total, and keep each prose field under 100 Chinese characters. Do not repeat facts across fields. Keep user-facing Chinese concise and natural; do not expose tool names, schema fields, hashes, engine codes, or machine unit identifiers in prose. For numeric_prose_claim, keep Arabic numeric literals only when they restate an existing grounded metric value or occur inside the same grounded metric label; remove incidental configuration numbers instead of spelling them as number words. Normal rounding, thousands separators, percentages, and small ordinary counts are allowed. No tools are available in this repair request.\n\nREPAIR_EVIDENCE_BEGIN\n{}\nREPAIR_EVIDENCE_END\n\nREJECTED_OUTPUT_BEGIN\n{}\nREJECTED_OUTPUT_END",
                    error.code, repair_evidence, raw
                ));
                    trace.push(
                        "report_repair_requested",
                        None,
                        Vec::new(),
                        Some(error.code.to_string()),
                    );
                }
                Err(_) => {
                    record_replay(
                        &replay_sink,
                        "report_validation",
                        serde_json::json!({
                            "status": "rejected",
                            "validation_code": error.code,
                            "validation_message": error.message,
                            "rejected_output": raw,
                        }),
                    );
                    if let Some(content) = evidence_preserving_provider_fallback(
                        &analysis_plan,
                        registry.evidence(),
                        "模型报告结构连续两次未通过校验，未发布其中的玩法结论。",
                    ) {
                        trace.push(
                            "report_structure_evidence_preserved",
                            None,
                            cited_evidence_ids(&content),
                            Some(error.code.to_string()),
                        );
                        return terminal_with_report(
                            provider,
                            &input,
                            &prompt,
                            AgentRunStatus::PartiallyVerified,
                            accounting,
                            content,
                            Some(fixed_error(error.code, error.message)),
                            trace,
                            started,
                            &registry,
                        );
                    }
                    let content = refusal_content(
                        "现有输出无法解析为可校验报告。",
                        "结构损坏的输出不会作为结论展示。",
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::EvidenceInsufficient,
                        accounting,
                        content,
                        Some(fixed_error(error.code, error.message)),
                        trace,
                        started,
                        &registry,
                    );
                }
            },
        }
    }
}

fn public_decision_summary(
    assistant_text: Option<&str>,
    tool_calls: &[ProviderToolCall],
) -> String {
    let compact = assistant_text
        .unwrap_or_default()
        .replace("<decision_summary>", "")
        .replace("</decision_summary>", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !compact.is_empty() && !compact.starts_with('{') {
        let mut summary = compact.chars().take(400).collect::<String>();
        if compact.chars().count() > 400 {
            summary.push('…');
        }
        return summary;
    }
    let tools = tool_calls
        .iter()
        .map(|call| call.name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    format!("模型未提供公开决策摘要；本轮请求调用：{tools}。可在私有复现记录中检查供应商原始响应。")
}

fn is_domain_experiment(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "simulate_scenario" | "compare_scenarios" | "analyze_timeline"
    )
}

fn has_rotation_diagnosis(evidence: &EvidenceStore) -> bool {
    evidence.values().any(|item| {
        item.get("tool_name").and_then(serde_json::Value::as_str) == Some("analyze_timeline")
            && item.pointer("/result/diagnostic_profile").is_some()
    })
}

/// Keep the tool visible but turn premature simulation/comparison calls into a
/// recoverable tool result. This preserves diagnosis-first semantics without a
/// brittle, shrinking provider tool catalog.
fn rotation_tool_requires_diagnosis(
    plan: &AnalysisPlanV1,
    evidence: &EvidenceStore,
    tool_name: &str,
) -> bool {
    let diagnosis_first = plan
        .routing_signals
        .iter()
        .any(|signal| signal == "rotation_diagnosis_first");
    diagnosis_first
        && !has_rotation_diagnosis(evidence)
        && matches!(tool_name, "simulate_scenario" | "compare_scenarios")
}

fn requires_knowledge_only_client_scope(question: &str) -> bool {
    let normalized = question.to_lowercase();
    normalized.contains("无界")
        || normalized.contains("分山劲·悟")
        || normalized.contains("分山劲・悟")
        || normalized.contains("wujie")
}

fn is_reference_lookup_call(call: &ProviderToolCall) -> bool {
    call.name == "search_knowledge_base"
        && call
            .arguments
            .get("version_scope")
            .and_then(|value| value.as_str())
            == Some("reference_lookup")
}

fn validate_input(input: &AgentRunInput, limits: &AgentRunLimits) -> Result<(), ()> {
    validate_trace_id(&input.run_id).map_err(|_| ())?;
    if input.question.trim().is_empty()
        || input.question.len() > MAX_QUESTION_BYTES
        || input
            .session_context
            .as_ref()
            .is_some_and(|context| context.len() > MAX_SESSION_CONTEXT_BYTES)
        || input
            .question
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        || limits.max_model_turns == 0
        || limits.max_model_turns > 6
        || limits.max_tool_calls == 0
        || limits.max_tool_calls > 8
        || limits.max_simulations == 0
        || limits.max_simulations > 8
        || limits.max_output_tokens_per_turn == 0
        || limits.max_output_tokens_per_turn > 8192
        || limits.wall_time_ms == 0
        || limits.wall_time_ms > 180_000
    {
        return Err(());
    }
    input.scenario.verify_hash().map_err(|_| ())
}

fn completed_report(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    status: AgentRunStatus,
    accounting: AgentRunAccountingV1,
    content: AgentReportContentV1,
    trace: TraceCollector,
    started: Instant,
    registry: &AgentToolRegistry<'_>,
) -> AgentRunResultV1 {
    terminal_with_report(
        provider, input, prompt, status, accounting, content, None, trace, started, registry,
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_with_report(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    status: AgentRunStatus,
    mut accounting: AgentRunAccountingV1,
    content: AgentReportContentV1,
    error: Option<AgentRunErrorV1>,
    mut trace: TraceCollector,
    started: Instant,
    registry: &AgentToolRegistry<'_>,
) -> AgentRunResultV1 {
    accounting.simulations = registry.used_simulations();
    accounting.knowledge_searches = registry.used_knowledge_searches();
    accounting.duration_ms = elapsed_ms(started);
    let termination = status_name(&status).to_string();
    let evidence_ids = cited_evidence_ids(&content);
    let sources = cited_knowledge_sources(&evidence_ids, registry.evidence());
    let report = AgentReportV1 {
        schema_version: AGENT_REPORT_SCHEMA_V1.to_string(),
        question: input.question.clone(),
        scenario_hash: input.scenario.scenario_hash.clone(),
        prompt_version: prompt.version.to_string(),
        prompt_sha256: prompt.sha256.clone(),
        provider_profile: provider.profile_id().to_string(),
        model: provider.model().to_string(),
        sources,
        evidence_ids,
        content,
        accounting: accounting.clone(),
        termination: termination.clone(),
    };
    trace.push(
        status_name(&status),
        None,
        report.evidence_ids.clone(),
        error.as_ref().map(|value| value.code.clone()),
    );
    result(
        provider,
        input,
        prompt,
        status,
        accounting,
        Some(report),
        error,
        trace,
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_with_registry(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    status: AgentRunStatus,
    mut accounting: AgentRunAccountingV1,
    report: Option<AgentReportV1>,
    error: Option<AgentRunErrorV1>,
    mut trace: TraceCollector,
    started: Instant,
    registry: &AgentToolRegistry<'_>,
) -> AgentRunResultV1 {
    accounting.simulations = registry.used_simulations();
    accounting.knowledge_searches = registry.used_knowledge_searches();
    accounting.duration_ms = elapsed_ms(started);
    trace.push(
        status_name(&status),
        None,
        Vec::new(),
        error.as_ref().map(|value| value.code.clone()),
    );
    result(
        provider, input, prompt, status, accounting, report, error, trace,
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    status: AgentRunStatus,
    mut accounting: AgentRunAccountingV1,
    report: Option<AgentReportV1>,
    error: Option<AgentRunErrorV1>,
    mut trace: TraceCollector,
    started: Instant,
) -> AgentRunResultV1 {
    accounting.duration_ms = elapsed_ms(started);
    trace.push(
        status_name(&status),
        None,
        Vec::new(),
        error.as_ref().map(|value| value.code.clone()),
    );
    result(
        provider, input, prompt, status, accounting, report, error, trace,
    )
}

#[allow(clippy::too_many_arguments)]
fn result(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    status: AgentRunStatus,
    accounting: AgentRunAccountingV1,
    report: Option<AgentReportV1>,
    error: Option<AgentRunErrorV1>,
    trace: TraceCollector,
) -> AgentRunResultV1 {
    AgentRunResultV1 {
        schema_version: AGENT_RUN_SCHEMA_V1.to_string(),
        run_id: input.run_id.clone(),
        scenario_hash: input.scenario.scenario_hash.clone(),
        prompt_version: prompt.version.to_string(),
        prompt_sha256: prompt.sha256.clone(),
        provider_profile: provider.profile_id().to_string(),
        model: provider.model().to_string(),
        status,
        accounting,
        report,
        error,
        trace: trace.into_events(),
    }
}

fn refusal_content(summary: &str, limitation: &str) -> AgentReportContentV1 {
    AgentReportContentV1 {
        schema_version: AGENT_REPORT_CONTENT_SCHEMA_V1.to_string(),
        summary: summary.to_string(),
        findings: Vec::new(),
        recommendations: Vec::new(),
        rotation_changes: Vec::new(),
        limitations: vec![limitation.to_string()],
        refusal_reason: Some(summary.to_string()),
    }
}

fn evidence_preserving_provider_fallback(
    plan: &AnalysisPlanV1,
    evidence: &super::report::EvidenceStore,
    limitation: &str,
) -> Option<AgentReportContentV1> {
    let evidence_ids = evidence
        .iter()
        .filter_map(|(evidence_id, envelope)| {
            (envelope
                .get("tool_name")
                .and_then(serde_json::Value::as_str)
                != Some("get_current_scenario"))
            .then(|| evidence_id.clone())
        })
        .collect::<Vec<_>>();
    if evidence_ids.is_empty() {
        return None;
    }
    let mut knowledge_findings = Vec::new();
    let mut seen_excerpts = Vec::<String>::new();
    for (evidence_id, envelope) in evidence {
        if envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            != Some("search_knowledge_base")
        {
            continue;
        }
        let Some(item) = envelope
            .pointer("/result/results/0")
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        if item
            .get("fact_eligible")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            continue;
        }
        let claim = item
            .get("domain_claims")
            .and_then(serde_json::Value::as_array)
            .and_then(|claims| claims.first())
            .and_then(|claim| claim.get("statement"))
            .and_then(serde_json::Value::as_str);
        let snippet = item.get("snippet").and_then(serde_json::Value::as_str);
        let excerpt = concise_evidence_excerpt(claim.or(snippet).unwrap_or_default(), 220);
        if excerpt.is_empty() || seen_excerpts.iter().any(|seen| seen == &excerpt) {
            continue;
        }
        seen_excerpts.push(excerpt.clone());
        let heading = item
            .get("heading")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty());
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("知识库证据");
        knowledge_findings.push(AgentFindingV1 {
            title: concise_evidence_excerpt(heading.unwrap_or(title), 48),
            explanation: format!("知识库原文摘录：{excerpt}"),
            evidence_ids: vec![evidence_id.clone()],
            metrics: Vec::new(),
        });
        if knowledge_findings.len() >= 3 {
            break;
        }
    }
    let has_readable_knowledge = !knowledge_findings.is_empty();
    let findings = if has_readable_knowledge {
        knowledge_findings
    } else {
        vec![AgentFindingV1 {
            title: format!("已保留{}证据", plan.playbook.label),
            explanation:
                "下列证据和来源由只读工具生成；本轮不使用结构损坏的模型文本补写玩法或数值结论。"
                    .to_string(),
            evidence_ids,
            metrics: Vec::new(),
        }]
    };
    Some(AgentReportContentV1 {
        schema_version: AGENT_REPORT_CONTENT_SCHEMA_V1.to_string(),
        summary: if has_readable_knowledge {
            format!(
                "模型解释未通过发布校验；以下直接展示“{}”命中的可溯源知识，不补写未经验证的结论。",
                plan.playbook.label
            )
        } else {
            format!(
                "“{}”已取得可验证证据，但模型解释未通过发布校验；证据已保留，可在当前会话继续追问。",
                plan.playbook.label
            )
        },
        findings,
        recommendations: Vec::new(),
        rotation_changes: Vec::new(),
        limitations: vec![limitation.to_string()],
        refusal_reason: None,
    })
}

fn concise_evidence_excerpt(value: &str, max_chars: usize) -> String {
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "---")
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['#', '`', '*'], "");
    let mut chars = normalized.chars();
    let excerpt = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{}……", excerpt.trim())
    } else {
        excerpt.trim().to_string()
    }
}

fn repair_evidence_context(evidence: &super::report::EvidenceStore) -> String {
    let mut items = Vec::new();
    for (evidence_id, envelope) in evidence {
        let tool_name = envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let result = envelope
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let compact_result = match tool_name {
            "get_current_scenario" => serde_json::json!({
                "rotation_mode": result.get("rotation_mode"),
                "rotation_input": result.get("rotation_input"),
                "network_delay_ms": result.get("network_delay_ms"),
                "game_version": result.get("game_version"),
                "mount": result.get("mount"),
            }),
            "simulate_scenario" => serde_json::json!({
                "dps": result.get("dps"),
                "total_damage": result.get("total_damage"),
                "fight_time": result.get("fight_time"),
                "skill_count": result.get("skill_count"),
                "skills": bounded_result_array(&result, "skills", 12),
                "metric_pointers": ["/result/dps", "/result/total_damage", "/result/fight_time", "/result/skill_count"]
            }),
            "analyze_timeline" => serde_json::json!({
                "fight_time": result.get("fight_time"),
                "active_event_count": result.get("active_event_count"),
                "triggered_event_count": result.get("triggered_event_count"),
                "total_cd_wait_seconds": result.get("total_cd_wait_seconds"),
                "total_observed_gcd_gap_seconds": result.get("total_observed_gcd_gap_seconds"),
                "rage": result.get("rage"),
                "skipped": bounded_result_array(&result, "skipped", 12),
                "cd_waits": bounded_result_array(&result, "cd_waits", 8),
                "gcd_gaps": bounded_result_array(&result, "gcd_gaps", 8),
            }),
            "search_knowledge_base" => {
                let results = result
                    .get("results")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(2)
                    .map(|item| {
                        serde_json::json!({
                            "title": item.get("title"),
                            "season": item.get("season"),
                            "version_match": item.get("version_match"),
                            "fact_eligible": item.get("fact_eligible"),
                            "heading": item.get("heading"),
                            "snippet": item.get("snippet").and_then(serde_json::Value::as_str).map(|value| concise_evidence_excerpt(value, 420)),
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({"results": results})
            }
            _ => serde_json::json!({"available": true}),
        };
        items.push(serde_json::json!({
            "evidence_id": evidence_id,
            "tool_name": tool_name,
            "result": compact_result,
        }));
    }
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

fn bounded_result_array(
    result: &serde_json::Value,
    key: &str,
    limit: usize,
) -> Vec<serde_json::Value> {
    result
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(limit)
        .cloned()
        .collect()
}

fn fixed_error(code: impl Into<String>, message: impl Into<String>) -> AgentRunErrorV1 {
    AgentRunErrorV1 {
        code: code.into(),
        message: message.into(),
    }
}

fn add_usage(accounting: &mut AgentRunAccountingV1, usage: &TokenUsage) {
    accounting.input_tokens = accounting.input_tokens.saturating_add(usage.input_tokens);
    accounting.output_tokens = accounting.output_tokens.saturating_add(usage.output_tokens);
    accounting.total_tokens = accounting.total_tokens.saturating_add(usage.total_tokens);
}

fn status_name(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::PartiallyVerified => "partially_verified",
        AgentRunStatus::Refused => "refused",
        AgentRunStatus::EvidenceInsufficient => "evidence_insufficient",
        AgentRunStatus::Cancelled => "cancelled",
        AgentRunStatus::BudgetExhausted => "budget_exhausted",
        AgentRunStatus::ProviderFailed => "provider_failed",
        AgentRunStatus::ProtocolFailed => "protocol_failed",
        AgentRunStatus::TimedOut => "timed_out",
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::{FakeProvider, ModelResponse, ProviderError, ProviderToolCall};
    use crate::{Attributes, TargetConfig};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::collections::{HashMap, VecDeque};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ScriptedProvider {
        responses: Mutex<VecDeque<Result<ModelResponse, ProviderError>>>,
        requests: Mutex<Vec<ModelRequest>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<Result<ModelResponse, ProviderError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<ModelRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        fn profile_id(&self) -> &str {
            "scripted"
        }

        fn model(&self) -> &str {
            "fixture-v1"
        }

        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.requests.lock().unwrap().push(request.clone());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted response exhausted")
        }
    }

    struct KnowledgeProvider;

    #[async_trait]
    impl LlmProvider for KnowledgeProvider {
        fn profile_id(&self) -> &str {
            "knowledge-fixture"
        }

        fn model(&self) -> &str {
            "fixture-v1"
        }

        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
            let knowledge_evidence = request.messages.iter().rev().find_map(|message| {
                let ModelMessage::ToolResult { output, .. } = message else {
                    return None;
                };
                (output.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base"))
                    .then_some(output)
            });
            if let Some(output) = knowledge_evidence {
                let evidence_id = output
                    .pointer("/evidence/0/evidence_id")
                    .and_then(Value::as_str)
                    .unwrap();
                return Ok(ModelResponse {
                    assistant_text: Some(
                        serde_json::to_string(&json!({
                            "schema_version": "agent-report-content/v1",
                            "summary": "当前版本资料给出了盾飞阶段的循环边界。",
                            "findings": [{
                                "title": "当前版本循环资料",
                                "explanation": "盾飞阶段需要关注劫刀数量并避免流血中断。",
                                "evidence_ids": [evidence_id],
                                "metrics": []
                            }],
                            "recommendations": [],
                            "limitations": [],
                            "refusal_reason": null
                        }))
                        .unwrap(),
                    ),
                    tool_calls: Vec::new(),
                    finish_reason: FinishReason::Stop,
                    usage: TokenUsage::default(),
                });
            }

            assert!(request
                .tools
                .iter()
                .any(|tool| tool.name == "search_knowledge_base"));
            Ok(tool_call(
                "call-knowledge",
                "search_knowledge_base",
                json!({
                    "query": "盾飞劫刀流血",
                    "version_scope": "current_only",
                    "season": null,
                    "category": "基础"
                }),
            ))
        }
    }

    struct DualEvidenceProvider;

    #[async_trait]
    impl LlmProvider for DualEvidenceProvider {
        fn profile_id(&self) -> &str {
            "dual-evidence-fixture"
        }

        fn model(&self) -> &str {
            "fixture-v1"
        }

        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
            let tool_output = |tool_name: &str| {
                request.messages.iter().rev().find_map(|message| {
                    let ModelMessage::ToolResult { output, .. } = message else {
                        return None;
                    };
                    (output.get("tool_name").and_then(Value::as_str) == Some(tool_name))
                        .then_some(output)
                })
            };
            if let Some(comparison) = tool_output("compare_scenarios") {
                let knowledge = tool_output("search_knowledge_base").unwrap();
                let knowledge_id = knowledge
                    .pointer("/evidence/0/evidence_id")
                    .and_then(Value::as_str)
                    .unwrap();
                let comparison_id = comparison
                    .pointer("/evidence/0/evidence_id")
                    .and_then(Value::as_str)
                    .unwrap();
                let baseline_dps = comparison
                    .pointer("/evidence/0/result/baseline/dps")
                    .and_then(Value::as_f64)
                    .unwrap();
                return Ok(ModelResponse {
                    assistant_text: Some(
                        serde_json::to_string(&json!({
                            "schema_version": "agent-report-content/v1",
                            "summary": "当前版本资料提出的循环假设已进入强类型候选实验。",
                            "findings": [{
                                "title": "资料假设与确定性实验",
                                "explanation": "资料用于解释候选来源，当前场景数值只采用确定性对比证据。",
                                "evidence_ids": [knowledge_id, comparison_id],
                                "metrics": [{
                                    "label": "基线平均 DPS",
                                    "value": baseline_dps,
                                    "unit": "damage_per_second",
                                    "evidence_id": comparison_id,
                                    "json_pointer": "/result/baseline/dps"
                                }]
                            }],
                            "recommendations": [],
                            "limitations": ["资料结论与本次模拟数值属于不同证据类型。"],
                            "refusal_reason": null
                        }))
                        .unwrap(),
                    ),
                    tool_calls: Vec::new(),
                    finish_reason: FinishReason::Stop,
                    usage: TokenUsage::default(),
                });
            }
            if tool_output("search_knowledge_base").is_some() {
                return Ok(tool_call(
                    "call-compare-from-knowledge",
                    "compare_scenarios",
                    json!({
                        "candidates": [{
                            "label": "延迟候选",
                            "patch": {
                                "haste_level": null,
                                "sequence": null,
                                "network_delay": 100,
                                "initial_rage": null,
                                "base_attack": null,
                                "target_defense_bonus": null
                            }
                        }]
                    }),
                ));
            }
            Ok(tool_call(
                "call-knowledge-for-candidate",
                "search_knowledge_base",
                json!({
                    "query": "盾飞劫刀流血循环",
                    "version_scope": "current_only",
                    "season": null,
                    "category": "基础"
                }),
            ))
        }
    }

    struct ReferenceKnowledgeProvider;

    #[async_trait]
    impl LlmProvider for ReferenceKnowledgeProvider {
        fn profile_id(&self) -> &str {
            "reference-knowledge-fixture"
        }

        fn model(&self) -> &str {
            "fixture-v1"
        }

        async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
            let knowledge = request.messages.iter().rev().find_map(|message| {
                let ModelMessage::ToolResult { output, .. } = message else {
                    return None;
                };
                (output.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base"))
                    .then_some(output)
            });
            if let Some(output) = knowledge {
                assert_eq!(
                    output
                        .pointer("/evidence/0/result/results/0/reference_entities/0/name")
                        .and_then(Value::as_str),
                    Some("author_a")
                );
                let evidence_id = output
                    .pointer("/evidence/0/evidence_id")
                    .and_then(Value::as_str)
                    .unwrap();
                return Ok(ModelResponse {
                    assistant_text: Some(
                        serde_json::to_string(&json!({
                            "schema_version": "agent-report-content/v1",
                            "summary": "资料中的这个称呼指向作者甲。",
                            "findings": [{
                                "title": "人物称呼",
                                "explanation": "旧赛季资料中，作者甲使用了这个自称。",
                                "evidence_ids": [evidence_id],
                                "metrics": []
                            }],
                            "recommendations": [],
                            "limitations": ["人物来源资料不能用于证明当前版本玩法机制。"],
                            "refusal_reason": null
                        }))
                        .unwrap(),
                    ),
                    tool_calls: Vec::new(),
                    finish_reason: FinishReason::Stop,
                    usage: TokenUsage::default(),
                });
            }
            Ok(tool_call(
                "call-reference-knowledge",
                "search_knowledge_base",
                json!({
                    "query": "请检索世一苍相关人物",
                    "version_scope": "reference_lookup",
                    "season": null,
                    "category": null
                }),
            ))
        }
    }

    fn scenario(runtime: &AgentRuntime) -> ScenarioSnapshotV1 {
        ScenarioSnapshotV1::capture(
            runtime.game_version(),
            runtime.mount(),
            crate::SimulateRequest {
                haste_level: 42_087,
                sequence: vec!["盾击".to_string(), "盾压".to_string()],
                talents: Vec::new(),
                channel_ticks: HashMap::new(),
                timing_offsets: HashMap::new(),
                network_delay: 0,
                recipes: Vec::new(),
                qijin_buffs: HashMap::new(),
                macro_text: None,
                macro_duration: None,
                attributes: Some(Attributes {
                    base_attack: 38_466.0,
                    weapon_damage: 10_986.0,
                    crit_level: 54_841.0,
                    crit_effect_level: 0.0,
                    overcome_level: 29_480.0,
                    strain_level: 66_031.0,
                    haste_level: 42_087.0,
                    ..Attributes::default()
                }),
                target: Some(TargetConfig {
                    level: 134,
                    defense_bonus: 0.0,
                    damage_cof: 0.0,
                }),
                initial_rage: Some(50),
                pauses: Vec::new(),
                boss_attack_interval: None,
                hanjia_expectation: None,
                tiegu_mode: 2,
                experimental: false,
                lite: false,
                lite_keep_timeline: false,
                equipment: HashMap::new(),
                team_buffs: Vec::new(),
                formation: None,
                pre_releases: Vec::new(),
            },
        )
        .unwrap()
    }

    fn input(runtime: &AgentRuntime, run_id: &str) -> AgentRunInput {
        AgentRunInput {
            run_id: run_id.to_string(),
            question: "分析当前循环的确定性输出。".to_string(),
            scenario: scenario(runtime),
            session_context: None,
            session_playbook_id: None,
        }
    }

    fn tool_call(call_id: &str, name: &str, arguments: serde_json::Value) -> ModelResponse {
        ModelResponse {
            assistant_text: None,
            tool_calls: vec![ProviderToolCall {
                call_id: call_id.to_string(),
                name: name.to_string(),
                arguments,
            }],
            finish_reason: FinishReason::ToolCalls,
            usage: TokenUsage::default(),
        }
    }

    #[test]
    fn default_limits_match_phase_plan() {
        let limits = AgentRunLimits::default();
        assert_eq!(limits.max_model_turns, 6);
        assert_eq!(limits.max_tool_calls, 8);
        assert_eq!(limits.max_simulations, 8);
        assert_eq!(limits.wall_time_ms, 120_000);
    }

    #[test]
    fn cancellation_is_shareable_and_monotonic() {
        let cancellation = AgentCancellation::default();
        let copy = cancellation.clone();
        assert!(!copy.is_cancelled());
        cancellation.cancel();
        assert!(copy.is_cancelled());
    }

    #[test]
    fn rotation_tools_use_a_soft_diagnosis_gate() {
        let runtime = AgentRuntime::fixture();
        let scenario = scenario(&runtime);
        let optimize = select_analysis_plan("分析并优化当前循环", &scenario);
        let empty = EvidenceStore::new();

        assert!(!rotation_tool_requires_diagnosis(
            &optimize,
            &empty,
            "analyze_timeline"
        ));
        assert!(rotation_tool_requires_diagnosis(
            &optimize,
            &empty,
            "simulate_scenario"
        ));
        assert!(rotation_tool_requires_diagnosis(
            &optimize,
            &empty,
            "compare_scenarios"
        ));

        let mut diagnosed = EvidenceStore::new();
        diagnosed.insert(
            "diagnosis".to_string(),
            json!({
                "tool_name": "analyze_timeline",
                "result": {"diagnostic_profile": {"input_mode": "manual_sequence"}}
            }),
        );
        assert!(!rotation_tool_requires_diagnosis(
            &optimize,
            &diagnosed,
            "analyze_timeline"
        ));
        assert!(!rotation_tool_requires_diagnosis(
            &optimize,
            &diagnosed,
            "compare_scenarios"
        ));

        let diagnose_only = select_analysis_plan("分析当前循环的优缺点", &scenario);
        assert!(!rotation_tool_requires_diagnosis(
            &diagnose_only,
            &diagnosed,
            "compare_scenarios"
        ));
    }

    #[test]
    fn refusal_content_contains_no_unverified_numbers() {
        let content = refusal_content("证据不足。", "需要更多只读实验。 ");
        assert_eq!(content.schema_version, AGENT_REPORT_CONTENT_SCHEMA_V1);
        assert!(content.findings.is_empty());
        assert!(content.refusal_reason.is_some());
        let _ = serde_json::json!({"content": content});
    }

    #[tokio::test]
    async fn fake_provider_completes_grounded_tool_loop() {
        let runtime = AgentRuntime::fixture();
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-success"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.accounting.model_turns, 2);
        assert_eq!(result.accounting.tool_calls, 2);
        assert_eq!(result.accounting.simulations, 1);
        assert!(result.trace.iter().any(|event| {
            event.kind == "decision_checkpoint"
                && event.code.as_deref() == Some("public_decision_summary")
                && event
                    .overview
                    .as_deref()
                    .is_some_and(|text| !text.is_empty())
        }));
        assert_eq!(
            result
                .trace
                .iter()
                .filter(|event| event.kind == "model_started")
                .count(),
            2
        );
        assert_eq!(
            result
                .trace
                .iter()
                .filter(|event| event.kind == "model_finished")
                .count(),
            2
        );
        let report = result.report.unwrap();
        assert_eq!(report.evidence_ids.len(), 2);
        assert_eq!(
            report.content.findings[0].metrics[0].json_pointer,
            "/result/dps"
        );
    }

    #[tokio::test]
    async fn knowledge_only_run_exposes_versioned_sources_without_a_simulation() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let result = run_agent(
            &KnowledgeProvider,
            &runtime,
            input(&runtime, "run-knowledge"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.prompt_version, "agent-system/v17");
        assert_eq!(result.accounting.knowledge_searches, 1);
        assert_eq!(result.accounting.simulations, 0);
        let report = result.report.unwrap();
        assert_eq!(report.sources.len(), 1);
        assert_eq!(report.sources[0].season, "暗影千机（2026）");
        assert_eq!(report.sources[0].version_match, "current_exact");
        assert!(report.sources[0].fact_eligible);
        assert_eq!(report.sources[0].source_url, "https://example.com/current");

        let expected_root = std::env::temp_dir();
        assert!(root.starts_with(&expected_root));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn wujie_questions_expose_knowledge_only_and_never_domain_experiments() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
            assistant_text: Some(
                serde_json::to_string(&refusal_content(
                    "无界端可以查询版本资料，但不能在当前计算器中形成战斗模拟结论。",
                    "这是知识资料说明；计算器未实现无界端，未经过本项目模拟验证。",
                ))
                .unwrap(),
            ),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: TokenUsage::default(),
        })]);
        let mut run_input = input(&runtime, "run-wujie-scope");
        run_input.question = "无界端分山劲·悟的循环和旗舰端 DPS 能否直接比较？".to_string();
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.simulations, 0);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "knowledge_only_client_scope"));
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["get_current_scenario", "search_knowledge_base"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn offline_provider_demonstrates_versioned_knowledge_end_to_end() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let provider = FakeProvider::new("offline".to_string(), "fixture-v1".to_string());
        let mut run_input = input(&runtime, "run-offline-knowledge");
        run_input.question = "结合当前版本攻略说明循环思路。".to_string();
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.prompt_version, "agent-system/v17");
        assert_eq!(result.accounting.knowledge_searches, 1);
        assert_eq!(result.accounting.simulations, 1);
        let report = result.report.unwrap();
        assert_eq!(report.sources.len(), 1);
        assert_eq!(report.sources[0].season, "暗影千机（2026）");
        assert_eq!(report.sources[0].version_match, "current_exact");
        assert!(report.sources[0].fact_eligible);
        assert!(report
            .content
            .findings
            .iter()
            .any(|finding| !finding.metrics.is_empty()));
        assert!(report
            .content
            .findings
            .iter()
            .any(|finding| finding.title == "循环诊断已建立"));

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reference_lookup_surfaces_old_identity_without_current_gameplay_claims() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let mut run_input = input(&runtime, "run-reference-knowledge");
        run_input.question = "世一苍是谁？给我一个名字。".to_string();
        let result = run_agent(
            &ReferenceKnowledgeProvider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.accounting.knowledge_searches, 1);
        let report = result.report.unwrap();
        assert_eq!(report.sources.len(), 1);
        assert_eq!(report.sources[0].season, "万灵当歌（2023）");
        assert_eq!(report.sources[0].version_match, "reference_only");
        assert!(report.content.limitations[0].contains("当前版本玩法机制"));

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reference_lookup_is_a_single_retrieval_then_report_state() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call(
                "call-reference-once",
                "search_knowledge_base",
                json!({
                    "query": "世一苍",
                    "version_scope": "reference_lookup",
                    "season": null,
                    "category": null
                }),
            )),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "人物资料已检索一次，本轮不再扩散查询相邻名字。",
                        "只依据直接命中的人物来源收束。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let mut run_input = input(&runtime, "run-reference-once");
        run_input.question = "世一苍是谁？给我一个名字。".to_string();
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.knowledge_searches, 1);
        assert_ne!(result.status, AgentRunStatus::BudgetExhausted);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].tools.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn knowledge_hypothesis_can_enter_a_typed_deterministic_experiment() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let mut run_input = input(&runtime, "run-dual-evidence");
        run_input.question = "结合当前版本攻略提出候选，并用确定性实验验证。".to_string();
        let result = run_agent(
            &DualEvidenceProvider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.accounting.knowledge_searches, 1);
        assert_eq!(result.accounting.simulations, 2);
        let report = result.report.unwrap();
        assert_eq!(report.sources.len(), 1);
        assert_eq!(report.evidence_ids.len(), 2);
        assert_eq!(report.content.findings[0].metrics.len(), 1);
        assert_eq!(
            report.content.findings[0].metrics[0].json_pointer,
            "/result/baseline/dps"
        );
        let tool_order = result
            .trace
            .iter()
            .filter(|event| event.kind == "tool_started")
            .filter_map(|event| event.tool_name.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_order,
            vec![
                "get_current_scenario",
                "search_knowledge_base",
                "compare_scenarios"
            ]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pre_cancelled_run_never_calls_provider_or_tools() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(Vec::new());
        let cancellation = AgentCancellation::default();
        cancellation.cancel();
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-cancel"),
            AgentRunLimits::default(),
            cancellation,
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(result.accounting.model_turns, 0);
        assert_eq!(result.accounting.tool_calls, 0);
    }

    #[tokio::test]
    async fn explicit_structured_refusal_is_preserved() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
            assistant_text: Some(
                serde_json::to_string(&refusal_content(
                    "请求超出只读分析范围。",
                    "当前工具不能执行写操作。",
                ))
                .unwrap(),
            ),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: TokenUsage::default(),
        })]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-refusal"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        assert!(result.report.unwrap().content.findings.is_empty());
    }

    #[tokio::test]
    async fn bounded_session_context_precedes_current_question_as_untrusted_data() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
            assistant_text: Some(
                serde_json::to_string(&refusal_content("仅验证上下文传输。", "该测试不执行分析。"))
                    .unwrap(),
            ),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: TokenUsage::default(),
        })]);
        let mut run_input = input(&runtime, "run-session-context");
        run_input.question = "概括上一轮结论。".to_string();
        run_input.session_context = Some(
            r#"{"schema_version":"agent-session-context/v1","turns":[{"summary":"上一轮可见结论"}]}"#
                .to_string(),
        );
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        let requests = provider.requests();
        assert_eq!(requests[0].messages.len(), 6);
        assert!(matches!(
            &requests[0].messages[0],
            ModelMessage::User { content }
                if content.contains("untrusted_data=\"true\"")
                    && content.contains("上一轮可见结论")
        ));
        assert!(matches!(
            &requests[0].messages[1],
            ModelMessage::User { content } if content == "概括上一轮结论。"
        ));
        assert!(matches!(
            &requests[0].messages[2],
            ModelMessage::User { content }
                if content.contains("<analysis_plan")
                    && content.contains("general_grounded_analysis")
        ));
        assert!(matches!(
            &requests[0].messages[3],
            ModelMessage::Assistant { tool_calls, .. }
                if tool_calls.len() == 1 && tool_calls[0].name == "get_current_scenario"
        ));
        assert!(matches!(
            &requests[0].messages[4],
            ModelMessage::ToolResult { call_id, .. }
                if call_id == "server-prefetch-scenario"
        ));
        assert!(matches!(
            &requests[0].messages[5],
            ModelMessage::User { content }
                if content.contains("<evidence_pack")
                    && content.contains("general_grounded_analysis")
        ));
        assert!(requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "get_current_scenario"));
    }

    #[tokio::test]
    async fn simulation_budget_preserves_evidence_and_finishes_a_limited_report() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "基线诊断已完成，但候选对照未运行。",
                        "剩余模拟预算不足，不能发布候选收益。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let limits = AgentRunLimits {
            max_simulations: 1,
            ..AgentRunLimits::default()
        };
        let result = run_agent(
            &provider,
            &runtime,
            AgentRunInput {
                question: "帮我比较当前一键宏和手动循环".to_string(),
                ..input(&runtime, "run-budget")
            },
            limits,
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.simulations, 1);
        let requests = provider.requests();
        assert!(requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "compare_scenarios"));
        assert!(requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "compare_scenarios"));
        assert_eq!(
            requests[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            requests[1]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn unavailable_tool_selection_is_corrected_without_losing_the_run() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-unknown", "shell", json!({}))),
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "基线诊断已完成。",
                        "本轮只验证错误工具选择可以恢复。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-tool-selection-recovery"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.model_turns, 3);
        assert_eq!(result.accounting.simulations, 1);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "tool_selection_recovered"));
        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].messages.iter().any(|message| matches!(
            message,
            ModelMessage::User { content }
                if content.contains("selected an unavailable tool")
        )));
    }

    #[tokio::test]
    async fn domain_prefetch_plus_one_refinement_force_a_report() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call(
                "call-knowledge-first",
                "search_knowledge_base",
                json!({
                    "query": "不存在的机制甲",
                    "version_scope": "current_only",
                    "season": null,
                    "category": null
                }),
            )),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "两次版本知识检索均未取得足够证据。",
                        "本轮不继续猜测检索条件，也不输出未经验证的机制结论。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);

        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-two-knowledge-searches"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.knowledge_searches, 2);
        assert_eq!(result.accounting.model_turns, 2);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "search_knowledge_base"));
        assert!(requests[1].tools.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn parallel_knowledge_searches_are_coalesced_without_budget_termination() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let calls = (0..4)
            .map(|index| ProviderToolCall {
                call_id: format!("call-knowledge-{index}"),
                name: "search_knowledge_base".to_string(),
                arguments: json!({
                    "query": format!("不存在的玩家名{index}"),
                    "version_scope": "current_only",
                    "season": null,
                    "category": null
                }),
            })
            .collect::<Vec<_>>();
        let provider = ScriptedProvider::new(vec![
            Ok(ModelResponse {
                assistant_text: None,
                tool_calls: calls,
                finish_reason: FinishReason::ToolCalls,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "本地版本资料中没有足够信息确认这个名字。",
                        "检索请求已在服务端合并，不再用重复搜索替代证据。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);

        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-parallel-knowledge-searches"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.knowledge_searches, 2);
        assert_eq!(result.accounting.tool_calls, 3);
        assert_ne!(result.status, AgentRunStatus::BudgetExhausted);
        assert_eq!(
            result
                .trace
                .iter()
                .filter(|event| {
                    event.kind == "tool_finished"
                        && event.tool_name.as_deref() == Some("search_knowledge_base")
                })
                .count(),
            2
        );
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "knowledge_searches_coalesced"));
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].tools.is_empty());
        assert_eq!(
            requests[1]
                .messages
                .iter()
                .filter(|message| matches!(message, ModelMessage::ToolResult { .. }))
                .count(),
            6
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn provider_failure_has_fixed_terminal_status() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Err(ProviderError::upstream_status(429))]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-provider-error"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::ProviderFailed);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "model_started"));
        assert!(!result
            .trace
            .iter()
            .any(|event| event.kind == "model_finished"));
        assert_eq!(result.error.unwrap().code, "provider_http_429");
    }

    #[tokio::test]
    async fn empty_provider_report_gets_one_bounded_report_only_retry() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Err(ProviderError::invalid_response_protocol(
                "provider_response_empty",
                "provider response contained neither text nor tool calls",
            )
            .with_usage(TokenUsage {
                input_tokens: 120,
                output_tokens: 7,
                total_tokens: 127,
            })),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "现有证据不足以形成额外结论。",
                        "本轮仅保留已经取得的工具证据。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-empty-provider-report"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.model_turns, 3);
        assert_eq!(result.accounting.total_tokens, 127);
        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].tools.is_empty());
        assert!(requests[2].messages.iter().any(|message| matches!(
            message,
            ModelMessage::User { content } if content.contains("previous provider response was empty")
        )));
        let report = result.report.unwrap();
        assert!(report.content.refusal_reason.is_some());
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_empty_retry"));
    }

    #[tokio::test]
    async fn repeated_empty_provider_report_preserves_tool_evidence() {
        let runtime = AgentRuntime::fixture();
        let empty = || {
            Err(ProviderError::invalid_response_protocol(
                "provider_response_empty",
                "provider response contained neither text nor tool calls",
            ))
        };
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            empty(),
            empty(),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-repeated-empty-provider-report"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        assert_eq!(result.accounting.model_turns, 3);
        assert_eq!(provider.requests().len(), 3);
        assert_eq!(
            result.error.as_ref().unwrap().code,
            "provider_response_empty"
        );
        let report = result.report.unwrap();
        assert_eq!(report.evidence_ids.len(), 2);
        assert_eq!(report.content.findings.len(), 1);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_empty_evidence_preserved"));
    }

    #[tokio::test]
    async fn ungrounded_numeric_report_is_salvaged_without_another_model_turn() {
        let runtime = AgentRuntime::fixture();
        let bad_report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "分析已完成。",
            "findings": [{
                "title": "错误结论",
                "explanation": "该数值没有本次运行证据。",
                "evidence_ids": ["b".repeat(64)],
                "metrics": [{
                    "label": "DPS",
                    "value": 999.0,
                    "unit": "damage_per_second",
                    "evidence_id": "b".repeat(64),
                    "json_pointer": "/result/dps"
                }]
            }],
            "recommendations": [],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(bad_report),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-invalid-evidence"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        assert_eq!(result.accounting.model_turns, 2);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].tools.is_empty());
        let report = result.report.unwrap();
        assert!(!report.content.findings.is_empty());
        assert!(report.content.findings[0]
            .metrics
            .iter()
            .any(|metric| metric.json_pointer == "/result/dps"));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "report_claims_sanitized"));
        assert!(!result
            .trace
            .iter()
            .any(|event| event.kind == "report_repair_requested"));
    }

    #[tokio::test]
    async fn structurally_invalid_json_still_gets_one_bounded_repair() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some("not-json".to_string()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "报告结构无法恢复为证据结论。",
                        "本轮不展示结构损坏的模型内容。",
                    ))
                    .unwrap(),
                ),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-invalid-json"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.model_turns, 3);
        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].tools.is_empty());
        assert!(requests[2].tools.is_empty());
        assert_eq!(requests[2].messages.len(), 1);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "report_repair_requested"));
    }

    #[tokio::test]
    async fn repeated_invalid_json_preserves_tool_evidence_instead_of_dead_ending() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some("not-json".to_string()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some("still-not-json".to_string()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-invalid-json-evidence-fallback"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        let report = result.report.expect("evidence-preserving report");
        assert!(!report.evidence_ids.is_empty());
        assert!(report.content.summary.contains("证据已保留"));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "report_structure_evidence_preserved"));
    }

    #[test]
    fn evidence_fallback_surfaces_readable_knowledge_excerpt() {
        let runtime = AgentRuntime::fixture();
        let plan = select_analysis_plan(
            "英雄阆风悬城老四的业火怎么交？",
            &runtime.fixture_scenario(),
        );
        let mut evidence = super::super::report::EvidenceStore::new();
        evidence.insert(
            "ev-knowledge".to_string(),
            json!({
                "tool_name": "search_knowledge_base",
                "result": {
                    "results": [{
                        "title": "英雄及挑战阆风悬城实战技巧",
                        "heading": "老四业火轴",
                        "snippet": "提前倒数10秒开启业火，并在第二次斩刀前完成窗口。",
                        "fact_eligible": true,
                        "domain_claims": []
                    }]
                }
            }),
        );

        let report = evidence_preserving_provider_fallback(&plan, &evidence, "格式错误")
            .expect("knowledge fallback");
        assert!(report.summary.contains("直接展示"));
        assert!(report.findings[0].explanation.contains("提前倒数10秒"));
        assert_eq!(report.findings[0].evidence_ids, vec!["ev-knowledge"]);
    }

    fn knowledge_fixture() -> (PathBuf, super::super::KnowledgeIndex) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("jx3-orchestrator-knowledge-{nonce}"));
        let relative = "暗影千机（2026）/基础/当前循环.md";
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "---\ntitle: 当前循环\n---\n\n# 当前循环\n\n盾飞阶段需要关注劫刀数量，并避免流血中断。\n",
        )
        .unwrap();
        let reference_relative = "万灵当歌（2023）/白皮书/旧版作者.md";
        let reference_path = root.join(reference_relative);
        fs::create_dir_all(reference_path.parent().unwrap()).unwrap();
        fs::write(
            &reference_path,
            "---\ntitle: 旧版作者\n---\n\n# 人物来源\n\n大家好，世一苍回来了。视频作者 author_a，修改自过崽攻略。\n",
        )
        .unwrap();
        fs::write(
            root.join("_migration-manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "entries": [
                    {
                        "title": "当前循环",
                        "season": "暗影千机（2026）",
                        "category": "基础",
                        "kind": "yuque_document",
                        "source": "https://example.com/current",
                        "output": relative,
                        "source_site": "example.com",
                        "yuque_url": "https://www.yuque.com/sgyxy/cangyun/current",
                        "updated_at": "2026-08-26T00:00:00Z"
                    },
                    {
                        "title": "旧版作者",
                        "season": "万灵当歌（2023）",
                        "category": "白皮书",
                        "kind": "yuque_document",
                        "source": "https://example.com/reference",
                        "output": reference_relative,
                        "source_site": "example.com",
                        "yuque_url": "https://www.yuque.com/sgyxy/cangyun/reference",
                        "updated_at": "2023-08-26T00:00:00Z"
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let index = super::super::KnowledgeIndex::load(&root).unwrap();
        (root, index)
    }
}
