//! Versioned interpretation context, separate from observations and planning.
//!
//! Names/selections come from the same frozen tables as the simulator. The
//! short relationship notes are audited against the named implementation;
//! guide links and known discrepancies retain their distinct provenance.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::{ScenarioSnapshotV1, SimulatorContext};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MechanicsContextV1 {
    pub schema_version: String,
    pub game_version: String,
    pub mount: String,
    pub client: String,
    pub scope_note: String,
    pub runtime_active_skill_names: Vec<String>,
    #[serde(default)]
    pub runtime_cast_requirements: BTreeMap<String, String>,
    #[serde(default)]
    pub macro_runtime_reference: String,
    pub selected_talents: Vec<SelectedTalentContextV1>,
    pub selected_recipe_names: Vec<String>,
    pub simulator_rules: Vec<MechanicNoteV1>,
    pub observation_semantics: BTreeMap<String, String>,
    pub known_differences: Vec<MechanicNoteV1>,
    /// Source keys in notes resolve to repo-relative paths or original URLs.
    pub sources: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedTalentContextV1 {
    pub id: u32,
    pub name: String,
    pub data_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MechanicNoteV1 {
    pub topic: String,
    pub detail: String,
    pub sources: Vec<String>,
}

fn note(topic: &str, detail: &str, sources: &[&str]) -> MechanicNoteV1 {
    MechanicNoteV1 {
        topic: topic.to_string(),
        detail: detail.to_string(),
        sources: sources.iter().map(|source| (*source).to_string()).collect(),
    }
}

#[cfg(test)]
mod macro_reference_tests {
    use crate::{macro_engine::MacroCondition, macro_parser::parse_macro_text};

    #[test]
    fn documented_boolean_syntax_matches_the_existing_parser() {
        let parsed = parse_macro_text("/cast [buff:狂绝&nobuff:血怒·惊涌] 血怒").unwrap();
        assert!(matches!(&parsed.pages[0].lines[0].condition, Some(MacroCondition::And(_, _))));
        let comma = parse_macro_text("/cast [buff:狂绝, nobuff:血怒·惊涌] 血怒").unwrap();
        assert!(matches!(&comma.pages[0].lines[0].condition, Some(MacroCondition::Buff(name)) if name.contains(',')));
        let rage = parse_macro_text("/cast [rage>49] 绝刀").unwrap();
        assert!(matches!(&rage.pages[0].lines[0].condition, Some(MacroCondition::Rage(_, 49))));
    }
}

pub fn build_mechanics_context(
    snapshot: &ScenarioSnapshotV1,
    context: &SimulatorContext<'_>,
) -> MechanicsContextV1 {
    let simulation = &snapshot.simulation;
    let has_talent = |id| simulation.talents.contains(&id);
    let has_recipe = |id| simulation.recipes.contains(&id);
    let mut result = MechanicsContextV1 {
        schema_version: "agent-mechanics-context/v1".to_string(),
        game_version: snapshot.game_version.clone(),
        mount: snapshot.mount.clone(),
        client: "旗舰端模拟器；无界端问题通过知识库另查对应资料".to_string(),
        scope_note: "技能名与已选奇穴来自本轮运行表；simulator_rules 说明实现，data_description 保留配置说明。事件是否触发由时间轴确认；攻略原文提供玩法语境，具体取舍由当前问题与实验决定。".to_string(),
        macro_runtime_reference: include_str!("../../prompts/macro_runtime_reference.md").to_string(),
        runtime_active_skill_names: context.skills.iter()
            .filter(|skill| !skill.passive && skill.requires_talent.is_none_or(has_talent))
            .map(|skill| skill.name.clone())
            .collect::<BTreeSet<_>>().into_iter().collect(),
        runtime_cast_requirements: context.skills.iter()
            .filter(|skill| !skill.passive && skill.requires_talent.is_none_or(has_talent))
            .map(|skill| (skill.name.clone(), format!("体态={:?};基础耗怒={};基础回怒={};切换体态={:?};连招要求={:?}",
                skill.stance, skill.rage_cost, skill.rage_gain, skill.stance_change, skill.requires_combo)))
            .collect(),
        selected_talents: simulation.talents.iter().copied().collect::<BTreeSet<_>>().into_iter().map(|id| {
            let definition = context.talents.iter().find(|talent| talent.id == id);
            SelectedTalentContextV1 {
                id,
                name: definition.map(|talent| talent.name.clone()).unwrap_or_else(|| "当前心法表中待核对的奇穴".to_string()),
                data_description: definition.map(|talent| talent.desc.clone()).unwrap_or_default(),
            }
        }).collect(),
        selected_recipe_names: context.recipes.iter()
            .filter(|recipe| has_recipe(recipe.id))
            .map(|recipe| format!("{} / {} / {}", recipe.id, recipe.skill, recipe.name))
            .collect(),
        simulator_rules: Vec::new(),
        observation_semantics: [
            ("怒气", "rage 为怒气资源；rage_cost 是本次技能计费档位。rage_spent 记录扣除总量，rage_gained 记录实得回复，rage_transactions 分列来源。净变化还包含技能附带回怒与返还。"),
            ("绝刀分档", "技能名中的绝刀·50怒、absolute_knives_by_rage_cost 按计费档位归类。区分免费绝刀与额外付费绝刀时结合施放前狂绝、天下宏愿状态及该次怒气交易。"),
            ("覆盖与层数", "coverage_percent 已是0到100的时间覆盖百分数，直接添加%展示。average_stacks_while_active 为Buff生效期间按时间加权的平均层数。次数采样占比的分母是采样次数。"),
            ("怒气触顶", "at_cap_observations 统计触顶采样；实际资源浪费看 rage_overflow、overflow_total 与具体产生来源。"),
            ("时间与等待", "cast_time 为实际施放秒数；operation_number 是手动输入编号。timing_offset=-1 表示跟随GCD结束。GCD空档、冷却等待可能重叠，成因结合相邻事件与输入条件判断。"),
        ].into_iter().map(|(key, value)| (key.to_string(), value.to_string())).collect(),
        known_differences: Vec::new(),
        sources: BTreeMap::from([
            ("talents".to_string(), format!("backend/data/{}/{}/talents.toml", crate::version_dir_name(context.game_version), crate::mount_dir_name(context.mount))),
            ("recipes".to_string(), format!("backend/data/{}/recipes.toml", crate::version_dir_name(context.game_version))),
            ("timeline".to_string(), "backend/src/agent/timeline.rs".to_string()),
            ("resource_events".to_string(), "backend/src/main.rs: Player::set_rage / Player::add_rage_from / simulate_core".to_string()),
        ]),
    };

    // Audited notes are scoped to the formal release and mount. Other scopes
    // still receive their own runtime definitions and observation semantics.
    if context.game_version != crate::GameVersion::AnYingQianJi
        || context.mount != crate::Mount::FenShanJin
        || simulation.experimental
    {
        result.observation_semantics.remove("绝刀分档");
        result
            .scope_note
            .push_str("本场景使用运行表定义；详细关系通过对应版本资料与事件展开。");
        return result;
    }

    let script_root = "backend/src/scripts/v2026_04_AnYingQianJi";
    for (key, path) in [
        ("shield_throw", "skills/dun_fei.rs"),
        ("shield_return", "skills/dun_hui.rs"),
        ("shield_expire", "buffs/buff_dun_fei.rs"),
        ("slash", "skills/zhan_dao.rs"),
        ("absolute", "skills/jue_dao.rs"),
        ("shield_strike", "skills/dun_ji.rs"),
        ("yuange_gain", "skills/ji_po_yuan_ge.rs"),
        ("yuange_damage", "skills/yuan_ge.rs"),
        ("blood_rage", "skills/xue_nu.rs"),
        ("linguang", "skills/lin_guang.rs"),
        ("buffs", "buffs/defs.rs"),
    ] {
        result
            .sources
            .insert(key.to_string(), format!("{script_root}/{path}"));
    }
    result.sources.insert(
        "whitepaper".to_string(),
        "https://www.yuque.com/sgyxy/cangyun/whitepaper-23".to_string(),
    );
    result.simulator_rules.push(note("盾刀与流血", "盾飞产生盾飞持续伤害、延迟给目标虚弱并转擎刀；斩刀命中已有虚弱或流血的目标时添加/刷新流血。主动盾回结束盾飞并回擎盾；盾飞自然到期也回擎盾。短暂停手期间体态由现有Buff和已执行技能决定。", &["shield_throw", "slash", "shield_return", "shield_expire"]));
    result.simulator_rules.push(note("绝刀档位", if has_recipe(3005) {
        "已选减耗秘籍3005：绝刀按施放时怒气选择10/20/30/40/50最高可用档位；50为最高伤害档。返还在扣除之后执行，技能名称保留原计费档位。"
    } else {
        "当前未选减耗秘籍3005：绝刀按施放时怒气选择25/35/45/55/65最高可用档位；65为最高伤害档。返还在扣除之后执行，技能名称保留原计费档位。"
    }, &["absolute", "recipes"]));
    if has_talent(13090) {
        result.simulator_rules.push(note("绝返与狂绝", "已选绝返：斩刀获得狂绝；下一次绝刀在结算时返还本次计费怒气并消耗狂绝。评估额外付费绝刀时结合这一免费机会及其持续时间。", &["slash", "absolute", "buffs"]));
    }
    if has_talent(36058) {
        result.simulator_rules.push(note("援戈与血影", "已选援戈：盾击触发击破·援戈并获取援戈层数；苍雪刀技能结算时有援戈则消耗一层并追加援戈·血影。援戈为有限的Buff层数，血影为触发伤害；血怒状态影响血影伤害。", &["shield_strike", "yuange_gain", "yuange_damage"]));
        result.known_differences.push(note("援戈资料", "白皮书援戈相关段落存在血怒增伤50%与100%两种表述；当前实现使用50%。精确游戏倍率及旧段落描述需结合来源位置核对。", &["whitepaper", "yuange_damage", "recipes"]));
    }
    result.simulator_rules.push(note("血怒", if has_talent(36205) {
        "已选惊涌：血怒施放后使用血怒·惊涌Buff，按该变体的层数和持续时间运行；秘籍可增加回怒和持续时间。"
    } else {
        "当前血怒使用普通血怒Buff；连续施放的叠层窗口与Buff持续时间分别计时，秘籍可增加回怒和持续时间。"
    }, &["blood_rage", "buffs", "recipes"]));
    if has_talent(38969) {
        result.simulator_rules.push(note(
            "血魄",
            "已选血魄：血怒额外重置斩刀冷却并回复怒气；这项效果取决于本次奇穴选择。",
            &["blood_rage"],
        ));
    }
    if has_talent(34912) {
        result.simulator_rules.push(note("业火麟光", "麟光甲有效时，苍雪刀消耗其层数并追加麟光甲寒；每累计三次触发会重置相关刀系技能冷却、回怒并结算业火焚城。分析绝刀后的怒气变化时可同时发生这一回怒。", &["linguang"]));
    }
    if has_talent(21281) {
        result.simulator_rules.push(note("嗜血", "已选嗜血：奇穴对绝刀的常驻加成与嗜血Buff的持续增益分别生效。绝刀结算获得/刷新嗜血Buff；覆盖时间与某次绝刀施放前是否带有增益分别读取。", &["absolute", "buffs", "resource_events"]));
        result.known_differences.push(note("嗜血的绝刀档位", "白皮书将嗜血的绝刀增伤限定在最高怒气档；当前隐藏秘籍99240按绝刀技能ID生效，缺少档位筛选。低怒气档收益需保留这一实现差异。", &["whitepaper", "recipes", "resource_events"]));
    }
    if has_talent(36205) {
        result.known_differences.push(note("惊涌触发条件", "白皮书把惊涌额外伤害描述为血怒期间的最高怒气档绝刀；当前脚本检查已选惊涌与最高档位，缺少血怒Buff判断。模拟中额外伤害的出现依当前实现解释。", &["whitepaper", "absolute"]));
    }
    result.known_differences.push(note("自然盾回保护时间", "白皮书区分主动盾回与自然到期盾回的保护时间；当前自然到期脚本也添加1秒保护冷却。涉及停手后衔接时应区分攻略规则与模拟表现。", &["whitepaper", "shield_expire"]));

    // Keep only referenced sources plus runtime metadata, avoiding a second
    // complete skill catalogue in every model request.
    let referenced = result
        .simulator_rules
        .iter()
        .chain(&result.known_differences)
        .flat_map(|rule| rule.sources.iter().cloned())
        .collect::<BTreeSet<_>>();
    result.sources.retain(|key, _| {
        referenced.contains(key)
            || matches!(
                key.as_str(),
                "talents" | "recipes" | "timeline" | "resource_events"
            )
    });
    result
}
