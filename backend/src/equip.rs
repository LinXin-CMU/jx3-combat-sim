// backend/src/equip.rs — 配装器：装备数据加载、搜索、属性计算
//
// 数据源：backend/data/equip/ 下的 6 张 GBK TSV 表
//   Attrib.tab          属性查找表 (ID → slot_name + value)
//   Custom_Armor.tab    防具 (帽子/上衣/腰带/护腕/下装/鞋子)
//   Custom_Trinket.tab  饰品 (项链/腰坠/戒指)
//   Custom_Weapon.tab   武器 (近身/远程)
//   Enchant.tab         附魔 (大附魔+小附魔+五彩石)
//   Set.tab             套装

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════════
// 常量
// ═══════════════════════════════════════════════════════════════════════════════

/// 精炼常量 P(L)，索引 = 精炼等级 0~8
pub const STRENGTH_P: [f64; 9] = [0.0, 0.005, 0.013, 0.024, 0.038, 0.055, 0.075, 0.098, 0.124];

/// 镶嵌系数（重制版·山海源流）
pub fn embedding_coeff(lv: u8) -> f64 {
    if lv == 0 { return 0.0; }
    let li = lv as f64;
    let ratio = 37400.0 / 27800.0; // ≈ 1.3453
    if lv <= 6 {
        0.195 * li * ratio
    } else {
        1.3 * (0.65 * li - 3.2) * ratio
    }
}

/// 属性节点 → 中文标签
pub fn slot_label(slot: &str) -> String {
    match slot {
        "atVitalityBase"                     => "体质",
        "atStrengthBase"                     => "力道",
        "atAgilityBase"                      => "身法",
        "atSpiritBase"                       => "根骨",
        "atSpunkBase"                        => "元气",
        "atPhysicsAttackPowerBase"           => "外功攻击",
        "atMagicAttackPowerBase"             => "内功攻击",
        "atPhysicsCriticalStrike"            => "外功会心",
        "atAllTypeCriticalStrike"            => "全会心",
        "atMagicCriticalStrike"              => "内功会心",
        "atPhysicsCriticalDamagePowerBase"   => "外功会效",
        "atPhysicsOvercomeBase"              => "外功破防",
        "atMagicOvercome"                    => "内功破防",
        "atStrainBase"                       => "无双",
        "atHasteBase"                        => "加速",
        "atSurplusValueBase"                 => "破招",
        "atPhysicsShieldBase"                => "外功防御",
        "atPhysicsShieldAdditional"          => "外功防御",
        "atMagicShield"                      => "内功防御",
        "atParryBase"                        => "招架",
        "atParryValueBase"                   => "拆招",
        "atDodge"                            => "闪避",
        "atToughnessBase"                    => "御劲",
        "atDecriticalDamagePowerBase"        => "化劲",
        "atMaxLifeAdditional"                => "最大气血值",

        // 其他伤害类型攻击/破防/会效/会心
        "atSolarAttackPowerBase"             => "阳性内功攻击",
        "atLunarAttackPowerBase"             => "阴性内功攻击",
        "atNeutralAttackPowerBase"           => "混元内功攻击",
        "atPoisonAttackPowerBase"            => "毒性内功攻击",
        "atSolarAndLunarAttackPowerBase"     => "阴阳内功攻击",
        "atAllTypeAttackPowerBase"           => "全攻击",

        "atSolarOvercomeBase"                => "阳性破防",
        "atLunarOvercomeBase"                => "阴性破防",
        "atNeutralOvercomeBase"              => "混元破防",
        "atPoisonOvercomeBase"               => "毒性破防",
        "atSolarAndLunarOvercomeBase"        => "阴阳破防",
        "atAllTypeOvercomeBase"              => "全破防",

        "atSolarCriticalDamagePowerBase"     => "阳性会心效果",
        "atLunarCriticalDamagePowerBase"     => "阴性会心效果",
        "atNeutralCriticalDamagePowerBase"   => "混元会心效果",
        "atPoisonCriticalDamagePowerBase"    => "毒性会心效果",
        "atMagicCriticalDamagePowerBase"     => "内功会心效果",
        "atSolarAndLunarCriticalDamagePowerBase" => "阴阳会心效果",

        "atSolarCriticalStrike"              => "阳性会心",
        "atLunarCriticalStrike"              => "阴性会心",
        "atNeutralCriticalStrike"            => "混元会心",
        "atPoisonCriticalStrike"             => "毒性会心",

        "atTherapyPowerBase"                 => "治疗量",
        "atDropDefence"                      => "易伤",
        "atManaReplenishExt"                 => "内力恢复",
        "atLifeReplenishExt"                 => "气血恢复",
        "atMoveSpeedPercent"                 => "移动速度",
        "atPVXAllRound"                      => "全能",

        // 马术
        "atAddHorseSprintPowerMax"           => "马力上限",
        "atAddHorseSprintPowerRevive"        => "马力恢复",
        "atAddSprintPowerRevive"             => "疾跑恢复",
        "atAddHorseSprintPowerCost"          => "马力消耗",
        "atAddSprintPowerMax"                => "疾跑上限",
        "atAddSprintPowerCost"               => "疾跑消耗",
        "atSolarAndLunarCriticalStrike"      => "阴阳会心",
        "atAllTypeCriticalDamagePowerBase"   => "全会心效果",
        "atDivingFrameBase"                  => "潜水时间",
        "atTherapyCoefficient"               => "治疗效果",
        "atBeTherapyCoefficient"             => "被治疗效果",
        "atGlobalResistPercent"              => "通用减伤",
        "atModifyCostManaPercent"            => "招式内力消耗",
        "atDamageToLifeForSelf"              => "伤害转化气血",
        "atVitalityBasePercentAdd"           => "体质百分比",
        "atStrengthBasePercentAdd"           => "力道百分比",
        "atAgilityBasePercentAdd"            => "身法百分比",
        "atSpiritBasePercentAdd"             => "根骨百分比",
        "atSpunkBasePercentAdd"              => "元气百分比",
        "atActiveThreatCoefficient"          => "威胁",
        "atMeleeWeaponDamageBase"            => "武器伤害",
        "atMeleeWeaponDamageRand"            => "武器伤害浮动",
        "atMeleeWeaponAttackSpeedBase"       => "武器攻速",
        "atBasePotentialAdd"                 => "全属性",
        "atSkillEventHandler"                => "技能效果",
        "atExecuteScript"                    => "脚本效果",
        "atSetEquipmentRecipe"               => "套装效果",
        other                                => return other.to_string(),
    }.to_string()
}

/// 属性节点 → 筛选标签（用于装备搜索）
fn attr_tag(slot: &str) -> Option<&'static str> {
    match slot {
        "atParryBase" | "atParryValueBase"                       => Some("招架"),
        "atPhysicsCriticalStrike" | "atAllTypeCriticalStrike"    => Some("会心"),
        "atStrainBase"                                           => Some("无双"),
        "atPhysicsOvercomeBase"                                  => Some("破防"),
        "atSurplusValueBase"                                     => Some("破招"),
        "atHasteBase"                                            => Some("加速"),
        "atDodge"                                                => Some("闪避"),
        "atToughnessBase"                                        => Some("御劲"),
        _ => None,
    }
}

/// 装分：位置系数
fn position_score_rate(sub_type: u8) -> f64 {
    match sub_type {
        0 => 1.2, 1 => 0.6, 2 => 1.0, 3 => 0.9, 4 => 0.5,
        5 => 0.5, 6 => 0.7, 7 => 0.5, 8 => 1.0, 9 => 0.7,
        10 => 0.7, _ => 1.0,
    }
}

/// 装分：品质系数
fn quality_score_rate(quality: u8) -> f64 {
    match quality {
        1 => 0.8, 2 => 1.4, 3 => 1.6, 4 => 1.8, 5 => 2.5, _ => 1.0,
    }
}

// 装分计算常数（重制版·山海源流）
const SCORE_A: f64 = 8.8;
const SCORE_B: f64 = 32.0;
const SCORE_C: f64 = 50.0;
const DIAMOND_RATIO: f64 = 37400.0 / 27800.0;

/// 中国式四舍五入（+0.5 后截断，非银行家舍入）
fn round_cn(v: f64) -> i64 {
    (v + 0.5) as i64
}

/// 单颗五行石（镶嵌）分数
fn diamond_score_single(lv: u8) -> f64 {
    if lv == 0 { return 0.0; }
    let li = lv as f64;
    let base = if lv > 6 {
        1.3 * (0.65 * li - 3.2) * SCORE_A * SCORE_B
    } else {
        0.195 * SCORE_A * SCORE_B * li
    };
    base * DIAMOND_RATIO
}

/// 五彩石分数
fn colorful_stone_score(stone_level: u8) -> f64 {
    3.5 * SCORE_A * SCORE_C * stone_level as f64
}

/// 精炼分数（floor(0.5 × base × L × (0.003L + 0.007))）
fn strength_score(base: f64, lv: u8) -> i64 {
    if lv == 0 || base <= 0.0 { return 0; }
    let l = lv as f64;
    (0.5 * base * l * (0.003 * l + 0.007)).floor() as i64
}

/// 面板百分比计算常量（130级 —— 来自游戏实测 / 社区校对）
const LP_CRIT: f64               = 197_703.0;   // 会心等级 / 此 = 会心率
const LP_CRIT_EFF: f64           =  72_844.2;   // 会心效果等级 / 此 = 会心效果 (+ 基础 1.75)
const LP_STRAIN: f64             = 133_333.2;   // 无双等级 / 此 = 无双率
const LP_OVERCOME: f64           = 225_957.6;   // 破防等级 / 此 = 破防率
const LP_HASTE: f64              = 210_078.0;   // 加速等级 / 此 = 加速率
const LP_TOUGHNESS: f64          = 197_703.0;   // 御劲等级 / 此 = 御劲率（线性）
const LP_TOUGHNESS_CRIT_EFF: f64 =  55_123.2;   // 御劲会效等级 / 此 = 御劲会效率（PvP 专属）

// 非线性（level / (level + K) 形式）
const DEFENSE_NONLINEAR: f64     = 126_007.2;   // 外/内防御（130 级）
const PARRY_NONLINEAR: f64       = 107_553.6;   // 招架
const DODGE_NONLINEAR: f64       =  91_634.4;   // 闪避
const DECRIT_NONLINEAR: f64      =  33_046.2;   // 化劲

// 化劲基础率（102/1024 ≈ 9.96%）
const DECRIT_BASE_RATE: f64      = 102.0 / 1024.0;

// 破招伤害系数：基础 = 破招值 × 7.421
pub const SURPLUS_DAMAGE_MULT: f64 = 7.421;

// 全能（atPVXAllRound）：1 点 → 0.5 破招等级 + 1.5 无双等级 + 1 化劲等级
const PVX_TO_SURPLUS: f64 = 0.5;
const PVX_TO_STRAIN:  f64 = 1.5;
const PVX_TO_DECRIT:  f64 = 1.0;

/// 石头名称中中文数字→等级
fn stone_level_from_name(name: &str) -> u8 {
    if name.contains("(陆)") || name.contains("（陆）") { 6 }
    else if name.contains("(伍)") || name.contains("（伍）") { 5 }
    else if name.contains("(肆)") || name.contains("（肆）") { 4 }
    else if name.contains("(叁)") || name.contains("（叁）") { 3 }
    else if name.contains("(贰)") || name.contains("（贰）") { 2 }
    else if name.contains("(壹)") || name.contains("（壹）") { 1 }
    else { 0 }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 数据结构
// ═══════════════════════════════════════════════════════════════════════════════

/// Attrib 查找表类型：ID → (slot_name, value)
pub type AttribTable = HashMap<u32, (String, i64)>;

/// 解析后的魔法属性
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MagicAttr {
    pub slot: String,
    pub label: String,
    pub value: i64,
    /// 装备特效描述（仅 atSkillEventHandler / atExecuteScript 有值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 解析后的镶嵌孔
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiamondSlot {
    pub slot: String,
    pub label: String,
    pub base_value: i64,
}

/// 装备条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquipItem {
    pub id: u32,
    pub name: String,
    pub sub_type: u8,
    pub detail_type: u8,
    pub level: u32,
    pub quality: u8,
    pub max_strength: u8,
    pub require_level: u32,
    pub max_durability: u32,
    pub belong_school: String,
    pub magic_kind: String,
    pub magic_type: String,
    pub set_id: u32,
    pub icon_id: u32,
    pub belong_map: String,

    /// Base1~6: (slot_name, min, max)
    pub bases: Vec<(String, i64, i64)>,
    /// Magic1~16: 已解析的魔法属性
    pub magics: Vec<MagicAttr>,
    /// Diamond1~3: 已解析的镶嵌孔
    pub diamonds: Vec<DiamondSlot>,
    /// 预计算的属性标签，用于筛选
    pub attr_tags: HashSet<String>,
}

/// 附魔/小附魔条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnchantEntry {
    pub id: u32,
    pub name: String,
    pub desc: String,
    pub score: i32,
    pub sub_type: i32,
    pub is_script: bool,
    pub belong_kungfu: u32,
    pub attributes: Vec<(String, i64)>, // (slot_name, value)
    /// 挑战附魔标记：仅首饰部位（项链 4 / 戒指 5 / 腰坠 7 / 暗器 1）有，
    /// 等级远超普通附魔（"白虹" / "暗影·荆岫" 系列），由 load_enchants 按 name 关键词 + sub_type 自动填
    #[serde(default)]
    pub is_challenge: bool,
    /// 英雄附魔标记（"昆仑玄石" 系列）：当前版本标准附魔，等级中高于普通；保留作元数据，
    /// 是否过滤由调用方决定（fit_curve 当前**不**过滤英雄）
    #[serde(default)]
    pub is_heroic: bool,
    /// 物品品质（Other.tab Quality 列）：1=白 / 2=绿 / 3=蓝 / 4=紫 / 5=橙 / 6=红
    /// 由 load_enchants 末尾从 backend/data/equip/enchant_quality.tsv 按 name 反查回填；
    /// 表里没有的（如染绣类无物品形式）保持默认 0（前端按未知处理）。
    #[serde(default)]
    pub quality: u8,
}

