//! 暗影千机（2026.04）- BuffDef 静态注册
//!
//! Types (BuffDef/AttribField/EffectEntry) 在顶层 `buffs.rs`；这里只放本版本的静态实例。

use crate::{
    AttribField, BuffDef, EffectEntry, BUFF_BU_CAN, BUFF_CHANG_QU, BUFF_CHENG_WU, BUFF_DUN_DANG,
    BUFF_DUN_DANG_QIAN_SHAN, BUFF_DUN_FEI, BUFF_DUN_FEI_DELAY, BUFF_DUN_WEI, BUFF_FENG_JUE,
    BUFF_FENG_LING, BUFF_FENG_MING, BUFF_FUMO_BODONG, BUFF_FU_DA_DPS_HAT, BUFF_FU_DA_DPS_YI,
    BUFF_FU_DA_T_HAT, BUFF_FU_DA_T_WRIST, BUFF_HAN_JIA, BUFF_HAN_JIA_LARGE, BUFF_HAN_JIA_SMALL,
    BUFF_HAN_XIAO, BUFF_HENG_JUE, BUFF_HUAN_SHEN, BUFF_JIAN_DING, BUFF_JIAN_TIE, BUFF_JIE_HUA,
    BUFF_JI_ANG, BUFF_JUAN_YUN, BUFF_KUANG_JUE, BUFF_LIAN_ZHAN_CD, BUFF_LIN_GUANG,
    BUFF_LIN_GUANG_COUNT, BUFF_LIU_XUE, BUFF_MIE_SHI, BUFF_NU_YAN, BUFF_SHEN_BING_WU_SHUANG,
    BUFF_SHI_XUE, BUFF_TIE_GU, BUFF_TIE_GU_SU_DI, BUFF_WU_JU, BUFF_XIAN_ZHEN_CD, BUFF_XUE_NU,
    BUFF_XUE_NU_CD, BUFF_XUE_NU_JY, BUFF_XU_RUO, BUFF_XU_RUO_DELAY, BUFF_YE_BELT_MULTI,
    BUFF_YE_BELT_SINGLE_CRIT, BUFF_YE_BELT_SINGLE_OVERCOME, BUFF_YE_HAT_CRIT, BUFF_YE_HAT_OVERCOME,
    BUFF_YE_HAT_SURPLUS, BUFF_YE_NECK_ATTACK, BUFF_YE_NECK_CRIT_EFF, BUFF_YE_PANTS_MULTI_CR,
    BUFF_YE_PANTS_MULTI_RATE, BUFF_YE_PANTS_SINGLE, BUFF_YE_PENDANT_CRIT_EFF,
    BUFF_YE_PENDANT_OVERCOME, BUFF_YE_RING_CRIT, BUFF_YE_RING_OVERCOME, BUFF_YE_RING_SURPLUS,
    BUFF_YE_SHOES_CRIT, BUFF_YE_SHOES_OVERCOME, BUFF_YUAN_GE_ID, BUFF_ZHAN_JUE, BUFF_ZHEN_FEN,
};

/// 嗜血绝刀加成秘籍 ID（由奇穴 21281 常驻激活，不依赖 buff）
pub const RECIPE_SHI_XUE_JUE_DAO: u32 = 99240;

/// 嗜血 buff 的字段加成（只有 +5% 全局增伤依赖 buff）
static EFFECTS_SHI_XUE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::AllDamageAddPercent,
        value: 51.0,
    }, // 51/1024 ≈ 5%
];

pub static BUFF_DUN_FEI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_DUN_FEI,
    name: "盾飞",
    description: "盾牌飞出攻击目标，每秒造成一次伤害",
    duration_frames: 240,
    tick_interval: 16,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 10,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6344.png",
};

static EFFECTS_XUE_NU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 102.0,
    }, // 102/1024 ≈ 10%
];
pub static BUFF_XUE_NU_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XUE_NU,
    name: "血怒",
    description: "外功基础攻击力提高10%，招式威胁值降低20%",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 3,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 20,
    short_name: None,
    effects: EFFECTS_XUE_NU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6432.png",
};

/// 惊涌·破招加成秘籍 ID
pub const RECIPE_JING_YONG_PO_ZHAO: u32 = 99220;
static RECIPES_XUE_NU_JY: &[u32] = &[RECIPE_JING_YONG_PO_ZHAO];

static EFFECTS_XUE_NU_JY: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 102.0,
    }, // 10%
    EffectEntry {
        field: AttribField::StrainBasePercentAdd,
        value: 307.0,
    }, // 30%
];

pub static BUFF_XUE_NU_JY_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XUE_NU_JY,
    name: "血怒·惊涌",
    description: "外功攻击+10%，无双+30%，斩/绝/盾舞破招+20%",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 20,
    short_name: Some("血怒"),
    effects: EFFECTS_XUE_NU_JY,
    activate_recipes: RECIPES_XUE_NU_JY,
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6351.png",
};

pub static BUFF_JIE_HUA_DEF: BuffDef = BuffDef {
    buff_id: BUFF_JIE_HUA,
    name: "劫化",
    description: "免疫控制效果（击退/被拉除外）",
    duration_frames: 64,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6346.png",
};

// 虚弱：effects 为空，由脚本按 level 设 extra_effects
// level 1 = 默认 -5%（-51），level 2 = 戍边 -7%（-72）
// effects 默认 -51 = -5%（lv1，团辅版 + 铁骨衣自身 lv1 通用）；
// 铁骨衣 lv2（北傲诀奇穴 44566）由 buff_xu_ruo_delay.rs 写 extra_effects 加 -21 差额到 -72
static EFF_XU_RUO: &[EffectEntry] = &[EffectEntry {
    field: AttribField::TargetPhysicsShieldPercent,
    value: -51.0,
}];
pub static BUFF_XU_RUO_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XU_RUO,
    name: "虚弱",
    description: "外功基础防御等级降低（level 1=-5%, level 2=-7%）",
    duration_frames: 400,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 50,
    short_name: None,
    effects: EFF_XU_RUO,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6347.png",
};

