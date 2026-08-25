//! RL 观测向量化 + 合法动作 mask。
//!
//! 总维度 = `OBS_DIM` = 82，全部 f32 归一化到 [0,1]。
//!
//! 设计原则：
//! - **层数信息一律用 one-hot 编码**（充能层数、buff 层数）。
//!   - 充能：每个充能技能贡献 `max_charges` 维 one-hot（无"剩余时间"概念）。
//!   - 多层 buff：贡献 `max_stacks` 维 one-hot + 1 维剩余时间。
//!   - 单层 buff：贡献 1 维剩余时间。
//! - **CD 进度**用单值（1=就绪，0=刚触发）。
//! - **GCD 归一化**对齐前端 GCD 条逻辑：当前活跃 GCD 的 `remaining / total_actual`
//!   （`get_actual_frames` 按加速换算）。
//! - **buff 剩余时间**按 `BuffDef.duration_frames / 16` 归一（流血等 haste_scaled 按基准长度）。
//! - **阵云_2/阵云_3** 由 `combo_buff_id` 合成 ID 不在 buff_def 表里；持续时间均为 720 帧 = 45s
//!   （30855_月照连营.toml combo_duration=720 + zhen_yun.rs add_state_buff(..., 720) 一致）。

use std::collections::HashMap;

use crate::{
    combo_buff_id, frames_to_sec, get_actual_frames, sec_to_frames, BUFF_CHENG_WU, BUFF_DUN_FEI,
    BUFF_FENG_MING, BUFF_JIAN_DING, BUFF_JIE_HUA, BUFF_KUANG_JUE, BUFF_LIN_AN, BUFF_LIN_GUANG,
    BUFF_LIU_XUE, BUFF_SHI_XUE, BUFF_XUE_NU, BUFF_XUE_NU_JY, BUFF_XUE_SHI_COUNT, BUFF_XU_RUO,
    BUFF_YUAN_GE_ID, Player, SkillSpec, Stance,
};

use super::action::{ACTION_COUNT, ACTION_SKILLS, WAIT_ACTION};

/// 阵云连段 buff 的最大持续时间（秒）—— 与脚本 / TOML 中 720 帧硬编码一致
const COMBO_ZHEN_YUN_SEC: f64 = 720.0 / 16.0;

/// 充能技能（顺序固定；每个贡献 max_charges 维 one-hot）
/// 注：max 取**奇穴修饰后的上限**（参考 `Player::effective_max_charges`）：
/// - 盾击：基础 3 + 援戈奇穴(36058) +1 = 4
/// - 盾飞 / 血怒：3
/// - 阵云结晦：2
const CHARGE_SKILLS: &[(&str, u32)] = &[
    ("盾击", 4),
    ("盾飞", 3),
    ("血怒", 3),
    ("阵云结晦", 2),
];

/// 单层自身 buff（贡献 1 维剩余时间）
const SELF_BUFFS_SINGLE: &[u32] = &[
    BUFF_XUE_NU_JY,
    BUFF_JIE_HUA,
    BUFF_FENG_MING,
    BUFF_JIAN_DING,
    BUFF_KUANG_JUE,
    BUFF_SHI_XUE,
    BUFF_CHENG_WU,
    BUFF_LIN_GUANG,
    BUFF_LIN_AN,
    BUFF_DUN_FEI,
];

/// 多层自身 buff（贡献 max_stacks 维 one-hot + 1 维剩余时间）
const SELF_BUFFS_MULTI: &[(u32, u32)] = &[
    (BUFF_XUE_NU, 3),       // 血怒
    (BUFF_YUAN_GE_ID, 7),   // 援戈
];

/// 单层目标 debuff
const TARGET_BUFFS_SINGLE: &[u32] = &[BUFF_XU_RUO, BUFF_LIU_XUE];

/// 多层目标 debuff
const TARGET_BUFFS_MULTI: &[(u32, u32)] = &[
    (BUFF_XUE_SHI_COUNT, 2),
];

// ─────────────────────────────────────────────────────────────────────────────
// 维度计算（编译期常量，便于其他模块引用 OBS_DIM）
// ─────────────────────────────────────────────────────────────────────────────

