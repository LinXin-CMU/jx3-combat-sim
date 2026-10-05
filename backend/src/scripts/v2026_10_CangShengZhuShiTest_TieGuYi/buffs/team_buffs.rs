//! 团队增益 BuffDef（暗影千机 2026.04）
//!
//! 50 条团辅条目，参考 jx3dps-online + 游戏 buff.txt 核对：
//! - 旗舰版（35 条）：buff.txt Case
//! - 无界版（15 条）：buff.txt Case_mobile
//!
//! 设计：
//! - 大多数团辅 buff_id 走 `0xD0_T0_00_xx` 段项目自定义占位
//! - 新寒啸无双（33210 = BUFF_HAN_XIAO）和振奋（8504 = BUFF_ZHEN_FEN）直接复用项目
//!   已有的 BuffDef（铁骨衣自身释放的就是这两个，团队成员从 UI 勾的也是同一个；
//!   重复挂会按 add_buff_with_stacks 直接覆盖 stacks，符合实际游戏逻辑）
//! - duration_frames 写默认值（按 jx3dps-online 覆盖率反推），运行时由
//!   `add_buff_with_stacks` / `add_target_buff_with_stacks` 用 selection 的
//!   `duration` 字段覆盖
//! - max_stacks 严格按数据；effects 单层值
//! - show_on_timeline=true，timeline_order=200+ 放在底部
//! - filter 不做（用户决定）

use crate::{AttribField, BuffDef, EffectEntry};

// ─────────────────────────────────────────────────────────────────────────────
// 团队增益 buff_id 常量（项目自定义占位段）
// ─────────────────────────────────────────────────────────────────────────────

// 旗舰版（35 条）
// ⚠️ 0xD0_00_00_01~0A 段被项目自身 BUFF_*（盾飞/血怒/狂绝/血怒叠层/虚弱延迟/陷阵CD 等）占用，
//    团辅 ID 必须挪到 0xD0_00_00_40+ 段，避免 get_buff_def 错误命中自身 BUFF 的 def
pub const TB_XIU_QI: u32 = 0xD0_00_00_41; // 袖气
pub const TB_GONG_ZHAN: u32 = 0xD0_00_00_42; // 共战江湖
pub const TB_HAN_RU_LEI: u32 = 0xD0_00_00_43; // 撼如雷
pub const TB_ZHENG_YU: u32 = 0xD0_00_00_44; // 蒸鱼菜盘
pub const TB_TONG_ZE: u32 = 0xD0_00_00_45; // 同泽宴
                                           // 虚弱复用 BUFF_XU_RUO=8248（铁骨衣自身和团队成员共享同一 debuff）
pub const TB_PO_FENG: u32 = 0xD0_00_00_47; // 破风
pub const TB_JIN_FENG: u32 = 0xD0_00_00_48; // 劲风（破风 +33%）
pub const TB_PO_JIA: u32 = 0xD0_00_00_49; // 破甲
pub const TB_JIE_HUO: u32 = 0xD0_00_00_4A; // 戒火
                                           // 0xD0_00_00_0B / 0xD0_00_00_0C 留空：寒啸千军 / 振奋 复用 BUFF_HAN_XIAO=33210 / BUFF_ZHEN_FEN=8504
pub const TB_HAO_LING_3J: u32 = 0xD0_00_00_0D; // 号令三军
pub const TB_CHAN_YU: u32 = 0xD0_00_00_0E; // 禅语
pub const TB_CHAO_SHENG: u32 = 0xD0_00_00_0F; // 朝圣
pub const TB_SHENG_YU_MX: u32 = 0xD0_00_00_10; // 圣浴明心
pub const TB_PIAO_HUANG: u32 = 0xD0_00_00_11; // 飘黄
pub const TB_XIAN_WANG: u32 = 0xD0_00_00_12; // 仙王蛊鼎
pub const TB_ZUO_XUAN: u32 = 0xD0_00_00_13; // 左旋右转
pub const TB_QIU_SU: u32 = 0xD0_00_00_14; // 秋肃
pub const TB_ZHUANG_ZHOU: u32 = 0xD0_00_00_15; // 庄周梦
pub const TB_JIAO_SU: u32 = 0xD0_00_00_16; // 皎素
pub const TB_LIAN_YU: u32 = 0xD0_00_00_17; // 炼狱水煮鱼
pub const TB_BAI_LIAN: u32 = 0xD0_00_00_18; // 百炼水煮鱼
pub const TB_YIN_DONG: u32 = 0xD0_00_00_19; // 吟冬卧雪
pub const TB_SHU_KUANG: u32 = 0xD0_00_00_1A; // 疏狂
pub const TB_LIE_LEI: u32 = 0xD0_00_00_1B; // 列雷（烈雷）
pub const TB_NONG_MEI: u32 = 0xD0_00_00_1C; // 弄梅
pub const TB_SUI_XING: u32 = 0xD0_00_00_1E; // 碎星辰
pub const TB_GUI_LI: u32 = 0xD0_00_00_1F; // 瑰栗粽
pub const TB_LU_DOU: u32 = 0xD0_00_00_20; // 芦兜粽
pub const TB_YUN_PIAN: u32 = 0xD0_00_00_21; // 云片糕
pub const TB_MEI_HUA_GAO: u32 = 0xD0_00_00_22; // 梅花糕
pub const TB_SHENG_JING: u32 = 0xD0_00_00_23; // 春节·升景