pub static BUFF_JIAN_DING_DEF: BuffDef = BuffDef {
    buff_id: BUFF_JIAN_DING,
    name: "坚定",
    description: "受到的伤害降低10%",
    duration_frames: 128,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6293.png",
};

/// 锋鸣·盾飞加成秘籍 ID
pub const RECIPE_FENG_MING_DUN_FEI: u32 = 99201;
static RECIPES_FENG_MING: &[u32] = &[RECIPE_FENG_MING_DUN_FEI];
static EFFECTS_FENG_MING: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 154.0,
    }, // 154/1024 ≈ 15%
];
pub static BUFF_FENG_MING_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FENG_MING,
    name: "锋鸣",
    description: "外功基础攻击力+15%，盾飞伤害+100%",
    duration_frames: 480,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 30,
    short_name: None,
    effects: EFFECTS_FENG_MING,
    activate_recipes: RECIPES_FENG_MING,
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6326.png",
};

pub static BUFF_DUN_FEI_DELAY_DEF: BuffDef = BuffDef {
    buff_id: BUFF_DUN_FEI_DELAY,
    name: "盾飞延迟",
    description: "盾牌飞出中，即将切换擎刀体态",
    duration_frames: 6,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6344.png",
};

pub static BUFF_LIU_XUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_LIU_XUE,
    name: "流血",
    description: "每2秒受到外功伤害（tick_interval 和 duration 受加速影响，总跳数不变）",
    duration_frames: 416,
    tick_interval: 32,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 40,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: true,
    icon: "https://icon.jx3box.com/icon/6323.png",
};

pub static BUFF_KUANG_JUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_KUANG_JUE,
    name: "狂绝",
    description: "绝刀未击杀返还怒气，可额外施展一次绝刀",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 35,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6317.png",
};

pub static BUFF_XU_RUO_DELAY_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XU_RUO_DELAY,
    name: "虚弱延迟",
    description: "即将给目标添加虚弱",
    duration_frames: 2,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6347.png",
};

pub static BUFF_YUAN_GE: BuffDef = BuffDef {
    buff_id: BUFF_YUAN_GE_ID,
    name: "援戈",
    description: "施展苍雪刀套路下招式将附带一次外功伤害并消耗一层",
    duration_frames: 960,
    tick_interval: 0,
    max_stacks: 12,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 25,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/20064.png",
};

pub static BUFF_LIN_GUANG_DEF: BuffDef = BuffDef {
    buff_id: BUFF_LIN_GUANG,
    name: "麟光甲",
    description: "苍雪刀招式消耗一层，附带麟光甲寒额外伤害（最多9次）",
    duration_frames: 400,
    tick_interval: 0,
    max_stacks: 9,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 60,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/19154.png",
};

pub static BUFF_LIN_GUANG_COUNT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_LIN_GUANG_COUNT,
    name: "麟光计数",
    description: "累计触发麟光甲寒次数",
    duration_frames: 400,
    tick_interval: 0,
    max_stacks: 3,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6315.png",
};

pub static BUFF_CHANG_QU_DEF: BuffDef = BuffDef {
    buff_id: BUFF_CHANG_QU,
    name: "长驱万里",
    description: "苍雪刀破招叠层；≥6层可施展阵云结晦·雾海",
    duration_frames: 1000, // 62.5秒
    tick_interval: 0,
    max_stacks: 45,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 50,
    short_name: Some("长驱"),
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/13373.png",
};

pub static BUFF_XUE_NU_CD_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XUE_NU_CD,
    name: "血怒叠层",
    description: "2秒内再次施展血怒可叠加层数",
    duration_frames: 32,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6432.png",
};

pub static BUFF_XIAN_ZHEN_CD_DEF: BuffDef = BuffDef {
    buff_id: BUFF_XIAN_ZHEN_CD,
    name: "陷阵·地坼冷却",
    description: "陷阵奇穴：地坼 15 秒内置 CD",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/24943.png",
};

pub static BUFF_CHENG_WU_DEF: BuffDef = BuffDef {
    buff_id: BUFF_CHENG_WU,
    name: "橙武",
    description: "擎刀：绝刀伤害+30%/消耗-100%；擎盾：盾压无调息/怒气+100%",
    duration_frames: 64,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/23384.png",
};

pub static BUFF_SHI_XUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_SHI_XUE,
    name: "嗜血",
    description: "造成伤害提高5%（双会+绝刀+40% 由奇穴 21281 常驻，不依赖本 buff）",
    duration_frames: 192,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 35,
    short_name: None,
    effects: EFFECTS_SHI_XUE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6288.png",
};

pub static BUFF_JUAN_YUN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_JUAN_YUN,
    name: "卷云",
    description: "移动速度降低60%",
    duration_frames: 128,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6345.png",
};

// 战绝：突破上限加速 +15%（154/1024）
static EFFECTS_ZHAN_JUE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::UnlimitedAdditionalHastePercent,
    value: 154.0,
}];
pub static BUFF_ZHAN_JUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_ZHAN_JUE,
    name: "战绝",
    description: "加速率+15%，每3秒回复100点怒气",
    duration_frames: 144,
    tick_interval: 48,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 15,
    short_name: None,
    effects: EFFECTS_ZHAN_JUE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/102041.png",
};

pub static BUFF_BU_CAN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_BU_CAN,
    name: "步残",
    description: "无法施展轻功",
    duration_frames: 64,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6284.png",
};

