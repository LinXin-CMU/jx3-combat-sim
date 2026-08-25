//! 冗余剪枝：列举候选原子删除方案。
//!
//! 后端只做 parse → 列举叶子 → 渲染一份"删了这个原子"的宏文本。
//! 具体每个候选是否要留下，由前端通过 /api/simulate 评估 fitness 决定。

use serde::{Deserialize, Serialize};

use crate::macro_engine::{CmpOp, MacroCondition, MacroConfig, MacroLine, MacroPage, MacroAction};
use crate::macro_parser::{parse_macro_text, render_macro_text};

#[derive(Debug, Serialize)]
pub struct PruneCandidate {
    pub rule_index: usize,
    pub page_index: usize,
    pub skill: String,
    pub atom: String,          // 被删的原子的 display_string
    pub before_cond: String,   // 删除前的整条条件
    pub after_cond: String,    // 删除后（可能为 "(无)"）
    pub after_macro: String,   // 删除后整个宏的文本
}

#[derive(Debug, Deserialize, Default)]
pub struct PruneOptions {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub max_loss_pct: Option<f64>,
    #[serde(default)]
    pub sensitive_skills: Option<Vec<String>>,
}

/// 枚举所有叶子原子的 path（每段 L/R）
fn enumerate_leaf_paths(c: &MacroCondition) -> Vec<Vec<char>> {
    fn walk(c: &MacroCondition, cur: Vec<char>, out: &mut Vec<Vec<char>>) {
        match c {
            MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
                let mut la = cur.clone(); la.push('L');
                walk(a, la, out);
                let mut lb = cur.clone(); lb.push('R');
                walk(b, lb, out);
            }
            _ => { out.push(cur); }
        }
    }
    let mut out = Vec::new();
    walk(c, Vec::new(), &mut out);
    out
}

fn leaf_at(c: &MacroCondition, path: &[char]) -> Option<String> {
    if path.is_empty() {
        return match c {
            MacroCondition::And(_, _) | MacroCondition::Or(_, _) => None,
            _ => Some(c.display_string()),
        };
    }
    match c {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            let target = if path[0] == 'L' { a.as_ref() } else { b.as_ref() };
            leaf_at(target, &path[1..])
        }
        _ => None,
    }
}

fn remove_leaf(c: &MacroCondition, path: &[char]) -> Option<MacroCondition> {
    if path.is_empty() { return None; }
    match c {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            let (first, rest) = (path[0], &path[1..]);
            let (target, other) = if first == 'L' { (a.as_ref(), b.as_ref()) } else { (b.as_ref(), a.as_ref()) };
            if rest.is_empty() { return Some(clone_cond(other)); }
            let new_target = remove_leaf(target, rest);
            match new_target {
                Some(nt) => Some(match c {
                    MacroCondition::And(_, _) =>
                        if first == 'L' { MacroCondition::And(Box::new(nt), Box::new(clone_cond(other))) }
                        else { MacroCondition::And(Box::new(clone_cond(other)), Box::new(nt)) },
                    MacroCondition::Or(_, _) =>
                        if first == 'L' { MacroCondition::Or(Box::new(nt), Box::new(clone_cond(other))) }
                        else { MacroCondition::Or(Box::new(clone_cond(other)), Box::new(nt)) },
                    _ => unreachable!(),
                }),
                None => Some(clone_cond(other)),
            }
        }
        _ => None,
    }
}