// 无界版（15 条）
pub const TB_JIE_HUO_WU: u32 = 0xD0_00_00_30; // 戒火·悟
pub const TB_LONG_YIN_WU: u32 = 0xD0_00_00_31; // 龙吟·悟（+3% 易伤）
pub const TB_ZHAN_FENG_WU: u32 = 0xD0_00_00_32; // 战锋·悟（+4% 易伤）
pub const TB_HAN_XIAO_WU: u32 = 0xD0_00_00_33; // 寒啸千军·悟（+5% 无双率）
pub const TB_HAO_LING_WU: u32 = 0xD0_00_00_34; // 号令三军·悟
pub const TB_HONG_FA_WU: u32 = 0xD0_00_00_35; // 弘法·悟
pub const TB_DUN_DANG_WU: u32 = 0xD0_00_00_36; // 盾挡·悟
pub const TB_LUO_SHANG_WU: u32 = 0xD0_00_00_37; // 左旋右转·悟（罗裳·悟）
pub const TB_XIAN_DING_WU: u32 = 0xD0_00_00_38; // 仙王蛊鼎·悟（仙鼎·悟）
pub const TB_JIN_LU_WU: u32 = 0xD0_00_00_39; // 梅花三弄·悟（金缕·悟）
pub const TB_QIU_SU_WU: u32 = 0xD0_00_00_3A; // 秋肃·悟
pub const TB_ZHONG_HE_WU: u32 = 0xD0_00_00_3B; // 中和·悟（灵素中和·悟）
pub const TB_ZHUO_LIAN_WU: u32 = 0xD0_00_00_3C; // 逐云寒蕊·悟（濯莲·悟）
pub const TB_CHAO_SHENG_WU: u32 = 0xD0_00_00_3D; // 朝圣·悟
pub const TB_SHI_HOU_WU: u32 = 0xD0_00_00_3E; // 狮吼·悟（禅语·悟）

// ─────────────────────────────────────────────────────────────────────────────
// BuffDef 静态实例
// ─────────────────────────────────────────────────────────────────────────────

// ── 常用增益 ──

static EFF_XIU_QI: &[EffectEntry] = &[EffectEntry {
    field: AttribField::BasePotentialAdd,
    value: 317.0,
}];
pub static BUFF_TB_XIU_QI_DEF: BuffDef = BuffDef {
    buff_id: TB_XIU_QI,
    name: "袖气",
    description: "全属性提升 317 点",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 200,
    short_name: None,
    effects: EFF_XIU_QI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/907.png",
};

static EFF_GONG_ZHAN: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::AllDamageAddPercent,
        value: 51.0,
    }, // 5%/层
];
pub static BUFF_TB_GONG_ZHAN_DEF: BuffDef = BuffDef {
    buff_id: TB_GONG_ZHAN,
    name: "共战江湖",
    description: "每层提升 5% 最终伤害和治疗",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 6,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 200,
    short_name: None,
    effects: EFF_GONG_ZHAN,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/2148.png",
};

static EFF_HAN_RU_LEI: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 51.0,
    }, // +5%
];
pub static BUFF_TB_HAN_RU_LEI_DEF: BuffDef = BuffDef {
    buff_id: TB_HAN_RU_LEI,
    name: "撼如雷",
    description: "外功基础攻击 +5%",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 200,
    short_name: None,
    effects: EFF_HAN_RU_LEI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/635.png",
};

