//! 技能效果脚本系统 - 顶级路由
//!
//! 按 `player.version` 分发到对应版本的脚本模块。
//! 同版本内，脚本跨心法共享，内部用 `player.mount` 分支心法差异。
//! Buff 脚本同样按版本分发。
//!
//! 旧版本（2025.10 山海源流 / 2026.04 暗影千机·测试服）已归档为 zip 移到
//! `src/scripts/v2025_10_ShanHaiYuanLiu.zip` 和 `data/2026_04_暗影千机测试服.zip`。
//! 枚举变体保留以兼容旧 plaza/loops 存档的反序列化；所有 match arm fallthrough 到 AnYingQianJi。

pub mod v2026_04_AnYingQianJi;

use crate::{Player, ScriptEmitter, SkillSpec, GameVersion, BuffDef};

/// 技能脚本函数签名
pub type SkillScriptFn = fn(&mut Player, &mut ScriptEmitter, f64);

// ─── BuffDef 路由 ──────────────────────────────────────────

/// 按版本查找 BuffDef 静态实例
pub fn get_buff_def(player: &Player, buff_id: u32) -> Option<&'static BuffDef> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::defs::get_buff_def(buff_id),
    }
}

/// 仅按版本枚举查（供不便传 Player 的地方用，如 aggregate/序列化）
pub fn get_buff_def_by_version(version: GameVersion, buff_id: u32) -> Option<&'static BuffDef> {
    match version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::defs::get_buff_def(buff_id),
    }
}

/// 列出指定版本的所有 BuffDef（与对应 get_buff_def match arms 配套维护）
/// 启动时扫描用，避免对每个 buff_id 单独 match
pub fn all_buff_defs_by_version(version: GameVersion) -> Vec<&'static BuffDef> {
    match version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::defs::all_buff_defs(),
    }
}

/// 收集指定版本所有 effects 含 AllDamageAddPercent 字段的 buff_id 集合
/// 用于前端 log 战斗记录 hover 显示"伤害增益"（只统计 Step 8 全局增伤型 buff）
pub fn collect_damage_add_buff_ids(version: GameVersion) -> Vec<u32> {
    use crate::AttribField;
    all_buff_defs_by_version(version)
        .into_iter()
        .filter(|def| def.effects.iter().any(|e| matches!(e.field, AttribField::AllDamageAddPercent)))
        .map(|def| def.buff_id)
        .collect()
}

/// AttribField → 前端实时面板的 attr_key 列表（属性 hover tooltip"增益来源"用）
/// 一个字段可能影响多个面板属性。仅列实时面板能显示的属性，
/// 伤害链字段（AllDamageAddPercent / PveAddition / RecipeDamagePercent 等）不在面板里 → 返回空
pub fn affected_attr_keys(field: crate::AttribField) -> &'static [&'static str] {
    use crate::AttribField::*;
    match field {
        // 主属性
        // 注：主属性 buff 只列到主属性面板（vit/agi/str/hp），不延伸到副属性 —— 副属性 hover
        // 已通过"主属性转化"展示（如 panel.shenfa_attack），不再重复显示"全属性 → 攻击"等
        VitalityBase | VitalityBasePercentAdd => &["vit", "hp"],
        AgilityBase => &["agi"],
        StrengthBase => &["str"],
        BasePotentialAdd => &["vit", "agi", "str", "hp"],
        // 攻击 / 副属性
        PhysicsAttackPowerBase | PhysicsAttackPowerPercent | VitalityToAttackCof => &["atk"],
        PhysicsCriticalStrike | PhysicsCriticalStrikePercent => &["crit"],
        PhysicsCriticalDamagePowerBase | PhysicsCriticalDamagePowerPercent => &["crit_eff"],
        PhysicsOvercomeBase | PhysicsOvercomePercent | VitalityToOvercomeCof => &["oc"],
        StrainBase | StrainBasePercentAdd | StrainPercent => &["strain"],
        SurplusValueBase | SurplusPercent => &["surplus"],
        ParryValueBase | VitalityToParryValueCof => &["parry_val"],
        ParryBase | ParryValuePercent => &["parry"],
        HasteBase | UnlimitedAdditionalHastePercent => &["haste"],
        // 不在实时面板里
        _ => &[],
    }
}

/// 扫所有 BuffDef.effects，生成 buff_id → 影响的 attr_keys 列表
/// 仅记录至少影响一个面板属性的 buff
pub fn collect_buff_attr_keys(version: GameVersion) -> std::collections::HashMap<u32, Vec<&'static str>> {
    let mut out = std::collections::HashMap::new();
    for def in all_buff_defs_by_version(version) {
        let mut keys: Vec<&'static str> = Vec::new();
        for e in def.effects {
            for k in affected_attr_keys(e.field) {
                if !keys.contains(k) { keys.push(k); }
            }
        }
        if !keys.is_empty() {
            out.insert(def.buff_id, keys);
        }
    }
    out
}

/// 把 EffectEntry 格式化为用户可读字符串（"+10%" / "+102级" / "×0.198"）
/// 给前端 hover 展示"嗜血 +10%"这种带数值的来源用
pub fn format_effect_value(field: crate::AttribField, value: f64) -> String {
    use crate::AttribField::*;
    match field {
        // 1024 制百分比字段
        PhysicsAttackPowerPercent | PhysicsOvercomePercent
        | StrainBasePercentAdd | StrainPercent
        | PhysicsCriticalStrikePercent | PhysicsCriticalDamagePowerPercent
        | AllDamageAddPercent | PveAddition | AllShieldIgnorePercent
        | SurplusPercent | UnlimitedAdditionalHastePercent
        | VitalityBasePercentAdd
        | TargetPhysicsShieldPercent | TargetDamageBonusPercent
            => format!("{:+.1}%", value / 1024.0 * 100.0),
        // 招架率 N/10000
        ParryValuePercent => format!("{:+.2}%", value / 10000.0 * 100.0),
        // 主属性 → 副属性转化系数：N/1024 倍
        VitalityToParryValueCof | VitalityToAttackCof | VitalityToOvercomeCof
            => format!("×{:.3}", value / 1024.0),
        // 等级数值加算（PhysicsAttackPowerBase 等）：直接级数
        _ => {
            let v = value.round() as i64;
            if v >= 0 { format!("+{}", v) } else { format!("{}", v) }
        },
    }
}

