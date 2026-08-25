use crate::{
    FormationEntry, GameVersion, Mount, MountConstants, RecipeEntry, SharedState, SkillSpec,
    TeamBuffEntry,
};

use super::{SimulatorContext, ToolProvenance};

/// Immutable, owned view of the simulator tables used for one Agent run.
///
/// Loading it under `agent_context_gate` prevents a run from observing a
/// partially switched version/mount while keeping the actual combat formulas
/// in the existing deterministic simulator.
pub struct AgentRuntime {
    game_version: GameVersion,
    mount: Mount,
    constants: MountConstants,
    skills: Vec<SkillSpec>,
    recipes: Vec<RecipeEntry>,
    team_buffs: Vec<TeamBuffEntry>,
    formations: Vec<FormationEntry>,
    provenance: ToolProvenance,
}

impl AgentRuntime {
    pub async fn load(state: &SharedState) -> Self {
        let _gate = state.agent_context_gate.read().await;
        Self {
            game_version: *state.version.read().await,
            mount: *state.mount.read().await,
            constants: *state.constants.read().await,
            skills: state.skills.read().await.clone(),
            recipes: state.recipes.read().await.clone(),
            team_buffs: state.team_buffs.read().await.clone(),
            formations: state.formations.read().await.clone(),
            provenance: state.agent_provenance.read().await.clone(),
        }
    }

    pub fn game_version(&self) -> GameVersion {
        self.game_version
    }

    pub fn mount(&self) -> Mount {
        self.mount
    }

    pub fn context(&self) -> SimulatorContext<'_> {
        SimulatorContext {
            game_version: self.game_version,
            mount: self.mount,
            constants: self.constants,
            skills: &self.skills,
            recipes: &self.recipes,
            team_buffs: &self.team_buffs,
            formations: &self.formations,
        }
    }

    pub fn provenance(&self) -> &ToolProvenance {
        &self.provenance
    }

    #[cfg(test)]
    pub fn fixture() -> Self {
        use crate::{
            formations_file, load_formations, load_recipes, load_school_toml, load_skills,
            load_team_buffs, recipes_file, skills_dir, team_buffs_file,
        };
        use std::path::Path;

        let game_version = GameVersion::AnYingQianJi;
        let mount = Mount::FenShanJin;
        let (constants, _, _, _, _) = load_school_toml(game_version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(game_version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(game_version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(game_version)));
        let formations = load_formations(Path::new(&formations_file(game_version)));
        let provenance = ToolProvenance::fixture();
        Self {
            game_version,
            mount,
            constants,
            skills,
            recipes,
            team_buffs,
            formations,
            provenance,
        }
    }

    #[cfg(test)]
    pub fn fixture_scenario(&self) -> super::ScenarioSnapshotV1 {
        use crate::{Attributes, TargetConfig};
        use std::collections::HashMap;

        super::ScenarioSnapshotV1::capture(
            self.game_version,
            self.mount,
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
}
