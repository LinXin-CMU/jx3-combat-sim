//! 装备效果统一注册表
//!
//! 所有"按装备 ID / 套装件数判定"的特效集中到此：
//!   1. 橙武 / 小橙武主武器 ID 列表
//!   2. 套装 N 件套 ID 列表（4 件套 hidden recipe 激活用）
//!   3. 神兵·无双气劲（buff 29608）单层 atStrainBase 数值表
//!
//! 用法：
//!   - `collect_recipes_indexed`：`player.has_equip_in(...)` / `player.count_equip_in(...)`
//!   - 脚本侧（dun_ji.rs / dun_ya.rs 等）：直接 `use crate::equip_effects::TIANXIA_HONGYUAN_WEAPON_IDS;`
//!
//! 维护提醒：橙武等级有新增时记得同步以下三处：
//!   1. 本文件的常量数组
//!   2. backend/data/{version}/{mount}/skills/{id}_*.toml 里的被动技能（如 25780 盾击神兵）
//!   3. backend/data/{version}/recipes.toml 里的 hidden recipe（如 99260 天下宏愿盾飞+5%）

// ─────────────────────────────────────────────────────────────────────────────
// 苍云外功橙武 / 小橙武主武器
// ─────────────────────────────────────────────────────────────────────────────

/// 天下宏愿（分山外功大橙武，10 把）
/// 装备特效：99260 盾飞+5% / 99261 斩刀+5% / 25780 盾击·神兵 触发
pub const TIANXIA_HONGYUAN_WEAPON_IDS: &[u32] = &[
    40213, 40245, 40277, 40309, 42128, 42160, 43553, 43586, 45287, 45320,
];

/// 驭焰（铁骨防御大橙武，10 把）
/// 装备特效：25797 盾压·神兵 触发
pub const YU_YAN_WEAPON_IDS: &[u32] = &[
    40214, 40246, 40278, 40310, 42129, 42161, 43554, 43587, 45288, 45321,
];

/// 幽烽蝶语·式微（外功老小橙武，6 把）
/// 装备特效：99262 盾刀会心+5% / 99263 斩刀会心+5%
pub const YOU_FENG_DIE_YU_SHI_WEI_IDS: &[u32] = &[44121, 44154, 44187, 44220, 44253, 44286];

/// 幽烽蝶语·不归（防御老小橙武，6 把）—— 当前无 PVE 输出特效，仅占位
pub const YOU_FENG_DIE_YU_BU_GUI_IDS: &[u32] = &[44122, 44155, 44188, 44221, 44254, 44287];

// ─────────────────────────────────────────────────────────────────────────────
// 苍云外功 T 套 4 件套（hidden recipe 99270 绝刀+10% / 99271 盾压+10%）
// 三套同效：5936 西塞·行歌 / 6481 孤漠·雁际 / 6782 玄阙·芦荡
// ─────────────────────────────────────────────────────────────────────────────

pub const CY_DPS_T_SET_IDS: &[u32] = &[
    // 5936 西塞·行歌（22500）
    101022, 101053, 101084, 101115, 101146, // 6481 孤漠·雁际（25900）
    101351, 101382, 101413, 101444, 101475,
    // 6782 玄阙·芦荡（30200/35300/41400）—— 3 个 lv 共 15 件
    104669, 104700, 104731, 104762, 104793, // 天极·风旗 lv30200
    106687, 106719, 106751, 106783, 106815, // 牧川·秋塞 lv35300
    109148, 109180, 109212, 109244, 109276, // 玄阙·芦荡 lv41400
];

// ─────────────────────────────────────────────────────────────────────────────
// 苍云 JJC 竞技场 4 件套（1929 血怒 CD-3s）
// 同效：6018 雪飞 / 6302 旷世无匹·从容 / 6516 剑意狂歌·景盛 / 6817 漫天飞羽·锋戎
// ─────────────────────────────────────────────────────────────────────────────

pub const CY_JJC_SET_IDS: &[u32] = &[
    // 6018 波光剑戟·雪飞（22500）
    102367, 102394, 102421, 102475, 102502,
    // 6302 旷世无匹·从容（25900/27100）
    102853, 102880, 102907, 102961, 102988, 103339, 103366, 103393, 103447, 103474,
    // 6516 剑意狂歌·景盛（29800/31200）
    105829, 105856, 105883, 105937, 105964, 105991, 106018, 106045, 106099, 106126,
    // 6817 漫天飞羽·锋戎（34800/36500/40800/42800）
    107897, 107925, 107953, 108009, 108037, 108065, 108093, 108121, 108177, 108205, 110358, 110386,
    110414, 110470, 110498, 110526, 110554, 110582, 110638, 110666,
];

