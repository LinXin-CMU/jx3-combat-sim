use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[cfg(test)]
use super::domain::select_analysis_plan;
use super::domain::{
    build_evidence_pack, select_model_led_analysis_plan, trace_annotation, AnalysisPlanV1,
    AnalysisSurface, AnalysisTaskType, EvidencePackV1,
};
use super::evidence::validate_trace_id;
use super::knowledge::{KnowledgeAudience, KnowledgeMountScope, KnowledgeVersionContext};
use super::prompt::agent_prompt;
use super::provider::protocol::MAX_MODEL_REQUEST_BYTES;
use super::provider::{
    FinishReason, LlmProvider, ModelMessage, ModelRequest, ProviderToolCall,
    StructuredOutputDefinition, TokenUsage,
};
use super::registry::{
    AgentToolRegistry, AskUserQuestionArguments, ToolDispatchOutcome, ASK_USER_QUESTION,
    MAX_KNOWLEDGE_SEARCHES,
};
use super::report::{
    cited_evidence_ids, cited_knowledge_sources, parse_and_salvage_report,
    parse_and_validate_report, report_content_json_schema, AgentFindingV1, AgentReportContentV1,
    AgentReportV1, AgentRunAccountingV1, EvidenceStore,
    AGENT_REPORT_CONTENT_SCHEMA_V1, AGENT_REPORT_SCHEMA_V1,
};
#[cfg(test)]
use super::report::GroundedMetricV1;
use super::{AgentRuntime, DomainTermKindV1, ResolvedDomainTermV1, ScenarioSnapshotV1};

pub const AGENT_RUN_SCHEMA_V1: &str = "agent-run/v1";
pub const AGENT_RUN_DEBUG_SCHEMA_V1: &str = "agent-run-debug/v1";
const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_SESSION_CONTEXT_BYTES: usize = 32 * 1024;
const MAX_REPORT_REPAIRS: u32 = 1;
const MAX_EMPTY_RESPONSE_RETRIES: u32 = 1;
const MAX_PROVIDER_PROTOCOL_RETRIES: u32 = 2;
const MAX_TOOL_SELECTION_RETRIES: u32 = 1;
const MODEL_TOOL_OUTPUT_BYTES: usize = 12 * 1024;
const MODEL_EVIDENCE_HANDOFF_BYTES: usize = 24 * 1024;
const MODEL_COMPACTION_TARGET_BYTES: usize = MAX_MODEL_REQUEST_BYTES - 8 * 1024;
const REPORT_REPAIR_EVIDENCE_BYTES: usize = 18 * 1024;
const REPORT_REPAIR_REJECTED_CHARS: usize = 6_000;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_text: Option<String>,
    pub schema_version: String,
    pub question: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<super::registry::ClarificationOptionV1>,
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
            max_simulations: 512,
            max_output_tokens_per_turn: 16384,
            wall_time_ms: 180_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentRunInput {
    /// Trusted same-session read-only queries; re-executed against this frozen scenario.
    pub resume_tools: Vec<(String, Value)>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_domain_terms: Vec<ResolvedDomainTermV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminology_index_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_pack: Option<EvidencePackV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_projection: Option<Value>,
    pub diagnostic_state: AgentDiagnosticStateV1,
}

/// Public, shareable working state for a model-led diagnosis. It records what
/// the Agent is currently trying to establish without exposing hidden model
/// reasoning or turning the initial route into a fixed workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentDiagnosticStateV1 {
    pub schema_version: String,
    pub user_objective: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_judgment: Option<String>,
    pub established_facts: Vec<String>,
    /// Capability-level coverage, independent of any task classifier. This is
    /// the compact working memory the model uses to see what kind of question
    /// has already been answered by tools.
    pub evidence_capabilities: Vec<String>,
    pub judgment_basis: Vec<String>,
    pub open_hypotheses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_falsification_experiment: Option<String>,
    pub last_actions: Vec<String>,
    pub no_new_evidence_rounds: u32,
    pub suggested_next_mode: String,
}

impl AgentDiagnosticStateV1 {
    fn new(question: &str) -> Self {
        Self {
            schema_version: "agent-diagnostic-state/v1".to_string(),
            user_objective: super::session::redact_sensitive_text(question),
            status: "investigating".to_string(),
            current_judgment: None,
            established_facts: Vec::new(),
            evidence_capabilities: Vec::new(),
            judgment_basis: Vec::new(),
            open_hypotheses: Vec::new(),
            best_falsification_experiment: None,
            last_actions: Vec::new(),
            no_new_evidence_rounds: 0,
            suggested_next_mode: "choose_high_information_action".to_string(),
        }
    }

    fn observe_model_action(&mut self, text: Option<&str>, calls: &[ProviderToolCall]) {
        let mut stated_experiment = None;
        if let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) {
            let parsed = serde_json::from_str::<Value>(text).ok();
            let public_text = parsed
                .as_ref()
                .and_then(|value| {
                    value
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| text.to_string());
            self.current_judgment = Some(bounded_public_text(&public_text, 600));
            self.open_hypotheses = extract_hypothesis_sentences(&public_text, 4);
            stated_experiment = parsed
                .as_ref()
                .and_then(|value| value.get("recommendations"))
                .and_then(Value::as_array)
                .and_then(|recommendations| {
                    recommendations.iter().find_map(|recommendation| {
                        let rationale = recommendation.get("rationale")?.as_str()?;
                        contains_experiment_language(rationale)
                            .then(|| bounded_public_text(rationale, 480))
                    })
                });
        }
        if !calls.is_empty() {
            self.last_actions = calls.iter().map(|call| call.name.clone()).collect();
        }
        let next_experiment = calls
            .iter()
            .find(|call| {
                matches!(
                    call.name.as_str(),
                    "compare_scenarios"
                        | "compare_saved_macros"
                        | "compare_saved_scenarios"
                        | "compare_focused_equipment"
                        | "compare_equipment_strategies"
                )
            })
            .map(|call| format!("{} {}", call.name, compact_json(&call.arguments, 360)))
            .or_else(|| {
                calls
                    .iter()
                    .find(|call| call.name == "inspect_timeline_events")
                    .map(|call| format!("{} {}", call.name, compact_json(&call.arguments, 360)))
            });
        if next_experiment.is_some() || stated_experiment.is_some() {
            self.best_falsification_experiment = next_experiment.or(stated_experiment);
        }
    }

    fn refresh_evidence(&mut self, evidence: &EvidenceStore) {
        // Full ids remain in EvidencePack and immutable replay. The injected
        // state is deliberately bounded so additional evidence cannot make
        // every later provider request larger forever.
        self.judgment_basis = evidence.keys().take(12).cloned().collect();
        self.evidence_capabilities = diagnostic_evidence_capabilities(evidence);
        self.suggested_next_mode = if self
            .evidence_capabilities
            .iter()
            .any(|capability| capability == "controlled_comparison")
        {
            "answer_from_validated_comparison"
        } else if ["versioned_knowledge", "timeline_diagnosis"]
            .iter()
            .all(|required| self.evidence_capabilities.iter().any(|actual| actual == required))
            && !self
                .evidence_capabilities
                .iter()
                .any(|actual| actual == "event_location")
        {
            "inspect_exact_events_if_location_matters_else_answer"
        } else if explanation_evidence_stack_complete(&self.evidence_capabilities) {
            "answer_or_run_one_explicit_falsification"
        } else {
            "choose_high_information_action"
        }
        .to_string();
        let mut facts = evidence
            .values()
            .filter_map(|envelope| {
                diagnostic_fact_summary(envelope)
                    .map(|summary| (evidence_priority(envelope), summary))
            })
            .collect::<Vec<_>>();
        facts.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        facts.dedup_by(|left, right| left.1 == right.1);
        self.established_facts = facts
            .into_iter()
            .map(|(_, summary)| summary)
            .take(8)
            .collect();
    }
}

fn diagnostic_evidence_capabilities(evidence: &EvidenceStore) -> Vec<String> {
    let mut capabilities = BTreeSet::new();
    for envelope in evidence.values() {
        match envelope.get("tool_name").and_then(Value::as_str) {
            Some("get_current_scenario") => { capabilities.insert("scenario_scope"); }
            Some("search_knowledge_base") => {
                let result = envelope.get("result").unwrap_or(&Value::Null);
                if result
                    .get("results")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|item| {
                        item.get("fact_eligible").and_then(Value::as_bool) == Some(true)
                    }))
                {
                    capabilities.insert("versioned_knowledge");
                }
                if result
                    .get("resolved_terms")
                    .and_then(Value::as_array)
                    .is_some_and(|terms| terms.iter().any(|term| {
                        term.get("cards").and_then(Value::as_array).is_some_and(|cards| !cards.is_empty())
                    }))
                {
                    capabilities.insert("terminology_resolved");
                }
            }
            Some("simulate_scenario") => { capabilities.insert("simulation_baseline"); }
            Some("analyze_timeline") => { capabilities.insert("timeline_diagnosis"); }
            Some("inspect_rotation_input") => { capabilities.insert("rotation_location"); }
            Some("inspect_timeline_events") => { capabilities.insert("event_location"); }
            Some("compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios" | "compare_focused_equipment" | "compare_equipment_strategies") => { capabilities.insert("controlled_comparison"); }
            Some("list_saved_artifacts" | "read_saved_artifact") => { capabilities.insert("saved_artifacts"); }
            Some("inspect_equipment_workspace" | "search_equipment_catalog") => { capabilities.insert("equipment_context"); }
            _ => {}
        }
    }
    capabilities.into_iter().map(str::to_string).collect()
}

fn explanation_evidence_stack_complete(capabilities: &[String]) -> bool {
    ["scenario_scope", "versioned_knowledge", "timeline_diagnosis", "event_location"]
        .iter()
        .all(|required| capabilities.iter().any(|actual| actual == required))
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
    draft_artifacts: super::artifacts::ArtifactStore,
    events: Vec<AgentTraceEventV1>,
    sink: Option<AgentTraceSink>,
    plan: AnalysisPlanV1,
    exposed_tools: Vec<String>,
    limits: AgentRunLimits,
    evidence_pack: Option<EvidencePackV1>,
    evidence_projection: Option<Value>,
    request_metrics: Vec<AgentModelRequestDebugV1>,
    tool_calls: Vec<AgentToolCallDebugV1>,
    resolved_domain_terms: Vec<ResolvedDomainTermV1>,
    terminology_index_hash: Option<String>,
    diagnostic_state: AgentDiagnosticStateV1,
}

impl TraceCollector {
    fn new(
        sink: Option<AgentTraceSink>,
        plan: AnalysisPlanV1,
        limits: AgentRunLimits,
        question: &str,
    ) -> Self {
        Self {
            draft_artifacts: super::artifacts::ArtifactStore::default(),
            events: Vec::new(),
            sink,
            plan,
            exposed_tools: Vec::new(),
            limits,
            evidence_pack: None,
            evidence_projection: None,
            request_metrics: Vec::new(),
            tool_calls: Vec::new(),
            resolved_domain_terms: Vec::new(),
            terminology_index_hash: None,
            diagnostic_state: AgentDiagnosticStateV1::new(question),
        }
    }

    fn set_exposed_tools(&mut self, exposed_tools: Vec<String>) {
        self.exposed_tools = exposed_tools;
    }

    fn set_resolved_domain_terms(
        &mut self,
        index_hash: &str,
        terms: &[ResolvedDomainTermV1],
    ) {
        self.terminology_index_hash = Some(index_hash.to_string());
        self.resolved_domain_terms = terms.to_vec();
    }

    fn refresh_evidence_pack(&mut self, evidence: &EvidenceStore) {
        self.evidence_pack = Some(build_evidence_pack(&self.plan, evidence));
        self.evidence_projection = Some(debug_evidence_projection(evidence));
        self.diagnostic_state.refresh_evidence(evidence);
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
            resolved_domain_terms: self.resolved_domain_terms.clone(),
            terminology_index_hash: self.terminology_index_hash.clone(),
            evidence_pack: self.evidence_pack.clone(),
            evidence_projection: self.evidence_projection.clone(),
            diagnostic_state: self.diagnostic_state.clone(),
        }
    }

    fn observe_model_action(&mut self, text: Option<&str>, calls: &[ProviderToolCall]) {
        self.diagnostic_state.observe_model_action(text, calls);
    }

    fn observe_evidence_progress(&mut self, has_new_evidence: bool) {
        self.diagnostic_state.no_new_evidence_rounds = if has_new_evidence {
            0
        } else {
            self.diagnostic_state.no_new_evidence_rounds.saturating_add(1)
        };
        self.diagnostic_state.status = "investigating".to_string();
    }

    fn mark_needs_user(&mut self) {
        self.diagnostic_state.status = "needs_user".to_string();
    }

    fn mark_ready_to_report(&mut self) {
        self.diagnostic_state.status = "ready_to_report".to_string();
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

fn bounded_public_text(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let bounded = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{}……", bounded.trim())
    } else {
        bounded
    }
}

fn compact_json(value: &Value, max_chars: usize) -> String {
    bounded_public_text(
        &serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
        max_chars,
    )
}

fn extract_hypothesis_sentences(value: &str, limit: usize) -> Vec<String> {
    value
        .split(['。', '！', '？', '\n'])
        .map(str::trim)
        .filter(|sentence| {
            !sentence.is_empty()
                && ["可能", "怀疑", "假设", "待验证", "推测", "更像"]
                    .iter()
                    .any(|marker| sentence.contains(marker))
        })
        .take(limit)
        .map(|sentence| bounded_public_text(sentence, 240))
        .collect()
}