/// MacroCondition 没 derive Clone（Box 里套递归），手写一个
fn clone_cond(c: &MacroCondition) -> MacroCondition {
    match c {
        MacroCondition::Rage(op, v) => MacroCondition::Rage(*op, *v),
        MacroCondition::Life(op, v) => MacroCondition::Life(*op, *v),
        MacroCondition::Buff(n) => MacroCondition::Buff(n.clone()),
        MacroCondition::NoBuff(n) => MacroCondition::NoBuff(n.clone()),
        MacroCondition::BuffTime(n, op, v) => MacroCondition::BuffTime(n.clone(), *op, *v),
        MacroCondition::BuffStack(n, op, v) => MacroCondition::BuffStack(n.clone(), *op, *v),
        MacroCondition::TBuff(n) => MacroCondition::TBuff(n.clone()),
        MacroCondition::TnoBuff(n) => MacroCondition::TnoBuff(n.clone()),
        MacroCondition::TBuffTime(n, op, v) => MacroCondition::TBuffTime(n.clone(), *op, *v),
        MacroCondition::SkillNotInCd(n) => MacroCondition::SkillNotInCd(n.clone()),
        MacroCondition::SkillExists(id) => MacroCondition::SkillExists(*id),
        MacroCondition::SkillNotExists(id) => MacroCondition::SkillNotExists(*id),
        MacroCondition::SkillEnergy(n, op, v) => MacroCondition::SkillEnergy(n.clone(), *op, *v),
        MacroCondition::LastSkill(n) => MacroCondition::LastSkill(n.clone()),
        MacroCondition::LastSkillNot(n) => MacroCondition::LastSkillNot(n.clone()),
        MacroCondition::NearbyEnemy(op, v) => MacroCondition::NearbyEnemy(*op, *v),
        MacroCondition::And(a, b) => MacroCondition::And(Box::new(clone_cond(a)), Box::new(clone_cond(b))),
        MacroCondition::Or(a, b) => MacroCondition::Or(Box::new(clone_cond(a)), Box::new(clone_cond(b))),
    }
}

fn clone_action(a: &MacroAction) -> MacroAction {
    match a {
        MacroAction::Cast(n) => MacroAction::Cast(n.clone()),
        MacroAction::FCast(n) => MacroAction::FCast(n.clone()),
    }
}

fn clone_line(l: &MacroLine) -> MacroLine {
    MacroLine {
        condition: l.condition.as_ref().map(clone_cond),
        action: clone_action(&l.action),
    }
}

fn clone_page(p: &MacroPage) -> MacroPage {
    MacroPage {
        stance_filter: p.stance_filter,
        lines: p.lines.iter().map(clone_line).collect(),
    }
}

fn clone_config(c: &MacroConfig) -> MacroConfig {
    MacroConfig {
        pages: c.pages.iter().map(clone_page).collect(),
    }
}

/// 返回 path 指向的节点（AND/OR/叶子都可）。path 空串表示根自身。
fn node_at<'a>(c: &'a MacroCondition, path: &[char]) -> Option<&'a MacroCondition> {
    if path.is_empty() { return Some(c); }
    match c {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            let target = if path[0] == 'L' { a.as_ref() } else { b.as_ref() };
            node_at(target, &path[1..])
        }
        _ => None,
    }
}

/// 判断叶子是否属于 "tnobuff:虚弱 & last_skill~=盾飞" 这对保护对 —— 任意一侧被删都会破坏虚弱延迟保护。
fn is_virtual_weak_pair_leaf(c: &MacroCondition, path: &[char]) -> bool {
    if path.is_empty() { return false; }
    let parent = match node_at(c, &path[..path.len()-1]) { Some(p) => p, None => return false };
    let (a, b) = match parent {
        MacroCondition::And(a, b) => (a.as_ref(), b.as_ref()),
        _ => return false,
    };
    let is_weak = |x: &MacroCondition| matches!(x, MacroCondition::TnoBuff(n) if n == "虚弱");
    let is_shield = |x: &MacroCondition| matches!(x, MacroCondition::LastSkillNot(n) if n == "盾飞");
    (is_weak(a) && is_shield(b)) || (is_weak(b) && is_shield(a))
}

/// 把 AND 链里的 last_skill / last_skill~= 原子重排到最前。OR 结构保持不变，内部递归处理 AND。
pub fn reorder_last_skill_first(c: &MacroCondition) -> MacroCondition {
    fn is_last_skill(x: &MacroCondition) -> bool {
        matches!(x, MacroCondition::LastSkill(_) | MacroCondition::LastSkillNot(_))
    }
    fn flatten_and(c: &MacroCondition, out: &mut Vec<MacroCondition>) {
        match c {
            MacroCondition::And(a, b) => { flatten_and(a, out); flatten_and(b, out); }
            other => out.push(reorder_last_skill_first(other)),
        }
    }
    match c {
        MacroCondition::And(_, _) => {
            let mut items = Vec::new();
            flatten_and(c, &mut items);
            let mut ls: Vec<MacroCondition> = Vec::new();
            let mut rest: Vec<MacroCondition> = Vec::new();
            for it in items {
                if is_last_skill(&it) { ls.push(it); } else { rest.push(it); }
            }
            let mut all = ls;
            all.extend(rest);
            let mut it = all.into_iter();
            let first = it.next().unwrap();
            it.fold(first, |acc, x| MacroCondition::And(Box::new(acc), Box::new(x)))
        }
        MacroCondition::Or(a, b) => MacroCondition::Or(
            Box::new(reorder_last_skill_first(a)),
            Box::new(reorder_last_skill_first(b)),
        ),
        other => clone_cond(other),
    }
}