// 普通盾挡（8499）：1~10 级由 BuffInstance.level 区分；动态加 ParryValueBase
pub static BUFF_DUN_DANG_DEF: BuffDef = BuffDef {
    buff_id: BUFF_DUN_DANG,
    name: "盾挡",
    description: "消耗怒气按级提高拆招值（系数见 atVitalityToParryValueCof 表）",
    duration_frames: 176,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 70,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6301.png",
};

// 千山盾挡（8448）：千山奇穴 13421 强化版（系数约 1.25x）
pub static BUFF_DUN_DANG_QIAN_SHAN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_DUN_DANG_QIAN_SHAN,
    name: "盾挡",
    description: "千山奇穴强化：消耗怒气按级提高拆招值（1.25x 普通盾挡）",
    duration_frames: 176,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 70,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6301.png",
};

pub static BUFF_HUAN_SHEN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HUAN_SHEN,
    name: "缓深",
    description: "效果期间施展盾压无法触发封轻功效果",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6332.png",
};

// ── 寒啸千军（铁骨衣）──
static EFFECTS_HAN_XIAO: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 67.0,
}];
pub static BUFF_HAN_XIAO_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HAN_XIAO,
    name: "寒啸千军",
    description: "每3310基础体质+67无双等级",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 25,
    short_name: Some("寒啸"),
    effects: EFFECTS_HAN_XIAO,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7514.png",
};

// ── 神兵·无双气劲（橙武装备特效；命中触发，每层 +X StrainBase 由武器档位决定）──
// effects 静态空 — 实际数值在 cast_skill 触发 hook 里通过 bind_buff_effects 动态绑定
pub static BUFF_SHEN_BING_WU_SHUANG_DEF: BuffDef = BuffDef {
    buff_id: BUFF_SHEN_BING_WU_SHUANG,
    name: "神兵·无双",
    description: "命中后获得，提升无双等级（按主武器档位绑定单层值），最多 5 层 6 秒",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 5,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3406.png",
};

// ── 大附魔（按装备面板原文）──
// DPS 帽 16453: 气血>75% +4099 破防
static EFFECTS_FU_DA_DPS_HAT: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 4099.0,
}];
pub static BUFF_FU_DA_DPS_HAT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FU_DA_DPS_HAT,
    name: "伤·帽",
    description: "气血>75% +4099 破防（伤·帽 16453）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 90,
    short_name: None,
    effects: EFFECTS_FU_DA_DPS_HAT,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3404.png",
};
// DPS 衣 16452: 永久 +1114 外攻 +1243 内攻（DPS 心法只用外攻）
static EFFECTS_FU_DA_DPS_YI: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 1114.0,
}];
pub static BUFF_FU_DA_DPS_YI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FU_DA_DPS_YI,
    name: "伤·衣",
    description: "永久 +1114 外攻 +1243 内攻（伤·衣 16452）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 91,
    short_name: None,
    effects: EFFECTS_FU_DA_DPS_YI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3404.png",
};
// T 帽 16443: Cast 触发自身 +1114 外攻 8s
static EFFECTS_FU_DA_T_HAT: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 1114.0,
}];
pub static BUFF_FU_DA_T_HAT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FU_DA_T_HAT,
    name: "御·帽",
    description: "Cast 触发 +1114 外攻 8s（御·帽 16443，对自己也生效）",
    duration_frames: 128,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 90,
    short_name: Some("御·帽"),
    effects: EFFECTS_FU_DA_T_HAT,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3405.png",
};
// T 腕 16441: Cast 10% 触发 +5% 全伤害 5s, CD 25s
static EFFECTS_FU_DA_T_WRIST: &[EffectEntry] = &[EffectEntry {
    field: AttribField::AllDamageAddPercent,
    value: 50.0,
}];
pub static BUFF_FU_DA_T_WRIST_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FU_DA_T_WRIST,
    name: "御·腕",
    description: "Cast 10% 触发 +5% 全伤害 5s, CD 25s（御·腕 16441）",
    duration_frames: 80,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 90,
    short_name: Some("御·腕"),
    effects: EFFECTS_FU_DA_T_WRIST,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3405.png",
};

// ── 黄字 帽 38934 max-pick 三变体 ──
static EFFECTS_YE_HAT_OV: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 3471.0,
}];
pub static BUFF_YE_HAT_OVERCOME_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_HAT_OVERCOME,
    name: "帽·破防特效",
    description: "黄字帽 38934 进战 max-pick 选破防 +3471",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 80,
    short_name: Some("帽·破"),
    effects: EFFECTS_YE_HAT_OV,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};
static EFFECTS_YE_HAT_CR: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalStrike,
    value: 3471.0,
}];
pub static BUFF_YE_HAT_CRIT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_HAT_CRIT,
    name: "帽·会心特效",
    description: "黄字帽 38934 进战 max-pick 选会心 +3471",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 80,
    short_name: Some("帽·会"),
    effects: EFFECTS_YE_HAT_CR,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3428.png",
};
static EFFECTS_YE_HAT_SP: &[EffectEntry] = &[EffectEntry {
    field: AttribField::SurplusValueBase,
    value: 3471.0,
}];
pub static BUFF_YE_HAT_SURPLUS_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_HAT_SURPLUS,
    name: "帽·破招特效",
    description: "黄字帽 38934 进战 max-pick 选破招 +3471",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 80,
    short_name: Some("帽·招"),
    effects: EFFECTS_YE_HAT_SP,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3406.png",
};

