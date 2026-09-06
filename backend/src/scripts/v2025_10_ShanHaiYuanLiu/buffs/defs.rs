//! 山海源流（2025.10）- BuffDef 静态注册
//!
//! Types (BuffDef/AttribField/EffectEntry) 在顶层 `buffs.rs`；这里只放本版本的静态实例。

use crate::{
    AttribField, BuffDef, EffectEntry, BUFF_BU_CAN, BUFF_CHENG_WU, BUFF_DUN_DANG,
    BUFF_DUN_DANG_QIAN_SHAN, BUFF_DUN_FEI, BUFF_DUN_FEI_DELAY, BUFF_DUN_WEI, BUFF_FENG_MING,
    BUFF_HAN_JIA, BUFF_HAN_JIA_LARGE, BUFF_HAN_JIA_SMALL, BUFF_HAN_XIAO, BUFF_HUAN_SHEN,
    BUFF_JIAN_DING, BUFF_JIAN_TIE, BUFF_JIE_HUA, BUFF_JI_ANG, BUFF_JUAN_YUN, BUFF_KUANG_JUE,
    BUFF_LIAN_ZHAN_CD, BUFF_LIN_AN, BUFF_LIN_GUANG, BUFF_LIN_GUANG_COUNT, BUFF_LIU_XUE,
    BUFF_MIE_SHI, BUFF_NU_YAN, BUFF_SHI_XUE, BUFF_TIE_GU, BUFF_TIE_GU_SU_DI, BUFF_WU_JU,
    BUFF_XUE_NU, BUFF_XUE_NU_CD, BUFF_XUE_NU_JY, BUFF_XUE_SHI, BUFF_XUE_SHI_COUNT, BUFF_XU_RUO,
    BUFF_XU_RUO_DELAY, BUFF_YAN_ZHEN, BUFF_YUAN_GE_ID, BUFF_ZHAN_JUE, BUFF_ZHEN_FEN,
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
    icon: "https://icon.jx3box.com/icon/6344.png",
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
};

static EFFECTS_XUE_NU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 102.0,
    }, // 102/1024 ≈ 10%
];
pub static BUFF_XUE_NU_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6432.png",
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
    icon: "https://icon.jx3box.com/icon/6351.png",
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
};

pub static BUFF_JIE_HUA_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6346.png",
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
};

static EFFECTS_XU_RUO: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetPhysicsShieldPercent,
        value: -51.0,
    }, // -5%
];
pub static BUFF_XU_RUO_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6347.png",
    buff_id: BUFF_XU_RUO,
    name: "虚弱",
    description: "外功基础防御等级降低5%",
    duration_frames: 400,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 50,
    short_name: None,
    effects: EFFECTS_XU_RUO,
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_JIAN_DING_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6293.png",
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
};

/// 锋鸣·盾飞加成秘籍 ID
pub const RECIPE_FENG_MING_DUN_FEI: u32 = 99201;
static RECIPES_FENG_MING: &[u32] = &[RECIPE_FENG_MING_DUN_FEI];
static EFFECTS_FENG_MING: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsOvercomePercent,
        value: 154.0,
    }, // 154/1024 ≈ 15%
];
pub static BUFF_FENG_MING_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6326.png",
    buff_id: BUFF_FENG_MING,
    name: "锋鸣",
    description: "外功破防+15%，盾飞伤害+100%",
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
};

pub static BUFF_DUN_FEI_DELAY_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6344.png",
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
};

pub static BUFF_LIU_XUE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6323.png",
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
};

pub static BUFF_KUANG_JUE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6317.png",
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
};

pub static BUFF_XU_RUO_DELAY_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6347.png",
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
};

