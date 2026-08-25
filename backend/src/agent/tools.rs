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
pub const SIMULATE_SCENARIO: &str = "simulate_scenario";

pub struct SimulatorContext<'a> {
    pub game_version: GameVersion,
    pub mount: Mount,
    pub constants: MountConstants,
    pub skills: &'a [SkillSpec],
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
    pub rotation_mode: String,
    pub sequence_entries: usize,
    pub macro_characters: usize,
    pub macro_duration: Option<f64>,
    pub talent_count: usize,
    pub recipe_count: usize,
    pub equipment_count: usize,
    pub enabled_team_buff_count: usize,
    pub formation_key: Option<String>,
    pub pre_release_count: usize,
    pub target_level: u32,
    pub network_delay_ms: u32,
    pub boss_attack_interval: Option<f64>,
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

pub struct SimulationExecution {
    pub evidence: EvidenceEnvelopeV1<SimulationSummary>,
    pub response: SimulateResponse,
}

pub fn get_current_scenario(
    trace_id: &str,
    snapshot: &ScenarioSnapshotV1,
    provenance: &ToolProvenance,
) -> Result<EvidenceEnvelopeV1<ScenarioSummary>, ToolError> {
    let started = Instant::now();
    snapshot.verify_hash()?;
    let simulation = &snapshot.simulation;
    let target = simulation
        .target
        .as_ref()
        .ok_or(ScenarioError::MissingField("simulation.target"))?;
    let result = ScenarioSummary {
        game_version: snapshot.game_version.clone(),
        mount: snapshot.mount.clone(),
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
            get_current_scenario("trace-summary", &snapshot, &ToolProvenance::fixture()).unwrap();

        assert_eq!(evidence.tool_name, GET_CURRENT_SCENARIO);
        assert_eq!(evidence.scenario_hash, snapshot.scenario_hash);
        assert_eq!(evidence.result.rotation_mode, "sequence");
        assert_eq!(evidence.result.sequence_entries, 2);
        assert_eq!(evidence.result.target_level, 134);
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
