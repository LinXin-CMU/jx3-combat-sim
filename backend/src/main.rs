use axum::{
    extract::{Json, State},
    http::Method,
    response::{IntoResponse, Response},
    routing::{get, post, delete},
    Router,
};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, path::Path, sync::Arc};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

mod auth;
mod router;
pub mod agent;
mod buffs;
mod scripts;
pub mod equip;
pub mod expectation;
pub mod macro_engine;
pub mod macro_parser;
pub mod optimizer;
pub mod macro_eval;
pub mod macro_gen;
pub mod macro_prune;
pub mod rl;
pub mod equip_effects;

pub use buffs::*;
pub use equip_effects::*;

// ─────────────────────────────────────────────────────────────────────────────
// 数据结构
// ─────────────────────────────────────────────────────────────────────────────

/// 伤害类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DamageKind {
    #[default]
    Physical,      // 普通外功
    Magical,       // 普通内功
    SurplusOnly,   // 纯破招段
}

/// 角色属性面板输入
///
/// 主属性 + 面板值（已含心法 + 主属性 + 体质转化）
/// - shen_fa/li_dao/gen_gu/yuan_qi/vitality 仅用于 buff 加主属性时的增量重算
/// - base_attack 等是已含基础主属性的最终面板值
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Attributes {
    // 主属性（130 级基础：体质 45，其余 44）
    #[serde(default = "default_vitality")] pub vitality: f64,
    #[serde(default = "default_major")]    pub li_dao:   f64,
    #[serde(default = "default_major")]    pub gen_gu:   f64,
    #[serde(default = "default_major")]    pub yuan_qi:  f64,
    #[serde(default = "default_major")]    pub shen_fa:  f64,

    // 面板数值（已含心法+主属性转化）
    pub base_attack:                          f64,
    #[serde(default)] pub base_magical_attack:f64,
    pub weapon_damage:                        f64,
    #[serde(default)] pub surplus_value:      f64,
    pub crit_level:                           f64,
    pub crit_effect_level:                    f64,
    pub overcome_level:                       f64,
    pub strain_level:                         f64,
    pub haste_level:                          f64,
    /// 拆招值（铁骨衣防御向；寒甲奇穴计算用）
    #[serde(default)] pub parry_value:        f64,
    /// 招架等级
    #[serde(default)] pub parry_level:        f64,
}

fn default_vitality() -> f64 { 45.0 }
fn default_major()    -> f64 { 44.0 }

/// 计算后的战斗属性（含 buff/秘籍聚合后的最终值）
#[derive(Debug, Serialize, Clone)]
pub struct CombatStats {
    pub shen_fa:       f64,
    pub panel_attack:  f64,        // 最终外功面板攻击
    pub magical_attack:f64,        // 最终内功面板攻击
    pub base_attack:   f64,        // 用户输入的基础面板（不含 buff 加成）
    pub shenfa_attack: f64,        // 显示用：身法转化攻击
    pub crit_rate:     f64,        // 0~1
    pub crit_effect:   f64,        // 1.75 + ...
    pub overcome:      f64,        // 0~ 比例
    pub strain:        f64,        // 0~ 比例
    pub haste_rate:    f64,        // 0~1
    pub surplus_value: f64,        // 最终破招值
    pub parry_value:   f64,        // 最终拆招值
    pub parry_level:   f64,        // 最终招架等级
    pub parry_rate:    f64,        // 最终招架率（含基础 3% + 坚铁等加成，可 >1）
    pub vitality:      f64,        // 最终体质（含活血百分比加成）
    pub base_vitality: f64,        // 基础体质（不含百分比加成，振奋用）
}

/// 目标配置
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TargetConfig {
    /// 目标等级 (131~134)
    pub level: u32,
    /// 防御等级加成 (%)，默认 0
    #[serde(default)]
    pub defense_bonus: f64,
    /// 易伤系数（外功）
    #[serde(default)]
    pub damage_cof: f64,
}

/// 技能伤害请求
#[derive(Debug, Deserialize)]
pub struct SkillDamageRequest {
    pub attributes: Attributes,
    pub target: TargetConfig,
    /// 已选秘籍 ID
    #[serde(default)]
    pub recipes: Option<Vec<u32>>,
    /// 已选奇穴 ID
    #[serde(default)]
    pub talents: Option<Vec<u32>>,
    /// 铁骨气劲模式：0=关，1=铁骨（副T），2=铁骨·宿敌（主T）
    #[serde(default = "default_tiegu_mode")]
    pub tiegu_mode: u8,
    /// 装备（含大附魔/黄字特效 ENCHANT_X / YEFFECT_X 特殊 key）
    #[serde(default)]
    pub equipment: HashMap<String, u32>,
    /// 团队增益启用列表（基础设置走 apply_team_buffs_for_static：周期型按平均覆盖率折算）
    #[serde(default)]
    pub team_buffs: Vec<TeamBuffSelection>,
    /// 选中的阵法（None = 不开阵）
    #[serde(default)]
    pub formation: Option<FormationSelection>,
}

/// 单个技能伤害结果
#[derive(Debug, Serialize, Clone)]
pub struct SkillResult {
    pub name: String,
    /// 系数伤害（最终攻击×attack_coeff + base + 武器；破招段为 surplus_coeff×破招值）
    /// 不含任何增减伤、防御、会心
    pub coefficient_damage: f64,
    pub normal_damage: f64,
    pub crit_damage: f64,
    pub expected_damage: f64,
    /// 该技能实际生效的防御减伤率
    pub defense_rate: f64,
    /// 实际会心率 (%)
    pub crit_rate: f64,
    /// 实际破防 (%)
    pub overcome: f64,
    /// 实际无双 (%)
    pub strain: f64,
}

/// 技能伤害响应
#[derive(Debug, Serialize)]
pub struct SkillDamageResponse {
    /// 目标基础防御率（不含技能减防）
    pub base_defense_rate: f64,
    /// 等级压制系数（>1 增伤，<1 减伤）
    pub level_suppression: f64,
    pub skills: Vec<SkillResult>,
}

// ─────────────────────────────────────────────────────────────────────────────
// CD 数据模型
// ─────────────────────────────────────────────────────────────────────────────

/// CD 绑定模式
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CdMode {
    /// 检测并触发：受到此CD影响，释放后触发此CD
    CheckAndTrigger,
    /// 检测但不触发：受到此CD影响，但释放后不触发此CD
    CheckOnly,
}

/// 技能绑定的单个CD
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdBinding {
    /// CD 标识（如 "gcd_1.5", "cd_盾压", "protect_血怒"）
    pub cd_id: String,
    /// 基础持续时间（秒）
    pub duration: f64,
    /// 绑定模式
    pub mode: CdMode,
    /// 是否受加速影响
    #[serde(default)]
    pub haste: bool,
}

// ─────────────────────────────────────────────────────────────────────────────

/// 姿态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    #[default]
    Any,      // 任意姿态均可施展
    Shield,   // 擎盾
    Blade,    // 擎刀
    Wall,     // 盾墙
    NotWall,  // 非盾墙（擎盾或擎刀均可）
}

/// 连招跟随条目（宏/手动序列中主技能自动重定向到子技能）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComboFollowEntry {
    pub requires_combo: String,
    pub skill_id: u32,
}

/// TOML 配置文件 — 顶层
#[derive(Debug, Deserialize)]
struct SkillFileConfig {
    id: u32,
    name: String,
    #[serde(default)]
    description: String,
    /// 图标 URL（与团辅 toml.icon 一致，前端序列块 icon 模式直接渲染）
    #[serde(default)]
    icon: String,
    #[serde(default)]
    skip: bool,
    /// 被动技能（不加入 skill_map，仅用于伤害计算）
    #[serde(default)]
    passive: bool,
    /// 真实伤害（jx3 大附魔腕/鞋固伤等；仅走等级压制+易伤+全局增伤）
    #[serde(default)]
    true_damage: bool,
    /// 伤害类型（默认 physical）
    #[serde(default)]
    damage_kind: DamageKind,
    #[serde(default)]
    cooldowns: Vec<CdBinding>,
    #[serde(default)]
    channel_frame: Option<u32>,
    #[serde(default)]
    channel_interval: Option<u32>,
    /// 首跳延迟帧数（0 = 立即触发，默认等于 channel_interval）
    #[serde(default)]
    first_tick_frame: Option<u32>,
    /// 姿态要求（默认 any = 任意姿态）
    #[serde(default)]
    stance: Stance,
    /// 怒气消耗（顶层默认值，rank 可覆盖）
    #[serde(default)]
    rage_cost: u32,
    /// 怒气回复（顶层默认值，rank 可覆盖）
    #[serde(default)]
    rage_gain: u32,
    /// 施展后切换姿态
    #[serde(default)]
    stance_change: Option<Stance>,
    /// 需要的奇穴 ID（0 = 无要求）
    #[serde(default)]
    requires_talent: Option<u32>,
    /// 充能最大层数（0 = 非充能技能）
    #[serde(default)]
    max_charges: u32,
    /// 充能 CD（秒，每层恢复时间）
    #[serde(default)]
    charge_cd: f64,
    #[serde(default)]
    ranks: Vec<SkillRankConfig>,
    /// 连招重定向（TOML [[combo_follow]] 数组）
    #[serde(default)]
    combo_follow: Vec<ComboFollowEntry>,
}

/// TOML 配置文件 — 单个品级
#[derive(Debug, Deserialize)]
struct SkillRankConfig {
    rank: u32,
    name: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    base_damage: f64,
    #[serde(default)]
    attack_coeff: f64,
    #[serde(default)]
    weapon_coeff: f64,
    #[serde(default)]
    defense_ignore: f64,
    /// 破招系数（仅 damage_kind=surplus_only 时使用）
    #[serde(default, alias = "break_coeff")]
    surplus_coeff: f64,
    /// 怒气消耗（覆盖顶层）
    #[serde(default)]
    rage_cost: Option<u32>,
    /// 怒气回复（覆盖顶层）
    #[serde(default)]
    rage_gain: Option<u32>,
    /// 需要的连招状态（如 "盾刀_2"）
    #[serde(default)]
    requires_combo: Option<String>,
    /// 释放后授予的连招状态（如 "盾刀_2"）
    #[serde(default)]
    grants_combo: Option<String>,
    /// 连招 Buff 持续帧数（默认 64 帧 = 4秒）
    #[serde(default)]
    combo_duration: Option<u32>,
    /// 覆盖顶层 cooldowns（None = 继承顶层；Some = 本 rank 独立配置）
    /// 用于同技能不同段的 GCD 差异，如 盾刀·一段全触发 vs 二三四段只触发 1s GCD
    #[serde(default)]
    cooldowns: Option<Vec<CdBinding>>,
}

/// 内存中的技能规格
#[derive(Debug, Clone, Serialize)]
pub struct SkillSpec {
    pub skill_id: u32,
    pub name: String,
    pub description: String,
    /// 图标 URL（toml.icon 透传；空串 = 没图标，前端 icon 模式降级显示首字）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    pub damage_kind: DamageKind,
    pub base_damage: f64,
    pub attack_coeff: f64,
    pub weapon_coeff: f64,
    pub defense_ignore: f64,
    /// 破招系数（仅 damage_kind=surplus_only 时使用）
    pub surplus_coeff: f64,
    pub cooldowns: Vec<CdBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_frame: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_interval: Option<u32>,
    /// 首跳延迟帧数（0 = 立即触发，None = 等于 channel_interval）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_tick_frame: Option<u32>,
    /// 姿态要求
    pub stance: Stance,
    /// 怒气消耗
    pub rage_cost: u32,
    /// 怒气回复
    pub rage_gain: u32,
    /// 施展后切换姿态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stance_change: Option<Stance>,
    /// 需要的奇穴 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_talent: Option<u32>,
    /// 需要的连招状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_combo: Option<String>,
    /// 释放后授予的连招状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grants_combo: Option<String>,
    /// 连招 Buff 持续帧数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combo_duration: Option<u32>,
    /// 充能最大层数（0 = 非充能技能）
    pub max_charges: u32,
    /// 充能 CD（秒）
    pub charge_cd: f64,
    /// 被动技能（不加入 skill_map）
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub passive: bool,
    /// 真实伤害（jx3 大附魔腕/鞋固伤等）：仅走 Step 5 等级压制 + Step 7 易伤 + Step 8 全局增伤
    /// 跳过：Step 2 秘籍增伤 / Step 3 破防×无双 / Step 4 防御 / Step 6 PVE+35% / Step 9 会心
    /// 参考 jx3dps-online src/计算模块/郭氏计算/技能伤害公式.ts
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub true_damage: bool,
    /// 连招重定向：宏/序列写本技能时，检查 combo buff 自动切换子技能
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub combo_follow: Vec<ComboFollowEntry>,
}

/// reload 响应
#[derive(Serialize)]
pub struct ReloadResult {
    pub loaded: usize,
    pub skills: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 常量
// ─────────────────────────────────────────────────────────────────────────────

// ── 130级 副属性 SCALE 常量（已固化）──
const LP_CRIT: f64     = 197_703.0;
const LP_CRIT_EFF: f64 =  72_844.2;
const LP_STRAIN: f64   = 133_333.2;
const LP_OVERCOME: f64 = 225_957.6;
const LP_HASTE: f64    = 210_078.0;
const LP_PARRY: f64    = 107_553.6;  // 130级招架等级 → 招架率 折算

// ── 心法 / 武学版本 枚举（Phase 0 抽象）──
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Mount { FenShanJin, TieGuYi }
impl Default for Mount { fn default() -> Self { Mount::FenShanJin } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GameVersion {
    /// 山海源流（2025.10）
    ShanHaiYuanLiu,
    /// 暗影千机（2026.04）
    AnYingQianJi,
    /// 暗影千机·测试服（基于 2026.04，独立目录供实验）
    AnYingQianJiTest,
}
impl Default for GameVersion { fn default() -> Self { GameVersion::AnYingQianJi } }

/// 心法常量：主属性 → 副属性转化系数（仅外功线 buff 加成时使用）
/// 从 school.toml 注入 Player（Phase 2）；当前全部默认分山劲值
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MountConstants {
    pub shenfa_to_attack:  f64, // 身法 → 外功攻击
    pub shenfa_to_crit:    f64, // 身法 → 外功会心等级
    pub lidao_to_attack:   f64, // 力道 → 外功攻击
    pub lidao_to_overcome: f64, // 力道 → 外功破防
    pub yuanqi_to_attack:  f64, // 元气 → 内功攻击
    pub vitality_to_attack: f64, // 体质 → 外功攻击（铁骨衣特化；分山劲为 0）
    pub vitality_to_parry_value: f64, // 体质 → 拆招值（铁骨衣 2.25；分山劲为 0）
    pub vitality_to_parry_level: f64, // 体质 → 招架等级（铁骨衣 0.18；分山劲为 0）
    pub parry_base_rate: f64,         // 心法自带基础招架率（全心法 +3% = 0.03）
    pub non_player_bonus: f64,        // 非侠士增伤（1024 制，358=35%，61=6%）
}

impl MountConstants {
    pub fn fenshanjin_default() -> Self {
        MountConstants {
            shenfa_to_attack:  1.88,
            shenfa_to_crit:    0.9,
            lidao_to_attack:   0.163,
            lidao_to_overcome: 0.3,
            yuanqi_to_attack:  0.181,
            vitality_to_attack: 0.0,
            vitality_to_parry_value: 0.0,
            vitality_to_parry_level: 0.0,
            parry_base_rate: 0.03,
            non_player_bonus: 358.0,
        }
    }
    pub fn tieguyi_default() -> Self {
        MountConstants {
            shenfa_to_attack:  0.0,    // 铁骨衣身法不转外攻
            shenfa_to_crit:    0.9,
            lidao_to_attack:   0.163,
            lidao_to_overcome: 0.3,
            yuanqi_to_attack:  0.181,
            vitality_to_attack: 0.04,  // 铁骨衣每点体质 +0.04 外攻
            vitality_to_parry_value: 2.25, // 每点体质 +2.25 拆招值
            vitality_to_parry_level: 0.18, // 每点体质 +0.18 招架等级
            parry_base_rate: 0.03,         // 基础招架率（全心法自带）
            non_player_bonus: 358.0,
        }
    }
    pub fn for_mount(mount: Mount) -> Self {
        match mount {
            Mount::FenShanJin => Self::fenshanjin_default(),
            Mount::TieGuYi    => Self::tieguyi_default(),
        }
    }
}

impl Default for MountConstants {
    fn default() -> Self { Self::fenshanjin_default() }
}

// ── school.toml 反序列化 ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchoolUiGroup {
    pub label: String,
    pub skills: Vec<String>,
    /// 渲染时该组前是否换行（前端用 sim-skill-bar-break 实现）
    #[serde(default)]
    pub new_row: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchoolUiComboReplace {
    pub combo: String,
    pub replaces: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SchoolUi {
    #[serde(default)]
    pub skill_groups: Vec<SchoolUiGroup>,
    #[serde(default)]
    pub combo_replace: Vec<SchoolUiComboReplace>,
    #[serde(default = "default_talent_tiers")]
    pub talent_tiers: u32,
}

fn default_talent_tiers() -> u32 { 7 }

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WorkflowA {
    #[serde(default)]
    pub default_consistency: Vec<String>,
    #[serde(default)]
    pub consistency_sensitive_skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SchoolConstantsToml {
    shenfa_to_attack:  f64,
    shenfa_to_crit:    f64,
    lidao_to_attack:   f64,
    lidao_to_overcome: f64,
    yuanqi_to_attack:  f64,
    #[serde(default)]
    vitality_to_attack: f64,
    #[serde(default)]
    vitality_to_parry_value: f64,
    #[serde(default)]
    vitality_to_parry_level: f64,
    #[serde(default = "default_parry_base_rate")]
    parry_base_rate: f64,
    #[serde(default = "default_non_player_bonus")]
    non_player_bonus: f64,
}

fn default_parry_base_rate() -> f64 { 0.03 }
fn default_non_player_bonus() -> f64 { 358.0 }

// 心法固定增益 / 心法转化 类型由 equip 模块定义（同名结构体，配装器内独享）
pub use equip::{MountBaseStats, MountConversions};

#[derive(Debug, Clone, Deserialize)]
struct SchoolToml {
    #[allow(dead_code)] name: String,
    #[allow(dead_code)] class: String,
    constants: SchoolConstantsToml,
    #[serde(default)]
    base_stats: equip::MountBaseStats,
    #[serde(default)]
    mount_conversions: equip::MountConversions,
    #[serde(default)]
    ui: SchoolUi,
    #[serde(default)]
    workflow_a: WorkflowA,
}

/// 加载 school.toml，返回 (constants, base_stats, conversions, ui, workflow_a)
pub fn load_school_toml(version: GameVersion, mount: Mount)
    -> Result<(MountConstants, equip::MountBaseStats, equip::MountConversions, SchoolUi, WorkflowA), String>
{
    let path = school_toml_path(version, mount);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path, e))?;
    let s: SchoolToml = toml::from_str(&text)
        .map_err(|e| format!("parse {}: {}", path, e))?;
    let c = MountConstants {
        shenfa_to_attack:  s.constants.shenfa_to_attack,
        shenfa_to_crit:    s.constants.shenfa_to_crit,
        lidao_to_attack:   s.constants.lidao_to_attack,
        lidao_to_overcome: s.constants.lidao_to_overcome,
        yuanqi_to_attack:  s.constants.yuanqi_to_attack,
        vitality_to_attack: s.constants.vitality_to_attack,
        vitality_to_parry_value: s.constants.vitality_to_parry_value,
        vitality_to_parry_level: s.constants.vitality_to_parry_level,
        parry_base_rate: s.constants.parry_base_rate,
        non_player_bonus: s.constants.non_player_bonus,
    };
    Ok((c, s.base_stats, s.mount_conversions, s.ui, s.workflow_a))
}

// ── 伤害链常量 ──
// 破招公式 = surplus_coeff × 破招值；TOML 里的 surplus_coeff 已含游戏内 7.421 常数
const BASE_CRIT_POWER: f64     = 1.75;   // 基础会心效果（100% + 75%）
const PLAYER_LEVEL: u32        = 130;

/// 自动检测数据目录：开发环境 ./skills，发布环境 backend/skills
fn data_path(dev: &str, release: &str) -> &'static str {
    if Path::new(dev).exists() { return Box::leak(dev.to_string().into_boxed_str()); }
    Box::leak(release.to_string().into_boxed_str())
}

/// 版本目录名（对齐 `scripts/v{version}/` 命名风格）。
/// 测试服枚举保留用于旧存档兼容；测试服归档未作为可选版本公开，运行时回退正式服数据。
pub fn version_dir_name(v: GameVersion) -> &'static str {
    match v {
        GameVersion::ShanHaiYuanLiu => "2025_10_山海源流",
        GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest => "2026_04_暗影千机",
    }
}

/// 心法目录名（中文原名）
pub fn mount_dir_name(m: Mount) -> &'static str {
    match m {
        Mount::FenShanJin => "分山劲",
        Mount::TieGuYi    => "铁骨衣",
    }
}

/// 数据根目录（dev: ./data；release: backend/data）
fn data_root() -> &'static str {
    data_path("./data", "backend/data")
}

/// 心法特有数据目录 data/{version}/{mount}
fn mount_root(version: GameVersion, mount: Mount) -> String {
    format!("{}/{}/{}", data_root(), version_dir_name(version), mount_dir_name(mount))
}

/// 版本级共享数据目录 data/{version}
fn version_root(version: GameVersion) -> String {
    format!("{}/{}", data_root(), version_dir_name(version))
}

fn skills_dir(version: GameVersion, mount: Mount) -> String {
    format!("{}/skills", mount_root(version, mount))
}
fn talents_file(version: GameVersion, mount: Mount) -> String {
    format!("{}/talents.toml", mount_root(version, mount))
}
fn recipes_file(version: GameVersion) -> String {
    // 秘籍按版本共享（两心法同一份）
    format!("{}/recipes.toml", version_root(version))
}
fn school_toml_path(version: GameVersion, mount: Mount) -> String {
    format!("{}/school.toml", mount_root(version, mount))
}
fn mount_defaults_path(version: GameVersion, mount: Mount) -> String {
    format!("{}/defaults.json", mount_root(version, mount))
}

fn macro_save_path() -> std::path::PathBuf {
    user_data_path("macros.json")
}

fn attrs_save_path(mount: Mount) -> std::path::PathBuf {
    user_data_path(&format!("attrs_{}.json", mount_dir_name(mount)))
}

/// userdata 根目录。进程隔离时由 Router 通过 `JX3_USERDATA_DIR` 给每个 worker 指定独立目录，
/// 从而所有写入（宏/属性/配装/循环存档/whitelist/icon 缓存）天然按用户隔离。
/// 未设该环境变量时回落到原逻辑（本地单机使用不变）。
pub fn userdata_base() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("JX3_USERDATA_DIR") {
        if !d.trim().is_empty() {
            let p = std::path::PathBuf::from(d);
            if !p.exists() { let _ = std::fs::create_dir_all(&p); }
            return p;
        }
    }
    let dir = if Path::new("./userdata").exists() || Path::new("./skills").exists() {
        "./userdata"
    } else {
        "backend/userdata"
    };
    let p = Path::new(dir).to_path_buf();
    if !p.exists() { let _ = std::fs::create_dir_all(&p); }
    p
}

fn user_data_path(file: &str) -> std::path::PathBuf {
    userdata_base().join(file)
}

fn user_data_dir() -> std::path::PathBuf {
    userdata_base()
}

/// icon 缓存文件路径。icon 是只读公共资源（非用户数据），进程隔离时由 Router 通过
/// `JX3_ICON_CACHE_DIR` 指向一个**所有 worker 共享**的目录，避免每个用户的 worker 各下一份 135 张。
fn icon_cache_file(id: u32) -> std::path::PathBuf {
    if let Ok(d) = std::env::var("JX3_ICON_CACHE_DIR") {
        if !d.trim().is_empty() {
            let p = std::path::PathBuf::from(d);
            if !p.exists() { let _ = std::fs::create_dir_all(&p); }
            return p.join(format!("{}.png", id));
        }
    }
    user_data_path(&format!("icon_cache/{}.png", id))
}

fn sanitize_profile_name(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() || matches!(ch, '_' | '-' | ' ')
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
        {
            out.push(ch);
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() || out.contains("..") { None } else { Some(out) }
}

const FRAMES_PER_SEC: u32 = 16;

// ─────────────────────────────────────────────────────────────────────────────
// 加速帧数计算
// ─────────────────────────────────────────────────────────────────────────────

/// 加速阈值算法：根据原始帧数和加速等级计算实际帧数
pub fn get_actual_frames(original_frames: u32, haste_level: u32) -> u32 {
    let num = 1024u64 * original_frames as u64;
    let den = (1024u64 * haste_level as u64) / LP_HASTE as u64 + 1024;
    (num / den) as u32
}

/// 加速等级使某基底帧数下降一段（actual = original - 1, original - 2, ...）所需的 haste 区间。
///
/// 对每段返回 `(tier, actual_frames, haste_min, haste_max)`：在 `[haste_min, haste_max]` 内，
/// `get_actual_frames(orig, h) == actual_frames`。
///
/// 自动停在 `haste_max >= cap` 之处（cap 默认 50000，覆盖 130 级合理 haste 上限）。
/// 配装搜索把"目标加速段"作为硬约束 → 直接读这表的 `(haste_min, haste_max)`。
pub fn haste_tier_boundaries(original_frames: u32, cap: u32) -> Vec<(u32, u32, u32, u32)> {
    let mut out: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut prev_actual = u32::MAX;
    let mut prev_min = 0u32;
    for h in 0..=cap {
        let a = get_actual_frames(original_frames, h);
        if a != prev_actual {
            if prev_actual != u32::MAX {
                let tier = original_frames.saturating_sub(prev_actual);
                out.push((tier, prev_actual, prev_min, h - 1));
            }
            prev_actual = a;
            prev_min = h;
        }
    }
    let tier = original_frames.saturating_sub(prev_actual);
    out.push((tier, prev_actual, prev_min, cap));
    out
}

/// 秒转帧（四舍五入）
fn sec_to_frames(sec: f64) -> u32 {
    (sec * FRAMES_PER_SEC as f64).round() as u32
}

/// 帧转秒
fn frames_to_sec(frames: u32) -> f64 {
    frames as f64 / FRAMES_PER_SEC as f64
}


// ─────────────────────────────────────────────────────────────────────────────
// 防御计算
// ─────────────────────────────────────────────────────────────────────────────

/// 各等级目标基础防御等级
fn target_base_defense(level: u32) -> f64 {
    match level {
        131 => 33_338.0,
        132 => 46_901.0,
        133 => 79_721.0,
        134 => 83_679.0,
        _   =>      0.0,
    }
}

/// 各等级防御转化系数
fn defense_level_param(level: u32) -> f64 {
    match level {
        130 => 126_007.20,
        131 => 133_357.62,
        132 => 140_708.04,
        133 => 148_058.46,
        134 => 155_408.88,
        _   => 126_007.20,
    }
}

// calc_defense_rate 已合并到 calc_defense_rate_with_ignore（伤害链 Step 4 的同一份实现）

// ─────────────────────────────────────────────────────────────────────────────
// 等级压制
// ─────────────────────────────────────────────────────────────────────────────

/// 计算等级压制系数
/// - 玩家等级 > 目标等级：每级 +15% 增伤，最高 +150%
/// - 玩家等级 < 目标等级：每级 -5%  减伤，最高 -50%
fn calc_level_suppression(player_level: u32, target_level: u32) -> f64 {
    let diff = target_level as i32 - player_level as i32;
    if diff > 0 {
        let penalty = (diff as f64 * 0.05).min(0.50);
        1.0 - penalty
    } else if diff < 0 {
        let bonus = ((-diff) as f64 * 0.15).min(1.50);
        1.0 + bonus
    } else {
        1.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 伤害计算（9 步取整链）
// ─────────────────────────────────────────────────────────────────────────────

/// 字段聚合槽（同字段多 buff 加总）
pub type AttribSlots = HashMap<AttribField, f64>;

fn slot(slots: &AttribSlots, f: AttribField) -> f64 {
    slots.get(&f).copied().unwrap_or(0.0)
}

/// 聚合自身 buff 的所有 effects（同字段累加，每项按 inst.stacks 倍乘）
/// 同时追加奇穴常驻加成（不依赖 buff）
pub fn aggregate_buff_fields(player: &Player) -> AttribSlots {
    let _t0 = std::time::Instant::now();
    let mut slots: AttribSlots = HashMap::new();
    let t = player.current_time;
    for inst in &player.active_buffs {
        if inst.expires_at != 0.0 && inst.expires_at <= t { continue; }
        let def = match player.buff_def(inst.buff_id) { Some(d) => d, None => continue };
        // 期望传播 buff 用 expected_stacks（连续浮点）作为乘数；常规 buff 用 stacks
        let mult = inst.expected_stacks.unwrap_or(inst.stacks as f64);
        for e in def.effects {
            *slots.entry(e.field).or_insert(0.0) += e.value * mult;
        }
        // 实例级动态 effects（由脚本在施展时挂上，如盾挡按 level 查的 cof）
        for e in &inst.extra_effects {
            *slots.entry(e.field).or_insert(0.0) += e.value * mult;
        }
    }
    // 奇穴 13126「恋战」常驻 +10% 招式伤害（全局增伤，含破招）
    if player.has_talent(13126) {
        *slots.entry(AttribField::AllDamageAddPercent).or_insert(0.0) += 102.0;
    }
    // 奇穴 13124「活血」常驻 +10% 体质（atVitalityBasePercentAdd = 102/1024）
    if player.has_talent(13124) {
        *slots.entry(AttribField::VitalityBasePercentAdd).or_insert(0.0) += 102.0;
    }
    // 奇穴 13366「从容」常驻（假设血量>60%）+10% 外功攻击 +15% 无双等级
    if player.has_talent(13366) {
        *slots.entry(AttribField::PhysicsAttackPowerPercent).or_insert(0.0) += 102.0;
        *slots.entry(AttribField::StrainBasePercentAdd).or_insert(0.0) += 154.0;
    }
    // ── 装备特效 / 黄字特效 结算顺序 ──
    // 站立/进战类 buff 已在 simulate 入口通过 apply_combat_start_buffs 挂上 buff 实例，
    // 上面的 buff 实例聚合循环会自动叠到 slots，无需在这里硬编码。
    //
    // VitalityToParryValueCof → ParryValueBase：按"基础体质" × ∑cof / 1024 追加
    let cof = slots.get(&AttribField::VitalityToParryValueCof).copied().unwrap_or(0.0);
    if cof != 0.0 {
        let bonus = player.effective_base_vitality() * cof / 1024.0;
        *slots.entry(AttribField::ParryValueBase).or_insert(0.0) += bonus;
    }

    // 阵法永久 effects（self/any → permanent_effects；other → public_effects_other）
    // 在 yellow_post_pass 之前注入，让 max-pick 等阈值类能感知阵法加的等级
    for (k, v) in &player.formation_permanent_slots {
        *slots.entry(*k).or_insert(0.0) += *v;
    }

    // POST-PASS：阈值/最大值类（必须基于其它 buff 全部聚合后的 level 决策）
    //  - 帽 38934 max-pick: 取破/会/招最大那个 → +3471
    //  - 项链 38945 / 38946: 每 8586 stat → 1 层（max 10）转化
    crate::equip_effects::apply_yellow_post_pass(player, &mut slots);

    let _ns = _t0.elapsed().as_nanos() as u64;
    perf_add(|p| { p.aggregate_n += 1; p.aggregate_ns += _ns; });
    slots
}

/// 聚合目标 buff/debuff 的所有 effects（同字段累加，每项按 inst.stacks 倍乘）
pub fn aggregate_target_buff_fields(player: &Player) -> AttribSlots {
    let mut slots: AttribSlots = HashMap::new();
    let t = player.current_time;
    for inst in &player.target_buffs {
        if inst.expires_at != 0.0 && inst.expires_at <= t { continue; }
        let def = match player.buff_def(inst.buff_id) { Some(d) => d, None => continue };
        let mult = inst.expected_stacks.unwrap_or(inst.stacks as f64);
        for e in def.effects {
            *slots.entry(e.field).or_insert(0.0) += e.value * mult;
        }
        // 实例级动态 effects（虚弱按 level 写 -51/-72 走这里）—— 与 aggregate_buff_fields 对齐
        for e in &inst.extra_effects {
            *slots.entry(e.field).or_insert(0.0) += e.value * mult;
        }
    }
    slots
}

/// 秘籍倒排索引（owned indices，可放进 thread_local 缓存）：
/// - 有 skill_filter 的秘籍：skill_id → 适用项的 index
/// - 无 skill_filter 的秘籍：skill 字段名 → 适用项的 index
pub struct RecipeIndex {
    pub by_skill_id: ahash::AHashMap<u32, Vec<usize>>,
    pub by_skill_name: ahash::AHashMap<String, Vec<usize>>,
    /// 当前缓存对应的 recipes_table 指针（识别表是否换过）
    pub source_ptr: usize,
    pub source_len: usize,
}

impl RecipeIndex {
    pub fn build(table: &[RecipeEntry]) -> Self {
        let mut by_skill_id: ahash::AHashMap<u32, Vec<usize>> = ahash::AHashMap::new();
        let mut by_skill_name: ahash::AHashMap<String, Vec<usize>> = ahash::AHashMap::new();
        for (idx, r) in table.iter().enumerate() {
            if !r.skill_filter.is_empty() {
                for &sid in &r.skill_filter {
                    by_skill_id.entry(sid).or_default().push(idx);
                }
            } else if !r.skill.is_empty() {
                by_skill_name.entry(r.skill.clone()).or_default().push(idx);
            }
        }
        Self {
            by_skill_id, by_skill_name,
            source_ptr: table.as_ptr() as usize,
            source_len: table.len(),
        }
    }
}

thread_local! {
    static RECIPE_INDEX: std::cell::RefCell<Option<RecipeIndex>> = std::cell::RefCell::new(None);
    /// 缓存当前激活的 recipe ID 集合，按 buff_generation 失效。
    /// 父事件 + N 个触发子事件共享同一个生成代，避免每次都重建 50 元素 HashSet。
    static ACTIVE_IDS: std::cell::RefCell<Option<(u64, ahash::AHashSet<u32>)>> = std::cell::RefCell::new(None);
}

/// 在 simulate_core 入口调用：确保 thread_local 索引对应当前 recipes_table；变了就重建
pub fn ensure_recipe_index(table: &[RecipeEntry]) {
    let ptr = table.as_ptr() as usize;
    let len = table.len();
    RECIPE_INDEX.with(|cell| {
        let needs_rebuild = match &*cell.borrow() {
            Some(idx) => idx.source_ptr != ptr || idx.source_len != len,
            None => true,
        };
        if needs_rebuild {
            *cell.borrow_mut() = Some(RecipeIndex::build(table));
        }
    });
}

// 装备 ID 常量（橙武 / T 套 / JJC / 威望 / 守护 T）+ 神兵·无双气劲数值表
// 集中在 backend/src/equip_effects.rs，main.rs 通过 `pub use equip_effects::*` 导入

/// 索引版 collect_recipes —— O(候选数 × 1) 而不是 O(全表 74)
/// active_ids 集合按 buff_generation 缓存：父事件构建一次，N 个触发子事件复用
pub fn collect_recipes_indexed<'a>(
    player: &Player,
    skill_id: u32,
    skill_base_name: &str,
    runtime_extra: &[u32],
    recipes_table: &'a [RecipeEntry],
) -> Vec<&'a RecipeEntry> {
    let _t0 = std::time::Instant::now();

    // 1. 准备 cached active set（按 buff_generation 失效）
    ACTIVE_IDS.with(|cell| {
        let mut c = cell.borrow_mut();
        let stale = c.as_ref().map(|(g, _)| *g != player.buff_generation).unwrap_or(true);
        if stale {
            let mut s = ahash::AHashSet::with_capacity(64);
            s.extend(player.active_recipes.iter().copied());
            s.extend(player.buff_recipes.keys().copied());
            if player.has_talent(21281) { s.insert(99240); s.insert(99241); }
            if player.has_talent(14838) { s.insert(99242); }
            if player.has_talent(13317) { s.insert(99310); s.insert(99311); }
            // 天下宏愿（分山）主武器装备特效：盾飞 +5% / 斩刀 +5%
            if player.has_equip_in("PRIMARY_WEAPON", &TIANXIA_HONGYUAN_WEAPON_IDS) {
                s.insert(99260); s.insert(99261);
            }
            // 幽烽蝶语·式微（外功小橙武）：盾刀会心+5% / 斩刀会心+5%
            if player.has_equip_in("PRIMARY_WEAPON", YOU_FENG_DIE_YU_SHI_WEI_IDS) {
                s.insert(99262); s.insert(99263);
            }
            // 苍云外功 T 套 4 件套：绝刀+10% / 盾压+10%（5936 / 6481 / 6782 任意一套凑齐 4 件）
            if player.count_equip_in(CY_DPS_T_SET_IDS) >= 4 {
                s.insert(99270); s.insert(99271);
            }
            s.insert(99341); s.insert(99342); s.insert(99343);
            *c = Some((player.buff_generation, s));
        }
    });

    // 2. 用 cached active + runtime_extra（极小集合，inline 检查）查询索引
    let out: Vec<&RecipeEntry> = ACTIVE_IDS.with(|aid_cell| {
        let aid_ref = aid_cell.borrow();
        let active = &aid_ref.as_ref().unwrap().1;

        RECIPE_INDEX.with(|ri_cell| {
            let ri_ref = ri_cell.borrow();
            let index = ri_ref.as_ref().expect("ensure_recipe_index() must be called before");

            let is_active = |rid: u32| -> bool {
                active.contains(&rid) || runtime_extra.iter().any(|&x| x == rid)
            };

            // 收集"命中的表索引"，再按索引排序 → 还原表顺序 → 与原 collect_recipes bit-equal
            let mut hit_indices: smallvec::SmallVec<[usize; 16]> = smallvec::SmallVec::new();
            if let Some(list) = index.by_skill_id.get(&skill_id) {
                for &i in list {
                    if is_active(recipes_table[i].id) { hit_indices.push(i); }
                }
            }
            if let Some(list) = index.by_skill_name.get(skill_base_name) {
                for &i in list {
                    if is_active(recipes_table[i].id) { hit_indices.push(i); }
                }
            }
            hit_indices.sort_unstable();
            hit_indices.into_iter().map(|i| &recipes_table[i]).collect()
        })
    });
    let _ns = _t0.elapsed().as_nanos() as u64;
    perf_add(|p| { p.collect_recipes_n += 1; p.collect_recipes_ns += _ns; });
    out
}

/// 收集本次伤害事件应用的秘籍（用户配 + buff 激活 + 瞬时 runtime；按 skill_id 过滤）
pub fn collect_recipes<'a>(
    player: &Player,
    skill_id: u32,
    skill_base_name: &str,
    runtime_extra: &[u32],
    recipes_table: &'a [RecipeEntry],
) -> Vec<&'a RecipeEntry> {
    let _t0 = std::time::Instant::now();
    use std::collections::HashSet;
    let mut ids: HashSet<u32> = player.active_recipes.iter().copied().collect();
    ids.extend(player.buff_recipes.keys().copied());
    ids.extend(runtime_extra.iter().copied());
    // 奇穴 21281「嗜血」常驻激活 99240（绝刀+40%）+ 99241（双会），不依赖 buff
    if player.has_talent(21281) {
        ids.insert(99240);
        ids.insert(99241);
    }
    // 奇穴 14838「刀煞」常驻激活 99242（绝刀+破·绝刀无视 100% 外功防御）
    if player.has_talent(14838) {
        ids.insert(99242);
    }
    // 奇穴 13317「赴敌」常驻激活 99310（盾猛+10%）+ 99311（斩刀+10%）
    if player.has_talent(13317) {
        ids.insert(99310);
        ids.insert(99311);
    }
    // 天下宏愿（分山）主武器装备特效：99260 盾飞+5% / 99261 斩刀+5%
    if player.has_equip_in("PRIMARY_WEAPON", &TIANXIA_HONGYUAN_WEAPON_IDS) {
        ids.insert(99260);
        ids.insert(99261);
    }
    // 技能内置加成（常驻激活，由 skill_filter 限制范围）
    ids.insert(99341); // 麟光甲寒·非侠士+150%
    ids.insert(99342); // 阵云结晦系列·非侠士+200%
    ids.insert(99343); // 阵云·雾海寻龙·非侠士+60%
    let out: Vec<&RecipeEntry> = recipes_table.iter()
        .filter(|r| ids.contains(&r.id))
        .filter(|r| r.applies_to(skill_id, skill_base_name))
        .collect();
    let _ns = _t0.elapsed().as_nanos() as u64;
    perf_add(|p| { p.collect_recipes_n += 1; p.collect_recipes_ns += _ns; });
    out
}

/// 由 Attributes + buff 字段 + 心法常量 推出最终战斗属性面板
pub fn build_runtime_stats(attr: &Attributes, slots: &AttribSlots, constants: &MountConstants) -> CombatStats {
    // 主属性增量重算（仅用于 buff 加主属性时）
    // 用户输入的面板已含基础主属性的转化，所以这里只算"buff 多给的主属性"带来的增量
    let extra_strength = slot(slots, AttribField::StrengthBase)
                       + slot(slots, AttribField::BasePotentialAdd);
    let extra_shenfa   = slot(slots, AttribField::AgilityBase)
                       + slot(slots, AttribField::BasePotentialAdd);
    let extra_yuanqi   = slot(slots, AttribField::SpunkBase)
                       + slot(slots, AttribField::BasePotentialAdd);
    let extra_vitality_raw = slot(slots, AttribField::VitalityBase)
                           + slot(slots, AttribField::BasePotentialAdd);
    // 体质百分比加成（活血奇穴 etc.）：(面板 + 加算增量) × (1 + N/1024) - 面板 = 最终增量
    let vitality_pct = slot(slots, AttribField::VitalityBasePercentAdd);
    let extra_vitality = if vitality_pct != 0.0 {
        let total = attr.vitality + extra_vitality_raw;
        (total * (1.0 + vitality_pct / 1024.0)).floor() - attr.vitality
    } else {
        extra_vitality_raw
    };

    // 外功攻击：基础攻击 + 身法×shenfa_to_attack（心法常量） + 体质×vitality_to_attack（铁骨衣） + buff 加成 + 力道增量×lidao_to_attack
    let final_vitality = attr.vitality + extra_vitality;
    let mut panel_attack = attr.base_attack
        + (attr.shen_fa * constants.shenfa_to_attack).floor()
        + (final_vitality * constants.vitality_to_attack).floor()
        + slot(slots, AttribField::PhysicsAttackPowerBase)
        + (extra_strength * constants.lidao_to_attack).floor();
    // 铁骨气劲：体质→攻击（0.198 或 0.594）
    let vta = slot(slots, AttribField::VitalityToAttackCof);
    if vta != 0.0 { panel_attack += (final_vitality * vta).floor(); }
    let pct = slot(slots, AttribField::PhysicsAttackPowerPercent);
    if pct != 0.0 {
        panel_attack += (panel_attack * pct / 1024.0).floor();
    }

    // 内功攻击（同理）
    let magical_attack = attr.base_magical_attack
        + (extra_yuanqi * constants.yuanqi_to_attack).floor();

    // 会心等级：面板 + buff 加算 + 主属性增量
    let crit_level = attr.crit_level
        + slot(slots, AttribField::PhysicsCriticalStrike)
        + (extra_shenfa * constants.shenfa_to_crit).floor();
    let crit_rate  = (crit_level / LP_CRIT).clamp(0.0, 1.0);

    // 会心效果等级
    let crit_eff_level = attr.crit_effect_level
        + slot(slots, AttribField::PhysicsCriticalDamagePowerBase);
    let crit_effect = BASE_CRIT_POWER + crit_eff_level / LP_CRIT_EFF;

    // 破防等级：面板 + buff 加算 + 力道增量
    let mut overcome_level = attr.overcome_level
        + slot(slots, AttribField::PhysicsOvercomeBase)
        + (extra_strength * constants.lidao_to_overcome).floor();
    // 铁骨气劲：体质→破防（0.152 或 0.456）
    let vto = slot(slots, AttribField::VitalityToOvercomeCof);
    if vto != 0.0 { overcome_level += (final_vitality * vto).floor(); }
    let opct = slot(slots, AttribField::PhysicsOvercomePercent);
    if opct != 0.0 {
        overcome_level += (overcome_level * opct / 1024.0).floor();
    }
    let overcome = overcome_level / LP_OVERCOME;

    // 无双等级
    let mut strain_level = attr.strain_level + slot(slots, AttribField::StrainBase);
    let strain_pct = slot(slots, AttribField::StrainBasePercentAdd);
    if strain_pct != 0.0 {
        strain_level += (strain_level * strain_pct / 1024.0).floor();
    }
    let mut strain = strain_level / LP_STRAIN;
    // StrainPercent: 直接加到最终无双率（如寒啸千军 +51/1024 ≈ +5%）
    let strain_direct = slot(slots, AttribField::StrainPercent);
    if strain_direct != 0.0 {
        strain += strain_direct / 1024.0;
    }

    // 加速：从加速等级换算的加速率封顶 25%；UnlimitedAdditionalHastePercent 绕过封顶
    let haste_level = attr.haste_level + slot(slots, AttribField::HasteBase);
    let base_haste  = (haste_level / LP_HASTE).clamp(0.0, 0.25);
    let extra_haste = slot(slots, AttribField::UnlimitedAdditionalHastePercent) / 1024.0;
    let haste_rate  = (base_haste + extra_haste).clamp(0.0, 1.0);

    // 破招值
    let surplus_value = attr.surplus_value + slot(slots, AttribField::SurplusValueBase);

    // 拆招值（防御向；铁骨衣寒甲奇穴用）：面板 + buff 加算 + 体质增量×vitality_to_parry_value
    let parry_value = attr.parry_value
        + slot(slots, AttribField::ParryValueBase)
        + (extra_vitality * constants.vitality_to_parry_value).floor();

    // 招架等级：面板 + buff 加算 + 体质增量×vitality_to_parry_level
    let parry_level = attr.parry_level
        + slot(slots, AttribField::ParryBase)
        + (extra_vitality * constants.vitality_to_parry_level).floor();

    // 招架率：level / (level + LP_PARRY) + 心法基础 + 直接加成 (ParryValuePercent/10000)
    let parry_direct = slot(slots, AttribField::ParryValuePercent) / 10000.0;
    let parry_rate = (parry_level / (parry_level + LP_PARRY)) + constants.parry_base_rate + parry_direct;
    // 不 clamp：坚铁满层可超过 100%

    CombatStats {
        shen_fa:       attr.shen_fa,
        panel_attack,
        magical_attack,
        base_attack:   attr.base_attack,
        shenfa_attack: attr.shen_fa * constants.shenfa_to_attack,
        crit_rate, crit_effect, overcome, strain, haste_rate, surplus_value,
        parry_value, parry_level, parry_rate,
        vitality: final_vitality,
        base_vitality: attr.vitality + extra_vitality_raw,
    }
}

/// 9 步取整伤害链
/// 返回 SkillResult（含 normal / crit / expected 三个数值）
pub fn calc_damage(
    spec: &SkillSpec,
    attr: &Attributes,
    target: &TargetConfig,
    rt: &CombatStats,
    recipes: &[&RecipeEntry],
    buff_slots: &AttribSlots,
    target_slots: &AttribSlots,
    non_player_bonus: f64,
) -> SkillResult {
    let _t0 = std::time::Instant::now();
    let _guard = scopeguard_perf(|ns| perf_add(|p| { p.calc_damage_n += 1; p.calc_damage_ns += ns; }), _t0);
    let is_surplus = matches!(spec.damage_kind, DamageKind::SurplusOnly);

    // ── 字段聚合 ──
    let damage_pct  = recipes.iter().map(|r| r.damage_pct).sum::<f64>();
    let crit_pct    = recipes.iter().map(|r| r.critical_pct).sum::<f64>();
    let crit_eff    = recipes.iter().map(|r| r.crit_eff_pct).sum::<f64>();
    let surplus_pct = recipes.iter().map(|r| r.surplus_pct).sum::<f64>()
                    + slot(buff_slots, AttribField::SurplusPercent) / 1024.0;
    let shield_ig   = recipes.iter().map(|r| r.shield_ignore).sum::<f64>()
                      + slot(buff_slots, AttribField::AllShieldIgnorePercent);
    let all_dmg_add = slot(buff_slots, AttribField::AllDamageAddPercent) / 1024.0;
    let pve_extra   = slot(buff_slots, AttribField::PveAddition) / 1024.0;
    let recipe_pve  = recipes.iter().map(|r| r.pve_addition).sum::<f64>();
    let pve_addition = non_player_bonus / 1024.0 + pve_extra + recipe_pve;

    // ── Step 1: 基础伤害 ──
    // 系数伤害 = 普通：attack_coeff × 最终攻击；破招：surplus_coeff × 破招值
    // 不含 base_damage / 武器伤害 / 任何增减伤
    let coefficient_damage: f64;
    let mut damage: i64 = if is_surplus {
        // 破招段：surplus_coeff 已含 7.421 常数
        let surplus_v = rt.surplus_value
            + (rt.surplus_value * surplus_pct).floor();
        let v = (spec.surplus_coeff * surplus_v).floor();
        coefficient_damage = v;
        v as i64
    } else {
        let attack_pow = match spec.damage_kind {
            DamageKind::Magical => rt.magical_attack,
            _ => rt.panel_attack,
        };
        // 武器伤害含团队增益加算（瑰栗粽/梅花糕/春节·升景）
        let weapon_dmg = attr.weapon_damage
            + slot(buff_slots, AttribField::WeaponDamageBase);
        coefficient_damage = (spec.attack_coeff * attack_pow).floor();
        (spec.base_damage
            + spec.attack_coeff * attack_pow
            + spec.weapon_coeff * weapon_dmg).floor() as i64
    };

    // ── Step 2: 秘籍增伤（仅普通伤害；真实伤害跳过）──
    if !is_surplus && !spec.true_damage && damage_pct != 0.0 {
        damage = ((damage as f64) * (1.0 + damage_pct)).floor() as i64;
    }

    // ── Step 3: 破防 × 无双（真实伤害跳过）──
    if !spec.true_damage {
        if rt.overcome != 0.0 {
            damage = ((damage as f64) * (1.0 + rt.overcome)).floor() as i64;
        }
        if rt.strain != 0.0 {
            damage = ((damage as f64) * (1.0 + rt.strain)).floor() as i64;
        }
    }

    // ── Step 4: 防御（真实伤害跳过；其他走防御计算）──
    let def_rate = if spec.true_damage {
        0.0
    } else {
        let target_shield_base = slot(target_slots, AttribField::TargetPhysicsShieldBase);
        let target_shield_pct  = slot(target_slots, AttribField::TargetPhysicsShieldPercent);
        let r = calc_defense_rate_with_ignore(
            target, spec.defense_ignore, shield_ig, target_shield_base, target_shield_pct,
        );
        if r > 0.0 {
            damage = ((damage as f64) * (1.0 - r)).floor() as i64;
        }
        r
    };

    // ── Step 5: 等级压制（所有伤害都走）──
    let lv_factor = calc_level_suppression(PLAYER_LEVEL, target.level);
    let lv_delta = lv_factor - 1.0;
    if lv_delta != 0.0 {
        damage = damage + ((damage as f64) * lv_delta).floor() as i64;
    }

    // ── Step 6: PVE 增伤（真实伤害跳过；jx3dps 一致）──
    if !spec.true_damage && pve_addition != 0.0 {
        damage = damage + ((damage as f64) * pve_addition).floor() as i64;
    }

    // ── Step 7: 易伤（含目标 debuff 团辅：戒火/龙吟·悟/战锋·悟/劲风/破甲等）──
    let damage_cof = target.damage_cof
        + slot(target_slots, AttribField::TargetDamageBonusPercent) / 1024.0;
    if damage_cof != 0.0 {
        damage = damage + ((damage as f64) * damage_cof).floor() as i64;
    }

    // ── Step 8: 全局增伤 ──
    if all_dmg_add != 0.0 {
        damage = ((damage as f64) * (1.0 + all_dmg_add)).floor() as i64;
    }

    // ── Step 9: 会心会效（最后；影响全部）──
    // true_damage=true（jx3 真实伤害，如腕鞋大附魔）：跳过会心，强制 normal=expected
    let crit_rate  = if spec.true_damage { 0.0 } else {
        (rt.crit_rate + crit_pct + slot(buff_slots, AttribField::PhysicsCriticalStrikePercent) / 1024.0).clamp(0.0, 1.0)
    };
    let crit_power = if spec.true_damage { 1.0 } else {
        rt.crit_effect + crit_eff + slot(buff_slots, AttribField::PhysicsCriticalDamagePowerPercent) / 1024.0
    };
    let normal     = damage as f64;
    let crit       = (normal * crit_power).floor();
    let expected   = if spec.true_damage { normal } else {
        (crit * crit_rate + normal * (1.0 - crit_rate)).floor()
    };

    SkillResult {
        name: spec.name.clone(),
        coefficient_damage,
        normal_damage: normal,
        crit_damage:   crit,
        expected_damage: expected,
        defense_rate:  def_rate,
        crit_rate:     crit_rate * 100.0,
        overcome:      rt.overcome * 100.0,
        strain:        rt.strain  * 100.0,
    }
}

/// 计算事件伤害（含 channel_ticks 倍数）
/// 返回 (SkillResult, 总伤害=expected×ticks)
pub fn calc_event_damage(
    spec: &SkillSpec,
    attr: &Attributes,
    target: &TargetConfig,
    player: &Player,
    runtime_recipes: &[u32],
    recipes_table: &[RecipeEntry],
    channel_ticks: u32,
) -> (SkillResult, f64, CombatStats) {
    let (buff_slots, target_slots, rt) = calc_stats_cached(player, attr);
    // 奇穴/加速 等动态覆盖 attack_coeff
    let spec_owned;
    let effective_spec = if let Some(coeff) = scripts::override_attack_coeff(player, spec) {
        let mut s = spec.clone();
        s.attack_coeff = coeff;
        spec_owned = s;
        &spec_owned
    } else {
        spec
    };
    let base_name = effective_spec.name.split('·').next().unwrap_or(&effective_spec.name);
    let recipes = collect_recipes_indexed(player, effective_spec.skill_id, base_name, runtime_recipes, recipes_table);
    let r = calc_damage(effective_spec, attr, target, &rt, &recipes, &buff_slots, &target_slots, player.constants.non_player_bonus);
    let total = r.expected_damage * channel_ticks.max(1) as f64;
    (r, total, rt)
}

/// 带缓存的属性计算：buff_generation 没变就复用上次结果
fn calc_stats_cached(player: &Player, attr: &Attributes) -> (AttribSlots, AttribSlots, CombatStats) {
    {
        let cache = player.buff_cache.borrow();
        if let Some((gen, ref bs, ref ts, ref rt)) = *cache {
            if gen == player.buff_generation {
                perf_add(|p| { p.cache_hit += 1; });
                return (bs.clone(), ts.clone(), rt.clone());
            }
        }
    }
    perf_add(|p| { p.cache_miss += 1; });
    let buff_slots = aggregate_buff_fields(player);
    let rt = build_runtime_stats(attr, &buff_slots, &player.constants);
    let target_slots = aggregate_target_buff_fields(player);
    *player.buff_cache.borrow_mut() = Some((player.buff_generation, buff_slots.clone(), target_slots.clone(), rt.clone()));
    (buff_slots, target_slots, rt)
}

/// 计算技能释放时的瞬时附加秘籍
/// 绝刀：分发到 jue_dao 脚本（按当前怒气段）
/// 血誓：触发时由 xue_shi 脚本通过 emit_with_recipes 直接绑事件（不走这里）
pub fn compute_runtime_recipes(skill: &SkillSpec, player: &Player) -> Vec<u32> {
    match skill.skill_id {
        13055 => scripts::jue_dao_runtime_recipes(player),
        _ => Vec::new(),
    }
}

/// 抓取 DoT 快照（斩刀添加/刷新流血时调用）
/// 快照字段：攻击、会心、会效、无双、全局增伤（含对应技能的秘籍加成）
pub fn capture_dot_snapshot(
    attr: &Attributes,
    player: &Player,
    parent_skill_id: u32,
    parent_base_name: &str,
    recipes_table: &[RecipeEntry],
) -> DotSnapshot {
    let buff_slots = aggregate_buff_fields(player);
    let rt = build_runtime_stats(attr, &buff_slots, &player.constants);
    let recipes = collect_recipes_indexed(player, parent_skill_id, parent_base_name, &[], recipes_table);
    let crit_pct = recipes.iter().map(|r| r.critical_pct).sum::<f64>();
    let crit_eff = recipes.iter().map(|r| r.crit_eff_pct).sum::<f64>();
    let all_dmg_add = slot(&buff_slots, AttribField::AllDamageAddPercent) / 1024.0;
    DotSnapshot {
        panel_attack: rt.panel_attack,
        crit_rate:  (rt.crit_rate + crit_pct + slot(&buff_slots, AttribField::PhysicsCriticalStrikePercent) / 1024.0).clamp(0.0, 1.0),
        crit_power: rt.crit_effect + crit_eff + slot(&buff_slots, AttribField::PhysicsCriticalDamagePowerPercent) / 1024.0,
        strain:     rt.strain,
        all_dmg_add,
    }
}

/// DoT 快照路径的伤害计算（仅用于流血每跳）
/// 快照字段：攻击 / 无双 / 全局增伤 / 会心 / 会效
/// 实时字段：破防 / 防御（含无视防御） / 等级压制 / PVE / 易伤
fn calc_damage_with_snapshot(
    spec: &SkillSpec,
    attr: &Attributes,
    target: &TargetConfig,
    snap: &DotSnapshot,
    player: &Player,
    recipes_table: &[RecipeEntry],
) -> SkillResult {
    let buff_slots_live = aggregate_buff_fields(player);
    let target_slots    = aggregate_target_buff_fields(player);
    let rt_live         = build_runtime_stats(attr, &buff_slots_live, &player.constants);
    let base_name = spec.name.split('·').next().unwrap_or(&spec.name);
    let recipes   = collect_recipes_indexed(player, spec.skill_id, base_name, &[], recipes_table);

    let shield_ig = recipes.iter().map(|r| r.shield_ignore).sum::<f64>()
                  + slot(&buff_slots_live, AttribField::AllShieldIgnorePercent);
    let recipe_pve = recipes.iter().map(|r| r.pve_addition).sum::<f64>();
    let pve_extra  = slot(&buff_slots_live, AttribField::PveAddition) / 1024.0;
    let pve_addition = player.constants.non_player_bonus / 1024.0 + pve_extra + recipe_pve;
    let target_shield_base = slot(&target_slots, AttribField::TargetPhysicsShieldBase);
    let target_shield_pct  = slot(&target_slots, AttribField::TargetPhysicsShieldPercent);

    // Step 1: 攻击用快照；武器伤害实时含团辅加算
    let weapon_dmg = attr.weapon_damage
        + slot(&buff_slots_live, AttribField::WeaponDamageBase);
    let mut damage = (spec.base_damage
        + spec.attack_coeff * snap.panel_attack
        + spec.weapon_coeff * weapon_dmg).floor() as i64;

    // Step 2: 流血无秘籍增伤（spec.skill_id=8249 不在任何秘籍 skill_filter）
    // Step 3: 破防实时、无双快照
    if rt_live.overcome != 0.0 {
        damage = ((damage as f64) * (1.0 + rt_live.overcome)).floor() as i64;
    }
    if snap.strain != 0.0 {
        damage = ((damage as f64) * (1.0 + snap.strain)).floor() as i64;
    }
    // Step 4: 防御实时
    let def_rate = calc_defense_rate_with_ignore(
        target, spec.defense_ignore, shield_ig, target_shield_base, target_shield_pct);
    if def_rate > 0.0 {
        damage = ((damage as f64) * (1.0 - def_rate)).floor() as i64;
    }
    // Step 5: 等级压制实时
    let lv_delta = calc_level_suppression(PLAYER_LEVEL, target.level) - 1.0;
    if lv_delta != 0.0 {
        damage = damage + ((damage as f64) * lv_delta).floor() as i64;
    }
    // Step 6: PVE 实时
    if pve_addition != 0.0 {
        damage = damage + ((damage as f64) * pve_addition).floor() as i64;
    }
    // Step 7: 易伤实时（含目标 debuff 团辅）
    let damage_cof = target.damage_cof
        + slot(&target_slots, AttribField::TargetDamageBonusPercent) / 1024.0;
    if damage_cof != 0.0 {
        damage = damage + ((damage as f64) * damage_cof).floor() as i64;
    }
    // Step 8: 全局增伤快照
    if snap.all_dmg_add != 0.0 {
        damage = ((damage as f64) * (1.0 + snap.all_dmg_add)).floor() as i64;
    }
    // Step 9: 会心会效快照
    let normal = damage as f64;
    let crit   = (normal * snap.crit_power).floor();
    let expected = (crit * snap.crit_rate + normal * (1.0 - snap.crit_rate)).floor();

    SkillResult {
        name: spec.name.clone(),
        coefficient_damage: (spec.attack_coeff * snap.panel_attack).floor(),
        normal_damage: normal,
        crit_damage:   crit,
        expected_damage: expected,
        defense_rate:  def_rate,
        crit_rate:     snap.crit_rate * 100.0,
        overcome:      rt_live.overcome * 100.0,
        strain:        snap.strain * 100.0,
    }
}

/// 给脚本 emit 的事件补伤害（含其 channel_ticks 跳数）
/// 优先用事件自带的 runtime_recipes（emit_with_recipes 设置）；否则用主体技能传入的
pub fn fill_event_damage(
    ev: &mut CastEvent,
    skill_by_id: &HashMap<u32, &SkillSpec>,
    dmg_ctx: Option<&(Attributes, TargetConfig)>,
    recipes_table: &[RecipeEntry],
    player: &Player,
    parent_runtime_recipes: &[u32],
) {
    let _t0 = std::time::Instant::now();
    let _guard = scopeguard_perf(|ns| perf_add(|p| { p.fill_event_n += 1; p.fill_event_ns += ns; }), _t0);
    let (a, t) = match dmg_ctx { Some(c) => c, None => return };
    let spec = match skill_by_id.get(&ev.skill_id) { Some(s) => s, None => return };
    let ticks = ev.channel_ticks.unwrap_or(1).max(1);

    // DoT 快照路径（流血每跳 8249）：按斩刀释放时记录的快照算伤害
    if ev.skill_id == 8249 {
        if let Some(snap) = player.target_buffs.iter()
            .find(|b| b.buff_id == BUFF_LIU_XUE)
            .and_then(|b| b.snapshot.as_ref())
        {
            let r = calc_damage_with_snapshot(spec, a, t, snap, player, recipes_table);
            ev.damage = Some(r.expected_damage);
            ev.damage_normal = Some(r.normal_damage);
            ev.damage_crit   = Some(r.crit_damage);
            ev.damage_total = Some(r.expected_damage * ticks as f64);
            // rt 只为 hover 详情准备 — lite 模式（含 ΔDPS 预览）跳过，省一次 current_stats
            if !player.lite_mode {
                ev.runtime_stats = Some(player.current_stats());
                // 秘籍来源（供前端 log hover 显示）— 与 calc_damage_with_snapshot 内部 collect 同语义
                let base_name = spec.name.split('·').next().unwrap_or(&spec.name);
                let applied = collect_recipes_indexed(player, spec.skill_id, base_name, &[], recipes_table);
                ev.applied_recipes = applied.iter().map(|r| r.id).collect();
            }
            return;
        }
    }

    let recipes: &[u32] = if !ev.runtime_recipes.is_empty() {
        &ev.runtime_recipes
    } else {
        parent_runtime_recipes
    };
    // 脚本动态覆盖 attack_coeff（绝国按层数变化）—— 仅在 override 时克隆
    let (r, dt, rt) = if let Some(coeff) = ev.override_attack_coeff {
        let mut s2 = (*spec).clone();
        s2.attack_coeff = coeff;
        calc_event_damage(&s2, a, t, player, recipes, recipes_table, ticks)
    } else {
        calc_event_damage(spec, a, t, player, recipes, recipes_table, ticks)
    };
    ev.damage = Some(r.expected_damage);
    ev.damage_normal = Some(r.normal_damage);
    ev.damage_crit   = Some(r.crit_damage);
    ev.damage_total = Some(dt);
    if !player.lite_mode {
        ev.runtime_stats = Some(rt);
        // 抓激活的秘籍 ID 列表（供前端 log hover 显示增伤来源）
        let base_name = spec.name.split('·').next().unwrap_or(&spec.name);
        let applied = collect_recipes_indexed(player, spec.skill_id, base_name, recipes, recipes_table);
        ev.applied_recipes = applied.iter().map(|r| r.id).collect();
    }
}

/// 给一组 tick/swing/advance 事件批量补伤害
pub fn fill_tick_events(
    events: &mut [CastEvent],
    skill_by_id: &HashMap<u32, &SkillSpec>,
    dmg_ctx: Option<&(Attributes, TargetConfig)>,
    recipes_table: &[RecipeEntry],
    player: &Player,
) {
    let _t0 = std::time::Instant::now();
    let _guard = scopeguard_perf(|ns| perf_add(|p| { p.fill_tick_n += 1; p.fill_tick_ns += ns; }), _t0);
    for ev in events {
        fill_event_damage(ev, skill_by_id, dmg_ctx, recipes_table, player, &[]);
    }
}

/// 防御率计算
///
/// 两类无视（同类加算，两类相乘）：
/// - **B 类**：技能特定无视（spec.defense_ignore，小数）+ 虚弱等目标 debuff 减防（target_shield_pct 负值）
/// - **A 类**：全局/秘籍无视（atAllShieldIgnorePercent，1024 制）
/// 最终无视 = 1 − (1 − B_sum) × (1 − A_sum)
/// 最终防御等级 = 基础防御 × (1 − 最终无视)
/// 防御率 = 最终防御 / (最终防御 + param)
fn calc_defense_rate_with_ignore(
    target: &TargetConfig,
    skill_defense_ignore: f64,       // B 类：小数 0~1
    buff_shield_ignore_1024: f64,    // A 类：1024 制
    target_shield_base: f64,         // 目标 debuff 防御等级数值加成（正值=增防）
    target_shield_pct_1024: f64,     // 目标 debuff 防御百分比（负值=减防，等价 B 类无视）
) -> f64 {
    let base = target_base_defense(target.level);
    // 基础防御（目标自身加成 + 数值 debuff）
    let mut shield = base * (1.0 + target.defense_bonus / 100.0) + target_shield_base;
    // 正向 debuff（增防，罕见）—— 直接乘到 shield 上
    if target_shield_pct_1024 > 0.0 {
        shield += (shield * target_shield_pct_1024 / 1024.0).floor();
    }
    // B 类无视：技能自身 + 虚弱（负向 pct 取绝对值）
    let weakness_ignore = if target_shield_pct_1024 < 0.0 {
        -target_shield_pct_1024 / 1024.0
    } else { 0.0 };
    let b_sum = skill_defense_ignore + weakness_ignore;
    // A 类无视：全局/秘籍
    let a_sum = buff_shield_ignore_1024 / 1024.0;
    // 最终无视 = 1 − (1 − B) × (1 − A)，然后作用于防御等级
    let total_ignore = 1.0 - (1.0 - b_sum) * (1.0 - a_sum);
    let final_shield = (shield * (1.0 - total_ignore)).max(0.0);
    if final_shield <= 0.0 { return 0.0; }
    let param = defense_level_param(target.level);
    (final_shield / (final_shield + param)).min(1.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// 技能配置加载
// ─────────────────────────────────────────────────────────────────────────────

fn load_skills(dir: &Path) -> Vec<SkillSpec> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => { eprintln!("[skills] 无法读取目录 {:?}: {e}", dir); return Vec::new(); }
    };

    let mut specs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
        // 跳过 _template.toml 等下划线开头的文件
        if path.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with('_')) { continue; }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => { eprintln!("[skills] 读取 {:?} 失败: {e}", path); continue; }
        };
        let config: SkillFileConfig = match toml::from_str(&content) {
            Ok(c) => c,
            Err(e) => { eprintln!("[skills] 解析 {:?} 失败: {e}", path); continue; }
        };

        if config.skip {
            println!("[skills] 跳过(非伤害): {}", config.name);
            continue;
        }

        for rank in &config.ranks {
            let name = rank.name.clone().unwrap_or_else(|| {
                format!("{}·{}级", config.name, rank.rank)
            });
            let effective_cooldowns = rank.cooldowns.clone()
                .unwrap_or_else(|| config.cooldowns.clone());
            specs.push(SkillSpec {
                skill_id:         config.id,
                name,
                description:      config.description.clone(),
                icon:             rank.icon.clone().unwrap_or_else(|| config.icon.clone()),
                damage_kind:      config.damage_kind,
                base_damage:      rank.base_damage,
                attack_coeff:     rank.attack_coeff,
                weapon_coeff:     rank.weapon_coeff,
                defense_ignore:   rank.defense_ignore,
                surplus_coeff:    rank.surplus_coeff,
                cooldowns:        effective_cooldowns,
                channel_frame:    config.channel_frame,
                channel_interval: config.channel_interval,
                first_tick_frame: config.first_tick_frame,
                stance:           config.stance,
                rage_cost:        rank.rage_cost.unwrap_or(config.rage_cost),
                rage_gain:        rank.rage_gain.unwrap_or(config.rage_gain),
                stance_change:    config.stance_change,
                requires_talent:  config.requires_talent,
                requires_combo:   rank.requires_combo.clone(),
                grants_combo:     rank.grants_combo.clone(),
                combo_duration:   rank.combo_duration,
                max_charges:      config.max_charges,
                charge_cd:        config.charge_cd,
                passive:          config.passive,
                true_damage:      config.true_damage,
                combo_follow:     config.combo_follow.clone(),
            });
        }
        println!("[skills] 已加载: {} ({} 个品级)", config.name, config.ranks.len());
    }

    println!("[skills] 共加载 {} 条技能规格", specs.len());
    specs
}

/// 根据技能的 combo_follow 字段，检查玩家连招 buff，重定向到子技能
/// 按 combo_follow 列表顺序匹配第一个满足条件的条目
pub(crate) fn resolve_combo_follow<'a>(
    skill: &SkillSpec,
    player: &Player,
    skill_by_id: &'a HashMap<u32, &'a SkillSpec>,
) -> Option<&'a SkillSpec> {
    for entry in &skill.combo_follow {
        let id = combo_buff_id(&entry.requires_combo);
        if player.has_buff(id) {
            return skill_by_id.get(&entry.skill_id).copied();
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// 玩家对象 & 循环模拟
// ─────────────────────────────────────────────────────────────────────────────

/// 模拟请求
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SimulateRequest {
    pub haste_level: u32,
    pub sequence: Vec<String>,
    #[serde(default)]
    pub talents: Vec<u32>,
    /// 引导技能跳数覆盖：序列索引 → 自定义跳数（0 = 使用默认最大跳数）
    #[serde(default)]
    pub channel_ticks: HashMap<String, u32>,
    /// 非主GCD技能释放偏移：序列索引 → 偏移秒数（相对最早可释放时间）
    #[serde(default)]
    pub timing_offsets: HashMap<String, f64>,
    /// 网络延迟（毫秒）
    #[serde(default)]
    pub network_delay: u32,
    /// 已选秘籍 ID 列表
    #[serde(default)]
    pub recipes: Vec<u32>,
    /// 移除气劲：序列索引 → 要移除的 buff_id
    #[serde(default)]
    pub qijin_buffs: HashMap<String, u32>,
    /// 宏文本（存在时使用宏模式，覆盖 sequence）
    #[serde(default)]
    pub macro_text: Option<String>,
    /// 宏模拟最大时长（秒），默认 3600
    #[serde(default)]
    pub macro_duration: Option<f64>,
    /// 角色属性（存在时计算伤害）
    #[serde(default)]
    pub attributes: Option<Attributes>,
    /// 目标配置（伤害计算需要）
    #[serde(default)]
    pub target: Option<TargetConfig>,
    /// 起始怒气（管理员调试用；None=默认0）
    #[serde(default)]
    pub initial_rage: Option<i32>,
    /// 停手场景：在随机位置暂停 N 秒（宏模式下也生效）
    /// `[(start_sec, duration_sec), ...]` — 简化：只支持一段，start 由前端传入
    #[serde(default)]
    pub pauses: Vec<(f64, f64)>,
    /// boss 平均攻击间隔（秒），用于坚铁概率分布（始终生效）和寒甲期望传播
    #[serde(default)]
    pub boss_attack_interval: Option<f64>,
    /// 寒甲期望传播开关（true 时寒甲 A/B 走期望；false/None 时走原 100% 招架假设）
    #[serde(default)]
    pub hanjia_expectation: Option<bool>,
    /// 铁骨气劲模式：0=关，1=铁骨（副T），2=铁骨·宿敌（主T）
    #[serde(default = "default_tiegu_mode")]
    pub tiegu_mode: u8,
    /// 实验性武学开关（雾海寻龙阵云等）
    #[serde(default)]
    pub experimental: bool,
    /// Lite 模式：跳过 timeline 详情/state_before/after/runtime_stats，只回 DPS/总伤/战斗时长。
    /// 用于配装评估、属性梯度、宏优化等批量场景。默认 false（Full）。
    #[serde(default)]
    pub lite: bool,
    /// Lite 模式下保留 timeline 数组（事件 .name/.skill_id/.cast_time/.triggered/.is_main 等基本字段
    /// 仍在；state_before/after/runtime_stats 等重字段已 None）。
    /// 用于宏蒸馏工作流：验证/剪枝/swap 需要 cast counts 但不需要 hover 详情。默认 false（保持原 lite 行为）。
    #[serde(default)]
    pub lite_keep_timeline: bool,
    /// 装备清单：position → equip_id（前端从配装器拷贝；后端 Player 持有副本，脚本可查）
    /// 用法：脚本层 `player.equip_id_at("PRIMARY_WEAPON")` 比对 ID 决定装备特效（如盾击神兵触发概率）。
    /// 不影响伤害计算 — 属性面板已通过 `attributes` 字段折好，此处仅供脚本逻辑分支。
    #[serde(default)]
    pub equipment: HashMap<String, u32>,
    /// 团队增益启用列表（循环模拟走 apply_team_buffs_for_simulate：按时间排程拟真）
    #[serde(default)]
    pub team_buffs: Vec<TeamBuffSelection>,
    /// 选中的阵法（None = 不开阵）
    #[serde(default)]
    pub formation: Option<FormationSelection>,
    /// 预释放：t=0 之前预读的技能列表（{skill, time_before}，time_before > 0）
    /// 主循环前在虚拟负时间点逐一 cast（按 time_before 降序，最早的先 cast），
    /// 让 buff/CD 在 t=0 时刻处于"已生效"状态；预释放阶段产生的 timeline 事件不进最终结果。
    #[serde(default)]
    pub pre_releases: Vec<PreReleaseSpec>,
}

/// 预释放规格：一次"战前预读"
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PreReleaseSpec {
    /// 技能名（按基础名匹配 skill_map）
    pub skill: String,
    /// 提前秒数（必须 > 0；t=0 时刻该 cast 已经发生 time_before 秒）
    pub time_before: f64,
}

fn default_tiegu_mode() -> u8 { 2 } // 默认主T

/// 单次技能释放事件
#[derive(Debug, Serialize, Clone)]
pub struct CastEvent {
    pub name: String,
    pub skill_id: u32,
    pub cast_time: f64,
    /// true = 被动触发（脚本/Buff），false = 主动释放
    pub triggered: bool,
    /// 该技能触发的最大 GCD 秒数
    pub gcd: f64,
    /// 是否占主 GCD 位
    pub is_main: bool,
    /// 等待技能 CD 的时间（秒，0 = 无等待）
    pub cd_wait: f64,
    /// 引导技能：当前跳数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_ticks: Option<u32>,
    /// 引导技能：最大跳数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_channel_ticks: Option<u32>,
    /// 引导技能：实际引导时长（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_duration: Option<f64>,
    /// 非主GCD技能：当前偏移秒数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_offset: Option<f64>,
    /// 非主GCD技能：最大允许偏移秒数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_timing_offset: Option<f64>,
    /// 移除气劲：释放时刻可选的 buff 列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_buffs: Option<Vec<BuffSnapshot>>,
    /// 宏释放
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_macro: bool,
    /// 释放后的怒气值（最终值，含脚本修改）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rage_after: Option<i32>,
    /// 怒气净变化量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rage_delta: Option<i32>,
    /// 技能实际扣除的怒气（apply_cast_effects 里的消耗，不含脚本返还）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rage_cost: Option<u32>,
    /// 释放前的玩家状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_before: Option<EventState>,
    /// 释放后的玩家状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_after: Option<EventState>,
    /// 单次期望伤害（含会心期望，不含跳数）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<f64>,
    /// 单次普通伤害（不会心）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_normal: Option<f64>,
    /// 单次会心伤害
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_crit: Option<f64>,
    /// 总伤害（单次期望 × 跳数）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_total: Option<f64>,
    /// 造成伤害时刻的最终面板属性（含 buff 加成）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_stats: Option<CombatStats>,
    /// 仅本次事件附加的运行时秘籍（如绝刀怒气段、血誓怒气段）—— 不序列化
    #[serde(skip)]
    pub runtime_recipes: Vec<u32>,
    /// 脚本动态覆盖 attack_coeff（绝国按长驱万里层数变化）—— 不序列化
    #[serde(skip)]
    pub override_attack_coeff: Option<f64>,
    /// 本次事件实际应用的秘籍 ID 列表（含奇穴常驻、buff 激活、装备激活、套装 4 件套等）
    /// 战斗记录 log hover 时显示"已吃 99270 T套+10%"等增伤来源
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub applied_recipes: Vec<u32>,
}

/// 事件时刻的玩家状态快照
#[derive(Debug, Serialize, Clone)]
pub struct EventState {
    pub rage: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_value: Option<i32>,
    pub stance: Stance,
    pub buffs: Vec<EventBuff>,
    pub target_buffs: Vec<EventBuff>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skill_cds: Vec<EventSkillCd>,
}

/// 状态快照中的技能 CD 条目
#[derive(Debug, Serialize, Clone)]
pub struct EventSkillCd {
    pub name: String,
    pub remaining: f64,
}

/// 状态快照中的 buff 条目
#[derive(Debug, Serialize, Clone)]
pub struct EventBuff {
    pub buff_id: u32,
    pub name: String,
    pub remaining: f64,
    pub stacks: u32,
    /// 图标 URL（来自 BuffDef.icon，空串前端跳过）—— 让序列项 hover 等场景能按 buff_id 取图标
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// Buff 快照（序列化给前端）
#[derive(Debug, Serialize, Clone)]
pub struct BuffSnapshot {
    pub buff_id: u32,
    pub name: String,
    pub description: String,
    pub stacks: u32,
    pub max_stacks: u32,
    pub remaining_sec: f64,
    pub duration_sec: f64,
    pub is_debuff: bool,
    /// true = 目标身上的效果，false = 自身
    pub is_target: bool,
    /// Buff 等级（盾挡等等级型 buff 用；0 = 无等级概念）
    #[serde(default)]
    pub level: u32,
    /// 期望层数（连续浮点；坚铁/寒甲合成 buff 才有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_stacks: Option<f64>,
    /// 层数概率分布（仅期望传播 buff 用于 UI 热力图）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_distribution: Option<Vec<f64>>,
    /// 图标 URL（来自 BuffDef.icon，空串前端跳过）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// 模拟响应
#[derive(Debug, Serialize)]
pub struct SimulateResponse {
    pub fight_time: f64,
    pub skill_count: usize,
    pub stance: Stance,
    pub rage: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_value: Option<i32>,
    /// 格挡值上限（铁骨衣基础 100；坚韧奇穴 13363 → 200）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_block_value: Option<i32>,
    pub timeline: Vec<CastEvent>,
    pub buffs: Vec<BuffSnapshot>,
    /// 当前状态下可施展的技能名列表
    pub available_skills: Vec<String>,
    /// 序列中被跳过的技能：序列索引 → 原因
    pub skipped: Vec<(usize, String)>,
    /// 各技能剩余 CD（技能基础名 → 剩余秒数）
    pub skill_cds: HashMap<String, f64>,
    /// 各技能当前充能层数（技能基础名 → 层数）
    pub skill_charges: HashMap<String, u32>,
    /// buff 时间轴事件（仅 show_on_timeline=true，用于轨道渲染）
    pub buff_timeline: Vec<BuffTimelineTrack>,
    /// 战斗记录 log 用：含所有 buff 事件（包括 show_on_timeline=false 的，如神兵·无双气劲）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub buff_log: Vec<BuffTimelineTrack>,
    /// 秘籍元信息（id → 简短描述），供前端把 cast.applied_recipes 数字翻成可读文字
    /// 仅含 timeline 中实际出现过的 ids（避免传 100+ 条全表）
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub recipes_meta: HashMap<u32, String>,
    /// 当前版本下所有 effects 含 AllDamageAddPercent 字段的 buff_id 列表
    /// 前端 log 战斗记录 hover 用：过滤 state_before.buffs 中属于"增伤型"的 buff 显示在"伤害增益"栏
    /// （加属性的 buff 如攻击力/会心/破防/无双不计入此列表）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub damage_add_buff_ids: Vec<u32>,
    /// 开战 t=0 时刻的 buff 快照（永久 buff/装备/团辅永久型/奇穴 hardcode 全部已挂）
    /// 实时面板 hover"增益来源"用
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub initial_buffs: Vec<BuffSnapshot>,
    /// buff_id → 受其影响的前端 attr_key 列表（如 atk/crit/oc/strain/...）
    /// 启动时按当前版本扫所有 BuffDef.effects 字段聚合
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub buff_attr_keys: HashMap<u32, Vec<String>>,
    /// buff_id → { attr_key → 单层贡献格式化字符串 }（如 "+10%" / "+102级" / "+10% / +102级"）
    /// 前端 hover 时显示"嗜血 ×3  +10%"这种带数值的小字注解
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub buff_attr_desc: HashMap<u32, HashMap<String, String>>,
    /// 当前阵法 permanent_slots 影响的 attr_key 列表（无阵法或都不在面板里 → 空）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub formation_attr_keys: Vec<String>,
    /// talent_id → 受其影响的前端 attr_key 列表（仅奇穴 hardcode 增益部分：13124 活血 / 13366 从容）
    /// 其他奇穴效果通过秘籍/buff 间接影响，已在 buff_attr_keys 里
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub talent_attr_keys: HashMap<u32, Vec<String>>,
    /// 当前剩余 GCD 时间（秒），供前端计算非GCD技能偏移
    pub remaining_gcd: f64,
    /// 当前 GCD 总时长（秒）
    pub total_gcd: f64,
    /// 连招状态：基础技能名 → (连招段名, 剩余秒数)
    pub combo_states: HashMap<String, (String, f64)>,
    /// 奇穴/秘籍修正后的技能数值：基础技能名 → {max_charges, charge_cd, rage_cost}
    pub skill_effective: HashMap<String, SkillEffective>,
    /// 宏调试信息
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub macro_debug: Vec<macro_eval::MacroStepDebug>,
    /// 宏单步模式：下一个应释放的技能名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macro_next_skill: Option<String>,
    /// 总伤害（所有技能事件之和）
    pub total_damage: f64,
    /// 开战面板属性（t=0 时刻：永久 buff / 阵法 / 装备 / 团辅永久型 全部已挂）
    /// 前端循环模拟左栏"实时面板"用；attributes 缺省时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_stats: Option<CombatStats>,
    /// DPS（总伤害 / 战斗时长）
    pub dps: f64,
    /// 坚铁/寒甲 期望传播 末态快照（仅启用时）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expectation: Option<ExpectationSnapshot>,
    /// 时间轴指纹（用于 baseline / event-driven 重构期间的差分验证）
    /// 64-bit ahash 哈希了所有事件的 (cast_time, skill_id, damage_total) 三元组，按顺序。
    /// 对相同输入的两次模拟，fingerprint 必须 bit-equal。
    pub fingerprint: u64,
}

/// 坚铁/寒甲 期望末态（前端 hover/状态面板显示用）
#[derive(Debug, Serialize)]
pub struct ExpectationSnapshot {
    pub boss_attack_interval: f64,
    pub h_per_frame: f64,
    /// 坚铁：期望层数（连续）
    pub jiantie_e_stacks: f64,
    /// 坚铁：期望招架率（含 +0.06×E[k]）
    pub jiantie_e_parry_rate: f64,
    /// 坚铁：层数概率分布 [P(k=0), ..., P(k=5)]
    pub jiantie_stack_probs: Vec<f64>,
    /// 寒甲：存活概率
    pub hanjia_p_alive: f64,
    /// 寒甲：A 期望层数（×30000 攻击力）
    pub hanjia_e_a: f64,
    /// 寒甲：B 期望层数（×300 攻击力）
    pub hanjia_e_b: f64,
    /// 寒甲：期望攻击力加成 = E[A]×30000 + E[B]×300
    pub hanjia_atk_bonus: f64,
}

impl ExpectationSnapshot {
    pub fn from_player(player: &Player) -> Option<Self> {
        let state = player.expectation.as_ref()?;
        let e_a = state.last_hanjia.e_a;
        let e_b = state.last_hanjia.e_b;
        Some(Self {
            boss_attack_interval: state.boss_attack_interval,
            h_per_frame: state.h_per_frame,
            jiantie_e_stacks: state.last_jiantie.e_stacks,
            jiantie_e_parry_rate: state.last_jiantie.e_parry_rate,
            jiantie_stack_probs: state.last_jiantie.stack_probs.to_vec(),
            hanjia_p_alive: state.last_hanjia.p_alive,
            hanjia_e_a: e_a,
            hanjia_e_b: e_b,
            hanjia_atk_bonus: e_a * 30000.0 + e_b * 300.0,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct SkillEffective {
    pub max_charges: u32,
    pub charge_cd: f64,
    pub rage_cost: u32,
}

/// 单个 buff 的时间轴轨道
#[derive(Debug, Serialize)]
pub struct BuffTimelineTrack {
    pub buff_id: u32,
    pub name: String,
    pub short_name: String,
    pub color: String,
    pub events: Vec<BuffTimelineEvent>,
}

/// buff 时间轴事件类型
#[derive(Debug, Serialize)]
pub struct BuffTimelineEvent {
    pub time: f64,
    /// "gain" = 获得, "expire" = 到期, "tick" = 每跳, "remove" = 被动移除
    pub event_type: String,
    /// 事件时刻的玩家状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<EventState>,
}

// ── 姿态 / 连招 Buff ID ────────────────────────────────────────────
// 姿态和连招统一通过 buff 系统管理，duration_frames=0 表示永久


/// 姿态枚举 → 对应的 Buff ID
fn stance_buff_id(stance: Stance) -> Option<u32> {
    match stance {
        Stance::Shield => Some(BUFF_STANCE_SHIELD),
        Stance::Blade  => Some(BUFF_STANCE_BLADE),
        Stance::Wall   => Some(BUFF_STANCE_WALL),
        Stance::Any | Stance::NotWall => None,
    }
}

/// 连招名称 → 确定性 Buff ID（高位避免与游戏 ID 冲突）
pub(crate) fn combo_buff_id(name: &str) -> u32 {
    let mut h: u32 = 0xE0_00_00_00;
    for b in name.bytes() { h = h.wrapping_mul(31).wrapping_add(b as u32); }
    h | 0xE0_00_00_00
}

/// 连招 Buff 默认持续帧数（4秒 = 64帧）
const COMBO_DURATION: u32 = 64;

/// 玩家对象：所有角色状态统一通过 Buff 管理
pub struct Player {
    /// 心法（决定脚本中的 mount 分支与常量来源）
    pub mount: Mount,
    /// 武学版本（决定脚本注册表路由）
    pub version: GameVersion,
    /// 心法常量（主属性 → 副属性 转化系数）
    pub constants: MountConstants,
    haste_level: u32,
    pub active_cds: HashMap<String, f64>,
    pub channel_end: f64,
    /// 当前模拟时钟
    pub current_time: f64,
    active_talents: HashSet<u32>,
    /// 已选秘籍 ID 集合（用户配置 + talent联动；整局不变）
    pub active_recipes: HashSet<u32>,
    /// buff 激活的秘籍 ID 集合（每个秘籍记激活计数；归零后移除）
    pub buff_recipes: HashMap<u32, u32>,
    /// 自身 Buff（游戏 Buff + 姿态 Buff + 连招 Buff）
    active_buffs: Vec<BuffInstance>,
    /// 目标 Buff/Debuff
    target_buffs: Vec<BuffInstance>,
    /// 当前怒气 (0~100)
    pub rage: i32,
    /// 当前格挡值 (0~100)（铁骨衣资源；寒啸千军等消耗）
    pub block_value: i32,
    /// 肃驾累积消耗怒气余量（每满 11 兑 1 格挡值）
    pub rage_spent_accumulator: u32,
    /// 脚本请求的时间推进量（秒），由 simulate 循环在脚本执行后处理
    pub pending_advance: f64,
    /// 上一次技能实际扣除的怒气量
    pub last_rage_cost: u32,
    /// 上一次主动施放的时刻（用于 cd_wait 计算的 base_time 下限）
    pub last_cast_time: f64,
    /// 装备清单：position → equip_id（前端从配装器拷贝；脚本可查 `equip_id_at(pos)`）
    /// 写入请走 set_equipped()（同步维护 equipped_values 索引）。
    pub equipped: HashMap<String, u32>,
    /// equipped 的所有 value 集合（含 ENCHANT_*）。`has_enchant` 走它做 O(1) 查询，
    /// 避免 on_post_cast 每次 cast 跑 27 次 has_enchant × O(N) 扫描。
    /// 战斗中装备清单不变，所以一次性构建后只读。
    pub equipped_values: ahash::AHashSet<u32>,
    /// 装备特效累计器（1024 制：每次主技能加 prob，>=1024 触发并 -1024）
    /// 当前用：盾击·神兵（天下宏愿，prob=307）/ 盾压·神兵（驭焰，prob=205）
    /// key = 触发段 skill_id；value = 累计值（i32 防越界）
    pub equip_effect_accum: HashMap<u32, i32>,
    /// 属性面板快照（simulate 开始前 set 一次，供脚本读取）
    pub base_attrs: Attributes,
    /// 上一次引导技能的实际跳数
    pub last_channel_ticks: u32,
    /// 引导技能的释放时间（用于打断时计算实际跳数）
    channel_cast_time: f64,
    /// 引导技能的首跳帧数（实际值）
    channel_first_frame: u32,
    /// 引导技能的每跳间隔帧数（实际值）
    channel_interval_frame: u32,
    /// 引导技能的技能 ID
    channel_skill_id: u32,
    /// buff 时间轴事件收集器：buff_id → Vec<(time, event_type, state_snapshot)>
    pub buff_events: HashMap<u32, Vec<(f64, String, EventState)>>,
    /// 充能状态：skill_id → (当前层数, 下一层恢复时间)
    charges: HashMap<u32, (u32, f64)>,
    /// 卷雪刀（平砍）：上次触发时刻；None 表示未启动循环
    pub last_swing_time: Option<f64>,
    /// 实验性武学开关（前端 experimental_skills toggle）
    pub experimental: bool,
    /// Lite 模式：跳过 buff_events 记录（snapshot），跳过 timeline 详情。
    /// 由 simulate_core 在 req.lite=true 时设置。批量场景用。
    pub lite_mode: bool,
    /// 当前选中阵法（None = 不开阵）
    pub formation: Option<FormationSelection>,
    /// 阵法永久 effects 聚合（simulate_core 入口预算；aggregate_buff_fields 合并）
    pub formation_permanent_slots: AttribSlots,
    /// 自己开阵的 formation_id（None = 非 self / 未开）；触发挂载守卫用，避免脚本查 table
    pub formation_self_id: Option<String>,
    /// 雾海寻龙·阵云结晦消耗的长驱万里层数（供雁门迢递·雾海读取）
    pub zhen_yun_consumed_stacks: u32,
    /// 坚铁/寒甲期望传播子系统（Some 时启用）
    pub expectation: Option<ExpectationState>,
    /// Boss 周期受击：下次受击时刻（None = 未启用受击模拟）
    pub next_boss_attack: Option<f64>,
    /// Boss 攻击间隔（秒）
    pub boss_attack_interval: f64,
    /// 盾压 CD 期望重置状态（仅铁骨衣）
    pub dunya_cd: Option<DunyaCdState>,
    /// 上一帧是否施放了非盾压盾系技能（盾刀/盾击/盾猛）
    pub last_cast_shield_non_dunya: bool,
    /// buff 变更代数（每次 add/remove/modify buff 递增，用于属性缓存）
    pub buff_generation: u64,
    /// 属性缓存：(generation, buff_slots, target_slots, stats)
    buff_cache: std::cell::RefCell<Option<(u64, AttribSlots, AttribSlots, CombatStats)>>,
    /// buff_id → active_buffs 索引（按 buff_generation 失效）
    /// 用于 macro_eval 阶段一的条件查询，跨多次 phase1 调用复用，避免重复 collect。
    buff_idx_cache: std::cell::RefCell<Option<(u64, ahash::AHashMap<u32, usize>)>>,
    /// 同上，目标 buff
    target_idx_cache: std::cell::RefCell<Option<(u64, ahash::AHashMap<u32, usize>)>>,
    /// 决策代数：每次 cast/buff_add/buff_remove/buff_tick 完成后递增。
    /// macro_eval 用来判断 phase1/phase2 输出是否需要重算（不含 bufftime，
    /// 那是另一种基于时间的失效，由事件队列处理）。
    pub decision_generation: u64,
    /// 团队增益排程队列（按时间挂 buff，simulate_core 入口预排，process_buff_ticks 内消费）
    /// 已按 release_at 升序排列；消费后从前端弹出
    pub pending_team_buffs: Vec<PendingTeamBuff>,
}

/// 团队增益排程：在 release_at 时刻把指定 buff（带 stacks 和 duration）挂到 player
#[derive(Debug, Clone)]
pub struct PendingTeamBuff {
    pub release_at: f64,
    pub buff_id: u32,
    pub level: u32,
    pub stacks: u32,
    /// 0 = 永久；其他 = 自定义 duration_frames（覆盖 BuffDef.duration_frames）
    pub duration_frames: u32,
    pub is_target: bool,
}

/// 盾压 CD 期望重置：小数 CD + 可用性信用
pub struct DunyaCdState {
    /// 期望剩余 CD（帧，小数）
    pub cd_remain: f64,
    /// 可用性信用 [0, 1]：累积到 1 时触发 reset_cd
    pub avail_credit: f64,
    /// 盾压实际 CD 帧数（含秘籍减 CD）
    pub cd_frames: u32,
    /// 秘籍额外触发几率加成（4007/4008 各 +5%）
    pub extra_reset_prob: f64,
    /// 上次推进到的帧数
    pub last_frame: u32,
}

/// 坚铁/寒甲 期望分布运行时状态
///
/// 由 `Player.advance_expectation_to(t)` 推进，主循环每次 advance 调用一次。
/// - `last_frame`：当前已推进到的帧数
/// - `last_stats`：最近一次 tick 的输出（注入给合成 buff、UI、tooltip）
pub struct ExpectationState {
    pub jiantie: expectation::JiantieDist,
    pub hanjia: expectation::HanjiaCarry,
    /// 已推进到的帧数（current_time = frame / FPS）
    pub last_frame: u32,
    /// boss 平均攻击间隔（秒）
    pub boss_attack_interval: f64,
    /// poisson 模式下的每帧受击概率（缓存，固定值）
    pub h_per_frame: f64,
    /// 寒甲期望传播开关（true = A/B 走 expected_stacks；false = A/B 走 recalc_stacks）
    pub hanjia_expectation: bool,
    /// 最近一帧的坚铁输出
    pub last_jiantie: expectation::JiantieFrameStats,
    /// 最近一帧的寒甲输出
    pub last_hanjia: expectation::HanjiaFrameStats,
}

impl ExpectationState {
    pub fn new(boss_attack_interval: f64, hanjia_expectation: bool) -> Self {
        Self {
            jiantie: expectation::JiantieDist::new(0.75),
            hanjia: expectation::HanjiaCarry::new(12.0),
            last_frame: 0,
            boss_attack_interval,
            h_per_frame: expectation::poisson_h(boss_attack_interval),
            hanjia_expectation,
            last_jiantie: expectation::JiantieFrameStats::default(),
            last_hanjia: expectation::HanjiaFrameStats::default(),
        }
    }
}

impl Player {
    /// 兼容调用：默认 分山劲 + 暗影千机 + 分山劲常量
    pub fn new(haste_level: u32, talents: Vec<u32>, recipes: Vec<u32>) -> Self {
        Self::with_mount(Mount::FenShanJin, GameVersion::AnYingQianJi,
                         MountConstants::fenshanjin_default(),
                         haste_level, talents, recipes)
    }

    pub fn with_mount(mount: Mount, version: GameVersion, constants: MountConstants,
                      haste_level: u32, talents: Vec<u32>, recipes: Vec<u32>) -> Self {
        let mut p = Player {
            mount,
            version,
            constants,
            haste_level,
            active_cds: HashMap::new(),
            channel_end: 0.0,
            channel_cast_time: 0.0,
            channel_first_frame: 0,
            channel_interval_frame: 0,
            channel_skill_id: 0,
            current_time: 0.0,
            active_talents: talents.into_iter().collect(),
            active_recipes: recipes.into_iter().collect(),
            buff_recipes: HashMap::new(),
            active_buffs: Vec::new(),
            target_buffs: Vec::new(),
            rage: 0,
            block_value: 100,
            rage_spent_accumulator: 0,
            pending_advance: 0.0,
            last_rage_cost: 0,
            last_cast_time: 0.0,
            equipped: HashMap::new(),
            equipped_values: ahash::AHashSet::new(),
            equip_effect_accum: HashMap::new(),
            base_attrs: Attributes::default(),
            last_channel_ticks: 0,
            buff_events: HashMap::new(),
            charges: HashMap::new(),
            last_swing_time: None,
            experimental: false,
            lite_mode: false,
            zhen_yun_consumed_stacks: 0,
            expectation: None,
            next_boss_attack: None,
            boss_attack_interval: 0.0,
            dunya_cd: None,
            last_cast_shield_non_dunya: false,
            buff_generation: 0,
            buff_cache: std::cell::RefCell::new(None),
            buff_idx_cache: std::cell::RefCell::new(None),
            target_idx_cache: std::cell::RefCell::new(None),
            decision_generation: 0,
            pending_team_buffs: Vec::new(),
            formation: None,
            formation_permanent_slots: HashMap::new(),
            formation_self_id: None,
        };
        p.set_stance(Stance::Shield);
        // 按奇穴计算格挡值上限（坚韧 13363 给 +100 max）并初始化为满值
        p.block_value = p.max_block_value();
        p
    }

    /// 格挡值上限（铁骨衣基础 100，奇穴 13363「坚韧」+100 → 200）
    pub fn max_block_value(&self) -> i32 {
        let base = 100;
        if self.has_talent(13363) { base + 100 } else { base }
    }

    /// 通用装备查询：返回 position 上的 equip_id（未装备 = 0）。
    /// 脚本层用法：`if player.equip_id_at("PRIMARY_WEAPON") == 40245 { /* 天下宏愿 */ }`
    /// position 字符串与配装器侧约定一致：HAT/JACKET/BELT/WRIST/BOTTOMS/SHOES/NECKLACE/PENDANT/RING_1/RING_2/PRIMARY_WEAPON/SECONDARY_WEAPON。
    pub fn equip_id_at(&self, position: &str) -> u32 {
        self.equipped.get(position).copied().unwrap_or(0)
    }
    /// 是否在指定 position 装备了 ID 列表中的任何一件
    pub fn has_equip_in(&self, position: &str, ids: &[u32]) -> bool {
        let cur = self.equip_id_at(position);
        cur != 0 && ids.contains(&cur)
    }

    /// 全身装备里有几件在 `ids` 列表里（用于套装 N 件套判定）
    /// 用法：`if player.count_equip_in(&CY_T_SET_6782_IDS) >= 4 { /* 4 件套激活 */ }`
    pub fn count_equip_in(&self, ids: &[u32]) -> u32 {
        self.equipped.values().filter(|&&eid| eid != 0 && ids.contains(&eid)).count() as u32
    }

    /// 大附魔查询：复用 Player.equipped；前端约定 key 为 ENCHANT_HAT / ENCHANT_JACKET /
    /// ENCHANT_BELT / ENCHANT_WRIST / ENCHANT_SHOES（与装备 slot key 同一 HashMap）。
    /// O(1) — 走 equipped_values 索引（set_equipped 同步维护）。
    pub fn has_enchant(&self, enchant_id: u32) -> bool {
        self.equipped_values.contains(&enchant_id)
    }

    /// 写装备清单 + 同步 equipped_values 索引（O(1) has_enchant 用）。
    /// 战斗中装备不变，simulate_core 入口 set 一次即可。
    pub fn set_equipped(&mut self, m: HashMap<String, u32>) {
        self.equipped_values = m.values().copied().filter(|&v| v != 0).collect();
        self.equipped = m;
    }

    /// 装备特效"期望累计触发"——1024 制累加器：
    ///   每次主技能 cast 时调一次 `accum_equip_effect(skill_id, prob, 1024)`；
    ///   累计 ≥ scale 时返回 true 并 -scale，调用方 emit 对应 skill_id 的伤害事件。
    /// `prob` 是 1024 制概率（如盾击神兵 307 ≈ 30%；盾压神兵 205 ≈ 20%）。
    /// 期望确定性：长时间下触发次数 = floor(N × prob / scale)（误差 ±1）。
    pub fn accum_equip_effect(&mut self, skill_id: u32, prob: i32, scale: i32) -> bool {
        let v = self.equip_effect_accum.entry(skill_id).or_insert(0);
        *v += prob;
        if *v >= scale {
            *v -= scale;
            true
        } else {
            false
        }
    }

    /// 决策代数 +1（任何会改变 phase1/phase2 输出的状态变更后调用）
    /// 同时保留与 buff_generation 的语义：`buff_generation` 为属性缓存键，
    /// `decision_generation` 为 macro 决策缓存键。两者必须同步。
    #[inline]
    pub fn bump_decision_gen(&mut self) {
        self.decision_generation = self.decision_generation.wrapping_add(1);
    }

    /// buff_id → active_buffs 索引的查询表（按 buff_generation 复用）。
    /// 调用方约定：仅在 phase1 期间使用（无 buff 修改），且 active_buffs 没有过期项
    /// （process_buff_ticks 已清理）。返回 Ref，借用期间持有 RefCell 的不可变锁。
    pub fn buff_idx_lookup(&self) -> std::cell::Ref<'_, ahash::AHashMap<u32, usize>> {
        let gen = self.buff_generation;
        let needs_rebuild = self.buff_idx_cache.borrow()
            .as_ref().map_or(true, |(g, _)| *g != gen);
        if needs_rebuild {
            let m: ahash::AHashMap<u32, usize> = self.active_buffs.iter().enumerate()
                .map(|(i, b)| (b.buff_id, i))
                .collect();
            *self.buff_idx_cache.borrow_mut() = Some((gen, m));
        }
        std::cell::Ref::map(self.buff_idx_cache.borrow(),
            |c| &c.as_ref().expect("buff_idx_cache initialized above").1)
    }

    /// 同 buff_idx_lookup，针对 target_buffs。
    pub fn target_idx_lookup(&self) -> std::cell::Ref<'_, ahash::AHashMap<u32, usize>> {
        let gen = self.buff_generation;
        let needs_rebuild = self.target_idx_cache.borrow()
            .as_ref().map_or(true, |(g, _)| *g != gen);
        if needs_rebuild {
            let m: ahash::AHashMap<u32, usize> = self.target_buffs.iter().enumerate()
                .map(|(i, b)| (b.buff_id, i))
                .collect();
            *self.target_idx_cache.borrow_mut() = Some((gen, m));
        }
        std::cell::Ref::map(self.target_idx_cache.borrow(),
            |c| &c.as_ref().expect("target_idx_cache initialized above").1)
    }

    /// debug-only 健康检查：所有不变量（仅 debug build 跑，release 编译消除）。
    /// 调用点：process_buff_ticks 末尾、cast_skill 末尾、关键 setter 后。
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    fn check_invariants(&self) {
        assert!(self.rage >= 0 && self.rage <= 100, "rage 越界: {}", self.rage);
        let max_bv = self.max_block_value();
        assert!(self.block_value >= 0 && self.block_value <= max_bv,
            "block_value 越界: {} max={}", self.block_value, max_bv);
        for (cd, &v) in &self.active_cds {
            assert!(v >= 0.0 && v.is_finite(),
                "active_cds[{}] 非法: {}", cd, v);
        }
        for inst in &self.active_buffs {
            assert!(inst.stacks > 0 || inst.expected_stacks.is_some(),
                "active_buff stacks=0 还在表里: id={}", inst.buff_id);
        }
    }
    #[cfg(not(debug_assertions))]
    #[inline]
    #[allow(dead_code)]
    fn check_invariants(&self) {}

    /// 计算"下一次状态可能改变的时刻"——所有可能影响 phase1/phase2 输出的事件源里取最早。
    /// 直接从当前状态计算，**不维护事件队列**，保证不会"忘了 push 事件"。
    ///
    /// `bufftime_thresholds`: macro 里所有 `bufftime:X<N` / `bufftime:X>N` 的 (buff_id, N, is_target) 三元组。
    /// 用于检测 "buff X 还剩 N 秒" 这一阈值跨越的时刻。
    pub fn next_decision_time(&self, bufftime_thresholds: &[(u32, f64, bool)]) -> f64 {
        let cur = self.current_time;
        let mut next = f64::INFINITY;
        let consider = |next: &mut f64, t: f64| {
            if t > cur && t < *next { *next = t; }
        };

        // 1. 所有 active_cds 的到期时刻
        for &t in self.active_cds.values() {
            consider(&mut next, t);
        }

        // 2. 充能恢复时刻（仅当当前层数 < max_charges 时才有意义）
        // 注：max_charges 由 effective_max_charges(skill) 决定；此处无 skill，
        //     只看 charges 表里的 next_t —— 若 ch=max_charges，next_t 不会被写新；
        //     即使写了，phase2 会无伤通过，没问题。
        for &(_ch, next_t) in self.charges.values() {
            consider(&mut next, next_t);
        }

        // 3. 引导结束
        consider(&mut next, self.channel_end);

        // 4. 自身 buff 过期 + 下次 tick
        for inst in &self.active_buffs {
            if inst.expires_at != 0.0 {
                consider(&mut next, inst.expires_at);
            }
            if inst.tick_interval_frames > 0 {
                let tick_sec = frames_to_sec(inst.tick_interval_frames);
                let buff_start = if inst.expires_at == 0.0 { 0.0 }
                    else { inst.expires_at - frames_to_sec(inst.duration_frames) };
                let elapsed = cur - buff_start;
                if elapsed >= 0.0 && tick_sec > 0.0 {
                    let next_tick = buff_start + ((elapsed / tick_sec).floor() + 1.0) * tick_sec;
                    if inst.expires_at == 0.0 || next_tick <= inst.expires_at {
                        consider(&mut next, next_tick);
                    }
                }
            }
        }

        // 5. 目标 buff 过期 + tick
        for inst in &self.target_buffs {
            if inst.expires_at != 0.0 {
                consider(&mut next, inst.expires_at);
            }
            if inst.tick_interval_frames > 0 {
                let tick_sec = frames_to_sec(inst.tick_interval_frames);
                let buff_start = if inst.expires_at == 0.0 { 0.0 }
                    else { inst.expires_at - frames_to_sec(inst.duration_frames) };
                let elapsed = cur - buff_start;
                if elapsed >= 0.0 && tick_sec > 0.0 {
                    let next_tick = buff_start + ((elapsed / tick_sec).floor() + 1.0) * tick_sec;
                    if inst.expires_at == 0.0 || next_tick <= inst.expires_at {
                        consider(&mut next, next_tick);
                    }
                }
            }
        }

        // 6. Boss 周期攻击（坚铁/寒甲驱动）
        if let Some(nba) = self.next_boss_attack {
            consider(&mut next, nba);
        }

        // 7. 卷雪刀下次触发（24 帧 × 加速折算）
        if let Some(last_swing) = self.last_swing_time {
            let actual = get_actual_frames(24, self.effective_haste_level());
            let next_swing = last_swing + frames_to_sec(actual);
            consider(&mut next, next_swing);
        }

        // 8. bufftime 阈值翻转：buff_X 剩余时长穿过 N 秒的那一刻
        for &(buff_id, threshold_sec, is_target) in bufftime_thresholds {
            let buffs = if is_target { &self.target_buffs } else { &self.active_buffs };
            for inst in buffs {
                if inst.buff_id == buff_id && inst.expires_at != 0.0 {
                    let flip_t = inst.expires_at - threshold_sec;
                    consider(&mut next, flip_t);
                }
            }
        }

        next
    }

    /// 怒气写入（自动 clamp 0~100 + bump decision generation）。
    /// 所有"直接 player.rage = X"应该走这个 setter。
    #[inline]
    pub fn set_rage(&mut self, v: i32) {
        self.rage = v.clamp(0, 100);
        self.bump_decision_gen();
        debug_assert!(self.rage >= 0 && self.rage <= 100, "rage out of range: {}", self.rage);
    }
    /// 怒气增减（自动 clamp 0~100 + bump）。负数为消耗。
    #[inline]
    pub fn add_rage(&mut self, delta: i32) {
        self.set_rage(self.rage + delta);
    }
    /// 设置 buff 实例当前层数（自动 bump）。
    /// 用于脚本里需要直接覆盖层数的场景（如业火麟光给 9 层）。
    pub fn set_buff_stacks(&mut self, buff_id: u32, stacks: u32) {
        for inst in &mut self.active_buffs {
            if inst.buff_id == buff_id {
                inst.stacks = stacks;
            }
        }
        self.bump_decision_gen();
    }
    /// 添加任意命名 CD（含 protect 系列）。自动 bump。
    pub fn add_protect_cd(&mut self, cd_id: &str, expires_at: f64) {
        self.active_cds.insert(cd_id.to_string(), expires_at);
        self.bump_decision_gen();
    }
    /// 格挡值写入（铁骨衣，自动 clamp 0~max + bump）。
    #[inline]
    pub fn set_block_value(&mut self, v: i32) {
        let max_bv = self.max_block_value();
        self.block_value = v.clamp(0, max_bv);
        self.bump_decision_gen();
        debug_assert!(self.block_value >= 0 && self.block_value <= max_bv,
            "block_value out of range: {} (max={})", self.block_value, max_bv);
    }
    #[inline]
    pub fn add_block_value(&mut self, delta: i32) {
        self.set_block_value(self.block_value + delta);
    }

    /// 启动卷雪刀循环（首次主动技能 cast 时调用；幂等）
    pub fn start_swing(&mut self, time: f64) {
        if self.last_swing_time.is_none() {
            self.last_swing_time = Some(time);
        }
    }

    /// 是否激活了某秘籍（用户配 OR buff 激活）
    pub fn recipe_active(&self, recipe_id: u32) -> bool {
        self.active_recipes.contains(&recipe_id)
            || self.buff_recipes.contains_key(&recipe_id)
    }

    /// buff 激活时主动激活其 activate_recipes（仅在 buff 首次添加时调用）
    fn buff_activate_recipes(&mut self, ids: &[u32]) {
        for &rid in ids {
            *self.buff_recipes.entry(rid).or_insert(0) += 1;
        }
    }

    /// buff 失效时撤销其 activate_recipes
    fn buff_deactivate_recipes(&mut self, ids: &[u32]) {
        for &rid in ids {
            if let Some(cnt) = self.buff_recipes.get_mut(&rid) {
                if *cnt > 1 { *cnt -= 1; }
                else { self.buff_recipes.remove(&rid); }
            }
        }
    }

    // ── Buff 操作 ──

    pub fn has_talent(&self, talent_id: u32) -> bool {
        self.active_talents.contains(&talent_id)
    }

    pub fn has_recipe(&self, recipe_id: u32) -> bool {
        self.active_recipes.contains(&recipe_id)
    }

    // ── 目标 Buff ──

    pub fn has_target_buff(&self, buff_id: u32) -> bool {
        self.target_buffs.iter().any(|b| {
            b.buff_id == buff_id &&
            (b.expires_at == 0.0 || b.expires_at > self.current_time)
        })
    }

    pub fn add_target_buff<T: Into<BuffSpec>>(&mut self, spec: T) {
        let spec = spec.into();
        self.buff_generation += 1;
        self.bump_decision_gen();
        let def = self.buff_def(spec.buff_id)
            .unwrap_or_else(|| panic!("add_target_buff: unknown buff_id {}", spec.buff_id));
        self.add_target_buff_impl(def, spec.level);
    }

    fn add_target_buff_impl(&mut self, def: &BuffDef, level: u32) {
        let is_new = !self.target_buffs.iter().any(|b| b.buff_id == def.buff_id);
        let eff_haste = self.effective_haste_level();
        if let Some(inst) = self.target_buffs.iter_mut().find(|b| b.buff_id == def.buff_id) {
            if def.haste_scaled && def.tick_interval > 0 && inst.tick_interval_frames > 0 {
                // 刷新快照：保留"下一跳"时刻（按旧 interval 算），之后按新 interval 重新布局
                let total_ticks = def.duration_frames / def.tick_interval;
                let old_interval_sec = frames_to_sec(inst.tick_interval_frames);
                let old_start = inst.expires_at - frames_to_sec(inst.duration_frames);
                let elapsed = (self.current_time - old_start).max(0.0);
                let k = (elapsed / old_interval_sec).floor() as u32;
                let next_tick = old_start + (k + 1) as f64 * old_interval_sec;

                let new_tick = get_actual_frames(def.tick_interval, eff_haste).max(1);
                let new_interval_sec = frames_to_sec(new_tick);
                let new_start = next_tick - new_interval_sec;
                let new_duration = new_tick * total_ticks;
                let new_expires = new_start + frames_to_sec(new_duration);

                inst.tick_interval_frames = new_tick;
                inst.duration_frames = new_duration;
                inst.expires_at = new_expires;
            } else {
                // 非加速型：原逻辑，保留 tick 节奏，延长 expires_at
                let expires = if def.duration_frames == 0 { 0.0 }
                    else { self.current_time + frames_to_sec(def.duration_frames) };
                let original_start = inst.expires_at - frames_to_sec(inst.duration_frames);
                inst.expires_at = expires;
                inst.duration_frames = sec_to_frames(expires - original_start);
            }
            if inst.stacks < def.max_stacks {
                inst.stacks += 1;
                self.record_buff_event(def.buff_id, "stack");
            }
        } else {
            // 初次添加：按当前 haste 快照 tick_interval 和 duration
            let (actual_tick, actual_dur) = if def.haste_scaled && def.tick_interval > 0 {
                let at = get_actual_frames(def.tick_interval, eff_haste).max(1);
                let total_ticks = def.duration_frames / def.tick_interval;
                (at, at * total_ticks)
            } else {
                (def.tick_interval, def.duration_frames)
            };
            let expires = if actual_dur == 0 { 0.0 }
                else { self.current_time + frames_to_sec(actual_dur) };
            self.target_buffs.push(BuffInstance {
                buff_id: def.buff_id, stacks: 1,
                duration_frames: actual_dur, expires_at: expires, tick_elapsed: 0,
                tick_interval_frames: actual_tick,
                snapshot: None,
                level,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
        if is_new { self.record_buff_event(def.buff_id, "gain"); }
    }

    pub fn remove_target_buff(&mut self, buff_id: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        if self.target_buffs.iter().any(|b| b.buff_id == buff_id) {
            self.record_buff_event(buff_id, "remove");
        }
        self.target_buffs.retain(|b| b.buff_id != buff_id);
    }

    pub fn has_buff(&self, buff_id: u32) -> bool {
        self.active_buffs.iter().any(|b| {
            b.buff_id == buff_id &&
            (b.expires_at == 0.0 || b.expires_at > self.current_time)
        })
    }

    /// 累加所有生效 buff（含目标 debuff）对指定 AttribField 的 value（按 inst.stacks 倍乘）
    pub fn sum_buff_field(&self, field: AttribField) -> f64 {
        let t = self.current_time;
        let mut total = 0.0;
        for inst in self.active_buffs.iter().chain(self.target_buffs.iter()) {
            if inst.expires_at != 0.0 && inst.expires_at <= t { continue; }
            let def = match self.buff_def(inst.buff_id) { Some(d) => d, None => continue };
            let mult = inst.expected_stacks.unwrap_or(inst.stacks as f64);
            for e in def.effects {
                if e.field == field { total += e.value * mult; }
            }
            // 实例级动态 effects
            for e in &inst.extra_effects {
                if e.field == field { total += e.value * mult; }
            }
        }
        // 跨字段换算：查 ParryValueBase 时追加"基础体质" × ∑cof / 1024
        // （不递归，cof 自己查询时不触发这段）
        if field == AttribField::ParryValueBase {
            let cof = self.sum_buff_field(AttribField::VitalityToParryValueCof);
            total += self.effective_base_vitality() * cof / 1024.0;
        }
        total
    }

    /// 获取当前完整面板属性。走 buff_cache 缓存（与 calc_stats_cached 共享）；
    /// buff_generation 没变时直接 clone CombatStats，省一次 aggregate + build_runtime_stats。
    /// 对 on_post_cast 鞋大附魔 / 黄字腰坠 等每 cast 都查 crit_rate 的路径影响显著。
    pub fn current_stats(&self) -> CombatStats {
        {
            let cache = self.buff_cache.borrow();
            if let Some((gen, _, _, ref rt)) = *cache {
                if gen == self.buff_generation {
                    perf_add(|p| { p.cache_hit += 1; });
                    return rt.clone();
                }
            }
        }
        perf_add(|p| { p.cache_miss += 1; });
        let buff_slots = aggregate_buff_fields(self);
        let target_slots = aggregate_target_buff_fields(self);
        let rt = build_runtime_stats(&self.base_attrs, &buff_slots, &self.constants);
        *self.buff_cache.borrow_mut() = Some((self.buff_generation, buff_slots, target_slots, rt.clone()));
        rt
    }

    /// 基础体质 = 面板体质 + 加算型 buff（VitalityBase + BasePotentialAdd）
    /// 不含百分比增益。仅供 aggregate_buff_fields / sum_buff_field 内部使用。
    /// 脚本侧用 current_stats().base_vitality。
    pub(crate) fn effective_base_vitality(&self) -> f64 {
        self.base_attrs.vitality
            + self.sum_buff_field(AttribField::VitalityBase)
            + self.sum_buff_field(AttribField::BasePotentialAdd)
    }

    /// 自身 buff 层数（不存在返回 0）
    pub fn buff_stacks(&self, buff_id: u32) -> u32 {
        self.active_buffs.iter()
            .find(|b| b.buff_id == buff_id && (b.expires_at == 0.0 || b.expires_at > self.current_time))
            .map(|b| b.stacks)
            .unwrap_or(0)
    }

    /// 目标 buff 层数
    pub fn target_buff_stacks(&self, buff_id: u32) -> u32 {
        self.target_buffs.iter()
            .find(|b| b.buff_id == buff_id && (b.expires_at == 0.0 || b.expires_at > self.current_time))
            .map(|b| b.stacks)
            .unwrap_or(0)
    }

    /// 自身 buff 剩余秒数（不存在返回 None）
    pub fn buff_remaining(&self, buff_id: u32) -> Option<f64> {
        self.active_buffs.iter()
            .find(|b| b.buff_id == buff_id && (b.expires_at == 0.0 || b.expires_at > self.current_time))
            .map(|b| if b.expires_at == 0.0 { f64::MAX } else { (b.expires_at - self.current_time).max(0.0) })
    }

    /// 目标 buff 剩余秒数
    pub fn target_buff_remaining(&self, buff_id: u32) -> Option<f64> {
        self.target_buffs.iter()
            .find(|b| b.buff_id == buff_id && (b.expires_at == 0.0 || b.expires_at > self.current_time))
            .map(|b| if b.expires_at == 0.0 { f64::MAX } else { (b.expires_at - self.current_time).max(0.0) })
    }

    /// 技能是否不在 CD 中（宏 skill_notin_cd 条件）
    /// 优先检查独立 CD（充能层数≥1视为无CD），再检查 GCD
    pub fn is_skill_not_in_cd(&self, skill: &SkillSpec) -> bool {
        // 1. 检查独立 CD（非 GCD）
        for cd in &skill.cooldowns {
            if cd.cd_id.starts_with("gcd_") { continue; }
            if cd.cd_id.starts_with("protect_") { continue; }
            if let Some(&expires) = self.active_cds.get(&cd.cd_id) {
                if expires > self.current_time + 0.001 {
                    // 有独立 CD 在冷却中，但如果是充能技能且有层数则视为无 CD
                    if skill.max_charges > 0 {
                        if let Some(ch) = self.get_charges(skill) {
                            if ch >= 1 { continue; } // 有充能层数，跳过此 CD
                        }
                    }
                    return false; // 独立 CD 冷却中
                }
            }
        }
        // 2. 检查 GCD
        for cd in &skill.cooldowns {
            if !cd.cd_id.starts_with("gcd_") { continue; }
            if let Some(&expires) = self.active_cds.get(&cd.cd_id) {
                if expires > self.current_time + 0.001 {
                    return false; // GCD 冷却中
                }
            }
        }
        true
    }

    /// 获取技能当前充能层数（公开版本，供宏引擎使用）
    pub fn get_charge_count(&self, skill: &SkillSpec) -> u32 {
        self.get_charges(skill).unwrap_or(0)
    }

    fn record_buff_event(&mut self, buff_id: u32, event_type: &str) {
        if self.lite_mode { return; }
        // 所有 buff 都记录到 buff_events（含 show_on_timeline=false 的，如神兵·无双气劲）；
        // 战斗记录 log（buff_log）需要全部，timeline 渲染（buff_timeline）在生成时按 show_on_timeline 过滤
        if let Some(_def) = self.buff_def(buff_id) {
            let state = snapshot_event_state(self);
            self.buff_events.entry(buff_id).or_default()
                .push((self.current_time, event_type.to_string(), state));
        }
    }

    /// 添加 Buff，可额外延长持续帧数。
    /// `spec` 接受 `BUFF_X` (u32) 或 `(BUFF_X, level)` 元组（盾挡等等级 buff 用）。
    /// **若已存在更高 level，则不刷新**（低等级不能顶高等级）。
    pub fn add_buff_extended<T: Into<BuffSpec>>(&mut self, spec: T, extra_frames: u32) {
        let s: BuffSpec = spec.into();
        let def = self.buff_def(s.buff_id)
            .unwrap_or_else(|| panic!("add_buff_extended: unknown buff_id {}", s.buff_id));
        self.add_buff_extended_impl(def, s.level, extra_frames);
    }

    fn add_buff_extended_impl(&mut self, def: &BuffDef, level: u32, extra_frames: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        let is_new = !self.active_buffs.iter().any(|b| b.buff_id == def.buff_id);
        let total = def.duration_frames + extra_frames;
        let expires = if total == 0 { 0.0 }
            else { self.current_time + frames_to_sec(total) };
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == def.buff_id) {
            // 低等级不能顶高等级（level=0 视为无等级概念，正常刷新）
            if level > 0 && inst.level > level { return; }
            inst.expires_at = expires;
            inst.duration_frames = total;
            if level > 0 { inst.level = level; }
            if inst.stacks < def.max_stacks {
                inst.stacks += 1;
                self.record_buff_event(def.buff_id, "stack");
            }
        } else {
            self.active_buffs.push(BuffInstance {
                buff_id: def.buff_id, stacks: 1,
                duration_frames: total, expires_at: expires, tick_elapsed: 0,
                tick_interval_frames: def.tick_interval,
                snapshot: None,
                level,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
        if is_new {
            self.record_buff_event(def.buff_id, "gain");
            // 仅首次添加时激活其秘籍
            let ids: Vec<u32> = def.activate_recipes.to_vec();
            if !ids.is_empty() { self.buff_activate_recipes(&ids); }
        }
    }

    /// 添加/刷新一个 Buff（接受 `BUFF_X` 或 `(BUFF_X, level)`）
    pub fn add_buff<T: Into<BuffSpec>>(&mut self, spec: T) {
        self.add_buff_extended(spec, 0);
    }

    /// 团队增益专用：一次性挂指定层数 + 自定义 duration（0 = 永久）
    /// duration_frames_override = 0 表示永久；其他覆盖 BuffDef.duration_frames
    pub fn add_buff_with_stacks<T: Into<BuffSpec>>(&mut self, spec: T, stacks: u32, duration_frames_override: u32) {
        let s: BuffSpec = spec.into();
        let def = match self.buff_def(s.buff_id) {
            Some(d) => d,
            None => return,
        };
        self.buff_generation += 1;
        self.bump_decision_gen();
        let dur = duration_frames_override;
        let expires = if dur == 0 { 0.0 } else { self.current_time + frames_to_sec(dur) };
        let max_stacks = def.max_stacks.max(1);
        let stacks_clamped = stacks.clamp(1, max_stacks);
        let is_new = !self.active_buffs.iter().any(|b| b.buff_id == s.buff_id);
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == s.buff_id) {
            if s.level > 0 && inst.level > s.level { return; }
            inst.expires_at = expires;
            inst.duration_frames = dur;
            if s.level > 0 { inst.level = s.level; }
            inst.stacks = stacks_clamped;
            self.record_buff_event(s.buff_id, "stack");
        } else {
            self.active_buffs.push(BuffInstance {
                buff_id: s.buff_id, stacks: stacks_clamped,
                duration_frames: dur, expires_at: expires, tick_elapsed: 0,
                tick_interval_frames: def.tick_interval,
                snapshot: None,
                level: s.level,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
        if is_new {
            self.record_buff_event(s.buff_id, "gain");
            let ids: Vec<u32> = def.activate_recipes.to_vec();
            if !ids.is_empty() { self.buff_activate_recipes(&ids); }
        }
    }

    /// 团队增益（debuff）专用：挂目标 buff 指定层数 + 自定义 duration
    pub fn add_target_buff_with_stacks<T: Into<BuffSpec>>(&mut self, spec: T, stacks: u32, duration_frames_override: u32) {
        let s: BuffSpec = spec.into();
        let def = match self.buff_def(s.buff_id) {
            Some(d) => d,
            None => return,
        };
        self.buff_generation += 1;
        self.bump_decision_gen();
        let dur = duration_frames_override;
        let expires = if dur == 0 { 0.0 } else { self.current_time + frames_to_sec(dur) };
        let max_stacks = def.max_stacks.max(1);
        let stacks_clamped = stacks.clamp(1, max_stacks);
        let is_new = !self.target_buffs.iter().any(|b| b.buff_id == s.buff_id);
        if let Some(inst) = self.target_buffs.iter_mut().find(|b| b.buff_id == s.buff_id) {
            if s.level > 0 && inst.level > s.level { return; }
            inst.expires_at = expires;
            inst.duration_frames = dur;
            if s.level > 0 { inst.level = s.level; }
            inst.stacks = stacks_clamped;
            self.record_buff_event(s.buff_id, "stack");
        } else {
            self.target_buffs.push(BuffInstance {
                buff_id: s.buff_id, stacks: stacks_clamped,
                duration_frames: dur, expires_at: expires, tick_elapsed: 0,
                tick_interval_frames: def.tick_interval,
                snapshot: None,
                level: s.level,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
        if is_new { self.record_buff_event(s.buff_id, "gain"); }
    }

    /// 排队团队增益：在 release_at 时刻挂 buff（process_buff_ticks 内消费）
    /// 队列按 release_at 升序保持
    pub fn schedule_team_buff(&mut self, p: PendingTeamBuff) {
        let pos = self.pending_team_buffs.binary_search_by(|x|
            x.release_at.partial_cmp(&p.release_at).unwrap_or(std::cmp::Ordering::Equal)
        ).unwrap_or_else(|e| e);
        self.pending_team_buffs.insert(pos, p);
    }

    /// 修改已挂 buff 的周期 tick 间隔（0 = 禁用 tick）
    /// 用于 Boss 受击模式下禁用寒甲的 3s 自刷新 tick
    pub fn set_buff_tick_interval(&mut self, buff_id: u32, interval_frames: u32) {
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == buff_id) {
            inst.tick_interval_frames = interval_frames;
        }
    }

    /// 找到已挂上的 buff 实例，给它绑定实例级 effects（如盾挡按 level 查的 cof）
    /// 典型用法：`player.add_buff((id, lvl)); player.bind_buff_effects(id, effects);`
    pub fn bind_buff_effects(&mut self, buff_id: u32, effects: Vec<EffectEntry>) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == buff_id) {
            inst.extra_effects = effects;
        }
    }

    /// 添加 Buff（持续时间累加模式，如劫化）
    pub fn add_buff_accumulate(&mut self, buff_id: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        let def = self.buff_def(buff_id)
            .unwrap_or_else(|| panic!("add_buff_accumulate: unknown buff_id {}", buff_id));
        self.add_buff_accumulate_impl(def);
    }

    fn add_buff_accumulate_impl(&mut self, def: &BuffDef) {
        let dur_sec = frames_to_sec(def.duration_frames);
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == def.buff_id) {
            // 累加时间
            let remaining = (inst.expires_at - self.current_time).max(0.0);
            inst.expires_at = self.current_time + remaining + dur_sec;
            inst.duration_frames = sec_to_frames(remaining + dur_sec);
        } else {
            self.active_buffs.push(BuffInstance {
                buff_id: def.buff_id, stacks: 1,
                duration_frames: def.duration_frames,
                expires_at: self.current_time + dur_sec, tick_elapsed: 0,
                tick_interval_frames: def.tick_interval,
                snapshot: None,
                level: 0,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
    }

    pub fn add_state_buff(&mut self, buff_id: u32, duration_frames: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        let expires = if duration_frames == 0 { 0.0 }
            else { self.current_time + frames_to_sec(duration_frames) };
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == buff_id) {
            inst.expires_at = expires;
            inst.duration_frames = duration_frames;
        } else {
            self.active_buffs.push(BuffInstance {
                buff_id, stacks: 1, duration_frames, expires_at: expires, tick_elapsed: 0,
                tick_interval_frames: 0,
                snapshot: None,
                level: 0,
                extra_effects: Vec::new(),
                expected_stacks: None,
                stack_distribution: None,
            });
        }
    }

    /// 移除指定 Buff
    pub fn remove_buff(&mut self, buff_id: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        let existed = self.active_buffs.iter().any(|b| b.buff_id == buff_id);
        if existed {
            self.record_buff_event(buff_id, "remove");
        }
        self.active_buffs.retain(|b| b.buff_id != buff_id);
        if existed {
            if let Some(def) = self.buff_def(buff_id) {
                let ids: Vec<u32> = def.activate_recipes.to_vec();
                if !ids.is_empty() { self.buff_deactivate_recipes(&ids); }
            }
        }
    }

    /// 消耗一层 Buff（归零则移除）
    pub fn remove_buff_stack(&mut self, buff_id: u32) {
        self.buff_generation += 1;
        self.bump_decision_gen();
        let will_remove = self.active_buffs.iter()
            .find(|b| b.buff_id == buff_id)
            .map_or(false, |b| b.stacks <= 1);
        if !will_remove {
            self.record_buff_event(buff_id, "consume");
        }
        let mut should_remove = false;
        if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == buff_id) {
            if inst.stacks > 1 { inst.stacks -= 1; } else { should_remove = true; }
        }
        if should_remove { self.remove_buff(buff_id); }
    }

    // ── 姿态（通过 Buff 管理）──

    /// 获取当前实际姿态
    pub fn stance(&self) -> Stance {
        if self.has_buff(BUFF_STANCE_WALL)   { Stance::Wall }
        else if self.has_buff(BUFF_STANCE_BLADE)  { Stance::Blade }
        else { Stance::Shield }
    }

    /// 获取预判姿态：如果 GCD 能覆盖盾飞延迟，预判为擎刀
    pub fn predicted_stance(&self) -> Stance {
        if let Some(inst) = self.active_buffs.iter().find(|b| b.buff_id == BUFF_DUN_FEI_DELAY) {
            if inst.expires_at > self.current_time {
                // GCD 覆盖延迟 → 预判擎刀（下个技能必定在延迟结束后释放）
                let gcd_covers = self.active_cds.iter()
                    .filter(|(k, _)| k.starts_with("gcd_"))
                    .any(|(_, &expires)| expires >= inst.expires_at);
                if gcd_covers {
                    return Stance::Blade;
                }
                return self.stance();
            }
        }
        self.stance()
    }

    /// 切换姿态：移除旧姿态 Buff，添加新姿态 Buff（永久）
    pub fn set_stance(&mut self, new: Stance) {
        self.remove_buff(BUFF_STANCE_SHIELD);
        self.remove_buff(BUFF_STANCE_BLADE);
        self.remove_buff(BUFF_STANCE_WALL);
        // 游戏原始姿态 buff（隐藏，宏判断用）
        self.remove_buff(BUFF_STANCE_SHIELD_GAME);
        self.remove_buff(BUFF_STANCE_BLADE_GAME);
        if let Some(bid) = stance_buff_id(new) {
            self.add_state_buff(bid, 0); // 0 = 永久
        }
        match new {
            Stance::Shield | Stance::Wall => self.add_state_buff(BUFF_STANCE_SHIELD_GAME, 0),
            Stance::Blade => self.add_state_buff(BUFF_STANCE_BLADE_GAME, 0),
            _ => {}
        }
    }

    // ── 充能 ──

    /// 获取技能当前充能层数（非充能技能返回 None）
    fn get_charges(&self, skill: &SkillSpec) -> Option<u32> {
        if skill.max_charges == 0 { return None; }
        let max_ch = self.effective_max_charges(skill);
        let cd = self.effective_charge_cd(skill);
        match self.charges.get(&skill.skill_id) {
            Some(&(ch, next_t)) => {
                let mut ch = ch;
                let mut t = next_t;
                while ch < max_ch && t <= self.current_time {
                    ch += 1;
                    t += cd;
                }
                Some(ch.min(max_ch))
            }
            None => Some(max_ch),
        }
    }

    /// 消耗一层充能，更新恢复计时
    fn consume_charge(&mut self, skill: &SkillSpec, cast_time: f64) {
        if skill.max_charges == 0 { return; }
        let max_ch = self.effective_max_charges(skill);
        let cd = self.effective_charge_cd(skill);
        let (mut ch, mut next_t) = self.charges.get(&skill.skill_id)
            .copied().unwrap_or((max_ch, 0.0));
        while ch < max_ch && next_t <= cast_time {
            ch += 1;
            next_t += cd;
        }
        if ch > 0 {
            let was_full = ch >= max_ch;
            ch -= 1;
            if was_full { next_t = cast_time + cd; }
            self.charges.insert(skill.skill_id, (ch, next_t));
        }
    }

    /// 打断引导：根据打断时刻计算实际完成的跳数，修正 last_channel_ticks
    /// interrupt_at: 打断发生的时刻（通常是下一个技能的释放时间）
    /// 返回 (原跳数, 实际跳数)
    pub fn interrupt_channel(&mut self, interrupt_at: f64) -> (u32, u32) {
        let old_ticks = self.last_channel_ticks;
        if old_ticks == 0 || self.channel_interval_frame == 0 {
            return (old_ticks, old_ticks);
        }
        let elapsed = interrupt_at - self.channel_cast_time;
        let actual_ticks = if elapsed <= 0.0 {
            0
        } else {
            let first_sec = frames_to_sec(self.channel_first_frame);
            if elapsed < first_sec - 0.001 {
                0
            } else {
                let after_first = elapsed - first_sec;
                let interval_sec = frames_to_sec(self.channel_interval_frame);
                1 + (after_first / interval_sec).floor() as u32
            }
        };
        let actual_ticks = actual_ticks.min(old_ticks);
        self.last_channel_ticks = actual_ticks;
        self.channel_end = self.current_time;
        self.bump_decision_gen();
        (old_ticks, actual_ticks)
    }

    /// 查询本版本的 BuffDef（按 self.version 路由到 v{版本}/buffs/defs.rs）
    pub fn buff_def(&self, buff_id: u32) -> Option<&'static BuffDef> {
        scripts::get_buff_def(self, buff_id)
    }

    /// 有效加速等级：基础加速等级（25% 封顶）+ buff 突破上限加速（折算为等级）
    /// 用于 get_actual_frames 的所有帧数缩减计算
    pub fn effective_haste_level(&self) -> u32 {
        let cap = (0.25 * LP_HASTE) as u32;
        let base = self.haste_level.min(cap);
        let t = self.current_time;
        let mut extra_pct = 0.0f64;
        for inst in &self.active_buffs {
            if inst.expires_at != 0.0 && inst.expires_at <= t { continue; }
            let def = match self.buff_def(inst.buff_id) { Some(d) => d, None => continue };
            for e in def.effects {
                if e.field == AttribField::UnlimitedAdditionalHastePercent {
                    extra_pct += e.value;
                }
            }
        }
        let extra_level = (extra_pct / 1024.0 * LP_HASTE) as u32;
        base + extra_level
    }

    /// 重置指定 CD（立即可用）
    pub fn reset_cd(&mut self, cd_id: &str) {
        self.active_cds.remove(cd_id);
        self.bump_decision_gen();
    }

    /// 减少指定 CD 的剩余时间（秒）
    pub fn reduce_cd(&mut self, cd_id: &str, seconds: f64) {
        if let Some(expires) = self.active_cds.get_mut(cd_id) {
            *expires -= seconds;
            if *expires <= self.current_time {
                self.active_cds.remove(cd_id);
            }
        }
        self.bump_decision_gen();
    }


    /// 减少指定技能的充能恢复时间（秒）
    pub fn reduce_charge_cd(&mut self, skill_id: u32, seconds: f64) {
        if let Some((_ch, next_t)) = self.charges.get_mut(&skill_id) {
            *next_t -= seconds;
        }
        self.bump_decision_gen();
    }

    /// 充能技能下一层恢复的剩余秒数（满层返回 0）
    fn charge_remaining(&self, skill: &SkillSpec) -> f64 {
        if skill.max_charges == 0 { return 0.0; }
        let max_ch = self.effective_max_charges(skill);
        let cd = self.effective_charge_cd(skill);
        match self.charges.get(&skill.skill_id) {
            Some(&(ch, next_t)) => {
                let mut ch = ch;
                let mut t = next_t;
                while ch < max_ch && t <= self.current_time {
                    ch += 1;
                    t += cd;
                }
                if ch >= max_ch { 0.0 }
                else { (t - self.current_time).max(0.0) }
            }
            None => 0.0,
        }
    }

    /// 充能技能的最早可用时间（有层数则立即，无层数则等下一层恢复）
    fn charge_ready_time(&self, skill: &SkillSpec) -> f64 {
        if skill.max_charges == 0 { return self.current_time; }
        let max_ch = self.effective_max_charges(skill);
        let cd = self.effective_charge_cd(skill);
        match self.charges.get(&skill.skill_id) {
            Some(&(ch, next_t)) => {
                let mut ch = ch;
                let mut t = next_t;
                while ch < max_ch && t <= self.current_time {
                    ch += 1;
                    t += cd;
                }
                if ch > 0 { self.current_time } else { t }
            }
            None => self.current_time,
        }
    }

    // ── 状态机：技能可施展判定 ──

    /// 计算奇穴/秘籍修正后的实际充能层数
    pub fn effective_max_charges(&self, skill: &SkillSpec) -> u32 {
        let mut charges = skill.max_charges;
        match skill.skill_id {
            13047 => {
                if self.has_talent(36058) {
                    if matches!(self.version, GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest) {
                        return 999; // 2026+: 无充能
                    }
                    charges += 1; // 2025: +1层
                }
            }
            _ => {}
        }
        charges
    }

    /// 计算奇穴/秘籍修正后的实际充能 CD
    pub fn effective_charge_cd(&self, skill: &SkillSpec) -> f64 {
        let mut cd = skill.charge_cd;
        match skill.skill_id {
            13047 => {
                if self.has_talent(36058) {
                    if matches!(self.version, GameVersion::AnYingQianJi | GameVersion::AnYingQianJiTest) {
                        return 0.001; // 2026+: 无CD
                    }
                    cd -= 1.0; // 2025: -1秒
                }
            }
            // 血怒：JJC 套 4 件套 atSetEquipmentRecipe 1929 — 充能 CD -3 秒
            13040 => {
                if self.count_equip_in(crate::equip_effects::CY_JJC_SET_IDS) >= 4 {
                    cd -= 3.0;
                }
            }
            _ => {}
        }
        cd.max(0.0)
    }

    /// 计算秘籍修正后的实际怒气消耗
    /// 13055 绝刀走 jue_dao 脚本按段计算；其他技能默认按 spec.rage_cost
    pub fn effective_rage_cost(&self, skill: &SkillSpec) -> u32 {
        match skill.skill_id {
            13055 => scripts::jue_dao_effective_rage_cost(self, skill),
            13391 => scripts::dun_dang_effective_rage_cost(self),
            13052 => { // 劫刀：秘籍1007 -5怒
                if self.has_recipe(1007) { skill.rage_cost.saturating_sub(5) }
                else { skill.rage_cost }
            }
            _ => skill.rage_cost,
        }
    }

    pub fn can_cast(&self, skill: &SkillSpec) -> bool {
        fn is_zhan_jue_allowed(id: u32) -> bool {
            matches!(id, 13052 | 13053 | 13054 | 13055 | 90001 | 90002)
        }
        // 移除气劲：需要有可移除的自身 buff
        if skill.skill_id == 90001 {
            let has_removable = self.active_buffs.iter().any(|b| {
                if b.expires_at != 0.0 && b.expires_at <= self.current_time { return false; }
                self.buff_def(b.buff_id).map_or(false, |d| !d.is_debuff)
            });
            if !has_removable { return false; }
        }
        // 奇穴检查
        if let Some(tid) = skill.requires_talent {
            if !self.has_talent(tid) { return false; }
        }
        // 姿态检查（含延迟预判）
        let cur = self.predicted_stance();
        let ok = match skill.stance {
            Stance::Any     => true,
            Stance::NotWall => cur != Stance::Wall,
            other           => other == cur,
        };
        if !ok { return false; }
        // 战绝：只能释放苍雪刀招式（斩/绝/劫/闪刀）+ 特殊技能（移除气劲/天下宏愿）
        if self.has_buff(BUFF_ZHAN_JUE) && !is_zhan_jue_allowed(skill.skill_id) {
            return false;
        }
        // 阵云结晦·雾海（90010）：需要长驱万里 ≥6 层
        if skill.skill_id == 90010 {
            let stacks = self.active_buffs.iter()
                .find(|b| b.buff_id == BUFF_CHANG_QU)
                .map(|b| b.stacks)
                .unwrap_or(0);
            if stacks < 6 { return false; }
        }
        // 格挡值检查（寒啸千军消耗 20 格挡值）
        if skill.skill_id == 15072 && self.block_value < 20 {
            return false;
        }
        // 怒气检查（含秘籍修正）
        if (self.rage as u32) < self.effective_rage_cost(skill) {
            return false;
        }
        // 连招检查（通过 Buff）
        if let Some(ref required) = skill.requires_combo {
            if !self.has_buff(combo_buff_id(required)) {
                return false;
            }
        }
        // 连招冲突检查：如果技能会授予的连招 buff 已存在，说明上次连招未消耗，不可重复释放
        if skill.requires_combo.is_none() {
            if let Some(ref grant) = skill.grants_combo {
                if self.has_buff(combo_buff_id(grant)) {
                    return false;
                }
            }
        }
        // 注意：技能 CD 不在此检查，由 next_cast_time 等待
        true
    }

    /// 返回技能不可释放的原因（可释放返回 None）
    pub fn reject_reason(&self, skill: &SkillSpec) -> Option<String> {
        if skill.skill_id == 90001 {
            let has = self.active_buffs.iter().any(|b| {
                if b.expires_at != 0.0 && b.expires_at <= self.current_time { return false; }
                self.buff_def(b.buff_id).map_or(false, |d| !d.is_debuff)
            });
            if !has { return Some("无可移除的气劲".into()); }
        }
        if let Some(tid) = skill.requires_talent {
            if !self.has_talent(tid) { return Some(format!("需要奇穴 {}", tid)); }
        }
        let cur = self.predicted_stance();
        let stance_ok = match skill.stance {
            Stance::Any     => true,
            Stance::NotWall => cur != Stance::Wall,
            other           => other == cur,
        };
        if !stance_ok {
            let need = match skill.stance {
                Stance::Shield => "擎盾", Stance::Blade => "擎刀",
                Stance::Wall => "盾墙", Stance::NotWall => "非盾墙", _ => "任意",
            };
            let have = match cur {
                Stance::Shield => "擎盾", Stance::Blade => "擎刀",
                Stance::Wall => "盾墙", _ => "未知",
            };
            return Some(format!("需要{}体态（当前{}）", need, have));
        }
        if self.has_buff(BUFF_ZHAN_JUE)
            && !matches!(skill.skill_id, 13052 | 13053 | 13054 | 13055 | 90001 | 90002)
        {
            return Some("战绝：仅可释放苍雪刀招式".into());
        }
        if skill.skill_id == 15072 && self.block_value < 20 {
            return Some(format!("格挡值不足（需要20，当前{}）", self.block_value));
        }
        let eff_cost = self.effective_rage_cost(skill);
        if (self.rage as u32) < eff_cost {
            return Some(format!("怒气不足（需要{}，当前{}，狂绝={}）", eff_cost, self.rage, self.has_buff(BUFF_KUANG_JUE)));
        }
        if let Some(ref req) = skill.requires_combo {
            if !self.has_buff(combo_buff_id(req)) {
                return Some("需要前置连招".into());
            }
        }
        if skill.requires_combo.is_none() {
            if let Some(ref grant) = skill.grants_combo {
                if self.has_buff(combo_buff_id(grant)) {
                    return Some("连招进行中，无法重复释放".into());
                }
            }
        }
        None
    }

    /// 从同名多品级中选出可施展的（优先连招后续段）
    pub fn pick_rank<'a>(&self, ranks: &[&'a SkillSpec]) -> Option<&'a SkillSpec> {
        // 优先连招后续段
        let combo = ranks.iter().rev()
            .find(|s| s.requires_combo.is_some() && self.can_cast(s));
        if let Some(s) = combo { return Some(s); }
        // 起手段（倒序选最高可用 rank，如绝刀按怒气选最高档）
        ranks.iter().rev()
            .find(|s| s.requires_combo.is_none() && self.can_cast(s))
            .copied()
    }

    // ── 施展后状态更新 ──

    pub fn apply_cast_effects(&mut self, skill: &SkillSpec) {
        // 怒气
        let eff_cost = self.effective_rage_cost(skill);
        self.last_rage_cost = eff_cost;
        self.set_rage(self.rage - eff_cost as i32 + skill.rage_gain as i32);
        // 肃驾（14840）：每消耗 11 怒回复 1 格挡值（铁骨衣专属）
        if eff_cost > 0 && self.mount == Mount::TieGuYi && self.has_talent(14840) {
            self.rage_spent_accumulator += eff_cost;
            let blocks = self.rage_spent_accumulator / 11;
            if blocks > 0 {
                let max_bv = self.max_block_value();
                self.add_block_value(blocks as i32);
                self.rage_spent_accumulator %= 11;
            }
        }
        // 消耗连招 Buff
        if let Some(ref req) = skill.requires_combo {
            self.remove_buff(combo_buff_id(req));
        }
        // 授予连招 Buff（带持续时间）
        if let Some(ref grant) = skill.grants_combo {
            let dur = skill.combo_duration.unwrap_or(COMBO_DURATION);
            self.add_state_buff(combo_buff_id(grant), dur);
        }
        // 姿态切换
        if let Some(new_stance) = skill.stance_change {
            self.set_stance(new_stance);
        }
    }

    /// 脚本请求：技能施展结束后推进指定时间（由 simulate 循环执行）
    pub fn request_advance(&mut self, seconds: f64) {
        self.pending_advance = seconds;
    }

    /// 消费 pending_advance，推进时间并处理区间内的 buff ticks
    pub fn flush_advance(&mut self) -> Vec<CastEvent> {
        if self.pending_advance <= 0.0 { return Vec::new(); }
        let from = self.current_time;
        let to = from + self.pending_advance;
        self.pending_advance = 0.0;
        self.current_time = to;
        self.process_buff_ticks(from, to)
    }

    // ── Buff Tick 处理 ──

    /// 处理从 from_time 到 to_time 之间所有 buff 的 tick 和到期事件
    /// tick/expire 行为通过脚本注册表调用
    pub fn process_buff_ticks(&mut self, from_time: f64, to_time: f64) -> Vec<CastEvent> {
        let _t0 = std::time::Instant::now();
        // 先消费 pending_team_buffs：把 release_at ∈ (from_time, to_time] 的项依次挂上
        // 队列已按 release_at 升序，依次 pop_front 即可
        while let Some(p) = self.pending_team_buffs.first().cloned() {
            if p.release_at > to_time { break; }
            self.pending_team_buffs.remove(0);
            let saved_time = self.current_time;
            self.current_time = p.release_at.max(0.0);
            if p.is_target {
                self.add_target_buff_with_stacks((p.buff_id, p.level), p.stacks, p.duration_frames);
            } else {
                self.add_buff_with_stacks((p.buff_id, p.level), p.stacks, p.duration_frames);
            }
            self.current_time = saved_time;
        }

        // 先收集需要处理的 tick 时间和到期 buff（避免借用冲突）
        let mut tick_schedule: Vec<(u32, f64)> = Vec::new(); // (buff_id, tick_time)
        let mut expired: Vec<(u32, f64)> = Vec::new();       // (buff_id, expire_time)

        let all_buffs: Vec<&BuffInstance> = self.active_buffs.iter()
            .chain(self.target_buffs.iter()).collect();
        for inst in all_buffs {
            let def = match self.buff_def(inst.buff_id) {
                Some(d) => d,
                None => continue,
            };

            // tick 事件（实例的 tick_interval_frames 在 add_buff 时从 def 复制；
            // set_buff_tick_interval(0) 可禁用 tick，不再 fallback 到 def）
            let tick_interval_frames = inst.tick_interval_frames;
            if tick_interval_frames > 0 && (inst.expires_at == 0.0 || inst.expires_at > from_time) {
                let tick_sec = frames_to_sec(tick_interval_frames);
                let buff_start = if inst.expires_at != 0.0 {
                    inst.expires_at - frames_to_sec(inst.duration_frames)
                } else { 0.0 };

                let mut t = buff_start + tick_sec;
                while t <= to_time {
                    if t > from_time && (inst.expires_at == 0.0 || t <= inst.expires_at) {
                        tick_schedule.push((inst.buff_id, t));
                    }
                    t += tick_sec;
                }
            }

            // 到期事件
            if inst.expires_at != 0.0 && inst.expires_at <= to_time && inst.expires_at > from_time {
                expired.push((inst.buff_id, inst.expires_at));
            }
        }

        let mut events = Vec::new();
        let mut em = ScriptEmitter::new();

        // 执行 tick 脚本 + 记录事件
        for (bid, t) in tick_schedule {
            self.current_time = t;
            if !self.lite_mode {
                if let Some(def) = self.buff_def(bid) {
                    if def.show_on_timeline {
                        let state = snapshot_event_state(self);
                        self.buff_events.entry(bid).or_default().push((t, "tick".into(), state));
                    }
                }
            }
            if let Some(script) = scripts::get_buff_on_tick(self, bid) {
                script(self, &mut em, t);
            }
        }

        // 执行 expire 脚本 + 记录事件
        for (bid, t) in &expired {
            self.current_time = *t;
            if !self.lite_mode {
                if let Some(def) = self.buff_def(*bid) {
                    if def.show_on_timeline {
                        let state = snapshot_event_state(self);
                        self.buff_events.entry(*bid).or_default().push((*t, "expire".into(), state));
                    }
                }
            }
            if let Some(script) = scripts::get_buff_on_expire(self, *bid) {
                script(self, &mut em, *t);
            }
        }

        // 移除到期 buff（自身 + 目标）— 不调用 remove_buff 以避免重复记录 remove 事件
        if !expired.is_empty() { self.buff_generation += 1; self.bump_decision_gen(); }
        let expired_ids: Vec<u32> = expired.iter().map(|(bid, _)| *bid).collect();
        for bid in &expired_ids {
            let was_self = self.active_buffs.iter().any(|b| b.buff_id == *bid);
            self.active_buffs.retain(|b| b.buff_id != *bid);
            self.target_buffs.retain(|b| b.buff_id != *bid || b.expires_at > to_time);
            // 自身 buff 到期：撤销其激活的秘籍
            if was_self {
                if let Some(def) = self.buff_def(*bid) {
                    let ids: Vec<u32> = def.activate_recipes.to_vec();
                    if !ids.is_empty() { self.buff_deactivate_recipes(&ids); }
                }
            }
        }

        events.extend(em.events);

        // Boss 周期受击：在 [from_time, to_time] 内触发受击事件
        while let Some(hit_time) = self.next_boss_attack {
            if hit_time > to_time { break; }
            if hit_time >= from_time {
                self.current_time = hit_time;
                let hit_events = scripts::on_player_hit(self, hit_time);
                events.extend(hit_events);
            }
            self.next_boss_attack = Some(hit_time + self.boss_attack_interval);
        }

        // 卷雪刀（平砍）：在区间 (last_swing_time, to_time] 内按当前加速产卡
        events.extend(scripts::juan_xue_process_swings(self, to_time));
        // 坚铁/寒甲 期望分布推进 + 同步合成 buff 层数
        self.advance_expectation();
        // 盾压 CD 期望重置
        self.advance_dunya_cd();
        let _ns = _t0.elapsed().as_nanos() as u64;
        crate::perf_add(|p| { p.buff_ticks_n += 1; p.buff_ticks_ns += _ns; });
        events
    }

    // ── 坚铁 / 寒甲 期望传播 ──────────────────────────────────────

    /// 推进期望分布到 `self.current_time` 对应的帧数，并把 E[A_寒甲]/E[B_寒甲]
    /// 同步到 `BUFF_HAN_JIA_SMALL/LARGE` 的 expected_stacks 字段。
    /// 仅当 `self.expectation = Some(_)` 时生效。
    pub fn advance_expectation(&mut self) {
        if self.expectation.is_none() { return; }
        let target_frame = (self.current_time * FRAMES_PER_SEC as f64).round() as u32;
        let from_frame = self.expectation.as_ref().unwrap().last_frame;
        if target_frame <= from_frame { return; }

        // p_0：当前面板招架率 **扣除坚铁自身贡献**（算法内部已用 p(k) = p_0 + 0.06×k）
        // cz：当前拆招值（含动态 buff 加成，如盾挡）
        let buff_slots = aggregate_buff_fields(self);
        let stats = build_runtime_stats(&self.base_attrs, &buff_slots, &self.constants);
        // 坚铁 buff 的 ParryValuePercent 贡献已被 aggregate 计入 stats.parry_rate，要减掉
        let jiantie_parry = self.active_buffs.iter()
            .find(|b| b.buff_id == BUFF_JIAN_TIE)
            .map_or(0.0, |b| {
                let mult = b.expected_stacks.unwrap_or(b.stacks as f64);
                mult * 600.0 / 10000.0   // 每层 6% = 600/10000
            });
        let p_0 = (stats.parry_rate - jiantie_parry).max(0.0);
        let cz  = stats.parry_value;

        let exp = self.expectation.as_mut().unwrap();
        let h = exp.h_per_frame;
        // 一段 advance 内 (p_0, cz) 视为常量；一段最长 ~10s = 160 帧，单段 < 1ms
        for _ in from_frame..target_frame {
            let jt = exp.jiantie.tick(p_0, h);
            let hj = exp.hanjia.tick(jt.q, cz);
            exp.last_jiantie = jt;
            exp.last_hanjia  = hj;
        }
        exp.last_frame = target_frame;

        // 同步合成寒甲层数到真实 buff
        self.sync_expectation_buffs();
    }

    // ── 盾压 CD 期望重置 ─────────────────────────────────────────

    /// 盾压 CD 期望重置：每帧推进小数 CD + 可用性信用。
    /// 当 avail_credit >= 1 时直接 reset_cd("cd_盾压") 并扣信用。
    pub fn advance_dunya_cd(&mut self) {
        if self.dunya_cd.is_none() { return; }

        let target_frame = (self.current_time * FRAMES_PER_SEC as f64).round() as u32;
        let from_frame = self.dunya_cd.as_ref().unwrap().last_frame;
        if target_frame <= from_frame { return; }
        let elapsed = target_frame - from_frame;

        const EPS: f64 = 1e-9;

        // 先取需要的外部状态（避免借用冲突）
        let need_reset_contrib = self.last_cast_shield_non_dunya;
        // 盾压重置概率公式：3.154X / (3.154X + 107553.6) + 3% + Δ招架率加成
        // 其中 X = 最终招架等级（含 buff），Δ招架率加成 = 坚铁等直接加成
        // 注意：与面板招架率公式 X/(X+LP) 不同，重置用 3.154X/(3.154X+LP)
        let reset_base_prob = if need_reset_contrib {
            let stats = self.current_stats();
            let x = stats.parry_level;
            const RESET_COEFF: f64 = 3.154;
            let base = RESET_COEFF * x / (RESET_COEFF * x + LP_PARRY);
            // Δ招架率加成 = 面板招架率 - 等级换算基础 - 心法基础
            let level_base = x / (x + LP_PARRY);
            let direct = stats.parry_rate - level_base - self.constants.parry_base_rate;
            base + self.constants.parry_base_rate + direct.max(0.0)
        } else { 0.0 };
        self.last_cast_shield_non_dunya = false;

        let dunya = self.dunya_cd.as_mut().unwrap();
        dunya.last_frame = target_frame;

        // (1) 非盾压盾系技能的随机重置贡献（先于自然冷却，作用在当前 cd_remain 上）
        if need_reset_contrib && dunya.cd_remain > 0.0 {
            let p_t = reset_base_prob + dunya.extra_reset_prob;
            let missing = 1.0 - dunya.avail_credit;
            dunya.avail_credit += p_t * missing;
            dunya.cd_remain *= 1.0 - p_t;
            if dunya.cd_remain <= EPS {
                dunya.cd_remain = 0.0;
                dunya.avail_credit = 1.0;
            }
        }

        // (2) 自然冷却：按经过帧数推进
        if dunya.cd_remain > 0.0 {
            dunya.cd_remain = (dunya.cd_remain - elapsed as f64).max(0.0);
            if dunya.cd_remain <= EPS {
                dunya.cd_remain = 0.0;
                dunya.avail_credit = (dunya.avail_credit + 1.0).min(1.0);
            }
        }

        // (3) 信用满 → 重置实际 CD
        if dunya.avail_credit >= 1.0 - EPS {
            dunya.avail_credit -= 1.0;
            if dunya.avail_credit < 0.0 { dunya.avail_credit = 0.0; }
            self.active_cds.remove("cd_盾压");
        }
    }

    /// 把当前 expectation.last_hanjia 的 E[A]/E[B] 写入 BUFF_HAN_JIA_SMALL/LARGE 的
    /// expected_stacks。若已存在则更新；若不存在则创建（永久 buff，由 expectation 管理）。
    ///
    /// 注：坚铁的分布/期望层数 **不**写到 BUFF_HAN_JIA 上（语义不同：寒甲 buff
    /// 没有"层数"概念，只是 12s 容器）。坚铁数据走专门的 expectation panel 显示。
    /// 同步坚铁/寒甲期望传播的合成 buff 到 active_buffs。
    /// - 坚铁 (8272)：仅选了坚铁奇穴 13138
    /// - 寒甲 (8437) + 寒甲·大/小：仅选了寒甲奇穴 13134
    ///
    /// **不变量维护**：本函数会直接 retain/push/mutate `active_buffs`（合成 buff 的特殊性使得
    /// 用 setter API 不便），因此末尾**必须** bump `buff_generation` 与 `decision_generation`
    /// 来失效 `buff_cache` / `buff_idx_cache` / `target_idx_cache`。
    /// `mutated` 在任何会改变 active_buffs 成员或 stacks/distribution/expected_stacks 时置 true。
    fn sync_expectation_buffs(&mut self) {
        let (e_a, e_b, p_alive, jt_e_stacks, jt_probs, hanjia_exp) = match self.expectation.as_ref() {
            Some(e) => (
                e.last_hanjia.e_a, e.last_hanjia.e_b, e.last_hanjia.p_alive,
                e.last_jiantie.e_stacks, e.last_jiantie.stack_probs,
                e.hanjia_expectation,
            ),
            None => return,
        };

        let argmax = jt_probs.iter().enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(0, |(k, _)| k as u32);

        let mut mutated = false;

        // ── 坚铁：正常 buff 逻辑 ──
        // 不存在 + argmax > 0 → add_buff（自带 BuffDef 的 8s 倒计时）
        // 存在 → 只更新 stacks / distribution，duration 自然倒计
        // argmax = 0 → 不创建，已有的自然过期
        if self.has_talent(13138) {
            let exists = self.active_buffs.iter().any(|b| b.buff_id == BUFF_JIAN_TIE);
            if !exists && argmax > 0 {
                self.add_buff(BUFF_JIAN_TIE);   // add_buff 内部已 bump
            }
            if let Some(inst) = self.active_buffs.iter_mut().find(|b| b.buff_id == BUFF_JIAN_TIE) {
                inst.stacks = argmax;
                inst.stack_distribution = Some(jt_probs.to_vec());
                inst.expected_stacks = if hanjia_exp { Some(jt_e_stacks) } else { None };
                mutated = true;   // stacks 改了 → aggregate cache 必须失效
            }
        } else {
            let len_before = self.active_buffs.len();
            self.active_buffs.retain(|b| b.buff_id != BUFF_JIAN_TIE);
            if self.active_buffs.len() != len_before { mutated = true; }
        }

        // ── 寒甲：仅期望传播模式 ──
        if self.has_talent(13134) && hanjia_exp {
            // 寒甲主 buff：存在就刷新持续时间
            if self.active_buffs.iter().any(|b| b.buff_id == BUFF_HAN_JIA) {
                self.add_buff(BUFF_HAN_JIA); // add_buff 内部已 bump
            }
            // 大/小：同坚铁逻辑，用 set_synthetic_layer
            if p_alive < 1e-6 {
                let len_before = self.active_buffs.len();
                self.active_buffs.retain(|b|
                    b.buff_id != BUFF_HAN_JIA_SMALL && b.buff_id != BUFF_HAN_JIA_LARGE);
                if self.active_buffs.len() != len_before { mutated = true; }
            } else {
                if Self::set_synthetic_layer(&mut self.active_buffs, BUFF_HAN_JIA_LARGE, e_a) { mutated = true; }
                if Self::set_synthetic_layer(&mut self.active_buffs, BUFF_HAN_JIA_SMALL, e_b) { mutated = true; }
            }
        }

        if mutated {
            self.buff_generation = self.buff_generation.wrapping_add(1);
            self.bump_decision_gen();
        }
    }

    /// 把 BUFF_X 设为合成"期望层数"buff（永久；expected_stacks=连续 E[k]）。
    /// 返回 true 表示 active_buffs 被修改（成员变化或 stacks/expected_stacks 改了）；
    /// 调用方据此决定是否 bump buff_generation。
    fn set_synthetic_layer(buffs: &mut Vec<BuffInstance>, buff_id: u32, e_stacks: f64) -> bool {
        let int_stacks = e_stacks.round().max(0.0) as u32;
        if let Some(inst) = buffs.iter_mut().find(|b| b.buff_id == buff_id) {
            // 仅当真有变化才返 true（避免无谓的 generation bump）
            let changed = inst.stacks != int_stacks
                || inst.expected_stacks.map_or(true, |old| (old - e_stacks).abs() > 1e-12);
            inst.expected_stacks = Some(e_stacks);
            inst.stacks = int_stacks;
            changed
        } else {
            buffs.push(BuffInstance {
                buff_id, stacks: int_stacks,
                duration_frames: 0,
                expires_at: 0.0,           // 永久（由 expectation 管理）
                tick_elapsed: 0,
                tick_interval_frames: 0,
                level: 0,
                snapshot: None,
                extra_effects: Vec::new(),
                expected_stacks: Some(e_stacks),
                stack_distribution: None,
            });
            true
        }
    }

    // ── GCD 时间推进 ──

    /// 预估技能释放时间（不修改状态，含偏移和延迟）
    pub fn estimate_cast_time(&self, skill: &SkillSpec, timing_offset: Option<f64>,
                              network_delay: f64) -> f64 {
        let base = self.next_cast_time(skill);
        match timing_offset {
            Some(off) if off > 0.001 => base + off,
            _ if skill_is_main(skill) && network_delay > 0.0 => base + network_delay,
            _ => base,
        }
    }

    pub fn next_cast_time(&self, skill: &SkillSpec) -> f64 {
        let mut earliest = f64::max(self.current_time, self.channel_end);
        // 需要擎刀姿态的技能：如果盾飞延迟切换还没完成，等它结束
        if skill.stance == Stance::Blade {
            if let Some(inst) = self.active_buffs.iter().find(|b| b.buff_id == BUFF_DUN_FEI_DELAY) {
                if inst.expires_at > earliest {
                    earliest = inst.expires_at;
                }
            }
        }
        // 充能技能：等充能恢复，不看技能 CD（充能替代了技能CD）
        if skill.max_charges > 0 {
            let charge_t = self.charge_ready_time(skill);
            if charge_t > earliest { earliest = charge_t; }
            // 仍需检查 GCD 和保护 CD
            for cd in &skill.cooldowns {
                if cd.cd_id.starts_with("cd_") { continue; } // 跳过技能 CD
                if let Some(&expires) = self.active_cds.get(&cd.cd_id) {
                    if expires > earliest { earliest = expires; }
                }
            }
        } else {
            // 非充能技能：检查所有 CD
            for cd in &skill.cooldowns {
                // 盾压 CD 特殊处理：有 DunyaCdState 时用期望剩余 CD 而非原始到期时间
                if cd.cd_id == "cd_盾压" {
                    if let Some(ref d) = self.dunya_cd {
                        let dunya_ready = self.current_time + d.cd_remain / FRAMES_PER_SEC as f64;
                        if dunya_ready > earliest { earliest = dunya_ready; }
                        continue;
                    }
                }
                if let Some(&expires) = self.active_cds.get(&cd.cd_id) {
                    if expires > earliest { earliest = expires; }
                }
            }
        }
        earliest
    }

    /// 释放技能：先检查释放条件，再 GCD 推进 + 状态机更新
    /// `network_delay`: 网络延迟（秒），仅对 is_main 且非首个技能生效
    /// 返回 (cast_time, cd_wait, ticks, max_ticks, ch_dur, applied_offset, max_offset) 或 None
    pub fn cast_skill(&mut self, skill: &SkillSpec, override_ticks: Option<u32>,
                      timing_offset: Option<f64>, max_offset: f64, network_delay: f64)
        -> Option<(f64, f64, Option<u32>, Option<u32>, Option<f64>, Option<f64>, Option<f64>)>
    {
        let _t0 = std::time::Instant::now();
        let _guard = crate::scopeguard_perf(
            |ns| crate::perf_add(|p| { p.cast_skill_n += 1; p.cast_skill_ns += ns; }),
            _t0
        );
        if !self.can_cast(skill) { return None; }

        // base_time：不考虑技能CD时最早能释放的时间点
        // = max(channel_end, 全局 GCD/保护 CD, 上次主动施放时刻)
        // 不混入 self.current_time，避免 buff 到期等中间事件把 current_time 推高导致 cd_wait 显示偏差
        // 用全局 active_cds 里的所有 gcd_/protect_ 类 CD（不限于当前技能声明）
        // last_cast_time 作为下限：无 GCD 的技能（如无惧）连续施放时以上次施放时刻为基线
        let mut base_time = self.channel_end.max(self.last_cast_time);
        for (cd_id, &expires) in &self.active_cds {
            if cd_id.starts_with("cd_") { continue; } // 跳过技能 CD
            if expires > base_time { base_time = expires; }
        }

        let earliest = self.next_cast_time(skill);
        let cd_wait = (earliest - base_time).max(0.0);

        // 非主GCD技能：应用释放偏移；主GCD技能：应用网络延迟
        let (cast_time, applied_offset, ret_max_offset) = if !skill_is_main(skill) && max_offset > 0.01 {
            let offset = timing_offset.unwrap_or(0.0).clamp(0.0, max_offset);
            (earliest + offset, Some(offset), Some(max_offset))
        } else if skill_is_main(skill) && network_delay > 0.0 {
            (earliest + network_delay, None, None)
        } else {
            (earliest, None, None)
        };

        // 充能技能：消耗一层（不触发技能 CD，由充能管理）
        if skill.max_charges > 0 {
            self.consume_charge(skill, cast_time);
        }

        // 触发 CD（充能技能跳过 cd_ 前缀）
        for cd in &skill.cooldowns {
            if cd.mode != CdMode::CheckAndTrigger { continue; }
            if skill.max_charges > 0 && cd.cd_id.starts_with("cd_") { continue; }
            let frames = sec_to_frames(cd.duration);
            let actual = if cd.haste {
                get_actual_frames(frames, self.effective_haste_level())
            } else { frames };
            self.active_cds.insert(cd.cd_id.clone(), cast_time + frames_to_sec(actual));
            // 盾压 CD 期望重置：同步 cd_remain 并重置信用
            if cd.cd_id == "cd_盾压" {
                if let Some(ref mut d) = self.dunya_cd {
                    d.cd_remain = actual as f64;
                    d.avail_credit = 0.0;
                    d.last_frame = (cast_time * FRAMES_PER_SEC as f64).round() as u32;
                }
            }
        }

        // 引导技能：计算跳数和实际引导时长
        let mut actual_ticks = None;
        let mut max_ticks = None;
        let mut ch_duration: Option<f64> = None;
        if let (Some(cf), Some(ci)) = (skill.channel_frame, skill.channel_interval) {
            // 秘籍加成引导时长
            let extra_channel = match skill.skill_id {
                13048 => { // 盾舞
                    self.has_recipe(8007) as u32 * 16   // +1秒=16帧
                  + self.has_recipe(8008) as u32 * 32   // +2秒=32帧
                }
                _ => 0,
            };
            let actual_frame = get_actual_frames(cf + extra_channel, self.effective_haste_level());
            let actual_interval = get_actual_frames(ci, self.effective_haste_level());
            let total = if actual_interval > 0 { actual_frame / actual_interval } else { 0 };
            max_ticks = Some(total);

            // 默认按 GCD 时长引导（GCD 内能完成的跳数），手动覆盖时按指定跳数
            let gcd_frames = skill.cooldowns.iter()
                .filter(|cd| cd.cd_id.starts_with("gcd_") && cd.mode == CdMode::CheckAndTrigger)
                .map(|cd| {
                    let f = sec_to_frames(cd.duration);
                    if cd.haste { get_actual_frames(f, self.effective_haste_level()) } else { f }
                })
                .max()
                .unwrap_or(0);
            let default_ticks = if gcd_frames > 0 && actual_interval > 0 {
                (gcd_frames / actual_interval).min(total).max(1)
            } else { total };

            let ticks = match override_ticks {
                Some(t) if t > 0 => t.min(total),
                _ => default_ticks,
            };
            actual_ticks = Some(ticks);

            // 引导时长 = 首跳延迟 + (ticks-1) * interval
            let first_frame = skill.first_tick_frame.unwrap_or(actual_interval);
            let actual_first = if first_frame == 0 { 0 }
                else if skill.first_tick_frame.is_some() {
                    get_actual_frames(first_frame, self.effective_haste_level())
                } else { actual_interval };
            let channel_dur = if ticks <= 1 {
                frames_to_sec(actual_first)
            } else {
                frames_to_sec(actual_first) + (ticks - 1) as f64 * frames_to_sec(actual_interval)
            };
            ch_duration = Some(channel_dur);
            self.channel_end = cast_time + channel_dur;
            self.channel_cast_time = cast_time;
            self.channel_first_frame = actual_first;
            self.channel_interval_frame = actual_interval;
            self.channel_skill_id = skill.skill_id;
        } else if let Some(cf) = skill.channel_frame {
            let actual = get_actual_frames(cf, self.effective_haste_level());
            self.channel_end = cast_time + frames_to_sec(actual);
        }

        self.current_time = cast_time;
        self.last_cast_time = cast_time;
        self.last_channel_ticks = actual_ticks.unwrap_or(0);
        self.apply_cast_effects(skill);

        // 神兵·无双气劲（橙武装备特效）：主动伤害招式命中后挂 buff
        // 参考盾挡范式（dun_dang.rs）：add_buff((id, level)) + bind_buff_effects 绑实例 effects
        // 条件：技能有伤害（attack_coeff 或 base_damage > 0）+ 主武器在橙武列表里
        let is_damage = skill.attack_coeff > 0.0 || skill.base_damage > 0.0;
        if is_damage {
            let weapon_id = self.equip_id_at("PRIMARY_WEAPON");
            if let Some((level, strain)) = shen_bing_wu_shuang_for(weapon_id) {
                self.add_buff((BUFF_SHEN_BING_WU_SHUANG, level));
                self.bind_buff_effects(BUFF_SHEN_BING_WU_SHUANG, vec![
                    EffectEntry { field: AttribField::StrainBase, value: strain },
                ]);
            }
        }

        // 装备 4 件套触发的常规 CD 缩减（atSetEquipmentRecipe 1929/1930/1976）
        // 仿奇穴范式：cast_skill 主路径插完 CD 后，按装备件数 reduce_cd
        match skill.skill_id {
            // 无惧：威望套 4 件套 → CD-2s
            13042 if self.count_equip_in(crate::equip_effects::CY_WEI_WANG_SET_IDS) >= 4 =>
                self.reduce_cd("cd_无惧", 2.0),
            // 盾壁：守护 T 套 4 件套 → CD-10s
            13070 if self.count_equip_in(crate::equip_effects::CY_GUARDIAN_T_SET_IDS) >= 4 =>
                self.reduce_cd("cd_盾壁", 10.0),
            _ => {}
        }

        // cast 边界 bump：cd / charge / channel_end / rage 全部可能变
        self.bump_decision_gen();
        Some((cast_time, cd_wait, actual_ticks, max_ticks, ch_duration, applied_offset, ret_max_offset))
    }

    /// 战斗结束时间 = 最后一次技能释放时间（由外部记录传入）
    /// 如有引导技能，取引导结束时间
    pub fn fight_end(&self, last_cast: f64) -> f64 {
        // 引导技能的引导时间需要算入战斗用时
        if self.channel_end > last_cast {
            self.channel_end
        } else {
            last_cast
        }
    }
}



// ─────────────────────────────────────────────────────────────────────────────
// 奇穴系统
// ─────────────────────────────────────────────────────────────────────────────

/// TOML 奇穴配置文件顶层
#[derive(Debug, Deserialize)]
struct TalentFileConfig {
    talents: Vec<TalentEntry>,
}

/// 单个奇穴条目（TOML 反序列化 + API 序列化）
///
/// 奇穴选择规则：
///   第 1-7 重：每重 4 选 1
///   第 8-10 重：混选池 12 选 3（tier = 8 表示混选池）
/// 战斗开始前锁定，运行期间不可更改。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TalentEntry {
    pub id: u32,
    pub name: String,
    /// 1-7 = 固定层，8 = 混选池（第八~十重）
    pub tier: u32,
    pub desc: String,
}

/// 从 TOML 文件加载奇穴列表
fn load_talents(path: &Path) -> Vec<TalentEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => { eprintln!("[talents] 读取 {:?} 失败: {e}", path); return Vec::new(); }
    };
    let config: TalentFileConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => { eprintln!("[talents] 解析 {:?} 失败: {e}", path); return Vec::new(); }
    };
    println!("[talents] 已加载 {} 个奇穴", config.talents.len());
    config.talents
}

// ─────────────────────────────────────────────────────────────────────────────
// 秘籍系统
// ─────────────────────────────────────────────────────────────────────────────

/// TOML 秘籍配置文件顶层
#[derive(Debug, Deserialize)]
struct RecipeFileConfig {
    recipes: Vec<RecipeEntry>,
}

/// 单个秘籍条目
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecipeEntry {
    pub id: u32,
    #[serde(default)]
    pub skill: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    /// 隐藏秘籍（不在 UI 显示，由 buff/脚本激活）
    #[serde(default)]
    pub hidden: bool,
    /// 限定技能 ID（非空时只对列表中的 skill_id 生效；空则按 `skill` 字段对应的技能名匹配）
    #[serde(default)]
    pub skill_filter: Vec<u32>,

    // ── 字段化加成（默认 0）──
    /// atRecipeDamagePercent —— 仅普通伤害
    #[serde(default)]
    pub damage_pct: f64,
    /// atRecipePhysicsCriticalPercent —— 全部伤害
    #[serde(default)]
    pub critical_pct: f64,
    /// atRecipeCriticalDamagePower —— 全部伤害
    #[serde(default)]
    pub crit_eff_pct: f64,
    /// atSurplusValueAddPercent —— 仅破招段
    #[serde(default)]
    pub surplus_pct: f64,
    /// atAllShieldIgnorePercent —— 全部伤害（按 1024 制传入，如 102 表示 ≈10%）
    #[serde(default)]
    pub shield_ignore: f64,
    /// 非侠士增伤增量（小数，1.50 = +150%）—— 仅对非侠士目标
    #[serde(default)]
    pub pve_addition: f64,
}

impl RecipeEntry {
    /// 该秘籍是否对指定 skill_id 生效
    pub fn applies_to(&self, skill_id: u32, skill_base_name: &str) -> bool {
        if !self.skill_filter.is_empty() {
            return self.skill_filter.contains(&skill_id);
        }
        // 兜底：用 skill 字段做名称匹配（旧秘籍）
        !self.skill.is_empty() && self.skill == skill_base_name
    }
}

/// 从 TOML 文件加载秘籍列表
fn load_recipes(path: &Path) -> Vec<RecipeEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => { eprintln!("[recipes] 读取 {:?} 失败: {e}", path); return Vec::new(); }
    };
    let config: RecipeFileConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => { eprintln!("[recipes] 解析 {:?} 失败: {e}", path); return Vec::new(); }
    };
    println!("[recipes] 已加载 {} 个秘籍", config.recipes.len());
    config.recipes
}

// ─────────────────────────────────────────────────────────────────────────────
// 团队增益（Team Buff）
// ─────────────────────────────────────────────────────────────────────────────

/// 团队增益条目（静态数据，从 team_buffs.toml 加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamBuffEntry {
    /// 唯一 key（如 "袖气" / "号令三军" / "战锋·悟"）
    pub key: String,
    /// 项目自定义 buff_id（指向 BuffDef）
    pub buff_id: u32,
    /// 分类（仅 UI 分组）
    pub category: String,
    /// 旗舰 / 无界 / 通用
    pub side: String,
    /// 是否目标 debuff（true → add_target_buff_with_stacks）
    #[serde(default)]
    pub is_target_buff: bool,
    /// 默认首次释放偏移（秒）
    #[serde(default)]
    pub default_first_release: f64,
    /// 默认周期（秒）；0 = 永久挂
    #[serde(default)]
    pub default_period: f64,
    /// 默认单次持续（秒）；period=0 时无意义
    #[serde(default)]
    pub default_duration: f64,
    /// 默认层数
    #[serde(default = "default_one_u32")]
    pub default_stacks: u32,
    /// 最大层数（UI 用）
    #[serde(default = "default_one_u32")]
    pub max_stacks: u32,
    /// 图标 URL
    #[serde(default)]
    pub icon: String,
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 来源（"七秀" / "铁骨衣" 等）
    #[serde(default)]
    pub source: String,
    /// 冲突的 key 列表（UI 互斥）
    #[serde(default)]
    pub conflicts: Vec<String>,
}

fn default_one_u32() -> u32 { 1 }

#[derive(Debug, Deserialize)]
struct TeamBuffFileConfig {
    entries: Vec<TeamBuffEntry>,
}

/// 用户在 UI 配置的单条团队增益启用状态（前端传后端）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TeamBuffSelection {
    pub key: String,
    pub enabled: bool,
    #[serde(default = "default_one_u32")]
    pub stacks: u32,
    #[serde(default)]
    pub first_release: f64,
    #[serde(default)]
    pub period: f64,
    #[serde(default)]
    pub duration: f64,
    /// 用户在时间轴独立调过的每次释放绝对时间（秒）；Some 且非空时优先于 first_release+period 等距推算
    #[serde(default)]
    pub release_times: Option<Vec<f64>>,
}

fn team_buffs_file(version: GameVersion) -> String {
    format!("{}/team_buffs.toml", version_root(version))
}

fn load_team_buffs(path: &Path) -> Vec<TeamBuffEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => { eprintln!("[team_buffs] 读取 {:?} 失败: {e}", path); return Vec::new(); }
    };
    let config: TeamBuffFileConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => { eprintln!("[team_buffs] 解析 {:?} 失败: {e}", path); return Vec::new(); }
    };
    println!("[team_buffs] 已加载 {} 个团队增益条目", config.entries.len());
    config.entries
}

/// 把 TeamBuffSelection 列表排程到 player（用于 simulate_core，按时间挂）
/// total_duration: 模拟时长（秒），决定周期型 buff 排程到何时
pub fn apply_team_buffs_for_simulate(player: &mut Player, sels: &[TeamBuffSelection],
                                       table: &[TeamBuffEntry], total_duration: f64) {
    for sel in sels {
        if !sel.enabled { continue; }
        let entry = match table.iter().find(|e| e.key == sel.key) {
            Some(e) => e, None => continue,
        };
        let dur_frames = if sel.duration > 0.0 { sec_to_frames(sel.duration) } else { 0 };
        // 优先：用户在时间轴独立调过的每次释放时间数组
        if let Some(times) = sel.release_times.as_ref().filter(|t| !t.is_empty()) {
            if sel.duration <= 0.0 { continue; }
            for &t in times {
                if t < 0.0 || t >= total_duration { continue; }
                player.schedule_team_buff(PendingTeamBuff {
                    release_at: t,
                    buff_id: entry.buff_id,
                    level: 0,
                    stacks: sel.stacks,
                    duration_frames: dur_frames,
                    is_target: entry.is_target_buff,
                });
            }
            continue;
        }
        if sel.period <= 0.0 {
            // 永久挂（period=0）：t=0 单次挂，duration 由 entry / 用户给的覆盖（0=永久）
            player.schedule_team_buff(PendingTeamBuff {
                release_at: 0.0,
                buff_id: entry.buff_id,
                level: 0,
                stacks: sel.stacks,
                duration_frames: dur_frames,
                is_target: entry.is_target_buff,
            });
        } else {
            // 周期：first_release + N×period 排程，直到 total_duration
            let mut t = sel.first_release.max(0.0);
            // 防御：周期或持续时间为 0 时不排
            if sel.period <= 0.0 || sel.duration <= 0.0 { continue; }
            // 上限：3600s + period=1s 极端配置下 3600 项（保护，避免 period 微小时死循环）
            let max_count = 4096;
            let mut count = 0;
            while t < total_duration && count < max_count {
                player.schedule_team_buff(PendingTeamBuff {
                    release_at: t,
                    buff_id: entry.buff_id,
                    level: 0,
                    stacks: sel.stacks,
                    duration_frames: dur_frames,
                    is_target: entry.is_target_buff,
                });
                t += sel.period;
                count += 1;
            }
        }
    }
}

/// 把 TeamBuffSelection 列表应用到 player（用于 /api/skill_damage 和 /api/calculate）
/// 没有时间维度，永久型直接挂；周期型按平均覆盖率折算 stacks 永久挂
pub fn apply_team_buffs_for_static(player: &mut Player, sels: &[TeamBuffSelection],
                                     table: &[TeamBuffEntry]) {
    for sel in sels {
        if !sel.enabled { continue; }
        let entry = match table.iter().find(|e| e.key == sel.key) {
            Some(e) => e, None => continue,
        };
        // 优先用 release_times 估算覆盖率（按 N 次平均 period 折算）
        let stacks = if let Some(times) = sel.release_times.as_ref().filter(|t| t.len() >= 2 && sel.duration > 0.0) {
            let lo = times.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let avg_period = ((hi - lo) / (times.len() as f64 - 1.0)).max(0.1);
            let coverage = (sel.duration / avg_period).clamp(0.0, 1.0);
            ((sel.stacks as f64) * coverage).round() as u32
        } else if sel.period > 0.0 && sel.duration > 0.0 {
            let coverage = (sel.duration / sel.period).clamp(0.0, 1.0);
            ((sel.stacks as f64) * coverage).round() as u32
        } else {
            sel.stacks
        };
        if stacks == 0 { continue; }
        if entry.is_target_buff {
            player.add_target_buff_with_stacks((entry.buff_id, 0u32), stacks, 0);
        } else {
            player.add_buff_with_stacks((entry.buff_id, 0u32), stacks, 0);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 阵法（Formation）— 自己开阵走拟真触发，他人开阵走 jx3dps 覆盖率折算后的永久数值
// ─────────────────────────────────────────────────────────────────────────────

/// 阵法 entry（静态数据，从 formations.toml 加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormationEntry {
    /// 唯一 key（如 "苍云阵_self" / "霸刀阵_other"）
    pub key: String,
    /// 同阵法多变体共享 ID（前端互斥单选用，如 "cangyun"）
    pub formation_id: String,
    /// 显示名（如 "苍云阵"）
    pub display_name: String,
    /// 阵法全名（如 "锋凌横绝阵"）
    #[serde(default)]
    pub formation_full_name: String,
    /// 游戏内释放 buff_id（UI 图标用，0 = 无）
    #[serde(default)]
    pub buff_id: u32,
    /// 心法所属（""=通用阵；非空时与 current_school 比较决定 self/other）
    #[serde(default)]
    pub owner_school: String,
    /// "any" / "self" / "other"
    pub applicable_when: String,
    /// "外功攻击阵" / "T 心法阵" / "通用阵"
    pub category: String,
    /// 图标 URL
    #[serde(default)]
    pub icon: String,
    /// 永久 effects（self/any 用）— Vec<(AttribField 名, 值)>
    /// 值的单位与 AttribField 注释一致（如 PhysicsAttackPowerPercent 是 N/1024 制）
    #[serde(default)]
    pub permanent_effects: Vec<(String, f64)>,
    /// 他人开"覆盖率折算后的永久数值"（other 用，含 jx3dps 把触发型平均掉的部分）
    #[serde(default)]
    pub public_effects_other: Vec<(String, f64)>,
    /// 描述
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct FormationFileConfig {
    #[serde(default)]
    formations: Vec<FormationEntry>,
}

/// 用户选中的阵法（前端传后端）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormationSelection {
    pub key: String,
}

fn formations_file(version: GameVersion) -> String {
    format!("{}/formations.toml", version_root(version))
}

fn load_formations(path: &Path) -> Vec<FormationEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => { eprintln!("[formations] 读取 {:?} 失败: {e}", path); return Vec::new(); }
    };
    let config: FormationFileConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => { eprintln!("[formations] 解析 {:?} 失败: {e}", path); return Vec::new(); }
    };
    println!("[formations] 已加载 {} 个阵法条目", config.formations.len());
    config.formations
}

/// 阵法 effects 字符串名 → AttribField
fn parse_attrib_field_for_formation(name: &str) -> Option<AttribField> {
    use AttribField::*;
    Some(match name {
        "PhysicsAttackPowerBase" => PhysicsAttackPowerBase,
        "PhysicsAttackPowerPercent" => PhysicsAttackPowerPercent,
        "PhysicsCriticalStrike" => PhysicsCriticalStrike,
        "PhysicsCriticalStrikePercent" => PhysicsCriticalStrikePercent,
        "PhysicsCriticalDamagePowerBase" => PhysicsCriticalDamagePowerBase,
        "PhysicsCriticalDamagePowerPercent" => PhysicsCriticalDamagePowerPercent,
        "PhysicsOvercomeBase" => PhysicsOvercomeBase,
        "PhysicsOvercomePercent" => PhysicsOvercomePercent,
        "StrainBase" => StrainBase,
        "StrainBasePercentAdd" => StrainBasePercentAdd,
        "StrainPercent" => StrainPercent,
        "SurplusValueBase" => SurplusValueBase,
        "SurplusPercent" => SurplusPercent,
        "AllDamageAddPercent" => AllDamageAddPercent,
        "AllShieldIgnorePercent" => AllShieldIgnorePercent,
        "WeaponDamageBase" => WeaponDamageBase,
        "VitalityBase" => VitalityBase,
        "VitalityBasePercentAdd" => VitalityBasePercentAdd,
        "ParryValueBase" => ParryValueBase,
        "ParryValuePercent" => ParryValuePercent,
        "ParryBase" => ParryBase,
        "HasteBase" => HasteBase,
        _ => return None,
    })
}

/// 选定阵法 → 永久 effects（按 self/any/other 选不同字段聚合到 AttribSlots）
pub fn formation_permanent_effects(
    sel: &Option<FormationSelection>,
    table: &[FormationEntry],
) -> AttribSlots {
    let mut slots: AttribSlots = HashMap::new();
    let Some(sel) = sel.as_ref() else { return slots; };
    let entry = match table.iter().find(|e| e.key == sel.key) {
        Some(e) => e, None => return slots,
    };
    let effects = match entry.applicable_when.as_str() {
        "self" | "any" => &entry.permanent_effects,
        "other" => &entry.public_effects_other,
        _ => return slots,
    };
    for (field_name, value) in effects {
        if let Some(f) = parse_attrib_field_for_formation(field_name) {
            *slots.entry(f).or_insert(0.0) += *value;
        } else {
            eprintln!("[formations] 未知 AttribField: {} (entry={})", field_name, entry.key);
        }
    }
    slots
}

/// 当前选中阵法是否"自己开"（self 变体且匹配 formation_id）
/// 触发挂载点（jue_dao.rs / equip_effects / on_hit）查这个，不依赖 SharedState
pub fn formation_is_self(player: &Player, formation_id: &str) -> bool {
    player.formation_self_id.as_deref() == Some(formation_id)
}

/// simulate_core 入口预算 player.formation_self_id：
/// - self 变体 → Some(formation_id)
/// - any / other / 未选 → None
pub fn resolve_formation_self_id(
    sel: &Option<FormationSelection>,
    table: &[FormationEntry],
) -> Option<String> {
    let sel = sel.as_ref()?;
    let entry = table.iter().find(|e| e.key == sel.key)?;
    if entry.applicable_when == "self" {
        Some(entry.formation_id.clone())
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 技能脚本（释放技能后的被动触发逻辑）
// ─────────────────────────────────────────────────────────────────────────────

/// 计算技能触发的最大 GCD（check_and_trigger 的最大 duration）
fn skill_gcd(skill: &SkillSpec) -> f64 {
    skill.cooldowns.iter()
        .filter(|cd| cd.cd_id.starts_with("gcd_") && cd.mode == CdMode::CheckAndTrigger)
        .map(|cd| cd.duration)
        .fold(0.0_f64, f64::max)
}

/// 判断技能是否占主 GCD 位：触发了 gcd_1.0（check_and_trigger）
/// 仅触发 gcd_1.5 的技能（业火、盾挡等）可插在 1s GCD 间隙，不算主技能
fn skill_is_main(skill: &SkillSpec) -> bool {
    skill.cooldowns.iter().any(|cd| {
        cd.mode == CdMode::CheckAndTrigger && cd.cd_id == "gcd_1.0"
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 技能效果脚本系统（独立模块）
// ─────────────────────────────────────────────────────────────────────────────

/// 事件收集器：脚本通过它输出被动触发事件
pub struct ScriptEmitter {
    pub events: Vec<CastEvent>,
    /// 主事件重写：脚本调用 override_primary 后，主事件名称和伤害改按子技能规格计算
    pub primary_override: Option<(String, u32)>,
}

impl ScriptEmitter {
    pub fn new() -> Self { ScriptEmitter { events: Vec::new(), primary_override: None } }

    /// 将主事件重定向到子技能（名称 + skill_id 用于重算伤害）
    pub fn override_primary(&mut self, name: &str, skill_id: u32) {
        self.primary_override = Some((name.to_string(), skill_id));
    }

    pub fn emit(&mut self, name: &str, skill_id: u32, cast_time: f64) {
        self.events.push(CastEvent {
            name: name.into(), skill_id, cast_time,
            triggered: true, gcd: 0.0, is_main: false, cd_wait: 0.0,
            channel_ticks: None, max_channel_ticks: None, channel_duration: None,
            timing_offset: None, max_timing_offset: None, available_buffs: None,
            is_macro: false, rage_after: None, rage_delta: None, rage_cost: None,
            state_before: None, state_after: None,
            damage: None, damage_normal: None, damage_crit: None, damage_total: None,
            runtime_recipes: Vec::new(),
            runtime_stats: None,
            override_attack_coeff: None,
            applied_recipes: Vec::new(),
        });
    }

    /// emit 一个带跳数的事件（破招段跟随主体引导跳数）
    pub fn emit_with_ticks(&mut self, name: &str, skill_id: u32, cast_time: f64, ticks: u32) {
        let mut ev = CastEvent {
            name: name.into(), skill_id, cast_time,
            triggered: true, gcd: 0.0, is_main: false, cd_wait: 0.0,
            channel_ticks: Some(ticks), max_channel_ticks: Some(ticks), channel_duration: None,
            timing_offset: None, max_timing_offset: None, available_buffs: None,
            is_macro: false, rage_after: None, rage_delta: None, rage_cost: None,
            state_before: None, state_after: None,
            damage: None, damage_normal: None, damage_crit: None, damage_total: None,
            runtime_recipes: Vec::new(),
            runtime_stats: None,
            override_attack_coeff: None,
            applied_recipes: Vec::new(),
        };
        if ticks <= 1 {
            ev.channel_ticks = None;
            ev.max_channel_ticks = None;
        }
        self.events.push(ev);
    }

    /// emit 时覆盖 attack_coeff（绝国按层数动态变化）
    pub fn emit_with_coeff(&mut self, name: &str, skill_id: u32, cast_time: f64, attack_coeff: f64) {
        self.events.push(CastEvent {
            name: name.into(), skill_id, cast_time,
            triggered: true, gcd: 0.0, is_main: false, cd_wait: 0.0,
            channel_ticks: None, max_channel_ticks: None, channel_duration: None,
            timing_offset: None, max_timing_offset: None, available_buffs: None,
            is_macro: false, rage_after: None, rage_delta: None, rage_cost: None,
            state_before: None, state_after: None,
            damage: None, damage_normal: None, damage_crit: None, damage_total: None,
            runtime_recipes: Vec::new(),
            runtime_stats: None,
            override_attack_coeff: Some(attack_coeff),
            applied_recipes: Vec::new(),
        });
    }

    /// emit 时附带运行时秘籍（如血誓怒气段）
    pub fn emit_with_recipes(&mut self, name: &str, skill_id: u32, cast_time: f64, recipes: Vec<u32>) {
        self.events.push(CastEvent {
            name: name.into(), skill_id, cast_time,
            triggered: true, gcd: 0.0, is_main: false, cd_wait: 0.0,
            channel_ticks: None, max_channel_ticks: None, channel_duration: None,
            timing_offset: None, max_timing_offset: None, available_buffs: None,
            is_macro: false, rage_after: None, rage_delta: None, rage_cost: None,
            state_before: None, state_after: None,
            damage: None, damage_normal: None, damage_crit: None, damage_total: None,
            runtime_recipes: recipes,
            runtime_stats: None,
            override_attack_coeff: None,
            applied_recipes: Vec::new(),
        });
    }
}


fn snapshot_buff_list(list: &[BuffInstance], current_time: f64, is_target: bool, version: GameVersion) -> Vec<BuffSnapshot> {
    list.iter().filter_map(|inst| {
        if inst.expires_at != 0.0 && inst.expires_at <= current_time { return None; }
        let def = scripts::get_buff_def_by_version(version, inst.buff_id)?;
        let remaining = if inst.expires_at == 0.0 { 0.0 }
            else { (inst.expires_at - current_time).max(0.0) };
        Some(BuffSnapshot {
            buff_id:       inst.buff_id,
            name:          def.name.to_string(),
            description:   def.description.to_string(),
            stacks:        inst.stacks,
            max_stacks:    def.max_stacks,
            remaining_sec: remaining,
            duration_sec:  frames_to_sec(inst.duration_frames),
            is_debuff:     def.is_debuff,
            is_target:     is_target,
            level:         inst.level,
            expected_stacks:    inst.expected_stacks,
            stack_distribution: inst.stack_distribution.clone(),
            icon:          def.icon.to_string(),
        })
    }).collect()
}

/// 时间轴指纹：对一组 timeline 事件计算确定性 64-bit hash。
/// **必须确定性**——ahash::AHasher::default() 每进程随机 seed，重启就变，不能用！
/// 用 FNV-1a（手写，10 行，0 依赖，绝对确定性）。
#[inline]
fn fnv1a_u64(state: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *state ^= b as u64;
        *state = state.wrapping_mul(0x100000001b3);  // FNV prime
    }
}
pub fn compute_fingerprint(timeline: &[CastEvent]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;  // FNV offset basis
    for ev in timeline {
        fnv1a_u64(&mut h, &ev.cast_time.to_bits().to_le_bytes());
        fnv1a_u64(&mut h, &ev.skill_id.to_le_bytes());
        match ev.damage_total {
            Some(d) => { fnv1a_u64(&mut h, &[1]); fnv1a_u64(&mut h, &d.to_bits().to_le_bytes()); }
            None    => { fnv1a_u64(&mut h, &[0]); }
        }
        fnv1a_u64(&mut h, &[ev.triggered as u8, ev.is_main as u8]);
        match ev.channel_ticks {
            Some(t) => { fnv1a_u64(&mut h, &[1]); fnv1a_u64(&mut h, &t.to_le_bytes()); }
            None    => { fnv1a_u64(&mut h, &[0]); }
        }
    }
    h
}

// ─── perf 计时（线程局部计数器；按 simulate_core 边界 reset/dump）───
#[derive(Default, Clone, Copy)]
pub struct SimPerf {
    pub snapshot_n: u32,        pub snapshot_ns: u64,
    pub run_scripts_n: u32,     pub run_scripts_ns: u64,
    pub buff_ticks_n: u32,      pub buff_ticks_ns: u64,
    pub collect_recipes_n: u32, pub collect_recipes_ns: u64,
    pub aggregate_n: u32,       pub aggregate_ns: u64,
    pub cache_hit: u32,         pub cache_miss: u32,
    pub fill_tick_n: u32,       pub fill_tick_ns: u64,
    pub fill_event_n: u32,      pub fill_event_ns: u64,
    pub calc_damage_n: u32,     pub calc_damage_ns: u64,
    pub macro_eval_n: u32,      pub macro_eval_ns: u64,    // simulate_macro 整次调用
    pub macro_cond_n: u32,      pub macro_cond_ns: u64,    // 条件求值
    pub cast_skill_n: u32,      pub cast_skill_ns: u64,    // Player::cast_skill
    // macro_eval 内部细分（找 105ms 隐藏开销）
    pub macro_phase1_n: u32,    pub macro_phase1_ns: u64,  // build skill pool（规则迭代+条件检查+ skill 查找）
    pub macro_phase2_n: u32,    pub macro_phase2_ns: u64,  // try cast loop（对候选 try cast_skill）
    pub macro_skill_lookup_n: u32, pub macro_skill_lookup_ns: u64, // skill_map 字符串 hash 查找
    pub macro_advance_n: u32,   pub macro_advance_ns: u64, // 时间推进 + buff_tick + flush_advance + Vec extend
    // macro_advance 内部三段细分
    pub macro_adv_next_n: u32,  pub macro_adv_next_ns: u64,  // next_decision_time
    pub macro_adv_ticks_n: u32, pub macro_adv_ticks_ns: u64, // process_buff_ticks（不含 fill）
    pub macro_adv_fill_n: u32,  pub macro_adv_fill_ns: u64,  // fill_tick_events
}
thread_local! {
    pub static SIM_PERF: std::cell::RefCell<SimPerf> = std::cell::RefCell::new(SimPerf::default());
}
#[inline]
pub fn perf_add(f: impl FnOnce(&mut SimPerf)) {
    SIM_PERF.with(|p| f(&mut *p.borrow_mut()));
}
/// RAII guard: drop 时把 (now - start) 喂给闭包；适合 early-return 场景
pub struct PerfGuard<F: FnMut(u64)> {
    start: std::time::Instant,
    cb: F,
}
impl<F: FnMut(u64)> Drop for PerfGuard<F> {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos() as u64;
        (self.cb)(ns);
    }
}
pub fn scopeguard_perf<F: FnMut(u64)>(cb: F, start: std::time::Instant) -> PerfGuard<F> {
    PerfGuard { start, cb }
}

/// 合并自身 + 目标 buff 快照
fn snapshot_event_state(player: &Player) -> EventState {
    let _t0 = std::time::Instant::now();
    let t = player.current_time;
    let buffs: Vec<EventBuff> = player.active_buffs.iter()
        .filter(|b| b.expires_at == 0.0 || b.expires_at > t)
        .filter_map(|b| {
            let def = player.buff_def(b.buff_id)?;
            if def.is_debuff { return None; } // 跳过 debuff
            Some(EventBuff {
                buff_id: b.buff_id,
                name: def.name.to_string(),
                remaining: if b.expires_at == 0.0 { 0.0 } else { (b.expires_at - t).max(0.0) },
                stacks: b.stacks,
                icon: def.icon.to_string(),
            })
        }).collect();
    let target_buffs: Vec<EventBuff> = player.target_buffs.iter()
        .filter(|b| b.expires_at == 0.0 || b.expires_at > t)
        .filter_map(|b| {
            let def = player.buff_def(b.buff_id)?;
            Some(EventBuff {
                buff_id: b.buff_id,
                name: def.name.to_string(),
                remaining: if b.expires_at == 0.0 { 0.0 } else { (b.expires_at - t).max(0.0) },
                stacks: b.stacks,
                icon: def.icon.to_string(),
            })
        }).collect();
    let mut skill_cds: Vec<EventSkillCd> = player.active_cds.iter()
        .filter(|(k, &v)| !k.starts_with("gcd_") && !k.starts_with("protect_") && v > t + 0.01)
        .map(|(k, &v)| {
            let name = k.strip_prefix("cd_").unwrap_or(k).to_string();
            EventSkillCd { name, remaining: (v - t).max(0.0) }
        })
        .collect();
    // 充能技能：显示当前层数和下一层恢复时间（经过时间推进）
    let charge_info: &[(u32, &str, u32, f64)] = &[
        // (skill_id, name, max_charges, charge_cd)
        (13047, "盾击", 3, 3.0),
        (13040, "血怒", 3, 30.0),
        (13050, "盾飞", 3, 15.0),
        (30769, "阵云结晦", 2, 30.0),
    ];
    for &(sid, name, max_ch, cd) in charge_info {
        if let Some(&(raw_ch, raw_next)) = player.charges.get(&sid) {
            // 时间推进：和 get_charges/charge_remaining 相同逻辑
            let mut ch = raw_ch;
            let mut next = raw_next;
            while ch < max_ch && next <= t { ch += 1; next += cd; }
            let ch = ch.min(max_ch);
            if ch < max_ch {
                let remaining = (next - t).max(0.0);
                skill_cds.push(EventSkillCd {
                    name: format!("{}({}层)", name, ch),
                    remaining,
                });
            }
        }
    }
    let block_value = if player.mount == Mount::TieGuYi { Some(player.block_value) } else { None };
    let _ns = _t0.elapsed().as_nanos() as u64;
    perf_add(|p| { p.snapshot_n += 1; p.snapshot_ns += _ns; });
    EventState { rage: player.rage, block_value, stance: player.stance(), buffs, target_buffs, skill_cds }
}

fn snapshot_buffs(player: &Player) -> Vec<BuffSnapshot> {
    let mut result = snapshot_buff_list(&player.active_buffs, player.current_time, false, player.version);
    result.extend(snapshot_buff_list(&player.target_buffs, player.current_time, true, player.version));
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// 路由处理
// ─────────────────────────────────────────────────────────────────────────────

/// 全局共享状态
#[derive(Clone)]
pub struct SharedState {
    /// Agent 读取运行时数据时持有读锁；心法切换/技能重载持有写锁，保证快照原子一致。
    pub agent_context_gate: Arc<RwLock<()>>,
    /// 当前版本/心法运行时数据的稳定 provenance；只在启动或显式重载时计算。
    pub agent_provenance: Arc<RwLock<agent::ToolProvenance>>,
    /// 服务端定义的模型供应商目录；安全摘要可见，凭据只在创建 adapter 时从环境变量解析。
    pub agent_providers: Arc<agent::provider::ProviderCatalog>,
    /// 当前 worker 的瞬态 Agent run；每个 worker 同时只允许一个活动 run。
    pub agent_runs: Arc<agent::run::AgentRunManager>,
    /// 当前 worker 用户目录内的 append-only Agent 会话。
    pub agent_sessions: Arc<agent::session::AgentSessionStore>,
    /// 启动时构建的只读、版本感知本地知识索引；未配置时 Agent 保持原有模拟能力。
    pub agent_knowledge: Option<Arc<agent::KnowledgeIndex>>,
    /// 当前选中的武学版本（初始 V2025_10 山海源流）
    pub version: Arc<RwLock<GameVersion>>,
    /// 当前选中的心法（初始 分山劲）
    pub mount: Arc<RwLock<Mount>>,
    /// 当前心法常量（从 school.toml 加载）
    pub constants: Arc<RwLock<MountConstants>>,
    /// 当前心法固定增益（穿戴心法本身就有的属性，从 school.toml 的 [base_stats] 加载）
    pub base_stats: Arc<RwLock<equip::MountBaseStats>>,
    /// 当前心法转化（主属性 → 副属性，作用在最终主属性，从 [mount_conversions] 加载）
    pub mount_conversions: Arc<RwLock<equip::MountConversions>>,
    /// 当前心法 UI 配置（技能栏布局 / 连招替换）
    pub school_ui: Arc<RwLock<SchoolUi>>,
    /// 当前心法 A 工作流参数
    pub workflow_a: Arc<RwLock<WorkflowA>>,
    pub skills: Arc<RwLock<Vec<SkillSpec>>>,
    pub talents: Arc<RwLock<Vec<TalentEntry>>>,
    pub recipes: Arc<RwLock<Vec<RecipeEntry>>>,
    /// 团队增益条目表（按版本加载；版本切换时 reload）
    pub team_buffs: Arc<RwLock<Vec<TeamBuffEntry>>>,
    /// 阵法条目表（按版本加载；版本切换时 reload）
    pub formations: Arc<RwLock<Vec<FormationEntry>>>,
    pub optimizer: Arc<optimizer::runtime::OptState>,
    pub rl_sessions: Arc<rl::session::RlSessions>,
    pub rl_train: Arc<rl::training::TrainState>,
    pub rl_analyze: Arc<rl::analysis::AnalyzeState>,
    pub rl_pretrain: Arc<rl::pretrain::PretrainState>,
    /// 装备配装数据（启动时加载，只读）
    pub equip_data: Arc<equip::EquipData>,
    /// 自动配装搜索进度（单 slot：同时只允许一个搜索）
    pub auto_search: Arc<std::sync::Mutex<AutoSearchProgress>>,
    /// 自动配装：取消标志（spawn 任务在 recurse / rayon iter 中检测，true 即中止）
    pub auto_search_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// 自动配装：暂停标志（true 时 spawn 任务 spin-wait 100ms 重检）
    pub auto_search_pause: Arc<std::sync::atomic::AtomicBool>,
}

/// 自动配装搜索进度状态。前端通过 GET /api/equip/auto_optimize/progress 拉。
/// `phase` 取值：idle / enumerate / calc / pareto / simulate / done / error
#[derive(Default, Clone, Serialize)]
pub struct AutoSearchProgress {
    pub running: bool,
    pub phase: String,
    pub total_combos: u64,
    pub enumerated: u64,
    /// 分支剪枝跳过的叶子数（subtree size 累加）。前端进度条用 (enumerated + branch_skipped) / total_combos
    /// 才是真实进度，否则分支剪掉的子树没体现，进度会停在 75% 然后跳到 100%。
    pub branch_skipped: u64,
    pub after_haste_pruning: u64,
    pub unique: u64,
    pub after_pareto: u64,
    /// 梯度预筛后保留数（智能模式下；普通模式 = after_pareto）
    pub after_rank: u64,
    pub simulated: u64,
    pub target_simulated: u64,
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub message: String,
    /// phase=done 时携带最终结果（前端 polling 拉取，不再依赖 POST 响应）
    pub result: Option<AutoOptimizeResponse>,
    /// 当前是否处于暂停状态（前端 polling 时同步暴露，便于按钮切换文字）
    pub paused: bool,
    /// 已收到取消请求（任务正在尽快收尾）
    pub cancelled: bool,
}

async fn health() -> impl IntoResponse { "OK" }

async fn macro_presets() -> String {
    let path = if Path::new("./macros/presets.json").exists() {
        "./macros/presets.json"
    } else {
        "backend/macros/presets.json"
    };
    std::fs::read_to_string(path).unwrap_or_else(|_| "[]".into())
}

async fn optimizer_candidates() -> String {
    let path = if Path::new("./macros/candidates.json").exists() {
        "./macros/candidates.json"
    } else {
        "backend/macros/candidates.json"
    };
    std::fs::read_to_string(path).unwrap_or_else(|_| "[]".into())
}

async fn macro_save(Json(body): Json<serde_json::Value>) -> String {
    let path = macro_save_path();
    match std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap_or_default()) {
        Ok(_) => "ok".into(),
        Err(e) => format!("error: {e}"),
    }
}

async fn macro_load() -> String {
    let path = macro_save_path();
    std::fs::read_to_string(&path).unwrap_or_else(|_| "null".into())
}

// ─── 循环（loop）保存/加载 ──────────────────────────────────────────
fn loops_dir() -> std::path::PathBuf {
    let p = user_data_path("loops");
    if !p.exists() { let _ = std::fs::create_dir_all(&p); }
    p
}

fn sanitize_loop_name(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ') || ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            out.push(ch);
        }
    }
    let out = out.trim().trim_matches('.').to_string();
    if out.is_empty() { "loop".into() } else { out }
}

#[derive(Debug, Deserialize)]
pub struct LoopSaveRequest {
    pub name: String,
    pub config: serde_json::Value,
}

async fn loop_save(Json(req): Json<LoopSaveRequest>) -> Json<serde_json::Value> {
    let safe = sanitize_loop_name(&req.name);
    let ts = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{}", secs)
    };
    let fname = format!("{}_{}.json", safe, ts);
    let path = loops_dir().join(&fname);
    match std::fs::write(&path, serde_json::to_string_pretty(&req.config).unwrap_or_default()) {
        Ok(_) => Json(serde_json::json!({ "ok": true, "file": fname, "path": path.display().to_string() })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn loop_list() -> Json<serde_json::Value> {
    let dir = loops_dir();
    let mut files: Vec<(String, std::time::SystemTime)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((name, mtime));
            }
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1));
    Json(serde_json::json!({ "files": files.into_iter().map(|(n, _)| n).collect::<Vec<_>>() }))
}

#[derive(Debug, Deserialize)]
pub struct LoopLoadQuery { pub name: String }

async fn loop_load(axum::extract::Query(q): axum::extract::Query<LoopLoadQuery>) -> Json<serde_json::Value> {
    let safe = sanitize_loop_name(&q.name.trim_end_matches(".json"));
    // 允许完整文件名或 slug 匹配 — 先按完整 name 直接找
    let dir = loops_dir();
    let direct = dir.join(&q.name);
    let path = if direct.exists() { direct } else { dir.join(format!("{}.json", safe)) };
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str::<serde_json::Value>(&t).map(Json).unwrap_or_else(|e| Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
pub struct LoopDeleteRequest { pub name: String }

async fn loop_delete(Json(req): Json<LoopDeleteRequest>) -> Json<serde_json::Value> {
    let dir = loops_dir();
    let path = dir.join(&req.name);
    // 必须在 loops 目录下，且扩展名是 json（防止路径穿越）
    if !path.starts_with(&dir) || path.extension().and_then(|s| s.to_str()) != Some("json") {
        return Json(serde_json::json!({ "ok": false, "error": "invalid name" }));
    }
    match std::fs::remove_file(&path) {
        Ok(_) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

// ─── 独立配装文件：userdata/equips/{name}_{ts}.json ───
fn equips_dir() -> std::path::PathBuf {
    let p = user_data_path("equips");
    if !p.exists() { let _ = std::fs::create_dir_all(&p); }
    p
}

#[derive(Debug, Deserialize)]
pub struct EquipConfigSaveRequest {
    pub name: String,
    pub config: serde_json::Value,  // { slots, stone_id, stone_name, mount }
}

async fn equip_config_save(Json(req): Json<EquipConfigSaveRequest>) -> Json<serde_json::Value> {
    let safe = sanitize_loop_name(&req.name);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let fname = format!("{}_{}.json", safe, ts);
    let path = equips_dir().join(&fname);
    // 在 config 里补一份 metadata，便于 list 时无需完整解析
    let mut data = req.config.clone();
    if let Some(obj) = data.as_object_mut() {
        obj.insert("name".to_string(), serde_json::Value::String(req.name.clone()));
        obj.insert("created_at".to_string(), serde_json::Value::Number(ts.into()));
    }
    match std::fs::write(&path, serde_json::to_string_pretty(&data).unwrap_or_default()) {
        Ok(_) => Json(serde_json::json!({ "ok": true, "file": fname })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn equip_config_list() -> Json<serde_json::Value> {
    let dir = equips_dir();
    let mut entries: Vec<serde_json::Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            let file = match p.file_name().and_then(|s| s.to_str()) {
                Some(f) => f.to_string(),
                None => continue,
            };
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mtime_secs = mtime.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            // 读取文件内容提取 meta；失败就只返回 file + mtime
            let (name, mount, slot_count, created_at) = match std::fs::read_to_string(&p) {
                Ok(txt) => {
                    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or(serde_json::Value::Null);
                    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let mount = v.get("mount").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let created = v.get("created_at").and_then(|x| x.as_u64()).unwrap_or(mtime_secs);
                    let slots = v.get("slots").and_then(|x| x.as_object());
                    let count = slots.map(|o| o.values().filter(|s| s.get("equip_id").and_then(|id| id.as_u64()).unwrap_or(0) > 0).count()).unwrap_or(0);
                    (name, mount, count, created)
                }
                Err(_) => (String::new(), String::new(), 0_usize, mtime_secs),
            };
            entries.push(serde_json::json!({
                "file": file,
                "name": name,
                "mount": mount,
                "slot_count": slot_count,
                "created_at": created_at,
                "updated_at": mtime_secs,
            }));
        }
    }
    entries.sort_by(|a, b| b.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0)
                      .cmp(&a.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0)));
    Json(serde_json::json!({ "files": entries }))
}

#[derive(Debug, Deserialize)]
pub struct EquipConfigLoadQuery { pub file: String }

async fn equip_config_load(axum::extract::Query(q): axum::extract::Query<EquipConfigLoadQuery>) -> Json<serde_json::Value> {
    let dir = equips_dir();
    let path = dir.join(&q.file);
    if !path.starts_with(&dir) || path.extension().and_then(|s| s.to_str()) != Some("json") {
        return Json(serde_json::json!({ "error": "invalid name" }));
    }
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str::<serde_json::Value>(&t).map(Json).unwrap_or_else(|e| Json(serde_json::json!({ "error": e.to_string() }))),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
pub struct EquipConfigDeleteRequest { pub file: String }

async fn equip_config_delete(Json(req): Json<EquipConfigDeleteRequest>) -> Json<serde_json::Value> {
    let dir = equips_dir();
    let path = dir.join(&req.file);
    if !path.starts_with(&dir) || path.extension().and_then(|s| s.to_str()) != Some("json") {
        return Json(serde_json::json!({ "ok": false, "error": "invalid name" }));
    }
    match std::fs::remove_file(&path) {
        Ok(_) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

// ─── 工作流恢复状态（顶栏"上次 X/步骤 N"）持久化到 userdata ───
fn resume_save_path() -> std::path::PathBuf { user_data_path("resume.json") }

async fn resume_save(Json(body): Json<serde_json::Value>) -> String {
    let path = resume_save_path();
    match std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap_or_default()) {
        Ok(_) => "ok".into(),
        Err(e) => format!("error: {e}"),
    }
}

async fn resume_load() -> String {
    let path = resume_save_path();
    std::fs::read_to_string(&path).unwrap_or_else(|_| "null".into())
}

// ─── 用户 UI 设置（主题/字体/序列显示模式等）账号级持久化 ───────────────
// 前端把要同步的设置项打包成一个 JSON 对象 POST 上来，后端原样存 per-user 目录；
// 登入后 GET 回来覆盖 localStorage → 设置跟账号走（换设备/换 origin/清缓存都跟随）。
fn settings_path() -> std::path::PathBuf { user_data_path("settings.json") }
async fn settings_save(Json(body): Json<serde_json::Value>) -> String {
    let body = filter_sensitive_settings(body);
    match std::fs::write(settings_path(), serde_json::to_string_pretty(&body).unwrap_or_default()) {
        Ok(_) => "ok".into(),
        Err(e) => format!("error: {e}"),
    }
}
async fn settings_load() -> String {
    let Ok(source) = std::fs::read_to_string(settings_path()) else {
        return "null".into();
    };
    let Ok(value) = serde_json::from_str(&source) else {
        return "null".into();
    };
    serde_json::to_string(&filter_sensitive_settings(value)).unwrap_or_else(|_| "null".into())
}

fn filter_sensitive_settings(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(settings) = value.as_object_mut() {
        settings.retain(|key, _| !is_sensitive_setting_key(key));
    }
    value
}

fn is_sensitive_setting_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "apikey",
        "authorization",
        "credential",
        "password",
        "accesstoken",
        "refreshtoken",
        "bearertoken",
        "clientsecret",
        "secret",
        "token",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod settings_security_tests {
    use super::*;

    #[test]
    fn sensitive_settings_are_removed_without_changing_normal_preferences() {
        let filtered = filter_sensitive_settings(serde_json::json!({
            "ui_theme": "indigo",
            "agent_provider_profile": "offline",
            "openai_api_key": "must-not-survive",
            "Authorization": "must-not-survive",
            "refresh-token": "must-not-survive",
            "provider_secret": "must-not-survive",
            "session_token": "must-not-survive"
        }));
        assert_eq!(filtered["ui_theme"], "indigo");
        assert_eq!(filtered["agent_provider_profile"], "offline");
        assert!(filtered.get("openai_api_key").is_none());
        assert!(filtered.get("Authorization").is_none());
        assert!(filtered.get("refresh-token").is_none());
        assert!(filtered.get("provider_secret").is_none());
        assert!(filtered.get("session_token").is_none());
    }
}

// ─── 当前心法/版本 per-user 持久化 ──────────────────────────────────────
// 进程隔离下 worker 空闲回收后重建，内存态心法会重置回默认。落盘 + 启动读回，
// 否则回收后 current_mount 变默认 → 前端 autosave 因"心法不匹配"跳过恢复 → 像循环丢了。
fn mount_state_path() -> std::path::PathBuf { user_data_path("mount_state.json") }
#[derive(serde::Serialize, serde::Deserialize)]
struct MountState { version: GameVersion, mount: Mount }
fn save_mount_state(version: GameVersion, mount: Mount) {
    let _ = std::fs::write(
        mount_state_path(),
        serde_json::to_string_pretty(&MountState { version, mount }).unwrap_or_default(),
    );
}
fn load_mount_state() -> Option<(GameVersion, Mount)> {
    let txt = std::fs::read_to_string(mount_state_path()).ok()?;
    let s: MountState = serde_json::from_str(&txt).ok()?;
    Some((s.version, s.mount))
}

async fn loop_open_folder() -> Json<serde_json::Value> {
    let dir = loops_dir();
    let p = dir.display().to_string();
    #[cfg(target_os = "windows")]
    let res = std::process::Command::new("explorer").arg(&p).spawn();
    #[cfg(target_os = "macos")]
    let res = std::process::Command::new("open").arg(&p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let res = std::process::Command::new("xdg-open").arg(&p).spawn();
    match res {
        Ok(_) => Json(serde_json::json!({ "ok": true, "path": p })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string(), "path": p })),
    }
}

#[derive(Debug, Deserialize)]
pub struct MacroFromSequenceRequest {
    pub timeline: Vec<macro_gen::InputCast>,
    #[serde(default)]
    pub options: macro_gen::GenOptions,
}

async fn macro_from_sequence(Json(req): Json<MacroFromSequenceRequest>) -> Json<macro_gen::GenResult> {
    Json(macro_gen::generate(&req.timeline, &req.options))
}

// ─── 冗余剪枝候选端点 ──────────────────────────────────────────────
// 前端按需调 /api/simulate 评估每个候选；这里只负责解析 + 列举 + 渲染。
#[derive(Debug, Deserialize)]
pub struct MacroPruneCandidatesRequest {
    pub macro_text: String,
}

async fn macro_prune_candidates(Json(req): Json<MacroPruneCandidatesRequest>) -> Response {
    match macro_prune::list_prune_candidates(&req.macro_text) {
        Ok(list) => Json(serde_json::json!({ "candidates": list })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

async fn macro_swap_candidates(Json(req): Json<MacroPruneCandidatesRequest>) -> Response {
    match macro_prune::list_swap_candidates(&req.macro_text) {
        Ok(list) => Json(serde_json::json!({ "candidates": list })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

async fn macro_tighten_candidates(Json(req): Json<MacroPruneCandidatesRequest>) -> Response {
    match macro_prune::list_tighten_candidates(&req.macro_text) {
        Ok(list) => Json(serde_json::json!({ "candidates": list })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

// ─── 批量模拟（工作流 A v2 用：一次请求跑多个场景）──────────────────
#[derive(Debug, Deserialize)]
pub struct BatchSimScenario {
    #[serde(default)] pub network_delay: Option<u32>,
    #[serde(default)] pub initial_rage: Option<i32>,
    #[serde(default)] pub macro_duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct BatchSimRequest {
    pub base: SimulateRequest,
    pub scenarios: Vec<BatchSimScenario>,
}

#[derive(Debug, Serialize)]
pub struct BatchSimResult {
    pub dps: f64,
    pub fight_time: f64,
    pub total_damage: f64,
    pub cast_counts: HashMap<String, u32>,
}

async fn batch_simulate(
    State(state): State<SharedState>,
    Json(req): Json<BatchSimRequest>,
) -> Json<Vec<BatchSimResult>> {
    let skills = state.skills.read().await;
    let cur_version = *state.version.read().await;
    let cur_mount = *state.mount.read().await;
    let cur_consts = *state.constants.read().await;
    let recipes_table = state.recipes.read().await;
    let team_buffs_table = state.team_buffs.read().await;
    let formations_table = state.formations.read().await;

    let results: Vec<BatchSimResult> = req.scenarios.iter().map(|sc| {
        let mut r = req.base.clone();
        if let Some(d) = sc.network_delay { r.network_delay = d; }
        if let Some(rage) = sc.initial_rage { r.initial_rage = Some(rage); }
        if let Some(dur) = sc.macro_duration { r.macro_duration = Some(dur); }
        // 确保序列足够长
        if let Some(dur) = r.macro_duration {
            let needed = (dur / 0.25) as usize + 20;
            if r.sequence.len() < needed {
                r.sequence.resize(needed, "__macro__".to_string());
            }
        }
        // 强制 lite + 保留 timeline：内部只需 cast counts + DPS，hover 数据全省
        r.lite = true;
        r.lite_keep_timeline = true;
        let resp = simulate_core(&r, &skills, cur_version, cur_mount, cur_consts, &recipes_table, &team_buffs_table, &formations_table);
        // 只提取轻量结果
        let mut cast_counts: HashMap<String, u32> = HashMap::new();
        for ev in &resp.timeline {
            if ev.triggered { continue; }
            // 雾海阵云系列保持完整名
            let key = if ev.skill_id >= 90010 && ev.skill_id <= 90012 {
                &ev.name as &str
            } else {
                ev.name.split('·').next().unwrap_or(&ev.name)
            };
            *cast_counts.entry(key.to_string()).or_insert(0) += 1;
        }
        BatchSimResult {
            dps: resp.dps,
            fight_time: resp.fight_time,
            total_damage: resp.total_damage,
            cast_counts,
        }
    }).collect();

    Json(results)
}

async fn attrs_save(State(state): State<SharedState>, Json(body): Json<serde_json::Value>) -> String {
    let mount = *state.mount.read().await;
    let path = attrs_save_path(mount);
    match std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap_or_default()) {
        Ok(_) => "ok".into(),
        Err(e) => format!("error: {e}"),
    }
}

async fn attrs_load(State(state): State<SharedState>) -> String {
    let mount = *state.mount.read().await;
    let path = attrs_save_path(mount);
    std::fs::read_to_string(&path).unwrap_or_else(|_| "null".into())
}

// ─── 属性/宏 多配置档 ────────────────────────────────────────────────

#[derive(Deserialize)]
struct ProfileQuery {
    name: String,
}

#[derive(Deserialize)]
struct ProfileSaveBody {
    name: String,
    data: serde_json::Value,
}

async fn attrs_profiles(State(state): State<SharedState>) -> Json<Vec<String>> {
    let mount = *state.mount.read().await;
    let prefix = format!("attrs_{}_", mount_dir_name(mount));
    let dir = user_data_dir();
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with(&prefix) && fname.ends_with(".json") {
                let name = &fname[prefix.len()..fname.len() - 5];
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    Json(names)
}

async fn attrs_save_profile(
    State(state): State<SharedState>,
    Json(body): Json<ProfileSaveBody>,
) -> Response {
    let Some(safe_name) = sanitize_profile_name(&body.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let mount = *state.mount.read().await;
    let filename = format!("attrs_{}_{}.json", mount_dir_name(mount), safe_name);
    let path = user_data_path(&filename);
    match std::fs::write(&path, serde_json::to_string_pretty(&body.data).unwrap_or_default()) {
        Ok(_) => "ok".into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("error: {e}")).into_response(),
    }
}

async fn attrs_load_profile(
    State(state): State<SharedState>,
    axum::extract::Query(q): axum::extract::Query<ProfileQuery>,
) -> Response {
    let Some(safe_name) = sanitize_profile_name(&q.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let mount = *state.mount.read().await;
    let filename = format!("attrs_{}_{}.json", mount_dir_name(mount), safe_name);
    let path = user_data_path(&filename);
    match std::fs::read_to_string(&path) {
        Ok(s) => s.into_response(),
        Err(_) => "null".into_response(),
    }
}

async fn attrs_delete_profile(
    State(state): State<SharedState>,
    axum::extract::Query(q): axum::extract::Query<ProfileQuery>,
) -> Response {
    let Some(safe_name) = sanitize_profile_name(&q.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let mount = *state.mount.read().await;
    let filename = format!("attrs_{}_{}.json", mount_dir_name(mount), safe_name);
    let path = user_data_path(&filename);
    match std::fs::remove_file(&path) {
        Ok(_) => "ok".into_response(),
        Err(e) => (axum::http::StatusCode::NOT_FOUND, format!("error: {e}")).into_response(),
    }
}

async fn macro_profiles() -> Json<Vec<String>> {
    let dir = user_data_dir();
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with("macros_") && fname.ends_with(".json") {
                let name = &fname[7..fname.len() - 5]; // "macros_".len() = 7
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    Json(names)
}

async fn macro_save_profile(Json(body): Json<ProfileSaveBody>) -> Response {
    let Some(safe_name) = sanitize_profile_name(&body.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let filename = format!("macros_{}.json", safe_name);
    let path = user_data_path(&filename);
    match std::fs::write(&path, serde_json::to_string_pretty(&body.data).unwrap_or_default()) {
        Ok(_) => "ok".into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("error: {e}")).into_response(),
    }
}

async fn macro_load_profile(
    axum::extract::Query(q): axum::extract::Query<ProfileQuery>,
) -> Response {
    let Some(safe_name) = sanitize_profile_name(&q.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let filename = format!("macros_{}.json", safe_name);
    let path = user_data_path(&filename);
    match std::fs::read_to_string(&path) {
        Ok(s) => s.into_response(),
        Err(_) => "null".into_response(),
    }
}

async fn macro_delete_profile(
    axum::extract::Query(q): axum::extract::Query<ProfileQuery>,
) -> Response {
    let Some(safe_name) = sanitize_profile_name(&q.name) else {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid profile name").into_response();
    };
    let filename = format!("macros_{}.json", safe_name);
    let path = user_data_path(&filename);
    match std::fs::remove_file(&path) {
        Ok(_) => "ok".into_response(),
        Err(e) => (axum::http::StatusCode::NOT_FOUND, format!("error: {e}")).into_response(),
    }
}

// ─── 优化器：宏分析 ───
#[derive(Deserialize)]
struct OptimizerAnalyzeRequest {
    macro_text: String,
}

async fn rl_rollout(
    State(state): State<SharedState>,
    Json(req): Json<rl::rollout::RolloutRequest>,
) -> Response {
    let skills = Arc::new(state.skills.read().await.clone());
    let recipes = Arc::new(state.recipes.read().await.clone());
    match rl::rollout::run_rollout(req, skills, recipes) {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        ).into_response(),
    }
}

async fn optimizer_analyze(Json(req): Json<OptimizerAnalyzeRequest>) -> Response {
    match optimizer::analyze::extract_tunables(&req.macro_text) {
        Ok(result) => Json(result).into_response(),
        Err(err) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err })),
        ).into_response(),
    }
}

async fn calculate(
    State(state): State<SharedState>,
    Json(attr): Json<Attributes>,
) -> impl IntoResponse {
    println!("[calculate] shen_fa={}", attr.shen_fa);
    let slots: AttribSlots = HashMap::new();
    let consts = *state.constants.read().await;
    Json(build_runtime_stats(&attr, &slots, &consts))
}

async fn skill_damage(
    State(state): State<SharedState>,
    Json(req): Json<SkillDamageRequest>,
) -> impl IntoResponse {
    let sd_start = std::time::Instant::now();
    let skills = state.skills.read().await;
    let recipes_guard = state.recipes.read().await;
    let recipes_table: &[RecipeEntry] = recipes_guard.as_slice();

    // 构建一个临时 player 反映已选秘籍 + talent 联动
    let cur_version = *state.version.read().await;
    let cur_mount = *state.mount.read().await;
    let cur_consts = *state.constants.read().await;
    let mut player = Player::with_mount(cur_mount, cur_version, cur_consts, 0,
        req.talents.clone().unwrap_or_default(), req.recipes.clone().unwrap_or_default());
    // 铁骨气劲
    if cur_mount == Mount::TieGuYi {
        match req.tiegu_mode {
            1 => { player.add_buff(BUFF_TIE_GU); }
            2 => { player.add_buff(BUFF_TIE_GU_SU_DI); }
            _ => {}
        }
    }
    player.base_attrs = req.attributes.clone();
    player.set_equipped(req.equipment.clone());
    // 战斗开始钩子：奇穴常驻 buff + 装备特效（EnterFight 类）
    scripts::on_battle_start(&mut player);
    // 团队增益（无时间维度入口）：永久型直接挂；周期型按 duration/period 平均覆盖率折算
    if !req.team_buffs.is_empty() {
        let table = state.team_buffs.read().await.clone();
        apply_team_buffs_for_static(&mut player, &req.team_buffs, &table);
    }
    // 阵法（基础设置入口）：self/any 用 permanent_effects，other 用 public_effects_other
    // self 变体的触发型 effects 在循环模拟里走拟真；基础设置无时间维度，只取永久部分。
    {
        let formations_table = state.formations.read().await;
        player.formation = req.formation.clone();
        player.formation_permanent_slots = formation_permanent_effects(&req.formation, &formations_table);
        player.formation_self_id = resolve_formation_self_id(&req.formation, &formations_table);
    }
    let buff_slots = aggregate_buff_fields(&player);
    let target_slots = aggregate_target_buff_fields(&player);
    let rt = build_runtime_stats(&req.attributes, &buff_slots, &player.constants);

    // 基线防御率：无技能减防、无 buff/debuff 修饰，等同于 9 步链 Step 4 的退化形式
    let base_defense_rate = calc_defense_rate_with_ignore(&req.target, 0.0, 0.0, 0.0, 0.0);
    let level_suppression = calc_level_suppression(PLAYER_LEVEL, req.target.level);

    ensure_recipe_index(recipes_table);
    let skill_results: Vec<SkillResult> = skills.iter().map(|s| {
        let base_name = s.name.split('·').next().unwrap_or(&s.name);
        let recipes = collect_recipes_indexed(&player, s.skill_id, base_name, &[], recipes_table);
        // 奇穴/加速 动态覆盖 attack_coeff
        if let Some(coeff) = scripts::override_attack_coeff(&player, s) {
            let mut s2 = s.clone();
            s2.attack_coeff = coeff;
            calc_damage(&s2, &req.attributes, &req.target, &rt, &recipes, &buff_slots, &target_slots, player.constants.non_player_bonus)
        } else {
            calc_damage(s, &req.attributes, &req.target, &rt, &recipes, &buff_slots, &target_slots, player.constants.non_player_bonus)
        }
    }).collect();
    let sd_elapsed = sd_start.elapsed();
    println!("[skill_damage] {} skills | {:.1}ms elapsed",
        skill_results.len(), sd_elapsed.as_secs_f64() * 1000.0);
    Json(SkillDamageResponse {
        base_defense_rate,
        level_suppression,
        skills: skill_results,
    })
}

// ─── 心法 / 武学版本 切换 API ──────────────────────────────────

#[derive(Serialize)]
struct MountOption {
    version:        String,  // 枚举序列化字符串，如 "ShanHaiYuanLiu"
    version_dir:    String,  // 目录名，如 "2025_10_山海源流"
    version_label:  String,  // 展示名，如 "山海源流（2025.10）"
    mount:          String,  // 枚举序列化字符串，如 "FenShanJin"
    mount_dir:      String,  // 目录名，如 "分山劲"
    mount_label:    String,  // 展示名
    class_name:     String,  // 心法所属门派，如 "苍云"
}

fn version_label(v: GameVersion) -> &'static str {
    match v {
        GameVersion::ShanHaiYuanLiu => "山海源流（2025.10）",
        GameVersion::AnYingQianJi => "暗影千机（2026.04）",
        GameVersion::AnYingQianJiTest => "暗影千机·测试服（归档兼容）",
    }
}

/// UI 暴露的正式服版本列表。
fn all_versions() -> Vec<GameVersion> {
    vec![GameVersion::AnYingQianJi, GameVersion::ShanHaiYuanLiu]
}

fn all_mounts() -> Vec<Mount> {
    vec![Mount::FenShanJin, Mount::TieGuYi]
}

async fn list_mounts() -> impl IntoResponse {
    let mut out = Vec::new();
    for v in all_versions() {
        for m in all_mounts() {
            // 只返回 school.toml 存在的 (version, mount) 组合（铁骨衣可能暂未填数据）
            if !Path::new(&school_toml_path(v, m)).exists() { continue; }
            let class = load_school_toml(v, m).ok()
                .and_then(|_| {
                    // 重新读一遍拿 class 字段（load_school_toml 目前丢了 class）
                    std::fs::read_to_string(school_toml_path(v, m)).ok()
                })
                .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
                .and_then(|v| v.get("class").and_then(|c| c.as_str()).map(String::from))
                .unwrap_or_else(|| "苍云".into());
            out.push(MountOption {
                version:       format!("{:?}", v),
                version_dir:   version_dir_name(v).to_string(),
                version_label: version_label(v).to_string(),
                mount:         format!("{:?}", m),
                mount_dir:     mount_dir_name(m).to_string(),
                mount_label:   mount_dir_name(m).to_string(),
                class_name:    class,
            });
        }
    }
    Json(out)
}

#[derive(Deserialize)]
struct SwitchMountRequest {
    version: GameVersion,
    mount:   Mount,
    /// UI 切换默认持久化；测试可传 false，避免污染本地用户选择。
    #[serde(default = "default_true")]
    persist: bool,
}

#[derive(Serialize)]
struct SwitchMountResponse {
    ok: bool,
    error: Option<String>,
    school_ui:  Option<SchoolUi>,
    workflow_a: Option<WorkflowA>,
}

async fn switch_mount(
    State(state): State<SharedState>,
    Json(req): Json<SwitchMountRequest>,
) -> impl IntoResponse {
    let _agent_context_guard = state.agent_context_gate.write().await;
    let (consts, base_stats, mount_conv, ui, wa) = match load_school_toml(req.version, req.mount) {
        Ok(v) => v,
        Err(e) => return Json(SwitchMountResponse {
            ok: false, error: Some(e), school_ui: None, workflow_a: None,
        }),
    };
    let new_skills = load_skills(Path::new(&skills_dir(req.version, req.mount)));
    let new_talents = load_talents(Path::new(&talents_file(req.version, req.mount)));
    let new_recipes = load_recipes(Path::new(&recipes_file(req.version)));
    let new_team_buffs = load_team_buffs(Path::new(&team_buffs_file(req.version)));
    let new_formations = load_formations(Path::new(&formations_file(req.version)));
    let new_agent_provenance = agent::ToolProvenance::from_runtime_data(
        req.version,
        req.mount,
        consts,
        &new_skills,
        &new_recipes,
        &new_team_buffs,
        &new_formations,
    );

    *state.version.write().await = req.version;
    *state.mount.write().await = req.mount;
    *state.constants.write().await = consts;
    *state.base_stats.write().await = base_stats;
    *state.mount_conversions.write().await = mount_conv;
    *state.school_ui.write().await = ui.clone();
    *state.workflow_a.write().await = wa.clone();
    *state.skills.write().await = new_skills;
    *state.talents.write().await = new_talents;
    *state.recipes.write().await = new_recipes;
    *state.team_buffs.write().await = new_team_buffs;
    *state.formations.write().await = new_formations;
    *state.agent_provenance.write().await = new_agent_provenance;

    // 落盘当前心法/版本，worker 回收重建后启动读回（否则重置回默认 → 前端跳过恢复）
    if req.persist {
        save_mount_state(req.version, req.mount);
    }

    Json(SwitchMountResponse {
        ok: true, error: None,
        school_ui: Some(ui), workflow_a: Some(wa),
    })
}

#[derive(Serialize)]
struct CurrentMountInfo {
    version:       String,
    version_dir:   String,
    version_label: String,
    mount:         String,
    mount_dir:     String,
    mount_label:   String,
    school_ui:     SchoolUi,
    workflow_a:    WorkflowA,
}

async fn current_mount(State(state): State<SharedState>) -> impl IntoResponse {
    let v = *state.version.read().await;
    let m = *state.mount.read().await;
    Json(CurrentMountInfo {
        version:       format!("{:?}", v),
        version_dir:   version_dir_name(v).to_string(),
        version_label: version_label(v).to_string(),
        mount:         format!("{:?}", m),
        mount_dir:     mount_dir_name(m).to_string(),
        mount_label:   mount_dir_name(m).to_string(),
        school_ui:     state.school_ui.read().await.clone(),
        workflow_a:    state.workflow_a.read().await.clone(),
    })
}

/// 心法默认配置（属性 / 奇穴 / 秘籍）：`data/{ver}/{mount}/defaults.json`
async fn mount_defaults(State(state): State<SharedState>) -> impl IntoResponse {
    let v = *state.version.read().await;
    let m = *state.mount.read().await;
    let path = mount_defaults_path(v, m);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(val) => Json(val).into_response(),
            Err(_) => Json(serde_json::json!(null)).into_response(),
        },
        Err(_) => Json(serde_json::json!(null)).into_response(),
    }
}

async fn reload_skills(State(state): State<SharedState>) -> impl IntoResponse {
    let _agent_context_guard = state.agent_context_gate.write().await;
    let version = *state.version.read().await;
    let mount = *state.mount.read().await;
    let new_skills = load_skills(Path::new(&skills_dir(version, mount)));
    let names: Vec<String> = new_skills.iter().map(|s| s.name.clone()).collect();
    let loaded = new_skills.len();
    let new_agent_provenance = agent::ToolProvenance::from_runtime_data(
        version,
        mount,
        *state.constants.read().await,
        &new_skills,
        &state.recipes.read().await,
        &state.team_buffs.read().await,
        &state.formations.read().await,
    );
    *state.skills.write().await = new_skills;
    *state.agent_provenance.write().await = new_agent_provenance;
    Json(ReloadResult { loaded, skills: names })
}

async fn list_skills(State(state): State<SharedState>) -> impl IntoResponse {
    Json(state.skills.read().await.clone())
}

async fn list_talents(State(state): State<SharedState>) -> impl IntoResponse {
    Json(state.talents.read().await.clone())
}

async fn list_team_buffs(State(state): State<SharedState>) -> impl IntoResponse {
    let entries = state.team_buffs.read().await.clone();
    Json(entries)
}

async fn list_formations(State(state): State<SharedState>) -> impl IntoResponse {
    let entries = state.formations.read().await.clone();
    Json(entries)
}

async fn list_recipes(State(state): State<SharedState>) -> impl IntoResponse {
    // 仅返回玩家可见秘籍（隐藏秘籍由 buff/脚本激活，不在 UI 显示）
    let visible: Vec<RecipeEntry> = state.recipes.read().await.iter()
        .filter(|r| !r.hidden)
        .cloned()
        .collect();
    Json(visible)
}

async fn simulate(
    State(state): State<SharedState>,
    Json(req): Json<SimulateRequest>,
) -> impl IntoResponse {
    let skills = state.skills.read().await;
    let cur_version = *state.version.read().await;
    let cur_mount = *state.mount.read().await;
    let cur_consts = *state.constants.read().await;
    let recipes_table = state.recipes.read().await;
    let team_buffs_table = state.team_buffs.read().await;
    let formations_table = state.formations.read().await;

    let t_core_start = std::time::Instant::now();
    let resp = simulate_core(&req, &skills, cur_version, cur_mount, cur_consts, &recipes_table, &team_buffs_table, &formations_table);
    let t_core = t_core_start.elapsed();

    // 序列化 to Vec<u8> 计时（与 axum 实际写出一致的工作量）
    let t_serde_start = std::time::Instant::now();
    let body = serde_json::to_vec(&resp).unwrap_or_default();
    let t_serde = t_serde_start.elapsed();

    if !req.lite {
        println!("[simulate] core={:.1}ms serde={:.1}ms (events={}, body={}KB)",
            t_core.as_secs_f64() * 1000.0,
            t_serde.as_secs_f64() * 1000.0,
            resp.timeline.len(),
            body.len() / 1024);
    }

    ([(axum::http::header::CONTENT_TYPE, "application/json")], body)
}

fn simulate_core(
    req: &SimulateRequest,
    skills: &[SkillSpec],
    cur_version: GameVersion,
    cur_mount: Mount,
    cur_consts: MountConstants,
    recipes_table: &[RecipeEntry],
    team_buffs_table: &[TeamBuffEntry],
    formations_table: &[FormationEntry],
) -> SimulateResponse {
    let sim_start = std::time::Instant::now();
    SIM_PERF.with(|p| *p.borrow_mut() = SimPerf::default());
    // 秘籍倒排索引（按需构建/重建）
    ensure_recipe_index(recipes_table);
    // 跨请求清空 active_ids 缓存（避免上一请求的 talents/recipes 残留）
    ACTIVE_IDS.with(|c| *c.borrow_mut() = None);

    // 构建 baseName → Vec<&SkillSpec> 映射（同名多品级，用于连招段数选择）
    // 跳过被动技能（passive=true），它们只用于伤害计算
    let mut skill_map: HashMap<&str, Vec<&SkillSpec>> = HashMap::new();
    for s in skills.iter() {
        if s.passive { continue; }
        // 雾海寻龙系列用完整名做 key（避免 "阵云结晦·雾海" 被拆成 "阵云结晦" 和旧版合并）
        let base = if s.skill_id >= 90010 && s.skill_id <= 90012 {
            &s.name as &str
        } else {
            s.name.split('·').next().unwrap_or(&s.name)
        };
        skill_map.entry(base).or_default().push(s);
    }

    let mut player = Player::with_mount(cur_mount, cur_version, cur_consts,
        req.haste_level, req.talents.clone(), req.recipes.clone());
    if let Some(rage) = req.initial_rage {
        player.set_rage(rage);
    }
    player.experimental = req.experimental;
    player.lite_mode = req.lite;
    // 装备清单：脚本通过 player.equip_id_at("PRIMARY_WEAPON") 等查询；
    // set_equipped 同步维护 equipped_values 索引（O(1) has_enchant）
    player.set_equipped(req.equipment.clone());
    // 属性面板快照（供脚本 current_stats 使用）
    if let Some(ref a) = req.attributes {
        player.base_attrs = a.clone();
    }
    // Boss 受击模拟：按攻击间隔周期产生受击事件（驱动坚铁/寒甲/承伤等）
    if let Some(interval) = req.boss_attack_interval {
        if interval > 0.0 {
            player.boss_attack_interval = interval;
            player.next_boss_attack = Some(interval); // 第一次受击在 t=interval
        }
    }
    // 期望传播子系统：坚铁概率始终计算，toggle 只控制寒甲 A/B 是否走期望
    if (player.has_talent(13134) || player.has_talent(13138)) {
        if let Some(interval) = req.boss_attack_interval {
            if interval > 0.0 {
                let hanjia_exp = req.hanjia_expectation.unwrap_or(false);
                player.expectation = Some(ExpectationState::new(interval, hanjia_exp));
            }
        }
    }
    // 盾压 CD 期望重置（仅铁骨衣）
    if player.mount == Mount::TieGuYi {
        let cd_frames = 192u32
            - if player.has_recipe(4005) { 16 } else { 0 }
            - if player.has_recipe(4006) { 16 } else { 0 };
        let extra_prob =
              if player.has_recipe(4007) { 0.05 } else { 0.0 }
            + if player.has_recipe(4008) { 0.05 } else { 0.0 };
        player.dunya_cd = Some(DunyaCdState {
            cd_remain: 0.0,
            avail_credit: 1.0,
            cd_frames,
            extra_reset_prob: extra_prob,
            last_frame: 0,
        });
    }
    // 铁骨气劲（仅铁骨衣）
    if player.mount == Mount::TieGuYi {
        match req.tiegu_mode {
            1 => { player.add_buff(BUFF_TIE_GU); }
            2 => { player.add_buff(BUFF_TIE_GU_SU_DI); }
            _ => {}
        }
    }
    // 战斗开始钩子：激活奇穴常驻 buff（如寒甲）
    scripts::on_battle_start(&mut player);
    // Boss 受击模式：禁用寒甲 3s 周期 tick（刷新改由 on_player_hit 招架成功时触发）
    if player.next_boss_attack.is_some() {
        player.set_buff_tick_interval(BUFF_HAN_JIA, 0);
    }
    // 团队增益：按时间排程到 player.pending_team_buffs（process_buff_ticks 内消费）
    // total_duration: 优先 macro_duration；否则 3600s 兜底（覆盖任意长手动序列；
    // pending 队列消费 O(N)，最坏 50 条 × 3600/period = 极端配置才会大；正常 < 200 项）
    if !req.team_buffs.is_empty() && !team_buffs_table.is_empty() {
        let total_dur = req.macro_duration.unwrap_or(3600.0);
        apply_team_buffs_for_simulate(&mut player, &req.team_buffs, team_buffs_table, total_dur);
    }
    // 阵法：永久 effects 预计算 + self/other 标识写入 player（aggregate_buff_fields 合并）
    player.formation = req.formation.clone();
    player.formation_permanent_slots = formation_permanent_effects(&req.formation, formations_table);
    player.formation_self_id = resolve_formation_self_id(&req.formation, formations_table);

    // 开战面板（t=0：永久 buff/团辅 已挂、阵法已注入 player.formation_permanent_slots、装备已 setup）
    // 消费 pending_team_buffs 中 release_at<=0 的项（永久型团辅）让永久 buff 在身
    let (initial_stats, initial_buffs): (Option<CombatStats>, Vec<BuffSnapshot>) = if let Some(a) = req.attributes.as_ref() {
        let _t0_events = player.process_buff_ticks(0.0, 0.0);
        let buff_slots = aggregate_buff_fields(&player);
        let stats = build_runtime_stats(a, &buff_slots, &player.constants);
        // 同时取 t=0 时刻 buff 快照，供前端实时面板 hover"增益来源"用
        let buffs = if req.lite { Vec::new() } else { snapshot_buffs(&player) };
        (Some(stats), buffs)
    } else {
        (None, Vec::new())
    };

    let delay_sec = req.network_delay as f64 / 1000.0;

    // 预计算战斗属性（用于伤害计算）
    // dmg_ctx 含 (attributes, target, runtime_stats_base)；runtime_stats 在 calc_event_damage 内会重算（含动态buff）
    let dmg_ctx: Option<(Attributes, TargetConfig)> = match (&req.attributes, &req.target) {
        (Some(a), Some(t)) => Some((a.clone(), t.clone())),
        _ => None,
    };
    // skill_id → SkillSpec 映射（含 passive 技能，用于触发事件伤害）
    let skill_by_id: HashMap<u32, &SkillSpec> = skills.iter().map(|s| (s.skill_id, s)).collect();

    // ── 统一流程：手动序列 → 可选宏模拟 ──
    let mut timeline = Vec::new();
    let mut skipped: Vec<(usize, String)> = Vec::new();
    let mut last_cast_time: f64 = 0.0;
    let mut prev_time: f64 = 0.0;
    let mut is_first_main = true;

    // 预解析宏配置（如果有宏文本）
    let macro_config = req.macro_text.as_ref().and_then(|text| {
        match macro_parser::parse_macro_text(text) {
            Ok(c) => Some(c),
            Err(e) => {
                skipped.push((0, e.to_string()));
                None
            }
        }
    });
    let mut macro_debug = Vec::new();
    let mut macro_last_skill: Option<String> = None;

    // ── 预释放预处理 ──
    // 按 time_before 降序（最早的先 cast）逐一在虚拟负时间点上 cast，让 buff/CD 在
    // t=0 时刻处于"已生效"状态。脚本 emit / cast 生成的 timeline 事件全部丢弃。
    // 注意：cast_skill 内部会推进 player.current_time（cast/channel duration），
    // 如果两条预释放间隔太短可能重叠 → 取 max 不回退；cast 失败（GCD/姿态/怒气）就跳过。
    if !req.pre_releases.is_empty() {
        let mut order: Vec<usize> = (0..req.pre_releases.len()).collect();
        order.sort_by(|&a, &b| {
            req.pre_releases[b].time_before
                .partial_cmp(&req.pre_releases[a].time_before)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // player 默认 current_time/channel_end/last_cast_time = 0.0，
        // next_cast_time 里 max(current_time, channel_end) 会把负数钳回 0 → 所有预释放都在 t=0 释放。
        // 初始化到最早的预释放时间点，让 cast_skill 在正确的负数时刻触发。
        let earliest_t = order.iter()
            .map(|&i| req.pre_releases[i].time_before)
            .fold(0.0f64, f64::max);
        player.current_time = -earliest_t;
        player.channel_end = -earliest_t;
        player.last_cast_time = -earliest_t;

        for &idx in &order {
            let p = &req.pre_releases[idx];
            if p.time_before <= 0.0 { continue; }
            let target = (-p.time_before).max(player.current_time);
            if target > player.current_time {
                let _ = player.process_buff_ticks(player.current_time, target);
                player.current_time = target;
            }
            let ranks = match skill_map.get(p.skill.as_str()) {
                Some(r) => r,
                None => continue,
            };
            let skill = match player.pick_rank(ranks) {
                Some(s) => s,
                None => continue,
            };
            if player.cast_skill(skill, None, None, 0.0, 0.0).is_none() {
                continue;
            }
            let now = player.current_time;
            if !skill.passive { player.start_swing(now); }
            let _ = scripts::run_scripts(&mut player, skill, now);
            player.bump_decision_gen();
        }

        // 推进到 t=0（让 buff tick / 平砍 / DoT 在 -t..0 之间正常推进，事件丢弃）
        if player.current_time < 0.0 {
            let _ = player.process_buff_ticks(player.current_time, 0.0);
        }
        player.current_time = 0.0;
        // channel_end/last_cast_time 可能停留在负数（GCD 在 t<0 就结束了），钳到 ≥0；
        // 若 GCD 跨过 t=0（如 t=-0.3 释放 1.5s GCD → channel_end=1.2），保留让主循环正确等待
        player.channel_end = player.channel_end.max(0.0);
        player.last_cast_time = 0.0;
        // 预释放期间产生的 buff_events 都在负数时间，前端时间轴 / 战斗记录不应显示。
        // 清空后给当前存活的 buff/debuff 补 t=0 的 "gain" 事件，让时间轴正确渲染。
        player.buff_events.clear();
        if !player.lite_mode {
            let state = snapshot_event_state(&player);
            for inst in &player.active_buffs {
                player.buff_events.entry(inst.buff_id).or_default()
                    .push((0.0, "gain".into(), state.clone()));
            }
            for inst in &player.target_buffs {
                player.buff_events.entry(inst.buff_id).or_default()
                    .push((0.0, "gain".into(), state.clone()));
            }
        }
        is_first_main = false;
        last_cast_time = 0.0;
        prev_time = 0.0;
    }

    let t_setup = sim_start.elapsed();
    let t_loop_start = std::time::Instant::now();

    for (seq_idx, name) in req.sequence.iter().enumerate() {
        let name: &str = name.as_str();

        // 切姿态：跳过盾飞延迟 buff 剩余时间
        if name == "__切体态延迟中__" {
            if let Some(delay) = player.active_buffs.iter().find(|b| b.buff_id == BUFF_DUN_FEI_DELAY) {
                let target_time = delay.expires_at;
                if target_time > prev_time {
                    let mut tick_events = player.process_buff_ticks(prev_time, target_time);
                    fill_tick_events(&mut tick_events, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
                    timeline.extend(tick_events);
                    prev_time = target_time;
                    player.current_time = target_time;
                }
            }
            continue;
        }

        // 战绝回怒：推进到 战绝 buff 下一跳 tick 时刻（+100 怒）
        if name == "__战绝回怒__" {
            if let Some(zj) = player.active_buffs.iter().find(|b| b.buff_id == BUFF_ZHAN_JUE) {
                if let Some(def) = player.buff_def(BUFF_ZHAN_JUE) {
                    let start = zj.expires_at - frames_to_sec(zj.duration_frames);
                    let tick_sec = frames_to_sec(def.tick_interval);
                    let target_time = if tick_sec > 0.0 {
                        let elapsed = (player.current_time - start).max(0.0);
                        let k = (elapsed / tick_sec).floor();
                        (start + (k + 1.0) * tick_sec).min(zj.expires_at)
                    } else { zj.expires_at };
                    if target_time > prev_time {
                        let mut tick_events = player.process_buff_ticks(prev_time, target_time);
                        fill_tick_events(&mut tick_events, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
                        timeline.extend(tick_events);
                        prev_time = target_time;
                        player.current_time = target_time;
                        if target_time > last_cast_time { last_cast_time = target_time; }
                    }
                }
            }
            continue;
        }

        // __clearCD__:技能名 — 清除指定技能的 CD 和恢复充能（不含 GCD/保护性 CD）
        if let Some(target) = name.strip_prefix("__clearCD__:") {
            // 清除 cd_ 前缀的技能 CD
            let cd_key = format!("cd_{}", target);
            player.active_cds.remove(&cd_key);
            // 恢复充能到满
            if let Some(spec) = skill_map.get(target).and_then(|v| v.first()) {
                if spec.max_charges > 0 {
                    let max_ch = player.effective_max_charges(spec);
                    player.charges.insert(spec.skill_id, (max_ch, 0.0));
                }
            }
            // 盾压特殊：同步 DunyaCdState
            if target == "盾压" {
                if let Some(ref mut d) = player.dunya_cd {
                    d.cd_remain = 0.0;
                    d.avail_credit = 1.0;
                }
            }
            continue;
        }

        // __macro__: 内联宏评估，跑 1 步
        if name == "__macro__" {
            if let Some(ref config) = macro_config {
                // 从 timeline 取 last_skill（若 macro_last_skill 已有则用它）
                if macro_last_skill.is_none() {
                    macro_last_skill = timeline.iter().rev()
                        .find(|e| !e.triggered)
                        .map(|e| e.name.split('·').next().unwrap_or(&e.name).to_string());
                }
                let (macro_tl, debug) = macro_eval::simulate_macro(
                    config, &mut player, &skill_map,
                    1, req.macro_duration.unwrap_or(3600.0), delay_sec,
                    &mut prev_time, &mut is_first_main, macro_last_skill.clone(),
                    dmg_ctx.as_ref(), recipes_table, &skill_by_id,
                    &req.pauses,
                );
                if let Some(last) = macro_tl.iter().rev().find(|e| !e.triggered) {
                    macro_last_skill = Some(last.name.split('·').next()
                        .unwrap_or(&last.name).to_string());
                }
                // 宏模拟的"战斗用时"按实际模拟推进到的时刻算（含末尾空闲/channel），
                // 但 clamp 到 macro_duration —— 否则 simulate_macro break 时 current_time 略超 max_duration
                // 会让 fight_time 比固化路径（fight_time = macro_duration）多 0.x 秒。
                last_cast_time = player.current_time.min(req.macro_duration.unwrap_or(f64::INFINITY));
                timeline.extend(macro_tl);
                macro_debug.extend(debug);
            }
            continue;
        }

        if let Some(ranks) = skill_map.get(name) {
            // 手动技能打断引导（先清 channel_end 让 est_time 基于 GCD）
            let was_channeling = player.channel_end > player.current_time + 0.001;
            let original_channel_end = player.channel_end;
            let channel_skill_id = player.channel_skill_id;
            if was_channeling {
                player.channel_end = player.current_time;
            }
            // 第一次 pick（buff tick 前）
            let skill = match player.pick_rank(ranks) {
                Some(s) => s,
                None => {
                    // 盾飞延迟等待：技能需要刀姿态但 GCD 不覆盖延迟 → 推进到延迟到期后重试
                    let has_blade_rank = ranks.iter().any(|s| s.stance == Stance::Blade);
                    let delay_end = player.active_buffs.iter()
                        .find(|b| b.buff_id == BUFF_DUN_FEI_DELAY && b.expires_at > player.current_time)
                        .map(|b| b.expires_at);
                    if has_blade_rank && delay_end.is_some() {
                        let target = delay_end.unwrap();
                        let mut tick_events = player.process_buff_ticks(player.current_time, target);
                        fill_tick_events(&mut tick_events, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
                        timeline.extend(tick_events);
                        prev_time = target;
                        player.current_time = target;
                        // 姿态延迟是不可调节的硬等待，纳入 last_cast_time 让 cast_skill
                        // 的 base_time 包含它 → cd_wait 不会把这段等待计入（不显示红色角标）
                        if target > last_cast_time { last_cast_time = target; }
                        if target > player.last_cast_time { player.last_cast_time = target; }
                        match player.pick_rank(ranks) {
                            Some(s) => s,
                            None => {
                                if was_channeling { player.channel_end = original_channel_end; }
                                let reasons: Vec<String> = ranks.iter()
                                    .filter_map(|s| player.reject_reason(s))
                                    .collect();
                                let reason = if reasons.is_empty() { "未知原因".into() }
                                    else { reasons.join("; ") };
                                skipped.push((seq_idx, reason));
                                continue;
                            }
                        }
                    } else {
                        if was_channeling { player.channel_end = original_channel_end; }
                        let reasons: Vec<String> = ranks.iter()
                            .filter_map(|s| player.reject_reason(s))
                            .collect();
                        let reason = if reasons.is_empty() { "未知原因".into() }
                            else { reasons.join("; ") };
                        skipped.push((seq_idx, reason));
                        continue;
                    }
                }
            };

            let raw_timing = req.timing_offsets
                .get(&seq_idx.to_string())
                .copied();
            // -1 = "跟随最大值"标记：用户选了 GCD 窗口末尾，加速变化时自动跟随
            let is_timing_max = raw_timing.map_or(false, |t| t < 0.0);
            let timing_offset = if is_timing_max { None } else { raw_timing.filter(|&t| t > 0.0) };

            // GCD 窗口（非主 GCD 技能可在此窗口内偏移释放时间）
            let auto_max_offset = if !skill_is_main(skill) {
                let earliest = player.next_cast_time(skill);
                let gcd_end = player.active_cds.iter()
                    .filter(|(k, _)| k.starts_with("gcd_"))
                    .map(|(_, &v)| v)
                    .fold(0.0_f64, f64::max);
                (gcd_end - earliest).max(0.0)
            } else { 0.0 };
            // -1 = "跟随最大值"：非主 GCD 技能自动取 GCD 窗口末尾
            let timing_offset = if is_timing_max && auto_max_offset > 0.01 {
                Some(auto_max_offset)
            } else { timing_offset };

            let skill_delay = if skill_is_main(skill) && !is_first_main { delay_sec } else { 0.0 };

            let est_time = player.estimate_cast_time(skill, timing_offset, skill_delay);

            // 打断引导：用 est_time 算实际跳数 + 修正 timeline/怒气
            if was_channeling && est_time < original_channel_end - 0.001 {
                let (old_ticks, actual_ticks) = player.interrupt_channel(est_time);
                if let Some(ev) = timeline.iter_mut().rev().find(|e| e.skill_id == channel_skill_id && !e.triggered) {
                    ev.channel_ticks = Some(actual_ticks);
                    let interval = frames_to_sec(player.channel_interval_frame);
                    let first = frames_to_sec(player.channel_first_frame);
                    ev.channel_duration = Some(if actual_ticks <= 1 { first } else {
                        first + (actual_ticks - 1) as f64 * interval
                    });
                }
                if channel_skill_id == 13048 && player.stance() == Stance::Shield {
                    let over_rage = (old_ticks as i32 - actual_ticks as i32).max(0);
                    player.add_rage(-over_rage);
                }
            }

            // cd_wait 计算：恢复 channel_end 让 cast_skill 正确算 base_time
            if was_channeling {
                player.channel_end = original_channel_end;
            }

            // 盾飞延迟：记录到期时间（process_buff_ticks 会消耗 buff，cast_skill 里查不到）
            let stance_delay_end = if skill.stance == Stance::Blade {
                player.active_buffs.iter()
                    .find(|b| b.buff_id == BUFF_DUN_FEI_DELAY && b.expires_at > player.current_time)
                    .map(|b| b.expires_at)
            } else { None };

            let mut tick_events = player.process_buff_ticks(prev_time, est_time);
            fill_tick_events(&mut tick_events, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
            timeline.extend(tick_events);
            prev_time = est_time;
            // process_buff_ticks 内部会把 current_time 推到中间 tick/hit 时刻；显式 set 到 est_time
            // 让后续 cast_skill 内的 next_cast_time 用 est_time 当 base，避免 timing_offset 被重复叠加
            player.current_time = est_time;

            // buff expire 后重新 pick
            let skill = match player.pick_rank(ranks) {
                Some(s) => s,
                None => {
                    let reason = ranks.iter()
                        .filter_map(|s| player.reject_reason(s))
                        .next()
                        .unwrap_or_else(|| "Buff到期后状态不满足".into());
                    skipped.push((seq_idx, reason));
                    continue;
                }
            };
            // combo_follow：序列写阵云结晦，按连招状态自动重定向到二/三段
            let skill = resolve_combo_follow(skill, &player, &skill_by_id).unwrap_or(skill);
            let override_ticks = req.channel_ticks
                .get(&seq_idx.to_string())
                .copied()
                .filter(|&t| t > 0);
            let state_before = if req.lite { None } else { Some(snapshot_event_state(&player)) };
            let rage_before = player.rage;
            // current_time 已 set 到 est_time（含 offset + 网络延迟）；cast_skill 不再传 offset / delay，
            // 否则 cast_time = (est_time) + offset/delay 会被重复叠加。
            let (cast_time, mut cd_wait, ch_ticks, ch_max, ch_dur, _applied_offset_unused, _ret_max_offset_unused) =
                match player.cast_skill(skill, override_ticks, None, 0.0, 0.0) {
                Some(r) => {
                    // cast_time 可能 > est_time；补一次 process_buff_ticks 覆盖缝隙
                    if player.current_time > prev_time {
                        let mut gap = player.process_buff_ticks(prev_time, player.current_time);
                        if !gap.is_empty() {
                            fill_tick_events(&mut gap, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
                        }
                        timeline.extend(gap);
                        prev_time = player.current_time;
                    }
                    r
                }
                None => {
                    let reason = player.reject_reason(skill).unwrap_or_else(|| "释放条件不满足".into());
                    skipped.push((seq_idx, reason));
                    continue;
                }
            };
            // 姿态延迟等待不计入 cd_wait（不可调节的硬约束）
            if let Some(sde) = stance_delay_end {
                let delay_portion = (sde - (cast_time - cd_wait)).max(0.0);
                cd_wait = (cd_wait - delay_portion).max(0.0);
            }
            // 网络延迟不计入 cd_wait（已知固定开销，不显示红色角标）
            cd_wait = (cd_wait - skill_delay).max(0.0);
            if skill_is_main(skill) { is_first_main = false; }
            // 启动卷雪刀循环（首次主动技能）
            if !skill.passive { player.start_swing(cast_time); }

            let qijin_snap = if skill.skill_id == 90001 {
                Some(snapshot_buff_list(&player.active_buffs, player.current_time, false, player.version)
                    .into_iter().filter(|b| !b.is_debuff).collect::<Vec<_>>())
            } else { None };

            let cost = player.last_rage_cost;
            // 释放前根据技能/怒气 决定瞬时附加的秘籍（如绝刀怒气段，破招段共享）
            let runtime_recipes = compute_runtime_recipes(skill, &player);
            // 释放主动技能的伤害（在脚本前算，使用当前 buff 状态）
            let (dmg, dmg_normal, dmg_crit, dmg_total, rt_snap) = if let Some((ref a, ref t)) = dmg_ctx {
                let ticks = ch_ticks.unwrap_or(1);
                let (r, dt, rt) = calc_event_damage(skill, a, t, &player, &runtime_recipes, recipes_table, ticks);
                (Some(r.expected_damage), Some(r.normal_damage), Some(r.crit_damage), Some(dt), Some(rt))
            } else { (None, None, None, None, None) };
            // 执行脚本（盾舞怒气回复、狂绝返还、emit 破招段等）
            let em = scripts::run_scripts(&mut player, skill, cast_time);
            // 脚本里可能直写 player.rage / inst.stacks 等绕过 helper —— 边界 bump 兜底
            player.bump_decision_gen();
            let extra = em.events;
            let primary_override = em.primary_override;
            // 蔑视奇穴 39045：伤害招式命中后获得蔑视 buff（下一招式无视 50% 外功防御）
            // 模拟器假设自身气血% > 目标气血% 条件总是满足
            if dmg_total.unwrap_or(0.0) > 0.0 && player.has_talent(39045) {
                player.add_buff(BUFF_MIE_SHI);
            }
            // 斩刀：添加/刷新流血后抓 DoT 快照（绑到流血 buff 实例）
            if skill.skill_id == 13054 {
                if let Some((ref a, _)) = dmg_ctx {
                    let snap = capture_dot_snapshot(a, &player, 13054, "斩刀", recipes_table);
                    if let Some(inst) = player.target_buffs.iter_mut().find(|b| b.buff_id == BUFF_LIU_XUE) {
                        inst.snapshot = Some(snap);
                    }
                }
            }
            let rage_delta = player.rage - rage_before;
            let has_rage_effect = rage_delta != 0 || cost > 0 || skill.rage_gain > 0;

            // 主事件重写：阵云母技能等按 override 确定名称/伤害
            let (event_name, final_dmg, final_dmg_n, final_dmg_c, final_dmg_t, final_rt, eff_recipes) =
                if let Some((ref ov_name, ov_id)) = primary_override {
                    if let (Some((ref a, ref t_cfg)), Some(ov_spec)) =
                        (&dmg_ctx, skill_by_id.get(&ov_id).copied())
                    {
                        let ticks = ch_ticks.unwrap_or(1);
                        let ov_recipes = compute_runtime_recipes(ov_spec, &player);
                        let (r, dt, rt2) = calc_event_damage(ov_spec, a, t_cfg, &player, &ov_recipes, recipes_table, ticks);
                        (ov_name.clone(), Some(r.expected_damage), Some(r.normal_damage), Some(r.crit_damage), Some(dt), Some(rt2), ov_recipes)
                    } else {
                        (ov_name.clone(), dmg, dmg_normal, dmg_crit, dmg_total, rt_snap, runtime_recipes.clone())
                    }
                } else if skill.skill_id == 13055 {
                    let name = if cost == 0 { "绝刀·免耗".to_string() } else { format!("绝刀·{}怒", cost) };
                    (name, dmg, dmg_normal, dmg_crit, dmg_total, rt_snap, runtime_recipes.clone())
                } else {
                    (skill.name.clone(), dmg, dmg_normal, dmg_crit, dmg_total, rt_snap, runtime_recipes.clone())
                };

            timeline.push(CastEvent {
                name: event_name,
                skill_id: skill.skill_id,
                cast_time,
                triggered: false,
                gcd: skill_gcd(skill),
                is_main: skill_is_main(skill),
                cd_wait,
                channel_ticks: ch_ticks,
                max_channel_ticks: ch_max,
                channel_duration: ch_dur,
                timing_offset: timing_offset.filter(|&o| o > 0.001),
                max_timing_offset: if auto_max_offset > 0.01 { Some(auto_max_offset) } else { None },
                available_buffs: qijin_snap,
                is_macro: false,
                rage_after: if has_rage_effect { Some(player.rage) } else { None },
                rage_delta: if has_rage_effect { Some(rage_delta) } else { None },
                rage_cost: if cost > 0 { Some(cost) } else { None },
                state_before,
                state_after: if req.lite { None } else { Some(snapshot_event_state(&player)) },
                damage: final_dmg,
                damage_normal: final_dmg_n,
                damage_crit:   final_dmg_c,
                damage_total: final_dmg_t,
                runtime_recipes: if req.lite { Vec::new() } else { eff_recipes.clone() },
                runtime_stats: if req.lite { None } else { final_rt.clone() },
                override_attack_coeff: None,
                applied_recipes: if req.lite { Vec::new() } else {
                    // 收集本次主事件实际激活的秘籍 ID（含奇穴常驻、buff 激活、装备激活、套装、用户配）
                    let base_name = skill.name.split('·').next().unwrap_or(&skill.name);
                    collect_recipes_indexed(&player, skill.skill_id, base_name, &eff_recipes, recipes_table)
                        .iter().map(|r| r.id).collect()
                },
            });
            // 给触发事件（脚本 emit 出来的）补伤害；破招段共享子技能的秘籍
            for mut ev in extra {
                fill_event_damage(&mut ev, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player, &eff_recipes);
                timeline.push(ev);
            }
            if skill.skill_id == 90001 {
                if let Some(&buff_id) = req.qijin_buffs.get(&seq_idx.to_string()) {
                    let mut em = ScriptEmitter::new();
                    if let Some(script) = scripts::get_buff_on_remove(&player, buff_id) {
                        script(&mut player, &mut em, cast_time);
                    }
                    player.remove_buff(buff_id);
                    timeline.extend(em.events);
                }
            }
            let mut advance_events = player.flush_advance();
            fill_tick_events(&mut advance_events, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
            timeline.extend(advance_events);
            prev_time = player.current_time;
            last_cast_time = player.current_time;
            // 手动技能释放后重置 macro_last_skill
            macro_last_skill = Some(skill.name.split('·').next()
                .unwrap_or(&skill.name).to_string());
        } else {
            skipped.push((seq_idx, format!("未知技能: {}", name)));
        }
    }

    let t_loop = t_loop_start.elapsed();
    let t_post_start = std::time::Instant::now();

    // ── 共享后处理 ──
    // fight_time 取 max(自然结束时刻, macro_duration)：
    //   - 普通序列：macro_duration 通常未传 → 用 fight_end (= max(channel_end, last_cast))
    //   - 纯宏路径：fight_end 已含末尾推进；macro_duration 兜底确保至少跑到用户指定时长
    //   - 固化复刻：前端传 macro_duration（跟原宏一致）→ 跑到同一时刻 → 卷雪刀/buff tick 范围一致
    let fight_time = {
        let base = player.fight_end(last_cast_time);
        match req.macro_duration {
            Some(md) if md > base => md,
            _ => base,
        }
    };
    // ── buff 区快照：在 process_buff_ticks(...,fight_time) 之前取 ──
    // 否则推进到 fight_time（如 300s）后所有非永久 buff 都过期，buff 列表只剩永久站立 buff（如大附魔）。
    // 此时 player.current_time 还是 prev_time（最后一次主动 cast 完成时刻），刚好是用户期望的 buff 状态。
    let buffs_snapshot = if req.lite { Vec::new() } else { snapshot_buffs(&player) };

    let mut final_ticks = player.process_buff_ticks(prev_time, fight_time);
    fill_tick_events(&mut final_ticks, &skill_by_id, dmg_ctx.as_ref(), recipes_table, &player);
    timeline.extend(final_ticks);
    player.current_time = fight_time; // 推进到战斗结束时刻，确保 skill_cds 等计算正确

    let macro_next_skill = None;

    // println!(
    //     "[simulate] haste={} seq={} → fight_time={:.3}s skills={}",
    //     req.haste_level, req.sequence.len(), fight_time, timeline.len()
    // );

    // Lite 模式：跳过所有"为 UI 服务"的后处理（buff 列表 / available / cd / charges / combo_states / skill_effective / buff_timeline）。
    // 只保留 dps/total/fight_time/skill_count。
    // 非 lite 用前面在 process_buff_ticks(...,fight_time) 之前取的 buffs_snapshot（最后一次主动 cast 时刻的 buff 状态）
    let buffs = buffs_snapshot;

    // 计算当前状态下可施展的技能
    let available_skills: Vec<String> = if req.lite { Vec::new() } else {
        skill_map.iter()
            .filter(|(_, ranks)| player.pick_rank(ranks).is_some())
            .map(|(&name, _)| name.to_string())
            .collect()
    };

    // 各技能剩余 CD（含充能技能的下一层恢复时间）
    let mut skill_cds: HashMap<String, f64> = HashMap::new();
    let cur_t = player.current_time;
    if !req.lite {
    for (&base_name, ranks) in &skill_map {
        for skill in ranks {
            // 充能技能：显示下一层恢复时间
            if skill.max_charges > 0 {
                let remaining = player.charge_remaining(skill);
                if remaining > 0.01 {
                    skill_cds.insert(base_name.to_string(), remaining);
                }
            } else {
                // 非充能技能：显示技能 CD
                for cd in &skill.cooldowns {
                    if cd.cd_id.starts_with("gcd_") { continue; }
                    if let Some(&expires) = player.active_cds.get(&cd.cd_id) {
                        let remaining = (expires - cur_t).max(0.0);
                        if remaining > 0.01 {
                            let entry = skill_cds.entry(base_name.to_string()).or_insert(0.0);
                            if remaining > *entry { *entry = remaining; }
                        }
                    }
                }
            }
            break;
        }
    }

    } // end if !req.lite (skill_cds)

    // 各技能充能层数
    let mut skill_charges: HashMap<String, u32> = HashMap::new();
    if !req.lite {
    for (&base_name, ranks) in &skill_map {
        if let Some(skill) = ranks.first() {
            if skill.max_charges > 0 {
                if let Some(ch) = player.get_charges(skill) {
                    skill_charges.insert(base_name.to_string(), ch);
                }
            }
        }
    }
    } // end if !req.lite (skill_charges)

    // 构建 buff 时间轴轨道
    // 构建 buff 时间轴轨道（仅 show_on_timeline=true，渲染用）
    let mut buff_tracks: Vec<(u32, BuffTimelineTrack)> = if req.lite { Vec::new() } else { player.buff_events.iter()
        .filter_map(|(&bid, events)| {
            let def = player.buff_def(bid)?;
            if !def.show_on_timeline { return None; }
            Some((def.timeline_order, BuffTimelineTrack {
                buff_id: bid,
                name: def.name.to_string(),
                short_name: def.short_name.unwrap_or(def.name).to_string(),
                // 颜色由前端 renderBuffTimeline 按顺序自动生成，这里占位即可
                color: String::new(),
                events: events.iter().map(|(t, ty, st)| BuffTimelineEvent {
                    time: *t, event_type: ty.clone(), state: Some(st.clone()),
                }).collect(),
            }))
        })
        .collect()
    };
    buff_tracks.sort_by_key(|(order, _)| *order);
    let buff_timeline: Vec<BuffTimelineTrack> = buff_tracks.into_iter().map(|(_, t)| t).collect();

    // 战斗记录 log 专用：含所有 buff 事件（包括 show_on_timeline=false 的，如神兵·无双气劲）
    // 前端战斗记录弹窗 (btn_history) 用这个字段；buff_timeline 仅用于轨道渲染
    let mut buff_log_tracks: Vec<(u32, BuffTimelineTrack)> = if req.lite { Vec::new() } else { player.buff_events.iter()
        .filter_map(|(&bid, events)| {
            let def = player.buff_def(bid)?;
            Some((def.timeline_order, BuffTimelineTrack {
                buff_id: bid,
                name: def.name.to_string(),
                short_name: def.short_name.unwrap_or(def.name).to_string(),
                color: String::new(),
                events: events.iter().map(|(t, ty, st)| BuffTimelineEvent {
                    time: *t, event_type: ty.clone(), state: Some(st.clone()),
                }).collect(),
            }))
        })
        .collect()
    };
    buff_log_tracks.sort_by_key(|(order, _)| *order);
    let buff_log: Vec<BuffTimelineTrack> = buff_log_tracks.into_iter().map(|(_, t)| t).collect();

    // 收集 timeline 中出现过的 recipe id，建立 id → 简短名的元信息表
    // 给前端战斗记录 log 显示"已吃 99270 T套+10%"等用
    let recipes_meta: HashMap<u32, String> = if req.lite {
        HashMap::new()
    } else {
        let mut used: ahash::AHashSet<u32> = ahash::AHashSet::new();
        for ev in &timeline {
            for &rid in &ev.applied_recipes {
                used.insert(rid);
            }
        }
        let mut meta = HashMap::new();
        for r in recipes_table.iter() {
            if !used.contains(&r.id) { continue; }
            // 拼简短描述：name + 关键 pct（如有）
            let mut tag = r.name.clone();
            let mut deltas: Vec<String> = Vec::new();
            if r.damage_pct != 0.0 { deltas.push(format!("伤害+{:.0}%", r.damage_pct * 100.0)); }
            if r.critical_pct != 0.0 { deltas.push(format!("会心+{:.1}%", r.critical_pct * 100.0)); }
            if r.crit_eff_pct != 0.0 { deltas.push(format!("会效+{:.1}%", r.crit_eff_pct * 100.0)); }
            if r.surplus_pct != 0.0 { deltas.push(format!("破招+{:.0}%", r.surplus_pct * 100.0)); }
            if r.shield_ignore != 0.0 { deltas.push(format!("无视防御+{:.1}%", r.shield_ignore / 1024.0 * 100.0)); }
            if r.pve_addition != 0.0 { deltas.push(format!("非侠士+{:.0}%", r.pve_addition * 100.0)); }
            if !deltas.is_empty() {
                tag.push_str(" (");
                tag.push_str(&deltas.join("/"));
                tag.push_str(")");
            }
            meta.insert(r.id, tag);
        }
        meta
    };

    // 计算当前剩余 GCD 和总 GCD（key 里的数字是 base duration，需按 haste 缩减得到"实际"时长）
    let (gcd_end, total_gcd_base) = player.active_cds.iter()
        .filter(|(k, _)| k.starts_with("gcd_"))
        .fold((0.0_f64, 0.0_f64), |(best_end, best_total), (k, &v)| {
            if v > best_end {
                let dur = k.strip_prefix("gcd_").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                (v, dur)
            } else {
                (best_end, best_total)
            }
        });
    let total_gcd = if total_gcd_base > 0.0 {
        let base_frames = sec_to_frames(total_gcd_base);
        frames_to_sec(get_actual_frames(base_frames, player.effective_haste_level()))
    } else { 0.0 };
    let remaining_gcd = (gcd_end - player.current_time).max(0.0);

    // 连招状态：哪些技能当前处于连招后续段
    let mut combo_states: HashMap<String, (String, f64)> = HashMap::new();
    if !req.lite {
    for (&base_name, ranks) in &skill_map {
        if let Some(picked) = player.pick_rank(ranks) {
            if let Some(ref req_combo) = picked.requires_combo {
                let bid = combo_buff_id(req_combo);
                let remaining = player.active_buffs.iter()
                    .find(|b| b.buff_id == bid)
                    .map(|b| (b.expires_at - player.current_time).max(0.0))
                    .unwrap_or(0.0);
                combo_states.insert(base_name.to_string(), (picked.name.clone(), remaining));
            }
        }
    }

    // 盾飞延迟切姿态：显示为连招状态
    if let Some(delay) = player.active_buffs.iter().find(|b| b.buff_id == BUFF_DUN_FEI_DELAY) {
        let remaining = (delay.expires_at - player.current_time).max(0.0);
        if remaining > 0.001 {
            combo_states.insert("盾飞".to_string(), ("切体态延迟中".to_string(), remaining));
        }
    }

    // 战绝：怒气不足的刀系技能（斩/绝/劫/闪）显示为"战绝回怒中"连招状态
    if let Some(zj) = player.active_buffs.iter().find(|b| b.buff_id == BUFF_ZHAN_JUE) {
        if let Some(def) = player.buff_def(BUFF_ZHAN_JUE) {
            let start = zj.expires_at - frames_to_sec(zj.duration_frames);
            let tick_sec = frames_to_sec(def.tick_interval);
            let remaining = if tick_sec > 0.0 {
                let elapsed = (player.current_time - start).max(0.0);
                let k = (elapsed / tick_sec).floor();
                let next = start + (k + 1.0) * tick_sec;
                (next.min(zj.expires_at) - player.current_time).max(0.0)
            } else { 0.0 };
            for (&base_name, ranks) in &skill_map {
                if !matches!(base_name, "斩刀" | "绝刀" | "劫刀" | "闪刀") { continue; }
                if combo_states.contains_key(base_name) { continue; }
                // 最低档位的有效怒气消耗（绝刀 25怒，其他按 spec）
                let min_cost = ranks.iter()
                    .map(|r| player.effective_rage_cost(r))
                    .min()
                    .unwrap_or(0);
                if (player.rage as u32) < min_cost {
                    combo_states.insert(base_name.to_string(),
                        ("战绝回怒中".to_string(), remaining));
                }
            }
        }
    }

    } // end if !req.lite (combo_states)

    // 奇穴/秘籍修正后的技能数值
    let mut skill_effective: HashMap<String, SkillEffective> = HashMap::new();
    if !req.lite {
    for (&base_name, ranks) in &skill_map {
        if let Some(skill) = ranks.first() {
            let eff = SkillEffective {
                max_charges: player.effective_max_charges(skill),
                charge_cd: player.effective_charge_cd(skill),
                rage_cost: player.effective_rage_cost(skill),
            };
            // 只返回和原始值不同的
            if eff.max_charges != skill.max_charges || (eff.charge_cd - skill.charge_cd).abs() > 0.001 || eff.rage_cost != skill.rage_cost {
                skill_effective.insert(base_name.to_string(), eff);
            }
        }
    }
    } // end if !req.lite (skill_effective)

    // 汇总伤害和 DPS
    let total_damage: f64 = timeline.iter()
        .filter_map(|e| e.damage_total)
        .sum();
    let dps = if fight_time > 0.001 { total_damage / fight_time } else { 0.0 };
    let skill_count = timeline.len();
    // ★ 在清空 timeline 之前算 fingerprint（lite 模式照样要返回它做差分校验）
    let fingerprint = compute_fingerprint(&timeline);
    // Lite 模式：默认清空 timeline（避免序列化数千事件）；若 lite_keep_timeline 则保留
    // （蒸馏工作流：验证/剪枝/swap 需要事件 .name 做 cast count，但不需要 hover 详情）
    if req.lite && !req.lite_keep_timeline { timeline.clear(); }

    let t_post = t_post_start.elapsed();
    let sim_elapsed = sim_start.elapsed();
    // Lite 模式不打印 perf：批量场景（蒸馏/配装搜索/batch_simulate）一次跑成千上万次 simulate，
    // 每次 ~17 行 println 把 conhost 渲染撑爆，stdout write 阻塞 → 整个 backend 卡住。
    // 单跑 Full 模式继续保留输出方便开发观察。
    if !req.lite {
        println!("[sim_core] total={:.1}ms (setup={:.1}ms loop={:.1}ms post={:.1}ms) fight={:.1}s events={} fp={:016x}",
            sim_elapsed.as_secs_f64() * 1000.0,
            t_setup.as_secs_f64() * 1000.0,
            t_loop.as_secs_f64() * 1000.0,
            t_post.as_secs_f64() * 1000.0,
            fight_time, skill_count, fingerprint);
        SIM_PERF.with(|p| {
            let p = *p.borrow();
            let ms = |ns: u64| ns as f64 / 1_000_000.0;
            println!("  ├─ snapshot:       {:>5} calls, {:>6.2}ms",  p.snapshot_n, ms(p.snapshot_ns));
            println!("  ├─ run_scripts:    {:>5} calls, {:>6.2}ms",  p.run_scripts_n, ms(p.run_scripts_ns));
            println!("  ├─ buff_ticks:     {:>5} calls, {:>6.2}ms",  p.buff_ticks_n, ms(p.buff_ticks_ns));
            println!("  ├─ collect_recipes:{:>5} calls, {:>6.2}ms",  p.collect_recipes_n, ms(p.collect_recipes_ns));
            println!("  ├─ aggregate:      {:>5} calls, {:>6.2}ms (cache hit={} miss={})",
                p.aggregate_n, ms(p.aggregate_ns), p.cache_hit, p.cache_miss);
            println!("  ├─ calc_damage:    {:>5} calls, {:>6.2}ms",  p.calc_damage_n, ms(p.calc_damage_ns));
            println!("  ├─ fill_event:     {:>5} calls, {:>6.2}ms",  p.fill_event_n, ms(p.fill_event_ns));
            println!("  ├─ fill_tick:      {:>5} calls, {:>6.2}ms",  p.fill_tick_n, ms(p.fill_tick_ns));
            println!("  ├─ macro_eval:     {:>5} calls, {:>6.2}ms",  p.macro_eval_n, ms(p.macro_eval_ns));
            println!("  │  ├─ macro_phase1:  {:>5} calls, {:>6.2}ms",  p.macro_phase1_n, ms(p.macro_phase1_ns));
            println!("  │  ├─ macro_phase2:  {:>5} calls, {:>6.2}ms",  p.macro_phase2_n, ms(p.macro_phase2_ns));
            println!("  │  ├─ skill_lookup:  {:>5} calls, {:>6.2}ms",  p.macro_skill_lookup_n, ms(p.macro_skill_lookup_ns));
            println!("  │  ├─ macro_advance: {:>5} calls, {:>6.2}ms",  p.macro_advance_n, ms(p.macro_advance_ns));
            println!("  │  │  ├─ next_decision: {:>5} calls, {:>6.2}ms", p.macro_adv_next_n, ms(p.macro_adv_next_ns));
            println!("  │  │  ├─ buff_ticks:    {:>5} calls, {:>6.2}ms", p.macro_adv_ticks_n, ms(p.macro_adv_ticks_ns));
            println!("  │  │  └─ fill_tick:     {:>5} calls, {:>6.2}ms", p.macro_adv_fill_n, ms(p.macro_adv_fill_ns));
            println!("  │  └─ macro_cond:    {:>5} calls, {:>6.2}ms",  p.macro_cond_n, ms(p.macro_cond_ns));
            println!("  └─ cast_skill:     {:>5} calls, {:>6.2}ms",  p.cast_skill_n, ms(p.cast_skill_ns));
        });
    }

    SimulateResponse {
        fight_time,
        skill_count,
        stance: player.stance(),
        rage: player.rage,
        block_value: if player.mount == Mount::TieGuYi { Some(player.block_value) } else { None },
        max_block_value: if player.mount == Mount::TieGuYi { Some(player.max_block_value()) } else { None },
        timeline,
        buffs,
        available_skills,
        skipped,
        skill_cds,
        skill_charges,
        buff_timeline,
        buff_log,
        recipes_meta,
        damage_add_buff_ids: if req.lite { Vec::new() } else { crate::scripts::collect_damage_add_buff_ids(player.version) },
        initial_buffs,
        buff_attr_keys: if req.lite { HashMap::new() } else {
            crate::scripts::collect_buff_attr_keys(player.version)
                .into_iter()
                .map(|(id, keys)| (id, keys.into_iter().map(|s| s.to_string()).collect()))
                .collect()
        },
        buff_attr_desc: if req.lite { HashMap::new() } else {
            crate::scripts::collect_buff_attr_desc(player.version)
        },
        formation_attr_keys: if req.lite { Vec::new() } else {
            // 收集当前阵法 permanent_slots 中字段影响的 attr_keys
            let mut keys: Vec<String> = Vec::new();
            for (field, _) in &player.formation_permanent_slots {
                for k in crate::scripts::affected_attr_keys(*field) {
                    let s = (*k).to_string();
                    if !keys.contains(&s) { keys.push(s); }
                }
            }
            keys
        },
        talent_attr_keys: if req.lite { HashMap::new() } else {
            // 仅奇穴 hardcode 增益部分（aggregate_buff_fields 内硬编码激活的）
            //   13124 活血：VitalityBasePercentAdd +10%  → 仅列在 vit/hp（招架/拆招/攻击靠转化间接得到，不重复列）
            //   13366 从容：PhysicsAttackPowerPercent +10% + StrainBasePercentAdd +15% → atk/strain（直接加副属性）
            // 其他奇穴通过激活秘籍/buff 间接影响，不在此列
            let mut m: HashMap<u32, Vec<String>> = HashMap::new();
            if player.has_talent(13124) {
                m.insert(13124, vec!["vit".into(), "hp".into()]);
            }
            if player.has_talent(13366) {
                m.insert(13366, vec!["atk".into(), "strain".into()]);
            }
            m
        },
        remaining_gcd,
        total_gcd,
        combo_states,
        skill_effective,
        macro_debug: Vec::new(), // 不发送 debug 数据（6MB+ 序列化开销）
        macro_next_skill,
        total_damage,
        initial_stats,
        dps,
        expectation: ExpectationSnapshot::from_player(&player),
        fingerprint,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 配装器 API
// ─────────────────────────────────────────────────────────────────────────────

async fn equip_search(
    State(state): State<SharedState>,
    Json(filter): Json<equip::SearchFilter>,
) -> impl IntoResponse {
    let items = equip::search_items(&state.equip_data, &filter);
    Json(items)
}

#[derive(Deserialize)]
struct EquipIdRequest {
    id: u32,
    /// 必填：id 跨表不唯一，必须指明部位（前端对应 POS_TO_SUB 映射）
    sub_type: u8,
}

async fn equip_detail(
    State(state): State<SharedState>,
    Json(req): Json<EquipIdRequest>,
) -> impl IntoResponse {
    match equip::get_detail(&state.equip_data, req.sub_type, req.id) {
        Some(d) => Json(serde_json::json!({"ok": true, "detail": d})),
        None => Json(serde_json::json!({"ok": false})),
    }
}

#[derive(Deserialize)]
struct SubTypeRequest { sub_type: i32 }

async fn equip_enhances(
    State(state): State<SharedState>,
    Json(req): Json<SubTypeRequest>,
) -> impl IntoResponse {
    Json(equip::get_enhances(&state.equip_data, req.sub_type))
}

async fn equip_enchants(
    State(state): State<SharedState>,
    Json(req): Json<SubTypeRequest>,
) -> impl IntoResponse {
    Json(equip::get_enchants_for(&state.equip_data, req.sub_type))
}

#[derive(Deserialize, Default)]
struct StoneQuery {
    #[serde(default)]
    selectors: Vec<String>,
}

async fn equip_stones(
    State(state): State<SharedState>,
    Json(req): Json<StoneQuery>,
) -> impl IntoResponse {
    Json(equip::get_stones(&state.equip_data, &req.selectors))
}

async fn equip_calculate(
    State(state): State<SharedState>,
    Json(req): Json<equip::CalcRequest>,
) -> impl IntoResponse {
    // 心法固定增益 + 心法转化 都从 state (school.toml) 注入。req.mount 字段仅作记录，不再决定数值。
    let bs = state.base_stats.read().await.clone();
    let mc = state.mount_conversions.read().await.clone();
    Json(equip::calculate(&state.equip_data, &req, &bs, &mc))
}

async fn equip_meta(State(state): State<SharedState>) -> impl IntoResponse {
    // 返回数据元信息：装备数量、心法分布、品级范围等
    let data = &state.equip_data;
    let mut schools: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut kinds: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut min_level = u32::MAX;
    let mut max_level = 0u32;

    for item in data.items.values() {
        if !item.belong_school.is_empty() { schools.insert(item.belong_school.clone()); }
        if !item.magic_kind.is_empty() { kinds.insert(item.magic_kind.clone()); }
        if item.level < min_level { min_level = item.level; }
        if item.level > max_level { max_level = item.level; }
    }

    Json(serde_json::json!({
        "total_items": data.items.len(),
        "total_stones": data.stones.len(),
        "total_sets": data.sets.len(),
        "schools": schools,
        "kinds": kinds,
        "min_level": if data.items.is_empty() { 0 } else { min_level },
        "max_level": max_level,
    }))
}

#[derive(Debug, Deserialize)]
pub struct HasteTiersQuery {
    /// 基底帧数。常见取值：24（1.5s GCD，主循环）、16（1.0s GCD，少量短GCD技能）、8（0.5s 引导跳）。
    /// 不传则同时返回 24/16 两套（最常用）。
    #[serde(default)]
    pub original_frames: Option<u32>,
    /// 计算上限 haste。默认 = 游戏内基础加速 25% 截断点（`(0.25 * LP_HASTE) as u32 = 52519`）；
    /// 超过该值不再缩短帧数（`Player::effective_haste_level` 会 clamp）。前端可不传走默认。
    #[serde(default)]
    pub cap: Option<u32>,
}

/// 加速档位边界查询：根据 `get_actual_frames` 公式直接推导每段加速对应的 haste 区间。
/// 配装搜索把"目标加速段"作为硬约束时，前端调一次拿表，把 (min, max) 透传给 auto_optimize。
async fn equip_haste_tiers(
    axum::extract::Query(q): axum::extract::Query<HasteTiersQuery>,
) -> Json<serde_json::Value> {
    // 默认走 25% 自然截断（`Player::effective_haste_level` 也是这么 clamp 的）
    let natural_cap = (0.25 * LP_HASTE) as u32;
    let cap = q.cap.unwrap_or(natural_cap).min(200_000); // 防滥用
    let to_obj = |orig: u32| -> serde_json::Value {
        let tiers: Vec<serde_json::Value> = haste_tier_boundaries(orig, cap).into_iter()
            .map(|(tier, frames, lo, hi)| serde_json::json!({
                "tier": tier,
                "actual_frames": frames,
                "actual_seconds": frames as f64 / FRAMES_PER_SEC as f64,
                "haste_min": lo,
                "haste_max": hi,
            }))
            .collect();
        serde_json::json!({ "original_frames": orig, "tiers": tiers })
    };
    if let Some(o) = q.original_frames {
        Json(to_obj(o))
    } else {
        // 默认返回 24（1.5s GCD）+ 16（1.0s GCD）两组
        Json(serde_json::json!({
            "lp_haste": LP_HASTE,
            "frames_per_sec": FRAMES_PER_SEC,
            "haste_cap": cap,
            "by_base_frames": [to_obj(24), to_obj(16)],
        }))
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 自动配装 (A 阶段：朴素枚举 → 全量 calc → dedup → rayon simulate → top N)
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize)]
pub struct HasteRange { pub min: u32, pub max: u32 }

#[derive(Debug, Deserialize)]
pub struct AutoOptimizeRequest {
    /// 锁定槽位（"HAT"/"JACKET"/.../"PRIMARY_WEAPON"/..."），不参与枚举
    #[serde(default)] pub fixed_slots: HashMap<String, equip::SlotConfig>,
    /// 候选槽位 → equip_id 列表（笛卡尔积枚举），不在此表的部位 = 强制按 fixed_slots 走
    pub candidates: HashMap<String, Vec<u32>>,
    /// 目标 haste 多区间：候选 haste 落入任一区间即通过；空数组 = 无段位约束（fallback 到 [0, cap]）
    pub target_haste: Vec<HasteRange>,
    /// 五彩石 ID（仅主武器有效）
    #[serde(default)] pub stone_id: u32,
    /// 模拟时长，默认 300s
    #[serde(default = "default_auto_duration")] pub duration: f64,
    pub mount: u32,
    #[serde(default)] pub talents: Vec<u32>,
    #[serde(default)] pub recipes: Vec<u32>,
    pub macro_text: String,
    #[serde(default)] pub target: Option<TargetConfig>,
    #[serde(default)] pub network_delay: u32,
    #[serde(default)] pub initial_rage: i32,
    #[serde(default)] pub boss_attack_interval: f64,
    #[serde(default)] pub hanjia_expectation: bool,
    /// 铁骨气劲模式（与模拟器 SimulateRequest.tiegu_mode 同语义；默认 2 = 主T·宿敌）
    #[serde(default = "default_tiegu_mode")] pub tiegu_mode: u8,
    /// 实验性武学开关（与模拟器同字段；默认 false）
    #[serde(default)] pub experimental: bool,
    /// 团队增益启用列表（与模拟器同字段，确保 wzc 候选模拟的 buff 状态与主页面对齐）
    #[serde(default)] pub team_buffs: Vec<TeamBuffSelection>,
    /// 选中的阵法（与模拟器同字段，确保 wzc 候选模拟的阵法状态与主页面对齐）
    #[serde(default)] pub formation: Option<FormationSelection>,
    /// 默认精炼 / 镶嵌模板（候选项 SlotConfig 全部按此模板）
    #[serde(default)] pub default_strength: Option<u8>,
    #[serde(default)] pub default_embedding: Option<Vec<u8>>,
    /// 每部位默认大附魔 ID（用户当前配装继承 + UI 可覆盖；缺省 0 = 无）
    #[serde(default)] pub default_enchants: HashMap<String, u32>,
    /// 每部位默认小附魔 ID（同上）
    #[serde(default)] pub default_enhances: HashMap<String, u32>,
    #[serde(default = "default_top_n")] pub top_n: usize,
    /// 桶内 Pareto 剪枝（按 set/特效 分桶后，做 8 维属性 Pareto）。默认开启。
    /// 期望传播子系统启用时（boss_attack_interval > 0 + 选了寒甲/坚铁奇穴）建议关闭：
    /// 概率链非完全单调，Pareto 可能漏 winner。
    #[serde(default = "default_true")] pub use_pareto: bool,
    /// 8 维属性向量量化 bin 大小（统一应用到 crit/crit_eff/overcome/strain/surplus/attack/agility/strength）。
    /// 原理：等级差异 < bin 时 floor(level/bin) 落同一桶，unique key + Pareto 都视为同点 → 大幅压低 unique 数。
    /// 默认 100：典型场景下与真实 DPS 影响 < 0.1%。设 0/1 = 不量化。
    #[serde(default = "default_bucket_size")] pub bucket_size: u32,

    // ─── Phase A 内置（加速 enhance pair）──────────────
    /// 每加速槽（HAT/SHOES/PRIMARY_WEAPON 等）的紫·急速 enhance ID。
    /// 空 = 该槽不参与加速 enhance 搜索（只有 (装备, 0) 一种 pair 候选）。
    #[serde(default)] pub haste_enhance_ids: HashMap<String, u32>,

    // ─── 梯度排序预筛（Pareto 后、真实 sim 前）─────────────────────────
    /// 取 1 个参考配置 + 9 维属性扰动跑 10 次 sim 得到 ∂DPS/∂attr 梯度，
    /// 对 Pareto 后所有配置算线性预测 DPS，按预测排序取前 K 个进真实 sim。
    /// 0 = 不预筛（全部 sim，慢）；默认 5000 足够覆盖真实 top-N。
    #[serde(default = "default_top_k_proxy")] pub top_k_proxy: usize,

    // ─── Phase B（偏导后处理：每非加速槽 enhance 候选）─────────────────
    /// 非加速槽 enhance 候选池：pos → [enhance_id, ...]。
    /// Phase B 用 9 维属性扰动求 ∂DPS/∂attr，每槽独立 argmax 选最佳 enhance。
    /// 旧字段名保留兼容（前端按外攻/T build 自动生成）。
    #[serde(default)] pub enhance_candidates: HashMap<String, Vec<u32>>,
    /// Phase A → Phase B 切口：取 top-K 个 (装备 + 加速 enhance + stone) 配置进偏导阶段。
    #[serde(default = "default_top_k_phase_b")] pub top_k_phase_b: usize,
    /// 是否对锁定槽（fixed_slots 里的位置）也做 Phase B 偏导附魔搜索。
    /// true（默认）：换了装备时附魔也跟着重新选最优；
    /// false：锁定槽附魔保持用户配装里的不变。
    #[serde(default = "default_search_locked_enhance")] pub search_locked_enhance: bool,

    // ─── 旧字段（已废弃，保留兼容） ──────────────────────────────────
    #[serde(default)] pub stone_candidates: Vec<u32>,
    #[serde(default = "default_top_k_phase_c")] pub top_k_phase_c: usize,
}
fn default_true() -> bool { true }
fn default_auto_duration() -> f64 { 300.0 }
fn default_top_n() -> usize { 10 }
fn default_bucket_size() -> u32 { 500 }
fn default_top_k_phase_b() -> usize { 500 }
fn default_top_k_proxy() -> usize { 5000 }
fn default_search_locked_enhance() -> bool { true }
fn default_top_k_phase_c() -> usize { 1000 }

#[derive(Debug, Serialize, Clone, Default)]
pub struct AutoOptimizeResponse {
    pub feasible: bool,
    pub baseline_dps: f64,
    pub baseline_haste: u32,
    pub top: Vec<AutoTopEntry>,
    pub stats: AutoOptimizeStats,
    pub warnings: Vec<String>,
    /// S6 二次拟合模型（mean_pool / 一阶梯度 / Hessian / 9 维 raw 池子范围）。
    /// 用户在前端展开 top 结果时画属性收益曲线/属性置换矩阵用。
    /// `None` 表示池子样本太少（< 12，自由度不够）跳过了拟合。
    #[serde(default)] pub fit_model: Option<AutoFitModel>,
    /// 拟合质量评价指标（R² / RMSE / MAE / top10 重合）。
    /// `None` 时同 `fit_model` —— 没拟合就没指标。
    #[serde(default)] pub fit_metrics: Option<AutoFitMetrics>,
}

/// S6 拟合输出：DPS ≈ ref + g·Δ + ½·Δᵀ·H·Δ，其中 Δ = raw - mean。
/// 9 维 raw 顺序：[strength, agility, base_attack, weapon_damage,
///                surplus_value, crit_level, crit_effect_level,
///                overcome_level, strain_level]
#[derive(Debug, Serialize, Clone, Default)]
pub struct AutoFitModel {
    /// 拟合参考点（after_pareto 池子的 9 维均值）
    pub mean: [f64; 9],
    /// 参考点处真实 DPS
    pub ref_dps: f64,
    /// 一阶梯度 g[9]
    pub grad: [f64; 9],
    /// 完整 Hessian H[9][9]
    pub hess: [[f64; 9]; 9],
    /// 9 维 raw 在 after_pareto 池子里的最小值（曲线 x 轴左端）
    pub axis_min: [f64; 9],
    /// 同上，最大值（曲线 x 轴右端）
    pub axis_max: [f64; 9],
    /// 9 维属性的中文显示名（前端不维护映射）
    pub axis_labels: Vec<String>,
    /// 池子样本数（mean 的 N）
    pub pool_size: usize,
    /// 各属性最大单附魔等级（遍历 enhance 表全部部位/品级取最大单值）；
    /// 顺序与 axis_labels 一致：[strength, agility, base_attack, weapon_damage,
    /// surplus_value, crit_level, crit_effect_level, overcome_level, strain_level]
    /// 用于"满附魔收益" + 属性置换矩阵的 Δ 取值（按各属性实际可换装幅度对齐）
    #[serde(default)]
    pub max_enhance_levels: [f64; 9],
    /// 全能（atPVXAllRound）的最大单附魔等级 —— 独立维度，前端合成 +0.5 surplus + 1.5 strain
    #[serde(default)]
    pub max_enhance_pvx: f64,
}

/// 拟合质量：用 simulate 阶段的真实 DPS 与 quadratic 预测对照计算
#[derive(Debug, Serialize, Clone, Default)]
pub struct AutoFitMetrics {
    /// 决定系数 R²（1 - SSres/SStot）
    pub r_squared: f64,
    /// 均方根误差（DPS 量纲）
    pub rmse: f64,
    /// 平均绝对误差
    pub mae: f64,
    /// 真实 top10 与预测 top10 的重合数（0..=10）
    pub top10_overlap: u32,
    /// S6 拟合用的 sim 数（固定 55）
    pub fit_samples: u32,
    /// 用于验证的真实 sim 数（即 simulate 阶段跑了多少）
    pub validation_samples: u32,
}

/// 通用属性收益曲线请求：以 attributes 为参考点，9 维各 ±Δ 跑 sim 拟合 quadratic
#[derive(Debug, Deserialize)]
pub struct FitCurveRequest {
    /// 模拟器请求模板：必须含 attributes（基线 raw）+ equipment（含 ENCHANT_*）
    /// + macro_text / talents / recipes / target / 其他参数。fit handler 复制此模板，
    /// 仅替换 attributes 跑各扰动 sim。equipment 不变 → 装备特效在每次 sim 都触发。
    #[serde(flatten)]
    pub sim_req: SimulateRequest,
    /// 拟合阶数：1 = 仅 grad（19 sim），2 = grad + Hessian（55 sim）
    #[serde(default = "default_fit_order")]
    pub order: u8,
    /// 各维度扰动幅度（默认 1000）
    #[serde(default = "default_fit_pert_delta")]
    pub pert_delta: f64,
    /// 坐标轴范围倍数（默认 5，即 raw_a ± 5×Δ）
    #[serde(default = "default_fit_axis_scale")]
    pub axis_scale: f64,
}
fn default_fit_order() -> u8 { 2 }
fn default_fit_pert_delta() -> f64 { 1000.0 }
fn default_fit_axis_scale() -> f64 { 5.0 }

#[derive(Debug, Serialize, Default)]
pub struct FitCurveResponse {
    pub feasible: bool,
    pub fit_model: AutoFitModel,
    /// 真实 sim 跑出来的本配装 DPS（= AutoFitModel.ref_dps；冗余字段方便前端直接读）
    pub ref_dps: f64,
    /// 总 sim 数（= 1 ref + 18 单轴 + 36 交叉 = 55，或 order=1 时 19）
    pub sim_count: u32,
    /// 耗时（毫秒）
    pub elapsed_ms: f64,
    /// 失败原因（feasible=false 时填）
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct AutoTopEntry {
    pub slots: HashMap<String, u32>,        // pos → equip_id（含固定槽与本组合）
    pub names: HashMap<String, String>,     // pos → 装备名
    pub dps: f64,
    pub delta_pct: f64,                     // vs baseline
    pub haste_level: u32,
    pub panel_attack: f64,
    /// 完整面板属性（hover tooltip 显示）；只 top_n 个回填
    #[serde(default)] pub panel: Option<equip::PanelAttrs>,
    /// 完整 RawAttrs 等级（hover tooltip 显示"等级 / 系数"用）；只 top_n 个回填
    #[serde(default)] pub raw: Option<equip::RawAttrs>,
    /// 选中的 enhance 配置（pos → enhance_id）；Phase B 后才有
    #[serde(default)] pub enhances: HashMap<String, u32>,
    /// 五彩石 ID（Phase C 后才有）
    #[serde(default)] pub stone_id: u32,
    /// 五彩石名（仅显示用）
    #[serde(default)] pub stone_name: String,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct AutoOptimizeStats {
    pub candidate_positions: usize,
    pub total_combinations: u64,
    pub after_haste_pruning: u64,
    pub unique_keys: usize,
    pub after_pareto: usize,
    /// 梯度预筛后保留的配置数（= top_k_proxy 或不预筛时 = after_pareto）
    #[serde(default)] pub after_rank: usize,
    /// 是否启用了智能模式（梯度预筛）。前端按此切换"智能模式"/"完整模拟"指示
    #[serde(default)] pub smart_mode: bool,
    /// 主武器是否锁定（fixed_slots 含 PRIMARY_WEAPON）。智能模式下武器未锁定 = 排序可能受紫橙特效非线性影响
    #[serde(default)] pub weapon_locked: bool,
    pub simulated: usize,
    pub time_total_ms: f64,
    pub time_calc_ms: f64,
    pub time_pareto_ms: f64,
    /// 梯度预筛耗时（包括 2 轮共 20 次 sim + 排序）
    #[serde(default)] pub time_rank_ms: f64,
    pub time_simulate_ms: f64,
    // ─── Phase B/C ─────────────────────────
    /// Phase B 起 base 数（top_k_phase_b 实际取到的数量）
    pub phase_b_bases: usize,
    /// Phase B 内总 (base, enhance) 三元组（迭代量）
    pub phase_b_iters: u64,
    /// Phase B bucket dedup 后 unique
    pub phase_b_unique: usize,
    /// Phase B 真正 sim 数
    pub phase_b_simulated: usize,
    pub time_phase_b_ms: f64,
    /// Phase C base (= top_k_phase_c)
    pub phase_c_bases: usize,
    /// Phase C 总迭代（K2 × stones）
    pub phase_c_iters: u64,
    pub phase_c_unique: usize,
    pub phase_c_simulated: usize,
    pub time_phase_c_ms: f64,
}

/// 单件装备的"边际加速等级"贡献：仅 magic + diamond 两条主路径，**不含套装效果**。
/// 用于枚举阶段的 lower-bound 剪枝（再加 SLACK 容下套装套数加成）。
fn item_haste_marginal(item: &equip::EquipItem, cfg: &equip::SlotConfig) -> u32 {
    let mut h: i64 = 0;
    for ma in &item.magics {
        if ma.slot == "atHasteBase" {
            h += ma.value;
            if cfg.strength > 0 {
                let lv = (cfg.strength.min(8)) as usize;
                h += (ma.value as f64 * equip::STRENGTH_P[lv]).round() as i64;
            }
        }
    }
    for (i, ds) in item.diamonds.iter().enumerate() {
        if ds.slot == "atHasteBase" {
            let lv = cfg.embedding.get(i).copied().unwrap_or(0);
            if lv > 0 {
                let coeff = equip::embedding_coeff(lv);
                h += ((ds.base_value as f64).floor() * coeff).floor() as i64;
            }
        }
    }
    h.max(0) as u32
}

/// 每槽位的 enhance haste 范围 [min, max]（atHasteBase 在候选 enhance 里的取值）。
/// 用于 Phase A 的 per-slot widening：装备 haste 范围 + enhance 范围 = 实际可达范围。
fn compute_enhance_haste_range_per_slot(
    candidates: &HashMap<String, Vec<u32>>,
    data: &equip::EquipData,
) -> HashMap<String, (i64, i64)> {
    let mut out = HashMap::new();
    for (pos, ids) in candidates.iter() {
        let sub = equip::pos_to_subtype(pos) as i32;
        let list = match data.enhances.get(&sub) { Some(l) => l, None => continue };
        let hastes: Vec<i64> = ids.iter().filter_map(|&id| {
            list.iter().find(|e| e.id == id).map(|e| {
                e.attributes.iter()
                    .filter(|(s, _)| s == "atHasteBase")
                    .map(|(_, v)| *v).sum::<i64>()
            })
        }).collect();
        let mn = hastes.iter().min().copied().unwrap_or(0);
        let mx = hastes.iter().max().copied().unwrap_or(0);
        out.insert(pos.clone(), (mn, mx));
    }
    out
}

/// 给定 stone_id，算它的 atHasteBase 总值（不依赖 dc/dl 阈值；用于 branch prune 估算上限）

/// 副属性 LP/1024 郭氏阈值（rate-converted 字段的最小有意义差，固定常量）
const BIN_CRIT:     u32 = 193;   // ≈ floor(LP_CRIT / 1024)
const BIN_CRIT_EFF: u32 = 71;    // ≈ floor(LP_CRIT_EFF / 1024)
const BIN_OVERCOME: u32 = 220;   // ≈ floor(LP_OVERCOME / 1024)
const BIN_STRAIN:   u32 = 130;   // ≈ floor(LP_STRAIN / 1024)

/// 把 RawAttrs 折成 8 维比对向量
///   前 4 维（crit/crit_eff/overcome/strain）—— 走郭氏阈值固定 bin（LP/1024）
///   后 4 维（surplus/attack/agility/strength）—— 走 bucket_size（用户可调，默认 100）
///   bucket=0/1 时后 4 维不量化（按整数等级直传）
fn raw_to_pareto_vec(r: &equip::RawAttrs, bucket: u32) -> [u32; 8] {
    let qn = |v: f64, b: u32| -> u32 {
        let x = v as u32;
        if b <= 1 { x } else { x / b }
    };
    [
        (r.crit_level         as u32) / BIN_CRIT,
        (r.crit_effect_level  as u32) / BIN_CRIT_EFF,
        (r.overcome_level     as u32) / BIN_OVERCOME,
        (r.strain_level       as u32) / BIN_STRAIN,
        qn(r.surplus_value,   bucket),
        qn(r.base_attack,     bucket),
        qn(r.agility,         bucket),
        qn(r.strength,        bucket),
    ]
}

/// 哈希键：raw 等级量化后整数化（8 维拼 128 位）+ 套装/特效 fingerprint（64 位）
fn encode_combo_key(raw: &equip::RawAttrs, set_eff_fp: u64, bucket: u32) -> (u128, u64) {
    let v = raw_to_pareto_vec(raw, bucket);
    let mut k: u128 = 0;
    let put = |k: &mut u128, x: u32, shift: u32, width: u32| {
        let mask = (1u128 << width) - 1;
        *k |= ((x as u128) & mask) << shift;
    };
    put(&mut k, v[0], 0,   17);
    put(&mut k, v[1], 17,  17);
    put(&mut k, v[2], 34,  17);
    put(&mut k, v[3], 51,  17);
    put(&mut k, v[4], 68,  17);
    put(&mut k, v[5], 85,  17);
    put(&mut k, v[6], 102, 13);
    put(&mut k, v[7], 115, 13);
    (k, set_eff_fp)
}

/// 把 RawAttrs 映射到 simulate 用的 Attributes
fn raw_to_attributes(r: &equip::RawAttrs) -> Attributes {
    Attributes {
        li_dao:            r.strength,
        shen_fa:           r.agility,
        vitality:          r.vitality,
        gen_gu:             44.0,
        yuan_qi:            44.0,
        base_attack:       r.base_attack,
        base_magical_attack: r.base_magical_attack,
        weapon_damage:     r.weapon_damage,
        surplus_value:     r.surplus_value,
        crit_level:        r.crit_level,
        crit_effect_level: r.crit_effect_level,
        overcome_level:    r.overcome_level,
        strain_level:      r.strain_level,
        haste_level:       r.haste_level,
        parry_value:       r.parry_value,
        parry_level:       r.parry_level,
    }
}

/// 入口：POST /api/equip/auto_optimize
/// 立即返回 `{ok:true, started}`，把实际搜索 spawn 到后台任务。
/// 前端通过 GET /api/equip/auto_optimize/progress 拉进度 + 最终结果（result 字段）。
/// 这样避免长跑 POST 长时间占用 HTTP/1.1 连接，浏览器并发限制不再 starvate polling。
async fn equip_auto_optimize(
    State(state): State<SharedState>,
    Json(req): Json<AutoOptimizeRequest>,
) -> Json<serde_json::Value> {
    use std::sync::atomic::Ordering;
    // 重置 progress 与控制标志
    state.auto_search_cancel.store(false, Ordering::Relaxed);
    state.auto_search_pause.store(false, Ordering::Relaxed);
    {
        let mut p = state.auto_search.lock().unwrap();
        *p = AutoSearchProgress {
            running: true, phase: "starting".into(),
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            ..Default::default()
        };
    }
    let state2 = state.clone();
    tokio::spawn(async move {
        let resp = run_auto_optimize_compute(state2.clone(), req).await;
        let mut p = state2.auto_search.lock().unwrap();
        p.running = false;
        p.phase = "done".into();
        p.result = Some(resp);
    });
    Json(serde_json::json!({ "ok": true, "started": true }))
}

/// 控制：cancel / pause / resume。前端通过 POST /api/equip/auto_optimize/control 调用。
#[derive(Debug, Deserialize)]
pub struct AutoOptimizeControlRequest { pub action: String }

async fn equip_auto_optimize_control(
    State(state): State<SharedState>,
    Json(req): Json<AutoOptimizeControlRequest>,
) -> Json<serde_json::Value> {
    use std::sync::atomic::Ordering;
    match req.action.as_str() {
        "cancel" => {
            state.auto_search_cancel.store(true, Ordering::Relaxed);
            // 取消时清除 pause（避免任务卡在 spin-wait）
            state.auto_search_pause.store(false, Ordering::Relaxed);
            Json(serde_json::json!({ "ok": true, "action": "cancel" }))
        }
        "pause" => {
            state.auto_search_pause.store(true, Ordering::Relaxed);
            Json(serde_json::json!({ "ok": true, "action": "pause" }))
        }
        "resume" => {
            state.auto_search_pause.store(false, Ordering::Relaxed);
            Json(serde_json::json!({ "ok": true, "action": "resume" }))
        }
        _ => Json(serde_json::json!({ "ok": false, "error": format!("unknown action: {}", req.action) })),
    }
}

/// 通用属性收益曲线 endpoint：以 attributes 为参考点，固定 equipment + macro，
/// 9 维各 ±Δ 跑 sim 拟合 quadratic 模型。供模拟器/配装器/广场/宏优化器/wzc 收益曲线复用。
///   raw 9 维顺序：[strength, agility, base_attack, weapon_damage, surplus_value,
///                 crit_level, crit_effect_level, overcome_level, strain_level]
///   返回 AutoFitModel（mean=raw_a，曲线在当前点附近最准）+ ref_dps（真实 sim DPS）
async fn equip_fit_curve(
    State(state): State<SharedState>,
    Json(req): Json<FitCurveRequest>,
) -> Json<FitCurveResponse> {
    let t_start = std::time::Instant::now();
    let attrs = match req.sim_req.attributes.as_ref() {
        Some(a) => a.clone(),
        None => return Json(FitCurveResponse {
            feasible: false,
            error: Some("attributes 字段必填".to_string()),
            ..Default::default()
        }),
    };

    let cur_version = *state.version.read().await;
    let cur_mount = *state.mount.read().await;
    let cur_consts = *state.constants.read().await;
    let skills = state.skills.read().await.clone();
    let recipes_table: Vec<RecipeEntry> = state.recipes.read().await.clone();
    let team_buffs_table = state.team_buffs.read().await.clone();
    let formations_table = state.formations.read().await.clone();

    // 各属性的最大单附魔等级（参考 jx3dps-online `获取当前各属性最大附魔` 实现）
    //   顺序与 RawAttrs 9 维一致：[strength, agility, base_attack, weapon_damage,
    //                            surplus_value, crit_level, crit_effect_level,
    //                            overcome_level, strain_level]
    //   全能（atPVXAllRound）单独存，前端合成第 10 维（+0.5 surplus + 1.5 strain）
    //
    //   规则：
    //   1) 排除"挑战附魔"（按 name 含"挑战"过滤，对应 jx3dps-online 的 `item.挑战附魔` 标记）
    //   2) 每条附魔只看第一个属性增益（单一附魔条目对应单一主属性；多 slot 是展开后的衍生）
    //   3) atAllType* 映射到外功对应字段（全会心 → 物理会心；全破防 → 物理破防 等），
    //      与 [equip.rs:1650-1665] calculate 后处理逻辑一致
    let mut max_enh = [0.0_f64; 9];
    let mut max_enh_pvx = 0.0_f64;
    // 来源追踪：每个属性 max 来自哪条附魔条目（name），调试日志用
    let mut max_enh_src: [String; 9] = Default::default();
    let mut max_enh_pvx_src = String::new();
    let slot_to_axis = |slot: &str| -> Option<usize> {
        match slot {
            "atStrengthBase"                                                      => Some(0),
            "atAgilityBase"                                                       => Some(1),
            "atPhysicsAttackPowerBase" | "atAllTypeAttackPowerBase"               => Some(2),
            "atMeleeWeaponDamageBase"                                             => Some(3),
            "atSurplusValueBase"                                                  => Some(4),
            "atPhysicsCriticalStrike" | "atAllTypeCriticalStrike"                 => Some(5),
            "atPhysicsCriticalDamagePowerBase" | "atAllTypeCriticalDamagePowerBase" => Some(6),
            "atPhysicsOvercomeBase" | "atAllTypeOvercomeBase"                     => Some(7),
            "atStrainBase"                                                        => Some(8),
            _ => None,
        }
    };
    for entries in state.equip_data.enhances.values() {
        for entry in entries {
            // 跳过首饰 + 暗器部位（项链 4 / 戒指 5 / 腰坠 7 / 暗器 1）：
            //   这些部位的附魔数值天然偏高（含挑战附魔系列、且整体规模大于身上其它部位），
            //   不能代表"普通附魔幅度"作为决策参考。只取衣裤/帽子/护腕/腰带/鞋/主武器 的附魔
            //   作为各属性的"满附魔等级"，让"满附魔 ΔDPS" 反映身上常规位的换装幅度。
            if matches!(entry.sub_type, 1 | 4 | 5 | 7) { continue; }
            if entry.is_challenge { continue; }   // 兜底（is_challenge 当前只在首饰标记）
            // 只看第一个属性增益（单一附魔的主属性；后面的 slot 多为展开衍生项，不算独立附魔属性）
            let first = match entry.attributes.first() { Some(a) => a, None => continue };
            let (slot, value) = (&first.0, first.1);
            let v = value as f64;
            if slot == "atPVXAllRound" {
                if v > max_enh_pvx {
                    max_enh_pvx = v;
                    max_enh_pvx_src = entry.name.clone();
                }
            } else if let Some(idx) = slot_to_axis(slot.as_str()) {
                if v > max_enh[idx] {
                    max_enh[idx] = v;
                    max_enh_src[idx] = entry.name.clone();
                }
            }
        }
    }
    // 会效 + 全能的附魔表里没有高数值条目（首饰已被过滤；身上其他部位 atPhysicsCriticalDamagePowerBase
    // / atPVXAllRound 的小附魔等级远低于会心等同类副属性）。把这俩的容量对齐到会心（idx 5），
    // 让"满附魔 ΔDPS" 在同等装备容量下对比，不会因为"会效附魔表数据少"显得收益异常低。
    let crit_idx = 5_usize;       // crit_level 在 9 维 raw 中的索引
    let crit_eff_idx = 6_usize;   // crit_effect_level
    max_enh[crit_eff_idx] = max_enh[crit_idx];
    max_enh_src[crit_eff_idx] = format!("（容量对齐到会心）{}", max_enh_src[crit_idx]);
    max_enh_pvx = max_enh[crit_idx];
    max_enh_pvx_src = format!("（容量对齐到会心）{}", max_enh_src[crit_idx]);

    let labels = ["力道", "身法", "攻击", "武伤", "破招", "会心", "会效", "破防", "无双"];
    for i in 0..9 {
        eprintln!("[fit_curve] max_enh[{}]={:.0}  来自 \"{}\"",
            labels[i], max_enh[i], max_enh_src[i]);
    }
    eprintln!("[fit_curve] max_enh[全能]={:.0}  来自 \"{}\"",
        max_enh_pvx, max_enh_pvx_src);

    // 跑拟合：spawn_blocking 避免占用 tokio worker（55 sim ≈ 0.4s）
    let mut sim_req_template = req.sim_req;
    // 让宏跑满 macro_duration 整个时长——sequence 至少要有 (duration/0.25)+20 个 __macro__ 占位，
    // 否则只跑 1 个 GCD 就停下，DPS 几乎全是平砍 + buff 自动触发，远低于真实值
    let dur = sim_req_template.macro_duration.unwrap_or(300.0);
    let needed_slots = ((dur / 0.25) as usize).saturating_add(20);
    if sim_req_template.sequence.len() < needed_slots {
        sim_req_template.sequence = vec!["__macro__".to_string(); needed_slots];
    }
    // lite 模式：fit_curve 不需要 timeline 详情，只要 dps
    sim_req_template.lite = true;
    sim_req_template.lite_keep_timeline = false;
    let pert_delta = req.pert_delta.max(1.0);
    let order = req.order.max(1);
    let axis_scale = req.axis_scale.max(0.5);

    let resp = tokio::task::spawn_blocking(move || {
        compute_fit_curve_sync(
            attrs, sim_req_template, pert_delta, order, axis_scale,
            &skills, cur_version, cur_mount, cur_consts,
            &recipes_table, &team_buffs_table, &formations_table,
        )
    }).await.unwrap_or_else(|e| FitCurveResponse {
        feasible: false,
        error: Some(format!("拟合任务 panic: {}", e)),
        ..Default::default()
    });

    let mut out = resp;
    out.elapsed_ms = t_start.elapsed().as_secs_f64() * 1000.0;
    out.fit_model.max_enhance_levels = max_enh;
    out.fit_model.max_enhance_pvx = max_enh_pvx;
    Json(out)
}

/// fit_curve 同步计算：9 维 ±Δ 扰动 + S6 二阶差分。封装在 spawn_blocking 内调用。
fn compute_fit_curve_sync(
    attrs: Attributes,
    sim_req_template: SimulateRequest,
    pert_delta: f64,
    order: u8,
    axis_scale: f64,
    skills: &[SkillSpec],
    cur_version: GameVersion,
    cur_mount: Mount,
    cur_consts: MountConstants,
    recipes_table: &[RecipeEntry],
    team_buffs_table: &[TeamBuffEntry],
    formations_table: &[FormationEntry],
) -> FitCurveResponse {
    use rayon::prelude::*;
    // 9 维 raw_a：从 attributes 字段提取（顺序与 axis_labels 一致）
    let raw_a: [f64; 9] = [
        attrs.li_dao,
        attrs.shen_fa,
        attrs.base_attack,
        attrs.weapon_damage,
        attrs.surplus_value,
        attrs.crit_level,
        attrs.crit_effect_level,
        attrs.overcome_level,
        attrs.strain_level,
    ];
    // 各扰动只换 attributes 这 9 维，其它字段（haste_level / vitality / parry_*）原样保留
    let attrs_with_axis = |axis: usize, delta: f64| -> Attributes {
        let mut a = attrs.clone();
        let v = (raw_a[axis] + delta).max(0.0);
        match axis {
            0 => a.li_dao            = v,
            1 => a.shen_fa           = v,
            2 => a.base_attack       = v,
            3 => a.weapon_damage     = v,
            4 => a.surplus_value     = v,
            5 => a.crit_level        = v,
            6 => a.crit_effect_level = v,
            7 => a.overcome_level    = v,
            8 => a.strain_level      = v,
            _ => {}
        }
        a
    };
    let attrs_with_two = |i: usize, j: usize, delta: f64| -> Attributes {
        let a1 = attrs_with_axis(i, delta);
        // 在 a1 基础上再扰动 j 维
        let mut a = a1.clone();
        let vj = (raw_a[j] + delta).max(0.0);
        match j {
            0 => a.li_dao            = vj,
            1 => a.shen_fa           = vj,
            2 => a.base_attack       = vj,
            3 => a.weapon_damage     = vj,
            4 => a.surplus_value     = vj,
            5 => a.crit_level        = vj,
            6 => a.crit_effect_level = vj,
            7 => a.overcome_level    = vj,
            8 => a.strain_level      = vj,
            _ => {}
        }
        a
    };
    let sim_one = |a: &Attributes| -> f64 {
        let mut req = sim_req_template.clone();
        req.attributes = Some(a.clone());
        simulate_core(&req, skills, cur_version, cur_mount, cur_consts,
            recipes_table, team_buffs_table, formations_table).dps
    };

    // 1 ref + 18 单轴 + (36 交叉 if order>=2) = 19 或 55 sim
    let ref_dps = sim_one(&attrs);
    let d_pairs: Vec<(f64, f64)> = (0..9usize).into_par_iter().map(|j| {
        let r1 = attrs_with_axis(j, pert_delta);
        let r2 = attrs_with_axis(j, 2.0 * pert_delta);
        (sim_one(&r1) - ref_dps, sim_one(&r2) - ref_dps)
    }).collect();
    let mut grad = [0f64; 9];
    let mut h_diag = [0f64; 9];
    for j in 0..9 {
        let (d1, d2) = d_pairs[j];
        grad[j]   = (4.0 * d1 - d2) / (2.0 * pert_delta);
        h_diag[j] = (d2 - 2.0 * d1) / (pert_delta * pert_delta);
    }
    let mut hess = [[0f64; 9]; 9];
    for j in 0..9 { hess[j][j] = h_diag[j]; }
    let mut sim_count: u32 = 19;   // 1 + 18
    if order >= 2 {
        let pairs: Vec<(usize, usize)> = (0..9).flat_map(|i| ((i+1)..9).map(move |j| (i, j))).collect();
        let cross: Vec<f64> = pairs.par_iter().map(|&(i, j)| {
            let r = attrs_with_two(i, j, pert_delta);
            let sim_ij = sim_one(&r);
            let lin = ref_dps + pert_delta * (grad[i] + grad[j]);
            let diag = 0.5 * pert_delta * pert_delta * (h_diag[i] + h_diag[j]);
            (sim_ij - lin - diag) / (pert_delta * pert_delta)
        }).collect();
        for (k, &(i, j)) in pairs.iter().enumerate() {
            hess[i][j] = cross[k];
            hess[j][i] = cross[k];
        }
        sim_count += 36;
    }

    // 坐标轴范围：raw_a ± axis_scale × Δ（min clamp 到 0，避免负值）
    let mut axis_min = [0f64; 9];
    let mut axis_max = [0f64; 9];
    for k in 0..9 {
        axis_min[k] = (raw_a[k] - axis_scale * pert_delta).max(0.0);
        axis_max[k] = raw_a[k] + axis_scale * pert_delta;
    }

    FitCurveResponse {
        feasible: true,
        ref_dps,
        sim_count,
        elapsed_ms: 0.0,   // outer handler 写
        error: None,
        fit_model: AutoFitModel {
            mean: raw_a,
            ref_dps,
            grad,
            hess,
            axis_min,
            axis_max,
            axis_labels: vec![
                "力道".to_string(), "身法".to_string(), "基础攻击".to_string(),
                "武器伤害".to_string(), "破招".to_string(), "会心".to_string(),
                "会心效果".to_string(), "破防".to_string(), "无双".to_string(),
            ],
            pool_size: 1,   // 单点拟合（不来自池子）
            max_enhance_levels: [0.0; 9],   // outer handler 填
            max_enhance_pvx: 0.0,           // outer handler 填
        },
    }
}

/// 实际跑搜索。结果以 AutoOptimizeResponse 形式返回，调用方负责写入 progress。
async fn run_auto_optimize_compute(
    state: SharedState,
    req: AutoOptimizeRequest,
) -> AutoOptimizeResponse {
    let t_start = std::time::Instant::now();
    let mut warnings: Vec<String> = Vec::new();

    // 排序使迭代有确定顺序（HashMap 顺序不固定 → 影响测试可重现性）
    let mut candidate_positions: Vec<String> = req.candidates.keys().cloned().collect();
    candidate_positions.sort();

    let total_combos: u64 = candidate_positions.iter()
        .map(|p| req.candidates.get(p).map(|v| v.len() as u64).unwrap_or(0))
        .filter(|&n| n > 0)
        .product::<u64>();

    if total_combos == 0 {
        return AutoOptimizeResponse {
            feasible: false, baseline_dps: 0.0, baseline_haste: 0, top: vec![],
            stats: AutoOptimizeStats { candidate_positions: candidate_positions.len(), ..Default::default() },
            warnings: vec!["候选池为空".into()],
            fit_model: None, fit_metrics: None,
        };
    }

    // 进度状态初始化（前端通过 /api/equip/auto_optimize/progress 拉）
    let progress = state.auto_search.clone();
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    {
        let mut p = progress.lock().unwrap();
        *p = AutoSearchProgress {
            running: true, phase: "enumerate".into(),
            total_combos, started_at_ms,
            ..Default::default()
        };
    }

    let bs  = state.base_stats.read().await.clone();
    let mc  = state.mount_conversions.read().await.clone();
    let cur_version = *state.version.read().await;
    let cur_mount   = *state.mount.read().await;
    let cur_consts  = *state.constants.read().await;
    let skills = state.skills.read().await;
    let recipes_table = state.recipes.read().await;
    let team_buffs_table = state.team_buffs.read().await;
    let formations_table = state.formations.read().await;

    let equip_data: &equip::EquipData = &state.equip_data;

    // 1. 候选物化（装备-only：每槽只 5 个装备候选，不展开加速 enhance pair）
    //    - 加速 enhance 单独存到 haste_enh_list，叶子里枚举 2^N 个"开/关"组合（HAT+SHOES = 4 种）
    //    - 非加速槽的 enhance 由 Phase B 偏导决策，Phase A 不带 enhance 贡献
    //    - 五彩石 stone_id 直接 fold 进 init_ctx，按 dc/dl 自动激活
    struct Cand { equip_id: u32, haste_marginal: u32 }
    let default_str = req.default_strength.unwrap_or(6);
    let default_emb = req.default_embedding.clone().unwrap_or_else(|| vec![6, 6, 6]);

    let pool: HashMap<String, Vec<Cand>> = candidate_positions.iter().map(|pos| {
        let sub = equip::pos_to_subtype(pos);
        let mut v: Vec<Cand> = Vec::new();
        for &id in &req.candidates[pos] {
            if let Some(it) = equip_data.items.get(&(sub, id)) {
                let cfg_no_enh = equip::SlotConfig {
                    equip_id: id, strength: default_str, embedding: default_emb.clone(),
                    enhance_id: 0, enchant_id: 0,
                };
                let item_h = item_haste_marginal(it, &cfg_no_enh);
                v.push(Cand { equip_id: id, haste_marginal: item_h });
            }
        }
        // 按 haste 降序：高 haste 先尝试，剪枝命中率高
        v.sort_by(|a, b| b.haste_marginal.cmp(&a.haste_marginal));
        (pos.clone(), v)
    }).collect();

    // 加速 enhance 信息：每加速槽对应一个 (slot_idx, enhance_id, haste, raw_delta)
    //   leaf 时枚举 2^N 个"开/关"组合，对每个组合在 raw 上做 haste 偏移（紫·急速 仅贡献 atHasteBase）
    //   若 enhance 给了非 haste 字段（other=true），fallback 到全 calc（紫·急速 通常不会触发）
    #[derive(Clone)]
    struct HasteEnhInfo {
        slot_idx: usize,
        enhance_id: u32,
        haste: u32,            // atHasteBase 贡献
        accum_delta: Vec<(u8, f64)>,  // 完整 enhance Δ accum（fallback 用）
        haste_only: bool,      // true = 只给 atHasteBase（fast path）
    }
    let haste_enh_list: Vec<HasteEnhInfo> = candidate_positions.iter().enumerate()
        .filter_map(|(i, pos)| {
            let enh_id = req.haste_enhance_ids.get(pos).copied().unwrap_or(0);
            if enh_id == 0 { return None; }
            let sub = equip::pos_to_subtype(pos) as i32;
            let list = equip_data.enhances.get(&sub)?;
            let e = list.iter().find(|e| e.id == enh_id)?;
            let mut haste: i64 = 0;
            let mut haste_only = true;
            let mut accum_delta: Vec<(u8, f64)> = Vec::new();
            for (slot, val) in &e.attributes {
                if slot == "atHasteBase" { haste += val; }
                else { haste_only = false; }
                if let Some(idx) = equip::search_calc::slot_to_idx(slot) {
                    accum_delta.push((idx as u8, *val as f64));
                }
            }
            Some(HasteEnhInfo {
                slot_idx: i,
                enhance_id: enh_id,
                haste: haste.max(0) as u32,
                accum_delta,
                haste_only,
            })
        })
        .collect();
    let max_enh_total: u32 = haste_enh_list.iter().map(|h| h.haste).sum();
    eprintln!("[auto-enum] haste_enh_list: {} slots, total max haste = {}",
        haste_enh_list.len(), max_enh_total);

    // h_minmax：每槽 cur_h 贡献范围（仅装备 atHasteBase + 加速 enhance 选项；不含 set bonus / stone）
    let enh_range_per_slot = compute_enhance_haste_range_per_slot(&req.enhance_candidates, equip_data);
    let h_minmax: Vec<(u32, u32)> = candidate_positions.iter().map(|pos| {
        let p = &pool[pos];
        if p.is_empty() { return (0, 0); }
        let item_mn = p.iter().map(|c| c.haste_marginal).min().unwrap() as i64;
        let item_mx = p.iter().map(|c| c.haste_marginal).max().unwrap() as i64;
        let (enh_mn, enh_mx) = enh_range_per_slot.get(pos).copied().unwrap_or((0, 0));
        let mn = (item_mn + enh_mn).max(0) as u32;
        let mx = (item_mx + enh_mx).max(0) as u32;
        (mn, mx)
    }).collect();

    // 实际枚举叶子数 = pool 各槽乘积（含加速 enhance pair 展开）。
    //   req-based 的 total_combos 只算装备 ID 数，遗漏了 pair 展开 → 进度条分母偏小。
    //   覆盖 progress.total_combos 让前端进度显示与真实枚举量对齐。
    let total_combos: u64 = candidate_positions.iter()
        .map(|p| pool.get(p).map(|v| v.len() as u64).unwrap_or(1))
        .product();
    {
        let mut p = progress.lock().unwrap();
        p.total_combos = total_combos;
    }

    // 2. 基线（fixed_slots only）
    let baseline_calc = equip::calculate(equip_data, &equip::CalcRequest {
        slots: req.fixed_slots.clone(),
        stone_id: req.stone_id,
        mount: req.mount,
        talents: req.talents.clone(),
    }, &bs, &mc);
    let baseline_attrs = raw_to_attributes(&baseline_calc.raw);
    let baseline_haste = baseline_calc.raw.haste_level as u32;

    // ─── Haste 预计算：紧 cur_h 边界，省掉 30M 进 calc 后才被丢的 leaf ───
    //   final_haste = baseline_haste + cur_h(候选 atHasteBase) + Δset_bonus + Δstone_extra
    //   其中:
    //     baseline_haste 已含 fixed_slots / 心法 / baseline 套装 / baseline 已激活 stone
    //     Δset_bonus = 候选触发的套装件数加成 (atHasteBase)，∈ [0, max_set_haste]
    //     Δstone_extra = 候选 dc/dl 抬升后新激活的 stone atHasteBase，∈ [0, max_stone_extra]
    //   → cur_h ∈ [target_min - baseline_haste - max_haste_delta, target_max - baseline_haste]
    let max_set_haste: u32 = {
        let mut total: u32 = 0;
        for set_entry in equip_data.sets.values() {
            // 每套装取最高 tier 的 atHasteBase 贡献（不同 tier 不同 attrs，取上限）
            let mut set_max: i64 = 0;
            for (_n, attrs) in &set_entry.bonuses {
                let mut tier_h: i64 = 0;
                for b in attrs {
                    if b.slot == "atHasteBase" { tier_h += b.value; }
                }
                if tier_h > set_max { set_max = tier_h; }
            }
            if set_max > 0 { total = total.saturating_add(set_max as u32); }
        }
        total
    };
    // Stone haste 上限：保守取 stone 全部 atHasteBase（无视激活阈值；候选可能多激活但绝不会超过这个值）
    let max_stone_extra_haste: u32 = if req.stone_id > 0 {
        let mut total: i64 = 0;
        if let Some(stone) = equip_data.stones.iter().find(|s| s.id == req.stone_id) {
            for sa in &stone.attributes {
                if sa.slot == "atHasteBase" { total += sa.value.max(0); }
            }
        }
        total.max(0) as u32
    } else { 0 };
    let max_haste_delta = max_set_haste + max_stone_extra_haste;
    // 候选 cur_h（装备-only atHasteBase 之和）必要窗口：
    //   final = baseline + cur_h + enh_haste + Δ_set + Δ_stone
    //   enh_haste ∈ [0, max_enh_total]（leaf 枚举 2^N 个加速 enhance 开关组合）
    //   → cur_h ∈ [target_min - baseline - max_haste_delta - max_enh_total, target_max - baseline]
    // 多区间：剪枝用 union envelope（min/max 包络更宽，不会漏候选）；leaf strict filter 才用 any 命中
    // 空数组 fallback 成 [0, cap=natural_cap]，等同"无约束"
    let natural_cap = (0.25 * LP_HASTE) as u32;
    let target_ranges: Vec<HasteRange> = if req.target_haste.is_empty() {
        vec![HasteRange { min: 0, max: natural_cap }]
    } else {
        req.target_haste.clone()
    };
    let union_min: u32 = target_ranges.iter().map(|r| r.min).min().unwrap_or(0);
    let union_max: u32 = target_ranges.iter().map(|r| r.max).max().unwrap_or(natural_cap);
    let adj_target_min: u32 = union_min
        .saturating_sub(baseline_haste)
        .saturating_sub(max_haste_delta)
        .saturating_sub(max_enh_total);
    let adj_target_max: u32 = union_max.saturating_sub(baseline_haste);
    eprintln!("[auto-enum] haste budget: baseline={} max_set={} max_stone={} max_enh_total={} → cur_h ∈ [{}, {}]  (segments={})",
        baseline_haste, max_set_haste, max_stone_extra_haste, max_enh_total,
        adj_target_min, adj_target_max, target_ranges.len());

    // 构造 equipment dict（pos → equip_id, ENCHANT_pos → enchant_id）：固定槽 + 本配置的候选位
    //   不传 equipment 会让脚本端 player.equip_id_at("PRIMARY_WEAPON") 永远 0，
    //   天下宏愿/驭焰主武器特效（盾击神兵/盾压神兵 + 99260/99261 隐藏秘籍）全部失效，
    //   导致 wzc 模拟 DPS 比模拟器实际值低 5-10%。
    //   ENCHANT_X：暗影千机大附魔（HAT/JACKET/BELT/WRIST/SHOES）走 has_enchant 命中，
    //   只看 equipped values 是否含目标 enchant_id（不绑定具体 slot），所以候选位即使
    //   id=0（baseline 状态）也要插入对应 ENCHANT_pos，保证 baseline 与候选大附魔触发对称。
    let build_equipment_map = |cand_ids: &[u32]| -> HashMap<String, u32> {
        let mut m: HashMap<String, u32> = HashMap::new();
        for (pos, cfg) in &req.fixed_slots {
            if cfg.equip_id   != 0 { m.insert(pos.clone(), cfg.equip_id); }
            if cfg.enchant_id != 0 { m.insert(format!("ENCHANT_{}", pos), cfg.enchant_id); }
        }
        for (i, pos) in candidate_positions.iter().enumerate() {
            if let Some(&id) = cand_ids.get(i) {
                if id != 0 { m.insert(pos.clone(), id); }
            }
            if let Some(&eid) = req.default_enchants.get(pos) {
                if eid != 0 { m.insert(format!("ENCHANT_{}", pos), eid); }
            }
        }
        m
    };
    // baseline 的 equipment：fixed_slots + 候选位的 default_enchants（候选位 equip 全 0）
    let baseline_equipment = build_equipment_map(&vec![0u32; candidate_positions.len()]);

    let make_sim_req = |attrs: Attributes, equipment: HashMap<String, u32>| -> SimulateRequest {
        let max_slots = (req.duration / 0.25) as usize + 20;
        SimulateRequest {
            haste_level: attrs.haste_level as u32,
            sequence: vec!["__macro__".to_string(); max_slots],
            channel_ticks: HashMap::new(),
            timing_offsets: HashMap::new(),
            qijin_buffs: HashMap::new(),
            macro_text: Some(req.macro_text.clone()),
            macro_duration: Some(req.duration),
            talents: req.talents.clone(),
            recipes: req.recipes.clone(),
            attributes: Some(attrs),
            target: req.target.clone(),
            network_delay: req.network_delay,
            initial_rage: Some(req.initial_rage),
            pauses: vec![],
            boss_attack_interval: if req.boss_attack_interval > 0.0 { Some(req.boss_attack_interval) } else { None },
            hanjia_expectation: Some(req.hanjia_expectation),
            tiegu_mode: req.tiegu_mode,
            experimental: req.experimental,
            lite: true,
            lite_keep_timeline: false,
            equipment,
            team_buffs: req.team_buffs.clone(),
            formation: req.formation.clone(),
            pre_releases: Vec::new(),
        }
    };
    let baseline_dps = simulate_core(
        &make_sim_req(baseline_attrs, baseline_equipment.clone()),
        &skills, cur_version, cur_mount, cur_consts, &recipes_table, &team_buffs_table, &formations_table,
    ).dps;

    // 3. 枚举 + 剪枝 + 增量 calc + dedup
    let t_calc_start = std::time::Instant::now();
    // 透传给 recurse：envelope 用于日志/边界对照；ranges 用于 leaf strict filter
    let target_min = union_min;
    let target_max = union_max;

    // 一次性预处理：固定槽 + 心法 + 系统基础 + 奇穴 + 五彩石 fold 进 init_ctx
    //   五彩石不再搜索（删除了 haste_stone_id 二态）；req.stone_id 直接进 ctx，
    //   leaf 时 calc_leaf_raw 按 live dc/dl 自动激活对应属性 → 单次 calc，无 stone iter
    let init_ctx = equip::search_calc::prepare_init(
        equip_data, &req.fixed_slots, req.stone_id, &req.talents, &bs, &mc,
    );

    // Sanity check：baseline (fixed_slots only) 时 calc_leaf_raw 应等同于 equip::calculate.raw
    // 这一次额外计算覆盖整套后处理 / 套装 / 五彩石 / 心法转化 的等价性。
    {
        let init_effect_count: HashMap<u32, u32> =
            init_ctx.initial_effect_ids.iter().map(|&e| (e, 1)).collect();
        let new_raw = equip::search_calc::calc_leaf_raw(
            &init_ctx, &init_ctx.initial_accum, &init_ctx.initial_set_counts,
            init_ctx.initial_diamond_count, init_ctx.initial_diamond_level,
        );
        let _ = init_effect_count;
        let old = &baseline_calc.raw;
        let mismatches = [
            ("crit",     new_raw.crit_level         as u32, old.crit_level         as u32),
            ("crit_eff", new_raw.crit_effect_level  as u32, old.crit_effect_level  as u32),
            ("overcome", new_raw.overcome_level     as u32, old.overcome_level     as u32),
            ("strain",   new_raw.strain_level       as u32, old.strain_level       as u32),
            ("surplus",  new_raw.surplus_value      as u32, old.surplus_value      as u32),
            ("attack",   new_raw.base_attack        as u32, old.base_attack        as u32),
            ("agility",  new_raw.agility            as u32, old.agility            as u32),
            ("strength", new_raw.strength           as u32, old.strength           as u32),
            ("haste",    new_raw.haste_level        as u32, old.haste_level        as u32),
            ("vit",      new_raw.vitality           as u32, old.vitality           as u32),
            ("parry",    new_raw.parry_level        as u32, old.parry_level        as u32),
        ];
        let bad: Vec<_> = mismatches.iter().filter(|(_, a, b)| a != b).collect();
        if !bad.is_empty() {
            println!("[auto-enum] WARN: incremental calc mismatch on baseline:");
            for (k, a, b) in &bad { println!("  {:<10} new={} old={}", k, a, b); }
            warnings.push(format!("增量 calc 校验未通过 (baseline)：{} 个字段不一致",  bad.len()));
        }
    }
    // 每候选物化 SlotContrib（装备-only：每槽 5 个装备，无 enhance；加速 enhance 在 leaf 时叠加）
    let cand_contribs: HashMap<String, Vec<equip::search_calc::SlotContrib>> = candidate_positions.iter().map(|pos| {
        let v: Vec<_> = pool[pos].iter().map(|c| {
            let cfg = equip::SlotConfig {
                equip_id: c.equip_id, strength: default_str,
                embedding: default_emb.clone(),
                enhance_id: 0,
                enchant_id: 0,
            };
            equip::search_calc::prepare_slot_contrib(equip_data, pos, &cfg).unwrap_or_default()
        }).collect();
        (pos.clone(), v)
    }).collect();

    // 注意：unique 存 (cand_ids, enh_ids, raw)，省掉每叶 HashMap clone
    //   - cand_ids: 按 candidate_positions 顺序的 equip_id
    //   - enh_ids:  按 candidate_positions 顺序的 加速 enhance ID（非加速槽 = 0）
    //   - raw:      该 (装备 pair + 固定 stone) 配置的真实 RawAttrs
    let mut unique: ahash::AHashMap<(u128, u64), (Box<[u32]>, Box<[u32]>, equip::RawAttrs)> = ahash::AHashMap::new();
    let mut enumerated: u64 = 0;
    let mut after_pruning: u64 = 0;

    /// enumerate 阶段细分耗时计数器（单线程递归，普通字段即可）
    #[derive(Default)]
    struct EnumStats {
        branches_pruned_min: u64,
        branches_pruned_max: u64,
        leaves_visited: u64,
        leaves_marginal_filtered: u64,
        leaves_passed_marginal: u64,
        leaves_strict_filtered: u64,
        leaves_kept: u64,
        time_apply_ns: u64,    // 增量 apply / unapply（accum + set_counts）
        time_calc_ns: u64,     // calc_leaf_raw（拷贝 accum + 套装 + 后处理）
        time_hash_ns: u64,     // fp + encode + dedup
        time_emit_ns: u64,     // 仅 was_new 时构造 slots HashMap
        last_log_at_ms: u64,
    }

    /// 增量维护的 live state（除 partial 外，均不参与递归回溯，由 apply/unapply 调整）
    struct LiveState {
        accum: [f64; equip::search_calc::N_ATTRS],
        set_counts: HashMap<u32, u32>,
        effect_count: HashMap<u32, u32>,
        diamond_count: u32,
        diamond_level: u32,
    }

    fn apply_contrib(s: &mut LiveState, c: &equip::search_calc::SlotContrib) {
        for (i, v) in &c.deltas { s.accum[*i as usize] += *v; }
        if c.set_id > 0 { *s.set_counts.entry(c.set_id).or_default() += 1; }
        for &eid in &c.effect_ids { *s.effect_count.entry(eid).or_default() += 1; }
        s.diamond_count += c.diamond_count;
        s.diamond_level += c.diamond_level;
    }
    fn unapply_contrib(s: &mut LiveState, c: &equip::search_calc::SlotContrib) {
        for (i, v) in &c.deltas { s.accum[*i as usize] -= *v; }
        if c.set_id > 0 {
            if let Some(n) = s.set_counts.get_mut(&c.set_id) {
                *n -= 1; if *n == 0 { s.set_counts.remove(&c.set_id); }
            }
        }
        for &eid in &c.effect_ids {
            if let Some(n) = s.effect_count.get_mut(&eid) {
                *n -= 1; if *n == 0 { s.effect_count.remove(&eid); }
            }
        }
        s.diamond_count -= c.diamond_count;
        s.diamond_level -= c.diamond_level;
    }

    /// fp 与旧的 compute_set_eff_fp 字节级一致：BTreeMap<set_id, u8> + 0xff 分隔 + BTreeSet<effect_id>
    fn compute_fp_live(s: &LiveState) -> u64 {
        use std::hash::Hasher;
        let mut sets: std::collections::BTreeMap<u32, u8> = Default::default();
        for (&sid, &cnt) in &s.set_counts { sets.insert(sid, cnt as u8); }
        let mut effects: std::collections::BTreeSet<u32> = Default::default();
        for (&eid, _) in &s.effect_count { effects.insert(eid); }
        let mut h = ahash::AHasher::default();
        for (sid, cnt) in &sets { h.write_u32(*sid); h.write_u8(*cnt); }
        h.write_u8(0xff);
        for eid in &effects { h.write_u32(*eid); }
        h.finish()
    }

    fn recurse(
        idx: usize,
        cur_h: u32,
        partial: &mut Vec<u32>,           // 装备 IDs（按槽位顺序）
        positions: &[String],
        pool: &HashMap<String, Vec<Cand>>,
        contribs: &HashMap<String, Vec<equip::search_calc::SlotContrib>>,
        h_minmax: &[(u32, u32)],
        // subtree_size[idx+1] = 在深度 idx 选一个候选后下面的叶子数；用于分支剪枝时累加 branch_skipped
        subtree_size: &[u64],
        // cur_h（候选 atHasteBase 之和）的必要窗口，已扣 baseline + max_haste_delta + max_enh_total
        cur_h_min: u32, cur_h_max: u32,
        // 绝对 haste 多区间（leaf 时 raw.haste_level 必须落入任一区间）
        abs_ranges: &[HasteRange],
        bucket_size: u32,
        // 加速 enhance 信息：leaf 枚举 2^N 个开关组合
        haste_enh_list: &[HasteEnhInfo],
        // 增量 calc 上下文（init_ctx 已含 req.stone_id；calc_leaf_raw 按 live dc/dl 自动激活属性）
        ctx: &equip::search_calc::InitCtx,
        live: &mut LiveState,
        unique: &mut ahash::AHashMap<(u128, u64), (Box<[u32]>, Box<[u32]>, equip::RawAttrs)>,
        enumerated: &mut u64, after_pruning: &mut u64, branch_skipped: &mut u64,
        progress: &Arc<std::sync::Mutex<AutoSearchProgress>>,
        started_at_ms: u64,
        cancel: &std::sync::atomic::AtomicBool,
        pause:  &std::sync::atomic::AtomicBool,
        stats:  &mut EnumStats,
    ) {
        use std::sync::atomic::Ordering;
        if cancel.load(Ordering::Relaxed) { return; }
        while pause.load(Ordering::Relaxed) {
            if cancel.load(Ordering::Relaxed) { return; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if idx == positions.len() {
            *enumerated += 1;
            stats.leaves_visited += 1;
            // 每 10000 次叶子刷新进度 + 5 秒一次日志
            if *enumerated % 10000 == 0 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                {
                    let mut p = progress.lock().unwrap();
                    p.enumerated = *enumerated;
                    p.branch_skipped = *branch_skipped;
                    p.after_haste_pruning = *after_pruning;
                    p.unique = unique.len() as u64;
                    p.elapsed_ms = now.saturating_sub(started_at_ms);
                }
                let elapsed = now.saturating_sub(started_at_ms);
                if elapsed.saturating_sub(stats.last_log_at_ms) >= 5000 {
                    stats.last_log_at_ms = elapsed;
                    let leaves_per_sec = if elapsed > 0 { *enumerated as f64 * 1000.0 / elapsed as f64 } else { 0.0 };
                    println!("[auto-enum] T={:.1}s  leaves={}  branch-prune(min={}, max={})  marginal-filter={}  passed={}  strict-filter={}  unique={}  rate={:.0} leaves/s  | apply={:.0}ms  calc={:.0}ms  hash={:.0}ms  emit={:.0}ms",
                        elapsed as f64 / 1000.0,
                        *enumerated, stats.branches_pruned_min, stats.branches_pruned_max,
                        stats.leaves_marginal_filtered, stats.leaves_passed_marginal,
                        stats.leaves_strict_filtered, unique.len(),
                        leaves_per_sec,
                        stats.time_apply_ns as f64 / 1e6,
                        stats.time_calc_ns as f64 / 1e6,
                        stats.time_hash_ns as f64 / 1e6,
                        stats.time_emit_ns as f64 / 1e6);
                }
            }
            // 边际预筛：cur_h 已扣过 baseline + max_haste_delta + max_enh_total
            //   超出 [cur_h_min, cur_h_max] 时即使开满所有加速 enhance 也合不到 target，可丢
            if cur_h < cur_h_min || cur_h > cur_h_max {
                stats.leaves_marginal_filtered += 1;
                return;
            }
            stats.leaves_passed_marginal += 1;
            *after_pruning += 1;

            // 单次 calc：算"全部加速 enhance 关"情况下的完整 raw（其它槽 enhance=0）
            let t_calc = std::time::Instant::now();
            let raw_no_enh = equip::search_calc::calc_leaf_raw(
                ctx, &live.accum, &live.set_counts, live.diamond_count, live.diamond_level,
            );
            stats.time_calc_ns += t_calc.elapsed().as_nanos() as u64;

            // 枚举 2^N 个加速 enhance 开关组合（紫·急速 enhance 仅给 atHasteBase，
            // post-process 不会传播到其它字段，可直接 mutate raw.haste_level）
            let n_enh = haste_enh_list.len();
            let total_subsets: u32 = if n_enh >= 32 { u32::MAX } else { 1u32 << n_enh };

            let t_hash_start = std::time::Instant::now();
            let fp = compute_fp_live(live);
            stats.time_hash_ns += t_hash_start.elapsed().as_nanos() as u64;

            for mask in 0..total_subsets {
                // 累加该组合的 enh haste
                let mut enh_haste_sum: u32 = 0;
                for (i, info) in haste_enh_list.iter().enumerate() {
                    if mask & (1 << i) != 0 { enh_haste_sum = enh_haste_sum.saturating_add(info.haste); }
                }
                let h_with_enh = (raw_no_enh.haste_level as u32).saturating_add(enh_haste_sum);

                // 严格 haste 过滤：必须落入 abs_ranges 的任一区间
                if !abs_ranges.iter().any(|r| h_with_enh >= r.min && h_with_enh <= r.max) {
                    stats.leaves_strict_filtered += 1;
                    continue;
                }

                // 构造该组合的 raw（haste mutate；其它字段 = raw_no_enh）
                let mut raw_combo = raw_no_enh.clone();
                raw_combo.haste_level += enh_haste_sum as f64;

                // mask 编进 fp，让不同 enhance 选择成不同 unique 项
                let fp_with_mask = fp.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(mask as u64);
                let key = encode_combo_key(&raw_combo, fp_with_mask, bucket_size);

                if !unique.contains_key(&key) {
                    let t_emit = std::time::Instant::now();
                    let cand_ids: Box<[u32]> = partial.iter().copied().collect();
                    let mut enh_ids: Box<[u32]> = vec![0u32; positions.len()].into_boxed_slice();
                    for (i, info) in haste_enh_list.iter().enumerate() {
                        if mask & (1 << i) != 0 { enh_ids[info.slot_idx] = info.enhance_id; }
                    }
                    unique.insert(key, (cand_ids, enh_ids, raw_combo));
                    stats.time_emit_ns += t_emit.elapsed().as_nanos() as u64;
                    stats.leaves_kept += 1;
                }
            }
            return;
        }

        let max_remaining: u32 = h_minmax[idx+1..].iter().map(|(_, mx)| *mx).sum();
        let min_remaining: u32 = h_minmax[idx+1..].iter().map(|(mn, _)| *mn).sum();
        let cands = &pool[&positions[idx]];
        let cands_contribs = &contribs[&positions[idx]];
        // 一个候选被剪掉时跳过的叶子数 = subtree_size[idx+1]（这个候选下面整棵子树）
        let skip_per_cand = subtree_size[idx + 1];
        for (ci, cand) in cands.iter().enumerate() {
            let nh = cur_h + cand.haste_marginal;
            // branch prune：nh + 剩余 max 仍达不到下限 OR nh + 剩余 min 已超上限 → 整个子树丢
            if nh + max_remaining < cur_h_min {
                stats.branches_pruned_min += 1;
                *branch_skipped += skip_per_cand;
                continue;
            }
            if nh + min_remaining > cur_h_max {
                stats.branches_pruned_max += 1;
                *branch_skipped += skip_per_cand;
                continue;
            }

            let t_apply = std::time::Instant::now();
            apply_contrib(live, &cands_contribs[ci]);
            stats.time_apply_ns += t_apply.elapsed().as_nanos() as u64;

            partial.push(cand.equip_id);
            recurse(idx + 1, nh, partial, positions, pool, contribs, h_minmax,
                subtree_size,
                cur_h_min, cur_h_max,
                abs_ranges,
                bucket_size,
                haste_enh_list,
                ctx, live,
                unique, enumerated, after_pruning, branch_skipped,
                progress, started_at_ms, cancel, pause, stats);
            partial.pop();

            let t_apply = std::time::Instant::now();
            unapply_contrib(live, &cands_contribs[ci]);
            stats.time_apply_ns += t_apply.elapsed().as_nanos() as u64;
        }
    }

    let cancel_flag = state.auto_search_cancel.clone();
    let pause_flag  = state.auto_search_pause.clone();
    let mut partial: Vec<u32> = Vec::with_capacity(candidate_positions.len());
    let mut enum_stats = EnumStats::default();
    let mut branch_skipped: u64 = 0;
    // subtree_size[idx] = 从深度 idx 起、所有候选组合下的叶子数
    //   subtree_size[N] = 1（叶子节点）；subtree_size[idx] = subtree_size[idx+1] × pool_size[idx]
    //   分支剪枝跳过 1 个候选 → skip subtree_size[idx+1] 叶子
    let mut subtree_size: Vec<u64> = vec![1; candidate_positions.len() + 1];
    for idx in (0..candidate_positions.len()).rev() {
        let pool_size = pool[&candidate_positions[idx]].len() as u64;
        subtree_size[idx] = subtree_size[idx + 1] * pool_size.max(1);
    }
    let mut live = LiveState {
        accum: init_ctx.initial_accum,
        set_counts: init_ctx.initial_set_counts.clone(),
        effect_count: init_ctx.initial_effect_ids.iter().map(|&e| (e, 1)).collect(),
        diamond_count: init_ctx.initial_diamond_count,
        diamond_level: init_ctx.initial_diamond_level,
    };
    println!("[auto-enum] start: total_combos={}  positions={}  abs_target=[{}, {}]  cur_h_window=[{}, {}]  haste_enh_slots={}  stone_id={}",
        total_combos, candidate_positions.len(),
        target_min, target_max, adj_target_min, adj_target_max,
        haste_enh_list.len(), req.stone_id);
    let bucket_size = req.bucket_size;
    recurse(0, 0, &mut partial, &candidate_positions, &pool, &cand_contribs, &h_minmax,
        &subtree_size,
        adj_target_min, adj_target_max,
        &target_ranges,
        bucket_size,
        &haste_enh_list,
        &init_ctx, &mut live,
        &mut unique, &mut enumerated, &mut after_pruning, &mut branch_skipped,
        &progress, started_at_ms, &cancel_flag, &pause_flag, &mut enum_stats);
    let enum_elapsed_ms = t_calc_start.elapsed().as_millis();
    println!(
        "[auto-enum] DONE T={:.1}s  total_combos={}  leaves_visited={}  branch-prune(min={}, max={})  marginal-filter={}  passed={}  strict-filter={}  unique={}  || apply={:.0}ms  calc={:.0}ms ({:.2}μs/件)  hash={:.0}ms  emit={:.0}ms",
        enum_elapsed_ms as f64 / 1000.0,
        total_combos,
        enum_stats.leaves_visited,
        enum_stats.branches_pruned_min, enum_stats.branches_pruned_max,
        enum_stats.leaves_marginal_filtered, enum_stats.leaves_passed_marginal,
        enum_stats.leaves_strict_filtered, unique.len(),
        enum_stats.time_apply_ns as f64 / 1e6,
        enum_stats.time_calc_ns as f64 / 1e6,
        if enum_stats.leaves_passed_marginal > 0 { enum_stats.time_calc_ns as f64 / 1e3 / enum_stats.leaves_passed_marginal as f64 } else { 0.0 },
        enum_stats.time_hash_ns as f64 / 1e6,
        enum_stats.time_emit_ns as f64 / 1e6,
    );

    // 取消时直接返回（feasible=false + warning）
    if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return AutoOptimizeResponse {
            feasible: false, baseline_dps, baseline_haste, top: vec![],
            stats: AutoOptimizeStats {
                candidate_positions: candidate_positions.len(),
                total_combinations: total_combos,
                after_haste_pruning: after_pruning,
                unique_keys: unique.len(),
                after_pareto: 0, simulated: 0,
                time_total_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                time_calc_ms: t_calc_start.elapsed().as_secs_f64() * 1000.0,
                time_pareto_ms: 0.0, time_simulate_ms: 0.0,
                ..Default::default()
            },
            warnings: vec!["搜索已取消（在枚举阶段）".into()],
            fit_model: None, fit_metrics: None,
        };
    }

    let unique_count = unique.len();
    let time_calc_ms = t_calc_start.elapsed().as_secs_f64() * 1000.0;
    {
        let mut p = progress.lock().unwrap();
        p.phase = "pareto".into();
        p.enumerated = enumerated;
        p.branch_skipped = branch_skipped;
        p.after_haste_pruning = after_pruning;
        p.unique = unique_count as u64;
    }

    if unique.is_empty() {
        return AutoOptimizeResponse {
            feasible: false, baseline_dps, baseline_haste, top: vec![],
            stats: AutoOptimizeStats {
                candidate_positions: candidate_positions.len(),
                total_combinations: total_combos,
                after_haste_pruning: after_pruning,
                unique_keys: 0, after_pareto: 0, simulated: 0,
                time_total_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                time_calc_ms,
                time_pareto_ms: 0.0,
                time_simulate_ms: 0.0,
                ..Default::default()
            },
            warnings: vec![format!(
                "经过加速筛选无可行解（候选 {}, 加速 {} 段, envelope {}~{}）。试试放宽 target_haste 或换更多候选。",
                total_combos, target_ranges.len(), target_min, target_max,
            )],
            fit_model: None, fit_metrics: None,
        };
    }

    // 4. 桶内 Pareto 剪枝（按 set/特效 fp 分桶；8 维 raw 等级单调性）
    let t_pareto_start = std::time::Instant::now();
    use rayon::prelude::*;
    // unique_vec: (key, cand_ids, enh_ids, raw)
    let unique_vec: Vec<((u128, u64), Box<[u32]>, Box<[u32]>, equip::RawAttrs)> =
        unique.into_iter().map(|(k, (cand_ids, enh_ids, r))| (k, cand_ids, enh_ids, r)).collect();

    let after_pareto_vec: Vec<((u128, u64), Box<[u32]>, Box<[u32]>, equip::RawAttrs)> = if req.use_pareto {
        // 4a. 按 fp 分桶（单线程；HashMap insert 顺序稳定）
        let mut buckets: ahash::AHashMap<u64, Vec<usize>> = ahash::AHashMap::new();
        for (i, ((_k, fp), _, _, _)) in unique_vec.iter().enumerate() {
            buckets.entry(*fp).or_default().push(i);
        }
        // 4b. 桶内 skyline sweep + rayon 跨桶并行
        let bucket_vec: Vec<Vec<usize>> = buckets.into_values().collect();
        let total_points: u64 = bucket_vec.iter().map(|v| v.len() as u64).sum();

        // 进度：以"已处理点数"计（target = total_points）。每桶处理完后原子加 idxs.len()
        {
            let mut p = progress.lock().unwrap();
            p.target_simulated = total_points;
            p.simulated = 0;
        }
        let pareto_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let hb_done_pareto = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hb_thread_pareto = {
            let progress = progress.clone();
            let counter = pareto_counter.clone();
            let done = hb_done_pareto.clone();
            std::thread::spawn(move || {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let cnt = counter.load(std::sync::atomic::Ordering::Relaxed);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                    let mut p = progress.lock().unwrap();
                    p.simulated = cnt;
                    p.elapsed_ms = now.saturating_sub(started_at_ms);
                }
            })
        };
        let counter_for_par = pareto_counter.clone();

        let bucket_kept: Vec<Vec<usize>> = bucket_vec.into_par_iter().map(|idxs| {
            let n_pts = idxs.len() as u64;
            let result = if idxs.len() <= 1 { idxs } else {
                let mut pts: Vec<(usize, [u32; 8])> = idxs.into_iter().map(|i| {
                    (i, raw_to_pareto_vec(&unique_vec[i].3, bucket_size))
                }).collect();
                pts.sort_unstable_by(|a, b| {
                    let sa: u64 = a.1.iter().map(|&v| v as u64).sum();
                    let sb: u64 = b.1.iter().map(|&v| v as u64).sum();
                    sb.cmp(&sa)
                });
                let mut kept: Vec<(usize, [u32; 8])> = Vec::new();
                'outer: for (i, v) in pts {
                    for (_, kv) in &kept {
                        let mut all_ge = true;
                        let mut any_gt = false;
                        for d in 0..8 {
                            if kv[d] < v[d] { all_ge = false; break; }
                            if kv[d] > v[d] { any_gt = true; }
                        }
                        if all_ge && any_gt { continue 'outer; }
                    }
                    kept.push((i, v));
                }
                kept.into_iter().map(|(i, _)| i).collect()
            };
            counter_for_par.fetch_add(n_pts, std::sync::atomic::Ordering::Relaxed);
            result
        }).collect();

        hb_done_pareto.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = hb_thread_pareto.join();

        let mut keep_idx: Vec<usize> = bucket_kept.into_iter().flatten().collect();
        keep_idx.sort_unstable();
        // 把保留的索引取出（unique_vec 不再需要，能用 swap 转移更轻；但实现上 collect 即可）
        let mut iter = unique_vec.into_iter().enumerate();
        let mut out = Vec::with_capacity(keep_idx.len());
        let mut keep_iter = keep_idx.into_iter().peekable();
        while let (Some(target), Some((i, item))) = (keep_iter.peek().copied(), iter.next()) {
            if i == target {
                out.push(item);
                keep_iter.next();
            }
        }
        out
    } else {
        unique_vec
    };
    let after_pareto = after_pareto_vec.len();
    let time_pareto_ms = t_pareto_start.elapsed().as_secs_f64() * 1000.0;

    let ctx = (cur_version, cur_mount, cur_consts);

    // 4.4 参考点策略评估（已废弃删除）
    //   历史实验对比 S1-S7 七种策略的 R² / top-K 重合度，结论：S6（池子均值 + 完整 Hessian，
    //   55 sims）最优 R²=0.99+，top10 9/10。已固定走 S6（见 4.5），不再每次搜索跑评估。
    //   节省每次搜索 ~3-5s（2000 ground-truth sims + 7 × 10-75 grad sims）。

    // 4.5 梯度排序预筛（S6 完整 Hessian 参考点）
    //
    // 启用条件智能检测：
    //   武器锁定 + 池子大     → S6 预筛（Hessian 准，能压数据量）
    //   武器锁定 + 池子小     → 老逻辑（直接 sim 全部，开销可接受）
    //   武器未锁定 + 池子大   → S6 预筛 + warning（紫橙武器特效非线性，可能误排）
    //   武器未锁定 + 池子小   → 老逻辑
    //
    // 预筛流程：见下方 S6 实现块（55 sims：1 ref + 9×2 对角 + 36 交叉 → quadratic 全池预测）
    use rayon::prelude::*;
    // 智能模式触发阈值：候选总组合数 > 1百万 时才启用梯度预筛（线性近似有误差，能省时但损精度）
    //   小于此阈值时直接全 sim（精度最高，时长可接受）
    const SMART_MODE_THRESHOLD: u64 = 1_000_000;
    let weapon_locked = req.fixed_slots.contains_key("PRIMARY_WEAPON");
    let prefilter_enabled = req.top_k_proxy > 0 && total_combos > SMART_MODE_THRESHOLD;
    if prefilter_enabled && !weapon_locked {
        warnings.push(
            "主武器未锁定且候选池较大，已启用梯度预筛。如候选包含多种武器（紫武/橙武），\
             特殊效果差异可能影响线性预测精度，建议优先锁定主武器。".to_string(),
        );
    }

    let t_rank_start = std::time::Instant::now();
    let after_pareto_vec = if prefilter_enabled {
        // ── S6 完整 Hessian 预筛 + 单属性 top-K union ──
        //   历史实验（已删除，见 4.4 注释）：完整 Hessian R²=0.99+，top10 9/10，top100 93%，
        //   远超线性策略（R²~0.96，top10 5-7/10）。
        //   55 sims 一次性算 (g, H)，Δᵀ·H·Δ 捕获属性间乘性交叉项（会心 × 会效、攻击 × 破防 等）
        //
        //   流程：
        //   1. 池子均值 ref → 1 ref + 9×2 对角 + 36 交叉对 = 55 sims → (g, H, ref_dps)
        //   2. 全池 quadratic 预测 → 取 top K_proxy
        //   3. 9 个属性各自 top K_per_attr → union（捕获单轴极端配置）
        const PERT_DELTA: f64 = 1000.0;
        const HESSIAN_SIMS: u64 = 55;            // 1 + 9*2 + 36
        const K_PER_ATTR_RATIO: usize = 5;       // 每属性 top = K_proxy / 5（默认 1000）

        {
            let mut p = progress.lock().unwrap();
            p.phase = "rank".into();
            p.target_simulated = HESSIAN_SIMS;
            p.simulated = 0;
        }

        let perturb_axis = |ref_raw: &equip::RawAttrs, axis: usize, delta: f64| -> equip::RawAttrs {
            let mut r = ref_raw.clone();
            match axis {
                0 => r.strength          += delta,
                1 => r.agility           += delta,
                2 => r.base_attack       += delta,
                3 => r.weapon_damage     += delta,
                4 => r.surplus_value     += delta,
                5 => r.crit_level        += delta,
                6 => r.crit_effect_level += delta,
                7 => r.overcome_level    += delta,
                8 => r.strain_level      += delta,
                _ => {}
            }
            r
        };
        let sim_one = |raw: &equip::RawAttrs| -> f64 {
            simulate_core(&make_sim_req(raw_to_attributes(raw), baseline_equipment.clone()),
                &skills, ctx.0, ctx.1, ctx.2, &recipes_table, &team_buffs_table, &formations_table).dps
        };
        let progress_atomic = std::sync::atomic::AtomicU64::new(0);
        let bump_progress = |delta: u64| {
            let cnt = progress_atomic.fetch_add(delta, std::sync::atomic::Ordering::Relaxed) + delta;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            let mut p = progress.lock().unwrap();
            p.simulated = cnt;
            p.elapsed_ms = now.saturating_sub(started_at_ms);
        };

        // ─── S6 计算：1 ref + 18 对角 + 36 交叉 ───
        let mean_of = |configs: &[((u128, u64), Box<[u32]>, Box<[u32]>, equip::RawAttrs)],
                       indices: Option<&[usize]>| -> equip::RawAttrs
        {
            let mut acc = equip::RawAttrs::default();
            let n = match indices {
                Some(idx) => { for &i in idx {
                    let r = &configs[i].3;
                    acc.strength += r.strength; acc.agility += r.agility;
                    acc.base_attack += r.base_attack; acc.weapon_damage += r.weapon_damage;
                    acc.surplus_value += r.surplus_value;
                    acc.crit_level += r.crit_level; acc.crit_effect_level += r.crit_effect_level;
                    acc.overcome_level += r.overcome_level; acc.strain_level += r.strain_level;
                    acc.haste_level += r.haste_level; acc.vitality += r.vitality;
                } idx.len() as f64 }
                None => { for c in configs {
                    let r = &c.3;
                    acc.strength += r.strength; acc.agility += r.agility;
                    acc.base_attack += r.base_attack; acc.weapon_damage += r.weapon_damage;
                    acc.surplus_value += r.surplus_value;
                    acc.crit_level += r.crit_level; acc.crit_effect_level += r.crit_effect_level;
                    acc.overcome_level += r.overcome_level; acc.strain_level += r.strain_level;
                    acc.haste_level += r.haste_level; acc.vitality += r.vitality;
                } configs.len() as f64 }
            };
            acc.strength /= n; acc.agility /= n;
            acc.base_attack /= n; acc.weapon_damage /= n; acc.surplus_value /= n;
            acc.crit_level /= n; acc.crit_effect_level /= n;
            acc.overcome_level /= n; acc.strain_level /= n;
            acc.haste_level /= n; acc.vitality /= n;
            acc
        };

        let mean_pool = mean_of(&after_pareto_vec, None);
        let ref_dps = sim_one(&mean_pool);
        bump_progress(1);

        // 9 维 × 2 扰动 → 一阶 g_i + 对角 h_ii
        let d_pairs: Vec<(f64, f64)> = (0..9).into_par_iter().map(|j| {
            let r1 = perturb_axis(&mean_pool, j, PERT_DELTA);
            let r2 = perturb_axis(&mean_pool, j, 2.0 * PERT_DELTA);
            (sim_one(&r1) - ref_dps, sim_one(&r2) - ref_dps)
        }).collect();
        bump_progress(18);

        let mut grad = [0f64; 9];
        let mut h_diag = [0f64; 9];
        for j in 0..9 {
            let (d1, d2) = d_pairs[j];
            grad[j]   = (4.0 * d1 - d2) / (2.0 * PERT_DELTA);
            h_diag[j] = (d2 - 2.0 * d1) / (PERT_DELTA * PERT_DELTA);
        }

        // 36 个交叉对 → 完整 Hessian
        let pairs: Vec<(usize, usize)> = (0..9).flat_map(|i| ((i+1)..9).map(move |j| (i, j))).collect();
        let cross: Vec<f64> = pairs.par_iter().map(|&(i, j)| {
            let mut r = mean_pool.clone();
            r = perturb_axis(&r, i, PERT_DELTA);
            r = perturb_axis(&r, j, PERT_DELTA);
            let sim_ij = sim_one(&r);
            let lin = ref_dps + PERT_DELTA * (grad[i] + grad[j]);
            let diag = 0.5 * PERT_DELTA * PERT_DELTA * (h_diag[i] + h_diag[j]);
            (sim_ij - lin - diag) / (PERT_DELTA * PERT_DELTA)
        }).collect();
        bump_progress(36);

        let mut hess = [[0f64; 9]; 9];
        for j in 0..9 { hess[j][j] = h_diag[j]; }
        for (k, &(i, j)) in pairs.iter().enumerate() {
            hess[i][j] = cross[k];
            hess[j][i] = cross[k];
        }
        eprintln!("[auto-enum] rank S6: ref_dps={:.0}  Hessian 完成（{} sims, {:.1}s）",
            ref_dps, HESSIAN_SIMS, t_rank_start.elapsed().as_secs_f64());

        // 全池 quadratic 预测：DPS ≈ ref + g·Δ + ½·Δᵀ·H·Δ
        let n = after_pareto_vec.len();
        let predicted: Vec<f64> = after_pareto_vec.par_iter().map(|c| {
            let r = &c.3;
            let d = [
                r.strength          - mean_pool.strength,
                r.agility           - mean_pool.agility,
                r.base_attack       - mean_pool.base_attack,
                r.weapon_damage     - mean_pool.weapon_damage,
                r.surplus_value     - mean_pool.surplus_value,
                r.crit_level        - mean_pool.crit_level,
                r.crit_effect_level - mean_pool.crit_effect_level,
                r.overcome_level    - mean_pool.overcome_level,
                r.strain_level      - mean_pool.strain_level,
            ];
            let mut s = ref_dps;
            for k in 0..9 { s += grad[k] * d[k]; }
            for i in 0..9 {
                for j in 0..9 {
                    s += 0.5 * hess[i][j] * d[i] * d[j];
                }
            }
            s
        }).collect();

        let mut union_set: ahash::AHashSet<usize> = ahash::AHashSet::new();
        let mut idx_sorted: Vec<usize> = (0..n).collect();
        idx_sorted.par_sort_by(|&a, &b| {
            predicted[b].partial_cmp(&predicted[a]).unwrap_or(std::cmp::Ordering::Equal)
        });
        for &i in idx_sorted.iter().take(req.top_k_proxy) { union_set.insert(i); }
        eprintln!("[auto-enum] rank S6 top {}: union {}", req.top_k_proxy, union_set.len());

        // ─── 单属性 top-K（9 个属性各取 K_per_attr）───
        let k_per_attr = (req.top_k_proxy / K_PER_ATTR_RATIO).max(500);
        let attr_getters: [fn(&equip::RawAttrs) -> f64; 9] = [
            |r| r.strength, |r| r.agility, |r| r.base_attack, |r| r.weapon_damage,
            |r| r.surplus_value, |r| r.crit_level, |r| r.crit_effect_level,
            |r| r.overcome_level, |r| r.strain_level,
        ];
        for (attr_i, get) in attr_getters.iter().enumerate() {
            let mut idx: Vec<usize> = (0..n).collect();
            idx.par_sort_by(|&a, &b| {
                get(&after_pareto_vec[b].3).partial_cmp(&get(&after_pareto_vec[a].3))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let before = union_set.len();
            for &i in idx.iter().take(k_per_attr) { union_set.insert(i); }
            eprintln!("[auto-enum] rank attr-top[{}]: +{} new (union now {})",
                attr_i, union_set.len() - before, union_set.len());
        }

        let filtered: Vec<_> = after_pareto_vec.into_iter().enumerate()
            .filter(|(i, _)| union_set.contains(i))
            .map(|(_, item)| item)
            .collect();
        eprintln!("[auto-enum] rank prefilter done: {} → {}  elapsed={:.1}s",
            n, filtered.len(), t_rank_start.elapsed().as_secs_f64());
        filtered
    } else {
        after_pareto_vec
    };
    let after_rank = after_pareto_vec.len();

    {
        let mut p = progress.lock().unwrap();
        p.phase = "simulate".into();
        p.after_pareto = after_pareto as u64;
        p.after_rank = after_rank as u64;
        p.target_simulated = after_rank as u64;
        p.simulated = 0;
    }

    // 5. simulate（rayon 并行；rayon::prelude::* 已在 pareto 阶段引入）
    let t_sim_start = std::time::Instant::now();
    let sim_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let hb_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 后台心跳：每 100ms 把原子计数刷到 progress 状态（避免每次 simulate 都锁 Mutex）
    let hb_thread = {
        let progress = progress.clone();
        let sim_counter = sim_counter.clone();
        let hb_done = hb_done.clone();
        std::thread::spawn(move || {
            while !hb_done.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let cnt = sim_counter.load(std::sync::atomic::Ordering::Relaxed);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                let mut p = progress.lock().unwrap();
                p.simulated = cnt;
                p.elapsed_ms = now.saturating_sub(started_at_ms);
            }
        })
    };
    let counter_for_par = sim_counter.clone();
    let cancel_for_par = cancel_flag.clone();
    let pause_for_par  = pause_flag.clone();
    // dps_results: (dps, cand_ids, enh_ids, raw)
    let dps_results: Vec<(f64, Box<[u32]>, Box<[u32]>, equip::RawAttrs)> = after_pareto_vec
        .into_par_iter()
        .filter_map(|(_k, cand_ids, enh_ids, raw)| {
            use std::sync::atomic::Ordering;
            if cancel_for_par.load(Ordering::Relaxed) { return None; }
            while pause_for_par.load(Ordering::Relaxed) {
                if cancel_for_par.load(Ordering::Relaxed) { return None; }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let attrs = raw_to_attributes(&raw);
            let eq = build_equipment_map(&cand_ids);
            let req2 = make_sim_req(attrs, eq);
            let resp = simulate_core(&req2, &skills, ctx.0, ctx.1, ctx.2, &recipes_table, &team_buffs_table, &formations_table);
            counter_for_par.fetch_add(1, Ordering::Relaxed);
            Some((resp.dps, cand_ids, enh_ids, raw))
        })
        .collect();
    hb_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = hb_thread.join();
    let time_simulate_ms = t_sim_start.elapsed().as_secs_f64() * 1000.0;

    // 把 (cand_ids, enh_ids, stone_id) → 完整 slots HashMap（顺序：固定槽 + 候选位置）
    //   cand_ids[i] = 装备 ID；enh_ids[i] = 加速 enhance ID（非加速槽 = 0）
    let make_slots_a = |cand_ids: &[u32], enh_ids: &[u32]| -> HashMap<String, equip::SlotConfig> {
        let mut s = req.fixed_slots.clone();
        for (i, (&id, &enh_id)) in cand_ids.iter().zip(enh_ids.iter()).enumerate() {
            let pos = &candidate_positions[i];
            s.insert(pos.clone(), equip::SlotConfig {
                equip_id: id, strength: default_str,
                embedding: default_emb.clone(),
                enhance_id: enh_id,    // Phase A 仅加速槽带 enhance；非加速槽 = 0（Phase B 偏导填）
                enchant_id: req.default_enchants.get(pos).copied().unwrap_or(0),
            });
        }
        s
    };

    // 取消时直接返回（保留已经跑出的 top；不进 Phase B 偏导）
    if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        let mut sorted = dps_results;
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(req.top_n);
        let top: Vec<AutoTopEntry> = sorted.into_iter().map(|(dps, cand_ids, enh_ids, raw)| {
            let slots = make_slots_a(&cand_ids, &enh_ids);
            let sl: HashMap<String, u32> = slots.iter().map(|(p, c)| (p.clone(), c.equip_id)).collect();
            let nm: HashMap<String, String> = slots.iter().filter_map(|(pos, cfg)| {
                let sub = equip::pos_to_subtype(pos);
                equip_data.items.get(&(sub, cfg.equip_id)).map(|it| (pos.clone(), it.name.clone()))
            }).collect();
            // 加速槽 enhance map（Phase A 完成的部分；Phase B 未跑）
            let enh_map: HashMap<String, u32> = candidate_positions.iter().zip(enh_ids.iter())
                .filter(|(_, &eid)| eid > 0)
                .map(|(p, &eid)| (p.clone(), eid))
                .collect();
            AutoTopEntry {
                slots: sl, names: nm, dps,
                delta_pct: if baseline_dps > 0.0 { (dps - baseline_dps) / baseline_dps * 100.0 } else { 0.0 },
                haste_level: raw.haste_level as u32,
                panel_attack: 0.0,
                panel: None,
                raw: None,
                enhances: enh_map,
                stone_id: req.stone_id,
                stone_name: String::new(),
            }
        }).collect();
        return AutoOptimizeResponse {
            feasible: !top.is_empty(),
            baseline_dps, baseline_haste, top,
            stats: AutoOptimizeStats {
                candidate_positions: candidate_positions.len(),
                total_combinations: total_combos,
                after_haste_pruning: after_pruning,
                unique_keys: unique_count,
                after_pareto,
                simulated: sim_counter.load(std::sync::atomic::Ordering::Relaxed) as usize,
                time_total_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                time_calc_ms,
                time_pareto_ms,
                time_simulate_ms,
                ..Default::default()
            },
            warnings: vec!["搜索已取消（保留已完成的部分模拟结果）".into()],
            fit_model: None, fit_metrics: None,
        };
    }

    // 6. Phase B：偏导后处理
    //    输入：Phase A top-K 个 (装备 + 加速 enhance) 配置（stone 已在 init_ctx）
    //    每个 base：
    //      a. 用 9 次扰动 sim 求 ∂DPS/∂raw_field（外功 build 的 9 个 raw 字段）
    //      b. 每个非加速槽：枚举 enhance 候选 → 算 Δraw → score = grad·Δraw → argmax
    //      c. 应用所选非加速 enhance Δ → 1 次 final sim
    //    总 sim 数 = K × (9 + 1)；K=200 时 ~2000 sims。
    type FinalRow = (f64, Box<[u32]>, equip::RawAttrs, HashMap<String, u32>);

    let phase_b_enabled = req.top_k_phase_b > 0 && !req.enhance_candidates.is_empty();
    let mut stats_b = PhaseBStats::default();

    let final_rows: Vec<FinalRow> = if phase_b_enabled {
        let t_pb_start = std::time::Instant::now();
        // 6.1 排序 + 取 top-K
        let mut phase_a_sorted = dps_results;
        phase_a_sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        phase_a_sorted.truncate(req.top_k_phase_b);
        stats_b.bases = phase_a_sorted.len();

        // 6.2 预查每个非加速槽的 enhance 候选 → enhance Δaccum 表
        //   key = (pos, enhance_id) → Vec<(accum_idx, value)>
        let mut enhance_accum_deltas: HashMap<(String, u32), Vec<(u8, f64)>> = HashMap::new();
        for (pos, ids) in &req.enhance_candidates {
            let sub = equip::pos_to_subtype(pos) as i32;
            if let Some(list) = equip_data.enhances.get(&sub) {
                for &id in ids {
                    if let Some(ench) = list.iter().find(|e| e.id == id) {
                        let mut d: Vec<(u8, f64)> = Vec::new();
                        for (slot, val) in &ench.attributes {
                            if let Some(idx) = equip::search_calc::slot_to_idx(slot) {
                                d.push((idx as u8, *val as f64));
                            }
                        }
                        enhance_accum_deltas.insert((pos.clone(), id), d);
                    }
                }
            }
        }

        // 6.3 phase 进度切到 phase_b
        {
            let mut p = progress.lock().unwrap();
            p.phase = "phase_b".into();
            p.target_simulated = phase_a_sorted.len() as u64;
            p.simulated = 0;
        }
        let counter_pb = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter_for_pb = counter_pb.clone();
        let cancel_for_pb = cancel_flag.clone();
        let pause_for_pb  = pause_flag.clone();
        // 后台心跳：与 Phase A 一致，每 100ms 把原子计数刷到 progress.simulated
        // 否则前端只看到 0/200 不动（fetch_add 不会自己写回 progress 状态）
        let pb_hb_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pb_hb_thread = {
            let progress = progress.clone();
            let counter = counter_pb.clone();
            let hb_done = pb_hb_done.clone();
            std::thread::spawn(move || {
                while !hb_done.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let cnt = counter.load(std::sync::atomic::Ordering::Relaxed);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                    let mut p = progress.lock().unwrap();
                    p.simulated = cnt;
                    p.elapsed_ms = now.saturating_sub(started_at_ms);
                }
            })
        };

        // 6.4 重建给定 (cand_ids, enh_ids) 配置下的 base accum / set_counts / dc / dl
        //   pool[pos] 是装备-only；如果 enh_ids[i] 是加速 enhance（在 haste_enh_list 里），
        //   额外把它的 accum_delta 叠到 base accum（让偏导评估时 base 已含加速 enhance 的贡献）
        let haste_enh_by_pos: HashMap<&str, &HasteEnhInfo> = haste_enh_list.iter()
            .map(|h| (candidate_positions[h.slot_idx].as_str(), h)).collect();
        let rebuild_base = |cand_ids: &[u32], enh_ids: &[u32]|
            -> ([f64; equip::search_calc::N_ATTRS], HashMap<u32, u32>, u32, u32)
        {
            let mut accum = init_ctx.initial_accum;
            let mut set_counts = init_ctx.initial_set_counts.clone();
            let mut dc = init_ctx.initial_diamond_count;
            let mut dl = init_ctx.initial_diamond_level;
            for (i, pos) in candidate_positions.iter().enumerate() {
                let id = cand_ids[i];
                let pool_v = match pool.get(pos) { Some(v) => v, None => continue };
                let idx = match pool_v.iter().position(|c| c.equip_id == id) {
                    Some(x) => x, None => continue,
                };
                let contrib = &cand_contribs[pos][idx];
                for (slot_idx, val) in &contrib.deltas { accum[*slot_idx as usize] += val; }
                if contrib.set_id > 0 { *set_counts.entry(contrib.set_id).or_default() += 1; }
                dc += contrib.diamond_count;
                dl += contrib.diamond_level;
                // 该槽如选了加速 enhance（enh_ids[i] != 0），叠 enhance Δ
                if enh_ids[i] != 0 {
                    if let Some(info) = haste_enh_by_pos.get(pos.as_str()) {
                        if info.enhance_id == enh_ids[i] {
                            for (slot_idx, val) in &info.accum_delta {
                                accum[*slot_idx as usize] += val;
                            }
                        }
                    }
                }
            }
            (accum, set_counts, dc, dl)
        };

        // 6.5 主循环（rayon 并行）
        //   phase_b_slots：枚举槽（candidate_positions）+ 锁定槽（fixed_slots，启用 search_locked_enhance 时）
        //   只要 enhance_candidates 里有该 pos 的候选就纳入
        let phase_b_slots: Vec<String> = {
            let mut slots: Vec<String> = candidate_positions.iter()
                .filter(|p| req.enhance_candidates.get(*p).map(|v| !v.is_empty()).unwrap_or(false))
                .cloned().collect();
            if req.search_locked_enhance {
                for pos in req.fixed_slots.keys() {
                    if !slots.contains(pos)
                        && req.enhance_candidates.get(pos).map(|v| !v.is_empty()).unwrap_or(false)
                    {
                        slots.push(pos.clone());
                    }
                }
            }
            slots
        };

        // 锁定槽 user existing enhance Δ：init_ctx.initial_accum 已含；偏导评估需要先减掉
        //   只有 search_locked_enhance + 该锁定槽在 enhance_candidates 里 才需要
        let locked_existing_enh: HashMap<String, Vec<(u8, f64)>> = {
            let mut m = HashMap::new();
            if req.search_locked_enhance {
                for (pos, cfg) in &req.fixed_slots {
                    if !req.enhance_candidates.get(pos).map(|v| !v.is_empty()).unwrap_or(false) { continue; }
                    if cfg.enhance_id == 0 { continue; }
                    let sub = equip::pos_to_subtype(pos) as i32;
                    if let Some(list) = equip_data.enhances.get(&sub) {
                        if let Some(e) = list.iter().find(|x| x.id == cfg.enhance_id) {
                            let d: Vec<(u8, f64)> = e.attributes.iter().filter_map(|(s, v)| {
                                equip::search_calc::slot_to_idx(s).map(|i| (i as u8, *v as f64))
                            }).collect();
                            m.insert(pos.clone(), d);
                        }
                    }
                }
            }
            m
        };
        eprintln!("[phase-b] phase_b_slots={:?}  locked-with-enh-search={}",
            phase_b_slots, locked_existing_enh.len());

        // 用于扰动梯度的 8 个 raw 字段（外功 build 关心的）
        // 每个：(field_setter_closure_index, perturb_delta)
        const PERT_DELTA: f64 = 1000.0;
        let final_rows: Vec<FinalRow> = phase_a_sorted.into_par_iter()
            .filter_map(|(dps_a, cand_ids, enh_ids, raw_a)| {
                use std::sync::atomic::Ordering;
                if cancel_for_pb.load(Ordering::Relaxed) { return None; }
                while pause_for_pb.load(Ordering::Relaxed) {
                    if cancel_for_pb.load(Ordering::Relaxed) { return None; }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                // 本 base 的 equipment（含本候选 主武器 → 装备特效正确触发）
                let eq_base = build_equipment_map(&cand_ids);

                // (a) 9 维 raw 字段梯度（用 raw_a 作基线，每次 perturb 一个字段）
                //   字段: strength, agility, base_attack, weapon_damage, surplus, crit, crit_eff, overcome, strain
                //   注意：haste_level 不在内（加速 enhance 已锁定 Phase A）
                let perturb_and_sim = |bump_field: usize| -> f64 {
                    let mut r = raw_a.clone();
                    match bump_field {
                        0 => r.strength          += PERT_DELTA,
                        1 => r.agility           += PERT_DELTA,
                        2 => r.base_attack       += PERT_DELTA,
                        3 => r.weapon_damage     += PERT_DELTA,
                        4 => r.surplus_value     += PERT_DELTA,
                        5 => r.crit_level        += PERT_DELTA,
                        6 => r.crit_effect_level += PERT_DELTA,
                        7 => r.overcome_level    += PERT_DELTA,
                        8 => r.strain_level      += PERT_DELTA,
                        _ => {}
                    }
                    let attrs = raw_to_attributes(&r);
                    let resp = simulate_core(&make_sim_req(attrs, eq_base.clone()), &skills, ctx.0, ctx.1, ctx.2, &recipes_table, &team_buffs_table, &formations_table);
                    resp.dps
                };
                let mut grad: [f64; 9] = [0.0; 9];
                for i in 0..9 {
                    let dps_p = perturb_and_sim(i);
                    grad[i] = (dps_p - dps_a) / PERT_DELTA;
                    if cancel_for_pb.load(Ordering::Relaxed) { return None; }
                }

                // (b) 重建 base accum 用于 enhance candidate 评分（init_ctx 已含 stone）
                let (base_accum, set_counts, base_dc, base_dl) = rebuild_base(&cand_ids, &enh_ids);

                // raw 字段 grad 点乘
                let raw_dot_grad = |r: &equip::RawAttrs| -> f64 {
                    grad[0] * (r.strength          - raw_a.strength)
                  + grad[1] * (r.agility           - raw_a.agility)
                  + grad[2] * (r.base_attack       - raw_a.base_attack)
                  + grad[3] * (r.weapon_damage     - raw_a.weapon_damage)
                  + grad[4] * (r.surplus_value     - raw_a.surplus_value)
                  + grad[5] * (r.crit_level        - raw_a.crit_level)
                  + grad[6] * (r.crit_effect_level - raw_a.crit_effect_level)
                  + grad[7] * (r.overcome_level    - raw_a.overcome_level)
                  + grad[8] * (r.strain_level      - raw_a.strain_level)
                };

                // (c) 对每个槽（含可搜的锁定槽）：按 grad 选最佳 enhance
                //   锁定槽：base_accum 已含 user existing enhance Δ → 评估时先减掉再加 candidate
                //   加速槽（HAT/SHOES/远武）若 Phase A 已选急速 enhance（enh_ids[idx]!=0），跳过
                //   —— 一个装备只能带一个附魔；急速优先，非急速候选作 mask=0 时的兜底
                let mut chosen_enh: HashMap<String, u32> = HashMap::new();
                for pos in &phase_b_slots {
                    let cands = match req.enhance_candidates.get(pos) { Some(v) => v, None => continue };
                    // 枚举槽 + Phase A 已选急速 → 跳过
                    if let Some(idx) = candidate_positions.iter().position(|p| p == pos) {
                        if enh_ids[idx] != 0 { continue; }
                    }
                    let existing = locked_existing_enh.get(pos);
                    let mut best_score = f64::NEG_INFINITY;
                    let mut best_id: u32 = 0;
                    for &eid in cands {
                        let delta = match enhance_accum_deltas.get(&(pos.clone(), eid)) { Some(d) => d, None => continue };
                        let mut accum = base_accum;
                        // 锁定槽：减掉已有的 user enhance Δ
                        if let Some(ex) = existing {
                            for (idx, val) in ex { accum[*idx as usize] -= val; }
                        }
                        for (idx, val) in delta { accum[*idx as usize] += val; }
                        let r = equip::search_calc::calc_leaf_raw(
                            &init_ctx, &accum, &set_counts, base_dc, base_dl);
                        let score = raw_dot_grad(&r);
                        if score > best_score { best_score = score; best_id = eid; }
                    }
                    if best_id > 0 { chosen_enh.insert(pos.clone(), best_id); }
                }

                // (d) 应用所有 chosen 非加速 enhance Δ → final raw
                //   锁定槽：先减掉 existing 再加 chosen
                let mut final_accum = base_accum;
                for (pos, &eid) in &chosen_enh {
                    if let Some(ex) = locked_existing_enh.get(pos) {
                        for (idx, val) in ex { final_accum[*idx as usize] -= val; }
                    }
                    if let Some(d) = enhance_accum_deltas.get(&(pos.clone(), eid)) {
                        for (idx, val) in d { final_accum[*idx as usize] += val; }
                    }
                }
                let final_raw = equip::search_calc::calc_leaf_raw(
                    &init_ctx, &final_accum, &set_counts, base_dc, base_dl);

                // 严格 haste 过滤（多区间 any 命中；防止偏导引导到非加速档）
                let h = final_raw.haste_level as u32;
                if !target_ranges.iter().any(|r| h >= r.min && h <= r.max) {
                    counter_for_pb.fetch_add(1, Ordering::Relaxed);
                    return None;
                }

                // (e) final sim — 用本 base 的 equipment（装备特效触发）
                let attrs = raw_to_attributes(&final_raw);
                let resp = simulate_core(&make_sim_req(attrs, eq_base.clone()), &skills, ctx.0, ctx.1, ctx.2, &recipes_table, &team_buffs_table, &formations_table);
                counter_for_pb.fetch_add(1, Ordering::Relaxed);

                // (f) 合并 enhance map：加速槽（来自 enh_ids）+ 非加速槽（chosen_enh）
                let mut enh_map = chosen_enh;
                for (i, pos) in candidate_positions.iter().enumerate() {
                    let eid = enh_ids[i];
                    if eid > 0 { enh_map.insert(pos.clone(), eid); }
                }

                Some((resp.dps, cand_ids, final_raw, enh_map))
            })
            .collect();

        // 停心跳并把最终累计值刷到 progress（避免最后 100ms 的进度未同步）
        pb_hb_done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = pb_hb_thread.join();
        {
            let mut p = progress.lock().unwrap();
            p.simulated = counter_pb.load(std::sync::atomic::Ordering::Relaxed);
        }

        stats_b.simulated = final_rows.len();
        stats_b.time_ms = t_pb_start.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[phase-b] 偏导 完成 bases={} simulated={} elapsed={:.1}s",
            stats_b.bases, stats_b.simulated, stats_b.time_ms / 1000.0);
        final_rows
    } else {
        // 无 Phase B：直接把 dps_results 转 FinalRow（加速槽 enhance 进 enh_map）
        dps_results.into_iter().map(|(dps, cand_ids, enh_ids, raw)| {
            let enh_map: HashMap<String, u32> = candidate_positions.iter().zip(enh_ids.iter())
                .filter(|(_, &eid)| eid > 0)
                .map(|(p, &eid)| (p.clone(), eid))
                .collect();
            (dps, cand_ids, raw, enh_map)
        }).collect()
    };

    // 7. 排序 + top N（top_n 个回填 panel.physics_attack_power）
    let mut sorted = final_rows;
    sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(req.top_n);

    let top: Vec<AutoTopEntry> = sorted.into_iter().map(|item| {
        let (dps, cand_ids, raw, enhance_map): FinalRow = item;
        // 用 enhance_map 覆盖 default_enhances 来构 slots（让最终展示反映 winning enhance）
        let slots = {
            let mut s = req.fixed_slots.clone();
            for (i, &id) in cand_ids.iter().enumerate() {
                let pos = &candidate_positions[i];
                let chosen_enh = enhance_map.get(pos).copied()
                    .unwrap_or_else(|| req.default_enhances.get(pos).copied().unwrap_or(0));
                s.insert(pos.clone(), equip::SlotConfig {
                    equip_id: id, strength: default_str,
                    embedding: default_emb.clone(),
                    enhance_id: chosen_enh,
                    enchant_id: req.default_enchants.get(pos).copied().unwrap_or(0),
                });
            }
            s
        };
        let sl: HashMap<String, u32> = slots.iter().map(|(p, c)| (p.clone(), c.equip_id)).collect();
        let nm: HashMap<String, String> = slots.iter().filter_map(|(pos, cfg)| {
            let sub = equip::pos_to_subtype(pos);
            equip_data.items.get(&(sub, cfg.equip_id)).map(|it| (pos.clone(), it.name.clone()))
        }).collect();
        let stone_name = if req.stone_id > 0 {
            equip_data.stones.iter().find(|s| s.id == req.stone_id).map(|s| s.name.clone()).unwrap_or_default()
        } else { String::new() };
        // 重新走 calc 拿 panel.physics_attack_power（含心法转化 + 全能展开后的最终面板攻击）
        // 注意：calc_resp.raw 跟 Phase B 实际跑 sim 用的 final_raw 累加路径不完全一致
        // （前者从 slots 重新跑全流程，后者基于 init_ctx + 增量），所以 t.raw 必须用 final_raw
        // ——否则前端拿 t.raw 反推出来的 attrs 跑 sim，DPS 跟 t.dps 对不上（实测差 ~3-4%）。
        let calc_resp = equip::calculate(equip_data, &equip::CalcRequest {
            slots: slots.clone(), stone_id: req.stone_id, mount: req.mount,
            talents: req.talents.clone(),
        }, &bs, &mc);
        let panel = calc_resp.panel;
        let haste_lv = raw.haste_level as u32;
        AutoTopEntry {
            slots: sl, names: nm, dps,
            delta_pct: if baseline_dps > 0.0 { (dps - baseline_dps) / baseline_dps * 100.0 } else { 0.0 },
            haste_level: haste_lv,
            panel_attack: panel.physics_attack_power,
            panel: Some(panel),
            raw: Some(raw),   // ← Phase B 实际跑 sim 用的 final_raw（之前误用 calc_resp.raw 导致 fit_curve 对不上）
            enhances: enhance_map,
            stone_id: req.stone_id,
            stone_name,
        }
    }).collect();

    let total_ms = t_start.elapsed().as_secs_f64() * 1000.0;
    // 不在这里把 progress 标 done —— 由外层 spawn 任务在拿到本函数 Result 后统一标记，
    // 这样 result 字段也能一并写入，避免 polling 看到 done 但 result 还是 None 的窗口。
    {
        let mut p = progress.lock().unwrap();
        p.simulated = after_pareto as u64;
        p.elapsed_ms = total_ms as u64;
    }

    AutoOptimizeResponse {
        feasible: true,
        baseline_dps,
        baseline_haste,
        top,
        stats: AutoOptimizeStats {
            candidate_positions: candidate_positions.len(),
            total_combinations: total_combos,
            after_haste_pruning: after_pruning,
            unique_keys: unique_count,
            after_pareto,
            after_rank,
            smart_mode: prefilter_enabled,
            weapon_locked,
            simulated: after_rank,
            time_total_ms: total_ms,
            time_calc_ms,
            time_pareto_ms,
            time_rank_ms: t_rank_start.elapsed().as_secs_f64() * 1000.0,
            time_simulate_ms,
            phase_b_bases: stats_b.bases,
            phase_b_iters: 0,
            phase_b_unique: 0,
            phase_b_simulated: stats_b.simulated,
            time_phase_b_ms: stats_b.time_ms,
            phase_c_bases: 0,
            phase_c_iters: 0,
            phase_c_unique: 0,
            phase_c_simulated: 0,
            time_phase_c_ms: 0.0,
        },
        warnings,
        // 旧字段保留 None；属性收益曲线现走独立 /api/equip/fit_curve 端点（按需以当前候选 raw + equipment 现场拟合）
        fit_model: None,
        fit_metrics: None,
    }
}

/// Phase B 偏导后处理统计
#[derive(Default, Clone)]
struct PhaseBStats {
    bases: usize,
    simulated: usize,
    time_ms: f64,
}

async fn equip_auto_optimize_progress(
    State(state): State<SharedState>,
) -> Json<AutoSearchProgress> {
    use std::sync::atomic::Ordering;
    let mut p = state.auto_search.lock().unwrap().clone();
    // 暴露当前控制标志（atomic 由 control 端点维护，前端按钮根据这个切换文字）
    p.paused = state.auto_search_pause.load(Ordering::Relaxed);
    p.cancelled = state.auto_search_cancel.load(Ordering::Relaxed);
    Json(p)
}

// ─────────────────────────────────────────────────────────────────────────────
// Icon 代理 + 本地缓存（用户数据目录 icon_cache/{id}.png）
// 首次请求拉 icon.jx3box.com 并落盘；后续直读盘，不再走网络
// ─────────────────────────────────────────────────────────────────────────────

async fn icon_proxy(axum::extract::Path(id): axum::extract::Path<u32>) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::response::Response;

    let cache_file = icon_cache_file(id);

    // 命中盘缓存：直读返回
    if let Ok(bytes) = std::fs::read(&cache_file) {
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(Body::from(bytes))
            .unwrap();
    }

    // 未命中：拉上游 + 落盘
    let url = format!("https://icon.jx3box.com/icon/{}.png", id);
    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            match resp.bytes().await {
                Ok(bytes) => {
                    if let Some(parent) = cache_file.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&cache_file, bytes.as_ref());
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "image/png")
                        .header("Cache-Control", "public, max-age=31536000, immutable")
                        .body(Body::from(bytes))
                        .unwrap()
                }
                Err(_) => Response::builder().status(StatusCode::BAD_GATEWAY).body(Body::empty()).unwrap(),
            }
        }
        _ => Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap(),
    }
}

/// 从 icon URL 提取数字 ID（"https://icon.jx3box.com/icon/1234.png" → Some(1234)）
fn extract_icon_id(url: &str) -> Option<u32> {
    let s = url.strip_prefix("https://icon.jx3box.com/icon/")?;
    let s = s.strip_suffix(".png")?;
    s.parse().ok()
}

/// 启动后台预热：收集所有技能/buff/团辅/阵法的 icon ID，本地缺失的批量拉取
fn spawn_icon_prefetch(
    team_buffs: &[TeamBuffEntry],
    formations: &[FormationEntry],
    version: GameVersion,
) {
    use std::collections::HashSet;
    let mut ids: HashSet<u32> = HashSet::new();

    // 所有心法的技能 icon（确保切换心法后 icon 也已缓存）
    for m in &[Mount::FenShanJin, Mount::TieGuYi] {
        let dir = skills_dir(version, *m);
        for s in load_skills(Path::new(&dir)) {
            if let Some(id) = extract_icon_id(&s.icon) { ids.insert(id); }
        }
    }
    // BuffDef icon（自身 + 团辅）
    for def in scripts::all_buff_defs_by_version(version) {
        if let Some(id) = extract_icon_id(def.icon) { ids.insert(id); }
    }
    // 团辅 toml icon
    for tb in team_buffs {
        if let Some(id) = extract_icon_id(&tb.icon) { ids.insert(id); }
    }
    // 阵法 icon
    for f in formations {
        if let Some(id) = extract_icon_id(&f.icon) { ids.insert(id); }
    }
    // 虚拟技能 icon（前端硬编码用，不在 /api/skills 里）
    ids.insert(21739); // 预释放
    ids.insert(21740); // 清除冷却

    let total_icons = ids.len();

    // 过滤掉已有缓存的
    let missing: Vec<u32> = ids.into_iter().filter(|id| {
        let p = icon_cache_file(*id);
        !p.exists()
    }).collect();

    if missing.is_empty() {
        println!("[icon] 全部 {} 个 icon 已缓存", total_icons);
        return;
    }
    let total = missing.len();
    println!("[icon] 需预拉取 {}/{} 个 icon...", total, total_icons);

    tokio::spawn(async move {
        let mut ok = 0u32;
        let mut fail = 0u32;
        for id in missing {
            let url = format!("https://icon.jx3box.com/icon/{}.png", id);
            match reqwest::get(&url).await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(bytes) = resp.bytes().await {
                        let cache_file = icon_cache_file(id);
                        if let Some(parent) = cache_file.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&cache_file, bytes.as_ref());
                        ok += 1;
                    } else { fail += 1; }
                }
                _ => { fail += 1; }
            }
        }
        println!("[icon] 预拉取完成：成功 {ok}/{total}，失败 {fail}");
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// 启动
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    std::panic::set_hook(Box::new(|info| { eprintln!("[PANIC] {info}"); }));

    // 进程隔离 Router 模式：不加载任何技能数据，只做认证 + 反向代理 + per-user worker 管理。
    if std::env::var("JX3_ROUTER").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        router::run().await;
        return;
    }

    // 默认：暗影千机（2026.04）+ 分山劲（当前最新版本，排序里也是第一个）；
    // 若该用户存过心法（进程隔离回收重建场景）则恢复，避免回收后重置回默认。
    let (version, mount) = load_mount_state()
        .unwrap_or((GameVersion::AnYingQianJi, Mount::FenShanJin));

    let (constants, base_stats, mount_conversions, school_ui, workflow_a) = load_school_toml(version, mount)
        .unwrap_or_else(|e| {
            eprintln!("[warn] load school.toml failed: {} (使用内置默认常量)", e);
            (MountConstants::for_mount(mount), equip::MountBaseStats::default(), equip::MountConversions::default(), SchoolUi::default(), WorkflowA::default())
        });

    let initial_skills = load_skills(Path::new(&skills_dir(version, mount)));
    let talents = load_talents(Path::new(&talents_file(version, mount)));
    let recipes = load_recipes(Path::new(&recipes_file(version)));
    let team_buffs = load_team_buffs(Path::new(&team_buffs_file(version)));
    let formations = load_formations(Path::new(&formations_file(version)));
    let agent_provenance = agent::ToolProvenance::from_runtime_data(
        version,
        mount,
        constants,
        &initial_skills,
        &recipes,
        &team_buffs,
        &formations,
    );
    let agent_providers = match agent::provider::ProviderCatalog::load_from_env() {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!(
                "[agent] provider 配置不可用（{}），已降级为离线 provider",
                error.code
            );
            agent::provider::ProviderCatalog::offline_default()
        }
    };
    println!("[agent] 已加载 {} 个 provider profile", agent_providers.len());

    let agent_knowledge = match agent::KnowledgeIndex::from_env() {
        Ok(index) => {
            println!(
                "[agent] 知识库已加载：{} 篇文档 / {} 个分块 / corpus {}",
                index.document_count(),
                index.chunk_count(),
                &index.corpus_hash()[..12]
            );
            Some(Arc::new(index))
        }
        Err(agent::KnowledgeIndexError::NotConfigured) => {
            println!("[agent] 未配置 JX3_KNOWLEDGE_ROOT，知识库工具关闭");
            None
        }
        Err(error) => {
            eprintln!("[agent] 知识库加载失败（{error}），已降级为原有模拟 Agent");
            None
        }
    };

    // 加载装备数据（优先 equip.json；否则回退 equip/*.tab 并自动生成 JSON 缓存）
    let equip_data = equip::load_equip_smart(Path::new(data_root()));

    // 后台预拉取缺失的 icon 到本地缓存（数据 move 到 SharedState 之前取引用）
    // worker 模式（JX3_NO_BROWSER）下跳过：预拉取会同步加载两套心法技能表收集 icon ID，
    // 拖慢冷启动；worker 走 Router 共享 icon 缓存，缺失的由 /api/icon 代理按需补，无需启动期预热。
    let is_worker = std::env::var("JX3_NO_BROWSER").map(|v| !v.trim().is_empty()).unwrap_or(false);
    if !is_worker {
        spawn_icon_prefetch(&team_buffs, &formations, version);
    }

    let agent_userdata_root = userdata_base();
    let agent_sessions = match agent::session::AgentSessionStore::open(agent_userdata_root.clone()) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "[agent] 会话存储不可用（{}），主站继续运行，Agent Run 将返回固定错误",
                error.code
            );
            agent::session::AgentSessionStore::unavailable(agent_userdata_root)
        }
    };
    let agent_runs = agent::run::AgentRunManager::new(agent_sessions.clone());

    let state = SharedState {
        agent_context_gate: Arc::new(RwLock::new(())),
        agent_provenance: Arc::new(RwLock::new(agent_provenance)),
        agent_providers: Arc::new(agent_providers),
        agent_runs,
        agent_sessions,
        agent_knowledge,
        version: Arc::new(RwLock::new(version)),
        mount: Arc::new(RwLock::new(mount)),
        constants: Arc::new(RwLock::new(constants)),
        base_stats: Arc::new(RwLock::new(base_stats)),
        mount_conversions: Arc::new(RwLock::new(mount_conversions)),
        school_ui: Arc::new(RwLock::new(school_ui)),
        workflow_a: Arc::new(RwLock::new(workflow_a)),
        skills: Arc::new(RwLock::new(initial_skills)),
        talents: Arc::new(RwLock::new(talents)),
        recipes: Arc::new(RwLock::new(recipes)),
        team_buffs: Arc::new(RwLock::new(team_buffs)),
        formations: Arc::new(RwLock::new(formations)),
        optimizer: optimizer::runtime::OptState::new(),
        rl_sessions: rl::session::RlSessions::new(),
        rl_train: rl::training::TrainState::new(),
        rl_analyze: rl::analysis::AnalyzeState::new(),
        rl_pretrain: rl::pretrain::PretrainState::new(),
        equip_data: Arc::new(equip_data),
        auto_search: Arc::new(std::sync::Mutex::new(AutoSearchProgress::default())),
        auto_search_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        auto_search_pause: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    // 公网发布鉴权：仅当 JX3_AUTH_PASSWORD 非空时启用（本地开发默认关闭）。
    match std::env::var("JX3_AUTH_PASSWORD") {
        Ok(pw) if !pw.trim().is_empty() => {
            auth::init(pw, user_data_path("whitelist.json"));
        }
        _ => {
            println!("[auth] 未设置 JX3_AUTH_PASSWORD，鉴权关闭（仅本地使用）");
        }
    }

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_origin(Any)
        .allow_headers([CONTENT_TYPE, AUTHORIZATION]);

    let app = Router::new()
        .route("/health",            get(health))
        .route("/api/calculate",     post(calculate))
        .route("/api/skill_damage",  post(skill_damage))
        .route("/api/skills",        get(list_skills))
        .route("/api/skills/reload", post(reload_skills))
        .route("/api/mounts",        get(list_mounts))
        .route("/api/mounts/current", get(current_mount))
        .route("/api/mounts/defaults", get(mount_defaults))
        .route("/api/mounts/switch", post(switch_mount))
        .route("/api/talents",       get(list_talents))
        .route("/api/recipes",       get(list_recipes))
        .route("/api/team_buffs",    get(list_team_buffs))
        .route("/api/icon/:id",      get(icon_proxy))
        .route("/api/formations",    get(list_formations))
        .route("/api/simulate",      post(simulate))
        .route("/api/agent/tools/scenario", post(agent::http::scenario_handler))
        .route("/api/agent/tools/simulate", post(agent::http::simulate_handler))
        .route("/api/agent/tools/compare", post(agent::http::compare_handler))
        .route("/api/agent/tools/timeline", post(agent::http::timeline_handler))
        .route("/api/agent/providers", get(agent::provider::providers_handler))
        .route("/api/agent/runs", post(agent::run::create_run_handler))
        .route("/api/agent/runs/:run_id", get(agent::run::run_status_handler))
        .route("/api/agent/runs/:run_id/stream", get(agent::run::run_stream_handler))
        .route("/api/agent/runs/:run_id/cancel", post(agent::run::cancel_run_handler))
        .route("/api/agent/sessions", get(agent::session::list_sessions_handler))
        .route("/api/agent/sessions/:session_id", get(agent::session::get_session_handler))
        .route("/api/macro/presets", get(macro_presets))
        .route("/api/macro/save",    post(macro_save))
        .route("/api/macro/load",    get(macro_load))
        .route("/api/loop/save",        post(loop_save))
        .route("/api/loop/list",        get(loop_list))
        .route("/api/loop/load",        get(loop_load))
        .route("/api/loop/open_folder", post(loop_open_folder))
        .route("/api/loop/delete",      post(loop_delete))
        .route("/api/equip/configs/save",   post(equip_config_save))
        .route("/api/equip/configs/list",   get(equip_config_list))
        .route("/api/equip/configs/load",   get(equip_config_load))
        .route("/api/equip/configs/delete", post(equip_config_delete))
        .route("/api/resume/save",      post(resume_save))
        .route("/api/resume/load",      get(resume_load))
        .route("/api/settings",         get(settings_load).post(settings_save))
        .route("/api/macro/from_sequence", post(macro_from_sequence))
        .route("/api/macro/prune_candidates", post(macro_prune_candidates))
        .route("/api/macro/swap_candidates",    post(macro_swap_candidates))
        .route("/api/macro/tighten_candidates", post(macro_tighten_candidates))
        .route("/api/macro/batch_simulate",  post(batch_simulate))
        .route("/api/attrs/save",    post(attrs_save))
        .route("/api/attrs/load",    get(attrs_load))
        .route("/api/attrs/profiles",       get(attrs_profiles))
        .route("/api/attrs/save_profile",   post(attrs_save_profile))
        .route("/api/attrs/load_profile",   get(attrs_load_profile))
        .route("/api/attrs/delete_profile", delete(attrs_delete_profile))
        .route("/api/macro/profiles",       get(macro_profiles))
        .route("/api/macro/save_profile",   post(macro_save_profile))
        .route("/api/macro/load_profile",   get(macro_load_profile))
        .route("/api/macro/delete_profile", delete(macro_delete_profile))
        .route("/api/optimizer/analyze", post(optimizer_analyze))
        .route("/api/optimizer/start",   post(optimizer::runtime::start_handler))
        .route("/api/optimizer/stop",    post(optimizer::runtime::stop_handler))
        .route("/api/optimizer/status",  get(optimizer::runtime::status_handler))
        .route("/api/optimizer/stream",  get(optimizer::runtime::stream_handler))
        .route("/api/optimizer/runs",    get(optimizer::runtime::list_runs_handler))
        .route("/api/optimizer/runs/:id", get(optimizer::runtime::run_detail_handler))
        .route("/api/optimizer/runs/:id/file", get(optimizer::runtime::run_file_handler))
        .route("/api/optimizer/candidates", get(optimizer_candidates))
        .route("/api/rl/rollout", post(rl_rollout))
        .route("/api/rl/spec",                    get(rl::http::spec_handler))
        .route("/api/rl/sessions",                get(rl::http::list_sessions_handler))
        .route("/api/rl/env/create",              post(rl::http::create_handler))
        .route("/api/rl/env/:id/reset",           post(rl::http::reset_handler))
        .route("/api/rl/env/:id/step",            post(rl::http::step_handler))
        .route("/api/rl/env/:id/step_advance",    post(rl::http::step_advance_handler))
        .route("/api/rl/env/:id/advance",         post(rl::http::advance_handler))
        .route("/api/rl/env/:id/macro_decision",  post(rl::http::macro_decision_handler))
        .route("/api/rl/env/:id/info",            get(rl::http::info_handler))
        .route("/api/rl/env/:id/close",           post(rl::http::close_handler))
        .route("/api/rl/train/start",             post(rl::training::start_handler))
        .route("/api/rl/train/stop",              post(rl::training::stop_handler))
        .route("/api/rl/train/status",            get(rl::training::status_handler))
        .route("/api/rl/train/stream",            get(rl::training::stream_handler))
        .route("/api/rl/train/runs",              get(rl::training::list_runs_handler))
        .route("/api/rl/train/params",            get(rl::training::get_runtime_params_handler).post(rl::training::set_runtime_params_handler))
        .route("/api/rl/analyze/start",           post(rl::analysis::start_handler))
        .route("/api/rl/analyze/stop",            post(rl::analysis::stop_handler))
        .route("/api/rl/analyze/status",          get(rl::analysis::status_handler))
        .route("/api/rl/analyze/stream",          get(rl::analysis::stream_handler))
        .route("/api/rl/analyze/latest_actions",  get(rl::analysis::latest_actions_handler))
        .route("/api/rl/pretrain/start",          post(rl::pretrain::start_handler))
        .route("/api/rl/pretrain/stop",           post(rl::pretrain::stop_handler))
        .route("/api/rl/pretrain/status",         get(rl::pretrain::status_handler))
        .route("/api/rl/pretrain/stream",         get(rl::pretrain::stream_handler))
        .route("/api/rl/pretrain/list",           get(rl::pretrain::list_handler))
        // ── 配装器 ──
        .route("/api/equip/search",     post(equip_search))
        .route("/api/equip/detail",     post(equip_detail))
        .route("/api/equip/enhances",   post(equip_enhances))
        .route("/api/equip/enchants",   post(equip_enchants))
        .route("/api/equip/stones",     post(equip_stones))
        .route("/api/equip/calculate",  post(equip_calculate))
        .route("/api/equip/meta",       get(equip_meta))
        .route("/api/equip/haste_tiers", get(equip_haste_tiers))
        .route("/api/equip/auto_optimize", post(equip_auto_optimize))
        .route("/api/equip/auto_optimize/progress", get(equip_auto_optimize_progress))
        .route("/api/equip/auto_optimize/control", post(equip_auto_optimize_control))
        .route("/api/equip/fit_curve", post(equip_fit_curve))
        .route("/api/auth/login",  post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me",     get(auth::me))
        .route("/api/auth/reload", post(auth::reload))
        .fallback_service(ServeDir::new(data_path("../frontend", "frontend")))
        .with_state(state)
        .layer(cors)
        .layer(axum::middleware::from_fn(auth::middleware));

    // 响应压缩：仅在部署 worker 模式（JX3_NO_BROWSER）启用。
    //   友端经 VPS 4M 带宽 + frp 隧道，大 JSON（timeline+runtime_stats）压 5~10x，延迟/带宽双改善。
    //   压缩在 worker（家 PC）上做，CPU 充足；router 的 reqwest 无 gzip feature → 原样转发压缩字节到友端。
    //   本地 cargo run / 打包 zip 版不设 JX3_NO_BROWSER → 不启用，行为与以前完全一致，不受"传输"影响。
    //   CompressionLayer 默认 predicate 自动跳过 text/event-stream（自动配装/RL 进度流不受影响）与图片/小响应。
    let app = if is_worker {
        app.layer(tower_http::compression::CompressionLayer::new())
    } else {
        app
    };

    // 绑定地址/端口可由环境变量覆盖（worker 模式：JX3_BIND=127.0.0.1 私有口；JX3_PORT=动态端口）
    let bind_addr = std::env::var("JX3_BIND").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("JX3_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(3005);
    let listener = tokio::net::TcpListener::bind(format!("{bind_addr}:{port}")).await.unwrap();
    println!("Backend running on http://{bind_addr}:{port}");

    // 自动打开浏览器（worker 模式 JX3_NO_BROWSER=1 时跳过）
    let open_browser = std::env::var("JX3_NO_BROWSER").map(|v| v.trim().is_empty()).unwrap_or(true);
    if open_browser {
        let url = format!("http://localhost:{port}");
        #[cfg(target_os = "windows")]
        std::process::Command::new("cmd").args(["/C", "start", &url]).spawn().ok();
        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&url).spawn().ok();
        #[cfg(target_os = "linux")]
        std::process::Command::new("xdg-open").arg(&url).spawn().ok();
    }

    axum::serve(listener, app).await.unwrap();
}

// ════════════════════════════════════════════════════════════════════
// Tier 5d：Player setter 不变量测试
// 每个 setter 的契约：
//   1. 写入后字段值在合法区间
//   2. decision_generation 严格 +1
//   3. clamp / 边界处理正确
//   4. setter 是幂等的（同 input 再调一次结果不变）
// ════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod player_setter_tests {
    use super::*;

    fn mk() -> Player {
        Player::new(42087, vec![], vec![])
    }

    // ── set_rage / add_rage ──
    #[test]
    fn set_rage_clamps_high() {
        let mut p = mk();
        let g0 = p.decision_generation;
        p.set_rage(200);
        assert_eq!(p.rage, 100, "应 clamp 到 100");
        assert!(p.decision_generation > g0, "应 bump");
    }

    #[test]
    fn set_rage_clamps_low() {
        let mut p = mk();
        p.set_rage(-50);
        assert_eq!(p.rage, 0);
    }

    #[test]
    fn set_rage_bumps_each_call() {
        let mut p = mk();
        let g0 = p.decision_generation;
        p.set_rage(50);
        let g1 = p.decision_generation;
        p.set_rage(50);  // 同值再调
        assert!(g1 > g0);
        assert!(p.decision_generation > g1);
    }

    #[test]
    fn add_rage_respects_clamp() {
        let mut p = mk();
        p.set_rage(95);
        p.add_rage(10);
        assert_eq!(p.rage, 100, "95+10=105 → 100");
        p.add_rage(-200);
        assert_eq!(p.rage, 0, "100-200=-100 → 0");
    }

    #[test]
    fn add_rage_random_in_range() {
        // 1000 个随机 (init, delta) 都必须落在 [0, 100]
        let mut p = mk();
        let mut state: u64 = 0xdeadbeef;
        for _ in 0..1000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let init = (state % 200) as i32 - 50;       // -50..150
            let delta = ((state >> 32) % 200) as i32 - 100;  // -100..100
            p.set_rage(init);
            p.add_rage(delta);
            assert!(p.rage >= 0 && p.rage <= 100,
                "rage 越界: init={} delta={} → rage={}", init, delta, p.rage);
        }
    }

    // ── set_block_value / add_block_value ──
    #[test]
    fn set_block_value_clamps_to_max() {
        let mut p = mk();
        p.set_block_value(99999);
        assert_eq!(p.block_value, p.max_block_value());
    }

    #[test]
    fn set_block_value_clamps_low() {
        let mut p = mk();
        p.set_block_value(-100);
        assert_eq!(p.block_value, 0);
    }

    #[test]
    fn add_block_value_clamp() {
        let mut p = mk();
        let max_bv = p.max_block_value();
        p.set_block_value(max_bv - 5);
        p.add_block_value(20);
        assert_eq!(p.block_value, max_bv);
    }

    // ── decision_generation 单调性 ──
    #[test]
    fn decision_generation_monotonic_after_bumps() {
        let mut p = mk();
        let mut last = p.decision_generation;
        for _ in 0..100 {
            p.bump_decision_gen();
            assert!(p.decision_generation > last, "应单调递增");
            last = p.decision_generation;
        }
    }

    // ── reset_cd / reduce_cd 都 bump ──
    #[test]
    fn cd_helpers_bump() {
        let mut p = mk();
        // 注：active_cds 此时为空，调用 reset_cd 也应 bump（即使没移除任何东西）
        let g0 = p.decision_generation;
        p.reset_cd("cd_test");
        assert!(p.decision_generation > g0);

        let g1 = p.decision_generation;
        p.reduce_cd("cd_test", 1.0);
        assert!(p.decision_generation > g1);

        let g2 = p.decision_generation;
        p.reduce_charge_cd(13047, 1.0);
        assert!(p.decision_generation > g2);
    }
}