static EFF_ZHENG_YU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 1334.0,
}];
pub static BUFF_TB_ZHENG_YU_DEF: BuffDef = BuffDef {
    buff_id: TB_ZHENG_YU,
    name: "蒸鱼菜盘",
    description: "无双 +1334",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 200,
    short_name: None,
    effects: EFF_ZHENG_YU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/552.png",
};

static EFF_TONG_ZE: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::StrainBase,
        value: 868.0,
    },
    EffectEntry {
        field: AttribField::SurplusValueBase,
        value: 868.0,
    },
];
pub static BUFF_TB_TONG_ZE_DEF: BuffDef = BuffDef {
    buff_id: TB_TONG_ZE,
    name: "同泽宴",
    description: "无双 +868、破招 +868",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 200,
    short_name: None,
    effects: EFF_TONG_ZE,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/9025.png",
};

// ── 目标减益 ──
// 虚弱复用 BUFF_XU_RUO_DEF（buff_id=8248，已在 defs.rs 定义）

static EFF_TB_PO_FENG: &[EffectEntry] = &[EffectEntry {
    field: AttribField::TargetPhysicsShieldBase,
    value: -3500.0,
}];
pub static BUFF_TB_PO_FENG_DEF: BuffDef = BuffDef {
    buff_id: TB_PO_FENG,
    name: "破风",
    description: "目标外功防御 -3500",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_PO_FENG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/647.png",
};

static EFF_TB_JIN_FENG: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetPhysicsShieldBase,
        value: -4655.0,
    }, // 破风 +33%
];
pub static BUFF_TB_JIN_FENG_DEF: BuffDef = BuffDef {
    buff_id: TB_JIN_FENG,
    name: "劲风",
    description: "目标外功防御 -4655（破风 +33%）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_JIN_FENG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/650.png",
};

static EFF_TB_PO_JIA: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetPhysicsShieldPercent,
        value: -102.0,
    }, // -10%
];
pub static BUFF_TB_PO_JIA_DEF: BuffDef = BuffDef {
    buff_id: TB_PO_JIA,
    name: "破甲",
    description: "目标外功防御 -10%",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: true,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_PO_JIA,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/16.png",
};

static EFF_TB_JIE_HUO: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetDamageBonusPercent,
        value: 21.0,
    }, // +2%
];
pub static BUFF_TB_JIE_HUO_DEF: BuffDef = BuffDef {
    buff_id: TB_JIE_HUO,
    name: "戒火",
    description: "目标受到伤害 +2%",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_JIE_HUO,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3798.png",
};

// ── 坦克 Buff ──
// 寒啸千军 / 振奋 直接复用项目已有 BUFF_HAN_XIAO_DEF / BUFF_ZHEN_FEN_DEF（数值/max 已是新版）

static EFF_TB_HAO_LING: &[EffectEntry] = &[
    // 一鼓二鼓平均 (826+413)/2 = 619.5
    EffectEntry {
        field: AttribField::StrainBase,
        value: 619.5,
    },
];
pub static BUFF_TB_HAO_LING_DEF: BuffDef = BuffDef {
    buff_id: TB_HAO_LING_3J,
    name: "号令三军",
    description: "每层无双 +619.5（一鼓二鼓平均）",
    duration_frames: 384,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("号令"),
    effects: EFF_TB_HAO_LING,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/4514.png",
};

static EFF_TB_CHAN_YU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_CHAN_YU_DEF: BuffDef = BuffDef {
    buff_id: TB_CHAN_YU,
    name: "禅语",
    description: "每层无双 +139",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 220,
    short_name: None,
    effects: EFF_TB_CHAN_YU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/4526.png",
};

static EFF_TB_CHAO_SHENG: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 612.0,
}];
pub static BUFF_TB_CHAO_SHENG_DEF: BuffDef = BuffDef {
    buff_id: TB_CHAO_SHENG,
    name: "朝圣",
    description: "每层无双 +612（明尊朝圣言）",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("朝圣"),
    effects: EFF_TB_CHAO_SHENG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3791.png",
};

static EFF_TB_SHENG_YU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 919.0,
}];
pub static BUFF_TB_SHENG_YU_DEF: BuffDef = BuffDef {
    buff_id: TB_SHENG_YU_MX,
    name: "圣浴明心",
    description: "每层无双 +919（明尊增伤朝圣言）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 10,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 220,
    short_name: None,
    effects: EFF_TB_SHENG_YU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7483.png",
};

