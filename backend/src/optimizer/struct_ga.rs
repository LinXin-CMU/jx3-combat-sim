//! Phase 3 规则结构 GA 引擎
//!
//! 个体编码：
//! - `shield_order: Vec<(rule_id, enabled)>` — 有序，决定擎盾页宏文本
//! - `blade_order:  Vec<(rule_id, enabled)>` — 有序，决定擎刀页宏文本
//! - `values: Vec<f64>` — 全规则的阈值，索引对齐 `StructCtx.param_keys`
//!
//! 变异：冒泡 / 换位 / 翻转启用 / 跨页移动 / 阈值扰动
//! 交叉：OX 顺序交叉 + 阈值算术交叉

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rand::prelude::*;
use rand_distr::{Distribution, Normal};
use rayon::prelude::*;
use serde::Serialize;

use crate::macro_engine::{MacroAction, MacroConfig};
use crate::macro_eval::simulate_macro;
use crate::{Attributes, Player, RecipeEntry, SkillSpec, Stance, TargetConfig};

use super::ga::{EffectiveTunable, EvalStats, Event as GaEvent, GaParams, ProgressSink, TopNEntry};
use super::loop_config::{LoopConfig, LoopMacro};
use super::render::{render_individual, RenderResult};
use super::rule_pool::{PageTag, Rule, RulePool, RuleSource};

// ─────────────────────────────────────────────────────────────────────────────
// 上下文
// ─────────────────────────────────────────────────────────────────────────────

pub struct StructCtx {
    pub pool: RulePool,
    /// 阈值 flat 索引，每个元素 = (rule_id, visit_idx)
    pub param_keys: Vec<(String, usize)>,
    /// 每个 param 的有效范围（与 param_keys 等长）
    pub tunables: Vec<EffectiveTunable>,

    // 仿真输入
    pub attrs: Attributes,
    pub target: TargetConfig,
    pub talents: Vec<u32>,
    pub recipes: Vec<u32>,
    pub initial_rage: Option<i32>,
    pub haste_level: u32,
    pub network_delay_ms: u32,
    pub duration_min: f64,
    pub duration_max: f64,
    /// 每个个体评估 K 次（不同时长），fitness = mean - 0.5*std
    pub samples_per_eval: usize,
    pub skills: Arc<Vec<SkillSpec>>,
    pub recipes_table: Arc<Vec<RecipeEntry>>,
}

impl StructCtx {
    pub fn midpoint_duration(&self) -> f64 {
        ((self.duration_min + self.duration_max) / 2.0).max(1.0)
    }
}