pub static BUFF_YUAN_GE: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/20064.png",
    buff_id: BUFF_YUAN_GE_ID,
    name: "援戈",
    description: "施展苍雪刀套路下招式将附带一次外功伤害并消耗一层",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 7,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 25,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_XUE_SHI_COUNT_DEF: BuffDef = BuffDef {
    icon: "",
    buff_id: BUFF_XUE_SHI_COUNT,
    name: "以血盟誓",
    description: "苍雪刀命中叠层，满2层后再次命中触发血誓",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 2,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_XUE_SHI_DEF: BuffDef = BuffDef {
    icon: "",
    buff_id: BUFF_XUE_SHI,
    name: "血誓",
    description: "被疗伤成效额外降低30%",
    duration_frames: 48,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_LIN_GUANG_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/19154.png",
    buff_id: BUFF_LIN_GUANG,
    name: "麟光玄甲",
    description: "施展苍雪刀套路附带麟光甲寒额外伤害和一次破招伤害",
    duration_frames: 224,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 60,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_LIN_GUANG_COUNT_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6315.png",
    buff_id: BUFF_LIN_GUANG_COUNT,
    name: "麟光计数",
    description: "累计触发麟光甲寒次数",
    duration_frames: 224,
    tick_interval: 0,
    max_stacks: 3,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_LIN_AN_DEF: BuffDef = BuffDef {
    icon: "",
    buff_id: BUFF_LIN_AN,
    name: "麟黯",
    description: "本次麟光玄甲内无法再次重置苍雪刀调息和获得额外怒气",
    duration_frames: 400,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 99,
    short_name: None,
    effects: &[],
    activate_recipes: &[],
    haste_scaled: false,
};

pub static BUFF_XUE_NU_CD_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6432.png",
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
};

pub static BUFF_CHENG_WU_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/23384.png",
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
};

pub static BUFF_SHI_XUE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6288.png",
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
};

pub static BUFF_JUAN_YUN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6345.png",
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
};

// 战绝：突破上限加速 +15%（154/1024）
static EFFECTS_ZHAN_JUE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::UnlimitedAdditionalHastePercent,
    value: 154.0,
}];
pub static BUFF_ZHAN_JUE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/102041.png",
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
};

pub static BUFF_BU_CAN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6284.png",
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
};

// 普通盾挡（8499）：1~10 级由 BuffInstance.level 区分；动态加 ParryValueBase（见 main.rs buff_dynamic_effects）
pub static BUFF_DUN_DANG_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6301.png",
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
};

// 千山盾挡（8448）：同上但有 千山奇穴 13421 时获得的强化版（系数约 1.25x）
pub static BUFF_DUN_DANG_QIAN_SHAN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6301.png",
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
};

pub static BUFF_HUAN_SHEN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6332.png",
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
};

// ── 寒啸千军（铁骨衣）──
static EFFECTS_HAN_XIAO: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::StrainPercent,
        value: 51.0,
    }, // 51/1024 ≈ +5% 无双率
];
pub static BUFF_HAN_XIAO_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/7514.png",
    buff_id: BUFF_HAN_XIAO,
    name: "寒啸千军",
    description: "无双率提高5%",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 25,
    short_name: Some("寒啸"),
    effects: EFFECTS_HAN_XIAO,
    activate_recipes: &[],
    haste_scaled: false,
};

// 振奋（奇穴 13422）：盾挡后 30s；每层 +101 StrainBase，max_stacks=100
// 实际层数 = floor(vitality/2820) ≤ 100，由 dun_dang.rs 施展时循环 add_buff 叠到目标层数
static EFFECTS_ZHEN_FEN: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 101.0,
}];
pub static BUFF_ZHEN_FEN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6436.png",
    buff_id: BUFF_ZHEN_FEN,
    name: "振奋",
    description: "每 2820 基础体质 +101 无双等级（每层 +101 StrainBase，上限 100 层），持续 30 秒",
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
};

// 蔑视（奇穴 39045）：伤害招式命中后获得，无视目标 50% 外功防御；10 秒
// 512/1024 = 50% AllShieldIgnorePercent（独立乘区，跟伤害链 Step 4 走）
static EFFECTS_MIE_SHI: &[EffectEntry] = &[EffectEntry {
    field: AttribField::AllShieldIgnorePercent,
    value: 512.0,
}];
pub static BUFF_MIE_SHI_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/7428.png",
    buff_id: BUFF_MIE_SHI,
    name: "蔑视",
    description: "无视目标 50% 外功防御等级",
    duration_frames: 160,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 28,
    short_name: Some("蔑视"),
    effects: EFFECTS_MIE_SHI,
    activate_recipes: &[],
    haste_scaled: false,
};

