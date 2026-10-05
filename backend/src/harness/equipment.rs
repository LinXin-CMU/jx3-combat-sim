//! Bounded equipment experiment. Every score is a complete simulator replay;
//! candidate attributes are always rebuilt from complete equipment slots.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::{hash::canonical_sha256, AgentRuntime, ScenarioSnapshotV1};
use crate::{equip, Attributes, GameVersion, Mount, SimulateRequest};

pub const POSITIONS: [&str; 12] = [
    "HAT",
    "JACKET",
    "BELT",
    "WRIST",
    "BOTTOMS",
    "SHOES",
    "NECKLACE",
    "PENDANT",
    "RING_1",
    "RING_2",
    "PRIMARY_WEAPON",
    "SECONDARY_WEAPON",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentSnapshot {
    pub slots: HashMap<String, equip::SlotConfig>,
    #[serde(default)]
    pub stone_id: u32,
    #[serde(default)]
    pub source_label: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    #[default]
    Catalog,
    ProvidedIds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentRequest {
    pub simulation: SimulateRequest,
    pub version: GameVersion,
    pub mount: Mount,
    pub equipment: EquipmentSnapshot,
    #[serde(default)]
    pub locked_slots: Vec<String>,
    #[serde(default)]
    pub candidate_source: CandidateSource,
    #[serde(default)]
    pub candidate_ids: BTreeMap<String, Vec<u32>>,
    #[serde(default)]
    pub min_item_level: Option<u32>,
    #[serde(default)]
    pub max_item_level: Option<u32>,
    /// Exact catalog `belong_map` values, not inferred acquisition availability.
    #[serde(default)]
    pub allowed_sources: Vec<String>,
    #[serde(default)]
    pub haste_min: Option<u32>,
    #[serde(default)]
    pub haste_max: Option<u32>,
    #[serde(default = "default_candidates")]
    pub max_candidates_per_slot: usize,
    #[serde(default = "default_simulations")]
    pub max_simulations: u32,
    #[serde(default = "default_wall")]
    pub wall_time_ms: u64,
    #[serde(default = "default_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_duration")]
    pub duration_seconds: f64,
}
fn default_candidates() -> usize {
    12
}
fn default_simulations() -> u32 {
    96
}
fn default_wall() -> u64 {
    60_000
}
fn default_rounds() -> u32 {
    6
}
fn default_duration() -> f64 {
    120.0
}

impl EquipmentRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=256).contains(&self.max_simulations)
            || !(1000..=120_000).contains(&self.wall_time_ms)
            || !(1..=12).contains(&self.max_rounds)
            || !(1..=32).contains(&self.max_candidates_per_slot)
            || !self.duration_seconds.is_finite()
            || !(10.0..=600.0).contains(&self.duration_seconds)
        {
            return Err("配装实验的预算、候选数或时长超出范围。".into());
        }
        if self.equipment.slots.len() != POSITIONS.len()
            || POSITIONS
                .iter()
                .any(|p| !self.equipment.slots.contains_key(*p))
        {
            return Err("必须提供全部 12 个部位的当前配装；空部位请显式提供 equip_id=0。".into());
        }
        if self.equipment.source_label.len() > 256
            || self.locked_slots.len() > 12
            || self
                .locked_slots
                .iter()
                .any(|p| !POSITIONS.contains(&p.as_str()))
            || self.locked_slots.iter().collect::<BTreeSet<_>>().len() != self.locked_slots.len()
            || self.candidate_ids.len() > 12
            || self.candidate_ids.iter().any(|(p, ids)| {
                !POSITIONS.contains(&p.as_str()) || ids.len() > 256 || ids.contains(&0)
            })
            || self.allowed_sources.len() > 32
            || self
                .allowed_sources
                .iter()
                .any(|s| s.is_empty() || s.len() > 256)
            || self
                .min_item_level
                .zip(self.max_item_level)
                .is_some_and(|(a, b)| a > b)
            || self
                .haste_min
                .zip(self.haste_max)
                .is_some_and(|(a, b)| a > b)
            || self.haste_max.is_some_and(|v| v > 10_000_000)
            || self.haste_min.is_some_and(|v| v > 10_000_000)
        {
            return Err("锁定部位、候选装备或属性约束无效。".into());
        }
        if self.equipment.slots["PRIMARY_WEAPON"].equip_id == 0 {
            return Err("请先在配装器配置主武器，再执行配装实验。".into());
        }
        for (position, slot) in &self.equipment.slots {
            if slot.strength > 8
                || slot.embedding.len() > 3
                || slot.embedding.iter().any(|v| *v > 8)
                || (slot.equip_id == 0 && (slot.enhance_id != 0 || slot.enchant_id != 0))
            {
                return Err(format!("{position} 的精炼、镶嵌或空槽附魔配置无效。"));
            }
        }
        if self.candidate_source == CandidateSource::ProvidedIds
            && self.candidate_ids.values().all(Vec::is_empty)
        {
            return Err("指定装备模式至少需要一个候选装备 ID。".into());
        }
        if self.candidate_source == CandidateSource::Catalog && !self.candidate_ids.is_empty() {
            return Err("目录模式不接受 candidate_ids；请明确选择 provided_ids。".into());
        }
        if self
            .candidate_ids
            .keys()
            .any(|p| self.locked_slots.contains(p))
        {
            return Err("锁定部位不能同时提供替换候选，请先明确该部位是否允许修改。".into());
        }
        let text = self
            .simulation
            .macro_text
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        if text.is_some()
            && (self.simulation.sequence.len() > 5000
                || self.simulation.sequence.iter().any(|s| s != "__macro__")
                || !self.simulation.channel_ticks.is_empty()
                || !self.simulation.timing_offsets.is_empty()
                || !self.simulation.qijin_buffs.is_empty())
        {
            return Err(
                "请选择完整手动技能轴或纯宏循环；当前不支持混合手动与宏的配装实验。".into(),
            );
        }
        // Validate the actual manual policy and its per-action overrides. Pure
        // macro requests reuse the same environment bounds through a placeholder;
        // the placeholder is never executed or substituted for a user's axis.
        let mut environment = self.simulation.clone();
        if text.is_some() {
            environment.sequence = vec!["validation_only".into()];
            environment.macro_text = None;
            environment.macro_duration = Some(self.duration_seconds);
        }
        super::contract::MacroCompileRequestV1 {
            simulation: environment,
            version: self.version,
            mount: self.mount,
            initial_macro: text.map(str::to_owned),
            max_simulations: self.max_simulations,
            wall_time_ms: self.wall_time_ms,
            max_rounds: self.max_rounds,
            max_pages: 6,
            time_tolerance: 1.0 / 16.0,
        }
        .validate()
        .map_err(str::to_owned)?;
        Ok(())
    }

    fn policy_scope(&self) -> &'static str {
        if self
            .simulation
            .macro_text
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            "fixed_macro_start_window"
        } else {
            "fixed_action_sequence"
        }
    }
}

