//! RL env HTTP 端点。
//!
//! 路由（在 main.rs 注册）：
//!   POST /api/rl/env/create                  → { session_id, obs, mask, obs_dim, action_count }
//!   POST /api/rl/env/{id}/reset              → { obs, mask }
//!   POST /api/rl/env/{id}/step               → { obs, reward, done, mask, cast_success, damage_delta }
//!   POST /api/rl/env/{id}/advance            → { obs, mask, done }
//!   POST /api/rl/env/{id}/macro_decision     → { action }
//!   GET  /api/rl/env/{id}/info               → { elapsed, total_damage, dps, rage }
//!   POST /api/rl/env/{id}/close              → { closed }
//!   GET  /api/rl/spec                        → { obs_dim, action_count, action_names, action_skills, layout }
//!   GET  /api/rl/sessions                    → { sessions: [...] }

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::macro_parser::parse_macro_text;
use crate::{Attributes, SharedState, TargetConfig};

use super::action::{ACTION_COUNT, ACTION_NAMES, ACTION_SKILLS};
use super::env::{CombatEnv, EnvConfig};
use super::obs::OBS_DIM;

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub attributes: Attributes,
    pub target: TargetConfig,
    pub duration: f64,
    pub haste_level: u32,
    #[serde(default)]
    pub talents: Vec<u32>,
    #[serde(default)]
    pub recipes: Vec<u32>,
    #[serde(default)]
    pub initial_rage: Option<i32>,
    #[serde(default)]
    pub network_delay: u32,
    /// 是否记录 timeline（rollout 用；训练时建议 false 减开销）
    #[serde(default)]
    pub collect_timeline: bool,
    /// 允许选择的动作 id（None=全部；0 等待自动允许）
    #[serde(default)]
    pub allowed_actions: Option<Vec<u32>>,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub session_id: String,
    pub obs: Vec<f32>,
    pub mask: Vec<bool>,
    pub obs_dim: usize,
    pub action_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ResetResponse {
    pub obs: Vec<f32>,
    pub mask: Vec<bool>,
}

#[derive(Debug, Deserialize)]
pub struct StepRequest {
    pub action: u32,
}

#[derive(Debug, Serialize)]
pub struct StepResponse {
    pub obs: Vec<f32>,
    pub reward: f64,
    pub done: bool,
    pub mask: Vec<bool>,
    pub cast_success: bool,
    pub damage_delta: f64,
}

#[derive(Debug, Serialize)]
pub struct AdvanceResponse {
    pub obs: Vec<f32>,
    pub mask: Vec<bool>,
    pub done: bool,
    /// 跳帧期间 buff tick 累计的伤害（Python 侧可折入 reward）
    pub damage_delta: f64,
}

