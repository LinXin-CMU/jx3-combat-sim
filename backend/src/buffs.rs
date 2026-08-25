//! Buff 系统：类型定义 + Buff ID 常量
//!
//! BuffDef 静态实例与 `get_buff_def` 注册表按武学版本分别放置：
//! `scripts/v{版本}/buffs/defs.rs`。本文件只保留跨版本稳定的 ID 常量与类型。

// ─────────────────────────────────────────────────────────────────────────────
// 类型定义
// ─────────────────────────────────────────────────────────────────────────────

/// 属性字段（buff/秘籍统一加成单元）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttribField {
    // ── 主属性（动态加成；常驻基础值已在面板内）──
    VitalityBase,                         // 体质
    AgilityBase,                          // 身法
    StrengthBase,                         // 力道
    SpiritBase,                           // 根骨
    SpunkBase,                            // 元气
    BasePotentialAdd,                     // 全主属性加算
    VitalityBasePercentAdd,               // 体质百分比加成 +N/1024（活血奇穴 102 ≈ +10%）

    // ── 攻击 / 副属性等级加算 ──
    PhysicsAttackPowerBase,               // 外功攻击 +N（数值）
    PhysicsAttackPowerPercent,            // 外功攻击 +N/1024（百分比）
    PhysicsCriticalStrike,                // 外功会心等级 +N
    PhysicsCriticalDamagePowerBase,       // 外功会心效果等级 +N
    PhysicsOvercomeBase,                  // 外功破防等级 +N
    PhysicsOvercomePercent,               // 外功破防等级 +N/1024
    StrainBase,                           // 无双等级 +N
    StrainBasePercentAdd,                 // 无双等级 +N/1024（百分比，乘性加成基础无双等级）
    StrainPercent,                        // 无双率 +N/1024（直接加到最终无双率，不乘基础值）
    SurplusValueBase,                     // 破招值 +N
    ParryValueBase,                       // 拆招值 +N（铁骨衣防御向；寒甲奇穴计算用）
    VitalityToParryValueCof,              // 体质→拆招值 转化系数 +N/1024（按 vitality×∑cof/1024 → ParryValueBase 聚合时换算）
    VitalityToAttackCof,                  // 体质→外功攻击 转化系数（铁骨气劲：0.198 或 0.594）
    VitalityToOvercomeCof,                // 体质→破防等级 转化系数（铁骨气劲：0.152 或 0.456）
    ParryBase,                            // 招架等级 +N（数值加算）
    ParryValuePercent,                    // 招架率 +N/10000（直接加到最终招架率；坚铁每层 600）
    HasteBase,                            // 加速等级 +N
    UnlimitedAdditionalHastePercent,      // 突破上限加速 +N/1024（绕过 25% 封顶，战绝等用）

    // ── 目标 debuff 字段（聚合自 target_buffs）──
    TargetPhysicsShieldBase,              // 目标外功防御等级 +N（数值）
    TargetPhysicsShieldPercent,           // 目标外功防御等级 +N/1024（百分比，负值=虚弱减防）
    TargetDamageBonusPercent,             // 目标受到伤害 +N/1024（戒火/龙吟·悟/战锋·悟/劲风等团辅 debuff）

    // ── 武器伤害（节日食物 buff 加算到 attr.weapon_damage 的通道）──
    WeaponDamageBase,                     // 武器伤害 +N（瑰栗粽/梅花糕/春节·升景）

    // ── 伤害链字段 ──
    AllDamageAddPercent,                  // 全局增伤 +N/1024（含破招）
    PveAddition,                          // 非侠士增伤 +N/1024
    AllShieldIgnorePercent,               // 全局无视防御 +N/1024（破招也吃）

    // ── 秘籍专属字段（不应直接由 buff 给出）──
    RecipeDamagePercent,                  // 秘籍增伤 +N（小数，仅普通伤害）
    SurplusValueAddPercent,               // 秘籍破招加成 +N（小数，仅破招）

    // ── buff 属性修改（1024 制）──
    SurplusPercent,                       // buff 破招加成 +N/1024（严阵等）
    RecipePhysicsCriticalPercent,         // 秘籍会心 +N（小数，全部伤害）
    RecipeCriticalDamagePower,            // 秘籍会心效果 +N（小数，全部伤害）

    // ── 最终百分比加成（直接加到最终比例，不走等级换算）──
    PhysicsCriticalStrikePercent,         // 外功会心率 +N/1024（直接加到最终会心率）
    PhysicsCriticalDamagePowerPercent,    // 外功会心效果 +N/1024（直接加到最终会心效果比例）
}