#[derive(Clone, Serialize)]
struct Candidate {
    equipment: EquipmentSnapshot,
    simulation: SimulateRequest,
    scenario_hash: String,
    equipment_hash: String,
    fingerprint: String,
    dps: f64,
    total_damage: f64,
    fight_time: f64,
    haste_level: f64,
    constraints_passed: bool,
    verified: bool,
    skipped_count: usize,
    active_casts: usize,
    full_snapshots: bool,
    policy: Value,
}

fn matches_mount(mount: Mount, item: &equip::EquipItem) -> bool {
    match mount {
        Mount::FenShanJin => matches!(
            (item.belong_school.as_str(), item.magic_kind.as_str()),
            ("苍云", "外功") | ("通用", "身法") | ("精简", "外功")
        ),
        Mount::TieGuYi => {
            item.magic_kind == "防御" && matches!(item.belong_school.as_str(), "苍云" | "通用")
        }
    }
}

fn slot_valid(
    runtime: &AgentRuntime,
    position: &str,
    slot: &equip::SlotConfig,
) -> Result<(), String> {
    if slot.equip_id == 0 {
        return Ok(());
    }
    let subtype = equip::pos_to_subtype(position);
    let data = runtime
        .equipment_data()
        .ok_or("当前运行环境未加载装备目录。")?;
    let item = runtime
        .equipment_item(subtype, slot.equip_id)
        .ok_or_else(|| format!("{position} 装备 {} 不在当前运行目录中。", slot.equip_id))?;
    if !matches_mount(runtime.mount(), item)
        || item.require_level > runtime.context().constants.level
    {
        return Err(format!(
            "{position} 装备 {} 不符合当前心法或角色等级。",
            slot.equip_id
        ));
    }
    if slot.strength > item.max_strength {
        return Err(format!("{position} 的精炼超过该装备上限。"));
    }
    let mount_id = match runtime.mount() {
        Mount::FenShanJin => 10390,
        Mount::TieGuYi => 10389,
    };
    let accepted = |table: &HashMap<i32, Vec<equip::EnchantEntry>>, id| {
        table.get(&(subtype as i32)).is_some_and(|entries| {
            entries
                .iter()
                .any(|e| e.id == id && (e.belong_kungfu == 0 || e.belong_kungfu == mount_id))
        })
    };
    if slot.enhance_id != 0 && !accepted(&data.enhances, slot.enhance_id) {
        return Err(format!(
            "{position} 的小附魔 {} 不属于当前部位或心法。",
            slot.enhance_id
        ));
    }
    // Keep admission consistent with equip::calculate: some large enchants are
    // intentionally stored in the enhancement table for this catalog version.
    if slot.enchant_id != 0
        && !accepted(&data.enchants, slot.enchant_id)
        && !accepted(&data.enhances, slot.enchant_id)
    {
        return Err(format!(
            "{position} 的大附魔 {} 不属于当前部位或心法。",
            slot.enchant_id
        ));
    }
    Ok(())
}