#[derive(Debug, Deserialize)]
pub struct MacroDecisionRequest {
    pub macro_text: String,
    #[serde(default)]
    pub last_skill: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MacroDecisionResponse {
    pub action: u32,
}

#[derive(Debug, Serialize)]
pub struct EnvInfo {
    pub elapsed: f64,
    pub total_damage: f64,
    pub dps: f64,
    pub rage: i32,
    pub done: bool,
}

#[derive(Debug, Serialize)]
pub struct SpecResponse {
    pub obs_dim: usize,
    pub action_count: usize,
    pub action_names: Vec<&'static str>,
    pub action_skills: Vec<Option<&'static str>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器
// ─────────────────────────────────────────────────────────────────────────────

pub async fn spec_handler() -> Json<SpecResponse> {
    Json(SpecResponse {
        obs_dim: OBS_DIM,
        action_count: ACTION_COUNT,
        action_names: ACTION_NAMES.to_vec(),
        action_skills: ACTION_SKILLS.to_vec(),
    })
}

pub async fn list_sessions_handler(State(shared): State<SharedState>) -> Json<serde_json::Value> {
    let ids = shared.rl_sessions.list().await;
    Json(serde_json::json!({ "sessions": ids }))
}

pub async fn create_handler(
    State(shared): State<SharedState>,
    Json(req): Json<CreateRequest>,
) -> Response {
    let cfg = EnvConfig {
        attrs: req.attributes,
        target: req.target,
        talents: req.talents,
        recipes: req.recipes,
        haste_level: req.haste_level,
        initial_rage: req.initial_rage.unwrap_or(0),
        network_delay: req.network_delay as f64 / 1000.0,
        duration: req.duration.max(1.0),
        collect_timeline: req.collect_timeline,
        allowed_actions: req.allowed_actions.map(|v| {
            let mut mask = [false; ACTION_COUNT];
            mask[0] = true; // 等待始终允许
            for a in v {
                if (a as usize) < ACTION_COUNT {
                    mask[a as usize] = true;
                }
            }
            mask
        }),
    };
    let skills = Arc::new(shared.skills.read().await.clone());
    let recipes = Arc::new(shared.recipes.read().await.clone());
    let mut env = CombatEnv::new(cfg, skills, recipes);
    let (obs, mask) = env.reset();
    let id = shared.rl_sessions.create(env).await;
    Json(CreateResponse {
        session_id: id,
        obs,
        mask: mask.to_vec(),
        obs_dim: OBS_DIM,
        action_count: ACTION_COUNT,
    })
    .into_response()
}

pub async fn reset_handler(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let mut env = env_arc.lock().await;
    let (obs, mask) = env.reset();
    Json(ResetResponse {
        obs,
        mask: mask.to_vec(),
    })
    .into_response()
}

pub async fn step_handler(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<StepRequest>,
) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let mut env = env_arc.lock().await;
    let out = env.step(req.action);
    Json(StepResponse {
        obs: out.obs,
        reward: out.reward,
        done: out.done,
        mask: out.legal_mask.to_vec(),
        cast_success: out.info.cast_success,
        damage_delta: out.info.damage_delta,
    })
    .into_response()
}

/// 合并 step + advance 为一次 HTTP —— 训练主循环专用（减半 HTTP 开销）
/// 语义：执行 action → 若未 done 则立即 advance_to_next_decision
/// reward 已包含 advance 期间 buff tick 伤害
pub async fn step_advance_handler(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<StepRequest>,
) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let mut env = env_arc.lock().await;
    let out = env.step(req.action);
    let step_reward = out.reward;
    let step_done = out.done;
    let mut total_reward = step_reward;
    let mut advance_damage = 0.0f64;
    if !step_done {
        let before = env.total_damage;
        env.advance_to_next_decision();
        advance_damage = env.total_damage - before;
        total_reward += advance_damage / 500_000.0; // 与 BASELINE_PER_HIT 一致
    }
    let done_now = env.done();
    // episode 结束时顺手把 dps 打进响应，Python 侧无需额外 /info 请求
    let episode_dps = if done_now {
        Some(env.fight_dps())
    } else {
        None
    };
    let total_damage = if done_now {
        Some(env.total_damage)
    } else {
        None
    };
    Json(serde_json::json!({
        "obs": env.observe(),
        "mask": env.legal_mask().to_vec(),
        "reward": total_reward,
        "done": done_now,
        "cast_success": out.info.cast_success,
        "step_damage": out.info.damage_delta,
        "advance_damage": advance_damage,
        "episode_dps": episode_dps,
        "episode_total_damage": total_damage,
    }))
    .into_response()
}

pub async fn advance_handler(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let mut env = env_arc.lock().await;
    let before = env.total_damage;
    env.advance_to_next_decision();
    Json(AdvanceResponse {
        obs: env.observe(),
        mask: env.legal_mask().to_vec(),
        done: env.done(),
        damage_delta: env.total_damage - before,
    })
    .into_response()
}

pub async fn macro_decision_handler(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<MacroDecisionRequest>,
) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let cfg = match parse_macro_text(&req.macro_text) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("宏解析失败: {}", e) })),
            )
                .into_response();
        }
    };
    let env = env_arc.lock().await;
    let action = env.macro_decision(&cfg, req.last_skill);
    Json(MacroDecisionResponse { action }).into_response()
}

pub async fn info_handler(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(env_arc) = shared.rl_sessions.get(&id).await else {
        return not_found(&id);
    };
    let env = env_arc.lock().await;
    Json(EnvInfo {
        elapsed: env.elapsed(),
        total_damage: env.total_damage,
        dps: env.fight_dps(),
        rage: env.player.rage,
        done: env.done(),
    })
    .into_response()
}

pub async fn close_handler(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let removed = shared.rl_sessions.remove(&id).await;
    Json(serde_json::json!({ "closed": removed })).into_response()
}

fn not_found(id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("session 不存在: {}", id) })),
    )
        .into_response()
}
