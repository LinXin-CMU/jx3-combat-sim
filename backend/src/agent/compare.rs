use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::{
    Attributes, FormationSelection, PreReleaseSpec, SimulateResponse, TargetConfig,
    TeamBuffSelection,
};

use super::evidence::{validate_trace_id, EvidenceEnvelopeV1, EvidenceError, ToolProvenance};
use super::schema::ScenarioSnapshotV1;
use super::tools::{
    elapsed_ms, run_simulation, summarize_simulation, verify_runtime, SimulatorContext, ToolBudget,
    ToolError,
};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonCandidate {
    pub label: String,
    pub changes: Vec<FieldChange>,
    pub metrics: ComparisonMetrics,
    pub delta_dps: f64,
    pub delta_percent: Option<f64>,
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
        prepared.push((candidate.label.clone(), snapshot, changes));
    }

    let simulation_count = u32::try_from(prepared.len() + 1).unwrap_or(u32::MAX);
    budget.reserve_simulations(simulation_count)?;

    let baseline_response = run_simulation(baseline, context);
    let baseline_metrics = metrics(baseline, &baseline_response);
    let mut result_candidates = Vec::with_capacity(prepared.len());
    let mut executions = Vec::with_capacity(prepared.len());

    for (label, snapshot, changes) in prepared {
        let response = run_simulation(&snapshot, context);
        let candidate_metrics = metrics(&snapshot, &response);
        let delta_dps = candidate_metrics.dps - baseline_metrics.dps;
        let delta_percent = if baseline_metrics.dps.abs() > f64::EPSILON {
            Some(delta_dps / baseline_metrics.dps * 100.0)
        } else {
            None
        };
        result_candidates.push(ComparisonCandidate {
            label: label.clone(),
            changes,
            metrics: candidate_metrics,
            delta_dps,
            delta_percent,
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
            execution.baseline_response.fingerprint,
            execution.evidence.result.baseline.fingerprint
        );
        assert_eq!(
            execution.candidates[0].response.fingerprint,
            result.metrics.fingerprint
        );
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
