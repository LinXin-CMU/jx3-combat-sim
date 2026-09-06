use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::{
    BuffTimelineTrack, CastEvent, EventBuff, EventState, RageOverflowCause, RageTransaction, Stance,
};
use std::collections::{BTreeMap, BTreeSet};

use super::evidence::{validate_trace_id, EvidenceEnvelopeV1, ToolProvenance};
use super::tools::{elapsed_ms, SimulationExecution, SkillDamageSummary, ToolError};

pub const ANALYZE_TIMELINE: &str = "analyze_timeline";
pub const INSPECT_TIMELINE_EVENTS: &str = "inspect_timeline_events";
const TIME_EPSILON: f64 = 0.001;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEventSelector {
    RageCap,
    RageOverflow,
    GcdGap,
    CooldownWait,
    Skill,
    Time,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEventQueryV1 {
    #[serde(default)]
    pub rage_cost_below: Option<u32>,
    #[serde(default)]
    pub event_number: Option<usize>,
    pub selector: TimelineEventSelector,
    pub skill_name: Option<String>,
    pub time_seconds: Option<f64>,
    pub start_match: usize,
    pub limit: usize,
    pub context_radius: usize,
    pub buff_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineBuffViewV1 {
    pub buff_id: u32,
    pub name: String,
    pub remaining_seconds: f64,
    pub stacks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineStateViewV1 {
    pub rage: i32,
    pub stance: String,
    pub buffs: Vec<TimelineBuffViewV1>,
    pub target_buffs: Vec<TimelineBuffViewV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEventViewV1 {
    #[serde(default)]
    pub operation_number: Option<usize>,
    /// Stable identity within this deterministic timeline. UI layout is not
    /// part of this identity.
    pub anchor_id: String,
    /// Zero-based index among active (non-triggered) casts.
    pub active_event_index: usize,
    /// One-based number used by the model transport marker `ev:`.
    pub event_number: usize,
    pub cast_time: f64,
    pub skill_id: u32,
    pub skill_name: String,
    pub is_main: bool,
    pub is_macro: bool,
    pub macro_page: Option<usize>,
    pub macro_line: Option<usize>,
    pub gcd_seconds: f64,
    pub cooldown_wait_seconds: f64,
    pub rage_delta: Option<i32>,
    pub rage_overflow: Option<u32>,
    pub rage_overflow_sources: Vec<RageOverflowCause>,
    pub rage_transactions: Vec<RageTransaction>,
    pub rage_generated: Option<u32>,
    pub rage_gained: Option<u32>,
    pub rage_spent: Option<u32>,
    pub rage_cost: Option<u32>,
    pub damage_total: Option<f64>,
    pub state_before: Option<TimelineStateViewV1>,
    pub state_after: Option<TimelineStateViewV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEventWindowV1 {
    pub matched: TimelineEventViewV1,
    pub context: Vec<TimelineEventViewV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEventInspectionV1 {
    pub selector: TimelineEventSelector,
    pub total_matches: usize,
    /// Lightweight index of matching events so the Agent can identify several
    /// occurrences in one read. Detailed neighboring context remains bounded
    /// by `windows`.
    pub match_index: Vec<TimelineEventMatchV1>,
    pub match_index_truncated: bool,
    pub start_match: usize,
    pub next_start_match: Option<usize>,
    /// How the detailed windows were selected.
    pub window_selection: String,
    pub windows: Vec<TimelineEventWindowV1>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEventMatchV1 {
    #[serde(default)]
    pub operation_number: Option<usize>,
    pub event_number: usize,
    pub cast_time: f64,
    pub skill_name: String,
    pub macro_page: Option<usize>,
    pub macro_line: Option<usize>,
    pub rage_before: Option<i32>,
    pub rage_after: Option<i32>,
    pub rage_cost: Option<u32>,
    pub rage_overflow: Option<u32>,
    pub rage_overflow_sources: Vec<RageOverflowCause>,
    pub rage_transactions: Vec<RageTransaction>,
    pub rage_generated: Option<u32>,
    pub rage_gained: Option<u32>,
    pub rage_spent: Option<u32>,
    pub buffs_before: Vec<TimelineBuffViewV1>,
    /// The nearest preceding occurrence of each important cycle landmark.
    /// This is positional evidence only; guide knowledge decides whether the
    /// relationship is desirable.
    pub prior_mechanic_landmarks: Vec<MechanicLandmarkV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_knife: Option<AbsoluteKnifeSemanticsV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MechanicLandmarkV1 {
    pub event_number: usize,
    pub cast_time: f64,
    pub skill_name: String,
    pub seconds_before: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AbsoluteKnifeSemanticsV1 {
    pub yuan_ge_before: u32,
    pub yuan_ge_after: u32,
    pub triggered_yuan_ge_blood_shadow: bool,
    pub without_yuan_ge_blood_shadow: bool,
    pub blood_rage_active: bool,
    pub kuang_jue_active: bool,
    pub tian_xia_hong_yuan_active: bool,
}

pub struct TimelineEventInspectionExecution {
    pub evidence: EvidenceEnvelopeV1<TimelineEventInspectionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WaitEvidence {
    pub event_number: usize,
    pub cast_time: f64,
    pub skill_id: u32,
    pub skill_name: String,
    pub wait_seconds: f64,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GcdGapEvidence {
    pub previous_event_number: usize,
    pub previous_cast_time: f64,
    pub previous_skill_id: u32,
    pub previous_skill_name: String,
    pub next_cast_time: f64,
    pub next_event_number: usize,
    pub next_skill_id: u32,
    pub next_skill_name: String,
    pub expected_ready_time: f64,
    pub observed_gap_seconds: f64,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RageOverflowSourceV1 {
    /// The concrete state mutation that attempted to add rage past the cap.
    pub rage_source: String,
    /// Active casts during which this source overflowed. These are locations,
    /// not causal labels.
    pub active_skill_names: Vec<String>,
    pub event_count: usize,
    pub overflow_total: u32,
    pub event_numbers: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RageObservation {
    pub minimum: i32,
    pub maximum: i32,
    pub ending: i32,
    pub sample_count: usize,
    pub at_cap_observations: usize,
    pub overflow_events: usize,
    pub overflow_total: u32,
    pub generated_before_cap: u32,
    pub gained_after_cap: u32,
    pub spent: u32,
    pub overflow_percent_of_generated: f64,
    /// Exact active events during which a capped gain was recorded. This
    /// distinguishes resource loss from a sample that merely sat at 100; use
    /// the neighboring event window to identify the causal source.
    pub overflow_sources: Vec<RageOverflowSourceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimeInterval {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuffCoverage {
    pub buff_id: u32,
    pub name: String,
    pub active_seconds: f64,
    pub coverage_percent: f64,
    /// Time-weighted stack count while the buff is active. This is deliberately
    /// separate from coverage and may be greater than one.
    pub average_stacks_while_active: f64,
    pub maximum_stacks_observed: u32,
    pub activation_count: usize,
    pub open_at_fight_end: bool,
    pub unmatched_close_events: usize,
    pub intervals: Vec<TimeInterval>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkippedSkill {
    pub sequence_index: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DurationObservation {
    pub count: usize,
    pub total_seconds: f64,
    pub maximum_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StanceObservation {
    pub active_casts_by_stance: BTreeMap<String, usize>,
    pub observed_transitions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSignal {
    pub code: String,
    pub summary: String,
    pub evidence_paths: Vec<String>,
    pub interpretation_boundary: String,
}

/// Compact, input-mode-independent observations used for the diagnosis stage.
/// These are not a hidden quality score: guide evidence and same-scenario
/// experiments are still required to interpret whether a signal matters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationDiagnosticProfile {
    pub input_mode: String,
    pub active_cast_count: usize,
    pub main_gcd_cast_count: usize,
    pub active_casts_per_minute: f64,
    pub active_skill_variety: usize,
    pub damaging_skill_variety: usize,
    pub cadence_gaps: DurationObservation,
    pub cooldown_waits: DurationObservation,
    pub stance: StanceObservation,
    pub observed_strengths: Vec<DiagnosticSignal>,
    pub observed_risks: Vec<DiagnosticSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationCycleSkillV1 {
    pub event_number: usize,
    pub cast_time: f64,
    pub skill_name: String,
    pub rage_before: Option<i32>,
    pub rage_after: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AbsoluteKnifeObservationV1 {
    pub event_number: usize,
    pub skill_name: String,
    pub rage_before: Option<i32>,
    pub rage_cost: Option<u32>,
    pub yuan_ge_before: Option<u32>,
    pub yuan_ge_after: Option<u32>,
    pub triggered_yuan_ge_blood_shadow: bool,
    pub without_yuan_ge_blood_shadow: bool,
    pub blood_rage_active: bool,
    pub kuang_jue_active: bool,
    pub tian_xia_hong_yuan_active: bool,
    pub prior_mechanic_landmarks: Vec<MechanicLandmarkV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationCycleObservationV1 {
    pub cycle_number: usize,
    pub start_event_number: usize,
    pub end_event_number: usize,
    pub start_time: f64,
    pub end_time: f64,
    pub completed_by_shield_return: bool,
    pub key_sequence: Vec<RotationCycleSkillV1>,
    pub shield_strike_count: usize,
    pub zhen_yun_count: usize,
    pub slash_count: usize,
    pub absolute_knives: Vec<AbsoluteKnifeObservationV1>,
    /// Total rage that effects attempted to generate before the 100-rage cap.
    pub rage_generated: u32,
    /// Rage actually added after cap clipping.
    pub rage_gained: u32,
    /// Rage actually consumed, including events that also generated rage.
    pub rage_spent: u32,
    pub rage_overflow: u32,
    pub yuan_ge_gained_stacks: u32,
    pub yuan_ge_consumed_stacks: u32,
    pub yuan_ge_at_cap_before_events: usize,
    pub yuan_ge_blocked_gain_event_numbers: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationCycleProfileV1 {
    pub cycle_count: usize,
    pub completed_cycle_count: usize,
    pub total_rage_generated: u32,
    pub total_rage_gained: u32,
    pub total_rage_spent: u32,
    pub total_rage_overflow: u32,
    pub total_yuan_ge_gained_stacks: u32,
    pub total_yuan_ge_consumed_stacks: u32,
    pub total_yuan_ge_blocked_gain_events: usize,
    pub absolute_knives_by_rage_cost: BTreeMap<String, usize>,
    pub cycle_shapes: Vec<RotationCycleShapeV1>,
    pub cycles: Vec<RotationCycleObservationV1>,
    pub boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RotationCycleShapeV1 {
    pub signature: String,
    pub count: usize,
    pub cycle_numbers: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonBuffObservationV1 {
    pub name: String,
    pub coverage_percent: f64,
    pub average_stacks_while_active: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PauseRecoveryObservationV1 {
    pub pause_start: f64,
    pub pause_duration: f64,
    pub last_event_before_pause: Option<usize>,
    pub last_skill_before_pause: Option<String>,
    pub stance_before_pause: Option<String>,
    pub shield_flying_seconds_at_pause: Option<f64>,
    pub first_event_after_pause: Option<usize>,
    pub first_skill_after_pause: Option<String>,
    pub stance_at_resume: Option<String>,
    pub shield_flying_seconds_at_resume: Option<f64>,
    pub resume_delay_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTimelineDiagnosticsV1 {
    pub active_event_count: usize,
    pub gcd_gap_count: usize,
    pub gcd_gap_seconds: f64,
    pub recorded_wait_count: usize,
    pub recorded_wait_seconds: f64,
    pub rage_cap_observations: usize,
    pub rage_overflow_events: usize,
    pub rage_overflow_total: u32,
    pub rage_overflow_sources: Vec<RageOverflowSourceV1>,
    pub rage_generated: u32,
    pub rage_gained: u32,
    pub rage_spent: u32,
    pub completed_cycle_count: usize,
    pub absolute_knives_by_rage_cost: BTreeMap<String, usize>,
    pub yuan_ge_gained_stacks: u32,
    pub yuan_ge_consumed_stacks: u32,
    pub yuan_ge_blocked_gain_events: usize,
    pub buffs: Vec<ComparisonBuffObservationV1>,
    pub pause_recovery: Vec<PauseRecoveryObservationV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineAnalysis {
    pub fingerprint: u64,
    pub fingerprint_hex: String,
    pub fight_time: f64,
    pub active_event_count: usize,
    pub triggered_event_count: usize,
    pub skills: Vec<SkillDamageSummary>,
    pub total_cd_wait_seconds: f64,
    pub cd_waits: Vec<WaitEvidence>,
    pub total_observed_gcd_gap_seconds: f64,
    pub gcd_gaps: Vec<GcdGapEvidence>,
    pub rage: Option<RageObservation>,
    pub buff_coverage: Vec<BuffCoverage>,
    pub skipped: Vec<SkippedSkill>,
    pub diagnostic_profile: RotationDiagnosticProfile,
    pub rotation_cycles: RotationCycleProfileV1,
    pub macro_line_stats: Vec<crate::macro_eval::MacroLineExecutionStats>,
    pub limitations: Vec<String>,
}

pub struct TimelineExecution {
    pub evidence: EvidenceEnvelopeV1<TimelineAnalysis>,
}

pub fn comparison_timeline_diagnostics(
    response: &crate::SimulateResponse,
    pauses: &[(f64, f64)],
) -> ComparisonTimelineDiagnosticsV1 {
    let gcd_gaps = collect_gcd_gaps(&response.timeline);
    let waits = collect_cd_waits(&response.timeline);
    let rage = observe_rage(&response.timeline, response.rage);
    let cycles = build_rotation_cycles(&response.timeline);
    let mut buffs = response
        .buff_timeline
        .iter()
        .filter_map(|track| calculate_buff_coverage(track, response.fight_time))
        .map(|coverage| ComparisonBuffObservationV1 {
            name: coverage.name,
            coverage_percent: coverage.coverage_percent,
            average_stacks_while_active: coverage.average_stacks_while_active,
        })
        .collect::<Vec<_>>();
    buffs.sort_by(|left, right| left.name.cmp(&right.name));
    ComparisonTimelineDiagnosticsV1 {
        active_event_count: response
            .timeline
            .iter()
            .filter(|event| !event.triggered)
            .count(),
        gcd_gap_count: gcd_gaps.len(),
        gcd_gap_seconds: gcd_gaps.iter().map(|gap| gap.observed_gap_seconds).sum(),
        recorded_wait_count: waits.len(),
        recorded_wait_seconds: waits.iter().map(|wait| wait.wait_seconds).sum(),
        rage_cap_observations: rage.as_ref().map_or(0, |rage| rage.at_cap_observations),
        rage_overflow_events: rage.as_ref().map_or(0, |rage| rage.overflow_events),
        rage_overflow_total: rage.as_ref().map_or(0, |rage| rage.overflow_total),
        rage_overflow_sources: rage
            .as_ref()
            .map(|rage| rage.overflow_sources.clone())
            .unwrap_or_default(),
        rage_generated: cycles.total_rage_generated,
        rage_gained: cycles.total_rage_gained,
        rage_spent: cycles.total_rage_spent,
        completed_cycle_count: cycles.completed_cycle_count,
        absolute_knives_by_rage_cost: cycles.absolute_knives_by_rage_cost,
        yuan_ge_gained_stacks: cycles.total_yuan_ge_gained_stacks,
        yuan_ge_consumed_stacks: cycles.total_yuan_ge_consumed_stacks,
        yuan_ge_blocked_gain_events: cycles.total_yuan_ge_blocked_gain_events,
        buffs,
        pause_recovery: observe_pause_recovery(&response.timeline, pauses),
    }
}

fn observe_pause_recovery(
    timeline: &[CastEvent],
    pauses: &[(f64, f64)],
) -> Vec<PauseRecoveryObservationV1> {
    let active = timeline
        .iter()
        .filter(|event| !event.triggered)
        .enumerate()
        .collect::<Vec<_>>();
    pauses
        .iter()
        .map(|(pause_start, pause_duration)| {
            let pause_end = pause_start + pause_duration;
            let before = active
                .iter()
                .rev()
                .find(|(_, event)| event.cast_time <= *pause_start);
            let after = active
                .iter()
                .find(|(_, event)| event.cast_time + TIME_EPSILON >= pause_end);
            PauseRecoveryObservationV1 {
                pause_start: *pause_start,
                pause_duration: *pause_duration,
                last_event_before_pause: before.map(|(index, _)| index + 1),
                last_skill_before_pause: before.map(|(_, event)| event.name.clone()),
                stance_before_pause: before
                    .and_then(|(_, event)| event.state_after.as_ref())
                    .map(|state| stance_name(state.stance).to_string()),
                shield_flying_seconds_at_pause: before.and_then(|(_, event)| {
                    remaining_buff_at_time(
                        event.state_after.as_ref()?,
                        "盾飞",
                        pause_start - event.cast_time,
                    )
                }),
                first_event_after_pause: after.map(|(index, _)| index + 1),
                first_skill_after_pause: after.map(|(_, event)| event.name.clone()),
                stance_at_resume: after
                    .and_then(|(_, event)| event.state_before.as_ref())
                    .map(|state| stance_name(state.stance).to_string()),
                shield_flying_seconds_at_resume: after.and_then(|(_, event)| {
                    remaining_buff_at_time(event.state_before.as_ref()?, "盾飞", 0.0)
                }),
                resume_delay_seconds: after
                    .map(|(_, event)| (event.cast_time - pause_end).max(0.0)),
            }
        })
        .collect()
}

fn remaining_buff_at_time(state: &EventState, name: &str, elapsed: f64) -> Option<f64> {
    state
        .buffs
        .iter()
        .find(|buff| buff.name == name)
        .map(|buff| (buff.remaining - elapsed).max(0.0))
}

pub fn analyze_timeline(
    trace_id: &str,
    simulation: &SimulationExecution,
    provenance: &ToolProvenance,
) -> Result<TimelineExecution, ToolError> {
    let started = Instant::now();
    validate_trace_id(trace_id)?;
    require_full_timeline(simulation)?;

    let response = &simulation.response;
    let active_event_count = response
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .count();
    let triggered_event_count = response.timeline.len().saturating_sub(active_event_count);
    let cd_waits = collect_cd_waits(&response.timeline);
    let total_cd_wait_seconds = cd_waits.iter().map(|wait| wait.wait_seconds).sum();
    let gcd_gaps = collect_gcd_gaps(&response.timeline);
    let total_observed_gcd_gap_seconds = gcd_gaps.iter().map(|gap| gap.observed_gap_seconds).sum();
    let mut buff_coverage: Vec<_> = response
        .buff_timeline
        .iter()
        .filter_map(|track| calculate_buff_coverage(track, response.fight_time))
        .collect();
    // The simulator builds buff tracks from a hash map. UI timeline order can
    // contain equal priorities, so source iteration order is not a stable
    // evidence identity. The Agent result uses the stable buff ID as its key.
    buff_coverage.sort_by_key(|coverage| coverage.buff_id);

    let skipped = response
        .skipped
        .iter()
        .map(|(sequence_index, reason)| SkippedSkill {
            sequence_index: *sequence_index,
            reason: reason.clone(),
        })
        .collect::<Vec<_>>();
    let rage = observe_rage(&response.timeline, response.rage);
    let diagnostic_profile = build_diagnostic_profile(
        &response.timeline,
        response.fight_time,
        &simulation.evidence.result.skills,
        &cd_waits,
        &gcd_gaps,
        rage.as_ref(),
        &skipped,
    );
    let rotation_cycles = build_rotation_cycles(&response.timeline);
    let result = TimelineAnalysis {
        fingerprint: response.fingerprint,
        fingerprint_hex: format!("{:016x}", response.fingerprint),
        fight_time: response.fight_time,
        active_event_count,
        triggered_event_count,
        skills: simulation.evidence.result.skills.clone(),
        total_cd_wait_seconds,
        cd_waits,
        total_observed_gcd_gap_seconds,
        gcd_gaps,
        rage,
        buff_coverage,
        skipped,
        diagnostic_profile,
        rotation_cycles,
        macro_line_stats: response.macro_line_stats.clone(),
        limitations: vec![
            "rage_cap_observations_do_not_measure_lost_rage".to_string(),
            "timeline_correlations_are_not_causal_without_ab_test".to_string(),
            "buff_coverage_uses_visible_buff_timeline_only".to_string(),
        ],
    };
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        ANALYZE_TIMELINE,
        &simulation.evidence.scenario_hash,
        serde_json::json!({
            "source_evidence_id": simulation.evidence.evidence_id,
            "fingerprint": response.fingerprint,
        }),
        result,
        provenance,
        elapsed_ms(started),
    )?;

    Ok(TimelineExecution { evidence })
}

fn build_rotation_cycles(timeline: &[CastEvent]) -> RotationCycleProfileV1 {
    let active = timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect::<Vec<_>>();
    let mut cycles = Vec::new();
    let mut start = 0usize;
    for (index, event) in active.iter().enumerate() {
        if event.name.split('·').next().unwrap_or(&event.name) == "盾回" {
            cycles.push(summarize_rotation_cycle(
                cycles.len() + 1,
                &active[start..=index],
                start,
            ));
            start = index + 1;
        }
    }
    if start < active.len() {
        cycles.push(summarize_rotation_cycle(
            cycles.len() + 1,
            &active[start..],
            start,
        ));
    }
    let completed_cycle_count = cycles
        .iter()
        .filter(|cycle| cycle.completed_by_shield_return)
        .count();
    let total_rage_generated = cycles.iter().map(|cycle| cycle.rage_generated).sum();
    let total_rage_gained = cycles.iter().map(|cycle| cycle.rage_gained).sum();
    let total_rage_spent = cycles.iter().map(|cycle| cycle.rage_spent).sum();
    let total_rage_overflow = cycles.iter().map(|cycle| cycle.rage_overflow).sum();
    let total_yuan_ge_gained_stacks = cycles.iter().map(|cycle| cycle.yuan_ge_gained_stacks).sum();
    let total_yuan_ge_consumed_stacks = cycles
        .iter()
        .map(|cycle| cycle.yuan_ge_consumed_stacks)
        .sum();
    let total_yuan_ge_blocked_gain_events = cycles
        .iter()
        .map(|cycle| cycle.yuan_ge_blocked_gain_event_numbers.len())
        .sum();
    let mut absolute_knives_by_rage_cost = BTreeMap::<String, usize>::new();
    let mut shapes = BTreeMap::<String, Vec<usize>>::new();
    for cycle in &cycles {
        for knife in &cycle.absolute_knives {
            let label = knife
                .rage_cost
                .map(|cost| format!("{cost}怒"))
                .unwrap_or_else(|| "未记录".to_string());
            *absolute_knives_by_rage_cost.entry(label).or_default() += 1;
        }
        let costs = cycle
            .absolute_knives
            .iter()
            .map(|knife| {
                knife
                    .rage_cost
                    .map(|cost| cost.to_string())
                    .unwrap_or_else(|| "?".to_string())
            })
            .collect::<Vec<_>>()
            .join("+");
        let signature = format!(
            "{}盾击/{}阵云/{}斩刀/绝刀耗怒[{}]{}",
            cycle.shield_strike_count,
            cycle.zhen_yun_count,
            cycle.slash_count,
            costs,
            if cycle.completed_by_shield_return {
                ""
            } else {
                "/未完成"
            }
        );
        shapes
            .entry(signature)
            .or_default()
            .push(cycle.cycle_number);
    }
    let mut cycle_shapes = shapes
        .into_iter()
        .map(|(signature, cycle_numbers)| RotationCycleShapeV1 {
            count: cycle_numbers.len(),
            signature,
            cycle_numbers,
        })
        .collect::<Vec<_>>();
    cycle_shapes.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.signature.cmp(&right.signature))
    });
    RotationCycleProfileV1 {
        cycle_count: cycles.len(),
        completed_cycle_count,
        total_rage_generated,
        total_rage_gained,
        total_rage_spent,
        total_rage_overflow,
        total_yuan_ge_gained_stacks,
        total_yuan_ge_consumed_stacks,
        total_yuan_ge_blocked_gain_events,
        absolute_knives_by_rage_cost,
        cycle_shapes,
        cycles,
        boundary: "以每次主动盾回作为一轮结束；末尾未盾回的片段保留为未完成轮次。".to_string(),
    }
}

fn summarize_rotation_cycle(
    cycle_number: usize,
    events: &[&CastEvent],
    global_start: usize,
) -> RotationCycleObservationV1 {
    let first = events.first().expect("rotation cycle is never empty");
    let last = events.last().expect("rotation cycle is never empty");
    let mut key_sequence = Vec::new();
    let mut absolute_knives = Vec::new();
    let mut shield_strike_count = 0usize;
    let mut zhen_yun_count = 0usize;
    let mut slash_count = 0usize;
    let mut rage_generated = 0u32;
    let mut rage_gained = 0u32;
    let mut rage_spent = 0u32;
    let mut rage_overflow = 0u32;
    let mut yuan_ge_gained_stacks = 0u32;
    let mut yuan_ge_consumed_stacks = 0u32;
    let mut yuan_ge_at_cap_before_events = 0usize;
    let mut yuan_ge_blocked_gain_event_numbers = Vec::new();

    for (offset, event) in events.iter().enumerate() {
        let event_number = global_start + offset + 1;
        let base_name = event.name.split('·').next().unwrap_or(&event.name);
        match base_name {
            "盾击" => shield_strike_count += 1,
            "阵云结晦" => zhen_yun_count += 1,
            "斩刀" => slash_count += 1,
            _ => {}
        }
        if matches!(
            base_name,
            "盾击" | "阵云结晦" | "斩刀" | "绝刀" | "盾飞" | "盾回"
        ) {
            key_sequence.push(RotationCycleSkillV1 {
                event_number,
                cast_time: event.cast_time,
                skill_name: event.name.clone(),
                rage_before: event.state_before.as_ref().map(|state| state.rage),
                rage_after: event
                    .state_after
                    .as_ref()
                    .map(|state| state.rage)
                    .or(event.rage_after),
            });
        }
        let yuan_ge_before = event.state_before.as_ref().and_then(yuan_ge_stacks);
        let yuan_ge_after = event.state_after.as_ref().and_then(yuan_ge_stacks);
        if yuan_ge_before.is_some_and(|stacks| stacks >= 12) {
            yuan_ge_at_cap_before_events += 1;
            if base_name == "盾击" && yuan_ge_after.is_some_and(|stacks| stacks >= 12) {
                yuan_ge_blocked_gain_event_numbers.push(event_number);
            }
        }
        match (yuan_ge_before, yuan_ge_after) {
            (Some(before), Some(after)) if after > before => {
                yuan_ge_gained_stacks += after - before;
            }
            (Some(before), Some(after)) if before > after => {
                yuan_ge_consumed_stacks += before - after;
            }
            _ => {}
        }
        if base_name == "绝刀" {
            let semantics = absolute_knife_semantics(event);
            absolute_knives.push(AbsoluteKnifeObservationV1 {
                event_number,
                skill_name: event.name.clone(),
                rage_before: event.state_before.as_ref().map(|state| state.rage),
                rage_cost: event.rage_cost,
                yuan_ge_before,
                yuan_ge_after,
                triggered_yuan_ge_blood_shadow: semantics.triggered_yuan_ge_blood_shadow,
                without_yuan_ge_blood_shadow: semantics.without_yuan_ge_blood_shadow,
                blood_rage_active: semantics.blood_rage_active,
                kuang_jue_active: semantics.kuang_jue_active,
                tian_xia_hong_yuan_active: semantics.tian_xia_hong_yuan_active,
                prior_mechanic_landmarks: prior_mechanic_landmarks(events, offset, global_start),
            });
        }
        rage_generated = rage_generated.saturating_add(event.rage_generated.unwrap_or(0));
        rage_gained = rage_gained.saturating_add(event.rage_gained.unwrap_or(0));
        rage_spent = rage_spent.saturating_add(event.rage_spent.unwrap_or(0));
        rage_overflow = rage_overflow.saturating_add(event.rage_overflow.unwrap_or(0));
    }

    RotationCycleObservationV1 {
        cycle_number,
        start_event_number: global_start + 1,
        end_event_number: global_start + events.len(),
        start_time: first.cast_time,
        end_time: last.cast_time,
        completed_by_shield_return: last.name.split('·').next().unwrap_or(&last.name) == "盾回",
        key_sequence,
        shield_strike_count,
        zhen_yun_count,
        slash_count,
        absolute_knives,
        rage_generated,
        rage_gained,
        rage_spent,
        rage_overflow,
        yuan_ge_gained_stacks,
        yuan_ge_consumed_stacks,
        yuan_ge_at_cap_before_events,
        yuan_ge_blocked_gain_event_numbers,
    }
}

fn yuan_ge_stacks(state: &EventState) -> Option<u32> {
    Some(
        state
            .buffs
            .iter()
            .find(|buff| buff.name == "援戈" || buff.buff_id == 27030)
            .map(|buff| buff.stacks)
            .unwrap_or(0),
    )
}

fn state_has_buff(state: &EventState, name: &str) -> bool {
    state
        .buffs
        .iter()
        .any(|buff| buff.name == name || buff.name.starts_with(name))
}

fn is_mechanic_landmark(skill_name: &str) -> bool {
    matches!(
        skill_name,
        "业火焚城" | "血怒" | "盾飞" | "盾回" | "斩刀" | "阵云结晦"
    )
}

fn prior_mechanic_landmarks(
    active: &[&CastEvent],
    matched_index: usize,
    global_start: usize,
) -> Vec<MechanicLandmarkV1> {
    let matched_time = active[matched_index].cast_time;
    let mut nearest = BTreeMap::<String, (usize, &CastEvent)>::new();
    for (index, event) in active[..matched_index].iter().copied().enumerate() {
        let base_name = event.name.split('·').next().unwrap_or(&event.name);
        if is_mechanic_landmark(base_name) {
            nearest.insert(base_name.to_string(), (index, event));
        }
    }
    let mut landmarks = nearest
        .into_values()
        .map(|(index, event)| MechanicLandmarkV1 {
            event_number: global_start + index + 1,
            cast_time: event.cast_time,
            skill_name: event.name.clone(),
            seconds_before: (matched_time - event.cast_time).max(0.0),
        })
        .collect::<Vec<_>>();
    landmarks.sort_by(|left, right| left.seconds_before.total_cmp(&right.seconds_before));
    landmarks
}

fn absolute_knife_semantics(event: &CastEvent) -> AbsoluteKnifeSemanticsV1 {
    let before = event
        .state_before
        .as_ref()
        .and_then(yuan_ge_stacks)
        .unwrap_or(0);
    let after = event
        .state_after
        .as_ref()
        .and_then(yuan_ge_stacks)
        .unwrap_or(0);
    let triggered = before > after;
    let state = event.state_before.as_ref();
    AbsoluteKnifeSemanticsV1 {
        yuan_ge_before: before,
        yuan_ge_after: after,
        triggered_yuan_ge_blood_shadow: triggered,
        without_yuan_ge_blood_shadow: !triggered,
        blood_rage_active: state.is_some_and(|state| {
            state_has_buff(state, "血怒") || state_has_buff(state, "血怒·惊涌")
        }),
        kuang_jue_active: state.is_some_and(|state| state_has_buff(state, "狂绝")),
        tian_xia_hong_yuan_active: state.is_some_and(|state| state_has_buff(state, "天下宏愿")),
    }
}

pub fn inspect_timeline_events(
    trace_id: &str,
    simulation: &SimulationExecution,
    query: &TimelineEventQueryV1,
    provenance: &ToolProvenance,
) -> Result<TimelineEventInspectionExecution, ToolError> {
    let started = Instant::now();
    validate_trace_id(trace_id)?;
    require_full_timeline(simulation)?;

    let active = simulation
        .response
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect::<Vec<_>>();
    let mut matching = match query.selector {
        TimelineEventSelector::RageCap => active
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                let before = event.state_before.as_ref().map(|state| state.rage);
                let after = event
                    .state_after
                    .as_ref()
                    .map(|state| state.rage)
                    .or(event.rage_after);
                (before.is_some_and(|rage| rage >= 100) || after.is_some_and(|rage| rage >= 100))
                    .then_some(index)
            })
            .collect::<Vec<_>>(),
        TimelineEventSelector::RageOverflow => active
            .iter()
            .enumerate()
            .filter_map(|(index, event)| (event.rage_overflow.unwrap_or(0) > 0).then_some(index))
            .collect::<Vec<_>>(),
        TimelineEventSelector::CooldownWait => active
            .iter()
            .enumerate()
            .filter_map(|(index, event)| (event.cd_wait > TIME_EPSILON).then_some(index))
            .collect::<Vec<_>>(),
        TimelineEventSelector::GcdGap => {
            let mut matches = Vec::new();
            let mut previous_main: Option<(usize, &CastEvent)> = None;
            for (index, event) in active.iter().copied().enumerate() {
                if !event.is_main {
                    continue;
                }
                if let Some((_, previous)) = previous_main {
                    let occupied = previous.channel_duration.unwrap_or(0.0).max(previous.gcd);
                    if event.cast_time - (previous.cast_time + occupied) > TIME_EPSILON {
                        matches.push(index);
                    }
                }
                previous_main = Some((index, event));
            }
            matches
        }
        TimelineEventSelector::Skill => {
            let expected = query.skill_name.as_deref().unwrap_or_default().trim();
            active
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    let base = event.name.split('·').next().unwrap_or(event.name.as_str());
                    (event.name == expected || base == expected).then_some(index)
                })
                .collect::<Vec<_>>()
        }
        TimelineEventSelector::Time => query
            .time_seconds
            .and_then(|time| {
                active
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| {
                        (left.cast_time - time)
                            .abs()
                            .total_cmp(&(right.cast_time - time).abs())
                    })
                    .map(|(index, _)| index)
            })
            .into_iter()
            .collect::<Vec<_>>(),
    };
    matching.retain(|index| {
        let event = active[*index];
        query.event_number.is_none_or(|number| number == index + 1)
            && query.rage_cost_below.is_none_or(|limit|
                event.rage_cost.is_some_and(|cost| cost > 0 && cost < limit))
    });
    matching.sort_unstable();
    matching.dedup();

    let total_matches = matching.len();
    let match_index = matching
        .iter()
        .take(64)
        .map(|index| {
            let event = active[*index];
            TimelineEventMatchV1 {
                operation_number: event.sequence_index.map(|index| index + 1),
                event_number: index + 1,
                cast_time: event.cast_time,
                skill_name: event.name.clone(),
                macro_page: event.macro_page,
                macro_line: event.macro_line,
                rage_before: event.state_before.as_ref().map(|state| state.rage),
                rage_after: event
                    .state_after
                    .as_ref()
                    .map(|state| state.rage)
                    .or(event.rage_after),
                rage_cost: event.rage_cost,
                rage_overflow: event.rage_overflow,
                rage_overflow_sources: event.rage_overflow_sources.clone(),
                rage_transactions: event.rage_transactions.clone(),
                rage_generated: event.rage_generated,
                rage_gained: event.rage_gained,
                rage_spent: event.rage_spent,
                buffs_before: event
                    .state_before
                    .as_ref()
                    .map(|state| filter_timeline_buffs(&state.buffs, &query.buff_names))
                    .unwrap_or_default(),
                prior_mechanic_landmarks: prior_mechanic_landmarks(&active, *index, 0),
                absolute_knife: (event.name.split('·').next() == Some("绝刀"))
                    .then(|| absolute_knife_semantics(event)),
            }
        })
        .collect::<Vec<_>>();
    let selected = matching
        .iter()
        .skip(query.start_match)
        .take(query.limit)
        .copied()
        .collect::<Vec<_>>();
    let windows = selected
        .into_iter()
        .map(|matched_index| {
            let start = matched_index.saturating_sub(query.context_radius);
            let end = (matched_index + query.context_radius + 1).min(active.len());
            TimelineEventWindowV1 {
                matched: timeline_event_view(
                    matched_index,
                    active[matched_index],
                    &query.buff_names,
                ),
                context: (start..end)
                    .map(|index| timeline_event_view(index, active[index], &query.buff_names))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    let consumed = query.start_match.saturating_add(windows.len());
    let next_start_match = (consumed < total_matches).then_some(consumed);
    let result = TimelineEventInspectionV1 {
        selector: query.selector,
        total_matches,
        match_index_truncated: total_matches > match_index.len(),
        match_index,
        start_match: query.start_match,
        next_start_match,
        window_selection: "requested_page".to_string(),
        windows,
        limitations: vec![
            "rage_at_cap_is_an_observation_not_measured_overflow".to_string(),
            "event_correlation_requires_comparison_for_causal_claims".to_string(),
        ],
    };
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        INSPECT_TIMELINE_EVENTS,
        &simulation.evidence.scenario_hash,
        serde_json::to_value(query).unwrap_or_else(|_| serde_json::json!({})),
        result,
        provenance,
        elapsed_ms(started),
    )?;
    Ok(TimelineEventInspectionExecution { evidence })
}

fn timeline_event_view(
    active_event_index: usize,
    event: &CastEvent,
    buff_names: &[String],
) -> TimelineEventViewV1 {
    TimelineEventViewV1 {
        operation_number: event.sequence_index.map(|index| index + 1),
        anchor_id: format!("timeline:active:{active_event_index}"),
        active_event_index,
        event_number: active_event_index + 1,
        cast_time: event.cast_time,
        skill_id: event.skill_id,
        skill_name: event.name.clone(),
        is_main: event.is_main,
        is_macro: event.is_macro,
        macro_page: event.macro_page,
        macro_line: event.macro_line,
        gcd_seconds: event.gcd,
        cooldown_wait_seconds: event.cd_wait,
        rage_delta: event.rage_delta,
        rage_overflow: event.rage_overflow,
        rage_overflow_sources: event.rage_overflow_sources.clone(),
        rage_transactions: event.rage_transactions.clone(),
        rage_generated: event.rage_generated,
        rage_gained: event.rage_gained,
        rage_spent: event.rage_spent,
        rage_cost: event.rage_cost,
        damage_total: event.damage_total,
        state_before: event
            .state_before
            .as_ref()
            .map(|state| timeline_state_view(state, buff_names)),
        state_after: event
            .state_after
            .as_ref()
            .map(|state| timeline_state_view(state, buff_names)),
    }
}

fn timeline_state_view(state: &EventState, buff_names: &[String]) -> TimelineStateViewV1 {
    TimelineStateViewV1 {
        rage: state.rage,
        stance: stance_name(state.stance).to_string(),
        buffs: filter_timeline_buffs(&state.buffs, buff_names),
        target_buffs: filter_timeline_buffs(&state.target_buffs, buff_names),
    }
}

fn filter_timeline_buffs(buffs: &[EventBuff], names: &[String]) -> Vec<TimelineBuffViewV1> {
    if names.is_empty() {
        return Vec::new();
    }
    buffs
        .iter()
        .filter(|buff| {
            names.iter().any(|expected| {
                let expected = expected.trim();
                !expected.is_empty()
                    && (buff.name == expected
                        || buff.name.starts_with(expected)
                        || expected.starts_with(&buff.name))
            })
        })
        .take(12)
        .map(|buff| TimelineBuffViewV1 {
            buff_id: buff.buff_id,
            name: buff.name.clone(),
            remaining_seconds: buff.remaining,
            stacks: buff.stacks,
        })
        .collect()
}

fn duration_observation(values: impl IntoIterator<Item = f64>) -> DurationObservation {
    let values = values.into_iter().collect::<Vec<_>>();
    DurationObservation {
        count: values.len(),
        total_seconds: values.iter().sum(),
        maximum_seconds: values.iter().copied().fold(0.0_f64, f64::max),
    }
}

fn stance_name(stance: Stance) -> &'static str {
    match stance {
        Stance::Any => "any",
        Stance::Shield => "shield",
        Stance::Blade => "blade",
        Stance::Wall => "wall",
        Stance::NotWall => "not_wall",
    }
}

fn diagnostic_signal(
    code: &str,
    summary: &str,
    evidence_paths: &[&str],
    interpretation_boundary: &str,
) -> DiagnosticSignal {
    DiagnosticSignal {
        code: code.to_string(),
        summary: summary.to_string(),
        evidence_paths: evidence_paths
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        interpretation_boundary: interpretation_boundary.to_string(),
    }
}

fn build_diagnostic_profile(
    timeline: &[CastEvent],
    fight_time: f64,
    skills: &[SkillDamageSummary],
    cd_waits: &[WaitEvidence],
    gcd_gaps: &[GcdGapEvidence],
    rage: Option<&RageObservation>,
    skipped: &[SkippedSkill],
) -> RotationDiagnosticProfile {
    let active = timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect::<Vec<_>>();
    let main_gcd_cast_count = active.iter().filter(|event| event.is_main).count();
    let active_skill_variety = active
        .iter()
        .map(|event| (event.skill_id, event.name.as_str()))
        .collect::<BTreeSet<_>>()
        .len();
    let damaging_skill_variety = skills
        .iter()
        .filter(|skill| skill.total_damage > 0.0)
        .map(|skill| (skill.skill_id, skill.name.as_str(), skill.triggered))
        .collect::<BTreeSet<_>>()
        .len();
    let input_mode = if active.iter().any(|event| event.is_macro) {
        "macro"
    } else {
        "manual_sequence"
    };
    let mut active_casts_by_stance = BTreeMap::<String, usize>::new();
    let mut last_stance = None;
    let mut observed_transitions = 0usize;
    for event in &active {
        let stance = event
            .state_before
            .as_ref()
            .map(|state| state.stance)
            .or_else(|| event.state_after.as_ref().map(|state| state.stance));
        if let Some(stance) = stance {
            *active_casts_by_stance
                .entry(stance_name(stance).to_string())
                .or_default() += 1;
            if last_stance.is_some_and(|previous| previous != stance) {
                observed_transitions += 1;
            }
            last_stance = Some(stance);
        }
    }
    let cadence_gaps = duration_observation(gcd_gaps.iter().map(|gap| gap.observed_gap_seconds));
    let cooldown_waits = duration_observation(cd_waits.iter().map(|wait| wait.wait_seconds));
    let mut observed_strengths = Vec::new();
    let mut observed_risks = Vec::new();
    if main_gcd_cast_count > 1 && cadence_gaps.count == 0 {
        observed_strengths.push(diagnostic_signal(
            "no_observed_main_gcd_gap",
            "本次时间轴未观察到主技能 GCD 就绪后的额外空档。",
            &["/result/diagnostic_profile/cadence_gaps/count"],
            "这只说明模拟时间轴连续，不证明循环已经最优。",
        ));
    } else if cadence_gaps.count > 0 {
        observed_risks.push(diagnostic_signal(
            "observed_main_gcd_gaps",
            "本次时间轴观察到主技能 GCD 就绪后的额外空档。",
            &[
                "/result/diagnostic_profile/cadence_gaps/count",
                "/result/diagnostic_profile/cadence_gaps/total_seconds",
                "/result/diagnostic_profile/cadence_gaps/maximum_seconds",
            ],
            "空档是现象，不自动证明由宏、手法、冷却或资源中的哪一项造成。",
        ));
    }
    if cooldown_waits.count == 0 && !active.is_empty() {
        observed_strengths.push(diagnostic_signal(
            "no_observed_cooldown_wait",
            "本次时间轴没有记录释放前等待。",
            &["/result/diagnostic_profile/cooldown_waits/count"],
            "没有释放前等待不等于技能顺序、资源转化或增益覆盖已经合理。",
        ));
    } else if cooldown_waits.count > 0 {
        observed_risks.push(diagnostic_signal(
            "observed_cooldown_wait",
            "本次时间轴记录了释放前等待。",
            &[
                "/result/diagnostic_profile/cooldown_waits/count",
                "/result/diagnostic_profile/cooldown_waits/total_seconds",
                "/result/diagnostic_profile/cooldown_waits/maximum_seconds",
            ],
            "分类字段用于区分宏决策等待与手动序列的技能可用性等待；是否为循环问题仍需结合相邻技能和资源。",
        ));
    }
    if skipped.is_empty() {
        observed_strengths.push(diagnostic_signal(
            "no_skipped_input",
            "当前输入没有被模拟器标记为跳过的操作。",
            &["/result/skipped"],
            "未跳过只说明输入可执行，不证明每个操作时机合理。",
        ));
    } else {
        observed_risks.push(diagnostic_signal(
            "skipped_input",
            "当前输入包含被模拟器跳过的操作。",
            &["/result/skipped"],
            "需逐项读取跳过原因，不能把所有跳过都归为玩家失误。",
        ));
    }
    if let Some(rage) = rage {
        if rage.at_cap_observations == 0 {
            observed_strengths.push(diagnostic_signal(
                "rage_cap_not_observed",
                "本次采样没有观察到怒气处于上限。",
                &["/result/rage/at_cap_observations"],
                "离散采样未触顶不等于已证明不存在瞬时怒气浪费。",
            ));
        } else {
            observed_risks.push(diagnostic_signal(
                "rage_cap_observed",
                "本次采样观察到怒气处于上限。",
                &[
                    "/result/rage/at_cap_observations",
                    "/result/rage/sample_count",
                ],
                "触顶样本不等于已测得具体损失怒气，需结合事件前后状态或候选实验。",
            ));
        }
        if rage.overflow_total > 0 {
            observed_risks.push(diagnostic_signal(
                "measured_rage_overflow",
                "本次时间轴记录到技能结算时怒气被上限实际截断。",
                &[
                    "/result/rage/overflow_events",
                    "/result/rage/overflow_total",
                ],
                "这是已测资源损失；是否值得改循环仍取决于同场景对照后的净收益。",
            ));
        }
    }

    RotationDiagnosticProfile {
        input_mode: input_mode.to_string(),
        active_cast_count: active.len(),
        main_gcd_cast_count,
        active_casts_per_minute: if fight_time > 0.0 {
            active.len() as f64 / fight_time * 60.0
        } else {
            0.0
        },
        active_skill_variety,
        damaging_skill_variety,
        cadence_gaps,
        cooldown_waits,
        stance: StanceObservation {
            active_casts_by_stance,
            observed_transitions,
        },
        observed_strengths,
        observed_risks,
    }
}

fn require_full_timeline(simulation: &SimulationExecution) -> Result<(), ToolError> {
    let response = &simulation.response;
    if response.skill_count > 0 && response.timeline.is_empty() {
        return Err(ToolError::TimelineDetailsUnavailable);
    }
    let has_incomplete_active_state = response.timeline.iter().any(|event| {
        !event.triggered && (event.state_before.is_none() || event.state_after.is_none())
    });
    if has_incomplete_active_state {
        return Err(ToolError::TimelineDetailsUnavailable);
    }
    Ok(())
}

fn collect_cd_waits(timeline: &[CastEvent]) -> Vec<WaitEvidence> {
    timeline
        .iter()
        .filter(|event| !event.triggered)
        .enumerate()
        .filter(|(_, event)| event.cd_wait > TIME_EPSILON)
        .map(|(active_index, event)| WaitEvidence {
            event_number: active_index + 1,
            cast_time: event.cast_time,
            skill_id: event.skill_id,
            skill_name: event.name.clone(),
            wait_seconds: event.cd_wait,
            classification: if event.is_macro {
                "macro_decision_wait"
            } else {
                "requested_skill_availability_wait"
            }
            .to_string(),
        })
        .collect()
}

fn collect_gcd_gaps(timeline: &[CastEvent]) -> Vec<GcdGapEvidence> {
    let mut main_events: Vec<_> = timeline
        .iter()
        .filter(|event| !event.triggered)
        .enumerate()
        .filter(|(_, event)| event.is_main)
        .map(|(active_index, event)| (active_index + 1, event))
        .collect();
    main_events.sort_by(|left, right| left.1.cast_time.total_cmp(&right.1.cast_time));

    main_events
        .windows(2)
        .filter_map(|pair| {
            let (previous_event_number, previous) = pair[0];
            let (next_event_number, next) = pair[1];
            let occupied = previous.channel_duration.unwrap_or(0.0).max(previous.gcd);
            let expected_ready_time = previous.cast_time + occupied;
            let observed_gap_seconds = next.cast_time - expected_ready_time;
            (observed_gap_seconds > TIME_EPSILON).then(|| GcdGapEvidence {
                previous_event_number,
                previous_cast_time: previous.cast_time,
                previous_skill_id: previous.skill_id,
                previous_skill_name: previous.name.clone(),
                next_cast_time: next.cast_time,
                next_event_number,
                next_skill_id: next.skill_id,
                next_skill_name: next.name.clone(),
                expected_ready_time,
                observed_gap_seconds,
                classification: if next.cd_wait > TIME_EPSILON {
                    if next.is_macro {
                        "macro_decision_wait"
                    } else {
                        "requested_skill_availability_wait"
                    }
                } else if next.is_macro {
                    "macro_condition_or_priority_gap"
                } else {
                    "input_timing_gap"
                }
                .to_string(),
            })
        })
        .collect()
}

fn observe_rage(timeline: &[CastEvent], ending: i32) -> Option<RageObservation> {
    let mut samples = Vec::new();
    for event in timeline.iter().filter(|event| !event.triggered) {
        if let Some(state) = &event.state_before {
            samples.push(state.rage);
        }
        if let Some(state) = &event.state_after {
            samples.push(state.rage);
        } else if let Some(rage) = event.rage_after {
            samples.push(rage);
        }
    }
    samples.push(ending);
    let minimum = samples.iter().copied().min()?;
    let maximum = samples.iter().copied().max()?;
    let at_cap_observations = samples.iter().filter(|&&rage| rage >= 100).count();
    let overflow_events = timeline
        .iter()
        .filter(|event| !event.triggered && event.rage_overflow.unwrap_or(0) > 0)
        .count();
    let overflow_total = timeline
        .iter()
        .filter(|event| !event.triggered)
        .filter_map(|event| event.rage_overflow)
        .sum();
    let generated_before_cap = timeline
        .iter()
        .filter(|event| !event.triggered)
        .filter_map(|event| event.rage_generated)
        .sum();
    let gained_after_cap = timeline
        .iter()
        .filter(|event| !event.triggered)
        .filter_map(|event| event.rage_gained)
        .sum();
    let spent = timeline
        .iter()
        .filter(|event| !event.triggered)
        .filter_map(|event| event.rage_spent)
        .sum();
    let mut sources = BTreeMap::<String, (BTreeSet<String>, usize, u32, Vec<usize>)>::new();
    for (index, event) in timeline.iter().filter(|event| !event.triggered).enumerate() {
        let overflow = event.rage_overflow.unwrap_or(0);
        if overflow == 0 {
            continue;
        }
        let attributed = if event.rage_overflow_sources.is_empty() {
            vec![RageOverflowCause {
                source: "未标注怒气来源".to_string(),
                amount: overflow,
            }]
        } else {
            event.rage_overflow_sources.clone()
        };
        for cause in attributed {
            let entry = sources.entry(cause.source).or_default();
            entry.0.insert(event.name.clone());
            entry.1 += 1;
            entry.2 = entry.2.saturating_add(cause.amount);
            entry.3.push(index + 1);
        }
    }
    let mut overflow_sources = sources
        .into_iter()
        .map(
            |(rage_source, (active_skill_names, event_count, overflow_total, event_numbers))| {
                RageOverflowSourceV1 {
                    rage_source,
                    active_skill_names: active_skill_names.into_iter().collect(),
                    event_count,
                    overflow_total,
                    event_numbers,
                }
            },
        )
        .collect::<Vec<_>>();
    overflow_sources.sort_by(|left, right| {
        right
            .overflow_total
            .cmp(&left.overflow_total)
            .then_with(|| left.rage_source.cmp(&right.rage_source))
    });
    Some(RageObservation {
        minimum,
        maximum,
        ending,
        sample_count: samples.len(),
        at_cap_observations,
        overflow_events,
        overflow_total,
        generated_before_cap,
        gained_after_cap,
        spent,
        overflow_percent_of_generated: if generated_before_cap > 0 {
            overflow_total as f64 / generated_before_cap as f64 * 100.0
        } else {
            0.0
        },
        overflow_sources,
    })
}

fn calculate_buff_coverage(track: &BuffTimelineTrack, fight_time: f64) -> Option<BuffCoverage> {
    if !fight_time.is_finite() || fight_time <= 0.0 {
        return None;
    }
    let mut events: Vec<_> = track.events.iter().collect();
    events.sort_by(|left, right| left.time.total_cmp(&right.time));

    let mut active_since = None;
    let mut intervals = Vec::new();
    let mut activation_count = 0;
    let mut unmatched_close_events = 0;
    let mut current_stacks = 0u32;
    let mut maximum_stacks_observed = 0u32;
    let mut last_stack_time = 0.0f64;
    let mut stack_seconds = 0.0f64;
    for event in events {
        let time = event.time.clamp(0.0, fight_time);
        if time > last_stack_time {
            stack_seconds += current_stacks as f64 * (time - last_stack_time);
            last_stack_time = time;
        }
        match event.event_type.as_str() {
            "gain" => {
                if active_since.is_none() {
                    active_since = Some(time);
                    activation_count += 1;
                }
            }
            "expire" | "remove" => {
                if let Some(start) = active_since.take() {
                    if time > start {
                        intervals.push(TimeInterval { start, end: time });
                    }
                } else {
                    unmatched_close_events += 1;
                }
            }
            _ => {}
        }
        current_stacks = buff_stacks_after_timeline_event(track.buff_id, event, current_stacks);
        maximum_stacks_observed = maximum_stacks_observed.max(current_stacks);
    }
    if fight_time > last_stack_time {
        stack_seconds += current_stacks as f64 * (fight_time - last_stack_time);
    }
    let open_at_fight_end = active_since.is_some();
    if let Some(start) = active_since {
        if fight_time > start {
            intervals.push(TimeInterval {
                start,
                end: fight_time,
            });
        }
    }
    if intervals.is_empty() && activation_count == 0 {
        return None;
    }
    let active_seconds: f64 = intervals
        .iter()
        .map(|interval| interval.end - interval.start)
        .sum();
    Some(BuffCoverage {
        buff_id: track.buff_id,
        name: track.name.clone(),
        active_seconds,
        coverage_percent: active_seconds / fight_time * 100.0,
        average_stacks_while_active: if active_seconds > 0.0 {
            stack_seconds / active_seconds
        } else {
            0.0
        },
        maximum_stacks_observed,
        activation_count,
        open_at_fight_end,
        unmatched_close_events,
        intervals,
    })
}

fn buff_stacks_after_timeline_event(
    buff_id: u32,
    event: &crate::BuffTimelineEvent,
    current: u32,
) -> u32 {
    match event.event_type.as_str() {
        "remove" | "expire" => 0,
        "consume" => event
            .state
            .as_ref()
            .and_then(|state| {
                state
                    .buffs
                    .iter()
                    .chain(state.target_buffs.iter())
                    .find(|buff| buff.buff_id == buff_id)
                    .map(|buff| buff.stacks.saturating_sub(1))
            })
            .unwrap_or_else(|| current.saturating_sub(1)),
        "gain" | "stack" | "tick" => event
            .state
            .as_ref()
            .and_then(|state| {
                state
                    .buffs
                    .iter()
                    .chain(state.target_buffs.iter())
                    .find(|buff| buff.buff_id == buff_id)
                    .map(|buff| buff.stacks)
            })
            .unwrap_or(current),
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ScenarioSnapshotV1;
    use crate::{
        formations_file, load_formations, load_recipes, load_school_toml, load_skills,
        load_team_buffs, recipes_file, skills_dir, team_buffs_file, Attributes, BuffTimelineEvent,
        EventState, GameVersion, Mount, Stance, TargetConfig,
    };
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::Path;

    fn event(name: &str, skill_id: u32, cast_time: f64, gcd: f64, rage: i32) -> CastEvent {
        let state = EventState {
            rage,
            block_value: None,
            stance: Stance::Shield,
            buffs: Vec::new(),
            target_buffs: Vec::new(),
            skill_cds: Vec::new(),
        };
       CastEvent {
            sequence_index: None,
           name: name.to_string(),
            skill_id,
            cast_time,
            triggered: false,
            gcd,
            is_main: true,
            cd_wait: 0.0,
            channel_ticks: None,
            max_channel_ticks: None,
            channel_duration: None,
            timing_offset: None,
            max_timing_offset: None,
            available_buffs: None,
            is_macro: false,
            macro_page: None,
            macro_line: None,
            rage_after: Some(rage),
            rage_delta: None,
            rage_overflow: None,
            rage_overflow_sources: Vec::new(),
            rage_transactions: Vec::new(),
            rage_generated: None,
            rage_gained: None,
            rage_spent: None,
            rage_cost: None,
            state_before: Some(state.clone()),
            state_after: Some(state),
            damage: None,
            damage_normal: None,
            damage_crit: None,
            damage_total: None,
            runtime_stats: None,
            runtime_recipes: Vec::new(),
            override_attack_coeff: None,
            applied_recipes: Vec::new(),
        }
    }

    fn simulation_execution() -> SimulationExecution {
        let version = GameVersion::AnYingQianJi;
        let mount = Mount::FenShanJin;
        let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(version)));
        let formations = load_formations(Path::new(&formations_file(version)));
        let context = super::super::tools::SimulatorContext {
            game_version: version,
            mount,
            constants,
            skills: &skills,
            talents: &[],
            recipes: &recipes,
            team_buffs: &team_buffs,
            formations: &formations,
        };
        let request = crate::SimulateRequest {
            haste_level: 42_087,
            sequence: vec!["盾击".to_string(), "盾压".to_string()],
            talents: Vec::new(),
            channel_ticks: HashMap::new(),
            timing_offsets: HashMap::new(),
            network_delay: 100,
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
        };
        let snapshot = ScenarioSnapshotV1::capture(version, mount, request).unwrap();
        let mut budget = super::super::tools::ToolBudget::new(1);
        super::super::tools::simulate_scenario(
            "trace-source",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        )
        .unwrap()
    }

    fn diagnostic_fixture_execution() -> SimulationExecution {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/agent_diagnostic_eval/scenario.json"
        ))
        .unwrap();
        let mut request: crate::SimulateRequest =
            serde_json::from_value(fixture["simulation"].clone()).unwrap();
        let macro_slots = fixture["macro_slots"].as_u64().unwrap_or(1220) as usize;
        request.sequence = vec!["__macro__".to_string(); macro_slots];
        let version = GameVersion::AnYingQianJi;
        let mount = Mount::FenShanJin;
        let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(version)));
        let formations = load_formations(Path::new(&formations_file(version)));
        let context = super::super::tools::SimulatorContext {
            game_version: version,
            mount,
            constants,
            skills: &skills,
            talents: &[],
            recipes: &recipes,
            team_buffs: &team_buffs,
            formations: &formations,
        };
        let snapshot = ScenarioSnapshotV1::capture(version, mount, request).unwrap();
        let mut budget = super::super::tools::ToolBudget::new(1);
        super::super::tools::simulate_scenario(
            "trace-diagnostic-fixture",
            &snapshot,
            &context,
            &ToolProvenance::fixture(),
            &mut budget,
        )
        .unwrap()
    }

    #[test]
    fn diagnostic_fixture_attributes_overflow_and_absolute_knife_effects_semantically() {
        let simulation = diagnostic_fixture_execution();
        let rage = observe_rage(&simulation.response.timeline, simulation.response.rage).unwrap();
        let lin_guang = rage
            .overflow_sources
            .iter()
            .find(|source| source.rage_source == "麟光甲三层结算回怒")
            .expect("fixture should expose Lin Guang rage overflow as the actual cause");
        assert!(lin_guang.overflow_total > 0);
        assert!(lin_guang
            .active_skill_names
            .iter()
            .any(|name| name.starts_with("绝刀")));
        let lin_guang_event = simulation
            .response
            .timeline
            .iter()
            .find(|event| {
                event
                    .rage_overflow_sources
                    .iter()
                    .any(|cause| cause.source == "麟光甲三层结算回怒")
            })
            .expect("an exact overflow event");
        let transaction = lin_guang_event
            .rage_transactions
            .iter()
            .find(|transaction| transaction.source == "麟光甲三层结算回怒")
            .expect("ordered rage transaction");
        assert_eq!(transaction.requested_delta, 65);
        assert_eq!(transaction.applied_delta + transaction.overflow as i32, 65);
        assert_eq!(transaction.rage_after, 100);

        let cycles = build_rotation_cycles(&simulation.response.timeline);
        let knives = cycles
            .cycles
            .iter()
            .flat_map(|cycle| cycle.absolute_knives.iter())
            .collect::<Vec<_>>();
        assert!(knives
            .iter()
            .any(|knife| knife.without_yuan_ge_blood_shadow));
        assert!(knives
            .iter()
            .any(|knife| knife.triggered_yuan_ge_blood_shadow));
        assert!(knives
            .iter()
            .all(|knife| knife.without_yuan_ge_blood_shadow != knife.triggered_yuan_ge_blood_shadow));
        assert_eq!(
            cycles.total_rage_generated,
            cycles.total_rage_gained + cycles.total_rage_overflow
        );
        let inspection = inspect_timeline_events(
            "trace-absolute-knife-effect-fixture",
            &simulation,
           &TimelineEventQueryV1 {
                rage_cost_below: None,
                event_number: None,
               selector: TimelineEventSelector::Skill,
                skill_name: Some("绝刀".to_string()),
                time_seconds: None,
                start_match: 0,
                limit: 8,
                context_radius: 1,
                buff_names: Vec::new(),
            },
            &ToolProvenance::fixture(),
        )
        .unwrap();
        assert_eq!(inspection.evidence.result.total_matches, knives.len());
        assert_eq!(inspection.evidence.result.window_selection, "requested_page");
        assert!(inspection.evidence.result.match_index.iter().any(|event| {
            event
                .absolute_knife
                .as_ref()
                .is_some_and(|knife| knife.without_yuan_ge_blood_shadow)
        }));
        assert!(inspection.evidence.result.match_index.iter().any(|event| {
            event
                .absolute_knife
                .as_ref()
                .is_some_and(|knife| knife.triggered_yuan_ge_blood_shadow)
        }));
    }

    #[test]
    fn gcd_gap_uses_previous_occupied_time_and_reports_locations() {
        let timeline = vec![
            event("盾击", 1, 0.0, 1.0, 20),
            event("盾压", 2, 1.5, 1.0, 100),
            event("盾猛", 3, 2.5, 1.0, 80),
        ];
        let gaps = collect_gcd_gaps(&timeline);

        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].previous_skill_name, "盾击");
        assert_eq!(gaps[0].next_skill_name, "盾压");
        assert!((gaps[0].observed_gap_seconds - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn rage_observation_reports_cap_samples_without_claiming_overflow() {
        let timeline = vec![
            event("盾击", 1, 0.0, 1.0, 20),
            event("盾压", 2, 1.0, 1.0, 100),
        ];
        let rage = observe_rage(&timeline, 80).unwrap();

        assert_eq!(rage.minimum, 20);
        assert_eq!(rage.maximum, 100);
        assert_eq!(rage.ending, 80);
        assert_eq!(rage.at_cap_observations, 2);
        assert!(rage.overflow_sources.is_empty());
    }

    #[test]
    fn rage_observation_attributes_real_overflow_to_exact_active_skills() {
        let mut first = event("盾击", 1, 0.0, 1.0, 100);
        first.rage_overflow = Some(5);
        first.rage_overflow_sources = vec![RageOverflowCause {
            source: "盾击回怒".to_string(),
            amount: 5,
        }];
        let mut second = event("盾击", 1, 1.0, 1.0, 100);
        second.rage_overflow = Some(3);
        second.rage_overflow_sources = vec![RageOverflowCause {
            source: "盾击回怒".to_string(),
            amount: 3,
        }];
        let mut third = event("盾压", 2, 2.0, 1.0, 100);
        third.rage_overflow = Some(4);
        third.rage_overflow_sources = vec![RageOverflowCause {
            source: "盾压额外回怒".to_string(),
            amount: 4,
        }];

        let rage = observe_rage(&[first, second, third], 100).unwrap();

        assert_eq!(rage.overflow_events, 3);
        assert_eq!(rage.overflow_total, 12);
        assert_eq!(rage.overflow_sources[0].rage_source, "盾击回怒");
        assert_eq!(rage.overflow_sources[0].active_skill_names, vec!["盾击"]);
        assert_eq!(rage.overflow_sources[0].overflow_total, 8);
        assert_eq!(rage.overflow_sources[0].event_numbers, vec![1, 2]);
        assert_eq!(rage.overflow_sources[1].rage_source, "盾压额外回怒");
        assert_eq!(rage.overflow_sources[1].event_numbers, vec![3]);
    }

    #[test]
    fn event_inspection_returns_exact_distinct_occurrences_with_context() {
        let mut simulation = simulation_execution();
        simulation.response.timeline = vec![
            event("盾击", 1, 0.0, 1.0, 100),
            event("绝刀·50怒", 2, 1.0, 1.0, 50),
            event("盾击", 1, 2.0, 1.0, 100),
            event("绝刀·50怒", 2, 3.0, 1.0, 50),
        ];
       let query = TimelineEventQueryV1 {
            rage_cost_below: None,
            event_number: None,
           selector: TimelineEventSelector::Skill,
            skill_name: Some("绝刀".to_string()),
            time_seconds: None,
            start_match: 0,
            limit: 8,
            context_radius: 1,
            buff_names: Vec::new(),
        };
        let result = inspect_timeline_events(
            "trace-event-window",
            &simulation,
            &query,
            &ToolProvenance::fixture(),
        )
        .unwrap();

        assert_eq!(result.evidence.tool_name, INSPECT_TIMELINE_EVENTS);
        assert_eq!(result.evidence.result.total_matches, 2);
        assert_eq!(result.evidence.result.match_index.len(), 2);
        assert_eq!(result.evidence.result.match_index[0].event_number, 2);
        assert_eq!(result.evidence.result.match_index[1].cast_time, 3.0);
        assert!(!result.evidence.result.match_index_truncated);
        assert_eq!(result.evidence.result.windows[0].matched.event_number, 2);
        assert_eq!(
            result.evidence.result.windows[0].matched.anchor_id,
            "timeline:active:1"
        );
        assert_eq!(result.evidence.result.windows[1].matched.event_number, 4);
        assert_eq!(result.evidence.result.windows[0].context.len(), 3);
        simulation.response.timeline[1].rage_cost = Some(30);
        simulation.response.timeline[1].sequence_index = Some(8);
        simulation.response.timeline[3].rage_cost = Some(50);
        let filtered = TimelineEventQueryV1 {
            rage_cost_below: Some(50), event_number: Some(2), ..query.clone()
        };
        let filtered_result = inspect_timeline_events("trace-filtered", &simulation,
            &filtered, &ToolProvenance::fixture()).unwrap().evidence.result;
        assert_eq!(filtered_result.total_matches, 1);
        assert_eq!(filtered_result.windows[0].matched.operation_number, Some(9));
        assert_eq!(filtered_result.match_index[0].operation_number, Some(9));
    }

    #[test]
    fn event_inspection_locates_unique_rage_cap_casts_not_raw_sample_count() {
        let mut simulation = simulation_execution();
        simulation.response.timeline = vec![
            event("盾击", 1, 0.0, 1.0, 100),
            event("盾压", 2, 1.0, 1.0, 80),
        ];
       let query = TimelineEventQueryV1 {
            rage_cost_below: None,
            event_number: None,
           selector: TimelineEventSelector::RageCap,
            skill_name: None,
            time_seconds: None,
            start_match: 0,
            limit: 8,
            context_radius: 1,
            buff_names: Vec::new(),
        };
        let result = inspect_timeline_events(
            "trace-rage-window",
            &simulation,
            &query,
            &ToolProvenance::fixture(),
        )
        .unwrap();

        assert_eq!(result.evidence.result.total_matches, 1);
        assert_eq!(result.evidence.result.windows[0].matched.cast_time, 0.0);
    }

    #[test]
    fn event_inspection_distinguishes_measured_overflow_from_being_at_cap() {
        let mut simulation = simulation_execution();
        let mut overflow = event("盾击", 1, 0.0, 1.0, 100);
        overflow.rage_overflow = Some(5);
        simulation.response.timeline = vec![overflow, event("盾压", 2, 1.0, 1.0, 100)];
       let query = TimelineEventQueryV1 {
            rage_cost_below: None,
            event_number: None,
           selector: TimelineEventSelector::RageOverflow,
            skill_name: None,
            time_seconds: None,
            start_match: 0,
            limit: 8,
            context_radius: 1,
            buff_names: Vec::new(),
        };
        let result = inspect_timeline_events(
            "trace-overflow-window",
            &simulation,
            &query,
            &ToolProvenance::fixture(),
        )
        .unwrap();

        assert_eq!(result.evidence.result.total_matches, 1);
        assert_eq!(
            result.evidence.result.windows[0].matched.rage_overflow,
            Some(5)
        );
        let rage = observe_rage(&simulation.response.timeline, 100).unwrap();
        assert_eq!(rage.overflow_events, 1);
        assert_eq!(rage.overflow_total, 5);
    }

    #[test]
    fn pause_recovery_reports_stance_and_first_active_cast_after_the_pause() {
        let mut before = event("绝刀·50怒", 13054, 119.5, 1.0, 30);
        before.state_after.as_mut().unwrap().stance = Stance::Blade;
        before.state_after.as_mut().unwrap().buffs.push(EventBuff {
            buff_id: crate::BUFF_DUN_FEI,
            name: "盾飞".to_string(),
            remaining: 4.0,
            stacks: 1,
            icon: String::new(),
        });
        let mut triggered = event("流血·每跳", 13044, 125.0, 0.0, 30);
        triggered.triggered = true;
        let mut after = event("盾击", 13045, 130.2, 1.0, 40);
        after.state_before.as_mut().unwrap().stance = Stance::Shield;
        let observations = observe_pause_recovery(&[before, triggered, after], &[(120.0, 10.0)]);

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].last_event_before_pause, Some(1));
        assert_eq!(observations[0].first_event_after_pause, Some(2));
        assert_eq!(
            observations[0].stance_before_pause.as_deref(),
            Some("blade")
        );
        assert_eq!(observations[0].stance_at_resume.as_deref(), Some("shield"));
        assert_eq!(
            observations[0].first_skill_after_pause.as_deref(),
            Some("盾击")
        );
        assert_eq!(observations[0].shield_flying_seconds_at_pause, Some(3.5));
        assert!((observations[0].resume_delay_seconds.unwrap() - 0.2).abs() < TIME_EPSILON);
    }

    #[test]
    fn buff_coverage_merges_refreshes_and_closes_open_interval_at_fight_end() {
        let buff_event = |time, event_type: &str| BuffTimelineEvent {
            time,
            event_type: event_type.to_string(),
            state: None,
        };
        let track = BuffTimelineTrack {
            buff_id: 42,
            name: "测试气劲".to_string(),
            short_name: "测试".to_string(),
            color: String::new(),
            events: vec![
                buff_event(1.0, "gain"),
                buff_event(2.0, "stack"),
                buff_event(4.0, "remove"),
                buff_event(7.0, "gain"),
            ],
        };
        let coverage = calculate_buff_coverage(&track, 10.0).unwrap();

        assert_eq!(coverage.activation_count, 2);
        assert_eq!(coverage.intervals.len(), 2);
        assert_eq!(coverage.active_seconds, 6.0);
        assert_eq!(coverage.coverage_percent, 60.0);
        assert!(coverage.open_at_fight_end);
    }

    #[test]
    fn buff_coverage_keeps_time_coverage_and_average_stacks_separate() {
        let state_with_stacks = |stacks| EventState {
            rage: 0,
            block_value: None,
            stance: Stance::Shield,
            buffs: vec![EventBuff {
                buff_id: 42,
                name: "测试气劲".to_string(),
                remaining: 10.0,
                stacks,
                icon: String::new(),
            }],
            target_buffs: Vec::new(),
            skill_cds: Vec::new(),
        };
        let track = BuffTimelineTrack {
            buff_id: 42,
            name: "测试气劲".to_string(),
            short_name: "测试".to_string(),
            color: String::new(),
            events: vec![
                BuffTimelineEvent {
                    time: 0.0,
                    event_type: "gain".to_string(),
                    state: Some(state_with_stacks(1)),
                },
                BuffTimelineEvent {
                    time: 5.0,
                    event_type: "stack".to_string(),
                    state: Some(state_with_stacks(3)),
                },
                BuffTimelineEvent {
                    time: 10.0,
                    event_type: "remove".to_string(),
                    state: Some(state_with_stacks(3)),
                },
            ],
        };

        let coverage = calculate_buff_coverage(&track, 10.0).unwrap();
        assert_eq!(coverage.coverage_percent, 100.0);
        assert_eq!(coverage.average_stacks_while_active, 2.0);
        assert_eq!(coverage.maximum_stacks_observed, 3);
    }

    #[test]
    fn rotation_cycles_preserve_each_absolute_knife_occurrence_and_resource_flow() {
        let mut shield = event("盾击·三段", 1, 0.0, 1.0, 20);
        shield.rage_delta = Some(10);
        shield.rage_generated = Some(10);
        shield.rage_gained = Some(10);
        let slash = event("斩刀", 2, 1.0, 1.0, 30);
        let mut absolute = event("绝刀·25怒", 3, 2.0, 1.0, 5);
        absolute.rage_delta = Some(-25);
        absolute.rage_spent = Some(25);
        absolute.rage_cost = Some(25);
        let shield_return = event("盾回", 4, 3.0, 0.0, 5);
        let profile = build_rotation_cycles(&[
            shield,
            slash,
            absolute,
            shield_return,
            event("盾击", 1, 4.0, 1.0, 10),
        ]);

        assert_eq!(profile.cycle_count, 2);
        assert_eq!(profile.completed_cycle_count, 1);
        assert_eq!(profile.cycles[0].shield_strike_count, 1);
        assert_eq!(profile.cycles[0].slash_count, 1);
        assert_eq!(profile.cycles[0].absolute_knives[0].event_number, 3);
        assert_eq!(profile.cycles[0].rage_gained, 10);
        assert_eq!(profile.cycles[0].rage_spent, 25);
        assert_eq!(profile.total_rage_generated, 10);
        assert_eq!(profile.total_rage_gained, 10);
        assert_eq!(profile.total_rage_spent, 25);
        assert_eq!(profile.absolute_knives_by_rage_cost.get("25怒"), Some(&1));
        assert!(profile.cycle_shapes[0].count >= 1);
        assert!(!profile.cycles[1].completed_by_shield_return);
    }

    #[test]
    fn cd_waits_ignore_triggered_and_zero_wait_events() {
        let mut waited = event("盾压", 2, 1.0, 1.0, 50);
        waited.cd_wait = 0.25;
        let mut triggered = event("流血", 3, 1.1, 0.0, 50);
        triggered.triggered = true;
        let waits = collect_cd_waits(&[event("盾击", 1, 0.0, 1.0, 20), waited, triggered]);

        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].skill_name, "盾压");
        assert_eq!(waits[0].wait_seconds, 0.25);
    }

    #[test]
    fn timeline_tool_returns_grounded_evidence_from_simulation_execution() {
        let simulation = simulation_execution();
        let timeline =
            analyze_timeline("trace-timeline", &simulation, &ToolProvenance::fixture()).unwrap();

        assert_eq!(timeline.evidence.tool_name, ANALYZE_TIMELINE);
        assert_eq!(
            timeline.evidence.result.fingerprint,
            simulation.response.fingerprint
        );
        assert!(timeline.evidence.result.active_event_count > 0);
        assert!(!timeline.evidence.result.skills.is_empty());
        assert!(timeline.evidence.result.rage.is_some());
        assert_eq!(
            timeline.evidence.result.diagnostic_profile.input_mode,
            "manual_sequence"
        );
        assert!(
            timeline
                .evidence
                .result
                .diagnostic_profile
                .active_cast_count
                > 0
        );
        assert!(timeline
            .evidence
            .result
            .diagnostic_profile
            .observed_strengths
            .iter()
            .all(|signal| !signal.evidence_paths.is_empty()
                && !signal.interpretation_boundary.is_empty()));
        assert!(timeline
            .evidence
            .result
            .diagnostic_profile
            .observed_risks
            .iter()
            .all(|signal| !signal.evidence_paths.is_empty()
                && !signal.interpretation_boundary.is_empty()));
        assert!(timeline
            .evidence
            .result
            .limitations
            .contains(&"timeline_correlations_are_not_causal_without_ab_test".to_string()));
    }

    #[test]
    fn timeline_evidence_is_stable_when_source_buff_tracks_are_reordered() {
        let buff_event = |time| BuffTimelineEvent {
            time,
            event_type: "gain".to_string(),
            state: None,
        };
        let track = |buff_id, name: &str| BuffTimelineTrack {
            buff_id,
            name: name.to_string(),
            short_name: name.to_string(),
            color: String::new(),
            events: vec![buff_event(0.0)],
        };
        let mut first = simulation_execution();
        first.response.buff_timeline = vec![track(200, "后一个气劲"), track(100, "前一个气劲")];
        let mut second = simulation_execution();
        second.response.buff_timeline = vec![track(100, "前一个气劲"), track(200, "后一个气劲")];

        let first_evidence =
            analyze_timeline("trace-order-a", &first, &ToolProvenance::fixture()).unwrap();
        let second_evidence =
            analyze_timeline("trace-order-b", &second, &ToolProvenance::fixture()).unwrap();

        assert_eq!(
            first_evidence.evidence.evidence_id,
            second_evidence.evidence.evidence_id
        );
        assert_eq!(
            first_evidence
                .evidence
                .result
                .buff_coverage
                .iter()
                .map(|coverage| coverage.buff_id)
                .collect::<Vec<_>>(),
            vec![100, 200]
        );
    }

    #[test]
    fn timeline_tool_rejects_lite_or_lite_keep_timeline_artifacts() {
        let mut no_timeline = simulation_execution();
        no_timeline.response.timeline.clear();
        let no_timeline_error =
            match analyze_timeline("trace-lite", &no_timeline, &ToolProvenance::fixture()) {
                Err(error) => error,
                Ok(_) => panic!("lite response without timeline should be rejected"),
            };
        assert!(matches!(
            no_timeline_error,
            ToolError::TimelineDetailsUnavailable
        ));

        let mut no_states = simulation_execution();
        for event in &mut no_states.response.timeline {
            event.state_before = None;
            event.state_after = None;
        }
        let no_states_error =
            match analyze_timeline("trace-lite-keep", &no_states, &ToolProvenance::fixture()) {
                Err(error) => error,
                Ok(_) => panic!("lite_keep_timeline response without states should be rejected"),
            };
        assert!(matches!(
            no_states_error,
            ToolError::TimelineDetailsUnavailable
        ));

        let mut partial_states = simulation_execution();
        let active = partial_states
            .response
            .timeline
            .iter_mut()
            .find(|event| !event.triggered)
            .unwrap();
        active.state_after = None;
        let partial_error = match analyze_timeline(
            "trace-partial-timeline",
            &partial_states,
            &ToolProvenance::fixture(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("partial timeline state should be rejected"),
        };
        assert!(matches!(
            partial_error,
            ToolError::TimelineDetailsUnavailable
        ));
    }
}