// ── 寒甲（铁骨衣 13134 奇穴）──
// 主 buff（时间轴可见），12s 持续，tick 每 3s 触发 on_tick 重算 8271/17772 层数
// 任意方式结束时通过 on_expire/on_remove 清除 8271/17772
pub static BUFF_HAN_JIA_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6309.png",
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
};

// 寒甲·小层：每层 +300 外功攻击（数值加算）
static EFFECTS_HAN_JIA_SMALL: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 300.0,
}];
pub static BUFF_HAN_JIA_SMALL_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6309.png",
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
};

// 寒甲·大层：每层 +30000 外功攻击
static EFFECTS_HAN_JIA_LARGE: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 30000.0,
}];
pub static BUFF_HAN_JIA_LARGE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6309.png",
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
};

// 坚铁 (8272)：每层 +6% 招架率（atParryBaseRate），最多 5 层
static EFFECTS_JIAN_TIE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::ParryValuePercent,
        value: 600.0,
    }, // 6% = 600/10000
];
pub static BUFF_JIAN_TIE_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6353.png",
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
};

// 恋战内置 CD (8321)：132 帧 = 8.25s；存在期间坚铁不叠层
pub static BUFF_LIAN_ZHAN_CD_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6303.png",
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
};

// 怒炎 (24755)：斩刀命中虚弱目标后 6 秒内下次绝刀返还怒气
pub static BUFF_NU_YAN_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6348.png",
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
};

// 无惧 (8247)：免疫控制 6 秒（PVE 无实际属性效果，仅状态显示）
pub static BUFF_WU_JU_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6299.png",
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
};

// 激昂 (8418)：盾猛后按体质叠层，每层 +66 招架 +166 拆招 +331 破招
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
    icon: "https://icon.jx3box.com/icon/2037.png",
    buff_id: BUFF_JI_ANG,
    name: "激昂",
    description: "每层提高招架等级、拆招值、破招值",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 32,
    short_name: Some("激昂"),
    effects: EFFECTS_JI_ANG,
    activate_recipes: &[],
    haste_scaled: false,
};

// 盾威 (8397)：目标 debuff，伤害输出-5%（纯防御，无 DPS 属性），15s
pub static BUFF_DUN_WEI_DEF: BuffDef = BuffDef {
    icon: "https://icon.jx3box.com/icon/6285.png",
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
};

// 严阵 (25356)：盾压叠层，每层 +50% 破招（512/1024），max 3，20s
static EFFECTS_YAN_ZHEN: &[EffectEntry] = &[EffectEntry {
    field: AttribField::SurplusPercent,
    value: 512.0,
}];
pub static BUFF_YAN_ZHEN_DEF: BuffDef = BuffDef {
    icon: "",
    buff_id: BUFF_YAN_ZHEN,
    name: "严阵",
    description: "每层提高50%破招",
    duration_frames: 320,
    tick_interval: 0,
    max_stacks: 3,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 33,
    short_name: Some("严阵"),
    effects: EFFECTS_YAN_ZHEN,
    activate_recipes: &[],
    haste_scaled: false,
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
    icon: "https://icon.jx3box.com/icon/6315.png",
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
    icon: "https://icon.jx3box.com/icon/6315.png",
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
};

// ─────────────────────────────────────────────────────────────────────────────
// 注册表
// ─────────────────────────────────────────────────────────────────────────────