/// Buff 字段加成项
#[derive(Debug, Clone, Copy)]
pub struct EffectEntry {
    pub field: AttribField,
    pub value: f64,
}

/// DoT 属性快照（斩刀释放时记录，流血每跳用其值）
/// 快照字段：攻击、会心、会效、无双、全局增伤
/// 实时字段：破防、防御、PVE、易伤、等级压制
#[derive(Debug, Clone)]
pub struct DotSnapshot {
    pub panel_attack: f64,
    pub crit_rate: f64,      // 0~1，已含秘籍加成
    pub crit_power: f64,     // 含 1.75 基底 + 秘籍
    pub strain: f64,         // 0~ 比例
    pub all_dmg_add: f64,    // 小数 (51/1024 ≈ 0.0498 的那种)
}

/// add_buff 系列方法的入参：u32（无级别）或 (u32, level)
#[derive(Debug, Clone, Copy)]
pub struct BuffSpec { pub buff_id: u32, pub level: u32 }
impl From<u32> for BuffSpec {
    fn from(id: u32) -> Self { BuffSpec { buff_id: id, level: 0 } }
}
impl From<(u32, u32)> for BuffSpec {
    fn from((id, lv): (u32, u32)) -> Self { BuffSpec { buff_id: id, level: lv } }
}

/// Buff 运行时实例（挂在玩家身上的活跃 Buff）
pub struct BuffInstance {
    pub buff_id: u32,
    pub stacks: u32,
    pub duration_frames: u32,
    pub expires_at: f64,
    pub tick_elapsed: u32,
    /// 实际每跳间隔帧数（DoT 受加速影响时存运行时值；非加速型 = def.tick_interval）
    pub tick_interval_frames: u32,
    /// Buff 等级（盾挡等"按等级映射不同属性"的 buff 用；0 = 无等级概念）
    pub level: u32,
    /// DoT 快照（仅流血等快照型 buff 使用；添加/刷新时写入）
    pub snapshot: Option<DotSnapshot>,
    /// 实例级动态 effects（如盾挡施展时按 level 查 cof 挂到实例上）
    /// 聚合时与 def.effects 一起累加
    pub extra_effects: Vec<EffectEntry>,
    /// 期望层数（连续值；坚铁/寒甲等期望传播 buff 用）
    /// - `None` ：常规离散 buff，聚合时用 `stacks as f64`
    /// - `Some(x)` ：替代 `stacks` 作为 effects 乘数（x 为连续浮点）
    pub expected_stacks: Option<f64>,
    /// 层数概率分布（坚铁等连续 buff 用于 UI 显示热力图）
    /// - `None` ：不展示分布
    /// - `Some(probs)` ：probs[k] = P(stacks=k)，长度按 buff 自定义
    pub stack_distribution: Option<Vec<f64>>,
}

