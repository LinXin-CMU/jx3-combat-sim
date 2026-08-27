use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;

use super::provider::ToolDefinition;
use super::report::EvidenceStore;
use super::{
    analyze_timeline, compare_scenarios, get_current_scenario, simulate_scenario, AgentRuntime,
    CandidatePatchV1, EvidenceEnvelopeV1, KnowledgeAudience, KnowledgeIndex, KnowledgeIndexError,
    KnowledgeMountScope, KnowledgeSearchQuery, KnowledgeVersionContext, KnowledgeVersionScope,
    PatchValueV1, ScenarioPatchV1, ScenarioSnapshotV1, ToolBudget, ToolError,
    MAX_KNOWLEDGE_RESULTS,
};
use crate::Mount;

pub const AGENT_TOOL_RESULT_SCHEMA_V1: &str = "agent-tool-result/v1";
pub const MAX_AGENT_CANDIDATES: usize = 3;
pub const MAX_KNOWLEDGE_SEARCHES: u32 = 2;

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
    knowledge: Option<&'a KnowledgeIndex>,
    budget: ToolBudget,
    knowledge_searches: u32,
    knowledge_audience: KnowledgeAudience,
    scenario_read: bool,
    evidence: EvidenceStore,
}

impl<'a> AgentToolRegistry<'a> {
    pub fn new(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
    ) -> Self {
        Self::new_with_knowledge(scenario, runtime, max_simulations, None)
    }

    pub fn new_with_knowledge(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
        knowledge: Option<&'a KnowledgeIndex>,
    ) -> Self {
        let mount = match runtime.mount() {
            Mount::FenShanJin => KnowledgeMountScope::Fenshanjin,
            Mount::TieGuYi => KnowledgeMountScope::Tieguyi,
        };
        Self {
            scenario,
            runtime,
            knowledge,
            budget: ToolBudget::new(max_simulations),
            knowledge_searches: 0,
            knowledge_audience: KnowledgeAudience::from_question("", Some(mount)),
            scenario_read: false,
            evidence: EvidenceStore::new(),
        }
    }