/// 五彩石条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoneEntry {
    pub id: u32,
    pub name: String,
    pub level: u8,
    pub attributes: Vec<StoneAttr>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoneAttr {
    pub slot: String,
    pub label: String,
    pub value: i64,
    pub need_count: u32,
    pub need_intensity: u32,
}

/// 单条套装加成
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetBonus {
    pub slot: String,
    pub value: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 套装条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEntry {
    pub id: u32,
    pub name: String,
    /// 件数 → 加成列表
    pub bonuses: BTreeMap<u8, Vec<SetBonus>>,
}

/// 全部装备数据（启动时加载，只读）
///
/// 注意：三张源表（Armor/Trinket/Weapon）ID 空间**有重叠**，同一个 id 在不同表里是不同物品。
/// 所以 items 必须用 (sub_type, id) 复合键，不能只用 id。
pub struct EquipData {
    pub attrib_table: AttribTable,
    /// (sub_type, id) → 装备条目
    pub items: HashMap<(u8, u32), EquipItem>,
    /// sub_type → 该部位所有装备 ID（按品级降序）
    pub items_by_subtype: HashMap<u8, Vec<u32>>,
    /// sub_type → 该位置的小附魔列表
    pub enhances: HashMap<i32, Vec<EnchantEntry>>,
    /// sub_type → 该位置的大附魔列表
    pub enchants: HashMap<i32, Vec<EnchantEntry>>,
    /// 全部五彩石
    pub stones: Vec<StoneEntry>,
    /// 套装 ID → 套装数据
    pub sets: HashMap<u32, SetEntry>,
}

impl EquipData {
    /// 通过 (sub_type, id) 查找装备；sub_type 应来自部位/筛选上下文
    pub fn get_item(&self, sub_type: u8, id: u32) -> Option<&EquipItem> {
        self.items.get(&(sub_type, id))
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// GBK TSV 解析
// ═══════════════════════════════════════════════════════════════════════════════

/// 读取 GBK 编码的 TSV 文件，返回 (表头, 数据行)
fn read_gbk_tsv(path: &Path) -> (Vec<String>, Vec<Vec<String>>) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[equip] 读取失败 {:?}: {}", path, e);
            return (vec![], vec![]);
        }
    };
    let (text, _, _) = encoding_rs::GBK.decode(&bytes);
    let mut lines = text.lines();
    let headers: Vec<String> = match lines.next() {
        Some(h) => h.split('\t').map(|s| s.to_string()).collect(),
        None => return (vec![], vec![]),
    };
    let rows: Vec<Vec<String>> = lines
        .map(|line| line.split('\t').map(|s| s.to_string()).collect())
        .collect();
    (headers, rows)
}

/// 通过列名查找列索引
fn col(headers: &[String], name: &str) -> Option<usize> {
    headers.iter().position(|h| h == name)
}

/// 安全取单元格字符串
fn cell(row: &[String], idx: Option<usize>) -> &str {
    idx.and_then(|i| row.get(i)).map(|s| s.as_str()).unwrap_or("")
}

/// 安全取单元格为 u32
fn cell_u32(row: &[String], idx: Option<usize>) -> u32 {
    cell(row, idx).parse().unwrap_or(0)
}

/// 安全取单元格为 i64
fn cell_i64(row: &[String], idx: Option<usize>) -> i64 {
    cell(row, idx).parse().unwrap_or(0)
}

fn cell_i32(row: &[String], idx: Option<usize>) -> i32 {
    cell(row, idx).parse().unwrap_or(0)
}

fn cell_u8(row: &[String], idx: Option<usize>) -> u8 {
    cell(row, idx).parse().unwrap_or(0)
}

// ═══════════════════════════════════════════════════════════════════════════════
// 加载
// ═══════════════════════════════════════════════════════════════════════════════

/// 主入口：从指定目录加载所有装备数据
pub fn load_equip_data(dir: &Path) -> EquipData {
    let t0 = std::time::Instant::now();

    // 1) 属性查找表
    let attrib_table = load_attrib_table(&dir.join("Attrib.tab"));
    eprintln!("[equip] Attrib: {} 条, {:?}", attrib_table.len(), t0.elapsed());

    // 1.5) 装备特效描述表 & 秘籍描述表（可选）
    let skill_events = load_skill_events(&dir.join("skillevent.txt"));
    eprintln!("[equip] SkillEvent: {} 条, {:?}", skill_events.len(), t0.elapsed());

    let mut skill_recipes = load_skill_recipes(&dir.join("SkillRecipeTable.txt"));
    let eqr = load_skill_recipes(&dir.join("equipmentrecipe.txt"));
    skill_recipes.extend(eqr);
    eprintln!("[equip] SkillRecipe: {} 条, {:?}", skill_recipes.len(), t0.elapsed());

    // 2) 装备 —— 三张表 ID 空间重叠，用 (sub_type, id) 复合键
    let mut items: HashMap<(u8, u32), EquipItem> = HashMap::new();
    for file in &["Custom_Armor.tab", "Custom_Trinket.tab", "Custom_Weapon.tab"] {
        let path = dir.join(file);
        let loaded = load_items_from_table(&path, &attrib_table, &skill_events, &skill_recipes);
        eprintln!("[equip] {}: {} 条, {:?}", file, loaded.len(), t0.elapsed());
        for item in loaded {
            items.insert((item.sub_type, item.id), item);
        }
    }

    // 3) 按部位索引（品级降序）
    let mut items_by_subtype: HashMap<u8, Vec<u32>> = HashMap::new();
    for (&(st, id), _item) in &items {
        items_by_subtype.entry(st).or_default().push(id);
    }
    for (&st, ids) in items_by_subtype.iter_mut() {
        ids.sort_by(|a, b| {
            let la = items[&(st, *a)].level;
            let lb = items[&(st, *b)].level;
            lb.cmp(&la).then(items[&(st, *a)].name.cmp(&items[&(st, *b)].name))
        });
    }

    // 4) 附魔 + 五彩石
    let (enhances, enchants, stones) = load_enchants(&dir.join("Enchant.tab"));
    eprintln!("[equip] Enchant: enhance={}, enchant={}, stones={}, {:?}",
        enhances.values().map(|v| v.len()).sum::<usize>(),
        enchants.values().map(|v| v.len()).sum::<usize>(),
        stones.len(), t0.elapsed());

    // 5) 套装
    let sets = load_sets(&dir.join("Set.tab"), &attrib_table, &skill_events, &skill_recipes);
    eprintln!("[equip] Set: {} 条, {:?}", sets.len(), t0.elapsed());

    eprintln!("[equip] 加载完成: {} 件装备, {:?}", items.len(), t0.elapsed());

    EquipData { attrib_table, items, items_by_subtype, enhances, enchants, stones, sets }
}

/// 加载 SkillRecipeTable.txt：套装秘籍（atSetEquipmentRecipe 指向）
/// 格式：ID Level TypeID SkillID IconID Name Desc BindSkillID ...
/// equipmentrecipe.txt 格式：ID Level Desc IsMobile（Desc 包 <text>text="..."</text>）
fn load_skill_recipes(path: &Path) -> HashMap<u32, String> {
    let (headers, rows) = read_gbk_tsv(path);
    let c_id   = col(&headers, "ID");
    let c_name = col(&headers, "Name");
    let c_desc = col(&headers, "Desc");
    let mut table = HashMap::with_capacity(rows.len());
    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }
        let raw_desc = cell(row, c_desc);
        // equipmentrecipe.txt 的 Desc 是 <text>text="..." font=...</text> 包装；
        // SkillRecipeTable.txt 的 Desc 是纯文本
        let text = if raw_desc.contains("text=\"") {
            extract_text_attr(raw_desc)
        } else {
            raw_desc.to_string()
        };
        let cleaned = clean_escape(text);
        if cleaned.is_empty() { continue; }
        // SkillRecipeTable 有 Name，可以拼在前面；没有时只用 Desc
        let name = cell(row, c_name);
        let final_text = if !name.is_empty() && name != "测试" {
            format!("{}：{}", name, cleaned)
        } else {
            cleaned
        };
        table.insert(id, final_text);
    }
    table
}

/// 清理转义字符和空白
fn clean_escape(text: String) -> String {
    let mut cleaned = text;
    for pat in &["\\\\n", "\\n", "\\r", "\\t"] {
        cleaned = cleaned.replace(pat, " ");
    }
    cleaned = cleaned.replace("\\\"", "\"");
    cleaned = cleaned.replace('\\', "");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 加载装备特效描述表
/// skillevent.txt 格式：ID \t <Text>text="描述" font=xxx </text> \t IsMobile \t Share
fn load_skill_events(path: &Path) -> HashMap<u32, String> {
    let (headers, rows) = read_gbk_tsv(path);
    let c_id   = col(&headers, "ID");
    let c_desc = col(&headers, "Desc");
    let mut table = HashMap::with_capacity(rows.len());
    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }
        let raw = cell(row, c_desc);
        if raw.is_empty() { continue; }
        // 提取 text="..." 之间的内容 + 清洗
        let text = extract_text_attr(raw);
        if !text.is_empty() {
            let cleaned = clean_escape(text);
            if !cleaned.is_empty() {
                table.insert(id, cleaned);
            }
        }
    }
    table
}

/// 从 `<Text>text="..." font=...</text>` 中提取 text 属性值
fn extract_text_attr(raw: &str) -> String {
    let marker = "text=\"";
    let Some(start) = raw.find(marker) else { return String::new(); };
    let body = &raw[start + marker.len()..];
    // 找到下一个未转义的 "
    let mut end = 0;
    let bytes = body.as_bytes();
    let mut prev_backslash = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' && !prev_backslash { end = i; break; }
        prev_backslash = b == b'\\' && !prev_backslash;
    }
    if end == 0 { return String::new(); }
    body[..end].to_string()
}

fn load_attrib_table(path: &Path) -> AttribTable {
    let (headers, rows) = read_gbk_tsv(path);
    let c_id   = col(&headers, "ID");
    let c_type = col(&headers, "ModifyType");
    let c_max  = col(&headers, "Param1Max");
    let c_max2 = col(&headers, "Param2Max");

    let mut table = HashMap::with_capacity(rows.len());
    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }
        let modify_type = cell(row, c_type).to_string();
        if modify_type.is_empty() { continue; }
        let val = cell_i64(row, c_max);
        let val2 = cell_i64(row, c_max2);
        // 取有值的那个
        let final_val = if val != 0 { val } else { val2 };
        table.insert(id, (modify_type, final_val));
    }
    table
}

fn load_items_from_table(
    path: &Path,
    attrib: &AttribTable,
    skill_events: &HashMap<u32, String>,
    skill_recipes: &HashMap<u32, String>,
) -> Vec<EquipItem> {
    let (headers, rows) = read_gbk_tsv(path);
    if headers.is_empty() { return vec![]; }

    let c_id       = col(&headers, "ID");
    let c_name     = col(&headers, "Name");
    let c_sub      = col(&headers, "SubType");
    let c_detail   = col(&headers, "DetailType");
    let c_level    = col(&headers, "Level");
    let c_quality  = col(&headers, "Quality");
    let c_max_str  = col(&headers, "MaxStrengthLevel");
    let c_req_val  = col(&headers, "Require1Value");
    let c_dur      = col(&headers, "MaxDurability");
    let c_school   = col(&headers, "BelongSchool");
    let c_kind     = col(&headers, "MagicKind");
    let c_mtype    = col(&headers, "MagicType");
    let c_set      = col(&headers, "SetID");
    let c_icon     = col(&headers, "UiID");
    let c_map      = col(&headers, "BelongMap");

    // Base1~6
    let base_cols: Vec<(Option<usize>, Option<usize>, Option<usize>)> = (1..=6)
        .map(|i| (
            col(&headers, &format!("Base{}Type", i)),
            col(&headers, &format!("Base{}Min", i)),
            col(&headers, &format!("Base{}Max", i)),
        ))
        .collect();

    // Magic1~16
    let magic_cols: Vec<Option<usize>> = (1..=16)
        .map(|i| col(&headers, &format!("Magic{}Type", i)))
        .collect();

    // Diamond1~3
    let diamond_cols: Vec<Option<usize>> = (1..=3)
        .map(|i| col(&headers, &format!("DiamondAttributeID{}", i)))
        .collect();

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }

        let name = cell(row, c_name).to_string();
        if name.is_empty() { continue; }

        let level = cell_u32(row, c_level);
        let quality = cell_u8(row, c_quality);

        // 解析 base 属性
        let mut bases = Vec::new();
        for (c_type, c_min, c_max) in &base_cols {
            let t = cell(row, *c_type);
            if t.is_empty() || t == "atInvalid" { continue; }
            let mn = cell_i64(row, *c_min);
            let mx = cell_i64(row, *c_max);
            if mn == 0 && mx == 0 { continue; }
            bases.push((t.to_string(), mn, mx));
        }

        // 解析 magic 属性（通过 Attrib 表解引用）
        let mut magics = Vec::new();
        let mut attr_tags = HashSet::new();
        for c in &magic_cols {
            let aid = cell_u32(row, *c);
            if aid == 0 { continue; }
            if let Some((slot, val)) = attrib.get(&aid) {
                let label = slot_label(slot).to_string();
                if let Some(tag) = attr_tag(slot) {
                    attr_tags.insert(tag.to_string());
                }
                // 特效类 slot 查对应表获取描述
                let desc = match slot.as_str() {
                    "atSkillEventHandler" | "atExecuteScript" => skill_events.get(&(*val as u32)).cloned(),
                    "atSetEquipmentRecipe" => skill_recipes.get(&(*val as u32)).cloned(),
                    _ => None,
                };
                magics.push(MagicAttr { slot: slot.clone(), label, value: *val, desc });
            }
        }

        // 解析 diamond 镶嵌孔
        let mut diamonds = Vec::new();
        for c in &diamond_cols {
            let aid = cell_u32(row, *c);
            if aid == 0 { continue; }
            if let Some((slot, val)) = attrib.get(&aid) {
                diamonds.push(DiamondSlot {
                    slot: slot.clone(),
                    label: slot_label(slot).to_string(),
                    base_value: *val,
                });
            }
        }

        let magic_type = cell(row, c_mtype).to_string();
        // magic_type 含"全能"字样的装备额外打标
        if magic_type.contains("全能") {
            attr_tags.insert("全能".to_string());
        }

        items.push(EquipItem {
            id, name,
            sub_type: cell_u8(row, c_sub),
            detail_type: cell_u8(row, c_detail),
            level, quality,
            max_strength: cell_u8(row, c_max_str),
            require_level: cell_u32(row, c_req_val),
            max_durability: cell_u32(row, c_dur),
            belong_school: cell(row, c_school).to_string(),
            magic_kind: cell(row, c_kind).to_string(),
            magic_type,
            set_id: cell_u32(row, c_set),
            icon_id: cell_u32(row, c_icon),
            belong_map: cell(row, c_map).to_string(),
            bases, magics, diamonds, attr_tags,
        });
    }
    items
}