fn validate_equipment(runtime: &AgentRuntime, config: &EquipmentSnapshot) -> Result<(), String> {
    if config.slots.len() != POSITIONS.len()
        || POSITIONS.iter().any(|p| !config.slots.contains_key(*p))
        || config.source_label.len() > 256
        || config
            .slots
            .get("PRIMARY_WEAPON")
            .is_none_or(|s| s.equip_id == 0)
        || config.slots.values().any(|s| {
            s.strength > 8
                || s.embedding.len() > 3
                || s.embedding.iter().any(|v| *v > 8)
                || (s.equip_id == 0 && (s.enhance_id != 0 || s.enchant_id != 0))
        })
    {
        return Err("候选必须包含完整合法的 12 部位配装。".into());
    }
    let data = runtime
        .equipment_data()
        .ok_or("当前运行环境未加载装备目录。")?;
    if config.stone_id != 0 && !data.stones.iter().any(|s| s.id == config.stone_id) {
        return Err("五彩石不在当前运行目录中。".into());
    }
    for position in POSITIONS {
        slot_valid(runtime, position, &config.slots[position])?;
    }
    Ok(())
}

fn catalog_filter(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    position: &str,
    item: &equip::EquipItem,
) -> bool {
    item.sub_type == equip::pos_to_subtype(position)
        && matches_mount(request.mount, item)
        && item.require_level <= runtime.context().constants.level
        && !item.magic_type.contains("(PVP)")
        && !item.magic_type.contains("(PVX)")
        && request.min_item_level.is_none_or(|v| item.level >= v)
        && request.max_item_level.is_none_or(|v| item.level <= v)
        && (request.allowed_sources.is_empty()
            || request.allowed_sources.contains(&item.belong_map))
}
pub(super) fn eligible(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    position: &str,
    item: &equip::EquipItem,
) -> bool {
    catalog_filter(request, runtime, position, item)
        && slot_valid(
            runtime,
            position,
            &equip::SlotConfig {
                equip_id: item.id,
                ..request.equipment.slots[position].clone()
            },
        )
        .is_ok()
}