// ── 治疗 Buff ──

static EFF_TB_PIAO_HUANG: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 163.0,
}];
pub static BUFF_TB_PIAO_HUANG_DEF: BuffDef = BuffDef {
    buff_id: TB_PIAO_HUANG,
    name: "飘黄",
    description: "每层无双 +163",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("飘黄"),
    effects: EFF_TB_PIAO_HUANG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/15692.png",
};

static EFF_TB_XIAN_WANG: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 500.0,
}];
pub static BUFF_TB_XIAN_WANG_DEF: BuffDef = BuffDef {
    buff_id: TB_XIAN_WANG,
    name: "仙王蛊鼎",
    description: "每层无双 +500",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("仙王"),
    effects: EFF_TB_XIAN_WANG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/2747.png",
};

static EFF_TB_ZUO_XUAN: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_ZUO_XUAN_DEF: BuffDef = BuffDef {
    buff_id: TB_ZUO_XUAN,
    name: "左旋右转",
    description: "每层无双 +139",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_ZUO_XUAN,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/904.png",
};

static EFF_TB_QIU_SU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_QIU_SU_DEF: BuffDef = BuffDef {
    buff_id: TB_QIU_SU,
    name: "秋肃",
    description: "每层无双 +139",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_QIU_SU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/403.png",
};

static EFF_TB_ZHUANG_ZHOU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_ZHUANG_ZHOU_DEF: BuffDef = BuffDef {
    buff_id: TB_ZHUANG_ZHOU,
    name: "庄周梦",
    description: "每层无双 +139",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_ZHUANG_ZHOU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/9555.png",
};

static EFF_TB_JIAO_SU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsCriticalDamagePowerBase,
        value: 51.0,
    }, // +5% 会效
];
pub static BUFF_TB_JIAO_SU_DEF: BuffDef = BuffDef {
    buff_id: TB_JIAO_SU,
    name: "皎素",
    description: "外功会心效果等级 +51（≈5%）",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("皎素"),
    effects: EFF_TB_JIAO_SU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/17706.png",
};

// ── 食物增益 ──

static EFF_TB_LIAN_YU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::StrainBase,
        value: 100.0,
    },
    EffectEntry {
        field: AttribField::SurplusValueBase,
        value: 100.0,
    },
];
pub static BUFF_TB_LIAN_YU_DEF: BuffDef = BuffDef {
    buff_id: TB_LIAN_YU,
    name: "炼狱水煮鱼",
    description: "无双 +100、破招 +100",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 240,
    short_name: None,
    effects: EFF_TB_LIAN_YU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7667.png",
};

static EFF_TB_BAI_LIAN: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::StrainBase,
        value: 600.0,
    },
    EffectEntry {
        field: AttribField::SurplusValueBase,
        value: 600.0,
    },
];
pub static BUFF_TB_BAI_LIAN_DEF: BuffDef = BuffDef {
    buff_id: TB_BAI_LIAN,
    name: "百炼水煮鱼",
    description: "无双 +600、破招 +600",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 240,
    short_name: None,
    effects: EFF_TB_BAI_LIAN,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7667.png",
};

static EFF_TB_YIN_DONG: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsOvercomeBase,
        value: 380.0,
    },
    EffectEntry {
        field: AttribField::PhysicsCriticalStrike,
        value: 380.0,
    },
];
pub static BUFF_TB_YIN_DONG_DEF: BuffDef = BuffDef {
    buff_id: TB_YIN_DONG,
    name: "吟冬卧雪",
    description: "每层全破防 +380、全会心 +380",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 8,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 240,
    short_name: None,
    effects: EFF_TB_YIN_DONG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/138.png",
};

// ── 稀缺增益 ──

static EFF_TB_SHU_KUANG: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerPercent,
        value: 307.0,
    }, // +30%
];
pub static BUFF_TB_SHU_KUANG_DEF: BuffDef = BuffDef {
    buff_id: TB_SHU_KUANG,
    name: "疏狂",
    description: "外功基础攻击 +30%",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 250,
    short_name: Some("疏狂"),
    effects: EFF_TB_SHU_KUANG,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/8638.png",
};

static EFF_TB_LIE_LEI: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 700.0,
}];
pub static BUFF_TB_LIE_LEI_DEF: BuffDef = BuffDef {
    buff_id: TB_LIE_LEI,
    name: "列雷",
    description: "外功基础攻击 +700（莫问武器）",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 250,
    short_name: Some("列雷"),
    effects: EFF_TB_LIE_LEI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/7113.png",
};