// ── 黄字 项链 38945/38946 多层 buff ──
static EFFECTS_YE_NECK_CRIT_EFF: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalDamagePowerBase,
    value: 192.0,
}];
pub static BUFF_YE_NECK_CRIT_EFF_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_NECK_CRIT_EFF,
    name: "项链·会效转化",
    description: "黄字项链 38945 进战 每 8586 会心 → 1 层 +192 会效, max 10",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 10,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 80,
    short_name: Some("项·效"),
    effects: EFFECTS_YE_NECK_CRIT_EFF,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3429.png",
};
static EFFECTS_YE_NECK_ATTACK: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 52.0,
}];
pub static BUFF_YE_NECK_ATTACK_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_NECK_ATTACK,
    name: "项链·攻击转化",
    description: "黄字项链 38946 进战 每 8586 破防 → 1 层 +52 攻击, max 10",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 10,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 80,
    short_name: Some("项·攻"),
    effects: EFFECTS_YE_NECK_ATTACK,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3422.png",
};
static EFFECTS_YE_BELT_MULTI: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsCriticalStrike,
        value: 1350.0,
    },
    EffectEntry {
        field: AttribField::PhysicsOvercomeBase,
        value: 1350.0,
    },
];
pub static BUFF_YE_BELT_MULTI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_BELT_MULTI,
    name: "腰带·多属性",
    description: "进战 +1350 破防+会心",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 92,
    short_name: None,
    effects: EFFECTS_YE_BELT_MULTI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3418.png",
};
static EFFECTS_YE_PANTS_SINGLE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 3857.0,
}];
pub static BUFF_YE_PANTS_SINGLE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PANTS_SINGLE,
    name: "裤·单属性",
    description: "进战 +3857 无双",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 93,
    short_name: None,
    effects: EFFECTS_YE_PANTS_SINGLE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3418.png",
};

// ── 输出伤害波动（DPS 腰附魔 16449；期望 3.8% 全局增伤；8s, CD 30s）──
static EFFECTS_FUMO_BODONG: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::AllDamageAddPercent,
        value: 38.0,
    }, // 38/1024 ≈ 3.71%
];
pub static BUFF_FUMO_BODONG_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FUMO_BODONG,
    name: "伤·腰",
    description: "Hit 20% 触发, +3.8% 全局增伤(30%×1%+70%×5% 期望), 8s, CD 30s",
    duration_frames: 128,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 50,
    short_name: Some("伤·腰"),
    effects: EFFECTS_FUMO_BODONG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3404.png",
};

// ── 鞋·会心 黄字特效（38939 → buff 29524 lvl 8）──
static EFFECTS_YE_SHOES_CRIT: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalStrike,
    value: 6000.0,
}];
pub static BUFF_YE_SHOES_CRIT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_SHOES_CRIT,
    name: "鞋·会心特效",
    description: "命中触发 +6000 会心等级，10s（副本/无修精简鞋 38939）",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 51,
    short_name: Some("鞋·会"),
    effects: EFFECTS_YE_SHOES_CRIT,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3428.png",
};

// ── 鞋·破防 黄字特效（38944 → buff 29526 lvl 8）──
static EFFECTS_YE_SHOES_OVERCOME: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 6000.0,
}];
pub static BUFF_YE_SHOES_OVERCOME_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_SHOES_OVERCOME,
    name: "鞋·破防特效",
    description: "命中触发 +6000 破防等级，10s（副本/无修精简鞋 38944）",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 52,
    short_name: Some("鞋·破"),
    effects: EFFECTS_YE_SHOES_OVERCOME,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};

// ── 腰单·会 (buff 30743 lvl 7) +18561 会心 20s ──
static EFFECTS_YE_BELT_SINGLE_CRIT: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalStrike,
    value: 18561.0,
}];
pub static BUFF_YE_BELT_SINGLE_CRIT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_BELT_SINGLE_CRIT,
    name: "腰带·会心特效",
    description: "proc +18561 会心，20s（精简腰带 40791 词条会）",
    duration_frames: 320,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 53,
    short_name: Some("腰单会"),
    effects: EFFECTS_YE_BELT_SINGLE_CRIT,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3428.png",
};
static EFFECTS_YE_BELT_SINGLE_OVERCOME: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 18561.0,
}];
pub static BUFF_YE_BELT_SINGLE_OVERCOME_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_BELT_SINGLE_OVERCOME,
    name: "腰带·破防特效",
    description: "proc +18561 破防，20s（精简腰带 40791 词条破）",
    duration_frames: 320,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 54,
    short_name: Some("腰单破"),
    effects: EFFECTS_YE_BELT_SINGLE_OVERCOME,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};

// ── 裤多·无双率 (buff 30749) +181 strain_rate ──
static EFFECTS_YE_PANTS_RATE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBasePercentAdd,
    value: 181.0,
}];
pub static BUFF_YE_PANTS_MULTI_RATE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PANTS_MULTI_RATE,
    name: "裤·无双率特效",
    description: "proc +17.7% 无双率（精简裤 40794 当前无双<90%）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 55,
    short_name: None,
    effects: EFFECTS_YE_PANTS_RATE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3418.png",
};
static EFFECTS_YE_PANTS_CR: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalStrike,
    value: 23540.0,
}];
pub static BUFF_YE_PANTS_MULTI_CR_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PANTS_MULTI_CR,
    name: "裤·会心特效",
    description: "+23540 会心（30770 L6, 无双>90%）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 56,
    short_name: None,
    effects: EFFECTS_YE_PANTS_CR,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3428.png",
};

use crate::BUFF_YE_PANTS_MULTI_OV;
static EFFECTS_YE_PANTS_OV: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 23540.0,
}];
pub static BUFF_YE_PANTS_MULTI_OV_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PANTS_MULTI_OV,
    name: "裤·破防特效",
    description: "+23540 破防（30749 L6, 无双>90%）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 56,
    short_name: None,
    effects: EFFECTS_YE_PANTS_OV,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};

