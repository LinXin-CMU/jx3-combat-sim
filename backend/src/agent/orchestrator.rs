use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

use super::evidence::validate_trace_id;
use super::prompt::agent_prompt_v1;
use super::provider::{
    FinishReason, LlmProvider, ModelMessage, ModelRequest, StructuredOutputDefinition, TokenUsage,
};
use super::registry::AgentToolRegistry;
use super::report::{
    cited_evidence_ids, parse_and_validate_report, report_content_json_schema,
    AgentReportContentV1, AgentReportV1, AgentRunAccountingV1, AGENT_REPORT_CONTENT_SCHEMA_V1,
    AGENT_REPORT_SCHEMA_V1,
};
use super::{AgentRuntime, ScenarioSnapshotV1};

pub const AGENT_RUN_SCHEMA_V1: &str = "agent-run/v1";
const MAX_QUESTION_BYTES: usize = 16 * 1024;
const MAX_REPORT_REPAIRS: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
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
            wall_time_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentRunInput {
    pub run_id: String,
    pub question: String,
    pub scenario: ScenarioSnapshotV1,
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
}

impl TraceCollector {
    fn new(sink: Option<AgentTraceSink>) -> Self {
        Self {
            events: Vec::new(),
            sink,
        }
    }

    fn push(
        &mut self,
        kind: &str,
        tool_name: Option<String>,
        evidence_ids: Vec<String>,
        code: Option<String>,
    ) {
        let event = AgentTraceEventV1 {
            sequence: self.events.len() as u32 + 1,
            kind: kind.to_string(),
            tool_name,
            evidence_ids,
            code,
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
    let started = Instant::now();
    let prompt = agent_prompt_v1();
    let mut accounting = AgentRunAccountingV1::default();
    let mut trace = TraceCollector::new(event_sink);

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

    let mut registry = AgentToolRegistry::new(&input.scenario, runtime, limits.max_simulations);
    let tools = AgentToolRegistry::definitions();
    let mut messages = vec![ModelMessage::User {
        content: input.question.clone(),
    }];
    let mut repairs = 0;
    trace.push("planning", None, Vec::new(), None);

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

        let request = ModelRequest {
            instructions: prompt.instructions.to_string(),
            messages: messages.clone(),
            tools: tools.clone(),
            response_format: Some(StructuredOutputDefinition {
                name: "agent_report_content_v1".to_string(),
                schema: report_content_json_schema(),
            }),
            max_output_tokens: limits.max_output_tokens_per_turn,
        };
        if request.validate().is_err() {
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

        accounting.model_turns += 1;
        let remaining =
            Duration::from_millis(limits.wall_time_ms).saturating_sub(started.elapsed());
        let provider_result = tokio::select! {
            result = tokio::time::timeout(remaining, provider.complete(&request)) => Some(result),
            _ = cancellation.cancelled() => None,
        };
        let response = match provider_result {
            None => {
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
                )
            }
            Some(Ok(Ok(response))) => response,
            Some(Ok(Err(error))) => {
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
                )
            }
            Some(Err(_)) => {
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
                )
            }
        };
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
        if response.validate_against(&request).is_err() {
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

        if !response.tool_calls.is_empty() {
            if accounting
                .tool_calls
                .saturating_add(response.tool_calls.len() as u32)
                > limits.max_tool_calls
            {
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
            });
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
                accounting.tool_calls += 1;
                trace.push("tool_started", Some(call.name.clone()), Vec::new(), None);
                let outcome = registry.dispatch(&input.run_id, &call.name, call.arguments);
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
                    return terminal_with_registry(
                        provider,
                        &input,
                        &prompt,
                        AgentRunStatus::BudgetExhausted,
                        accounting,
                        None,
                        Some(fixed_error(
                            "simulation_budget",
                            "Simulation budget is exhausted",
                        )),
                        trace,
                        started,
                        &registry,
                    );
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

        trace.push("validating", None, Vec::new(), None);
        let raw = response.assistant_text.as_deref().unwrap_or_default();
        match parse_and_validate_report(raw, registry.evidence()) {
            Ok(content) => {
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
            Err(error) if repairs < MAX_REPORT_REPAIRS => {
                repairs += 1;
                messages.push(ModelMessage::Assistant {
                    content: Some(raw.to_string()),
                    tool_calls: Vec::new(),
                });
                messages.push(ModelMessage::User {
                    content: format!(
                        "The report was rejected with code {}. Return one corrected AgentReportContentV1 JSON object only; do not call more tools unless evidence is missing.",
                        error.code
                    ),
                });
                trace.push(
                    "report_repair_requested",
                    None,
                    Vec::new(),
                    Some(error.code.to_string()),
                );
            }
            Err(error) => {
                let content =
                    refusal_content("现有输出未通过证据校验。", "未验证的数值不会作为结论展示。");
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

fn validate_input(input: &AgentRunInput, limits: &AgentRunLimits) -> Result<(), ()> {
    validate_trace_id(&input.run_id).map_err(|_| ())?;
    if input.question.trim().is_empty()
        || input.question.len() > MAX_QUESTION_BYTES
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
        || limits.wall_time_ms > 60_000
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
    accounting.duration_ms = elapsed_ms(started);
    let termination = status_name(&status).to_string();
    let report = AgentReportV1 {
        schema_version: AGENT_REPORT_SCHEMA_V1.to_string(),
        question: input.question.clone(),
        scenario_hash: input.scenario.scenario_hash.clone(),
        prompt_version: prompt.version.to_string(),
        prompt_sha256: prompt.sha256.clone(),
        provider_profile: provider.profile_id().to_string(),
        model: provider.model().to_string(),
        evidence_ids: cited_evidence_ids(&content),
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
        limitations: vec![limitation.to_string()],
        refusal_reason: Some(summary.to_string()),
    }
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
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    struct ScriptedProvider {
        responses: Mutex<VecDeque<Result<ModelResponse, ProviderError>>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<Result<ModelResponse, ProviderError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
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

        async fn complete(&self, _request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted response exhausted")
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
        assert_eq!(limits.wall_time_ms, 60_000);
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
        assert_eq!(result.accounting.model_turns, 3);
        assert_eq!(result.accounting.tool_calls, 2);
        assert_eq!(result.accounting.simulations, 1);
        let report = result.report.unwrap();
        assert_eq!(report.evidence_ids.len(), 1);
        assert_eq!(
            report.content.findings[0].metrics[0].json_pointer,
            "/result/dps"
        );
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
    async fn simulation_budget_stops_comparison_before_execution() {
        let runtime = AgentRuntime::fixture();
        let provider = ScriptedProvider::new(vec![
            Ok(tool_call("call-read", "get_current_scenario", json!({}))),
            Ok(tool_call(
                "call-compare",
                "compare_scenarios",
                json!({
                    "candidates": [{
                        "label": "增加延迟",
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
            )),
        ]);
        let limits = AgentRunLimits {
            max_simulations: 1,
            ..AgentRunLimits::default()
        };
        let result = run_agent(
            &provider,
            &runtime,
            input(&runtime, "run-budget"),
            limits,
            AgentCancellation::default(),
        )
        .await;
        assert_eq!(result.status, AgentRunStatus::BudgetExhausted);
        assert_eq!(result.accounting.simulations, 0);
        assert_eq!(result.error.unwrap().code, "simulation_budget");
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
        assert_eq!(result.error.unwrap().code, "provider_http_error");
    }

    #[tokio::test]
    async fn ungrounded_numeric_report_gets_one_repair_then_safe_fallback() {
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
            Ok(tool_call("call-read", "get_current_scenario", json!({}))),
            Ok(tool_call("call-sim", "simulate_scenario", json!({}))),
            Ok(ModelResponse {
                assistant_text: Some(bad_report.clone()),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            }),
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

        assert_eq!(result.status, AgentRunStatus::EvidenceInsufficient);
        assert_eq!(result.accounting.model_turns, 4);
        assert!(result.report.unwrap().content.findings.is_empty());
        assert!(result
            .trace
            .iter()
            .any(|event| event.kind == "report_repair_requested"));
    }
}
