//! 策略回放：把一条策略跑完，产出与 `/api/simulate` 同 schema 的 CastEvent 时间轴。
//!
//! 两种策略：
//! - `Policy::Macro`    — 用宏驱动（内部调 `simulate_macro`，零额外代码）
//! - `Policy::Actions`  — 回放一串离散动作（Python 训练得到的轨迹）
//!
//! 前端循环模拟页可直接渲染返回的 timeline。

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::macro_eval::simulate_macro;
use crate::{Attributes, CastEvent, Player, RecipeEntry, SkillSpec, TargetConfig};

use super::env::{CombatEnv, EnvConfig};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Policy {
    /// 用宏驱动：macro_text 是宏页文本
    Macro { macro_text: String },
    /// 回放动作序列：每个元素是动作 ID（0..18）；长度应 ≥ 决策点数
    Actions { actions: Vec<u32> },
}

#[derive(Debug, Deserialize)]
pub struct RolloutRequest {
    pub policy: Policy,
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
}

#[derive(Debug, Serialize)]
pub struct RolloutResponse {
    pub timeline: Vec<CastEvent>,
    pub total_damage: f64,
    pub dps: f64,
    pub fight_time: f64,
    pub skill_count: usize,
}

pub fn run_rollout(
    req: RolloutRequest,
    skills: Arc<Vec<SkillSpec>>,
    recipes_table: Arc<Vec<RecipeEntry>>,
) -> Result<RolloutResponse, String> {
    let duration = req.duration.max(1.0);
    let delay_sec = req.network_delay as f64 / 1000.0;
    let initial_rage = req.initial_rage.unwrap_or(0);

    match req.policy {
        Policy::Macro { macro_text } => {
            let cfg = crate::macro_parser::parse_macro_text(&macro_text)
                .map_err(|e| format!("宏解析失败: {}", e))?;

            let mut skill_map: HashMap<&str, Vec<&SkillSpec>> = HashMap::new();
            for s in skills.iter() {
                if s.passive {
                    continue;
                }
                let base = s.name.split('·').next().unwrap_or(&s.name);
                skill_map.entry(base).or_default().push(s);
            }
            let skill_by_id: HashMap<u32, &SkillSpec> =
                skills.iter().map(|s| (s.skill_id, s)).collect();

            let mut player = Player::new(req.haste_level, req.talents.clone(), req.recipes.clone());
            player.rage = initial_rage.clamp(0, 100);
            let mut prev_time = 0.0f64;
            let mut is_first_main = true;
            let dmg_ctx = Some((req.attributes.clone(), req.target.clone()));
            let max_slots = (duration / 0.4).ceil() as u32 + 10;

            let (timeline, _, _) = simulate_macro(
                &cfg,
                &mut player,
                &skill_map,
                max_slots,
                duration,
                delay_sec,
                &mut prev_time,
                &mut is_first_main,
                None,
                dmg_ctx.as_ref(),
                &recipes_table,
                &skill_by_id,
                &[],
            );
            Ok(summarize(timeline, &player, duration))
        }

        Policy::Actions { actions } => {
            let cfg = EnvConfig {
                attrs: req.attributes,
                target: req.target,
                talents: req.talents,
                recipes: req.recipes,
                haste_level: req.haste_level,
                initial_rage,
                network_delay: delay_sec,
                duration,
                collect_timeline: true,
                allowed_actions: None,
            };
            let mut env = CombatEnv::new(cfg, skills, recipes_table);
            env.reset();

            let mut idx = 0usize;
            while !env.done() {
                env.advance_to_next_decision();
                if env.done() {
                    break;
                }
                let action = actions
                    .get(idx)
                    .copied()
                    .unwrap_or(super::action::WAIT_ACTION as u32);
                env.step(action);
                idx += 1;
                if idx > 100_000 {
                    break;
                } // 安全阈值
            }
            let duration_real = duration;
            Ok(summarize(
                std::mem::take(&mut env.timeline),
                &env.player,
                duration_real,
            ))
        }
    }
}

fn summarize(timeline: Vec<CastEvent>, player: &Player, duration: f64) -> RolloutResponse {
    let last_cast = timeline
        .iter()
        .rev()
        .find(|e| !e.triggered)
        .map(|e| e.cast_time)
        .unwrap_or(0.0);
    let fight_time = player.fight_end(last_cast).min(duration).max(0.001);
    let total_damage: f64 = timeline.iter().filter_map(|e| e.damage_total).sum();
    let dps = total_damage / fight_time;
    let skill_count = timeline.iter().filter(|e| !e.triggered).count();
    RolloutResponse {
        timeline,
        total_damage,
        dps,
        fight_time,
        skill_count,
    }
}
