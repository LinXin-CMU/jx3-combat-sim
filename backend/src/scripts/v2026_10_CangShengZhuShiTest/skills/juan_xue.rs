//! 卷雪刀（平砍）脚本 (ID: 13039)
//!
//! - 自动周期触发，间隔 = 基础 25帧 受加速影响
//! - 系数 = (实际帧数 - 1) × 0.00625
//! - 不被任何主动技能打断，不吃技能秘籍，不触发苍雪刀公共效果
//! - 首个主动技能 cast 时启动循环

use crate::*;

/// 平砍基础间隔（24 帧 = 1.5s，0 加速档）
pub const BASE_FRAMES: u32 = 24;

/// 当前加速下的实际间隔帧数
pub fn interval_frames(haste_level: u32) -> u32 {
    get_actual_frames(BASE_FRAMES, haste_level).max(1)
}

/// 当前加速下的实际攻击系数
pub fn attack_coeff(haste_level: u32) -> f64 {
    let frames = interval_frames(haste_level);
    (frames.saturating_sub(1) as f64) * 0.00625
}

/// 产出 (last_swing_time, to_time] 区间内所有平砍事件
/// 仅生成 CastEvent（伤害字段空着，主流程通过 fill_event_damage 算）
pub fn process_swings(player: &mut Player, to_time: f64) -> Vec<CastEvent> {
    let last = match player.last_swing_time {
        Some(t) => t,
        None => return Vec::new(),
    };
    let interval = frames_to_sec(interval_frames(player.effective_haste_level()));
    if interval <= 0.0 {
        return Vec::new();
    }

    let mut events = Vec::new();
    let mut t = last + interval;
    while t <= to_time + 0.0001 {
       events.push(CastEvent {
            sequence_index: None,
           name: "卷雪刀".into(),
            skill_id: 13039,
            cast_time: t,
            triggered: true,
            gcd: 0.0,
            is_main: false,
            cd_wait: 0.0,
            channel_ticks: None,
            max_channel_ticks: None,
            channel_duration: None,
            timing_offset: None,
            max_timing_offset: None,
            available_buffs: None,
            is_macro: false,
            macro_page: None,
            macro_line: None,
            rage_after: None,
            rage_delta: None,
            rage_overflow: None,
            rage_overflow_sources: Vec::new(),
            rage_transactions: Vec::new(),
            rage_generated: None,
            rage_gained: None,
            rage_spent: None,
            rage_cost: None,
            state_before: None,
            state_after: None,
            damage: None,
            damage_normal: None,
            damage_crit: None,
            damage_total: None,
            runtime_recipes: Vec::new(),
            runtime_stats: None,
            override_attack_coeff: None,
            applied_recipes: Vec::new(),
        });
        t += interval;
    }
    if let Some(last_ev) = events.last() {
        player.last_swing_time = Some(last_ev.cast_time);
    }
    events
}
