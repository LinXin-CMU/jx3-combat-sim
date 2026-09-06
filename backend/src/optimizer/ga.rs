//! Phase 2 阈值 GA 引擎 + 适应度评估
//!
//! 个体编码 = `Vec<f64>`，每个 f64 对应一个 enabled TunableParam 的值。
//! 评估 = 克隆 baseline MacroConfig → 写入阈值 → simulate_macro → DPS。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rand::prelude::*;
use rand_distr::{Distribution, Normal};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::macro_engine::{MacroAction, MacroConfig, MacroLine};
use crate::macro_eval::simulate_macro;
use crate::{Attributes, Player, RecipeEntry, SkillSpec, Stance, TargetConfig};

use super::analyze::{apply_values, TunableParam};
use super::loop_config::{LoopConfig, LoopMacro};

#[derive(Debug, Clone)]
pub struct GaParams {
    pub pop_size: usize,
    pub generations: usize,
    pub elitism_pct: f64,
    pub tournament_k: usize,
    pub crossover_rate: f64,
    pub mutation_rate: f64,
    pub mutation_sigma_pct: f64,
    pub top_n: usize,
    /// 遗传方向：true = DPS 最大化（fitness），false = CV 最小化（稳定性）
    pub fitness_dps_mode: bool,
}

impl Default for GaParams {
    fn default() -> Self {
        Self {
            pop_size: 50,
            generations: 500,
            elitism_pct: 0.10,
            tournament_k: 5,
            crossover_rate: 0.80,
            mutation_rate: 0.25,
            mutation_sigma_pct: 0.15,
            top_n: 10,
            fitness_dps_mode: true,
        }
    }
}

/// 可调参数的有效范围 + 步长（用户可能在 UI 里调过，不同于 suggested_*）
#[derive(Debug, Clone)]
pub struct EffectiveTunable {
    pub param: TunableParam,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl EffectiveTunable {
    pub fn random(&self, rng: &mut impl Rng) -> f64 {
        if self.max <= self.min {
            return self.min;
        }
        let raw = rng.gen_range(self.min..=self.max);
        snap(raw, self.min, self.max, self.step)
    }
    pub fn clamp_snap(&self, v: f64) -> f64 {
        let c = v.clamp(self.min, self.max);
        snap(c, self.min, self.max, self.step)
    }
    pub fn range(&self) -> f64 {
        (self.max - self.min).max(self.step)
    }
}

fn snap(v: f64, min: f64, max: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return v.clamp(min, max);
    }
    let scale = (1.0 / step).round().max(1.0);
    ((v * scale).round() / scale).clamp(min, max)
}

/// 评估场景：显式指定 delay/rage/duration/pauses，替代 K 次随机时长采样
#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioSpec {
    #[serde(default)]
    pub delay_ms: u32,
    #[serde(default)]
    pub initial_rage: Option<i32>,
    pub duration: f64,
    #[serde(default)]
    pub pauses: Vec<(f64, f64)>,
}

/// 适应度上下文（所有评估共享；Send+Sync 才能跨 rayon 线程）
pub struct FitnessCtx {
    pub baseline_macro: MacroConfig,
    pub tunables: Vec<EffectiveTunable>,
    pub attrs: Attributes,
    pub target: TargetConfig,
    pub talents: Vec<u32>,
    pub recipes: Vec<u32>,
    pub initial_rage: Option<i32>,
    pub haste_level: u32,
    pub network_delay_ms: u32,
    /// 每次评估时长从 [duration_min, duration_max] 区间按均匀分布随机采样
    pub duration_min: f64,
    pub duration_max: f64,
    /// 每个个体评估 K 次（不同时长），fitness = mean - 0.5*std
    /// K=1 时退化为单次随机采样（即旧行为）
    pub samples_per_eval: usize,
    /// 场景集：非空时 fitness 遍历所有场景（取代 K 次随机时长），
    /// 每个场景用自己的 delay/rage/duration/pauses。空时走老逻辑
    pub scenarios: Vec<ScenarioSpec>,
    pub skills: Arc<Vec<SkillSpec>>,
    pub recipes_table: Arc<Vec<RecipeEntry>>,
}

/// 多次评估的统计量
#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalStats {
    pub fitness: f64, // mean - 0.5*std（用于选拔）
    pub mean: f64,
    pub std: f64,
    pub min_dps: f64,
    pub max_dps: f64,
}

