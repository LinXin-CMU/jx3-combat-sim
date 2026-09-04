use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[cfg(test)]
use super::domain::select_analysis_plan;
use super::domain::{
    build_evidence_pack,
    routing_semantic_prototypes, select_analysis_plan_routed, trace_annotation, AnalysisPlanV1,
    AnalysisSurface, AnalysisTaskType, EvidencePackV1,
    SemanticRouteScoreV1,
};
use super::evidence::validate_trace_id;
use super::prompt::agent_prompt;
use super::provider::{
    FinishReason, LlmProvider, ModelMessage, ModelRequest, ProviderToolCall,
    StructuredOutputDefinition, TokenUsage,
};
use super::provider::protocol::MAX_MODEL_REQUEST_BYTES;
use super::registry::{
    AgentToolRegistry, AskUserQuestionArguments, ToolDispatchOutcome, ASK_USER_QUESTION,
    MAX_KNOWLEDGE_SEARCHES,
};
use super::report::{
    cited_evidence_ids, cited_knowledge_sources, parse_and_salvage_report,
    parse_and_validate_report, report_content_json_schema, AgentFindingV1, AgentReportContentV1,
    AgentReportV1, AgentRunAccountingV1, EvidenceStore, GroundedMetricV1,
    AGENT_REPORT_CONTENT_SCHEMA_V1, AGENT_REPORT_SCHEMA_V1,
};
use super::{AgentRuntime, ScenarioSnapshotV1};

