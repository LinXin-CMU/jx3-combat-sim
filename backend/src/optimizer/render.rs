//! Phase 3 渲染：StructIndividual → MacroConfig（含 128 字约束）
//!
//! 策略：按 shield_order / blade_order 依次渲染 enabled 规则，为每条规则克隆
//! parsed MacroLine + 写入该规则对应的阈值。渲染完一页后序列化，若超 128 字，
//! **从末尾开始禁用非 locked 规则**直到合规（locked 规则保留）。

use std::collections::HashMap;

use crate::macro_engine::{MacroConfig, MacroLine, MacroPage, MacroCondition};
use crate::Stance;

use super::analyze::TunableParam;
use super::rule_pool::{clone_macro_line, serialize_line, RulePool};

pub const MAX_PAGE_CHARS: usize = 1024;

/// 渲染结果：两个完整页 + 实际启用的规则 id 列表（含 locked 自动保留）
pub struct RenderResult {
    pub config: MacroConfig,
    pub shield_emitted: Vec<String>,
    pub blade_emitted: Vec<String>,
    /// 因字数超限被自动禁用的规则（仅 non-locked）
    pub shield_truncated: Vec<String>,
    pub blade_truncated: Vec<String>,
}

/// 渲染一个个体
///
/// `values` 是 flat vector，索引与 `param_keys` 对齐
/// `param_keys[i]` = (rule_id, visit_idx)
pub fn render_individual(
    pool: &RulePool,
    shield_order: &[(String, bool)],
    blade_order: &[(String, bool)],
    param_keys: &[(String, usize)],
    values: &[f64],
) -> RenderResult {
    // 按 rule_id 分组 values，便于局部写入
    let mut values_by_rule: HashMap<&str, Vec<(usize, f64)>> = HashMap::new();
    for (i, (rid, vi)) in param_keys.iter().enumerate() {
        values_by_rule.entry(rid.as_str()).or_default().push((*vi, values[i]));
    }

    let (shield_page, shield_emitted, shield_truncated) =
        render_page(Stance::Shield, shield_order, pool, &values_by_rule);
    let (blade_page, blade_emitted, blade_truncated) =
        render_page(Stance::Blade, blade_order, pool, &values_by_rule);

    RenderResult {
        config: MacroConfig { pages: vec![shield_page, blade_page] },
        shield_emitted,
        blade_emitted,
        shield_truncated,
        blade_truncated,
    }
}

fn render_page(
    stance: Stance,
    order: &[(String, bool)],
    pool: &RulePool,
    values_by_rule: &HashMap<&str, Vec<(usize, f64)>>,
) -> (MacroPage, Vec<String>, Vec<String>) {
    // 先构造 (rule_id, MacroLine, locked) 的启用列表
    let mut staged: Vec<(String, MacroLine, bool)> = Vec::new();
    for (rid, enabled) in order {
        if !enabled { continue; }
        let Some(rule) = pool.rules.get(rid) else { continue; };
        let mut line = clone_macro_line(&rule.parsed);
        if let Some(vals) = values_by_rule.get(rid.as_str()) {
            apply_rule_values(&mut line, &rule.tunables, vals);
        }
        staged.push((rid.clone(), line, rule.locked));
    }

    // 128 字约束：从末尾开始禁用非 locked 规则直到合规
    // 用"游戏实际字符数"（去括号、去 .0 尾的 compact 形式），与最终粘贴宏一致
    let mut truncated: Vec<String> = Vec::new();
    loop {
        let total: usize = staged.iter()
            .map(|(_, l, _)| compact_len(&serialize_line(l)) + 1)  // +1 换行符
            .sum::<usize>()
            .saturating_sub(1);  // 最后一行无换行
        if total <= MAX_PAGE_CHARS { break; }
        // 从末尾找第一个非 locked 的去掉
        let pos = staged.iter().rposition(|(_, _, locked)| !*locked);
        match pos {
            Some(p) => {
                let (rid, _, _) = staged.remove(p);
                truncated.push(rid);
            }
            None => break,  // 全部 locked 也超限：兜底放行（渲染出无法模拟的宏，评估会得 0 DPS）
        }
    }

    let emitted: Vec<String> = staged.iter().map(|(rid, _, _)| rid.clone()).collect();
    let lines: Vec<MacroLine> = staged.into_iter().map(|(_, l, _)| l).collect();
    (MacroPage { stance_filter: Some(stance), lines }, emitted, truncated)
}

/// 游戏里粘贴宏的实际字符数：去 `[` `]` 括号 + 去 `N.0` 尾的 `.0`
/// （与 ga.rs / struct_ga.rs 里的 format_macro_compact 等价，单行版本）
fn compact_len(src: &str) -> usize {
    let chars: Vec<char> = src.chars().collect();
    let mut n: usize = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' || c == ']' { i += 1; continue; }
        if c == '.'
            && i + 1 < chars.len() && chars[i + 1] == '0'
            && (i + 2 >= chars.len() || !chars[i + 2].is_ascii_digit())
            && i > 0 && chars[i - 1].is_ascii_digit()
        {
            i += 2;
            continue;
        }
        n += 1;
        i += 1;
    }
    n
}

/// 把 values 写入一条 line 的 condition leaf（按 visit_idx 定位）
fn apply_rule_values(line: &mut MacroLine, tunables: &[TunableParam], values: &[(usize, f64)]) {
    let Some(cond) = &mut line.condition else { return; };
    for (vi, v) in values {
        let mut counter = 0usize;
        set_nth_leaf(cond, *vi, &mut counter, *v);
        let _ = tunables;  // 保留参数签名一致性，暂不使用
    }
}

fn set_nth_leaf(cond: &mut MacroCondition, target: usize, counter: &mut usize, v: f64) -> bool {
    use MacroCondition::*;
    match cond {
        And(a, b) | Or(a, b) => {
            if set_nth_leaf(a, target, counter, v) { return true; }
            set_nth_leaf(b, target, counter, v)
        }
        Rage(_, val) => {
            if *counter == target { *val = v.round() as i32; return true; }
            *counter += 1;
            false
        }
        Life(_, val) => {
            if *counter == target { *val = v; return true; }
            *counter += 1;
            false
        }
        BuffTime(_, _, val) | TBuffTime(_, _, val) => {
            if *counter == target { *val = v; return true; }
            *counter += 1;
            false
        }
        SkillEnergy(_, _, val) => {
            if *counter == target { *val = v.max(0.0).round() as u32; return true; }
            *counter += 1;
            false
        }
        NearbyEnemy(_, val) => {
            if *counter == target { *val = v.max(0.0).round() as u32; return true; }
            *counter += 1;
            false
        }
        _ => false,
    }
}