/// Buff 定义（静态模板）
/// 所有事件行为通过 scripts/mod.rs 注册表查找
pub struct BuffDef {
    pub buff_id: u32,
    pub name: &'static str,
    pub description: &'static str,
    pub duration_frames: u32,
    pub tick_interval: u32,
    pub max_stacks: u32,
    pub is_debuff: bool,
    /// 是否显示到时间轴（颜色由前端 renderBuffTimeline 自动按顺序生成渐变色）
    pub show_on_timeline: bool,
    /// 时间轴显示顺序（越小越靠上）
    pub timeline_order: u32,
    /// 时间轴标签简写（None = 用 name）
    pub short_name: Option<&'static str>,
    /// 字段化属性加成（buff 在身上时常驻生效）
    pub effects: &'static [EffectEntry],
    /// 此 buff 激活时主动激活的隐藏秘籍 ID
    pub activate_recipes: &'static [u32],
    /// DoT 是否受加速影响（true：tick_interval 和 duration 按加速缩短，总跳数不变）
    pub haste_scaled: bool,
    /// 图标 URL（按 name 在 ui/scheme/case/buff.txt 反查 IconID 拼成 jx3box CDN URL）；
    /// 空串 = 没图标，前端 buff 列表显示原文字风格
    pub icon: &'static str,
}

// ─────────────────────────────────────────────────────────────────────────────
// Buff ID 常量（游戏 ID，跨版本稳定）
// ─────────────────────────────────────────────────────────────────────────────

// 姿态（由 Player 内部使用）
pub const BUFF_STANCE_SHIELD: u32 = 0xF0_00_00_01;
pub const BUFF_STANCE_BLADE:  u32 = 0xF0_00_00_02;

// 游戏原始姿态 buff ID（隐藏，宏判断用）
pub const BUFF_STANCE_SHIELD_GAME: u32 = 8277; // 盾姿态（擎盾）
pub const BUFF_STANCE_BLADE_GAME:  u32 = 8278; // 刀姿态（擎刀）
pub const BUFF_STANCE_WALL:   u32 = 0xF0_00_00_03;

// 技能/效果 Buff（项目自定义 ID）
pub const BUFF_DUN_FEI: u32       = 0xD0_00_00_01; // 盾飞（每秒跳伤害）
pub const BUFF_XUE_NU: u32        = 0xD0_00_00_02; // 血怒
pub const BUFF_XUE_NU_JY: u32     = 0xD0_00_00_03; // 血怒·惊涌
pub const BUFF_JIE_HUA: u32       = 0xD0_00_00_04; // 劫化
pub const BUFF_FENG_MING: u32     = 0xD0_00_00_05; // 锋鸣
pub const BUFF_DUN_FEI_DELAY: u32 = 0xD0_00_00_06; // 盾飞延迟切换
pub const BUFF_XU_RUO_DELAY: u32  = 0xD0_00_00_07; // 虚弱添加延迟
pub const BUFF_KUANG_JUE: u32     = 0xD0_00_00_08; // 狂绝（绝返奇穴）
pub const BUFF_XUE_NU_CD: u32     = 0xD0_00_00_09; // 血怒叠层窗口
pub const BUFF_XIAN_ZHEN_CD: u32  = 0xD0_00_00_0A; // 陷阵奇穴：地坼 15s 内置 CD

