use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::{
    simulate_core, FormationEntry, GameVersion, Mount, MountConstants, RecipeEntry,
    SimulateResponse, SkillSpec, TeamBuffEntry,
};

use super::evidence::{validate_trace_id, EvidenceEnvelopeV1, EvidenceError, ToolProvenance};
use super::schema::{game_version_id, mount_id, ScenarioError, ScenarioSnapshotV1};

pub const GET_CURRENT_SCENARIO: &str = "get_current_scenario";
pub const INSPECT_ROTATION_INPUT: &str = "inspect_rotation_input";
pub const SIMULATE_SCENARIO: &str = "simulate_scenario";

pub struct SimulatorContext<'a> {
    pub game_version: GameVersion,
    pub mount: Mount,
    pub constants: MountConstants,
    pub skills: &'a [SkillSpec],
    pub talents: &'a [crate::TalentEntry],
    pub recipes: &'a [RecipeEntry],
    pub team_buffs: &'a [TeamBuffEntry],
    pub formations: &'a [FormationEntry],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolBudget {
    pub max_simulations: u32,
    pub used_simulations: u32,
}

impl ToolBudget {
    pub fn new(max_simulations: u32) -> Self {
        Self {
            max_simulations,
            used_simulations: 0,
        }
    }

    pub fn remaining_simulations(&self) -> u32 {
        self.max_simulations.saturating_sub(self.used_simulations)
    }

    pub(super) fn reserve_simulations(&mut self, count: u32) -> Result<(), ToolError> {
        if count > self.remaining_simulations() {
            return Err(ToolError::BudgetExceeded {
                resource: "simulations",
                limit: self.max_simulations,
            });
        }
        self.used_simulations += count;
        Ok(())
    }
}

#[derive(Debug)]
pub enum ToolError {
    Scenario(ScenarioError),
    Evidence(EvidenceError),
    RuntimeMismatch {
        expected_version: String,
        actual_version: String,
        expected_mount: String,
        actual_mount: String,
    },
    BudgetExceeded {
        resource: &'static str,
        limit: u32,
    },
    InvalidCandidateCount {
        count: usize,
        max: usize,
    },
    InvalidCandidateLabel,
    DuplicateCandidateLabel {
        label: String,
    },
    NoScenarioChanges {
        label: String,
    },
    TimelineDetailsUnavailable,
    EquipmentFocusUnavailable,
    EquipmentStrategyUnavailable,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scenario(error) => write!(f, "{error}"),
            Self::Evidence(error) => write!(f, "{error}"),
            Self::RuntimeMismatch {
                expected_version,
                actual_version,
                expected_mount,
                actual_mount,
            } => write!(
                f,
                "scenario/runtime mismatch: expected {expected_version}/{expected_mount}, got {actual_version}/{actual_mount}"
            ),
            Self::BudgetExceeded { resource, limit } => {
                write!(f, "tool budget exceeded: {resource} limit is {limit}")
            }
            Self::InvalidCandidateCount { count, max } => {
                write!(f, "candidate count must be 1..={max}, got {count}")
            }
            Self::InvalidCandidateLabel => {
                write!(f, "candidate label must be 1..64 characters without controls")
            }
            Self::DuplicateCandidateLabel { label } => {
                write!(f, "duplicate candidate label: {label}")
            }
            Self::NoScenarioChanges { label } => {
                write!(f, "candidate '{label}' does not change the baseline scenario")
            }
            Self::TimelineDetailsUnavailable => {
                write!(f, "full timeline details are required for deterministic analysis")
            }
            Self::EquipmentFocusUnavailable => {
                write!(f, "equipment workspace or focused candidate is unavailable")
            }
            Self::EquipmentStrategyUnavailable => {
                write!(f, "four-piece set or qiegao candidates are unavailable")
            }
        }
    }
}

impl std::error::Error for ToolError {}

impl From<ScenarioError> for ToolError {
    fn from(value: ScenarioError) -> Self {
        Self::Scenario(value)
    }
}