impl StructCtx {
    /// 从规则池构造 param_keys + tunables flat 列表
    pub fn build_param_table(pool: &RulePool) -> (Vec<(String, usize)>, Vec<EffectiveTunable>) {
        let mut keys: Vec<(String, usize)> = Vec::new();
        let mut tunables: Vec<EffectiveTunable> = Vec::new();
        // 按规则 id 的稳定顺序遍历（先 shield_default_order，再 blade_default_order，再其余）
        let mut seen: HashSet<String> = HashSet::new();
        let push_rule = |rid: &str,
                         keys: &mut Vec<(String, usize)>,
                         tunables: &mut Vec<EffectiveTunable>,
                         seen: &mut HashSet<String>| {
            if seen.contains(rid) {
                return;
            }
            seen.insert(rid.to_string());
            let Some(rule) = pool.rules.get(rid) else {
                return;
            };
            for (vi, tp) in rule.tunables.iter().enumerate() {
                keys.push((rid.to_string(), vi));
                tunables.push(EffectiveTunable {
                    param: tp.clone(),
                    min: tp.suggested_min,
                    max: tp.suggested_max,
                    step: tp.suggested_step.max(1e-6),
                });
            }
        };
        for rid in &pool.shield_default_order {
            push_rule(rid, &mut keys, &mut tunables, &mut seen);
        }
        for rid in &pool.blade_default_order {
            push_rule(rid, &mut keys, &mut tunables, &mut seen);
        }
        // 其余（Either candidate 未放到两个默认顺序里的情况）
        let all_ids: Vec<String> = pool.rules.keys().cloned().collect();
        for rid in all_ids {
            push_rule(&rid, &mut keys, &mut tunables, &mut seen);
        }
        (keys, tunables)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 个体
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct StructIndividual {
    pub shield_order: Vec<(String, bool)>,
    pub blade_order: Vec<(String, bool)>,
    pub values: Vec<f64>,
    pub fitness: f64,
    pub stats: EvalStats,
}

impl StructIndividual {
    pub fn seed(ctx: &StructCtx) -> Self {
        let shield_order: Vec<(String, bool)> = ctx
            .pool
            .shield_default_order
            .iter()
            .map(|id| (id.clone(), ctx.pool.shield_default_enabled.contains(id)))
            .collect();
        let blade_order: Vec<(String, bool)> = ctx
            .pool
            .blade_default_order
            .iter()
            .map(|id| (id.clone(), ctx.pool.blade_default_enabled.contains(id)))
            .collect();
        let values: Vec<f64> = ctx
            .tunables
            .iter()
            .map(|t| t.clamp_snap(t.param.original))
            .collect();
        Self {
            shield_order,
            blade_order,
            values,
            fitness: 0.0,
            stats: EvalStats::default(),
        }
    }

    pub fn half_random(ctx: &StructCtx, rng: &mut impl Rng) -> Self {
        let mut ind = Self::seed(ctx);
        // 擎盾页：original 90% 启用 / candidate 20% 启用
        for (id, en) in ind.shield_order.iter_mut() {
            let Some(rule) = ctx.pool.rules.get(id) else {
                continue;
            };
            if rule.locked {
                *en = true;
                continue;
            }
            *en = match rule.source {
                RuleSource::Original => rng.gen_bool(0.9),
                RuleSource::Candidate => rng.gen_bool(0.2),
            };
        }
        for (id, en) in ind.blade_order.iter_mut() {
            let Some(rule) = ctx.pool.rules.get(id) else {
                continue;
            };
            if rule.locked {
                *en = true;
                continue;
            }
            *en = match rule.source {
                RuleSource::Original => rng.gen_bool(0.9),
                RuleSource::Candidate => rng.gen_bool(0.2),
            };
        }
        shuffle_respecting_locks(&mut ind.shield_order, &ctx.pool, rng);
        shuffle_respecting_locks(&mut ind.blade_order, &ctx.pool, rng);
        for (i, v) in ind.values.iter_mut().enumerate() {
            let t = &ctx.tunables[i];
            let sigma = t.range() * 0.15;
            let n = Normal::new(0.0, sigma.max(1e-6)).unwrap();
            *v = t.clamp_snap(*v + n.sample(rng));
        }
        ind
    }

    pub fn full_random(ctx: &StructCtx, rng: &mut impl Rng) -> Self {
        let mut shield_order: Vec<(String, bool)> = ctx
            .pool
            .shield_default_order
            .iter()
            .map(|id| (id.clone(), false))
            .collect();
        let mut blade_order: Vec<(String, bool)> = ctx
            .pool
            .blade_default_order
            .iter()
            .map(|id| (id.clone(), false))
            .collect();
        for (id, en) in shield_order.iter_mut() {
            let locked = ctx.pool.rules.get(id).map(|r| r.locked).unwrap_or(false);
            *en = if locked { true } else { rng.gen_bool(0.5) };
        }
        for (id, en) in blade_order.iter_mut() {
            let locked = ctx.pool.rules.get(id).map(|r| r.locked).unwrap_or(false);
            *en = if locked { true } else { rng.gen_bool(0.5) };
        }
        shuffle_respecting_locks(&mut shield_order, &ctx.pool, rng);
        shuffle_respecting_locks(&mut blade_order, &ctx.pool, rng);
        let values: Vec<f64> = ctx.tunables.iter().map(|t| t.random(rng)).collect();
        Self {
            shield_order,
            blade_order,
            values,
            fitness: 0.0,
            stats: EvalStats::default(),
        }
    }
}

/// 洗牌但保持 locked 规则位置不变
fn shuffle_respecting_locks(order: &mut Vec<(String, bool)>, pool: &RulePool, rng: &mut impl Rng) {
    // 提取可移动位置 + 可移动规则
    let mut movable_positions: Vec<usize> = Vec::new();
    let mut movable_rules: Vec<(String, bool)> = Vec::new();
    for (i, (id, en)) in order.iter().enumerate() {
        let locked = pool.rules.get(id).map(|r| r.locked).unwrap_or(false);
        if !locked {
            movable_positions.push(i);
            movable_rules.push((id.clone(), *en));
        }
    }
    movable_rules.shuffle(rng);
    for (pos, rule) in movable_positions.into_iter().zip(movable_rules.into_iter()) {
        order[pos] = rule;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 评估
// ─────────────────────────────────────────────────────────────────────────────

/// 单次评估（指定时长）
pub fn run_once(ind: &StructIndividual, ctx: &StructCtx, duration: f64) -> f64 {
    let rendered = render_individual(
        &ctx.pool,
        &ind.shield_order,
        &ind.blade_order,
        &ctx.param_keys,
        &ind.values,
    );
    let any_lines = rendered.config.pages.iter().any(|p| !p.lines.is_empty());
    if !any_lines {
        return 0.0;
    }

    let skills = &*ctx.skills;
    let mut skill_map: HashMap<&str, Vec<&SkillSpec>> = HashMap::new();
    for s in skills.iter() {
        if s.passive {
            continue;
        }
        let base = s.name.split('·').next().unwrap_or(&s.name);
        skill_map.entry(base).or_default().push(s);
    }
    let skill_by_id: HashMap<u32, &SkillSpec> = skills.iter().map(|s| (s.skill_id, s)).collect();
    let recipes_table = ctx.recipes_table.as_slice();

    let mut player = Player::new(ctx.haste_level, ctx.talents.clone(), ctx.recipes.clone());
    if let Some(r) = ctx.initial_rage {
        player.rage = r.clamp(0, 100);
    }
    let delay_sec = ctx.network_delay_ms as f64 / 1000.0;
    let dmg_ctx = Some((ctx.attrs.clone(), ctx.target.clone()));

    let dur = duration.max(1.0);
    let max_slots = (dur / 0.4).ceil() as u32 + 10;
    let mut prev_time = 0.0f64;
    let mut is_first_main = true;

    let (timeline, _debug, _line_stats) = simulate_macro(
        &rendered.config,
        &mut player,
        &skill_map,
        max_slots,
        dur,
        delay_sec,
        &mut prev_time,
        &mut is_first_main,
        None,
        dmg_ctx.as_ref(),
        recipes_table,
        &skill_by_id,
        &[],
    );
    let total: f64 = timeline.iter().filter_map(|e| e.damage_total).sum();
    let last_cast = timeline
        .iter()
        .rev()
        .find(|e| !e.triggered)
        .map(|e| e.cast_time)
        .unwrap_or(0.0);
    let fight_time = player.fight_end(last_cast).min(dur).max(0.001);
    total / fight_time
}

/// K 次随机采样 → fitness = mean - 0.5*std
pub fn evaluate(ind: &StructIndividual, ctx: &StructCtx) -> EvalStats {
    let k = ctx.samples_per_eval.max(1);
    let lo = ctx.duration_min.max(1.0);
    let hi = ctx.duration_max.max(lo);
    let mut rng = rand::thread_rng();
    let mut samples: Vec<f64> = Vec::with_capacity(k);
    for _ in 0..k {
        let dur = if (hi - lo).abs() < 1e-6 {
            lo
        } else {
            rng.gen_range(lo..=hi)
        };
        samples.push(run_once(ind, ctx, dur));
    }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std = var.sqrt();
    let min_dps = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_dps = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    // fitness = 0.5×均值 + 0.5×最差，与 ga.rs 对齐
    EvalStats {
        fitness: 0.5 * mean + 0.5 * min_dps,
        mean,
        std,
        min_dps,
        max_dps,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 遗传操作
// ─────────────────────────────────────────────────────────────────────────────

fn tournament<'a>(
    pop: &'a [StructIndividual],
    k: usize,
    rng: &mut impl Rng,
) -> &'a StructIndividual {
    tournament_by(pop, k, rng, &|ind: &StructIndividual| ind.fitness)
}

fn tournament_by<'a>(
    pop: &'a [StructIndividual],
    k: usize,
    rng: &mut impl Rng,
    key: &dyn Fn(&StructIndividual) -> f64,
) -> &'a StructIndividual {
    let mut best: Option<&'a StructIndividual> = None;
    for _ in 0..k.max(1) {
        let i = rng.gen_range(0..pop.len());
        let cand = &pop[i];
        if best.map_or(true, |b| key(cand) > key(b)) {
            best = Some(cand);
        }
    }
    best.unwrap()
}

/// OX（顺序交叉）：分别对 shield_order 和 blade_order 做 OX，
/// 启用标志 70% 继承 parent1，30% 继承 parent2；values 算术交叉。
fn crossover(
    p1: &StructIndividual,
    p2: &StructIndividual,
    ctx: &StructCtx,
    rate: f64,
    rng: &mut impl Rng,
) -> StructIndividual {
    if !rng.gen_bool(rate) {
        return p1.clone();
    }

    let shield_order = ox_cross(&p1.shield_order, &p2.shield_order, &ctx.pool, rng);
    let blade_order = ox_cross(&p1.blade_order, &p2.blade_order, &ctx.pool, rng);

    let alpha = rng.gen_range(0.3..=0.7);
    let values: Vec<f64> = p1
        .values
        .iter()
        .zip(p2.values.iter())
        .enumerate()
        .map(|(i, (&x, &y))| ctx.tunables[i].clamp_snap(alpha * x + (1.0 - alpha) * y))
        .collect();

    StructIndividual {
        shield_order,
        blade_order,
        values,
        fitness: 0.0,
        stats: EvalStats::default(),
    }
}

/// OX：取 p1 的 [start, end) 作为固定段（位置不变），剩余位置按 p2 的顺序填充
/// locked 规则保持 p1 位置不变
fn ox_cross(
    p1: &[(String, bool)],
    p2: &[(String, bool)],
    pool: &RulePool,
    rng: &mut impl Rng,
) -> Vec<(String, bool)> {
    let n = p1.len();
    if n == 0 {
        return Vec::new();
    }
    if n < 3 {
        // 太短不 OX，直接继承 p1
        return p1.to_vec();
    }
    let start = rng.gen_range(0..n);
    let end = rng.gen_range(start + 1..=n);

    let mut child: Vec<Option<(String, bool)>> = vec![None; n];
    let mut used: HashSet<String> = HashSet::new();

    // Step 1: locked 规则的位置用 p1 的锁定，其 id 记入 used
    for (i, (id, en)) in p1.iter().enumerate() {
        if pool.rules.get(id).map(|r| r.locked).unwrap_or(false) {
            child[i] = Some((id.clone(), *en));
            used.insert(id.clone());
        }
    }

    // Step 2: 复制 p1 的 [start, end) 到 child（跳过已被 locked 占用的位置）
    for i in start..end {
        if child[i].is_some() {
            continue;
        }
        let (id, en) = &p1[i];
        if !used.contains(id) {
            child[i] = Some((id.clone(), *en));
            used.insert(id.clone());
        }
    }

    // Step 3: 按 p2 顺序遍历，把尚未出现的 id 填入 child 空位
    let fill_list: Vec<(String, bool)> = p2
        .iter()
        .filter(|(id, _)| !used.contains(id))
        .cloned()
        .collect();
    let mut fill_iter = fill_list.into_iter();
    for slot in child.iter_mut() {
        if slot.is_none() {
            if let Some((id, _en_p2)) = fill_iter.next() {
                // 启用标志：70% 继承 p1（从 p1 中查找同 id），30% 继承 p2
                let en = if rng.gen_bool(0.7) {
                    p1.iter()
                        .find(|(i, _)| i == &id)
                        .map(|(_, e)| *e)
                        .unwrap_or(false)
                } else {
                    p2.iter()
                        .find(|(i, _)| i == &id)
                        .map(|(_, e)| *e)
                        .unwrap_or(false)
                };
                used.insert(id.clone());
                *slot = Some((id, en));
            }
        }
    }

    // Step 4: 未填满（p1/p2 不一致的边界情况）用 p1 剩余补齐
    for (id, en) in p1.iter() {
        if used.contains(id) {
            continue;
        }
        if let Some(pos) = child.iter().position(|s| s.is_none()) {
            child[pos] = Some((id.clone(), *en));
            used.insert(id.clone());
        }
    }

    child
        .into_iter()
        .map(|o| o.unwrap_or_else(|| (String::new(), false)))
        .collect()
}

fn mutate(ind: &mut StructIndividual, ctx: &StructCtx, rng: &mut impl Rng) {
    // 冒泡（20%）
    if rng.gen_bool(0.20) {
        bubble_swap(&mut ind.shield_order, &ctx.pool, rng);
    }
    if rng.gen_bool(0.20) {
        bubble_swap(&mut ind.blade_order, &ctx.pool, rng);
    }
    // 换位（25%）
    if rng.gen_bool(0.25) {
        random_swap(&mut ind.shield_order, &ctx.pool, rng);
    }
    if rng.gen_bool(0.25) {
        random_swap(&mut ind.blade_order, &ctx.pool, rng);
    }
    // 翻转启用（15%）
    if rng.gen_bool(0.15) {
        toggle_enabled(&mut ind.shield_order, &ctx.pool, rng);
    }
    if rng.gen_bool(0.15) {
        toggle_enabled(&mut ind.blade_order, &ctx.pool, rng);
    }
    // 跨页移动（5%）—— 仅对 page=Either 规则
    if rng.gen_bool(0.05) {
        cross_page_move(ind, ctx, rng);
    }
    // 阈值扰动（30%）
    for (i, v) in ind.values.iter_mut().enumerate() {
        if rng.gen_bool(0.30) {
            let t = &ctx.tunables[i];
            let sigma = t.range() * 0.12;
            let n = Normal::new(0.0, sigma.max(1e-6)).unwrap();
            *v = t.clamp_snap(*v + n.sample(rng));
        }
    }
}

fn bubble_swap(order: &mut Vec<(String, bool)>, pool: &RulePool, rng: &mut impl Rng) {
    let n = order.len();
    if n < 2 {
        return;
    }
    for _ in 0..n {
        let i = rng.gen_range(0..n - 1);
        let j = i + 1;
        if is_locked(&order[i].0, pool) || is_locked(&order[j].0, pool) {
            continue;
        }
        order.swap(i, j);
        return;
    }
}

fn random_swap(order: &mut Vec<(String, bool)>, pool: &RulePool, rng: &mut impl Rng) {
    let n = order.len();
    if n < 2 {
        return;
    }
    for _ in 0..n * 2 {
        let i = rng.gen_range(0..n);
        let j = rng.gen_range(0..n);
        if i == j {
            continue;
        }
        if is_locked(&order[i].0, pool) || is_locked(&order[j].0, pool) {
            continue;
        }
        order.swap(i, j);
        return;
    }
}

fn toggle_enabled(order: &mut Vec<(String, bool)>, pool: &RulePool, rng: &mut impl Rng) {
    let n = order.len();
    if n == 0 {
        return;
    }
    for _ in 0..n {
        let i = rng.gen_range(0..n);
        if is_locked(&order[i].0, pool) {
            continue;
        }
        order[i].1 = !order[i].1;
        return;
    }
}

fn cross_page_move(ind: &mut StructIndividual, ctx: &StructCtx, rng: &mut impl Rng) {
    // 找所有 Either 规则
    let either_ids: Vec<String> = ctx
        .pool
        .rules
        .iter()
        .filter(|(_, r)| r.page == PageTag::Either && !r.locked)
        .map(|(k, _)| k.clone())
        .collect();
    if either_ids.is_empty() {
        return;
    }
    let pick = either_ids[rng.gen_range(0..either_ids.len())].clone();
    // 当前在哪页？
    let in_shield = ind.shield_order.iter().any(|(id, _)| id == &pick);
    let in_blade = ind.blade_order.iter().any(|(id, _)| id == &pick);
    if in_shield && !in_blade {
        if let Some(pos) = ind.shield_order.iter().position(|(id, _)| id == &pick) {
            let (id, en) = ind.shield_order.remove(pos);
            ind.blade_order.push((id, en));
        }
    } else if in_blade && !in_shield {
        if let Some(pos) = ind.blade_order.iter().position(|(id, _)| id == &pick) {
            let (id, en) = ind.blade_order.remove(pos);
            ind.shield_order.push((id, en));
        }
    }
}

fn is_locked(rid: &str, pool: &RulePool) -> bool {
    pool.rules.get(rid).map(|r| r.locked).unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────────────
// 主循环
// ─────────────────────────────────────────────────────────────────────────────

pub struct StructGaRun<'a> {
    pub ctx: &'a StructCtx,
    pub params: &'a GaParams,
    pub base_loop: &'a LoopConfig,
    pub archive: &'a mut super::archive::ArchiveWriter,
    pub sink: &'a ProgressSink,
    pub stop: Arc<AtomicBool>,
}

impl<'a> StructGaRun<'a> {
    pub fn run(&mut self) -> (Option<StructIndividual>, f64, usize) {
        let pop_size = self.params.pop_size.max(8);
        let mut rng = rand::thread_rng();

        // 初始化：5% seed + 95% 半随机/全随机（按 15/50/30 比例）
        let mut population: Vec<StructIndividual> = Vec::with_capacity(pop_size);
        population.push(StructIndividual::seed(self.ctx));
        // 15% Phase 2 继承 ≈ 阈值扰动 seed
        let p2_count = ((pop_size as f64) * 0.15).ceil() as usize;
        for _ in 0..p2_count {
            let mut ind = StructIndividual::seed(self.ctx);
            for (i, v) in ind.values.iter_mut().enumerate() {
                let t = &self.ctx.tunables[i];
                let sigma = t.range() * 0.10;
                let n = Normal::new(0.0, sigma.max(1e-6)).unwrap();
                *v = t.clamp_snap(*v + n.sample(&mut rng));
            }
            population.push(ind);
        }
        let half_count = ((pop_size as f64) * 0.50).ceil() as usize;
        for _ in 0..half_count {
            population.push(StructIndividual::half_random(self.ctx, &mut rng));
        }
        while population.len() < pop_size {
            population.push(StructIndividual::full_random(self.ctx, &mut rng));
        }

        evaluate_population(&mut population, self.ctx);

        // 结构搜索始终按 DPS fitness 选优
        let sort_key = |ind: &StructIndividual| -> f64 { ind.fitness };

        let baseline_dps = population.first().map(|i| i.fitness).unwrap_or(0.0);
        (self.sink)(GaEvent::Baseline { dps: baseline_dps });

        let mut best_score = population
            .iter()
            .map(|i| sort_key(i))
            .fold(f64::NEG_INFINITY, f64::max);
        let mut best_ind = population
            .iter()
            .max_by(|a, b| {
                sort_key(a)
                    .partial_cmp(&sort_key(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();

        if let Some(ref bi) = best_ind {
            let rendered = render_individual(
                &self.ctx.pool,
                &bi.shield_order,
                &bi.blade_order,
                &self.ctx.param_keys,
                &bi.values,
            );
            let diff = compute_struct_diff(bi, &self.ctx.pool, &rendered);
            let canonical = run_once(bi, self.ctx, self.ctx.midpoint_duration());
            let cfg = build_loop_config_struct(bi, self.ctx, self.base_loop, &rendered);
            let macro_text = build_macro_text_struct(&rendered);
            if let Ok(rel) = self.archive.write_milestone(0, canonical, &cfg) {
                (self.sink)(GaEvent::Milestone {
                    gen: 0,
                    dps: canonical,
                    file: rel,
                    values: bi.values.clone(),
                    macro_text,
                    stats: Some(bi.stats.clone()),
                    struct_diff: Some(diff),
                });
            }
        }

        let mut gens_completed = 0usize;
        for gen in 0..self.params.generations {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            gens_completed = gen + 1;

            let stats = compute_stats(&population);
            let best_values = population
                .iter()
                .max_by(|a, b| {
                    sort_key(a)
                        .partial_cmp(&sort_key(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|i| i.values.clone())
                .unwrap_or_default();
            self.archive
                .append_progress(&serde_json::json!({
                    "gen": gen, "best": stats.best, "avg": stats.avg, "worst": stats.worst,
                    "div": stats.diversity, "values": best_values,
                }))
                .ok();
            let cv_best = population.iter().min_by(|a, b| {
                let ca = if a.stats.mean.abs() > 1e-9 {
                    a.stats.std / a.stats.mean
                } else {
                    f64::INFINITY
                };
                let cb = if b.stats.mean.abs() > 1e-9 {
                    b.stats.std / b.stats.mean
                } else {
                    f64::INFINITY
                };
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            });
            let (best_cv, best_mean_dps, best_min_dps) = match cv_best {
                Some(i) => {
                    let cv = if i.stats.mean.abs() > 1e-9 {
                        i.stats.std / i.stats.mean
                    } else {
                        0.0
                    };
                    (Some(cv), Some(i.stats.mean), Some(i.stats.min_dps))
                }
                None => (None, None, None),
            };
            (self.sink)(GaEvent::Progress {
                gen,
                best: stats.best,
                avg: stats.avg,
                worst: stats.worst,
                diversity: stats.diversity,
                best_values,
                best_cv,
                best_mean_dps,
                best_min_dps,
            });

            // 下一代
            let elite_count = ((pop_size as f64) * self.params.elitism_pct).ceil() as usize;
            let mut sorted = population.clone();
            sorted.sort_by(|a, b| {
                sort_key(b)
                    .partial_cmp(&sort_key(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut new_pop: Vec<StructIndividual> = sorted.into_iter().take(elite_count).collect();

            while new_pop.len() < pop_size {
                let a = tournament_by(&population, self.params.tournament_k, &mut rng, &sort_key)
                    .clone();
                let b = tournament_by(&population, self.params.tournament_k, &mut rng, &sort_key)
                    .clone();
                let mut child = crossover(&a, &b, self.ctx, self.params.crossover_rate, &mut rng);
                mutate(&mut child, self.ctx, &mut rng);
                new_pop.push(child);
            }

            evaluate_population(&mut new_pop[elite_count..], self.ctx);
            population = new_pop;

            let gen_best = population
                .iter()
                .max_by(|a, b| {
                    sort_key(a)
                        .partial_cmp(&sort_key(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned();
            if let Some(gb) = gen_best {
                let gb_score = sort_key(&gb);
                if gb_score > best_score + 1e-6 {
                    best_score = gb_score;
                    let rendered = render_individual(
                        &self.ctx.pool,
                        &gb.shield_order,
                        &gb.blade_order,
                        &self.ctx.param_keys,
                        &gb.values,
                    );
                    let diff = compute_struct_diff(&gb, &self.ctx.pool, &rendered);
                    let canonical = run_once(&gb, self.ctx, self.ctx.midpoint_duration());
                    let cfg = build_loop_config_struct(&gb, self.ctx, self.base_loop, &rendered);
                    let macro_text = build_macro_text_struct(&rendered);
                    if let Ok(rel) = self.archive.write_milestone(gen + 1, canonical, &cfg) {
                        (self.sink)(GaEvent::Milestone {
                            gen: gen + 1,
                            dps: canonical,
                            file: rel,
                            values: gb.values.clone(),
                            macro_text,
                            stats: Some(gb.stats.clone()),
                            struct_diff: Some(diff),
                        });
                    }
                    best_ind = Some(gb);
                }
            }
        }

        // Top N 按遗传方向排序
        population.sort_by(|a, b| {
            sort_key(b)
                .partial_cmp(&sort_key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let effective_top_n = self.params.top_n.min((gens_completed + 1).max(1));
        let mut top: Vec<StructIndividual> = Vec::new();
        for ind in population.iter() {
            if top.len() >= effective_top_n {
                break;
            }
            if !top.iter().any(|t| struct_approx_eq(t, ind)) {
                top.push(ind.clone());
            }
        }
        let mut entries = Vec::new();
        for (i, ind) in top.iter().enumerate() {
            let rendered = render_individual(
                &self.ctx.pool,
                &ind.shield_order,
                &ind.blade_order,
                &self.ctx.param_keys,
                &ind.values,
            );
            let diff = compute_struct_diff(ind, &self.ctx.pool, &rendered);
            let canonical = run_once(ind, self.ctx, self.ctx.midpoint_duration());
            let cfg = build_loop_config_struct(ind, self.ctx, self.base_loop, &rendered);
            let macro_text = build_macro_text_struct(&rendered);
            if let Ok(rel) = self.archive.write_topn(i + 1, canonical, &cfg) {
                entries.push(TopNEntry {
                    rank: i + 1,
                    dps: canonical,
                    file: rel,
                    values: ind.values.clone(),
                    macro_text,
                    stats: Some(ind.stats.clone()),
                    struct_diff: Some(diff),
                });
            }
        }
        (self.sink)(GaEvent::TopN { items: entries });

        let final_dps = best_ind.as_ref().map(|i| i.fitness).unwrap_or(0.0);
        (best_ind, final_dps, gens_completed)
    }
}

fn cmp_fit(a: &&StructIndividual, b: &&StructIndividual) -> std::cmp::Ordering {
    a.fitness
        .partial_cmp(&b.fitness)
        .unwrap_or(std::cmp::Ordering::Equal)
}
fn cmp_fit_desc(a: &StructIndividual, b: &StructIndividual) -> std::cmp::Ordering {
    b.fitness
        .partial_cmp(&a.fitness)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn evaluate_population(pop: &mut [StructIndividual], ctx: &StructCtx) {
    pop.par_iter_mut().for_each(|ind| {
        let s = evaluate(ind, ctx);
        ind.fitness = s.fitness;
        ind.stats = s;
    });
}

pub struct Stats {
    pub best: f64,
    pub avg: f64,
    pub worst: f64,
    pub diversity: f64,
}

fn compute_stats(pop: &[StructIndividual]) -> Stats {
    let n = pop.len().max(1);
    let fits: Vec<f64> = pop.iter().map(|i| i.fitness).collect();
    let best = fits.iter().cloned().fold(f64::MIN, f64::max);
    let worst = fits.iter().cloned().fold(f64::MAX, f64::min);
    let avg = fits.iter().sum::<f64>() / n as f64;
    // 结构多样性：对每对个体，以启用集合不同的 rule 数除以规则总数
    let mut total = 0.0;
    let mut pairs = 0usize;
    for i in 0..pop.len() {
        for j in i + 1..pop.len().min(i + 10) {
            let d = enabled_set_distance(&pop[i], &pop[j]);
            total += d;
            pairs += 1;
        }
    }
    let diversity = if pairs == 0 {
        0.0
    } else {
        total / pairs as f64
    };
    Stats {
        best,
        avg,
        worst,
        diversity,
    }
}

fn enabled_set_distance(a: &StructIndividual, b: &StructIndividual) -> f64 {
    let ea: HashSet<&str> = a
        .shield_order
        .iter()
        .chain(a.blade_order.iter())
        .filter(|(_, en)| *en)
        .map(|(id, _)| id.as_str())
        .collect();
    let eb: HashSet<&str> = b
        .shield_order
        .iter()
        .chain(b.blade_order.iter())
        .filter(|(_, en)| *en)
        .map(|(id, _)| id.as_str())
        .collect();
    let sym: usize = ea.symmetric_difference(&eb).count();
    let total = ea.union(&eb).count().max(1);
    sym as f64 / total as f64
}

fn struct_approx_eq(a: &StructIndividual, b: &StructIndividual) -> bool {
    if a.shield_order != b.shield_order {
        return false;
    }
    if a.blade_order != b.blade_order {
        return false;
    }
    a.values
        .iter()
        .zip(b.values.iter())
        .all(|(x, y)| (x - y).abs() < 1e-3)
}

// ─────────────────────────────────────────────────────────────────────────────
// 个体 → LoopConfig / 宏文本
// ─────────────────────────────────────────────────────────────────────────────

fn build_loop_config_struct(
    ind: &StructIndividual,
    ctx: &StructCtx,
    base: &LoopConfig,
    rendered: &RenderResult,
) -> LoopConfig {
    let macro_obj = render_to_loop_macro(&rendered.config);
    let _ = ind;
    LoopConfig {
        version: base.version,
        exported_at: Some(time_stamp()),
        target: base.target.clone(),
        talents: base.talents.clone(),
        recipes: base.recipes.clone(),
        sequence: base.sequence.clone(),
        macro_pages: macro_obj,
        network_delay: base.network_delay,
        initial_rage: base.initial_rage,
        macro_duration: Some((ctx.duration_min + ctx.duration_max) / 2.0),
    }
}

fn render_to_loop_macro(cfg: &MacroConfig) -> LoopMacro {
    let mut shield = String::new();
    let mut blade = String::new();
    for p in &cfg.pages {
        let text = serialize_lines(&p.lines);
        match p.stance_filter {
            Some(Stance::Shield) => shield = text,
            Some(Stance::Blade) => blade = text,
            _ => {}
        }
    }
    LoopMacro {
        mode: "stance".into(),
        general: String::new(),
        shield,
        blade,
    }
}

fn serialize_lines(lines: &[crate::macro_engine::MacroLine]) -> String {
    lines
        .iter()
        .map(|l| {
            let prefix = if l.action.is_fcast() {
                "/fcast"
            } else {
                "/cast"
            };
            let cond = l
                .condition
                .as_ref()
                .map(|c| format!(" [{}]", c.display_string()))
                .unwrap_or_default();
            let skill = match &l.action {
                MacroAction::Cast(n) | MacroAction::FCast(n) => n.as_str(),
            };
            format!("{}{} {}", prefix, cond, skill)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_macro_text_struct(rendered: &RenderResult) -> String {
    let mut out = String::new();
    let shield_text = rendered
        .config
        .pages
        .iter()
        .find(|p| p.stance_filter == Some(Stance::Shield))
        .map(|p| serialize_lines(&p.lines))
        .unwrap_or_default();
    let blade_text = rendered
        .config
        .pages
        .iter()
        .find(|p| p.stance_filter == Some(Stance::Blade))
        .map(|p| serialize_lines(&p.lines))
        .unwrap_or_default();
    if !shield_text.is_empty() {
        out.push_str("#page shield\n");
        out.push_str(&shield_text);
        out.push('\n');
    }
    if !blade_text.is_empty() {
        out.push_str("#page blade\n");
        out.push_str(&blade_text);
    }
    format_compact(&out)
}

fn format_compact(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' || c == ']' {
            i += 1;
            continue;
        }
        if c == '.'
            && i + 1 < chars.len()
            && chars[i + 1] == '0'
            && (i + 2 >= chars.len() || !chars[i + 2].is_ascii_digit())
            && i > 0
            && chars[i - 1].is_ascii_digit()
        {
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn time_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("t{}", secs)
}

// ─────────────────────────────────────────────────────────────────────────────
// 结构差异（用于前端卡片）
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct StructDiff {
    pub shield_emitted: Vec<String>,
    pub blade_emitted: Vec<String>,
    pub shield_truncated: Vec<String>,
    pub blade_truncated: Vec<String>,
    /// 启用的 candidate 规则 id（用于识别"新发现"）
    pub new_rules_enabled: Vec<String>,
    /// 相对 seed 被禁用的 original 规则 id
    pub original_disabled: Vec<String>,
}

pub fn compute_struct_diff(
    ind: &StructIndividual,
    pool: &RulePool,
    rendered: &RenderResult,
) -> StructDiff {
    let mut new_rules_enabled = Vec::new();
    let mut original_disabled = Vec::new();
    let emitted_set: HashSet<&str> = rendered
        .shield_emitted
        .iter()
        .chain(rendered.blade_emitted.iter())
        .map(|s| s.as_str())
        .collect();
    // 扫描 individual 的 order
    for (id, en) in ind.shield_order.iter().chain(ind.blade_order.iter()) {
        let Some(rule) = pool.rules.get(id) else {
            continue;
        };
        if *en && emitted_set.contains(id.as_str()) {
            if rule.source == RuleSource::Candidate {
                new_rules_enabled.push(id.clone());
            }
        }
        if !*en && rule.source == RuleSource::Original && !rule.locked {
            original_disabled.push(id.clone());
        }
    }
    StructDiff {
        shield_emitted: rendered.shield_emitted.clone(),
        blade_emitted: rendered.blade_emitted.clone(),
        shield_truncated: rendered.shield_truncated.clone(),
        blade_truncated: rendered.blade_truncated.clone(),
        new_rules_enabled,
        original_disabled,
    }
}

// 对外：让 runtime 可以 eager-compute struct_diff for milestones
pub fn render_and_diff(ind: &StructIndividual, ctx: &StructCtx) -> (RenderResult, StructDiff) {
    let r = render_individual(
        &ctx.pool,
        &ind.shield_order,
        &ind.blade_order,
        &ctx.param_keys,
        &ind.values,
    );
    let d = compute_struct_diff(ind, &ctx.pool, &r);
    (r, d)
}

// 避免 unused warning
#[allow(dead_code)]
fn _touch_rule(_r: &Rule) {}