fn candidate_pool(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
) -> Result<(BTreeMap<String, Vec<u32>>, Value), String> {
    let mut pool = BTreeMap::new();
    let mut sizes = BTreeMap::new();
    for position in POSITIONS {
        if request.locked_slots.iter().any(|p| p == position) {
            continue;
        }
        let mut items = match request.candidate_source {
            CandidateSource::Catalog => runtime
                .equipment_items()
                .filter(|item| eligible(request, runtime, position, item))
                .collect::<Vec<_>>(),
            CandidateSource::ProvidedIds => {
                let mut out = Vec::new();
                for id in request.candidate_ids.get(position).into_iter().flatten() {
                    let item = runtime
                        .equipment_item(equip::pos_to_subtype(position), *id)
                        .ok_or_else(|| format!("指定候选 {position}:{id} 不在当前装备目录。"))?;
                    if !eligible(request, runtime, position, item) {
                        return Err(format!(
                            "指定候选 {position}:{id} 不符合部位、心法、品级、来源或强化约束。"
                        ));
                    }
                    out.push(item);
                }
                out
            }
        };
        items.sort_by(|a, b| b.level.cmp(&a.level).then(a.id.cmp(&b.id)));
        items.dedup_by_key(|item| item.id);
        let available = items.len();
        items.truncate(request.max_candidates_per_slot);
        let mut ids = items.iter().map(|item| item.id).collect::<Vec<_>>();
        let current = request.equipment.slots[position].equip_id;
        ids.retain(|id| *id != current);
        if !ids.is_empty() {
            // Keeping the equipped item means grade/source filters govern replacements;
            // a legitimate current build is not silently discarded or partially removed.
            ids.insert(0, current);
            sizes.insert(position.to_string(),json!({"eligible_replacements":available,"retained_choices":ids.len(),"truncated":available > request.max_candidates_per_slot}));
            pool.insert(position.to_string(), ids);
        }
    }
    Ok((pool, json!(sizes)))
}

fn attributes(raw: &equip::RawAttrs) -> Attributes {
    Attributes {
        vitality: raw.vitality,
        li_dao: raw.strength,
        gen_gu: raw.spirit,
        yuan_qi: raw.spunk,
        shen_fa: raw.agility,
        base_attack: raw.base_attack,
        base_magical_attack: raw.base_magical_attack,
        weapon_damage: raw.weapon_damage,
        surplus_value: raw.surplus_value,
        crit_level: raw.crit_level,
        crit_effect_level: raw.crit_effect_level,
        overcome_level: raw.overcome_level,
        strain_level: raw.strain_level,
        haste_level: raw.haste_level,
        parry_value: raw.parry_value,
        parry_level: raw.parry_level,
    }
}

/// Rebuilds all attributes and equipment effects, preserving every other frozen
/// environment field, including initial resources, procs, buffs and pre-releases.
fn build_simulation(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    config: &EquipmentSnapshot,
) -> Result<(SimulateRequest, bool), String> {
    validate_equipment(runtime, config)?;
    let calc =
        runtime.calculate_equipment(&config.slots, config.stone_id, &scenario.simulation.talents);
    let mut simulation = scenario.simulation.clone();
    simulation.attributes = Some(attributes(&calc.raw));
    simulation.haste_level = calc.raw.haste_level.round().max(0.0) as u32;
    simulation.equipment.clear();
    for (position, slot) in &config.slots {
        if slot.equip_id != 0 {
            simulation.equipment.insert(position.clone(), slot.equip_id);
        }
        if slot.enchant_id != 0 {
            simulation
                .equipment
                .insert(format!("ENCHANT_{position}"), slot.enchant_id);
        }
    }
    if request.policy_scope() == "fixed_macro_start_window" {
        simulation.sequence =
            vec!["__macro__".into(); (request.duration_seconds / 0.25).ceil() as usize + 20];
        simulation.macro_duration = Some(request.duration_seconds);
    }
    simulation.lite = false;
    simulation.lite_keep_timeline = false;
    let passes = request
        .haste_min
        .is_none_or(|v| calc.raw.haste_level >= v as f64)
        && request
            .haste_max
            .is_none_or(|v| calc.raw.haste_level <= v as f64);
    Ok((simulation, passes))
}