// ── 腰坠·破防 (buff 29536) +385/层 5 层 12s ──
static EFFECTS_YE_PENDANT_OVERCOME: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 385.0,
}];
pub static BUFF_YE_PENDANT_OVERCOME_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PENDANT_OVERCOME,
    name: "腰坠·破防特效",
    description: "命中触发 +385 破防/层，5 层 12s（精简腰坠 38948）",
    duration_frames: 192,
    tick_interval: 0,
    max_stacks: 5,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 57,
    short_name: Some("坠破"),
    effects: EFFECTS_YE_PENDANT_OVERCOME,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};
static EFFECTS_YE_PENDANT_CRIT_EFF: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalDamagePowerBase,
    value: 385.0,
}];
pub static BUFF_YE_PENDANT_CRIT_EFF_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_PENDANT_CRIT_EFF,
    name: "腰坠·会效特效",
    description: "命中触发 +385 会效/层，5 层 12s（精简腰坠 38949）",
    duration_frames: 192,
    tick_interval: 0,
    max_stacks: 5,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 58,
    short_name: Some("坠效"),
    effects: EFFECTS_YE_PENDANT_CRIT_EFF,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3429.png",
};

// ── 戒指·破防 (buff 30755) +1928 破防 ──
static EFFECTS_YE_RING_OVERCOME: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 1928.0,
}];
pub static BUFF_YE_RING_OVERCOME_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_RING_OVERCOME,
    name: "戒指·破防特效",
    description: "+1928 破防 10s（精简戒指 40802）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 59,
    short_name: None,
    effects: EFFECTS_YE_RING_OVERCOME,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3424.png",
};
static EFFECTS_YE_RING_CRIT: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsCriticalStrike,
    value: 1928.0,
}];
pub static BUFF_YE_RING_CRIT_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_RING_CRIT,
    name: "戒指·会心特效",
    description: "+1928 会心 10s（精简戒指 40803）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 60,
    short_name: None,
    effects: EFFECTS_YE_RING_CRIT,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3428.png",
};
static EFFECTS_YE_RING_SURPLUS: &[EffectEntry] = &[EffectEntry {
    field: AttribField::SurplusValueBase,
    value: 1928.0,
}];
pub static BUFF_YE_RING_SURPLUS_DEF: BuffDef = BuffDef {
    buff_id: BUFF_YE_RING_SURPLUS,
    name: "戒指·破招特效",
    description: "+1928 破招 10s（精简戒指 40804）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 61,
    short_name: None,
    effects: EFFECTS_YE_RING_SURPLUS,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3406.png",
};

// 振奋（奇穴 13422）：盾挡后 30s；每层 +111 StrainBase，max_stacks=100
static EFFECTS_ZHEN_FEN: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 111.0,
}];
pub static BUFF_ZHEN_FEN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_ZHEN_FEN,
    name: "振奋",
    description: "每 3310 基础体质 +111 无双等级（每层 +111 StrainBase，上限 100 层），持续 30 秒",
    duration_frames: 480,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 24,
    short_name: Some("振奋"),
    effects: EFFECTS_ZHEN_FEN,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6436.png",
};

// 蔑视（奇穴 39045）：伤害招式命中后获得，外功会心率和会心效果各+11%；30 秒
static EFFECTS_MIE_SHI: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsCriticalStrikePercent,
        value: 112.0,
    }, // 112/1024 ≈ 11%
    EffectEntry {
        field: AttribField::PhysicsCriticalDamagePowerPercent,
        value: 112.0,
    }, // 112/1024 ≈ 11%
];
pub static BUFF_MIE_SHI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_MIE_SHI,
    name: "蔑视",
    description: "外功会心率和会心效果各提高11%",
    duration_frames: 480,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 28,
    short_name: Some("蔑视"),
    effects: EFFECTS_MIE_SHI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7428.png",
};

// ── 寒甲（铁骨衣 13134 奇穴）──
pub static BUFF_HAN_JIA_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HAN_JIA,
    name: "寒甲",
    description: "提升自身外功攻击力",
    duration_frames: 192,
    tick_interval: 48,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 30,
    short_name: Some("寒甲"),
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6309.png",
};

static EFFECTS_HAN_JIA_SMALL: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 300.0,
}];
pub static BUFF_HAN_JIA_SMALL_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HAN_JIA_SMALL,
    name: "寒甲·小",
    description: "寒甲内部层（每层 +300 外功攻击）",
    duration_frames: 192,
    tick_interval: 0,
    max_stacks: 125,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 0,
    short_name: None,
    effects: EFFECTS_HAN_JIA_SMALL,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6309.png",
};

static EFFECTS_HAN_JIA_LARGE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 30000.0,
}];
pub static BUFF_HAN_JIA_LARGE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HAN_JIA_LARGE,
    name: "寒甲·大",
    description: "寒甲内部层（每层 +30000 外功攻击）",
    duration_frames: 192,
    tick_interval: 0,
    max_stacks: 125,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 0,
    short_name: None,
    effects: EFFECTS_HAN_JIA_LARGE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6309.png",
};

// 坚铁 (8272)：每层 +6% 招架率（atParryBaseRate），最多 5 层
static EFFECTS_JIAN_TIE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::ParryValuePercent,
        value: 600.0,
    }, // 6% = 600/10000
];
pub static BUFF_JIAN_TIE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_JIAN_TIE,
    name: "坚铁",
    description: "每层使自身招架率提高6%",
    duration_frames: 128,
    tick_interval: 0,
    max_stacks: 5,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 0,
    short_name: Some("坚铁"),
    effects: EFFECTS_JIAN_TIE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6353.png",
};