pub const AGENT_RUN_SCHEMA_V1: &str = "agent-run/v1";
pub const AGENT_RUN_DEBUG_SCHEMA_V1: &str = "agent-run-debug/v1";
const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_SESSION_CONTEXT_BYTES: usize = 16 * 1024;
const MAX_REPORT_REPAIRS: u32 = 1;
const MAX_EMPTY_RESPONSE_RETRIES: u32 = 1;
const MAX_PROVIDER_PROTOCOL_RETRIES: u32 = 1;
const MAX_TOOL_SELECTION_RETRIES: u32 = 1;
const MODEL_TOOL_OUTPUT_BYTES: usize = 16 * 1024;
const MODEL_EVIDENCE_HANDOFF_BYTES: usize = 24 * 1024;
const MODEL_COMPACTION_TARGET_BYTES: usize = 48 * 1024;
const DEBUG_EVIDENCE_PROJECTION_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
    PartiallyVerified,
    NeedsUserInput,
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
pub struct AgentClarificationV1 {
    pub schema_version: String,
    pub question: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_hint: Option<String>,
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
            max_model_turns: 10,
            max_tool_calls: 12,
            max_simulations: 8,
            max_output_tokens_per_turn: 8192,
            wall_time_ms: 180_000,
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
    /// A typed client hint. It cannot bypass server tool allowlists or grant write access.
    pub task_hint: Option<AnalysisTaskType>,
    /// Client surface context used only as a low-weight prior, never as authorization.
    pub analysis_surface: Option<AnalysisSurface>,
    /// Structured, bounded context captured by the equipment configurator.
    pub equipment_workspace: Option<super::EquipmentWorkspaceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunErrorV1 {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunDebugV1 {
    pub schema_version: String,
    pub question: String,
    pub analysis_plan: AnalysisPlanV1,
    pub exposed_tools: Vec<String>,
    pub limits: AgentRunLimits,
    pub request_metrics: Vec<AgentModelRequestDebugV1>,
    pub tool_calls: Vec<AgentToolCallDebugV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_pack: Option<EvidencePackV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_projection: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentModelRequestDebugV1 {
    pub turn: u32,
    pub mode: String,
    pub request_bytes: usize,
    pub max_request_bytes: usize,
    pub original_message_bytes: usize,
    pub compacted_message_bytes: usize,
    pub message_count: usize,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentToolCallDebugV1 {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub evidence_ids: Vec<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub budget_exhausted: bool,
    pub server_initiated: bool,
    pub reused: bool,
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
    pub clarification: Option<AgentClarificationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentRunErrorV1>,
    pub trace: Vec<AgentTraceEventV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<AgentRunDebugV1>,
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
    exposed_tools: Vec<String>,
    limits: AgentRunLimits,
    evidence_pack: Option<EvidencePackV1>,
    evidence_projection: Option<Value>,
    request_metrics: Vec<AgentModelRequestDebugV1>,
    tool_calls: Vec<AgentToolCallDebugV1>,
}

impl TraceCollector {
    fn new(sink: Option<AgentTraceSink>, plan: AnalysisPlanV1, limits: AgentRunLimits) -> Self {
        Self {
            events: Vec::new(),
            sink,
            plan,
            exposed_tools: Vec::new(),
            limits,
            evidence_pack: None,
            evidence_projection: None,
            request_metrics: Vec::new(),
            tool_calls: Vec::new(),
        }
    }

    fn set_exposed_tools(&mut self, exposed_tools: Vec<String>) {
        self.exposed_tools = exposed_tools;
    }

    fn refresh_evidence_pack(&mut self, evidence: &EvidenceStore) {
        self.evidence_pack = Some(build_evidence_pack(&self.plan, evidence));
        self.evidence_projection = Some(debug_evidence_projection(evidence));
    }

    fn record_tool_call(
        &mut self,
        call_id: &str,
        tool_name: &str,
        arguments: &Value,
        outcome: &ToolDispatchOutcome,
        server_initiated: bool,
        reused: bool,
    ) {
        self.tool_calls.push(AgentToolCallDebugV1 {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: redact_debug_value(arguments),
            evidence_ids: outcome.evidence_ids.clone(),
            ok: outcome.output.get("ok").and_then(Value::as_bool) == Some(true),
            code: outcome
                .output
                .pointer("/error/code")
                .and_then(Value::as_str)
                .map(str::to_string),
            budget_exhausted: outcome.budget_exhausted,
            server_initiated,
            reused,
        });
    }

    fn record_model_request(
        &mut self,
        turn: u32,
        mode: &str,
        request: &ModelRequest,
        original_message_bytes: usize,
        compacted_message_bytes: usize,
    ) {
        self.request_metrics.push(AgentModelRequestDebugV1 {
            turn,
            mode: mode.to_string(),
            request_bytes: request_bytes(request),
            max_request_bytes: MAX_MODEL_REQUEST_BYTES,
            original_message_bytes,
            compacted_message_bytes,
            message_count: request.messages.len(),
            tool_count: request.tools.len(),
        });
    }

    fn debug(&self, question: &str) -> AgentRunDebugV1 {
        AgentRunDebugV1 {
            schema_version: AGENT_RUN_DEBUG_SCHEMA_V1.to_string(),
            question: super::session::redact_sensitive_text(question),
            analysis_plan: self.plan.clone(),
            exposed_tools: self.exposed_tools.clone(),
            limits: self.limits.clone(),
            request_metrics: self.request_metrics.clone(),
            tool_calls: self.tool_calls.clone(),
            evidence_pack: self.evidence_pack.clone(),
            evidence_projection: self.evidence_projection.clone(),
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

    fn push_checkpoint(
        &mut self,
        kind: &str,
        label: &str,
        overview: String,
        code: Option<String>,
        evidence_ids: Vec<String>,
    ) {
        let annotation = trace_annotation(&self.plan, kind, None, evidence_ids.len());
        let event = AgentTraceEventV1 {
            sequence: self.events.len() as u32 + 1,
            kind: kind.to_string(),
            tool_name: None,
            evidence_ids,
            code,
            stage_id: Some(annotation.stage_id),
            label: Some(label.to_string()),
            overview: Some(overview),
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

fn redact_debug_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(super::session::redact_sensitive_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_debug_value).collect()),
        Value::Object(items) => Value::Object(
            items
                .iter()
                .map(|(key, value)| (key.clone(), redact_debug_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn debug_evidence_projection(evidence: &EvidenceStore) -> Value {
    let mut items = evidence
        .values()
        .map(|envelope| {
            let wrapper = serde_json::json!({
                "schema_version": "agent-tool-result/v1",
                "ok": true,
                "tool_name": envelope.get("tool_name"),
                "evidence_ids": [envelope.get("evidence_id")],
                "evidence": [envelope]
            });
            let projected = model_tool_output(&wrapper)
                .get("evidence")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            (evidence_priority(envelope), redact_debug_value(&projected))
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|(priority, item)| {
        (
            *priority,
            item.get("evidence_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    });
    let mut included = Vec::new();
    let mut omitted = Vec::new();
    for (_, item) in items {
        let mut trial = included.clone();
        trial.push(item.clone());
        if serde_json::to_vec(&trial)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX)
            <= DEBUG_EVIDENCE_PROJECTION_BYTES
        {
            included.push(item);
        } else if let Some(id) = item.get("evidence_id").and_then(Value::as_str) {
            omitted.push(id.to_string());
        }
    }
    serde_json::json!({
        "schema_version": "agent-debug-evidence/v1",
        "items": included,
        "omitted_evidence_ids": omitted,
        "byte_budget": DEBUG_EVIDENCE_PROJECTION_BYTES,
        "boundary": "Bounded and redacted shareable projection; immutable full evidence remains server-side."
    })
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
    let prompt = agent_prompt();
    let semantic_prototypes = routing_semantic_prototypes();
    let semantic_result = if input.task_hint.is_none() {
        runtime.knowledge().map(|knowledge| {
            let texts = semantic_prototypes
                .iter()
                .map(|(_, prototype)| (*prototype).to_string())
                .collect::<Vec<_>>();
            knowledge.semantic_route_similarities(&input.question, &texts)
        })
    } else {
        None
    };
    let semantic_scores = semantic_result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|similarities| {
            semantic_prototypes
                .iter()
                .zip(similarities)
                .map(|((task_type, _), similarity_millis)| SemanticRouteScoreV1 {
                    task_type: *task_type,
                    similarity_millis: *similarity_millis,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut analysis_plan = select_analysis_plan_routed(
        &input.question,
        input.task_hint,
        input.analysis_surface,
        &semantic_scores,
        input.session_playbook_id.as_deref(),
        &input.scenario,
    );
    if let Some(Err(code)) = semantic_result {
        analysis_plan
            .routing_signals
            .push(format!("semantic_router_fallback:{code}"));
    }
    let mut accounting = AgentRunAccountingV1::default();
    let mut trace = TraceCollector::new(event_sink, analysis_plan.clone(), limits.clone());
    record_replay(
        &replay_sink,
        "run_input",
        serde_json::json!({
            "question": &input.question,
            "scenario": &input.scenario,
            "session_context": &input.session_context,
            "session_playbook_id": &input.session_playbook_id,
            "task_hint": &input.task_hint,
            "analysis_surface": &input.analysis_surface,
            "equipment_workspace": &input.equipment_workspace,
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

    let mut registry = AgentToolRegistry::new_with_context(
        &input.scenario,
        runtime,
        limits.max_simulations,
        runtime.knowledge(),
        input.equipment_workspace.clone(),
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
    let equipment_focus_available = input
        .equipment_workspace
        .as_ref()
        .and_then(|workspace| workspace.focus.as_ref())
        .is_some();
    let tools = definitions
        .into_iter()
        .filter(|tool| {
            !knowledge_only_client_scope
                || matches!(
                    tool.name.as_str(),
                    "get_current_scenario" | "search_knowledge_base" | ASK_USER_QUESTION
                )
        })
        .filter(|tool| tool.name != "compare_focused_equipment" || equipment_focus_available)
        .collect::<Vec<_>>();
    trace.set_exposed_tools(tools.iter().map(|tool| tool.name.clone()).collect());
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
    let mut repairs = 0;
    let mut empty_response_retries = 0;
    let mut provider_protocol_retries = 0;
    let mut tool_selection_retries = 0;
    let mut repair_message = None;
    let mut final_report_only = false;
    let mut deterministic_tool_cache = HashMap::<String, ToolDispatchOutcome>::new();
    trace.push(
        "analysis_context_prepared",
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
    trace.record_tool_call(
        PREFETCH_CALL_ID,
        PREFETCH_TOOL,
        &serde_json::json!({}),
        &prefetched,
        true,
        false,
    );
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
        reasoning_content: None,
    });
    messages.push(ModelMessage::ToolResult {
        call_id: PREFETCH_CALL_ID.to_string(),
        output: model_tool_output(&prefetched.output),
    });

    // Page state is trusted context. Candidate searches and comparisons remain
    // model-selected actions in the normal planning loop.
    if input.equipment_workspace.is_some() {
        const EQUIPMENT_INSPECT_CALL_ID: &str = "server-prefetch-equipment";
        const EQUIPMENT_INSPECT_TOOL: &str = "inspect_equipment_workspace";
        trace.push("tool_started", Some(EQUIPMENT_INSPECT_TOOL.to_string()), Vec::new(), Some("server_equipment_prefetch".to_string()));
        let inspected = registry.dispatch(&input.run_id, EQUIPMENT_INSPECT_TOOL, serde_json::json!({}));
        trace.record_tool_call(EQUIPMENT_INSPECT_CALL_ID, EQUIPMENT_INSPECT_TOOL, &serde_json::json!({}), &inspected, true, false);
        accounting.tool_calls += 1;
        trace.push("tool_finished", Some(EQUIPMENT_INSPECT_TOOL.to_string()), inspected.evidence_ids.clone(), inspected.output.pointer("/error/code").and_then(|value| value.as_str()).map(str::to_string));
        record_replay(&replay_sink, "tool_dispatch", serde_json::json!({
            "call_id": EQUIPMENT_INSPECT_CALL_ID, "tool_name": EQUIPMENT_INSPECT_TOOL,
            "arguments": {}, "output": &inspected.output, "evidence_ids": &inspected.evidence_ids,
            "budget_exhausted": inspected.budget_exhausted, "server_initiated": true,
        }));
        messages.push(ModelMessage::Assistant { content: None, tool_calls: vec![ProviderToolCall {
            call_id: EQUIPMENT_INSPECT_CALL_ID.to_string(), name: EQUIPMENT_INSPECT_TOOL.to_string(), arguments: serde_json::json!({}),
        }], reasoning_content: None });
        messages.push(ModelMessage::ToolResult { call_id: EQUIPMENT_INSPECT_CALL_ID.to_string(), output: model_tool_output(&inspected.output) });

    }

    trace.refresh_evidence_pack(registry.evidence());
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
            if let Some(content) = recover_report_from_messages(
                &input.question,
                &analysis_plan,
                &messages,
                registry.evidence(),
            )
            .or_else(|| {
                evidence_preserving_provider_fallback(
                    &analysis_plan,
                    registry.evidence(),
                    "本轮达到时限；已保留时限内形成的可验证分析。",
                )
            }) {
                return terminal_with_report(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::PartiallyVerified,
                    accounting,
                    content,
                    Some(fixed_error(
                        "run_timeout",
                        "Agent run exceeded its wall-time limit",
                    )),
                    trace,
                    started,
                    &registry,
                );
            }
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
            if let Some(content) = recover_report_from_messages(
                &input.question,
                &analysis_plan,
                &messages,
                registry.evidence(),
            )
            .or_else(|| {
                evidence_preserving_provider_fallback(
                    &analysis_plan,
                    registry.evidence(),
                    "模型已用完本轮规划次数；下方保留已取得的本地证据，不把未完成的解释伪装成结论。",
                )
            }) {
                trace.push(
                    "model_turn_budget_evidence_preserved",
                    None,
                    cited_evidence_ids(&content),
                    Some("model_turn_budget".to_string()),
                );
                return terminal_with_report(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::PartiallyVerified,
                    accounting,
                    content,
                    Some(fixed_error(
                        "model_turn_budget",
                        "Model turn budget is exhausted",
                    )),
                    trace,
                    started,
                    &registry,
                );
            }
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
        let repair_content = repair_message.take();
        let original_message_bytes = serde_json::to_vec(&messages)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        let mut request_messages = repair_content
            .map(|content| vec![ModelMessage::User { content }])
            .unwrap_or_else(|| compact_transcript_messages(&messages));
        let compacted_message_bytes = serde_json::to_vec(&request_messages)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        if compacted_message_bytes < original_message_bytes {
            trace.push(
                "model_context_compacted",
                None,
                registry.evidence().keys().cloned().collect(),
                Some(format!(
                    "message_bytes_{original_message_bytes}_to_{compacted_message_bytes}"
                )),
            );
        }
        let mut request = ModelRequest {
            instructions: prompt.instructions.to_string(),
            messages: request_messages.clone(),
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
        if request_bytes(&request) > MODEL_COMPACTION_TARGET_BYTES && !is_repair {
            for evidence_bytes in [MODEL_EVIDENCE_HANDOFF_BYTES, 12 * 1024, 6 * 1024] {
                request_messages = compact_handoff_messages(
                    &input,
                    &messages,
                    registry.evidence(),
                    evidence_bytes,
                );
                request.messages = request_messages.clone();
                if request_bytes(&request) <= MODEL_COMPACTION_TARGET_BYTES {
                    trace.push(
                        "model_context_handoff",
                        None,
                        registry.evidence().keys().cloned().collect(),
                        Some(format!(
                            "bounded_request_{}_bytes",
                            request_bytes(&request)
                        )),
                    );
                    break;
                }
            }
        }
        if request_bytes(&request) > MAX_MODEL_REQUEST_BYTES {
            record_replay(
                &replay_sink,
                "model_context_limit",
                serde_json::json!({
                    "code": "model_request_too_large",
                    "request_bytes": request_bytes(&request),
                    "max_bytes": MAX_MODEL_REQUEST_BYTES,
                    "evidence_ids": registry.evidence().keys().collect::<Vec<_>>(),
                }),
            );
            if let Some(content) = evidence_preserving_provider_fallback(
                &analysis_plan,
                registry.evidence(),
                "模型上下文达到本地硬上限；未继续发送超长请求，下方保留已经取得的可验证证据。",
            ) {
                trace.push(
                    "model_context_limit_evidence_preserved",
                    None,
                    cited_evidence_ids(&content),
                    Some("model_request_too_large".to_string()),
                );
                return terminal_with_report(
                    provider,
                    &input,
                    &prompt,
                    AgentRunStatus::PartiallyVerified,
                    accounting,
                    content,
                    Some(fixed_error(
                        "model_request_too_large",
                        "Model request exceeded the local context budget",
                    )),
                    trace,
                    started,
                    &registry,
                );
            }
            let content = refusal_content(
                "当前上下文超过本地安全预算，已停止发送超长请求。",
                "请缩小问题范围后重试；本轮没有向供应商发送超限内容。",
            );
            return terminal_with_report(
                provider,
                &input,
                &prompt,
                AgentRunStatus::EvidenceInsufficient,
                accounting,
                content,
                Some(fixed_error(
                    "model_request_too_large",
                    "Model request exceeded the local context budget",
                )),
                trace,
                started,
                &registry,
            );
        }
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

        let request_mode = if is_repair {
            "report_repair"
        } else if final_report_only {
            "final_report"
        } else {
            "tool_selection"
        };
        let final_message_bytes = serde_json::to_vec(&request.messages)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        trace.record_model_request(
            accounting.model_turns + 1,
            request_mode,
            &request,
            original_message_bytes,
            final_message_bytes,
        );
        trace.push(
            "model_started",
            None,
            Vec::new(),
            Some(request_mode.to_string()),
        );
        record_replay(
            &replay_sink,
            "model_request",
            serde_json::to_value(&request).unwrap_or_else(|_| serde_json::json!({})),
        );
        record_replay(
            &replay_sink,
            "model_context_budget",
            serde_json::json!({
                "request_bytes": request_bytes(&request),
                "max_request_bytes": MAX_MODEL_REQUEST_BYTES,
                "message_count": request.messages.len(),
                "tool_count": request.tools.len(),
            }),
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
                        "request_bytes": request_bytes(&request),
                        "max_request_bytes": MAX_MODEL_REQUEST_BYTES,
                        "usage": &error.usage,
                    }),
                );
                add_usage(&mut accounting, &error.usage);
                if error.code == "provider_balance_insufficient" {
                    let content = refusal_content(
                        "模型服务当前不可用，尚未生成战斗分析。",
                        "供应商返回余额不足；已取得的攻略片段不会冒充本次循环诊断。",
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::ProviderFailed,
                        accounting,
                        content,
                        Some(fixed_error(error.code, error.message)),
                        trace,
                        started,
                        &registry,
                    );
                }
                if error.code == "provider_tool_arguments_invalid"
                    && provider_protocol_retries < MAX_PROVIDER_PROTOCOL_RETRIES
                    && accounting.model_turns < limits.max_model_turns
                {
                    provider_protocol_retries += 1;
                    let can_finish_from_evidence = registry.evidence().len() > 1;
                    final_report_only = can_finish_from_evidence;
                    messages.push(ModelMessage::User { content: if can_finish_from_evidence {
                        "已有证据足以形成有边界的回答。请直接输出 AgentReportContentV1，围绕用户原问题组织结论。".to_string()
                    } else {
                        "请重新选择下一项动作，并为每个工具调用提供一个 JSON 对象；零参数工具使用 {}。".to_string()
                    }});
                    trace.push(
                        "provider_tool_arguments_retry",
                        None,
                        Vec::new(),
                        Some(if can_finish_from_evidence {
                            "finish_from_registered_evidence".to_string()
                        } else {
                            "retry_tool_selection".to_string()
                        }),
                    );
                    continue;
                }
                if error.code == "provider_response_empty" {
                    if empty_response_retries < MAX_EMPTY_RESPONSE_RETRIES
                        && !registry.evidence().is_empty()
                        && accounting.model_turns < limits.max_model_turns
                    {
                        empty_response_retries += 1;
                        final_report_only = true;
                        messages.push(ModelMessage::User {
                            content: "Return one complete AgentReportContentV1 JSON object from the evidence already present in this transcript. Express remaining uncertainty in limitations.".to_string(),
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
                if !registry.evidence().is_empty() {
                    let recovered = recover_report_from_messages(
                        &input.question,
                        &analysis_plan,
                        &messages,
                        registry.evidence(),
                    )
                    .or_else(|| {
                        evidence_preserving_provider_fallback(
                            &analysis_plan,
                            registry.evidence(),
                            "模型后续请求失败；下方保留失败前已经取得的可验证证据。",
                        )
                    });
                    if let Some(content) = recovered {
                        trace.push(
                            "provider_failure_evidence_preserved",
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

        if response.tool_calls.is_empty()
            && tools_available
            && looks_like_unexecuted_plan(response.assistant_text.as_deref())
            && accounting.model_turns < limits.max_model_turns
        {
            messages.push(ModelMessage::User {
                content: "你刚才形成了下一步计划，但尚未执行动作。现在直接调用最有价值的工具；若现有证据已经足够，则直接提交最终报告。".to_string(),
            });
            trace.push_checkpoint(
                "model_plan_continued",
                "继续执行模型计划",
                "模型已经选出下一步，正在把计划转为实际工具动作。".to_string(),
                Some("unexecuted_action_plan".to_string()),
                Vec::new(),
            );
            continue;
        }

        if !response.tool_calls.is_empty() {
            trace.push_decision_summary(public_decision_summary(
                response.assistant_text.as_deref(),
                &response.tool_calls,
            ));
            if response.tool_calls.len() == 1
                && response.tool_calls[0].name == ASK_USER_QUESTION
            {
                let call = &response.tool_calls[0];
                match parse_clarification(&call.arguments) {
                    Ok(clarification) => {
                        accounting.tool_calls += 1;
                        let outcome = ToolDispatchOutcome {
                            output: serde_json::json!({
                                "schema_version": "agent-tool-result/v1",
                                "ok": true,
                                "tool_name": ASK_USER_QUESTION,
                                "paused": true,
                            }),
                            evidence_ids: Vec::new(),
                            budget_exhausted: false,
                        };
                        trace.record_tool_call(
                            &call.call_id,
                            &call.name,
                            &call.arguments,
                            &outcome,
                            false,
                            false,
                        );
                        trace.push_checkpoint(
                            "needs_user_input",
                            "等待用户补充",
                            clarification.question.clone(),
                            Some("ask_user_question".to_string()),
                            Vec::new(),
                        );
                        record_replay(
                            &replay_sink,
                            "clarification_requested",
                            serde_json::to_value(&clarification)
                                .unwrap_or_else(|_| serde_json::json!({})),
                        );
                        return terminal_needs_user_input(
                            provider,
                            &input,
                            &prompt,
                            accounting,
                            clarification,
                            trace,
                            started,
                            &registry,
                        );
                    }
                    Err(message) => {
                        messages.push(ModelMessage::Assistant {
                            content: response.assistant_text,
                            tool_calls: response.tool_calls.clone(),
                            reasoning_content: response.reasoning_content,
                        });
                        messages.push(ModelMessage::ToolResult {
                            call_id: call.call_id.clone(),
                            output: serde_json::json!({
                                "schema_version": "agent-tool-result/v1",
                                "ok": false,
                                "tool_name": ASK_USER_QUESTION,
                                "error": {"code": "invalid_clarification", "message": message},
                            }),
                        });
                        trace.push(
                            "tool_rejected",
                            Some(ASK_USER_QUESTION.to_string()),
                            Vec::new(),
                            Some("invalid_clarification".to_string()),
                        );
                        continue;
                    }
                }
            }
            let requested_knowledge_calls = response
                .tool_calls
                .iter()
                .filter(|call| call.name == "search_knowledge_base")
                .count() as u32;
            let available_knowledge_calls =
                MAX_KNOWLEDGE_SEARCHES.saturating_sub(registry.used_knowledge_searches());
            let coalesced_knowledge_calls =
                requested_knowledge_calls.saturating_sub(available_knowledge_calls);
            let effective_tool_calls =
                (response.tool_calls.len() as u32).saturating_sub(coalesced_knowledge_calls);
            if accounting.tool_calls.saturating_add(effective_tool_calls) > limits.max_tool_calls {
                if accounting.model_turns < limits.max_model_turns {
                    final_report_only = true;
                    messages.push(ModelMessage::User {
                        content: "The remaining tool budget is reserved for the report. Finish from registered evidence and place unfinished checks in limitations.".to_string(),
                    });
                    trace.push(
                        "budget_limit_reached",
                        None,
                        Vec::new(),
                        Some("tool_call_batch_trimmed_to_report".to_string()),
                    );
                    continue;
                }
                if let Some(content) = evidence_preserving_provider_fallback(
                    &analysis_plan,
                    registry.evidence(),
                    "模型请求的工具超过本轮上限；下方保留预算内已经取得的本地证据。",
                ) {
                    trace.push(
                        "tool_call_budget_evidence_preserved",
                        None,
                        cited_evidence_ids(&content),
                        Some("tool_call_budget".to_string()),
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::PartiallyVerified,
                        accounting,
                        content,
                        Some(fixed_error(
                            "tool_call_budget",
                            "Tool call budget is exhausted",
                        )),
                        trace,
                        started,
                        &registry,
                    );
                }
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
            messages.push(ModelMessage::Assistant {
                content: response.assistant_text,
                tool_calls: response.tool_calls.clone(),
                reasoning_content: response.reasoning_content,
            });
            let mut knowledge_calls_processed = 0_u32;
            let mut knowledge_calls_coalesced = 0_u32;
            for call in response.tool_calls {
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
                        output: model_tool_output(&coalesced.output),
                    });
                    continue;
                }
                if call.name == "search_knowledge_base" {
                    knowledge_calls_processed += 1;
                }
                accounting.tool_calls += 1;
                trace.push(
                    "tool_started",
                    Some(call.name.clone()),
                    Vec::new(),
                    None,
                );
                let arguments = call.arguments.clone();
                let cache_key = is_reusable_deterministic_tool(&call.name)
                    .then(|| {
                        super::hash::canonical_sha256(&serde_json::json!({
                            "tool": &call.name,
                            "arguments": &arguments,
                        }))
                        .ok()
                    })
                    .flatten();
                let cached = cache_key
                    .as_ref()
                    .and_then(|key| deterministic_tool_cache.get(key))
                    .cloned();
                let reused = cached.is_some();
                let outcome = if let Some(cached) = cached {
                    cached
                } else {
                    registry.dispatch(&input.run_id, &call.name, call.arguments.clone())
                };
                trace.record_tool_call(
                    &call.call_id,
                    &call.name,
                    &arguments,
                    &outcome,
                    false,
                    reused,
                );
                if !reused
                    && !outcome.budget_exhausted
                    && outcome
                        .output
                        .get("ok")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                {
                    if let Some(key) = cache_key {
                        deterministic_tool_cache.insert(key, outcome.clone());
                    }
                }
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
                        "coalesced": reused,
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
                        .map(str::to_string)
                        .or_else(|| reused.then(|| "deterministic_result_reused".to_string())),
                );
                messages.push(ModelMessage::ToolResult {
                    call_id: call.call_id,
                    output: model_tool_output(&outcome.output),
                });
                if outcome.budget_exhausted {
                    let knowledge_budget = call.name == "search_knowledge_base";
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
                    if !knowledge_budget {
                        final_report_only = true;
                        messages.push(ModelMessage::User {
                            content: "The simulation budget is complete. Synthesize the result from registered evidence and include any unfinished experiment in limitations.".to_string(),
                        });
                        break;
                    }
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
            trace.refresh_evidence_pack(registry.evidence());
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
        trace.refresh_evidence_pack(registry.evidence());
        trace.push_checkpoint(
            "report_validation_started",
            "校验报告证据",
            "核对结构、数值、单位、来源和证据引用。".to_string(),
            None,
            evidence_pack.evidence_ids.clone(),
        );
        trace.push("validating", None, Vec::new(), None);
        let raw = response.assistant_text.as_deref().unwrap_or_default();
        match parse_and_validate_report(raw, registry.evidence()) {
            Ok(validated) => {
                trace.push_checkpoint(
                    "report_validation_passed",
                    "证据校验通过",
                    "报告中的可验证事实均已绑定本轮证据。".to_string(),
                    Some("evidence_contract_satisfied".to_string()),
                    cited_evidence_ids(&validated.content),
                );
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
            Err(error) => {
                match parse_and_salvage_report(raw, registry.evidence()) {
                Ok(salvaged) => {
                    trace.push_checkpoint(
                        "report_validation_salvaged",
                        "保留已验证内容",
                        "已移除无法绑定本轮证据的报告字段。".to_string(),
                        Some("evidence_contract_salvaged".to_string()),
                        cited_evidence_ids(&salvaged.content),
                    );
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
                        "Correct the rejected output into one AgentReportContentV1 JSON object. Validation code: {}. Detail: {}. Use the registered evidence ids, metric values, units and JSON Pointers. Preserve supported analysis and repair the invalid fields. `limitations` is an array of strings; `refusal_reason` is a string or null; findings include `metrics`; rotation changes include `edit_operation` and `evidence_ids`. Write concise, natural Chinese.\n\nREPAIR_EVIDENCE_BEGIN\n{}\nREPAIR_EVIDENCE_END\n\nREJECTED_OUTPUT_BEGIN\n{}\nREJECTED_OUTPUT_END",
                    error.code, error.message, repair_evidence, raw
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
                    if let Some(content) = model_judgment_with_evidence_fallback(
                        raw,
                        &analysis_plan,
                        registry.evidence(),
                        "模型正文已保留；结构化证据附录未完全通过协议校验。",
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
            }
            }
        }
    }
}

fn recover_report_from_messages(
    _question: &str,
    _plan: &AnalysisPlanV1,
    messages: &[ModelMessage],
    evidence: &EvidenceStore,
) -> Option<AgentReportContentV1> {
    messages.iter().rev().find_map(|message| {
        let ModelMessage::Assistant {
            content: Some(raw),
            ..
        } = message
        else {
            return None;
        };
        parse_and_validate_report(raw, evidence)
            .map(|validated| validated.content)
            .or_else(|_| parse_and_salvage_report(raw, evidence).map(|salvaged| salvaged.content))
            .ok()
    })
}

fn model_tool_output(output: &Value) -> Value {
    let mut projected = output.clone();
    if let Some(evidence) = projected.get_mut("evidence").and_then(Value::as_array_mut) {
        for envelope in evidence {
            let tool_name = envelope
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(result) = envelope.get_mut("result") {
                project_tool_result(&tool_name, result);
                bound_json_value(result, 32, 1_600, 0);
            }
        }
    }
    if serde_json::to_vec(&projected)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
        > MODEL_TOOL_OUTPUT_BYTES
    {
        bound_json_value(&mut projected, 12, 800, 0);
    }
    if serde_json::to_vec(&projected)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
        > MODEL_TOOL_OUTPUT_BYTES
    {
        let evidence = projected
            .get("evidence")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| {
                serde_json::json!({
                    "evidence_id": item.get("evidence_id"),
                    "tool_name": item.get("tool_name"),
                    "result": compact_result_facts(item.get("result")),
                })
            })
            .collect::<Vec<_>>();
        projected = serde_json::json!({
            "schema_version": projected.get("schema_version"),
            "ok": projected.get("ok"),
            "tool_name": projected.get("tool_name"),
            "evidence_ids": projected.get("evidence_ids"),
            "evidence": evidence,
            "model_projection": "full evidence retained by server"
        });
    }
    projected
}

fn project_tool_result(tool_name: &str, result: &mut Value) {
    match tool_name {
        "get_current_scenario" => {
            if let Some(items) = result
                .pointer_mut("/rotation_input/manual_operations")
                .and_then(Value::as_array_mut)
            {
                items.truncate(16);
            }
        }
        "search_knowledge_base" => {
            if let Some(items) = result.get_mut("results").and_then(Value::as_array_mut) {
                items.truncate(3);
                for item in items {
                    if let Some(object) = item.as_object_mut() {
                        object.retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "document_id"
                                    | "title"
                                    | "heading"
                                    | "snippet"
                                    | "category"
                                    | "season"
                                    | "source_site"
                                    | "source_url"
                                    | "source_updated_at"
                                    | "yuque_url"
                                    | "version_match"
                                    | "version_warning"
                                    | "fact_eligible"
                                    | "quality"
                                    | "reference_entities"
                                    | "domain_claims"
                            )
                        });
                    }
                }
            }
        }
        "analyze_timeline" => {
            let metric_catalog = timeline_metric_catalog(result);
            if let Some(buffs) = result
                .get_mut("buff_coverage")
                .and_then(Value::as_array_mut)
            {
                for buff in buffs {
                    if let Some(object) = buff.as_object_mut() {
                        object.remove("intervals");
                    }
                }
            }
            for key in ["gcd_gaps", "cd_waits"] {
                if let Some(items) = result.get_mut(key).and_then(Value::as_array_mut) {
                    items.truncate(8);
                }
            }
            if let Some(object) = result.as_object_mut() {
                object.insert("metric_catalog".to_string(), metric_catalog);
            }
        }
        "simulate_scenario" => {
            let ranked = {
                let derived = ranked_damage_sources(result, 12);
                if derived.is_empty() {
                    result
                        .get("ranked_damage_sources")
                        .and_then(Value::as_array)
                        .map(|items| items.iter().take(12).cloned().collect())
                        .unwrap_or_default()
                } else {
                    derived
                }
            };
            if let Some(object) = result.as_object_mut() {
                // The full unordered skill list remains in immutable server-side
                // evidence for validation. The model sees a compact damage-ranked
                // view so low-charge variants cannot eclipse the actual carrier.
                object.remove("skills");
                object.insert("ranked_damage_sources".to_string(), Value::Array(ranked));
            }
        }
        "compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios" => {
            if let Some(candidates) = result.get_mut("candidates").and_then(Value::as_array_mut) {
                candidates.truncate(3);
                for candidate in candidates {
                    if let Some(deltas) = candidate
                        .get_mut("skill_deltas")
                        .and_then(Value::as_array_mut)
                    {
                        deltas.sort_by(|left, right| {
                            let left = left
                                .get("damage_delta")
                                .and_then(Value::as_f64)
                                .unwrap_or(0.0)
                                .abs();
                            let right = right
                                .get("damage_delta")
                                .and_then(Value::as_f64)
                                .unwrap_or(0.0)
                                .abs();
                            right.total_cmp(&left)
                        });
                        deltas.truncate(12);
                    }
                }
            }
        }
        _ => {}
    }
}

fn ranked_damage_sources(result: &Value, limit: usize) -> Vec<Value> {
    let mut ranked = result
        .get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, skill)| {
            let name = skill.get("name").and_then(Value::as_str)?;
            let damage_share = skill.get("damage_share").and_then(Value::as_f64)?;
            let total_damage = skill.get("total_damage").and_then(Value::as_f64)?;
            let event_count = skill.get("event_count").and_then(Value::as_u64);
            (total_damage > 0.0).then(|| {
                serde_json::json!({
                    "name": name,
                    "damage_share": damage_share,
                    "total_damage": total_damage,
                    "event_count": event_count,
                    "damage_share_json_pointer": format!("/result/skills/{index}/damage_share"),
                    "total_damage_json_pointer": format!("/result/skills/{index}/total_damage"),
                    "event_count_json_pointer": format!("/result/skills/{index}/event_count"),
                })
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .get("total_damage")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .total_cmp(
                &left
                    .get("total_damage")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            )
    });
    ranked.truncate(limit);
    ranked
}

fn timeline_metric_catalog(result: &Value) -> Value {
    let scalar = |label: &str, pointer: &str| {
        result.pointer(pointer).and_then(Value::as_f64).map(|value| {
            serde_json::json!({"label": label, "value": value, "json_pointer": pointer})
        })
    };
    let mut metrics = [
        ("主技能空档次数", "/diagnostic_profile/cadence_gaps/count"),
        (
            "主技能空档总时长",
            "/diagnostic_profile/cadence_gaps/total_seconds",
        ),
        (
            "冷却等待次数",
            "/diagnostic_profile/cooldown_waits/count",
        ),
        (
            "冷却等待总时长",
            "/diagnostic_profile/cooldown_waits/total_seconds",
        ),
        ("怒气触顶采样", "/rage/at_cap_observations"),
        ("怒气采样总数", "/rage/sample_count"),
    ]
    .into_iter()
    .filter_map(|(label, relative)| {
        scalar(label, relative).map(|mut item| {
            item["json_pointer"] = Value::String(format!("/result{relative}"));
            item
        })
    })
    .collect::<Vec<_>>();
    if let Some(coverage) = result.get("buff_coverage").and_then(Value::as_array) {
        metrics.extend(coverage.iter().enumerate().filter_map(|(index, item)| {
            let name = item.get("name").and_then(Value::as_str)?;
            let value = item.get("coverage_percent").and_then(Value::as_f64)?;
            Some(serde_json::json!({
                "label": format!("{name}时间覆盖率"),
                "value": value,
                "json_pointer": format!("/result/buff_coverage/{index}/coverage_percent"),
            }))
        }));
    }
    Value::Array(metrics)
}

fn bound_json_value(value: &mut Value, array_limit: usize, string_limit: usize, depth: usize) {
    if depth > 16 {
        *value = Value::String("[depth bounded]".to_string());
        return;
    }
    match value {
        Value::String(text) if text.chars().count() > string_limit => {
            *text = format!("{}…", text.chars().take(string_limit).collect::<String>());
        }
        Value::Array(items) => {
            items.truncate(array_limit);
            for item in items {
                bound_json_value(item, array_limit, string_limit, depth + 1);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                bound_json_value(item, array_limit, string_limit, depth + 1);
            }
        }
        _ => {}
    }
}

fn compact_result_facts(result: Option<&Value>) -> Value {
    let Some(result) = result.and_then(Value::as_object) else {
        return serde_json::json!({});
    };
    let mut facts = serde_json::Map::new();
    for key in [
        "dps",
        "total_damage",
        "fight_time",
        "fingerprint_hex",
        "diagnostic_profile",
        "rage",
        "stance",
        "selection",
        "results",
        "candidates",
        "rotation_input",
        "total_items",
        "query",
        "matches",
        "window",
        "next_start_index",
        "game_version",
        "mount",
        "network_delay_ms",
        "haste_level",
    ] {
        if let Some(value) = result.get(key) {
            facts.insert(key.to_string(), value.clone());
        }
    }
    let mut facts = Value::Object(facts);
    bound_json_value(&mut facts, 6, 600, 0);
    restore_ordered_macro_statements(&Value::Object(result.clone()), &mut facts);
    facts
}

fn restore_ordered_macro_statements(source: &Value, target: &mut Value) {
    const MAX_MODEL_MACRO_STATEMENTS: usize = 32;
    let Some(source_items) = source
        .pointer("/rotation_input/macro_statements")
        .and_then(Value::as_array)
    else {
        return;
    };
    let Some(rotation) = target
        .get_mut("rotation_input")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    rotation.insert(
        "macro_statements".to_string(),
        Value::Array(
            source_items
                .iter()
                .take(MAX_MODEL_MACRO_STATEMENTS)
                .map(|item| {
                    let Some(fields) = item.as_object() else {
                        return item.clone();
                    };
                    let mut compact = serde_json::Map::new();
                    for key in [
                        "source_line",
                        "page",
                        "stance",
                        "command",
                        "skill_name",
                        "condition",
                        "condition_semantics",
                        "statement",
                    ] {
                        if let Some(value) = fields.get(key) {
                            compact.insert(key.to_string(), value.clone());
                        }
                    }
                    Value::Object(compact)
                })
                .collect(),
        ),
    );
    rotation.insert(
        "macro_statements_total".to_string(),
        serde_json::json!(source_items.len()),
    );
    rotation.insert(
        "model_projection_macro_statements_truncated".to_string(),
        Value::Bool(source_items.len() > MAX_MODEL_MACRO_STATEMENTS),
    );
}

fn evidence_priority(envelope: &Value) -> u8 {
    let has_domain_claim = envelope
        .pointer("/result/results")
        .and_then(Value::as_array)
        .is_some_and(|results| {
            results.iter().any(|result| {
                result
                    .get("domain_claims")
                    .and_then(Value::as_array)
                    .is_some_and(|claims| !claims.is_empty())
            })
        });
    if has_domain_claim {
        return 0;
    }
    match envelope.get("tool_name").and_then(Value::as_str) {
        Some("compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios") => 1,
        Some("compare_focused_equipment" | "compare_equipment_strategies") => 1,
        Some("search_knowledge_base") => 2,
        Some("inspect_rotation_input") => 1,
        Some("get_current_scenario") => 1,
        Some("analyze_timeline") => 2,
        Some("simulate_scenario") => 3,
        _ => 4,
    }
}

fn compact_handoff_evidence(envelope: &Value) -> Value {
    let tool_name = envelope
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut result = envelope
        .get("result")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    project_tool_result(tool_name, &mut result);
    let mut result = compact_result_facts(Some(&result));
    let ordered_source = result.clone();
    bound_json_value(&mut result, 8, 500, 0);
    restore_ordered_macro_statements(&ordered_source, &mut result);
    serde_json::json!({
        "evidence_id": envelope.get("evidence_id"),
        "tool_name": tool_name,
        "result": result,
    })
}

fn model_evidence_handoff(evidence: &EvidenceStore, max_bytes: usize) -> String {
    let mut candidates = evidence
        .values()
        .map(|envelope| {
            let projected = compact_handoff_evidence(envelope);
            (evidence_priority(envelope), projected)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(priority, item)| {
        (
            *priority,
            item.get("evidence_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    });
    let mut items = Vec::new();
    let mut omitted = Vec::new();
    for (_, item) in candidates {
        let mut trial = items.clone();
        trial.push(item.clone());
        let encoded = serde_json::to_vec(&trial)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        if encoded <= max_bytes {
            items.push(item);
        } else if let Some(id) = item.get("evidence_id").and_then(Value::as_str) {
            omitted.push(id.to_string());
        }
    }
    let payload = serde_json::json!({
        "schema_version": "agent-model-evidence/v1",
        "items": items,
        "omitted_evidence_ids": omitted,
        "boundary": "This is a bounded model projection. Full immutable evidence remains available to server-side validators."
    });
    format!(
        "<model_evidence server_generated=\"true\" schema=\"agent-model-evidence/v1\">\n{}\n</model_evidence>",
        serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
    )
}

fn compact_transcript_messages(messages: &[ModelMessage]) -> Vec<ModelMessage> {
    messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User { content } if content.starts_with("<session_context") => {
                Some(ModelMessage::User {
                    content: clip_model_text(content, 2_500),
                })
            }
            ModelMessage::Assistant {
                content,
                tool_calls,
                reasoning_content,
            } => Some(ModelMessage::Assistant {
                content: content.as_ref().map(|text| clip_model_text(text, 1_000)),
                tool_calls: tool_calls.clone(),
                reasoning_content: reasoning_content.clone(),
            }),
            ModelMessage::ToolResult { call_id, output } => Some(ModelMessage::ToolResult {
                call_id: call_id.clone(),
                output: model_tool_output(output),
            }),
            _ => Some(message.clone()),
        })
        .collect()
}

fn compact_handoff_messages(
    input: &AgentRunInput,
    transcript: &[ModelMessage],
    evidence: &EvidenceStore,
    evidence_bytes: usize,
) -> Vec<ModelMessage> {
    let mut messages = Vec::new();
    if let Some(context) = &input.session_context {
        messages.push(ModelMessage::User {
            content: format!(
                "<session_context untrusted_data=\"true\">\n{}\n</session_context>",
                clip_model_text(context, 2_000)
            ),
        });
    }
    messages.push(ModelMessage::User {
        content: input.question.clone(),
    });
    let mut working_notes = transcript
        .iter()
        .rev()
        .filter_map(|message| match message {
            ModelMessage::Assistant {
                content: Some(content),
                ..
            } if !content.trim().is_empty() && !content.trim_start().starts_with('{') => {
                Some(clip_model_text(content, 700))
            }
            _ => None,
        })
        .take(2)
        .collect::<Vec<_>>();
    working_notes.reverse();
    if !working_notes.is_empty() {
        messages.push(ModelMessage::User {
            content: format!(
                "<working_notes source=\"earlier_public_phase_summaries\">\n{}\n</working_notes>",
                working_notes.join("\n")
            ),
        });
    }
    messages.push(ModelMessage::User {
        content: model_evidence_handoff(evidence, evidence_bytes),
    });
    messages.push(ModelMessage::User {
        content: "Earlier provider messages were compacted into the evidence above. Continue from the current conclusions. Reuse registered evidence, avoid repeating completed deterministic tools, and choose only an action that can materially change the answer; otherwise answer the user now.".to_string(),
    });
    messages
}

fn clip_model_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    format!("{}…", value.chars().take(max_chars).collect::<String>())
}

fn request_bytes(request: &ModelRequest) -> usize {
    let serialized = serde_json::to_vec(request)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX);
    if serialized == usize::MAX {
        return serialized;
    }
    serialized.saturating_add(
        request
            .messages
            .iter()
            .filter_map(|message| match message {
                ModelMessage::Assistant {
                    reasoning_content: Some(content),
                    ..
                } => Some(content.len()),
                _ => None,
            })
            .sum::<usize>(),
    )
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

fn looks_like_unexecuted_plan(assistant_text: Option<&str>) -> bool {
    let Some(value) = assistant_text
        .map(str::trim)
        .filter(|text| text.starts_with('{'))
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("next_action")
        && (object.contains_key("action_plan") || object.contains_key("current_understanding"))
        && !object.contains_key("findings")
        && !object.contains_key("summary")
}

fn is_reusable_deterministic_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "get_current_scenario"
            | "inspect_rotation_input"
            | "simulate_scenario"
            | "compare_scenarios"
            | "analyze_timeline"
            | "list_saved_artifacts"
            | "read_saved_artifact"
            | "compare_saved_macros"
            | "compare_saved_scenarios"
    )
}

fn requires_knowledge_only_client_scope(question: &str) -> bool {
    let normalized = question.to_lowercase();
    normalized.contains("无界")
        || normalized.contains("分山劲·悟")
        || normalized.contains("分山劲・悟")
        || normalized.contains("wujie")
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
        || limits.max_model_turns > 10
        || limits.max_tool_calls == 0
        || limits.max_tool_calls > 12
        || limits.max_simulations == 0
        || limits.max_simulations > 8
        || limits.max_output_tokens_per_turn == 0
        || limits.max_output_tokens_per_turn > 8192
        || limits.wall_time_ms == 0
        || limits.wall_time_ms > 240_000
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
fn terminal_needs_user_input(
    provider: &dyn LlmProvider,
    input: &AgentRunInput,
    prompt: &super::prompt::PromptSpec,
    accounting: AgentRunAccountingV1,
    clarification: AgentClarificationV1,
    trace: TraceCollector,
    started: Instant,
    registry: &AgentToolRegistry<'_>,
) -> AgentRunResultV1 {
    let mut output = terminal_with_registry(
        provider,
        input,
        prompt,
        AgentRunStatus::NeedsUserInput,
        accounting,
        None,
        None,
        trace,
        started,
        registry,
    );
    output.clarification = Some(clarification);
    output
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
    trace.refresh_evidence_pack(registry.evidence());
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
        equipment_comparisons: registry.equipment_comparisons().to_vec(),
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
    trace.refresh_evidence_pack(registry.evidence());
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
    let debug = trace.debug(&input.question);
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
        clarification: None,
        error,
        trace: trace.into_events(),
        debug: Some(debug),
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
    let mut diagnostic_findings = Vec::new();
    let mut has_saved_catalog = false;
    let mut has_equipment_evidence = false;
    if let Some((evidence_id, envelope)) = evidence.iter().find(|(_, envelope)| {
        envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            == Some("list_saved_artifacts")
            && envelope.pointer("/result/items").is_some()
    }) {
        has_saved_catalog = true;
        let items = envelope
            .pointer("/result/items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let entries = items
            .iter()
            .filter_map(|item| {
                let name = item.get("name").and_then(serde_json::Value::as_str)?;
                let kind = match item.get("kind").and_then(serde_json::Value::as_str) {
                    Some("macro") => "宏",
                    Some("loop") => "循环",
                    Some("plaza") => "战斗广场方案",
                    Some("equipment") => "配装",
                    Some("attributes") => "属性方案",
                    _ => "方案",
                };
                Some(format!("{name}（{kind}）"))
            })
            .collect::<Vec<_>>();
        let explanation = if entries.is_empty() {
            "本地保存目录当前没有可供 Agent 读取的宏、循环或战斗广场方案。".to_string()
        } else {
            format!(
                "本地保存目录返回 {} 个方案：{}。同类方案可直接进入对应比较；宏与完整场景需要先明确比较口径。",
                entries.len(),
                entries.join("；")
            )
        };
        diagnostic_findings.push(AgentFindingV1 {
            title: "当前已保存方案".to_string(),
            explanation,
            evidence_ids: vec![evidence_id.clone()],
            metrics: Vec::new(),
        });
    }
    if let Some((evidence_id, envelope)) = evidence.iter().find(|(_, envelope)| {
        envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            == Some("inspect_equipment_workspace")
            && envelope.pointer("/result/equipped").is_some()
    }) {
        has_equipment_evidence = true;
        let equipped = envelope
            .pointer("/result/equipped")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut set_counts = HashMap::<String, usize>::new();
        for item in &equipped {
            if let Some(set_name) = item.get("set_name").and_then(serde_json::Value::as_str) {
                *set_counts.entry(set_name.to_string()).or_default() += 1;
            }
        }
        let mut sets = set_counts.into_iter().collect::<Vec<_>>();
        sets.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let set_summary = if sets.is_empty() {
            "未识别到成套装备".to_string()
        } else {
            sets.into_iter()
                .map(|(name, count)| format!("{name}：{count} 件"))
                .collect::<Vec<_>>()
                .join("；")
        };
        let metric_specs = [
            ("attack", "攻击", "attack"),
            ("crit", "会心", "percent"),
            ("overcome", "破防", "percent"),
            ("strain", "无双", "percent"),
            ("haste", "加速", "percent"),
            ("surplus", "破招", "rating"),
        ];
        let metrics = metric_specs
            .into_iter()
            .filter_map(|(key, label, unit)| {
                envelope
                    .pointer(&format!("/result/panel/{key}"))
                    .and_then(serde_json::Value::as_f64)
                    .map(|value| GroundedMetricV1 {
                        label: label.to_string(),
                        value,
                        unit: unit.to_string(),
                        evidence_id: evidence_id.clone(),
                        json_pointer: format!("/result/panel/{key}"),
                    })
            })
            .take(4)
            .collect::<Vec<_>>();
        diagnostic_findings.push(AgentFindingV1 {
            title: "当前配装结构".to_string(),
            explanation: format!(
                "配装器已读取全部 {} 个装备槽。套装构成：{}。这些数值描述当前面板，不代表任何未实测换装方案的收益。",
                equipped.len(), set_summary
            ),
            evidence_ids: vec![evidence_id.clone()],
            metrics,
        });
    }
    if let Some((evidence_id, envelope)) = evidence.iter().find(|(_, envelope)| {
        envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            == Some("simulate_scenario")
            && envelope.pointer("/result/dps").is_some()
    }) {
        if let Some(dps) = envelope
            .pointer("/result/dps")
            .and_then(serde_json::Value::as_f64)
        {
            let mut metrics = vec![GroundedMetricV1 {
                label: "平均 DPS".to_string(),
                value: dps,
                unit: "damage_per_second".to_string(),
                evidence_id: evidence_id.clone(),
                json_pointer: "/result/dps".to_string(),
            }];
            if let Some(total_damage) = envelope
                .pointer("/result/total_damage")
                .and_then(serde_json::Value::as_f64)
            {
                metrics.push(GroundedMetricV1 {
                    label: "总伤害".to_string(),
                    value: total_damage,
                    unit: "damage".to_string(),
                    evidence_id: evidence_id.clone(),
                    json_pointer: "/result/total_damage".to_string(),
                });
            }
            diagnostic_findings.push(AgentFindingV1 {
                title: "当前输出基线".to_string(),
                explanation: "这是冻结场景的模拟结果，用于描述当前表现，不自动代表循环已经最优。"
                    .to_string(),
                evidence_ids: vec![evidence_id.clone()],
                metrics,
            });

            let mut ranked_skills = envelope
                .pointer("/result/skills")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(_, skill)| {
                    skill
                        .get("total_damage")
                        .and_then(serde_json::Value::as_f64)
                        .is_some_and(|damage| damage > 0.0)
                })
                .collect::<Vec<_>>();
            ranked_skills.sort_by(|(_, left), (_, right)| {
                right
                    .get("total_damage")
                    .and_then(serde_json::Value::as_f64)
                    .partial_cmp(
                        &left
                            .get("total_damage")
                            .and_then(serde_json::Value::as_f64),
                    )
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let damage_metrics = ranked_skills
                .into_iter()
                .take(3)
                .filter_map(|(index, skill)| {
                    let name = skill.get("name").and_then(serde_json::Value::as_str)?;
                    let share = skill
                        .get("damage_share")
                        .and_then(serde_json::Value::as_f64)?;
                    Some(GroundedMetricV1 {
                        label: format!("{name}伤害占比"),
                        value: share,
                        unit: "ratio".to_string(),
                        evidence_id: evidence_id.clone(),
                        json_pointer: format!("/result/skills/{index}/damage_share"),
                    })
                })
                .collect::<Vec<_>>();
            if !damage_metrics.is_empty() {
                diagnostic_findings.push(AgentFindingV1 {
                    title: "主要伤害来源".to_string(),
                    explanation:
                        "按本次总伤害从高到低列出主要技能；这里只描述构成，不据占比单独判断循环优劣。"
                            .to_string(),
                    evidence_ids: vec![evidence_id.clone()],
                    metrics: damage_metrics,
                });
            }
        }
    }
    if let Some((evidence_id, envelope)) = evidence.iter().find(|(_, envelope)| {
        envelope
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            == Some("analyze_timeline")
            && envelope.pointer("/result/diagnostic_profile").is_some()
    }) {
        let profile = envelope
            .pointer("/result/diagnostic_profile")
            .expect("timeline profile checked above");
        let input_mode = profile
            .get("input_mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let input_label = match input_mode {
            "macro" => "宏循环",
            "manual_sequence" => "手动序列",
            _ => "当前输入",
        };
        let mut observations = Vec::new();
        if envelope
            .pointer("/result/skipped")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            observations.push("没有输入被模拟器标记为跳过，但这不等于技能时机已经最优".to_string());
        }
        if let Some(gaps) = envelope
            .pointer("/result/gcd_gaps")
            .and_then(serde_json::Value::as_array)
            .filter(|gaps| !gaps.is_empty())
        {
            let first_pair = gaps.first().and_then(|gap| {
                Some((
                    gap.get("previous_skill_name")
                        .and_then(serde_json::Value::as_str)?,
                    gap.get("next_skill_name")
                        .and_then(serde_json::Value::as_str)?,
                ))
            });
            if let Some((previous, next)) = first_pair.filter(|(previous, next)| {
                gaps.iter().all(|gap| {
                    gap.get("previous_skill_name")
                        .and_then(serde_json::Value::as_str)
                        == Some(*previous)
                        && gap
                            .get("next_skill_name")
                            .and_then(serde_json::Value::as_str)
                            == Some(*next)
                })
            }) {
                observations.push(format!("观察到的主技能空档均位于{previous}接{next}"));
            } else {
                observations.push("时间轴观察到主技能空档，具体原因仍需单变量对照".to_string());
            }
        }
        if envelope.pointer("/result/rage/at_cap_observations").is_some() {
            observations.push("怒气触顶是采样现象，不等同于已经测得怒气损失".to_string());
        }
        if envelope.pointer("/result/buff_coverage").is_some() {
            observations.push("增益覆盖率按有效时长计算，与平均层数分开".to_string());
        }

        let mut metrics = Vec::new();
        for (pointer, label, unit) in [
            (
                "/result/diagnostic_profile/cadence_gaps/count",
                "主技能空档次数",
                "count",
            ),
            (
                "/result/diagnostic_profile/cadence_gaps/total_seconds",
                "主技能空档总时长",
                "seconds",
            ),
            (
                "/result/rage/at_cap_observations",
                "怒气触顶采样",
                "count",
            ),
            ("/result/rage/sample_count", "怒气采样总数", "count"),
        ] {
            if let Some(value) = envelope.pointer(pointer).and_then(serde_json::Value::as_f64) {
                metrics.push(GroundedMetricV1 {
                    label: label.to_string(),
                    value,
                    unit: unit.to_string(),
                    evidence_id: evidence_id.clone(),
                    json_pointer: pointer.to_string(),
                });
            }
        }
        if let Some(coverage) = envelope
            .pointer("/result/buff_coverage")
            .and_then(serde_json::Value::as_array)
        {
            for preferred_name in ["嗜血", "援戈", "血怒·惊涌"] {
                if let Some((index, item)) = coverage.iter().enumerate().find(|(_, item)| {
                    item.get("name").and_then(serde_json::Value::as_str) == Some(preferred_name)
                }) {
                    if let Some(value) = item
                        .get("coverage_percent")
                        .and_then(serde_json::Value::as_f64)
                    {
                        metrics.push(GroundedMetricV1 {
                            label: format!("{preferred_name}时间覆盖率"),
                            value,
                            unit: "percent".to_string(),
                            evidence_id: evidence_id.clone(),
                            json_pointer: format!(
                                "/result/buff_coverage/{index}/coverage_percent"
                            ),
                        });
                    }
                }
            }
        }
        diagnostic_findings.push(AgentFindingV1 {
            title: format!("执行稳定性与待验证风险 · {input_label}"),
            explanation: format!("{}。", observations.join("；")),
            evidence_ids: vec![evidence_id.clone()],
            metrics,
        });
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
            explanation: format!("资料要点：{excerpt}"),
            evidence_ids: vec![evidence_id.clone()],
            metrics: Vec::new(),
        });
        if knowledge_findings.len() >= 3 {
            break;
        }
    }
    let has_diagnostic_evidence = !diagnostic_findings.is_empty();
    let has_readable_knowledge = !knowledge_findings.is_empty();
    let findings = if has_diagnostic_evidence {
        diagnostic_findings
    } else if has_readable_knowledge {
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
        summary: if has_saved_catalog {
            format!(
                "已读取“{}”的本地保存目录；下方名称与类型均来自只读目录工具。",
                plan.playbook.label
            )
        } else if has_equipment_evidence {
            format!(
                "已读取“{}”的当前装备、面板与套装构成。模型未完成解释，因此这里只发布配装器可直接证明的内容。",
                plan.playbook.label
            )
        } else if has_diagnostic_evidence {
            "当前循环的伤害构成、衔接、资源与增益覆盖如下；这些都是当前冻结条件下的观测结果。"
                .to_string()
        } else if has_readable_knowledge {
            format!(
                "模型解释未通过发布校验；以下整理“{}”命中的可溯源资料要点，不补写未经验证的结论。",
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

/// Keep the model's useful expert judgment visible even when its JSON dialect
/// misses the typed report contract. Deterministic metric cards are rebuilt
/// separately from immutable evidence, so prose transport errors cannot erase
/// the actual answer and malformed metric citations cannot become facts.
fn model_judgment_with_evidence_fallback(
    raw: &str,
    plan: &AnalysisPlanV1,
    evidence: &super::report::EvidenceStore,
    limitation: &str,
) -> Option<AgentReportContentV1> {
    let mut verified = evidence_preserving_provider_fallback(plan, evidence, limitation)?;
    let trimmed = raw.trim().trim_matches('`').trim();
    let value = serde_json::from_str::<Value>(trimmed).ok();
    let object = value.as_ref().and_then(Value::as_object);

    let summary = object
        .and_then(|fields| fields.get("summary"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| clip_model_text(text, 1_024));

    let mut judgments = object
        .and_then(|fields| fields.get("findings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|fields| {
            let explanation = ["explanation", "claim", "description", "statement", "finding"]
                .into_iter()
                .find_map(|key| fields.get(key).and_then(Value::as_str))?
                .trim();
            if explanation.is_empty() {
                return None;
            }
            let title = fields
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(|title| clip_model_text(title, 160))
                .unwrap_or_else(|| explanation.chars().take(36).collect::<String>());
            let evidence_ids = fields
                .get("evidence_ids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| evidence.contains_key(*id))
                .map(str::to_string)
                .collect::<Vec<_>>();
            Some(AgentFindingV1 {
                title,
                explanation: clip_model_text(explanation, 1_024),
                evidence_ids,
                metrics: Vec::new(),
            })
        })
        .take(8)
        .collect::<Vec<_>>();

    if judgments.is_empty() && value.is_none() && trimmed.chars().count() >= 40 {
        judgments.push(AgentFindingV1 {
            title: "模型分析".to_string(),
            explanation: clip_model_text(trimmed, 1_024),
            evidence_ids: Vec::new(),
            metrics: Vec::new(),
        });
    }
    if summary.is_none() && judgments.is_empty() {
        return Some(verified);
    }

    if let Some(summary) = summary {
        verified.summary = summary;
    }
    judgments.extend(verified.findings);
    verified.findings = judgments;

    let recommendation_text = object
        .and_then(|fields| fields.get("recommendations"))
        .and_then(|recommendations| match recommendations {
            Value::String(text) => Some(text.as_str()),
            Value::Array(items) => items.iter().find_map(|item| {
                item.as_str().or_else(|| {
                    item.as_object()
                        .and_then(|fields| fields.get("rationale"))
                        .and_then(Value::as_str)
                })
            }),
            _ => None,
        })
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if let Some(rationale) = recommendation_text {
        verified.recommendations.insert(
            0,
            super::report::AgentRecommendationV1 {
                title: "模型建议".to_string(),
                rationale: clip_model_text(rationale, 1_024),
                evidence_ids: verified
                    .findings
                    .iter()
                    .flat_map(|finding| finding.evidence_ids.iter().cloned())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            },
        );
    }
    Some(verified)
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
            "inspect_rotation_input" => serde_json::json!({
                "mode": result.get("mode"),
                "total_items": result.get("total_items"),
                "query": result.get("query"),
                "matches": result.get("matches"),
                "window": result.get("window"),
                "next_start_index": result.get("next_start_index"),
            }),
            "simulate_scenario" => serde_json::json!({
                "dps": result.get("dps"),
                "total_damage": result.get("total_damage"),
                "fight_time": result.get("fight_time"),
                "skill_count": result.get("skill_count"),
                "ranked_damage_sources": ranked_damage_sources(&result, 8),
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
                "metric_catalog": timeline_metric_catalog(&result),
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

fn parse_clarification(arguments: &Value) -> Result<AgentClarificationV1, &'static str> {
    let parsed = serde_json::from_value::<AskUserQuestionArguments>(arguments.clone())
        .map_err(|_| "clarification arguments must match the declared schema")?;
    let question = parsed.question.trim();
    let reason = parsed.reason.trim();
    let answer_hint = parsed
        .answer_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if question.is_empty() || question.chars().count() > 500 {
        return Err("question must contain 1 to 500 characters");
    }
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err("reason must contain 1 to 500 characters");
    }
    if answer_hint.is_some_and(|value| value.chars().count() > 240) {
        return Err("answer_hint must contain at most 240 characters");
    }
    let sensitive = format!("{question} {reason} {}", answer_hint.unwrap_or_default())
        .to_ascii_lowercase();
    if ["api key", "apikey", "token", "password", "密码", "密钥", "令牌"]
        .iter()
        .any(|needle| sensitive.contains(needle))
    {
        return Err("clarification cannot request credentials or secrets");
    }
    Ok(AgentClarificationV1 {
        schema_version: "agent-clarification/v1".to_string(),
        question: question.to_string(),
        reason: reason.to_string(),
        answer_hint: answer_hint.map(str::to_string),
    })
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
        AgentRunStatus::NeedsUserInput => "needs_user_input",
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

    #[test]
    fn model_projection_bounds_large_timeline_without_touching_full_evidence() {
        let intervals = (0..2_000)
            .map(|index| json!({"start": index as f64, "end": index as f64 + 0.5}))
            .collect::<Vec<_>>();
        let full = json!({
            "schema_version": "agent-tool-result/v1",
            "ok": true,
            "tool_name": "analyze_timeline",
            "evidence_ids": ["a".repeat(64)],
            "evidence": [{
                "evidence_id": "a".repeat(64),
                "tool_name": "analyze_timeline",
                "result": {
                    "diagnostic_profile": {"cadence_gaps": {"count": 0}},
                    "buff_coverage": [{
                        "name": "嗜血",
                        "coverage_percent": 97.5,
                        "intervals": intervals
                    }]
                }
            }]
        });
        let projected = model_tool_output(&full);
        assert!(serde_json::to_vec(&projected).unwrap().len() <= MODEL_TOOL_OUTPUT_BYTES);
        assert!(projected
            .pointer("/evidence/0/result/buff_coverage/0/intervals")
            .is_none());
        assert!(full
            .pointer("/evidence/0/result/buff_coverage/0/intervals")
            .is_some());
    }

    #[test]
    fn simulation_projection_is_ranked_and_does_not_expose_unordered_skills() {
        let full = json!({
            "schema_version": "agent-tool-result/v1",
            "ok": true,
            "tool_name": "simulate_scenario",
            "evidence_ids": ["a".repeat(64)],
            "evidence": [{
                "evidence_id": "a".repeat(64),
                "tool_name": "simulate_scenario",
                "result": {
                    "dps": 100.0,
                    "skills": [
                        {"name": "绝刀·20怒", "damage_share": 0.01, "total_damage": 10.0, "event_count": 1},
                        {"name": "绝刀·50怒", "damage_share": 0.40, "total_damage": 400.0, "event_count": 80}
                    ]
                }
            }]
        });
        let projected = model_tool_output(&full);
        assert!(projected.pointer("/evidence/0/result/skills").is_none());
        assert_eq!(
            projected
                .pointer("/evidence/0/result/ranked_damage_sources/0/name")
                .and_then(Value::as_str),
            Some("绝刀·50怒")
        );
        assert_eq!(
            projected
                .pointer("/evidence/0/result/ranked_damage_sources/0/event_count")
                .and_then(Value::as_u64),
            Some(80)
        );
        assert!(full.pointer("/evidence/0/result/skills").is_some());

        let projected_twice = model_tool_output(&projected);
        assert_eq!(
            projected_twice
                .pointer("/evidence/0/result/ranked_damage_sources/0/name")
                .and_then(Value::as_str),
            Some("绝刀·50怒")
        );
    }

    #[test]
    fn compact_facts_keep_the_complete_small_stance_macro() {
        let statements = (0..10)
            .map(|index| {
                json!({
                    "source_line": index + 1,
                    "page": if index < 6 { 0 } else { 1 },
                    "stance": if index < 6 { "shield" } else { "blade" },
                    "statement": format!("/cast 技能{}", index + 1),
                })
            })
            .collect::<Vec<_>>();
        let compact = compact_result_facts(Some(&json!({
            "rotation_input": {
                "mode": "macro",
                "macro_statements": statements,
            }
        })));

        assert_eq!(
            compact
                .pointer("/rotation_input/macro_statements")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(10)
        );
        assert_eq!(
            compact
                .pointer("/rotation_input/macro_statements/9/stance")
                .and_then(Value::as_str),
            Some("blade")
        );
        assert_eq!(
            compact
                .pointer("/rotation_input/model_projection_macro_statements_truncated")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn evidence_handoff_has_a_hard_byte_budget() {
        let mut evidence = EvidenceStore::new();
        for index in 0..20 {
            let id = format!("{index:064x}");
            evidence.insert(
                id.clone(),
                json!({
                    "evidence_id": id,
                    "tool_name": "search_knowledge_base",
                    "result": {
                        "results": [{
                            "document_id": format!("doc-{index}"),
                            "title": format!("资料 {index}"),
                            "snippet": "长文本".repeat(4_000),
                            "fact_eligible": true,
                            "version_match": "current_exact"
                        }]
                    }
                }),
            );
        }
        let handoff = model_evidence_handoff(&evidence, MODEL_EVIDENCE_HANDOFF_BYTES);
        assert!(handoff.len() <= MODEL_EVIDENCE_HANDOFF_BYTES + 4_096);
        assert!(handoff.contains("omitted_evidence_ids"));
    }

    #[test]
    fn compact_handoff_prioritizes_source_bound_domain_claims() {
        let mut evidence = EvidenceStore::new();
        evidence.insert(
            "1".repeat(64),
            json!({
                "evidence_id": "1".repeat(64),
                "tool_name": "get_current_scenario",
                "result": {"rotation_input": {"macro_statements": (0..30).map(|index| json!({
                    "source_line": index + 1,
                    "statement": "很长的宏语句".repeat(100)
                })).collect::<Vec<_>>()}}
            }),
        );
        evidence.insert(
            "f".repeat(64),
            json!({
                "evidence_id": "f".repeat(64),
                "tool_name": "search_knowledge_base",
                "result": {"results": [{
                    "title": "当前白皮书",
                    "snippet": "白刀是未触发援戈血影的苍雪刀。",
                    "fact_eligible": true,
                    "domain_claims": [{
                        "claim_id": "fs-white-blade-001",
                        "statement": "白刀指未触发援戈·血影的苍雪刀斩绝绝。"
                    }]
                }]}
            }),
        );

        let handoff = model_evidence_handoff(&evidence, 3 * 1024);
        assert!(handoff.contains("fs-white-blade-001"));
        assert!(handoff.contains("白刀指未触发援戈"));
    }

    #[test]
    fn saved_catalog_fallback_keeps_real_names_when_provider_explanation_fails() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let plan = select_analysis_plan("我保存了哪些可以互相比较的宏或循环？", &scenario);
        let evidence_id = "a".repeat(64);
        let mut evidence = EvidenceStore::new();
        evidence.insert(
            evidence_id.clone(),
            json!({
                "evidence_id": evidence_id,
                "tool_name": "list_saved_artifacts",
                "result": {
                    "total_matches": 2,
                    "items": [
                        {"name": "分山绝云", "kind": "macro"},
                        {"name": "木桩基线", "kind": "plaza"}
                    ]
                }
            }),
        );

        let report = evidence_preserving_provider_fallback(&plan, &evidence, "模型失败")
            .expect("saved catalog fallback");
        assert!(report.summary.contains("本地保存目录"));
        assert!(report.findings[0].explanation.contains("分山绝云（宏）"));
        assert!(report.findings[0]
            .explanation
            .contains("木桩基线（战斗广场方案）"));
        assert!(!report.findings[0].explanation.contains("知识库"));
    }

    #[test]
    fn compact_handoff_builds_a_provider_safe_request_from_oversized_evidence() {
        let runtime = AgentRuntime::fixture();
        let input = input(&runtime, "run-context-handoff");
        let mut evidence = EvidenceStore::new();
        for index in 0..24 {
            let id = format!("{index:064x}");
            evidence.insert(
                id.clone(),
                json!({
                    "evidence_id": id,
                    "tool_name": "search_knowledge_base",
                    "result": {"results": [{
                        "document_id": format!("doc-{index}"),
                        "title": "超长资料",
                        "snippet": "证据正文".repeat(5_000),
                        "fact_eligible": true,
                        "version_match": "current_exact"
                    }]}
                }),
            );
        }
        let prompt = agent_prompt();
        let transcript = vec![ModelMessage::Assistant {
            content: Some("已确认白刀的资料定义，下一步定位当前循环。".to_string()),
            tool_calls: Vec::new(),
            reasoning_content: None,
        }];
        let request = ModelRequest {
            instructions: prompt.instructions.to_string(),
            messages: compact_handoff_messages(&input, &transcript, &evidence, 6 * 1024),
            tools: Vec::new(),
            response_format: Some(StructuredOutputDefinition {
                name: "agent_report_content_v1".to_string(),
                schema: report_content_json_schema(),
            }),
            max_output_tokens: 2_048,
        };
        assert!(request_bytes(&request) <= MAX_MODEL_REQUEST_BYTES);
        assert!(request.messages.iter().any(|message| matches!(
            message,
            ModelMessage::User { content } if content.contains("已确认白刀的资料定义")
        )));
        request.validate().unwrap();
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
                    reasoning_content: None,
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
                assert!(request
                    .tools
                    .iter()
                    .any(|tool| tool.name == ASK_USER_QUESTION));
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
                    reasoning_content: None,
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
                    reasoning_content: None,
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
            task_hint: None,
            analysis_surface: None,
            equipment_workspace: None,
        }
    }

    fn tool_call(call_id: &str, name: &str, arguments: serde_json::Value) -> ModelResponse {
        ModelResponse {
            assistant_text: None,
            reasoning_content: None,
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
        assert_eq!(limits.max_model_turns, 10);
        assert_eq!(limits.max_tool_calls, 12);
        assert_eq!(limits.max_simulations, 8);
        assert_eq!(limits.max_output_tokens_per_turn, 8192);
        assert_eq!(limits.wall_time_ms, 180_000);
    }

    #[test]
    fn cancellation_is_shareable_and_monotonic() {
        let cancellation = AgentCancellation::default();
        let copy = cancellation.clone();
        assert!(!copy.is_cancelled());
        cancellation.cancel();
        assert!(copy.is_cancelled());
    }

    #[tokio::test]
    async fn model_can_pause_for_one_material_user_answer() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Ok(tool_call(
            "call-clarify",
            ASK_USER_QUESTION,
            json!({
                "question": "你说的两个方案分别是哪两个？",
                "reason": "当前会话没有能唯一定位它们的名称。",
                "answer_hint": "回复两个保存方案的名称即可"
            }),
        ))]);

        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-needs-user-input"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::NeedsUserInput);
        assert!(result.report.is_none());
        let clarification = result.clarification.expect("clarification");
        assert_eq!(clarification.question, "你说的两个方案分别是哪两个？");
        assert_eq!(result.accounting.tool_calls, 2);
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "needs_user_input"));
    }

    #[test]
    fn shareable_debug_projection_redacts_nested_secret_values() {
        let secret = "sk-123456789012345678901234567890";
        let redacted = redact_debug_value(&json!({
            "question": format!("测试 {secret}"),
            "nested": [{"authorization": format!("Bearer {secret}")}]
        }));
        let encoded = serde_json::to_string(&redacted).unwrap();
        assert!(!encoded.contains(secret));
        assert!(encoded.contains("REDACTED"));
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
        assert_eq!(result.prompt_version, "agent-system/v30");
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
            reasoning_content: None,
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
            vec!["get_current_scenario", "ask_user_question", "search_knowledge_base"]
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
        assert_eq!(result.prompt_version, "agent-system/v30");
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
    async fn reference_lookup_keeps_follow_up_tools_available() {
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
                reasoning_content: None,
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
        assert!(requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "search_knowledge_base"));

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
            reasoning_content: None,
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
            reasoning_content: None,
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
        assert_eq!(requests[0].messages.len(), 4);
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
            ModelMessage::Assistant { tool_calls, .. }
                if tool_calls.len() == 1 && tool_calls[0].name == "get_current_scenario"
        ));
        assert!(matches!(
            &requests[0].messages[3],
            ModelMessage::ToolResult { call_id, .. }
                if call_id == "server-prefetch-scenario"
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
                reasoning_content: None,
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
    async fn model_turn_budget_preserves_completed_tool_evidence() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Ok(tool_call(
            "call-diagnose",
            "analyze_timeline",
            json!({}),
        ))]);
        let limits = AgentRunLimits {
            max_model_turns: 1,
            ..AgentRunLimits::default()
        };
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-model-budget-evidence"),
            limits,
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        assert_eq!(result.error.as_ref().unwrap().code, "model_turn_budget");
        let report = result.report.unwrap();
        assert!(!report.evidence_ids.is_empty());
        assert!(report
            .content
            .findings
            .iter()
            .any(|finding| finding.title.contains("当前输出基线")));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "model_turn_budget_evidence_preserved"));
    }

    #[tokio::test]
    async fn current_equipment_question_keeps_model_tools_available() {
        let runtime = AgentRuntime::fixture().with_equipment_fixture();
        let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
            assistant_text: Some(
                serde_json::to_string(&refusal_content(
                    "仅验证当前配装读取。",
                    "测试不生成玩法解释。",
                ))
                .unwrap(),
            ),
            reasoning_content: None,
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: TokenUsage::default(),
        })]);
        let mut run_input = input(&runtime, "run-current-equipment");
        run_input.question = "查看我当前配装。".to_string();
        run_input.equipment_workspace = Some(crate::agent::EquipmentWorkspaceV1 {
            slots: HashMap::from([(
                "PRIMARY_WEAPON".to_string(),
                crate::equip::SlotConfig {
                    equip_id: 45320,
                    strength: 6,
                    embedding: Vec::new(),
                    enhance_id: 0,
                    enchant_id: 0,
                },
            )]),
            stone_id: 0,
            source_label: "当前循环".to_string(),
            focus: None,
        });
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.tool_calls, 2);
        assert_eq!(result.accounting.simulations, 0);
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "inspect_equipment_workspace"));
    }

    #[tokio::test]
    async fn diagnosis_contract_stops_before_a_repeated_deterministic_experiment() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-timeline-1", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "确定性诊断已完成。",
                        "本测试验证诊断题在证据充分后停止，不重复消耗模拟预算。",
                    ))
                    .unwrap(),
                ),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);

        let mut repeated_input = input(&runtime, "run-deterministic-reuse");
        repeated_input.question = "分析当前循环的确定性输出。".to_string();
        let result = run_agent(
            &provider,
            &runtime,
            repeated_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.simulations, 1);
        assert_eq!(result.accounting.tool_calls, 2);
        let timeline_results = result
            .trace
            .iter()
            .filter(|event| {
                event.kind == "tool_finished"
                    && event.tool_name.as_deref() == Some("analyze_timeline")
            })
            .collect::<Vec<_>>();
        assert_eq!(timeline_results.len(), 1);
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
                reasoning_content: None,
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
    async fn knowledge_refinement_keeps_planner_open() {
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
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);

        let mut run_input = input(&runtime, "run-two-knowledge-searches");
        run_input.question = "结合当前版本资料分析当前循环的确定性输出。".to_string();
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
        assert_eq!(result.accounting.model_turns, 2);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "search_knowledge_base"));
        assert!(requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "search_knowledge_base"));

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn parallel_knowledge_searches_fit_the_elastic_budget() {
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
                reasoning_content: None,
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
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);

        let mut run_input = input(&runtime, "run-parallel-knowledge-searches");
        run_input.question = "结合当前版本资料分析当前循环的确定性输出。".to_string();
        let result = run_agent(
            &provider,
            &runtime,
            run_input,
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.knowledge_searches, 4);
        assert_eq!(result.accounting.tool_calls, 5);
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
            4
        );
        assert!(!result
            .trace
            .iter()
            .any(|event| event.kind == "knowledge_searches_coalesced"));
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "search_knowledge_base"));
        assert_eq!(
            requests[1]
                .messages
                .iter()
                .filter(|message| matches!(message, ModelMessage::ToolResult { .. }))
                .count(),
            5
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
        let debug = result.debug.as_ref().expect("shareable debug projection");
        assert_eq!(debug.schema_version, AGENT_RUN_DEBUG_SCHEMA_V1);
        assert_eq!(debug.request_metrics.len(), 1);
        assert!(debug.request_metrics[0].request_bytes > 0);
        assert!(debug.request_metrics[0].request_bytes <= MAX_MODEL_REQUEST_BYTES);
        assert!(debug
            .exposed_tools
            .iter()
            .any(|tool| tool == "get_current_scenario"));
        assert!(debug
            .tool_calls
            .iter()
            .any(|call| call.server_initiated && call.tool_name == "get_current_scenario"));
        assert!(debug.evidence_pack.is_some());
        assert_eq!(result.error.unwrap().code, "provider_http_429");
    }

    #[tokio::test]
    async fn insufficient_balance_never_masquerades_knowledge_as_analysis() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Err(ProviderError::upstream_status(402))]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-provider-balance"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::ProviderFailed);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("provider_balance_insufficient")
        );
        let content = &result.report.expect("explicit provider failure report").content;
        assert!(content.findings.is_empty());
        assert!(content.summary.contains("尚未生成战斗分析"));
        assert!(content.limitations[0].contains("不会冒充"));
    }

    #[tokio::test]
    async fn later_provider_failure_preserves_completed_tool_evidence() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Err(ProviderError::upstream_status(400)),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-provider-error-after-evidence"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        assert!(result.report.is_some());
        assert_eq!(result.error.unwrap().code, "provider_http_400");
        assert!(result.trace.iter().any(|event| {
            event.kind == "provider_failure_evidence_preserved"
        }));
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
                reasoning_content: None,
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
            ModelMessage::User { content } if content.contains("complete AgentReportContentV1")
        )));
        let report = result.report.unwrap();
        assert!(report.content.refusal_reason.is_some());
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_empty_retry"));
    }

