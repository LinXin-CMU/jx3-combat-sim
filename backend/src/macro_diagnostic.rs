//! Bounded, opt-in observations of the real two-step macro executor.
use axum::{extract::State, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::{Deserialize, Serialize};
use crate::{macro_eval::{Phase1LineDebug, Phase2EntryDebug}, GameVersion, Mount, Player, SharedState, SimulateRequest};

#[derive(Debug, Serialize)]
pub struct Decision {
    pub time: f64,
    pub page: usize,
    pub stance: String,
    pub last_skill: Option<String>,
    pub rage: i32,
    pub block_value: i32,
    pub berserk_value: i32,
    pub phase1: Vec<Phase1LineDebug>,
    pub phase2: Vec<Phase2EntryDebug>,
    pub selected_line: Option<usize>,
    pub selected: Option<String>,
    pub cast_success: Option<bool>,
    pub cast_time: Option<f64>,
    pub target: Option<crate::macro_eval::diagnostic_evidence::TargetEvidence>,
}

#[derive(Debug, Serialize)]
pub struct Collector {
    #[serde(skip)]
    pub(crate) exact: Option<crate::macro_exact::Probe>,
    pub start: f64,
    pub end: f64,
    pub decisions: Vec<Decision>,
    pub truncated: bool,
    #[serde(skip)]
    rows: usize,
    #[serde(skip)]
    pub target_skill: Option<String>,
    #[serde(skip)]
    evidence_rows: usize,
}

impl Collector {
    pub fn new(start: f64, end: f64) -> Self {
        Self { exact: None, start, end, decisions: Vec::new(), truncated: false, rows: 0, target_skill: None, evidence_rows: 0 }
    }
    pub fn contains(&self, time: f64) -> bool { time >= self.start && time <= self.end }
    pub fn record(&mut self, player: &Player, page: usize, last_skill: Option<&str>, phase1: Vec<Phase1LineDebug>, phase2: Vec<Phase2EntryDebug>, selected: Option<(usize, String)>) -> Option<usize> {
        let rows = phase1.len() + phase2.len();
        if self.decisions.len() >= 128 || self.rows + rows > 4096 {
            self.truncated = true;
            return None;
        }
        self.rows += rows;
        let (selected_line, selected) = selected.map(|(line, name)| (Some(line), Some(name))).unwrap_or_default();
        self.decisions.push(Decision {
            time: player.current_time, page: page + 1, stance: format!("{:?}", player.stance()),
            last_skill: last_skill.map(str::to_owned), rage: player.rage,
            block_value: player.block_value, berserk_value: player.berserk_value,
            phase1, phase2, selected_line, selected, cast_success: None, cast_time: None, target: None,
        });
        Some(self.decisions.len() - 1)
    }
    pub fn finish(&mut self, index: usize, success: bool, time: Option<f64>) {
        let step = &mut self.decisions[index];
        step.cast_success = Some(success);
        step.cast_time = time;
    }
    pub fn focus<'a>(&mut self, index: Option<usize>, config: &crate::macro_engine::MacroConfig, player: &'a Player,
        map: &'a std::collections::HashMap<&'a str, Vec<&'a crate::SkillSpec>>, ids: &'a std::collections::HashMap<u32, &'a crate::SkillSpec>) {
        let (Some(index), Some(target)) = (index, self.target_skill.as_deref()) else { return; };
        if self.evidence_rows >= 4096 { self.truncated = true; return; }
        let step = &mut self.decisions[index];
        let evidence = crate::macro_eval::diagnostic_evidence::inspect(target, config, step.page-1, player, map, ids, step.last_skill.as_deref());
        let rows = evidence.lines.iter().map(|line| 1 + line.atoms.len()).sum::<usize>() + 1;
        if self.evidence_rows + rows > 4096 { self.truncated = true; return; }
        self.evidence_rows += rows;
        step.target = Some(evidence);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub simulation: SimulateRequest,
    pub version: GameVersion,
    pub mount: Mount,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub target_skill: Option<String>,
    #[serde(default)]
    pub include_result: bool,
}

fn validate(req: &Request) -> Result<(), &'static str> {
    if req.target_skill.as_ref().is_some_and(|s| s.trim().is_empty() || s.len() > 128) { return Err("目标技能名无效。"); }
    let duration = req.simulation.macro_duration.unwrap_or(0.0);
    if !duration.is_finite() || duration <= 0.0 || duration > 1200.0 { return Err("诊断需要 0～1200 秒的完整宏运行。") }
    if !req.start.is_finite() || !req.end.is_finite() || req.start < 0.0 || req.end < req.start || req.end - req.start > 10.0 || req.end > duration + 1.0 {
        return Err("诊断窗口必须位于本次运行内，且最多 10 秒。");
    }
    let text = req.simulation.macro_text.as_deref().unwrap_or("");
    if text.is_empty() || text.len() > 32768 || req.simulation.sequence.len() > 6000 { return Err("宏或序列超出诊断范围。") }
    let config = crate::macro_parser::parse_macro_text(text).map_err(|_| "宏语法错误，请修正后重新运行。")?;
    if config.pages.iter().map(|page| page.lines.len()).sum::<usize>() > 128 { return Err("诊断最多支持 128 条宏语句。") }
    if req.simulation.lite { return Err("请先完成完整宏对照，再查看诊断。") }
    Ok(())
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

pub async fn handler(State(state): State<SharedState>, Json(req): Json<Request>) -> Response {
    if let Err(message) = validate(&req) { return error(StatusCode::BAD_REQUEST, message) }
    let runtime = crate::agent::AgentRuntime::load(&state).await;
    if runtime.game_version() != req.version || runtime.mount() != req.mount {
        return error(StatusCode::CONFLICT, "版本或心法已变化，请重新运行对照。");
    }
    match tokio::task::spawn_blocking(move || {
        let context = runtime.context();
        let mut trace = Collector::new(req.start, req.end);
        trace.target_skill = req.target_skill;
        let result = crate::simulate_core_with_trace(&req.simulation, context.skills, context.game_version, context.mount, context.constants, context.recipes, context.team_buffs, context.formations, Some(&mut trace));
        let mut response = serde_json::json!({"trace":trace, "fingerprint":result.fingerprint});
        if req.include_result { response["simulation"] = serde_json::to_value(result).unwrap(); }
        response
    }).await {
        Ok(result) => Json(result).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "诊断未完成，请重试。"),
    }
}

#[cfg(test)]
#[path = "../tests/macro_assist/diagnostic.rs"]
mod tests;
