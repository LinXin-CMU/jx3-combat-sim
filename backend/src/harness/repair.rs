//! Candidate edits are hypotheses. The compiler replays every accepted change.
use std::collections::BTreeSet;

use serde_json::json;

use super::Candidate;
use crate::{
    macro_engine::{CmpOp, MacroAction, MacroCondition},
    CastEvent, EventState, SimulateResponse,
};

pub struct Proposal {
    pub text: String,
    pub origin: String,
}

pub fn action_name(event: &CastEvent) -> String {
    super::alignment::skill_key(event)
}

/// The simulator accepts unbracketed conditions for backwards compatibility.
/// Harness exports always use game macro brackets, and that exact text is
/// replayed and counted against the page limit. Keep comments/page separators
/// so normalization cannot silently make an oversized user page appear valid.
pub fn game_macro_text(text: &str) -> Result<String, String> {
    crate::macro_parser::parse_macro_text(text).map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#page") {
            lines.push(line.to_owned());
            continue;
        }
        let parsed = crate::macro_parser::parse_macro_text(trimmed).map_err(|e| e.to_string())?;
        let statement = parsed
            .pages
            .first()
            .and_then(|p| p.lines.first())
            .ok_or("宏语句为空")?;
        let command = if statement.action.is_fcast() {
            "/fcast"
        } else {
            "/cast"
        };
        let condition = statement
            .condition
            .as_ref()
            .map(|condition| format!("[{}] ", condition.display_string()))
            .unwrap_or_default();
        lines.push(format!(
            "{command} {condition}{}",
            statement.action.skill_name()
        ));
    }
    let mut normalized = lines.join("\n");
    if text.ends_with('\n') {
        normalized.push('\n');
    }
    Ok(normalized)
}

fn edited(
    text: &str,
    page: usize,
    line: Option<usize>,
    replacement: &str,
    before: Option<usize>,
) -> Option<String> {
    let mut config = crate::macro_parser::parse_macro_text(text).ok()?;
    let mut parsed = crate::macro_parser::parse_macro_text(replacement).ok()?;
    let new_line = parsed.pages.first_mut()?.lines.remove(0);
    let lines = &mut config.pages.get_mut(page)?.lines;
    match before {
        Some(index) => {
            let mut index = index.min(lines.len());
            if let Some(old) = line.filter(|i| *i < lines.len()) {
                lines.remove(old);
                if old < index {
                    index = index.saturating_sub(1);
                }
            }
            lines.insert(index.min(lines.len()), new_line);
        }
        None => match line {
            Some(index) if index < lines.len() => lines[index] = new_line,
            _ => lines.push(new_line),
        },
    }
    game_macro_text(&crate::macro_parser::render_macro_text(&config)).ok()
}