impl FitnessCtx {
    /// 单次模拟 → DPS（指定场景）
    pub fn run_once_scen(&self, values: &[f64], scen: &ScenarioSpec) -> f64 {
        let mut cfg = clone_macro_config(&self.baseline_macro);
        let params: Vec<TunableParam> = self.tunables.iter().map(|t| t.param.clone()).collect();
        apply_values(&mut cfg, &params, values);

        let skills = &*self.skills;
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
        let recipes_table = self.recipes_table.as_slice();

        let mut player = Player::new(self.haste_level, self.talents.clone(), self.recipes.clone());
        let rage = scen.initial_rage.or(self.initial_rage);
        if let Some(r) = rage {
            player.rage = r.clamp(0, 100);
        }
        let delay_sec = scen.delay_ms as f64 / 1000.0;
        let dmg_ctx = Some((self.attrs.clone(), self.target.clone()));

        let dur = scen.duration.max(1.0);
        let max_slots = (dur / 0.4).ceil() as u32 + 10;
        let mut prev_time = 0.0f64;
        let mut is_first_main = true;

        let (timeline, _debug, _line_stats) = simulate_macro(
            &cfg,
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
            &scen.pauses,
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

    /// 旧 API：单次模拟（只指定时长，延迟/怒气走 ctx 默认）
    pub fn run_once(&self, values: &[f64], duration: f64) -> f64 {
        self.run_once_scen(
            values,
            &ScenarioSpec {
                delay_ms: self.network_delay_ms,
                initial_rage: self.initial_rage,
                duration,
                pauses: Vec::new(),
            },
        )
    }

    /// 中点时长（用于归档 canonical DPS，与回放一致）
    pub fn midpoint_duration(&self) -> f64 {
        ((self.duration_min + self.duration_max) / 2.0).max(1.0)
    }

    /// 评估：scenarios 非空走场景遍历；否则 K 次随机时长采样
    pub fn evaluate(&self, values: &[f64]) -> EvalStats {
        let samples: Vec<f64> = if !self.scenarios.is_empty() {
            self.scenarios
                .iter()
                .map(|s| self.run_once_scen(values, s))
                .collect()
        } else {
            let k = self.samples_per_eval.max(1);
            let lo = self.duration_min.max(1.0);
            let hi = self.duration_max.max(lo);
            let mut rng = rand::thread_rng();
            let mut out: Vec<f64> = Vec::with_capacity(k);
            for _ in 0..k {
                let dur = if (hi - lo).abs() < 1e-6 {
                    lo
                } else {
                    rng.gen_range(lo..=hi)
                };
                out.push(self.run_once(values, dur));
            }
            out
        };
        let n = samples.len() as f64;
        let mean = samples.iter().sum::<f64>() / n;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let std = var.sqrt();
        let min_dps = samples.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_dps = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // fitness = 0.5×均值 + 0.5×最差（复合：均值驱动搜索，保底内化稳定性）
        EvalStats {
            fitness: 0.5 * mean + 0.5 * min_dps,
            mean,
            std,
            min_dps,
            max_dps,
        }
    }
}

// MacroConfig 没有 Clone derive，手动递归复制
fn clone_macro_config(cfg: &MacroConfig) -> MacroConfig {
    MacroConfig {
        pages: cfg
            .pages
            .iter()
            .map(|p| crate::macro_engine::MacroPage {
                stance_filter: p.stance_filter,
                lines: p.lines.iter().map(clone_macro_line).collect(),
            })
            .collect(),
    }
}

fn clone_macro_line(l: &MacroLine) -> MacroLine {
    MacroLine {
        condition: l.condition.as_ref().map(clone_condition),
        action: match &l.action {
            MacroAction::Cast(n) => MacroAction::Cast(n.clone()),
            MacroAction::FCast(n) => MacroAction::FCast(n.clone()),
        },
    }
}

fn clone_condition(c: &crate::macro_engine::MacroCondition) -> crate::macro_engine::MacroCondition {
    use crate::macro_engine::MacroCondition::*;
    match c {
        Rage(op, v) => Rage(*op, *v),
        Life(op, v) => Life(*op, *v),
        Buff(n) => Buff(n.clone()),
        NoBuff(n) => NoBuff(n.clone()),
        BuffTime(n, op, v) => BuffTime(n.clone(), *op, *v),
        BuffStack(n, op, v) => BuffStack(n.clone(), *op, *v),
        TBuff(n) => TBuff(n.clone()),
        TnoBuff(n) => TnoBuff(n.clone()),
        TBuffTime(n, op, v) => TBuffTime(n.clone(), *op, *v),
        SkillNotInCd(n) => SkillNotInCd(n.clone()),
        SkillExists(id) => SkillExists(*id),
        SkillNotExists(id) => SkillNotExists(*id),
        SkillEnergy(n, op, v) => SkillEnergy(n.clone(), *op, *v),
        LastSkill(n) => LastSkill(n.clone()),
        LastSkillNot(n) => LastSkillNot(n.clone()),
        NearbyEnemy(op, v) => NearbyEnemy(*op, *v),
        And(a, b) => And(Box::new(clone_condition(a)), Box::new(clone_condition(b))),
        Or(a, b) => Or(Box::new(clone_condition(a)), Box::new(clone_condition(b))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GA 个体 + 操作
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Individual {
    pub values: Vec<f64>,
    pub fitness: f64,
    pub stats: EvalStats,
}

fn random_individual(tunables: &[EffectiveTunable]) -> Individual {
    let mut rng = rand::thread_rng();
    let values = tunables.iter().map(|t| t.random(&mut rng)).collect();
    Individual {
        values,
        fitness: 0.0,
        stats: EvalStats::default(),
    }
}

/// 首代 seed：默认参数（用户在 UI 给的 original）
fn seed_individual(tunables: &[EffectiveTunable]) -> Individual {
    let values = tunables
        .iter()
        .map(|t| t.clamp_snap(t.param.original))
        .collect();
    Individual {
        values,
        fitness: 0.0,
        stats: EvalStats::default(),
    }
}

fn tournament<'a>(pop: &'a [Individual], k: usize, rng: &mut impl Rng) -> &'a Individual {
    tournament_by(pop, k, rng, &|ind: &Individual| ind.fitness)
}

fn tournament_by<'a>(
    pop: &'a [Individual],
    k: usize,
    rng: &mut impl Rng,
    key: &dyn Fn(&Individual) -> f64,
) -> &'a Individual {
    let mut best: Option<&'a Individual> = None;
    for _ in 0..k.max(1) {
        let i = rng.gen_range(0..pop.len());
        let cand = &pop[i];
        if best.map_or(true, |b| key(cand) > key(b)) {
            best = Some(cand);
        }
    }
    best.unwrap()
}

fn crossover(
    a: &Individual,
    b: &Individual,
    tunables: &[EffectiveTunable],
    rate: f64,
    rng: &mut impl Rng,
) -> Individual {
    if !rng.gen_bool(rate) {
        return a.clone();
    }
    let alpha = rng.gen_range(0.0..=1.0);
    let values = a
        .values
        .iter()
        .zip(b.values.iter())
        .enumerate()
        .map(|(i, (&x, &y))| tunables[i].clamp_snap(alpha * x + (1.0 - alpha) * y))
        .collect();
    Individual {
        values,
        fitness: 0.0,
        stats: EvalStats::default(),
    }
}

fn mutate(
    ind: &mut Individual,
    tunables: &[EffectiveTunable],
    rate: f64,
    sigma_pct: f64,
    rng: &mut impl Rng,
) {
    for (i, v) in ind.values.iter_mut().enumerate() {
        if rng.gen_bool(rate) {
            let t = &tunables[i];
            let sigma = t.range() * sigma_pct;
            let normal = Normal::new(0.0, sigma.max(1e-6)).unwrap();
            let delta = normal.sample(rng);
            *v = t.clamp_snap(*v + delta);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 进度事件 + 回调
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        run_id: String,
        total_gens: usize,
        pop_size: usize,
        n_params: usize,
        /// 启用的 tunable id 列表，顺序与后续事件中的 values 对齐
        enabled_ids: Vec<String>,
    },
    /// seed 个体（默认阈值）的 DPS —— 初代评估完成后立即发送，用作"提升 %"基准
    Baseline {
        dps: f64,
    },
    Progress {
        gen: usize,
        best: f64,
        avg: f64,
        worst: f64,
        diversity: f64,
        /// 当前代最优个体的值（与 enabled_ids 对齐）
        best_values: Vec<f64>,
        /// 当前代最优个体的 CV、均值、最差（用于前端 CV-主导展示）
        #[serde(skip_serializing_if = "Option::is_none")]
        best_cv: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        best_mean_dps: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        best_min_dps: Option<f64>,
    },
    Milestone {
        gen: usize,
        /// canonical DPS（在中点时长重算一次，与回放完全一致）
        dps: f64,
        file: String,
        values: Vec<f64>,
        macro_text: String,
        /// K 次随机采样的统计（mean / std / min / max）—— 用于"稳定性"展示
        #[serde(skip_serializing_if = "Option::is_none")]
        stats: Option<EvalStats>,
        #[serde(skip_serializing_if = "Option::is_none")]
        struct_diff: Option<super::struct_ga::StructDiff>,
    },
    TopN {
        items: Vec<TopNEntry>,
    },
    Done {
        best_dps: f64,
        total_gens: usize,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct TopNEntry {
    pub rank: usize,
    /// canonical DPS（中点时长重算）
    pub dps: f64,
    pub file: String,
    /// 该个体的值（与 enabled_ids 对齐）
    pub values: Vec<f64>,
    /// 已将值填入的完整宏文本，前端可直接显示
    pub macro_text: String,
    /// K 次采样的统计
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<EvalStats>,
    /// Phase 3：结构搜索产物（启用/禁用/新发现的规则）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_diff: Option<super::struct_ga::StructDiff>,
}

pub type ProgressSink = Box<dyn Fn(Event) + Send + Sync + 'static>;

// ─────────────────────────────────────────────────────────────────────────────
// GA 主循环
// ─────────────────────────────────────────────────────────────────────────────

pub struct GaRun<'a> {
    pub ctx: &'a FitnessCtx,
    pub params: &'a GaParams,
    pub base_loop: &'a LoopConfig,
    pub archive: &'a mut super::archive::ArchiveWriter,
    pub sink: &'a ProgressSink,
    pub stop: Arc<AtomicBool>,
}

impl<'a> GaRun<'a> {
    /// 返回 (最优个体, 最优 DPS, 实际跑过的代数)
    pub fn run(&mut self) -> (Option<Individual>, f64, usize) {
        let tunables = &self.ctx.tunables;
        let pop_size = self.params.pop_size.max(4);
        let dps_mode = self.params.fitness_dps_mode;

        // 个体排序键：DPS 模式 = fitness↑ 越好；CV 模式 = CV↓ 越好（用负 CV 统一为↑越好）
        let sort_key = |ind: &Individual| -> f64 {
            if dps_mode {
                ind.fitness
            } else {
                let cv = if ind.stats.mean.abs() > 1e-9 {
                    ind.stats.std / ind.stats.mean
                } else {
                    f64::INFINITY
                };
                -cv // 负 CV：越小的 CV 越好 → 负值越大越好
            }
        };

        // 初始化：1 个 seed（默认参数）+ 其余随机
        let mut population: Vec<Individual> = Vec::with_capacity(pop_size);
        population.push(seed_individual(tunables));
        while population.len() < pop_size {
            population.push(random_individual(tunables));
        }

        // 初代评估
        evaluate_population(&mut population, self.ctx);

        // 广播 baseline（seed 个体的 DPS）
        let baseline_dps = population.first().map(|i| i.fitness).unwrap_or(0.0);
        (self.sink)(Event::Baseline { dps: baseline_dps });

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
            let canonical = self.ctx.run_once(&bi.values, self.ctx.midpoint_duration());
            let cfg = build_loop_config(bi, self.ctx, self.base_loop);
            let macro_text = build_macro_text_for(bi, self.ctx);
            if let Ok(rel) = self.archive.write_milestone(0, canonical, &cfg) {
                (self.sink)(Event::Milestone {
                    gen: 0,
                    dps: canonical,
                    file: rel,
                    values: bi.values.clone(),
                    macro_text,
                    stats: Some(bi.stats.clone()),
                    struct_diff: None,
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
            let best_ind_ref = population.iter().max_by(|a, b| {
                sort_key(a)
                    .partial_cmp(&sort_key(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let best_values = best_ind_ref.map(|i| i.values.clone()).unwrap_or_default();
            // 另外取 CV 最低的个体（前端以它为"当前代最稳"来画 CV 曲线 + 仪表盘）
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
            self.archive.append_progress(&serde_json::json!({
                "gen": gen, "best": stats.best, "avg": stats.avg, "worst": stats.worst,
                "div": stats.diversity, "values": best_values,
                "best_cv": best_cv, "best_mean_dps": best_mean_dps, "best_min_dps": best_min_dps,
            })).ok();
            (self.sink)(Event::Progress {
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

            // 下一代：精英 + 锦标赛/交叉/变异
            let elite_count = ((pop_size as f64) * self.params.elitism_pct).ceil() as usize;
            let mut sorted = population.clone();
            sorted.sort_by(|a, b| {
                sort_key(b)
                    .partial_cmp(&sort_key(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut new_pop: Vec<Individual> = sorted.into_iter().take(elite_count).collect();

            let mut rng = rand::thread_rng();
            while new_pop.len() < pop_size {
                let p1 = tournament_by(&population, self.params.tournament_k, &mut rng, &sort_key)
                    .clone();
                let p2 = tournament_by(&population, self.params.tournament_k, &mut rng, &sort_key)
                    .clone();
                let mut child = crossover(&p1, &p2, tunables, self.params.crossover_rate, &mut rng);
                mutate(
                    &mut child,
                    tunables,
                    self.params.mutation_rate,
                    self.params.mutation_sigma_pct,
                    &mut rng,
                );
                new_pop.push(child);
            }

            // 评估新个体（精英已有 fitness，跳过）
            evaluate_population(&mut new_pop[elite_count..], self.ctx);
            population = new_pop;

            // 里程碑检查
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
                    let canonical = self.ctx.run_once(&gb.values, self.ctx.midpoint_duration());
                    let cfg = build_loop_config(&gb, self.ctx, self.base_loop);
                    let macro_text = build_macro_text_for(&gb, self.ctx);
                    if let Ok(rel) = self.archive.write_milestone(gen + 1, canonical, &cfg) {
                        (self.sink)(Event::Milestone {
                            gen: gen + 1,
                            dps: canonical,
                            file: rel,
                            values: gb.values.clone(),
                            macro_text,
                            stats: Some(gb.stats.clone()),
                            struct_diff: None,
                        });
                    }
                    best_ind = Some(gb);
                }
            }
        }

        // Top N（末代去重）按遗传方向排序
        population.sort_by(|a, b| {
            sort_key(b)
                .partial_cmp(&sort_key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let effective_top_n = self.params.top_n.min((gens_completed + 1).max(1));
        let mut top: Vec<Individual> = Vec::new();
        for ind in population.iter() {
            if top.len() >= effective_top_n {
                break;
            }
            if !top.iter().any(|t| values_approx_eq(&t.values, &ind.values)) {
                top.push(ind.clone());
            }
        }
        let mut entries = Vec::new();
        for (i, ind) in top.iter().enumerate() {
            let canonical = self.ctx.run_once(&ind.values, self.ctx.midpoint_duration());
            let cfg = build_loop_config(ind, self.ctx, self.base_loop);
            let macro_text = build_macro_text_for(ind, self.ctx);
            if let Ok(rel) = self.archive.write_topn(i + 1, canonical, &cfg) {
                entries.push(TopNEntry {
                    rank: i + 1,
                    dps: canonical,
                    file: rel,
                    values: ind.values.clone(),
                    macro_text,
                    stats: Some(ind.stats.clone()),
                    struct_diff: None,
                });
            }
        }
        (self.sink)(Event::TopN { items: entries });

        let final_dps = best_ind.as_ref().map(|i| i.fitness).unwrap_or(0.0);
        (best_ind, final_dps, gens_completed)
    }
}

fn cmp_fit(a: &&Individual, b: &&Individual) -> std::cmp::Ordering {
    a.fitness
        .partial_cmp(&b.fitness)
        .unwrap_or(std::cmp::Ordering::Equal)
}
fn cmp_fit_desc(a: &Individual, b: &Individual) -> std::cmp::Ordering {
    b.fitness
        .partial_cmp(&a.fitness)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn evaluate_population(pop: &mut [Individual], ctx: &FitnessCtx) {
    pop.par_iter_mut().for_each(|ind| {
        let s = ctx.evaluate(&ind.values);
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

fn compute_stats(pop: &[Individual]) -> Stats {
    let n = pop.len().max(1);
    let fits: Vec<f64> = pop.iter().map(|i| i.fitness).collect();
    let best = fits.iter().cloned().fold(f64::MIN, f64::max);
    let worst = fits.iter().cloned().fold(f64::MAX, f64::min);
    let avg = fits.iter().sum::<f64>() / n as f64;
    // 粒度：每维方差的均值标准化
    let dim = pop[0].values.len().max(1);
    let mut total_std = 0.0;
    for d in 0..dim {
        let mean = pop.iter().map(|i| i.values[d]).sum::<f64>() / n as f64;
        let var = pop
            .iter()
            .map(|i| (i.values[d] - mean).powi(2))
            .sum::<f64>()
            / n as f64;
        total_std += var.sqrt();
    }
    let diversity = total_std / dim as f64;
    Stats {
        best,
        avg,
        worst,
        diversity,
    }
}

fn values_approx_eq(a: &[f64], b: &[f64]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-3)
}

// ─────────────────────────────────────────────────────────────────────────────
// 个体 → LoopConfig（用于存档 + 前端"在循环模拟中查看"）
// ─────────────────────────────────────────────────────────────────────────────

fn build_loop_config(ind: &Individual, ctx: &FitnessCtx, base: &LoopConfig) -> LoopConfig {
    // 克隆 baseline MacroConfig + 写入个体值 → 序列化回文本
    let mut cfg = clone_macro_config(&ctx.baseline_macro);
    let params: Vec<TunableParam> = ctx.tunables.iter().map(|t| t.param.clone()).collect();
    apply_values(&mut cfg, &params, &ind.values);
    let macro_obj = macro_config_to_loop_macro(&cfg);

    LoopConfig {
        version: base.version,
        exported_at: Some(chrono_like_iso()),
        target: base.target.clone(),
        talents: base.talents.clone(),
        recipes: base.recipes.clone(),
        sequence: base.sequence.clone(),
        macro_pages: macro_obj,
        network_delay: base.network_delay,
        initial_rage: base.initial_rage,
        // 归档的 LoopConfig 在循环模拟中回放时用区间中位数（与优化器口径保持一致的近似）
        macro_duration: Some((ctx.duration_min + ctx.duration_max) / 2.0),
    }
}

fn macro_config_to_loop_macro(cfg: &MacroConfig) -> LoopMacro {
    let any_stance = cfg.pages.iter().any(|p| p.stance_filter.is_some());
    if !any_stance {
        let text = if cfg.pages.is_empty() {
            String::new()
        } else {
            serialize_lines(&cfg.pages[0].lines)
        };
        return LoopMacro {
            mode: "general".into(),
            general: text,
            shield: String::new(),
            blade: String::new(),
        };
    }
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

/// 个体 → 完整宏文本（用于前端 TopN/Milestone 内嵌预览 + 复制）
/// 精简版：去掉 X.0 的 .0 后缀、去掉 [] 方括号（省字数，JX3 宏 128 字符限制友好）
fn build_macro_text_for(ind: &Individual, ctx: &FitnessCtx) -> String {
    let mut cfg = clone_macro_config(&ctx.baseline_macro);
    let params: Vec<TunableParam> = ctx.tunables.iter().map(|t| t.param.clone()).collect();
    apply_values(&mut cfg, &params, &ind.values);
    let any_stance = cfg.pages.iter().any(|p| p.stance_filter.is_some());
    let raw = if !any_stance {
        cfg.pages
            .get(0)
            .map(|p| serialize_lines(&p.lines))
            .unwrap_or_default()
    } else {
        let mut out = String::new();
        for p in &cfg.pages {
            let tag = match p.stance_filter {
                Some(Stance::Shield) => Some("shield"),
                Some(Stance::Blade) => Some("blade"),
                Some(Stance::Wall) => Some("wall"),
                _ => None,
            };
            if let Some(t) = tag {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("#page {}\n", t));
            }
            out.push_str(&serialize_lines(&p.lines));
            out.push('\n');
        }
        out.trim().to_string()
    };
    format_macro_compact(&raw)
}

/// 精简宏文本：去 .0 尾、去 [] 方括号
fn format_macro_compact(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' || c == ']' {
            i += 1;
            continue;
        }
        // ".0" 尾：前面是数字，后面不是数字（即不是 "10.01" 这种小数中段）
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

fn serialize_lines(lines: &[MacroLine]) -> String {
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

fn chrono_like_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("t{}", secs)
}
