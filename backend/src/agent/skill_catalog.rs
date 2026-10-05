//! Read-only definitions across supported rulesets; never changes the active simulation.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::{GameVersion, Mount};
use super::{AgentRuntime, ScenarioSnapshotV1, ScenarioError, ToolError, EvidenceEnvelopeV1};

pub const LOOKUP_SKILL_DEFINITIONS: &str = "lookup_skill_definitions";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DefinitionQuery {
    pub query: String,
    pub game_version: String,
    pub mount: String,
    pub limit: usize,
}

impl Default for DefinitionQuery {
    fn default() -> Self {
        Self { query:String::new(), game_version:"current".into(), mount:"all".into(), limit:8 }
    }
}

pub fn parameters() -> Value {
    json!({"type":"object","additionalProperties":false,
        "properties":{
            "query":{"type":"string","description":"Skill/talent name, partial name, numeric ID or effect keyword. Prefers name/ID matches, then searches descriptions. Includes unselected talents and other mounts."},
            "game_version":{"type":"string","enum":["current","2025_10_shanhai_yuanliu","2026_04_anying_qianji","2026_04_anying_qianji_test","2026_10_cangsheng_zhushi_test"]},
            "mount":{"type":"string","enum":["all","current","fenshanjin","tieguyi"]},
            "limit":{"type":"integer","minimum":1,"maximum":12}},
        "required":["query"]})
}

pub fn lookup(
    trace_id: &str, query: DefinitionQuery, scenario: &ScenarioSnapshotV1, runtime: &AgentRuntime,
) -> Result<EvidenceEnvelopeV1<Value>, ToolError> {
    let started = std::time::Instant::now();
    let needle = query.query.trim();
    if needle.is_empty() || needle.chars().count() > 64 || needle.chars().any(char::is_control)
        || query.limit == 0 || query.limit > 12 {
        return Err(ScenarioError::InvalidField("definition_query").into());
    }
    let version = match query.game_version.as_str() {
        "current" => runtime.game_version(),
        "2025_10_shanhai_yuanliu" => GameVersion::ShanHaiYuanLiu,
        "2026_04_anying_qianji" => GameVersion::AnYingQianJi,
        "2026_04_anying_qianji_test" => GameVersion::AnYingQianJiTest,
        "2026_10_cangsheng_zhushi_test" => GameVersion::CangShengZhuShiTest,
        _ => return Err(ScenarioError::InvalidField("game_version").into()),
    };
    let mounts = match query.mount.as_str() {
        "all" => vec![runtime.mount(), if runtime.mount() == Mount::FenShanJin { Mount::TieGuYi } else { Mount::FenShanJin }],
        "current" => vec![runtime.mount()],
        "fenshanjin" => vec![Mount::FenShanJin],
        "tieguyi" => vec![Mount::TieGuYi],
        _ => return Err(ScenarioError::InvalidField("mount").into()),
    };
    let mut matches = Vec::new();
    let mut scopes = Vec::new();
    let mut search_mode = "name_or_id";
    for search_descriptions in [false, true] {
      scopes.clear();
      if search_descriptions { search_mode = "description"; }
      for &mount in &mounts {
        let mut skills = crate::load_skills(std::path::Path::new(&crate::skills_dir(version, mount)));
        let mut talents = crate::load_talents(std::path::Path::new(&crate::talents_file(version, mount)));
        // Some rulesets retain copied skill files for unavailable talents.
        // These are not capabilities of that mount.
        skills.retain(|s| s.requires_talent.map_or(true, |id| talents.iter().any(|t| t.id == id)));
        skills.sort_by(|a,b| a.skill_id.cmp(&b.skill_id).then_with(||a.name.cmp(&b.name)));
        talents.sort_by_key(|talent| (talent.tier, talent.id));
        let same_scope = version == runtime.game_version() && mount == runtime.mount();
        let definition_hash = super::hash::canonical_sha256(&json!({"skills":skills,"talents":talents}))
            .map_err(|_| ScenarioError::InvalidField("definition_data"))?;
        let scope = json!({"game_version":super::schema::game_version_id(version),
            "mount":super::schema::mount_id(mount),"client":"flagship",
            "is_current_simulation_scope":same_scope,"definition_hash":definition_hash});
        scopes.push(scope.clone());
        let name_matches = |id: u32, name: &str, description: &str| name.contains(needle) || id.to_string() == needle
            || (search_descriptions && description.contains(needle));
        let found_talents = talents.iter().filter(|t| name_matches(t.id, &t.name, &t.desc)).collect::<Vec<_>>();
        let found_skills = skills.iter().filter(|s| name_matches(s.skill_id, &s.name, &s.description)
            || found_talents.iter().any(|t| s.requires_talent == Some(t.id)
                || t.description_skills.as_ref().is_some_and(|ids| ids.contains(&s.skill_id))))
            .collect::<Vec<_>>();
        for talent in &found_talents {
            matches.push(json!({"scope":scope,"kind":"talent","definition":talent,
                "selected_in_current_scenario":same_scope && scenario.simulation.talents.contains(&talent.id)}));
        }
        for skill in found_skills {
            matches.push(json!({"scope":scope,"kind":"skill","definition":skill,
                "stance_requirement":match skill.stance {
                    crate::Stance::Shield => "仅擎盾体态可施放",
                    crate::Stance::Blade => "仅擎刀体态可施放",
                    crate::Stance::Wall => "仅盾墙体态可施放",
                    crate::Stance::NotWall => "擎盾或擎刀可施放，盾墙体态不可施放",
                    crate::Stance::Any => "不限制体态",
                },
                "talent_enabled_in_current_scenario":same_scope && skill.requires_talent.map_or(true,|id|scenario.simulation.talents.contains(&id)),
                "related_talents":talents.iter().filter(|t|Some(t.id)==skill.requires_talent
                    || t.description_skills.as_ref().is_some_and(|ids|ids.contains(&skill.skill_id))).collect::<Vec<_>>() }));
        }
      }
      if !matches.is_empty() { break; }
    }
    let total = matches.len();
    matches.truncate(query.limit);
    EvidenceEnvelopeV1::new(trace_id, LOOKUP_SKILL_DEFINITIONS, &scenario.scenario_hash,
        serde_json::to_value(&query).unwrap(),
        json!({"query":needle,"search_mode":search_mode,"searched_scopes":scopes,"total_matches":total,"matches":matches,
            "truncated":total>query.limit,"applies_rules_to_scenario":false,
            "usage":"定义用于机制分析；跨心法移植属于假设规则，宏文本替换不会移植资源、技能脚本或奇穴效果。"}),
        runtime.provenance(), started.elapsed().as_millis() as u64).map_err(ToolError::Evidence)
}

#[cfg(test)]
#[path = "../../tests/agent/rule_catalog.rs"]
mod tests;