/// 枚举所有可交换的相邻规则对（仅限同一页内相邻两行），返回交换后的宏文本列表。
#[derive(Debug, Serialize)]
pub struct SwapCandidate {
    pub page_index: usize,
    pub line_index: usize,    // 页内 i，表示 i 与 i+1 交换
    pub skill_a: String,
    pub skill_b: String,
    pub after_macro: String,
}

pub fn list_swap_candidates(macro_text: &str) -> Result<Vec<SwapCandidate>, String> {
    let cfg = parse_macro_text(macro_text).map_err(|e| e.to_string())?;
    let mut out: Vec<SwapCandidate> = Vec::new();
    for (pi, page) in cfg.pages.iter().enumerate() {
        if page.lines.len() < 2 { continue; }
        for i in 0..page.lines.len() - 1 {
            let mut cfg2 = clone_config(&cfg);
            cfg2.pages[pi].lines.swap(i, i + 1);
            let after_text = render_macro_text(&cfg2);
            out.push(SwapCandidate {
                page_index: pi,
                line_index: i,
                skill_a: page.lines[i].action.skill_name().to_string(),
                skill_b: page.lines[i + 1].action.skill_name().to_string(),
                after_macro: after_text,
            });
        }
    }
    Ok(out)
}

// ─── 条件收紧候选（v2-2.7）──────────────────────────────────────────
// 对每个数值条件（rage/bufftime/tbufftime），枚举 ±5/±10 的变体

#[derive(Debug, Serialize)]
pub struct TightenCandidate {
    pub page_index: usize,
    pub skill: String,
    pub original: String,     // 原条件原子 display_string
    pub tightened: String,    // 收紧后的原子
    pub after_macro: String,
}

fn tighten_leaf(c: &MacroCondition) -> Vec<MacroCondition> {
    match c {
        MacroCondition::Rage(op, v) => {
            let deltas = [5, 10, -5, -10];
            deltas.iter().filter_map(|d| {
                let nv = v + d;
                if nv < 0 || nv > 100 || nv == *v { return None; }
                // 收紧 = 缩小匹配范围：GtEq/Gt → 增大阈值，Lt → 减小阈值
                let is_tighter = match op {
                    CmpOp::GtEq | CmpOp::Gt => nv > *v,
                    CmpOp::Lt | CmpOp::LtEq => nv < *v,
                    _ => false,
                };
                if !is_tighter { return None; }
                Some(MacroCondition::Rage(*op, nv))
            }).collect()
        }
        MacroCondition::BuffTime(name, op, v) | MacroCondition::TBuffTime(name, op, v) => {
            let is_target = matches!(c, MacroCondition::TBuffTime(..));
            let deltas = [1.0, 2.0, -1.0, -2.0];
            deltas.iter().filter_map(|d| {
                let nv = v + d;
                if nv <= 0.0 || nv >= 60.0 || (nv - v).abs() < 0.01 { return None; }
                let is_tighter = match op {
                    CmpOp::Lt | CmpOp::LtEq => nv < *v,
                    CmpOp::Gt | CmpOp::GtEq => nv > *v,
                    _ => false,
                };
                if !is_tighter { return None; }
                if is_target {
                    Some(MacroCondition::TBuffTime(name.clone(), *op, nv))
                } else {
                    Some(MacroCondition::BuffTime(name.clone(), *op, nv))
                }
            }).collect()
        }
        _ => Vec::new(),
    }
}

fn replace_leaf(c: &MacroCondition, path: &[char], new_leaf: &MacroCondition) -> MacroCondition {
    if path.is_empty() { return new_leaf.clone(); }
    match c {
        MacroCondition::And(a, b) => {
            if path[0] == 'L' {
                MacroCondition::And(Box::new(replace_leaf(a, &path[1..], new_leaf)), Box::new((**b).clone()))
            } else {
                MacroCondition::And(Box::new((**a).clone()), Box::new(replace_leaf(b, &path[1..], new_leaf)))
            }
        }
        MacroCondition::Or(a, b) => {
            if path[0] == 'L' {
                MacroCondition::Or(Box::new(replace_leaf(a, &path[1..], new_leaf)), Box::new((**b).clone()))
            } else {
                MacroCondition::Or(Box::new((**a).clone()), Box::new(replace_leaf(b, &path[1..], new_leaf)))
            }
        }
        _ => new_leaf.clone(),
    }
}