static EFF_TB_NONG_MEI: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsOvercomeBase,
        value: 700.0,
    },
    EffectEntry {
        field: AttribField::AllShieldIgnorePercent,
        value: 205.0,
    }, // +20%
];
pub static BUFF_TB_NONG_MEI_DEF: BuffDef = BuffDef {
    buff_id: TB_NONG_MEI,
    name: "弄梅",
    description: "破防 +700、无视防御 +20%（相知武器）",
    duration_frames: 288,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 250,
    short_name: Some("弄梅"),
    effects: EFF_TB_NONG_MEI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/11417.png",
};

static EFF_TB_SUI_XING: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsCriticalDamagePowerPercent,
        value: 102.4,
    }, // 102.4/1024 = +10% 最终会效
];
pub static BUFF_TB_SUI_XING_DEF: BuffDef = BuffDef {
    buff_id: TB_SUI_XING,
    name: "碎星辰",
    description: "外功会心效果 +10%",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 250,
    short_name: Some("碎星辰"),
    effects: EFF_TB_SUI_XING,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/1450.png",
};

// ── 节日增益 ──

static EFF_TB_GUI_LI: &[EffectEntry] = &[EffectEntry {
    field: AttribField::WeaponDamageBase,
    value: 2477.0,
}];
pub static BUFF_TB_GUI_LI_DEF: BuffDef = BuffDef {
    buff_id: TB_GUI_LI,
    name: "瑰栗粽",
    description: "武器伤害 +2477",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 260,
    short_name: None,
    effects: EFF_TB_GUI_LI,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/2583.png",
};

static EFF_TB_LU_DOU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 1641.0,
}];
pub static BUFF_TB_LU_DOU_DEF: BuffDef = BuffDef {
    buff_id: TB_LU_DOU,
    name: "芦兜粽",
    description: "外功攻击 +1641",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 260,
    short_name: None,
    effects: EFF_TB_LU_DOU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/2584.png",
};

static EFF_TB_YUN_PIAN: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsAttackPowerBase,
    value: 1762.0,
}];
pub static BUFF_TB_YUN_PIAN_DEF: BuffDef = BuffDef {
    buff_id: TB_YUN_PIAN,
    name: "云片糕",
    description: "外功攻击 +1762",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 260,
    short_name: None,
    effects: EFF_TB_YUN_PIAN,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6892.png",
};

static EFF_TB_MEI_HUA_GAO: &[EffectEntry] = &[EffectEntry {
    field: AttribField::WeaponDamageBase,
    value: 2657.0,
}];
pub static BUFF_TB_MEI_HUA_GAO_DEF: BuffDef = BuffDef {
    buff_id: TB_MEI_HUA_GAO,
    name: "梅花糕",
    description: "武器伤害 +2657",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 260,
    short_name: None,
    effects: EFF_TB_MEI_HUA_GAO,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/6782.png",
};

static EFF_TB_SHENG_JING: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::PhysicsAttackPowerBase,
        value: 6849.0,
    },
    EffectEntry {
        field: AttribField::WeaponDamageBase,
        value: 2296.0,
    },
];
pub static BUFF_TB_SHENG_JING_DEF: BuffDef = BuffDef {
    buff_id: TB_SHENG_JING,
    name: "春节·升景",
    description: "外功攻击 +6849、武器伤害 +2296",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 260,
    short_name: None,
    effects: EFF_TB_SHENG_JING,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/341.png",
};

// ─────────────────────────────────────────────────────────────────────────────
// 无界版（15 条）
// ─────────────────────────────────────────────────────────────────────────────

// 戒火·悟（与戒火数值相同 +5%/2%，但 jx3dps 复用 21）
static EFF_TB_JIE_HUO_WU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetDamageBonusPercent,
        value: 51.0,
    }, // +5%（无界游戏内 70188 描述）
];
pub static BUFF_TB_JIE_HUO_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_JIE_HUO_WU,
    name: "戒火·悟",
    description: "目标受到伤害 +5%",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_JIE_HUO_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/3798.png",
};

static EFF_TB_LONG_YIN_WU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetDamageBonusPercent,
        value: 31.0,
    }, // +3%
];
pub static BUFF_TB_LONG_YIN_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_LONG_YIN_WU,
    name: "龙吟·悟",
    description: "目标受到外功伤害 +3%",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_LONG_YIN_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/650.png",
};