pub fn proposals(
    best: &Candidate,
    baseline: &SimulateResponse,
    actual: &SimulateResponse,
    window: f64,
    stopped: impl Fn() -> bool,
) -> Vec<Proposal> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::from([best.macro_text.clone()]);
    let mut add = |text: String, origin: &str| {
        let Ok(text) = game_macro_text(&text) else {
            return;
        };
        if out.len() < 48
            && text.len() <= 32768
            && seen.insert(text.clone())
            && crate::macro_parser::parse_macro_text(&text).is_ok()
        {
            out.push(Proposal {
                text,
                origin: origin.into(),
            });
        }
    };
    let Ok(config) = crate::macro_parser::parse_macro_text(&best.macro_text) else {
        return out;
    };
    // Keep observed false casts in the negative pool. They are never promoted
    // to target examples just because their skill name matches the target.
    let threshold_edits = threshold_edits(best, baseline, actual, window, &stopped);
    for edit in &threshold_edits {
        if let Some(text) = render_edits(&best.macro_text, &[edit]) {
            add(text, "counterexample_threshold");
        }
    }
    let mut combinations = 0;
    'pairs: for (i, left) in threshold_edits.iter().enumerate() {
        for right in threshold_edits.iter().skip(i + 1) {
            if (left.page, left.line) == (right.page, right.line) {
                continue;
            }
            if let Some(text) = render_edits(&best.macro_text, &[left, right]) {
                add(text, "joint_counterexample_thresholds");
                combinations += 1;
                if combinations == 4 {
                    break 'pairs;
                }
            }
        }
    }
    let focus = best.alignment.summary.first_difference.and_then(|i| {
        best.alignment.rows[i..]
            .iter()
            .find_map(|r| r.reference_index)
    });
    if let Some(index) = focus {
        let event = &baseline.timeline[index];
        let target = action_name(event);
        let page = best
            .diagnosis
            .as_ref()
            .and_then(|d| d.page)
            .unwrap_or(1)
            .saturating_sub(1)
            .min(config.pages.len().saturating_sub(1));
        if let Some(macro_page) = config.pages.get(page) {
            let own = macro_page
                .lines
                .iter()
                .position(|line| line.action.skill_name() == target);
            let selected = best
                .diagnosis
                .as_ref()
                .and_then(|d| d.selected_line)
                .unwrap_or(1)
                .saturating_sub(1);
            let command = if own
                .is_some_and(|i| matches!(macro_page.lines[i].action, MacroAction::FCast(_)))
            {
                "/fcast"
            } else {
                "/cast"
            };
            if let Some(i) = own.filter(|i| *i > selected) {
                let line = &macro_page.lines[i];
                let condition = line
                    .condition
                    .as_ref()
                    .map(|c| format!("[{}] ", c.display_string()))
                    .unwrap_or_default();
                if let Some(text) = edited(
                    &best.macro_text,
                    page,
                    own,
                    &format!("{command} {condition}{target}"),
                    Some(selected),
                ) {
                    add(text, "priority_move");
                }
            }
            // An unconditional contrast also helps recover a completely blocked initial macro.
            if let Some(text) = edited(
                &best.macro_text,
                page,
                own,
                &format!("{command} {target}"),
                Some(selected),
            ) {
                add(text, "unconditional_contrast");
            }
            let active_index = baseline.timeline[..index]
                .iter()
                .filter(|e| {
                    !e.triggered
                        && e.skill_id != 90001
                        && !e.name.starts_with("__")
                        && !e.name.starts_with("移除气劲")
                        && super::alignment::within_window(e, window)
                })
                .count();
            let input = baseline.timeline.iter().filter(|e| super::alignment::within_window(e, window)).map(|e| json!({
                "name":e.name,"skill_id":e.skill_id,"triggered":e.triggered,"cast_time":e.cast_time,"state_before":e.state_before
            })).collect::<Vec<_>>();
            for terms in [1, 2] {
                if stopped() {
                    break;
                }
                let Ok(request) = serde_json::from_value::<crate::macro_assist::AssistRequest>(
                    json!({
                        "timeline":input,"selection_start":active_index,"selection_end":active_index,
                        "options":{"max_terms":terms,"max_candidates":24}
                    }),
                ) else {
                    continue;
                };
                let Ok(suggestions) = crate::macro_assist::analyze(&request) else {
                    continue;
                };
                for candidate in suggestions
                    .candidates
                    .iter()
                    .filter(|c| terms == 1 || c.terms > 1)
                    .take(6)
                {
                    let condition = if candidate.expression.is_empty() {
                        String::new()
                    } else {
                        format!("[{}] ", candidate.expression)
                    };
                    let replacement = format!("{command} {condition}{target}");
                    if let Some(text) = edited(&best.macro_text, page, own, &replacement, None) {
                        add(text, "snapshot_condition");
                    }
                    if own.is_none_or(|i| i > selected) {
                        if let Some(text) =
                            edited(&best.macro_text, page, own, &replacement, Some(selected))
                        {
                            add(text, "snapshot_condition_and_priority");
                        }
                    }
                }
            }
        }
    }
    if stopped() {
        return out;
    }
    // Existing candidate algorithms are reused; their old DPS/cast-count scoring is not.
    if let Ok(candidates) = crate::macro_prune::list_prune_candidates(&best.macro_text) {
        for candidate in candidates.into_iter().take(8) {
            add(candidate.after_macro, "prune_condition");
        }
    }
    if let Ok(candidates) = crate::macro_prune::list_swap_candidates(&best.macro_text) {
        for candidate in candidates.into_iter().take(8) {
            add(candidate.after_macro, "adjacent_swap");
        }
    }
    if let Ok(candidates) = crate::macro_prune::list_tighten_candidates(&best.macro_text) {
        for candidate in candidates.into_iter().take(8) {
            add(candidate.after_macro, "tighten_condition");
        }
    }
    out
}

struct ThresholdEdit {
    page: usize,
    line: usize,
    condition: MacroCondition,
}