const RAGE_DIM: usize = 1;
const STANCE_DIM: usize = 3;
const CD_DIM: usize = ACTION_COUNT; // 18
const CHARGES_DIM: usize = sum_charges();
const SELF_SINGLE_DIM: usize = SELF_BUFFS_SINGLE.len();
const SELF_MULTI_DIM: usize = sum_multi(SELF_BUFFS_MULTI);
const COMBO_DIM: usize = 2; // 阵云_2 / 阵云_3 剩余
const TARGET_SINGLE_DIM: usize = TARGET_BUFFS_SINGLE.len();
const TARGET_MULTI_DIM: usize = sum_multi(TARGET_BUFFS_MULTI);
const TIME_DIM: usize = 2; // elapsed + gcd
const LAST_ACTION_DIM: usize = ACTION_COUNT;

const fn sum_charges() -> usize {
    let mut total = 0usize;
    let mut i = 0;
    while i < CHARGE_SKILLS.len() {
        total += CHARGE_SKILLS[i].1 as usize;
        i += 1;
    }
    total
}

const fn sum_multi(arr: &[(u32, u32)]) -> usize {
    let mut total = 0usize;
    let mut i = 0;
    while i < arr.len() {
        total += arr[i].1 as usize + 1; // one-hot + 剩余
        i += 1;
    }
    total
}

pub const OBS_DIM: usize = RAGE_DIM
    + STANCE_DIM
    + CD_DIM
    + CHARGES_DIM
    + SELF_SINGLE_DIM
    + SELF_MULTI_DIM
    + COMBO_DIM
    + TARGET_SINGLE_DIM
    + TARGET_MULTI_DIM
    + TIME_DIM
    + LAST_ACTION_DIM;

// ─────────────────────────────────────────────────────────────────────────────
// observe / legal_mask
// ─────────────────────────────────────────────────────────────────────────────

pub fn observe(
    player: &Player,
    skill_map: &HashMap<&str, Vec<&SkillSpec>>,
    last_action: usize,
    elapsed: f64,
    duration: f64,
) -> Vec<f32> {
    let mut obs = vec![0.0f32; OBS_DIM];
    let mut o = 0usize;

    // 怒气
    obs[o] = (player.rage as f32 / 100.0).clamp(0.0, 1.0);
    o += RAGE_DIM;

    // 姿态 one-hot
    match player.stance() {
        Stance::Shield => obs[o] = 1.0,
        Stance::Blade => obs[o + 1] = 1.0,
        Stance::Wall => obs[o + 2] = 1.0,
        _ => {}
    }
    o += STANCE_DIM;

    // 18 动作的 CD 就绪度
    for (i, slot) in ACTION_SKILLS.iter().enumerate() {
        let Some(name) = slot else { continue; };
        if let Some(ranks) = skill_map.get(name) {
            if let Some(spec) = ranks.first() {
                obs[o + i] = cd_progress(player, spec);
            }
        }
    }
    o += CD_DIM;

    // 充能 one-hot（每个技能 max_charges 维；当前层数 k → dim k-1 = 1）
    for (name, declared_max) in CHARGE_SKILLS.iter() {
        if let Some(ranks) = skill_map.get(*name) {
            if let Some(spec) = ranks.first() {
                let cnt = player.get_charge_count(spec).min(*declared_max);
                if cnt >= 1 {
                    obs[o + (cnt as usize - 1)] = 1.0;
                }
            }
        }
        o += *declared_max as usize;
    }

    // 单层自身 buff（剩余时间）
    for &id in SELF_BUFFS_SINGLE.iter() {
        obs[o] = buff_remaining_norm(player, player.buff_remaining(id), id);
        o += 1;
    }

    // 多层自身 buff（one-hot + 剩余）
    for (id, max_stacks) in SELF_BUFFS_MULTI.iter() {
        let stacks = player.buff_stacks(*id).min(*max_stacks);
        if stacks >= 1 {
            obs[o + (stacks as usize - 1)] = 1.0;
        }
        o += *max_stacks as usize;
        obs[o] = buff_remaining_norm(player, player.buff_remaining(*id), *id);
        o += 1;
    }

    // 阵云连段 buff 剩余（连续值）
    let combo2 = player.buff_remaining(combo_buff_id("阵云_2")).unwrap_or(0.0);
    let combo3 = player.buff_remaining(combo_buff_id("阵云_3")).unwrap_or(0.0);
    obs[o] = (combo2 / COMBO_ZHEN_YUN_SEC).clamp(0.0, 1.0) as f32;
    obs[o + 1] = (combo3 / COMBO_ZHEN_YUN_SEC).clamp(0.0, 1.0) as f32;
    o += COMBO_DIM;

    // 单层目标 debuff
    for &id in TARGET_BUFFS_SINGLE.iter() {
        obs[o] = buff_remaining_norm(player, player.target_buff_remaining(id), id);
        o += 1;
    }

    // 多层目标 debuff（one-hot + 剩余）
    for (id, max_stacks) in TARGET_BUFFS_MULTI.iter() {
        let stacks = player.target_buff_stacks(*id).min(*max_stacks);
        if stacks >= 1 {
            obs[o + (stacks as usize - 1)] = 1.0;
        }
        o += *max_stacks as usize;
        obs[o] = buff_remaining_norm(player, player.target_buff_remaining(*id), *id);
        o += 1;
    }

    // 时间 / GCD
    if duration > 0.0 {
        obs[o] = (elapsed / duration).clamp(0.0, 1.0) as f32;
    }
    obs[o + 1] = gcd_progress(player) as f32;
    o += TIME_DIM;

    // 上一动作 one-hot
    if last_action < ACTION_COUNT {
        obs[o + last_action] = 1.0;
    }
    o += LAST_ACTION_DIM;

    debug_assert_eq!(o, OBS_DIM);
    obs
}

