//! Deterministic, bounded executable-macro search for the Harness runtime.
//! Neighbours are hypotheses; scores and accepted artifacts only come from the
//! complete simulator with a frozen environment. This is not a global optimizer.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::compiler::{generate_macro_seed, normalize_game_macro, pages, Metrics, Page};
use crate::agent::{AgentRuntime, ScenarioSnapshotV1};
use crate::macro_engine::{CmpOp, MacroCondition};
use crate::{GameVersion, Mount, SimulateRequest, SimulateResponse};

pub const ALGORITHM_VERSION: &str = "rotation-search/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationRequest {
    pub simulation: SimulateRequest,
    pub version: GameVersion,
    pub mount: Mount,
    #[serde(default)]
    pub initial_macro: Option<String>,
    /// Empty means the union of the seed's commands and the manual axis.
    #[serde(default)]
    pub allowed_skills: Vec<String>,
    #[serde(default = "default_simulations")]
    pub max_simulations: u32,
    #[serde(default = "default_wall_time")]
    pub wall_time_ms: u64,
    #[serde(default = "default_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_duration")]
    pub duration_seconds: f64,
    #[serde(default = "default_pages")]
    pub max_pages: usize,
}
fn default_simulations() -> u32 {
    96
}
fn default_wall_time() -> u64 {
    60_000
}
fn default_rounds() -> u32 {
    6
}
fn default_duration() -> f64 {
    120.0
}
fn default_pages() -> usize {
    2
}

impl RotationRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.duration_seconds.is_finite()
            || !(10.0..=600.0).contains(&self.duration_seconds)
            || self.allowed_skills.len() > 64
            || self
                .allowed_skills
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 128 || s.starts_with("__"))
            || self.simulation.sequence.len() > 4096
            || self
                .simulation
                .sequence
                .iter()
                .any(|s| s.is_empty() || s.len() > 128)
        {
            return Err("循环时长、技能池或序列超出有界实验范围。");
        }
        let has_macro = self
            .initial_macro
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(self.simulation.macro_text.as_deref())
            .is_some_and(|s| !s.trim().is_empty());
        if !has_macro && self.simulation.sequence.is_empty() {
            return Err("请提供起始宏或手动技能轴。");
        }
        // Reuse the authoritative safety boundary for the complete environment;
        // rotation search intentionally accepts a macro in place of a target axis.
        let mut environment = self.simulation.clone();
        environment.sequence = vec!["盾刀".into()];
        environment.macro_text = None;
        let boundary = super::contract::MacroCompileRequestV1 {
            simulation: environment,
            version: self.version,
            mount: self.mount,
            initial_macro: self.initial_macro.clone(),
            max_simulations: self.max_simulations,
            wall_time_ms: self.wall_time_ms,
            max_rounds: self.max_rounds,
            max_pages: self.max_pages,
            time_tolerance: 1.0 / 16.0,
        };
        boundary.validate()?;
        if let Some(text) = &self.simulation.macro_text {
            let mut second = boundary;
            second.initial_macro = Some(text.clone());
            second.validate()?;
        }
        if !has_macro
            && (self.simulation.sequence.len() > 2048
                || self.simulation.sequence.iter().any(|s| {
                    s.starts_with("__") || s.starts_with("移除气劲") || s.starts_with("清除冷却")
                }))
        {
            return Err("手动种子轴最多 2048 项，不支持清冷却、移除气劲等模拟器专用操作。");
        }
        Ok(())
    }

    pub fn snapshot(&self, runtime: &AgentRuntime) -> Result<ScenarioSnapshotV1, &'static str> {
        self.validate()?;
        if self.version != runtime.game_version() || self.mount != runtime.mount() {
            return Err("版本或心法已变化，请重新冻结场景。");
        }
        ScenarioSnapshotV1::capture(self.version, self.mount, self.simulation.clone())
            .map_err(|_| "无法冻结完整循环实验场景。")
    }
}

#[derive(Clone, Serialize)]
struct Candidate {
    macro_text: String,
    pages: Vec<Page>,
    page_constraints_passed: bool,
    allowed_skills_passed: bool,
    verified: bool,
    full_snapshots: bool,
    metrics: Metrics,
    fingerprint: String,
    scenario_hash: String,
    /// Reusable by later equipment/rotation experiments; never a partial panel.
    simulation: SimulateRequest,
    origin: String,
}

