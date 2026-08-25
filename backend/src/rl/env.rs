//! CombatEnv：RL 训练/推理用的单步环境。
//!
//! 设计要点：
//! - `step(action)` 一次调用 = 一个"决策点"（释放一个技能或等待一段时间）。
//! - 跳帧由 `advance_to_next_decision` 处理：在 GCD 中或池为空时自动推进直到有多个合法动作。
//! - 奖励 = 本次 step 内新增的总伤害 / baseline_per_hit（归一化到 ~1）。
//! - 所有核心逻辑（cast / scripts / emit / fill / advance）委托给 `macro_eval::execute_cast`，
//!   保持与宏路径、手动序列路径完全一致的伤害/状态更新。
//!
//! `collect_timeline=true` 时会累积每次 step 产生的 CastEvent，供 rollout 导出使用。

use std::collections::HashMap;
use std::sync::Arc;

use crate::macro_eval::{execute_cast, evaluate_phase1, evaluate_phase2, CastCtx};
use crate::macro_engine::MacroConfig;
use crate::{
    frames_to_sec, resolve_combo_follow, Attributes, CastEvent, Player, RecipeEntry, SkillSpec,
    TargetConfig,
};

use super::action::{skill_to_action, ACTION_COUNT, ACTION_SKILLS, WAIT_ACTION};
use super::obs::{legal_mask, observe, OBS_DIM};

/// 战斗基线 per-hit 伤害，用于归一化 step reward 到 ~1 量级
const BASELINE_PER_HIT: f64 = 500_000.0;

pub struct EnvConfig {
    pub attrs: Attributes,
    pub target: TargetConfig,
    pub talents: Vec<u32>,
    pub recipes: Vec<u32>,
    pub haste_level: u32,
    pub initial_rage: i32,
    pub network_delay: f64,
    pub duration: f64,
    /// 是否记录 CastEvent timeline（rollout 导出用；训练时可关）
    pub collect_timeline: bool,
    /// 动作白名单 mask（None=全部允许）；与 legal_mask 取交集
    pub allowed_actions: Option<[bool; ACTION_COUNT]>,
}

pub struct StepOutput {
    pub obs: Vec<f32>,
    pub reward: f64,
    pub done: bool,
    pub legal_mask: [bool; ACTION_COUNT],
    pub info: StepInfo,
}

#[derive(Default)]
pub struct StepInfo {
    /// 是否成功释放了技能（false = 等待 or 非法动作）
    pub cast_success: bool,
    /// 本次 step 新增伤害
    pub damage_delta: f64,
}

pub struct CombatEnv {
    pub cfg: EnvConfig,
    pub skills: Arc<Vec<SkillSpec>>,
    pub recipes_table: Arc<Vec<RecipeEntry>>,

    pub player: Player,
    pub timeline: Vec<CastEvent>,
    pub total_damage: f64,

    last_action: usize,
    prev_time: f64,
    is_first_main: bool,
    last_busy_end: f64,
}

impl CombatEnv {
    pub fn new(
        cfg: EnvConfig,
        skills: Arc<Vec<SkillSpec>>,
        recipes_table: Arc<Vec<RecipeEntry>>,
    ) -> Self {
        let mut env = CombatEnv {
            player: Player::new(cfg.haste_level, cfg.talents.clone(), cfg.recipes.clone()),
            timeline: Vec::new(),
            total_damage: 0.0,
            last_action: WAIT_ACTION,
            prev_time: 0.0,
            is_first_main: true,
            last_busy_end: 0.0,
            cfg,
            skills,
            recipes_table,
        };
        env.player.rage = env.cfg.initial_rage.clamp(0, 100);
        env
    }

    pub fn reset(&mut self) -> (Vec<f32>, [bool; ACTION_COUNT]) {
        self.player = Player::new(
            self.cfg.haste_level,
            self.cfg.talents.clone(),
            self.cfg.recipes.clone(),
        );
        self.player.rage = self.cfg.initial_rage.clamp(0, 100);
        self.timeline.clear();
        self.total_damage = 0.0;
        self.last_action = WAIT_ACTION;
        self.prev_time = 0.0;
        self.is_first_main = true;
        self.last_busy_end = 0.0;
        (self.observe(), self.legal_mask())
    }