// ─────────────────────────────────────────────────────────────────────────────
// 苍云威望套 4 件套（1930 无惧 CD-2s）
// 同效：6329 千帆 / 6356 雷鸣 / 6544 覆海 / 6572 焚野 / 6845 霁月 / 6873 魇雾
// ─────────────────────────────────────────────────────────────────────────────

pub const CY_WEI_WANG_SET_IDS: &[u32] = &[
    // 6329 千帆竞发·尘骨（25900/27100）
    102529, 102556, 102583, 102637, 102664, 103015, 103042, 103069, 103123, 103150,
    // 6356 雷鸣风怒·关寒（25900/27100）
    102691, 102718, 102745, 102799, 102826, 103177, 103204, 103231, 103285, 103312,
    // 6544 覆海惊澜·重楼（29800/31200）
    105181, 105208, 105235, 105289, 105316, 105505, 105532, 105559, 105613, 105640,
    // 6572 焚野烬风·刃寒（29800/31200）
    105343, 105370, 105397, 105451, 105478, 105667, 105694, 105721, 105775, 105802,
    // 6845 霁月玄霜·代春 / 天风霄汉·塞北（34800~42800）
    107225, 107253, 107281, 107337, 107365, 107561, 107589, 107617, 107673, 107701, 109686, 109714,
    109742, 109798, 109826, 110022, 110050, 110078, 110134, 110162,
    // 6873 魇雾苍火·替冬 / 地火冥幽·惊敌（34800~42800）
    107393, 107421, 107449, 107505, 107533, 107729, 107757, 107785, 107841, 107869, 109854, 109882,
    109910, 109966, 109994, 110190, 110218, 110246, 110302, 110330,
];

// ─────────────────────────────────────────────────────────────────────────────
// 苍云守护 T 套（4 件套：1976 盾壁 CD-10s + 17250 对阵+5%；2 件套：1222 黄字）
// 三套同效：5937 西塞·旋草 / 6482 孤漠·远别 / 6783 玄阙·武盏
// ─────────────────────────────────────────────────────────────────────────────

pub const CY_GUARDIAN_T_SET_IDS: &[u32] = &[
    // 5937 西塞·旋草（22500）
    101023, 101054, 101085, 101116, 101147, // 6482 孤漠·远别（25900）
    101352, 101383, 101414, 101445, 101476,
    // 6783 玄阙·武盏 / 天极·铸铁 / 牧川·仞横（30200/35300/41400）
    104670, 104701, 104732, 104763, 104794, // 天极·铸铁 lv30200
    106688, 106720, 106752, 106784, 106816, // 牧川·仞横 lv35300
    109149, 109181, 109213, 109245, 109277, // 玄阙·武盏 lv41400
];

// ─────────────────────────────────────────────────────────────────────────────
// 神兵·无双气劲（buff 29608）单层 atStrainBase 查表
// 触发源：苍云外功橙武 atSkillEventHandler 命中后
// 数值映射：按主武器 level 升序与表（用户提供 11+6 档）升序一一对应
// ─────────────────────────────────────────────────────────────────────────────

const SHEN_BING_WU_SHUANG_STRAIN_TABLE: &[(u32, f64)] = &[
    // 天下宏愿（10 把按 level 升序对应表 17100~32400 这 10 档）
    (40213, 671.0),  // lv 22875 → 17100CW
    (40245, 718.0),  // lv 24375 → 18300CW
    (40277, 765.0),  // lv 25940 → 19500CW
    (40309, 814.0),  // lv 27500 → 20750CW
    (42128, 864.0),  // lv 29575 → 22000CW
    (42160, 930.0),  // lv 31625 → 23500CW
    (43553, 997.0),  // lv 34375 → 25000CW
    (43586, 1080.0), // lv 37125 → 27500CW
    (45287, 1166.0), // lv 40500 → 29700CW
    (45320, 1272.0), // lv 42500 → 32400CW
    // 幽烽蝶语·式微（小橙武 6 把一一对应）
    (44121, 769.0),  // lv 24500 → 19600 小橙武
    (44154, 848.0),  // lv 27000 → 21600
    (44187, 926.0),  // lv 29500 → 23600
    (44220, 1005.0), // lv 32000 → 25600
    (44253, 1099.0), // lv 35000 → 28000
    (44286, 1241.0), // lv 39750 → 31600
];

/// 查给定主武器 id 的"神兵·无双气劲" buff (level, 单层 atStrainBase)
/// level: 武器在数值表里的档位序号（1~16，前 10 = 天下宏愿，后 6 = 幽烽蝶语·式微）
/// None = 该武器不在橙武列表，无此特效
pub fn shen_bing_wu_shuang_for(weapon_id: u32) -> Option<(u32, f64)> {
    SHEN_BING_WU_SHUANG_STRAIN_TABLE
        .iter()
        .enumerate()
        .find(|(_, (id, _))| *id == weapon_id)
        .map(|(i, (_, v))| ((i as u32) + 1, *v))
}