/// 扫所有 BuffDef.effects，生成 buff_id → { attr_key → 单层贡献格式化字符串 } 映射
/// 同一 buff 的多个 effects 落在同一 attr_key 时用 " / " 串联（如"+102级 / +15%"）
/// 给前端 hover 展示"嗜血 ×3  +10%"这种带数值的小字注解用
pub fn collect_buff_attr_desc(version: GameVersion)
    -> std::collections::HashMap<u32, std::collections::HashMap<String, String>>
{
    let mut out: std::collections::HashMap<u32, std::collections::HashMap<String, String>> = std::collections::HashMap::new();
    for def in all_buff_defs_by_version(version) {
        let mut m: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for e in def.effects {
            let desc = format_effect_value(e.field, e.value);
            for k in affected_attr_keys(e.field) {
                let key = (*k).to_string();
                m.entry(key)
                    .and_modify(|s| { s.push_str(" / "); s.push_str(&desc); })
                    .or_insert_with(|| desc.clone());
            }
        }
        if !m.is_empty() {
            out.insert(def.buff_id, m);
        }
    }
    out
}

// ─── 技能脚本路由 ───────────────────────────────────────────

fn get_skill_script(player: &Player, skill_id: u32) -> Option<SkillScriptFn> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::get_skill_script(skill_id),
    }
}

pub fn run_scripts(player: &mut Player, skill: &SkillSpec, cast_time: f64) -> ScriptEmitter {
    let _t0 = std::time::Instant::now();
    let mut em = ScriptEmitter::new();
    if let Some(script) = get_skill_script(player, skill.skill_id) {
        script(player, &mut em, cast_time);
    }
    // 大附魔触发后处理（腰/腕/鞋；帽走 aggregate_buff_fields 不在这里）
    crate::equip_effects::on_post_cast(player, &mut em, skill, cast_time);
    let _ns = _t0.elapsed().as_nanos() as u64;
    crate::perf_add(|p| { p.run_scripts_n += 1; p.run_scripts_ns += _ns; });
    em
}

// ─── Buff 脚本路由（签名新增 &Player）─────────────────────

pub fn get_buff_on_tick(player: &Player, buff_id: u32) -> Option<SkillScriptFn> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::get_buff_on_tick(buff_id),
    }
}

pub fn get_buff_on_expire(player: &Player, buff_id: u32) -> Option<SkillScriptFn> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::get_buff_on_expire(buff_id),
    }
}

pub fn get_buff_on_remove(player: &Player, buff_id: u32) -> Option<SkillScriptFn> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::get_buff_on_remove(buff_id),
    }
}

// ─── 版本化 helper 路由（main.rs 直接调用的） ─────────────

/// 动态覆盖 attack_coeff（卷雪刀按加速、卷云按奇穴等）
/// 返回 None 表示不覆盖，使用 TOML 原值
pub fn override_attack_coeff(player: &Player, spec: &SkillSpec) -> Option<f64> {
    match spec.skill_id {
        // 卷雪刀（平砍）：按当前加速实时算
        13039 => Some(juan_xue_attack_coeff(player)),
        // 盾刀：卷云奇穴覆盖三段系数
        13044 if player.has_talent(13321) => {
            match spec.name.as_str() {
                "盾刀·一段" => Some(1.47500),
                "盾刀·二段" => Some(1.69375),
                "盾刀·三段" => Some(1.93125),
                _ => None,
            }
        }
        _ => None,
    }
}

/// 卷雪刀（平砍）attack_coeff：按心法+版本决定
pub fn juan_xue_attack_coeff(player: &Player) -> f64 {
    let haste = player.effective_haste_level();
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::juan_xue_attack_coeff(haste),
    }
}

/// 卷雪刀（平砍）产卡
pub fn juan_xue_process_swings(player: &mut Player, to_time: f64) -> Vec<crate::CastEvent> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::juan_xue_process_swings(player, to_time),
    }
}

/// 绝刀运行时附加秘籍
pub fn jue_dao_runtime_recipes(player: &Player) -> Vec<u32> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::jue_dao_runtime_recipes(player),
    }
}

/// 绝刀按怒气段 effective_rage_cost
pub fn jue_dao_effective_rage_cost(player: &Player, skill: &SkillSpec) -> u32 {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::jue_dao_effective_rage_cost(player, skill),
    }
}

/// 盾挡按怒气分摊 10~100
pub fn dun_dang_effective_rage_cost(player: &Player) -> u32 {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::skills::dun_dang::effective_rage_cost(player),
    }
}

/// 战斗开始钩子（simulate 入口调用）：按奇穴激活心法/版本特有的常驻 buff
pub fn on_battle_start(player: &mut Player) {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::buffs::on_battle_start(player),
    }
}

/// 自身受击事件（Boss 周期攻击触发）
/// 处理坚铁叠层、招架判定、寒甲刷新等受击效果
pub fn on_player_hit(player: &mut Player, t: f64) -> Vec<crate::CastEvent> {
    match player.version {
        GameVersion::ShanHaiYuanLiu | GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest =>
            v2026_04_AnYingQianJi::on_hit::on_player_hit(player, t),
    }
}