/// 给 EnchantEntry 填挑战附魔 / 英雄附魔标记。
///
/// **数据层 helper**——load_enchants 解析 .tab 时调用一次；load_from_processed 加载旧 JSON 缓存
/// 后也调用一次（旧缓存可能缺这俩字段，serde default=false，需要补填）。
///
/// 首饰部位（项链=4 / 戒指=5 / 腰坠=7 / 暗器=1）的高品级特殊附魔：
///   - **挑战附魔**（sub_type ∈ {1,4,5,7} 才有）：
///       · "X·白虹" 系列：山海·白虹 / 暗影·白虹 / 后续赛季同模式命名
///       · "白虹贯岩" 系列（旧赛季变体）
///       · "X·荆岫" 系列：暗影·荆岫（2026.04）/ 后续赛季同模式命名
///       · "荆岫璞玉" 系列
///     数值远超普通附魔（如 "暗影·白虹·腰坠（会心）" 的会心 5739，是普通附魔 ~1300 的 4 倍多）。
///     属性收益分析时排除避免污染 max。
///   - **英雄附魔**："昆仑玄石" 系列（当前版本标准 PVE 附魔，元数据保留但不强制过滤）
///
/// Enchant.tab 表头无 Quality / Color 字段（已确认），靠 name 关键词识别。
/// 新增挑战附魔系列时只改这里的关键词列表 —— 上层（fit_curve / wzc）按字段过滤即可。
pub fn enrich_enhance_flags(entry: &mut EnchantEntry) {
    let is_jewelry_slot = matches!(entry.sub_type, 1 | 4 | 5 | 7);
    entry.is_challenge = is_jewelry_slot
        && (entry.name.contains("·白虹")        // 覆盖 山海·白虹 / 暗影·白虹 / 后续 X·白虹
            || entry.name.contains("白虹贯岩")
            || entry.name.contains("·荆岫")     // 覆盖 暗影·荆岫 / 后续 X·荆岫
            || entry.name.contains("荆岫璞玉"));
    entry.is_heroic = entry.name.contains("昆仑玄石");
}

/// 从 backend/data/equip/enchant_quality.tsv（UTF-8）加载 name → quality 映射。
/// 离线由 Other.tab 的 ScriptName=`附魔道具使用.lua` 物品 (Name, Quality) 抽取去重生成；
/// Enchant.tab 自身不含 Quality 字段，必须靠这张外表查。
fn load_enchant_quality(dir: &Path) -> HashMap<String, u8> {
    let path = dir.join("enchant_quality.tsv");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut map = HashMap::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        let name = match parts.next() { Some(s) if !s.is_empty() => s.to_string(), _ => continue };
        let q: u8 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        map.insert(name, q);
    }
    map
}

fn load_enchants(path: &Path) -> (HashMap<i32, Vec<EnchantEntry>>, HashMap<i32, Vec<EnchantEntry>>, Vec<StoneEntry>) {
    let (headers, rows) = read_gbk_tsv(path);
    if headers.is_empty() { return (HashMap::new(), HashMap::new(), vec![]); }

    let c_id       = col(&headers, "ID");
    let c_name     = col(&headers, "Name");
    let c_desc     = col(&headers, "AttriName");
    let c_score    = col(&headers, "Score");
    let c_sub      = col(&headers, "DestItemSubType");
    let c_tab      = col(&headers, "TabType");
    let c_kungfu   = col(&headers, "BelongKungfuID");

    // Attribute1~4
    let attr_cols: Vec<(Option<usize>, Option<usize>, Option<usize>)> = (1..=4)
        .map(|i| {
            let id_col = col(&headers, &format!("Attribute{}ID", i));
            let v1_col = col(&headers, &format!("Attribute{}Value1", i));
            let v2_col = col(&headers, &format!("Attribute{}Value2", i));
            (id_col, v1_col, v2_col)
        })
        .collect();

    // Diamond conditions (for stones)
    let diamond_cond_cols: Vec<(Option<usize>, Option<usize>)> = (1..=3)
        .map(|i| (
            col(&headers, &format!("DiamondCount{}", i)),
            col(&headers, &format!("DiamondIntensity{}", i)),
        ))
        .collect();

    let mut enhances: HashMap<i32, Vec<EnchantEntry>> = HashMap::new();
    let mut enchants_map: HashMap<i32, Vec<EnchantEntry>> = HashMap::new();
    let mut stones: Vec<StoneEntry> = Vec::new();

    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }

        let name = cell(row, c_name).to_string();
        if name.is_empty() { continue; }

        let tab_type = cell(row, c_tab).trim();
        let sub_type = cell_i32(row, c_sub);

        // 解析属性
        let mut attributes: Vec<(String, i64)> = Vec::new();
        let mut is_script = false;
        for (c_aid, c_v1, c_v2) in &attr_cols {
            let slot = cell(row, *c_aid).to_string();
            if slot.is_empty() { continue; }
            if slot == "atSkillEventHandler" || slot == "atExecuteScript" {
                is_script = true;
                let v1 = cell_i64(row, *c_v1);
                attributes.push((slot, v1));
            } else {
                let v1 = cell_i64(row, *c_v1);
                let v2 = cell_i64(row, *c_v2);
                let val = v1.max(v2);
                if val != 0 {
                    attributes.push((slot, val));
                }
            }
        }

        if tab_type == "5" {
            // ── 五彩石 ──
            // 马饰挂件也混在 TabType=5 里，按名字筛选：只保留以「彩·」开头的条目
            if !name.starts_with("彩·") { continue; }
            let level = stone_level_from_name(&name);
            let mut stone_attrs = Vec::new();
            for (i, (c_aid, c_v1, c_v2)) in attr_cols.iter().enumerate() {
                let slot = cell(row, *c_aid).to_string();
                if slot.is_empty() { continue; }
                if slot == "atSkillEventHandler" || slot == "atExecuteScript" { continue; }
                let v1 = cell_i64(row, *c_v1);
                let v2 = cell_i64(row, *c_v2);
                let val = v1.max(v2);
                let (need_count, need_intensity) = if i < diamond_cond_cols.len() {
                    (cell_u32(row, diamond_cond_cols[i].0), cell_u32(row, diamond_cond_cols[i].1))
                } else {
                    (0, 0)
                };
                stone_attrs.push(StoneAttr {
                    label: slot_label(&slot).to_string(),
                    slot, value: val, need_count, need_intensity,
                });
            }
            if !stone_attrs.is_empty() {
                stones.push(StoneEntry { id, name, level, attributes: stone_attrs });
            }
        } else {
            // ── 大附魔 / 小附魔 ──
            let mut entry = EnchantEntry {
                id, name,
                desc: cell(row, c_desc).to_string(),
                score: cell_i32(row, c_score),
                sub_type,
                is_script,
                belong_kungfu: cell_u32(row, c_kungfu),
                attributes,
                is_challenge: false,
                is_heroic: false,
                quality: 0,
            };
            enrich_enhance_flags(&mut entry);
            if is_script {
                enchants_map.entry(sub_type).or_default().push(entry);
            } else {
                enhances.entry(sub_type).or_default().push(entry);
            }
        }
    }

    // 回填物品品质：从同目录下 enchant_quality.tsv 按 name 查表
    if let Some(dir) = path.parent() {
        let quality_map = load_enchant_quality(dir);
        if !quality_map.is_empty() {
            let mut hits = 0usize;
            for v in enhances.values_mut().chain(enchants_map.values_mut()) {
                for e in v.iter_mut() {
                    if let Some(&q) = quality_map.get(&e.name) {
                        e.quality = q;
                        hits += 1;
                    }
                }
            }
            eprintln!("[equip] enchant_quality.tsv: {} 条映射，命中 {} 个 EnchantEntry", quality_map.len(), hits);
        }
    }

    // 大附魔 fallback：所有大附魔（is_script=true）历史上都是紫品（Q=4）。
    // 旧赛季（昆吾焰晶/砂/珀/珩 等）在 Other.tab 已被官方清空 Quality=0，但 Enchant.tab 仍保留，
    // 这里给个默认值避免 UI 显示无色。
    let mut defaulted = 0usize;
    for v in enchants_map.values_mut() {
        for e in v.iter_mut() {
            if e.quality == 0 && e.is_script {
                e.quality = 4;
                defaulted += 1;
            }
        }
    }
    if defaulted > 0 {
        eprintln!("[equip] 大附魔 fallback Q=4: {} 个 (Other.tab 已下架物品)", defaulted);
    }

    (enhances, enchants_map, stones)
}

fn load_sets(
    path: &Path,
    attrib: &AttribTable,
    skill_events: &HashMap<u32, String>,
    skill_recipes: &HashMap<u32, String>,
) -> HashMap<u32, SetEntry> {
    let (headers, rows) = read_gbk_tsv(path);
    if headers.is_empty() { return HashMap::new(); }

    let c_id   = col(&headers, "ID");
    let c_name = col(&headers, "Name");

    // 2_1 ~ 24_4 列索引
    let mut bonus_cols: BTreeMap<u8, Vec<Option<usize>>> = BTreeMap::new();
    for n in 2..=24u8 {
        let cols: Vec<Option<usize>> = (1..=4)
            .map(|i| col(&headers, &format!("{}_{}", n, i)))
            .collect();
        bonus_cols.insert(n, cols);
    }

    let mut sets = HashMap::new();
    for row in &rows {
        let id = cell_u32(row, c_id);
        if id == 0 { continue; }
        let name = cell(row, c_name).to_string();

        let mut bonuses: BTreeMap<u8, Vec<SetBonus>> = BTreeMap::new();
        for (&n, cols) in &bonus_cols {
            let mut attrs = Vec::new();
            for c in cols {
                let aid = cell_u32(row, *c);
                if aid == 0 { continue; }
                if let Some((slot, val)) = attrib.get(&aid) {
                    let desc = match slot.as_str() {
                        "atSkillEventHandler" | "atExecuteScript" => skill_events.get(&(*val as u32)).cloned(),
                        "atSetEquipmentRecipe" => skill_recipes.get(&(*val as u32)).cloned(),
                        _ => None,
                    };
                    attrs.push(SetBonus { slot: slot.clone(), value: *val, desc });
                }
            }
            if !attrs.is_empty() {
                bonuses.insert(n, attrs);
            }
        }
        if !bonuses.is_empty() {
            sets.insert(id, SetEntry { id, name, bonuses });
        }
    }
    sets
}

// ═══════════════════════════════════════════════════════════════════════════════
// API 类型
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct SearchFilter {
    pub sub_type: u8,
    #[serde(default)]
    pub min_level: u32,
    #[serde(default)]
    pub max_level: u32,
    #[serde(default)]
    pub schools: Vec<String>,       // 通用, 精简, 苍云, ...
    #[serde(default)]
    pub kinds: Vec<String>,         // 防御, 力道, 身法, 外功, ...
    #[serde(default)]
    pub attrs: Vec<String>,         // 招架, 会心, 无双, ...
    #[serde(default)]
    pub keyword: String,            // 名称搜索
    #[serde(default)]
    pub battle_types: Vec<String>,  // PVE, PVP, PVX
    /// 装备类型分类（OR 关系：任一匹配即过）。可选值：
    /// 无修 / 散件 / 切糕 / 普通精简 / 黄字精简 / 橙武 / 紫武 / 橙坠 / 紫坠
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Serialize)]
pub struct EquipListItem {
    pub id: u32,
    pub name: String,
    pub level: u32,
    pub quality: u8,
    pub max_strength: u8,
    pub magic_type: String,
    pub belong_school: String,
    pub magic_kind: String,
    /// 套装 ID（0 = 无套装）。前端"只看套装件"等过滤要用。
    pub set_id: u32,
    pub set_name: Option<String>,
    pub attr_tags: Vec<String>,
}

#[derive(Serialize)]
pub struct EquipDetailResp {
    pub id: u32,
    pub name: String,
    pub sub_type: u8,
    pub detail_type: u8,
    pub level: u32,
    pub quality: u8,
    pub max_strength: u8,
    pub require_level: u32,
    pub max_durability: u32,
    pub belong_school: String,
    pub magic_kind: String,
    pub magic_type: String,
    pub set_id: u32,
    pub set_name: Option<String>,
    pub belong_map: String,
    pub bases: Vec<BaseAttrResp>,
    pub magics: Vec<MagicAttr>,
    pub diamonds: Vec<DiamondSlot>,
    pub set_bonuses: Option<SetBonusResp>,
}