// ─────────────────────────────────────────────────────────────────────────────
// 暗影千机赛季 5 件大附魔（仅 4 件影响 DPS：帽 / 腰 / 腕 / 鞋）
// ─────────────────────────────────────────────────────────────────────────────

/// 帽 16453（atExecuteScript LUA → AddBuff 15413 L17 暗影千机品级）
/// 站立 buff：atAllTypeAttackPowerBase=1114, atMagicAttackPowerBase=129（DPS 心法只用外攻）
pub const ENCHANT_HAT_AYQJ: u32 = 16453;

/// 衣 16452（atExecuteScript LUA → AddBuff 15415 L17 暗影千机品级）
/// 站立 buff：atGlobalDamageAbsorb=267300（伤害承担护盾，**不影响 DPS**，本模拟器不建模）
pub const ENCHANT_JACKET_AYQJ: u32 = 16452;

/// 腰 16449（atSkillEventHandler 3108 → SkillID 22169 → AddBuff 15455 L2）
/// 命中触发：atAllDamageAddPercent=51（≈+5% 全局增伤），持续 8s
pub const ENCHANT_BELT_AYQJ: u32 = 16449;

/// 腕 16451（atSkillEventHandler 3109 → SkillID 38984 lv5）
/// 命中触发：直接打真实伤害（不吃会心，其他逻辑全走）
pub const ENCHANT_WRIST_AYQJ: u32 = 16451;

/// 鞋 16450（atSkillEventHandler 3110 → SkillID 38985 lv5）
/// 会心触发：直接打真实伤害（不吃会心，其他逻辑全走）
pub const ENCHANT_SHOES_AYQJ: u32 = 16450;

/// 帽附魔 站立加成（暗影千机赛季成品）：外功攻击 base
pub const ENCHANT_HAT_ATTACK_BASE: f64 = 1114.0;

/// 腕附魔真实伤害（暗影千机品级，jx3dps-online 数值）
pub const ENCHANT_WRIST_TRUE_DAMAGE: f64 = 2_415_000.0;
/// 鞋附魔真实伤害（暗影千机品级，jx3dps-online 数值）
pub const ENCHANT_SHOES_TRUE_DAMAGE: f64 = 1_610_000.0;

/// 腰附魔触发概率（1024 制；3108 odds=205）
pub const ENCHANT_BELT_PROB: i32 = 205;
/// 腕附魔触发概率（1024 制；3109 odds=102）
pub const ENCHANT_WRIST_PROB: i32 = 102;
/// 鞋附魔触发概率（1024 制；3110 odds=1024，会心 100%；后端按 cast 累加 + 内置 CD 节流）
pub const ENCHANT_SHOES_PROB: i32 = 1024;

/// 腕附魔内置 CD（秒；jx3dps-online 模型 ≈ 每 15s 一次）
pub const ENCHANT_WRIST_INNER_CD: f64 = 15.0;
/// 鞋附魔内置 CD（秒；≈ 每 10s 一次）
pub const ENCHANT_SHOES_INNER_CD: f64 = 10.0;

/// 腰 buff 持续帧（8s × 16fps = 128 帧）
pub const ENCHANT_BELT_BUFF_FRAMES: u32 = 128;

// ─────────────────────────────────────────────────────────────────────────────
// 副本精简 / 无修精简 装备黄字特效（暗影千机赛季 41400 品级）
// 数据源：IcyTide/Generator (gains/gears.py + buffs.json + skills.json)
// ID 即 IcyTide SPECIAL_GEAR_GAINS 的 gain_id（atSkillEventHandler ref）
// 复用 Player.equipped HashMap：前端把 "YEFFECT_<id>": <gain_id> 塞进 equipment 即激活
// ─────────────────────────────────────────────────────────────────────────────

// ── 站立类（永久加成，aggregate_buff_fields 直接加）──
/// 帽 38934 → +3471 自身 破防/会心/破招 中**最高**那个属性
pub const YE_HAT_3471: u32 = 38934;
/// 腰带·多 40790 → 进战后 +1350 破防 + 1350 会心
pub const YE_BELT_MULTI_1350: u32 = 40790;
/// 裤·单 40793 → 进战后 +3857 无双等级
pub const YE_PANTS_SINGLE_3857: u32 = 40793;

// ── proc 类（命中触发，equip_effect_accum + 内置 CD）──
/// 鞋·会 38939 → 命中 proc +6000 会心，10s 持续 + 10s CD（buff 29524 lvl 8）
pub const YE_SHOES_CRIT_6000: u32 = 38939;
/// 鞋·破 38944 → 命中 proc +6000 破防，10s 持续 + 10s CD（buff 29526 lvl 8）
pub const YE_SHOES_OVERCOME_6000: u32 = 38944;

