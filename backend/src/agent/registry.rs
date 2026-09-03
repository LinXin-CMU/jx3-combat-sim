use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;

use super::provider::ToolDefinition;
use super::equipment::{
    compare_focus, compare_strategies, inspect_workspace, search_catalog, EquipmentCatalogQueryV1,
    EquipmentComparisonPresentationV1, EquipmentWorkspaceV1, COMPARE_FOCUSED_EQUIPMENT,
    INSPECT_EQUIPMENT_WORKSPACE, SEARCH_EQUIPMENT_CATALOG, COMPARE_EQUIPMENT_STRATEGIES,
};
use super::report::EvidenceStore;
use super::{
    analyze_timeline, compare_scenarios, get_current_scenario, inspect_rotation_input,
    simulate_scenario, AgentRuntime, CandidatePatchV1, EvidenceEnvelopeV1, KnowledgeAudience,
    KnowledgeIndex, KnowledgeIndexError, KnowledgeMountScope, KnowledgeSearchQuery,
    KnowledgeVersionContext, KnowledgeVersionScope, PatchValueV1, SavedArtifactError,
    SavedArtifactKind, ScenarioPatchV1, ScenarioSnapshotV1, ToolBudget, ToolError,
    COMPARE_SAVED_MACROS, COMPARE_SAVED_SCENARIOS, INSPECT_ROTATION_INPUT,
    LIST_SAVED_ARTIFACTS, MAX_KNOWLEDGE_RESULTS, READ_SAVED_ARTIFACT,
};
use crate::macro_parser::parse_macro_text;
use crate::Mount;