#[derive(Serialize)]
pub struct BaseAttrResp {
    pub slot: String,
    pub label: String,
    pub value: i64,
}

#[derive(Serialize)]
pub struct SetBonusResp {
    pub name: String,
    /// [(件数, [(label, value, desc?)])]
    pub tiers: Vec<(u8, Vec<SetBonusResp1>)>,
}

#[derive(Serialize)]
pub struct SetBonusResp1 {
    pub label: String,
    pub value: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

#[derive(Serialize)]
pub struct EnchantResp {
    pub id: u32,
    pub name: String,
    pub desc: String,
    pub score: i32,
    pub is_script: bool,
    pub attributes: Vec<(String, String, i64)>, // (slot, label, value)
    /// 物品品质（Other.tab Quality）：1=白 / 2=绿 / 3=蓝 / 4=紫；前端按此着色
    pub quality: u8,
    /// 挑战附魔（首饰白虹/荆岫系列），前端覆盖为橙色
    pub is_challenge: bool,
}

#[derive(Serialize)]
pub struct StoneResp {
    pub id: u32,
    pub name: String,
    pub level: u8,
    pub attributes: Vec<StoneAttrResp>,
}

#[derive(Serialize)]
pub struct StoneAttrResp {
    pub slot: String,
    pub label: String,
    pub value: i64,
    pub need_count: u32,
    pub need_intensity: u32,
}

// ─── 属性计算请求/响应 ───

#[derive(Debug, Deserialize, Clone)]
pub struct SlotConfig {
    pub equip_id: u32,
    #[serde(default = "default_strength")]
    pub strength: u8,
    #[serde(default)]
    pub embedding: Vec<u8>,     // 每孔镶嵌等级
    #[serde(default)]
    pub enhance_id: u32,        // 小附魔 ID
    #[serde(default)]
    pub enchant_id: u32,        // 大附魔 ID
}

fn default_strength() -> u8 { 6 }

#[derive(Deserialize, Clone)]
pub struct CalcRequest {
    pub slots: HashMap<String, SlotConfig>,
    #[serde(default)]
    pub stone_id: u32,
    #[serde(default = "default_mount")]
    pub mount: u32,             // 10390=分山劲, 10389=铁骨衣
    #[serde(default)]
    pub talents: Vec<u32>,      // 奇穴 ID（影响面板属性的被动奇穴）
}

fn default_mount() -> u32 { 10390 }

#[derive(Serialize)]
pub struct CalcResponse {
    /// 装分（S_score + ΔS_score）
    pub score: i64,
    /// 品质等级（S_quality + ΔS_quality，仅精炼影响）
    pub quality_level: i64,
    /// 原始属性集合（可直接填入基础设置）
    pub raw: RawAttrs,
    /// 面板显示值（含心法转化）
    pub panel: PanelAttrs,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct RawAttrs {
    pub vitality: f64,
    pub strength: f64,
    pub agility: f64,
    pub spirit: f64,
    pub spunk: f64,
    pub base_attack: f64,
    pub base_magical_attack: f64,
    pub weapon_damage: f64,
    pub weapon_damage_rand: f64,
    pub surplus_value: f64,
    pub crit_level: f64,
    pub crit_effect_level: f64,
    pub overcome_level: f64,
    pub strain_level: f64,
    pub haste_level: f64,
    pub parry_level: f64,
    pub parry_value: f64,
    pub dodge_level: f64,
    pub toughness_level: f64,
    pub decritical_damage_level: f64,
    pub physics_shield: f64,
    pub magic_shield: f64,
    pub threat: f64,
    pub weapon_speed: f64,
    /// 全能 (atPVXAllRound) —— 展开前的原始值；每点 = 0.5破招 + 1.5无双 + 1化劲
    pub pvx_all_round: f64,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct PanelAttrs {
    pub physics_attack_power: f64,
    pub crit_rate: f64,
    pub crit_effect: f64,
    pub overcome_rate: f64,
    pub strain_rate: f64,
    pub haste_rate: f64,
    pub surplus_value: f64,
    pub physics_shield_rate: f64,
    pub magic_shield_rate: f64,
    pub parry_rate: f64,
    pub parry_value: f64,
    pub dodge_rate: f64,
    pub toughness_rate: f64,
    pub decritical_damage_rate: f64,
    /// 最大气血 = floor(最终体质 × (10 + mount.vitality_to_hp)) + atMaxLifeAdditional 累加
    pub max_life: f64,
    /// 主属性最终值（用于 wzc 结果卡按心法显示对应主属性）
    pub agility: f64,
    pub strength: f64,
    pub vitality: f64,
}

// ═══════════════════════════════════════════════════════════════════════════════
// 搜索
// ═══════════════════════════════════════════════════════════════════════════════

/// 装备搜索（按部位 + 筛选条件）
pub fn search_items(data: &EquipData, filter: &SearchFilter) -> Vec<EquipListItem> {
    let ids = match data.items_by_subtype.get(&filter.sub_type) {
        Some(ids) => ids,
        None => return vec![],
    };

    let schools: HashSet<&str> = filter.schools.iter().map(|s| s.as_str()).collect();
    let kinds: HashSet<&str> = filter.kinds.iter().map(|s| s.as_str()).collect();
    let need_attrs: HashSet<&str> = filter.attrs.iter().map(|s| s.as_str()).collect();
    let keyword = filter.keyword.trim();
    let battle_types: HashSet<&str> = filter.battle_types.iter().map(|s| s.as_str()).collect();

    ids.iter()
        .filter_map(|id| {
            let item = data.items.get(&(filter.sub_type, *id))?;

            // 品级范围
            if filter.min_level > 0 && item.level < filter.min_level { return None; }
            if filter.max_level > 0 && item.level > filter.max_level { return None; }

            // 门派筛选
            if !schools.is_empty() && !schools.contains(item.belong_school.as_str()) {
                return None;
            }

            // 心法/类型筛选
            if !kinds.is_empty() && !kinds.contains(item.magic_kind.as_str()) {
                return None;
            }

            // 属性标签筛选（全部要求的标签都必须存在）
            for tag in &need_attrs {
                if !item.attr_tags.contains(*tag) {
                    return None;
                }
            }

            // 名称搜索
            if !keyword.is_empty() && !item.name.contains(keyword) && !item.magic_type.contains(keyword) {
                return None;
            }

            // 战斗类型筛选（magic_type 含 "(PVP)" → PVP；含 "(PVX)" → PVX；否则 PVE）
            if !battle_types.is_empty() {
                let is_pvp = item.magic_type.contains("(PVP)");
                let is_pvx = item.magic_type.contains("(PVX)");
                let tag = if is_pvp { "PVP" } else if is_pvx { "PVX" } else { "PVE" };
                if !battle_types.contains(tag) {
                    return None;
                }
            }

            let set_name = if item.set_id > 0 {
                data.sets.get(&item.set_id).map(|s| s.name.clone())
            } else { None };

            // 装备类型分类（OR：任一匹配即通过；空 = 不过滤）
            if !filter.categories.is_empty() {
                let is_jianjian = item.belong_school == "精简";
                let has_yellow = item.magic_type.contains("黄字");
                let has_wuxiu = item.name.contains("无修");
                let is_san = item.set_id == 0;
                let is_qiegao = set_name.as_deref().map(|n| n.contains("切糕")).unwrap_or(false);
                let is_weapon = matches!(item.sub_type, 0 | 1);
                let is_pendant = item.sub_type == 7;
                let q = item.quality;
                let matched = filter.categories.iter().any(|c| match c.as_str() {
                    "无修"     => has_wuxiu,
                    "散件"     => is_san,
                    "切糕"     => is_qiegao,
                    // 精简两类排除无修（无修在数据里 school 也是"精简"，但用户语义里独立）
                    "普通精简" => is_jianjian && !has_yellow && !has_wuxiu,
                    "黄字精简" => is_jianjian && has_yellow && !has_wuxiu,
                    "橙武"     => is_weapon && q == 5,
                    "紫武"     => is_weapon && q == 4,
                    "橙坠"     => is_pendant && q == 5,
                    "紫坠"     => is_pendant && q == 4,
                    _ => false,
                });
                if !matched { return None; }
            }

            Some(EquipListItem {
                id: item.id,
                name: item.name.clone(),
                level: item.level,
                quality: item.quality,
                max_strength: item.max_strength,
                magic_type: item.magic_type.clone(),
                belong_school: item.belong_school.clone(),
                magic_kind: item.magic_kind.clone(),
                set_id: item.set_id,
                set_name,
                attr_tags: item.attr_tags.iter().cloned().collect(),
            })
        })
        .collect()
}

/// 装备详情（需要 sub_type 定位，因为 id 跨表可能重复）
pub fn get_detail(data: &EquipData, sub_type: u8, id: u32) -> Option<EquipDetailResp> {
    let item = data.items.get(&(sub_type, id))?;

    let set_name = if item.set_id > 0 {
        data.sets.get(&item.set_id).map(|s| s.name.clone())
    } else { None };

    let set_bonuses = if item.set_id > 0 {
        data.sets.get(&item.set_id).map(|s| SetBonusResp {
            name: s.name.clone(),
            tiers: s.bonuses.iter().map(|(&n, attrs)| {
                (n, attrs.iter().map(|b| SetBonusResp1 {
                    label: slot_label(&b.slot),
                    value: b.value,
                    desc: b.desc.clone(),
                }).collect())
            }).collect(),
        })
    } else { None };

    let bases = item.bases.iter().map(|(slot, _min, max)| BaseAttrResp {
        slot: slot.clone(),
        label: slot_label(slot).to_string(),
        value: *max,
    }).collect();

    Some(EquipDetailResp {
        id: item.id,
        name: item.name.clone(),
        sub_type: item.sub_type,
        detail_type: item.detail_type,
        level: item.level,
        quality: item.quality,
        max_strength: item.max_strength,
        require_level: item.require_level,
        max_durability: item.max_durability,
        belong_school: item.belong_school.clone(),
        magic_kind: item.magic_kind.clone(),
        magic_type: item.magic_type.clone(),
        set_id: item.set_id,
        set_name,
        belong_map: item.belong_map.clone(),
        bases,
        magics: item.magics.clone(),
        diamonds: item.diamonds.clone(),
        set_bonuses,
    })
}

/// 获取指定部位的小附魔
pub fn get_enhances(data: &EquipData, sub_type: i32) -> Vec<EnchantResp> {
    data.enhances.get(&sub_type).map(|list| {
        list.iter().map(|e| EnchantResp {
            id: e.id, name: e.name.clone(), desc: e.desc.clone(),
            score: e.score, is_script: e.is_script,
            attributes: e.attributes.iter().map(|(s, v)| {
                (s.clone(), slot_label(s).to_string(), *v)
            }).collect(),
            quality: e.quality,
            is_challenge: e.is_challenge,
        }).collect()
    }).unwrap_or_default()
}

/// 获取指定部位的大附魔
pub fn get_enchants_for(data: &EquipData, sub_type: i32) -> Vec<EnchantResp> {
    data.enchants.get(&sub_type).map(|list| {
        list.iter().map(|e| EnchantResp {
            id: e.id, name: e.name.clone(), desc: e.desc.clone(),
            score: e.score, is_script: e.is_script,
            attributes: e.attributes.iter().map(|(s, v)| {
                (s.clone(), slot_label(s).to_string(), *v)
            }).collect(),
            quality: e.quality,
            is_challenge: e.is_challenge,
        }).collect()
    }).unwrap_or_default()
}

/// 获取五彩石列表（可选按属性关键字筛选）
pub fn get_stones(data: &EquipData, selectors: &[String]) -> Vec<StoneResp> {
    data.stones.iter()
        .filter(|s| {
            if selectors.is_empty() { return true; }
            selectors.iter().all(|sel| {
                if sel.is_empty() || sel == ".." { return true; }
                // 检查石头名称或属性标签是否包含筛选词
                s.name.contains(sel.as_str()) ||
                s.attributes.iter().any(|a| a.label.contains(sel.as_str()))
            })
        })
        .map(|s| StoneResp {
            id: s.id, name: s.name.clone(), level: s.level,
            attributes: s.attributes.iter().map(|a| StoneAttrResp {
                slot: a.slot.clone(), label: a.label.clone(),
                value: a.value, need_count: a.need_count,
                need_intensity: a.need_intensity,
            }).collect(),
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 属性计算
// ═══════════════════════════════════════════════════════════════════════════════

/// 位置字符串 → SubType 映射
pub fn pos_to_subtype(pos: &str) -> u8 {
    match pos {
        "HAT"              => 3,
        "JACKET"           => 2,
        "BELT"             => 6,
        "WRIST"            => 10,
        "BOTTOMS"          => 8,
        "SHOES"            => 9,
        "NECKLACE"         => 4,
        "PENDANT"          => 7,
        "RING_1" | "RING_2"=> 5,
        "PRIMARY_WEAPON"   => 0,
        "SECONDARY_WEAPON" => 1,
        _ => 255,
    }
}

/// 属性汇总表
struct AttrAccum {
    map: HashMap<String, f64>,
}

impl AttrAccum {
    fn new() -> Self { Self { map: HashMap::new() } }

    fn add(&mut self, slot: &str, val: f64) {
        *self.map.entry(slot.to_string()).or_default() += val;
    }

    fn get(&self, slot: &str) -> f64 {
        self.map.get(slot).copied().unwrap_or(0.0)
    }
}

/// 心法固定增益（穿戴心法本身就有的属性）。从 school.toml 的 [base_stats] 加载，
/// 字段命名对应 atXxx slot；缺省 = 0。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MountBaseStats {
    #[serde(default)] pub physics_attack_power: f64,   // atPhysicsAttackPowerBase
    #[serde(default)] pub physics_overcome:     f64,   // atPhysicsOvercomeBase
    #[serde(default)] pub vitality:             f64,   // atVitalityBase
    #[serde(default)] pub agility:              f64,   // atAgilityBase
    #[serde(default)] pub strength:             f64,   // atStrengthBase
    #[serde(default)] pub parry:                f64,   // atParryBase
    #[serde(default)] pub parry_value:          f64,   // atParryValueBase
    #[serde(default)] pub physics_shield:       f64,   // atPhysicsShieldBase
    #[serde(default)] pub magic_shield:         f64,   // atMagicShield
}

/// 心法转化（mount-specific）：作用在 **最终主属性** 上的额外副属性产出。
/// 与"系统固定转化"相对：后者全角色通用、属于游戏底层规则；本结构每心法独立。
/// 转化结果直接累加到对应副属性，**不再** 进入攻击百分比 / 等级压制 等乘性增益。
/// 从 school.toml 的 [mount_conversions] 加载，缺省全 0。
///
/// **字段以郭氏值 (/1024) 存储**（贴近游戏内部定点表示），apply 时 `(primary × value / 1024).floor()`，每项单独 floor。
/// 唯一例外：`vitality_to_hp` 是直接小数（+N HP/体质，不走 /1024）。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MountConversions {
    /// 身法 → 外功攻击（郭氏 /1024）。分山 1925 ≈ 1.88
    #[serde(default)] pub agility_to_attack:       f64,
    /// 身法 → 招架等级（郭氏 /1024）。分山 113 ≈ 0.11
    #[serde(default)] pub agility_to_parry:        f64,
    /// 身法 → 拆招值（郭氏 /1024）。分山 1024 = 1.0
    #[serde(default)] pub agility_to_parry_value:  f64,
    /// 体质 → 外功攻击（郭氏 /1024）。铁骨 41 ≈ 0.04
    #[serde(default)] pub vitality_to_attack:      f64,
    /// 体质 → 招架等级（郭氏 /1024）。铁骨 184 ≈ 0.18
    #[serde(default)] pub vitality_to_parry:       f64,
    /// 体质 → 拆招值（郭氏 /1024）。铁骨 2304 = 2.25
    #[serde(default)] pub vitality_to_parry_value: f64,
    /// 体质 → 气血额外加成（**直接小数**，不是郭氏）。铁骨 = 2.2；总系数 = SYS_VITALITY_TO_HP (10) + 此项
    #[serde(default)] pub vitality_to_hp:          f64,
}

// ─── 系统固定转化（全角色通用，不属于任何心法。游戏底层规则）───────────────
//   身法  → 会心等级
//   力道  → 外功攻击 / 外功破防
// 数值是 130 级面板的固定系数，不需要也不应该写进 school.toml
pub const SYS_AGILITY_TO_CRIT:      f64 = 0.9;
pub const SYS_STRENGTH_TO_ATTACK:   f64 = 0.163;
pub const SYS_STRENGTH_TO_OVERCOME: f64 = 0.3;

// ─── 全角色基础属性（130 级人物默认值 + 系统给所有角色的基础防御）───────────
// 不属于任何心法，所有角色一律有这些；mount 自带的额外加成在 [base_stats] 里给。
pub const PLAYER_BASE_VITALITY:        f64 = 45.0;    // 体质
pub const PLAYER_BASE_STRENGTH:        f64 = 44.0;    // 力道
pub const PLAYER_BASE_AGILITY:         f64 = 44.0;    // 身法
pub const PLAYER_BASE_SPIRIT:          f64 = 44.0;    // 根骨
pub const PLAYER_BASE_SPUNK:           f64 = 44.0;    // 元气
pub const PLAYER_BASE_PHYSICS_SHIELD:  f64 = 2850.0;  // 外功防御
pub const PLAYER_BASE_MAGIC_SHIELD:    f64 = 2850.0;  // 内功防御
pub const PLAYER_BASE_MAX_LIFE:        f64 = 199476.0; // 全心法基础气血值（与体质/心法无关）

/// 系统通用体质 → 气血系数（每点体质 +10 气血，所有心法都享有）
pub const SYS_VITALITY_TO_HP:          f64 = 10.0;

/// 计算最终属性
pub fn calculate(
    data: &EquipData,
    req: &CalcRequest,
    base_stats: &MountBaseStats,
    conversions: &MountConversions,
) -> CalcResponse {
    let mut acc = AttrAccum::new();
    let mut total_score: i64 = 0;
    let mut total_quality: i64 = 0;
    let mut quality_count: u32 = 0;
    let mut diamond_count: u32 = 0;
    let mut diamond_level: u32 = 0;

    // 获取五彩石等级（用于主武器装分）
    let stone_level = if req.stone_id > 0 {
        data.stones.iter().find(|s| s.id == req.stone_id).map(|s| s.level).unwrap_or(0)
    } else { 0 };

    for (pos, cfg) in &req.slots {
        let sub_type = pos_to_subtype(pos);
        let item = match data.items.get(&(sub_type, cfg.equip_id)) {
            Some(i) => i,
            None => continue,
        };

        // ── 装分 ──
        // S_quality：原始品质等级 = item.level
        // S_score：原始装备分数 = round(level × quality_rate × position_rate)
        let s_quality = item.level as f64;
        let s_score = (item.level as f64 * quality_score_rate(item.quality) * position_score_rate(sub_type)).round();

        // 精炼加成
        let v_strength_quality = strength_score(s_quality, cfg.strength);
        let v_strength_score = strength_score(s_score, cfg.strength);

        // 五行石（镶嵌）分数：三颗累加
        let mut v_diamond: f64 = 0.0;
        for i in 0..item.diamonds.len().min(3) {
            let lv = cfg.embedding.get(i).copied().unwrap_or(0);
            v_diamond += diamond_score_single(lv);
        }

        // 五彩石（仅主武器 sub_type=0）
        let v_colorful = if sub_type == 0 && stone_level > 0 {
            colorful_stone_score(stone_level)
        } else { 0.0 };

        // 附魔分数（来自 Enchant.tab 的 Score 字段）
        let mut v_enchant_score: i64 = 0;
        if cfg.enhance_id > 0 {
            if let Some(list) = data.enhances.get(&(sub_type as i32)) {
                if let Some(e) = list.iter().find(|x| x.id == cfg.enhance_id) {
                    v_enchant_score += e.score as i64;
                }
            }
        }
        if cfg.enchant_id > 0 {
            if let Some(list) = data.enchants.get(&(sub_type as i32)) {
                if let Some(e) = list.iter().find(|x| x.id == cfg.enchant_id) {
                    v_enchant_score += e.score as i64;
                }
            }
        }

        // 品质等级总加成（仅精炼）
        total_quality += s_quality as i64 + v_strength_quality;
        quality_count += 1;

        // 装备分数总加成
        // ΔS_score = Round(V_diamond + V_colorful + V_strength^score + V_enchant)
        let advance = round_cn(v_diamond + v_colorful + v_strength_score as f64 + v_enchant_score as f64);
        total_score += s_score as i64 + advance;

        // ── 基础属性 ──
        for (slot, _min, max) in &item.bases {
            acc.add(slot, *max as f64);
        }

        // ── 魔法属性 + 精炼 ──
        for ma in &item.magics {
            acc.add(&ma.slot, ma.value as f64);
            // 精炼加成（威胁也吃精炼；仅脚本/特效类 slot 不参与）
            if cfg.strength > 0
                && ma.slot != "atSkillEventHandler" && ma.slot != "atExecuteScript"
                && ma.slot != "atSetEquipmentRecipe"
            {
                let lv = cfg.strength.min(8) as usize;
                let bonus = (ma.value as f64 * STRENGTH_P[lv]).round();
                acc.add(&ma.slot, bonus);
            }
        }

        // ── 镶嵌 ──
        for (i, ds) in item.diamonds.iter().enumerate() {
            let embed_lv = cfg.embedding.get(i).copied().unwrap_or(0);
            if embed_lv > 0 {
                let coeff = embedding_coeff(embed_lv);
                let val = ((ds.base_value as f64).floor() * coeff).floor();
                acc.add(&ds.slot, val);
                diamond_count += 1;
                diamond_level += embed_lv as u32;
            }
        }

        // ── 小附魔 ──
        if cfg.enhance_id > 0 {
            if let Some(list) = data.enhances.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enhance_id) {
                    for (slot, val) in &ench.attributes {
                        acc.add(slot, *val as f64);
                    }
                }
            }
        }

        // ── 大附魔（仅直接属性型） ──
        if cfg.enchant_id > 0 {
            if let Some(list) = data.enchants.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enchant_id) {
                    if !ench.is_script {
                        for (slot, val) in &ench.attributes {
                            acc.add(slot, *val as f64);
                        }
                    }
                }
            }
            // 也搜索小附魔表（大附魔可能混在里面）
            if let Some(list) = data.enhances.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enchant_id) {
                    for (slot, val) in &ench.attributes {
                        acc.add(slot, *val as f64);
                    }
                }
            }
        }
    }

    // ── 套装加成 ──
    let mut set_counts: HashMap<u32, u32> = HashMap::new();
    for (pos, cfg) in &req.slots {
        let sub_type = pos_to_subtype(pos);
        if let Some(item) = data.items.get(&(sub_type, cfg.equip_id)) {
            if item.set_id > 0 {
                *set_counts.entry(item.set_id).or_default() += 1;
            }
        }
    }
    for (&set_id, &count) in &set_counts {
        if let Some(set) = data.sets.get(&set_id) {
            for (&n, attrs) in &set.bonuses {
                if count >= n as u32 {
                    for b in attrs {
                        // 跳过脚本类套装效果（仅做展示）
                        if b.slot == "atSkillEventHandler"
                            || b.slot == "atExecuteScript"
                            || b.slot == "atSetEquipmentRecipe"
                        {
                            continue;
                        }
                        acc.add(&b.slot, b.value as f64);
                    }
                }
            }
        }
    }

    // ── 五彩石 ──
    if req.stone_id > 0 {
        if let Some(stone) = data.stones.iter().find(|s| s.id == req.stone_id) {
            for sa in &stone.attributes {
                if diamond_count >= sa.need_count && diamond_level >= sa.need_intensity {
                    acc.add(&sa.slot, sa.value as f64);
                }
            }
        }
    }

    // ── 全角色基础（130 级人物默认主属性 + 系统给所有角色的基础防御）──
    acc.add("atVitalityBase",       PLAYER_BASE_VITALITY);
    acc.add("atStrengthBase",       PLAYER_BASE_STRENGTH);
    acc.add("atAgilityBase",        PLAYER_BASE_AGILITY);
    acc.add("atSpiritBase",         PLAYER_BASE_SPIRIT);
    acc.add("atSpunkBase",          PLAYER_BASE_SPUNK);
    acc.add("atPhysicsShieldBase",  PLAYER_BASE_PHYSICS_SHIELD);
    acc.add("atMagicShield",        PLAYER_BASE_MAGIC_SHIELD);

    // ── 心法固定增益（从 school.toml [base_stats] 注入；req.mount 仅作记录，不再决定数值）──
    let _mount_id = req.mount;  // 保留兼容字段，未来可能做 sanity check
    if base_stats.physics_attack_power != 0.0 { acc.add("atPhysicsAttackPowerBase", base_stats.physics_attack_power); }
    if base_stats.physics_overcome     != 0.0 { acc.add("atPhysicsOvercomeBase",    base_stats.physics_overcome); }
    if base_stats.vitality             != 0.0 { acc.add("atVitalityBase",            base_stats.vitality); }
    if base_stats.agility              != 0.0 { acc.add("atAgilityBase",             base_stats.agility); }
    if base_stats.strength             != 0.0 { acc.add("atStrengthBase",            base_stats.strength); }
    if base_stats.parry                != 0.0 { acc.add("atParryBase",               base_stats.parry); }
    if base_stats.parry_value          != 0.0 { acc.add("atParryValueBase",          base_stats.parry_value); }
    if base_stats.physics_shield       != 0.0 { acc.add("atPhysicsShieldBase",       base_stats.physics_shield); }
    if base_stats.magic_shield         != 0.0 { acc.add("atMagicShield",             base_stats.magic_shield); }

    // ── 奇穴被动属性加成（在百分比展开之前加入） ──
    // 影响面板数值的常驻奇穴表（ID → 属性加成）
    // 数值单位：百分比加成走 1024 制（102 ≈ 10%, 205 ≈ 20%）
    for &tid in &req.talents {
        match tid {
            13124 => {
                // 活血：体质 +10%
                acc.add("atVitalityBasePercentAdd", 102.0);
            }
            13366 => {
                // 从容：外功攻击 +20%
                acc.add("atPhysicsAttackPowerPercent", 205.0);
            }
            // 可按需扩展（老版本）：
            // "活脉" => vitality% + agility%
            // "用御" => haste%
            _ => {}
        }
    }

    // ── 全属性加算 → 主属性 ──
    let all_type_add = acc.get("atBasePotentialAdd");
    if all_type_add > 0.0 {
        acc.add("atVitalityBase", all_type_add);
        acc.add("atAgilityBase", all_type_add);
        acc.add("atStrengthBase", all_type_add);
    }

    // ── 全能 (atPVXAllRound) → 破招 / 无双 / 化劲 ──
    // 每点全能 = 0.5 破招 + 1.5 无双 + 1 化劲（每项单独 floor）
    let pvx = acc.get("atPVXAllRound");
    if pvx > 0.0 {
        acc.add("atSurplusValueBase",         (pvx * PVX_TO_SURPLUS).floor());
        acc.add("atStrainBase",               (pvx * PVX_TO_STRAIN).floor());
        acc.add("atDecriticalDamagePowerBase",(pvx * PVX_TO_DECRIT).floor());
    }

    // ── 主属性百分比加成 ──
    let pct_pairs = [
        ("atVitalityBasePercentAdd", "atVitalityBase"),
        ("atAgilityBasePercentAdd", "atAgilityBase"),
        ("atStrengthBasePercentAdd", "atStrengthBase"),
    ];
    for (pct_slot, base_slot) in &pct_pairs {
        let pct = acc.get(pct_slot);
        if pct > 0.0 {
            let base = acc.get(base_slot);
            let bonus = (base * pct / 1024.0).floor();
            acc.add(base_slot, bonus);
        }
    }

    // ── 外功防御额外加算 ──
    let shield_add = acc.get("atPhysicsShieldAdditional");
    if shield_add > 0.0 {
        acc.add("atPhysicsShieldBase", shield_add);
    }

    // ── AllType 等级展开（全会心/全破防/全攻击/全会心效果 → 外功对应字段加算）──
    let all_crit = acc.get("atAllTypeCriticalStrike");
    if all_crit > 0.0 {
        acc.add("atPhysicsCriticalStrike", all_crit);
    }
    let all_overcome = acc.get("atAllTypeOvercomeBase");
    if all_overcome > 0.0 {
        acc.add("atPhysicsOvercomeBase", all_overcome);
    }
    let all_attack = acc.get("atAllTypeAttackPowerBase");
    if all_attack > 0.0 {
        acc.add("atPhysicsAttackPowerBase", all_attack);
    }
    let all_crit_dmg = acc.get("atAllTypeCriticalDamagePowerBase");
    if all_crit_dmg > 0.0 {
        acc.add("atPhysicsCriticalDamagePowerBase", all_crit_dmg);
    }

    // ── 系统固定转化（全角色通用，不属于任何心法）──
    let agility  = acc.get("atAgilityBase");
    let strength = acc.get("atStrengthBase");
    let vitality = acc.get("atVitalityBase");
    acc.add("atPhysicsCriticalStrike",  (agility  * SYS_AGILITY_TO_CRIT).floor());
    acc.add("atPhysicsAttackPowerBase", (strength * SYS_STRENGTH_TO_ATTACK).floor());
    acc.add("atPhysicsOvercomeBase",    (strength * SYS_STRENGTH_TO_OVERCOME).floor());

    // ── 攻击百分比加成（在心法转化之前算，心法转化结果不进百分比）──
    let atk_pct  = acc.get("atPhysicsAttackPowerPercent");
    let atk_base = acc.get("atPhysicsAttackPowerBase");
    let atk_pct_bonus = if atk_pct > 0.0 {
        (atk_base * atk_pct / 1024.0).floor()
    } else { 0.0 };

    // ── 心法转化（mount-specific，作用在最终主属性上；结果不再受乘性增益影响）──
    // 系数是郭氏 (/1024)；每项单独 / 1024 再 floor，符合游戏内部定点定约
    let mc = conversions;
    let extra_attack    = (agility  * mc.agility_to_attack       / 1024.0).floor()
                        + (vitality * mc.vitality_to_attack      / 1024.0).floor();
    let extra_parry     = (agility  * mc.agility_to_parry        / 1024.0).floor()
                        + (vitality * mc.vitality_to_parry       / 1024.0).floor();
    let extra_parry_val = (agility  * mc.agility_to_parry_value  / 1024.0).floor()
                        + (vitality * mc.vitality_to_parry_value / 1024.0).floor();

    acc.add("atParryBase",      extra_parry);
    acc.add("atParryValueBase", extra_parry_val);

    let final_attack = atk_base + atk_pct_bonus + extra_attack;

    // ── 加速百分比 ──
    let haste_base = acc.get("atHasteBase");
    let haste_pct = acc.get("atHasteBasePercentAdd");
    let haste_rate = (haste_base / LP_HASTE + haste_pct / 1024.0).min(0.25);

    // ── 组装结果（所有整数字段 floor 栅栏，防止浮点误差导致前端显示 8 变成 9） ──
    let f = |slot: &str| acc.get(slot).floor();
    let raw = RawAttrs {
        vitality: vitality.floor(),
        strength: f("atStrengthBase"),
        agility: agility.floor(),
        spirit: f("atSpiritBase"),
        spunk: f("atSpunkBase"),
        base_attack: atk_base.floor(),
        base_magical_attack: f("atMagicAttackPowerBase"),
        weapon_damage: f("atMeleeWeaponDamageBase"),
        weapon_damage_rand: f("atMeleeWeaponDamageRand"),
        surplus_value: f("atSurplusValueBase"),
        crit_level: f("atPhysicsCriticalStrike"),
        crit_effect_level: f("atPhysicsCriticalDamagePowerBase"),
        overcome_level: f("atPhysicsOvercomeBase"),
        strain_level: f("atStrainBase"),
        haste_level: haste_base.floor(),
        parry_level: f("atParryBase"),
        parry_value: f("atParryValueBase"),
        dodge_level: f("atDodge"),
        toughness_level: f("atToughnessBase"),
        decritical_damage_level: f("atDecriticalDamagePowerBase"),
        physics_shield: f("atPhysicsShieldBase"),
        magic_shield: f("atMagicShield"),
        threat: f("atActiveThreatCoefficient"),
        weapon_speed: f("atMeleeWeaponAttackSpeedBase"),
        pvx_all_round: f("atPVXAllRound"),
    };

    let phys_shield = acc.get("atPhysicsShieldBase");
    let mag_shield = acc.get("atMagicShield");
    let parry = acc.get("atParryBase");
    let dodge = acc.get("atDodge");
    let toughness = acc.get("atToughnessBase");
    let crit = acc.get("atPhysicsCriticalStrike");
    let crit_eff = acc.get("atPhysicsCriticalDamagePowerBase");
    let overcome = acc.get("atPhysicsOvercomeBase");
    let strain = acc.get("atStrainBase");

    // 气血：全心法基础 + 体质×10（系统）+ 体质×心法附加 + 装备/镶嵌的 atMaxLifeAdditional
    // 每项系数单独 floor
    let final_vit = acc.get("atVitalityBase");
    let max_life = PLAYER_BASE_MAX_LIFE
        + (final_vit * SYS_VITALITY_TO_HP).floor()
        + (final_vit * conversions.vitality_to_hp).floor()
        + acc.get("atMaxLifeAdditional");

    let decrit_level = acc.get("atDecriticalDamagePowerBase");
    // 面板数值：整数字段 floor；百分比保留 f64 给前端格式化
    let panel = PanelAttrs {
        physics_attack_power: final_attack.floor(),
        crit_rate: crit / LP_CRIT,
        crit_effect: 1.75 + crit_eff / LP_CRIT_EFF,
        overcome_rate: overcome / LP_OVERCOME,
        strain_rate: strain / LP_STRAIN,
        haste_rate,
        surplus_value: acc.get("atSurplusValueBase").floor(),
        physics_shield_rate: (phys_shield / (phys_shield + DEFENSE_NONLINEAR)).min(0.75),
        magic_shield_rate: (mag_shield / (mag_shield + DEFENSE_NONLINEAR)).min(0.75),
        parry_rate: parry / (parry + PARRY_NONLINEAR) + 0.03,
        parry_value: acc.get("atParryValueBase").floor(),
        dodge_rate: dodge / (dodge + DODGE_NONLINEAR),
        toughness_rate: toughness / LP_TOUGHNESS,
        max_life: max_life.floor(),
        // 化劲：level / (level + 33046.2) + 102/1024（基础 9.96%）
        decritical_damage_rate: decrit_level / (decrit_level + DECRIT_NONLINEAR) + DECRIT_BASE_RATE,
        agility:  acc.get("atAgilityBase").floor(),
        strength: acc.get("atStrengthBase").floor(),
        vitality: final_vit.floor(),
    };

    // 品质等级：显示所有装备的平均值（保留 1 位小数的想法太啰嗦，取整）
    let avg_quality = if quality_count > 0 {
        (total_quality + (quality_count as i64 / 2)) / quality_count as i64
    } else { 0 };
    CalcResponse { score: total_score, quality_level: avg_quality, raw, panel }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 预处理：.tab → JSON（发布时只带 JSON，不暴露原始 .tab）