// ── 真实伤害类（命中触发，emit 真伤事件 + 内置 CD）──
/// 护手 40788 → 命中触发 1,138,846 真伤（自身气血>75%；CD 10s）
pub const YE_WRIST_TRUE_DMG: u32 = 40788;
/// 暗器 38950 → 命中触发 976,154 真伤（CD 10s）
pub const YE_DART_TRUE_DMG: u32 = 38950;
/// 加速腰带 42767 → 命中触发 268,000 真伤（CD 40s）
pub const YE_BELT_HASTE_TRUE: u32 = 42767;

// ── 数值（暗影千机赛季顶级 lvl）──
pub const YE_HAT_BONUS: f64 = 3471.0;
pub const YE_BELT_MULTI_BONUS: f64 = 1350.0;
pub const YE_PANTS_STRAIN_BONUS: f64 = 3857.0;
pub const YE_SHOES_PROC_BONUS: f64 = 6000.0;

pub const YE_WRIST_TRUE_DAMAGE: f64 = 1_138_846.0;
pub const YE_DART_TRUE_DAMAGE: f64 = 976_154.0;
pub const YE_BELT_HASTE_TRUE_DAMAGE: f64 = 268_000.0;

/// 鞋 proc 概率（skillevent.txt 100% Hit）
pub const YE_SHOES_PROC_PROB: i32 = 1024;
/// 鞋 proc 内置 CD（装备面板原文：CD 20s）
pub const YE_SHOES_INNER_CD: f64 = 20.0;
/// 鞋 proc 持续时间（10s = 160 帧）
pub const YE_SHOES_BUFF_FRAMES: u32 = 160;

/// 真伤类内置 CD
pub const YE_WRIST_INNER_CD: f64 = 10.0;
pub const YE_DART_INNER_CD: f64 = 10.0;
pub const YE_BELT_HASTE_INNER_CD: f64 = 40.0;

// ─────────────────────────────────────────────────────────────────────────────
// T 大附魔（御·X 系列）— 仅 2 件影响 DPS（帽 16443 / 腕 16441）
// ─────────────────────────────────────────────────────────────────────────────
/// T 帽 16443（atSkillEventHandler 3117 → SkillID 22122 lvl 17 → AddBuff 15413 L17, dur=128f 8s）
pub const ENCHANT_HAT_T: u32 = 16443;
/// T 腕 16441（atSkillEventHandler 3115 → SkillID 33249 → AddBuff 24767 L1, dur=48f 3s）
pub const ENCHANT_WRIST_T: u32 = 16441;
/// T 帽/T 腕 触发概率（102/1024 ≈ 10%, EventType=Cast）
pub const ENCHANT_T_HAT_PROB: i32 = 102;
pub const ENCHANT_T_WRIST_PROB: i32 = 102;

// 项链阈值（来源：装备面板 8586 暗影千机品级）
pub const YE_NECK_CRIT_THRESHOLD_OV: f64 = 8586.0;

// ── 第二批：proc / 叠层 / 转化（属性词条决定 variant，每个 variant 独立 ID）──

/// 腰单 40791 → 命中触发 18561 → 取破防/会心**最低**那个属性
/// 装备面板原文："命中目标后, 自身破防/会心中最低的属性 +18561, 持续20s, CD 20s"
pub const YE_BELT_SINGLE: u32 = 40791;
/// 旧的两变体保留兼容（已 deprecated; 现统一走 YE_BELT_SINGLE 动态选最低）
pub const YE_BELT_SINGLE_CRIT: u32 = 407911;
pub const YE_BELT_SINGLE_OVERCOME: u32 = 407912;

/// 裤多·无双率 → buff 30749 lvl 5 +181 strain_rate (≈+17.7%)
pub const YE_PANTS_MULTI_RATE: u32 = 407941;
/// 裤多·会破 → buff 30770 lvl 6 +23540 会心+破防
pub const YE_PANTS_MULTI_CRIT_OVERCOME: u32 = 407942;

/// 项链·会效转化 38945 → 每 8586 会心等级 +192 会效，最多 10 层
pub const YE_NECKLACE_CRIT_TO_CRIT_EFF: u32 = 38945;
/// 项链·攻击转化 38946 → 每 8586 破防等级 +52 攻击 +6 内攻，最多 10 层
pub const YE_NECKLACE_OVERCOME_TO_ATTACK: u32 = 38946;

/// 腰坠·破防 38948 → 命中触发 +385 破防/层，5 层 12s
pub const YE_PENDANT_OVERCOME: u32 = 38948;
/// 腰坠·会效 38949 → 会心触发 +385 会效/层，5 层 12s
pub const YE_PENDANT_CRIT_EFF: u32 = 38949;

/// 戒指·破防 40802 → +1928 破防（自身血量>目标，模拟器假设满足）
pub const YE_RING_OVERCOME: u32 = 40802;
/// 戒指·会心 40803 → +1928 会心
pub const YE_RING_CRIT: u32 = 40803;
/// 戒指·破招 40804 → +1928 破招
pub const YE_RING_SURPLUS: u32 = 40804;