struct Budget<'a> {
    request: &'a RotationRequest,
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
    fn replay(
        &mut self,
        runtime: &AgentRuntime,
        request: &SimulateRequest,
    ) -> Result<SimulateResponse, String> {
        if let Some(reason) = self.stop() {
            return Err(reason.into());
        }
        // Charge before every engine entry, including failed/seed/held-out runs.
        self.simulations += 1;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
        }))
        .map_err(|_| "完整模拟失败，候选未被接受。".to_owned())
    }
}

fn macro_request(base: &SimulateRequest, text: &str, duration: f64) -> SimulateRequest {
    let mut request = base.clone();
    request.sequence = vec!["__macro__".into(); (duration / 0.25).ceil() as usize + 20];
    request.macro_text = Some(text.into());
    request.macro_duration = Some(duration);
    request.channel_ticks.clear();
    request.timing_offsets.clear();
    request.solidified_casts.clear();
    request.qijin_buffs.clear();
    request.lite = false;
    request.lite_keep_timeline = false;
    request
}

fn commands(text: &str) -> Result<BTreeSet<String>, String> {
    Ok(crate::macro_parser::parse_macro_text(text)
        .map_err(|e| e.to_string())?
        .pages
        .iter()
        .flat_map(|p| p.lines.iter().map(|l| l.action.skill_name().to_owned()))
        .collect())
}

fn legal(text: &str, request: &RotationRequest, allowed: &BTreeSet<String>) -> bool {
    let p = pages(text);
    !p.is_empty()
        && p.len() <= request.max_pages
        && p.iter().all(|p| p.within_limit)
        && commands(text).is_ok_and(|c| !c.is_empty() && c.is_subset(allowed))
}

fn candidate(
    request: &RotationRequest,
    simulation: SimulateRequest,
    response: &SimulateResponse,
    allowed: &BTreeSet<String>,
    origin: &str,
) -> Result<Candidate, String> {
    // Never reward a channel tail that extends only one candidate's denominator.
    // Such an artifact is rejected until the engine supports strict clipping.
    if !response.dps.is_finite()
        || !response.total_damage.is_finite()
        || !response.fight_time.is_finite()
        || (response.fight_time - request.duration_seconds).abs() > 1e-6
    {
        return Err("候选回放未覆盖相同固定时长，不能纳入 DPS 比较。".into());
    }
    let text = simulation.macro_text.as_deref().ok_or("缺少已回放宏")?;
    let p = pages(text);
    let snapshot = ScenarioSnapshotV1::capture(request.version, request.mount, simulation.clone())
        .map_err(|e| e.to_string())?;
    let active: Vec<_> = response
        .timeline
        .iter()
        .filter(|e| {
            super::alignment::is_active(e)
                && super::alignment::within_window(e, request.duration_seconds)
        })
        .collect();
    let full_snapshots = active
        .iter()
        .all(|e| e.state_before.as_ref().is_some_and(|s| s.time.is_finite()));
    let page_constraints_passed =
        !p.is_empty() && p.len() <= request.max_pages && p.iter().all(|p| p.within_limit);
    let allowed_skills_passed = commands(text)?.is_subset(allowed);
    Ok(Candidate {
        macro_text: text.into(),
        page_constraints_passed,
        allowed_skills_passed,
        pages: p,
        verified: page_constraints_passed
            && allowed_skills_passed
            && !active.is_empty()
            && full_snapshots
            && response.skipped.is_empty(),
        full_snapshots,
        metrics: Metrics {
            dps: response.dps,
            total_damage: response.total_damage,
            fight_time: response.fight_time,
            active_casts: active.len(),
            skipped_count: response.skipped.len(),
        },
        fingerprint: response.fingerprint.to_string(),
        scenario_hash: snapshot.scenario_hash,
        simulation,
        origin: origin.into(),
    })
}

fn better(candidate: &Candidate, current: Option<&Candidate>) -> bool {
    candidate.verified
        && candidate.page_constraints_passed
        && candidate.allowed_skills_passed
        && current.is_none_or(|c| {
            candidate.metrics.dps > c.metrics.dps + 1e-8
                || ((candidate.metrics.dps - c.metrics.dps).abs() <= 1e-8
                    && candidate.macro_text.encode_utf16().count()
                        < c.macro_text.encode_utf16().count())
        })
}

