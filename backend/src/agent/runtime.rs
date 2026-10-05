use crate::{
    equip, FormationEntry, GameVersion, Mount, MountConstants, RecipeEntry, SharedState, SkillSpec,
    TeamBuffEntry,
};

use super::{KnowledgeIndex, SimulatorContext, ToolProvenance};
use std::sync::Arc;

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
    talents: Vec<crate::TalentEntry>,
    recipes: Vec<RecipeEntry>,
    team_buffs: Vec<TeamBuffEntry>,
    formations: Vec<FormationEntry>,
    provenance: ToolProvenance,
    knowledge: Option<Arc<KnowledgeIndex>>,
    equip_data: Option<Arc<equip::EquipData>>,
    base_stats: equip::MountBaseStats,
    mount_conversions: equip::MountConversions,
}

impl AgentRuntime {
    pub async fn load(state: &SharedState) -> Self {
        let _gate = state.agent_context_gate.read().await;
        Self {
            game_version: *state.version.read().await,
            mount: *state.mount.read().await,
            constants: *state.constants.read().await,
            skills: state.skills.read().await.clone(),
            talents: state.talents.read().await.clone(),
            recipes: state.recipes.read().await.clone(),
            team_buffs: state.team_buffs.read().await.clone(),
            formations: state.formations.read().await.clone(),
            provenance: state.agent_provenance.read().await.clone(),
            knowledge: state.agent_knowledge.clone(),
            equip_data: Some(state.current_equipment().await),
            base_stats: state.base_stats.read().await.clone(),
            mount_conversions: state.mount_conversions.read().await.clone(),
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
            talents: &self.talents,
            recipes: &self.recipes,
            team_buffs: &self.team_buffs,
            formations: &self.formations,
        }
    }

    pub fn provenance(&self) -> &ToolProvenance {
        &self.provenance
    }

    pub fn knowledge(&self) -> Option<&KnowledgeIndex> {
        self.knowledge.as_deref()
    }

    pub fn calculate_equipment(
        &self,
        slots: &std::collections::HashMap<String, equip::SlotConfig>,
        stone_id: u32,
        talents: &[u32],
    ) -> equip::CalcResponse {
        let data = self
            .equip_data
            .as_deref()
            .expect("equipment data is available in live runtime");
        equip::calculate(
            data,
            &equip::CalcRequest {
                slots: slots.clone(),
                stone_id,
                mount: match self.mount {
                    Mount::FenShanJin => 10390,
                    Mount::TieGuYi => 10389,
                },
                talents: talents.to_vec(),
            },
            &self.base_stats,
            &self.mount_conversions,
        )
    }

    pub fn equipment_item(&self, subtype: u8, id: u32) -> Option<&equip::EquipItem> {
        self.equip_data.as_deref()?.get_item(subtype, id)
    }

    /// Frozen, read-only catalog for typed experiment constraint validation.
    pub(crate) fn equipment_data(&self) -> Option<&equip::EquipData> {
        self.equip_data.as_deref()
    }

    /// Identity of every table used by equipment recalculation, including the
    /// mount conversions. Composite item keys and set-valued attribute tags
    /// need explicit ordering; canonical JSON only sorts object keys.
    pub(crate) fn equipment_identity(&self) -> Result<String, String> {
        let data = self.equipment_data().ok_or("装备目录不可用")?;
        let mut items = data.items.iter().collect::<Vec<_>>();
        items.sort_by_key(|(key, _)| **key);
        let items = items
            .into_iter()
            .map(|(key, item)| {
                let mut value = serde_json::to_value(item)?;
                let mut tags = item.attr_tags.iter().collect::<Vec<_>>();
                tags.sort_unstable();
                value["attr_tags"] = serde_json::to_value(tags)?;
                Ok((*key, value))
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()
            .map_err(|e| e.to_string())?;
        super::hash::canonical_sha256(&serde_json::json!({
            "items":items,"attrib_table":data.attrib_table,
            "enhances":data.enhances,"enchants":data.enchants,
            "stones":data.stones,"sets":data.sets,
            "base_stats":self.base_stats,"mount_conversions":self.mount_conversions
        })).map_err(|e|e.to_string())
    }

    pub fn equipment_items(&self) -> impl Iterator<Item = &equip::EquipItem> {
        self.equip_data
            .as_deref()
            .into_iter()
            .flat_map(|data| data.items.values())
    }

    pub fn equipment_set_name(&self, set_id: u32) -> Option<String> {
        self.equip_data
            .as_deref()?
            .sets
            .get(&set_id)
            .map(|set| set.name.clone())
    }

    #[cfg(test)]
    pub fn fixture() -> Self {
        Self::fixture_for(GameVersion::AnYingQianJi, Mount::FenShanJin)
    }

    #[cfg(test)]
    pub fn fixture_for(game_version: GameVersion, mount: Mount) -> Self {
        use crate::{
            formations_file, load_formations, load_recipes, load_school_toml, load_skills,
            load_talents, load_team_buffs, recipes_file, skills_dir, talents_file, team_buffs_file,
        };
        use std::path::Path;

        let (constants, _, _, _, _) = load_school_toml(game_version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(game_version, mount)));
        let talents = load_talents(Path::new(&talents_file(game_version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(game_version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(game_version)));
        let formations = load_formations(Path::new(&formations_file(game_version)));
        let provenance = ToolProvenance::fixture();
        Self {
            game_version,
            mount,
            constants,
            skills,
            talents,
            recipes,
            team_buffs,
            formations,
            provenance,
            knowledge: None,
            equip_data: None,
            base_stats: equip::MountBaseStats::default(),
            mount_conversions: equip::MountConversions::default(),
        }
    }

    #[cfg(test)]
    pub fn with_knowledge_fixture(mut self, knowledge: KnowledgeIndex) -> Self {
        self.knowledge = Some(Arc::new(knowledge));
        self
    }

    #[cfg(test)]
    pub fn with_equipment_fixture(mut self) -> Self {
        use std::path::Path;

        let (_, base_stats, mount_conversions, _, _) =
            crate::load_school_toml(self.game_version, self.mount).unwrap();
        self.equip_data = Some(Arc::new(equip::load_equip_smart(Path::new(
            crate::data_root(),
        ))));
        self.base_stats = base_stats;
        self.mount_conversions = mount_conversions;
        self
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
                solidified_casts: HashMap::new(),
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
            dunya_reset_seed: Default::default(),
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

#[cfg(test)]
#[path = "../../tests/agent/runtime_identity.rs"]
mod identity_tests;
