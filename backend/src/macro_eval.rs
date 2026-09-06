//! 宏求值器：两阶段执行模型 + 宏模拟主循环

use crate::{frames_to_sec, macro_engine::*, CastEvent, Player, RecipeEntry, SkillSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ─────────────────────────────────────────────────────────────────────────────
// 调试输出结构
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct MacroStepDebug {
    pub time: f64,
    pub page: usize,
    pub last_skill: String,
    pub phase1: Vec<Phase1LineDebug>,
    pub phase2: Vec<Phase2EntryDebug>,
    pub selected: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Phase1LineDebug {
    pub line: usize,
    pub condition: String,
    pub skill: String,
    pub passed: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct Phase2EntryDebug {
    pub line: usize,
    pub skill: String,
    pub castable: bool,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct MacroLineExecutionStats {
    pub page: usize,
    pub line: usize,
    pub skill: String,
    pub condition: String,
    pub evaluations: u32,
    pub condition_passes: u32,
    pub condition_failures: u32,
    pub selected_casts: u32,
    pub priority_bypassed: u32,
    pub castability_failures: BTreeMap<String, u32>,
}

pub fn merge_line_execution_stats(
    accumulated: &mut Vec<MacroLineExecutionStats>,
    next: Vec<MacroLineExecutionStats>,
) {
    if accumulated.is_empty() {
        *accumulated = next;
        return;
    }
    for next_item in next {
        let Some(item) = accumulated
            .iter_mut()
            .find(|item| item.page == next_item.page && item.line == next_item.line)
        else {
            accumulated.push(next_item);
            continue;
        };
        item.evaluations = item.evaluations.saturating_add(next_item.evaluations);
        item.condition_passes = item
            .condition_passes
            .saturating_add(next_item.condition_passes);
        item.condition_failures = item
            .condition_failures
            .saturating_add(next_item.condition_failures);
        item.selected_casts = item.selected_casts.saturating_add(next_item.selected_casts);
        item.priority_bypassed = item
            .priority_bypassed
            .saturating_add(next_item.priority_bypassed);
        for (reason, count) in next_item.castability_failures {
            let total = item.castability_failures.entry(reason).or_default();
            *total = total.saturating_add(count);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Buff 名称 → ID 映射
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn buff_name_to_id(name: &str) -> Option<u32> {
    use crate::*;
    match name {
        "血怒" => Some(BUFF_XUE_NU),
        "血怒·惊涌" => Some(BUFF_XUE_NU_JY),
        "劫化" => Some(BUFF_JIE_HUA),
        "锋鸣" => Some(BUFF_FENG_MING),
        "坚定" => Some(BUFF_JIAN_DING),
        "狂绝" => Some(BUFF_KUANG_JUE),
        "怒炎" => Some(BUFF_NU_YAN),
        "激昂" => Some(BUFF_JI_ANG),
        "无惧" => Some(BUFF_WU_JU),
        "嗜血" => Some(BUFF_SHI_XUE),
        "援戈" => Some(BUFF_YUAN_GE_ID),
        "橙武" | "驭焰" | "天下宏愿" => Some(BUFF_CHENG_WU),
        "麟光玄甲" | "麟光甲" => Some(BUFF_LIN_GUANG),
        "麟黯" => Some(BUFF_LIN_AN),
        "盾飞" => Some(BUFF_DUN_FEI),
        "虚弱" => Some(BUFF_XU_RUO),
        "流血" => Some(BUFF_LIU_XUE),
        "卷云" => Some(BUFF_JUAN_YUN),
        "步残" => Some(BUFF_BU_CAN),
        "缓深" => Some(BUFF_HUAN_SHEN),
        "血誓" => Some(BUFF_XUE_SHI),
        "以血盟誓" => Some(BUFF_XUE_SHI_COUNT),
        "战绝" => Some(BUFF_ZHAN_JUE),
        "盾挡" => Some(BUFF_DUN_DANG),
        "千山盾挡" => Some(BUFF_DUN_DANG_QIAN_SHAN),
        "寒啸千军" => Some(BUFF_HAN_XIAO),
        "蔑视" => Some(BUFF_MIE_SHI),
        "振奋" => Some(BUFF_ZHEN_FEN),
        "寒甲" => Some(BUFF_HAN_JIA),
        "坚铁" => Some(BUFF_JIAN_TIE),
        "盾威" => Some(BUFF_DUN_WEI),
        "严阵" => Some(BUFF_YAN_ZHEN),
        "铁骨" => Some(BUFF_TIE_GU),
        "铁骨·宿敌" | "宿敌" => Some(BUFF_TIE_GU_SU_DI),
        "切换至盾姿态" | "擎盾" => Some(BUFF_STANCE_SHIELD_GAME),
        "切换至刀姿态" | "擎刀" => Some(BUFF_STANCE_BLADE_GAME),
        _ => {
            // 尝试按数字 ID 解析（如 buff:8249）
            name.parse::<u32>().ok()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 阶段一：条件求值
// ─────────────────────────────────────────────────────────────────────────────

/// 阶段一求值状态
struct Phase1State<'a> {
    player: &'a Player,
    skill_map: &'a HashMap<&'a str, Vec<&'a SkillSpec>>,
    skill_by_id: &'a HashMap<u32, &'a SkillSpec>,
    last_skill: Option<String>,
    /// buff_id → active_buffs 索引的引用（来自 Player 的 gen-keyed 缓存，跨多次 phase1 复用）。
    /// 调用 player.active_buffs[idx] 解引到完整 BuffInstance。
    buff_lookup: std::cell::Ref<'a, ahash::AHashMap<u32, usize>>,
    target_buff_lookup: std::cell::Ref<'a, ahash::AHashMap<u32, usize>>,
}

/// 技能池条目
pub struct PoolEntry {
    pub line: usize,
    pub skill_name: String,
    pub is_fcast: bool,
}

/// 阶段一：扫描宏页所有行，构建技能池
///
/// `enable_debug=false` 时跳过 `Vec<Phase1LineDebug>` 构建（含 `display_string` /
/// `skill_name.to_string()` 等堆分配），返回空 Vec。
/// 主循环（无前端调试请求）传 false；前端调试路径未来若需要可传 true。
pub fn evaluate_phase1<'a>(
    page: &MacroPage,
    player: &'a Player,
    skill_map: &'a HashMap<&'a str, Vec<&'a SkillSpec>>,
    skill_by_id: &'a HashMap<u32, &'a SkillSpec>,
    last_skill_from_prev: Option<String>,
    enable_debug: bool,
) -> (Vec<PoolEntry>, Option<String>, Vec<Phase1LineDebug>) {
    // buff lookup 走 Player 的 gen-keyed 缓存：跨多次 phase1 调用复用，
    // 仅在 buff_generation 变化时重建。
    // 前置不变量：process_buff_ticks 已在 phase1 之前清理过期 buff，因此
    // active_buffs 里所有项都"当前活跃"（expires_at == 0 或 > current_time），
    // 无需再过滤 expires_at。
    let buff_lookup = player.buff_idx_lookup();
    let target_buff_lookup = player.target_idx_lookup();

    // last_skill 只反映"上一次真正成功释放的技能"（由调用方在成功 cast 后传入），
    // phase 1 内部扫描时 **不再**实时更新。
    let state = Phase1State {
        player,
        skill_map,
        skill_by_id,
        last_skill: last_skill_from_prev.clone(),
        buff_lookup,
        target_buff_lookup,
    };

    let mut pool = Vec::new();
    let mut debug = Vec::new();

    for (i, line) in page.lines.iter().enumerate() {
        let passes = match &line.condition {
            None => true,
            Some(cond) => eval_condition(cond, &state),
        };

        if enable_debug {
            let cond_str = match &line.condition {
                None => "(无条件)".to_string(),
                Some(c) => c.display_string(),
            };
            let skill_name = line.action.skill_name().to_string();
            debug.push(Phase1LineDebug {
                line: i + 1,
                condition: cond_str,
                skill: skill_name,
                passed: passes,
            });
        }

        if passes {
            pool.push(PoolEntry {
                line: i + 1,
                skill_name: line.action.skill_name().to_string(),
                is_fcast: line.action.is_fcast(),
            });
        }
    }

    // 返回的 last 仍是传入值（未变），保持调用方接口兼容
    let last = last_skill_from_prev;
    (pool, last, debug)
}

/// 条件递归求值
fn eval_condition(cond: &MacroCondition, state: &Phase1State) -> bool {
    let _t0 = std::time::Instant::now();
    let _guard = crate::scopeguard_perf(
        |ns| {
            crate::perf_add(|p| {
                p.macro_cond_n += 1;
                p.macro_cond_ns += ns;
            })
        },
        _t0,
    );
    match cond {
        MacroCondition::Rage(op, val) => op.compare_i32(state.player.rage, *val),
        MacroCondition::Life(op, val) => {
            // 模拟器默认满血
            op.compare_f64(1.0, *val)
        }
        MacroCondition::Buff(name) => {
            // 走 per-call lookup，O(1)；旧版是 player.has_buff 内部 O(N) 扫
            buff_name_to_id(name)
                .map(|id| state.buff_lookup.contains_key(&id))
                .unwrap_or(false)
        }
        MacroCondition::NoBuff(name) => buff_name_to_id(name)
            .map(|id| !state.buff_lookup.contains_key(&id))
            .unwrap_or(true),
        MacroCondition::BuffStack(name, op, val) => buff_name_to_id(name)
            .map(|id| {
                let stacks = state
                    .buff_lookup
                    .get(&id)
                    .map(|&idx| state.player.active_buffs[idx].stacks)
                    .unwrap_or(0);
                op.compare_u32(stacks, *val)
            })
            .unwrap_or(false),
        MacroCondition::BuffTime(name, op, val) => {
            // buff 不存在 → 整条判断返 false（"不存在"不应该被 <N 误判为"剩余<N"）
            buff_name_to_id(name)
                .and_then(|id| state.buff_lookup.get(&id).copied())
                .map(|idx| {
                    let inst = &state.player.active_buffs[idx];
                    let r = if inst.expires_at == 0.0 {
                        f64::MAX
                    } else {
                        (inst.expires_at - state.player.current_time).max(0.0)
                    };
                    op.compare_f64(r, *val)
                })
                .unwrap_or(false)
        }
        MacroCondition::TBuff(name) => buff_name_to_id(name)
            .map(|id| state.target_buff_lookup.contains_key(&id))
            .unwrap_or(false),
        MacroCondition::TnoBuff(name) => buff_name_to_id(name)
            .map(|id| !state.target_buff_lookup.contains_key(&id))
            .unwrap_or(true),
        MacroCondition::TBuffTime(name, op, val) => buff_name_to_id(name)
            .and_then(|id| state.target_buff_lookup.get(&id).copied())
            .map(|idx| {
                let inst = &state.player.target_buffs[idx];
                let r = if inst.expires_at == 0.0 {
                    f64::MAX
                } else {
                    (inst.expires_at - state.player.current_time).max(0.0)
                };
                op.compare_f64(r, *val)
            })
            .unwrap_or(false),
        MacroCondition::SkillNotInCd(name) => {
            // 按名字查，fallback 按数字 ID 查
            let ranks = state.skill_map.get(name.as_str()).or_else(|| {
                name.parse::<u32>()
                    .ok()
                    .and_then(|id| state.skill_by_id.get(&id))
                    .and_then(|spec| {
                        let base = spec.name.split('·').next().unwrap_or(&spec.name);
                        state.skill_map.get(base)
                    })
            });
            if let Some(ranks) = ranks {
                if let Some(skill) = ranks.first() {
                    return state.player.is_skill_not_in_cd(skill);
                }
            }
            false
        }
        MacroCondition::SkillExists(id) => state.player.has_talent(*id),
        MacroCondition::SkillNotExists(id) => !state.player.has_talent(*id),
        MacroCondition::SkillEnergy(name, op, val) => {
            let ranks = state.skill_map.get(name.as_str()).or_else(|| {
                name.parse::<u32>()
                    .ok()
                    .and_then(|id| state.skill_by_id.get(&id))
                    .and_then(|spec| {
                        let base = spec.name.split('·').next().unwrap_or(&spec.name);
                        state.skill_map.get(base)
                    })
            });
            if let Some(ranks) = ranks {
                if let Some(skill) = ranks.first() {
                    let count = state.player.get_charge_count(skill);
                    return op.compare_u32(count, *val);
                }
            }
            false
        }
        MacroCondition::LastSkill(name) => {
            // 支持名字或数字 ID
            if let Some(last) = state.last_skill.as_deref() {
                if last == name.as_str() {
                    return true;
                }
                // 数字 ID：查技能名比较
                if let Some(id) = name.parse::<u32>().ok() {
                    if let Some(spec) = state.skill_by_id.get(&id) {
                        let base = spec.name.split('·').next().unwrap_or(&spec.name);
                        return last == base;
                    }
                }
            }
            false
        }
        MacroCondition::LastSkillNot(name) => {
            if let Some(last) = state.last_skill.as_deref() {
                if last == name.as_str() {
                    return false;
                }
                if let Some(id) = name.parse::<u32>().ok() {
                    if let Some(spec) = state.skill_by_id.get(&id) {
                        let base = spec.name.split('·').next().unwrap_or(&spec.name);
                        return last != base;
                    }
                }
            }
            true
        }
        MacroCondition::NearbyEnemy(op, val) => {
            // 模拟器默认1个目标
            op.compare_u32(1, *val)
        }
        MacroCondition::And(a, b) => eval_condition(a, state) && eval_condition(b, state),
        MacroCondition::Or(a, b) => eval_condition(a, state) || eval_condition(b, state),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 阶段二：角色状态检查
// ─────────────────────────────────────────────────────────────────────────────

/// 阶段二结果
pub struct Phase2Result<'a> {
    pub skill: &'a SkillSpec,
    pub line: usize,
    pub skill_name: String,
    pub is_fcast: bool,
}

/// 阶段二：从技能池中选出第一个可释放的技能
///
/// `enable_debug=false` 时跳过 `Vec<Phase2EntryDebug>` 构建（含 format! / clone String），
/// 返回空 Vec。
pub fn evaluate_phase2<'a>(
    pool: &[PoolEntry],
    player: &Player,
    skill_map: &'a HashMap<&str, Vec<&'a SkillSpec>>,
    skill_by_id: &'a HashMap<u32, &'a SkillSpec>,
    enable_debug: bool,
) -> (Option<Phase2Result<'a>>, Vec<Phase2EntryDebug>) {
    let mut debug = Vec::new();

    macro_rules! push_dbg {
        ($line:expr, $skill:expr, $castable:expr, $reason:expr) => {
            if enable_debug {
                debug.push(Phase2EntryDebug {
                    line: $line,
                    skill: $skill,
                    castable: $castable,
                    reason: $reason,
                });
            }
        };
    }

    for entry in pool {
        // 按名字查找，fallback 按数字 ID 查
        let ranks = match skill_map.get(entry.skill_name.as_str()) {
            Some(r) => r,
            None => {
                // 尝试数字 ID
                if let Some(id) = entry.skill_name.parse::<u32>().ok() {
                    if let Some(spec) = skill_by_id.get(&id) {
                        // 用基础名再查 skill_map 获取 ranks
                        let base = spec.name.split('·').next().unwrap_or(&spec.name);
                        if let Some(r) = skill_map.get(base) {
                            r
                        } else {
                            push_dbg!(
                                entry.line,
                                entry.skill_name.clone(),
                                false,
                                format!("ID {} 无法找到技能组", id)
                            );
                            continue;
                        }
                    } else {
                        push_dbg!(
                            entry.line,
                            entry.skill_name.clone(),
                            false,
                            format!("未知技能ID {}", id)
                        );
                        continue;
                    }
                } else {
                    push_dbg!(
                        entry.line,
                        entry.skill_name.clone(),
                        false,
                        "未知技能".into()
                    );
                    continue;
                }
            }
        };

        let skill = match player.pick_rank(ranks) {
            Some(s) => s,
            None => {
                push_dbg!(
                    entry.line,
                    entry.skill_name.clone(),
                    false,
                    "体态/条件不满足".into()
                );
                continue;
            }
        };

        // 战绝：只能释放苍雪刀招式
        if player.has_buff(crate::BUFF_ZHAN_JUE)
            && !matches!(
                skill.skill_id,
                13052 | 13053 | 13054 | 13055 | 90001 | 90002
            )
        {
            push_dbg!(
                entry.line,
                entry.skill_name.clone(),
                false,
                "战绝：仅可释放苍雪刀招式".into()
            );
            continue;
        }

        // combo_follow：若本技能会重定向到子技能，用子技能判断 CD 就绪
        let effective_skill =
            crate::resolve_combo_follow(skill, player, skill_by_id).unwrap_or(skill);

        // 检查技能是否现在就能释放（不等待）
        let ready_time = player.next_cast_time(effective_skill);
        if ready_time > player.current_time + 0.001 {
            if enable_debug {
                let wait = ready_time - player.current_time;
                debug.push(Phase2EntryDebug {
                    line: entry.line,
                    skill: entry.skill_name.clone(),
                    castable: false,
                    reason: format!("CD/GCD未就绪 ({:.2}s)", wait),
                });
            }
            continue;
        }

        // 引导检查
        if player.channel_end > player.current_time + 0.001 {
            if entry.is_fcast {
                // /fcast 可以打断引导，继续
            } else {
                push_dbg!(
                    entry.line,
                    entry.skill_name.clone(),
                    false,
                    "引导中 (非fcast)".into()
                );
                continue;
            }
        }

        push_dbg!(entry.line, entry.skill_name.clone(), true, "可释放".into());

        return (
            Some(Phase2Result {
                skill,
                line: entry.line,
                skill_name: entry.skill_name.clone(),
                is_fcast: entry.is_fcast,
            }),
            debug,
        );
    }
    (None, debug)
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面选择
// ─────────────────────────────────────────────────────────────────────────────

fn select_page(config: &MacroConfig, player: &Player) -> usize {
    for (i, page) in config.pages.iter().enumerate() {
        match page.stance_filter {
            None => return i,
            Some(stance) if stance == player.stance() => return i,
            _ => {}
        }
    }
    0
}

// ─────────────────────────────────────────────────────────────────────────────
// 辅助函数
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn skill_is_main(skill: &SkillSpec) -> bool {
    use crate::CdMode;
    skill.cooldowns.iter().any(|cd| {
        (cd.cd_id == "gcd_1.0" || cd.cd_id == "gcd_1.5") && cd.mode == CdMode::CheckAndTrigger
    })
}

pub(crate) fn skill_gcd(skill: &SkillSpec) -> f64 {
    use crate::CdMode;
    skill
        .cooldowns
        .iter()
        .filter(|cd| cd.cd_id.starts_with("gcd_") && cd.mode == CdMode::CheckAndTrigger)
        .map(|cd| cd.duration)
        .fold(0.0_f64, f64::max)
}

/// 单次"释放技能"上下文（不含调用侧状态）
pub struct CastCtx<'a> {
    pub skill_by_id: &'a HashMap<u32, &'a SkillSpec>,
    pub recipes_table: &'a [RecipeEntry],
    pub dmg_ctx: Option<&'a (crate::Attributes, crate::TargetConfig)>,
    pub network_delay: f64,
    /// true = 宏路径（CastEvent.is_macro = true），false = RL / 手动
    pub is_macro: bool,
}

/// 单次 cast 的产出：事件列表 + 是否成功释放
pub struct CastOutcome {
    pub events: Vec<CastEvent>,
    /// true = cast_skill 成功并构建了主事件；false = cast_skill 拒绝（仍可能有前置 tick 事件）
    pub cast_success: bool,
}

/// 释放一个技能并构建相关事件（主事件 + 脚本 emit + flush_advance tick）。
///
/// 行为与原 simulate_macro 内联块一致：
/// - `process_buff_ticks` 产出的 tick 事件与 `prev_time` 更新**无条件**发生。
/// - 仅当 `cast_skill` 成功时才推入主 CastEvent、运行脚本、flush_advance、更新 is_first_main/last_busy_end。
///
/// 调用方约定：
/// - `actual_skill` 必须已经过 `resolve_combo_follow` 处理。
/// - 调用前**不要**自行 `process_buff_ticks`。
/// - 调用方负责：`last_skill` 名称更新、`cast_count`/计数等上层状态。
pub fn execute_cast(
    player: &mut Player,
    actual_skill: &SkillSpec,
    prev_time: &mut f64,
    is_first_main: &mut bool,
    last_busy_end: &mut f64,
    ctx: &CastCtx,
) -> CastOutcome {
    let mut out: Vec<CastEvent> = Vec::new();

    // 1. 网络延迟预纳入 est_time（与普通序列路径一致）
    let delay = if skill_is_main(actual_skill) && !*is_first_main {
        ctx.network_delay
    } else {
        0.0
    };
    let est_time = player.next_cast_time(actual_skill) + delay;
    let mut tick_events = player.process_buff_ticks(*prev_time, est_time);
    crate::fill_tick_events(
        &mut tick_events,
        ctx.skill_by_id,
        ctx.dmg_ctx,
        ctx.recipes_table,
        player,
    );
    out.extend(tick_events);
    *prev_time = est_time;
    player.current_time = est_time;

    // 2. 释放
    let state_before = if player.lite_mode {
        None
    } else {
        Some(crate::snapshot_event_state(player))
    };
    let rage_before = player.rage;
    let rage_overflow_before = player.rage_overflow_total;
    player.rage_overflow_sources_current.clear();
    player.rage_transactions_current.clear();
    player.rage_generated_current = 0;
    player.rage_gained_current = 0;
    player.rage_spent_current = 0;
    let channel_override = if actual_skill.channel_interval.is_some() {
        Some(u32::MAX)
    } else {
        None
    };
    let cast_result = player.cast_skill(actual_skill, channel_override, None, 0.0, 0.0);
    if player.current_time > *prev_time {
        let mut gap_events = player.process_buff_ticks(*prev_time, player.current_time);
        if !gap_events.is_empty() {
            crate::fill_tick_events(
                &mut gap_events,
                ctx.skill_by_id,
                ctx.dmg_ctx,
                ctx.recipes_table,
                player,
            );
        }
        out.extend(gap_events);
        *prev_time = player.current_time;
    }
    let (cast_time, _cd_wait, ch_ticks, ch_max, ch_dur, applied_offset, ret_max_offset) =
        match cast_result {
            Some(r) => r,
            None => {
                return CastOutcome {
                    events: out,
                    cast_success: false,
                }
            }
        };

    if skill_is_main(actual_skill) {
        *is_first_main = false;
    }
    if !actual_skill.passive {
        player.start_swing(cast_time);
    }

    let idle_wait = if skill_is_main(actual_skill) {
        let raw = (cast_time - *last_busy_end).max(0.0);
        (raw - ctx.network_delay).max(0.0)
    } else {
        0.0
    };

    let cost = player.last_rage_cost;
    let runtime_recipes = crate::compute_runtime_recipes(actual_skill, player);

    // 4. 主动技能伤害（脚本前算）
    let (dmg, dmg_normal, dmg_crit, dmg_total, rt_snap) = if let Some((a, t)) = ctx.dmg_ctx {
        let ticks = ch_ticks.unwrap_or(1);
        let (r, dt, rt) = crate::calc_event_damage(
            actual_skill,
            a,
            t,
            player,
            &runtime_recipes,
            ctx.recipes_table,
            ticks,
        );
        (
            Some(r.expected_damage),
            Some(r.normal_damage),
            Some(r.crit_damage),
            Some(dt),
            Some(rt),
        )
    } else {
        (None, None, None, None, None)
    };

    // 5. 脚本（狂绝返还、盾舞怒气、emit 破招段等）
    let em = crate::scripts::run_scripts(player, actual_skill, cast_time);
    // 脚本可能直写 player.rage / inst.stacks 等绕过 helper —— 边界 bump 兜底
    player.bump_decision_gen();
    let extra = em.events;
    let primary_override = em.primary_override;

    // 6. 蔑视奇穴 39045：伤害招式命中后获得蔑视 buff
    if dmg_total.unwrap_or(0.0) > 0.0 && player.has_talent(39045) {
        player.add_buff(crate::BUFF_MIE_SHI);
    }

    // 7. 斩刀：抓流血 DoT 快照
    if actual_skill.skill_id == 13054 {
        if let Some((a, _)) = ctx.dmg_ctx {
            let snap = crate::capture_dot_snapshot(a, player, 13054, "斩刀", ctx.recipes_table);
            if let Some(inst) = player
                .target_buffs
                .iter_mut()
                .find(|b| b.buff_id == crate::BUFF_LIU_XUE)
            {
                inst.snapshot = Some(snap);
            }
        }
    }

    let rage_delta = player.rage - rage_before;
    let rage_overflow = player
        .rage_overflow_total
        .saturating_sub(rage_overflow_before);
    let rage_overflow_sources = player
        .rage_overflow_sources_current
        .iter()
        .map(|(source, amount)| crate::RageOverflowCause {
            source: source.clone(),
            amount: *amount,
        })
        .collect::<Vec<_>>();
    let rage_transactions = player.rage_transactions_current.clone();
    let rage_generated = player.rage_generated_current;
    let rage_gained = player.rage_gained_current;
    let rage_spent = player.rage_spent_current;
    let has_rage_effect = rage_delta != 0 || cost > 0;

    let (event_name, final_dmg, final_dmg_n, final_dmg_c, final_dmg_t, final_rt, eff_recipes) =
        if let Some((ref ov_name, ov_id)) = primary_override {
            if let (Some((a, t_cfg)), Some(ov_spec)) =
                (ctx.dmg_ctx, ctx.skill_by_id.get(&ov_id).copied())
            {
                let ticks = ch_ticks.unwrap_or(1);
                let ov_recipes = crate::compute_runtime_recipes(ov_spec, player);
                let (r, dt, rt2) = crate::calc_event_damage(
                    ov_spec,
                    a,
                    t_cfg,
                    player,
                    &ov_recipes,
                    ctx.recipes_table,
                    ticks,
                );
                (
                    ov_name.clone(),
                    Some(r.expected_damage),
                    Some(r.normal_damage),
                    Some(r.crit_damage),
                    Some(dt),
                    Some(rt2),
                    ov_recipes,
                )
            } else {
                (
                    ov_name.clone(),
                    dmg,
                    dmg_normal,
                    dmg_crit,
                    dmg_total,
                    rt_snap,
                    runtime_recipes.clone(),
                )
            }
        } else if actual_skill.skill_id == 13055 {
            let name = if cost == 0 {
                "绝刀·免耗".to_string()
            } else {
                format!("绝刀·{}怒", cost)
            };
            (
                name,
                dmg,
                dmg_normal,
                dmg_crit,
                dmg_total,
                rt_snap,
                runtime_recipes.clone(),
            )
        } else {
            (
                actual_skill.name.clone(),
                dmg,
                dmg_normal,
                dmg_crit,
                dmg_total,
                rt_snap,
                runtime_recipes.clone(),
            )
        };

   out.push(CastEvent {
        sequence_index: None,
       name: event_name,
        skill_id: actual_skill.skill_id,
        cast_time,
        triggered: false,
        gcd: skill_gcd(actual_skill),
        is_main: skill_is_main(actual_skill),
        cd_wait: idle_wait,
        channel_ticks: ch_ticks,
        max_channel_ticks: ch_max,
        channel_duration: ch_dur,
        timing_offset: applied_offset,
        max_timing_offset: ret_max_offset,
        available_buffs: None,
        is_macro: ctx.is_macro,
        macro_page: None,
        macro_line: None,
        rage_after: if has_rage_effect {
            Some(player.rage)
        } else {
            None
        },
        rage_delta: if has_rage_effect {
            Some(rage_delta)
        } else {
            None
        },
        rage_overflow: (rage_overflow > 0).then_some(rage_overflow),
        rage_overflow_sources,
        rage_transactions,
        rage_generated: (rage_generated > 0).then_some(rage_generated),
        rage_gained: (rage_gained > 0).then_some(rage_gained),
        rage_spent: (rage_spent > 0).then_some(rage_spent),
        rage_cost: if cost > 0 { Some(cost) } else { None },
        state_before,
        state_after: if player.lite_mode {
            None
        } else {
            Some(crate::snapshot_event_state(player))
        },
        damage: final_dmg,
        damage_normal: final_dmg_n,
        damage_crit: final_dmg_c,
        damage_total: final_dmg_t,
        runtime_recipes: if player.lite_mode {
            Vec::new()
        } else {
            eff_recipes.clone()
        },
        runtime_stats: if player.lite_mode {
            None
        } else {
            final_rt.clone()
        },
        override_attack_coeff: None,
        applied_recipes: if player.lite_mode {
            Vec::new()
        } else {
            // 主体 cast 事件填 applied_recipes（与 main.rs 主路径一致）
            let base_name = actual_skill
                .name
                .split('·')
                .next()
                .unwrap_or(&actual_skill.name);
            crate::collect_recipes_indexed(
                player,
                actual_skill.skill_id,
                base_name,
                &eff_recipes,
                ctx.recipes_table,
            )
            .iter()
            .map(|r| r.id)
            .collect()
        },
    });

    // 7. 脚本 emit 出的子事件补伤害
    for mut ev in extra {
        crate::fill_event_damage(
            &mut ev,
            ctx.skill_by_id,
            ctx.dmg_ctx,
            ctx.recipes_table,
            player,
            &eff_recipes,
        );
        out.push(ev);
    }

    // 8. flush_advance
    let mut advance_events = player.flush_advance();
    crate::fill_tick_events(
        &mut advance_events,
        ctx.skill_by_id,
        ctx.dmg_ctx,
        ctx.recipes_table,
        player,
    );
    out.extend(advance_events);

    *prev_time = player.current_time;

    if skill_is_main(actual_skill) {
        let gcd_end = player
            .active_cds
            .iter()
            .filter(|(k, _)| k.starts_with("gcd_"))
            .map(|(_, &v)| v)
            .fold(0.0_f64, f64::max);
        *last_busy_end = f64::max(gcd_end, player.channel_end);
    }

    CastOutcome {
        events: out,
        cast_success: true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 宏模拟主循环
// ─────────────────────────────────────────────────────────────────────────────

/// 宏模式模拟
/// max_casts: 最大成功释放次数
/// prev_time/is_first_main/last_skill_init: 接续手动序列的状态
/// dmg_ctx/skill_by_id: 伤害计算上下文（None 时不算伤害）
pub fn simulate_macro(
    config: &MacroConfig,
    player: &mut Player,
    skill_map: &HashMap<&str, Vec<&SkillSpec>>,
    max_casts: u32,
    max_duration: f64,
    network_delay: f64,
    prev_time: &mut f64,
    is_first_main: &mut bool,
    last_skill_init: Option<String>,
    dmg_ctx: Option<&(crate::Attributes, crate::TargetConfig)>,
    recipes_table: &[RecipeEntry],
    skill_by_id: &HashMap<u32, &SkillSpec>,
    pauses: &[(f64, f64)],
) -> (
    Vec<CastEvent>,
    Vec<MacroStepDebug>,
    Vec<MacroLineExecutionStats>,
) {
    let _t0 = std::time::Instant::now();
    let _guard = crate::scopeguard_perf(
        |ns| {
            crate::perf_add(|p| {
                p.macro_eval_n += 1;
                p.macro_eval_ns += ns;
            })
        },
        _t0,
    );
    // 停手窗口：把 (start, dur) 转成 (start, end)，按 start 排序
    let mut pause_ranges: Vec<(f64, f64)> = pauses
        .iter()
        .map(|&(s, d)| (s, s + d))
        .filter(|&(s, e)| e > s && s < max_duration)
        .collect();
    pause_ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // 如果当前时间落在某个停手窗口中或之前，直接跳到窗口结束
    let skip_pause = |player: &mut Player,
                      prev_time: &mut f64,
                      timeline: &mut Vec<CastEvent>,
                      skill_by_id: &HashMap<u32, &SkillSpec>,
                      dmg_ctx: Option<&(crate::Attributes, crate::TargetConfig)>,
                      recipes_table: &[RecipeEntry],
                      pauses: &[(f64, f64)]| {
        for &(ps, pe) in pauses {
            if player.current_time >= ps && player.current_time < pe {
                let mut tick = player.process_buff_ticks(*prev_time, pe);
                crate::fill_tick_events(&mut tick, skill_by_id, dmg_ctx, recipes_table, player);
                timeline.extend(tick);
                player.current_time = pe;
                *prev_time = pe;
            }
        }
    };
    let mut timeline = Vec::new();
    let debug_steps = Vec::new();
    let collect_execution_stats = !player.lite_mode;
    let mut line_stats = if collect_execution_stats {
        config
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page_index, page)| {
                page.lines
                    .iter()
                    .enumerate()
                    .map(move |(line_index, line)| MacroLineExecutionStats {
                        page: page_index + 1,
                        line: line_index + 1,
                        skill: line.action.skill_name().to_string(),
                        condition: line
                            .condition
                            .as_ref()
                            .map(MacroCondition::semantic_string)
                            .unwrap_or_else(|| "(无条件)".to_string()),
                        evaluations: 0,
                        condition_passes: 0,
                        condition_failures: 0,
                        selected_casts: 0,
                        priority_bypassed: 0,
                        castability_failures: BTreeMap::new(),
                    })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let page_offsets = config
        .pages
        .iter()
        .scan(0usize, |offset, page| {
            let current = *offset;
            *offset += page.lines.len();
            Some(current)
        })
        .collect::<Vec<_>>();
    let mut last_skill: Option<String> = last_skill_init;
    let mut cast_count = 0u32;
    // 记录上一个技能的"忙碌结束时间"（GCD 或引导，取较大值），用于计算停手等待
    let mut last_busy_end: f64 = player
        .active_cds
        .iter()
        .filter(|(k, _)| k.starts_with("gcd_"))
        .map(|(_, &v)| v)
        .fold(player.channel_end, f64::max);

    // 最小推进时间：1帧
    let min_advance = frames_to_sec(1);
    let safety_limit = max_casts.max(1) * 10000; // 防死循环

    // 预解析当前 macro 里所有 bufftime/tbufftime 阈值（buff_name → buff_id）
    // 用于 next_decision_time 计算"条件翻转时刻"
    let bufftime_str = config.bufftime_thresholds();
    let bufftime_thresholds: Vec<(u32, f64, bool)> = bufftime_str
        .iter()
        .filter_map(|(n, v, is_t)| buff_name_to_id(n).map(|id| (id, *v, *is_t)))
        .collect();

    for _ in 0..safety_limit {
        if cast_count >= max_casts {
            break;
        }
        if player.current_time > max_duration {
            break;
        }

        // 停手：当前时间落在暂停窗口内 → 跳到窗口结束（期间只跑 buff tick）
        skip_pause(
            player,
            prev_time,
            &mut timeline,
            skill_by_id,
            dmg_ctx,
            recipes_table,
            &pause_ranges,
        );

        // 选择活跃宏页（体态翻页）
        let page_idx = select_page(config, player);

        // 阶段一：构建技能池（debug 关闭：debug_steps 已注释，不消费）
        let _t_p1 = std::time::Instant::now();
        let (pool, new_last, p1_debug) = evaluate_phase1(
            &config.pages[page_idx],
            player,
            skill_map,
            skill_by_id,
            last_skill.clone(),
            collect_execution_stats,
        );
        let _ns_p1 = _t_p1.elapsed().as_nanos() as u64;
        crate::perf_add(|p| {
            p.macro_phase1_n += 1;
            p.macro_phase1_ns += _ns_p1;
        });
        if collect_execution_stats {
            let offset = page_offsets[page_idx];
            for observation in &p1_debug {
                if let Some(stats) = line_stats.get_mut(offset + observation.line - 1) {
                    stats.evaluations += 1;
                    stats.condition_passes += u32::from(observation.passed);
                    stats.condition_failures += u32::from(!observation.passed);
                }
            }
        }

        // 更新 last_skill（池中最后一个）
        if new_last.is_some() {
            last_skill = new_last;
        }

        if pool.is_empty() {
            // 无技能通过条件，跳到下一次状态可能变化的时刻
            let _t_adv = std::time::Instant::now();
            let _t_next = std::time::Instant::now();
            let next_event = player.next_decision_time(&bufftime_thresholds);
            let _ns_next = _t_next.elapsed().as_nanos() as u64;
            crate::perf_add(|p| {
                p.macro_adv_next_n += 1;
                p.macro_adv_next_ns += _ns_next;
            });
            let target = if next_event.is_finite() {
                next_event
            } else {
                player.current_time + min_advance
            };
            let target = target.min(max_duration + min_advance);
            let _t_ticks = std::time::Instant::now();
            let mut tick_events = player.process_buff_ticks(*prev_time, target);
            let _ns_ticks = _t_ticks.elapsed().as_nanos() as u64;
            crate::perf_add(|p| {
                p.macro_adv_ticks_n += 1;
                p.macro_adv_ticks_ns += _ns_ticks;
            });
            let _t_fill = std::time::Instant::now();
            crate::fill_tick_events(
                &mut tick_events,
                skill_by_id,
                dmg_ctx,
                recipes_table,
                player,
            );
            let _ns_fill = _t_fill.elapsed().as_nanos() as u64;
            crate::perf_add(|p| {
                p.macro_adv_fill_n += 1;
                p.macro_adv_fill_ns += _ns_fill;
            });
            timeline.extend(tick_events);
            *prev_time = target;
            player.current_time = target;
            let _ns_adv = _t_adv.elapsed().as_nanos() as u64;
            crate::perf_add(|p| {
                p.macro_advance_n += 1;
                p.macro_advance_ns += _ns_adv;
            });
            continue;
        }

        // 阶段二：选出第一个可释放的技能（debug 关闭，同上）
        let _t_p2 = std::time::Instant::now();
        let (phase2_result, p2_debug) = evaluate_phase2(
            &pool,
            player,
            skill_map,
            skill_by_id,
            collect_execution_stats,
        );
        let _ns_p2 = _t_p2.elapsed().as_nanos() as u64;
        crate::perf_add(|p| {
            p.macro_phase2_n += 1;
            p.macro_phase2_ns += _ns_p2;
        });
        if collect_execution_stats {
            let offset = page_offsets[page_idx];
            for observation in &p2_debug {
                if observation.castable {
                    continue;
                }
                if let Some(stats) = line_stats.get_mut(offset + observation.line - 1) {
                    let reason = if observation.reason.starts_with("CD/GCD未就绪") {
                        "cooldown_or_gcd_not_ready"
                    } else if observation.reason == "体态/条件不满足" {
                        "stance_or_rank_unavailable"
                    } else if observation.reason.starts_with("战绝") {
                        "zhan_jue_restriction"
                    } else if observation.reason.starts_with("引导中") {
                        "channel_not_interruptible"
                    } else if observation.reason.starts_with("未知技能") {
                        "unknown_skill"
                    } else {
                        "other_castability_failure"
                    };
                    *stats
                        .castability_failures
                        .entry(reason.to_string())
                        .or_default() += 1;
                }
            }
        }
        let result = match phase2_result {
            Some(r) => r,
            None => {
                // 池中所有技能 CD/GCD 未就绪，跳到下一次状态可能变化的时刻
                let _t_adv = std::time::Instant::now();
                let _t_next = std::time::Instant::now();
                let next_event = player.next_decision_time(&bufftime_thresholds);
                let _ns_next = _t_next.elapsed().as_nanos() as u64;
                crate::perf_add(|p| {
                    p.macro_adv_next_n += 1;
                    p.macro_adv_next_ns += _ns_next;
                });
                let target = if next_event.is_finite() {
                    next_event
                } else {
                    player.current_time + min_advance
                };
                let target = target.min(max_duration + min_advance);
                let _t_ticks = std::time::Instant::now();
                let mut tick_events = player.process_buff_ticks(*prev_time, target);
                let _ns_ticks = _t_ticks.elapsed().as_nanos() as u64;
                crate::perf_add(|p| {
                    p.macro_adv_ticks_n += 1;
                    p.macro_adv_ticks_ns += _ns_ticks;
                });
                let _t_fill = std::time::Instant::now();
                crate::fill_tick_events(
                    &mut tick_events,
                    skill_by_id,
                    dmg_ctx,
                    recipes_table,
                    player,
                );
                let _ns_fill = _t_fill.elapsed().as_nanos() as u64;
                crate::perf_add(|p| {
                    p.macro_adv_fill_n += 1;
                    p.macro_adv_fill_ns += _ns_fill;
                });
                timeline.extend(tick_events);
                *prev_time = target;
                player.current_time = target;
                let _ns_adv = _t_adv.elapsed().as_nanos() as u64;
                crate::perf_add(|p| {
                    p.macro_advance_n += 1;
                    p.macro_advance_ns += _ns_adv;
                });
                continue;
            }
        };

        // combo_follow：宏写阵云结晦，按连招状态自动重定向到二/三段
        let actual_skill =
            crate::resolve_combo_follow(result.skill, player, skill_by_id).unwrap_or(result.skill);

        // /fcast 打断引导
        if result.is_fcast && player.channel_end > player.current_time + 0.001 {
            let skill_id = player.channel_skill_id;
            let interrupt_at = player.next_cast_time(actual_skill);
            let (old_ticks, actual_ticks) = player.interrupt_channel(interrupt_at);
            // 修正 timeline 中引导技能的跳数和时长
            if let Some(ev) = timeline
                .iter_mut()
                .rev()
                .find(|e| e.skill_id == skill_id && !e.triggered)
            {
                ev.channel_ticks = Some(actual_ticks);
                let interval = frames_to_sec(player.channel_interval_frame);
                let first = frames_to_sec(player.channel_first_frame);
                ev.channel_duration = Some(if actual_ticks <= 1 {
                    first
                } else {
                    first + (actual_ticks - 1) as f64 * interval
                });
            }
            // 盾舞：修正怒气
            if skill_id == 13048 && player.stance() == crate::Stance::Shield {
                let over_rage = (old_ticks as i32 - actual_ticks as i32).max(0);
                player.add_rage(-over_rage);
            }
        }

        // 释放：使用 execute_cast 统一处理 tick/cast/脚本/emit/advance
        let cast_ctx = CastCtx {
            skill_by_id,
            recipes_table,
            dmg_ctx,
            network_delay,
            is_macro: true,
        };
        let mut outcome = execute_cast(
            player,
            actual_skill,
            prev_time,
            is_first_main,
            &mut last_busy_end,
            &cast_ctx,
        );
        if let Some(event) = outcome.events.iter_mut().find(|event| !event.triggered) {
            event.macro_page = Some(page_idx + 1);
            event.macro_line = Some(result.line);
        }
        timeline.extend(outcome.events);
        if outcome.cast_success {
            if collect_execution_stats {
                let offset = page_offsets[page_idx];
                if let Some(stats) = line_stats.get_mut(offset + result.line - 1) {
                    stats.selected_casts += 1;
                }
                for entry in pool.iter().filter(|entry| entry.line > result.line) {
                    if let Some(stats) = line_stats.get_mut(offset + entry.line - 1) {
                        stats.priority_bypassed += 1;
                    }
                }
            }
            // 更新 last_skill 为实际释放的技能（跨轮次持久化）
            last_skill = Some(
                actual_skill
                    .name
                    .split('·')
                    .next()
                    .unwrap_or(&actual_skill.name)
                    .to_string(),
            );
            cast_count += 1;
        }
    }

    // // 调试输出（暂时关闭）
    // for step in &debug_steps {
    //     let sel = step.selected.as_deref().unwrap_or("(空)");
    //     let ls = last_skill.as_deref().unwrap_or("(无)");
    //     println!("┌─ [宏] {:.3}s  页{}  → {}", step.time, step.page, sel);
    //     println!("│ 阶段一");
    //     for l in &step.phase1 {
    //         let mark = if l.passed { "✓" } else { "✗" };
    //         println!("│   {} L{}: [{}] {} ", mark, l.line, l.condition, l.skill);
    //     }
    //     if !step.phase2.is_empty() {
    //         println!("│ 阶段二");
    //         for e in &step.phase2 {
    //             let mark = if e.castable { "✓" } else { "✗" };
    //             println!("│   {} {} — {}", mark, e.skill, e.reason);
    //         }
    //     }
    //     println!("└─ last_skill={}", ls);
    // }

    (timeline, debug_steps, line_stats)
}
