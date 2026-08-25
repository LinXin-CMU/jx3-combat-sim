use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::provider::ToolDefinition;
use super::report::EvidenceStore;
use super::{
    analyze_timeline, compare_scenarios, get_current_scenario, simulate_scenario, AgentRuntime,
    CandidatePatchV1, EvidenceEnvelopeV1, PatchValueV1, ScenarioPatchV1, ScenarioSnapshotV1,
    ToolBudget, ToolError,
};

pub const AGENT_TOOL_RESULT_SCHEMA_V1: &str = "agent-tool-result/v1";
pub const MAX_AGENT_CANDIDATES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCandidateV1 {
    pub label: String,
    pub patch: AgentScenarioPatchV1,
}

/// Deliberately smaller than the HTTP comparison patch. These six fields are
/// the first portfolio experiments and map one-to-one onto simulator inputs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentScenarioPatchV1 {
    pub haste_level: Option<u32>,
    pub sequence: Option<Vec<String>>,
    pub network_delay: Option<u32>,
    pub initial_rage: Option<i32>,
    pub base_attack: Option<f64>,
    pub target_defense_bonus: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ToolDispatchOutcome {
    pub output: Value,
    pub evidence_ids: Vec<String>,
    pub budget_exhausted: bool,
}

pub struct AgentToolRegistry<'a> {
    scenario: &'a ScenarioSnapshotV1,
    runtime: &'a AgentRuntime,
    budget: ToolBudget,
    scenario_read: bool,
    evidence: EvidenceStore,
}

impl<'a> AgentToolRegistry<'a> {
    pub fn new(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
    ) -> Self {
        Self {
            scenario,
            runtime,
            budget: ToolBudget::new(max_simulations),
            scenario_read: false,
            evidence: EvidenceStore::new(),
        }
    }