pub fn list_tighten_candidates(macro_text: &str) -> Result<Vec<TightenCandidate>, String> {
    let cfg = parse_macro_text(macro_text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (pi, page) in cfg.pages.iter().enumerate() {
        for (li, line) in page.lines.iter().enumerate() {
            let Some(cond) = line.condition.as_ref() else { continue; };
            let paths = enumerate_leaf_paths(cond);
            for path in &paths {
                let Some(leaf) = node_at(cond, path) else { continue; };
                let variants = tighten_leaf(leaf);
                for var in variants {
                    let new_cond = replace_leaf(cond, path, &var);
                    let mut cfg2 = clone_config(&cfg);
                    cfg2.pages[pi].lines[li].condition = Some(new_cond);
                    let after_text = render_macro_text(&cfg2);
                    out.push(TightenCandidate {
                        page_index: pi,
                        skill: line.action.skill_name().to_string(),
                        original: leaf.display_string(),
                        tightened: var.display_string(),
                        after_macro: after_text,
                    });
                }
            }
        }
    }
    Ok(out)
}

pub fn list_prune_candidates(macro_text: &str) -> Result<Vec<PruneCandidate>, String> {
    let cfg = parse_macro_text(macro_text).map_err(|e| e.to_string())?;
    let mut out: Vec<PruneCandidate> = Vec::new();
    let mut global_rule_idx: usize = 0;
    for (pi, page) in cfg.pages.iter().enumerate() {
        for (li, line) in page.lines.iter().enumerate() {
            let Some(cond) = line.condition.as_ref() else { global_rule_idx += 1; continue; };
            let paths = enumerate_leaf_paths(cond);
            // 先找出所有 虚弱+盾飞 AND 对的父路径；这些父路径对应一个整体候选（整对一起删）
            let mut pair_parents: Vec<Vec<char>> = Vec::new();
            for path in &paths {
                if is_virtual_weak_pair_leaf(cond, path) {
                    let pp = path[..path.len()-1].to_vec();
                    if !pair_parents.contains(&pp) { pair_parents.push(pp); }
                }
            }
            for pp in &pair_parents {
                // 整对删除：把该 AND 子树作为一个"叶"从条件树里拿掉
                let new_cond = if pp.is_empty() {
                    None
                } else {
                    remove_leaf(cond, pp).map(|c| reorder_last_skill_first(&c))
                };
                let before_str = cond.display_string();
                let after_str = new_cond.as_ref()
                    .map(|c| c.display_string())
                    .unwrap_or_else(|| "(无)".to_string());
                let mut cfg2 = clone_config(&cfg);
                cfg2.pages[pi].lines[li].condition = new_cond;
                let after_text = render_macro_text(&cfg2);
                out.push(PruneCandidate {
                    rule_index: global_rule_idx,
                    page_index: pi,
                    skill: line.action.skill_name().to_string(),
                    atom: "tnobuff:虚弱&last_skill~=盾飞".to_string(),
                    before_cond: before_str,
                    after_cond: after_str,
                    after_macro: after_text,
                });
            }

            for path in &paths {
                // 保护 tnobuff:虚弱 & last_skill~=盾飞 这对，单侧删除跳过；整对由上面的循环处理
                if is_virtual_weak_pair_leaf(cond, path) { continue; }
                let atom_str = leaf_at(cond, path).unwrap_or_default();
                let new_cond = remove_leaf(cond, path).map(|c| reorder_last_skill_first(&c));
                let before_str = cond.display_string();
                let after_str = new_cond.as_ref()
                    .map(|c| c.display_string())
                    .unwrap_or_else(|| "(无)".to_string());
                // 克隆整个 cfg，改这一条，再渲染
                let mut cfg2 = clone_config(&cfg);
                cfg2.pages[pi].lines[li].condition = new_cond;
                let after_text = render_macro_text(&cfg2);
                out.push(PruneCandidate {
                    rule_index: global_rule_idx,
                    page_index: pi,
                    skill: line.action.skill_name().to_string(),
                    atom: atom_str,
                    before_cond: before_str,
                    after_cond: after_str,
                    after_macro: after_text,
                });
            }
            global_rule_idx += 1;
        }
    }
    Ok(out)
}