// 游戏原始 ID
pub const BUFF_XU_RUO: u32         = 8248;   // 虚弱（目标 debuff）
pub const BUFF_LIU_XUE: u32        = 8249;   // 流血（目标 debuff）
pub const BUFF_JIAN_DING: u32      = 8424;   // 坚定（自身 buff）
pub const BUFF_DUN_WEI: u32        = 8397;   // 盾威（目标 debuff，奇穴 13320）
pub const BUFF_YUAN_GE_ID: u32     = 27030;  // 援戈
pub const BUFF_SHI_XUE: u32        = 17176;  // 嗜血
pub const BUFF_CHENG_WU: u32       = 8474;   // 橙武
pub const BUFF_XUE_SHI_COUNT: u32  = 24323;  // 以血盟誓（血誓计数）
pub const BUFF_XUE_SHI: u32        = 24324;  // 血誓
pub const BUFF_LIN_GUANG: u32      = 25941;  // 麟光玄甲
pub const BUFF_LIN_GUANG_COUNT: u32 = 26214; // 麟光甲寒计数（隐藏）
pub const BUFF_CHANG_QU: u32       = 27444;  // 长驱万里（雾海寻龙·阵云实验性）
pub const BUFF_LIN_AN: u32         = 26212;  // 麟黯
pub const BUFF_BU_CAN: u32         = 562;    // 步残
pub const BUFF_HUAN_SHEN: u32      = 8738;   // 缓深
pub const BUFF_JUAN_YUN: u32       = 8398;   // 卷云
pub const BUFF_ZHAN_JUE: u32       = 31518;  // 战绝
pub const BUFF_DUN_DANG: u32           = 8499;   // 普通盾挡（铁骨衣；无千山时获得；1~10级给不同 atVitalityToParryValueCof）
pub const BUFF_DUN_DANG_QIAN_SHAN: u32 = 8448;   // 千山盾挡（铁骨衣+千山奇穴 13421；系数约为普通版 1.25x）
pub const BUFF_HAN_XIAO: u32      = 33210;  // 新寒啸无双（暗影千机赛季；每层 +67 StrainBase，max 100，15 秒；旧 ID 10031 已弃用）
pub const BUFF_SHEN_BING_WU_SHUANG: u32 = 29608; // 神兵·无双气劲（橙武 atSkillEventHandler 3149/3068 等触发；按武器档位绑 atStrainBase；最多 5 层 6 秒）
pub const BUFF_FUMO_BODONG: u32   = 15455;  // 输出伤害波动（暗影千机赛季腰附魔 16449 触发；+5% 全局增伤 8s）

// ── 大附魔 站立类（atExecuteScript LUA AddBuff；模拟开始挂；永久）──
pub const BUFF_FU_DA_DPS_HAT:    u32 = 15436;  // DPS 帽大附魔 16453: 气血>75% +4099 破防 lvl 17
pub const BUFF_FU_DA_DPS_YI:     u32 = 17012;  // DPS 衣大附魔 16452: 永久 +1114 外攻+1243 内攻（伪 ID）
// T 大附魔（铁骨衣 T 玩家用）— DPS 相关 2 件
pub const BUFF_FU_DA_T_HAT:      u32 = 154131; // T 帽 16443: Cast 10% → 队友含自己 +1114 外攻 8s（伪 ID）
pub const BUFF_FU_DA_T_WRIST:    u32 = 24767;  // T 腕 16441: Cast 10% → +5% 全伤害 5s, CD 25s

// ── 副本精简/无修精简 装备黄字特效 buff ID ──
// 数据源：Buff.tab + IcyTide/Generator buffs.json（暗影千机品级）
pub const BUFF_YE_BELT_MULTI:    u32 = 30742;  // 腰多 进战 +1350 破+1350 会
pub const BUFF_YE_PANTS_SINGLE:  u32 = 30748;  // 裤单 进战 +3857 无双
pub const BUFF_YE_SHOES_CRIT:    u32 = 29524;  // 鞋·会 proc +6000 会，10s
pub const BUFF_YE_SHOES_OVERCOME: u32 = 29526; // 鞋·破 proc +6000 破，10s

// 腰单 proc（1 个 buff_id，2 个 lvl 表示词条；这里拆 2 个伪 ID 以便区分）
pub const BUFF_YE_BELT_SINGLE_CRIT:    u32 = 307437;  // buff 30743 lvl 7 +18561 会
pub const BUFF_YE_BELT_SINGLE_OVERCOME: u32 = 307438; // buff 30743 lvl 8 +18561 破

// 裤多 proc — 拆 3 个 buff（rate / ov-only / crit-only），按装备词条 + strain 阈值切换
pub const BUFF_YE_PANTS_MULTI_RATE:    u32 = 30749;  // 30749/30770 L5 共用 +181 strain_rate
pub const BUFF_YE_PANTS_MULTI_OV:       u32 = 307496;  // 30749 L6 +23540 破防 (伪 ID)
pub const BUFF_YE_PANTS_MULTI_CR:       u32 = 30770;  // 30770 L6 +23540 会心