// ── 第二批数值（暗影千机赛季顶级 lvl）──
pub const YE_BELT_SINGLE_BONUS: f64 = 18561.0;
pub const YE_PANTS_MULTI_RATE_1024: f64 = 181.0; // 1024 制
pub const YE_PANTS_MULTI_BONUS: f64 = 23540.0;
pub const YE_NECK_CRIT_THRESHOLD: f64 = 8586.0;
pub const YE_NECK_CRIT_PER_STACK: f64 = 192.0;
pub const YE_NECK_ATTACK_PER_STACK: f64 = 52.0;
pub const YE_NECK_MAGIC_PER_STACK: f64 = 6.0;
pub const YE_NECK_MAX_STACKS: u32 = 10;
pub const YE_PENDANT_PER_STACK: f64 = 385.0;
pub const YE_PENDANT_MAX_STACKS: u32 = 5;
pub const YE_RING_BONUS: f64 = 1928.0;

// ─────────────────────────────────────────────────────────────────────────────
// 大附魔 触发后处理 hook（在 run_scripts 末尾调用）
// 累加器复用 Player.equip_effect_accum（HashMap<u32, i32>），key 用 buff_id/skill_id：
//   - 15455 (BUFF_FUMO_BODONG): 腰累加器
//   - 38984 / 38985:            腕/鞋累加器
// 跟现有装备特效 key（13045/13047 = 盾击/盾压神兵）不撞。
// ─────────────────────────────────────────────────────────────────────────────

use crate::{
    AttribSlots, Player, ScriptEmitter, BUFF_FENG_JUE, BUFF_FUMO_BODONG, BUFF_FU_DA_DPS_HAT,
    BUFF_FU_DA_DPS_YI, BUFF_FU_DA_T_HAT, BUFF_FU_DA_T_WRIST, BUFF_YE_BELT_MULTI,
    BUFF_YE_BELT_SINGLE_CRIT, BUFF_YE_BELT_SINGLE_OVERCOME, BUFF_YE_HAT_CRIT, BUFF_YE_HAT_OVERCOME,
    BUFF_YE_HAT_SURPLUS, BUFF_YE_NECK_ATTACK, BUFF_YE_NECK_CRIT_EFF, BUFF_YE_PANTS_MULTI_CR,
    BUFF_YE_PANTS_MULTI_OV, BUFF_YE_PANTS_MULTI_RATE, BUFF_YE_PANTS_SINGLE,
    BUFF_YE_PENDANT_CRIT_EFF, BUFF_YE_PENDANT_OVERCOME, BUFF_YE_RING_CRIT, BUFF_YE_RING_OVERCOME,
    BUFF_YE_RING_SURPLUS, BUFF_YE_SHOES_CRIT, BUFF_YE_SHOES_OVERCOME,
};