// ═══════════════════════════════════════════════════════════════════════════════

/// 序列化用的中间结构
#[derive(Serialize, Deserialize)]
struct ProcessedData {
    items: Vec<EquipItem>,
    enhances: HashMap<i32, Vec<EnchantEntry>>,
    enchants: HashMap<i32, Vec<EnchantEntry>>,
    stones: Vec<StoneEntry>,
    sets: HashMap<u32, SetEntry>,
}

/// 从已加载的 EquipData 过滤并保存为 JSON
///
/// 过滤规则（苍云 T / 外攻 配装）：
///   - 品级 >= min_level（建议 22000）
///   - AND magic_kind NOT IN {根骨, 元气, 治疗, 内功}
///   - AND (
///         belong_school IN {通用, 苍云}
///       OR belong_school = "精简" AND magic_kind IN {外功, 防御}
///     )
///   - 排除测试装备
pub fn save_processed(data: EquipData, output: &Path, min_level: u32) {
    let EquipData { items: items_map, enhances, enchants, stones, sets, .. } = data;

    let exclude_kinds: HashSet<&str> = ["根骨", "元气", "治疗", "内功"].iter().copied().collect();
    let core_schools: HashSet<&str> = ["通用", "苍云"].iter().copied().collect();
    let jianjian_allow_kinds: HashSet<&str> = ["外功", "防御"].iter().copied().collect();

    let items: Vec<EquipItem> = items_map.into_values()
        .filter(|item| {
            if item.name.is_empty() || item.name.ends_with("_测试用") { return false; }
            if item.level < min_level { return false; }
            if exclude_kinds.contains(item.magic_kind.as_str()) { return false; }
            let school = item.belong_school.as_str();
            if core_schools.contains(school) { return true; }
            if school == "精简" && jianjian_allow_kinds.contains(item.magic_kind.as_str()) {
                return true;
            }
            false
        })
        .collect();

    eprintln!("[equip] 预处理: 过滤后 {} 件装备", items.len());

    let processed = ProcessedData { items, enhances, enchants, stones, sets };

    let json = serde_json::to_vec(&processed).expect("序列化失败");
    std::fs::write(output, &json).expect("写入失败");
    eprintln!("[equip] 预处理完成: {:?} ({:.1} MB)", output, json.len() as f64 / 1_048_576.0);
}