pub static BUFF_NU_YAN_DEF: BuffDef = BuffDef {
    buff_id: BUFF_NU_YAN,
    name: "怒炎",
    description: "下一次施展绝刀可返还所消耗的怒气",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 26,
    short_name: Some("怒炎"),
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6348.png",
};

pub static BUFF_WU_JU_DEF: BuffDef = BuffDef {
    buff_id: BUFF_WU_JU,
    name: "无惧",
    description: "免疫恐惧和控制效果",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 35,
    short_name: Some("无惧"),
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6299.png",
};

static EFFECTS_JI_ANG: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::ParryBase,
        value: 66.0,
    },
    EffectEntry {
        field: AttribField::ParryValueBase,
        value: 166.0,
    },
    EffectEntry {
        field: AttribField::SurplusValueBase,
        value: 331.0,
    },
];
pub static BUFF_JI_ANG_DEF: BuffDef = BuffDef {
    buff_id: BUFF_JI_ANG,
    name: "激昂",
    description: "每层提高招架等级、拆招值、破招值",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 32,
    short_name: Some("激昂"),
    effects: EFFECTS_JI_ANG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/2037.png",
};

pub static BUFF_LIAN_ZHAN_CD_DEF: BuffDef = BuffDef {
    buff_id: BUFF_LIAN_ZHAN_CD,
    name: "恋战CD",
    description: "坚铁招架成功后内置冷却，期间不再叠层",
    duration_frames: 132,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 0,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6303.png",
};

// 盾威 (8397)：目标 debuff，伤害输出-5%（纯防御，无 DPS 属性），15s
pub static BUFF_DUN_WEI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_DUN_WEI,
    name: "盾威",
    description: "目标伤害输出降低5%",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 35,
    short_name: Some("盾威"),
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6285.png",
};

// 铁骨 (29938)：副T 体质→攻击/破防（永久，战斗中激活）
static EFFECTS_TIE_GU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::VitalityToAttackCof,
        value: 0.198,
    },
    EffectEntry {
        field: AttribField::VitalityToOvercomeCof,
        value: 0.152,
    },
];
pub static BUFF_TIE_GU_DEF: BuffDef = BuffDef {
    buff_id: BUFF_TIE_GU,
    name: "铁骨",
    description: "每点体质提高0.198攻击力、0.152破防等级",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: Some("铁骨"),
    effects: EFFECTS_TIE_GU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6315.png",
};

// 铁骨·宿敌 (17885)：主T 体质→攻击/破防 ×3（永久，战斗中激活）
static EFFECTS_TIE_GU_SU_DI: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::VitalityToAttackCof,
        value: 0.594,
    },
    EffectEntry {
        field: AttribField::VitalityToOvercomeCof,
        value: 0.456,
    },
];
pub static BUFF_TIE_GU_SU_DI_DEF: BuffDef = BuffDef {
    buff_id: BUFF_TIE_GU_SU_DI,
    name: "铁骨·宿敌",
    description: "每点体质提高0.594攻击力、0.456破防等级（第一仇恨）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: Some("宿敌"),
    effects: EFFECTS_TIE_GU_SU_DI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6315.png",
};