/// 战斗开始时挂 EventType=EnterFight 类 buff（被各版本 buffs::on_battle_start 调用）
///
/// 包含（确认范围）：
///   - 大附魔 DPS 帽 16453 → permanent BUFF_FU_DA_DPS_HAT
///   - 黄字 帽 38934 → max-pick → BUFF_YE_HAT_OVERCOME/CRIT/SURPLUS
///   - 黄字 项链 38945（会效转化）→ floor(crit/8586) min 10 层 → BUFF_YE_NECK_CRIT_EFF
///   - 黄字 项链 38946（攻击转化）→ floor(ov/8586) min 10 层 → BUFF_YE_NECK_ATTACK
///   - 黄字 腰多 40790 → permanent BUFF_YE_BELT_MULTI
///   - 黄字 裤·正常 40793 → permanent BUFF_YE_PANTS_SINGLE
///   - 黄字 裤·高级 40794 → conditional 基于当前 strain rate
///
/// EventType=Hit/Cast/CriticalStrike 类（T 帽/T 腕/腰单/鞋/腰坠/戒指/真伤）→ 走 on_post_cast
pub fn on_battle_start(player: &mut Player) {
    // ── Step A：无条件 EnterFight 类 buff ──
    // 大附魔 DPS 帽 16453: 气血>75% +4099 破防（模拟器假设满足）
    if player.has_enchant(ENCHANT_HAT_AYQJ) {
        player.add_buff(BUFF_FU_DA_DPS_HAT);
    }
    // 大附魔 DPS 衣 16452: 永久 +1114 外攻
    if player.has_enchant(ENCHANT_JACKET_AYQJ) {
        player.add_buff(BUFF_FU_DA_DPS_YI);
    }

    // 黄字 腰多 40790 → +1350 破 + 1350 会 永久
    if player.has_enchant(YE_BELT_MULTI_1350) {
        player.add_buff(BUFF_YE_BELT_MULTI);
    }
    // 黄字 裤·正常 40793 → +3857 无双 永久
    if player.has_enchant(YE_PANTS_SINGLE_3857) {
        player.add_buff(BUFF_YE_PANTS_SINGLE);
    }
    // 黄字 裤·高级 40794 → 基于当前 strain rate 选分支
    let has_pants_multi_rate = player.has_enchant(YE_PANTS_MULTI_RATE);
    let has_pants_multi_x = player.has_enchant(YE_PANTS_MULTI_CRIT_OVERCOME);

    // ── Step B：基于 Step A 后的状态做 max-pick / threshold-stack / conditional ──
    // 把当前累计 slots 计算出来（含 Step A 挂的 buff）
    let slots = crate::aggregate_buff_fields(player);
    let base = &player.base_attrs;
    use crate::AttribField::*;
    let cur_overcome =
        base.overcome_level + slots.get(&PhysicsOvercomeBase).copied().unwrap_or(0.0);
    let cur_crit = base.crit_level + slots.get(&PhysicsCriticalStrike).copied().unwrap_or(0.0);
    let cur_surplus = base.surplus_value + slots.get(&SurplusValueBase).copied().unwrap_or(0.0);
    // strain rate（用于 裤·高级 conditional）：
    let strain_pct = slots.get(&StrainBasePercentAdd).copied().unwrap_or(0.0) / 1024.0;
    let cur_strain_lvl = base.strain_level + slots.get(&StrainBase).copied().unwrap_or(0.0);
    let cur_strain_rate = (cur_strain_lvl / crate::LP_STRAIN) * (1.0 + strain_pct);

    // 黄字帽 38934 max-pick
    if player.has_enchant(YE_HAT_3471) {
        let max = cur_overcome.max(cur_crit).max(cur_surplus);
        if max == cur_overcome {
            player.add_buff(BUFF_YE_HAT_OVERCOME);
        } else if max == cur_crit {
            player.add_buff(BUFF_YE_HAT_CRIT);
        } else {
            player.add_buff(BUFF_YE_HAT_SURPLUS);
        }
    }

    // 黄字项链·会效转化 38945（buff 29528 stacks）
    if player.has_enchant(YE_NECKLACE_CRIT_TO_CRIT_EFF) {
        let stacks = ((cur_crit / YE_NECK_CRIT_THRESHOLD).floor() as u32).min(YE_NECK_MAX_STACKS);
        for _ in 0..stacks {
            player.add_buff(BUFF_YE_NECK_CRIT_EFF);
        }
    }
    // 黄字项链·攻击转化 38946（buff 29529 stacks）
    if player.has_enchant(YE_NECKLACE_OVERCOME_TO_ATTACK) {
        let stacks =
            ((cur_overcome / YE_NECK_CRIT_THRESHOLD).floor() as u32).min(YE_NECK_MAX_STACKS);
        for _ in 0..stacks {
            player.add_buff(BUFF_YE_NECK_ATTACK);
        }
    }

    // 黄字裤·高级 40794 conditional（按装备面板）
    //   - YE_PMR (装备 30749 体系，破防词条): 无双率 ≤ 90% → +17.67% 无双率, 否则 +23540 破防
    //   - YE_PMX (装备 30770 体系，会心词条): 无双率 ≤ 90% → +17.67% 无双率, 否则 +23540 会心
    if has_pants_multi_rate {
        if cur_strain_rate <= 0.9 {
            player.add_buff(BUFF_YE_PANTS_MULTI_RATE);
        } else {
            player.add_buff(BUFF_YE_PANTS_MULTI_OV); // 30749 L6 +23540 破防
        }
    }
    if has_pants_multi_x {
        if cur_strain_rate <= 0.9 {
            player.add_buff(BUFF_YE_PANTS_MULTI_RATE);
        } else {
            player.add_buff(BUFF_YE_PANTS_MULTI_CR); // 30770 L6 +23540 会心
        }
    }
}

/// 占位 — 保持 main.rs 调用兼容。所有 post-pass 逻辑已搬到 apply_combat_start_buffs
pub fn apply_yellow_post_pass(_player: &Player, _slots: &mut AttribSlots) {
    // intentionally empty — see apply_combat_start_buffs Step B
}

