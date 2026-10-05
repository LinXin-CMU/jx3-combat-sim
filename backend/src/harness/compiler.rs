//! Bounded, deterministic macro compilation against one frozen simulator scene.
//! Candidate generation is heuristic; only full simulator replays can validate it.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use super::alignment::{self, Alignment};
use super::contract::MacroCompileRequestV1;
use crate::agent::{AgentRuntime, ScenarioSnapshotV1};
use crate::{SimulateRequest, SimulateResponse};

#[path = "repair.rs"]
mod repair;

/// Shared export boundary: replay and page accounting must use this exact text.
pub(super) fn normalize_game_macro(text: &str) -> Result<String, String> {
    repair::game_macro_text(text)
}

pub(super) fn generate_macro_seed(response: &SimulateResponse) -> Result<String, String> {
    generated(response)
}

#[derive(Clone, Serialize)]
pub struct CompileProgress {
    pub phase: String,
    pub message: String,
    pub simulations: u32,
    pub best: Option<Candidate>,
}

#[derive(Clone, Serialize)]
pub struct Metrics {
    pub dps: f64,
    pub total_damage: f64,
    pub fight_time: f64,
    pub active_casts: usize,
    pub skipped_count: usize,
}

#[derive(Clone, Serialize)]
pub struct Page {
    pub stance: String,
    pub chars: usize,
    pub limit: usize,
    pub within_limit: bool,
}

#[derive(Clone, Serialize)]
pub struct Diagnosis {
    pub kind: String,
    pub message: String,
    pub time: f64,
    pub target_skill: Option<String>,
    pub selected_skill: Option<String>,
    pub page: Option<usize>,
    pub selected_line: Option<usize>,
    pub trace: Value,
}

#[derive(Clone, Serialize)]
pub struct ActionView {
    pub index: usize,
    pub name: String,
    pub skill_id: u32,
    pub cast_time: f64,
}

#[derive(Clone, Serialize)]
pub struct Candidate {
    pub macro_text: String,
    pub pages: Vec<Page>,
    pub page_constraints_passed: bool,
    pub verified: bool,
    pub full_snapshots: bool,
    pub reproduced: bool,
    pub alignment: Alignment,
    pub first_difference: Option<Value>,
    pub reference_actions: Vec<ActionView>,
    pub actual_actions: Vec<ActionView>,
    pub diagnosis: Option<Diagnosis>,
    pub metrics: Metrics,
    /// String avoids loss of u64 precision in JavaScript.
    pub fingerprint: String,
    pub origin: String,
}

#[derive(Clone, Serialize)]
pub struct Trial {
    pub round: u32,
    pub origin: String,
    pub accepted: bool,
    pub summary: super::alignment::AlignmentSummary,
    pub page_constraints_passed: bool,
    pub fingerprint: String,
}

#[derive(Clone, Serialize)]
pub struct CompileResult {
    pub schema_version: String,
    pub stop_reason: String,
    pub simulations: u32,
    pub rounds: u32,
    pub elapsed_ms: u64,
    pub scenario_hash: String,
    pub acceptance_scope: String,
    pub window_seconds: f64,
    pub baseline: Option<Metrics>,
    pub baseline_fingerprint: Option<String>,
    pub best: Option<Candidate>,
    pub history: Vec<Trial>,
    pub failed_candidates: u32,
    pub limitations: Vec<String>,
}

struct Budget<'a> {
    request: &'a MacroCompileRequestV1,
    cancel: &'a AtomicBool,
    started: Instant,
    simulations: u32,
}

impl Budget<'_> {
    fn stop(&self) -> Option<&'static str> {
        if self.cancel.load(Ordering::Relaxed) {
            Some("cancelled")
        } else if self.started.elapsed().as_millis() >= u128::from(self.request.wall_time_ms) {
            Some("time_budget")
        } else if self.simulations >= self.request.max_simulations {
            Some("budget_exhausted")
        } else {
            None
        }
    }

    fn reserve(&mut self) -> Result<(), &'static str> {
        if let Some(reason) = self.stop() {
            return Err(reason);
        }
        self.simulations += 1;
        Ok(())
    }
}

