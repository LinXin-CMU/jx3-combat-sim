//! Phase 3 规则池
//!
//! 每条 Rule = 一行宏命令 + 局部 tunables + 页签 + 是否锁定 + 来源。
//! `RulePool` 负责把 baseline 宏（original 规则）和前端传入的 candidate 规则
//! 合并成一个可搜索的注册表，并提供给 struct_ga 渲染 / 变异使用。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::macro_engine::{MacroAction, MacroCondition, MacroLine};
use crate::macro_parser::parse_macro_text;
use crate::Stance;

use super::analyze::{LeafKind, TunableParam};

// ─────────────────────────────────────────────────────────────────────────────
// 类型
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PageTag {
    Shield,
    Blade,
    Either,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    Original,
    Candidate,
}

/// 单条规则
pub struct Rule {
    pub id: String,
    pub name: String,
    pub page: PageTag,
    pub source: RuleSource,
    pub locked: bool,
    /// 原始宏行文本（展示用）
    pub line_text: String,
    /// 解析后的 MacroLine（不存 Clone，clone_macro_line 手动复制）
    pub parsed: MacroLine,
    /// 该行内的可调阈值（visit_idx 从 0 计）
    pub tunables: Vec<TunableParam>,
}

pub struct RulePool {
    /// id → Rule
    pub rules: HashMap<String, Rule>,
    /// 擎盾页初始顺序（original 规则 + candidate 规则）
    pub shield_default_order: Vec<String>,
    /// 擎刀页初始顺序
    pub blade_default_order: Vec<String>,
    /// 擎盾页默认启用集合（original=默认启用，candidate=默认禁用）
    pub shield_default_enabled: std::collections::HashSet<String>,
    /// 擎刀页默认启用集合
    pub blade_default_enabled: std::collections::HashSet<String>,
}