/// 主动伤害招式 cast 后调用：检查大附魔 + 副本/无修精简黄字特效 触发
/// 注意：站立类（大附魔帽 16453 / 精简帽 38934 / 腰多 40790 / 裤单 40793）走 aggregate_buff_fields，不在这里
pub fn on_post_cast(player: &mut Player, em: &mut ScriptEmitter, skill: &crate::SkillSpec, t: f64) {
    let is_damage = skill.attack_coeff > 0.0 || skill.base_damage > 0.0;
    if !is_damage {
        return;
    }
    // 自身真伤事件不要触发自己 → 防递归
    matches!(skill.skill_id, 38984 | 38985 | 40789 | 38966 | 42837)
        .then(|| ()) // no-op
        .map(|_| {
            return;
        });
    if matches!(skill.skill_id, 38984 | 38985 | 40789 | 38966 | 42837) {
        return;
    }

    // ── 大附魔 腰 16449: Hit 20% → 期望 3.8% 全局增伤 buff 8s, CD 30s ──
    if player.has_enchant(ENCHANT_BELT_AYQJ) {
        let cd_key = "cd_enchant_腰";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            if player.accum_equip_effect(BUFF_FUMO_BODONG, ENCHANT_BELT_PROB, 1024) {
                player.active_cds.insert(cd_key.to_string(), t + 30.0);
                player.add_buff(BUFF_FUMO_BODONG);
            }
        }
    }

    // ── 大附魔 腕 16451：Hit 10% → 真伤 38984，内置 CD 15s ──
    if player.has_enchant(ENCHANT_WRIST_AYQJ) {
        let cd_key = "cd_enchant_腕";
        let cd_ready = player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t;
        if cd_ready && player.accum_equip_effect(38984, ENCHANT_WRIST_PROB, 1024) {
            player
                .active_cds
                .insert(cd_key.to_string(), t + ENCHANT_WRIST_INNER_CD);
            em.emit("昆吾·弦刃", 38984, t);
        }
    }

    // ── T 大附魔 帽 16443: Cast → 自身 +1114 外攻 8s ──
    // 装备面板"释放招式有几率…20尺范围 5 个队友 +1114 外攻"，用户确认对自己也生效
    if player.has_enchant(ENCHANT_HAT_T) {
        if player.accum_equip_effect(BUFF_FU_DA_T_HAT, ENCHANT_T_HAT_PROB, 1024) {
            player.add_buff(BUFF_FU_DA_T_HAT);
        }
    }
    // ── T 大附魔 腕 16441: Cast 10% → +5% 全伤害 5s, **CD 25s** ──
    if player.has_enchant(ENCHANT_WRIST_T) {
        let cd_key = "cd_enchant_T_腕";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            if player.accum_equip_effect(BUFF_FU_DA_T_WRIST, ENCHANT_T_WRIST_PROB, 1024) {
                player.active_cds.insert(cd_key.to_string(), t + 25.0);
                player.add_buff(BUFF_FU_DA_T_WRIST);
            }
        }
    }

    // ── 大附魔 鞋 16450：crit_rate 比例累加 → 真伤 38985，内置 CD 10s ──
    if player.has_enchant(ENCHANT_SHOES_AYQJ) {
        let cd_key = "cd_enchant_鞋";
        let cd_ready = player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t;
        if cd_ready {
            let crit_rate = player.current_stats().crit_rate.clamp(0.0, 1.0);
            let crit_prob_1024 = (crit_rate * 1024.0).round() as i32;
            if player.accum_equip_effect(38985, crit_prob_1024.max(0), 1024) {
                player
                    .active_cds
                    .insert(cd_key.to_string(), t + ENCHANT_SHOES_INNER_CD);
                em.emit("刃凌", 38985, t);
            }
        }
    }

    // ─── 副本/无修精简 黄字特效 — 全部走真实 buff 实例 / 真实事件 ───

    // ── 腰单 40791：命中触发取**破防/会心中最低**的属性 +18561 (20s, CD 20s) ──
    // 装备面板原文 "破防/会心中最低的属性 +18561"，需要每次 trigger 时动态比较
    if player.has_enchant(YE_BELT_SINGLE) {
        let cd_key = "cd_ye_腰单";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            // 比较当前 ov_level vs crit_level，给较低的那个加 buff
            let base = &player.base_attrs;
            let buff_id = if base.crit_level <= base.overcome_level {
                BUFF_YE_BELT_SINGLE_CRIT // crit 更低 → +18561 crit
            } else {
                BUFF_YE_BELT_SINGLE_OVERCOME // ov 更低 → +18561 ov
            };
            player.active_cds.insert(cd_key.to_string(), t + 20.0);
            player.add_buff(buff_id);
        }
    }
    // 旧兼容：仍允许前端直接传 YE_BELT_SINGLE_CRIT/OVERCOME（强制方向，调试用）
    if player.has_enchant(YE_BELT_SINGLE_CRIT) {
        let cd_key = "cd_ye_腰单";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player.active_cds.insert(cd_key.to_string(), t + 20.0);
            player.add_buff(BUFF_YE_BELT_SINGLE_CRIT);
        }
    }
    if player.has_enchant(YE_BELT_SINGLE_OVERCOME) {
        let cd_key = "cd_ye_腰单";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player.active_cds.insert(cd_key.to_string(), t + 20.0);
            player.add_buff(BUFF_YE_BELT_SINGLE_OVERCOME);
        }
    }

    // ── 鞋·会 38939：命中触发 +6000 会心 10s, CD 20s（装备面板原文）──
    if player.has_enchant(YE_SHOES_CRIT_6000) {
        let cd_key = "cd_ye_鞋会";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player
                .active_cds
                .insert(cd_key.to_string(), t + YE_SHOES_INNER_CD);
            player.add_buff(BUFF_YE_SHOES_CRIT);
        }
    }
    // ── 鞋·破 38944：命中触发 +6000 破防 10s, CD 20s ──
    if player.has_enchant(YE_SHOES_OVERCOME_6000) {
        let cd_key = "cd_ye_鞋破";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player
                .active_cds
                .insert(cd_key.to_string(), t + YE_SHOES_INNER_CD);
            player.add_buff(BUFF_YE_SHOES_OVERCOME);
        }
    }

    // ── 腰坠·破防 38948：命中 60% 概率 +1 层（max 5 层），12s（装备面板原文 60% Hit）──
    if player.has_enchant(YE_PENDANT_OVERCOME) {
        if player.accum_equip_effect(BUFF_YE_PENDANT_OVERCOME, 614, 1024) {
            player.add_buff(BUFF_YE_PENDANT_OVERCOME);
        }
    }
    // ── 苍云阵·4重锋绝（自己开阵时触发）：招式会心后 +154 破防等级 5s ──
    // crit_rate 加权累加（同鞋大附魔模式）；buff.txt 8484 描述"外功破防提高15%"
    if crate::formation_is_self(player, "cangyun") {
        let crit_rate = player.current_stats().crit_rate.clamp(0.0, 1.0);
        let crit_prob_1024 = (crit_rate * 1024.0).round() as i32;
        if player.accum_equip_effect(BUFF_FENG_JUE, crit_prob_1024.max(0), 1024) {
            player.add_buff(BUFF_FENG_JUE);
        }
    }

    // ── 腰坠·会效 38949：会心触发 +1 层（用 crit_rate 加权累加） ──
    if player.has_enchant(YE_PENDANT_CRIT_EFF) {
        let crit_rate = player.current_stats().crit_rate.clamp(0.0, 1.0);
        let crit_prob_1024 = (crit_rate * 1024.0).round() as i32;
        if player.accum_equip_effect(BUFF_YE_PENDANT_CRIT_EFF, crit_prob_1024.max(0), 1024) {
            player.add_buff(BUFF_YE_PENDANT_CRIT_EFF);
        }
    }

    // ── 戒指·破 40802：命中触发 +1928 破防 10s, CD 10s ──
    if player.has_enchant(YE_RING_OVERCOME) {
        let cd_key = "cd_ye_戒指破";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player.active_cds.insert(cd_key.to_string(), t + 10.0);
            player.add_buff(BUFF_YE_RING_OVERCOME);
        }
    }
    // ── 戒指·会 40803：命中触发 +1928 会心 10s, CD 10s ──
    if player.has_enchant(YE_RING_CRIT) {
        let cd_key = "cd_ye_戒指会";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player.active_cds.insert(cd_key.to_string(), t + 10.0);
            player.add_buff(BUFF_YE_RING_CRIT);
        }
    }
    // ── 戒指·招 40804：命中触发 +1928 破招 10s, CD 10s ──
    if player.has_enchant(YE_RING_SURPLUS) {
        let cd_key = "cd_ye_戒指招";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player.active_cds.insert(cd_key.to_string(), t + 10.0);
            player.add_buff(BUFF_YE_RING_SURPLUS);
        }
    }

    // ── 真伤类（emit 名按 skill.txt 查表）──
    // 护手 40788 → SkillID 40789 lvl 1 = "巽"，CD 10s
    if player.has_enchant(YE_WRIST_TRUE_DMG) {
        let cd_key = "cd_ye_护手";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player
                .active_cds
                .insert(cd_key.to_string(), t + YE_WRIST_INNER_CD);
            em.emit("巽", 40789, t);
        }
    }
    // 暗器 38950 → SkillID 38966 lvl 8 = "无修·天"，CD 10s
    if player.has_enchant(YE_DART_TRUE_DMG) {
        let cd_key = "cd_ye_暗器";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player
                .active_cds
                .insert(cd_key.to_string(), t + YE_DART_INNER_CD);
            em.emit("无修·天", 38966, t);
        }
    }
    // 加速腰带 42767 → SkillID 42837 lvl 1 = "速·震"，CD 40s
    if player.has_enchant(YE_BELT_HASTE_TRUE) {
        let cd_key = "cd_ye_加速腰带";
        if player.active_cds.get(cd_key).copied().unwrap_or(0.0) <= t {
            player
                .active_cds
                .insert(cd_key.to_string(), t + YE_BELT_HASTE_INNER_CD);
            em.emit("速·震", 42837, t);
        }
    }
}