fn run(runtime: &AgentRuntime, request: &SimulateRequest) -> SimulateResponse {
    let c = runtime.context();
    crate::simulate_core(
        request,
        c.skills,
        c.game_version,
        c.mount,
        c.constants,
        c.recipes,
        c.team_buffs,
        c.formations,
    )
}

/// Preserve every environmental field; only replace the rotation and output mode.
fn macro_request(base: &SimulateRequest, text: &str, duration: f64) -> SimulateRequest {
    let mut request = base.clone();
    request.sequence = vec!["__macro__".into(); (duration / 0.25).ceil() as usize + 20];
    request.macro_text = Some(text.to_owned());
    request.macro_duration = Some(duration);
    request.channel_ticks.clear();
    request.timing_offsets.clear();
    request.solidified_casts.clear();
    request.qijin_buffs.clear();
    request.lite = false;
    request.lite_keep_timeline = false;
    request
}

fn metrics(response: &SimulateResponse, window: f64) -> Metrics {
    Metrics {
        dps: response.dps,
        total_damage: response.total_damage,
        fight_time: response.fight_time,
        skipped_count: response.skipped.len(),
        active_casts: response
            .timeline
            .iter()
            .filter(|e| alignment::is_active(e) && alignment::within_window(e, window))
            .count(),
    }
}

