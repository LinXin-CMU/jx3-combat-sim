use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::{equip, Attributes, Mount, SimulateRequest};

use super::{
    simulate_scenario, AgentRuntime, EvidenceEnvelopeV1, ScenarioSnapshotV1, SimulationSummary,
    ToolBudget, ToolError,
};

pub const INSPECT_EQUIPMENT_WORKSPACE: &str = "inspect_equipment_workspace";
pub const COMPARE_FOCUSED_EQUIPMENT: &str = "compare_focused_equipment";
pub const SEARCH_EQUIPMENT_CATALOG: &str = "search_equipment_catalog";
pub const COMPARE_EQUIPMENT_STRATEGIES: &str = "compare_equipment_strategies";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentFocusV1 {
    pub position: String,
    pub candidate_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentWorkspaceV1 {
    pub slots: HashMap<String, equip::SlotConfig>,
    #[serde(default)]
    pub stone_id: u32,
    #[serde(default)]
    pub source_label: String,
    #[serde(default)]
    pub focus: Option<EquipmentFocusV1>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentItemSummaryV1 {
    pub position: String,
    pub id: u32,
    pub name: String,
    pub level: u32,
    pub quality: u8,
    pub set_name: Option<String>,
    pub category: String,
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentWorkspaceSummaryV1 {
    pub source_label: String,
    pub equipped: Vec<EquipmentItemSummaryV1>,
    pub panel: BTreeMap<String, f64>,
    pub focused_candidate: Option<EquipmentItemSummaryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EquipmentPanelRowV1 {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub before: f64,
    pub after: f64,
    pub delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EquipmentComparisonPresentationV1 {
    pub title: String,
    pub position: String,
    pub before_item: String,
    pub after_item: String,
    pub panel_rows: Vec<EquipmentPanelRowV1>,
    pub before_dps: f64,
    pub after_dps: f64,
    pub dps_delta: f64,
    pub dps_delta_percent: f64,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquipmentComparisonEvidenceV1 {
    pub comparison: EquipmentComparisonPresentationV1,
    pub before_equipment: Vec<EquipmentItemSummaryV1>,
    pub after_equipment: Vec<EquipmentItemSummaryV1>,
    pub baseline: SimulationSummary,
    pub candidate: SimulationSummary,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentCatalogQueryV1 {
    pub query: String,
    #[serde(default)]
    pub position: Option<String>,
}

pub fn inspect_workspace(
    runtime: &AgentRuntime,
    workspace: &EquipmentWorkspaceV1,
    talents: &[u32],
) -> EquipmentWorkspaceSummaryV1 {
    let panel = runtime.calculate_equipment(&workspace.slots, workspace.stone_id, talents);
    let mut equipped = workspace
        .slots
        .iter()
        .filter_map(|(position, slot)| item_summary(runtime, position, slot.equip_id))
        .collect::<Vec<_>>();
    equipped.sort_by(|left, right| left.position.cmp(&right.position));
    let focused_candidate = workspace
        .focus
        .as_ref()
        .and_then(|focus| item_summary(runtime, &focus.position, focus.candidate_id));
    EquipmentWorkspaceSummaryV1 {
        source_label: workspace.source_label.clone(),
        equipped,
        panel: panel_map(&panel),
        focused_candidate,
    }
}

pub fn search_catalog(
    runtime: &AgentRuntime,
    query: &EquipmentCatalogQueryV1,
) -> Vec<EquipmentItemSummaryV1> {
    let normalized = query.query.trim().to_lowercase();
    let wants_qiegao = normalized.contains("切糕");
    let wants_set = normalized.contains("套装")
        || normalized.contains("四件套")
        || normalized.contains("4件套");
    let mut items = runtime
        .equipment_items()
        .filter_map(|item| {
            let position = subtype_position(item.sub_type)?;
            if query
                .position
                .as_deref()
                .is_some_and(|value| value != position)
            {
                return None;
            }
            let set_name = (item.set_id > 0)
                .then(|| runtime.equipment_set_name(item.set_id))
                .flatten();
            let haystack = format!(
                "{} {} {} {}",
                item.name,
                item.magic_type,
                item.belong_school,
                set_name.as_deref().unwrap_or("")
            );
            let matches = if wants_qiegao {
                set_name
                    .as_deref()
                    .is_some_and(|name| name.contains("切糕"))
            } else if wants_set {
                item.set_id > 0
                    && !set_name
                        .as_deref()
                        .is_some_and(|name| name.contains("切糕"))
            } else {
                normalized.is_empty() || haystack.to_lowercase().contains(&normalized)
            };
            matches.then(|| item_summary_from_item(runtime, position, item))
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .level
            .cmp(&left.level)
            .then_with(|| left.name.cmp(&right.name))
    });
    items.truncate(16);
    items
}

pub fn compare_focus(
    trace_id: &str,
    baseline: &ScenarioSnapshotV1,
    runtime: &AgentRuntime,
    workspace: &EquipmentWorkspaceV1,
    budget: &mut ToolBudget,
) -> Result<
    (
        EvidenceEnvelopeV1<EquipmentComparisonEvidenceV1>,
        EquipmentComparisonPresentationV1,
    ),
    ToolError,
> {
    let focus = workspace
        .focus
        .as_ref()
        .ok_or(ToolError::EquipmentFocusUnavailable)?;
    let current = workspace
        .slots
        .get(&focus.position)
        .ok_or(ToolError::EquipmentFocusUnavailable)?;
    if current.equip_id == focus.candidate_id {
        return Err(ToolError::EquipmentFocusUnavailable);
    }
    let before_item = item_summary(runtime, &focus.position, current.equip_id)
        .ok_or(ToolError::EquipmentFocusUnavailable)?;
    let after_item = item_summary(runtime, &focus.position, focus.candidate_id)
        .ok_or(ToolError::EquipmentFocusUnavailable)?;
    let mut candidate_slots = workspace.slots.clone();
    let mut candidate_slot = current.clone();
    candidate_slot.equip_id = focus.candidate_id;
    candidate_slots.insert(focus.position.clone(), candidate_slot);

    let before_calc = runtime.calculate_equipment(
        &workspace.slots,
        workspace.stone_id,
        &baseline.simulation.talents,
    );
    let after_calc = runtime.calculate_equipment(
        &candidate_slots,
        workspace.stone_id,
        &baseline.simulation.talents,
    );
    let baseline_snapshot =
        snapshot_with_build(baseline, runtime, &workspace.slots, &before_calc.raw)?;
    let candidate_snapshot =
        snapshot_with_build(baseline, runtime, &candidate_slots, &after_calc.raw)?;
    let context = runtime.context();
    let before_sim = simulate_scenario(
        trace_id,
        &baseline_snapshot,
        &context,
        runtime.provenance(),
        budget,
    )?;
    let after_sim = simulate_scenario(
        trace_id,
        &candidate_snapshot,
        &context,
        runtime.provenance(),
        budget,
    )?;
    let before_dps = before_sim.evidence.result.dps;
    let after_dps = after_sim.evidence.result.dps;
    let presentation = EquipmentComparisonPresentationV1 {
        title: format!(
            "{}：{} → {}",
            focus.position, before_item.name, after_item.name
        ),
        position: focus.position.clone(),
        before_item: before_item.name.clone(),
        after_item: after_item.name.clone(),
        panel_rows: panel_rows(&before_calc, &after_calc),
        before_dps,
        after_dps,
        dps_delta: after_dps - before_dps,
        dps_delta_percent: if before_dps.abs() > f64::EPSILON {
            (after_dps / before_dps - 1.0) * 100.0
        } else {
            0.0
        },
        source_label: workspace.source_label.clone(),
    };
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        COMPARE_FOCUSED_EQUIPMENT,
        &baseline.scenario_hash,
        serde_json::json!({"position": focus.position, "candidate_id": focus.candidate_id}),
        EquipmentComparisonEvidenceV1 {
            comparison: presentation.clone(),
            before_equipment: vec![before_item],
            after_equipment: vec![after_item],
            baseline: before_sim.evidence.result,
            candidate: after_sim.evidence.result,
        },
        runtime.provenance(),
        0,
    )?;
    Ok((evidence, presentation))
}

pub fn compare_strategies(
    trace_id: &str,
    baseline: &ScenarioSnapshotV1,
    runtime: &AgentRuntime,
    workspace: &EquipmentWorkspaceV1,
    budget: &mut ToolBudget,
) -> Result<
    (
        EvidenceEnvelopeV1<EquipmentComparisonEvidenceV1>,
        EquipmentComparisonPresentationV1,
    ),
    ToolError,
> {
    let (set_name, set_slots, set_items) = build_four_piece(runtime, workspace, false)
        .ok_or(ToolError::EquipmentStrategyUnavailable)?;
    let (qiegao_name, qiegao_slots, qiegao_items) = build_four_piece(runtime, workspace, true)
        .ok_or(ToolError::EquipmentStrategyUnavailable)?;
    let set_calc =
        runtime.calculate_equipment(&set_slots, workspace.stone_id, &baseline.simulation.talents);
    let qiegao_calc = runtime.calculate_equipment(
        &qiegao_slots,
        workspace.stone_id,
        &baseline.simulation.talents,
    );
    let set_snapshot = snapshot_with_build(baseline, runtime, &set_slots, &set_calc.raw)?;
    let qiegao_snapshot = snapshot_with_build(baseline, runtime, &qiegao_slots, &qiegao_calc.raw)?;
    let context = runtime.context();
    let set_sim = simulate_scenario(
        trace_id,
        &set_snapshot,
        &context,
        runtime.provenance(),
        budget,
    )?;
    let qiegao_sim = simulate_scenario(
        trace_id,
        &qiegao_snapshot,
        &context,
        runtime.provenance(),
        budget,
    )?;
    let before_dps = set_sim.evidence.result.dps;
    let after_dps = qiegao_sim.evidence.result.dps;
    let presentation = EquipmentComparisonPresentationV1 {
        title: "四件套 vs 四切糕".to_string(),
        position: "MULTI".to_string(),
        before_item: format!("四件套 · {set_name}"),
        after_item: format!("四切糕 · {qiegao_name}"),
        panel_rows: panel_rows(&set_calc, &qiegao_calc),
        before_dps,
        after_dps,
        dps_delta: after_dps - before_dps,
        dps_delta_percent: if before_dps.abs() > f64::EPSILON {
            (after_dps / before_dps - 1.0) * 100.0
        } else {
            0.0
        },
        source_label: workspace.source_label.clone(),
    };
    let evidence = EvidenceEnvelopeV1::new(
        trace_id,
        COMPARE_EQUIPMENT_STRATEGIES,
        &baseline.scenario_hash,
        serde_json::json!({"strategy":"four_set_vs_four_qiegao"}),
        EquipmentComparisonEvidenceV1 {
            comparison: presentation.clone(),
            before_equipment: set_items,
            after_equipment: qiegao_items,
            baseline: set_sim.evidence.result,
            candidate: qiegao_sim.evidence.result,
        },
        runtime.provenance(),
        0,
    )?;
    Ok((evidence, presentation))
}

fn build_four_piece(
    runtime: &AgentRuntime,
    workspace: &EquipmentWorkspaceV1,
    qiegao: bool,
) -> Option<(
    String,
    HashMap<String, equip::SlotConfig>,
    Vec<EquipmentItemSummaryV1>,
)> {
    let armor = ["HAT", "JACKET", "BELT", "WRIST", "BOTTOMS", "SHOES"];
    let mut groups = BTreeMap::<u32, Vec<(&str, &equip::EquipItem)>>::new();
    for item in runtime.equipment_items() {
        let Some(position) = subtype_position(item.sub_type) else {
            continue;
        };
        if !armor.contains(&position) || !workspace.slots.contains_key(position) || item.set_id == 0
        {
            continue;
        }
        if item.magic_type.contains("(PVP)") || item.magic_type.contains("(PVX)") {
            continue;
        }
        if !item_matches_mount(runtime.mount(), item) {
            continue;
        }
        let set_name = runtime.equipment_set_name(item.set_id).unwrap_or_default();
        if set_name.contains("切糕") != qiegao {
            continue;
        }
        groups
            .entry(item.set_id)
            .or_default()
            .push((position, item));
    }
    let mut candidates = groups
        .into_iter()
        .filter_map(|(set_id, entries)| {
            let mut best = BTreeMap::<String, &equip::EquipItem>::new();
            for (position, item) in entries {
                let replace = best
                    .get(position)
                    .is_none_or(|current| item.level > current.level);
                if replace {
                    best.insert(position.to_string(), item);
                }
            }
            if best.len() < 4 {
                return None;
            }
            let score = best.values().map(|item| item.level as u64).sum::<u64>();
            Some((score, set_id, best))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let (_, set_id, best) = candidates.into_iter().next()?;
    let mut choices = best.into_iter().collect::<Vec<_>>();
    choices.sort_by(|left, right| right.1.level.cmp(&left.1.level));
    choices.truncate(4);
    let mut slots = workspace.slots.clone();
    let mut summaries = Vec::new();
    for (position, item) in choices {
        let mut slot = slots.get(&position)?.clone();
        slot.equip_id = item.id;
        slots.insert(position.clone(), slot);
        summaries.push(item_summary_from_item(runtime, &position, item));
    }
    Some((
        runtime
            .equipment_set_name(set_id)
            .unwrap_or_else(|| format!("套装#{set_id}")),
        slots,
        summaries,
    ))
}

fn item_matches_mount(mount: Mount, item: &equip::EquipItem) -> bool {
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

fn snapshot_with_build(
    baseline: &ScenarioSnapshotV1,
    runtime: &AgentRuntime,
    slots: &HashMap<String, equip::SlotConfig>,
    raw: &equip::RawAttrs,
) -> Result<ScenarioSnapshotV1, ToolError> {
    let mut simulation: SimulateRequest = baseline.simulation.clone();
    simulation.haste_level = raw.haste_level.round().max(0.0) as u32;
    simulation.attributes = Some(Attributes {
        vitality: raw.vitality,
        li_dao: raw.strength,
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
        ..Attributes::default()
    });
    simulation.equipment = equipment_map(slots);
    ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), simulation)
        .map_err(ToolError::from)
}

fn equipment_map(slots: &HashMap<String, equip::SlotConfig>) -> HashMap<String, u32> {
    let mut result = HashMap::new();
    for (position, slot) in slots {
        if slot.equip_id > 0 {
            result.insert(position.clone(), slot.equip_id);
        }
        if slot.enchant_id > 0 {
            result.insert(format!("ENCHANT_{position}"), slot.enchant_id);
        }
    }
    result
}

fn item_summary(runtime: &AgentRuntime, position: &str, id: u32) -> Option<EquipmentItemSummaryV1> {
    let subtype = position_subtype(position)?;
    runtime
        .equipment_item(subtype, id)
        .map(|item| item_summary_from_item(runtime, position, item))
}

fn item_summary_from_item(
    runtime: &AgentRuntime,
    position: &str,
    item: &equip::EquipItem,
) -> EquipmentItemSummaryV1 {
    let set_name = (item.set_id > 0)
        .then(|| runtime.equipment_set_name(item.set_id))
        .flatten();
    let category = if set_name
        .as_deref()
        .is_some_and(|name| name.contains("切糕"))
    {
        "切糕"
    } else if item.set_id > 0 {
        "套装"
    } else if item.belong_school == "精简" {
        "精简"
    } else {
        "散件"
    };
    EquipmentItemSummaryV1 {
        position: position.to_string(),
        id: item.id,
        name: item.name.clone(),
        level: item.level,
        quality: item.quality,
        set_name,
        category: category.to_string(),
        attributes: item.attr_tags.iter().cloned().collect(),
    }
}

fn panel_map(calc: &equip::CalcResponse) -> BTreeMap<String, f64> {
    let p = &calc.panel;
    BTreeMap::from([
        ("score".into(), calc.score as f64),
        ("attack".into(), p.physics_attack_power),
        ("crit".into(), p.crit_rate * 100.0),
        ("crit_effect".into(), p.crit_effect * 100.0),
        ("overcome".into(), p.overcome_rate * 100.0),
        ("strain".into(), p.strain_rate * 100.0),
        ("haste".into(), p.haste_rate * 100.0),
        ("surplus".into(), p.surplus_value),
        ("agility".into(), p.agility),
        ("strength".into(), p.strength),
        ("vitality".into(), p.vitality),
    ])
}

fn panel_rows(
    before: &equip::CalcResponse,
    after: &equip::CalcResponse,
) -> Vec<EquipmentPanelRowV1> {
    let b = panel_map(before);
    let a = panel_map(after);
    let labels = [
        ("score", "装分", ""),
        ("attack", "外功攻击", ""),
        ("crit", "会心", "%"),
        ("crit_effect", "会心效果", "%"),
        ("overcome", "破防", "%"),
        ("strain", "无双", "%"),
        ("haste", "加速", "%"),
        ("surplus", "破招", ""),
        ("agility", "身法", ""),
        ("strength", "力道", ""),
    ];
    labels
        .into_iter()
        .map(|(key, label, unit)| {
            let before = *b.get(key).unwrap_or(&0.0);
            let after = *a.get(key).unwrap_or(&0.0);
            EquipmentPanelRowV1 {
                key: key.into(),
                label: label.into(),
                unit: unit.into(),
                before,
                after,
                delta: after - before,
            }
        })
        .collect()
}

fn position_subtype(position: &str) -> Option<u8> {
    Some(match position {
        "PRIMARY_WEAPON" => 0,
        "SECONDARY_WEAPON" => 1,
        "NECKLACE" => 4,
        "RING_1" | "RING_2" => 5,
        "PENDANT" => 7,
        "HAT" => 3,
        "JACKET" => 2,
        "BELT" => 6,
        "WRIST" => 10,
        "BOTTOMS" => 8,
        "SHOES" => 9,
        _ => return None,
    })
}
fn subtype_position(subtype: u8) -> Option<&'static str> {
    Some(match subtype {
        0 => "PRIMARY_WEAPON",
        1 => "SECONDARY_WEAPON",
        2 => "JACKET",
        3 => "HAT",
        4 => "NECKLACE",
        5 => "RING_1",
        6 => "BELT",
        7 => "PENDANT",
        8 => "BOTTOMS",
        9 => "SHOES",
        10 => "WRIST",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{position_subtype, subtype_position};

    #[test]
    fn equipment_positions_match_the_existing_equipment_page_contract() {
        let expected = [
            ("PRIMARY_WEAPON", 0),
            ("SECONDARY_WEAPON", 1),
            ("JACKET", 2),
            ("HAT", 3),
            ("NECKLACE", 4),
            ("RING_1", 5),
            ("BELT", 6),
            ("PENDANT", 7),
            ("BOTTOMS", 8),
            ("SHOES", 9),
            ("WRIST", 10),
        ];
        for (position, subtype) in expected {
            assert_eq!(position_subtype(position), Some(subtype));
            assert_eq!(subtype_position(subtype), Some(position));
        }
        assert_eq!(position_subtype("RING_2"), Some(5));
    }
}