impl From<EvidenceError> for ToolError {
    fn from(value: EvidenceError) -> Self {
        Self::Evidence(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSummary {
    pub game_version: String,
    pub mount: String,
    /// Exact simulator haste input. Do not infer a haste band from guide text.
    pub haste_level: u32,
    /// Frozen panel attributes used by the simulator, including equipment output.
    pub attributes: Option<serde_json::Value>,
    pub rotation_mode: String,
    pub sequence_entries: usize,
    pub macro_characters: usize,
    pub macro_duration: Option<f64>,
    pub talent_count: usize,
    pub recipe_count: usize,
    pub equipment_count: usize,
    /// Exact immutable build selections, so candidate experiments never invent the baseline.
    pub selected_talents: Vec<u32>,
    pub selected_recipes: Vec<u32>,
    pub selected_equipment: std::collections::HashMap<String, u32>,
    pub enabled_team_buff_count: usize,
    pub formation_key: Option<String>,
    pub pre_release_count: usize,
    pub target_level: u32,
    pub network_delay_ms: u32,
    pub boss_attack_interval: Option<f64>,
    pub rotation_input: RotationInputSummary,
    /// Versioned definitions for interpretation; observations remain in the
    /// simulator/timeline evidence. Optional for historical summary decoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanics_context: Option<super::mechanics::MechanicsContextV1>,
}

const MAX_ROTATION_INPUT_ITEMS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationInputSummary {
    pub mode: String,
    pub parse_status: String,
    pub parse_error: Option<String>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macro_semantics: Option<MacroSemanticsSummary>,
    pub macro_statements: Vec<MacroStatementSummary>,
    pub manual_operations: Vec<ManualOperationSummary>,
    #[serde(default)]
    pub total_items: usize,
    #[serde(default)]
    pub returned_item_count: usize,
    #[serde(default)]
    pub skill_semantics: BTreeMap<String, SkillExecutionSemanticsV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MacroSemanticsSummary {
    /// Stable product-facing classification. A page without a stance filter is
    /// a general one-page macro, not a "single-stance" macro.
    pub structure: String,
    pub page_count: usize,
    pub operator_precedence: String,
    pub associativity: String,
    pub line_selection: String,
    pub absent_bufftime_result: bool,
    pub stance_pages_present: bool,
    pub page_selection: String,
    pub page_selection_is_automatic: bool,
    pub pause_stance_rule: String,
    pub dun_fei_stance_rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MacroStatementSummary {
    pub source_line: usize,
    pub page: usize,
    pub stance: Option<String>,
    pub command: String,
    pub skill_name: String,
    pub condition: Option<String>,
    /// Exact parser-produced tree. This, not the flat source string, is the
    /// authoritative condition grouping for Agent reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_ast: Option<serde_json::Value>,
    /// Fully parenthesized, human-readable projection of `condition_ast`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_semantics: Option<String>,
    pub statement: String,
}

fn macro_condition_ast(condition: &crate::macro_engine::MacroCondition) -> serde_json::Value {
    use crate::macro_engine::MacroCondition;
    match condition {
        MacroCondition::Rage(op, value) => serde_json::json!({
            "kind": "rage", "operator": op.symbol(), "value": value
        }),
        MacroCondition::Life(op, value) => serde_json::json!({
            "kind": "life", "operator": op.symbol(), "value": value
        }),
        MacroCondition::Buff(name) => serde_json::json!({
            "kind": "buff", "name": name,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::NoBuff(name) => serde_json::json!({
            "kind": "no_buff", "name": name,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::BuffTime(name, op, value) => serde_json::json!({
            "kind": "buff_time", "name": name, "operator": op.symbol(),
            "value_seconds": value,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::BuffStack(name, op, value) => serde_json::json!({
            "kind": "buff_stack", "name": name, "operator": op.symbol(), "value": value,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::TBuff(name) => serde_json::json!({
            "kind": "target_buff", "name": name,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::TnoBuff(name) => serde_json::json!({
            "kind": "target_no_buff", "name": name,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::TBuffTime(name, op, value) => serde_json::json!({
            "kind": "target_buff_time", "name": name, "operator": op.symbol(),
            "value_seconds": value,
            "resolved_buff_id": crate::macro_eval::buff_name_to_id(name)
        }),
        MacroCondition::SkillNotInCd(name) => {
            serde_json::json!({"kind": "skill_not_in_cd", "skill_name": name})
        }
        MacroCondition::SkillExists(id) => {
            serde_json::json!({"kind": "skill_exists", "skill_id": id})
        }
        MacroCondition::SkillNotExists(id) => {
            serde_json::json!({"kind": "skill_not_exists", "skill_id": id})
        }
        MacroCondition::SkillEnergy(name, op, value) => serde_json::json!({
            "kind": "skill_charge_count", "skill_name": name,
            "operator": op.symbol(), "value": value
        }),
        MacroCondition::LastSkill(name) => serde_json::json!({"kind": "last_skill", "name": name}),
        MacroCondition::LastSkillNot(name) => {
            serde_json::json!({"kind": "last_skill_not", "name": name})
        }
        MacroCondition::NearbyEnemy(op, value) => serde_json::json!({
            "kind": "nearby_enemy_count", "operator": op.symbol(), "value": value
        }),
        MacroCondition::And(left, right) => serde_json::json!({
            "kind": "and",
            "left": macro_condition_ast(left),
            "right": macro_condition_ast(right)
        }),
        MacroCondition::Or(left, right) => serde_json::json!({
            "kind": "or",
            "left": macro_condition_ast(left),
            "right": macro_condition_ast(right)
        }),
    }
}

pub type ManualOperationSummary = RotationInputEntryV1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillCooldownSemanticsV1 {
    pub skill_id: u32,
    pub cooldown_id: String,
    pub kind: String,
    pub mode: crate::CdMode,
    pub base_duration_seconds: f64,
    pub haste_scaled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillExecutionSemanticsV1 {
    pub basis: String,
    pub skill_ids: Vec<u32>,
    /// None means unresolved or rank-dependent. This follows the simulator's
    /// own main-GCD predicate, including auxiliary GCD skills.
    pub is_main_gcd: Option<bool>,
    pub cooldown_semantics: String,
    pub cooldowns: Vec<SkillCooldownSemanticsV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationInputEntryV1 {
    /// Stable identity for this exact occurrence. A skill name alone is not an
    /// anchor because the same skill may appear many times in one sequence.
    pub anchor_id: String,
    /// Stable zero-based index for typed tool patches.
    pub sequence_index: usize,
    /// Human-facing one-based operation number. This is never a visual row.
    pub operation_number: usize,
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_ticks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing_offset_seconds: Option<f64>,
    /// Positive delay from this operation's earliest legal cast time. Dynamic
    /// follow-GCD-end scheduling has no fixed delay and serializes as null.
    #[serde(default)]
    pub delay_seconds: Option<f64>,
    #[serde(default)]
    pub timing_mode: String,
    /// Original scenario encoding for exact replay, including negative sentinels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_timing_offset: Option<f64>,
    #[serde(default)]
    pub is_main_gcd: Option<bool>,
    #[serde(default)]
    pub cooldown_semantics: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationInputMatchV1 {
    pub matched: RotationInputEntryV1,
    pub before: Vec<RotationInputEntryV1>,
    pub after: Vec<RotationInputEntryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationInputInspectionV1 {
    pub mode: String,
    pub total_items: usize,
    pub query: Option<String>,
    pub matches: Vec<RotationInputMatchV1>,
    pub window: Vec<RotationInputEntryV1>,
    pub next_start_index: Option<usize>,
    /// Query matches in the entire input sequence, before pagination.
    #[serde(default)]
    pub total_matches: Option<usize>,
    #[serde(default)]
    pub returned_match_count: usize,
    #[serde(default)]
    pub returned_window_count: usize,
    #[serde(default)]
    pub page_start_index: usize,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub skill_semantics: BTreeMap<String, SkillExecutionSemanticsV1>,
}

fn skill_execution_semantics(name: &str, skills: &[SkillSpec]) -> SkillExecutionSemanticsV1 {
    let mut matched = skills.iter().filter(|skill| !skill.passive && skill.name == name)
        .collect::<Vec<_>>();
    if matched.is_empty() {
        matched = skills.iter().filter(|skill| {
            !skill.passive && skill.name.split('·').next() == Some(name)
        }).collect();
    }
    let is_main_gcd = matched.first().and_then(|first| {
        let expected = crate::skill_is_main(first);
        matched.iter().all(|skill| crate::skill_is_main(skill) == expected).then_some(expected)
    });
    let cooldown_semantics = match is_main_gcd {
        Some(true) => "main_gcd",
        Some(false) if matched.iter().any(|skill| skill.cooldowns.iter().any(|cd| {
            cd.cd_id.starts_with("gcd_") && cd.mode == crate::CdMode::CheckAndTrigger
        })) => "auxiliary_gcd",
        Some(false) => "independent_of_main_gcd",
        None if matched.is_empty() => "unresolved_skill",
        None => "rank_dependent",
    };
    SkillExecutionSemanticsV1 {
        basis: "runtime_skill_definition_base_values".to_string(),
        skill_ids: matched.iter().map(|skill| skill.skill_id).collect(),
        is_main_gcd,
        cooldown_semantics: cooldown_semantics.to_string(),
        cooldowns: matched.iter().flat_map(|skill| skill.cooldowns.iter().map(|cd| {
            SkillCooldownSemanticsV1 {
                skill_id: skill.skill_id,
                cooldown_id: cd.cd_id.clone(),
                kind: if cd.cd_id.starts_with("gcd_") { "shared_gcd" }
                    else if cd.cd_id.starts_with("protect_") { "skill_protection" }
                    else { "skill_cooldown" }.to_string(),
                mode: cd.mode,
                base_duration_seconds: cd.duration,
                haste_scaled: cd.haste,
            }
        })).collect(),
    }
}

fn input_skill_semantics<'a>(
    names: impl IntoIterator<Item = &'a str>,
    skills: &[SkillSpec],
) -> BTreeMap<String, SkillExecutionSemanticsV1> {
    names.into_iter().collect::<std::collections::BTreeSet<_>>().into_iter()
        .map(|name| (name.to_string(), skill_execution_semantics(name, skills))).collect()
}

fn manual_rotation_entry(
    simulation: &crate::SimulateRequest,
    sequence_index: usize,
    skill_semantics: &BTreeMap<String, SkillExecutionSemanticsV1>,
) -> RotationInputEntryV1 {
    let raw_timing_offset = simulation.timing_offsets.get(&sequence_index.to_string()).copied();
    let timing_mode = match raw_timing_offset {
        Some(value) if value < 0.0 => "follow_gcd_end",
        Some(value) if value > 0.0 => "delay_from_earliest_cast",
        _ => "as_soon_as_available",
    };
    let delay_seconds = raw_timing_offset.filter(|value| *value >= 0.0);
    let semantics = skill_semantics.get(&simulation.sequence[sequence_index]);
    RotationInputEntryV1 {
        anchor_id: format!("sequence:{sequence_index}"),
        sequence_index,
        operation_number: sequence_index + 1,
        skill_name: simulation.sequence[sequence_index].clone(),
        channel_ticks: simulation
            .channel_ticks
            .get(&sequence_index.to_string())
            .copied(),
        timing_offset_seconds: delay_seconds,
        delay_seconds,
        timing_mode: timing_mode.to_string(),
        raw_timing_offset,
        is_main_gcd: semantics.and_then(|value| value.is_main_gcd),
        cooldown_semantics: semantics.map(|value| value.cooldown_semantics.clone())
            .unwrap_or_else(|| "unresolved_skill".to_string()),
    }
}

/// Read a bounded window or search every operation in the immutable manual
/// sequence. This avoids forcing the model to ingest or reproduce a long loop.
pub fn inspect_rotation_input(
    trace_id: &str,
    snapshot: &ScenarioSnapshotV1,
    query: Option<&str>,
    start_index: usize,
    limit: usize,
    context_radius: usize,
    context: &SimulatorContext<'_>,
    provenance: &ToolProvenance,
) -> Result<EvidenceEnvelopeV1<RotationInputInspectionV1>, ToolError> {
    let started = Instant::now();
    validate_trace_id(trace_id)?;
    snapshot.verify_hash()?;
    verify_runtime(snapshot, context)?;
    let simulation = &snapshot.simulation;
    let total_items = simulation.sequence.len();
    let mut skill_semantics = input_skill_semantics(simulation.sequence.iter().map(String::as_str), context.skills);
    let normalized_query = query.map(str::trim).filter(|value| !value.is_empty());
    let mut matches = Vec::new();
    let mut query_next_start_index = None;
    let total_matches = normalized_query.map(|query| {
        let query = query.to_lowercase();
        simulation.sequence.iter().filter(|name| name.to_lowercase().contains(&query)).count()
    });
    if let Some(query) = normalized_query {
        let query = query.to_lowercase();
        let match_limit = limit.min(8);
        for sequence_index in start_index.min(total_items)..total_items {
            if !simulation.sequence[sequence_index]
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            if matches.len() >= match_limit {
                query_next_start_index = Some(sequence_index);
                break;
            }
            let before_start = sequence_index.saturating_sub(context_radius);
            let after_end = (sequence_index + context_radius + 1).min(total_items);
            matches.push(RotationInputMatchV1 {
                matched: manual_rotation_entry(simulation, sequence_index, &skill_semantics),
                before: (before_start..sequence_index)
                    .map(|index| manual_rotation_entry(simulation, index, &skill_semantics))
                    .collect(),
                after: ((sequence_index + 1)..after_end)
                    .map(|index| manual_rotation_entry(simulation, index, &skill_semantics))
                    .collect(),
            });
        }
    }
    let window = if normalized_query.is_none() {
        let end = start_index.saturating_add(limit).min(total_items);
        (start_index.min(total_items)..end)
            .map(|index| manual_rotation_entry(simulation, index, &skill_semantics))
            .collect()
    } else {
        Vec::new()
    };
    let next_start_index = if normalized_query.is_some() {
        query_next_start_index
    } else if start_index.saturating_add(limit) < total_items {
        Some(start_index + limit)
    } else {
        None
    };
    let names = matches.iter().flat_map(|item| {
        std::iter::once(&item.matched).chain(item.before.iter()).chain(item.after.iter())
    }).chain(window.iter()).map(|item| item.skill_name.as_str()).collect::<std::collections::BTreeSet<_>>();
    skill_semantics.retain(|name, _| names.contains(name.as_str()));
    let result = RotationInputInspectionV1 {
        mode: if simulation
            .macro_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            "macro_with_generated_sequence".to_string()
        } else {
            "manual_sequence".to_string()
        },
        total_items,
        total_matches,
        returned_match_count: matches.len(),
        returned_window_count: window.len(),
        page_start_index: start_index.min(total_items),
        has_more: next_start_index.is_some(),
        skill_semantics,
        query: normalized_query.map(str::to_string),
        matches,
        window,
        next_start_index,
    };
    Ok(EvidenceEnvelopeV1::new(
        trace_id,
        INSPECT_ROTATION_INPUT,
        &snapshot.scenario_hash,
        serde_json::json!({
            "query": normalized_query,
            "start_index": start_index,
            "limit": limit,
            "context_radius": context_radius,
        }),
        result,
        provenance,
        elapsed_ms(started),
    )?)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillDamageSummary {
    pub skill_id: u32,
    pub name: String,
    pub triggered: bool,
    pub event_count: u32,
    pub total_damage: f64,
    pub damage_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SimulationSummary {
    pub dps: f64,
    pub total_damage: f64,
    pub fight_time: f64,
    pub skill_count: usize,
    pub fingerprint: u64,
    pub fingerprint_hex: String,
    pub skills: Vec<SkillDamageSummary>,
}

#[derive(Clone)]
pub struct SimulationExecution {
    pub evidence: EvidenceEnvelopeV1<SimulationSummary>,
    pub response: SimulateResponse,
}

pub fn get_current_scenario(
    trace_id: &str,
    snapshot: &ScenarioSnapshotV1,
    context: &SimulatorContext<'_>,
    provenance: &ToolProvenance,
) -> Result<EvidenceEnvelopeV1<ScenarioSummary>, ToolError> {
    let started = Instant::now();
    snapshot.verify_hash()?;
    verify_runtime(snapshot, context)?;
    let simulation = &snapshot.simulation;
    let target = simulation
        .target
        .as_ref()
        .ok_or(ScenarioError::MissingField("simulation.target"))?;
    let rotation_input = summarize_rotation_input(simulation, context.skills);
    let result = ScenarioSummary {
        game_version: snapshot.game_version.clone(),
        mount: snapshot.mount.clone(),
        haste_level: simulation.haste_level,
        attributes: simulation
            .attributes
            .as_ref()
            .and_then(|attributes| serde_json::to_value(attributes).ok()),
        rotation_mode: if simulation
            .macro_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            "macro".to_string()
        } else {
            "sequence".to_string()
        },
        sequence_entries: simulation.sequence.len(),
        macro_characters: simulation
            .macro_text
            .as_deref()
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or(0),
        macro_duration: simulation.macro_duration,
        talent_count: simulation.talents.len(),
        recipe_count: simulation.recipes.len(),
        equipment_count: simulation.equipment.len(),
        selected_talents: simulation.talents.clone(),
        selected_recipes: simulation.recipes.clone(),
        selected_equipment: simulation.equipment.clone(),
        enabled_team_buff_count: simulation
            .team_buffs
            .iter()
            .filter(|selection| selection.enabled)
            .count(),
        formation_key: simulation
            .formation
            .as_ref()
            .map(|formation| formation.key.clone()),
        pre_release_count: simulation.pre_releases.len(),
        target_level: target.level,
        network_delay_ms: simulation.network_delay,
        boss_attack_interval: simulation.boss_attack_interval,
        rotation_input,
        mechanics_context: Some(super::mechanics::build_mechanics_context(snapshot, context)),
    };

    Ok(EvidenceEnvelopeV1::new(
        trace_id,
        GET_CURRENT_SCENARIO,
        &snapshot.scenario_hash,
        serde_json::json!({}),
        result,
        provenance,
        elapsed_ms(started),
    )?)
}

fn summarize_rotation_input(simulation: &crate::SimulateRequest, skills: &[SkillSpec]) -> RotationInputSummary {
    let Some(macro_text) = simulation
        .macro_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    else {
        let skill_semantics = input_skill_semantics(simulation.sequence.iter().take(MAX_ROTATION_INPUT_ITEMS).map(String::as_str), skills);
        let operations: Vec<_> = simulation
            .sequence
            .iter()
            .take(MAX_ROTATION_INPUT_ITEMS)
            .enumerate()
            .map(|(sequence_index, _)| manual_rotation_entry(simulation, sequence_index, &skill_semantics))
            .collect();
        return RotationInputSummary {
            mode: "manual_sequence".to_string(),
            parse_status: "not_applicable".to_string(),
            parse_error: None,
            truncated: simulation.sequence.len() > MAX_ROTATION_INPUT_ITEMS,
            macro_semantics: None,
            macro_statements: Vec::new(),
            total_items: simulation.sequence.len(),
            returned_item_count: operations.len(),
            skill_semantics,
            manual_operations: operations,
        };
    };

    let parsed = crate::macro_parser::parse_macro_text(macro_text);
    let (parse_status, parse_error) = match &parsed {
        Ok(_) => ("valid".to_string(), None),
        Err(error) => (
            "invalid".to_string(),
            Some(format!("line {}: {}", error.line + 1, error.message)),
        ),
    };
    let parsed_conditions = parsed.as_ref().ok().map(|config| {
        config
            .pages
            .iter()
            .flat_map(|page| page.lines.iter())
            .map(|line| {
                line.condition
                    .as_ref()
                    .map(|condition| (macro_condition_ast(condition), condition.semantic_string()))
            })
            .collect::<Vec<_>>()
    });
    let stance_pages_present = parsed
        .as_ref()
        .ok()
        .is_some_and(|config| config.pages.iter().any(|page| page.stance_filter.is_some()));
    let page_count = parsed
        .as_ref()
        .ok()
        .map(|config| config.pages.len())
        .unwrap_or(0);
    let macro_structure = if stance_pages_present {
        "stance_split_pages"
    } else {
        "general_single_page"
    };
    let mut parsed_statement_index = 0usize;
    let mut page = 0usize;
    let mut stance = None;
    let mut statements = Vec::new();
    let mut total_statements = 0usize;
    for (source_index, source) in macro_text.lines().enumerate() {
        let statement = source.trim();
        if statement.is_empty() || statement.starts_with("//") {
            continue;
        }
        if let Some(rest) = statement.strip_prefix("#page") {
            if total_statements > 0 {
                page += 1;
            }
            stance = match rest.trim() {
                "shield" | "擎盾" => Some("shield".to_string()),
                "blade" | "擎刀" => Some("blade".to_string()),
                "wall" | "盾墙" => Some("wall".to_string()),
                _ => None,
            };
            continue;
        }
        let Some((command, rest)) = statement
            .strip_prefix("/fcast")
            .map(|rest| ("fcast", rest.trim()))
            .or_else(|| {
                statement
                    .strip_prefix("/cast")
                    .map(|rest| ("cast", rest.trim()))
            })
        else {
            continue;
        };
        total_statements += 1;
        let parsed_condition = parsed_conditions
            .as_ref()
            .and_then(|conditions| conditions.get(parsed_statement_index))
            .cloned()
            .flatten();
        parsed_statement_index += 1;
        if statements.len() >= MAX_ROTATION_INPUT_ITEMS {
            continue;
        }
        let (condition, skill_name) = if let Some(rest) = rest.strip_prefix('[') {
            match rest.find(']') {
                Some(end) => (
                    Some(rest[..end].trim().to_string()),
                    rest[end + 1..].trim().to_string(),
                ),
                None => (None, rest.trim().to_string()),
            }
        } else {
            let skill_name = rest
                .split_whitespace()
                .last()
                .unwrap_or_default()
                .to_string();
            let condition = rest
                .strip_suffix(&skill_name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            (condition, skill_name)
        };
        statements.push(MacroStatementSummary {
            source_line: source_index + 1,
            page,
            stance: stance.clone(),
            command: command.to_string(),
            skill_name,
            condition,
            condition_ast: parsed_condition.as_ref().map(|value| value.0.clone()),
            condition_semantics: parsed_condition.map(|value| value.1),
            statement: statement.to_string(),
        });
    }
    let skill_semantics = input_skill_semantics(statements.iter().map(|statement| statement.skill_name.as_str()), skills);
    RotationInputSummary {
        mode: "macro".to_string(),
        parse_status,
        parse_error,
        truncated: total_statements > MAX_ROTATION_INPUT_ITEMS,
        total_items: total_statements,
        returned_item_count: statements.len(),
        skill_semantics,
        macro_semantics: Some(MacroSemanticsSummary {
            structure: macro_structure.to_string(),
            page_count,
            operator_precedence: "and_or_equal".to_string(),
            associativity: "right".to_string(),
            line_selection: "source_order_first_condition_true_and_castable".to_string(),
            absent_bufftime_result: false,
            stance_pages_present,
            page_selection: if stance_pages_present {
                "current_stance_page_in_source_order".to_string()
            } else {
                "single_unfiltered_page_used_in_all_stances".to_string()
            },
            page_selection_is_automatic: true,
            pause_stance_rule: "pausing_input_does_not_change_stance; resume_uses_the_page_for_the_stance_after_buff_time_has_advanced".to_string(),
            dun_fei_stance_rule: "after_the_shield_flight_delay_stance_is_blade; before_the_shield_flight_buff_expires_a_short_pause_resumes_on_the_blade_page_unless_shield_return_was_cast; natural_expiration_returns_shield".to_string(),
        }),
        macro_statements: statements,
        manual_operations: Vec::new(),
    }
}

pub fn simulate_scenario(
    trace_id: &str,
    snapshot: &ScenarioSnapshotV1,
    context: &SimulatorContext<'_>,
    provenance: &ToolProvenance,
    budget: &mut ToolBudget,
) -> Result<SimulationExecution, ToolError> {
    let started = Instant::now();
    snapshot.verify_hash()?;
    verify_runtime(snapshot, context)?;
    validate_trace_id(trace_id)?;
    budget.reserve_simulations(1)?;

    let response = run_simulation(snapshot, context);
    let summary = summarize_simulation(&response);
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        SIMULATE_SCENARIO,
        &snapshot.scenario_hash,
        serde_json::json!({"mode": "full"}),
        summary,
        provenance,
        elapsed_ms(started),
    )?;

    Ok(SimulationExecution { evidence, response })
}

pub(super) fn verify_runtime(
    snapshot: &ScenarioSnapshotV1,
    context: &SimulatorContext<'_>,
) -> Result<(), ToolError> {
    let actual_version = game_version_id(context.game_version).to_string();
    let actual_mount = mount_id(context.mount).to_string();
    if snapshot.game_version == actual_version && snapshot.mount == actual_mount {
        Ok(())
    } else {
        Err(ToolError::RuntimeMismatch {
            expected_version: snapshot.game_version.clone(),
            actual_version,
            expected_mount: snapshot.mount.clone(),
            actual_mount,
        })
    }
}

pub(super) fn run_simulation(
    snapshot: &ScenarioSnapshotV1,
    context: &SimulatorContext<'_>,
) -> SimulateResponse {
    simulate_core(
        &snapshot.simulation,
        context.skills,
        context.game_version,
        context.mount,
        context.constants,
        context.recipes,
        context.team_buffs,
        context.formations,
    )
}

pub(super) fn summarize_simulation(response: &SimulateResponse) -> SimulationSummary {
    let mut grouped: BTreeMap<(u32, String, bool), (u32, f64)> = BTreeMap::new();
    for event in &response.timeline {
        let entry = grouped
            .entry((event.skill_id, event.name.clone(), event.triggered))
            .or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += event.damage_total.unwrap_or(0.0);
    }

    let skills = grouped
        .into_iter()
        .map(
            |((skill_id, name, triggered), (event_count, total_damage))| SkillDamageSummary {
                skill_id,
                name,
                triggered,
                event_count,
                total_damage,
                damage_share: if response.total_damage.abs() > f64::EPSILON {
                    total_damage / response.total_damage
                } else {
                    0.0
                },
            },
        )
        .collect();

    SimulationSummary {
        dps: response.dps,
        total_damage: response.total_damage,
        fight_time: response.fight_time,
        skill_count: response.skill_count,
        fingerprint: response.fingerprint,
        fingerprint_hex: format!("{:016x}", response.fingerprint),
        skills,
    }
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        formations_file, load_formations, load_recipes, load_school_toml, load_skills,
        load_team_buffs, recipes_file, skills_dir, team_buffs_file, Attributes, TargetConfig,
    };
    use std::collections::HashMap;
    use std::path::Path;

    struct Fixture {
        version: GameVersion,
        mount: Mount,
        constants: MountConstants,
        skills: Vec<SkillSpec>,
        recipes: Vec<RecipeEntry>,
        team_buffs: Vec<TeamBuffEntry>,
        formations: Vec<FormationEntry>,
    }

    impl Fixture {
        fn load() -> Self {
            let version = GameVersion::AnYingQianJi;
            let mount = Mount::FenShanJin;
            let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
            Self {
                version,
                mount,
                constants,
                skills: load_skills(Path::new(&skills_dir(version, mount))),
                recipes: load_recipes(Path::new(&recipes_file(version))),
                team_buffs: load_team_buffs(Path::new(&team_buffs_file(version))),
                formations: load_formations(Path::new(&formations_file(version))),
            }
        }

        fn context(&self) -> SimulatorContext<'_> {
            SimulatorContext {
                game_version: self.version,
                mount: self.mount,
                constants: self.constants,
                skills: &self.skills,
                talents: &[],
                recipes: &self.recipes,
                team_buffs: &self.team_buffs,
                formations: &self.formations,
            }
        }
    }

    fn request() -> crate::SimulateRequest {
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
        }
    }

    #[test]
    fn current_scenario_returns_a_compact_grounded_summary() {
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();
        let evidence =
            get_current_scenario("trace-summary", &snapshot, &Fixture::load().context(), &ToolProvenance::fixture()).unwrap();

        assert_eq!(evidence.tool_name, GET_CURRENT_SCENARIO);
        assert_eq!(evidence.scenario_hash, snapshot.scenario_hash);
        assert_eq!(evidence.result.rotation_mode, "sequence");
        assert_eq!(evidence.result.sequence_entries, 2);
        assert_eq!(evidence.result.target_level, 134);
        assert_eq!(evidence.result.rotation_input.mode, "manual_sequence");
        assert_eq!(
            evidence.result.rotation_input.manual_operations[1].skill_name,
            "盾压"
        );
        assert!(evidence.result.rotation_input.macro_statements.is_empty());
    }

    #[test]
    fn rotation_inspection_searches_the_complete_sequence_and_returns_neighbors() {
        let mut value = request();
        value.sequence = (0..180)
            .map(|index| {
                if index == 167 {
                    "业火麟光".to_string()
                } else {
                    format!("技能{index}")
                }
            })
            .collect();
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, value)
                .unwrap();
        let evidence = inspect_rotation_input(
            "trace-rotation-search",
            &snapshot,
            Some("业火"),
            0,
            16,
            2,
            &Fixture::load().context(),
            &ToolProvenance::fixture(),
        )
        .unwrap();

        assert_eq!(evidence.tool_name, INSPECT_ROTATION_INPUT);
        assert_eq!(evidence.result.total_items, 180);
        assert_eq!(evidence.result.matches.len(), 1);
        let matched = &evidence.result.matches[0];
        assert_eq!(matched.matched.sequence_index, 167);
        assert_eq!(matched.matched.anchor_id, "sequence:167");
        assert_eq!(matched.matched.operation_number, 168);
        assert_eq!(matched.before[1].operation_number, 167);
        assert_eq!(matched.after[0].operation_number, 169);
    }

    #[test]
    fn rotation_inspection_pages_frequent_matches_without_losing_later_lines() {
        let mut value = request();
        value.sequence = vec!["斩刀".to_string(); 20];
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, value)
                .unwrap();
        let first = inspect_rotation_input(
            "trace-rotation-page-1",
            &snapshot,
            Some("斩刀"),
            0,
            32,
            1,
            &Fixture::load().context(),
            &ToolProvenance::fixture(),
        )
        .unwrap();
        assert_eq!(first.result.matches.len(), 8);
        assert_eq!(first.result.total_matches, Some(20));
        assert_eq!(first.result.returned_match_count, 8);
        assert_eq!(first.result.returned_window_count, 0);
        assert!(first.result.has_more);
        assert_eq!(first.result.next_start_index, Some(8));

        let second = inspect_rotation_input(
            "trace-rotation-page-2",
            &snapshot,
            Some("斩刀"),
            first.result.next_start_index.unwrap(),
            32,
            1,
            &Fixture::load().context(),
            &ToolProvenance::fixture(),
        )
        .unwrap();
        assert_eq!(second.result.matches[0].matched.operation_number, 9);
        assert_eq!(second.result.total_matches, Some(20));
        assert_eq!(second.result.page_start_index, 8);
        let last = inspect_rotation_input(
            "trace-rotation-page-last", &snapshot, Some("斩刀"), 16, 32, 1,
            &Fixture::load().context(), &ToolProvenance::fixture(),
        ).unwrap();
        assert_eq!(last.result.total_matches, Some(20));
        assert_eq!(last.result.returned_match_count, 4);
        assert!(!last.result.has_more);
    }

    #[test]
    fn rotation_input_describes_dynamic_timing_and_runtime_cooldown_roles() {
        let fixture = Fixture::load();
        let mut value = request();
        value.sequence = vec!["盾击".to_string(), "血怒".to_string(), "业火麟光".to_string(), "盾挡".to_string()];
        value.timing_offsets.insert("1".to_string(), -1.0);
        value.timing_offsets.insert("2".to_string(), 0.25);
        let snapshot = ScenarioSnapshotV1::capture(fixture.version, fixture.mount, value).unwrap();
        let before = serde_json::to_value(&snapshot).unwrap();
        let inspection = inspect_rotation_input(
            "trace-timing-semantics", &snapshot, None, 0, 32, 0,
            &fixture.context(), &ToolProvenance::fixture(),
        ).unwrap().result;
        let blood_rage = &inspection.window[1];
        assert_eq!(blood_rage.raw_timing_offset, Some(-1.0));
        assert_eq!(blood_rage.timing_mode, "follow_gcd_end");
        assert_eq!(blood_rage.delay_seconds, None);
        assert_eq!(blood_rage.timing_offset_seconds, None);
        assert_eq!(blood_rage.is_main_gcd, Some(false));
        assert_eq!(blood_rage.cooldown_semantics, "independent_of_main_gcd");
        assert!(serde_json::to_value(blood_rage).unwrap()["delay_seconds"].is_null());
        let blood_definition = &inspection.skill_semantics["血怒"];
        assert!(blood_definition.cooldowns.iter().any(|cd| {
            cd.kind == "skill_protection" && cd.base_duration_seconds == 0.5
        }));
        assert!(!blood_definition.cooldowns.iter().any(|cd| cd.kind == "shared_gcd"));
        assert_eq!(inspection.window[0].is_main_gcd, Some(true));
        assert_eq!(inspection.window[2].is_main_gcd, Some(false));
        assert_eq!(inspection.window[2].cooldown_semantics, "independent_of_main_gcd");
        assert_eq!(inspection.window[3].cooldown_semantics, "auxiliary_gcd");
        assert_eq!(inspection.window[3].is_main_gcd, Some(false));
        assert_eq!(inspection.window[2].timing_mode, "delay_from_earliest_cast");
        assert_eq!(inspection.window[2].delay_seconds, Some(0.25));
        assert_eq!(inspection.total_matches, None);
        assert_eq!(inspection.returned_window_count, 4);
        assert_eq!(inspection.skill_semantics.len(), 4);
        let summary = get_current_scenario(
            "trace-timing-summary", &snapshot, &fixture.context(), &ToolProvenance::fixture(),
        ).unwrap().result;
        assert_eq!(summary.rotation_input.manual_operations[1], *blood_rage);
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);
    }

    #[test]
    fn current_scenario_exposes_validated_macro_statements_without_manual_operations() {
        let mut value = request();
        value.macro_text = Some(
            "#page shield\n/cast [rage>64&nobuff:嗜血] 盾飞\n#page blade\n/fcast 业火麟光"
                .to_string(),
        );
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, value)
                .unwrap();
        let evidence =
            get_current_scenario("trace-macro-input", &snapshot, &Fixture::load().context(), &ToolProvenance::fixture())
                .unwrap();

        let input = evidence.result.rotation_input;
        assert_eq!(input.mode, "macro");
        assert_eq!(input.parse_status, "valid");
        assert!(input.parse_error.is_none());
        assert!(input.manual_operations.is_empty());
        assert_eq!(input.macro_statements.len(), 2);
        assert_eq!(input.macro_statements[0].source_line, 2);
        assert_eq!(input.macro_statements[0].stance.as_deref(), Some("shield"));
        assert_eq!(input.macro_statements[0].skill_name, "盾飞");
        assert_eq!(
            input.macro_statements[0].condition_semantics.as_deref(),
            Some("(rage>64 AND nobuff:嗜血)")
        );
        assert_eq!(
            input.macro_statements[0]
                .condition_ast
                .as_ref()
                .and_then(|value| value.pointer("/kind"))
                .and_then(serde_json::Value::as_str),
            Some("and")
        );
        assert_eq!(
            input
                .macro_semantics
                .as_ref()
                .map(|semantics| semantics.associativity.as_str()),
            Some("right")
        );
        let semantics = input.macro_semantics.as_ref().unwrap();
        assert!(semantics.stance_pages_present);
        assert_eq!(semantics.structure, "stance_split_pages");
        assert_eq!(semantics.page_count, 2);
        assert_eq!(
            semantics.page_selection,
            "current_stance_page_in_source_order"
        );
        assert!(semantics.page_selection_is_automatic);
        assert!(semantics
            .pause_stance_rule
            .contains("pausing_input_does_not_change_stance"));
        assert!(semantics
            .dun_fei_stance_rule
            .contains("short_pause_resumes_on_the_blade_page"));
        assert_eq!(
            input.macro_statements[0].statement,
            "/cast [rage>64&nobuff:嗜血] 盾飞"
        );
        assert_eq!(input.macro_statements[1].command, "fcast");
        assert_eq!(input.macro_statements[1].page, 1);
    }

    #[test]
    fn current_scenario_calls_an_unfiltered_macro_a_general_single_page() {
        let mut value = request();
        value.macro_text = Some("/cast 斩刀\n/cast 盾击".to_string());
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, value)
                .unwrap();
        let evidence =
            get_current_scenario("trace-general-macro", &snapshot, &Fixture::load().context(), &ToolProvenance::fixture())
                .unwrap();
        let semantics = evidence.result.rotation_input.macro_semantics.unwrap();
        assert_eq!(semantics.structure, "general_single_page");
        assert_eq!(semantics.page_count, 1);
        assert!(!semantics.stance_pages_present);
        assert_eq!(
            semantics.page_selection,
            "single_unfiltered_page_used_in_all_stances"
        );
    }

    #[test]
    fn simulation_tool_matches_the_shared_simulator_core() {
        let fixture = Fixture::load();
        let context = fixture.context();
        let snapshot =
            ScenarioSnapshotV1::capture(fixture.version, fixture.mount, request()).unwrap();
        let direct = simulate_core(
            &snapshot.simulation,
            context.skills,
            context.game_version,
            context.mount,
            context.constants,
            context.recipes,
            context.team_buffs,
            context.formations,
        );
        let mut budget = ToolBudget::new(1);
        let tool = simulate_scenario(
            "trace-sim",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        )
        .unwrap();

        assert_eq!(tool.evidence.result.dps, direct.dps);
        assert_eq!(tool.evidence.result.total_damage, direct.total_damage);
        assert_eq!(tool.evidence.result.fight_time, direct.fight_time);
        assert_eq!(tool.evidence.result.skill_count, direct.skill_count);
        assert_eq!(tool.evidence.result.fingerprint, direct.fingerprint);
        assert_eq!(tool.response.fingerprint, direct.fingerprint);
        let summarized_damage: f64 = tool
            .evidence
            .result
            .skills
            .iter()
            .map(|skill| skill.total_damage)
            .sum();
        assert!((summarized_damage - direct.total_damage).abs() < 0.001);
        assert_eq!(budget.used_simulations, 1);
    }

    #[test]
    fn simulation_tool_enforces_budget_before_running() {
        let fixture = Fixture::load();
        let context = fixture.context();
        let snapshot =
            ScenarioSnapshotV1::capture(fixture.version, fixture.mount, request()).unwrap();
        let mut budget = ToolBudget::new(0);
        let error = match simulate_scenario(
            "trace-budget",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("zero simulation budget should be rejected"),
        };

        assert!(matches!(
            error,
            ToolError::BudgetExceeded {
                resource: "simulations",
                limit: 0
            }
        ));
        assert_eq!(budget.used_simulations, 0);
    }

    #[test]
    fn simulation_tool_rejects_runtime_version_mismatch() {
        let fixture = Fixture::load();
        let context = fixture.context();
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::ShanHaiYuanLiu, fixture.mount, request())
                .unwrap();
        let mut budget = ToolBudget::new(1);
        let error = match simulate_scenario(
            "trace-version",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("runtime version mismatch should be rejected"),
        };

        assert!(matches!(error, ToolError::RuntimeMismatch { .. }));
        assert_eq!(budget.used_simulations, 0);
    }

    #[test]
    fn invalid_trace_id_is_rejected_without_spending_budget() {
        let fixture = Fixture::load();
        let context = fixture.context();
        let snapshot =
            ScenarioSnapshotV1::capture(fixture.version, fixture.mount, request()).unwrap();
        let mut budget = ToolBudget::new(1);
        let error = match simulate_scenario(
            "../../userdata",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("unsafe trace id should be rejected"),
        };

        assert!(matches!(
            error,
            ToolError::Evidence(EvidenceError::InvalidTraceId)
        ));
        assert_eq!(budget.used_simulations, 0);
    }
}