pub fn pages(text: &str) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut stance = "any".to_owned();
    let mut body = String::new();
    let flush = |pages: &mut Vec<Page>, stance: &str, body: &str| {
        let chars = body.trim_end().encode_utf16().count();
        if chars > 0 {
            pages.push(Page {
                stance: stance.into(),
                chars,
                limit: 128,
                within_limit: chars <= 128,
            });
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("#page") {
            flush(&mut pages, &stance, &body);
            body.clear();
            stance = if rest.trim().is_empty() {
                "any"
            } else {
                rest.trim()
            }
            .into();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut pages, &stance, &body);
    pages
}

fn constraints(pages: &[Page], max_pages: usize) -> bool {
    !pages.is_empty() && pages.len() <= max_pages && pages.iter().all(|p| p.within_limit)
}

fn make_candidate(
    text: String,
    origin: String,
    response: &SimulateResponse,
    baseline: &SimulateResponse,
    window: f64,
    request: &MacroCompileRequestV1,
) -> Result<Candidate, String> {
    let alignment = alignment::align(
        &baseline.timeline,
        &response.timeline,
        window,
        request.time_tolerance,
    )?;
    let pages = pages(&text);
    let passed = constraints(&pages, request.max_pages);
    let full_snapshots = [&baseline.timeline, &response.timeline]
        .into_iter()
        .all(|timeline| {
            timeline
                .iter()
                .filter(|e| alignment::is_active(e) && alignment::within_window(e, window))
                .all(|e| e.state_before.is_some())
        });
    let reproduced = passed
        && full_snapshots
        && response.skipped.is_empty()
        && alignment.summary.missing == 0
        && alignment.summary.extra == 0
        && alignment.summary.changed == 0;
    let first_difference = alignment.summary.first_difference.map(|i| {
        let row = &alignment.rows[i];
        let view = |event: &crate::CastEvent| {
            json!({"name":event.name,"skill_id":event.skill_id,
            "cast_time":event.cast_time,"state_before":event.state_before})
        };
        json!({"kind":row.kind,
            "reference":row.reference_index.and_then(|i| baseline.timeline.get(i)).map(view),
            "actual":row.actual_index.and_then(|i| response.timeline.get(i)).map(view)})
    });
    let actions = |response: &SimulateResponse| {
        response
            .timeline
            .iter()
            .enumerate()
            .filter(|(_, e)| alignment::is_active(e) && alignment::within_window(e, window))
            .map(|(index, e)| ActionView {
                index,
                name: e.name.clone(),
                skill_id: e.skill_id,
                cast_time: e.cast_time,
            })
            .collect()
    };
    Ok(Candidate {
        macro_text: text,
        pages,
        page_constraints_passed: passed,
        verified: true,
        full_snapshots,
        reproduced,
        alignment,
        first_difference,
        diagnosis: None,
        metrics: metrics(response, window),
        reference_actions: actions(baseline),
        actual_actions: actions(response),
        fingerprint: response.fingerprint.to_string(),
        origin,
    })
}

fn score(candidate: &Candidate, max_pages: usize) -> (usize, usize, usize, usize, u64, usize) {
    let overflow = candidate
        .pages
        .iter()
        .map(|p| p.chars.saturating_sub(128))
        .sum::<usize>()
        + candidate.pages.len().saturating_sub(max_pages) * 128
        + usize::from(candidate.pages.is_empty()) * 128;
    let s = &candidate.alignment.summary;
    (
        overflow,
        candidate.metrics.skipped_count,
        s.missing + s.extra,
        s.changed,
        (s.time_error * 1_000_000.0).round() as u64,
        candidate.macro_text.encode_utf16().count(),
    )
}

fn generated(baseline: &SimulateResponse) -> Result<String, String> {
    let casts = baseline.timeline.iter().map(|event| serde_json::from_value::<crate::macro_gen::InputCast>(json!({
        "name":event.name,"triggered":event.triggered,"is_main":event.is_main,"state_before":event.state_before
    }))).collect::<Result<Vec<_>, _>>().map_err(|e| format!("无法读取模板状态：{e}"))?;
    let options = serde_json::from_value::<crate::macro_gen::GenOptions>(json!({}))
        .map_err(|e| e.to_string())?;
    Ok(crate::macro_gen::generate(&casts, &options).macro_text)
}

fn diagnose(
    runtime: &AgentRuntime,
    base: &SimulateRequest,
    baseline: &SimulateResponse,
    response: &SimulateResponse,
    best: &Candidate,
    window: f64,
) -> Result<Diagnosis, String> {
    let row_index = best
        .alignment
        .summary
        .first_difference
        .ok_or("当前宏没有待诊断分歧")?;
    let row = &best.alignment.rows[row_index];
    let reference_index = row.reference_index.or_else(|| {
        best.alignment.rows[row_index..]
            .iter()
            .find_map(|r| r.reference_index)
    });
    let reference = reference_index.and_then(|i| baseline.timeline.get(i));
    let actual = row.actual_index.and_then(|i| response.timeline.get(i));
    let time = if row.kind == "missing" {
        reference
    } else {
        actual.or(reference)
    }
    .map(|e| e.state_before.as_ref().map_or(e.cast_time, |s| s.time))
    .unwrap_or(0.0)
    .clamp(0.0, window);
    let target = reference.map(repair::action_name);
    let mut trace =
        crate::macro_diagnostic::Collector::new((time - 0.5).max(0.0), (time + 1.5).min(window));
    trace.target_skill = target.clone();
    let req = macro_request(base, &best.macro_text, window);
    let c = runtime.context();
    let checked = crate::simulate_core_with_trace(
        &req,
        c.skills,
        c.game_version,
        c.mount,
        c.constants,
        c.recipes,
        c.team_buffs,
        c.formations,
        Some(&mut trace),
    );
    if checked.fingerprint != response.fingerprint {
        return Err("诊断重放与候选指纹不一致，不能继续修复".into());
    }
    let exact = actual
        .and_then(|event| {
            trace.decisions.iter().find(|d| {
                d.cast_success == Some(true)
                    && d.page == event.macro_page.unwrap_or(0)
                    && d.selected_line == event.macro_line
                    && d.cast_time
                        .is_some_and(|t| (t - event.cast_time).abs() < 1e-6)
            })
        })
        .or_else(|| {
            trace
                .decisions
                .iter()
                .find(|d| (d.time - time).abs() < 1e-6)
        });
    let chosen = exact.or_else(|| {
        trace
            .decisions
            .iter()
            .min_by(|a, b| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
    });
    let (kind, message) = match (chosen, exact.is_some()) {
        (Some(_), false) => (
            "approximate",
            "仅观察到分歧附近的另一轮宏决策；不能据此断定目标动作被阻止的原因。".to_owned(),
        ),
        (None, _) => (
            "unobserved",
            "分歧附近没有宏决策记录；可能处于前序技能占用期。".to_owned(),
        ),
        (Some(step), true) => match step.target.as_ref() {
            None => (
                "extra",
                "模板后续没有对应动作；检查多放动作及前序循环。".into(),
            ),
            Some(target)
                if step.cast_success == Some(true)
                    && target
                        .lines
                        .iter()
                        .any(|l| Some(l.line) == step.selected_line) =>
            {
                (
                    "timing_or_state",
                    "目标动作已选中，分歧来自时间、档位或释放前状态。".into(),
                )
            }
            Some(target) if target.lines.is_empty() => {
                ("absent", "当前活动宏页没有目标动作。".into())
            }
            Some(target) if target.lines.iter().all(|l| !l.passed) => {
                ("condition", "目标行的条件未成立。".into())
            }
            Some(target)
                if target
                    .lines
                    .iter()
                    .filter(|l| l.passed)
                    .all(|l| !l.probe.castable) =>
            {
                (
                    "castability",
                    "条件已成立，但姿态、资源、冷却或技能限制阻止释放。".into(),
                )
            }
            Some(target)
                if step.cast_success == Some(true)
                    && step.selected.is_some()
                    && target.lines.iter().any(|line| {
                        line.passed
                            && line.probe.castable
                            && step
                                .selected_line
                                .is_some_and(|selected| selected < line.line)
                    }) =>
            {
                (
                    "priority",
                    "该决策时目标可释放，但更前宏行先被选中并成功释放。".into(),
                )
            }
            Some(_) => (
                "execution_observation",
                "目标通过条件与独立可释放性检查；现有记录不足以归因为宏行优先级。".into(),
            ),
        },
    };
    Ok(Diagnosis {
        kind: kind.into(),
        message,
        time,
        target_skill: target,
        selected_skill: chosen.and_then(|s| s.selected.clone()),
        page: chosen.map(|s| s.page),
        selected_line: chosen.and_then(|s| s.selected_line),
        trace: serde_json::to_value(&trace).map_err(|e| e.to_string())?,
    })
}

pub fn compile(
    request: &MacroCompileRequestV1,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    cancel: &AtomicBool,
    mut progress: impl FnMut(CompileProgress),
) -> Result<CompileResult, String> {
    scenario.verify_hash().map_err(|e| e.to_string())?;
    if runtime.game_version() != request.version || runtime.mount() != request.mount {
        return Err("版本或心法与冻结运行环境不一致".into());
    }
    let captured =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .map_err(|e| e.to_string())?;
    if captured.scenario_hash != scenario.scenario_hash {
        return Err("请求与冻结场景不一致".into());
    }
    if scenario
        .simulation
        .sequence
        .iter()
        .any(|s| s.starts_with("移除气劲") || s == "清除冷却")
    {
        return Err("首版宏编译不支持移除气劲、清除冷却等手动模拟操作，请先从目标轴移除".into());
    }
    let mut budget = Budget {
        request,
        cancel,
        started: Instant::now(),
        simulations: 0,
    };
    let mut result = CompileResult { schema_version: "pve-macro-compile/v1".into(), stop_reason: "no_improvement".into(),
        simulations: 0, rounds: 0, elapsed_ms: 0, scenario_hash: scenario.scenario_hash.clone(),
        acceptance_scope: "active_action_identity_timing_resources".into(),
        window_seconds: 0.0, baseline: None, baseline_fingerprint: None, best: None, history: Vec::new(), failed_candidates: 0, limitations: vec![
            "仅验证本次冻结环境与模板共同窗口；没有证明全局最优或其他战斗场景可复现。".into(),
            "伤害与 DPS 是各自完整模拟的观测值，结算尾段可能不同；不据此宣称同窗口收益。".into(),
            "停止检查发生在每次模拟和候选搜索之间；正在执行的一次模拟会先完成。".into(),
            "未复现仅表示本次局部搜索没有找到满足约束的宏，不证明该技能轴不可写成宏。".into(),
            "动作对齐使用 0≤释放时刻<共同窗口 的半开区间；窗口至少覆盖末次目标主动动作后 1 帧，窗口外动作不参与验收。".into(),
            "还原验收覆盖主动动作身份、释放时间、怒气/暴怒/格挡等资源及档位；不代表全部气劲、冷却或战斗状态等价。".into(),
        ] };
    if !scenario.simulation.pauses.is_empty() {
        result.limitations.push(
            "当前引擎的停手窗口只作用于宏回放，手动目标轴不应用该调度；这可能造成时间或动作差异。"
                .into(),
        );
    }
    let mut emit = |phase: &str, message: &str, budget: &Budget<'_>, result: &CompileResult| {
        progress(CompileProgress {
            phase: phase.into(),
            message: message.into(),
            simulations: budget.simulations,
            best: result.best.clone(),
        });
    };
    if let Err(reason) = budget.reserve() {
        result.stop_reason = reason.into();
        return Ok(result);
    }
    emit("baseline", "正在完整回放冻结技能轴", &budget, &result);
    let mut base = scenario.simulation.clone();
    base.lite = false;
    base.lite_keep_timeline = false;
    let baseline = run(runtime, &base);
    if let Some((index, reason)) = baseline.skipped.first() {
        return Err(format!(
            "模板含 {} 个未成功释放的动作（第 {} 个：{}），请先修正技能轴",
            baseline.skipped.len(),
            index + 1,
            reason
        ));
    }
    // fight_time can equal the last active cast timestamp. A half-open window
    // must extend beyond that cast or a missing final action could pass.
    let last_active = baseline
        .timeline
        .iter()
        .filter(|e| alignment::is_active(e))
        .map(|e| e.cast_time)
        .max_by(f64::total_cmp)
        .unwrap_or(0.0);
    let window = baseline.fight_time.max(last_active + 1.0 / 16.0);
    if !window.is_finite() || !(0.0..=1200.0).contains(&window) || window == 0.0 {
        return Err("模板共同窗口必须大于 0 且不超过 1200 秒".into());
    }
    let active = baseline
        .timeline
        .iter()
        .filter(|e| alignment::is_active(e) && alignment::within_window(e, window))
        .count();
    if active == 0 || active > 2048 {
        return Err("模板须包含 1～2048 个有效主动动作".into());
    }
    result.window_seconds = window;
    result.baseline = Some(metrics(&baseline, window));
    result.baseline_fingerprint = Some(baseline.fingerprint.to_string());
    if let Some(reason) = budget.stop() {
        result.stop_reason = reason.into();
        result.simulations = budget.simulations;
        result.elapsed_ms = budget.started.elapsed().as_millis() as u64;
        emit(
            "finished",
            "任务已停止，基线已验证但尚未试跑候选",
            &budget,
            &result,
        );
        return Ok(result);
    }
    let initial = match request
        .initial_macro
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(text) => text.clone(),
        None => generated(&baseline)?,
    };
    if crate::macro_parser::parse_macro_text(&initial).is_err() {
        return Err("初始宏语法无效".into());
    }
    let mut tested = BTreeSet::new();
    let mut best_response: Option<SimulateResponse> = None;
    let mut pending = vec![repair::Proposal {
        text: initial,
        origin: "initial".into(),
    }];
    'search: for round in 0..=request.max_rounds {
        result.rounds = round;
        let mut improved = false;
        for mut proposal in std::mem::take(&mut pending) {
            // Normalize before simulation, deduplication and page accounting.
            // The returned text must be the same macro that was actually tested.
            proposal.text = match repair::game_macro_text(&proposal.text) {
                Ok(text) => text,
                Err(message) => {
                    result.failed_candidates += 1;
                    if result.failed_candidates <= 5 {
                        result
                            .limitations
                            .push(format!("某候选无法导出为游戏宏，已跳过：{message}"));
                    }
                    continue;
                }
            };
            if !tested.insert(proposal.text.clone()) {
                continue;
            }
            if let Err(reason) = budget.reserve() {
                result.stop_reason = reason.into();
                break 'search;
            }
            emit(
                "replay",
                "正在同环境回放并比较完整动作序列",
                &budget,
                &result,
            );
            let response = run(runtime, &macro_request(&base, &proposal.text, window));
            let candidate = match make_candidate(
                proposal.text,
                proposal.origin,
                &response,
                &baseline,
                window,
                request,
            ) {
                Ok(candidate) => candidate,
                Err(message) => {
                    result.failed_candidates += 1;
                    if result.failed_candidates <= 5 {
                        result
                            .limitations
                            .push(format!("某候选无法验收，已跳过：{message}"));
                    }
                    continue;
                }
            };
            let accepted = result.best.as_ref().is_none_or(|best| {
                score(&candidate, request.max_pages) < score(best, request.max_pages)
            });
            result.history.push(Trial {
                round,
                origin: candidate.origin.clone(),
                accepted,
                summary: candidate.alignment.summary.clone(),
                page_constraints_passed: candidate.page_constraints_passed,
                fingerprint: candidate.fingerprint.clone(),
            });
            if accepted {
                improved = true;
                best_response = Some(response);
                let reproduced = candidate.reproduced;
                result.best = Some(candidate);
                emit(
                    "candidate",
                    "已保留更符合模板的完整验证候选",
                    &budget,
                    &result,
                );
                if reproduced {
                    result.stop_reason = "target_reproduced".into();
                    break 'search;
                }
            }
        }
        if let Some(reason) = budget.stop() {
            result.stop_reason = reason.into();
            break;
        }
        if round == request.max_rounds {
            result.stop_reason = "max_rounds".into();
            break;
        }
        if !improved && round > 0 {
            result.stop_reason = "no_improvement".into();
            break;
        }
        let Some(best) = result.best.as_mut() else {
            break;
        };
        if best.alignment.summary.first_difference.is_some() && best.diagnosis.is_none() {
            if let Err(reason) = budget.reserve() {
                result.stop_reason = reason.into();
                break;
            }
            match diagnose(
                runtime,
                &base,
                &baseline,
                best_response.as_ref().unwrap(),
                best,
                window,
            ) {
                Ok(diagnosis) => best.diagnosis = Some(diagnosis),
                Err(message) => {
                    result
                        .limitations
                        .push(format!("诊断未完成，已保留完整验证候选：{message}"));
                    break;
                }
            }
        }
        emit(
            "repair",
            "正在从首次分歧及正反例产生局部改法",
            &budget,
            &result,
        );
        if let Some(reason) = budget.stop() {
            result.stop_reason = reason.into();
            break;
        }
        pending = repair::proposals(
            result.best.as_ref().unwrap(),
            &baseline,
            best_response.as_ref().unwrap(),
            window,
            || budget.stop().is_some(),
        );
        if pending.is_empty() {
            result.stop_reason = "no_improvement".into();
            break;
        }
    }
    // A final best candidate must never carry a diagnosis from a previous macro.
    if let (Some(best), Some(response)) = (result.best.as_mut(), best_response.as_ref()) {
        if best.alignment.summary.first_difference.is_some()
            && best.diagnosis.is_none()
            && budget.reserve().is_ok()
        {
            match diagnose(runtime, &base, &baseline, response, best, window) {
                Ok(diagnosis) => best.diagnosis = Some(diagnosis),
                Err(message) => result
                    .limitations
                    .push(format!("最终诊断未完成，已保留完整验证候选：{message}")),
            }
        }
    }
    result.simulations = budget.simulations;
    result.elapsed_ms = budget.started.elapsed().as_millis() as u64;
    // Cancellation arriving during a replay remains a cancellation, even if that replay succeeded.
    if cancel.load(Ordering::Relaxed) {
        result.stop_reason = "cancelled".into();
    }
    emit("finished", "任务已停止，保留已验证候选", &budget, &result);
    Ok(result)
}

#[cfg(test)]
#[path = "../../tests/harness/compiler.rs"]
mod tests;