fn numeric(condition: &MacroCondition) -> Option<(CmpOp, f64, f64)> {
    use MacroCondition::*;
    match condition {
        Rage(op, n) | Energy(op, n) | Berserk(op, n) => Some((*op, f64::from(*n), 1.0)),
        BuffTime(_, op, n) | TBuffTime(_, op, n) => Some((*op, *n, 0.1)),
        BuffStack(_, op, n) | SkillEnergy(_, op, n) => Some((*op, f64::from(*n), 1.0)),
        _ => None,
    }
}

fn observed(condition: &MacroCondition, state: &EventState) -> Option<f64> {
    use MacroCondition::*;
    match condition {
        Rage(..) => Some(f64::from(state.rage)),
        Energy(..) => state.block_value.map(f64::from),
        Berserk(..) => state.berserk_value.map(f64::from),
        BuffStack(name, ..) => Some(
            state
                .buffs
                .iter()
                .find(|b| &b.name == name)
                .map_or(0.0, |b| f64::from(b.stacks)),
        ),
        BuffTime(name, ..) | TBuffTime(name, ..) => {
            let buffs = if matches!(condition, TBuffTime(..)) {
                &state.target_buffs
            } else {
                &state.buffs
            };
            // Missing is unknown/false, never fabricated as a zero timer.
            buffs.iter().find(|b| &b.name == name).map(|b| {
                if b.remaining == 0.0 {
                    f64::INFINITY
                } else {
                    b.remaining
                }
            })
        }
        SkillEnergy(name, ..) => state
            .skill_states
            .as_ref()?
            .iter()
            .find(|s| &s.name == name)?
            .charges
            .map(f64::from),
        _ => None,
    }
}

fn replace_number(condition: &MacroCondition, number: f64) -> MacroCondition {
    use MacroCondition::*;
    match condition {
        Rage(op, _) => Rage(*op, number as i32),
        Energy(op, _) => Energy(*op, number as i32),
        Berserk(op, _) => Berserk(*op, number as i32),
        BuffTime(name, op, _) => BuffTime(name.clone(), *op, number),
        TBuffTime(name, op, _) => TBuffTime(name.clone(), *op, number),
        BuffStack(name, op, _) => BuffStack(name.clone(), *op, number as u32),
        SkillEnergy(name, op, _) => SkillEnergy(name.clone(), *op, number as u32),
        other => other.clone(),
    }
}

fn paths<'a>(
    condition: &'a MacroCondition,
    path: Vec<bool>,
    output: &mut Vec<(Vec<bool>, &'a MacroCondition)>,
) {
    match condition {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            let mut left = path.clone();
            left.push(false);
            paths(a, left, output);
            let mut right = path;
            right.push(true);
            paths(b, right, output);
        }
        _ if numeric(condition).is_some() => output.push((path, condition)),
        _ => {}
    }
}

fn with_atom(
    condition: &MacroCondition,
    path: &[bool],
    replacement: MacroCondition,
) -> MacroCondition {
    if path.is_empty() {
        return replacement;
    }
    use MacroCondition::*;
    match condition {
        And(a, b) => {
            if path[0] {
                And(a.clone(), Box::new(with_atom(b, &path[1..], replacement)))
            } else {
                And(Box::new(with_atom(a, &path[1..], replacement)), b.clone())
            }
        }
        Or(a, b) => {
            if path[0] {
                Or(a.clone(), Box::new(with_atom(b, &path[1..], replacement)))
            } else {
                Or(Box::new(with_atom(a, &path[1..], replacement)), b.clone())
            }
        }
        _ => condition.clone(),
    }
}