    pub fn definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "get_current_scenario".to_string(),
                description: "Read the immutable current scenario and its evidence identity. This must be called first.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: "simulate_scenario".to_string(),
                description: "Run the immutable baseline scenario once and return deterministic summary evidence.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: "compare_scenarios".to_string(),
                description: "Compare one to three explicit typed candidate patches against the immutable baseline.".to_string(),
                parameters: compare_schema(),
            },
            ToolDefinition {
                name: "analyze_timeline".to_string(),
                description: "Run the immutable baseline and return deterministic timeline diagnosis evidence.".to_string(),
                parameters: empty_object_schema(),
            },
        ]
    }

    pub fn evidence(&self) -> &EvidenceStore {
        &self.evidence
    }

    pub fn used_simulations(&self) -> u32 {
        self.budget.used_simulations
    }

    pub fn scenario_was_read(&self) -> bool {
        self.scenario_read
    }

    pub fn dispatch(
        &mut self,
        trace_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> ToolDispatchOutcome {
        if tool_name != "get_current_scenario" && !self.scenario_read {
            return failure(
                tool_name,
                "scenario_not_read",
                "get_current_scenario must succeed before other tools",
                false,
            );
        }

        match tool_name {
            "get_current_scenario" => {
                if serde_json::from_value::<EmptyArguments>(arguments).is_err() {
                    return invalid_arguments(tool_name);
                }
                match get_current_scenario(trace_id, self.scenario, self.runtime.provenance()) {
                    Ok(evidence) => {
                        self.scenario_read = true;
                        self.success(tool_name, vec![serialize_evidence(evidence)])
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            "simulate_scenario" => {
                if serde_json::from_value::<EmptyArguments>(arguments).is_err() {
                    return invalid_arguments(tool_name);
                }
                let context = self.runtime.context();
                match simulate_scenario(
                    trace_id,
                    self.scenario,
                    &context,
                    self.runtime.provenance(),
                    &mut self.budget,
                ) {
                    Ok(execution) => {
                        self.success(tool_name, vec![serialize_evidence(execution.evidence)])
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            "compare_scenarios" => {
                let args = match serde_json::from_value::<CompareArguments>(arguments) {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                if args.candidates.is_empty() || args.candidates.len() > MAX_AGENT_CANDIDATES {
                    return failure(
                        tool_name,
                        "invalid_candidate_count",
                        "candidate count must be within the configured limit",
                        false,
                    );
                }
                let candidates = match args
                    .candidates
                    .into_iter()
                    .map(|candidate| self.expand_candidate(candidate))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(candidates) => candidates,
                    Err(code) => {
                        return failure(
                            tool_name,
                            code,
                            "candidate patch cannot be applied to this scenario",
                            false,
                        )
                    }
                };
                let context = self.runtime.context();
                match compare_scenarios(
                    trace_id,
                    self.scenario,
                    &candidates,
                    &context,
                    self.runtime.provenance(),
                    &mut self.budget,
                ) {
                    Ok(execution) => {
                        self.success(tool_name, vec![serialize_evidence(execution.evidence)])
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            "analyze_timeline" => {
                if serde_json::from_value::<EmptyArguments>(arguments).is_err() {
                    return invalid_arguments(tool_name);
                }
                let context = self.runtime.context();
                match simulate_scenario(
                    trace_id,
                    self.scenario,
                    &context,
                    self.runtime.provenance(),
                    &mut self.budget,
                ) {
                    Ok(simulation) => {
                        let simulation_evidence = serialize_evidence(simulation.evidence.clone());
                        match analyze_timeline(trace_id, &simulation, self.runtime.provenance()) {
                            Ok(timeline) => self.success(
                                tool_name,
                                vec![simulation_evidence, serialize_evidence(timeline.evidence)],
                            ),
                            Err(error) => tool_failure(tool_name, error),
                        }
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            _ => failure(
                tool_name,
                "unknown_tool",
                "tool is not registered for this Agent",
                false,
            ),
        }
    }

    fn expand_candidate(
        &self,
        candidate: AgentCandidateV1,
    ) -> Result<CandidatePatchV1, &'static str> {
        let patch = candidate.patch;
        validate_agent_patch(&patch)?;
        let attributes = if let Some(base_attack) = patch.base_attack {
            let mut attributes = self
                .scenario
                .simulation
                .attributes
                .clone()
                .ok_or("missing_attributes")?;
            attributes.base_attack = base_attack;
            Some(attributes)
        } else {
            None
        };
        let target = if let Some(defense_bonus) = patch.target_defense_bonus {
            let mut target = self
                .scenario
                .simulation
                .target
                .clone()
                .ok_or("missing_target")?;
            target.defense_bonus = defense_bonus;
            Some(target)
        } else {
            None
        };
        Ok(CandidatePatchV1 {
            label: candidate.label,
            patch: ScenarioPatchV1 {
                haste_level: patch.haste_level,
                sequence: patch.sequence,
                network_delay: patch.network_delay,
                initial_rage: patch.initial_rage.map(PatchValueV1::Set),
                attributes,
                target,
                ..ScenarioPatchV1::default()
            },
        })
    }

    fn success(&mut self, tool_name: &str, envelopes: Vec<Value>) -> ToolDispatchOutcome {
        let mut evidence_ids = Vec::with_capacity(envelopes.len());
        for envelope in &envelopes {
            if let Some(id) = envelope.get("evidence_id").and_then(Value::as_str) {
                evidence_ids.push(id.to_string());
                self.evidence.insert(id.to_string(), envelope.clone());
            }
        }
        ToolDispatchOutcome {
            output: json!({
                "schema_version": AGENT_TOOL_RESULT_SCHEMA_V1,
                "ok": true,
                "tool_name": tool_name,
                "evidence_ids": evidence_ids,
                "evidence": envelopes,
            }),
            evidence_ids,
            budget_exhausted: false,
        }
    }
}

fn validate_agent_patch(patch: &AgentScenarioPatchV1) -> Result<(), &'static str> {
    if patch.haste_level.is_some_and(|value| value > 10_000_000) {
        return Err("invalid_haste_level");
    }
    if patch.network_delay.is_some_and(|value| value > 5_000) {
        return Err("invalid_network_delay");
    }
    if patch
        .initial_rage
        .is_some_and(|value| !(-1_000..=1_000).contains(&value))
    {
        return Err("invalid_initial_rage");
    }
    if patch
        .base_attack
        .is_some_and(|value| !value.is_finite() || !(0.0..=1_000_000_000.0).contains(&value))
    {
        return Err("invalid_base_attack");
    }
    if patch
        .target_defense_bonus
        .is_some_and(|value| !value.is_finite() || !(-100.0..=1_000.0).contains(&value))
    {
        return Err("invalid_target_defense");
    }
    if patch.sequence.as_ref().is_some_and(|sequence| {
        sequence.len() > 256
            || sequence.iter().any(|entry| {
                entry.trim().is_empty()
                    || entry.chars().count() > 128
                    || entry.chars().any(char::is_control)
            })
    }) {
        return Err("invalid_sequence");
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArguments {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareArguments {
    candidates: Vec<AgentCandidateV1>,
}

fn serialize_evidence<T: Serialize>(evidence: EvidenceEnvelopeV1<T>) -> Value {
    serde_json::to_value(evidence).expect("evidence envelope must remain serializable")
}

fn invalid_arguments(tool_name: &str) -> ToolDispatchOutcome {
    failure(
        tool_name,
        "invalid_tool_arguments",
        "tool arguments do not match the registered schema",
        false,
    )
}

fn tool_failure(tool_name: &str, error: ToolError) -> ToolDispatchOutcome {
    let (code, message, budget_exhausted) = match error {
        ToolError::BudgetExceeded { .. } => (
            "simulation_budget_exhausted",
            "simulation budget is exhausted",
            true,
        ),
        ToolError::RuntimeMismatch { .. } => (
            "runtime_mismatch",
            "scenario does not match the immutable runtime",
            false,
        ),
        ToolError::InvalidCandidateCount { .. } => (
            "invalid_candidate_count",
            "candidate count is invalid",
            false,
        ),
        ToolError::InvalidCandidateLabel => (
            "invalid_candidate_label",
            "candidate label is invalid",
            false,
        ),
        ToolError::DuplicateCandidateLabel { .. } => (
            "duplicate_candidate_label",
            "candidate labels must be unique",
            false,
        ),
        ToolError::NoScenarioChanges { .. } => (
            "no_scenario_changes",
            "candidate must change the baseline",
            false,
        ),
        ToolError::TimelineDetailsUnavailable => (
            "timeline_unavailable",
            "timeline details are unavailable",
            false,
        ),
        ToolError::Scenario(_) | ToolError::Evidence(_) => (
            "invalid_scenario_or_evidence",
            "tool input is invalid",
            false,
        ),
    };
    failure(tool_name, code, message, budget_exhausted)
}

fn failure(
    tool_name: &str,
    code: &'static str,
    message: &'static str,
    budget_exhausted: bool,
) -> ToolDispatchOutcome {
    ToolDispatchOutcome {
        output: json!({
            "schema_version": AGENT_TOOL_RESULT_SCHEMA_V1,
            "ok": false,
            "tool_name": tool_name,
            "error": {"code": code, "message": message},
        }),
        evidence_ids: Vec::new(),
        budget_exhausted,
    }
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
}

fn compare_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "candidates": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_AGENT_CANDIDATES,
                "items": {
                    "type": "object",
                    "properties": {
                        "label": {"type": "string", "minLength": 1, "maxLength": 64},
                        "patch": {
                            "type": "object",
                            "properties": {
                                "haste_level": {"type": ["integer", "null"], "minimum": 0, "maximum": 10000000},
                                "sequence": {"type": ["array", "null"], "maxItems": 256, "items": {"type": "string", "maxLength": 128}},
                                "network_delay": {"type": ["integer", "null"], "minimum": 0, "maximum": 5000},
                                "initial_rage": {"type": ["integer", "null"], "minimum": -1000, "maximum": 1000},
                                "base_attack": {"type": ["number", "null"], "minimum": 0, "maximum": 1000000000},
                                "target_defense_bonus": {"type": ["number", "null"], "minimum": -100, "maximum": 1000}
                            },
                            "required": ["haste_level", "sequence", "network_delay", "initial_rage", "base_attack", "target_defense_bonus"],
                            "additionalProperties": false
                        }
                    },
                    "required": ["label", "patch"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["candidates"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_four_read_only_tools_are_exposed() {
        let definitions = AgentToolRegistry::definitions();
        let names = definitions
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "get_current_scenario",
                "simulate_scenario",
                "compare_scenarios",
                "analyze_timeline"
            ]
        );
        let encoded = serde_json::to_string(&definitions).unwrap();
        assert!(!encoded.contains("path"));
        assert!(!encoded.contains("url"));
        assert!(!encoded.contains("write"));
    }

    #[test]
    fn comparison_schema_is_closed_and_bounded() {
        let schema = compare_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["candidates"]["maxItems"], 3);
        assert_eq!(
            schema["properties"]["candidates"]["items"]["properties"]["patch"]
                ["additionalProperties"],
            false
        );
    }

    #[test]
    fn local_patch_validation_does_not_trust_upstream_schema_enforcement() {
        let invalid = AgentScenarioPatchV1 {
            network_delay: Some(5_001),
            ..AgentScenarioPatchV1::default()
        };
        assert_eq!(validate_agent_patch(&invalid), Err("invalid_network_delay"));

        let oversized = AgentScenarioPatchV1 {
            sequence: Some(vec!["盾击".to_string(); 257]),
            ..AgentScenarioPatchV1::default()
        };
        assert_eq!(validate_agent_patch(&oversized), Err("invalid_sequence"));
    }
}
