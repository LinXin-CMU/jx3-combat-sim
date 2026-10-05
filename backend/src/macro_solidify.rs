//! A frozen macro cast is a concrete action plus its relative wait checkpoints.
//! Checkpoints preserve event scheduling (including expected cooldown resets),
//! without retaining or evaluating any macro conditions.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::{CastEvent, Player, SkillSpec};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FrozenCast {
    pub skill_id: u32,
    pub waits: Vec<f64>,
    pub fcast: bool,
}

pub fn interrupt(player: &mut Player, skill: &SkillSpec, timeline: &mut [CastEvent]) {
    if player.channel_end <= player.current_time + 0.001 { return; }
    let skill_id = player.channel_skill_id;
    let at = player.next_cast_time(skill);
    let (old, ticks) = player.interrupt_channel(at);
    if let Some(ev) = timeline.iter_mut().rev().find(|e| e.skill_id == skill_id && !e.triggered) {
        ev.channel_ticks = Some(ticks);
        let first = crate::frames_to_sec(player.channel_first_frame);
        let interval = crate::frames_to_sec(player.channel_interval_frame);
        ev.channel_duration = Some(first + ticks.saturating_sub(1) as f64 * interval);
    }
    if skill_id == 13048 && player.stance() == crate::Stance::Shield {
        player.add_rage(-(old as i32 - ticks as i32).max(0));
    }
}

pub fn replay(
    frozen: &FrozenCast, name: &str, player: &mut Player,
    skill_map: &HashMap<&str, Vec<&SkillSpec>>, timeline: &mut Vec<CastEvent>,
    prev_time: &mut f64, first: &mut bool, ctx: &crate::macro_eval::CastCtx,
) -> Result<(), String> {
    // Validate all checkpoints before advancing any state.
    let mut previous = 0.0;
    for &dt in &frozen.waits {
        if !dt.is_finite() || dt < previous || !(player.current_time + dt).is_finite() {
            return Err("固化等待记录无效".into());
        }
        previous = dt;
    }
    let start = player.current_time;
    let mut busy = player.active_cds.iter().filter(|(k, _)| k.starts_with("gcd_"))
        .map(|(_, &v)| v).fold(player.channel_end, f64::max);
    for &dt in &frozen.waits {
        let target = start + dt;
        let mut events = player.process_buff_ticks(*prev_time, target);
        crate::fill_tick_events(&mut events, ctx.skill_by_id, ctx.dmg_ctx, ctx.recipes_table, player);
        timeline.extend(events);
        *prev_time = target;
        player.current_time = target;
    }
    let ranks = skill_map.get(name).ok_or("固化技能不存在")?;
    let rank = player.pick_rank(ranks).ok_or("固化技能当前不可施放")?;
    let skill = crate::resolve_combo_follow(rank, player, ctx.skill_by_id).unwrap_or(rank);
    if skill.skill_id != frozen.skill_id { return Err("固化技能段数或状态已改变".into()); }
    if frozen.fcast { interrupt(player, skill, timeline); }
    let outcome = crate::macro_eval::execute_cast(player, skill, prev_time, first, &mut busy, ctx);
    timeline.extend(outcome.events);
    if outcome.cast_success { Ok(()) } else { Err("固化技能施放失败".into()) }
}

#[cfg(test)]
#[path = "../tests/macro_solidify/mod.rs"]
mod tests;

