use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::{BuffTimelineTrack, CastEvent};

use super::evidence::{validate_trace_id, EvidenceEnvelopeV1, ToolProvenance};
use super::tools::{elapsed_ms, SimulationExecution, SkillDamageSummary, ToolError};

pub const ANALYZE_TIMELINE: &str = "analyze_timeline";
const TIME_EPSILON: f64 = 0.001;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WaitEvidence {
    pub cast_time: f64,
    pub skill_id: u32,
    pub skill_name: String,
    pub wait_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GcdGapEvidence {
    pub previous_cast_time: f64,
    pub previous_skill_id: u32,
    pub previous_skill_name: String,
    pub next_cast_time: f64,
    pub next_skill_id: u32,
    pub next_skill_name: String,
    pub expected_ready_time: f64,
    pub observed_gap_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RageObservation {
    pub minimum: i32,
    pub maximum: i32,
    pub ending: i32,
    pub sample_count: usize,
    pub at_cap_observations: usize,
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
    pub limitations: Vec<String>,
}

pub struct TimelineExecution {
    pub evidence: EvidenceEnvelopeV1<TimelineAnalysis>,
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
        rage: observe_rage(&response.timeline, response.rage),
        buff_coverage,
        skipped: response
            .skipped
            .iter()
            .map(|(sequence_index, reason)| SkippedSkill {
                sequence_index: *sequence_index,
                reason: reason.clone(),
            })
            .collect(),
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
        .filter(|event| !event.triggered && event.cd_wait > TIME_EPSILON)
        .map(|event| WaitEvidence {
            cast_time: event.cast_time,
            skill_id: event.skill_id,
            skill_name: event.name.clone(),
            wait_seconds: event.cd_wait,
        })
        .collect()
}

fn collect_gcd_gaps(timeline: &[CastEvent]) -> Vec<GcdGapEvidence> {
    let mut main_events: Vec<_> = timeline
        .iter()
        .filter(|event| !event.triggered && event.is_main)
        .collect();
    main_events.sort_by(|left, right| left.cast_time.total_cmp(&right.cast_time));

    main_events
        .windows(2)
        .filter_map(|pair| {
            let previous = pair[0];
            let next = pair[1];
            let occupied = previous.channel_duration.unwrap_or(0.0).max(previous.gcd);
            let expected_ready_time = previous.cast_time + occupied;
            let observed_gap_seconds = next.cast_time - expected_ready_time;
            (observed_gap_seconds > TIME_EPSILON).then(|| GcdGapEvidence {
                previous_cast_time: previous.cast_time,
                previous_skill_id: previous.skill_id,
                previous_skill_name: previous.name.clone(),
                next_cast_time: next.cast_time,
                next_skill_id: next.skill_id,
                next_skill_name: next.name.clone(),
                expected_ready_time,
                observed_gap_seconds,
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
    Some(RageObservation {
        minimum,
        maximum,
        ending,
        sample_count: samples.len(),
        at_cap_observations,
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
    for event in events {
        let time = event.time.clamp(0.0, fight_time);
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
        activation_count,
        open_at_fight_end,
        unmatched_close_events,
        intervals,
    })
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
            rage_after: Some(rage),
            rage_delta: None,
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
        first.response.buff_timeline =
            vec![track(200, "后一个气劲"), track(100, "前一个气劲")];
        let mut second = simulation_execution();
        second.response.buff_timeline =
            vec![track(100, "前一个气劲"), track(200, "后一个气劲")];

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