fn buff_remaining_norm(player: &Player, remaining: Option<f64>, buff_id: u32) -> f32 {
    let Some(r) = remaining else { return 0.0; };
    let max_dur = player.buff_def(buff_id)
        .map(|d| d.duration_frames as f64 / 16.0)
        .unwrap_or(0.0);
    if r.is_finite() && max_dur > 0.0 {
        (r / max_dur).clamp(0.0, 1.0) as f32
    } else if r.is_finite() {
        0.0
    } else {
        1.0 // 永久
    }
}

fn gcd_progress(player: &Player) -> f64 {
    let (gcd_end, total_base) = player.active_cds.iter()
        .filter(|(k, _)| k.starts_with("gcd_"))
        .fold((0.0_f64, 0.0_f64), |(best_end, best_total), (k, &v)| {
            if v > best_end {
                let dur = k.strip_prefix("gcd_").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                (v, dur)
            } else {
                (best_end, best_total)
            }
        });
    if total_base <= 0.0 { return 0.0; }
    let total_actual = frames_to_sec(get_actual_frames(sec_to_frames(total_base), player.effective_haste_level()));
    if total_actual <= 0.0 { return 0.0; }
    let remaining = (gcd_end - player.current_time).max(0.0);
    (remaining / total_actual).clamp(0.0, 1.0)
}

fn cd_progress(player: &Player, spec: &SkillSpec) -> f32 {
    if spec.max_charges > 0 {
        let cnt = player.get_charge_count(spec) as f32;
        return (cnt / spec.max_charges as f32).clamp(0.0, 1.0);
    }
    let mut worst: f32 = 1.0;
    for cd in &spec.cooldowns {
        if !cd.cd_id.starts_with("cd_") { continue; }
        let max = cd.duration.max(0.001);
        let expires = player.active_cds.get(&cd.cd_id).copied().unwrap_or(0.0);
        let remaining = (expires - player.current_time).max(0.0);
        let ready = (1.0 - (remaining / max)).clamp(0.0, 1.0) as f32;
        if ready < worst { worst = ready; }
    }
    worst
}

pub fn legal_mask(
    player: &Player,
    skill_map: &HashMap<&str, Vec<&SkillSpec>>,
) -> [bool; ACTION_COUNT] {
    let mut mask = [false; ACTION_COUNT];
    mask[WAIT_ACTION] = true;

    for (i, slot) in ACTION_SKILLS.iter().enumerate() {
        if i == WAIT_ACTION { continue; }
        let Some(name) = slot else { continue; };
        let Some(ranks) = skill_map.get(name) else { continue; };
        let Some(spec) = player.pick_rank(ranks) else { continue; };
        if !player.can_cast(spec) { continue; }
        let ready = player.next_cast_time(spec);
        if ready <= player.current_time + 1e-3 {
            mask[i] = true;
        }
    }
    mask
}