pub const AGENT_TOOL_RESULT_SCHEMA_V1: &str = "agent-tool-result/v1";
pub const ASK_USER_QUESTION: &str = "ask_user_question";
pub const MAX_AGENT_CANDIDATES: usize = 3;
pub const MAX_KNOWLEDGE_SEARCHES: u32 = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AskUserQuestionArguments {
    pub question: String,
    pub reason: String,
    #[serde(default)]
    pub answer_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCandidateV1 {
    pub label: String,
    pub patch: AgentScenarioPatchV1,
}

/// Bounded, typed fields the Agent may vary in an immutable comparison run.
/// Every field maps one-to-one onto a simulator input and is validated locally.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentScenarioPatchV1 {
    pub haste_level: Option<u32>,
    pub sequence: Option<Vec<String>>,
    pub sequence_edits: Option<Vec<AgentSequenceEditV1>>,
    pub sequence_splices: Option<Vec<AgentSequenceSpliceV1>>,
    pub network_delay: Option<u32>,
    pub initial_rage: Option<i32>,
    pub base_attack: Option<f64>,
    pub target_defense_bonus: Option<f64>,
    pub macro_text: Option<String>,
    pub talents: Option<Vec<u32>>,
    pub recipes: Option<Vec<u32>>,
    pub equipment: Option<std::collections::HashMap<String, u32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSequenceEditV1 {
    pub op: AgentSequenceEditOperationV1,
    /// One-based line number from inspect_rotation_input.
    pub line_number: usize,
    pub skill_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentSequenceEditOperationV1 {
    InsertBefore,
    InsertAfter,
    Replace,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSequenceSpliceV1 {
    /// Inclusive, one-based baseline range.
    pub start_line_number: usize,
    pub end_line_number: usize,
    /// Empty removes the range; otherwise this replaces it atomically.
    pub replacement: Vec<String>,
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
    equipment_workspace: Option<EquipmentWorkspaceV1>,
    equipment_comparisons: Vec<EquipmentComparisonPresentationV1>,
}

impl<'a> AgentToolRegistry<'a> {
    pub fn new(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
    ) -> Self {
        Self::new_with_context(scenario, runtime, max_simulations, None, None)
    }

    pub fn new_with_knowledge(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
        knowledge: Option<&'a KnowledgeIndex>,
    ) -> Self {
        Self::new_with_context(scenario, runtime, max_simulations, knowledge, None)
    }

    pub fn new_with_context(
        scenario: &'a ScenarioSnapshotV1,
        runtime: &'a AgentRuntime,
        max_simulations: u32,
        knowledge: Option<&'a KnowledgeIndex>,
        equipment_workspace: Option<EquipmentWorkspaceV1>,
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
            equipment_workspace,
            equipment_comparisons: Vec::new(),
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
                description: "Read the immutable current scenario, resolved version and evidence identity.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: ASK_USER_QUESTION.to_string(),
                description: "Pause this run and ask the user for one essential decision or missing fact. Use when the answer materially changes the analysis and local tools cannot determine it. The next user message continues in the same session.".to_string(),
                parameters: ask_user_question_schema(),
            },
            ToolDefinition {
                name: INSPECT_ROTATION_INPUT.to_string(),
                description: "Search a manual rotation by skill name and return exact one-based line numbers with neighboring operations. Macro mode has no manual-operation rows; its complete parsed statements are provided by get_current_scenario. Results are paged at eight matches with next_start_index.".to_string(),
                parameters: inspect_rotation_schema(),
            },
            ToolDefinition {
                name: "simulate_scenario".to_string(),
                description: "Run the immutable baseline scenario once and return deterministic summary evidence.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: "compare_scenarios".to_string(),
                description: "Compare one to three typed candidate patches against the immutable baseline. Each patch contains the fields that change; omitted fields inherit the frozen baseline.".to_string(),
                parameters: compare_schema(),
            },
            ToolDefinition {
                name: "analyze_timeline".to_string(),
                description: "Run the immutable baseline and return deterministic timeline diagnosis evidence.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: LIST_SAVED_ARTIFACTS.to_string(),
                description: "Search the current user's saved simulator artifacts by display name and return opaque artifact IDs for reading or comparison.".to_string(),
                parameters: list_saved_schema(),
            },
            ToolDefinition {
                name: READ_SAVED_ARTIFACT.to_string(),
                description: "Read one saved simulator artifact selected by an opaque ID from list_saved_artifacts.".to_string(),
                parameters: read_saved_schema(),
            },
            ToolDefinition {
                name: COMPARE_SAVED_MACROS.to_string(),
                description: "Run a deterministic A/B comparison of two saved artifacts that contain macros. The server freezes the current attributes, equipment, talents, recipes, target, latency and buffs, and changes only the macro text. IDs must come from list_saved_artifacts.".to_string(),
                parameters: compare_saved_schema(),
            },
            ToolDefinition {
                name: COMPARE_SAVED_SCENARIOS.to_string(),
                description: "Run a deterministic comparison of two complete saved loop or battle-plaza scenarios. The server validates version and mount and reports the actual changed fields. IDs must come from list_saved_artifacts.".to_string(),
                parameters: compare_saved_schema(),
            },
            ToolDefinition {
                name: INSPECT_EQUIPMENT_WORKSPACE.to_string(),
                description: "Read the current equipment workspace, exact equipped item names, computed panel and the focused candidate. Use for equipment questions; it is read-only.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: COMPARE_FOCUSED_EQUIPMENT.to_string(),
                description: "Recalculate the focused item swap and run the before/after builds with the same frozen rotation. Returns exact two-column panel deltas and deterministic DPS/skill results. Use this before judging whether the focused replacement is better.".to_string(),
                parameters: empty_object_schema(),
            },
            ToolDefinition {
                name: SEARCH_EQUIPMENT_CATALOG.to_string(),
                description: "Search the local equipment catalog by exact name or common jargon. '四件套/4件套' means normal set pieces; '四切糕/4切糕' means crafted 切糕 set pieces. Results are candidates, not proof of DPS.".to_string(),
                parameters: equipment_search_schema(),
            },
            ToolDefinition {
                name: COMPARE_EQUIPMENT_STRATEGIES.to_string(),
                description: "Build a current-catalog four-piece ordinary set and four-piece crafted 切糕 variant on the frozen workspace, recalculate both panels, and simulate both with the same rotation. Use only for 四件套 versus 四切糕 questions.".to_string(),
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
            description: "Search the local JX3 knowledge snapshot. The server selects evidence count and source roles, applies version scope, and supports reference_lookup for people, authors, sources and nicknames. Use null for broad category search and an allowed season name for specific_season.".to_string(),
            parameters: knowledge_search_schema(seasons, categories),
        });
        definitions
    }

    pub fn evidence(&self) -> &EvidenceStore {
        &self.evidence
    }

    pub fn equipment_comparisons(&self) -> &[EquipmentComparisonPresentationV1] {
        &self.equipment_comparisons
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
            INSPECT_ROTATION_INPUT => {
                let args = match serde_json::from_value::<InspectRotationArguments>(arguments) {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                if args.limit == 0
                    || args.limit > 32
                    || args.context_radius > 8
                    || args.query.as_ref().is_some_and(|query| {
                        query.trim().is_empty()
                            || query.chars().count() > 64
                            || query.chars().any(char::is_control)
                    })
                {
                    return invalid_arguments(tool_name);
                }
                match inspect_rotation_input(
                    trace_id,
                    self.scenario,
                    args.query.as_deref(),
                    args.start_index,
                    args.limit,
                    if args.query.is_some() {
                        args.context_radius.max(6)
                    } else {
                        args.context_radius
                    },
                    self.runtime.provenance(),
                ) {
                    Ok(evidence) => self.success(tool_name, vec![serialize_evidence(evidence)]),
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
            LIST_SAVED_ARTIFACTS => {
                let args = match serde_json::from_value::<ListSavedArguments>(arguments.clone()) {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                let started = Instant::now();
                match super::list_saved_artifacts(&crate::userdata_base(), &args.query, &args.kinds)
                {
                    Ok(result) => match EvidenceEnvelopeV1::new(
                        trace_id,
                        tool_name,
                        &self.scenario.scenario_hash,
                        arguments,
                        result,
                        self.runtime.provenance(),
                        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    ) {
                        Ok(evidence) => self.success(tool_name, vec![serialize_evidence(evidence)]),
                        Err(_) => saved_failure(tool_name, SavedArtifactError::Unreadable),
                    },
                    Err(error) => saved_failure(tool_name, error),
                }
            }
            READ_SAVED_ARTIFACT => {
                let args = match serde_json::from_value::<ReadSavedArguments>(arguments.clone()) {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                let started = Instant::now();
                match super::read_saved_artifact(&crate::userdata_base(), &args.artifact_id) {
                    Ok(result) => match EvidenceEnvelopeV1::new(
                        trace_id,
                        tool_name,
                        &self.scenario.scenario_hash,
                        arguments,
                        result,
                        self.runtime.provenance(),
                        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    ) {
                        Ok(evidence) => self.success(tool_name, vec![serialize_evidence(evidence)]),
                        Err(_) => saved_failure(tool_name, SavedArtifactError::Unreadable),
                    },
                    Err(error) => saved_failure(tool_name, error),
                }
            }
            COMPARE_SAVED_MACROS | COMPARE_SAVED_SCENARIOS => {
                let args = match serde_json::from_value::<CompareSavedArguments>(arguments.clone())
                {
                    Ok(args) => args,
                    Err(_) => return invalid_arguments(tool_name),
                };
                let prepared = if tool_name == COMPARE_SAVED_MACROS {
                    super::prepare_saved_macro_comparison(
                        &crate::userdata_base(),
                        &args.left_id,
                        &args.right_id,
                        self.scenario,
                        self.runtime.game_version(),
                        self.runtime.mount(),
                    )
                } else {
                    super::prepare_saved_scenario_comparison(
                        &crate::userdata_base(),
                        &args.left_id,
                        &args.right_id,
                        self.scenario,
                        self.runtime.game_version(),
                        self.runtime.mount(),
                    )
                };
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => return saved_failure(tool_name, error),
                };
                let started = Instant::now();
                let context_evidence = match EvidenceEnvelopeV1::new(
                    trace_id,
                    tool_name,
                    &prepared.baseline.scenario_hash,
                    arguments,
                    prepared.context,
                    self.runtime.provenance(),
                    0,
                ) {
                    Ok(evidence) => serialize_evidence(evidence),
                    Err(_) => return saved_failure(tool_name, SavedArtifactError::Unreadable),
                };
                let context = self.runtime.context();
                match compare_scenarios(
                    trace_id,
                    &prepared.baseline,
                    &[prepared.candidate],
                    &context,
                    self.runtime.provenance(),
                    &mut self.budget,
                ) {
                    Ok(mut execution) => {
                        execution.evidence.duration_ms =
                            started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                        self.success(
                            tool_name,
                            vec![context_evidence, serialize_evidence(execution.evidence)],
                        )
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            INSPECT_EQUIPMENT_WORKSPACE => {
                if serde_json::from_value::<EmptyArguments>(arguments.clone()).is_err() {
                    return invalid_arguments(tool_name);
                }
                let Some(workspace) = self.equipment_workspace.as_ref() else {
                    return failure(tool_name, "equipment_workspace_unavailable", "open the equipment page and capture its current build first", false);
                };
                let started = Instant::now();
                let result = inspect_workspace(
                    self.runtime,
                    workspace,
                    &self.scenario.simulation.talents,
                );
                match EvidenceEnvelopeV1::new(trace_id, tool_name, &self.scenario.scenario_hash, arguments, result, self.runtime.provenance(), started.elapsed().as_millis() as u64) {
                    Ok(evidence) => self.success(tool_name, vec![serialize_evidence(evidence)]),
                    Err(_) => failure(tool_name, "equipment_evidence_failed", "equipment evidence could not be created", false),
                }
            }
            COMPARE_FOCUSED_EQUIPMENT => {
                if serde_json::from_value::<EmptyArguments>(arguments).is_err() {
                    return invalid_arguments(tool_name);
                }
                let Some(workspace) = self.equipment_workspace.as_ref() else {
                    return failure(tool_name, "equipment_workspace_unavailable", "open the equipment page and focus a candidate first", false);
                };
                match compare_focus(trace_id, self.scenario, self.runtime, workspace, &mut self.budget) {
                    Ok((evidence, presentation)) => {
                        self.equipment_comparisons.push(presentation);
                        self.success(tool_name, vec![serialize_evidence(evidence)])
                    }
                    Err(error) => tool_failure(tool_name, error),
                }
            }
            SEARCH_EQUIPMENT_CATALOG => {
                let args = match serde_json::from_value::<EquipmentCatalogQueryV1>(arguments.clone()) {
                    Ok(args) if !args.query.trim().is_empty() && args.query.chars().count() <= 80 => args,
                    _ => return invalid_arguments(tool_name),
                };
                let started = Instant::now();
                let result = search_catalog(self.runtime, &args);
                match EvidenceEnvelopeV1::new(trace_id, tool_name, &self.scenario.scenario_hash, arguments, result, self.runtime.provenance(), started.elapsed().as_millis() as u64) {
                    Ok(evidence) => self.success(tool_name, vec![serialize_evidence(evidence)]),
                    Err(_) => failure(tool_name, "equipment_evidence_failed", "equipment evidence could not be created", false),
                }
            }
            COMPARE_EQUIPMENT_STRATEGIES => {
                if serde_json::from_value::<EmptyArguments>(arguments).is_err() {
                    return invalid_arguments(tool_name);
                }
                let Some(workspace) = self.equipment_workspace.as_ref() else {
                    return failure(tool_name, "equipment_workspace_unavailable", "open the equipment page and capture its current build first", false);
                };
                match compare_strategies(trace_id, self.scenario, self.runtime, workspace, &mut self.budget) {
                    Ok((evidence, presentation)) => {
                        self.equipment_comparisons.push(presentation);
                        self.success(tool_name, vec![serialize_evidence(evidence)])
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
        let sequence = if let Some(splices) = patch.sequence_splices.as_deref() {
            Some(apply_sequence_splices(
                &self.scenario.simulation.sequence,
                splices,
            )?)
        } else if let Some(edits) = patch.sequence_edits.as_deref() {
            Some(apply_sequence_edits(&self.scenario.simulation.sequence, edits)?)
        } else {
            patch.sequence.clone()
        };
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
                sequence,
                network_delay: patch.network_delay,
                initial_rage: patch.initial_rage.map(PatchValueV1::Set),
                attributes,
                target,
                macro_text: patch.macro_text.map(PatchValueV1::Set),
                talents: patch.talents,
                recipes: patch.recipes,
                equipment: patch.equipment,
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
    let sequence_patch_kinds = usize::from(patch.sequence.is_some())
        + usize::from(patch.sequence_edits.is_some())
        + usize::from(patch.sequence_splices.is_some());
    if sequence_patch_kinds > 1 {
        return Err("conflicting_sequence_patch");
    }
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
    if patch.sequence_edits.as_ref().is_some_and(|edits| {
        edits.is_empty()
            || edits.len() > 4
            || edits.iter().any(|edit| {
                let valid_skill = edit.skill_name.as_deref().is_some_and(|skill| {
                    !skill.trim().is_empty()
                        && skill.chars().count() <= 128
                        && !skill.chars().any(char::is_control)
                });
                edit.line_number == 0
                    || match edit.op {
                        AgentSequenceEditOperationV1::Remove => edit.skill_name.is_some(),
                        _ => !valid_skill,
                    }
            })
    }) {
        return Err("invalid_sequence_edits");
    }
    if patch.sequence_splices.as_ref().is_some_and(|splices| {
        splices.is_empty()
            || splices.len() > 2
            || splices.iter().any(|splice| {
                splice.start_line_number == 0
                    || splice.end_line_number < splice.start_line_number
                    || splice.replacement.len() > 16
                    || splice.replacement.iter().any(|skill| {
                        skill.trim().is_empty()
                            || skill.chars().count() > 128
                            || skill.chars().any(char::is_control)
                    })
            })
    }) {
        return Err("invalid_sequence_splices");
    }
    if let Some(macro_text) = patch.macro_text.as_deref() {
        if macro_text.trim().is_empty()
            || macro_text.chars().count() > 16_384
            || macro_text
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
            || parse_macro_text(macro_text).is_err()
        {
            return Err("invalid_macro_text");
        }
    }
    for selection in [&patch.talents, &patch.recipes] {
        if selection.as_ref().is_some_and(|ids| {
            ids.len() > 128
                || ids.contains(&0)
                || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        }) {
            return Err("invalid_build_selection");
        }
    }
    if patch.equipment.as_ref().is_some_and(|equipment| {
        equipment.len() > 32
            || equipment.iter().any(|(slot, id)| {
                slot.trim().is_empty()
                    || slot.chars().count() > 64
                    || slot.chars().any(char::is_control)
                    || *id == 0
            })
    }) {
        return Err("invalid_equipment");
    }
    Ok(())
}

fn apply_sequence_edits(
    baseline: &[String],
    edits: &[AgentSequenceEditV1],
) -> Result<Vec<String>, &'static str> {
    if baseline.is_empty() {
        return Err("missing_sequence");
    }
    let mut seen = std::collections::HashSet::new();
    if edits.iter().any(|edit| {
        edit.line_number > baseline.len() || !seen.insert(edit.line_number)
    }) {
        return Err("invalid_sequence_edit_line");
    }
    let mut ordered = edits.to_vec();
    ordered.sort_by(|left, right| right.line_number.cmp(&left.line_number));
    let mut sequence = baseline.to_vec();
    for edit in ordered {
        let index = edit.line_number - 1;
        match edit.op {
            AgentSequenceEditOperationV1::InsertBefore => {
                sequence.insert(index, edit.skill_name.ok_or("missing_sequence_edit_skill")?);
            }
            AgentSequenceEditOperationV1::InsertAfter => {
                sequence.insert(
                    index + 1,
                    edit.skill_name.ok_or("missing_sequence_edit_skill")?,
                );
            }
            AgentSequenceEditOperationV1::Replace => {
                sequence[index] = edit.skill_name.ok_or("missing_sequence_edit_skill")?;
            }
            AgentSequenceEditOperationV1::Remove => {
                sequence.remove(index);
            }
        }
    }
    Ok(sequence)
}

fn apply_sequence_splices(
    baseline: &[String],
    splices: &[AgentSequenceSpliceV1],
) -> Result<Vec<String>, &'static str> {
    if baseline.is_empty() {
        return Err("missing_sequence");
    }
    let mut ordered = splices.to_vec();
    ordered.sort_by(|left, right| right.start_line_number.cmp(&left.start_line_number));
    let mut previous_start = baseline.len() + 1;
    for splice in &ordered {
        if splice.end_line_number > baseline.len() || splice.end_line_number >= previous_start {
            return Err("invalid_sequence_splice_range");
        }
        previous_start = splice.start_line_number;
    }
    let mut sequence = baseline.to_vec();
    for splice in ordered {
        let start = splice.start_line_number - 1;
        let end_exclusive = splice.end_line_number;
        sequence.splice(start..end_exclusive, splice.replacement);
    }
    Ok(sequence)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArguments {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectRotationArguments {
    query: Option<String>,
    start_index: usize,
    limit: usize,
    context_radius: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareArguments {
    candidates: Vec<AgentCandidateV1>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListSavedArguments {
    query: String,
    kinds: Vec<SavedArtifactKind>,
}

impl Default for ListSavedArguments {
    fn default() -> Self {
        Self {
            query: String::new(),
            kinds: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadSavedArguments {
    artifact_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareSavedArguments {
    left_id: String,
    right_id: String,
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
        let season = self.season.and_then(|season| {
            let season = season.trim().to_string();
            (!season.is_empty()).then_some(season)
        });
        let version_scope = match self.version_scope.as_str() {
            // Providers sometimes repeat the current season even though the scope already
            // determines it. Treat that field as redundant instead of aborting a valid search;
            // the index still enforces every real version boundary from the query and scope.
            "current_only" => KnowledgeVersionScope::CurrentOnly,
            "specific_season" => KnowledgeVersionScope::SpecificSeason {
                season: season.ok_or(KnowledgeIndexError::InvalidQuery(
                    "specific season is required",
                ))?,
            },
            "cross_version" => KnowledgeVersionScope::CrossVersion,
            "reference_lookup" => KnowledgeVersionScope::ReferenceLookup,
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
        "请检索",
        "检索",
        "相关人物",
        "相关",
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
        ToolError::EquipmentFocusUnavailable => (
            "equipment_focus_unavailable",
            "select a different candidate from the equipment list before requesting a swap comparison",
            false,
        ),
        ToolError::EquipmentStrategyUnavailable => (
            "equipment_strategy_unavailable",
            "the local catalog does not contain enough compatible set and qiegao pieces for an automatic four-piece comparison",
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

fn saved_failure(tool_name: &str, error: SavedArtifactError) -> ToolDispatchOutcome {
    failure(tool_name, error.code(), error.message(), false)
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

fn ask_user_question_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["question", "reason"],
        "properties": {
            "question": {"type": "string", "minLength": 1, "maxLength": 500},
            "reason": {"type": "string", "minLength": 1, "maxLength": 500},
            "answer_hint": {"type": ["string", "null"], "maxLength": 240}
        }
    })
}

fn inspect_rotation_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": ["string", "null"], "minLength": 1, "maxLength": 64},
            "start_index": {"type": "integer", "minimum": 0},
            "limit": {"type": "integer", "minimum": 1, "maximum": 32},
            "context_radius": {"type": "integer", "minimum": 0, "maximum": 8}
        },
        "required": ["query", "start_index", "limit", "context_radius"],
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
                                "sequence_edits": {
                                    "type": ["array", "null"],
                                    "minItems": 1,
                                    "maxItems": 4,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "op": {"type": "string", "enum": ["insert_before", "insert_after", "replace", "remove"]},
                                            "line_number": {"type": "integer", "minimum": 1},
                                            "skill_name": {"type": ["string", "null"], "minLength": 1, "maxLength": 128}
                                        },
                                        "required": ["op", "line_number", "skill_name"],
                                        "additionalProperties": false
                                    }
                                },
                                "sequence_splices": {
                                    "type": ["array", "null"],
                                    "minItems": 1,
                                    "maxItems": 2,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "start_line_number": {"type": "integer", "minimum": 1},
                                            "end_line_number": {"type": "integer", "minimum": 1},
                                            "replacement": {"type": "array", "maxItems": 16, "items": {"type": "string", "minLength": 1, "maxLength": 128}}
                                        },
                                        "required": ["start_line_number", "end_line_number", "replacement"],
                                        "additionalProperties": false
                                    }
                                },
                                "network_delay": {"type": ["integer", "null"], "minimum": 0, "maximum": 5000},
                                "initial_rage": {"type": ["integer", "null"], "minimum": -1000, "maximum": 1000},
                                "base_attack": {"type": ["number", "null"], "minimum": 0, "maximum": 1000000000},
                                "target_defense_bonus": {"type": ["number", "null"], "minimum": -100, "maximum": 1000}
                                ,"macro_text": {"type": ["string", "null"], "minLength": 1, "maxLength": 16384}
                                ,"talents": {"type": ["array", "null"], "maxItems": 128, "items": {"type": "integer", "minimum": 1}}
                                ,"recipes": {"type": ["array", "null"], "maxItems": 128, "items": {"type": "integer", "minimum": 1}}
                                ,"equipment": {"type": ["object", "null"], "maxProperties": 32, "additionalProperties": {"type": "integer", "minimum": 1}}
                            },
                            "required": [],
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

fn list_saved_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "maxLength": 128},
            "kinds": {
                "type": "array",
                "maxItems": 5,
                "uniqueItems": true,
                "items": {"type": "string", "enum": ["macro", "loop", "equipment", "attributes", "plaza"]}
            }
        },
        "required": ["query", "kinds"],
        "additionalProperties": false
    })
}

fn read_saved_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "artifact_id": {"type": "string", "pattern": "^sa_[0-9a-f]{64}$"}
        },
        "required": ["artifact_id"],
        "additionalProperties": false
    })
}

fn compare_saved_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "left_id": {"type": "string", "pattern": "^sa_[0-9a-f]{64}$"},
            "right_id": {"type": "string", "pattern": "^sa_[0-9a-f]{64}$"}
        },
        "required": ["left_id", "right_id"],
        "additionalProperties": false
    })
}