    #[tokio::test]
    async fn malformed_tool_arguments_get_one_bounded_protocol_retry() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Err(ProviderError::invalid_response_protocol(
                "provider_tool_arguments_invalid",
                "provider returned invalid JSON tool arguments",
            )),
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "已完成受控重试。",
                        "本测试只验证协议恢复路径。",
                    ))
                    .unwrap(),
                ),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-tool-arguments-retry"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(provider.requests().len(), 3);
        assert!(provider.requests()[1].messages.iter().any(|message| matches!(
            message,
            ModelMessage::User { content } if content.contains("重新选择下一项动作")
        )));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_tool_arguments_retry"));
        assert_ne!(result.status, AgentRunStatus::ProviderFailed);
    }

    #[tokio::test]
    async fn malformed_tool_arguments_finish_from_existing_tool_evidence() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Err(ProviderError::invalid_response_protocol(
                "provider_tool_arguments_invalid",
                "provider returned invalid JSON tool arguments",
            )),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "已基于现有证据完成整理。",
                        "本测试验证证据充足时的协议恢复路径。",
                    ))
                    .unwrap(),
                ),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-tool-arguments-evidence-finish"),
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].tools.is_empty());
        assert!(requests[2].messages.iter().any(|message| matches!(
            message,
            ModelMessage::User { content } if content.contains("已有证据足以")
        )));
        assert_ne!(result.status, AgentRunStatus::ProviderFailed);
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
        assert!(report.content.findings.len() >= 2);
        assert!(report.content.findings[0].title.contains("当前输出基线"));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_empty_evidence_preserved"));
    }

    #[tokio::test]
    async fn invalid_baseline_report_salvages_grounded_evidence() {
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
                assistant_text: Some(bad_report.clone()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some(bad_report),
                reasoning_content: None,
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
        assert!(!requests[1].tools.is_empty());
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
    }

    #[tokio::test]
    async fn structurally_invalid_json_still_gets_one_bounded_repair() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some("not-json".to_string()),
                reasoning_content: None,
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
                reasoning_content: None,
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
        assert!(!requests[1].tools.is_empty());
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
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some("still-not-json".to_string()),
                reasoning_content: None,
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
        assert!(report.content.summary.contains("伤害构成、衔接、资源与增益覆盖"));
        assert!(report.content.findings[0].title.contains("当前输出基线"));
        assert!(!report.content.findings[0].metrics.is_empty());
    }

    #[test]
    fn evidence_fallback_keeps_damage_timeline_resource_and_coverage_facts() {
        let runtime = AgentRuntime::fixture();
        let plan = select_analysis_plan(
            "这套循环的整体输出和伤害结构怎么样？",
            &runtime.fixture_scenario(),
        );
        let simulation_id = "a".repeat(64);
        let timeline_id = "b".repeat(64);
        let mut evidence = EvidenceStore::new();
        evidence.insert(
            simulation_id.clone(),
            json!({
                "evidence_id": simulation_id,
                "tool_name": "simulate_scenario",
                "result": {
                    "dps": 2969004.31,
                    "total_damage": 890701293.0,
                    "skills": [
                        {"name": "援戈·血影", "total_damage": 127856419.0, "damage_share": 0.143545788},
                        {"name": "绝刀·50怒", "total_damage": 296300677.0, "damage_share": 0.332659983},
                        {"name": "业火焚城", "total_damage": 89616872.0, "damage_share": 0.100613834},
                        {"name": "业火麟光", "total_damage": 0.0, "damage_share": 0.0}
                    ]
                }
            }),
        );
        evidence.insert(
            timeline_id.clone(),
            json!({
                "evidence_id": timeline_id,
                "tool_name": "analyze_timeline",
                "result": {
                    "skipped": [],
                    "diagnostic_profile": {
                        "input_mode": "macro",
                        "cadence_gaps": {"count": 11, "total_seconds": 2.3125}
                    },
                    "gcd_gaps": [{
                        "previous_skill_name": "盾击·三段",
                        "next_skill_name": "斩刀"
                    }],
                    "rage": {"at_cap_observations": 28, "sample_count": 627},
                    "buff_coverage": [
                        {"name": "流血", "coverage_percent": 97.8125},
                        {"name": "嗜血", "coverage_percent": 97.3541666667},
                        {"name": "援戈", "coverage_percent": 84.875}
                    ]
                }
            }),
        );

        let report = evidence_preserving_provider_fallback(&plan, &evidence, "模型格式错误")
            .expect("evidence fallback");

        assert_eq!(report.findings.len(), 3);
        assert_eq!(report.findings[1].title, "主要伤害来源");
        assert_eq!(
            report.findings[1]
                .metrics
                .iter()
                .map(|metric| metric.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "绝刀·50怒伤害占比",
                "援戈·血影伤害占比",
                "业火焚城伤害占比"
            ]
        );
        let quality = &report.findings[2];
        assert!(quality.explanation.contains("盾击·三段接斩刀"));
        assert!(quality.explanation.contains("与平均层数分开"));
        assert!(!quality.explanation.contains("。。"));
        assert!(!quality.explanation.contains("。；"));
        assert!(quality
            .metrics
            .iter()
            .any(|metric| metric.label == "援戈时间覆盖率" && metric.value == 84.875));
        super::super::report::validate_report(&report, &evidence).unwrap();
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
        assert!(report.summary.contains("可溯源资料要点"));
        assert!(report.findings[0].explanation.contains("提前倒数10秒"));
        assert!(!report.findings[0].explanation.contains("知识库原文摘录"));
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