fn modify(
    text: &str,
    page: usize,
    line: usize,
    condition: Option<MacroCondition>,
) -> Option<String> {
    let mut config = crate::macro_parser::parse_macro_text(text).ok()?;
    config.pages.get_mut(page)?.lines.get_mut(line)?.condition = condition;
    normalize_game_macro(&crate::macro_parser::render_macro_text(&config)).ok()
}

/// Bidirectional numeric edits preserve boolean structure and game syntax.
fn numeric_neighbours(condition: &MacroCondition) -> Vec<MacroCondition> {
    use MacroCondition::*;
    match condition {
        And(a, b) => numeric_neighbours(a)
            .into_iter()
            .map(|x| And(Box::new(x), b.clone()))
            .chain(
                numeric_neighbours(b)
                    .into_iter()
                    .map(|x| And(a.clone(), Box::new(x))),
            )
            .take(24)
            .collect(),
        Or(a, b) => numeric_neighbours(a)
            .into_iter()
            .map(|x| Or(Box::new(x), b.clone()))
            .chain(
                numeric_neighbours(b)
                    .into_iter()
                    .map(|x| Or(a.clone(), Box::new(x))),
            )
            .take(24)
            .collect(),
        Rage(op, value) => [-10, -1, 1, 10]
            .iter()
            .map(|d| Rage(*op, value.saturating_add(*d).clamp(0, 100)))
            .collect(),
        Energy(op, value) => [-10, -1, 1, 10]
            .iter()
            .map(|d| Energy(*op, value.saturating_add(*d).clamp(0, 100)))
            .collect(),
        Berserk(op, value) => [-10, -1, 1, 10]
            .iter()
            .map(|d| Berserk(*op, value.saturating_add(*d).clamp(0, 100)))
            .collect(),
        BuffTime(name, op, value) => [-1.0, -0.0625, 0.0625, 1.0]
            .iter()
            .map(|d| BuffTime(name.clone(), *op, (value + d).clamp(0.0, 1200.0)))
            .collect(),
        TBuffTime(name, op, value) => [-1.0, -0.0625, 0.0625, 1.0]
            .iter()
            .map(|d| TBuffTime(name.clone(), *op, (value + d).clamp(0.0, 1200.0)))
            .collect(),
        BuffStack(name, op, value) => vec![
            BuffStack(name.clone(), *op, value.saturating_sub(1)),
            BuffStack(name.clone(), *op, value.saturating_add(1).min(1024)),
        ],
        SkillEnergy(name, op, value) => vec![
            SkillEnergy(name.clone(), *op, value.saturating_sub(1)),
            SkillEnergy(name.clone(), *op, value.saturating_add(1).min(1024)),
        ],
        _ => Vec::new(),
    }
}