// 腰坠 proc 5 层
pub const BUFF_YE_PENDANT_OVERCOME:    u32 = 29536;  // 腰坠·破防 +385/层
pub const BUFF_YE_PENDANT_CRIT_EFF:    u32 = 29537;  // 腰坠·会效 +385/层

// 戒指
pub const BUFF_YE_RING_OVERCOME:       u32 = 30755;  // 戒指·破防 +1928
pub const BUFF_YE_RING_CRIT:           u32 = 30756;  // 戒指·会心 +1928
pub const BUFF_YE_RING_SURPLUS:        u32 = 30757;  // 戒指·破招 +1928

// 帽 38934 max-pick 三个 stat 变体（buff 29519 lvl 22/23/24 的伪 ID 拆分）
pub const BUFF_YE_HAT_OVERCOME:        u32 = 2951922; // +3471 破防
pub const BUFF_YE_HAT_CRIT:            u32 = 2951923; // +3471 会心
pub const BUFF_YE_HAT_SURPLUS:         u32 = 2951924; // +3471 破招

// 项链 max-stack 10 buff
pub const BUFF_YE_NECK_CRIT_EFF:       u32 = 29528;  // +192 会效/层 (max 10)
pub const BUFF_YE_NECK_ATTACK:         u32 = 29529;  // +52 攻击/层 (max 10)
pub const BUFF_MIE_SHI: u32       = 9889;   // 蔑视（奇穴 39045）：无视目标 50% 外功防御，10秒；伤害招式命中后触发
pub const BUFF_ZHEN_FEN: u32      = 8504;   // 振奋（奇穴 13422）：盾挡后每 2820 vitality 一层，+111 StrainBase/层，最大100层，30秒
pub const BUFF_HAN_JIA: u32       = 8437;   // 寒甲（12秒，由 on_tick 刷新 8271/17772 层数）
pub const BUFF_HAN_JIA_SMALL: u32 = 8271;   // 寒甲小层：每层 +300 PhysicsAttackPowerBase（最大125层）
pub const BUFF_HAN_JIA_LARGE: u32 = 17772;  // 寒甲大层：每层 +30000 PhysicsAttackPowerBase（最大125层）
pub const BUFF_JIAN_TIE: u32      = 8272;  // 坚铁（8272）：每层 +6% 招架率，最多 5 层
pub const BUFF_LIAN_ZHAN_CD: u32  = 8321;  // 恋战内置 CD（132 帧 = 8.25s）：存在期间坚铁不叠层
pub const BUFF_NU_YAN: u32        = 24755; // 怒炎（奇穴 13133）：6 秒内下次绝刀返还怒气
pub const BUFF_WU_JU: u32         = 8247;  // 无惧（免疫控制 6 秒）
pub const BUFF_JI_ANG: u32        = 8418;  // 激昂（奇穴 13356）：每层 +66 招架 +166 拆招 +331 破招，max 100
pub const BUFF_YAN_ZHEN: u32      = 25356; // 严阵（奇穴 25356）：每层 +50% 破招，max 3，20s
pub const BUFF_TIE_GU: u32        = 29938; // 铁骨（副T）：每点体质 +0.198 攻击 +0.152 破防
pub const BUFF_TIE_GU_SU_DI: u32  = 17885; // 铁骨·宿敌（主T）：每点体质 +0.594 攻击 +0.456 破防

// ── 阵法触发型 buff（自己开苍云阵时按 §5.3 拟真挂载）──
pub const BUFF_FENG_LING: u32     = 8403;  // 锋凌（5重，绝刀触发，5层叠满 +100 会效等级，30s）
pub const BUFF_FENG_JUE:  u32     = 8484;  // 锋绝（4重，招式会心 → +154 破防等级，5s）
pub const BUFF_HENG_JUE:  u32     = 8404;  // 横绝（6重，被攻击 → +102 攻击百分比，5s）