    pub fn elapsed(&self) -> f64 { self.player.current_time }
    pub fn done(&self) -> bool { self.player.current_time >= self.cfg.duration }

    pub fn observe(&self) -> Vec<f32> {
        let (skill_map, _) = self.build_maps();
        observe(
            &self.player,
            &skill_map,
            self.last_action,
            self.player.current_time,
            self.cfg.duration,
        )
    }

    pub fn legal_mask(&self) -> [bool; ACTION_COUNT] {
        let (skill_map, _) = self.build_maps();
        let mut mask = legal_mask(&self.player, &skill_map);
        if let Some(allow) = &self.cfg.allowed_actions {
            for i in 0..ACTION_COUNT {
                if !allow[i] { mask[i] = false; }
            }
            mask[WAIT_ACTION] = true;
        }
        mask
    }

    /// 用宏配置推断一个动作号（不一致分析用）
    pub fn macro_decision(&self, cfg: &MacroConfig, last_skill: Option<String>) -> u32 {
        let (skill_map, skill_by_id) = self.build_maps();
        // 选活跃宏页
        let page_idx = cfg
            .pages
            .iter()
            .position(|p| match p.stance_filter {
                None => true,
                Some(s) => s == self.player.stance(),
            })
            .unwrap_or(0);
        let (pool, _, _) = evaluate_phase1(&cfg.pages[page_idx], &self.player, &skill_map, &skill_by_id, last_skill, false);
        let (result, _) = evaluate_phase2(&pool, &self.player, &skill_map, &skill_by_id, false);
        match result {
            Some(r) => {
                let base = r.skill_name.split('·').next().unwrap_or(&r.skill_name);
                skill_to_action(base).unwrap_or(WAIT_ACTION) as u32
            }
            None => WAIT_ACTION as u32,
        }
    }

    /// 跳帧：推进 1 帧直到至少有 2 个合法动作，或 episode 结束
    pub fn advance_to_next_decision(&mut self) {
        let min_adv = frames_to_sec(1);
        loop {
            if self.done() { return; }
            let mask = self.legal_mask();
            let legal_count = mask.iter().filter(|&&x| x).count();
            if legal_count > 1 { return; }
            if legal_count == 1 && !mask[WAIT_ACTION] { return; }
            self.tick_one_frame(min_adv);
        }
    }

    /// 推进 target - current_time（不超过 duration），产出 buff ticks
    fn tick_one_frame(&mut self, advance: f64) {
        let target = (self.player.current_time + advance).min(self.cfg.duration);
        let dmg_ctx = (self.cfg.attrs.clone(), self.cfg.target.clone());
        let skill_by_id = build_skill_by_id(&self.skills);
        let mut tick_events = self.player.process_buff_ticks(self.prev_time, target);
        crate::fill_tick_events(
            &mut tick_events,
            &skill_by_id,
            Some(&dmg_ctx),
            &self.recipes_table,
            &self.player,
        );
        for ev in &tick_events {
            if let Some(d) = ev.damage_total { self.total_damage += d; }
        }
        if self.cfg.collect_timeline {
            self.timeline.extend(tick_events);
        }
        self.prev_time = target;
        self.player.current_time = target;
    }