fn proposals(
    text: &str,
    allowed: &BTreeSet<String>,
    observations: Option<&SimulateResponse>,
) -> Vec<(String, String)> {
    let mut result = Vec::new();
    // Complete unconditional contrast is essential when multiple conditions block.
    if let Ok(mut config) = crate::macro_parser::parse_macro_text(text) {
        for page in &mut config.pages {
            page.lines
                .retain(|l| allowed.contains(l.action.skill_name()));
            for line in &mut page.lines {
                line.condition = None;
            }
        }
        result.push((
            crate::macro_parser::render_macro_text(&config),
            "unconditional_contrast".into(),
        ));
    }
    if let Ok(candidates) = crate::macro_prune::list_prune_candidates(text) {
        result.extend(
            candidates
                .into_iter()
                .take(12)
                .map(|c| (c.after_macro, "prune_condition".into())),
        );
    }
    if let Ok(candidates) = crate::macro_prune::list_swap_candidates(text) {
        result.extend(
            candidates
                .into_iter()
                .take(12)
                .map(|c| (c.after_macro, "swap_priority".into())),
        );
    }
    let Ok(config) = crate::macro_parser::parse_macro_text(text) else {
        return result;
    };
    let rage: Vec<i32> = observations
        .into_iter()
        .flat_map(|r| &r.timeline)
        .filter_map(|e| e.state_before.as_ref().map(|s| s.rage.clamp(0, 100)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(6)
        .collect();
    for (p, page) in config.pages.iter().enumerate() {
        for (l, line) in page.lines.iter().enumerate() {
            if let Some(condition) = &line.condition {
                for edit in numeric_neighbours(condition).into_iter().take(8) {
                    if let Some(edit) = modify(text, p, l, Some(edit)) {
                        result.push((edit, "numeric_threshold".into()));
                    }
                }
            } else {
                for v in rage.iter().copied().chain([24, 49, 79]).take(8) {
                    if let Some(edit) = modify(text, p, l, Some(MacroCondition::Rage(CmpOp::Gt, v)))
                    {
                        result.push((edit, "observed_resource_gate".into()));
                    }
                }
            }
            let mut deletion = crate::macro_parser::parse_macro_text(text).unwrap();
            deletion.pages[p].lines.remove(l);
            result.push((
                crate::macro_parser::render_macro_text(&deletion),
                "remove_command".into(),
            ));
        }
    }
    // A permitted skill can enter at either priority boundary of any page. The
    // simulator, including stance/talent/CD checks, decides whether it is useful.
    for skill in allowed {
        for p in 0..config.pages.len() {
            for front in [true, false] {
                let mut edit = crate::macro_parser::parse_macro_text(text).unwrap();
                let action = crate::macro_engine::MacroLine {
                    condition: None,
                    action: crate::macro_engine::MacroAction::Cast(skill.clone()),
                };
                if front {
                    edit.pages[p].lines.insert(0, action);
                } else {
                    edit.pages[p].lines.push(action);
                }
                result.push((
                    crate::macro_parser::render_macro_text(&edit),
                    "insert_allowed_skill".into(),
                ));
            }
        }
        // A legal fallback lets an oversized or forbidden seed find a feasible
        // anchor without treating its unverified score as a valid result.
        result.push((format!("/cast {skill}"), "single_skill_seed".into()));
    }
    let mut seen = BTreeSet::new();
    result
        .into_iter()
        .filter_map(|(t, origin)| {
            let t = normalize_game_macro(&t).ok()?;
            (t != text && seen.insert(t.clone())).then_some((t, origin))
        })
        .take(256)
        .collect()
}

pub fn run(
    request: &RotationRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Value),
) -> Result<Value, String> {
    request.validate().map_err(str::to_owned)?;
    scenario.verify_hash().map_err(|e| e.to_string())?;
    let captured = request.snapshot(runtime).map_err(str::to_owned)?;
    if captured.scenario_hash != scenario.scenario_hash {
        return Err("循环请求与冻结场景不一致。".into());
    }
    let available: BTreeSet<String> = runtime
        .context()
        .skills
        .iter()
        .filter(|s| {
            !s.passive
                && s.skill_id != 90001
                && s.requires_talent
                    .is_none_or(|t| scenario.simulation.talents.contains(&t))
        })
        .map(|s| {
            // Match simulate_core's command identity, including separate 雾海.
            if (90010..=90012).contains(&s.skill_id) {
                s.name.clone()
            } else {
                s.name.split('·').next().unwrap_or(&s.name).to_owned()
            }
        })
        .collect();
    if request
        .allowed_skills
        .iter()
        .any(|s| !available.contains(s))
    {
        return Err("技能池含当前版本/心法/奇穴下不可用的主动技能。".into());
    }
    let mut budget = Budget {
        request,
        cancel,
        started: Instant::now(),
        simulations: 0,
    };
    let mut limitations = vec![
        "这是给定技能池、起始宏与预算内的局部搜索，不证明全局最优。".to_owned(),
        "每页需手动按姿态使用；只验证模拟器已实现的游戏宏规则。".to_owned(),
        "只用一个不同延迟的留出场景检查迁移，不能证明所有延迟或战斗场景均提升。".to_owned(),
        "评分要求完整回放恰好结束于固定时长；超出窗口的引导尾段候选不参与比较。".to_owned(),
    ];
    let mut history = Vec::<Value>::new();
    let mut baseline: Option<Candidate> = None;
    let mut best: Option<Candidate> = None;
    let mut rounds = 0;
    let mut stop_reason = "max_rounds".to_owned();
    let mut validation = json!({"status":"not_run","scope":"one_held_out_delay","improved":false});
    let mut allowed = BTreeSet::new();
    let mut failed_candidates = 0_u32;
    // Labeled block permits cancellation/budget stops without losing artifacts.
    'experiment: {
        if let Some(reason) = budget.stop() {
            stop_reason = reason.into();
            break 'experiment;
        }
        let input_macro = request
            .initial_macro
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                scenario
                    .simulation
                    .macro_text
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
            });
        let seed = if let Some(text) = input_macro {
            normalize_game_macro(text)?
        } else {
            progress(
                json!({"phase":"seed","message":"完整回放手动轴以生成宏种子","simulations":budget.simulations}),
            );
            let mut manual = scenario.simulation.clone();
            manual.lite = false;
            manual.lite_keep_timeline = false;
            let response = match budget.replay(runtime, &manual) {
                Ok(r) => r,
                Err(error) => {
                    stop_reason = budget.stop().unwrap_or("seed_failed").into();
                    limitations.push(error);
                    break 'experiment;
                }
            };
            if response.fight_time > 1200.0
                || response
                    .timeline
                    .iter()
                    .filter(|e| super::alignment::is_active(e))
                    .count()
                    > 2048
            {
                stop_reason = "seed_failed".into();
                limitations.push("手动种子回放超过 1200 秒或 2048 个主动动作。".into());
                break 'experiment;
            }
            if !response.skipped.is_empty() {
                limitations.push(format!(
                    "手动种子有 {} 个未释放操作；基线是生成宏的固定时长回放，不是原手动轴 DPS。",
                    response.skipped.len()
                ));
            }
            match generate_macro_seed(&response).and_then(|text| normalize_game_macro(&text)) {
                Ok(text) => text,
                Err(error) => {
                    stop_reason = "seed_failed".into();
                    limitations.push(error);
                    break 'experiment;
                }
            }
        };
        if request.allowed_skills.is_empty() {
            allowed = commands(&seed)?;
            allowed.extend(
                scenario
                    .simulation
                    .sequence
                    .iter()
                    .filter(|s| !s.starts_with("__"))
                    .cloned(),
            );
            allowed.retain(|s| available.contains(s));
            if allowed.len() > 64 {
                stop_reason = "invalid_skill_pool".into();
                limitations.push("默认技能池超过 64 项，请显式缩小 allowed_skills。".into());
                break 'experiment;
            }
        } else {
            allowed = request.allowed_skills.iter().cloned().collect();
        }
        if allowed.is_empty() {
            stop_reason = "invalid_skill_pool".into();
            limitations.push("当前场景没有可用于循环搜索的允许技能。".into());
            break 'experiment;
        }
        let base_request = macro_request(&scenario.simulation, &seed, request.duration_seconds);
        let response = match budget.replay(runtime, &base_request) {
            Ok(r) => r,
            Err(error) => {
                stop_reason = budget.stop().unwrap_or("baseline_failed").into();
                limitations.push(error);
                break 'experiment;
            }
        };
        let initial = match candidate(request, base_request, &response, &allowed, "baseline") {
            Ok(c) => c,
            Err(error) => {
                stop_reason = "baseline_failed".into();
                limitations.push(error);
                break 'experiment;
            }
        };
        if better(&initial, None) {
            best = Some(initial.clone());
        }
        baseline = Some(initial);
        let mut best_response = Some(response);
        progress(
            json!({"phase":"baseline","simulations":budget.simulations,"best":best,"baseline":baseline}),
        );
        let mut seen = BTreeSet::from([seed.clone()]);
        let mut anchor = seed;
        for round in 1..=request.max_rounds {
            if let Some(reason) = budget.stop() {
                stop_reason = reason.into();
                break;
            }
            if budget.simulations.saturating_add(2) >= request.max_simulations {
                stop_reason = "budget_exhausted".into();
                break;
            }
            rounds = round;
            let mut accepted = false;
            let neighbours = proposals(&anchor, &allowed, best_response.as_ref());
            for (text, origin) in neighbours {
                if let Some(reason) = budget.stop() {
                    stop_reason = reason.into();
                    break;
                }
                // Reserve a full pair of held-out replays; both count globally.
                if budget.simulations.saturating_add(2) >= request.max_simulations {
                    stop_reason = "budget_exhausted".into();
                    break;
                }
                if !legal(&text, request, &allowed) || !seen.insert(text.clone()) {
                    continue;
                }
                let sim = macro_request(&scenario.simulation, &text, request.duration_seconds);
                let response = match budget.replay(runtime, &sim) {
                    Ok(r) => r,
                    Err(error) => {
                        failed_candidates += 1;
                        history.push(
                            json!({"round":round,"origin":origin,"accepted":false,"error":error}),
                        );
                        continue;
                    }
                };
                let tested = match candidate(request, sim, &response, &allowed, &origin) {
                    Ok(c) => c,
                    Err(error) => {
                        failed_candidates += 1;
                        history.push(
                            json!({"round":round,"origin":origin,"accepted":false,"error":error}),
                        );
                        continue;
                    }
                };
                let accept = better(&tested, best.as_ref());
                history.push(json!({"round":round,"origin":origin,"accepted":accept,
                    "dps":tested.metrics.dps,"fingerprint":tested.fingerprint}));
                if accept {
                    accepted = true;
                    anchor = text;
                    best = Some(tested);
                    best_response = Some(response);
                }
                progress(
                    json!({"phase":"search","round":round,"simulations":budget.simulations,"best":best}),
                );
            }
            if budget.stop().is_some()
                || budget.simulations.saturating_add(2) >= request.max_simulations
            {
                break;
            }
            if !accepted {
                stop_reason = "no_improvement".into();
                break;
            }
        }
        if let Some(reason) = budget.stop() {
            stop_reason = reason.into();
        }
        let (Some(base), Some(winner)) = (baseline.as_ref(), best.as_ref()) else {
            limitations.push("未找到满足页长和技能池约束的已回放宏。".into());
            break 'experiment;
        };
        if let Some(reason) = budget.stop() {
            validation["status"] = json!(reason);
            break 'experiment;
        }
        // Same held-out environment for both; only delay differs from training.
        let delay = if scenario.simulation.network_delay <= 9950 {
            scenario.simulation.network_delay + 50
        } else {
            scenario.simulation.network_delay - 50
        };
        validation = json!({"status":"incomplete","scope":"one_held_out_delay","network_delay":delay,"improved":false});
        progress(json!({"phase":"validation","simulations":budget.simulations,"best":best}));
        for (key, tested) in [("baseline", base), ("best", winner)] {
            let mut held_out = tested.simulation.clone();
            held_out.network_delay = delay;
            match budget
                .replay(runtime, &held_out)
                .and_then(|r| candidate(request, held_out, &r, &allowed, "held_out_delay"))
            {
                Ok(c) => validation[key] = serde_json::to_value(c).map_err(|e| e.to_string())?,
                Err(error) => {
                    validation["status"] = json!(budget.stop().unwrap_or("failed"));
                    limitations.push(format!("留出验证未完成：{error}"));
                    break;
                }
            }
        }
        if validation.get("baseline").is_some() && validation.get("best").is_some() {
            let b = validation["baseline"]["metrics"]["dps"]
                .as_f64()
                .unwrap_or(0.0);
            let w = validation["best"]["metrics"]["dps"].as_f64().unwrap_or(0.0);
            validation["status"] = json!("completed");
            validation["improved"] = json!(
                winner.verified
                    && validation["best"]["verified"] == true
                    && winner.metrics.dps > base.metrics.dps + 1e-8
                    && w > b + 1e-8
            );
        }
        if let Some(reason) = budget.stop() {
            stop_reason = reason.into();
        }
    }
    if validation["status"] != "completed" {
        limitations.push("尚未完成一对留出回放，不能宣称提升可迁移。".into());
    }
    let improvement_pct = baseline.as_ref().zip(best.as_ref()).and_then(|(a, b)| {
        (a.metrics.dps > 0.0).then_some((b.metrics.dps / a.metrics.dps - 1.0) * 100.0)
    });
    Ok(
        json!({"schema_version":ALGORITHM_VERSION,"task":"search_rotation","stop_reason":stop_reason,
        "simulations":budget.simulations,"rounds":rounds,"elapsed_ms":budget.started.elapsed().as_millis() as u64,
        "scenario_hash":scenario.scenario_hash,"duration_seconds":request.duration_seconds,
        "allowed_skills":allowed,"baseline":baseline,"best":best,"improvement_pct":improvement_pct,
        "validation":validation,"history":history,"failed_candidates":failed_candidates,"limitations":limitations}),
    )
}

#[cfg(test)]
#[path = "../../tests/harness/rotation.rs"]
mod tests;