fn contains_experiment_language(value: &str) -> bool {
    let normalized = value.to_lowercase();
    [
        "a/b",
        "ab测试",
        "同场景 a/b",
        "同场景a/b",
        "同场景对照",
        "对照验证",
        "对比验证",
        "直接 a/b",
        "直接a/b",
        "运行对照",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn diagnostic_fact_summary(envelope: &Value) -> Option<String> {
    let tool = envelope.get("tool_name")?.as_str()?;
    let result = envelope.get("result").unwrap_or(&Value::Null);
    let summary = match tool {
        "get_current_scenario" => format!(
            "场景：{} / {} / {}输入",
            result
                .get("game_version")
                .and_then(Value::as_str)
                .unwrap_or("版本未知"),
            result
                .get("mount")
                .and_then(Value::as_str)
                .unwrap_or("心法未知"),
            result
                .get("rotation_mode")
                .and_then(Value::as_str)
                .unwrap_or("未知")
        ),
        "simulate_scenario" => format!(
            "模拟：DPS {}，总伤害 {}",
            result
                .get("dps")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .get("total_damage")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string())
        ),
        "analyze_timeline" => format!(
            "时间轴：主动释放 {}，GCD 空档 {} 次，冷却等待 {} 次，实测溢出怒气 {}{}",
            result
                .get("active_event_count")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .pointer("/diagnostic_profile/cadence_gaps/count")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .pointer("/diagnostic_profile/cooldown_waits/count")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .pointer("/rage/overflow_total")
                .map(Value::to_string)
                .unwrap_or_else(|| "0".to_string()),
            result
                .pointer("/rage/overflow_sources")
                .and_then(Value::as_array)
                .filter(|sources| !sources.is_empty())
                .map(|sources| format!(
                    "（来源：{}）",
                    sources
                        .iter()
                        .take(3)
                        .filter_map(|source| Some(format!(
                            "{} {}点",
                            source.get("rage_source")?.as_str()?,
                            source.get("overflow_total")?
                        )))
                        .collect::<Vec<_>>()
                        .join("、")
                ))
                .unwrap_or_default()
        ),
        "inspect_timeline_events" => format!(
            "事件定位：{}，命中 {} 处",
            result
                .get("selector")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .get("total_matches")
                .map(Value::to_string)
                .unwrap_or_else(|| "0".to_string())
        ),
        "inspect_rotation_input" => format!(
            "循环定位：共 {} 个操作，本次命中 {} 处",
            result
                .get("total_items")
                .map(Value::to_string)
                .unwrap_or_else(|| "—".to_string()),
            result
                .get("matches")
                .and_then(Value::as_array)
                .map(|items| items.len().to_string())
                .unwrap_or_else(|| "0".to_string())
        ),
        "search_knowledge_base" => {
            let terms = result
                .get("resolved_terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|term| term.get("matched_surface").and_then(Value::as_str))
                .take(3)
                .collect::<Vec<_>>();
            let sources = result
                .get("results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    let title = item.get("title")?.as_str()?;
                    let heading = item.get("heading").and_then(Value::as_str).unwrap_or_default();
                    Some(if heading.is_empty() { title.to_string() } else { format!("{title} / {heading}") })
                })
                .take(2)
                .collect::<Vec<_>>();
            let confidence = result
                .pointer("/selection/confidence")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if sources.is_empty() {
                format!("知识检索：本次查询未取得可引用的当前版本资料（置信度 {confidence}）")
            } else {
                format!(
                    "知识检索：已取得当前版本资料 {}{}（置信度 {confidence}）",
                    sources.join("、"),
                    if terms.is_empty() { String::new() } else { format!("；已解析术语 {}", terms.join("、")) }
                )
            }
        }
        name if name.starts_with("compare_") => format!("对照实验：{name} 已完成"),
        _ => return None,
    };
    Some(summary)
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
    let mut time_budget = super::time_budget::AdaptiveTimeBudget::new(limits.wall_time_ms, provider.model());
    let prompt = agent_prompt();
    let analysis_plan = select_model_led_analysis_plan(
        &input.question,
        input.analysis_surface,
        &input.scenario,
    );
    let mut accounting = AgentRunAccountingV1::default();
    let mut trace = TraceCollector::new(
        event_sink,
        analysis_plan.clone(),
        limits.clone(),
        &input.question,
    );
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
    let equipment_workspace_available = input.equipment_workspace.is_some();
    let tools = definitions
        .into_iter()
        .filter(|tool| {
            !knowledge_only_client_scope
                || matches!(
                    tool.name.as_str(),
                    "get_current_scenario" | "search_knowledge_base" | ASK_USER_QUESTION
                )
        })
        .filter(|tool| {
            equipment_workspace_available
                || !matches!(
                    tool.name.as_str(),
                    "inspect_equipment_workspace"
                        | "compare_focused_equipment"
                        | "search_equipment_catalog"
                        | "compare_equipment_strategies"
                )
        })
        .filter(|tool| tool.name != "compare_focused_equipment" || equipment_focus_available)
        .collect::<Vec<_>>();
    trace.set_exposed_tools(tools.iter().map(|tool| tool.name.clone()).collect());
    let mut messages = Vec::new();
    if let Some(context) = &input.session_context {
        messages.push(ModelMessage::User {
            content: wrap_session_context(context),
        });
    }
    messages.push(ModelMessage::User {
        content: input.question.clone(),
    });
    if let Some(knowledge) = runtime.knowledge() {
        let knowledge_mount = match runtime.mount() {
            crate::Mount::FenShanJin => KnowledgeMountScope::Fenshanjin,
            crate::Mount::TieGuYi => KnowledgeMountScope::Tieguyi,
        };
        let knowledge_audience = KnowledgeAudience::from_question(&input.question, Some(knowledge_mount));
        let resolved_terms = knowledge.resolve_domain_terms_for_context(
            &input.question,
            &KnowledgeVersionContext::from_game_version(runtime.game_version()),
            knowledge_audience,
        );
        trace.set_resolved_domain_terms(knowledge.terminology_index_hash(), &resolved_terms);
        let model_terms = resolved_terms
            .iter()
            .filter(|term| {
                term.resolution != "current_scope_discovered_phrase"
                    || term
                        .cards
                        .iter()
                        .any(|card| card.confidence == "high")
            })
            .filter(|term| {
                term.cards.iter().any(|card| {
                    card.meaning_basis == "source_definition"
                        || card.kind != DomainTermKindV1::Acronym
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        if !model_terms.is_empty() {
            let projected = model_term_projection(
                &model_terms,
                &analysis_plan.resolved_scope.season,
            );
            let content = format!(
                "<domain_term_resolution server_generated=\"true\" evidence=\"false\">\n{}\n</domain_term_resolution>\n这些术语卡仅用于理解用户语言；需要在答案中引用定义时，请检索卡片所列来源取得正式 Evidence。",
                serde_json::to_string(&projected).unwrap_or_else(|_| "[]".to_string())
            );
            messages.push(ModelMessage::User { content });
            trace.push_checkpoint(
                "domain_terms_resolved",
                "解析领域语言",
                format!("从当前知识语料确认 {} 个版本化术语；模型仍自主决定分析路径。", model_terms.len()),
                Some("corpus_derived_terminology".to_string()),
                Vec::new(),
            );
            record_replay(
                &replay_sink,
                "domain_term_resolution",
                serde_json::json!({
                    "index_hash": knowledge.terminology_index_hash(),
                    "resolved_terms": resolved_terms,
                    "evidence": false,
                }),
            );
        }
    }
    let mut repairs = 0;
    let mut empty_response_retries = 0;
    let mut output_tokens = limits.max_output_tokens_per_turn;
    let mut provider_protocol_retries = 0;
    let mut tool_selection_retries = 0;
    let mut repair_message = None;
    let mut final_report_only = false;
    let mut deterministic_tool_cache = HashMap::<String, ToolDispatchOutcome>::new();
    let mut seen_knowledge_source_keys = HashSet::<String>::new();
    let mut reflected_deferred_actions = HashSet::<String>::new();
    let mut evidence_stack_guidance_sent = false;
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
    if !prefetched.budget_exhausted
        && prefetched
            .output
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        let cache_key = tool_result_cache_key(PREFETCH_TOOL, &serde_json::json!({}));
        if let Some(cache_key) = cache_key {
            deterministic_tool_cache.insert(cache_key, prefetched.clone());
        }
    }
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

    let mut restored = 0;
    for (name, arguments) in input.resume_tools.iter().take(24) {
        if !matches!(name.as_str(), "simulate_scenario" | "analyze_timeline" |
            "inspect_timeline_events" | "inspect_rotation_input") { continue; }
        let outcome = registry.dispatch(&input.run_id, name, arguments.clone());
        if outcome.output.get("ok").and_then(Value::as_bool) != Some(true) { continue; }
        if let Some(key) = tool_result_cache_key(name, arguments) {
            deterministic_tool_cache.insert(key, outcome.clone());
        }
        messages.push(ModelMessage::Assistant {
            content: None, reasoning_content: None,
            tool_calls: vec![ProviderToolCall {
                call_id: format!("resume-{restored}"), name: name.clone(), arguments: arguments.clone()
            }],
        });
        messages.push(ModelMessage::ToolResult {
            call_id: format!("resume-{restored}"), output: model_tool_output(&outcome.output),
        });
        trace.record_tool_call(&format!("resume-{restored}"), name, arguments, &outcome, true, false);
        record_replay(&replay_sink, "tool_dispatch", serde_json::json!({
            "call_id": format!("resume-{restored}"), "tool_name": name,
            "arguments": arguments, "output": outcome.output,
            "evidence_ids": outcome.evidence_ids, "server_initiated": true
        }));
        restored += 1;
    }
    if restored > 0 {
        messages.push(ModelMessage::User { content:
            format!("已从同会话同场景恢复并重新核对 {restored} 项只读检查，结果已登记在当前证据中。结合用户最新回答继续未完成的分析或实验。") });
        trace.push_checkpoint("analysis_resumed", "恢复分析进度",
            format!("已重新核对 {restored} 项历史检查，沿用同场景定位结果。"),
            None, Vec::new());
    }

    // Page state is trusted context. Candidate searches and comparisons remain
    // model-selected actions in the normal planning loop.
    if input.equipment_workspace.is_some() {
        const EQUIPMENT_INSPECT_CALL_ID: &str = "server-prefetch-equipment";
        const EQUIPMENT_INSPECT_TOOL: &str = "inspect_equipment_workspace";
        trace.push(
            "tool_started",
            Some(EQUIPMENT_INSPECT_TOOL.to_string()),
            Vec::new(),
            Some("server_equipment_prefetch".to_string()),
        );
        let inspected =
            registry.dispatch(&input.run_id, EQUIPMENT_INSPECT_TOOL, serde_json::json!({}));
        trace.record_tool_call(
            EQUIPMENT_INSPECT_CALL_ID,
            EQUIPMENT_INSPECT_TOOL,
            &serde_json::json!({}),
            &inspected,
            true,
            false,
        );
        accounting.tool_calls += 1;
        trace.push(
            "tool_finished",
            Some(EQUIPMENT_INSPECT_TOOL.to_string()),
            inspected.evidence_ids.clone(),
            inspected
                .output
                .pointer("/error/code")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        );
        record_replay(
            &replay_sink,
            "tool_dispatch",
            serde_json::json!({
                "call_id": EQUIPMENT_INSPECT_CALL_ID, "tool_name": EQUIPMENT_INSPECT_TOOL,
                "arguments": {}, "output": &inspected.output, "evidence_ids": &inspected.evidence_ids,
                "budget_exhausted": inspected.budget_exhausted, "server_initiated": true,
            }),
        );
        messages.push(ModelMessage::Assistant {
            content: None,
            tool_calls: vec![ProviderToolCall {
                call_id: EQUIPMENT_INSPECT_CALL_ID.to_string(),
                name: EQUIPMENT_INSPECT_TOOL.to_string(),
                arguments: serde_json::json!({}),
            }],
            reasoning_content: None,
        });
        messages.push(ModelMessage::ToolResult {
            call_id: EQUIPMENT_INSPECT_CALL_ID.to_string(),
            output: model_tool_output(&inspected.output),
        });
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
        time_budget.allow_output(output_tokens);
        if time_budget.advance(elapsed_ms(started), registry.evidence().len()) {
            trace.push_checkpoint("deadline_extended", "调整分析时限",
                format!("已取得新证据，结合模型耗时将本轮总时限调整为 {} 秒。", time_budget.deadline_ms() / 1000),
                None, Vec::new());
        }
        if started.elapsed() >= Duration::from_millis(time_budget.deadline_ms()) {
            if let Some(content) = recover_report_from_messages(
                &input.question,
                &analysis_plan,
                &messages,
                registry.evidence(),
            )
            .or_else(|| {
                task_preserving_provider_fallback(
                    &input.question, &trace.diagnostic_state,
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
                task_preserving_provider_fallback(
                    &input.question, &trace.diagnostic_state,
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

        let finishing_window = started.elapsed().as_millis() as u64
            >= time_budget.deadline_ms().saturating_sub(45_000.min(time_budget.deadline_ms() / 3));
        if (accounting.model_turns + 1 == limits.max_model_turns || finishing_window) && !final_report_only {
            final_report_only = true;
            messages.push(ModelMessage::User {
                content: format!("本轮进入收尾阶段，回答用户问题：{}。依据现有资料和观察给出当前判断，保留已定位对象和待执行实验，说明仍待确认的部分。", input.question),
            });
            trace.push("final_response_reserved", None, Vec::new(), Some("last_model_turn".to_string()));
        }
        let is_repair = repair_message.is_some();
        if final_report_only && !is_repair {
            // This branch is bounded by the remaining model turns and absolute ceiling.
            time_budget.reserve_completion(elapsed_ms(started));
        }
        let tools_available = !is_repair && !final_report_only;
        let repair_content = repair_message.take();
        let original_message_bytes = serde_json::to_vec(&messages)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        let mut request_messages = repair_content
            .map(|content| report_repair_messages(&input, &trace.diagnostic_state, content))
            .unwrap_or_else(|| compact_transcript_messages(&messages));
        prepend_mechanics_context(&mut request_messages, &input, registry.evidence());
        append_working_artifacts(&mut request_messages, &input, &trace.draft_artifacts);
        if !is_repair && accounting.model_turns > 0 {
            request_messages.push(ModelMessage::User {
                content: format!(
                    "<diagnostic_state server_generated=\"true\">\n{}\n</diagnostic_state>",
                    serde_json::to_string(&trace.diagnostic_state)
                        .unwrap_or_else(|_| "{}".to_string())
                ),
            });
        }
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
            max_output_tokens: output_tokens,
        };
        if should_handoff_request(&request, is_repair) {
            // Spend the available context on evidence before reducing it.
            // The complete serialized request, including schemas and escaping,
            // remains subject to the same transport limit below.
            for evidence_bytes in [28 * 1024, 20 * 1024, MODEL_EVIDENCE_HANDOFF_BYTES, 8 * 1024, 4 * 1024] {
                request_messages = compact_handoff_messages(
                    &input,
                    &messages,
                    registry.evidence(),
                    evidence_bytes,
                );
                prepend_mechanics_context(&mut request_messages, &input, registry.evidence());
                append_working_artifacts(&mut request_messages, &input, &trace.draft_artifacts);
                request_messages.push(ModelMessage::User {
                    content: format!(
                        "<diagnostic_state server_generated=\"true\">\n{}\n</diagnostic_state>",
                        serde_json::to_string(&trace.diagnostic_state)
                            .unwrap_or_else(|_| "{}".to_string())
                    ),
                });
                request.messages = request_messages.clone();
                // The soft threshold triggers compaction; it is not a second
                // context limit. Keep the useful evidence when the compacted
                // request already fits the real limit with transport headroom.
                if request_bytes(&request) <= MAX_MODEL_REQUEST_BYTES - 4 * 1024 {
                    trace.push(
                        "model_context_handoff",
                        None,
                        registry.evidence().keys().cloned().collect(),
                        Some(format!("bounded_request_{}_bytes", request_bytes(&request))),
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
            if let Some(content) = task_preserving_provider_fallback(
                &input.question, &trace.diagnostic_state,
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
        if let Err(protocol_error) = request.validate() {
            let recovered = if is_repair {
                false
            } else {
                let mut recovered_messages =
                    compact_handoff_messages(&input, &messages, registry.evidence(), 12 * 1024);
                append_working_artifacts(&mut recovered_messages, &input, &trace.draft_artifacts);
                recovered_messages.push(ModelMessage::User {
                    content: format!(
                        "<diagnostic_state server_generated=\"true\">\n{}\n</diagnostic_state>",
                        serde_json::to_string(&trace.diagnostic_state)
                            .unwrap_or_else(|_| "{}".to_string())
                    ),
                });
                request.messages = recovered_messages;
                request_bytes(&request) <= MAX_MODEL_REQUEST_BYTES && request.validate().is_ok()
            };
            record_replay(
                &replay_sink,
                "local_protocol_recovery",
                serde_json::json!({
                    "code": protocol_error.code,
                    "recovered": recovered,
                    "request_bytes": request_bytes(&request),
                }),
            );
            if recovered {
                trace.push(
                    "model_transcript_recovered",
                    None,
                    registry.evidence().keys().cloned().collect(),
                    Some(protocol_error.code.to_string()),
                );
            } else {
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
            Duration::from_millis(time_budget.deadline_ms()).saturating_sub(started.elapsed());
        let request_timeout = Duration::from_millis(time_budget.request_ms(elapsed_ms(started)));
        let timeout_code = if request_timeout < remaining { "provider_timeout" } else { "run_timeout" };
        record_replay(&replay_sink, "adaptive_time_budget", serde_json::json!({
            "model_turn": accounting.model_turns, "elapsed_ms": elapsed_ms(started),
            "deadline_ms": time_budget.deadline_ms(), "request_timeout_ms": request_timeout.as_millis()
        }));
        let request_started = Instant::now();
        let provider_result = tokio::select! {
            result = tokio::time::timeout(request_timeout, provider.complete(&request)) => Some(result),
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
            Some(Ok(Ok(response))) => {
                time_budget.observe_response(elapsed_ms(request_started));
                response
            },
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
                        "private_detail": &error.private_detail,
                    }),
                );
                add_usage(&mut accounting, &error.usage);
                time_budget.observe_response(elapsed_ms(request_started));
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
                    // A malformed action is not evidence that the investigation is
                    // complete. Give the model one clean chance to issue the action
                    // again; forcing an early report here used to erase precise event
                    // lookup and A/B work that the model had already decided to do.
                    final_report_only = false;
                    messages.push(ModelMessage::User {
                        content: "上一个工具动作的参数不是有效 JSON，因此没有执行。保留当前判断并重新发出该动作；参数必须是一个 JSON 对象，缺省字段也要按工具 schema 明确填写。若你决定不再执行，则说明原因并直接提交最终报告。".to_string(),
                    });
                    trace.push(
                        "provider_tool_arguments_retry",
                        None,
                        Vec::new(),
                        Some("retry_intended_action".to_string()),
                    );
                    continue;
                }
                if matches!(error.code, "provider_response_empty" | "provider_output_limit") {
                    if empty_response_retries < MAX_EMPTY_RESPONSE_RETRIES
                        && !registry.evidence().is_empty()
                        && accounting.model_turns < limits.max_model_turns
                    {
                        empty_response_retries += 1;
                        if error.code == "provider_output_limit" || error.usage.output_tokens >= u64::from(output_tokens) {
                            output_tokens = output_tokens.saturating_mul(2)
                                .min(super::provider::protocol::MAX_OUTPUT_TOKENS);
                        }
                        time_budget.reserve_completion(elapsed_ms(started));
                        final_report_only = true;
                        messages.push(ModelMessage::User {
                            content: "Return one complete AgentReportContentV1 JSON object from the evidence already present in this transcript. Express remaining uncertainty in limitations.".to_string(),
                        });
                        trace.push(
                            "provider_empty_retry",
                            None,
                            Vec::new(),
                            Some(if error.code == "provider_output_limit" { "output_allowance_retry" } else { "bounded_final_report_retry" }.to_string()),
                        );
                        continue;
                    }
                    if let Some(content) = task_preserving_provider_fallback(
                        &input.question, &trace.diagnostic_state,
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
                        task_preserving_provider_fallback(
                            &input.question, &trace.diagnostic_state,
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
                    timeout_code,
                    serde_json::json!({"model_turn": accounting.model_turns}),
                );
                if let Some(content) = task_preserving_provider_fallback(
                    &input.question,
                    &trace.diagnostic_state,
                    registry.evidence(),
                    "模型请求超时，本轮判断尚未完成。已完成的工具记录与分析进展已保留。",
                ) {
                    trace.push(
                        "provider_failure_evidence_preserved",
                        None,
                        Vec::new(),
                        Some(timeout_code.to_string()),
                    );
                    return terminal_with_report(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::TimedOut,
                        accounting,
                        content,
                        Some(fixed_error(timeout_code, "Agent model request reached its time limit")),
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
                    Some(fixed_error(timeout_code, "Agent model request reached its time limit")),
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
                if let Some(content) = task_preserving_provider_fallback(
                    &input.question, &trace.diagnostic_state,
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
        trace.observe_model_action(response.assistant_text.as_deref(), &response.tool_calls);
        trace.draft_artifacts.capture_report(response.assistant_text.as_deref().unwrap_or_default());
        for call in &response.tool_calls {
            trace.draft_artifacts.capture_tool(&call.name, &call.arguments);
        }

        let deferred_action = (!is_repair
            && response.tool_calls.is_empty()
            && reflected_deferred_actions.len() < 2
            && accounting.model_turns < limits.max_model_turns)
            .then(|| {
                unexecuted_plan_marker(
                    response.assistant_text.as_deref(),
                    &reflected_deferred_actions,
                    registry.evidence(),
                )
            })
            .flatten()
            .filter(|marker| {
                accounting.tool_calls < limits.max_tool_calls
                    && (marker != "proposed_comparison"
                        || registry.used_simulations() < limits.max_simulations)
            });
        if let Some(marker) = deferred_action {
            reflected_deferred_actions.insert(marker);
            final_report_only = false;
            // Preserve the answer that created the next action. Drafts also
            // have a dedicated, whole-text channel across transcript handoffs.
            messages.push(ModelMessage::Assistant {
                content: response.assistant_text.clone(),
                tool_calls: Vec::new(),
                reasoning_content: None,
            });
            messages.push(ModelMessage::User {
                content: "已保留当前候选。选择能改变判断的下一步；完整宏可用 compare_scenarios 的 candidates[].patch.macro_text 在冻结场景验证，随后交付完整候选与实际差异。信息足够时可直接完成回答。".to_string(),
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
            if response.tool_calls.len() == 1 && response.tool_calls[0].name == ASK_USER_QUESTION {
                let call = &response.tool_calls[0];
                match parse_clarification(&call.arguments) {
                    Ok(mut clarification) => {
                        clarification.analysis_text = response.assistant_text.as_deref()
                            .map(super::session::redact_sensitive_text)
                            .filter(|text| !text.trim().is_empty())
                            .map(|text| text.chars().take(12_000).collect());
                        trace.mark_needs_user();
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
            let available_knowledge_calls = MAX_KNOWLEDGE_SEARCHES
                .saturating_sub(registry.used_knowledge_searches());
            // Reserve uncached executions; identical successful calls share
            // the run-local cache when this batch is dispatched.
            let mut planned_knowledge_calls = 0_u32;
            let mut effective_tool_calls = 0_u32;
            for call in &response.tool_calls {
                if tool_result_cache_key(&call.name, &call.arguments)
                    .is_some_and(|key| deterministic_tool_cache.contains_key(&key))
                {
                    continue;
                }
                if call.name == "search_knowledge_base" {
                    if planned_knowledge_calls >= available_knowledge_calls {
                        continue;
                    }
                    planned_knowledge_calls += 1;
                }
                effective_tool_calls += 1;
            }
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
                if let Some(content) = task_preserving_provider_fallback(
                    &input.question, &trace.diagnostic_state,
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
            let mut dispatched_calls = 0_u32;
            let evidence_ids_before = registry.evidence().keys().cloned().collect::<HashSet<_>>();
            let mut batch_added_evidence = false;
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
                let cache_key = tool_result_cache_key(&call.name, &call.arguments);
                let cached = cache_key
                    .as_ref()
                    .and_then(|key| deterministic_tool_cache.get(key))
                    .cloned();
                if cached.is_none()
                    && call.name == "search_knowledge_base"
                    && knowledge_calls_processed >= available_knowledge_calls
                {
                    dispatched_calls += 1;
                    knowledge_calls_coalesced += 1;
                    let mut coalesced = AgentToolRegistry::coalesced_knowledge_search();
                    coalesced.output["error"]["message"] = Value::String(
                        "本轮新的知识查询已达到总次数上限。已有查询结果可以复用，其他定位、模拟和对照工具继续可用。".to_string(),
                    );
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
                if cached.is_none() && call.name == "search_knowledge_base" {
                    knowledge_calls_processed += 1;
                }
                dispatched_calls += 1;
                trace.push("tool_started", Some(call.name.clone()), Vec::new(), None);
                let arguments = call.arguments.clone();
                let reused = cached.is_some();
                let outcome = if let Some(cached) = cached {
                    cached
                } else {
                    registry.dispatch(&input.run_id, &call.name, arguments.clone())
                };
                if !reused {
                    accounting.tool_calls += 1;
                }
                if call.name == super::distillation::DISTILL_MACRO {
                    if let Some(envelopes) = outcome.output["evidence"].as_array() {
                        for envelope in envelopes {
                            if let Some(code) = envelope.pointer("/result/macro_text").and_then(Value::as_str) {
                                trace.draft_artifacts.extend([super::artifacts::DraftArtifactV1 {
                                    title: "宏蒸馏候选".into(), language: "jx3_macro".into(),
                                    content: code.into(), syntax: String::new(),
                                }]);
                            }
                        }
                    }
                }
                let semantic_knowledge_reuse = if call.name == "search_knowledge_base" {
                    let source_keys = knowledge_source_keys(&outcome.output);
                    let added_new = source_keys.into_iter().fold(false, |added, key| {
                        seen_knowledge_source_keys.insert(key) || added
                    });
                    batch_added_evidence |= added_new;
                    !added_new
                } else {
                    batch_added_evidence |= outcome.evidence_ids.iter()
                        .any(|id| !evidence_ids_before.contains(id));
                    false
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
                let mut projected_output = model_tool_output(&outcome.output);
                if semantic_knowledge_reuse {
                    if let Some(object) = projected_output.as_object_mut() {
                        object.insert("semantic_reuse".to_string(), Value::Bool(true));
                        object.insert(
                            "guidance".to_string(),
                            Value::String("该检索没有带来新的知识分块；请使用已取得内容形成判断，或改用能检验不同不确定性的工具。".to_string()),
                        );
                    }
                }
                messages.push(ModelMessage::ToolResult {
                    call_id: call.call_id,
                    output: projected_output,
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
                    Some("knowledge_query_limit_reached".to_string()),
                );
            }
            trace.refresh_evidence_pack(registry.evidence());
            if !evidence_stack_guidance_sent
                && explanation_evidence_stack_complete(
                    &trace.diagnostic_state.evidence_capabilities,
                )
                && accounting.model_turns < limits.max_model_turns
            {
                evidence_stack_guidance_sent = true;
                messages.push(ModelMessage::User {
                    content: "当前证据已经覆盖场景口径、当前版本资料、聚合时间轴和具体事件位置，足以形成带机制解释的回答。下一步由你决定：直接回答；或在已经能明确写出候选变量时运行一次对照。继续使用观察类工具时，请先指出一个现有证据尚未回答的具体事实。".to_string(),
                });
                trace.push_checkpoint(
                    "evidence_stack_complete",
                    "证据覆盖已成形",
                    "已具备资料、诊断与位置三层证据；模型可回答，或选择一次明确的证伪实验。".to_string(),
                    Some("model_decides_report_or_experiment".to_string()),
                    registry.evidence().keys().cloned().collect(),
                );
            }
            if dispatched_calls > 0 {
                trace.observe_evidence_progress(batch_added_evidence);
            }
            if dispatched_calls > 0 && !batch_added_evidence && accounting.model_turns < limits.max_model_turns {
                messages.push(ModelMessage::User {
                    content: "本轮调用复用了已有证据或未取得新内容。可依据已有证据回答，或针对仍未解决的问题调整查询、观察范围或实验变量；其余工具继续可用。".to_string(),
                });
                trace.push_checkpoint(
                    "evidence_reused",
                    "复用已有证据",
                    "本轮没有新增事实；模型可调整查询或实验，也可形成回答。".to_string(),
                    Some("deterministic_evidence_unchanged".to_string()),
                    registry.evidence().keys().cloned().collect(),
                );
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
        trace.mark_ready_to_report();
        trace.refresh_evidence_pack(registry.evidence());
        trace.push_checkpoint(
            "report_validation_started",
            "校验报告证据",
            "核对结构、数值、单位、来源和证据引用。".to_string(),
            None,
            evidence_pack.evidence_ids.clone(),
        );
        trace.push("validating", None, Vec::new(), None);
        let restored_report = trace.draft_artifacts.restore_report(response.assistant_text.as_deref().unwrap_or_default());
        let raw = restored_report.as_str();
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
                        Some("report_reference_normalized".to_string()),
                    );
                }
                let content = validated.content;
                let status = if content.refusal_reason.is_some() {
                    AgentRunStatus::Refused
                } else if content.findings.is_empty() && !content.artifacts.is_empty() {
                    AgentRunStatus::PartiallyVerified
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
                    if matches!(response.finish_reason, FinishReason::Length) {
                        output_tokens = output_tokens.saturating_mul(2)
                            .min(super::provider::protocol::MAX_OUTPUT_TOKENS);
                    }
                    time_budget.reserve_completion(elapsed_ms(started));
                    let repair_evidence = repair_evidence_context(registry.evidence());
                    let rejected_output = clip_model_text(raw, REPORT_REPAIR_REJECTED_CHARS);
                    repair_message = Some(
                        if matches!(response.finish_reason, FinishReason::Length) {
                            format!(
                            "The previous report was cut off by the output limit. Recreate one complete AgentReportContentV1 JSON object from the evidence below. Keep the direct answer, at most 4 findings, at most 1 metric per finding, and no more than 3000 Chinese characters total. Omit optional rotation_changes instead of expanding them. `limitations` is an array of strings and `refusal_reason` is a string or null.\n\nREPAIR_EVIDENCE_BEGIN\n{}\nREPAIR_EVIDENCE_END",
                            repair_evidence
                        )
                        } else {
                            format!(
                            "Correct the rejected output into one AgentReportContentV1 JSON object. Validation code: {}. Detail: {}. Use the registered evidence ids, metric values, units and JSON Pointers. Preserve supported analysis and repair the invalid fields. `limitations` is an array of strings; `refusal_reason` is a string or null; findings include `metrics`; rotation changes include `edit_operation` and `evidence_ids`. Use at most 4 findings, at most 1 metric per finding, and no more than 3000 Chinese characters total. Write concise, natural Chinese.\n\nREPAIR_EVIDENCE_BEGIN\n{}\nREPAIR_EVIDENCE_END\n\nREJECTED_OUTPUT_BEGIN\n{}\nREJECTED_OUTPUT_END",
                            error.code, error.message, repair_evidence, rejected_output
                        )
                        },
                    );
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
            },
        }
    }
}

fn model_term_projection(terms: &[ResolvedDomainTermV1], current_season: &str) -> Value {
    Value::Array(
        terms
            .iter()
            .take(4)
            .filter_map(|resolved| {
                let card = resolved
                    .cards
                    .iter()
                    .find(|card| card.season == current_season)
                    .or_else(|| resolved.cards.first())?;
                let source = card.sources.first();
                Some(serde_json::json!({
                    "matched_surface": resolved.matched_surface,
                    "resolution": resolved.resolution,
                    "term": {
                        "term_id": card.term_id,
                        "surface": card.surface,
                        "aliases": card.aliases,
                        "kind": card.kind,
                        "meaning": card.meaning.chars().take(240).collect::<String>(),
                        "meaning_basis": card.meaning_basis,
                        "season": card.season,
                        "audience": card.audience,
                        "category": card.category,
                        "fact_eligible": card.fact_eligible,
                        "confidence": card.confidence,
                    },
                    "source": source.map(|source| serde_json::json!({
                        "document_id": source.document_id,
                        "title": source.title,
                        "heading": source.heading,
                        "source_url": source.source_url,
                        "chunk_hash": source.chunk_hash,
                    })),
                    "other_seasons": resolved.cards.iter()
                        .filter(|candidate| candidate.term_id != card.term_id)
                        .map(|candidate| candidate.season.clone())
                        .collect::<Vec<_>>(),
                    "alternate_definitions": resolved.cards.iter()
                        .filter(|candidate| candidate.term_id != card.term_id)
                        .take(3)
                        .map(|candidate| serde_json::json!({
                            "season": candidate.season,
                            "audience": candidate.audience,
                            "category": candidate.category,
                            "fact_eligible": candidate.fact_eligible,
                            "meaning": candidate.meaning.chars().take(240).collect::<String>(),
                            "source_url": candidate.sources.first().map(|source| &source.source_url),
                        }))
                        .collect::<Vec<_>>(),
                }))
            })
            .collect(),
    )
}

fn recover_report_from_messages(
    _question: &str,
    _plan: &AnalysisPlanV1,
    messages: &[ModelMessage],
    evidence: &EvidenceStore,
) -> Option<AgentReportContentV1> {
    messages.iter().rev().find_map(|message| {
        let ModelMessage::Assistant {
            content: Some(raw), ..
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
    if output.get("model_projection").and_then(Value::as_str) == Some("bounded evidence projection")
        && model_json_bytes(output) <= MODEL_TOOL_OUTPUT_BYTES
    {
        return output.clone();
    }
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
                let ordered = result.clone();
                bound_json_value(result, 32, 1_600, 0);
                restore_compact_rotation_matches(&ordered, result);
            }
        }
    }
    if serde_json::to_vec(&projected)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
        > MODEL_TOOL_OUTPUT_BYTES
    {
        let ordered = projected.clone();
        bound_json_value(&mut projected, 12, 800, 0);
        if let Some(items) = projected.get_mut("evidence").and_then(Value::as_array_mut) {
            for (index, item) in items.iter_mut().enumerate() {
                if let Some(result) = item.get_mut("result") {
                    restore_compact_rotation_matches(&ordered["evidence"][index]["result"], result);
                }
            }
        }
    }
    if serde_json::to_vec(&projected)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
        > MODEL_TOOL_OUTPUT_BYTES
    {
        let source_evidence = output
            .get("evidence")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let envelope_budget = (MODEL_TOOL_OUTPUT_BYTES - 1_024) / source_evidence.len().max(1);
        let evidence = source_evidence.into_iter().map(|item| {
            shrink_model_evidence_item(&compact_handoff_evidence(item), envelope_budget)
        }).collect::<Vec<_>>();
        projected = serde_json::json!({
            "schema_version": projected.get("schema_version"),
            "ok": projected.get("ok"),
            "tool_name": projected.get("tool_name"),
            "evidence_ids": projected.get("evidence_ids"),
            "evidence": evidence,
            "model_projection": "bounded evidence projection"
        });
    }
    projected
}

fn project_tool_result(tool_name: &str, result: &mut Value) {
    if tool_name == "distill_macro" {
        if let Some(tuning) = result.get_mut("tuning").and_then(Value::as_object_mut) {
            tuning.remove("trials");
            if let Some(history) = tuning.get_mut("history").and_then(Value::as_array_mut) {
                for entry in history { if let Some(item) = entry.as_object_mut() { item.remove("macro_text"); } }
            }
        }
        if let Some(object) = result.as_object_mut() { object.remove("initial_macro_text"); }
    }
    // Full cooldown definitions remain in immutable evidence. Keep the
    // distinction needed to interpret timing without repeating every GCD.
    for pointer in ["/skill_semantics", "/rotation_input/skill_semantics"] {
        if let Some(skills) = result.pointer_mut(pointer).and_then(Value::as_object_mut) {
            for skill in skills.values_mut().filter_map(Value::as_object_mut) {
                skill.retain(|key, _| matches!(key.as_str(), "is_main_gcd" | "cooldown_semantics"));
            }
        }
    }
    match tool_name {
        "get_current_scenario" => {
            // This interpretation context is carried once, intact, in its own
            // message on every request (including repair and handoff). Generic
            // tool/result truncation must not sever a rule from its conditions.
            if let Some(object) = result.as_object_mut() {
                object.remove("mechanics_context");
            }
            if let Some(items) = result
                .pointer_mut("/rotation_input/manual_operations")
                .and_then(Value::as_array_mut)
            {
                items.truncate(16);
            }
        }
        "search_knowledge_base" => {
            if let Some(terms) = result.get_mut("resolved_terms").and_then(Value::as_array_mut) {
                *terms = terms
                    .iter()
                    .take(6)
                    .filter_map(compact_resolved_term)
                    .collect();
            }
            if let Some(items) = result.get_mut("results").and_then(Value::as_array_mut) {
                items.truncate(3);
                for item in items {
                    if let Some(object) = item.as_object_mut() {
                        object.retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "document_id"
                                    | "chunk_hash"
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
            if let Some(cycles) = result
                .pointer_mut("/rotation_cycles/cycles")
                .and_then(Value::as_array_mut)
            {
                cycles.truncate(12);
                for cycle in cycles {
                    if let Some(sequence) =
                        cycle.get_mut("key_sequence").and_then(Value::as_array_mut)
                    {
                        sequence.truncate(24);
                    }
                    if let Some(knives) = cycle
                        .get_mut("absolute_knives")
                        .and_then(Value::as_array_mut)
                    {
                        knives.truncate(8);
                    }
                }
            }
            if let Some(stats) = result
                .get_mut("macro_line_stats")
                .and_then(Value::as_array_mut)
            {
                stats.truncate(32);
            }
            if let Some(object) = result.as_object_mut() {
                object.insert("metric_catalog".to_string(), metric_catalog);
            }
        }
        "inspect_timeline_events" => {
            // Retain an inexpensive ordered neighborhood alongside each index
            // row, even when detailed state windows exceed the transport budget.
            let sequences = result.get("windows").and_then(Value::as_array).into_iter().flatten()
                .filter_map(|window| {
                    let number = window.pointer("/matched/event_number")?.as_u64()?;
                    let context = window.get("context")?.as_array()?;
                    let sequence = context.iter().map(|event| format!("{}ev{} {}s {}",
                        if event.get("event_number").and_then(Value::as_u64) == Some(number) { "*" } else { "" },
                        event.get("event_number").and_then(Value::as_u64).unwrap_or(0),
                        event.get("cast_time").and_then(Value::as_f64).unwrap_or(0.0),
                        event.get("skill_name").and_then(Value::as_str).unwrap_or("?")))
                        .collect::<Vec<_>>().join(" → ");
                    (!context.is_empty()).then_some((number, sequence))
                }).collect::<std::collections::BTreeMap<_, _>>();
            if let Some(index) = result.get_mut("match_index").and_then(Value::as_array_mut) {
                for event in index {
                    if let Some(sequence) = event.get("event_number").and_then(Value::as_u64).and_then(|number| sequences.get(&number)) {
                        event["local_sequence"] = Value::String(sequence.clone());
                    }
                }
            }
            if let Some(windows) = result.get_mut("windows").and_then(Value::as_array_mut) {
                *windows = windows
                    .iter()
                    .take(8)
                    .filter_map(|window| {
                        let object = window.as_object()?;
                        let matched = object.get("matched").map(compact_timeline_event_view)?;
                        let context = object
                            .get("context")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .take(9)
                            .map(compact_timeline_context_event)
                            .collect::<Vec<_>>();
                        Some(serde_json::json!({
                            "matched": matched,
                            "context": context,
                        }))
                    })
                    .collect();
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
            *result = compact_comparison_result(result, true);
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

fn compact_comparison_result(source: &Value, include_details: bool) -> Value {
    let metrics = |value: &Value| {
        let mut out = serde_json::Map::new();
        for key in ["dps", "total_damage", "fight_time", "skill_count", "fingerprint_hex"] {
            if let Some(value) = value.get(key) { out.insert(key.into(), value.clone()); }
        }
        Value::Object(out)
    };
    let candidates = source["candidates"].as_array().into_iter().flatten().take(3).map(|candidate| {
        let mut out = serde_json::json!({"label":candidate["label"],"metrics":metrics(&candidate["metrics"]),
            "delta_dps":candidate["delta_dps"],"delta_percent":candidate["delta_percent"],"same_fingerprint":candidate["same_fingerprint"]});
        if include_details {
            for key in ["macro_pages", "observed_outcome", "diagnostic_delta", "condition_semantics", "skill_deltas"] {
                if let Some(value) = candidate.get(key) { out[key] = value.clone(); }
            }
            bound_json_value(&mut out, 6, 500, 0);
        }
        out
    }).collect::<Vec<_>>();
    serde_json::json!({"baseline":metrics(&source["baseline"]),"candidates":candidates})
}

fn compact_resolved_term(term: &Value) -> Option<Value> {
    let object = term.as_object()?;
    let cards = object
        .get("cards")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(2)
        .filter_map(|card| {
            let card = card.as_object()?;
            let source_refs = card
                .get("sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(2)
                .map(|source| serde_json::json!({
                    "title": source.get("title"),
                    "heading": source.get("heading"),
                    "source_url": source.get("source_url"),
                }))
                .collect::<Vec<_>>();
            Some(serde_json::json!({
                "term_id": card.get("term_id"),
                "surface": card.get("surface"),
                "aliases": card.get("aliases"),
                "kind": card.get("kind"),
                "meaning": card.get("meaning"),
                "meaning_basis": card.get("meaning_basis"),
                "season": card.get("season"),
                "audience": card.get("audience"),
                "category": card.get("category"),
                "fact_eligible": card.get("fact_eligible"),
                "confidence": card.get("confidence"),
                "source_refs": source_refs,
            }))
        })
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "matched_surface": object.get("matched_surface"),
        "resolution": object.get("resolution"),
        "cards": cards,
    }))
}

fn compact_timeline_event_view(event: &Value) -> Value {
    let Some(object) = event.as_object() else {
        return event.clone();
    };
    let mut projected = serde_json::Map::new();
    for key in [
        "anchor_id",
        "local_sequence",
        "stance_before",
        "stance_after",
           "event_number",
           "cast_time",
            "operation_number",
        "skill_name",
        "macro_page",
        "macro_line",
        "gcd_seconds",
        "cooldown_wait_seconds",
        "rage_delta",
        "rage_before",
        "rage_after",
        "rage_overflow",
        "rage_overflow_sources",
        "rage_transactions",
        "rage_generated",
        "rage_gained",
        "rage_spent",
        "rage_cost",
        "damage_total",
        "state_before",
        "state_after",
        "buffs_before",
        "buffs_after",
        "absolute_knife",
        "prior_mechanic_landmarks",
    ] {
        if let Some(value) = object.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(projected)
}

fn compact_timeline_context_event(event: &Value) -> Value {
    let Some(object) = event.as_object() else {
        return event.clone();
    };
    let mut projected = serde_json::Map::new();
    for key in [
        "anchor_id",
        "operation_number",
        "stance_before",
        "stance_after",
        "rage_before",
        "rage_after",
        "event_number",
        "cast_time",
        "skill_name",
        "macro_page",
        "macro_line",
        "rage_delta",
        "rage_overflow",
        "rage_cost",
    ] {
        if let Some(value) = object.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    if let Some(rage) = event
        .pointer("/state_before/rage")
        .or_else(|| event.pointer("/state_after/rage"))
    {
        projected.insert("rage".to_string(), rage.clone());
    }
    for phase in ["before", "after"] {
        for field in ["stance", "rage"] {
            if let Some(value) = event.pointer(&format!("/state_{phase}/{field}")) {
                projected.insert(format!("{field}_{phase}"), value.clone());
            }
        }
    }
    Value::Object(projected)
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
        result.pointer(pointer).and_then(Value::as_f64).map(
            |value| serde_json::json!({"label": label, "value": value, "json_pointer": pointer}),
        )
    };
    let mut metrics = [
        ("主技能空档次数", "/diagnostic_profile/cadence_gaps/count"),
        (
            "主技能空档总时长",
            "/diagnostic_profile/cadence_gaps/total_seconds",
        ),
        ("冷却等待次数", "/diagnostic_profile/cooldown_waits/count"),
        (
            "冷却等待总时长",
            "/diagnostic_profile/cooldown_waits/total_seconds",
        ),
        ("怒气触顶采样", "/rage/at_cap_observations"),
        ("怒气采样总数", "/rage/sample_count"),
        ("实际怒气溢出事件", "/rage/overflow_events"),
        ("实际溢出怒气", "/rage/overflow_total"),
        ("尝试产生怒气", "/rage/generated_before_cap"),
        ("实际获得怒气", "/rage/gained_after_cap"),
        ("实际消耗怒气", "/rage/spent"),
        ("怒气产生量溢出占比", "/rage/overflow_percent_of_generated"),
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
            if object.contains_key("source_pointer") && object.contains_key("columns")
                && object.contains_key("constants") && object.contains_key("source_indices")
                && object.contains_key("rows")
            {
                // Columnar evidence is atomic here. Its dedicated reducer
                // removes complete rows together with their source indices.
                return;
            }
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
        "skill_id", "macro_text", "pages", "rule_diagnostics", "dropped_rules", "candidate_status", "tuning",
        "dps",
        "total_damage",
        "fight_time",
        "fingerprint_hex",
        "ranked_damage_sources",
        "active_event_count",
        "triggered_event_count",
        "buff_coverage",
        "total_cd_wait_seconds",
        "total_observed_gcd_gap_seconds",
        "skipped",
        "diagnostic_profile",
        "metric_catalog",
        "rotation_cycles",
        "macro_line_stats",
        "rage",
        "stance",
        "selection",
        "results",
        "resolved_terms",
        "terminology_index_hash",
        "candidates",
        "baseline",
        "rotation_input",
        "mode",
        "total_items",
        "returned_item_count",
        "skill_semantics",
        "query",
        "selector",
        "skill_name",
        "start_match",
        "returned_match_count",
        "returned_window_count",
        "page_start_index",
        "has_more",
        "matches",
        "window",
        "windows",
        "match_index",
        "match_index_table",
        "index_coverage",
        "match_index_truncated",
        "total_matches",
        "window_selection",
        "next_start_match",
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
    restore_compact_timeline_match_index(&Value::Object(result.clone()), &mut facts);
    restore_compact_timeline_windows(&Value::Object(result.clone()), &mut facts);
    restore_compact_rotation_matches(&Value::Object(result.clone()), &mut facts);
    facts
}

// Keep adjacent manual inputs adjacent. Generic prefix clipping of `before`
// retained the farthest operations and hid the ones immediately before a hit.
fn restore_compact_rotation_matches(source: &Value, target: &mut Value) {
    let Some(matches) = source.get("matches").and_then(Value::as_array) else { return };
    if matches.is_empty() { return }
    let entry = |value: &Value| {
        let mut fields = serde_json::Map::new();
        for key in ["operation_number", "sequence_index", "anchor_id", "skill_name", "timing_mode", "delay_seconds", "raw_timing_offset", "timing_offset_seconds", "cooldown_semantics", "is_main_gcd"] {
            if let Some(value) = value.get(key) { fields.insert(key.into(), value.clone()); }
        }
        Value::Object(fields)
    };
    let compact = matches.iter().take(8).map(|hit| {
        if !hit["matched"].is_object() { return hit.clone() }
        let before = hit["before"].as_array().cloned().unwrap_or_default();
        let after = hit["after"].as_array().cloned().unwrap_or_default();
        json_rotation_hit(entry(&hit["matched"]), before.iter().skip(before.len().saturating_sub(8)).map(&entry).collect(), after.iter().take(8).map(&entry).collect(), before.len() > 8 || after.len() > 8 || hit["context_partial"].as_bool() == Some(true))
    }).collect::<Vec<_>>();
    target["matches"] = Value::Array(compact);
}

fn json_rotation_hit(matched: Value, before: Vec<Value>, after: Vec<Value>, partial: bool) -> Value {
    serde_json::json!({"matched":matched,"before":before,"after":after,"context_partial":partial})
}

/// `match_index` is deliberately a lightweight all-occurrence index. The
/// generic transcript bound used to cut it to six entries, which made the
/// model page through an already available deterministic result one event at
/// a time. Restore a compact projection after bounding so a single inspection
/// can answer questions involving repeated events while the heavier
/// neighboring `windows` remain bounded.
fn restore_compact_timeline_match_index(source: &Value, target: &mut Value) {
    let Some(source_items) = source.get("match_index").and_then(Value::as_array) else {
        return;
    };
    let Some(target_object) = target.as_object_mut() else {
        return;
    };
    let compact = source_items
        .iter()
        .take(64)
        .filter_map(|item| {
            let object = item.as_object()?;
            let mut projected = serde_json::Map::new();
            for key in [
                "event_number",
                "local_sequence",
                "operation_number",
                "cast_time",
                "skill_name",
                "macro_page",
                "macro_line",
                "rage_before",
                "rage_after",
                "rage_cost",
                "rage_overflow",
                "rage_generated",
                "rage_gained",
                "rage_spent",
                "buffs_before",
                "absolute_knife",
                "prior_mechanic_landmarks",
            ] {
                if let Some(value) = object.get(key) {
                    projected.insert(key.to_string(), value.clone());
                }
            }
            for key in ["rage_overflow_sources", "rage_transactions"] {
                if let Some(values) = object.get(key).and_then(Value::as_array) {
                    if !values.is_empty() {
                        projected.insert(
                            key.to_string(),
                            Value::Array(values.iter().take(3).cloned().collect()),
                        );
                    }
                }
            }
            Some(Value::Object(projected))
        })
        .collect::<Vec<_>>();
    target_object.insert("match_index".to_string(), Value::Array(compact));
    if source_items.len() > 64 {
        target_object.insert("match_index_truncated".to_string(), Value::Bool(true));
    }
}

/// Full event windows are useful on the turn that requested them but are too
/// expensive to carry forever. Preserve their navigational and causal shape
/// in later turns so exact event evidence is never dropped as one oversized
/// item during transcript compaction.
fn restore_compact_timeline_windows(source: &Value, target: &mut Value) {
    fn compact_event(event: &Value) -> Value {
        let Some(object) = event.as_object() else {
            return Value::Null;
        };
        let mut compact = serde_json::Map::new();
        for key in [
            "event_number",
            "stance_before",
            "stance_after",
            "operation_number",
            "cast_time",
            "skill_name",
            "macro_page",
            "macro_line",
            "rage_delta",
            "rage_before",
            "rage_after",
            "rage_overflow",
            "rage_generated",
            "rage_gained",
            "rage_spent",
            "rage_cost",
            "rage_transactions",
            "rage_overflow_sources",
            "buffs_before",
            "buffs_after",
            "absolute_knife",
            "prior_mechanic_landmarks",
        ] {
            if let Some(value) = object.get(key) {
                compact.insert(key.to_string(), value.clone());
            }
        }
        for (source_key, target_key) in [
            ("/state_before/rage", "rage_before"),
            ("/state_after/rage", "rage_after"),
            ("/state_before/stance", "stance_before"),
            ("/state_after/stance", "stance_after"),
            ("/state_before/buffs", "buffs_before"),
            ("/state_after/buffs", "buffs_after"),
            ("/state_before/target_buffs", "target_buffs_before"),
            ("/state_after/target_buffs", "target_buffs_after"),
        ] {
            if let Some(value) = event.pointer(source_key) {
                compact.insert(target_key.to_string(), value.clone());
            }
        }
        Value::Object(compact)
    }

    let Some(windows) = source.get("windows").and_then(Value::as_array) else {
        return;
    };
    let Some(target_object) = target.as_object_mut() else {
        return;
    };
    let compact = windows
        .iter()
        .take(8)
        .filter_map(|window| {
            let matched = window.get("matched")?;
            let source_context = window
                .get("context")
                .and_then(Value::as_array)?;
            let matched_number = matched.get("event_number").and_then(Value::as_u64);
            let center = matched_number
                .and_then(|number| {
                    source_context.iter().position(|event| {
                        event.get("event_number").and_then(Value::as_u64) == Some(number)
                    })
                })
                .unwrap_or(0);
            let start = center.saturating_sub(4);
            let end = (center + 5).min(source_context.len());
            let context = source_context[start..end]
                .iter()
                .map(compact_event)
                .filter(|event| !event.is_null())
                .collect::<Vec<_>>();
            Some(serde_json::json!({
                "matched": compact_event(matched),
                "context": context,
            }))
        })
        .collect::<Vec<_>>();
    target_object.insert("windows".to_string(), Value::Array(compact));
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
    match envelope.get("tool_name").and_then(Value::as_str) {
        Some("distill_macro") => 0,
        Some("compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios") => 1,
        Some("compare_focused_equipment" | "compare_equipment_strategies") => 1,
        Some("search_knowledge_base") => 2,
        // Exact event reads answer the model's latest concrete question and
        // must survive transcript compaction. A rotation lookup that produced
        // no matches (common when a macro was mistakenly queried as a manual
        // sequence) carries no new detail and must not evict the frozen macro.
        Some("inspect_timeline_events") => 0,
        Some("inspect_rotation_input")
            if envelope
                .pointer("/result/matches")
                .and_then(Value::as_array)
                .is_some_and(|matches| !matches.is_empty()) =>
        {
            0
        }
        Some("get_current_scenario") => 1,
        Some("analyze_timeline") => 2,
        Some("simulate_scenario") => 3,
        _ => 4,
    }
}

fn model_evidence_group(item: &Value) -> &str {
    match item.get("tool_name").and_then(Value::as_str) {
        Some("get_current_scenario") => "scenario",
        Some("search_knowledge_base") => "knowledge",
        Some("simulate_scenario") => "simulation",
        Some("analyze_timeline") => "timeline",
        Some("inspect_timeline_events") => "events",
        Some("inspect_rotation_input") => "rotation",
        Some("compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios" | "compare_focused_equipment" | "compare_equipment_strategies") => "comparison",
        Some("list_saved_artifacts" | "read_saved_artifact") => "saved",
        Some("inspect_equipment_workspace" | "search_equipment_catalog") => "equipment",
        _ => "other",
    }
}

fn model_evidence_group_limit(group: &str) -> usize {
    match group {
        "knowledge" => 3,
        "events" | "rotation" => 4,
        "comparison" | "saved" | "equipment" => 2,
        _ => 1,
    }
}

fn model_json_bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map(|encoded| encoded.len()).unwrap_or(usize::MAX)
}

/// A reversible table removes repeated object keys while keeping exact values
/// and their paths in immutable evidence. Shared column values live in
/// `constants`; each row adds the remaining `columns` in order.
fn compact_record_table(records: &[Value], source_pointer: &str) -> Value {
    fn flatten(value: &Value, pointer: &str, row: &mut serde_json::Map<String, Value>) {
        if let Some(object) = value.as_object() {
            for (key, child) in object {
                let key = key.replace('~', "~0").replace('/', "~1");
                flatten(child, &format!("{pointer}/{key}"), row);
            }
        } else {
            row.insert(pointer.to_string(), value.clone());
        }
    }
    let rows = records.iter().map(|record| {
        let mut row = serde_json::Map::new();
        flatten(record, "", &mut row);
        row
    }).collect::<Vec<_>>();
    let all_columns = rows.iter().flat_map(|row| row.keys().cloned()).collect::<BTreeSet<_>>();
    let mut columns = Vec::new();
    let mut constants = serde_json::Map::new();
    for column in all_columns {
        let first = rows.first().and_then(|row| row.get(&column));
        if first.is_some() && rows.iter().all(|row| row.get(&column) == first) {
            constants.insert(column, first.cloned().unwrap_or(Value::Null));
        } else {
            columns.push(column);
        }
    }
    let mut array_columns = serde_json::Map::new();
    for column in &columns {
        let values = rows.iter().filter_map(|row| row.get(column)).collect::<Vec<_>>();
        if !values.is_empty() && values.iter().all(|value| value.as_array().is_some_and(|items| items.iter().all(Value::is_object))) {
            let fields = values.iter().flat_map(|value| value.as_array().into_iter().flatten())
                .flat_map(|item| { let mut fields = serde_json::Map::new(); flatten(item, "", &mut fields); fields.into_iter().map(|(key, _)| key) })
                .collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
            if !fields.is_empty() { array_columns.insert(column.clone(), serde_json::json!(fields)); }
        }
    }
    let encoded_rows = rows.iter().map(|row| columns.iter().map(|column| {
        let value = row.get(column).cloned().unwrap_or(Value::Null);
        match (array_columns.get(column).and_then(Value::as_array), value.as_array()) {
            (Some(fields), Some(items)) => Value::Array(items.iter().map(|item| {
                let mut flattened = serde_json::Map::new();
                flatten(item, "", &mut flattened);
                Value::Array(fields.iter().map(|field| flattened.get(field.as_str().unwrap_or_default()).cloned().unwrap_or(Value::Null)).collect())
            }).collect()),
            _ => value,
        }
    }).collect::<Vec<_>>()).collect::<Vec<_>>();
    serde_json::json!({
        "source_pointer": source_pointer,
        "encoding": "Each row uses columns (relative JSON pointers); add constants. Nested object-array cells use array_columns.",
        "columns": columns,
        "constants": constants,
        "array_columns": array_columns,
        "source_indices": (0..records.len()).collect::<Vec<_>>(),
        "rows": encoded_rows,
    })
}

fn table_compact_timeline_item(item: &Value) -> Value {
    let mut compact = item.clone();
    let Some(result) = compact.get_mut("result").and_then(Value::as_object_mut) else { return compact; };
    if let Some(mut matches) = result.remove("match_index").and_then(|value| value.as_array().cloned()) {
        let count = matches.len();
        let requested_events = result.get("windows").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|window| window.pointer("/matched/event_number").and_then(Value::as_u64)).collect::<HashSet<_>>();
        if let Some(windows) = result.get_mut("windows").and_then(Value::as_array_mut) {
            for window in windows {
                let number = window.pointer("/matched/event_number").cloned();
                if let Some(source) = matches.iter().find(|record| record.get("event_number") == number.as_ref()) {
                    if let Some(matched) = window.get_mut("matched").and_then(Value::as_object_mut) {
                        for key in ["rage_transactions", "prior_mechanic_landmarks"] {
                            if let Some(value) = source.get(key) { matched.insert(key.to_string(), value.clone()); }
                        }
                    }
                }
            }
        }
        for record in &mut matches {
            if let Some(fields) = record.as_object_mut() {
                fields.remove("rage_transactions");
                fields.remove("prior_mechanic_landmarks");
            }
        }
        let mut ordered = matches.into_iter().enumerate().collect::<Vec<_>>();
        ordered.sort_by_key(|(index, event)| (!event.get("event_number").and_then(Value::as_u64).is_some_and(|number| requested_events.contains(&number)), *index));
        let source_indices = ordered.iter().map(|(index, _)| *index).collect::<Vec<_>>();
        let records = ordered.into_iter().map(|(_, event)| event).collect::<Vec<_>>();
        let mut table = compact_record_table(&records, "/result/match_index");
        table["source_indices"] = serde_json::json!(source_indices);
        result.insert("match_index_table".to_string(), table);
        result.insert("index_coverage".to_string(), serde_json::json!({
            "returned_index_count": count,
            "total_matches": result.get("total_matches"),
            "next_unseen_match": (result.get("total_matches").and_then(Value::as_u64).unwrap_or(count as u64) > count as u64).then_some(count),
            "detail_page_start": result.get("start_match"),
            "detail_page_next": result.get("next_start_match"),
        }));
    }
    compact
}

fn shrink_model_evidence_item(item: &Value, max_bytes: usize) -> Value {
    if model_json_bytes(item) <= max_bytes {
        return item.clone();
    }
    if item.get("tool_name").and_then(Value::as_str) == Some("inspect_timeline_events") {
        let mut compact = table_compact_timeline_item(item);
        let indexed_events = item.pointer("/result/match_index").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|event| event.get("event_number").and_then(Value::as_u64)).collect::<HashSet<_>>();
        // Keep exact index facts first. Window context is the bounded detailed
        // page, so remove only neighboring duplicates before reducing coverage.
        while model_json_bytes(&compact) > max_bytes {
            let windows = compact.pointer_mut("/result/windows").and_then(Value::as_array_mut);
            if let Some(windows) = windows.filter(|windows| !windows.is_empty()) {
                // Reduce payload fields, never silently erase neighbors: their
                // ordering is essential to distinguish second/third casts.
                let mut reduced = false;
                for window in windows.iter_mut() {
                    for event in window.get_mut("context").and_then(Value::as_array_mut).into_iter().flatten() {
                        if let Some(fields) = event.as_object_mut() {
                            let before = fields.len();
                            fields.retain(|key, _| matches!(key.as_str(), "event_number" | "operation_number" | "cast_time" | "skill_name" | "stance_before" | "stance_after" | "rage_before" | "rage_after" | "rage_cost"));
                            reduced |= fields.len() < before;
                        }
                    }
                    if let Some(fields) = window.get_mut("matched").and_then(Value::as_object_mut) {
                        for key in ["buffs_before", "buffs_after", "target_buffs_before", "target_buffs_after", "rage_transactions", "prior_mechanic_landmarks"] {
                            reduced |= fields.remove(key).is_some();
                        }
                    }
                }
                if reduced {
                    continue;
                }
                if windows.len() > 1 {
                    if let Some(index) = windows.iter().rposition(|window| window.pointer("/matched/event_number").and_then(Value::as_u64).is_some_and(|number| indexed_events.contains(&number))) {
                        windows.remove(index);
                        compact["result"]["context_coverage"] = Value::from("partial: some event windows omitted for size; retrieve omitted event_number before classifying its sequence");
                        continue;
                    }
                }
            }
            let rows = compact.pointer_mut("/result/match_index_table/rows").and_then(Value::as_array_mut);
            if let Some(rows) = rows.filter(|rows| rows.len() > 1) {
                rows.pop();
                let retained = rows.len();
                let mut next_unseen = 0_u64;
                if let Some(indices) = compact.pointer_mut("/result/match_index_table/source_indices").and_then(Value::as_array_mut) {
                    indices.truncate(retained);
                    let included = indices.iter().filter_map(Value::as_u64).collect::<HashSet<_>>();
                    while included.contains(&next_unseen) { next_unseen += 1; }
                }
                compact["result"]["index_coverage"]["returned_index_count"] = Value::from(retained);
                compact["result"]["index_coverage"]["next_unseen_match"] = Value::from(next_unseen);
                compact["result"]["model_projection_truncated"] = Value::Bool(true);
                continue;
            }
            break;
        }
        if model_json_bytes(&compact) <= max_bytes { return compact; }
    }
    for (array_limit, string_limit) in [(20, 320), (12, 240), (6, 160)] {
        let mut compact = item.clone();
        bound_json_value(&mut compact, array_limit, string_limit, 0);
        if model_json_bytes(&compact) <= max_bytes {
            compact["model_projection_truncated"] = Value::Bool(true);
            if model_json_bytes(&compact) > max_bytes { continue; }
            return compact;
        }
    }
    let tool_name = item.get("tool_name").and_then(Value::as_str).unwrap_or_default();
    if matches!(tool_name, "compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios") {
        let core = serde_json::json!({"evidence_id":item.get("evidence_id"),"tool_name":tool_name,
            "result":compact_comparison_result(&item["result"], false)});
        if model_json_bytes(&core) <= max_bytes { return core }
    }
    if tool_name == "inspect_rotation_input" && item.pointer("/result/matches").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) {
        let mut compact = item.clone();
        restore_compact_rotation_matches(&item["result"], &mut compact["result"]);
        compact["result"].as_object_mut().unwrap().remove("skill_semantics");
        while model_json_bytes(&compact) > max_bytes {
            let matches = compact["result"]["matches"].as_array_mut().unwrap();
            if matches.len() <= 1 { break }
            let removed = matches.pop().unwrap();
            let retained = matches.len();
            compact["result"]["returned_match_count"] = Value::from(retained);
            compact["result"]["next_start_index"] = removed["matched"]["sequence_index"].clone();
            compact["result"]["has_more"] = Value::Bool(true);
        }
        if model_json_bytes(&compact) <= max_bytes { return compact }
        // Omit a whole window instead of presenting a non-adjacent prefix as
        // the preceding skills. The original query remains available.
        return serde_json::json!({"evidence_id":item.get("evidence_id"),"tool_name":tool_name,
            "projection":"Manual context window omitted for size; request one match with a smaller context_radius."});
    }
    let fallback_result = if tool_name == "inspect_timeline_events" {
        compact_timeline_event_index_result(item.get("result"))
    } else {
        compact_result_facts(item.get("result"))
    };
    let mut fallback = serde_json::json!({
        "evidence_id": item.get("evidence_id"),
        "tool_name": item.get("tool_name"),
        "args": item.get("args"),
        "result": fallback_result,
        "projection": "further detail remains in immutable server evidence"
    });
    for (array_limit, string_limit) in [(4, 160), (2, 80), (1, 40)] {
        if model_json_bytes(&fallback) <= max_bytes { return fallback; }
        bound_json_value(&mut fallback, array_limit, string_limit, 0);
    }
    if model_json_bytes(&fallback) > max_bytes {
        fallback = serde_json::json!({
            "evidence_id": item.get("evidence_id"),
            "tool_name": item.get("tool_name"),
            "args": item.get("args"),
            "projection": "Detailed result is registered; request a smaller observation window if needed."
        });
        if model_json_bytes(&fallback) > max_bytes { fallback.as_object_mut().unwrap().remove("args"); }
    }
    fallback
}

fn compact_timeline_event_index_result(result: Option<&Value>) -> Value {
    let Some(result) = result.and_then(Value::as_object) else {
        return serde_json::json!({});
    };
    let match_index = result
        .get("match_index")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
        .map(compact_timeline_event_view)
        .collect::<Vec<_>>();
    serde_json::json!({
        "selector": result.get("selector"),
        "total_matches": result.get("total_matches"),
        "match_index": match_index,
        "next_start_match": result.get("next_start_match"),
    })
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
    // Carry source prose before derived terminology, and aggregate cycle
    // shapes before the large per-event breakdown. These are projections of
    // existing fields, not generated gameplay judgments.
    if tool_name == "search_knowledge_base" {
        if let Some(object) = result.as_object_mut() {
            object.remove("resolved_terms");
        }
    }
    if tool_name == "analyze_timeline" {
        if let Some(object) = result.as_object_mut() { object.remove("metric_catalog"); }
        if let Some(profile) = result.get_mut("diagnostic_profile").and_then(Value::as_object_mut) {
            profile.remove("observed_strengths");
            profile.remove("observed_risks");
        }
        if let Some(cycles) = result.get_mut("rotation_cycles").and_then(Value::as_object_mut) {
            cycles.remove("cycles");
        }
    }
    let mut result = compact_result_facts(Some(&result));
    if tool_name == "analyze_timeline" {
        if let Some(cycles) = result
            .pointer_mut("/rotation_cycles/cycles")
            .and_then(Value::as_array_mut)
        {
            cycles.truncate(3);
            for cycle in cycles {
                if let Some(sequence) = cycle
                    .get_mut("key_sequence")
                    .and_then(Value::as_array_mut)
                {
                    sequence.truncate(8);
                }
                if let Some(knives) = cycle
                    .get_mut("absolute_knives")
                    .and_then(Value::as_array_mut)
                {
                    knives.truncate(4);
                }
            }
        }
    }
    let ordered_source = result.clone();
    bound_json_value(&mut result, 8, 500, 0);
    restore_ordered_macro_statements(&ordered_source, &mut result);
    restore_compact_timeline_match_index(&ordered_source, &mut result);
    restore_compact_timeline_windows(&ordered_source, &mut result);
    restore_compact_rotation_matches(&ordered_source, &mut result);
    serde_json::json!({
        "evidence_id": envelope.get("evidence_id"),
        "tool_name": tool_name,
        "args": envelope.get("args"),
        "result": result,
    })
}

fn model_evidence_handoff(evidence: &EvidenceStore, max_bytes: usize) -> String {
    model_evidence_handoff_prioritized(evidence, max_bytes, &[])
}

fn model_evidence_handoff_prioritized(
    evidence: &EvidenceStore,
    max_bytes: usize,
    preferred_evidence_ids: &[String],
) -> String {
    let mut candidates = evidence
        .values()
        .map(|envelope| {
            let projected = compact_handoff_evidence(envelope);
            let id = envelope
                .get("evidence_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (
                preferred_evidence_ids.iter().position(|preferred| preferred == id).unwrap_or(usize::MAX),
                evidence_priority(envelope),
                projected,
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(recency_priority, evidence_priority, item)| {
        (
            *recency_priority,
            *evidence_priority,
            item.get("evidence_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    });
    // The latest observation from each capability gets a first pass; older
    // pages can then use remaining space instead of one hash-selected event
    // query suppressing all other event observations.
    let mut first_groups = HashSet::new();
    let mut primary = Vec::new();
    let mut additional = Vec::new();
    for candidate in candidates {
        if first_groups.insert(model_evidence_group(&candidate.2).to_string()) {
            primary.push(candidate);
        } else {
            additional.push(candidate);
        }
    }
    let mut items = Vec::new();
    let mut omitted = Vec::new();
    let mut group_counts = HashMap::<String, usize>::new();
    let primary_count = primary.len();
    let mut remaining_primary_weight = primary.iter().map(|(_, _, item)| {
        if model_evidence_group(item) == "events" { 2 } else { 1 }
    }).sum::<usize>();
    for (position, (_, _, original_item)) in primary.into_iter().chain(additional).enumerate() {
        let group = model_evidence_group(&original_item).to_string();
        let count = group_counts.entry(group.clone()).or_default();
        if *count >= model_evidence_group_limit(&group) {
            if let Some(id) = original_item.get("evidence_id").and_then(Value::as_str) {
                omitted.push(id.to_string());
            }
            continue;
        }
        let occupied = model_json_bytes(&Value::Array(items.clone()));
        let remaining = max_bytes.saturating_sub(occupied).saturating_sub(1);
        if remaining < 384 {
            if let Some(id) = original_item.get("evidence_id").and_then(Value::as_str) { omitted.push(id.to_string()); }
            continue;
        }
        // Reserve useful room for every capability before carrying extra
        // pages. A large scenario/term list must not turn later evidence into
        // an ID-only header. Unused space is recycled for subsequent items.
        // Event pages contain both a reusable index and the requested detail
        // window; reserve two shares so neither is replaced by an ID header.
        let weight = if group == "events" { 2 } else { 1 };
        let item_budget = if position < primary_count {
            let budget = remaining * weight / remaining_primary_weight.max(weight);
            remaining_primary_weight = remaining_primary_weight.saturating_sub(weight);
            budget
        } else { remaining };
        let item = shrink_model_evidence_item(&original_item, item_budget);
        let mut trial = items.clone();
        trial.push(item.clone());
        let encoded = serde_json::to_vec(&trial)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        if encoded <= max_bytes {
            items.push(item);
            *count += 1;
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

fn recent_tool_evidence_ids(messages: &[ModelMessage], max_tool_results: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    messages
        .iter()
        .rev()
        .filter_map(|message| match message {
            ModelMessage::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .take(max_tool_results)
        .flat_map(|output| {
            output
                .get("evidence_ids")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .rev()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

fn prepend_mechanics_context(
    messages: &mut Vec<ModelMessage>,
    input: &AgentRunInput,
    evidence: &EvidenceStore,
) {
    if evidence.values().any(|value| value["tool_name"] == super::distillation::DISTILL_MACRO) {
        messages.retain(|message| !matches!(message, ModelMessage::User { content } if content.starts_with("<active_agent_skill")));
        messages.insert(0, ModelMessage::User { content: format!("<active_agent_skill id=\"macro-distillation/v2\">\n{}\n</active_agent_skill>", super::distillation::INSTRUCTIONS) });
    }
    let Some(envelope) = evidence.values().find(|envelope| {
        envelope.get("tool_name").and_then(Value::as_str) == Some("get_current_scenario")
            && envelope.get("scenario_hash").and_then(Value::as_str)
                == Some(input.scenario.scenario_hash.as_str())
            && envelope.pointer("/result/mechanics_context").is_some_and(Value::is_object)
    }) else { return; };
    let context = serde_json::json!({
        "evidence_id": envelope.get("evidence_id"),
        "scenario_hash": envelope.get("scenario_hash"),
        "source_pointer": "/result/mechanics_context",
        "context": envelope.pointer("/result/mechanics_context"),
    });
    messages.retain(|message| !matches!(message,
        ModelMessage::User { content } if content.starts_with("<mechanics_context")));
    messages.insert(0, ModelMessage::User {
        content: format!("<mechanics_context source=\"frozen_runtime\" purpose=\"domain_interpretation\">\n{context}\n</mechanics_context>"),
    });
}

fn append_working_artifacts(messages: &mut Vec<ModelMessage>, input: &AgentRunInput, current: &super::artifacts::ArtifactStore) {
    let mut working = super::artifacts::ArtifactStore::default();
    if let Some(context) = &input.session_context { working.capture_session(context); }
    working.extend(current.items());
    if let Some(content) = working.context_message() { messages.push(ModelMessage::User { content }); }
}

fn compact_transcript_messages(messages: &[ModelMessage]) -> Vec<ModelMessage> {
    messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User { content } if content.starts_with("<session_context") => {
                Some(ModelMessage::User {
                    content: bounded_session_message(content, 8_000),
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
            content: wrap_session_context(&compact_session_context(context, 8_000)),
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
    let preferred_evidence_ids = recent_tool_evidence_ids(transcript, 4);
    let failures = transcript.iter().rev().filter_map(|message| match message {
        ModelMessage::ToolResult { call_id, output } => Some((call_id, output)),
        _ => None,
    }).take(4).filter(|(_, output)| output.get("ok").and_then(Value::as_bool) == Some(false))
        .take(2).map(|(call_id, output)| {
            let arguments = transcript.iter().rev().find_map(|message| match message {
                ModelMessage::Assistant { tool_calls, .. } => tool_calls.iter()
                    .find(|call| &call.call_id == call_id).map(|call| call.arguments.clone()),
                _ => None,
            });
            let mut feedback = serde_json::json!({"tool_name": output.get("tool_name"), "arguments": arguments, "error": output.get("error")});
            bound_json_value(&mut feedback, 8, 500, 0);
            feedback
        }).collect::<Vec<_>>();
    if !failures.is_empty() {
        messages.push(ModelMessage::User {
            content: format!("<recent_tool_feedback>\n{}\n</recent_tool_feedback>", Value::Array(failures)),
        });
    }
    messages.push(ModelMessage::User {
        content: model_evidence_handoff_prioritized(
            evidence,
            evidence_bytes,
            &preferred_evidence_ids,
        ),
    });
    messages.push(ModelMessage::User {
        content: "前序消息已整理为上述证据与工作记录。继续完成当前用户目标，根据已有信息选择检索、定位、对照实验、追问或回答。需要细节时，可调整具名工具的查询范围读取原始观察。".to_string(),
    });
    if let Some(correction) = transcript.iter().rev().find_map(|message| match message {
        ModelMessage::User { content } if content != &input.question && !content.starts_with('<') => Some(content),
        _ => None,
    }) {
        messages.push(ModelMessage::User { content: clip_model_text(correction, 800) });
    }
    messages
}

/// Report repair is still the same user task. Retain its conversational
/// referents and current objective even when the normal transcript is omitted.
fn report_repair_messages(
    input: &AgentRunInput,
    state: &AgentDiagnosticStateV1,
    correction: String,
) -> Vec<ModelMessage> {
    let mut messages = Vec::new();
    if let Some(context) = &input.session_context {
        messages.push(ModelMessage::User {
            content: wrap_session_context(&compact_session_context(context, 8_000)),
        });
    }
    messages.push(ModelMessage::User { content: input.question.clone() });
    let focus = serde_json::json!({
        "user_objective": &input.question,
        "scenario_hash": &input.scenario.scenario_hash,
        "current_judgment": state.current_judgment.as_deref().map(|text| clip_model_text(text, 600)),
        "open_hypotheses": state.open_hypotheses.iter().take(4).map(|text| clip_model_text(text, 240)).collect::<Vec<_>>(),
        "last_actions": &state.last_actions,
    });
    messages.push(ModelMessage::User {
        content: format!("<report_task_context>\n{focus}\n</report_task_context>\n继续回答上面的用户问题，保留针对该问题的判断、具体位置和依据，修正下面指出的报告字段。"),
    });
    messages.push(ModelMessage::User { content: correction });
    messages
}

fn wrap_session_context(context: &str) -> String {
    format!("<session_context untrusted_data=\"true\" purpose=\"conversation_continuity\" fact_status=\"historical_assistant_interpretation\">\n{context}\n</session_context>")
}

fn bounded_session_message(message: &str, max_chars: usize) -> String {
    let Some((_, body)) = message.split_once('>') else { return message.to_string(); };
    let context = body.trim().strip_suffix("</session_context>").unwrap_or(body).trim();
    wrap_session_context(&compact_session_context(context, max_chars))
}

/// Session turns are chronological. Keep recent referents and valid JSON when
/// reducing history; prefix clipping retained the oldest answer and lost the
/// immediately preceding question in multi-turn follow-ups.
fn compact_session_context(context: &str, max_chars: usize) -> String {
    if context.chars().count() <= max_chars { return context.to_string(); }
    if let Ok(parsed) = serde_json::from_str::<Value>(context) {
        if let Some(turns) = parsed.get("turns").and_then(Value::as_array) {
            for count in [2usize, 1] {
                for field_chars in [240usize, 120, 60] {
                    let mut recent = turns.iter().rev().take(count).cloned().collect::<Vec<_>>();
                    recent.reverse();
                    for turn in &mut recent {
                        let code = turn.get("proposed_code_blocks").cloned();
                        let goal = turn.get("continuation_goal").cloned();
                        bound_json_value(turn, 2, field_chars, 0);
                        // Preserve small candidate artifacts whole; a sliced
                        // condition is a different proposal, not a summary.
                        if let Some(blocks) = code.and_then(|value| value.as_array().cloned()) {
                            turn["proposed_code_blocks"] = Value::Array(blocks.into_iter()
                                .filter(|block| block.as_str().is_some_and(|text| text.chars().count() <= 800)).take(2).collect());
                        }
                        if let Some(goal) = goal { turn["continuation_goal"] = goal; }
                    }
                    let compact = serde_json::json!({"schema_version": parsed.get("schema_version"), "purpose": "conversation_continuity", "fact_status": "historical_assistant_interpretation", "turns": recent, "compacted": true}).to_string();
                    if compact.chars().count() <= max_chars { return compact; }
                }
            }
            if let Some(last) = turns.last() {
                return serde_json::json!({"purpose": "conversation_continuity", "fact_status": "historical_assistant_interpretation", "turns": [{
                    "scenario_hash": last.get("scenario_hash"),
                    "prompt_version": last.get("prompt_version"),
                    "question": last.get("question").and_then(Value::as_str).map(|text| clip_model_text(text, 240)),
                    "summary": last.get("summary").and_then(Value::as_str).map(|text| clip_model_text(text, 600)),
                }], "compacted": true}).to_string();
            }
        }
    }
    let skip = context.chars().count().saturating_sub(max_chars.saturating_sub(1));
    format!("…{}", context.chars().skip(skip).collect::<String>())
}

fn clip_model_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    format!("{}…", value.chars().take(max_chars).collect::<String>())
}

fn should_handoff_request(request: &ModelRequest, is_repair: bool) -> bool {
    !is_repair && request_bytes(request) > MODEL_COMPACTION_TARGET_BYTES
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

fn unexecuted_plan_marker(
    assistant_text: Option<&str>,
    reflected_actions: &HashSet<String>,
    evidence: &EvidenceStore,
) -> Option<String> {
    let Some(value) = assistant_text
        .map(str::trim)
        .filter(|text| text.starts_with('{'))
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    else {
        return None;
    };
    let Some(object) = value.as_object() else {
        return None;
    };
    if object.contains_key("next_action")
        && (object.contains_key("action_plan") || object.contains_key("current_understanding"))
        && !object.contains_key("findings")
        && !object.contains_key("summary")
    {
        let marker = "structured_next_action".to_string();
        return (!reflected_actions.contains(&marker)).then_some(marker);
    }
    let recommendations = object
        .get("recommendations")
        .and_then(Value::as_array)
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_default();
    let named_tool = [
        "simulate_scenario",
        "analyze_timeline",
        "inspect_timeline_events",
        "inspect_rotation_input",
        "compare_scenarios",
        "compare_saved_macros",
        "compare_saved_scenarios",
        "compare_focused_equipment",
        "compare_equipment_strategies",
    ]
    .iter()
    .find(|tool| recommendations.contains(**tool) && !reflected_actions.contains(**tool)
        && !evidence.values().any(|item| item.get("tool_name").and_then(Value::as_str) == Some(**tool)))
    .map(|tool| (*tool).to_string());
    if named_tool.is_some() {
        return named_tool;
    }

    let comparison_already_ran = evidence.values().any(|envelope| {
        matches!(
            envelope.get("tool_name").and_then(Value::as_str),
            Some(
                "compare_scenarios"
                    | "compare_saved_macros"
                    | "compare_saved_scenarios"
                    | "compare_focused_equipment"
                    | "compare_equipment_strategies"
            )
        )
    });
    let proposes_experiment = contains_experiment_language(&recommendations);
    let has_executable_candidate = object
        .get("rotation_changes")
        .and_then(Value::as_array)
        .is_some_and(|changes| !changes.is_empty())
        || object.get("artifacts").and_then(Value::as_array).is_some_and(|items| !items.is_empty())
        || ["macro_replacements", "sequence_edits", "sequence_splices", "/cast ["]
            .iter()
            .any(|marker| recommendations.contains(marker));
    let marker = "proposed_comparison".to_string();
    (proposes_experiment
        && has_executable_candidate
        && !comparison_already_ran
        && !reflected_actions.contains(&marker))
    .then_some(marker)
}

fn is_reusable_deterministic_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "get_current_scenario"
            | "distill_macro"
            | "inspect_rotation_input"
            | "simulate_scenario"
            | "compare_scenarios"
            | "analyze_timeline"
            | "inspect_timeline_events"
            | "list_saved_artifacts"
            | "read_saved_artifact"
            | "compare_saved_macros"
            | "compare_saved_scenarios"
            | "search_knowledge_base"
    )
}

fn knowledge_source_keys(output: &Value) -> HashSet<String> {
    let mut keys = HashSet::new();
    let Some(evidence) = output.get("evidence").and_then(Value::as_array) else {
        return keys;
    };
    for envelope in evidence {
        if envelope.get("tool_name").and_then(Value::as_str) != Some("search_knowledge_base") {
            continue;
        }
        if let Some(results) = envelope.pointer("/result/results").and_then(Value::as_array) {
            if results.is_empty() {
                if let Some(arguments) = envelope.get("args") {
                    if let Ok(hash) = super::hash::canonical_sha256(arguments) {
                        keys.insert(format!("empty_query:{hash}"));
                    }
                }
            }
            for result in results {
                if let Some(key) = knowledge_source_key(result) {
                    keys.insert(key);
                }
            }
        }
        if let Some(terms) = envelope
            .pointer("/result/resolved_terms")
            .and_then(Value::as_array)
        {
            for term in terms {
                let Some(cards) = term.get("cards").and_then(Value::as_array) else {
                    continue;
                };
                for card in cards {
                    let Some(sources) = card.get("sources").and_then(Value::as_array) else {
                        continue;
                    };
                    for source in sources {
                        if let Some(key) = knowledge_source_key(source) {
                            keys.insert(key);
                        }
                    }
                }
            }
        }
    }
    keys
}

fn knowledge_source_key(source: &Value) -> Option<String> {
    let document_id = source.get("document_id").and_then(Value::as_str)?;
    let chunk = source.get("chunk_hash")
        .and_then(Value::as_str)
        .filter(|hash| !hash.is_empty())
        .map(str::to_string)
        .or_else(|| {
            // Compatibility with older source projections that kept an
            // excerpt but no chunk hash. A shared heading is not identity.
            let text = source.get("snippet").or_else(|| source.get("excerpt"))?.as_str()?;
            super::hash::canonical_sha256(&serde_json::json!({
                "document_hash": source.get("document_hash"),
                "season": source.get("season"),
                "text": text,
            })).ok()
        })?;
    Some(format!("{document_id}\0{chunk}"))
}

fn tool_result_cache_key(tool_name: &str, arguments: &Value) -> Option<String> {
    is_reusable_deterministic_tool(tool_name).then(|| {
        // Every requested field can change what the model learns. Hashing
        // canonical JSON affects identity only; execution receives the original
        // arguments and the registry records its validated effective query.
        super::hash::canonical_sha256(&serde_json::json!({
            "tool": tool_name,
            "arguments": arguments,
        })).ok()
    }).flatten()
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
        || limits.max_simulations > 1024
        || limits.max_output_tokens_per_turn == 0
        || limits.max_output_tokens_per_turn > super::provider::protocol::MAX_OUTPUT_TOKENS
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
    mut content: AgentReportContentV1,
    error: Option<AgentRunErrorV1>,
    mut trace: TraceCollector,
    started: Instant,
    registry: &AgentToolRegistry<'_>,
) -> AgentRunResultV1 {
    // Report repair/salvage may omit code. Restore the run's latest drafts as
    // candidate deliverables, independently of verified numeric claims.
    if content.artifacts.is_empty() { content.artifacts = trace.draft_artifacts.items(); }
    super::artifacts::normalize_artifacts(&mut content.artifacts);
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
    if report.is_none() && !trace.draft_artifacts.is_empty() {
        let mut content = refusal_content("已保留本轮候选草稿，验证尚未完成。", "候选尚未确认与目标循环一致，可在当前会话继续修改或验证。");
        content.refusal_reason = None;
        return terminal_with_report(provider, input, prompt, status, accounting, content, error, trace, started, registry);
    }
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
        artifacts: Vec::new(),
        limitations: vec![limitation.to_string()],
        refusal_reason: Some(summary.to_string()),
    }
}

fn task_preserving_provider_fallback(
    question: &str,
    state: &AgentDiagnosticStateV1,
    evidence: &EvidenceStore,
    limitation: &str,
) -> Option<AgentReportContentV1> {
    if !evidence.values().any(|item| item.get("tool_name").and_then(Value::as_str)
        .is_some_and(|name| name != "get_current_scenario")) { return None; }
    let findings = state.current_judgment.as_deref().filter(|text|
        !text.trim_start().starts_with('{') && !text.contains("</tool_call>")
    ).map(|text| AgentFindingV1 {
        title: "分析进展（尚未完成）".to_string(),
        explanation: clip_model_text(text, 800),
        evidence_ids: Vec::new(),
        metrics: Vec::new(),
    }).into_iter().collect();
    Some(AgentReportContentV1 {
        schema_version: AGENT_REPORT_CONTENT_SCHEMA_V1.to_string(),
        summary: format!("关于“{}”，本轮还未完成判断。已取得的工具记录保留在调试信息中。", question),
        findings,
        recommendations: Vec::new(),
        rotation_changes: Vec::new(),
        artifacts: Vec::new(),
        limitations: vec![limitation.to_string(), "以上为中途分析进展。".to_string()],
        refusal_reason: None,
    })
}

#[cfg(test)]
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
            == Some("inspect_timeline_events")
            && envelope.pointer("/result/match_index").is_some()
    }) {
        let indexed = envelope
            .pointer("/result/match_index")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let all = indexed
            .iter()
            .filter_map(|item| {
                let number = item.get("event_number")?.as_u64()?;
                let time = item.get("cast_time")?.as_f64()?;
                let skill = item.get("skill_name")?.as_str()?;
                Some(format!("[[{time:.2}s {skill}|ev:{number}]]"))
            })
            .collect::<Vec<_>>();
        let explanation = format!("已定位事件：{}。", all.join("、"));
        if !indexed.is_empty() {
            diagnostic_findings.push(AgentFindingV1 {
                title: "精确事件位置".to_string(),
                explanation,
                evidence_ids: vec![evidence_id.clone()],
                metrics: Vec::new(),
            });
        }
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
                    .partial_cmp(&left.get("total_damage").and_then(serde_json::Value::as_f64))
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
        if envelope
            .pointer("/result/rage/at_cap_observations")
            .is_some()
        {
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
            ("/result/rage/at_cap_observations", "怒气触顶采样", "count"),
            ("/result/rage/sample_count", "怒气采样总数", "count"),
        ] {
            if let Some(value) = envelope
                .pointer(pointer)
                .and_then(serde_json::Value::as_f64)
            {
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
                            json_pointer: format!("/result/buff_coverage/{index}/coverage_percent"),
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
        artifacts: Vec::new(),
        limitations: vec![limitation.to_string()],
        refusal_reason: None,
    })
}

/// Keep the model's useful expert judgment visible even when its JSON dialect
/// misses the typed report contract. Keep its subject intact; unrelated
/// baseline cards cannot stand in for a failed answer to a narrower question.
fn model_judgment_with_evidence_fallback(
    raw: &str,
    _plan: &AnalysisPlanV1,
    evidence: &super::report::EvidenceStore,
    limitation: &str,
) -> Option<AgentReportContentV1> {
    if evidence.is_empty() {
        return None;
    }
    let mut verified = AgentReportContentV1 {
        schema_version: AGENT_REPORT_CONTENT_SCHEMA_V1.to_string(),
        summary: String::new(),
        findings: Vec::new(),
        recommendations: Vec::new(),
        rotation_changes: Vec::new(),
        artifacts: Vec::new(),
        limitations: vec![limitation.to_string(), "以下模型解释尚未完成逐项证据校验。".to_string()],
        refusal_reason: None,
    };
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
            let explanation = [
                "explanation",
                "claim",
                "description",
                "statement",
                "finding",
            ]
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
        return None;
    }

    if let Some(summary) = summary {
        verified.summary = summary;
    } else if let Some(first) = judgments.first() {
        verified.summary = first.explanation.clone();
    }
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
    // Repairs use the same capability-balanced projection as ordinary handoffs,
    // so knowledge volume cannot evict simulation or event evidence.
    model_evidence_handoff(evidence, REPORT_REPAIR_EVIDENCE_BYTES)
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
    let mut options = parsed.options.clone();
    if options.len() == 1 || options.len() > 4 {
        return Err("options must be omitted or contain 2 to 4 answers");
    }
    let mut labels = HashSet::new();
    for option in &mut options {
        option.label = option.label.trim().to_string();
        option.description = option.description.trim().to_string();
        if option.label.is_empty() || option.label.chars().count() > 120
            || option.description.chars().count() > 240
            || !labels.insert(option.label.clone())
        {
            return Err("options must have distinct labels (1..120 characters) and descriptions up to 240 characters");
        }
    }
    let option_text = options.iter().map(|option| format!("{} {}", option.label, option.description)).collect::<Vec<_>>().join(" ");
    let sensitive =
        format!("{question} {reason} {} {option_text}", answer_hint.unwrap_or_default()).to_ascii_lowercase();
    if [
        "api key", "apikey", "token", "password", "密码", "密钥", "令牌",
    ]
    .iter()
    .any(|needle| sensitive.contains(needle))
    {
        return Err("clarification cannot request credentials or secrets");
    }
    // Intent belongs to the model; this parser validates structure and secret safety.
    Ok(AgentClarificationV1 {
        analysis_text: None,
        schema_version: "agent-clarification/v1".to_string(),
        question: question.to_string(),
        reason: reason.to_string(),
        answer_hint: answer_hint.map(str::to_string),
        options,
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

    #[tokio::test]
    async fn distillation_skill_loads_on_demand_and_publishes_only_selected_macro() {
        let runtime = AgentRuntime::fixture();
        let final_report = json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,
            "summary":"最终候选见下方。", "findings":[], "recommendations":[],
            "rotation_changes":[], "limitations":["测试仅验证交付流程。"], "refusal_reason":null,
            "artifacts":[{"title":"最终宏", "language":"jx3_macro", "content":"#page shield\n/cast 盾击"}]}).to_string();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("generate", "distill_macro", json!({}))),
            Ok(ModelResponse { assistant_text:Some(final_report), reasoning_content:None,
                tool_calls:vec![], finish_reason:FinishReason::Stop, usage:TokenUsage::default() })]);
        let result = run_agent(&provider, &runtime, input(&runtime,"distill-skill-test"),
            AgentRunLimits::default(), AgentCancellation::default()).await;
        let requests = provider.requests();
        assert!(requests[0].tools.iter().any(|tool| tool.name == "distill_macro"));
        let first = serde_json::to_string(&requests[0].messages).unwrap();
        assert!(!first.contains("<active_agent_skill"));
        let second = serde_json::to_string(&requests[1].messages).unwrap();
        assert!(second.contains("<active_agent_skill"));
        assert!(second.contains("macro-distillation/v2"));
        assert!(second.contains("working_artifacts"));
        let report = result.report.unwrap();
        assert_eq!(report.content.artifacts.len(), 1);
        assert_eq!(report.content.artifacts[0].title, "最终宏");
        assert!(result.accounting.simulations >= 2);
    }

    #[tokio::test]
    async fn real_macro_draft_survives_continuation_and_final_omission() {
        let runtime = AgentRuntime::fixture();
        let draft = include_str!("../../tests/agent_delivery_eval/macro-draft-v44.json");
        let response = |text: String| Ok(ModelResponse {
            assistant_text: Some(text), reasoning_content: None,
            tool_calls: Vec::new(), finish_reason: FinishReason::Stop,
            usage: TokenUsage::default(),
        });
        let final_text = json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,
            "summary":"当前宏候选见下方，复刻效果尚待核对。", "findings":[],
            "recommendations":[], "rotation_changes":[], "artifacts":[],
            "limitations":[], "refusal_reason":null}).to_string();
        let provider = ScriptedProvider::new(vec![response(draft.into()), response(final_text)]);
        let mut run_input = input(&runtime, "run-delivery-replay");
        run_input.question = "把宏写出来".into();
        let result = run_agent(&provider, &runtime, run_input, AgentRunLimits::default(), AgentCancellation::default()).await;
        let report = result.report.expect("draft must be delivered");
        assert_eq!(result.accounting.model_turns, 2);
        assert_eq!(result.status, AgentRunStatus::PartiallyVerified);
        assert_eq!(report.content.artifacts.len(), 1);
        let original: Value = serde_json::from_str(draft).unwrap();
        assert_eq!(report.content.artifacts[0].content, original["rotation_changes"][0]["proposed"].as_str().unwrap());
        assert!(provider.requests()[1].messages.iter().any(|m| matches!(m,
            ModelMessage::User { content } if content.starts_with("<working_artifacts") && content.contains("/cast [rage<20] 盾回"))));
    }

    #[test]
    fn handoff_and_repair_keep_whole_artifacts_separate_from_old_history() {
        let runtime = AgentRuntime::fixture();
        let mut run_input = input(&runtime, "run-artifact-handoff");
        let code = format!("#page shield\n{}\n#page blade\n/cast 斩刀", "/cast [nobuff:血怒·惊涌] 血怒\n".repeat(30));
        run_input.session_context = Some(json!({"turns":[{"artifacts":[{"title":"宏", "language":"jx3_macro", "content":code}]}]}).to_string());
        let current = super::super::artifacts::ArtifactStore::default();
        let mut messages = compact_handoff_messages(&run_input, &[], &EvidenceStore::new(), 4096);
        append_working_artifacts(&mut messages, &run_input, &current);
        let payload = messages.iter().find_map(|m| match m {
            ModelMessage::User { content } if content.starts_with("<working_artifacts") => Some(content), _ => None
        }).unwrap();
        let json = payload.split_once('>').unwrap().1.split("</working_artifacts>").next().unwrap();
        let drafts: Value = serde_json::from_str(json).unwrap();
        assert_eq!(drafts[0]["content"], code);
        assert!(current.is_empty(), "historical drafts must not be auto-published into an unrelated answer");
    }

    #[test]
    fn manual_lookup_compaction_preserves_immediate_predecessors() {
        let entry = |n| json!({"operation_number":n,"sequence_index":n-1,"skill_name":format!("技能{n}"),"timing_mode":"as_soon_as_available"});
        let full = json!({"tool_name":"inspect_rotation_input","evidence_id":"a".repeat(64),
            "result":{"matches":[{"matched":entry(20),"before":(1..20).map(entry).collect::<Vec<_>>(),"after":(21..32).map(entry).collect::<Vec<_>>() }],"total_matches":1}});
        let compact = compact_handoff_evidence(&full);
        let compact = compact_handoff_evidence(&compact);
        let before = compact["result"]["matches"][0]["before"].as_array().unwrap();
        assert_eq!(before.last().unwrap()["operation_number"],20-1);
        assert_eq!(before[0]["operation_number"],12);
        assert_eq!(compact["result"]["matches"][0]["context_partial"],true);
        let tiny = shrink_model_evidence_item(&compact, 350);
        assert!(tiny.pointer("/result/matches").is_none(), "tiny budgets must omit a whole window, not lie about adjacency");
    }

    #[test]
    fn comparison_projection_always_keeps_both_sides_and_delta() {
        let envelope = json!({"evidence_id":"a".repeat(64),"tool_name":"compare_scenarios",
            "result":{"baseline":{"dps":1000.0,"fight_time":300.0},"candidates":[{
                "label":"宏候选","metrics":{"dps":500.0,"fight_time":300.0},"delta_dps":-500.0,
                "delta_percent":-50.0,"same_fingerprint":false,
                "changes":[{"field":"simulation.sequence","before":vec!["盾击";2000],"after":vec!["__macro__";2000]}],
                "skill_deltas":[],"diagnostic_delta":{},"observed_outcome":{},"condition_semantics":[]}]}});
        let output = json!({"ok":true,"tool_name":"compare_scenarios","evidence":[envelope]});
        let projected = model_tool_output(&model_tool_output(&output));
        assert_eq!(projected["evidence"][0]["result"]["baseline"]["dps"],1000.0);
        assert_eq!(projected["evidence"][0]["result"]["candidates"][0]["metrics"]["dps"],500.0);
        let handoff = compact_handoff_evidence(&projected["evidence"][0]);
        let tiny = shrink_model_evidence_item(&handoff,1024);
        assert_eq!(tiny["result"]["baseline"]["dps"],1000.0);
        assert_eq!(tiny["result"]["candidates"][0]["delta_dps"],-500.0);
        assert!(model_json_bytes(&tiny)<=1024);
    }

    #[tokio::test]
    async fn final_turn_candidate_is_delivered_without_reopening_tools() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
            assistant_text:Some(include_str!("../../tests/agent_delivery_eval/macro-draft-v44.json").into()),
            reasoning_content:None, tool_calls:Vec::new(), finish_reason:FinishReason::Stop, usage:TokenUsage::default(),
        })]);
        let result = run_agent(&provider, &runtime, input(&runtime,"run-draft-final-turn"),
            AgentRunLimits { max_model_turns:1, ..AgentRunLimits::default() }, AgentCancellation::default()).await;
        assert_eq!(provider.requests().len(),1);
        assert!(provider.requests()[0].tools.is_empty());
        assert!(!result.report.unwrap().content.artifacts.is_empty());
    }

    #[tokio::test]
    async fn trimmed_research_batch_keeps_remaining_capacity_for_candidate_comparison() {
        let runtime = AgentRuntime::fixture();
        let response = |text: String| Ok(ModelResponse { assistant_text:Some(text), reasoning_content:None,
            tool_calls:Vec::new(), finish_reason:FinishReason::Stop, usage:TokenUsage::default() });
        let mut oversized = tool_call("look-0", "inspect_rotation_input", json!({"start_index":0}));
        for n in 1..4 { oversized.tool_calls.push(ProviderToolCall { call_id:format!("look-{n}"), name:"inspect_rotation_input".into(), arguments:json!({"start_index":n}) }); }
        let provider = ScriptedProvider::new(vec![
            Ok(oversized),
            response(include_str!("../../tests/agent_delivery_eval/macro-draft-v44.json").into()),
            Ok(tool_call("validate-draft", "compare_scenarios", json!({"candidates":[{"label":"候选", "patch":{"macro_text":"/cast 盾击"}}]}))),
            response(json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,"summary":"候选已保留。","findings":[],"recommendations":[],"rotation_changes":[],"artifacts":[],"limitations":[],"refusal_reason":null}).to_string()),
        ]);
        let result = run_agent(&provider, &runtime, input(&runtime,"run-trim-then-compare"),
            AgentRunLimits {max_tool_calls:3,..AgentRunLimits::default()},AgentCancellation::default()).await;
        let requests = provider.requests();
        assert!(requests[1].tools.is_empty());
        assert!(requests[2].tools.iter().any(|t| t.name == "compare_scenarios"));
        assert!(result.debug.as_ref().unwrap().tool_calls.iter().any(|t| t.tool_name == "compare_scenarios" && t.ok));
        assert!(result.accounting.tool_calls <= 3);
        assert!(!result.report.unwrap().content.artifacts.is_empty());
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
    fn compact_facts_keep_all_lightweight_timeline_matches() {
        let matches = (0..17)
            .map(|index| {
                json!({
                    "event_number": index + 1,
                    "cast_time": index as f64 + 0.5,
                    "skill_name": "绝刀·50怒",
                    "macro_page": 1,
                    "macro_line": 3,
                    "rage_before": 80,
                    "rage_after": 30,
                    "rage_cost": 50,
                    "rage_overflow": 0,
                    "rage_overflow_sources": [],
                    "rage_transactions": [],
                    "rage_generated": 0,
                    "rage_gained": 0,
                    "rage_spent": 50,
                    "buffs_before": [],
                    "absolute_knife": {
                        "yuan_ge_before": 0,
                        "yuan_ge_after": 0,
                        "triggered_yuan_ge_blood_shadow": false,
                        "without_yuan_ge_blood_shadow": true,
                        "blood_rage_active": false,
                        "kuang_jue_active": false,
                        "tian_xia_hong_yuan_active": false
                    }
                })
            })
            .collect::<Vec<_>>();
        let compact = compact_result_facts(Some(&json!({
            "selector": "skill",
            "skill_name": "绝刀",
            "total_matches": 17,
            "match_index": matches,
            "windows": []
        })));

        assert_eq!(
            compact
                .get("match_index")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(17)
        );
        assert_eq!(
            compact
                .pointer("/match_index/16/absolute_knife/without_yuan_ge_blood_shadow")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn large_event_evidence_uses_a_bounded_reversible_index() {
        let item = large_event_projection_fixture("a", 0);
        let compact = shrink_model_evidence_item(&compact_handoff_evidence(&item), 8 * 1024);
        assert!(model_json_bytes(&compact) <= 8 * 1024);
        let table = &compact["result"]["match_index_table"];
        assert_eq!(table["source_pointer"], "/result/match_index");
        assert_eq!(table["rows"].as_array().unwrap().len(), 64);
        assert_eq!(table["source_indices"][63], 63);
        let columns = table["columns"].as_array().unwrap();
        for (row_index, field) in [(0, "/rage_before"), (63, "/cast_time")] {
            let value = if let Some(column) = columns.iter().position(|value| value.as_str() == Some(field)) {
                &table["rows"][row_index][column]
            } else {
                &table["constants"][field]
            };
            assert_eq!(value, item["result"]["match_index"][row_index].pointer(field).unwrap());
        }
        let buff_column = columns.iter().position(|field| field == "/buffs_before").unwrap();
        let buff_fields = table["array_columns"]["/buffs_before"].as_array().unwrap();
        let remaining_column = buff_fields.iter().position(|field| field == "/remaining_seconds").unwrap();
        assert_eq!(table["rows"][63][buff_column][0][remaining_column], item["result"]["match_index"][63]["buffs_before"][0]["remaining_seconds"]);
        assert_eq!(compact["result"]["index_coverage"]["next_unseen_match"], 64);
        assert_eq!(compact["args"]["buff_names"][0], "测试增益");
        for budget in [384, 1_024, 4_096] {
            assert!(model_json_bytes(&shrink_model_evidence_item(&compact_handoff_evidence(&item), budget)) <= budget);
        }
        let later_page = shrink_model_evidence_item(&compact_handoff_evidence(&large_event_projection_fixture("b", 48)), 4_096);
        let indices = later_page["result"]["match_index_table"]["source_indices"].as_array().unwrap();
        assert_eq!(indices[0], 48);
        assert!((48..56).all(|index| indices.contains(&Value::from(index))));
    }

    #[test]
    fn latest_event_pages_survive_handoff_in_observation_order() {
        let mut evidence = EvidenceStore::new();
        for (id, page) in [("0", 0), ("a", 8), ("f", 16)] {
            let mut item = large_event_projection_fixture(id, page);
            item["result"]["match_index"].as_array_mut().unwrap().truncate(2);
            item["result"]["windows"] = json!([]);
            evidence.insert(id.repeat(64), item);
        }
        let handoff = model_evidence_handoff_prioritized(&evidence, 12 * 1024, &["f".repeat(64), "a".repeat(64)]);
        let payload: Value = serde_json::from_str(handoff.lines().nth(1).unwrap()).unwrap();
        assert_eq!(payload["items"][0]["evidence_id"], "f".repeat(64));
        assert_eq!(payload["items"][0]["args"]["start_match"], 16);
        assert!(payload["items"].as_array().unwrap().iter().any(|item| item["evidence_id"] == "a".repeat(64)));

        // Optional private replay audit supplements the portable fixture. Only
        // deterministic tool envelopes are read; provider reasoning is ignored.
        if let Some(directory) = std::env::var_os("JX3_AGENT_REPLAY_EVENTS") {
            let mut files = fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
            files.sort();
            let mut replay_evidence = EvidenceStore::new();
            let mut transcript = Vec::new();
            let mut latest_event_id = None;
            for file in files {
                let event: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
                if event["kind"] != "tool_dispatch" { continue; }
                let payload = &event["payload"];
                for envelope in payload["output"]["evidence"].as_array().into_iter().flatten() {
                    let id = envelope["evidence_id"].as_str().unwrap().to_string();
                    if envelope["tool_name"] == "inspect_timeline_events" { latest_event_id = Some(id.clone()); }
                    replay_evidence.insert(id, envelope.clone());
                }
                transcript.push(ModelMessage::ToolResult {
                    call_id: payload["call_id"].as_str().unwrap_or_default().to_string(),
                    output: payload["output"].clone(),
                });
            }
            let latest_id = latest_event_id.expect("replay contains an event observation");
            let ordered_ids = recent_tool_evidence_ids(&transcript, 4);
            let handoff = model_evidence_handoff_prioritized(&replay_evidence, 12 * 1024, &ordered_ids);
            let payload: Value = serde_json::from_str(handoff.lines().nth(1).unwrap()).unwrap();
            let latest = payload["items"].as_array().unwrap().iter().find(|item| item["evidence_id"] == latest_id).expect("latest event evidence survives handoff");
            let result = &latest["result"];
            let retained = result["match_index_table"]["rows"].as_array().or_else(|| result["match_index"].as_array()).unwrap().len();
            eprintln!("private replay projection: retained index rows={retained}, requested windows={}, item bytes={}, handoff item bytes={}", result["windows"].as_array().map_or(0, Vec::len), model_json_bytes(latest), model_json_bytes(&payload["items"]));
            assert!(retained >= 8);
            assert!(result["windows"].as_array().is_some_and(|windows| !windows.is_empty()));
            assert_eq!(latest["args"]["buff_names"], replay_evidence[&latest_id]["args"]["buff_names"]);
            assert!(model_json_bytes(&payload["items"]) <= 12 * 1024);
            let normal_handoff = model_evidence_handoff_prioritized(&replay_evidence, 28 * 1024, &ordered_ids);
            let normal: Value = serde_json::from_str(normal_handoff.lines().nth(1).unwrap()).unwrap();
            let latest = normal["items"].as_array().unwrap().iter().find(|item| item["evidence_id"] == latest_id).unwrap();
            let result = &latest["result"];
            let retained = result["match_index_table"]["rows"].as_array().or_else(|| result["match_index"].as_array()).unwrap().len();
            assert!(retained >= 8, "normal handoff retains a complete requested match page");
            eprintln!("private replay normal projection: latest index rows={retained}, item bytes={}", model_json_bytes(latest));
        }
    }

    #[test]
    fn mixed_handoff_retains_each_capability_core() {
        let mut evidence = EvidenceStore::new();
        let results = [
            ("get_current_scenario", json!({"game_version":"test", "rotation_input":{"mode":"manual_sequence", "total_items":100,"manual_operations":[{"skill_name":"test","is_main_gcd":true}],"skill_semantics":{"test":{"is_main_gcd":true,"cooldown_semantics":"main_gcd","cooldowns":vec![json!({"description":"large runtime metadata".repeat(200) });20]}}}})),
            ("simulate_scenario", json!({"dps":100.0,"total_damage":30000,"fight_time":300,"ranked_damage_sources":[{"name":"test","damage":30000}]})),
            ("analyze_timeline", json!({"diagnostic_profile":{"main_gcd_cast_count":100},"rotation_cycles":{"cycle_shapes":[{"shape":"test sequence", "count":12}],"cycles":vec![json!({"data":"detail".repeat(500)});20]},"rage":{"overflow_total":20}})),
            ("search_knowledge_base", json!({"results":[{"snippet":"原文机制说明".repeat(120),"source_url":"https://example.org/guide","fact_eligible":true}],"resolved_terms":vec![json!({"cards":vec![json!({"meaning":"duplicate definition".repeat(300)});4]});8]})),
            ("inspect_rotation_input", json!({"mode":"manual_sequence","total_matches":1,"matches":[{"matched":{"skill_name":"test","timing_mode":"follow_gcd_end","raw_timing_offset":-1,"is_main_gcd":false}}]})),
        ];
        for (index, (tool, result)) in results.into_iter().enumerate() {
            let id = format!("{index:064x}");
            evidence.insert(id.clone(), json!({"evidence_id":id,"tool_name":tool,"args":{},"result":result}));
        }
        let mut ordered_ids = Vec::new();
        if let Some(directory) = std::env::var_os("JX3_AGENT_REPLAY_CORE_EVENTS") {
            evidence.clear();
            let mut files = fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
            files.sort();
            for file in files {
                let event: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
                if event["sequence"].as_u64().unwrap_or(0) > 32 || event["kind"] != "tool_dispatch" { continue; }
                for envelope in event["payload"]["output"]["evidence"].as_array().into_iter().flatten() {
                    let id = envelope["evidence_id"].as_str().unwrap().to_string();
                    ordered_ids.insert(0, id.clone());
                    evidence.insert(id, envelope.clone());
                }
            }
        }
        let handoff = model_evidence_handoff_prioritized(&evidence, 12 * 1024, &ordered_ids);
        let payload: Value = serde_json::from_str(handoff.lines().nth(1).unwrap()).unwrap();
        let items = payload["items"].as_array().unwrap();
        assert!(model_json_bytes(&payload["items"]) <= 12 * 1024);
        for (tool, path) in [
            ("get_current_scenario", "/result/rotation_input"),
            ("simulate_scenario", "/result/dps"),
            ("analyze_timeline", "/result/rotation_cycles/cycle_shapes"),
            ("search_knowledge_base", "/result/results/0/snippet"),
            ("inspect_rotation_input", "/result/matches/0/matched/skill_name"),
        ] {
            let item = items.iter().find(|item| item["tool_name"] == tool).unwrap_or_else(|| panic!("missing {tool}"));
            assert!(item.pointer(path).is_some_and(|value| !value.is_null()), "{tool} lost {path}, bytes={}", model_json_bytes(item));
            eprintln!("core projection: {tool} bytes={}", model_json_bytes(item));
        }
    }

    #[test]
    fn repeated_tool_projection_keeps_large_event_facts_and_input_semantics() {
        let item = large_event_projection_fixture("f", 0);
        let output = json!({"ok": true, "tool_name": "inspect_timeline_events", "evidence_ids": ["f".repeat(64)], "evidence": [item]});
        let projected = model_tool_output(&output);
        assert!(model_json_bytes(&projected) <= MODEL_TOOL_OUTPUT_BYTES);
        assert_eq!(model_tool_output(&projected), projected);
        let facts = compact_result_facts(projected["evidence"][0].get("result"));
        assert_eq!(facts["match_index_table"], projected["evidence"][0]["result"]["match_index_table"]);
        assert_eq!(projected["evidence"][0]["args"]["buff_names"][0], "测试增益");
        assert!(projected["evidence"][0]["result"]["match_index_table"]["rows"].as_array().is_some_and(|rows| !rows.is_empty()));
        let manual = compact_result_facts(Some(&json!({
            "skill_semantics": [{"skill_name":"测试技能", "cooldown_semantics":"charge"}],
            "total_matches": 1, "returned_match_count": 1, "returned_window_count": 1,
            "page_start_index": 0, "has_more": false,
            "matches": [{"timing_mode":"delay", "delay_seconds":1.25, "raw_timing_offset":-1.25, "is_main_gcd":true, "cooldown_semantics":"charge"}]
        })));
        assert_eq!(manual["skill_semantics"][0]["cooldown_semantics"], "charge");
        assert_eq!(manual["matches"][0]["delay_seconds"], 1.25);
        assert_eq!(manual["returned_match_count"], 1);
    }

    fn large_event_projection_fixture(id: &str, start_match: usize) -> Value {
        let matches = (0..64).map(|index| json!({
            "event_number": index * 4 + 12, "cast_time": index as f64 * 4.25 + 7.9375,
            "skill_name": "测试技能", "rage_before": 80 + index % 3, "rage_after": 30 + index % 3,
            "rage_cost": 50, "rage_generated": 0, "rage_gained": 0, "rage_spent": 50,
            "buffs_before": [{"name":"测试增益", "remaining_seconds": 10.0 - (index % 4) as f64 * 0.5, "stacks":1}],
            "absolute_knife": {"blood_rage_active": index % 5 != 0, "yuan_ge_before":index % 6, "yuan_ge_after":index % 5},
            "rage_transactions": [{"source":"技能消耗", "rage_before":80 + index % 3, "rage_after":30 + index % 3, "requested_delta":-50, "applied_delta":-50}],
            "prior_mechanic_landmarks": [{"event_number":index * 4 + 11, "cast_time":index as f64 * 4.25 + 6.5, "skill_name":"前置技能"}]
        })).collect::<Vec<_>>();
        let windows = matches.iter().skip(start_match).take(8).map(|event| json!({
            "matched": event,
            "context": (0..5).map(|offset| json!({"event_number":offset+1, "cast_time":offset as f64, "skill_name":"相邻技能", "state_before":{"rage":80,"buffs":[{"name":"测试增益","remaining_seconds":7.0,"stacks":1}]}})).collect::<Vec<_>>()
        })).collect::<Vec<_>>();
        json!({
            "evidence_id": id.repeat(64), "tool_name":"inspect_timeline_events",
            "args":{"selector":"skill", "skill_name":"测试技能", "start_match":start_match, "limit":8, "buff_names":["测试增益"]},
            "result":{"selector":"skill", "total_matches":85, "match_index":matches, "match_index_truncated":true,
                "start_match":start_match, "next_start_match":start_match+8, "windows":windows}
        })
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
    fn evidence_handoff_balances_knowledge_simulation_diagnosis_and_events() {
        let mut evidence = EvidenceStore::new();
        for index in 0..10 {
            let id = format!("{index:064x}");
            evidence.insert(id.clone(), json!({
                "evidence_id": id,
                "tool_name": "search_knowledge_base",
                "result": {"results": [{
                    "document_id": format!("guide-{index}"),
                    "heading": format!("章节 {index}"),
                    "snippet": "攻略语境".repeat(800),
                    "fact_eligible": true
                }]}
            }));
        }
        for (id, tool_name, result) in [
            ("a".repeat(64), "get_current_scenario", json!({"game_version": "2026_04_anying_qianji", "mount": "fenshanjin"})),
            ("b".repeat(64), "simulate_scenario", json!({"dps": 123.5, "total_damage": 37050.0})),
            ("c".repeat(64), "analyze_timeline", json!({"active_event_count": 300, "rage": {"overflow_total": 45}})),
            ("d".repeat(64), "inspect_timeline_events", json!({
                "selector": "skill",
                "total_matches": 1,
                "match_index": [{
                    "event_number": 15,
                    "cast_time": 13.5,
                    "skill_name": "绝刀·50怒",
                    "absolute_knife": {"without_yuan_ge_blood_shadow": true}
                }]
            })),
        ] {
            evidence.insert(id.clone(), json!({"evidence_id": id, "tool_name": tool_name, "result": result}));
        }

        let handoff = model_evidence_handoff(&evidence, MODEL_EVIDENCE_HANDOFF_BYTES);
        for tool in ["get_current_scenario", "simulate_scenario", "analyze_timeline", "inspect_timeline_events"] {
            assert!(handoff.contains(tool), "missing capability {tool}: {handoff}");
        }
        assert!(handoff.contains("without_yuan_ge_blood_shadow"));
    }

    #[test]
    fn compact_handoff_prioritizes_source_bound_knowledge_passages() {
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
                    "source_url": "https://example.com/guide"
                }]}
            }),
        );

        let handoff = model_evidence_handoff(&evidence, 3 * 1024);
        assert!(handoff.contains("白刀是未触发援戈"));
        assert!(handoff.contains("https://example.com/guide"));
    }

    #[test]
    fn model_knowledge_projection_keeps_term_meaning_without_repeating_source_bodies() {
        let output = json!({
            "ok": true,
            "tool_name": "search_knowledge_base",
            "evidence": [{
                "evidence_id": "a".repeat(64),
                "tool_name": "search_knowledge_base",
                "result": {
                    "resolved_terms": [{
                        "matched_surface": "领域黑话",
                        "resolution": "current_scope_discovered_phrase",
                        "cards": [{
                            "term_id": "b".repeat(64),
                            "surface": "领域黑话",
                            "aliases": [],
                            "kind": "colloquial",
                            "meaning": "来自攻略原文的简明含义",
                            "meaning_basis": "source_definition",
                            "season": "当前赛季",
                            "confidence": "high",
                            "sources": [{
                                "title": "当前攻略",
                                "heading": "机制",
                                "source_url": "https://example.com/guide",
                                "excerpt": "不应进入模型上下文的超长原文".repeat(1_000)
                            }]
                        }]
                    }],
                    "results": [{
                        "title": "当前攻略",
                        "heading": "机制",
                        "snippet": "用于正式引用的检索片段",
                        "fact_eligible": true
                    }]
                }
            }]
        });

        let projected = model_tool_output(&output);
        let encoded = serde_json::to_string(&projected).unwrap();
        assert!(encoded.len() < MODEL_TOOL_OUTPUT_BYTES);
        assert!(encoded.contains("来自攻略原文的简明含义"));
        assert!(encoded.contains("用于正式引用的检索片段"));
        assert!(!encoded.contains("不应进入模型上下文的超长原文"));
    }

    #[test]
    fn knowledge_progress_distinguishes_chunks_and_empty_queries() {
        let result = |chunk: &str, snippet: &str| json!({
            "document_id": "same-document",
            "heading": "同一章节",
            "chunk_hash": chunk,
            "snippet": snippet,
        });
        let first = result("chunk-a", "第一个机制");
        let second = result("chunk-b", "第二个机制");
        assert_ne!(knowledge_source_key(&first), knowledge_source_key(&second));
        assert_eq!(knowledge_source_key(&first), knowledge_source_key(&result("chunk-a", "另一种摘录")));
        let empty = |query: &str| json!({"evidence": [{
            "tool_name": "search_knowledge_base",
            "args": {"query": query, "version_scope": "current_only"},
            "result": {"results": []}
        }]});
        assert_ne!(knowledge_source_keys(&empty("机制甲")), knowledge_source_keys(&empty("机制乙")));
        assert_eq!(knowledge_source_keys(&empty("机制甲")), knowledge_source_keys(&empty("机制甲")));
    }

    #[test]
    fn compact_handoff_keeps_exact_event_index_ahead_of_aggregate_payloads() {
        let mut evidence = EvidenceStore::new();
        evidence.insert(
            "1".repeat(64),
            json!({
                "evidence_id": "1".repeat(64),
                "tool_name": "analyze_timeline",
                "result": {"rotation_cycles": {"cycles": (0..20).map(|_| json!({
                    "key_sequence": (0..30).map(|index| json!({
                        "event_number": index + 1,
                        "skill_name": "盾击·三段",
                        "detail": "聚合时间轴".repeat(100)
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()}}
            }),
        );
        evidence.insert(
            "f".repeat(64),
            json!({
                "evidence_id": "f".repeat(64),
                "tool_name": "inspect_timeline_events",
                "result": {
                    "selector": "skill",
                    "skill_name": "绝刀",
                    "total_matches": 17,
                    "match_index": (0..17).map(|index| json!({
                        "event_number": index + 1,
                        "cast_time": index as f64 + 0.5,
                        "skill_name": "绝刀·50怒",
                        "absolute_knife": {"without_yuan_ge_blood_shadow": true}
                    })).collect::<Vec<_>>(),
                    "windows": []
                }
            }),
        );

        let handoff = model_evidence_handoff(&evidence, 4 * 1024);
        assert!(handoff.contains("inspect_timeline_events"));
        let payload: Value = serde_json::from_str(handoff.lines().nth(1).unwrap()).unwrap();
        let event = payload["items"].as_array().unwrap().iter().find(|item| item["tool_name"] == "inspect_timeline_events").unwrap();
        if let Some(table) = event.pointer("/result/match_index_table") {
            let columns = table["columns"].as_array().unwrap();
            let column = columns.iter().position(|field| field == "/event_number").unwrap();
            assert!(table["rows"].as_array().unwrap().iter().any(|row| row[column] == 17));
            assert!(table["source_indices"].as_array().unwrap().contains(&json!(16)));
        } else {
            assert!(handoff.contains("\"event_number\":17"));
        }
    }

    #[test]
    fn compact_handoff_keeps_neutral_skill_effect_events_and_bounded_contexts() {
        let matches = (0..17)
            .map(|index| {
                json!({
                    "event_number": index + 15,
                    "cast_time": index as f64 * 10.0 + 13.5,
                    "skill_name": "绝刀·50怒",
                    "macro_page": 1,
                    "macro_line": if index % 2 == 0 { 5 } else { 7 },
                    "rage_before": 80,
                    "rage_after": 30,
                    "rage_cost": 50,
                    "absolute_knife": {
                        "without_yuan_ge_blood_shadow": true,
                        "blood_rage_active": index < 11,
                        "yuan_ge_before": 0,
                        "yuan_ge_after": 0
                    }
                })
            })
            .collect::<Vec<_>>();
        let windows = (0..8)
            .map(|index| {
                json!({
                    "matched": {
                        "event_number": index + 15,
                        "cast_time": index as f64 * 10.0 + 13.5,
                        "skill_name": "绝刀·50怒",
                        "macro_line": 7,
                        "state_before": {"rage": 80, "stance": "blade"},
                        "state_after": {"rage": 30, "stance": "blade"},
                        "damage_total": 12345678.0,
                        "buffs": "oversized detail".repeat(100)
                    },
                    "context": (0..7).map(|offset| json!({
                        "event_number": index * 7 + offset + 1,
                        "cast_time": index as f64 * 10.0 + offset as f64,
                        "skill_name": "相邻技能",
                        "macro_line": 6,
                        "state_before": {"rage": 65, "stance": "blade"},
                        "state_after": {"rage": 50, "stance": "blade"},
                        "damage_total": 9999999.0,
                        "oversized": "detail".repeat(100)
                    })).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let evidence = EvidenceStore::from([(
            "f".repeat(64),
            json!({
                "evidence_id": "f".repeat(64),
                "tool_name": "inspect_timeline_events",
                "result": {
                    "selector": "skill",
                    "skill_name": "绝刀",
                    "total_matches": 17,
                    "window_selection": "requested_page",
                    "match_index": matches,
                    "windows": windows,
                    "next_start_match": null
                }
            }),
        )]);

        let handoff = model_evidence_handoff(&evidence, 16 * 1024);
        assert!(handoff.contains("inspect_timeline_events"));
        assert!(handoff.contains("\"event_number\":15"));
        assert!(handoff.contains("\"without_yuan_ge_blood_shadow\":true"));
        assert!(!handoff.contains("oversized detail"));
        assert!(!handoff.contains("9999999"));
    }

    #[test]
    fn repeated_compaction_preserves_every_local_cast_and_stance() {
        let windows = (0..5).map(|index| {
            let events = ["斩刀", "绝刀·50怒", "绝刀·50怒", "血怒", "绝刀·30怒", "盾回"]
                .iter().enumerate().map(|(offset, name)| json!({
                    "event_number": index * 10 + offset + 1,
                    "operation_number": index * 10 + offset + 7,
                    "cast_time": index * 10 + offset,
                    "skill_name": name,
                    "state_before": {"stance":"blade", "rage":35},
                    "state_after": {"stance":"blade", "rage":5}
                })).collect::<Vec<_>>();
            json!({"matched": events[4], "context": events})
        }).collect::<Vec<_>>();
        let matches = windows.iter().map(|window| window["matched"].clone()).collect::<Vec<_>>();
        let mut envelope = json!({"tool_name":"inspect_timeline_events", "result":{"windows":windows, "match_index": matches}});
        project_tool_result("inspect_timeline_events", &mut envelope["result"]);
        let once = compact_handoff_evidence(&envelope);
        let twice = compact_handoff_evidence(&once);
        for matched in twice["result"]["match_index"].as_array().unwrap() {
            let sequence = matched["local_sequence"].as_str().unwrap();
            assert_eq!(sequence.matches("绝刀·50怒").count(), 2);
            assert!(sequence.contains("绝刀·30怒"));
        }
        for window in twice["result"]["windows"].as_array().unwrap() {
            let context = window["context"].as_array().unwrap();
            assert_eq!(context.len(), 6);
            assert_eq!(context[0]["skill_name"], "斩刀");
            assert_eq!(context[1]["skill_name"], "绝刀·50怒");
            assert_eq!(context[2]["skill_name"], "绝刀·50怒");
            assert_eq!(context[4]["stance_before"], "blade");
        }
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
    fn event_fallback_lists_every_indexed_event_with_navigation_anchors() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let plan = select_analysis_plan("具体到时间点列出白刀", &scenario);
        let evidence_id = "f".repeat(64);
        let evidence = EvidenceStore::from([(
            evidence_id.clone(),
            json!({
                "evidence_id": evidence_id,
                "tool_name": "inspect_timeline_events",
                "result": {
                    "match_index": [
                        {"event_number": 15, "cast_time": 13.5975, "skill_name": "绝刀·50怒"},
                        {"event_number": 74, "cast_time": 74.48, "skill_name": "绝刀·50怒"}
                    ]
                }
            }),
        )]);

        let report = evidence_preserving_provider_fallback(&plan, &evidence, "模型响应为空")
            .expect("event evidence fallback");
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "精确事件位置")
            .expect("exact event finding");
        assert!(finding
            .explanation
            .contains("[[13.60s 绝刀·50怒|ev:15]]"));
        assert!(finding
            .explanation
            .contains("[[74.48s 绝刀·50怒|ev:74]]"));
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

    #[test]
    fn handoff_depends_on_serialized_context_size_instead_of_turn_count() {
        let mut request = ModelRequest {
            instructions: agent_prompt().instructions.to_string(),
            messages: (0..8).map(|index| ModelMessage::User {
                content: format!("第{index}轮已查到的上下文：{}", "x".repeat(3_600)),
            }).collect(),
            tools: Vec::new(),
            response_format: None,
            max_output_tokens: 2_048,
        };
        assert!(request_bytes(&request) > 32 * 1024);
        assert!(!should_handoff_request(&request, false));
        request.messages.extend((0..3).map(|_| ModelMessage::User { content: "x".repeat(40 * 1024) }));
        assert!(should_handoff_request(&request, false));
        assert!(!should_handoff_request(&request, true));
    }

    #[test]
    fn report_repair_evidence_is_bounded_even_with_full_cycle_diagnostics() {
        let mut evidence = EvidenceStore::new();
        for index in 0..12 {
            let id = format!("{index:064x}");
            evidence.insert(
                id.clone(),
                json!({
                    "evidence_id": id,
                    "tool_name": "analyze_timeline",
                    "result": {
                        "rage": {"overflow_total": 45, "overflow_sources": [{
                            "skill_name": "盾击·三段", "event_count": 5,
                            "overflow_total": 45, "event_numbers": [8, 64, 122, 180, 238]
                        }]},
                        "rotation_cycles": {
                            "cycle_count": 21,
                            "cycles": (0..100).map(|cycle| json!({
                                "cycle_number": cycle,
                                "key_sequence": (0..100).map(|event| json!({
                                    "event_number": event, "skill_name": "绝刀·50怒"
                                })).collect::<Vec<_>>()
                            })).collect::<Vec<_>>()
                        },
                        "macro_line_stats": (0..200).map(|line| json!({
                            "line": line, "skill": "盾击", "evaluations": 1200
                        })).collect::<Vec<_>>()
                    }
                }),
            );
        }

        let context = repair_evidence_context(&evidence);
        assert!(context.len() <= REPORT_REPAIR_EVIDENCE_BYTES + 256);
        assert!(context.contains("overflow_total") || context.contains("analyze_timeline"));
    }

    #[test]
    fn repair_keeps_latest_followup_and_its_conversational_referent() {
        let runtime = AgentRuntime::fixture();
        for question in ["那哪些绝刀打的最亏", "那两件装备具体差在哪", "这个判断依据哪段攻略"] {
            let mut run_input = input(&runtime, "repair-focus");
            run_input.question = question.to_string();
            run_input.session_context = Some("上一轮问题与回答：当前讨论的对象仍是同一个冻结方案。".to_string());
            let mut state = AgentDiagnosticStateV1::new(question);
            state.current_judgment = Some("需要检查刚才提到的具体对象。".to_string());
            let messages = report_repair_messages(&run_input, &state, "修复字段类型。".to_string());
            assert!(messages.iter().any(|message| matches!(message,
                ModelMessage::User { content } if content == question
            )));
            assert!(messages.iter().any(|message| matches!(message,
                ModelMessage::User { content } if content.contains("同一个冻结方案")
            )));
            assert!(messages.iter().any(|message| matches!(message,
                ModelMessage::User { content } if content.contains("需要检查刚才提到的具体对象")
            )));
            assert!(matches!(messages.last(), Some(ModelMessage::User { content }) if content == "修复字段类型。"));
        }
    }

    #[test]
    fn history_compaction_preserves_recent_turn_as_valid_json() {
        let history = json!({"schema_version":"agent-session-context/v1", "turns":[
            {"question":"最早问的是整场基线", "summary":"很长的旧分析".repeat(1500)},
            {"question":"最近问的是关键技能的释放时机", "summary":"接下来要检查具体操作。", "findings":[{"title":"释放时机", "explanation":"状态说明".repeat(900)}]},
        ]}).to_string();
        let compact = compact_session_context(&history, 2_000);
        assert!(compact.chars().count() <= 2_000);
        let parsed: Value = serde_json::from_str(&compact).unwrap();
        let latest = parsed["turns"].as_array().unwrap().last().unwrap();
        assert_eq!(latest["question"], "最近问的是关键技能的释放时机");
        let wrapped = bounded_session_message(&format!("<session_context untrusted_data=\"true\">{history}</session_context>"), 2_000);
        assert!(wrapped.contains("最近问的是关键技能的释放时机"));
        assert!(wrapped.ends_with("</session_context>"));
    }

    #[test]
    fn history_compaction_keeps_pending_goal_and_whole_code() {
        let code = "/cast [buff:狂绝&nobuff:血怒·惊涌] 血怒";
        let history = json!({"turns":[{"summary":"旧分析".repeat(2000)}, {
            "question":"应该放在哪个宏页", "analysis_text":"说明".repeat(2000),
            "continuation_goal":"完全复刻当前技能轴中血怒的释放位置",
            "proposed_code_blocks":[code], "proposal_status":"historical_unverified_candidate"
        }]}).to_string();
        let compact = compact_session_context(&history, 2_000);
        assert!(compact.chars().count() <= 2_000);
        let value: Value = serde_json::from_str(&compact).unwrap();
        let pending = value["turns"].as_array().unwrap().last().unwrap();
        assert_eq!(pending["proposed_code_blocks"][0], code);
        assert_eq!(pending["continuation_goal"], "完全复刻当前技能轴中血怒的释放位置");
    }

    #[test]
    fn handoff_retains_tool_parameter_feedback_for_the_next_action() {
        let runtime = AgentRuntime::fixture();
        let run_input = input(&runtime, "feedback-handoff");
        let transcript = vec![
            ModelMessage::Assistant { content: None, reasoning_content: None, tool_calls: vec![ProviderToolCall {
                call_id: "invalid-page".to_string(), name: "inspect_timeline_events".to_string(), arguments: json!({"limit":85}),
            }] },
            ModelMessage::ToolResult { call_id: "invalid-page".to_string(), output: json!({
                "ok":false, "tool_name":"inspect_timeline_events", "error":{"code":"invalid_tool_arguments", "message":"limit exceeds page size"},
            }) },
            ModelMessage::User { content: "请修正分页参数后继续当前问题。".to_string() },
        ];
        let messages = compact_handoff_messages(&run_input, &transcript, &EvidenceStore::new(), 4 * 1024);
        let encoded = serde_json::to_string(&messages).unwrap();
        assert!(encoded.contains("invalid_tool_arguments"));
        assert!(encoded.contains("85"));
        assert!(encoded.contains("请修正分页参数后继续当前问题"));
    }

    #[test]
    fn budget_fallback_keeps_followup_subject_without_injecting_baseline() {
        let question = "那哪些操作最需要调整";
        let mut state = AgentDiagnosticStateV1::new(question);
        state.current_judgment = Some("正在核对这些操作发生时的资源和增益。".to_string());
        let evidence = EvidenceStore::from([( "a".repeat(64), json!({
            "tool_name":"simulate_scenario", "result":{"dps":100.0, "total_damage":1000.0},
        }))]);
        let report = task_preserving_provider_fallback(question, &state, &evidence, "本轮预算已用完。").unwrap();
        assert!(report.summary.contains(question));
        assert!(report.findings.iter().all(|finding| finding.metrics.is_empty()));
        assert!(!serde_json::to_string(&report).unwrap().contains("当前输出基线"));
        assert!(report.findings[0].explanation.contains("这些操作"));
    }

    #[tokio::test]
    async fn report_repair_request_retains_followup_in_actual_provider_loop() {
        let runtime = AgentRuntime::fixture();
        let question = "那哪些操作需要先检查";
        let provider = ScriptedProvider::new(vec![
            Ok(ModelResponse {
                assistant_text: Some("{unclosed report".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
            Ok(ModelResponse {
                assistant_text: Some(serde_json::to_string(&refusal_content(
                    "当前缺少逐次操作证据，暂时无法列出需要检查的位置。",
                    "需要逐次操作证据。",
                )).unwrap()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let mut run_input = input(&runtime, "repair-followup");
        run_input.question = question.to_string();
        run_input.session_context = Some("用户此前正在讨论一组技能的释放时机。".to_string());
        let _result = run_agent(&provider, &runtime, run_input, AgentRunLimits::default(), AgentCancellation::default()).await;
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        let repair = &requests[1];
        assert!(repair.tools.is_empty());
        assert!(repair.messages.iter().any(|message| matches!(message,
            ModelMessage::User { content } if content == question
        )));
        assert!(repair.messages.iter().any(|message| matches!(message,
            ModelMessage::User { content } if content.contains("一组技能的释放时机")
        )));
        assert!(request_bytes(repair) <= MAX_MODEL_REQUEST_BYTES);
    }

    #[tokio::test]
    async fn run_timeout_preserves_followup_progress_and_tool_evidence() {
        struct SleepProvider {
            calls: std::sync::atomic::AtomicUsize,
        }
        #[async_trait]
        impl LlmProvider for SleepProvider {
            fn profile_id(&self) -> &str { "sleep-fixture" }
            fn model(&self) -> &str { "sleep-fixture" }
            async fn complete(&self, _request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
                if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    let mut response = tool_call("read-before-timeout", "analyze_timeline", json!({}));
                    response.assistant_text = Some("正在核对这些操作的资源和增益状态。".to_string());
                    return Ok(response);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                unreachable!("the local run deadline should cancel this provider call")
            }
        }
        let runtime = AgentRuntime::fixture();
        let provider = SleepProvider { calls: std::sync::atomic::AtomicUsize::new(0) };
        let question = "那哪些操作更值得调整？";
        let mut run_input = input(&runtime, "provider-timeout-keeps-progress");
        run_input.question = question.to_string();
        let limits = AgentRunLimits { wall_time_ms: 200, ..AgentRunLimits::default() };
        let result = run_agent(&provider, &runtime, run_input, limits, AgentCancellation::default()).await;

        assert_eq!(result.status, AgentRunStatus::TimedOut);
        assert_eq!(result.error.as_ref().unwrap().code, "run_timeout");
        let report = result.report.as_ref().expect("timed-out progress remains readable");
        assert!(report.content.summary.contains(question));
        assert!(report.content.summary.contains("还未完成"));
        assert!(report.content.findings.iter().any(|finding| finding.explanation.contains("这些操作")));
        assert!(report.content.findings.iter().all(|finding| finding.metrics.is_empty() && finding.evidence_ids.is_empty()));
        assert!(!serde_json::to_string(&report.content).unwrap().contains("DPS"));
        let debug = result.debug.as_ref().unwrap();
        assert!(debug.tool_calls.iter().any(|call| call.tool_name == "analyze_timeline" && call.ok));
        assert!(debug.evidence_projection.as_ref().unwrap()["items"].as_array().unwrap()
            .iter().any(|item| item["tool_name"] == "analyze_timeline"));
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
            resume_tools: Vec::new(),
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
        assert_eq!(limits.max_simulations, 512);
        assert_eq!(limits.max_output_tokens_per_turn, 16384);
        assert_eq!(limits.wall_time_ms, 180_000);
    }

    #[tokio::test]
    async fn clarification_preserves_public_analysis_but_not_private_reasoning() {
        let runtime = AgentRuntime::fixture();
        let mut response = tool_call("clarify-with-analysis", ASK_USER_QUESTION, json!({
            "question": "优先极限输出还是一键容错？",
            "reason": "候选修改取决于操作目标。"
        }));
        response.assistant_text = Some("已定位两处可比较的位置。".into());
        response.reasoning_content = Some("private-test-reasoning".into());
        let provider = ScriptedProvider::new(vec![Ok(response)]);
        let result = run_agent(&provider, &runtime, input(&runtime, "run-public-analysis"),
            AgentRunLimits::default(), AgentCancellation::default()).await;
        let clarification = result.clarification.unwrap();
        assert_eq!(clarification.analysis_text.as_deref(), Some("已定位两处可比较的位置。"));
        assert!(!serde_json::to_string(&clarification).unwrap().contains("private-test-reasoning"));
    }

    #[tokio::test]
    async fn resume_rehydrates_same_scenario_queries_without_model_reinvestigation() {
        let runtime = AgentRuntime::fixture();
        let first_input = input(&runtime, "run-resume-first");
        let scenario_hash = first_input.scenario.scenario_hash.clone();
        let first = run_agent(&FakeProvider::new("offline".into(), "fixture-v1".into()),
            &runtime, first_input, AgentRunLimits::default(), AgentCancellation::default()).await;
        let events = vec![super::super::session::AgentSessionEventV1::run_result(&first)];
        let queries = super::super::session::resume_tool_queries(&events, &scenario_hash);
        assert!(!queries.is_empty());
        assert!(super::super::session::resume_tool_queries(&events, "different-scenario").is_empty());
        let mut second = input(&runtime, "run-resume-second");
        second.question = "继续".into();
        second.resume_tools = queries;
        let provider = ScriptedProvider::new(vec![Ok(tool_call("repeat", "analyze_timeline", json!({}))),
            Ok(ModelResponse { assistant_text: Some(serde_json::to_string(&refusal_content("已恢复。", "测试结束。")).unwrap()),
                reasoning_content: None, tool_calls: vec![], finish_reason: FinishReason::Stop, usage: TokenUsage::default() })]);
        let result = run_agent(&provider, &runtime, second, AgentRunLimits::default(), AgentCancellation::default()).await;
        assert!(result.trace.iter().any(|event| event.kind == "analysis_resumed"));
        assert!(result.debug.as_ref().unwrap().tool_calls.iter().any(|call| call.call_id == "repeat" && call.reused));
        let text = serde_json::to_string(&provider.requests()[0].messages).unwrap();
        assert!(text.contains("resume-"));
        assert!(text.contains("analyze_timeline"));
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
    fn clarification_options_are_typed_and_history_remains_compatible() {
        assert!(parse_clarification(&json!({
            "question": "你更关心哪一种：极限输出还是一键容错？",
            "reason": "目标不同会改变候选循环。",
            "options": [
                {"label": "极限输出", "description": "接受手动操作"},
                {"label": "一键容错", "description": "降低操作要求"}
            ]
        })).is_ok());
        let parsed = parse_clarification(&json!({
            "question": "要比较的保存方案是哪一个？",
            "reason": "有两个同名候选，需要确认具体对象。",
            "options": [
                {"label": " 方案A ", "description": "宏循环"},
                {"label": "方案B", "description": "手动循环"}
            ]
        })).unwrap();
        assert_eq!(parsed.options[0].label, "方案A");
        let old: AgentClarificationV1 = serde_json::from_value(json!({
            "schema_version": "agent-clarification/v1", "question": "哪个方案？",
            "reason": "需要确定对象。", "answer_hint": "A / B"
        })).unwrap();
        assert!(old.options.is_empty());
        assert!(parse_clarification(&json!({
            "question": "哪个方案？", "reason": "需要确定对象。",
            "options": [{"label": "A"}, {"label": "A"}]
        })).is_err());
    }






    #[test]
    fn report_named_tool_is_reflected_once_even_if_other_queries_used_the_tool() {
        let report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "当前只有宏结构。",
            "findings": [],
            "recommendations": [{
                "title": "继续体检",
                "rationale": "下一步调用 analyze_timeline 再回答。",
                "evidence_ids": []
            }],
            "rotation_changes": [],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();
        let reflected = HashSet::new();
        let evidence = EvidenceStore::new();
        assert_eq!(
            unexecuted_plan_marker(Some(&report), &reflected, &evidence).as_deref(),
            Some("analyze_timeline")
        );
        let reflected = HashSet::from(["analyze_timeline".to_string()]);
        assert_eq!(
            unexecuted_plan_marker(Some(&report), &reflected, &evidence),
            None
        );
    }

    #[test]
    fn report_limitation_that_names_a_tool_does_not_reopen_investigation() {
        let report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "已形成机制判断，具体位置尚未读取。",
            "findings": [],
            "recommendations": [],
            "rotation_changes": [],
            "limitations": ["需要 inspect_timeline_events 定位实际事件后再回答位置。"],
            "refusal_reason": null
        })
        .to_string();

        assert_eq!(
            unexecuted_plan_marker(Some(&report), &HashSet::new(), &EvidenceStore::new()).as_deref(),
            None
        );
    }

    #[test]
    fn report_limitations_alone_do_not_force_an_unselected_tool() {
        let report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "已形成机制判断。",
            "findings": [],
            "recommendations": [],
            "rotation_changes": [],
            "limitations": ["当前缺少足以完成用户问题的一层实测细节。"],
            "refusal_reason": null
        })
        .to_string();
        let evidence = EvidenceStore::from([
            ("a".repeat(64), json!({
                "tool_name": "search_knowledge_base",
                "result": {"results": [{"fact_eligible": true}]}
            })),
            ("b".repeat(64), json!({
                "tool_name": "analyze_timeline",
                "result": {"active_event_count": 300}
            })),
        ]);

        assert_eq!(
            unexecuted_plan_marker(Some(&report), &HashSet::new(), &evidence).as_deref(),
            None
        );
    }

    #[test]
    fn report_that_proposes_an_unrun_comparison_is_reflected_once() {
        let report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "已找到主要假设。",
            "findings": [],
            "recommendations": [{
                "title": "验证假设",
                "rationale": "用同场景 A/B 对照验证这个局部宏改动。",
                "evidence_ids": []
            }],
            "rotation_changes": [{
                "change_type": "macro_statement",
                "edit_operation": "replace",
                "target": "第七行绝刀条件",
                "current": "/cast [bufftime:嗜血>7] 绝刀",
                "proposed": "/cast [bufftime:嗜血>7&buff:援戈] 绝刀",
                "rationale": "验证援戈门控",
                "evidence_ids": []
            }],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();
        let evidence = EvidenceStore::new();
        let reflected = HashSet::new();
        assert_eq!(
            unexecuted_plan_marker(Some(&report), &reflected, &evidence).as_deref(),
            Some("proposed_comparison")
        );
        let reflected = HashSet::from(["proposed_comparison".to_string()]);
        assert_eq!(
            unexecuted_plan_marker(Some(&report), &reflected, &evidence),
            None
        );
    }

    #[test]
    fn generic_future_comparison_advice_does_not_reopen_the_tool_loop() {
        let report = json!({
            "schema_version": "agent-report-content/v1",
            "summary": "目前只能给出假设。",
            "findings": [],
            "recommendations": [{
                "title": "以后验证",
                "rationale": "建议做同场景 A/B，但当前还没有可执行候选。",
                "evidence_ids": []
            }],
            "rotation_changes": [],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();
        assert_eq!(
            unexecuted_plan_marker(Some(&report), &HashSet::new(), &EvidenceStore::new()),
            None
        );
    }

    #[test]
    fn deterministic_event_cache_keys_preserve_observation_parameters() {
        let arguments = json!({
                "selector": "skill",
                "skill_name": "绝刀",
                "time_seconds": 74.48,
                "start_match": 6,
                "limit": 5,
                "context_radius": 2,
                "buff_names": ["血怒·惊涌"]
            });
        let key = tool_result_cache_key("inspect_timeline_events", &arguments);
        assert!(key.is_some());
        assert_eq!(key, tool_result_cache_key("inspect_timeline_events", &arguments.clone()));
        for (field, value) in [("limit", json!(1)), ("context_radius", json!(4)), ("buff_names", json!(["援戈"]))] {
            let mut changed = arguments.clone();
            changed[field] = value;
            assert_ne!(key, tool_result_cache_key("inspect_timeline_events", &changed), "{field} changes the observed evidence");
        }
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
        assert_eq!(result.prompt_version, super::super::prompt::AGENT_PROMPT_VERSION);
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
            vec![
                "get_current_scenario",
                "ask_user_question",
                "search_knowledge_base"
            ]
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
        assert_eq!(result.prompt_version, super::super::prompt::AGENT_PROMPT_VERSION);
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
        let transport_messages = requests[0].messages.iter().filter(|message|
            !matches!(message, ModelMessage::User { content } if content.starts_with("<mechanics_context"))
        ).collect::<Vec<_>>();
        assert_eq!(transport_messages.len(), 4);
        assert!(matches!(
            transport_messages[0],
            ModelMessage::User { content }
                if content.contains("untrusted_data=\"true\"")
                    && content.contains("上一轮可见结论")
        ));
        assert!(matches!(
            transport_messages[1],
            ModelMessage::User { content } if content == "概括上一轮结论。"
        ));
        assert!(matches!(
            transport_messages[2],
            ModelMessage::Assistant { tool_calls, .. }
                if tool_calls.len() == 1 && tool_calls[0].name == "get_current_scenario"
        ));
        assert!(matches!(
            transport_messages[3],
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
    async fn last_model_turn_is_reserved_for_answer_after_tool_evidence() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-diagnose", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(serde_json::to_string(&refusal_content("已读取事件，具体判断仍需核对。", "本测试检查回答回合。")).unwrap()),
                reasoning_content: None, tool_calls: Vec::new(), finish_reason: FinishReason::Stop, usage: TokenUsage::default(),
            }),
        ]);
        let limits = AgentRunLimits {
            max_model_turns: 2,
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

        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.model_turns, 2);
        assert_eq!(result.accounting.simulations, 1);
        let requests = provider.requests();
        assert!(!requests[0].tools.is_empty());
        assert!(requests[1].tools.is_empty());
        assert!(requests[1].messages.iter().any(|message| matches!(message,
            ModelMessage::User { content } if content.contains("分析当前循环的确定性输出")
        )));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "final_response_reserved"));
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
    async fn repeated_deterministic_diagnosis_reuses_evidence_without_closing_tools() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-timeline-1", "analyze_timeline", json!({}))),
            Ok(tool_call("call-timeline-2", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(
                    serde_json::to_string(&refusal_content(
                        "确定性诊断已完成。",
                        "本测试由模型选择停止，重复读取复用模拟结果。",
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
        // The server-prefetched scenario and first timeline execution count;
        // the identical second timeline request is a cache read, not a tool execution.
        assert_eq!(result.accounting.tool_calls, 2);
        let debug = result.debug.as_ref().expect("shareable debug record");
        assert_eq!(debug.diagnostic_state.no_new_evidence_rounds, 1);
        assert_eq!(debug.diagnostic_state.status, "ready_to_report".to_string());
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "evidence_reused"));
        assert!(provider.requests()[2].tools.iter().any(|tool| tool.name == "inspect_timeline_events"));
        let timeline_results = result
            .trace
            .iter()
            .filter(|event| {
                event.kind == "tool_finished"
                    && event.tool_name.as_deref() == Some("analyze_timeline")
            })
            .collect::<Vec<_>>();
        assert_eq!(timeline_results.len(), 2);
        assert_eq!(
            debug
                .tool_calls
                .iter()
                .filter(|call| call.tool_name == "analyze_timeline" && call.reused)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn model_event_queries_keep_requested_buffs_windows_and_distinct_cache_entries() {
        let runtime = AgentRuntime::fixture();
        let mut run_input = input(&runtime, "run-observation-parameters");
        let mut simulation = run_input.scenario.simulation.clone();
        simulation.sequence = ["血怒", "盾击", "盾击", "盾击", "盾压"]
            .into_iter().map(str::to_string).collect();
        run_input.scenario = ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), simulation).unwrap();
        let narrow = json!({"selector": "skill", "skill_name": "盾击", "limit": 1, "context_radius": 0, "buff_names": ["血怒"]});
        let wide = json!({"selector": "skill", "skill_name": "盾击", "limit": 2, "context_radius": 4, "buff_names": []});
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("events-narrow", "inspect_timeline_events", narrow.clone())),
            Ok(tool_call("events-wide", "inspect_timeline_events", wide.clone())),
            Ok(tool_call("events-cached", "inspect_timeline_events", narrow.clone())),
            Ok(ModelResponse {
                assistant_text: Some(serde_json::to_string(&refusal_content("已完成观察参数验证。", "测试不发布玩法建议。")).unwrap()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(&provider, &runtime, run_input, AgentRunLimits::default(), AgentCancellation::default()).await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.simulations, 1);
        assert_eq!(result.accounting.tool_calls, 3);
        let debug = result.debug.as_ref().unwrap();
        let calls = debug.tool_calls.iter().filter(|call| call.tool_name == "inspect_timeline_events").collect::<Vec<_>>();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].arguments, narrow);
        assert_eq!(calls[1].arguments, wide);
        assert!(!calls[0].reused && !calls[1].reused && calls[2].reused);
        assert_eq!(calls[0].evidence_ids, calls[2].evidence_ids);
        assert_ne!(calls[0].evidence_ids, calls[1].evidence_ids);
        let observations = debug.evidence_projection.as_ref().unwrap()["items"].as_array().unwrap()
            .iter().filter(|item| item["tool_name"] == "inspect_timeline_events").collect::<Vec<_>>();
        assert_eq!(observations.len(), 2);
        let narrow_result = observations.iter().find(|item| item["args"]["limit"] == 1).unwrap();
        let narrow_windows = narrow_result["result"]["windows"].as_array().unwrap();
        assert_eq!(narrow_windows.len(), 1);
        assert_eq!(narrow_windows[0]["context"].as_array().unwrap().len(), 1);
        let buffs = narrow_windows[0]["matched"]["state_before"]["buffs"].as_array().unwrap();
        assert!(buffs.iter().any(|buff| buff["name"].as_str().is_some_and(|name| name.starts_with("血怒"))));
        let wide_result = observations.iter().find(|item| item["args"]["limit"] == 2).unwrap();
        let wide_windows = wide_result["result"]["windows"].as_array().unwrap();
        assert_eq!(wide_windows.len(), 2);
        assert!(wide_windows[0]["context"].as_array().unwrap().len() > 1);
        assert!(wide_windows[0]["matched"]["state_before"]["buffs"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn repeated_observations_allow_new_actions_and_reset_consecutive_no_progress() {
        let runtime = AgentRuntime::fixture();
        let event_query = json!({"selector": "skill", "skill_name": "盾击", "limit": 1});
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("diagnose-1", "analyze_timeline", json!({}))),
            Ok(tool_call("diagnose-2", "analyze_timeline", json!({}))),
            Ok(tool_call("diagnose-3", "analyze_timeline", json!({}))),
            Ok(tool_call("events-new", "inspect_timeline_events", event_query.clone())),
            Ok(tool_call("events-repeat", "inspect_timeline_events", event_query)),
            Ok(ModelResponse {
                assistant_text: Some(serde_json::to_string(&refusal_content("已验证后续定位工具。", "测试不发布玩法建议。")).unwrap()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(&provider, &runtime, input(&runtime, "run-reuse-then-new-action"), AgentRunLimits::default(), AgentCancellation::default()).await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.model_turns, 6);
        assert_eq!(result.accounting.tool_calls, 3);
        assert_eq!(result.accounting.simulations, 1);
        assert_eq!(result.debug.as_ref().unwrap().diagnostic_state.no_new_evidence_rounds, 1);
        let requests = provider.requests();
        assert!(requests.iter().all(|request| request.tools.iter().any(|tool| tool.name == "inspect_timeline_events")));
        let diagnostic_state = requests[4].messages.iter().find_map(|message| match message {
            ModelMessage::User { content } => content.strip_prefix("<diagnostic_state server_generated=\"true\">\n")
                .and_then(|text| text.strip_suffix("\n</diagnostic_state>"))
                .and_then(|text| serde_json::from_str::<Value>(text).ok()),
            _ => None,
        }).unwrap();
        assert_eq!(diagnostic_state["no_new_evidence_rounds"], 0);
        assert_eq!(diagnostic_state["status"], "investigating");
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
    async fn knowledge_cache_and_total_budget_leave_other_capabilities_available() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture().with_knowledge_fixture(knowledge);
        let query = |text: &str| json!({"query": text, "version_scope": "current_only", "season": null, "category": null});
        let parallel_searches = ["盾飞 流血", "劫刀 流血", "盾飞 劫刀 流血"]
            .into_iter().enumerate().map(|(index, text)| ProviderToolCall {
                call_id: format!("knowledge-batch-{index}"),
                name: "search_knowledge_base".to_string(),
                arguments: query(text),
            }).collect();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("knowledge-first", "search_knowledge_base", query("盾飞"))),
            Ok(tool_call("knowledge-second", "search_knowledge_base", query("劫刀"))),
            Ok(tool_call("knowledge-third", "search_knowledge_base", query("流血"))),
            Ok(ModelResponse { assistant_text: None, reasoning_content: None, tool_calls: parallel_searches, finish_reason: FinishReason::ToolCalls, usage: TokenUsage::default() }),
            Ok(ModelResponse {
                assistant_text: None, reasoning_content: None,
                tool_calls: vec![
                    ProviderToolCall { call_id: "knowledge-repeat".to_string(), name: "search_knowledge_base".to_string(), arguments: query("盾飞") },
                    ProviderToolCall { call_id: "knowledge-over-budget".to_string(), name: "search_knowledge_base".to_string(), arguments: query("新的机制问题") },
                ],
                finish_reason: FinishReason::ToolCalls, usage: TokenUsage::default(),
            }),
            Ok(tool_call("diagnose-after-searches", "analyze_timeline", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(serde_json::to_string(&refusal_content("已完成本地工具验证。", "测试不发布玩法建议。")).unwrap()),
                reasoning_content: None, tool_calls: Vec::new(), finish_reason: FinishReason::Stop, usage: TokenUsage::default(),
            }),
        ]);
        let result = run_agent(&provider, &runtime, input(&runtime, "run-knowledge-budget-reuse"), AgentRunLimits::default(), AgentCancellation::default()).await;
        assert_eq!(result.status, AgentRunStatus::Refused);
        assert_eq!(result.accounting.knowledge_searches, MAX_KNOWLEDGE_SEARCHES);
        assert_eq!(result.accounting.tool_calls, MAX_KNOWLEDGE_SEARCHES + 2);
        assert_eq!(result.accounting.simulations, 1);
        assert_eq!(result.accounting.model_turns, 7);
        let debug = result.debug.as_ref().unwrap();
        assert!(debug.tool_calls.iter().any(|call| call.call_id == "knowledge-repeat" && call.reused));
        assert!(debug.tool_calls.iter().any(|call| call.call_id == "diagnose-after-searches" && call.ok));
        assert_eq!(debug.diagnostic_state.no_new_evidence_rounds, 0);
        assert!(provider.requests().iter().all(|request| request.tools.iter().any(|tool| tool.name == "analyze_timeline")));
        assert_eq!(result.trace.iter().filter(|event| event.kind == "knowledge_searches_coalesced").count(), 1);
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
        let content = &result
            .report
            .expect("explicit provider failure report")
            .content;
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
        assert!(result
            .trace
            .iter()
            .any(|event| { event.kind == "provider_failure_evidence_preserved" }));
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
        assert!(provider.requests()[1]
            .messages
            .iter()
            .any(|message| matches!(
                message,
                ModelMessage::User { content } if content.contains("重新发出该动作")
            )));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_tool_arguments_retry"));
        assert_ne!(result.status, AgentRunStatus::ProviderFailed);
    }

    #[tokio::test]
    async fn malformed_tool_arguments_do_not_force_an_early_report() {
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
        assert!(!requests[2].tools.is_empty());
        assert!(requests[2].messages.iter().any(|message| matches!(
            message, ModelMessage::User { content }
                if content.contains("上一个工具动作的参数不是有效 JSON")
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
        assert!(report.evidence_ids.is_empty());
        assert!(report.content.summary.contains("分析当前循环的确定性输出"));
        assert!(report.content.findings.iter().all(|finding| finding.metrics.is_empty()));
        assert!(result.debug.as_ref().unwrap().tool_calls.iter().any(|call| !call.evidence_ids.is_empty()));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "provider_empty_evidence_preserved"));
    }

    #[tokio::test]
    async fn uncited_report_requests_repair_before_any_evidence_fallback() {
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
        assert_eq!(result.accounting.model_turns, 3);
        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert!(!requests[1].tools.is_empty());
        assert!(requests[2].tools.is_empty());
        assert!(requests[2].messages.iter().any(|message| matches!(message,
            ModelMessage::User { content } if content == "分析当前循环的确定性输出。"
        )));
        let report = result.report.unwrap();
        assert!(!report.content.findings.is_empty());
        let unverified_prose = report.content.findings.iter().find(|finding| finding.title == "错误结论").unwrap();
        assert!(unverified_prose.metrics.is_empty());
        assert!(unverified_prose.evidence_ids.is_empty());
        assert!(report.content.findings.iter().flat_map(|finding| &finding.metrics)
            .all(|metric| metric.evidence_id != "b".repeat(64) && metric.value != 999.0));
        assert!(report.evidence_ids.iter().all(|id| id != &"b".repeat(64)));
        assert!(result
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
        assert!(requests[2].messages.iter().any(|message| matches!(message,
            ModelMessage::User { content } if content == "分析当前循环的确定性输出。"
        )));
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "report_repair_requested"));
    }

    #[tokio::test]
    async fn repeated_invalid_json_keeps_debug_evidence_without_replacing_followup_with_baseline() {
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
            AgentRunInput { question: "刚才提到的操作具体发生在哪里？".to_string(), ..input(&runtime, "run-invalid-json-evidence-fallback") },
            AgentRunLimits::default(),
            AgentCancellation::default(),
        )
        .await;

        assert_eq!(result.status, AgentRunStatus::EvidenceInsufficient);
        assert!(result.debug.as_ref().unwrap().tool_calls.iter().any(|call| !call.evidence_ids.is_empty()));
        let report = result.report.expect("explicit report failure");
        assert!(report.content.findings.is_empty());
        assert!(!report.content.summary.contains("当前输出基线"));
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
            vec!["绝刀·50怒伤害占比", "援戈·血影伤害占比", "业火焚城伤害占比"]
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