/// 从预处理的 JSON 加载装备数据
pub fn load_from_processed(path: &Path) -> Option<EquipData> {
    let t0 = std::time::Instant::now();
    let bytes = std::fs::read(path).ok()?;
    let processed: ProcessedData = serde_json::from_slice(&bytes).ok()?;

    let mut items: HashMap<(u8, u32), EquipItem> = HashMap::with_capacity(processed.items.len());
    for item in processed.items {
        items.insert((item.sub_type, item.id), item);
    }

    // 重建索引
    let mut items_by_subtype: HashMap<u8, Vec<u32>> = HashMap::new();
    for (&(st, id), _item) in &items {
        items_by_subtype.entry(st).or_default().push(id);
    }
    for (&st, ids) in items_by_subtype.iter_mut() {
        ids.sort_by(|a, b| {
            let la = items[&(st, *a)].level;
            let lb = items[&(st, *b)].level;
            lb.cmp(&la).then(items[&(st, *a)].name.cmp(&items[&(st, *b)].name))
        });
    }

    eprintln!("[equip] 从 JSON 加载: {} 件装备, {:?}", items.len(), t0.elapsed());

    // schema 检测：旧 JSON 缓存里 EnchantEntry 可能缺 is_challenge / is_heroic / quality 字段
    //   （serde default=false / 0）。如果发现任意含挑战关键词的 entry 但 is_challenge=false，
    //   或全部 enhance 的 quality 都为 0（说明没经过 enchant_quality.tsv 回填），
    //   返回 None 让 load_equip_smart 走 .tab 路径自动重新生成。
    //   未来加新挑战附魔系列时同步更新 enrich_enhance_flags 的关键词列表 + 这里的检测列表。
    let needs_rebuild_chal = processed.enhances.values().flat_map(|v| v.iter()).any(|e| {
        let has_chal_kw = e.name.contains("·白虹")
            || e.name.contains("白虹贯岩")
            || e.name.contains("·荆岫")
            || e.name.contains("荆岫璞玉");
        has_chal_kw && !e.is_challenge
    });
    let needs_rebuild_quality = processed.enhances.values().flat_map(|v| v.iter())
        .chain(processed.enchants.values().flat_map(|v| v.iter()))
        .all(|e| e.quality == 0);
    if needs_rebuild_chal {
        eprintln!("[equip] JSON 缓存缺 is_challenge 字段，触发从 .tab 重新生成");
        return None;
    }
    if needs_rebuild_quality {
        eprintln!("[equip] JSON 缓存缺 quality 字段，触发从 .tab 重新生成");
        return None;
    }

    Some(EquipData {
        attrib_table: HashMap::new(), // 预处理后不需要
        items,
        items_by_subtype,
        enhances: processed.enhances,
        enchants: processed.enchants,
        stones: processed.stones,
        sets: processed.sets,
    })
}