fn equipment_search_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type":"string","minLength":1,"maxLength":80},
            "position": {"type":["string","null"],"enum":[null,"HAT","JACKET","BELT","WRIST","BOTTOMS","SHOES","NECKLACE","PENDANT","RING_1","RING_2","PRIMARY_WEAPON","SECONDARY_WEAPON"]}
        },
        "required": ["query", "position"],
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
    fn only_allow_listed_read_only_tools_are_exposed() {
        let definitions = AgentToolRegistry::definitions();
        let names = definitions
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "get_current_scenario",
                "ask_user_question",
                "inspect_rotation_input",
                "simulate_scenario",
                "compare_scenarios",
                "analyze_timeline",
                "list_saved_artifacts",
                "read_saved_artifact",
                "compare_saved_macros",
                "compare_saved_scenarios",
                "inspect_equipment_workspace",
                "compare_focused_equipment",
                "search_equipment_catalog",
                "compare_equipment_strategies"
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
        assert_eq!(definitions.len(), 15);
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
        let redundant_season = KnowledgeArguments {
            query: "盾飞".to_string(),
            version_scope: "current_only".to_string(),
            season: Some("山海源流（2025）".to_string()),
            category: None,
        }
        .into_query()
        .unwrap();
        assert!(matches!(
            redundant_season.version_scope,
            KnowledgeVersionScope::CurrentOnly
        ));

        let invalid = KnowledgeArguments {
            query: "盾飞".to_string(),
            version_scope: "unbounded".to_string(),
            season: None,
            category: None,
        };
        assert!(matches!(
            invalid.into_query(),
            Err(KnowledgeIndexError::InvalidQuery(_))
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
        for _ in 2..MAX_KNOWLEDGE_SEARCHES {
            let next = registry.dispatch(
                "knowledge-run",
                "search_knowledge_base",
                arguments.clone(),
            );
            assert_eq!(next.output["ok"], true);
        }
        let exhausted = registry.dispatch("knowledge-run", "search_knowledge_base", arguments);
        assert_eq!(
            exhausted.output["error"]["code"],
            "knowledge_search_budget_exhausted"
        );
        assert!(exhausted.budget_exhausted);
        assert_eq!(registry.used_knowledge_searches(), MAX_KNOWLEDGE_SEARCHES);

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

        let conflicting = AgentScenarioPatchV1 {
            sequence: Some(vec!["盾击".to_string()]),
            sequence_edits: Some(vec![AgentSequenceEditV1 {
                op: AgentSequenceEditOperationV1::Replace,
                line_number: 1,
                skill_name: Some("盾压".to_string()),
            }]),
            ..AgentScenarioPatchV1::default()
        };
        assert_eq!(
            validate_agent_patch(&conflicting),
            Err("conflicting_sequence_patch")
        );
    }

    #[test]
    fn one_based_sequence_edits_build_a_complete_candidate_server_side() {
        let baseline = vec!["斩刀".to_string(), "绝刀".to_string(), "盾回".to_string()];
        let edited = apply_sequence_edits(
            &baseline,
            &[AgentSequenceEditV1 {
                op: AgentSequenceEditOperationV1::InsertBefore,
                line_number: 3,
                skill_name: Some("苍雪刀".to_string()),
            }],
        )
        .unwrap();
        assert_eq!(edited, vec!["斩刀", "绝刀", "苍雪刀", "盾回"]);

        let replaced = apply_sequence_edits(
            &baseline,
            &[AgentSequenceEditV1 {
                op: AgentSequenceEditOperationV1::Replace,
                line_number: 2,
                skill_name: Some("苍雪刀".to_string()),
            }],
        )
        .unwrap();
        assert_eq!(replaced, vec!["斩刀", "苍雪刀", "盾回"]);

        let removed = apply_sequence_edits(
            &baseline,
            &[AgentSequenceEditV1 {
                op: AgentSequenceEditOperationV1::Remove,
                line_number: 2,
                skill_name: None,
            }],
        )
        .unwrap();
        assert_eq!(removed, vec!["斩刀", "盾回"]);

        let spliced = apply_sequence_splices(
            &["业火", "盾击", "盾击", "盾飞", "血怒", "绝刀"].map(str::to_string),
            &[AgentSequenceSpliceV1 {
                start_line_number: 2,
                end_line_number: 5,
                replacement: vec!["盾飞", "斩刀", "绝刀", "绝刀"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            }],
        )
        .unwrap();
        assert_eq!(spliced, vec!["业火", "盾飞", "斩刀", "绝刀", "绝刀", "绝刀"]);
    }

    #[test]
    fn macro_and_build_candidates_are_locally_typed_and_validated() {
        let patch = AgentScenarioPatchV1 {
            macro_text: Some("/cast [skill_energy:血怒>1] 血怒".to_string()),
            talents: Some(vec![1001, 1002]),
            recipes: Some(vec![2001]),
            equipment: Some(std::collections::HashMap::from([(
                "PRIMARY_WEAPON".to_string(),
                3001,
            )])),
            ..AgentScenarioPatchV1::default()
        };
        assert_eq!(validate_agent_patch(&patch), Ok(()));

        let mut invalid = patch;
        invalid.macro_text = Some("/cast [skill_energy:血怒>] 血怒".to_string());
        assert_eq!(validate_agent_patch(&invalid), Err("invalid_macro_text"));
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