// ─────────────────────────────────────────────────────────────────────────────
// 注册表
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_buff_def(buff_id: u32) -> Option<&'static BuffDef> {
    match buff_id {
        BUFF_YUAN_GE_ID => Some(&BUFF_YUAN_GE),
        BUFF_DUN_FEI => Some(&BUFF_DUN_FEI_DEF),
        BUFF_XUE_NU => Some(&BUFF_XUE_NU_DEF),
        BUFF_XUE_NU_JY => Some(&BUFF_XUE_NU_JY_DEF),
        BUFF_JIE_HUA => Some(&BUFF_JIE_HUA_DEF),
        BUFF_XU_RUO => Some(&BUFF_XU_RUO_DEF),
        BUFF_JIAN_DING => Some(&BUFF_JIAN_DING_DEF),
        BUFF_FENG_MING => Some(&BUFF_FENG_MING_DEF),
        BUFF_DUN_FEI_DELAY => Some(&BUFF_DUN_FEI_DELAY_DEF),
        BUFF_LIU_XUE => Some(&BUFF_LIU_XUE_DEF),
        BUFF_KUANG_JUE => Some(&BUFF_KUANG_JUE_DEF),
        BUFF_XU_RUO_DELAY => Some(&BUFF_XU_RUO_DELAY_DEF),
        BUFF_BU_CAN => Some(&BUFF_BU_CAN_DEF),
        BUFF_HUAN_SHEN => Some(&BUFF_HUAN_SHEN_DEF),
        BUFF_XUE_NU_CD => Some(&BUFF_XUE_NU_CD_DEF),
        BUFF_XIAN_ZHEN_CD => Some(&BUFF_XIAN_ZHEN_CD_DEF),
        BUFF_LIN_GUANG => Some(&BUFF_LIN_GUANG_DEF),
        BUFF_LIN_GUANG_COUNT => Some(&BUFF_LIN_GUANG_COUNT_DEF),
        BUFF_CHENG_WU => Some(&BUFF_CHENG_WU_DEF),
        BUFF_SHI_XUE => Some(&BUFF_SHI_XUE_DEF),
        BUFF_JUAN_YUN => Some(&BUFF_JUAN_YUN_DEF),
        BUFF_ZHAN_JUE => Some(&BUFF_ZHAN_JUE_DEF),
        BUFF_DUN_DANG => Some(&BUFF_DUN_DANG_DEF),
        BUFF_DUN_DANG_QIAN_SHAN => Some(&BUFF_DUN_DANG_QIAN_SHAN_DEF),
        BUFF_ZHEN_FEN => Some(&BUFF_ZHEN_FEN_DEF),
        BUFF_MIE_SHI => Some(&BUFF_MIE_SHI_DEF),
        BUFF_HAN_XIAO => Some(&BUFF_HAN_XIAO_DEF),
        BUFF_SHEN_BING_WU_SHUANG => Some(&BUFF_SHEN_BING_WU_SHUANG_DEF),
        BUFF_FU_DA_DPS_HAT => Some(&BUFF_FU_DA_DPS_HAT_DEF),
        BUFF_FU_DA_DPS_YI => Some(&BUFF_FU_DA_DPS_YI_DEF),
        BUFF_FU_DA_T_HAT => Some(&BUFF_FU_DA_T_HAT_DEF),
        BUFF_FU_DA_T_WRIST => Some(&BUFF_FU_DA_T_WRIST_DEF),
        BUFF_YE_BELT_MULTI => Some(&BUFF_YE_BELT_MULTI_DEF),
        BUFF_YE_PANTS_SINGLE => Some(&BUFF_YE_PANTS_SINGLE_DEF),
        BUFF_YE_HAT_OVERCOME => Some(&BUFF_YE_HAT_OVERCOME_DEF),
        BUFF_YE_HAT_CRIT => Some(&BUFF_YE_HAT_CRIT_DEF),
        BUFF_YE_HAT_SURPLUS => Some(&BUFF_YE_HAT_SURPLUS_DEF),
        BUFF_YE_NECK_CRIT_EFF => Some(&BUFF_YE_NECK_CRIT_EFF_DEF),
        BUFF_YE_NECK_ATTACK => Some(&BUFF_YE_NECK_ATTACK_DEF),
        BUFF_FUMO_BODONG => Some(&BUFF_FUMO_BODONG_DEF),
        BUFF_YE_SHOES_CRIT => Some(&BUFF_YE_SHOES_CRIT_DEF),
        BUFF_YE_SHOES_OVERCOME => Some(&BUFF_YE_SHOES_OVERCOME_DEF),
        BUFF_YE_BELT_SINGLE_CRIT => Some(&BUFF_YE_BELT_SINGLE_CRIT_DEF),
        BUFF_YE_BELT_SINGLE_OVERCOME => Some(&BUFF_YE_BELT_SINGLE_OVERCOME_DEF),
        BUFF_YE_PANTS_MULTI_RATE => Some(&BUFF_YE_PANTS_MULTI_RATE_DEF),
        BUFF_YE_PANTS_MULTI_CR => Some(&BUFF_YE_PANTS_MULTI_CR_DEF),
        BUFF_YE_PANTS_MULTI_OV => Some(&BUFF_YE_PANTS_MULTI_OV_DEF),
        BUFF_YE_PENDANT_OVERCOME => Some(&BUFF_YE_PENDANT_OVERCOME_DEF),
        BUFF_YE_PENDANT_CRIT_EFF => Some(&BUFF_YE_PENDANT_CRIT_EFF_DEF),
        BUFF_YE_RING_OVERCOME => Some(&BUFF_YE_RING_OVERCOME_DEF),
        BUFF_YE_RING_CRIT => Some(&BUFF_YE_RING_CRIT_DEF),
        BUFF_YE_RING_SURPLUS => Some(&BUFF_YE_RING_SURPLUS_DEF),
        BUFF_HAN_JIA => Some(&BUFF_HAN_JIA_DEF),
        BUFF_HAN_JIA_SMALL => Some(&BUFF_HAN_JIA_SMALL_DEF),
        BUFF_HAN_JIA_LARGE => Some(&BUFF_HAN_JIA_LARGE_DEF),
        BUFF_JIAN_TIE => Some(&BUFF_JIAN_TIE_DEF),
        BUFF_LIAN_ZHAN_CD => Some(&BUFF_LIAN_ZHAN_CD_DEF),
        BUFF_NU_YAN => Some(&BUFF_NU_YAN_DEF),
        BUFF_WU_JU => Some(&BUFF_WU_JU_DEF),
        BUFF_JI_ANG => Some(&BUFF_JI_ANG_DEF),
        BUFF_DUN_WEI => Some(&BUFF_DUN_WEI_DEF),
        BUFF_TIE_GU => Some(&BUFF_TIE_GU_DEF),
        BUFF_TIE_GU_SU_DI => Some(&BUFF_TIE_GU_SU_DI_DEF),
        BUFF_CHANG_QU => Some(&BUFF_CHANG_QU_DEF),
        BUFF_FENG_LING => Some(&BUFF_FENG_LING_DEF),
        BUFF_FENG_JUE => Some(&BUFF_FENG_JUE_DEF),
        BUFF_HENG_JUE => Some(&BUFF_HENG_JUE_DEF),
        // 团队增益（50 条）：fallback 到 team_buffs 注册表
        _ => super::team_buffs::get_team_buff_def(buff_id),
    }
}