fn boundary_values(
    atom: &MacroCondition,
    positive: &[Option<f64>],
    negative: &[Option<f64>],
) -> Vec<f64> {
    let Some((op, current, unit)) = numeric(atom) else {
        return Vec::new();
    };
    let mut values = BTreeSet::new();
    for value in positive
        .iter()
        .chain(negative)
        .flatten()
        .filter(|v| v.is_finite())
    {
        let floor = (value / unit).floor() as i64;
        let ceil = (value / unit).ceil() as i64;
        for n in [floor - 1, floor, ceil, ceil + 1] {
            if (0..=1_000_000).contains(&n) {
                values.insert(n);
            }
        }
    }
    let loss = |threshold: f64| {
        positive
            .iter()
            .filter(|v| v.is_none_or(|v| !op.compare_f64(v, threshold)))
            .count() as f64
            / positive.len().max(1) as f64
            + negative
                .iter()
                .filter(|v| v.is_some_and(|v| op.compare_f64(v, threshold)))
                .count() as f64
                / negative.len().max(1) as f64
    };
    let choose = |above: bool| {
        let mut values = values
            .iter()
            .map(|n| *n as f64 * unit)
            .filter(|v| {
                if above {
                    *v > current + 1e-6
                } else {
                    *v < current - 1e-6
                }
            })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| {
            loss(*a)
                .total_cmp(&loss(*b))
                .then_with(|| (a - current).abs().total_cmp(&(b - current).abs()))
                .then_with(|| a.total_cmp(b))
        });
        values.truncate(2);
        values
    };
    let lower = choose(false);
    let upper = choose(true);
    (0..2)
        .flat_map(|i| {
            [lower.get(i).copied(), upper.get(i).copied()]
                .into_iter()
                .flatten()
        })
        .collect()
}

fn threshold_edits(
    best: &Candidate,
    baseline: &SimulateResponse,
    actual: &SimulateResponse,
    window: f64,
    stopped: &impl Fn() -> bool,
) -> Vec<ThresholdEdit> {
    let Ok(config) = crate::macro_parser::parse_macro_text(&best.macro_text) else {
        return Vec::new();
    };
    let extras = best
        .alignment
        .rows
        .iter()
        .filter(|r| r.kind == "extra")
        .filter_map(|r| r.actual_index)
        .collect::<BTreeSet<_>>();
    let mut related = BTreeSet::new();
    if let Some(diagnosis) = &best.diagnosis {
        related.extend(diagnosis.target_skill.iter().cloned());
        related.extend(diagnosis.selected_skill.iter().cloned());
    }
    for &index in &extras {
        if let Some(event) = actual.timeline.get(index) {
            related.insert(action_name(event));
        }
    }
    let mut buckets = Vec::new();
    for (page, macro_page) in config.pages.iter().enumerate() {
        for (line, statement) in macro_page.lines.iter().enumerate() {
            if stopped() {
                return Vec::new();
            }
            let skill = statement.action.skill_name();
            if !related.contains(skill) {
                continue;
            }
            let Some(condition) = &statement.condition else {
                continue;
            };
            let mut atoms = Vec::new();
            paths(condition, Vec::new(), &mut atoms);
            let mut edits = Vec::new();
            for (path, atom) in atoms.into_iter().take(8) {
                let mut positives = Vec::new();
                let mut negatives = Vec::new();
                for event in baseline.timeline.iter().filter(|e| {
                    super::alignment::is_active(e) && super::alignment::within_window(e, window)
                }) {
                    let value = event.state_before.as_ref().and_then(|s| observed(atom, s));
                    if action_name(event) == skill {
                        positives.push(value);
                    } else {
                        negatives.push(value);
                    }
                }
                for &index in &extras {
                    if let Some(event) = actual
                        .timeline
                        .get(index)
                        .filter(|e| action_name(e) == skill)
                    {
                        negatives.push(event.state_before.as_ref().and_then(|s| observed(atom, s)));
                    }
                }
                if positives.is_empty() && negatives.is_empty() {
                    continue;
                }
                for number in boundary_values(atom, &positives, &negatives) {
                    edits.push(ThresholdEdit {
                        page,
                        line,
                        condition: with_atom(condition, &path, replace_number(atom, number)),
                    });
                }
            }
            if !edits.is_empty() {
                buckets.push(edits);
            }
        }
    }
    let mut edits = Vec::new();
    // Round-robin across lines prevents one long rule consuming the entire budget.
    for _ in 0..4 {
        for bucket in &mut buckets {
            if !bucket.is_empty() {
                edits.push(bucket.remove(0));
                if edits.len() == 8 {
                    return edits;
                }
            }
        }
    }
    edits
}

fn render_edits(text: &str, edits: &[&ThresholdEdit]) -> Option<String> {
    let mut config = crate::macro_parser::parse_macro_text(text).ok()?;
    for edit in edits {
        config
            .pages
            .get_mut(edit.page)?
            .lines
            .get_mut(edit.line)?
            .condition = Some(edit.condition.clone());
    }
    game_macro_text(&crate::macro_parser::render_macro_text(&config)).ok()
}