fn evaluate(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    config: EquipmentSnapshot,
) -> Result<Candidate, String> {
    let (simulation, constraints_passed) = build_simulation(request, runtime, scenario, &config)?;
    let snapshot = ScenarioSnapshotV1::capture(request.version, request.mount, simulation.clone())
        .map_err(|e| e.to_string())?;
    let equipment_hash = canonical_sha256(&config).map_err(|e| e.to_string())?;
    let policy_hash = canonical_sha256(&json!({
        "scope":request.policy_scope(),"sequence":simulation.sequence,
        "macro_text":simulation.macro_text,"macro_duration":simulation.macro_duration,
        "channel_ticks":simulation.channel_ticks,"timing_offsets":simulation.timing_offsets,
        "qijin_buffs":simulation.qijin_buffs,
    }))
    .map_err(|e| e.to_string())?;
    let policy = json!({"scope":request.policy_scope(),"hash":policy_hash,
        "action_count":simulation.sequence.len(),
        "macro_start_window_seconds":if request.policy_scope()=="fixed_macro_start_window" {Some(request.duration_seconds)}else{None}});
    let context = runtime.context();
    let response = crate::simulate_core(
        &simulation,
        context.skills,
        context.game_version,
        context.mount,
        context.constants,
        context.recipes,
        context.team_buffs,
        context.formations,
    );
    if !response.dps.is_finite()
        || !response.total_damage.is_finite()
        || !response.fight_time.is_finite()
        || response.fight_time <= 0.0
    {
        return Err("配装候选未产生有限的有效模拟结果。".into());
    }
    let active = response
        .timeline
        .iter()
        .filter(|event| super::alignment::is_active(event))
        .collect::<Vec<_>>();
    let full_snapshots = active.iter().all(|event| {
        event
            .state_before
            .as_ref()
            .is_some_and(|state| state.time.is_finite())
    });
    let verified = !active.is_empty() && full_snapshots && response.skipped.is_empty();
    Ok(Candidate {
        equipment: config,
        haste_level: simulation.attributes.as_ref().unwrap().haste_level,
        simulation,
        scenario_hash: snapshot.scenario_hash,
        equipment_hash,
        fingerprint: response.fingerprint.to_string(),
        dps: response.dps,
        total_damage: response.total_damage,
        fight_time: response.fight_time,
        constraints_passed,
        verified,
        skipped_count: response.skipped.len(),
        active_casts: active.len(),
        full_snapshots,
        policy,
    })
}

fn same_slot(left: &equip::SlotConfig, right: &equip::SlotConfig) -> bool {
    left.equip_id == right.equip_id
        && left.strength == right.strength
        && left.embedding == right.embedding
        && left.enhance_id == right.enhance_id
        && left.enchant_id == right.enchant_id
}

/// Admission and attribute rebuild for a model-proposed complete build. This
/// performs zero simulations and does not apply the search pool's top-N cutoff.
/// Provided IDs, source/grade filters and locked slots remain hard constraints.
pub fn evaluate_snapshot(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    candidate: &EquipmentSnapshot,
) -> Result<SimulateRequest, String> {
    request.validate()?;
    if request.version != runtime.game_version() || request.mount != runtime.mount() {
        return Err("候选与当前运行版本或心法不一致。".into());
    }
    scenario.verify_hash().map_err(|e| e.to_string())?;
    let expected =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .map_err(|e| e.to_string())?;
    if expected.scenario_hash != scenario.scenario_hash {
        return Err("候选与冻结场景不一致。".into());
    }
    validate_equipment(runtime, &request.equipment)?;
    validate_equipment(runtime, candidate)?;
    for position in POSITIONS {
        let original = &request.equipment.slots[position];
        let next = &candidate.slots[position];
        if request.locked_slots.iter().any(|p| p == position) && !same_slot(original, next) {
            return Err(format!("候选修改了锁定部位 {position}。"));
        }
        if original.equip_id != next.equip_id {
            let item = runtime
                .equipment_item(equip::pos_to_subtype(position), next.equip_id)
                .ok_or_else(|| format!("{position} 的替换件不存在。"))?;
            if !catalog_filter(request, runtime, position, item) {
                return Err(format!("{position} 的替换件不符合目录、来源或品级约束。"));
            }
            if request.candidate_source == CandidateSource::ProvidedIds
                && !request
                    .candidate_ids
                    .get(position)
                    .is_some_and(|ids| ids.contains(&next.equip_id))
            {
                return Err(format!("{position} 的替换件不在明确提供的候选范围。"));
            }
        }
    }
    if request.locked_slots.iter().any(|p| p == "PRIMARY_WEAPON")
        && candidate.stone_id != request.equipment.stone_id
    {
        return Err("主武器锁定时不能更改五彩石。".into());
    }
    let (simulation, passes) = build_simulation(request, runtime, scenario, candidate)?;
    if !passes {
        return Err("候选配装不满足加速等级约束。".into());
    }
    Ok(simulation)
}