    pub fn set_knowledge_question(&mut self, question: &str) {
        let mount = match self.runtime.mount() {
            Mount::FenShanJin => KnowledgeMountScope::Fenshanjin,
            Mount::TieGuYi => KnowledgeMountScope::Tieguyi,
        };
        self.knowledge_audience = KnowledgeAudience::from_question(question, Some(mount));
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

    pub fn definitions_with_knowledge(
        seasons: &[String],
        categories: &[String],
    ) -> Vec<ToolDefinition> {
        let mut definitions = Self::definitions();
        definitions.push(ToolDefinition {
            name: "search_knowledge_base".to_string(),
            description: "Search the bounded local JX3 knowledge snapshot. The server adaptively selects evidence count and source roles; do not request a fixed top-k. Version scope is enforced by the server; use reference_lookup only for version-independent people, author, source, or nickname identity; use null for category unless an exact allowed category is needed, and copy an exact allowed season for specific_season.".to_string(),
            parameters: knowledge_search_schema(seasons, categories),
        });
        definitions
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

    pub fn used_knowledge_searches(&self) -> u32 {
        self.knowledge_searches
    }

    pub fn coalesced_knowledge_search() -> ToolDispatchOutcome {
        failure(
            "search_knowledge_base",
            "knowledge_search_coalesced",
            "this redundant search was coalesced; use the completed search results and return a report",
            false,
        )
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
            "search_knowledge_base" => {
                let args = match serde_json::from_value::<KnowledgeArguments>(arguments) {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                let query = match args.into_query() {
                    Ok(query) => query,
                    Err(error) => return knowledge_failure(tool_name, error),
                };
                let Some(knowledge) = self.knowledge else {
                    return failure(
                        tool_name,
                        "knowledge_unavailable",
                        "local knowledge index is not configured",
                        false,
                    );
                };
                if self.knowledge_searches >= MAX_KNOWLEDGE_SEARCHES {
                    return failure(
                        tool_name,
                        "knowledge_search_budget_exhausted",
                        "knowledge search budget is exhausted",
                        true,
                    );
                }
                self.knowledge_searches += 1;
                let started = Instant::now();
                let evidence_args =
                    serde_json::to_value(&query).expect("knowledge query must remain serializable");
                let context =
                    KnowledgeVersionContext::from_game_version(self.runtime.game_version());
                let result =
                    if matches!(&query.version_scope, KnowledgeVersionScope::ReferenceLookup) {
                        knowledge.search(&context, query)
                    } else {
                        knowledge.search_with_audience(&context, query, self.knowledge_audience)
                    };
                match result {
                    Ok(result) => {
                        // A search response can contain several documents. Give every result its
                        // own evidence identity so the model can cite the exact supporting chunk
                        // instead of accidentally attaching every retrieved source to one claim.
                        // Keep one empty response envelope when nothing matched so insufficiency
                        // remains an auditable result.
                        let evidence_results = if result.results.is_empty() {
                            vec![result]
                        } else {
                            result
                                .results
                                .iter()
                                .cloned()
                                .map(|item| {
                                    let mut single = result.clone();
                                    single.results = vec![item];
                                    single
                                })
                                .collect()
                        };
                        let duration_ms =
                            started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                        let mut envelopes = Vec::with_capacity(evidence_results.len());
                        for evidence_result in evidence_results {
                            let evidence = match EvidenceEnvelopeV1::new(
                                trace_id,
                                tool_name,
                                &self.scenario.scenario_hash,
                                evidence_args.clone(),
                                evidence_result,
                                self.runtime.provenance(),
                                duration_ms,
                            ) {
                                Ok(evidence) => evidence,
                                Err(_) => {
                                    return failure(
                                        tool_name,
                                        "knowledge_evidence_failed",
                                        "knowledge search evidence could not be created",
                                        false,
                                    )
                                }
                            };
                            envelopes.push(serialize_evidence(evidence));
                        }
                        self.success(tool_name, envelopes)
                    }
                    Err(error) => knowledge_failure(tool_name, error),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnowledgeArguments {
    query: String,
    version_scope: String,
    season: Option<String>,
    category: Option<String>,
}

impl KnowledgeArguments {
    fn into_query(self) -> Result<KnowledgeSearchQuery, KnowledgeIndexError> {
        let version_scope = match self.version_scope.as_str() {
            "current_only" if self.season.is_none() => KnowledgeVersionScope::CurrentOnly,
            "specific_season" => KnowledgeVersionScope::SpecificSeason {
                season: self
                    .season
                    .filter(|season| !season.trim().is_empty())
                    .ok_or(KnowledgeIndexError::InvalidQuery(
                        "specific season is required",
                    ))?,
            },
            "cross_version" if self.season.is_none() => KnowledgeVersionScope::CrossVersion,
            "reference_lookup" if self.season.is_none() => KnowledgeVersionScope::ReferenceLookup,
            _ => {
                return Err(KnowledgeIndexError::InvalidQuery(
                    "version scope and season do not match",
                ))
            }
        };
        let query = if matches!(&version_scope, KnowledgeVersionScope::ReferenceLookup) {
            normalize_reference_query(&self.query)
        } else {
            self.query
        };
        Ok(KnowledgeSearchQuery {
            query,
            version_scope,
            category: self.category,
            top_k: MAX_KNOWLEDGE_RESULTS,
        })
    }
}

pub(super) fn normalize_reference_query(value: &str) -> String {
    let original = value.trim();
    let mut normalized = original.to_string();
    // Reference lookup is an entity point query, not a semantic gameplay query.
    // Remove conversational intent words so exact alias matching and deterministic
    // relationship extraction can run even when a provider submits a full question.
    for noise in [
        "给我一个名字",
        "给出一个名字",
        "告诉我名字",
        "你觉得",
        "请问",
        "到底",
        "具体",
        "真实身份",
        "这个人",
        "是谁",
        "谁是",
        "身份",
        "玩家",
        "作者",
        "昵称",
        "名字",
        "苍云",
        "剑网三",
        "剑网3",
    ] {
        normalized = normalized.replace(noise, "");
    }
    let normalized = normalized
        .chars()
        .filter(|character| {
            !character.is_whitespace()
                && !matches!(
                    *character,
                    '?' | '？'
                        | '!'
                        | '！'
                        | ','
                        | '，'
                        | '。'
                        | ':'
                        | '：'
                        | '"'
                        | '\''
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '['
                        | ']'
                        | '【'
                        | '】'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                )
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('的').to_string();
    if normalized.is_empty() {
        original.to_string()
    } else {
        normalized
    }
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

fn knowledge_failure(tool_name: &str, error: KnowledgeIndexError) -> ToolDispatchOutcome {
    let (code, message) = match error {
        KnowledgeIndexError::VersionConflict { .. } => (
            "knowledge_version_conflict",
            "the question names a version outside the requested scope",
        ),
        KnowledgeIndexError::CrossVersionIntentRequired => (
            "cross_version_intent_required",
            "cross-version search requires an explicit history or comparison question",
        ),
        KnowledgeIndexError::UnknownSeason(_) => (
            "unknown_knowledge_season",
            "the requested season is not present in the local knowledge snapshot",
        ),
        KnowledgeIndexError::UnknownCategory(_) => (
            "unknown_knowledge_category",
            "the requested category is not present in the local knowledge snapshot",
        ),
        KnowledgeIndexError::InvalidQuery(_) => (
            "invalid_knowledge_query",
            "the knowledge query is invalid or outside configured bounds",
        ),
        KnowledgeIndexError::NotConfigured
        | KnowledgeIndexError::Io(_)
        | KnowledgeIndexError::InvalidManifest(_)
        | KnowledgeIndexError::UnsafePath(_)
        | KnowledgeIndexError::CorpusLimit(_) => (
            "knowledge_unavailable",
            "the local knowledge snapshot is unavailable",
        ),
    };
    failure(tool_name, code, message, false)
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

fn knowledge_search_schema(seasons: &[String], categories: &[String]) -> Value {
    let season_values = std::iter::once(Value::Null)
        .chain(seasons.iter().cloned().map(Value::String))
        .collect::<Vec<_>>();
    let category_values = std::iter::once(Value::Null)
        .chain(categories.iter().cloned().map(Value::String))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "minLength": 1, "maxLength": 200},
            "version_scope": {
                "type": "string",
                "enum": ["current_only", "specific_season", "cross_version", "reference_lookup"]
            },
            "season": {"type": ["string", "null"], "enum": season_values, "maxLength": 64},
            "category": {"type": ["string", "null"], "enum": category_values, "maxLength": 64},
        },
        "required": ["query", "version_scope", "season", "category"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn knowledge_schema_is_closed_bounded_and_does_not_expose_paths_or_urls() {
        let seasons = vec![
            "暗影千机（2026）".to_string(),
            "太极秘录（2025）".to_string(),
        ];
        let categories = vec!["基础".to_string(), "白皮书".to_string()];
        let definitions = AgentToolRegistry::definitions_with_knowledge(&seasons, &categories);
        assert_eq!(definitions.len(), 5);
        let knowledge = definitions.last().unwrap();
        assert_eq!(knowledge.name, "search_knowledge_base");
        assert_eq!(knowledge.parameters["additionalProperties"], false);
        assert!(knowledge.parameters["properties"].get("top_k").is_none());
        assert_eq!(
            knowledge.parameters["properties"]["season"]["enum"],
            json!([null, "暗影千机（2026）", "太极秘录（2025）"])
        );
        assert_eq!(
            knowledge.parameters["properties"]["category"]["enum"],
            json!([null, "基础", "白皮书"])
        );
        assert!(knowledge.parameters["properties"]["version_scope"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("reference_lookup")));
        let encoded = serde_json::to_string(knowledge).unwrap();
        assert!(!encoded.contains("path"));
        assert!(!encoded.contains("url"));
        assert!(!encoded.contains("write"));
    }

    #[test]
    fn knowledge_argument_scope_is_validated_locally() {
        let invalid = KnowledgeArguments {
            query: "盾飞".to_string(),
            version_scope: "current_only".to_string(),
            season: Some("山海源流（2025）".to_string()),
            category: None,
        };
        assert!(matches!(
            invalid.into_query().unwrap_err(),
            KnowledgeIndexError::InvalidQuery(_)
        ));

        let valid = KnowledgeArguments {
            query: "山海源流盾飞".to_string(),
            version_scope: "specific_season".to_string(),
            season: Some("山海源流（2025）".to_string()),
            category: Some("基础".to_string()),
        }
        .into_query()
        .unwrap();
        assert!(matches!(
            valid.version_scope,
            KnowledgeVersionScope::SpecificSeason { .. }
        ));

        let reference = KnowledgeArguments {
            query: "世一苍是谁".to_string(),
            version_scope: "reference_lookup".to_string(),
            season: None,
            category: None,
        }
        .into_query()
        .unwrap();
        assert!(matches!(
            reference.version_scope,
            KnowledgeVersionScope::ReferenceLookup
        ));
        assert_eq!(reference.query, "世一苍");
        assert_eq!(
            normalize_reference_query("谁是苍云玩家 dereck365？"),
            "dereck365"
        );
    }

    #[test]
    fn knowledge_dispatch_is_grounded_and_budgeted() {
        let (root, knowledge) = knowledge_fixture();
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let mut registry =
            AgentToolRegistry::new_with_knowledge(&scenario, &runtime, 1, Some(&knowledge));
        let prefetched = registry.dispatch("knowledge-run", "get_current_scenario", json!({}));
        assert_eq!(prefetched.output["ok"], true);

        let arguments = json!({
            "query": "盾飞劫刀流血",
            "version_scope": "current_only",
            "season": null,
            "category": null
        });
        let first = registry.dispatch("knowledge-run", "search_knowledge_base", arguments.clone());
        assert_eq!(first.output["ok"], true);
        assert_eq!(first.evidence_ids.len(), 2);
        assert!(first.output["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|evidence| evidence["result"]["results"].as_array().unwrap().len() == 1));
        assert_eq!(
            first.output["evidence"][0]["result"]["results"][0]["season"],
            "暗影千机（2026）"
        );
        assert_eq!(
            first.output["evidence"][0]["result"]["results"][0]["version_match"],
            "current_exact"
        );
        let second = registry.dispatch("knowledge-run", "search_knowledge_base", arguments.clone());
        assert_eq!(second.output["ok"], true);
        let third = registry.dispatch("knowledge-run", "search_knowledge_base", arguments);
        assert_eq!(
            third.output["error"]["code"],
            "knowledge_search_budget_exhausted"
        );
        assert!(third.budget_exhausted);
        assert_eq!(registry.used_knowledge_searches(), 2);

        let expected_root = env::temp_dir();
        assert!(root.starts_with(&expected_root));
        let _ = fs::remove_dir_all(root);
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

    fn knowledge_fixture() -> (PathBuf, KnowledgeIndex) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("jx3-registry-knowledge-{nonce}"));
        let relative = "暗影千机（2026）/基础/循环.md";
        let document = root.join(relative);
        fs::create_dir_all(document.parent().unwrap()).unwrap();
        fs::write(
            &document,
            "---\ntitle: 当前循环\n---\n\n# 当前循环\n\n盾飞阶段使用劫刀并关注流血。\n",
        )
        .unwrap();
        let second_relative = "暗影千机（2026）/基础/流血.md";
        fs::write(
            root.join(second_relative),
            "---\ntitle: 流血说明\n---\n\n# 流血说明\n\n盾飞后衔接劫刀可用于流血思路。\n",
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
                        "source": "https://www.yuque.com/sgyxy/cangyun/current",
                        "output": relative,
                        "source_site": "www.yuque.com",
                        "yuque_url": "https://www.yuque.com/sgyxy/cangyun/current",
                        "updated_at": "2026-08-26T00:00:00Z"
                    },
                    {
                        "title": "流血说明",
                        "season": "暗影千机（2026）",
                        "category": "基础",
                        "kind": "yuque_document",
                        "source": "https://www.yuque.com/sgyxy/cangyun/bleed",
                        "output": second_relative,
                        "source_site": "www.yuque.com",
                        "yuque_url": "https://www.yuque.com/sgyxy/cangyun/bleed",
                        "updated_at": "2026-08-26T00:00:00Z"
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let index = KnowledgeIndex::load(&root).unwrap();
        (root, index)
    }
}