/// 所有 BuffDef 列表（与 get_buff_def 的 match arms 配套维护）
/// 用于启动时扫描 effects 含特定字段的 buff（如 AllDamageAddPercent 增伤型）
/// 新增 BuffDef 时务必同时加进 match 和这里
pub fn all_buff_defs() -> Vec<&'static BuffDef> {
    let mut v: Vec<&'static BuffDef> = vec![
        &BUFF_YUAN_GE,
        &BUFF_DUN_FEI_DEF,
        &BUFF_XUE_NU_DEF,
        &BUFF_XUE_NU_JY_DEF,
        &BUFF_JIE_HUA_DEF,
        &BUFF_XU_RUO_DEF,
        &BUFF_JIAN_DING_DEF,
        &BUFF_FENG_MING_DEF,
        &BUFF_DUN_FEI_DELAY_DEF,
        &BUFF_LIU_XUE_DEF,
        &BUFF_KUANG_JUE_DEF,
        &BUFF_XU_RUO_DELAY_DEF,
        &BUFF_BU_CAN_DEF,
        &BUFF_HUAN_SHEN_DEF,
        &BUFF_XUE_NU_CD_DEF,
        &BUFF_XIAN_ZHEN_CD_DEF,
        &BUFF_LIN_GUANG_DEF,
        &BUFF_LIN_GUANG_COUNT_DEF,
        &BUFF_CHENG_WU_DEF,
        &BUFF_SHI_XUE_DEF,
        &BUFF_JUAN_YUN_DEF,
        &BUFF_ZHAN_JUE_DEF,
        &BUFF_DUN_DANG_DEF,
        &BUFF_DUN_DANG_QIAN_SHAN_DEF,
        &BUFF_ZHEN_FEN_DEF,
        &BUFF_MIE_SHI_DEF,
        &BUFF_HAN_XIAO_DEF,
        &BUFF_SHEN_BING_WU_SHUANG_DEF,
        &BUFF_FU_DA_DPS_HAT_DEF,
        &BUFF_FU_DA_DPS_YI_DEF,
        &BUFF_FU_DA_T_HAT_DEF,
        &BUFF_FU_DA_T_WRIST_DEF,
        &BUFF_YE_BELT_MULTI_DEF,
        &BUFF_YE_PANTS_SINGLE_DEF,
        &BUFF_YE_HAT_OVERCOME_DEF,
        &BUFF_YE_HAT_CRIT_DEF,
        &BUFF_YE_HAT_SURPLUS_DEF,
        &BUFF_YE_NECK_CRIT_EFF_DEF,
        &BUFF_YE_NECK_ATTACK_DEF,
        &BUFF_FUMO_BODONG_DEF,
        &BUFF_YE_SHOES_CRIT_DEF,
        &BUFF_YE_SHOES_OVERCOME_DEF,
        &BUFF_YE_BELT_SINGLE_CRIT_DEF,
        &BUFF_YE_BELT_SINGLE_OVERCOME_DEF,
        &BUFF_YE_PANTS_MULTI_RATE_DEF,
        &BUFF_YE_PANTS_MULTI_CR_DEF,
        &BUFF_YE_PANTS_MULTI_OV_DEF,
        &BUFF_YE_PENDANT_OVERCOME_DEF,
        &BUFF_YE_PENDANT_CRIT_EFF_DEF,
        &BUFF_YE_RING_OVERCOME_DEF,
        &BUFF_YE_RING_CRIT_DEF,
        &BUFF_YE_RING_SURPLUS_DEF,
        &BUFF_HAN_JIA_DEF,
        &BUFF_HAN_JIA_SMALL_DEF,
        &BUFF_HAN_JIA_LARGE_DEF,
        &BUFF_JIAN_TIE_DEF,
        &BUFF_LIAN_ZHAN_CD_DEF,
        &BUFF_NU_YAN_DEF,
        &BUFF_WU_JU_DEF,
        &BUFF_JI_ANG_DEF,
        &BUFF_DUN_WEI_DEF,
        &BUFF_TIE_GU_DEF,
        &BUFF_TIE_GU_SU_DI_DEF,
        &BUFF_CHANG_QU_DEF,
        &BUFF_FENG_LING_DEF,
        &BUFF_FENG_JUE_DEF,
        &BUFF_HENG_JUE_DEF,
    ];
    v.extend(super::team_buffs::all_team_buff_defs());
    v
}

// ── 阵法触发型 buff（苍云阵自己开时按 §5.3 拟真挂载）──
// 数值来源：jx3dps-online-public 「苍云阵(阵眼)」+ 项目本地 buff.txt 8403/8404/8484
//
// 8403 锋凌：游戏字段 atPhysicsCriticalDamagePowerBaseKiloNumRate=20（KiloNumRate = /1000，即每层 +2% 会效）
// 8404 横绝：buff.txt "外功基础攻击力提高10%"
// 8484 锋绝：buff.txt "外功破防提高15%" → 154/1024

static EFFECTS_FENG_LING: &[EffectEntry] = &[
    // 每层 +2% 会效（5 层叠满 +10%）。游戏 1000 制下 20，项目 1024 制下 20.48，两者等价
    // 用 PhysicsCriticalDamagePowerPercent（直接比例加成），不是 PhysicsCriticalDamagePowerBase（等级加算）
    EffectEntry {
        field: AttribField::PhysicsCriticalDamagePowerPercent,
        value: 20.48,
    },
];
pub static BUFF_FENG_LING_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FENG_LING,
    name: "锋凌",
    description: "苍云阵·5重：绝刀触发，每层外功会心效果+2%，最多 5 层（持续 30 秒）",
    duration_frames: 480,
    tick_interval: 0,
    max_stacks: 5,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 60,
    short_name: None,
    effects: EFFECTS_FENG_LING,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6439.png",
};

static EFFECTS_FENG_JUE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsOvercomePercent,
        value: 154.0,
    }, // 154/1024 ≈ +15% 破防
];
pub static BUFF_FENG_JUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_FENG_JUE,
    name: "锋绝",
    description: "苍云阵·4重：招式会心后外功破防提高 15%，持续 5 秒",
    duration_frames: 80,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 61,
    short_name: None,
    effects: EFFECTS_FENG_JUE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6424.png",
};

static EFFECTS_HENG_JUE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 102.0,
    }, // 102/1024 ≈ +10% 攻击
];
pub static BUFF_HENG_JUE_DEF: BuffDef = BuffDef {
    buff_id: BUFF_HENG_JUE,
    name: "横绝",
    description: "苍云阵·6重：受到攻击后外功基础攻击力提高 10%，持续 5 秒",
    duration_frames: 80,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 62,
    short_name: None,
    effects: EFFECTS_HENG_JUE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6442.png",
};