fn config_key(config: &EquipmentSnapshot) -> Vec<u32> {
    POSITIONS
        .iter()
        .map(|p| config.slots[*p].equip_id)
        .collect()
}
fn better(left: &Candidate, right: &Candidate) -> bool {
    left.dps > right.dps + 1e-9
        || ((left.dps - right.dps).abs() <= 1e-9
            && config_key(&left.equipment) < config_key(&right.equipment))
}
fn stop(
    request: &EquipmentRequest,
    cancel: &AtomicBool,
    start: Instant,
    simulations: u32,
) -> Option<&'static str> {
    if cancel.load(Ordering::Relaxed) {
        Some("cancelled")
    } else if start.elapsed().as_millis() >= u128::from(request.wall_time_ms) {
        Some("time_budget")
    } else if simulations >= request.max_simulations {
        Some("budget_exhausted")
    } else {
        None
    }
}

pub fn equipment_diff(
    runtime: &AgentRuntime,
    before: &EquipmentSnapshot,
    after: &EquipmentSnapshot,
) -> Value {
    let view = |position: &str, slot: &equip::SlotConfig| {
        let item = runtime.equipment_item(equip::pos_to_subtype(position), slot.equip_id);
        json!({"config":slot,"id":slot.equip_id,"name":item.map(|i|i.name.as_str()).unwrap_or("空部位"),
            "item_level":item.map(|i|i.level),"source":item.map(|i|i.belong_map.as_str()),"price":Value::Null})
    };
    let mut diff = POSITIONS
        .into_iter()
        .filter_map(|p| match (before.slots.get(p), after.slots.get(p)) {
            (Some(a), Some(b)) if !same_slot(a, b) => {
                Some(json!({"position":p,"before":view(p,a),"after":view(p,b)}))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if before.stone_id != after.stone_id {
        let stone = |id| json!({"id":id,"name":runtime.equipment_data().and_then(|d|d.stones.iter().find(|s|s.id==id)).map(|s|s.name.as_str()),"price":Value::Null});
        diff.push(json!({"position":"STONE","before":stone(before.stone_id),"after":stone(after.stone_id)}));
    }
    json!(diff)
}

/// Typed experiment operator; no files, settings, saved builds or user data are written.
pub fn run(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    cancel: &AtomicBool,
    progress: impl FnMut(Value),
) -> Result<Value, String> {
    run_with_evaluator(request, runtime, scenario, cancel, progress, evaluate)
}

fn run_with_evaluator(
    request: &EquipmentRequest,
    runtime: &AgentRuntime,
    scenario: &ScenarioSnapshotV1,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Value),
    mut evaluator: impl FnMut(
        &EquipmentRequest,
        &AgentRuntime,
        &ScenarioSnapshotV1,
        EquipmentSnapshot,
    ) -> Result<Candidate, String>,
) -> Result<Value, String> {
    request.validate()?;
    if request.version != runtime.game_version() || request.mount != runtime.mount() {
        return Err("配装实验与当前运行版本或心法不一致。".into());
    }
    scenario.verify_hash().map_err(|e| e.to_string())?;
    let expected =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .map_err(|e| e.to_string())?;
    if expected.scenario_hash != scenario.scenario_hash {
        return Err("配装实验输入与冻结场景不一致。".into());
    }
    validate_equipment(runtime, &request.equipment)?;
    let start = Instant::now();
    let mut simulations = 0_u32;
    let mut history = Vec::new();
    let mut best: Option<Candidate> = None;
    let mut baseline: Option<Candidate> = None;
    let mut rounds = 0;
    let mut reason = stop(request, cancel, start, simulations).unwrap_or("completed");
    let (pool, pool_stats) = candidate_pool(request, runtime)?;
    let product = pool
        .values()
        .fold(1_u64, |n, ids| n.saturating_mul(ids.len() as u64));
    let exhaustive = product <= u64::from(request.max_simulations);
    let mut visited = BTreeSet::new();
    let mut skipped_constraints = 0_u32;
    let mut failed_candidates = 0_u32;
    let mut unverified_candidates = 0_u32;
    let mut validation_errors = 0_u32;
    let mut beam = Vec::<Candidate>::new();
    if reason == "completed" {
        // Admission failures before the first replay may still reject the
        // request. Once a replay starts, retain spent budget and evidence even
        // if a later result cannot be admitted as a verified candidate.
        let (prepared, _) = build_simulation(request, runtime, scenario, &request.equipment)?;
        ScenarioSnapshotV1::capture(request.version, request.mount, prepared)
            .map_err(|e| e.to_string())?;
        canonical_sha256(&request.equipment).map_err(|e| e.to_string())?;
        simulations += 1;
        progress(
            json!({"phase":"equipment_baseline","message":"正在从完整当前配装重算并模拟基线。","simulations":simulations,"best":best}),
        );
        match evaluator(request, runtime, scenario, request.equipment.clone()) {
            Ok(original) => {
                visited.insert(config_key(&original.equipment));
                if original.constraints_passed && original.verified {
                    best = Some(original.clone());
                }
                if !original.verified {
                    unverified_candidates += 1;
                }
                baseline = Some(original.clone());
                // The original configuration remains a proposal seed even if
                // its observed axis skips an action. It is not admissible as
                // best evidence until a full candidate replay is verified.
                beam.push(original);
                progress(
                    json!({"phase":"equipment_baseline","message":"已从完整当前配装重算并模拟基线。","simulations":simulations,"best":best}),
                );
            }
            Err(error) => {
                failed_candidates += 1;
                reason = "baseline_failed";
                history.push(json!({"round":0,"accepted":false,"verified":false,"phase":"baseline","error":error}));
                progress(
                    json!({"phase":"equipment_baseline","message":"基线模拟未产生可采纳结果，已保留消耗计数。","simulations":simulations,"best":best}),
                );
            }
        }
    }
    let positions = pool.keys().cloned().collect::<Vec<_>>();
    'rounds: for round in 0..if exhaustive { 1 } else { request.max_rounds } {
        if baseline.is_none() {
            break;
        }
        if let Some(why) = stop(request, cancel, start, simulations) {
            reason = why;
            break;
        }
        rounds = round + 1;
        let mut proposals = Vec::new();
        if exhaustive {
            for mut index in 0..product {
                let mut config = request.equipment.clone();
                for position in &positions {
                    let ids = &pool[position];
                    config.slots.get_mut(position).unwrap().equip_id =
                        ids[index as usize % ids.len()];
                    index /= ids.len() as u64;
                }
                proposals.push(config);
            }
        } else {
            // Interleave candidate rank and position so a small budget does not
            // spend every replay on the first slot. Beam width is bounded at 3.
            let depth = pool.values().map(Vec::len).max().unwrap_or(0);
            for rank in 0..depth {
                for parent in &beam {
                    for offset in 0..positions.len() {
                        let position = &positions[(offset + round as usize) % positions.len()];
                        if let Some(id) = pool[position].get(rank) {
                            let mut config = parent.equipment.clone();
                            config.slots.get_mut(position).unwrap().equip_id = *id;
                            proposals.push(config);
                        }
                    }
                }
            }
        }
        let mut evaluated = 0;
        let mut next_beam = beam.clone();
        for config in proposals {
            if let Some(why) = stop(request, cancel, start, simulations) {
                reason = why;
                break 'rounds;
            }
            if !visited.insert(config_key(&config)) {
                continue;
            }
            let prepared = build_simulation(request, runtime, scenario, &config).and_then(
                |(simulation, passes)| {
                    ScenarioSnapshotV1::capture(request.version, request.mount, simulation)
                        .map_err(|e| e.to_string())?;
                    canonical_sha256(&config).map_err(|e| e.to_string())?;
                    Ok(passes)
                },
            );
            let passes = match prepared {
                Ok(passes) => passes,
                Err(error) => {
                    validation_errors += 1;
                    history.push(json!({"round":rounds,"accepted":false,"verified":false,"phase":"preparation","error":error}));
                    continue;
                }
            };
            if !passes {
                skipped_constraints += 1;
                continue;
            }
            simulations += 1;
            evaluated += 1;
            progress(
                json!({"phase":"equipment_search","message":"正在执行完整候选模拟。","simulations":simulations,"best":best}),
            );
            let candidate = match evaluator(request, runtime, scenario, config) {
                Ok(candidate) => candidate,
                Err(error) => {
                    failed_candidates += 1;
                    history.push(json!({"round":rounds,"accepted":false,"verified":false,"phase":"simulation","error":error}));
                    progress(
                        json!({"phase":"equipment_search","message":"一个候选模拟未产生可采纳结果，继续保留已验证方案。","simulations":simulations,"best":best}),
                    );
                    continue;
                }
            };
            if !candidate.verified {
                unverified_candidates += 1;
            }
            let accepted = candidate.verified
                && candidate.constraints_passed
                && best.as_ref().is_none_or(|b| better(&candidate, b));
            history.push(json!({"round":rounds,"accepted":accepted,"equipment_hash":candidate.equipment_hash,
                "scenario_hash":candidate.scenario_hash,"fingerprint":candidate.fingerprint,"dps":candidate.dps,
                "constraints_passed":candidate.constraints_passed,"verified":candidate.verified,
                "skipped_count":candidate.skipped_count,"active_casts":candidate.active_casts}));
            if accepted {
                best = Some(candidate.clone());
            }
            if candidate.verified {
                next_beam.push(candidate);
            }
            progress(
                json!({"phase":"equipment_search","message":"正在重算候选配装并执行完整模拟。","simulations":simulations,"best":best}),
            );
        }
        next_beam.sort_by(|a, b| {
            b.dps
                .total_cmp(&a.dps)
                .then(config_key(&a.equipment).cmp(&config_key(&b.equipment)))
        });
        next_beam.dedup_by(|a, b| config_key(&a.equipment) == config_key(&b.equipment));
        next_beam.truncate(3);
        beam = next_beam;
        if exhaustive {
            reason = "candidate_pool_exhausted";
            break;
        }
        if evaluated == 0 {
            reason = "no_new_candidates";
            break;
        }
        reason = "max_rounds";
    }
    let delta = baseline
        .as_ref()
        .zip(best.as_ref())
        .map(|(a, b)| b.dps - a.dps);
    let diff = best
        .as_ref()
        .map(|b| equipment_diff(runtime, &request.equipment, &b.equipment))
        .unwrap_or_else(|| json!([]));
    let baseline_policy = baseline.as_ref().map(|b| &b.policy);
    let candidate_policy = best.as_ref().map(|b| &b.policy);
    Ok(
        json!({"schema_version":"harness-equipment/v1","task":"optimize_equipment","stop_reason":reason,
        "simulations":simulations,"rounds":rounds,"elapsed_ms":start.elapsed().as_millis(),"duration_seconds":request.duration_seconds,
        "input_scenario_hash":scenario.scenario_hash,"baseline":baseline,"best":best,"slot_diff":diff,"dps_delta":delta,
        "policy_scope":request.policy_scope(),"baseline_policy":baseline_policy,"candidate_policy":candidate_policy,
        "history":history,"candidate_pool":pool_stats,"candidate_combinations":product,"search_method":if exhaustive {"enumeration"} else {"bounded_beam_width_3"},
        "pool_search_complete":exhaustive && reason=="candidate_pool_exhausted" && failed_candidates==0 && validation_errors==0 && unverified_candidates==0,"skipped_constraints":skipped_constraints,
        "failed_candidates":failed_candidates,"unverified_candidates":unverified_candidates,"validation_errors":validation_errors,
        "limitations":["仅比较冻结环境与同一动作策略下的配装输出；不代表实战全局最优。",
            "手动轴按原动作、顺序及逐动作覆盖完整执行，duration_seconds不截断或填充手动轴；加速导致的实际fight_time变化属于该策略输出。",
            "纯宏由duration_seconds限制启动窗口，并保留自然引导尾段。实际fight_time与DPS按引擎完整结果记录，不用事件时间伪截断伤害。手动轴与纯宏窗口的结果不能直接横比收益。",
            "候选按当前装备目录的心法、角色等级、PVE标记、品级和来源元数据过滤；不推断赛季投放日期或实际可获取性。",
            "品级与来源限制作用于替换件；已有装备和锁定件完整保留。目录候选按品级降序截断，大候选池使用有限束搜索。",
            "精炼、镶嵌、大小附魔与五彩石继承当前配装；不搜索这些维度或奇穴、秘籍、宏。",
            "未提供价格数据，价格保持未知，未评估金币预算、购买或获取成本。",
            "仅验证当前随机种子；取消在一次完整模拟结束后生效。"]}),
    )
}

#[cfg(test)]
#[path = "../../tests/harness/equipment.rs"]
mod tests;