static EFF_TB_ZHAN_FENG_WU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::TargetDamageBonusPercent,
        value: 41.0,
    }, // +4%
];
pub static BUFF_TB_ZHAN_FENG_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_ZHAN_FENG_WU,
    name: "战锋·悟",
    description: "目标受到外功伤害 +4%",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: true,
    show_on_timeline: false,
    timeline_order: 210,
    short_name: None,
    effects: EFF_TB_ZHAN_FENG_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/14126.png",
};

static EFF_TB_HAN_XIAO_WU: &[EffectEntry] = &[
    EffectEntry {
        field: AttribField::StrainPercent,
        value: 51.0,
    }, // +5% 无双率
];
pub static BUFF_TB_HAN_XIAO_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_HAN_XIAO_WU,
    name: "寒啸千军·悟",
    description: "无双率 +5%",
    duration_frames: 144,
    tick_interval: 0,
    max_stacks: 1,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("寒啸·悟"),
    effects: EFF_TB_HAN_XIAO_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_HAO_LING_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 715.0,
}];
pub static BUFF_TB_HAO_LING_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_HAO_LING_WU,
    name: "号令三军·悟",
    description: "每层无双 +715",
    duration_frames: 384,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("号令·悟"),
    effects: EFF_TB_HAO_LING_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/4514.png",
};

static EFF_TB_HONG_FA_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 759.0,
}];
pub static BUFF_TB_HONG_FA_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_HONG_FA_WU,
    name: "弘法·悟",
    description: "每层无双 +759",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("弘法·悟"),
    effects: EFF_TB_HONG_FA_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_DUN_DANG_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::PhysicsOvercomeBase,
    value: 80.0,
}];
pub static BUFF_TB_DUN_DANG_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_DUN_DANG_WU,
    name: "盾挡·悟",
    description: "每层全破防 +80",
    duration_frames: 106,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("盾挡·悟"),
    effects: EFF_TB_DUN_DANG_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70169.png",
};

static EFF_TB_LUO_SHANG_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 126.0,
}];
pub static BUFF_TB_LUO_SHANG_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_LUO_SHANG_WU,
    name: "罗裳·悟",
    description: "每层无双 +126（左旋右转·悟）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_LUO_SHANG_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_XIAN_DING_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 306.0,
}];
pub static BUFF_TB_XIAN_DING_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_XIAN_DING_WU,
    name: "仙鼎·悟",
    description: "每层无双 +306（仙王蛊鼎·悟）",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("仙鼎·悟"),
    effects: EFF_TB_XIAN_DING_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_JIN_LU_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 126.0,
}];
pub static BUFF_TB_JIN_LU_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_JIN_LU_WU,
    name: "金缕·悟",
    description: "每层无双 +126（梅花三弄·悟）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_JIN_LU_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_QIU_SU_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_QIU_SU_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_QIU_SU_WU,
    name: "秋肃·悟",
    description: "每层无双 +139",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 230,
    short_name: None,
    effects: EFF_TB_QIU_SU_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_ZHONG_HE_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 100.0,
}];
pub static BUFF_TB_ZHONG_HE_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_ZHONG_HE_WU,
    name: "中和·悟",
    description: "每层无双 +100（灵素中和·悟）",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("中和·悟"),
    effects: EFF_TB_ZHONG_HE_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/16025.png",
};

static EFF_TB_ZHUO_LIAN_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 304.0,
}];
pub static BUFF_TB_ZHUO_LIAN_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_ZHUO_LIAN_WU,
    name: "濯莲·悟",
    description: "每层无双 +304（逐云寒蕊·悟）",
    duration_frames: 240,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 230,
    short_name: Some("濯莲·悟"),
    effects: EFF_TB_ZHUO_LIAN_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_CHAO_SHENG_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 244.0,
}];
pub static BUFF_TB_CHAO_SHENG_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_CHAO_SHENG_WU,
    name: "朝圣·悟",
    description: "每层无双 +244",
    duration_frames: 96,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: true,
    timeline_order: 220,
    short_name: Some("朝圣·悟"),
    effects: EFF_TB_CHAO_SHENG_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