    /// 执行一个动作
    pub fn step(&mut self, action: u32) -> StepOutput {
        let action = (action as usize).min(ACTION_COUNT - 1);
        let mut info = StepInfo::default();

        let mask = self.legal_mask();
        let before_damage = self.total_damage;

        if action == WAIT_ACTION || !mask[action] {
            self.tick_one_frame(frames_to_sec(1));
        } else {
            // 释放技能 —— 先通过 &self.skills 构建 map（仅借 skills，不借 self 整体）
            let name = ACTION_SKILLS[action].expect("legal non-wait action has skill");
            let skills_slice: &[SkillSpec] = &self.skills;
            let (skill_by_id, skill_map) = build_maps_from_slice(skills_slice);

            let ranks = match skill_map.get(name) {
                Some(r) => r,
                None => return self.finalize(before_damage, info),
            };
            let spec = match self.player.pick_rank(ranks) {
                Some(s) => s,
                None => return self.finalize(before_damage, info),
            };
            let actual_skill = resolve_combo_follow(spec, &self.player, &skill_by_id).unwrap_or(spec);

            let dmg_ctx = (self.cfg.attrs.clone(), self.cfg.target.clone());
            let recipes_slice: &[RecipeEntry] = &self.recipes_table;
            let ctx = CastCtx {
                skill_by_id: &skill_by_id,
                recipes_table: recipes_slice,
                dmg_ctx: Some(&dmg_ctx),
                network_delay: self.cfg.network_delay,
                is_macro: false,
            };
            let outcome = execute_cast(
                &mut self.player,
                actual_skill,
                &mut self.prev_time,
                &mut self.is_first_main,
                &mut self.last_busy_end,
                &ctx,
            );
            for ev in &outcome.events {
                if let Some(d) = ev.damage_total { self.total_damage += d; }
            }
            if self.cfg.collect_timeline {
                self.timeline.extend(outcome.events);
            }
            if outcome.cast_success {
                info.cast_success = true;
                self.last_action = action;
            }
        }

        info.damage_delta = self.total_damage - before_damage;
        self.finalize(before_damage, info)
    }

    fn finalize(&self, before_damage: f64, mut info: StepInfo) -> StepOutput {
        info.damage_delta = self.total_damage - before_damage;
        let reward = info.damage_delta / BASELINE_PER_HIT;
        let done = self.done();
        StepOutput {
            obs: self.observe(),
            reward,
            done,
            legal_mask: self.legal_mask(),
            info,
        }
    }

    /// 构建 (name → ranks) 和 (id → spec) 两个 map（只借 skills 切片，不借整个 self）
    fn build_maps<'a>(&'a self) -> (HashMap<&'a str, Vec<&'a SkillSpec>>, HashMap<u32, &'a SkillSpec>) {
        let (by_id, by_name) = build_maps_from_slice(&self.skills);
        (by_name, by_id)
    }

    pub fn fight_dps(&self) -> f64 {
        let last_cast = self
            .timeline
            .iter()
            .rev()
            .find(|e| !e.triggered)
            .map(|e| e.cast_time)
            .unwrap_or(self.player.current_time);
        let fight = self.player.fight_end(last_cast).max(0.001);
        self.total_damage / fight
    }
}

pub const OBS_SIZE: usize = OBS_DIM;

/// 从技能切片构建 (id→spec, name→ranks) 两个 map（借用切片；不借 env/self）
fn build_maps_from_slice<'a>(
    skills: &'a [SkillSpec],
) -> (HashMap<u32, &'a SkillSpec>, HashMap<&'a str, Vec<&'a SkillSpec>>) {
    let mut skill_by_id: HashMap<u32, &SkillSpec> = HashMap::new();
    let mut skill_map: HashMap<&str, Vec<&SkillSpec>> = HashMap::new();
    for s in skills.iter() {
        skill_by_id.insert(s.skill_id, s);
        if s.passive { continue; }
        let base = s.name.split('·').next().unwrap_or(&s.name);
        skill_map.entry(base).or_default().push(s);
    }
    (skill_by_id, skill_map)
}

/// 仅构建 id→spec（tick_one_frame 用）
fn build_skill_by_id<'a>(skills: &'a [SkillSpec]) -> HashMap<u32, &'a SkillSpec> {
    let mut m: HashMap<u32, &SkillSpec> = HashMap::new();
    for s in skills.iter() { m.insert(s.skill_id, s); }
    m
}