/// 列出本版本的全部 Buff 定义，供 UI 元数据与属性来源分析使用。
pub fn all_buff_defs() -> Vec<&'static BuffDef> {
    vec![
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
        &BUFF_YUAN_GE,
        &BUFF_XUE_SHI_COUNT_DEF,
        &BUFF_XUE_SHI_DEF,
        &BUFF_LIN_GUANG_DEF,
        &BUFF_LIN_GUANG_COUNT_DEF,
        &BUFF_LIN_AN_DEF,
        &BUFF_XUE_NU_CD_DEF,
        &BUFF_CHENG_WU_DEF,
        &BUFF_SHI_XUE_DEF,
        &BUFF_JUAN_YUN_DEF,
        &BUFF_ZHAN_JUE_DEF,
        &BUFF_BU_CAN_DEF,
        &BUFF_DUN_DANG_DEF,
        &BUFF_DUN_DANG_QIAN_SHAN_DEF,
        &BUFF_HUAN_SHEN_DEF,
        &BUFF_HAN_XIAO_DEF,
        &BUFF_ZHEN_FEN_DEF,
        &BUFF_MIE_SHI_DEF,
        &BUFF_HAN_JIA_DEF,
        &BUFF_HAN_JIA_SMALL_DEF,
        &BUFF_HAN_JIA_LARGE_DEF,
        &BUFF_JIAN_TIE_DEF,
        &BUFF_LIAN_ZHAN_CD_DEF,
        &BUFF_NU_YAN_DEF,
        &BUFF_WU_JU_DEF,
        &BUFF_JI_ANG_DEF,
        &BUFF_DUN_WEI_DEF,
        &BUFF_YAN_ZHEN_DEF,
        &BUFF_TIE_GU_DEF,
        &BUFF_TIE_GU_SU_DI_DEF,
    ]
}

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
        BUFF_XUE_SHI_COUNT => Some(&BUFF_XUE_SHI_COUNT_DEF),
        BUFF_XUE_SHI => Some(&BUFF_XUE_SHI_DEF),
        BUFF_LIN_GUANG => Some(&BUFF_LIN_GUANG_DEF),
        BUFF_LIN_GUANG_COUNT => Some(&BUFF_LIN_GUANG_COUNT_DEF),
        BUFF_LIN_AN => Some(&BUFF_LIN_AN_DEF),
        BUFF_CHENG_WU => Some(&BUFF_CHENG_WU_DEF),
        BUFF_SHI_XUE => Some(&BUFF_SHI_XUE_DEF),
        BUFF_JUAN_YUN => Some(&BUFF_JUAN_YUN_DEF),
        BUFF_ZHAN_JUE => Some(&BUFF_ZHAN_JUE_DEF),
        BUFF_DUN_DANG => Some(&BUFF_DUN_DANG_DEF),
        BUFF_DUN_DANG_QIAN_SHAN => Some(&BUFF_DUN_DANG_QIAN_SHAN_DEF),
        BUFF_ZHEN_FEN => Some(&BUFF_ZHEN_FEN_DEF),
        BUFF_MIE_SHI => Some(&BUFF_MIE_SHI_DEF),
        BUFF_HAN_XIAO => Some(&BUFF_HAN_XIAO_DEF),
        BUFF_HAN_JIA => Some(&BUFF_HAN_JIA_DEF),
        BUFF_HAN_JIA_SMALL => Some(&BUFF_HAN_JIA_SMALL_DEF),
        BUFF_HAN_JIA_LARGE => Some(&BUFF_HAN_JIA_LARGE_DEF),
        BUFF_JIAN_TIE => Some(&BUFF_JIAN_TIE_DEF),
        BUFF_LIAN_ZHAN_CD => Some(&BUFF_LIAN_ZHAN_CD_DEF),
        BUFF_NU_YAN => Some(&BUFF_NU_YAN_DEF),
        BUFF_WU_JU => Some(&BUFF_WU_JU_DEF),
        BUFF_JI_ANG => Some(&BUFF_JI_ANG_DEF),
        BUFF_YAN_ZHEN => Some(&BUFF_YAN_ZHEN_DEF),
        BUFF_DUN_WEI => Some(&BUFF_DUN_WEI_DEF),
        BUFF_TIE_GU => Some(&BUFF_TIE_GU_DEF),
        BUFF_TIE_GU_SU_DI => Some(&BUFF_TIE_GU_SU_DI_DEF),
        _ => None,
    }
}
