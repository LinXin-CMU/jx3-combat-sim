use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use crate::{
    Attributes, FormationSelection, PreReleaseSpec, SimulateResponse, TargetConfig,
    TeamBuffSelection,
};

use super::evidence::{validate_trace_id, EvidenceEnvelopeV1, EvidenceError, ToolProvenance};
use super::schema::ScenarioSnapshotV1;
use super::tools::{
    elapsed_ms, run_simulation, summarize_simulation, verify_runtime, SimulatorContext,
    SkillDamageSummary, ToolBudget, ToolError,
};
use super::{comparison_timeline_diagnostics, ComparisonTimelineDiagnosticsV1};

pub const COMPARE_SCENARIOS: &str = "compare_scenarios";
pub const MAX_COMPARISON_CANDIDATES: usize = 3;

/// Explicit mutation for fields that may either hold a value or be cleared.
/// A tagged operation avoids the missing-vs-null ambiguity of `Option<Option<T>>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum PatchValueV1<T> {
    Set(T),
    Clear,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScenarioPatchV1 {
    pub haste_level: Option<u32>,
    pub sequence: Option<Vec<String>>,
    pub talents: Option<Vec<u32>>,
    pub channel_ticks: Option<HashMap<String, u32>>,
    pub timing_offsets: Option<HashMap<String, f64>>,
    pub network_delay: Option<u32>,
    pub recipes: Option<Vec<u32>>,
    pub qijin_buffs: Option<HashMap<String, u32>>,
    pub macro_text: Option<PatchValueV1<String>>,
    pub macro_duration: Option<PatchValueV1<f64>>,
    pub attributes: Option<Attributes>,
    pub target: Option<TargetConfig>,
    pub initial_rage: Option<PatchValueV1<i32>>,
    pub pauses: Option<Vec<(f64, f64)>>,
    pub boss_attack_interval: Option<PatchValueV1<f64>>,
    pub hanjia_expectation: Option<PatchValueV1<bool>>,
    pub tiegu_mode: Option<u8>,
    pub experimental: Option<bool>,
    pub equipment: Option<HashMap<String, u32>>,
    pub team_buffs: Option<Vec<TeamBuffSelection>>,
    pub formation: Option<PatchValueV1<FormationSelection>>,
    pub pre_releases: Option<Vec<PreReleaseSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePatchV1 {
    pub label: String,
    pub patch: ScenarioPatchV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldChange {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonMetrics {
    pub scenario_hash: String,
    pub dps: f64,
    pub total_damage: f64,
    pub fight_time: f64,
    pub skill_count: usize,
    pub fingerprint: u64,
    pub fingerprint_hex: String,
    pub diagnostics: ComparisonTimelineDiagnosticsV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_line_stats: Vec<crate::macro_eval::MacroLineExecutionStats>,
    /// Complete simulator-derived damage composition for the baseline. Candidate
    /// metrics omit this redundant list and expose actual changes in `skill_deltas`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillDamageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonCandidate {
    pub label: String,
    #[serde(default)]
    pub macro_pages: serde_json::Value,
    pub changes: Vec<FieldChange>,
    /// Deterministic meaning of edited conditions. This keeps the model from
    /// reversing countdown predicates such as `bufftime:x<5.3` -> `<5.0`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub condition_semantics: Vec<ConditionChangeSemanticV1>,
    pub metrics: ComparisonMetrics,
    pub delta_dps: f64,
    pub delta_percent: Option<f64>,
    pub observed_outcome: ComparisonOutcomeSummaryV1,
    /// True means the candidate produced the exact same deterministic combat trace.
    pub same_fingerprint: bool,
    pub diagnostic_delta: ComparisonDiagnosticDeltaV1,
    pub skill_deltas: Vec<SkillDamageDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConditionChangeSemanticV1 {
    pub line_number: usize,
    pub controlled_skill: String,
    pub condition: String,
    pub operator: String,
    pub before_threshold: f64,
    pub after_threshold: f64,
    pub truth_set_change: String,
    pub countdown_timing: String,
    pub interpretation_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonDiagnosticDeltaV1 {
    pub gcd_gap_count: i64,
    pub gcd_gap_seconds: f64,
    pub recorded_wait_count: i64,
    pub recorded_wait_seconds: f64,
    pub rage_overflow_total: i64,
    pub completed_cycle_count: i64,
    pub buffs: Vec<ComparisonBuffDeltaV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonBuffDeltaV1 {
    pub name: String,
    pub coverage_percentage_points: f64,
    pub average_stacks_while_active: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonOutcomeSummaryV1 {
    pub dps_relation_to_baseline: String,
    pub signed_dps_delta: f64,
    pub signed_percent_delta: Option<f64>,
    pub plain_language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillDamageDelta {
    pub skill_id: u32,
    pub name: String,
    pub triggered: bool,
    pub baseline_event_count: u32,
    pub candidate_event_count: u32,
    pub event_count_delta: i64,
    pub baseline_damage: f64,
    pub candidate_damage: f64,
    pub damage_delta: f64,
    pub baseline_share: f64,
    pub candidate_share: f64,
    pub share_delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioComparison {
    pub baseline: ComparisonMetrics,
    pub candidates: Vec<ComparisonCandidate>,
}

pub struct CandidateExecution {
    pub label: String,
    pub snapshot: ScenarioSnapshotV1,
    pub response: SimulateResponse,
}

pub struct ComparisonExecution {
    pub evidence: EvidenceEnvelopeV1<ScenarioComparison>,
    pub baseline_response: SimulateResponse,
    pub candidates: Vec<CandidateExecution>,
}

pub fn compare_scenarios(
    trace_id: &str,
    baseline: &ScenarioSnapshotV1,
    candidates: &[CandidatePatchV1],
    context: &SimulatorContext<'_>,
    provenance: &ToolProvenance,
    budget: &mut ToolBudget,
) -> Result<ComparisonExecution, ToolError> {
    compare_scenarios_with_baseline(
        trace_id, baseline, candidates, context, provenance, budget, None,
    )
}

pub fn compare_scenarios_with_baseline(
    trace_id: &str,
    baseline: &ScenarioSnapshotV1,
    candidates: &[CandidatePatchV1],
    context: &SimulatorContext<'_>,
    provenance: &ToolProvenance,
    budget: &mut ToolBudget,
    cached_baseline: Option<&SimulateResponse>,
) -> Result<ComparisonExecution, ToolError> {
    let started = Instant::now();
    validate_trace_id(trace_id)?;
    baseline.verify_hash()?;
    verify_runtime(baseline, context)?;
    validate_candidate_count(candidates.len())?;

    let mut labels = HashSet::new();
    let mut prepared = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        validate_label(&candidate.label)?;
        if !labels.insert(candidate.label.clone()) {
            return Err(ToolError::DuplicateCandidateLabel {
                label: candidate.label.clone(),
            });
        }
        let (snapshot, changes) = apply_patch(baseline, &candidate.patch, context)?;
        if changes.is_empty() {
            return Err(ToolError::NoScenarioChanges {
                label: candidate.label.clone(),
            });
        }
        let align_macro_duration = is_new_macro_program(baseline, &candidate.patch)
            && baseline.simulation.macro_duration.is_none() && candidate.patch.macro_duration.is_none();
        prepared.push((candidate.label.clone(), snapshot, changes, align_macro_duration));
    }

    let simulation_count =
        u32::try_from(prepared.len() + usize::from(cached_baseline.is_none())).unwrap_or(u32::MAX);
    budget.reserve_simulations(simulation_count)?;

    let baseline_response = cached_baseline
        .cloned()
        .unwrap_or_else(|| run_simulation(baseline, context));
    let baseline_metrics = metrics(baseline, &baseline_response);
    let mut result_candidates = Vec::with_capacity(prepared.len());
    let mut executions = Vec::with_capacity(prepared.len());

    for (label, mut snapshot, mut changes, align_macro_duration) in prepared {
        if align_macro_duration {
            let duration = baseline_response.fight_time.clamp(1.0, 3600.0);
            let duration_patch = ScenarioPatchV1 {
                macro_duration: Some(PatchValueV1::Set(duration)),
                sequence: Some(vec!["__macro__".to_string(); (duration / 0.25).ceil() as usize + 20]),
                ..ScenarioPatchV1::default()
            };
            let (aligned, alignment_changes) = apply_patch(&snapshot, &duration_patch, context)?;
            snapshot = aligned;
            for change in alignment_changes {
                if let Some(original) = changes.iter_mut().find(|old| old.field == change.field) {
                    original.after = change.after;
                } else { changes.push(change); }
            }
        }
        let response = run_simulation(&snapshot, context);
        let mut candidate_metrics = metrics(&snapshot, &response);
        let delta_dps = candidate_metrics.dps - baseline_metrics.dps;
        let delta_percent = if baseline_metrics.dps.abs() > f64::EPSILON {
            Some(delta_dps / baseline_metrics.dps * 100.0)
        } else {
            None
        };
        let same_fingerprint = candidate_metrics.fingerprint == baseline_metrics.fingerprint;
        let observed_outcome = comparison_outcome_summary(delta_dps, delta_percent);
        let diagnostic_delta = compare_diagnostics(
            &baseline_metrics.diagnostics,
            &candidate_metrics.diagnostics,
        );
        let skill_deltas =
            compare_skill_damage(&baseline_metrics.skills, &candidate_metrics.skills);
        let condition_semantics = condition_change_semantics(&changes);
        // Baseline composition plus the candidate's changed rows is lossless for
        // analysis and avoids repeating every unchanged skill on every model turn.
        candidate_metrics.skills.clear();
        result_candidates.push(ComparisonCandidate {
            label: label.clone(),
            macro_pages: snapshot.simulation.macro_text.as_deref()
                .map(super::distillation::page_lengths).unwrap_or(serde_json::Value::Null),
            changes,
            condition_semantics,
            metrics: candidate_metrics,
            delta_dps,
            delta_percent,
            observed_outcome,
            same_fingerprint,
            diagnostic_delta,
            skill_deltas,
        });
        executions.push(CandidateExecution {
            label,
            snapshot,
            response,
        });
    }

    let result = ScenarioComparison {
        baseline: baseline_metrics,
        candidates: result_candidates,
    };
    let args = serde_json::to_value(candidates)
        .map_err(|error| EvidenceError::Serialization(error.to_string()))?;
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        COMPARE_SCENARIOS,
        &baseline.scenario_hash,
        args,
        result,
        provenance,
        elapsed_ms(started),
    )?;

    Ok(ComparisonExecution {
        evidence,
        baseline_response,
        candidates: executions,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct BufftimeThreshold {
    line_number: usize,
    controlled_skill: String,
    condition: String,
    operator: char,
    threshold: f64,
}

fn condition_change_semantics(changes: &[FieldChange]) -> Vec<ConditionChangeSemanticV1> {
    let Some(change) = changes
        .iter()
        .find(|change| change.field == "simulation.macro_text")
    else {
        return Vec::new();
    };
    let Some(before) = change.before.as_str() else {
        return Vec::new();
    };
    let Some(after) = change.after.as_str() else {
        return Vec::new();
    };
    let before = bufftime_thresholds(before);
    let after = bufftime_thresholds(after);
    before
        .iter()
        .zip(after.iter())
        .filter(|(left, right)| {
            left.condition == right.condition
                && left.operator == right.operator
                && (left.threshold - right.threshold).abs() > f64::EPSILON
        })
        .map(|(left, right)| {
            let (truth_set_change, countdown_timing) = match (
                left.operator,
                right.threshold.total_cmp(&left.threshold),
            ) {
                ('<', std::cmp::Ordering::Less) => (
                    "满足区间缩小".to_string(),
                    "Buff 剩余时间递减时更晚满足；若此条件决定施放，候选倾向于更晚施放该技能。".to_string(),
                ),
                ('<', std::cmp::Ordering::Greater) => (
                    "满足区间扩大".to_string(),
                    "Buff 剩余时间递减时更早满足；若此条件决定施放，候选倾向于更早施放该技能。".to_string(),
                ),
                ('>', std::cmp::Ordering::Less) => (
                    "满足区间扩大".to_string(),
                    "Buff 剩余时间递减时会更晚失效，因此该条件保持满足的时间更长。".to_string(),
                ),
                ('>', std::cmp::Ordering::Greater) => (
                    "满足区间缩小".to_string(),
                    "Buff 剩余时间递减时会更早失效，因此该条件保持满足的时间更短。".to_string(),
                ),
                _ => ("阈值已变化".to_string(), "按条件真值区间解释。".to_string()),
            };
            ConditionChangeSemanticV1 {
                line_number: left.line_number,
                controlled_skill: left.controlled_skill.clone(),
                condition: format!("bufftime:{}", left.condition),
                operator: left.operator.to_string(),
                before_threshold: left.threshold,
                after_threshold: right.threshold,
                truth_set_change,
                countdown_timing,
                interpretation_boundary: "这是条件方向的确定性解释；DPS 与技能次数变化由同场景对照证明，具体连锁机制仍需结合时间轴判断。".to_string(),
            }
        })
        .collect()
}

fn bufftime_thresholds(macro_text: &str) -> Vec<BufftimeThreshold> {
    let mut result = Vec::new();
    for (line_index, line) in macro_text.lines().enumerate() {
        let controlled_skill = line
            .split_once(']')
            .map(|(_, skill)| skill.trim().to_string())
            .unwrap_or_default();
        let mut remainder = line;
        while let Some(start) = remainder.find("bufftime:") {
            let token = &remainder[start + "bufftime:".len()..];
            let Some(operator_index) = token.find(|character| character == '<' || character == '>') else {
                break;
            };
            let condition = token[..operator_index].trim();
            let operator = token.as_bytes()[operator_index] as char;
            let number = &token[operator_index + 1..];
            let number_len = number
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '.')
                .count();
            if !condition.is_empty() && number_len > 0 {
                if let Ok(threshold) = number[..number_len].parse::<f64>() {
                    result.push(BufftimeThreshold {
                        line_number: line_index + 1,
                        controlled_skill: controlled_skill.clone(),
                        condition: condition.to_string(),
                        operator,
                        threshold,
                    });
                }
            }
            remainder = &token[operator_index + 1 + number_len..];
        }
    }
    result
}

fn compare_diagnostics(
    baseline: &ComparisonTimelineDiagnosticsV1,
    candidate: &ComparisonTimelineDiagnosticsV1,
) -> ComparisonDiagnosticDeltaV1 {
    let baseline_buffs = baseline
        .buffs
        .iter()
        .map(|buff| (buff.name.as_str(), buff))
        .collect::<HashMap<_, _>>();
    let mut buffs = candidate
        .buffs
        .iter()
        .filter_map(|candidate_buff| {
            let baseline_buff = baseline_buffs.get(candidate_buff.name.as_str())?;
            Some(ComparisonBuffDeltaV1 {
                name: candidate_buff.name.clone(),
                coverage_percentage_points: candidate_buff.coverage_percent
                    - baseline_buff.coverage_percent,
                average_stacks_while_active: candidate_buff.average_stacks_while_active
                    - baseline_buff.average_stacks_while_active,
            })
        })
        .collect::<Vec<_>>();
    buffs.sort_by(|left, right| left.name.cmp(&right.name));
    ComparisonDiagnosticDeltaV1 {
        gcd_gap_count: candidate.gcd_gap_count as i64 - baseline.gcd_gap_count as i64,
        gcd_gap_seconds: candidate.gcd_gap_seconds - baseline.gcd_gap_seconds,
        recorded_wait_count: candidate.recorded_wait_count as i64
            - baseline.recorded_wait_count as i64,
        recorded_wait_seconds: candidate.recorded_wait_seconds - baseline.recorded_wait_seconds,
        rage_overflow_total: candidate.rage_overflow_total as i64
            - baseline.rage_overflow_total as i64,
        completed_cycle_count: candidate.completed_cycle_count as i64
            - baseline.completed_cycle_count as i64,
        buffs,
    }
}

fn comparison_outcome_summary(
    delta_dps: f64,
    delta_percent: Option<f64>,
) -> ComparisonOutcomeSummaryV1 {
    let relation = if delta_dps > f64::EPSILON {
        "higher"
    } else if delta_dps < -f64::EPSILON {
        "lower"
    } else {
        "unchanged"
    };
    let plain_language = match relation {
        "higher" => format!("候选 DPS 比基线高 {:.2}。", delta_dps.abs()),
        "lower" => format!("候选 DPS 比基线低 {:.2}。", delta_dps.abs()),
        _ => "候选 DPS 与基线相同。".to_string(),
    };
    ComparisonOutcomeSummaryV1 {
        dps_relation_to_baseline: relation.to_string(),
        signed_dps_delta: delta_dps,
        signed_percent_delta: delta_percent,
        plain_language,
    }
}

fn compare_skill_damage(
    baseline: &[SkillDamageSummary],
    candidate: &[SkillDamageSummary],
) -> Vec<SkillDamageDelta> {
    type Key = (u32, String, bool);
    let baseline_by_key = baseline
        .iter()
        .map(|skill| ((skill.skill_id, skill.name.clone(), skill.triggered), skill))
        .collect::<BTreeMap<Key, _>>();
    let candidate_by_key = candidate
        .iter()
        .map(|skill| ((skill.skill_id, skill.name.clone(), skill.triggered), skill))
        .collect::<BTreeMap<Key, _>>();
    let mut keys = baseline_by_key
        .keys()
        .chain(candidate_by_key.keys())
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    let mut deltas = keys
        .into_iter()
        .filter_map(|(skill_id, name, triggered)| {
            let before = baseline_by_key.get(&(skill_id, name.clone(), triggered));
            let after = candidate_by_key.get(&(skill_id, name.clone(), triggered));
            let baseline_event_count = before.map_or(0, |skill| skill.event_count);
            let candidate_event_count = after.map_or(0, |skill| skill.event_count);
            let baseline_damage = before.map_or(0.0, |skill| skill.total_damage);
            let candidate_damage = after.map_or(0.0, |skill| skill.total_damage);
            let baseline_share = before.map_or(0.0, |skill| skill.damage_share);
            let candidate_share = after.map_or(0.0, |skill| skill.damage_share);
            let damage_delta = candidate_damage - baseline_damage;
            let share_delta = candidate_share - baseline_share;
            (baseline_event_count != candidate_event_count
                || damage_delta.abs() > f64::EPSILON
                || share_delta.abs() > f64::EPSILON)
                .then_some(SkillDamageDelta {
                    skill_id,
                    name,
                    triggered,
                    baseline_event_count,
                    candidate_event_count,
                    event_count_delta: i64::from(candidate_event_count)
                        - i64::from(baseline_event_count),
                    baseline_damage,
                    candidate_damage,
                    damage_delta,
                    baseline_share,
                    candidate_share,
                    share_delta,
                })
        })
        .collect::<Vec<_>>();
    // Ignore share-only movement caused by a changed total-damage denominator.
    // These rows have identical casts and damage and distract from causal changes.
    deltas.retain(|delta| delta.event_count_delta != 0 || delta.damage_delta.abs() >= 0.5);
    deltas.sort_by(|left, right| {
        right
            .damage_delta
            .abs()
            .total_cmp(&left.damage_delta.abs())
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });
    deltas
}

fn validate_candidate_count(count: usize) -> Result<(), ToolError> {
    if (1..=MAX_COMPARISON_CANDIDATES).contains(&count) {
        Ok(())
    } else {
        Err(ToolError::InvalidCandidateCount {
            count,
            max: MAX_COMPARISON_CANDIDATES,
        })
    }
}

fn validate_label(label: &str) -> Result<(), ToolError> {
    let count = label.chars().count();
    if (1..=64).contains(&count) && !label.trim().is_empty() && !label.chars().any(char::is_control)
    {
        Ok(())
    } else {
        Err(ToolError::InvalidCandidateLabel)
    }
}

pub(super) fn configure_macro_rotation(simulation: &mut crate::SimulateRequest, macro_text: String) {
    let duration = simulation.macro_duration.unwrap_or(300.0).clamp(1.0, 3600.0);
    simulation.sequence = vec!["__macro__".to_string(); (duration / 0.25).ceil() as usize + 20];
    simulation.macro_text = Some(macro_text);
    simulation.macro_duration = Some(duration);
    simulation.channel_ticks.clear();
    simulation.timing_offsets.clear();
    simulation.qijin_buffs.clear();
}

fn is_new_macro_program(baseline: &ScenarioSnapshotV1, patch: &ScenarioPatchV1) -> bool {
    matches!(patch.macro_text, Some(PatchValueV1::Set(_))) && patch.sequence.is_none()
        && !baseline.simulation.sequence.iter().any(|entry| entry == "__macro__")
}

fn apply_patch(
    baseline: &ScenarioSnapshotV1,
    patch: &ScenarioPatchV1,
    context: &SimulatorContext<'_>,
) -> Result<(ScenarioSnapshotV1, Vec<FieldChange>), ToolError> {
    let mut simulation = baseline.simulation.clone();
    let mut changes = Vec::new();

    apply_value(
        "simulation.haste_level",
        &mut simulation.haste_level,
        &patch.haste_level,
        &mut changes,
    )?;
    apply_value(
        "simulation.sequence",
        &mut simulation.sequence,
        &patch.sequence,
        &mut changes,
    )?;
    apply_value(
        "simulation.talents",
        &mut simulation.talents,
        &patch.talents,
        &mut changes,
    )?;
    apply_value(
        "simulation.channel_ticks",
        &mut simulation.channel_ticks,
        &patch.channel_ticks,
        &mut changes,
    )?;
    apply_value(
        "simulation.timing_offsets",
        &mut simulation.timing_offsets,
        &patch.timing_offsets,
        &mut changes,
    )?;
    apply_value(
        "simulation.network_delay",
        &mut simulation.network_delay,
        &patch.network_delay,
        &mut changes,
    )?;
    apply_value(
        "simulation.recipes",
        &mut simulation.recipes,
        &patch.recipes,
        &mut changes,
    )?;
    apply_value(
        "simulation.qijin_buffs",
        &mut simulation.qijin_buffs,
        &patch.qijin_buffs,
        &mut changes,
    )?;
    apply_nullable(
        "simulation.macro_text",
        &mut simulation.macro_text,
        &patch.macro_text,
        &mut changes,
    )?;
    apply_nullable(
        "simulation.macro_duration",
        &mut simulation.macro_duration,
        &patch.macro_duration,
        &mut changes,
    )?;
    apply_value(
        "simulation.attributes",
        &mut simulation.attributes,
        &patch.attributes.as_ref().map(|value| Some(value.clone())),
        &mut changes,
    )?;
    apply_value(
        "simulation.target",
        &mut simulation.target,
        &patch.target.as_ref().map(|value| Some(value.clone())),
        &mut changes,
    )?;
    apply_nullable(
        "simulation.initial_rage",
        &mut simulation.initial_rage,
        &patch.initial_rage,
        &mut changes,
    )?;
    apply_value(
        "simulation.pauses",
        &mut simulation.pauses,
        &patch.pauses,
        &mut changes,
    )?;
    apply_nullable(
        "simulation.boss_attack_interval",
        &mut simulation.boss_attack_interval,
        &patch.boss_attack_interval,
        &mut changes,
    )?;
    apply_nullable(
        "simulation.hanjia_expectation",
        &mut simulation.hanjia_expectation,
        &patch.hanjia_expectation,
        &mut changes,
    )?;
    apply_value(
        "simulation.tiegu_mode",
        &mut simulation.tiegu_mode,
        &patch.tiegu_mode,
        &mut changes,
    )?;
    apply_value(
        "simulation.experimental",
        &mut simulation.experimental,
        &patch.experimental,
        &mut changes,
    )?;
    apply_value(
        "simulation.equipment",
        &mut simulation.equipment,
        &patch.equipment,
        &mut changes,
    )?;
    apply_value(
        "simulation.team_buffs",
        &mut simulation.team_buffs,
        &patch.team_buffs,
        &mut changes,
    )?;
    apply_nullable(
        "simulation.formation",
        &mut simulation.formation,
        &patch.formation,
        &mut changes,
    )?;
    apply_value(
        "simulation.pre_releases",
        &mut simulation.pre_releases,
        &patch.pre_releases,
        &mut changes,
    )?;

    if is_new_macro_program(baseline, patch) {
        let mut macro_simulation = simulation.clone();
        configure_macro_rotation(&mut macro_simulation, simulation.macro_text.clone().unwrap());
        apply_value("simulation.sequence", &mut simulation.sequence, &Some(macro_simulation.sequence), &mut changes)?;
        apply_value("simulation.macro_duration", &mut simulation.macro_duration, &Some(macro_simulation.macro_duration), &mut changes)?;
        apply_value("simulation.channel_ticks", &mut simulation.channel_ticks, &Some(macro_simulation.channel_ticks), &mut changes)?;
        apply_value("simulation.timing_offsets", &mut simulation.timing_offsets, &Some(macro_simulation.timing_offsets), &mut changes)?;
        apply_value("simulation.qijin_buffs", &mut simulation.qijin_buffs, &Some(macro_simulation.qijin_buffs), &mut changes)?;
    }
    let snapshot = ScenarioSnapshotV1::capture(context.game_version, context.mount, simulation)?;
    verify_runtime(&snapshot, context)?;
    Ok((snapshot, changes))
}

fn apply_value<T: Clone + Serialize>(
    field: &str,
    current: &mut T,
    patch: &Option<T>,
    changes: &mut Vec<FieldChange>,
) -> Result<(), ToolError> {
    let Some(after) = patch else {
        return Ok(());
    };
    let before_value = to_value(&*current)?;
    let after_value = to_value(after)?;
    if before_value != after_value {
        *current = after.clone();
        changes.push(FieldChange {
            field: field.to_string(),
            before: before_value,
            after: after_value,
        });
    }
    Ok(())
}

fn apply_nullable<T: Clone + Serialize>(
    field: &str,
    current: &mut Option<T>,
    patch: &Option<PatchValueV1<T>>,
    changes: &mut Vec<FieldChange>,
) -> Result<(), ToolError> {
    let Some(operation) = patch else {
        return Ok(());
    };
    let after = match operation {
        PatchValueV1::Set(value) => Some(value.clone()),
        PatchValueV1::Clear => None,
    };
    let before_value = to_value(&*current)?;
    let after_value = to_value(&after)?;
    if before_value != after_value {
        *current = after;
        changes.push(FieldChange {
            field: field.to_string(),
            before: before_value,
            after: after_value,
        });
    }
    Ok(())
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ToolError> {
    serde_json::to_value(value)
        .map_err(|error| EvidenceError::Serialization(error.to_string()).into())
}

fn metrics(snapshot: &ScenarioSnapshotV1, response: &SimulateResponse) -> ComparisonMetrics {
    let summary = summarize_simulation(response);
    ComparisonMetrics {
        scenario_hash: snapshot.scenario_hash.clone(),
        dps: summary.dps,
        total_damage: summary.total_damage,
        fight_time: summary.fight_time,
        skill_count: summary.skill_count,
        fingerprint: summary.fingerprint,
        fingerprint_hex: summary.fingerprint_hex,
        diagnostics: comparison_timeline_diagnostics(response, &snapshot.simulation.pauses),
        macro_line_stats: response.macro_line_stats.clone(),
        skills: summary.skills,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        formations_file, load_formations, load_recipes, load_school_toml, load_skills,
        load_team_buffs, recipes_file, skills_dir, team_buffs_file, GameVersion, Mount,
        MountConstants, RecipeEntry, SimulateRequest, SkillSpec, TeamBuffEntry,
    };
    use std::path::Path;

    struct Fixture {
        version: GameVersion,
        mount: Mount,
        constants: MountConstants,
        skills: Vec<SkillSpec>,
        recipes: Vec<RecipeEntry>,
        team_buffs: Vec<TeamBuffEntry>,
        formations: Vec<crate::FormationEntry>,
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

        fn snapshot(&self) -> ScenarioSnapshotV1 {
            let request: SimulateRequest = serde_json::from_value(serde_json::json!({
                "haste_level": 42087,
                "sequence": ["盾击", "盾压"],
                "network_delay": 0,
                "attributes": {
                    "base_attack": 38466,
                    "weapon_damage": 10986,
                    "crit_level": 54841,
                    "crit_effect_level": 0,
                    "overcome_level": 29480,
                    "strain_level": 66031,
                    "haste_level": 42087
                },
                "target": {"level": 134, "defense_bonus": 0},
                "initial_rage": 50
            }))
            .unwrap();
            ScenarioSnapshotV1::capture(self.version, self.mount, request).unwrap()
        }
    }

    fn candidate(label: &str, network_delay: u32) -> CandidatePatchV1 {
        CandidatePatchV1 {
            label: label.to_string(),
            patch: ScenarioPatchV1 {
                network_delay: Some(network_delay),
                ..ScenarioPatchV1::default()
            },
        }
    }

    #[test]
    fn patch_json_has_explicit_clear_and_rejects_unknown_fields() {
        let patch: ScenarioPatchV1 = serde_json::from_value(serde_json::json!({
            "macro_duration": {"op": "clear"},
            "network_delay": 100
        }))
        .unwrap();
        assert!(matches!(patch.macro_duration, Some(PatchValueV1::Clear)));
        assert_eq!(patch.network_delay, Some(100));

        assert!(
            serde_json::from_value::<ScenarioPatchV1>(serde_json::json!({
                "arbitrary_path": "../../userdata"
            }))
            .is_err()
        );
    }

    #[test]
    fn typed_patch_inherits_unspecified_fields_and_reports_exact_change() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let (patched, changes) = apply_patch(
            &baseline,
            &candidate("100ms", 100).patch,
            &fixture.context(),
        )
        .unwrap();

        assert_eq!(patched.simulation.network_delay, 100);
        assert_eq!(
            serde_json::to_value(&patched.simulation.attributes).unwrap(),
            serde_json::to_value(&baseline.simulation.attributes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&patched.simulation.target).unwrap(),
            serde_json::to_value(&baseline.simulation.target).unwrap()
        );
        assert_eq!(patched.simulation.equipment, baseline.simulation.equipment);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].field, "simulation.network_delay");
        assert_ne!(patched.scenario_hash, baseline.scenario_hash);
    }

    #[test]
    fn nullable_patch_can_clear_an_optional_field() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let patch = ScenarioPatchV1 {
            initial_rage: Some(PatchValueV1::Clear),
            ..ScenarioPatchV1::default()
        };
        let (patched, changes) = apply_patch(&baseline, &patch, &fixture.context()).unwrap();

        assert_eq!(patched.simulation.initial_rage, None);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].before, serde_json::json!(50));
        assert_eq!(changes[0].after, Value::Null);
    }

    #[test]
    fn bufftime_less_than_threshold_reports_countdown_direction() {
        let changes = vec![FieldChange {
            field: "simulation.macro_text".to_string(),
            before: serde_json::json!("/cast [rage>64&bufftime:嗜血<5.3|nobuff:嗜血] 盾飞"),
            after: serde_json::json!("/cast [rage>64&bufftime:嗜血<5.0|nobuff:嗜血] 盾飞"),
        }];

        let semantics = condition_change_semantics(&changes);

        assert_eq!(semantics.len(), 1);
        assert_eq!(semantics[0].controlled_skill, "盾飞");
        assert_eq!(semantics[0].truth_set_change, "满足区间缩小");
        assert!(semantics[0].countdown_timing.contains("更晚满足"));
    }

    #[test]
    fn outcome_summary_preserves_the_sign_of_a_negative_delta() {
        let summary = comparison_outcome_summary(-12_805.86, Some(-0.42));

        assert_eq!(summary.dps_relation_to_baseline, "lower");
        assert_eq!(summary.signed_dps_delta, -12_805.86);
        assert_eq!(summary.signed_percent_delta, Some(-0.42));
        assert!(summary.plain_language.contains("低"));
    }

    #[test]
    fn compare_runs_baseline_and_candidate_with_atomic_budget() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let mut budget = ToolBudget::new(2);
        let execution = compare_scenarios(
            "trace-compare",
            &baseline,
            &[candidate("100ms", 100)],
            &fixture.context(),
            &ToolProvenance::fixture(),
            &mut budget,
        )
        .unwrap();

        assert_eq!(budget.used_simulations, 2);
        assert_eq!(execution.evidence.tool_name, COMPARE_SCENARIOS);
        assert_eq!(execution.evidence.result.candidates.len(), 1);
        let result = &execution.evidence.result.candidates[0];
        assert_eq!(result.label, "100ms");
        assert_eq!(
            result.delta_dps,
            result.metrics.dps - execution.evidence.result.baseline.dps
        );
        assert_eq!(
            result.diagnostic_delta.gcd_gap_count,
            result.metrics.diagnostics.gcd_gap_count as i64
                - execution.evidence.result.baseline.diagnostics.gcd_gap_count as i64
        );
        assert_eq!(
            execution.baseline_response.fingerprint,
            execution.evidence.result.baseline.fingerprint
        );
        assert_eq!(
            execution.candidates[0].response.fingerprint,
            result.metrics.fingerprint
        );
    }

    #[test]
    fn full_macro_candidate_runs_macro_engine_instead_of_original_manual_inputs() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let original = serde_json::to_value(&baseline).unwrap();
        let execution = compare_scenarios("trace-new-macro", &baseline, &[CandidatePatchV1 {
            label:"只打盾击反例".into(), patch:ScenarioPatchV1 {
                macro_text:Some(PatchValueV1::Set("/cast 盾击".into())), ..ScenarioPatchV1::default()
            }
        }], &fixture.context(), &ToolProvenance::fixture(), &mut ToolBudget::new(2)).unwrap();
        let candidate = &execution.candidates[0];
        assert!(candidate.snapshot.simulation.sequence.iter().all(|name| name == "__macro__"));
        assert!(!candidate.response.macro_line_stats.is_empty());
        assert_eq!(candidate.snapshot.simulation.macro_duration, Some(execution.baseline_response.fight_time.clamp(1.0,3600.0)));
        assert_ne!(candidate.response.fingerprint, execution.baseline_response.fingerprint);
        assert_eq!(serde_json::to_value(&baseline).unwrap(),original);
        assert_eq!(serde_json::to_value(&candidate.snapshot.simulation.attributes).unwrap(),serde_json::to_value(&baseline.simulation.attributes).unwrap());
        assert_eq!(candidate.snapshot.simulation.talents,baseline.simulation.talents);
        assert_eq!(candidate.snapshot.simulation.recipes,baseline.simulation.recipes);
    }

    #[test]
    fn compare_rejects_noop_before_spending_budget() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let mut budget = ToolBudget::new(2);
        let error = match compare_scenarios(
            "trace-noop",
            &baseline,
            &[candidate("same", 0)],
            &fixture.context(),
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("no-op candidate should be rejected"),
        };

        assert!(matches!(error, ToolError::NoScenarioChanges { .. }));
        assert_eq!(budget.used_simulations, 0);
    }

    #[test]
    fn skill_damage_delta_reports_count_damage_and_share_changes() {
        let baseline = vec![SkillDamageSummary {
            skill_id: 1,
            name: "绝刀".to_string(),
            triggered: false,
            event_count: 10,
            total_damage: 1_000.0,
            damage_share: 0.5,
        }];
        let candidate = vec![SkillDamageSummary {
            skill_id: 1,
            name: "绝刀".to_string(),
            triggered: false,
            event_count: 12,
            total_damage: 1_300.0,
            damage_share: 0.6,
        }];

        let deltas = compare_skill_damage(&baseline, &candidate);

        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].event_count_delta, 2);
        assert_eq!(deltas[0].damage_delta, 300.0);
        assert!((deltas[0].share_delta - 0.1).abs() < 1e-12);
    }

    #[test]
    fn compare_rejects_insufficient_total_budget_without_partial_run() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let mut budget = ToolBudget::new(1);
        let error = match compare_scenarios(
            "trace-budget",
            &baseline,
            &[candidate("100ms", 100)],
            &fixture.context(),
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("insufficient comparison budget should be rejected"),
        };

        assert!(matches!(error, ToolError::BudgetExceeded { .. }));
        assert_eq!(budget.used_simulations, 0);
    }

    #[test]
    fn compare_enforces_candidate_limit_and_unique_labels() {
        let fixture = Fixture::load();
        let baseline = fixture.snapshot();
        let too_many = vec![
            candidate("a", 10),
            candidate("b", 20),
            candidate("c", 30),
            candidate("d", 40),
        ];
        let mut budget = ToolBudget::new(10);
        let count_error = match compare_scenarios(
            "trace-count",
            &baseline,
            &too_many,
            &fixture.context(),
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("too many candidates should be rejected"),
        };
        assert!(matches!(
            count_error,
            ToolError::InvalidCandidateCount { count: 4, max: 3 }
        ));

        let duplicate = vec![candidate("same", 10), candidate("same", 20)];
        let label_error = match compare_scenarios(
            "trace-label",
            &baseline,
            &duplicate,
            &fixture.context(),
            &ToolProvenance::fixture(),
            &mut budget,
        ) {
            Err(error) => error,
            Ok(_) => panic!("duplicate candidate labels should be rejected"),
        };
        assert!(matches!(
            label_error,
            ToolError::DuplicateCandidateLabel { .. }
        ));
        assert_eq!(budget.used_simulations, 0);
    }
}