// 前端传入的 candidate 规则
#[derive(Debug, Clone, Deserialize)]
pub struct CandidateRuleInput {
    pub id: Option<String>,
    pub name: String,
    pub page: PageTag,
    pub line_text: String,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub initial_enabled: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// 构建
// ─────────────────────────────────────────────────────────────────────────────

/// 根据 baseline 宏文本构建 original 规则列表，并追加前端传入的 candidate 规则。
/// 首条 original 规则默认锁定（不可禁用 / 不可移位）。
pub fn build_pool(
    baseline_text: &str,
    candidates: &[CandidateRuleInput],
    lock_first_of_each_page: bool,
) -> Result<RulePool, String> {
    let cfg = parse_macro_text(baseline_text).map_err(|e| e.to_string())?;

    let mut rules: HashMap<String, Rule> = HashMap::new();
    let mut shield_order: Vec<String> = Vec::new();
    let mut blade_order: Vec<String> = Vec::new();
    let mut shield_enabled: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut blade_enabled: std::collections::HashSet<String> = std::collections::HashSet::new();

    for page in &cfg.pages {
        let page_tag = match page.stance_filter {
            Some(Stance::Shield) => PageTag::Shield,
            Some(Stance::Blade) => PageTag::Blade,
            // 非姿态/其他页（general/wall/any/not_wall）暂时归为 shield
            _ => PageTag::Shield,
        };
        for (idx, line) in page.lines.iter().enumerate() {
            let id = match page_tag {
                PageTag::Shield => format!("S{}", shield_order.len() + 1),
                PageTag::Blade => format!("B{}", blade_order.len() + 1),
                PageTag::Either => format!("E{}", rules.len() + 1),
            };
            let name = rule_display_name(line);
            let line_text = serialize_line(line);
            let tunables = extract_line_tunables(line, page_tag_name(page_tag), idx + 1, &line_text);
            let is_first = match page_tag {
                PageTag::Shield => shield_order.is_empty(),
                PageTag::Blade => blade_order.is_empty(),
                _ => false,
            };
            let parsed = clone_macro_line(line);
            rules.insert(
                id.clone(),
                Rule {
                    id: id.clone(),
                    name,
                    page: page_tag,
                    source: RuleSource::Original,
                    locked: lock_first_of_each_page && is_first,
                    line_text,
                    parsed,
                    tunables,
                },
            );
            match page_tag {
                PageTag::Shield => {
                    shield_order.push(id.clone());
                    shield_enabled.insert(id);
                }
                PageTag::Blade => {
                    blade_order.push(id.clone());
                    blade_enabled.insert(id);
                }
                _ => {}
            }
        }
    }

    // 追加 candidate 规则
    for (i, c) in candidates.iter().enumerate() {
        let id = c.id.clone().unwrap_or_else(|| format!("C{}", i + 1));
        if rules.contains_key(&id) {
            return Err(format!("候选规则 id 冲突: {}", id));
        }
        let parsed_cfg = parse_macro_text(&c.line_text)
            .map_err(|e| format!("候选规则 {} 解析失败: {}", id, e))?;
        let Some(first_page) = parsed_cfg.pages.first() else {
            return Err(format!("候选规则 {} 空", id));
        };
        let Some(first_line) = first_page.lines.first() else {
            return Err(format!("候选规则 {} 无有效行", id));
        };
        let parsed = clone_macro_line(first_line);
        let line_text = serialize_line(first_line);
        let page_label = match c.page {
            PageTag::Shield => "shield",
            PageTag::Blade => "blade",
            PageTag::Either => "either",
        };
        let tunables = extract_line_tunables(first_line, page_label, 1, &line_text);
        let target_page = c.page;
        rules.insert(
            id.clone(),
            Rule {
                id: id.clone(),
                name: c.name.clone(),
                page: target_page,
                source: RuleSource::Candidate,
                locked: c.locked,
                line_text,
                parsed,
                tunables,
            },
        );
        match target_page {
            PageTag::Shield => {
                shield_order.push(id.clone());
                if c.initial_enabled { shield_enabled.insert(id); }
            }
            PageTag::Blade => {
                blade_order.push(id.clone());
                if c.initial_enabled { blade_enabled.insert(id); }
            }
            PageTag::Either => {
                // Either 规则在两页都可候选，默认放擎盾页末尾
                shield_order.push(id.clone());
                if c.initial_enabled { shield_enabled.insert(id); }
            }
        }
    }

    Ok(RulePool {
        rules,
        shield_default_order: shield_order,
        blade_default_order: blade_order,
        shield_default_enabled: shield_enabled,
        blade_default_enabled: blade_enabled,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 辅助：line 序列化 / tunables 抽取
// ─────────────────────────────────────────────────────────────────────────────

pub fn serialize_line(line: &MacroLine) -> String {
    let prefix = if line.action.is_fcast() { "/fcast" } else { "/cast" };
    let cond = line
        .condition
        .as_ref()
        .map(|c| format!(" [{}]", c.display_string()))
        .unwrap_or_default();
    let skill = line.action.skill_name();
    format!("{}{} {}", prefix, cond, skill)
}

fn page_tag_name(p: PageTag) -> &'static str {
    match p {
        PageTag::Shield => "shield",
        PageTag::Blade => "blade",
        PageTag::Either => "either",
    }
}

fn rule_display_name(line: &MacroLine) -> String {
    line.action.skill_name().to_string()
}

/// 抽取一行里的可调阈值（visit_idx 从 0 计）
fn extract_line_tunables(
    line: &MacroLine,
    page: &str,
    line_no: usize,
    preview: &str,
) -> Vec<TunableParam> {
    let mut out = Vec::new();
    let Some(cond) = &line.condition else { return out };
    let mut leaves = Vec::new();
    walk_leaves(cond, &mut leaves);
    for (vi, leaf) in leaves.iter().enumerate() {
        if let Some(p) = leaf_to_tunable(leaf, page, line_no, vi, preview) {
            out.push(p);
        }
    }
    out
}

enum Leaf<'a> {
    Rage(crate::macro_engine::CmpOp, i32),
    Life(crate::macro_engine::CmpOp, f64),
    BuffTime(&'a str, crate::macro_engine::CmpOp, f64),
    TBuffTime(&'a str, crate::macro_engine::CmpOp, f64),
    SkillEnergy(&'a str, crate::macro_engine::CmpOp, u32),
    NearbyEnemy(crate::macro_engine::CmpOp, u32),
}

fn walk_leaves<'a>(cond: &'a MacroCondition, out: &mut Vec<Leaf<'a>>) {
    use MacroCondition::*;
    match cond {
        And(a, b) | Or(a, b) => { walk_leaves(a, out); walk_leaves(b, out); }
        Rage(op, v) => out.push(Leaf::Rage(*op, *v)),
        Life(op, v) => out.push(Leaf::Life(*op, *v)),
        BuffTime(n, op, v) => out.push(Leaf::BuffTime(n.as_str(), *op, *v)),
        TBuffTime(n, op, v) => out.push(Leaf::TBuffTime(n.as_str(), *op, *v)),
        SkillEnergy(n, op, v) => out.push(Leaf::SkillEnergy(n.as_str(), *op, *v)),
        NearbyEnemy(op, v) => out.push(Leaf::NearbyEnemy(*op, *v)),
        _ => {}
    }
}

fn leaf_to_tunable(
    leaf: &Leaf,
    page: &str,
    line_no: usize,
    vi: usize,
    preview: &str,
) -> Option<TunableParam> {
    use crate::macro_engine::CmpOp;
    let op_raw = match leaf {
        Leaf::Rage(op, _)
        | Leaf::Life(op, _)
        | Leaf::BuffTime(_, op, _)
        | Leaf::TBuffTime(_, op, _)
        | Leaf::SkillEnergy(_, op, _)
        | Leaf::NearbyEnemy(op, _) => *op,
    };
    let op = match op_raw {
        CmpOp::Gt | CmpOp::Lt | CmpOp::GtEq | CmpOp::LtEq => op_raw.symbol().to_string(),
        CmpOp::Eq | CmpOp::Neq => return None,
    };
    let id = format!("{}_L{}_V{}", page, line_no, vi);

    let (key, original, min, max, step, kind, leaf_kind) = match leaf {
        Leaf::Rage(_, v) => {
            let fv = *v as f64;
            let mn = (fv - 30.0).max(0.0);
            let mx = (fv + 30.0).min(100.0);
            ("rage".to_string(), fv, mn, mx, 1.0, "int", LeafKind::Rage)
        }
        Leaf::Life(_, v) => {
            let mn = (*v - 30.0).max(0.0);
            let mx = (*v + 30.0).min(100.0);
            ("life".to_string(), *v, mn, mx, 1.0, "int", LeafKind::Life)
        }
        Leaf::BuffTime(n, _, v) => {
            let mn = round_step((*v - 5.0).max(0.0), 0.1);
            let mx = round_step(*v + 5.0, 0.1);
            (format!("bufftime:{}", n), *v, mn, mx, 0.1, "float", LeafKind::BuffTime)
        }
        Leaf::TBuffTime(n, _, v) => {
            let mn = round_step((*v - 5.0).max(0.0), 0.1);
            let mx = round_step(*v + 5.0, 0.1);
            (format!("tbufftime:{}", n), *v, mn, mx, 0.1, "float", LeafKind::TBuffTime)
        }
        Leaf::SkillEnergy(n, _, v) => {
            let fv = *v as f64;
            let mn = (fv - 3.0).max(0.0);
            let mx = (fv + 3.0).min(10.0);
            (format!("skill_energy:{}", n), fv, mn, mx, 1.0, "int", LeafKind::SkillEnergy)
        }
        Leaf::NearbyEnemy(_, v) => {
            let fv = *v as f64;
            let mn = (fv - 5.0).max(0.0);
            let mx = (fv + 5.0).min(30.0);
            ("nearby_enemy".to_string(), fv, mn, mx, 1.0, "int", LeafKind::NearbyEnemy)
        }
    };

    Some(TunableParam {
        id,
        page: page.to_string(),
        line: line_no,
        rule_preview: preview.to_string(),
        key,
        op,
        original,
        suggested_min: min,
        suggested_max: max,
        suggested_step: step,
        kind: kind.to_string(),
        page_idx: 0,
        line_idx: 0,
        visit_idx: vi,
        leaf_kind,
    })
}

fn round_step(v: f64, step: f64) -> f64 {
    if step <= 0.0 { return v; }
    let scale = (1.0 / step).round();
    (v * scale).round() / scale
}

// ─────────────────────────────────────────────────────────────────────────────
// MacroLine / MacroCondition 深拷贝（macro_engine 没 derive Clone）
// ─────────────────────────────────────────────────────────────────────────────

pub fn clone_macro_line(l: &MacroLine) -> MacroLine {
    MacroLine {
        condition: l.condition.as_ref().map(clone_condition),
        action: match &l.action {
            MacroAction::Cast(n) => MacroAction::Cast(n.clone()),
            MacroAction::FCast(n) => MacroAction::FCast(n.clone()),
        },
    }
}

pub fn clone_condition(c: &MacroCondition) -> MacroCondition {
    use MacroCondition::*;
    match c {
        Rage(op, v) => Rage(*op, *v),
        Life(op, v) => Life(*op, *v),
        Buff(n) => Buff(n.clone()),
        NoBuff(n) => NoBuff(n.clone()),
        BuffTime(n, op, v) => BuffTime(n.clone(), *op, *v),
        BuffStack(n, op, v) => BuffStack(n.clone(), *op, *v),
        TBuff(n) => TBuff(n.clone()),
        TnoBuff(n) => TnoBuff(n.clone()),
        TBuffTime(n, op, v) => TBuffTime(n.clone(), *op, *v),
        SkillNotInCd(n) => SkillNotInCd(n.clone()),
        SkillExists(id) => SkillExists(*id),
        SkillNotExists(id) => SkillNotExists(*id),
        SkillEnergy(n, op, v) => SkillEnergy(n.clone(), *op, *v),
        LastSkill(n) => LastSkill(n.clone()),
        LastSkillNot(n) => LastSkillNot(n.clone()),
        NearbyEnemy(op, v) => NearbyEnemy(*op, *v),
        And(a, b) => And(Box::new(clone_condition(a)), Box::new(clone_condition(b))),
        Or(a, b) => Or(Box::new(clone_condition(a)), Box::new(clone_condition(b))),
    }
}
