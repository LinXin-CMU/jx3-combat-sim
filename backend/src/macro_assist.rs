//! Deterministic condition suggestions from observed active-cast snapshots.
//!
//! These scores describe matches in the supplied timeline. They do not estimate
//! whether a different skill could have been cast, or predict a changed macro.
use std::collections::{BTreeMap, BTreeSet};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::macro_engine::{CmpOp, MacroCondition};

const MAX_EVENTS: usize = 2048;
const MAX_ATOMS: usize = 4096;
const BEAM_WIDTH: usize = 40;

#[derive(Debug, Deserialize)]
pub struct AssistRequest {
    pub timeline: Vec<InputCast>,
    pub selection_start: usize,
    pub selection_end: usize,
    #[serde(default)]
    pub step: usize,
    #[serde(default)]
    pub options: AssistOptions,
}

#[derive(Debug, Deserialize)]
pub struct AssistOptions {
    #[serde(default = "default_max_terms")]
    pub max_terms: usize,
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
}

fn default_max_terms() -> usize {
    2
}
fn default_max_candidates() -> usize {
    120
}
impl Default for AssistOptions {
    fn default() -> Self {
        Self {
            max_terms: default_max_terms(),
            max_candidates: default_max_candidates(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InputCast {
    pub name: String,
    #[serde(default)]
    pub skill_id: Option<u32>,
    #[serde(default)]
    pub triggered: bool,
    #[serde(default)]
    pub cast_time: Option<f64>,
    #[serde(default)]
    pub state_before: Option<InputState>,
}

/// Optional fields stay unknown when an older or incomplete trace omits them.
#[derive(Debug, Deserialize, Default)]
pub struct InputState {
    pub time: Option<f64>,
    pub rage: Option<i32>,
    pub block_value: Option<i32>,
    pub berserk_value: Option<i32>,
    pub max_berserk_value: Option<i32>,
    pub buffs: Option<Vec<InputBuff>>,
    pub target_buffs: Option<Vec<InputBuff>>,
    pub skill_states: Option<Vec<InputSkillState>>,
}

#[derive(Debug, Deserialize)]
pub struct InputBuff {
    pub name: String,
    pub buff_id: Option<u32>,
    pub remaining: Option<f64>,
    pub stacks: Option<u32>,
    pub permanent: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InputSkillState {
    pub name: String,
    pub skill_id: Option<u32>,
    pub charges: Option<u32>,
    pub max_charges: Option<u32>,
    pub not_in_cd: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Selection {
    pub start_active_index: usize,
    pub end_active_index: usize,
    pub skill_names: Vec<String>,
    pub selected_step: usize,
    pub step_target: String,
}

#[derive(Debug, Serialize)]
pub struct Occurrence {
    pub start_active_index: usize,
    pub end_active_index: usize,
    pub step_active_index: usize,
    pub cast_time: Option<f64>,
    pub snapshot_time: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: String,
    pub expression: String,
    pub equivalent_expressions: Vec<String>,
    pub semantic_expression: String,
    pub macro_text: String,
    pub terms: usize,
    /// Same UTF-16 convention as the existing macro editor.
    pub chars: usize,
    pub condition_chars: usize,
    pub tp: usize,
    pub fp: usize,
    pub r#fn: usize,
    pub unknown_positive: usize,
    pub unknown_negative: usize,
    pub coverage: f64,
    pub precision: Option<f64>,
    pub false_match_rate: Option<f64>,
    pub f1: f64,
    pub same_skill_outside_combo_fp: usize,
    pub same_skill_outside_combo_unknown: usize,
    pub same_skill_outside_combo_f1: Option<f64>,
    pub matched_same_skill_outside_combo_indices: Vec<usize>,
    pub matched_positive_indices: Vec<usize>,
    pub matched_negative_indices: Vec<usize>,
    pub missed_positive_indices: Vec<usize>,
    pub unknown_positive_indices: Vec<usize>,
    pub unknown_negative_indices: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchSummary {
    pub atoms_evaluated: usize,
    pub compounds_evaluated: usize,
    pub total_candidates: usize,
    pub returned_candidates: usize,
    pub candidates_truncated: bool,
    pub atoms_truncated: bool,
    pub max_terms: usize,
    pub beam_width: usize,
    pub exhaustive_compounds: bool,
}

#[derive(Debug, Serialize)]
pub struct AssistResult {
    pub selection: Selection,
    pub occurrences: Vec<Occurrence>,
    pub positives: Vec<usize>,
    pub negative_count: usize,
    pub same_skill_outside_combo_indices: Vec<usize>,
    pub same_skill_other_step_indices: Vec<usize>,
    pub other_skill_negative_indices: Vec<usize>,
    pub active_count: usize,
    pub missing_state_indices: Vec<usize>,
    pub candidates: Vec<Candidate>,
    pub limitations: Vec<String>,
    pub search: SearchSummary,
}

pub async fn handler(Json(request): Json<AssistRequest>) -> Response {
    match tokio::task::spawn_blocking(move || analyze(&request)).await {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(message)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": message})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"条件分析未完成，请缩短序列后重试。"})),
        )
            .into_response(),
    }
}

/// Whole-program scores are still observed condition matches, not executions.
#[derive(Debug, Serialize)]
pub struct ProgramLine {
    pub step: usize,
    pub candidate_id: String,
    pub expression: String,
    pub skill: String,
}

#[derive(Debug, Serialize)]
pub struct ProgramCandidate {
    pub macro_text: String,
    pub lines: Vec<ProgramLine>,
    pub matched_occurrences: usize,
    pub total_occurrences: usize,
    pub outside_matches: usize,
    pub priority_conflicts: usize,
    pub unknowns: usize,
    pub chars: usize,
}

#[derive(Debug, Serialize)]
pub struct ProgramResult {
    pub steps: Vec<AssistResult>,
    pub programs: Vec<ProgramCandidate>,
    pub limitations: Vec<String>,
}

pub async fn program_handler(Json(request): Json<AssistRequest>) -> Response {
    match tokio::task::spawn_blocking(move || analyze_program(&request)).await {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(message)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": message})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"组合方案分析未完成，请缩短序列后重试。"})),
        )
            .into_response(),
    }
}

const PROGRAM_STEP_CANDIDATES: usize = 96;
const PROGRAM_POOL_WIDTH: usize = 12;
const PROGRAM_BEAM_WIDTH: usize = 32;
const MAX_PROGRAMS: usize = 6;

struct ProgramChoice {
    step: usize,
    candidate: usize,
    skill: usize,
    truth: Vec<Truth>,
}

#[derive(Clone, Copy, Default)]
struct ProgramDecision {
    skill: Option<usize>,
    /// A prior unknown may take priority over the first observed true line.
    unknown: bool,
}

#[derive(Clone)]
struct ProgramBeam {
    lines: Vec<usize>,
    decisions: Vec<ProgramDecision>,
    outside: Vec<Truth>,
    matched: usize,
    correct: usize,
    conflicts: usize,
    unknowns: usize,
    outside_matches: usize,
    outside_unknowns: usize,
    chars: usize,
}

fn candidate_truth(candidate: &Candidate, count: usize) -> Vec<Truth> {
    let mut truth = vec![Truth::False; count];
    for &index in candidate
        .matched_positive_indices
        .iter()
        .chain(&candidate.matched_negative_indices)
    {
        truth[index] = Truth::True;
    }
    for &index in candidate
        .unknown_positive_indices
        .iter()
        .chain(&candidate.unknown_negative_indices)
    {
        truth[index] = Truth::Unknown;
    }
    truth
}

fn compare_programs(a: &ProgramBeam, b: &ProgramBeam) -> std::cmp::Ordering {
    b.matched
        .cmp(&a.matched)
        .then_with(|| b.correct.cmp(&a.correct))
        .then_with(|| a.conflicts.cmp(&b.conflicts))
        .then_with(|| a.unknowns.cmp(&b.unknowns))
        .then_with(|| {
            (a.outside_matches + a.outside_unknowns).cmp(&(b.outside_matches + b.outside_unknowns))
        })
        .then_with(|| a.outside_unknowns.cmp(&b.outside_unknowns))
        .then_with(|| a.chars.cmp(&b.chars))
        .then_with(|| a.lines.cmp(&b.lines))
}

/// Keep both individually ranked conditions and low cross-skill interference
/// conditions. A candidate which ranks lower in isolation can win jointly.
fn program_pool(result: &AssistResult) -> Vec<usize> {
    let mut pool: Vec<_> = (0..result.candidates.len().min(PROGRAM_POOL_WIDTH / 2)).collect();
    let mut compatible: Vec<_> = (0..result.candidates.len()).collect();
    compatible.sort_by(|&left, &right| {
        let a = &result.candidates[left];
        let b = &result.candidates[right];
        let cross_matches = |item: &Candidate| {
            item.matched_negative_indices
                .iter()
                .filter(|index| {
                    result
                        .other_skill_negative_indices
                        .binary_search(index)
                        .is_ok()
                })
                .count()
        };
        (a.r#fn + a.unknown_positive)
            .cmp(&(b.r#fn + b.unknown_positive))
            .then_with(|| cross_matches(a).cmp(&cross_matches(b)))
            .then_with(|| a.unknown_negative.cmp(&b.unknown_negative))
            .then_with(|| a.chars.cmp(&b.chars))
            .then_with(|| left.cmp(&right))
    });
    for index in compatible {
        if pool.len() >= PROGRAM_POOL_WIDTH {
            break;
        }
        if !pool.contains(&index) {
            pool.push(index);
        }
    }
    pool
}

fn score_program(
    beam: &mut ProgramBeam,
    event_skills: &[usize],
    combo: &[bool],
    included_skills: &BTreeSet<usize>,
    occurrences: &[Occurrence],
) {
    let correct: Vec<_> = beam
        .decisions
        .iter()
        .zip(event_skills)
        .map(|(decision, &skill)| !decision.unknown && decision.skill == Some(skill))
        .collect();
    beam.matched = occurrences
        .iter()
        .filter(|occurrence| {
            (occurrence.start_active_index..=occurrence.end_active_index)
                .filter(|&index| included_skills.contains(&event_skills[index]))
                .all(|index| correct[index])
        })
        .count();
    beam.correct = 0;
    beam.conflicts = 0;
    beam.unknowns = 0;
    for (index, decision) in beam.decisions.iter().enumerate() {
        if !combo[index] || !included_skills.contains(&event_skills[index]) {
            continue;
        }
        beam.correct += usize::from(correct[index]);
        beam.unknowns += usize::from(decision.unknown);
        beam.conflicts += usize::from(
            !decision.unknown
                && decision
                    .skill
                    .is_some_and(|skill| skill != event_skills[index]),
        );
    }
    beam.outside_matches = beam
        .outside
        .iter()
        .filter(|&&truth| truth == Truth::True)
        .count();
    beam.outside_unknowns = beam
        .outside
        .iter()
        .filter(|&&truth| truth == Truth::Unknown)
        .count();
}

pub fn analyze_program(request: &AssistRequest) -> Result<ProgramResult, String> {
    // The shared validation retains the 32-step, 2048-active-event and snapshot
    // budgets. The request's selected detail step does not restrict a program.
    let first = analyze_step(request, 0, PROGRAM_STEP_CANDIDATES)?;
    let step_count = first.selection.skill_names.len();
    let mut steps = vec![first];
    for step in 1..step_count {
        steps.push(analyze_step(request, step, PROGRAM_STEP_CANDIDATES)?);
    }
    let names = &steps[0].selection.skill_names;
    let mut skill_ids = BTreeMap::new();
    for name in names {
        let next = skill_ids.len();
        skill_ids.entry(name.as_str()).or_insert(next);
    }
    let event_skills: Vec<_> = request
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .map(|event| {
            skill_ids
                .get(event_name(event))
                .copied()
                .unwrap_or(usize::MAX)
        })
        .collect();
    let mut combo = vec![false; event_skills.len()];
    for occurrence in &steps[0].occurrences {
        combo[occurrence.start_active_index..=occurrence.end_active_index].fill(true);
    }
    let mut choices: Vec<ProgramChoice> = Vec::new();
    let mut choice_ids = BTreeMap::new();
    let mut pools = Vec::new();
    for (step, result) in steps.iter().enumerate() {
        let skill = skill_ids[result.selection.step_target.as_str()];
        let mut pool = Vec::new();
        for candidate in program_pool(result) {
            let truth = candidate_truth(&result.candidates[candidate], event_skills.len());
            // Only identical skill AND whole-timeline truth can share a line.
            // Repeated-skill steps have a union of positive samples, while
            // same-skill samples outside all occurrences remain negative.
            let id = *choice_ids.entry((skill, truth.clone())).or_insert_with(|| {
                let id = choices.len();
                choices.push(ProgramChoice {
                    step,
                    candidate,
                    skill,
                    truth,
                });
                id
            });
            if !pool.contains(&id) {
                pool.push(id);
            }
        }
        pools.push(pool);
    }
    let mut beam = vec![ProgramBeam {
        lines: Vec::new(),
        decisions: vec![ProgramDecision::default(); event_skills.len()],
        outside: vec![Truth::False; event_skills.len()],
        matched: 0,
        correct: 0,
        conflicts: 0,
        unknowns: 0,
        outside_matches: 0,
        outside_unknowns: 0,
        chars: 0,
    }];
    let mut included_skills = BTreeSet::new();
    for (step, pool) in pools.iter().enumerate() {
        included_skills.insert(skill_ids[names[step].as_str()]);
        let mut next = Vec::new();
        let mut seen = BTreeSet::new();
        for previous in &beam {
            for &choice_id in pool {
                let choice = &choices[choice_id];
                let candidate = &steps[choice.step].candidates[choice.candidate];
                // Appending and prepending explores line priority as well as
                // condition combinations, without factorial permutations.
                for prepend in [false, true] {
                    let mut item = previous.clone();
                    if !item.lines.contains(&choice_id) {
                        if prepend {
                            item.lines.insert(0, choice_id);
                        } else {
                            item.lines.push(choice_id);
                        }
                        item.chars += candidate.chars + usize::from(item.lines.len() > 1);
                        for (index, &truth) in choice.truth.iter().enumerate() {
                            let decision = &mut item.decisions[index];
                            if prepend || decision.skill.is_none() {
                                match truth {
                                    Truth::True => {
                                        decision.skill = Some(choice.skill);
                                        if prepend {
                                            decision.unknown = false;
                                        }
                                    }
                                    Truth::Unknown => decision.unknown = true,
                                    Truth::False => {}
                                }
                            }
                            if !combo[index] && event_skills[index] == choice.skill {
                                item.outside[index] = item.outside[index].or(truth);
                            }
                        }
                    }
                    if !seen.insert(item.lines.clone()) {
                        continue;
                    }
                    score_program(
                        &mut item,
                        &event_skills,
                        &combo,
                        &included_skills,
                        &steps[0].occurrences,
                    );
                    next.push(item);
                }
            }
        }
        next.sort_by(compare_programs);
        next.truncate(PROGRAM_BEAM_WIDTH);
        beam = next;
    }
    let programs = beam
        .into_iter()
        .take(MAX_PROGRAMS)
        .map(|item| {
            let mut text = Vec::new();
            let lines = item
                .lines
                .iter()
                .map(|&id| {
                    let choice = &choices[id];
                    let candidate = &steps[choice.step].candidates[choice.candidate];
                    text.push(candidate.macro_text.clone());
                    ProgramLine {
                        step: choice.step,
                        candidate_id: candidate.id.clone(),
                        expression: candidate.expression.clone(),
                        skill: names[choice.step].clone(),
                    }
                })
                .collect();
            ProgramCandidate {
                macro_text: text.join("\n"),
                lines,
                matched_occurrences: item.matched,
                total_occurrences: steps[0].occurrences.len(),
                outside_matches: item.outside_matches,
                priority_conflicts: item.conflicts,
                unknowns: item.unknowns,
                chars: item.chars,
            }
        })
        .collect();
    let limitations = vec![
        "模板联合匹配只检查释放前快照：每一步首个已知成立的宏行必须对应目标技能，且前面没有未知条件；不等于宏已复现组合。".into(),
        "行间冲突只表示前面的条件会先命中其他技能；未检查技能可释放性、GCD、延迟或宏执行阶段，必须用相同完整环境运行对照。".into(),
        "组合外同技能成立按快照去重：仅统计宏行技能与该处实际技能相同且条件成立的组合外样本，不代表实际误放。未知反例在排序中受惩罚。".into(),
        format!("每步保留最多 {PROGRAM_STEP_CANDIDATES} 个候选，从中取最多 {PROGRAM_POOL_WIDTH} 个进行联合搜索；每轮保留 {PROGRAM_BEAM_WIDTH} 个方案并尝试前后插入，返回最多 {MAX_PROGRAMS} 段，不保证全局最优。"),
        "相同技能且全部样本真值相同的行可合并；同技能在组合内其他步骤的正确命中不作为冲突，组合外同技能反例仍保留。".into(),
        "气劲条件只使用可识别名称或别名，气劲时间最多一位小数。整段可能超过游戏单页 128 字符限制，插入后需按编辑器分页规则检查。".into(),
    ];
    Ok(ProgramResult {
        steps,
        programs,
        limitations,
    })
}

fn base_name(name: &str) -> &str {
    name.split('·').next().unwrap_or(name)
}

fn event_name(event: &InputCast) -> &str {
    if matches!(event.skill_id, Some(90010..=90012)) {
        &event.name
    } else {
        base_name(&event.name)
    }
}

fn copyable(event: &InputCast) -> bool {
    event.skill_id != Some(90001)
        && !event.name.starts_with("__")
        && !event.name.starts_with("移除气劲")
        && !event.name.contains(['\n', '\r', '[', ']'])
        && !event.name.trim().is_empty()
}

pub fn analyze(request: &AssistRequest) -> Result<AssistResult, String> {
    analyze_step(request, request.step, request.options.max_candidates)
}

fn analyze_step(
    request: &AssistRequest,
    step: usize,
    max_candidates: usize,
) -> Result<AssistResult, String> {
    if request.timeline.len() > MAX_EVENTS * 16 {
        return Err("时间轴过长，请缩短模拟序列后分析。".into());
    }
    if !(1..=3).contains(&request.options.max_terms)
        || !(1..=1000).contains(&request.options.max_candidates)
    {
        return Err("候选设置超出范围：条件项数为 1–3，候选条数为 1–1000。".into());
    }
    let active: Vec<_> = request
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect();
    if active.len() > MAX_EVENTS {
        return Err("主动技能超过 2048 次，请缩短模拟序列后分析。".into());
    }
    if request.selection_start > request.selection_end || request.selection_end >= active.len() {
        return Err("所选技能已失效，请重新运行模拟并选择技能。".into());
    }
    let selected = &active[request.selection_start..=request.selection_end];
    if selected.len() > 32 || step >= selected.len() {
        return Err("最多选择 32 个连续技能，分析步骤必须在所选组合内。".into());
    }
    if selected.iter().any(|event| !copyable(event)) {
        return Err("移除气劲或虚拟操作不能作为可复制的施放宏技能，请只选择战斗技能。".into());
    }
    if active.iter().any(|event| event.name.chars().count() > 160) {
        return Err("时间轴中存在过长的技能名称。".into());
    }
    for state in active
        .iter()
        .filter_map(|event| event.state_before.as_ref())
    {
        if state.buffs.as_ref().is_some_and(|items| items.len() > 256)
            || state
                .target_buffs
                .as_ref()
                .is_some_and(|items| items.len() > 256)
            || state
                .skill_states
                .as_ref()
                .is_some_and(|items| items.len() > 256)
        {
            return Err("单个快照的 Buff 或技能条目超过 256 项。".into());
        }
        if state
            .buffs
            .iter()
            .chain(state.target_buffs.iter())
            .flatten()
            .any(|buff| buff.name.chars().count() > 160)
            || state
                .skill_states
                .iter()
                .flatten()
                .any(|skill| skill.name.chars().count() > 160)
        {
            return Err("快照中存在过长的 Buff 或技能名称。".into());
        }
    }
    let names: Vec<_> = selected
        .iter()
        .map(|event| event_name(event).to_string())
        .collect();
    let occurrences: Vec<_> = active
        .windows(names.len())
        .enumerate()
        .filter(|(_, window)| {
            window
                .iter()
                .zip(&names)
                .all(|(event, name)| event_name(event) == name)
        })
        .map(|(start, _)| Occurrence {
            start_active_index: start,
            end_active_index: start + names.len() - 1,
            step_active_index: start + step,
            cast_time: active[start + step].cast_time,
            snapshot_time: active[start + step]
                .state_before
                .as_ref()
                .and_then(|state| state.time),
        })
        .collect();
    let positives: Vec<_> = occurrences
        .iter()
        .map(|item| item.step_active_index)
        .collect();
    let positive_set: BTreeSet<_> = positives.iter().copied().collect();
    let target = names[step].clone();
    let combo_indices: BTreeSet<_> = occurrences
        .iter()
        .flat_map(|occurrence| occurrence.start_active_index..=occurrence.end_active_index)
        .collect();
    let mut same_skill_outside_combo_indices = Vec::new();
    let mut same_skill_other_step_indices = Vec::new();
    let mut other_skill_negative_indices = Vec::new();
    for (index, event) in active.iter().enumerate() {
        // An overlapping occurrence may make another occurrence's non-target
        // step positive. Positive membership always wins over negative groups.
        if positive_set.contains(&index) {
            continue;
        }
        if event_name(event) != target {
            other_skill_negative_indices.push(index);
        } else if combo_indices.contains(&index) {
            same_skill_other_step_indices.push(index);
        } else {
            same_skill_outside_combo_indices.push(index);
        }
    }
    let outside_set: BTreeSet<_> = same_skill_outside_combo_indices.iter().copied().collect();
    let rows: Vec<_> = active
        .iter()
        .enumerate()
        .map(|(index, event)| Sample {
            index,
            positive: positive_set.contains(&index),
            same_skill_outside_combo: outside_set.contains(&index),
            state: event.state_before.as_ref(),
            // Runtime last_skill always strips the middle-dot suffix, including
            // mist variants; matching/copying the action above preserves them.
            last_skill: index
                .checked_sub(1)
                .map(|previous| base_name(&active[previous].name)),
        })
        .collect();
    let (atoms, atoms_truncated) = enumerate_atoms(&rows);
    let atoms_evaluated = atoms.len();
    let mut scored: Vec<_> = atoms
        .into_iter()
        .filter_map(|condition| score(condition, &rows, &target, 1))
        .collect();
    // Keep a no-condition baseline so absent distinguishing evidence stays visible.
    if let Some(baseline) = score_unconditional(&rows, &target) {
        scored.push(baseline);
    }
    scored.sort_by(compare_scored);
    let beam: Vec<_> = distinct_beam(&scored, BEAM_WIDTH);
    let mut compounds_evaluated = 0;
    let mut seen: BTreeSet<String> = scored
        .iter()
        .map(|item| item.candidate.expression.clone())
        .collect();
    let mut previous_level = beam.clone();
    for terms in 2..=request.options.max_terms {
        let mut level = Vec::new();
        for atom in &beam {
            for tail in &previous_level {
                for and in [true, false] {
                    // A right-associated tree is exactly what the macro parser accepts.
                    let condition = if and {
                        MacroCondition::And(
                            Box::new(atom.condition.clone()),
                            Box::new(tail.condition.clone()),
                        )
                    } else {
                        MacroCondition::Or(
                            Box::new(atom.condition.clone()),
                            Box::new(tail.condition.clone()),
                        )
                    };
                    if !seen.insert(condition.display_string()) {
                        continue;
                    }
                    compounds_evaluated += 1;
                    if let Some(candidate) = score(condition, &rows, &target, terms) {
                        // A compound with the same observed truth table as either part
                        // adds characters without adding evidence.
                        if candidate.truth != atom.truth && candidate.truth != tail.truth {
                            level.push(candidate);
                        }
                    }
                }
            }
        }
        level.sort_by(compare_scored);
        previous_level = distinct_beam(&level, BEAM_WIDTH);
        scored.extend(level);
    }
    scored.sort_by(compare_scored);
    let mut distinct: Vec<Scored> = Vec::new();
    let mut truth_to_index = BTreeMap::new();
    for item in scored {
        if let Some(&index) = truth_to_index.get(&item.truth) {
            let representative: &mut Scored = &mut distinct[index];
            if !item.candidate.expression.is_empty() {
                representative
                    .candidate
                    .equivalent_expressions
                    .push(item.candidate.expression);
            }
        } else {
            truth_to_index.insert(item.truth.clone(), distinct.len());
            distinct.push(item);
        }
    }
    let total_candidates = distinct.len();
    distinct.truncate(max_candidates);
    let candidates: Vec<_> = distinct
        .into_iter()
        .map(|mut item| {
            fill_indices(&mut item.candidate, &rows, &item.truth);
            item.candidate
        })
        .collect();
    let missing_state_indices = rows
        .iter()
        .filter(|row| row.state.is_none())
        .map(|row| row.index)
        .collect();
    let mut limitations = vec![
        "覆盖率与误匹配只描述当前序列中主动释放前的状态；包含非主 GCD 技能，不采样被动伤害、每帧或等待区间。".into(),
        "误匹配指条件在所选组合步骤之外的快照为真；未检查目标技能当时能否释放，也不等于宏运行时一定误放。".into(),
        "组合严格按主动释放顺序匹配，包含重叠出现；只合并同名技能的·等级变体，后续段与雾海技能保留各自名称。".into(),
        "快照时间可能早于或晚于施放时间，也可能晚于宏条件求值（例如网络延迟或技能内部推进时间）；修改宏后仍需用相同完整环境重新模拟验证。".into(),
        "数值候选取当前样本的分界点，气劲时长阈值最多保留一位小数并按该阈值计算覆盖；组合使用有限搜索，不宣称穷尽所有宏表达式。".into(),
        "气劲条件使用宏可识别的名称或别名；只有内部数字 ID、没有可用名称的气劲不生成建议。".into(),
        "当前全部样本上命中与未知情况相同的条件合并展示，其他写法可展开；这些写法在其他战斗状态下可能不同。".into(),
        "缺失字段记为未知，不推测充能、冷却或资源；生命、目标数量及奇穴常量不用于推荐。旧版快照可能遗漏自身减益。".into(),
    ];
    if same_skill_outside_combo_indices.is_empty() {
        limitations.push("当前序列缺少该技能在匹配组合之外的施放证据，暂按全部负样本排序；不能据此确认条件能区分组合内外。".into());
    } else {
        limitations.push("优先区分当前步骤与同一技能在所有匹配组合之外的施放；组合内其他步骤及其他技能另列。组内未知样本在排序中受惩罚，不当作已排除的负例。".into());
    }
    if rows
        .iter()
        .any(|row| row.state.is_some_and(|state| state.skill_states.is_none()))
    {
        limitations.push("部分快照缺少完整技能状态，无法从旧的冷却显示文本推断满层充能或 GCD；相关样本记为未知。".into());
    }
    Ok(AssistResult {
        selection: Selection {
            start_active_index: request.selection_start,
            end_active_index: request.selection_end,
            skill_names: names,
            selected_step: step,
            step_target: target,
        },
        negative_count: active.len() - positives.len(),
        same_skill_outside_combo_indices,
        same_skill_other_step_indices,
        other_skill_negative_indices,
        active_count: active.len(),
        occurrences,
        positives,
        missing_state_indices,
        search: SearchSummary {
            atoms_evaluated,
            compounds_evaluated,
            total_candidates,
            returned_candidates: candidates.len(),
            candidates_truncated: total_candidates > candidates.len(),
            atoms_truncated,
            max_terms: request.options.max_terms,
            beam_width: BEAM_WIDTH,
            exhaustive_compounds: false,
        },
        candidates,
        limitations,
    })
}

struct Sample<'a> {
    index: usize,
    positive: bool,
    same_skill_outside_combo: bool,
    state: Option<&'a InputState>,
    last_skill: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Truth {
    False,
    True,
    Unknown,
}
impl Truth {
    fn from_option(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::True,
            Some(false) => Self::False,
            None => Self::Unknown,
        }
    }
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}

// Preferred spellings only: identities always come from the runtime resolver.
// Keeping IDs out of this list prevents an assistant-specific identity mapping.
const PREFERRED_BUFF_NAMES: &[&str] = &[
    "血怒",
    "血怒·惊涌",
    "劫化",
    "锋鸣",
    "坚定",
    "狂绝",
    "怒炎",
    "激昂",
    "无惧",
    "嗜血",
    "援戈",
    "天下宏愿",
    "麟光甲",
    "麟黯",
    "盾飞",
    "虚弱",
    "流血",
    "卷云",
    "步残",
    "缓深",
    "血誓",
    "以血盟誓",
    "战绝",
    "盾挡",
    "千山盾挡",
    "寒啸千军",
    "蔑视",
    "振奋",
    "寒甲",
    "坚铁",
    "盾威",
    "威压",
    "惊涌",
    "角斗场",
    "严阵",
    "铁骨",
    "宿敌",
    "擎盾",
    "擎刀",
];

fn buff_name(buff: &InputBuff) -> Option<String> {
    let resolved = crate::macro_eval::buff_name_to_id(&buff.name);
    let id = buff.buff_id.or(resolved)?;
    if let Some(name) = PREFERRED_BUFF_NAMES
        .iter()
        .find(|name| crate::macro_eval::buff_name_to_id(name) == Some(id))
    {
        return Some((*name).to_string());
    }
    // A newly supported runtime name can be used even before it is added to
    // the preference list. Unnamed/internal IDs never become numeric macros.
    (resolved == Some(id) && buff.name.parse::<u32>().is_err()).then(|| buff.name.clone())
}

fn find_buff<'a>(buffs: Option<&'a Vec<InputBuff>>, name: &str) -> Option<Option<&'a InputBuff>> {
    let list = buffs?;
    let id = crate::macro_eval::buff_name_to_id(name)?;
    Some(list.iter().rev().find(|buff| {
        buff.buff_id
            .or_else(|| crate::macro_eval::buff_name_to_id(&buff.name))
            == Some(id)
    }))
}

fn buff_time(buff: &InputBuff) -> Option<f64> {
    if buff.permanent == Some(true) {
        return Some(f64::MAX);
    }
    let remaining = buff
        .remaining
        .filter(|value| value.is_finite() && *value >= 0.0)?;
    // In EventState, active entries with remaining == 0 denote permanent buffs.
    Some(if remaining == 0.0 && buff.permanent != Some(false) {
        f64::MAX
    } else {
        remaining
    })
}

fn skill_state<'a>(state: &'a InputState, name: &str) -> Option<&'a InputSkillState> {
    state.skill_states.as_ref()?.iter().find(|skill| {
        skill.name == name
            || name
                .parse::<u32>()
                .ok()
                .is_some_and(|id| skill.skill_id == Some(id))
    })
}

fn eval(condition: &MacroCondition, row: &Sample<'_>) -> Truth {
    use MacroCondition::*;
    match condition {
        And(left, right) => return eval(left, row).and(eval(right, row)),
        Or(left, right) => return eval(left, row).or(eval(right, row)),
        LastSkill(name) => return Truth::from_option(Some(row.last_skill == Some(name.as_str()))),
        LastSkillNot(name) => {
            return Truth::from_option(Some(row.last_skill != Some(name.as_str())))
        }
        _ => {}
    }
    let Some(state) = row.state else {
        return Truth::Unknown;
    };
    let result = match condition {
        Rage(op, value) => state.rage.map(|resource| op.compare_i32(resource, *value)),
        Energy(op, value) => state
            .block_value
            .map(|resource| op.compare_i32(resource, *value)),
        Berserk(op, value) => state
            .berserk_value
            .map(|resource| op.compare_i32(resource, *value)),
        Buff(name) => find_buff(state.buffs.as_ref(), name).map(|buff| buff.is_some()),
        NoBuff(name) => find_buff(state.buffs.as_ref(), name).map(|buff| buff.is_none()),
        BuffStack(name, op, value) => find_buff(state.buffs.as_ref(), name)
            .and_then(|buff| match buff {
                Some(buff) => buff.stacks,
                None => Some(0),
            })
            .map(|stacks| op.compare_u32(stacks, *value)),
        BuffTime(name, op, value) => {
            find_buff(state.buffs.as_ref(), name).and_then(|buff| match buff {
                Some(buff) => buff_time(buff).map(|time| op.compare_f64(time, *value)),
                None => Some(false),
            })
        }
        TBuff(name) => find_buff(state.target_buffs.as_ref(), name).map(|buff| buff.is_some()),
        TnoBuff(name) => find_buff(state.target_buffs.as_ref(), name).map(|buff| buff.is_none()),
        TBuffTime(name, op, value) => {
            find_buff(state.target_buffs.as_ref(), name).and_then(|buff| match buff {
                Some(buff) => buff_time(buff).map(|time| op.compare_f64(time, *value)),
                None => Some(false),
            })
        }
        SkillNotInCd(name) => skill_state(state, name).and_then(|skill| skill.not_in_cd),
        SkillEnergy(name, op, value) => skill_state(state, name)
            .and_then(|skill| skill.charges)
            .map(|charges| op.compare_u32(charges, *value)),
        // These atoms lack corresponding observations in this DTO. Never let an
        // unsupported condition silently become true as in the legacy generator.
        Life(..) | NearbyEnemy(..) | SkillExists(..) | SkillNotExists(..) => None,
        LastSkill(..) | LastSkillNot(..) | And(..) | Or(..) => unreachable!(),
    };
    Truth::from_option(result)
}

fn enumerate_atoms(rows: &[Sample<'_>]) -> (Vec<MacroCondition>, bool) {
    use MacroCondition::*;
    let mut truncated = false;
    let mut candidates = BTreeMap::<String, MacroCondition>::new();
    let mut add = |condition: MacroCondition| {
        candidates
            .entry(condition.display_string())
            .or_insert(condition);
    };
    let ops = [
        CmpOp::Eq,
        CmpOp::Neq,
        CmpOp::Lt,
        CmpOp::LtEq,
        CmpOp::Gt,
        CmpOp::GtEq,
    ];
    for resource in 0..3 {
        let mut thresholds = BTreeSet::new();
        for state in rows.iter().filter_map(|row| row.state) {
            let value = match resource {
                0 => state.rage,
                1 => state.block_value,
                _ => state.berserk_value,
            };
            if let Some(value) = value {
                thresholds.insert(value);
                if value > 0 {
                    thresholds.insert(value - 1);
                }
                if value < 100_000 {
                    thresholds.insert(value + 1);
                }
            }
            if resource == 2 {
                if let Some(cap) = state.max_berserk_value {
                    thresholds.insert(cap);
                }
            }
        }
        for threshold in thin_values(thresholds, 49, &mut truncated)
            .into_iter()
            .filter(|value| (0..=100_000).contains(value))
        {
            for op in ops {
                add(match resource {
                    0 => Rage(op, threshold),
                    1 => Energy(op, threshold),
                    _ => Berserk(op, threshold),
                });
            }
        }
    }
    for target in [false, true] {
        let mut buffs: BTreeMap<String, (BTreeSet<u32>, BTreeSet<i64>)> = BTreeMap::new();
        for state in rows.iter().filter_map(|row| row.state) {
            let list = if target {
                &state.target_buffs
            } else {
                &state.buffs
            };
            for buff in list.iter().flatten() {
                let Some(name) = buff_name(buff).filter(|name| name.len() <= 256) else {
                    continue;
                };
                if !buffs.contains_key(&name) && buffs.len() >= 128 {
                    truncated = true;
                    continue;
                }
                let (stacks, times) = buffs.entry(name).or_default();
                stacks.insert(0);
                if let Some(value) = buff.stacks.filter(|value| *value <= 100_000) {
                    stacks.insert(value);
                }
                if let Some(time) = buff_time(buff).filter(|time| *time < 86_400.0) {
                    // Search on a 0.1-second grid, including both surrounding
                    // boundaries. Snapshots keep their original precision;
                    // score() evaluates these rendered thresholds, not `time`.
                    let lower = (time * 10.0).floor() as i64;
                    let upper = (time * 10.0).ceil() as i64;
                    for tenth in lower.saturating_sub(1)..=upper.saturating_add(1) {
                        times.insert(tenth.max(0));
                    }
                }
            }
        }
        for (name, (stacks, times)) in buffs {
            add(if target {
                TBuff(name.clone())
            } else {
                Buff(name.clone())
            });
            add(if target {
                TnoBuff(name.clone())
            } else {
                NoBuff(name.clone())
            });
            if !target {
                for stack in thin_values(stacks, 12, &mut truncated) {
                    for op in ops {
                        add(BuffStack(name.clone(), op, stack));
                    }
                }
            }
            for tenth in thin_values(times, 24, &mut truncated) {
                for op in ops {
                    add(if target {
                        TBuffTime(name.clone(), op, tenth as f64 / 10.0)
                    } else {
                        BuffTime(name.clone(), op, tenth as f64 / 10.0)
                    });
                }
            }
        }
    }
    let mut skills: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for state in rows.iter().filter_map(|row| row.state) {
        for skill in state.skill_states.iter().flatten() {
            if skill.name.len() > 256 {
                continue;
            }
            if !skills.contains_key(&skill.name) && skills.len() >= 128 {
                truncated = true;
                continue;
            }
            let values = skills.entry(skill.name.clone()).or_default();
            if let Some(charges) = skill.charges.filter(|value| *value <= 1000) {
                values.insert(charges);
                values.insert(0);
            }
            if let Some(max) = skill.max_charges.filter(|value| *value <= 1000) {
                values.insert(max);
            }
        }
    }
    for (name, values) in skills {
        add(SkillNotInCd(name.clone()));
        for value in thin_values(values, 12, &mut truncated) {
            for op in ops {
                add(SkillEnergy(name.clone(), op, value));
            }
        }
    }
    for name in rows
        .iter()
        .filter_map(|row| row.last_skill)
        .collect::<BTreeSet<_>>()
    {
        if !name.starts_with("__") && !name.starts_with("移除气劲") {
            add(LastSkill(name.to_string()));
            add(LastSkillNot(name.to_string()));
        }
    }
    truncated |= candidates.len() > MAX_ATOMS;
    // Round-robin across semantic families before applying the global budget;
    // thousands of buff-time boundaries must not displace sun or last_skill.
    let mut families: BTreeMap<u8, Vec<MacroCondition>> = BTreeMap::new();
    for condition in candidates.into_values() {
        let family = match &condition {
            Rage(..) => 0,
            Energy(..) => 1,
            Berserk(..) => 2,
            Buff(..) | NoBuff(..) => 3,
            BuffStack(..) => 4,
            BuffTime(..) => 5,
            TBuff(..) | TnoBuff(..) => 6,
            TBuffTime(..) => 7,
            SkillEnergy(..) => 8,
            SkillNotInCd(..) => 9,
            LastSkill(..) | LastSkillNot(..) => 10,
            _ => 11,
        };
        families.entry(family).or_default().push(condition);
    }
    let mut iterators: Vec<_> = families
        .into_values()
        .map(|items| items.into_iter())
        .collect();
    let mut out = Vec::new();
    while out.len() < MAX_ATOMS {
        let previous = out.len();
        for items in &mut iterators {
            if out.len() == MAX_ATOMS {
                break;
            }
            if let Some(condition) = items.next() {
                out.push(condition);
            }
        }
        if out.len() == previous {
            break;
        }
    }
    (out, truncated)
}

fn thin_values<T: Copy + Ord>(values: BTreeSet<T>, limit: usize, truncated: &mut bool) -> Vec<T> {
    let values: Vec<_> = values.into_iter().collect();
    if values.len() <= limit {
        return values;
    }
    *truncated = true;
    (0..limit)
        .map(|index| values[index * (values.len() - 1) / (limit - 1)])
        .collect()
}

struct Scored {
    condition: MacroCondition,
    candidate: Candidate,
    truth: Vec<Truth>,
}

#[derive(Clone)]
struct BeamEntry {
    condition: MacroCondition,
    truth: Vec<Truth>,
}

fn distinct_beam(scored: &[Scored], count: usize) -> Vec<BeamEntry> {
    let mut seen = BTreeSet::new();
    scored
        .iter()
        .filter(|item| item.candidate.terms > 0 && seen.insert(item.truth.clone()))
        .take(count)
        .map(|item| BeamEntry {
            condition: item.condition.clone(),
            truth: item.truth.clone(),
        })
        .collect()
}

fn score(
    condition: MacroCondition,
    rows: &[Sample<'_>],
    target: &str,
    terms: usize,
) -> Option<Scored> {
    if !has_copyable_buff_atoms(&condition) {
        return None;
    }
    let expression = condition.display_string();
    let macro_text = format!("/cast [{expression}] {target}");
    if macro_text.encode_utf16().count() > 128 {
        return None;
    }
    let parsed = crate::macro_parser::parse_macro_text(&macro_text).ok()?;
    let parsed_condition = parsed.pages.first()?.lines.first()?.condition.as_ref()?;
    // Reject expressions whose rendering changes meaning (including currently
    // unsupported name~=value forms); scores always use the real parsed AST.
    if parsed_condition.semantic_string() != condition.semantic_string() {
        return None;
    }
    let rendered = crate::macro_parser::render_macro_text(&parsed);
    let rendered_config = crate::macro_parser::parse_macro_text(&rendered).ok()?;
    if rendered_config
        .pages
        .first()?
        .lines
        .first()?
        .condition
        .as_ref()?
        .semantic_string()
        != parsed_condition.semantic_string()
    {
        return None;
    }
    let truth: Vec<_> = rows.iter().map(|row| eval(parsed_condition, row)).collect();
    if !rows
        .iter()
        .zip(&truth)
        .any(|(row, value)| row.positive && *value == Truth::True)
    {
        return None;
    }
    let candidate = summarize(
        parsed_condition,
        expression,
        macro_text,
        terms,
        rows,
        &truth,
    );
    Some(Scored {
        condition: parsed_condition.clone(),
        candidate,
        truth,
    })
}

/// This restricts suggestions only; the runtime parser/evaluator still accepts
/// existing numeric-ID macros and higher-precision user-authored thresholds.
fn has_copyable_buff_atoms(condition: &MacroCondition) -> bool {
    use MacroCondition::*;
    let readable = |name: &str| {
        name.parse::<u32>().is_err() && crate::macro_eval::buff_name_to_id(name).is_some()
    };
    match condition {
        Buff(name) | NoBuff(name) | BuffStack(name, ..) | TBuff(name) | TnoBuff(name) => {
            readable(name)
        }
        BuffTime(name, _, value) | TBuffTime(name, _, value) => {
            let rendered = value.to_string();
            readable(name)
                && value.is_finite()
                && !rendered.contains(['e', 'E'])
                && rendered
                    .split_once('.')
                    .is_none_or(|(_, decimal)| decimal.len() <= 1)
        }
        And(left, right) | Or(left, right) => {
            has_copyable_buff_atoms(left) && has_copyable_buff_atoms(right)
        }
        _ => true,
    }
}

fn score_unconditional(rows: &[Sample<'_>], target: &str) -> Option<Scored> {
    let macro_text = format!("/cast {target}");
    crate::macro_parser::parse_macro_text(&macro_text).ok()?;
    let truth = vec![Truth::True; rows.len()];
    // The condition is unused for this baseline and is excluded from the beam.
    let condition = MacroCondition::Rage(CmpOp::GtEq, 0);
    let mut candidate = summarize(&condition, String::new(), macro_text, 0, rows, &truth);
    candidate.id = "unconditional".into();
    candidate.semantic_expression = "无条件".into();
    Some(Scored {
        condition,
        candidate,
        truth,
    })
}

fn summarize(
    condition: &MacroCondition,
    expression: String,
    macro_text: String,
    terms: usize,
    rows: &[Sample<'_>],
    truth: &[Truth],
) -> Candidate {
    let mut tp = 0;
    let mut fp = 0;
    let mut missed = 0;
    let mut unknown_p = 0;
    let mut unknown_n = 0;
    let mut negatives = 0;
    let mut outside_count = 0;
    let mut outside_fp = 0;
    let mut outside_unknown = 0;
    for (row, result) in rows.iter().zip(truth) {
        if !row.positive {
            negatives += 1;
        }
        if row.same_skill_outside_combo {
            outside_count += 1;
            outside_fp += usize::from(*result == Truth::True);
            outside_unknown += usize::from(*result == Truth::Unknown);
        }
        match (row.positive, result) {
            (true, Truth::True) => tp += 1,
            (false, Truth::True) => fp += 1,
            (true, Truth::False) => missed += 1,
            (true, Truth::Unknown) => unknown_p += 1,
            (false, Truth::Unknown) => unknown_n += 1,
            _ => {}
        }
    }
    let positives = rows.len() - negatives;
    let precision = (tp + fp > 0).then(|| tp as f64 / (tp + fp) as f64);
    let coverage = tp as f64 / positives.max(1) as f64;
    let f1 = 2.0 * tp as f64 / (2 * tp + fp + missed + unknown_p).max(1) as f64;
    Candidate {
        id: expression.clone(),
        semantic_expression: condition.semantic_string(),
        condition_chars: expression.encode_utf16().count(),
        expression,
        equivalent_expressions: Vec::new(),
        chars: macro_text.encode_utf16().count(),
        macro_text,
        terms,
        tp,
        fp,
        r#fn: missed,
        unknown_positive: unknown_p,
        unknown_negative: unknown_n,
        coverage,
        precision,
        false_match_rate: (negatives > unknown_n)
            .then(|| fp as f64 / (negatives - unknown_n) as f64),
        f1,
        same_skill_outside_combo_fp: outside_fp,
        same_skill_outside_combo_unknown: outside_unknown,
        same_skill_outside_combo_f1: (outside_count > 0)
            .then(|| 2.0 * tp as f64 / (2 * tp + outside_fp + missed + unknown_p).max(1) as f64),
        matched_same_skill_outside_combo_indices: Vec::new(),
        matched_positive_indices: Vec::new(),
        matched_negative_indices: Vec::new(),
        missed_positive_indices: Vec::new(),
        unknown_positive_indices: Vec::new(),
        unknown_negative_indices: Vec::new(),
    }
}

fn fill_indices(candidate: &mut Candidate, rows: &[Sample<'_>], truth: &[Truth]) {
    for (row, result) in rows.iter().zip(truth) {
        if row.same_skill_outside_combo && *result == Truth::True {
            candidate
                .matched_same_skill_outside_combo_indices
                .push(row.index);
        }
        match (row.positive, result) {
            (true, Truth::True) => candidate.matched_positive_indices.push(row.index),
            (false, Truth::True) => candidate.matched_negative_indices.push(row.index),
            (true, Truth::False) => candidate.missed_positive_indices.push(row.index),
            (true, Truth::Unknown) => candidate.unknown_positive_indices.push(row.index),
            (false, Truth::Unknown) => candidate.unknown_negative_indices.push(row.index),
            _ => {}
        }
    }
}

fn compare_scored(left: &Scored, right: &Scored) -> std::cmp::Ordering {
    let a = &left.candidate;
    let b = &right.candidate;
    // Prefer an observed complete separator, then balance precision and coverage.
    let exact = |item: &Candidate| {
        item.fp == 0 && item.r#fn == 0 && item.unknown_positive == 0 && item.unknown_negative == 0
    };
    compare_same_skill_outside(a, b).then_with(|| {
        exact(b)
            .cmp(&exact(a))
            .then_with(|| b.f1.total_cmp(&a.f1))
            .then_with(|| {
                (a.unknown_positive + a.unknown_negative)
                    .cmp(&(b.unknown_positive + b.unknown_negative))
            })
            .then_with(|| a.fp.cmp(&b.fp))
            .then_with(|| b.tp.cmp(&a.tp))
            .then_with(|| a.terms.cmp(&b.terms))
            .then_with(|| a.chars.cmp(&b.chars))
            .then_with(|| a.expression.cmp(&b.expression))
    })
}

fn compare_same_skill_outside(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    if a.same_skill_outside_combo_f1.is_none() || b.same_skill_outside_combo_f1.is_none() {
        return std::cmp::Ordering::Equal;
    }
    let exact = |item: &Candidate| {
        item.same_skill_outside_combo_fp == 0
            && item.same_skill_outside_combo_unknown == 0
            && item.r#fn == 0
            && item.unknown_positive == 0
    };
    // The published group F1 retains its stated formula. For ranking, count an
    // unknown outside sample conservatively so missing data cannot masquerade
    // as a correctly excluded outside cast.
    let conservative_f1 = |item: &Candidate| {
        2.0 * item.tp as f64
            / (2 * item.tp
                + item.same_skill_outside_combo_fp
                + item.same_skill_outside_combo_unknown
                + item.r#fn
                + item.unknown_positive)
                .max(1) as f64
    };
    exact(b)
        .cmp(&exact(a))
        .then_with(|| conservative_f1(b).total_cmp(&conservative_f1(a)))
        .then_with(|| {
            a.same_skill_outside_combo_unknown
                .cmp(&b.same_skill_outside_combo_unknown)
        })
}

#[cfg(test)]
#[path = "../tests/macro_assist/conditions.rs"]
mod tests;