/// 智能加载：优先 JSON，回退 .tab
pub fn load_equip_smart(data_dir: &Path) -> EquipData {
    let json_path = data_dir.join("equip.json");
    let tab_dir = data_dir.join("equip");

    // 优先加载预处理的 JSON
    if json_path.exists() {
        if let Some(data) = load_from_processed(&json_path) {
            return data;
        }
        eprintln!("[equip] JSON 加载失败，尝试 .tab 回退");
    }

    // 回退到 .tab 原始文件
    if tab_dir.exists() {
        let data = load_equip_data(&tab_dir);
        // 自动生成 JSON 缓存（复用已加载的数据，无需重新读 .tab）
        eprintln!("[equip] 从 .tab 加载成功，自动生成 JSON 缓存...");
        save_processed(data, &json_path, 22000);
        // 生成后再从 JSON 加载（确保启动后就是过滤后的数据集）
        return load_from_processed(&json_path).unwrap_or_else(|| {
            eprintln!("[equip] JSON 重新加载失败");
            EquipData {
                attrib_table: HashMap::new(),
                items: HashMap::new(),
                items_by_subtype: HashMap::new(),
                enhances: HashMap::new(),
                enchants: HashMap::new(),
                stones: Vec::new(),
                sets: HashMap::new(),
            }
        });
    }

    eprintln!("[equip] 无装备数据（{:?} 和 {:?} 均不存在）", json_path, tab_dir);
    EquipData {
        attrib_table: HashMap::new(),
        items: HashMap::new(),
        items_by_subtype: HashMap::new(),
        enhances: HashMap::new(),
        enchants: HashMap::new(),
        stones: Vec::new(),
        sets: HashMap::new(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// search_calc — 配装搜索专用增量计算
//
// 与 equip::calculate 等价，但拆成 (一次预处理 + 每叶子小步) 两阶段：
//   prepare_slot_contrib(item, cfg)          一次/候选 — 把单件装备折叠成稀疏 dense 增量
//   prepare_search_init(...)                 一次/搜索 — 折叠固定槽 + 心法 + 系统基础 + 奇穴
//   calc_leaf_raw(ctx, accum, set_counts, …) 一次/叶子 — 拷贝 accum + 套装 + 后处理 → RawAttrs
//
// 主要目的：避免每片叶子都重新构造 HashMap<String, f64>、再走 12 个槽的 acc.add 字符串哈希。
// 替换为定长 [f64; N] 数组的稠密累加，叶子 ~1μs（vs 全量 calc 的 8μs，预期 5~10× 提速）。
//
// 不变：算出的 RawAttrs 字段必须与 equip::calculate 完全一致（含 floor 取整顺序）。
// ═══════════════════════════════════════════════════════════════════════════════

pub mod search_calc {
    use super::*;

    // 槽位下标（参与 calc 的全部 atXxx 字段；其他 slot 字符串自动丢弃）
    pub const ATTR_VITALITY_BASE:               usize = 0;
    pub const ATTR_STRENGTH_BASE:               usize = 1;
    pub const ATTR_AGILITY_BASE:                usize = 2;
    pub const ATTR_SPIRIT_BASE:                 usize = 3;
    pub const ATTR_SPUNK_BASE:                  usize = 4;
    pub const ATTR_PHYSICS_SHIELD_BASE:         usize = 5;
    pub const ATTR_MAGIC_SHIELD:                usize = 6;
    pub const ATTR_PHYSICS_ATTACK_POWER_BASE:   usize = 7;
    pub const ATTR_PHYSICS_ATTACK_POWER_PCT:    usize = 8;
    pub const ATTR_MAGIC_ATTACK_POWER_BASE:     usize = 9;
    pub const ATTR_PHYSICS_CRITICAL_STRIKE:     usize = 10;
    pub const ATTR_PHYSICS_CRIT_DAMAGE_BASE:    usize = 11;
    pub const ATTR_PHYSICS_OVERCOME_BASE:       usize = 12;
    pub const ATTR_PHYSICS_OVERCOME_PCT:        usize = 13;
    pub const ATTR_STRAIN_BASE:                 usize = 14;
    pub const ATTR_STRAIN_PCT:                  usize = 15;  // atStrainBasePercentAdd
    pub const ATTR_SURPLUS_VALUE_BASE:          usize = 16;
    pub const ATTR_HASTE_BASE:                  usize = 17;
    pub const ATTR_HASTE_PCT:                   usize = 18;
    pub const ATTR_DECRIT_DAMAGE_BASE:          usize = 19;
    pub const ATTR_TOUGHNESS_BASE:              usize = 20;
    pub const ATTR_PARRY_BASE:                  usize = 21;
    pub const ATTR_PARRY_VALUE_BASE:            usize = 22;
    pub const ATTR_DODGE:                       usize = 23;
    pub const ATTR_THREAT_COEFF:                usize = 24;
    pub const ATTR_MELEE_WEAPON_DMG_BASE:       usize = 25;
    pub const ATTR_MELEE_WEAPON_DMG_RAND:       usize = 26;
    pub const ATTR_MELEE_WEAPON_SPEED_BASE:     usize = 27;
    pub const ATTR_MAX_LIFE_ADD:                usize = 28;
    pub const ATTR_PHYSICS_SHIELD_ADD:          usize = 29;
    pub const ATTR_BASE_POTENTIAL_ADD:          usize = 30;
    pub const ATTR_PVX_ALL_ROUND:               usize = 31;
    pub const ATTR_VITALITY_PCT:                usize = 32;
    pub const ATTR_AGILITY_PCT:                 usize = 33;
    pub const ATTR_STRENGTH_PCT:                usize = 34;
    pub const ATTR_ALL_TYPE_CRIT:               usize = 35;
    pub const ATTR_ALL_TYPE_OVERCOME_BASE:      usize = 36;
    pub const ATTR_ALL_TYPE_ATTACK_POWER_BASE:  usize = 37;
    pub const ATTR_ALL_TYPE_CRIT_DAMAGE_BASE:   usize = 38;
    pub const N_ATTRS:                          usize = 39;

    /// 进程级 unknown slot 缓存：每种 slot 字符串首次出现时打印一次警告，避免日志刷屏。
    /// 仅 search_calc 路径会用；equip::calculate 仍按 HashMap<String, f64> 收集，不受影响。
    static UNKNOWN_SLOTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>>
        = std::sync::OnceLock::new();

    /// 已知"故意忽略"的 slot 名单（脚本/特效/秘籍触发，没有数值贡献），不打印警告
    fn is_silently_ignored(s: &str) -> bool {
        matches!(s, "atSkillEventHandler" | "atExecuteScript" | "atSetEquipmentRecipe")
    }

    fn warn_unknown_slot(s: &str) {
        if is_silently_ignored(s) { return; }
        let set = UNKNOWN_SLOTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
        let mut g = set.lock().unwrap();
        if g.insert(s.to_string()) {
            eprintln!("[search_calc] WARN: unknown attribute slot \"{}\" 被增量计算丢弃 —— 如果它影响面板请添加映射", s);
        }
    }

    /// slot 字符串 → 下标；不识别的（含 atSkillEventHandler / atExecuteScript / atSetEquipmentRecipe）返回 None
    pub fn slot_to_idx(s: &str) -> Option<usize> {
        Some(match s {
            "atVitalityBase"                    => ATTR_VITALITY_BASE,
            "atStrengthBase"                    => ATTR_STRENGTH_BASE,
            "atAgilityBase"                     => ATTR_AGILITY_BASE,
            "atSpiritBase"                      => ATTR_SPIRIT_BASE,
            "atSpunkBase"                       => ATTR_SPUNK_BASE,
            "atPhysicsShieldBase"               => ATTR_PHYSICS_SHIELD_BASE,
            "atMagicShield"                     => ATTR_MAGIC_SHIELD,
            "atPhysicsAttackPowerBase"          => ATTR_PHYSICS_ATTACK_POWER_BASE,
            "atPhysicsAttackPowerPercent"       => ATTR_PHYSICS_ATTACK_POWER_PCT,
            "atMagicAttackPowerBase"            => ATTR_MAGIC_ATTACK_POWER_BASE,
            "atPhysicsCriticalStrike"           => ATTR_PHYSICS_CRITICAL_STRIKE,
            "atPhysicsCriticalDamagePowerBase"  => ATTR_PHYSICS_CRIT_DAMAGE_BASE,
            "atPhysicsOvercomeBase"             => ATTR_PHYSICS_OVERCOME_BASE,
            "atPhysicsOvercomePercent"          => ATTR_PHYSICS_OVERCOME_PCT,
            "atStrainBase"                      => ATTR_STRAIN_BASE,
            "atStrainBasePercentAdd"            => ATTR_STRAIN_PCT,
            "atSurplusValueBase"                => ATTR_SURPLUS_VALUE_BASE,
            "atHasteBase"                       => ATTR_HASTE_BASE,
            "atHasteBasePercentAdd"             => ATTR_HASTE_PCT,
            "atDecriticalDamagePowerBase"       => ATTR_DECRIT_DAMAGE_BASE,
            "atToughnessBase"                   => ATTR_TOUGHNESS_BASE,
            "atParryBase"                       => ATTR_PARRY_BASE,
            "atParryValueBase"                  => ATTR_PARRY_VALUE_BASE,
            "atDodge"                           => ATTR_DODGE,
            "atActiveThreatCoefficient"         => ATTR_THREAT_COEFF,
            "atMeleeWeaponDamageBase"           => ATTR_MELEE_WEAPON_DMG_BASE,
            "atMeleeWeaponDamageRand"           => ATTR_MELEE_WEAPON_DMG_RAND,
            "atMeleeWeaponAttackSpeedBase"      => ATTR_MELEE_WEAPON_SPEED_BASE,
            "atMaxLifeAdditional"               => ATTR_MAX_LIFE_ADD,
            "atPhysicsShieldAdditional"         => ATTR_PHYSICS_SHIELD_ADD,
            "atBasePotentialAdd"                => ATTR_BASE_POTENTIAL_ADD,
            "atPVXAllRound"                     => ATTR_PVX_ALL_ROUND,
            "atVitalityBasePercentAdd"          => ATTR_VITALITY_PCT,
            "atAgilityBasePercentAdd"           => ATTR_AGILITY_PCT,
            "atStrengthBasePercentAdd"          => ATTR_STRENGTH_PCT,
            "atAllTypeCriticalStrike"           => ATTR_ALL_TYPE_CRIT,
            "atAllTypeOvercomeBase"             => ATTR_ALL_TYPE_OVERCOME_BASE,
            "atAllTypeAttackPowerBase"          => ATTR_ALL_TYPE_ATTACK_POWER_BASE,
            "atAllTypeCriticalDamagePowerBase"  => ATTR_ALL_TYPE_CRIT_DAMAGE_BASE,
            other => { warn_unknown_slot(other); return None; },
        })
    }

    /// 单件装备（已绑定 cfg 的精炼/镶嵌等级）折叠出来的增量
    #[derive(Clone, Default)]
    pub struct SlotContrib {
        /// (idx, value) 稀疏表示
        pub deltas: Vec<(u8, f64)>,
        /// 套装 ID（0 = 无）
        pub set_id: u32,
        /// 装备特效 ID 列表（atSkillEventHandler / atExecuteScript 的 value 部分）
        pub effect_ids: Vec<u32>,
        /// 该装备贡献的 diamond_count（仅 embed > 0 的孔）—— 用于五彩石条件
        pub diamond_count: u32,
        /// 该装备贡献的 diamond_level（embed_lv 累加）—— 用于五彩石条件
        pub diamond_level: u32,
    }

    /// 装备 → SlotContrib（包含 base + magic 精炼 + diamond 镶嵌 + enhance + enchant）
    pub fn prepare_slot_contrib(
        data: &EquipData,
        pos: &str,
        cfg: &SlotConfig,
    ) -> Option<SlotContrib> {
        let sub_type = pos_to_subtype(pos);
        let item = data.items.get(&(sub_type, cfg.equip_id))?;

        // 用 [f64; N] 缓冲累加 + 最后压成稀疏（候选数有限，每次预处理一次即可）
        let mut buf = [0.0_f64; N_ATTRS];
        let mut effect_ids: Vec<u32> = Vec::new();
        let mut diamond_count: u32 = 0;
        let mut diamond_level: u32 = 0;

        let add = |buf: &mut [f64; N_ATTRS], slot: &str, val: f64| {
            if let Some(idx) = slot_to_idx(slot) {
                buf[idx] += val;
            }
        };

        // ── Base （取 max）──
        for (slot, _min, max) in &item.bases {
            add(&mut buf, slot, *max as f64);
        }
        // ── Magic + 精炼 ──
        for ma in &item.magics {
            // 特效收集（用于 fp）
            if ma.slot == "atSkillEventHandler" || ma.slot == "atExecuteScript" {
                effect_ids.push(ma.value as u32);
                continue;
            }
            // atSetEquipmentRecipe 不参与累加，也不参与精炼
            if ma.slot == "atSetEquipmentRecipe" { continue; }
            add(&mut buf, &ma.slot, ma.value as f64);
            if cfg.strength > 0 {
                let lv = cfg.strength.min(8) as usize;
                let bonus = (ma.value as f64 * STRENGTH_P[lv]).round();
                add(&mut buf, &ma.slot, bonus);
            }
        }
        // ── 镶嵌 ──
        for (i, ds) in item.diamonds.iter().enumerate() {
            let embed_lv = cfg.embedding.get(i).copied().unwrap_or(0);
            if embed_lv > 0 {
                let coeff = embedding_coeff(embed_lv);
                let val = ((ds.base_value as f64).floor() * coeff).floor();
                add(&mut buf, &ds.slot, val);
                diamond_count += 1;
                diamond_level += embed_lv as u32;
            }
        }
        // ── 小附魔 ──
        if cfg.enhance_id > 0 {
            if let Some(list) = data.enhances.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enhance_id) {
                    for (slot, val) in &ench.attributes {
                        add(&mut buf, slot, *val as f64);
                    }
                }
            }
        }
        // ── 大附魔（直接属性型 + 也走小附魔表搜索）──
        if cfg.enchant_id > 0 {
            if let Some(list) = data.enchants.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enchant_id) {
                    if !ench.is_script {
                        for (slot, val) in &ench.attributes {
                            add(&mut buf, slot, *val as f64);
                        }
                    }
                }
            }
            if let Some(list) = data.enhances.get(&(sub_type as i32)) {
                if let Some(ench) = list.iter().find(|e| e.id == cfg.enchant_id) {
                    for (slot, val) in &ench.attributes {
                        add(&mut buf, slot, *val as f64);
                    }
                }
            }
        }

        // 压成稀疏
        let mut deltas: Vec<(u8, f64)> = Vec::new();
        for i in 0..N_ATTRS {
            if buf[i] != 0.0 {
                deltas.push((i as u8, buf[i]));
            }
        }
        Some(SlotContrib {
            deltas, set_id: item.set_id, effect_ids,
            diamond_count, diamond_level,
        })
    }

    /// 套装件数级别的预 fold（[(piece_count, sparse_delta)], 件数升序）
    pub type SetBonusByCount = Vec<(u32, Vec<(u8, f64)>)>;

    /// 一次/搜索 的预处理上下文
    pub struct InitCtx {
        /// 已折入：固定槽 + 系统全角色基础 + 心法 base_stats + 奇穴
        pub initial_accum: [f64; N_ATTRS],
        /// 已折入：固定槽中各装备的 set_id 件数
        pub initial_set_counts: HashMap<u32, u32>,
        /// 已折入：固定槽中各装备的 effect IDs（不变 fingerprint 里）
        pub initial_effect_ids: HashSet<u32>,
        /// 已折入：固定槽贡献的 diamond_count / diamond_level
        pub initial_diamond_count: u32,
        pub initial_diamond_level: u32,

        /// 全部套装 → 件数级别累加表
        pub set_bonus_table: HashMap<u32, SetBonusByCount>,

        /// 五彩石（拷贝以保留原 attributes 含 need_count/need_intensity）
        pub stone: Option<StoneEntry>,

        /// 心法转化（用于 calc_leaf_raw）
        pub mount_conv: MountConversions,
    }

    pub fn prepare_init(
        data: &EquipData,
        fixed_slots: &HashMap<String, SlotConfig>,
        stone_id: u32,
        talents: &[u32],
        base_stats: &MountBaseStats,
        conversions: &MountConversions,
    ) -> InitCtx {
        let mut accum = [0.0_f64; N_ATTRS];
        let mut set_counts: HashMap<u32, u32> = HashMap::new();
        let mut effect_ids: HashSet<u32> = HashSet::new();
        let mut diamond_count: u32 = 0;
        let mut diamond_level: u32 = 0;

        // 1. 固定槽
        for (pos, cfg) in fixed_slots {
            if let Some(c) = prepare_slot_contrib(data, pos, cfg) {
                for (idx, v) in &c.deltas { accum[*idx as usize] += v; }
                if c.set_id > 0 { *set_counts.entry(c.set_id).or_default() += 1; }
                for eid in c.effect_ids { effect_ids.insert(eid); }
                diamond_count += c.diamond_count;
                diamond_level += c.diamond_level;
            }
        }

        // 2. 全角色基础属性
        accum[ATTR_VITALITY_BASE]       += PLAYER_BASE_VITALITY;
        accum[ATTR_STRENGTH_BASE]       += PLAYER_BASE_STRENGTH;
        accum[ATTR_AGILITY_BASE]        += PLAYER_BASE_AGILITY;
        accum[ATTR_SPIRIT_BASE]         += PLAYER_BASE_SPIRIT;
        accum[ATTR_SPUNK_BASE]          += PLAYER_BASE_SPUNK;
        accum[ATTR_PHYSICS_SHIELD_BASE] += PLAYER_BASE_PHYSICS_SHIELD;
        accum[ATTR_MAGIC_SHIELD]        += PLAYER_BASE_MAGIC_SHIELD;

        // 3. 心法 base_stats
        if base_stats.physics_attack_power != 0.0 { accum[ATTR_PHYSICS_ATTACK_POWER_BASE] += base_stats.physics_attack_power; }
        if base_stats.physics_overcome     != 0.0 { accum[ATTR_PHYSICS_OVERCOME_BASE]     += base_stats.physics_overcome; }
        if base_stats.vitality             != 0.0 { accum[ATTR_VITALITY_BASE]             += base_stats.vitality; }
        if base_stats.agility              != 0.0 { accum[ATTR_AGILITY_BASE]              += base_stats.agility; }
        if base_stats.strength             != 0.0 { accum[ATTR_STRENGTH_BASE]             += base_stats.strength; }
        if base_stats.parry                != 0.0 { accum[ATTR_PARRY_BASE]                += base_stats.parry; }
        if base_stats.parry_value          != 0.0 { accum[ATTR_PARRY_VALUE_BASE]          += base_stats.parry_value; }
        if base_stats.physics_shield       != 0.0 { accum[ATTR_PHYSICS_SHIELD_BASE]       += base_stats.physics_shield; }
        if base_stats.magic_shield         != 0.0 { accum[ATTR_MAGIC_SHIELD]              += base_stats.magic_shield; }

        // 4. 奇穴常驻被动（与 calculate 中相同）
        for &tid in talents {
            match tid {
                13124 => accum[ATTR_VITALITY_PCT]              += 102.0,
                13366 => accum[ATTR_PHYSICS_ATTACK_POWER_PCT]  += 205.0,
                _ => {}
            }
        }

        // 5. 套装件数表（全表 fold；候选可能引入新的 set_id，必须覆盖完整）
        let mut set_bonus_table: HashMap<u32, SetBonusByCount> = HashMap::new();
        for set_entry in data.sets.values() {
            let mut levels: SetBonusByCount = Vec::new();
            for (&n, attrs) in &set_entry.bonuses {
                let mut deltas: Vec<(u8, f64)> = Vec::new();
                for b in attrs {
                    if matches!(b.slot.as_str(), "atSkillEventHandler" | "atExecuteScript" | "atSetEquipmentRecipe") {
                        continue;
                    }
                    if let Some(idx) = slot_to_idx(&b.slot) {
                        deltas.push((idx as u8, b.value as f64));
                    }
                }
                if !deltas.is_empty() { levels.push((n as u32, deltas)); }
            }
            levels.sort_by_key(|(n, _)| *n);
            if !levels.is_empty() { set_bonus_table.insert(set_entry.id, levels); }
        }

        // 6. 五彩石
        let stone = if stone_id > 0 {
            data.stones.iter().find(|s| s.id == stone_id).cloned()
        } else { None };

        InitCtx {
            initial_accum: accum,
            initial_set_counts: set_counts,
            initial_effect_ids: effect_ids,
            initial_diamond_count: diamond_count,
            initial_diamond_level: diamond_level,
            set_bonus_table,
            stone,
            mount_conv: conversions.clone(),
        }
    }

    /// 叶子级 calc：拷贝 accum，叠套装 + 五彩石 + 后处理 → RawAttrs
    pub fn calc_leaf_raw(
        ctx: &InitCtx,
        live_accum: &[f64; N_ATTRS],
        live_set_counts: &HashMap<u32, u32>,
        live_diamond_count: u32,
        live_diamond_level: u32,
    ) -> RawAttrs {
        let mut a = *live_accum;

        // 套装件数加成
        for (set_id, count) in live_set_counts {
            if let Some(levels) = ctx.set_bonus_table.get(set_id) {
                for (n, deltas) in levels {
                    if *count >= *n {
                        for (idx, val) in deltas {
                            a[*idx as usize] += val;
                        }
                    }
                }
            }
        }

        // 五彩石条件激活
        if let Some(stone) = &ctx.stone {
            for sa in &stone.attributes {
                if live_diamond_count >= sa.need_count && live_diamond_level >= sa.need_intensity {
                    if let Some(idx) = slot_to_idx(&sa.slot) {
                        a[idx] += sa.value as f64;
                    }
                }
            }
        }

        // ── 后处理（与 calculate 中顺序一致）──

        // 1) atBasePotentialAdd → 主属性
        let all_type_add = a[ATTR_BASE_POTENTIAL_ADD];
        if all_type_add > 0.0 {
            a[ATTR_VITALITY_BASE] += all_type_add;
            a[ATTR_AGILITY_BASE]  += all_type_add;
            a[ATTR_STRENGTH_BASE] += all_type_add;
        }

        // 2) PVX 全能展开
        let pvx = a[ATTR_PVX_ALL_ROUND];
        if pvx > 0.0 {
            a[ATTR_SURPLUS_VALUE_BASE]   += (pvx * 0.5).floor();
            a[ATTR_STRAIN_BASE]          += (pvx * 1.5).floor();
            a[ATTR_DECRIT_DAMAGE_BASE]   += (pvx * 1.0).floor();
        }

        // 3) 主属性百分比
        let pct_pairs = [
            (ATTR_VITALITY_PCT, ATTR_VITALITY_BASE),
            (ATTR_AGILITY_PCT,  ATTR_AGILITY_BASE),
            (ATTR_STRENGTH_PCT, ATTR_STRENGTH_BASE),
        ];
        for (pi, bi) in &pct_pairs {
            let pct = a[*pi];
            if pct > 0.0 {
                let bonus = (a[*bi] * pct / 1024.0).floor();
                a[*bi] += bonus;
            }
        }

        // 4) 外功防御额外
        let shield_add = a[ATTR_PHYSICS_SHIELD_ADD];
        if shield_add > 0.0 {
            a[ATTR_PHYSICS_SHIELD_BASE] += shield_add;
        }

        // 5) AllType 等级展开（全会心/全破防/全攻击/全会心效果 → 外功对应字段加算）
        let all_crit = a[ATTR_ALL_TYPE_CRIT];
        if all_crit > 0.0 {
            a[ATTR_PHYSICS_CRITICAL_STRIKE] += all_crit;
        }
        let all_overcome = a[ATTR_ALL_TYPE_OVERCOME_BASE];
        if all_overcome > 0.0 {
            a[ATTR_PHYSICS_OVERCOME_BASE] += all_overcome;
        }
        let all_attack = a[ATTR_ALL_TYPE_ATTACK_POWER_BASE];
        if all_attack > 0.0 {
            a[ATTR_PHYSICS_ATTACK_POWER_BASE] += all_attack;
        }
        let all_crit_dmg = a[ATTR_ALL_TYPE_CRIT_DAMAGE_BASE];
        if all_crit_dmg > 0.0 {
            a[ATTR_PHYSICS_CRIT_DAMAGE_BASE] += all_crit_dmg;
        }

        // 6) 系统通用转化
        let agility  = a[ATTR_AGILITY_BASE];
        let strength = a[ATTR_STRENGTH_BASE];
        let vitality = a[ATTR_VITALITY_BASE];
        a[ATTR_PHYSICS_CRITICAL_STRIKE]   += (agility  * SYS_AGILITY_TO_CRIT).floor();
        a[ATTR_PHYSICS_ATTACK_POWER_BASE] += (strength * SYS_STRENGTH_TO_ATTACK).floor();
        a[ATTR_PHYSICS_OVERCOME_BASE]     += (strength * SYS_STRENGTH_TO_OVERCOME).floor();

        // 7) 攻击百分比
        let atk_pct  = a[ATTR_PHYSICS_ATTACK_POWER_PCT];
        let atk_base = a[ATTR_PHYSICS_ATTACK_POWER_BASE];
        let atk_pct_bonus = if atk_pct > 0.0 { (atk_base * atk_pct / 1024.0).floor() } else { 0.0 };

        // 8) 心法转化
        let mc = &ctx.mount_conv;
        let extra_attack    = (agility  * mc.agility_to_attack       / 1024.0).floor()
                            + (vitality * mc.vitality_to_attack      / 1024.0).floor();
        let extra_parry     = (agility  * mc.agility_to_parry        / 1024.0).floor()
                            + (vitality * mc.vitality_to_parry       / 1024.0).floor();
        let extra_parry_val = (agility  * mc.agility_to_parry_value  / 1024.0).floor()
                            + (vitality * mc.vitality_to_parry_value / 1024.0).floor();
        a[ATTR_PARRY_BASE]       += extra_parry;
        a[ATTR_PARRY_VALUE_BASE] += extra_parry_val;

        let final_attack = atk_base + atk_pct_bonus + extra_attack;

        // 9) 装配 RawAttrs（每字段单独 floor，与 calculate 一致；
        //    RawAttrs.base_attack = atk_base.floor()，PanelAttrs 的 physics_attack_power 用 final_attack —— 我们只产出 RawAttrs）
        let _ = final_attack;
        let f = |i: usize| a[i].floor();
        RawAttrs {
            vitality:                vitality.floor(),
            strength:                f(ATTR_STRENGTH_BASE),
            agility:                 agility.floor(),
            spirit:                  f(ATTR_SPIRIT_BASE),
            spunk:                   f(ATTR_SPUNK_BASE),
            base_attack:             atk_base.floor(),
            base_magical_attack:     f(ATTR_MAGIC_ATTACK_POWER_BASE),
            weapon_damage:           f(ATTR_MELEE_WEAPON_DMG_BASE),
            weapon_damage_rand:      f(ATTR_MELEE_WEAPON_DMG_RAND),
            surplus_value:           f(ATTR_SURPLUS_VALUE_BASE),
            crit_level:              f(ATTR_PHYSICS_CRITICAL_STRIKE),
            crit_effect_level:       f(ATTR_PHYSICS_CRIT_DAMAGE_BASE),
            overcome_level:          f(ATTR_PHYSICS_OVERCOME_BASE),
            strain_level:            f(ATTR_STRAIN_BASE),
            haste_level:             a[ATTR_HASTE_BASE].floor(),
            parry_level:             f(ATTR_PARRY_BASE),
            parry_value:             f(ATTR_PARRY_VALUE_BASE),
            dodge_level:             f(ATTR_DODGE),
            toughness_level:         f(ATTR_TOUGHNESS_BASE),
            decritical_damage_level: f(ATTR_DECRIT_DAMAGE_BASE),
            physics_shield:          f(ATTR_PHYSICS_SHIELD_BASE),
            magic_shield:            f(ATTR_MAGIC_SHIELD),
            threat:                  f(ATTR_THREAT_COEFF),
            weapon_speed:            f(ATTR_MELEE_WEAPON_SPEED_BASE),
            pvx_all_round:           f(ATTR_PVX_ALL_ROUND),
        }
    }
}