static EFF_TB_SHI_HOU_WU: &[EffectEntry] = &[EffectEntry {
    field: AttribField::StrainBase,
    value: 139.0,
}];
pub static BUFF_TB_SHI_HOU_WU_DEF: BuffDef = BuffDef {
    buff_id: TB_SHI_HOU_WU,
    name: "狮吼·悟",
    description: "每层无双 +139（禅语·悟）",
    duration_frames: 0,
    tick_interval: 0,
    max_stacks: 100,
    is_debuff: false,
    show_on_timeline: false,
    timeline_order: 220,
    short_name: None,
    effects: EFF_TB_SHI_HOU_WU,
    activate_recipes: &[],
    haste_scaled: false,
    icon: "https://icon.jx3box.com/icon/70161.png",
};

// ─────────────────────────────────────────────────────────────────────────────
// 注册表
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_team_buff_def(buff_id: u32) -> Option<&'static BuffDef> {
    match buff_id {
        TB_XIU_QI => Some(&BUFF_TB_XIU_QI_DEF),
        TB_GONG_ZHAN => Some(&BUFF_TB_GONG_ZHAN_DEF),
        TB_HAN_RU_LEI => Some(&BUFF_TB_HAN_RU_LEI_DEF),
        TB_ZHENG_YU => Some(&BUFF_TB_ZHENG_YU_DEF),
        TB_TONG_ZE => Some(&BUFF_TB_TONG_ZE_DEF),
        // 虚弱 (8248) 复用 defs.rs 的 BUFF_XU_RUO_DEF
        TB_PO_FENG => Some(&BUFF_TB_PO_FENG_DEF),
        TB_JIN_FENG => Some(&BUFF_TB_JIN_FENG_DEF),
        TB_PO_JIA => Some(&BUFF_TB_PO_JIA_DEF),
        TB_JIE_HUO => Some(&BUFF_TB_JIE_HUO_DEF),
        // 新寒啸无双 (33210) / 振奋 (8504) 复用 defs.rs 的 BUFF_HAN_XIAO_DEF / BUFF_ZHEN_FEN_DEF
        TB_HAO_LING_3J => Some(&BUFF_TB_HAO_LING_DEF),
        TB_CHAN_YU => Some(&BUFF_TB_CHAN_YU_DEF),
        TB_CHAO_SHENG => Some(&BUFF_TB_CHAO_SHENG_DEF),
        TB_SHENG_YU_MX => Some(&BUFF_TB_SHENG_YU_DEF),
        TB_PIAO_HUANG => Some(&BUFF_TB_PIAO_HUANG_DEF),
        TB_XIAN_WANG => Some(&BUFF_TB_XIAN_WANG_DEF),
        TB_ZUO_XUAN => Some(&BUFF_TB_ZUO_XUAN_DEF),
        TB_QIU_SU => Some(&BUFF_TB_QIU_SU_DEF),
        TB_ZHUANG_ZHOU => Some(&BUFF_TB_ZHUANG_ZHOU_DEF),
        TB_JIAO_SU => Some(&BUFF_TB_JIAO_SU_DEF),
        TB_LIAN_YU => Some(&BUFF_TB_LIAN_YU_DEF),
        TB_BAI_LIAN => Some(&BUFF_TB_BAI_LIAN_DEF),
        TB_YIN_DONG => Some(&BUFF_TB_YIN_DONG_DEF),
        TB_SHU_KUANG => Some(&BUFF_TB_SHU_KUANG_DEF),
        TB_LIE_LEI => Some(&BUFF_TB_LIE_LEI_DEF),
        TB_NONG_MEI => Some(&BUFF_TB_NONG_MEI_DEF),
        TB_SUI_XING => Some(&BUFF_TB_SUI_XING_DEF),
        TB_GUI_LI => Some(&BUFF_TB_GUI_LI_DEF),
        TB_LU_DOU => Some(&BUFF_TB_LU_DOU_DEF),
        TB_YUN_PIAN => Some(&BUFF_TB_YUN_PIAN_DEF),
        TB_MEI_HUA_GAO => Some(&BUFF_TB_MEI_HUA_GAO_DEF),
        TB_SHENG_JING => Some(&BUFF_TB_SHENG_JING_DEF),
        TB_JIE_HUO_WU => Some(&BUFF_TB_JIE_HUO_WU_DEF),
        TB_LONG_YIN_WU => Some(&BUFF_TB_LONG_YIN_WU_DEF),
        TB_ZHAN_FENG_WU => Some(&BUFF_TB_ZHAN_FENG_WU_DEF),
        TB_HAN_XIAO_WU => Some(&BUFF_TB_HAN_XIAO_WU_DEF),
        TB_HAO_LING_WU => Some(&BUFF_TB_HAO_LING_WU_DEF),
        TB_HONG_FA_WU => Some(&BUFF_TB_HONG_FA_WU_DEF),
        TB_DUN_DANG_WU => Some(&BUFF_TB_DUN_DANG_WU_DEF),
        TB_LUO_SHANG_WU => Some(&BUFF_TB_LUO_SHANG_WU_DEF),
        TB_XIAN_DING_WU => Some(&BUFF_TB_XIAN_DING_WU_DEF),
        TB_JIN_LU_WU => Some(&BUFF_TB_JIN_LU_WU_DEF),
        TB_QIU_SU_WU => Some(&BUFF_TB_QIU_SU_WU_DEF),
        TB_ZHONG_HE_WU => Some(&BUFF_TB_ZHONG_HE_WU_DEF),
        TB_ZHUO_LIAN_WU => Some(&BUFF_TB_ZHUO_LIAN_WU_DEF),
        TB_CHAO_SHENG_WU => Some(&BUFF_TB_CHAO_SHENG_WU_DEF),
        TB_SHI_HOU_WU => Some(&BUFF_TB_SHI_HOU_WU_DEF),
        _ => None,
    }
}

/// 所有团辅 BuffDef 列表（与 get_team_buff_def 的 match arms 配套维护）
/// 用于启动时扫描 effects 含特定字段的 buff（如 AllDamageAddPercent 增伤型）
pub fn all_team_buff_defs() -> Vec<&'static BuffDef> {
    vec![
        &BUFF_TB_XIU_QI_DEF,
        &BUFF_TB_GONG_ZHAN_DEF,
        &BUFF_TB_HAN_RU_LEI_DEF,
        &BUFF_TB_ZHENG_YU_DEF,
        &BUFF_TB_TONG_ZE_DEF,
        &BUFF_TB_PO_FENG_DEF,
        &BUFF_TB_JIN_FENG_DEF,
        &BUFF_TB_PO_JIA_DEF,
        &BUFF_TB_JIE_HUO_DEF,
        &BUFF_TB_HAO_LING_DEF,
        &BUFF_TB_CHAN_YU_DEF,
        &BUFF_TB_CHAO_SHENG_DEF,
        &BUFF_TB_SHENG_YU_DEF,
        &BUFF_TB_PIAO_HUANG_DEF,
        &BUFF_TB_XIAN_WANG_DEF,
        &BUFF_TB_ZUO_XUAN_DEF,
        &BUFF_TB_QIU_SU_DEF,
        &BUFF_TB_ZHUANG_ZHOU_DEF,
        &BUFF_TB_JIAO_SU_DEF,
        &BUFF_TB_LIAN_YU_DEF,
        &BUFF_TB_BAI_LIAN_DEF,
        &BUFF_TB_YIN_DONG_DEF,
        &BUFF_TB_SHU_KUANG_DEF,
        &BUFF_TB_LIE_LEI_DEF,
        &BUFF_TB_NONG_MEI_DEF,
        &BUFF_TB_SUI_XING_DEF,
        &BUFF_TB_GUI_LI_DEF,
        &BUFF_TB_LU_DOU_DEF,
        &BUFF_TB_YUN_PIAN_DEF,
        &BUFF_TB_MEI_HUA_GAO_DEF,
        &BUFF_TB_SHENG_JING_DEF,
        &BUFF_TB_JIE_HUO_WU_DEF,
        &BUFF_TB_LONG_YIN_WU_DEF,
        &BUFF_TB_ZHAN_FENG_WU_DEF,
        &BUFF_TB_HAN_XIAO_WU_DEF,
        &BUFF_TB_HAO_LING_WU_DEF,
        &BUFF_TB_HONG_FA_WU_DEF,
        &BUFF_TB_DUN_DANG_WU_DEF,
        &BUFF_TB_LUO_SHANG_WU_DEF,
        &BUFF_TB_XIAN_DING_WU_DEF,
        &BUFF_TB_JIN_LU_WU_DEF,
        &BUFF_TB_QIU_SU_WU_DEF,
        &BUFF_TB_ZHONG_HE_WU_DEF,
        &BUFF_TB_ZHUO_LIAN_WU_DEF,
        &BUFF_TB_CHAO_SHENG_WU_DEF,
        &BUFF_TB_SHI_HOU_WU_DEF,
    ]
}
