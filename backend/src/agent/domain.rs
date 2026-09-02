use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

use super::hash::canonical_sha256;
use super::report::EvidenceStore;
use super::ScenarioSnapshotV1;

pub const DOMAIN_CLAIM_SCHEMA_V1: &str = "agent-domain-claim/v1";
pub const ANALYSIS_PLAN_SCHEMA_V1: &str = "agent-analysis-plan/v1";
pub const EVIDENCE_PACK_SCHEMA_V1: &str = "agent-evidence-pack/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainClient {
    Flagship,
    Wujie,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisTaskType {
    BaselineAnalysis,
    RotationStallDiagnosis,
    HasteDecision,
    OrangeWeaponTiming,
    MacroAnalysis,
    EquipmentAnalysis,
    EncounterAdvice,
    MechanismExplanation,
    ReferenceLookup,
    GeneralAnalysis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainClaimType {
    OfficialChange,
    GameMechanic,
    MeasuredMechanic,
    DerivedFormula,
    PlayerPractice,
    OptimizationHypothesis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainAuthority {
    OfficialCurrentPatch,
    CurrentWhitepaper,
    CurrentMechanismTest,
    CurrentPractical,
    CurrentDerived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainConflictStatus {
    Clear,
    SupersedesOlder,
    UnresolvedInternal,
    ImplementationMismatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SimulatorSupport {
    Implemented,
    PartiallyObservable,
    Unsupported,
    KnowledgeOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainEntityV1 {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainScopeV1 {
    pub client: DomainClient,
    pub game_version: String,
    pub season: String,
    pub mount: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainClaimSourceV1 {
    pub document_id: String,
    pub document_hash: String,
    pub chunk_hash: String,
    pub title: String,
    pub heading: String,
    pub source_url: String,
    pub yuque_url: String,
    pub source_updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainVerificationV1 {
    pub simulator_support: SimulatorSupport,
    pub observable_fields: Vec<String>,
    pub applicable_tools: Vec<String>,
    pub boundary_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainClaimV1 {
    pub schema_version: String,
    pub claim_id: String,
    pub subject: DomainEntityV1,
    pub relation: String,
    pub object: DomainEntityV1,
    pub statement: String,
    pub claim_type: DomainClaimType,
    pub authority: DomainAuthority,
    pub scope: DomainScopeV1,
    pub conditions: Vec<String>,
    pub conflict_status: DomainConflictStatus,
    pub verification: DomainVerificationV1,
    pub source: DomainClaimSourceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainRelationV1 {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub claim_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisStageV1 {
    pub stage_id: String,
    pub label: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisPlaybookV1 {
    pub playbook_id: String,
    pub label: String,
    pub goal: String,
    pub required_dimensions: Vec<String>,
    pub optional_dimensions: Vec<String>,
    pub preferred_tools: Vec<String>,
    pub knowledge_search_hints: Vec<String>,
    pub forbidden_inferences: Vec<String>,
    pub stages: Vec<AnalysisStageV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisPlanV1 {
    pub schema_version: String,
    pub task_type: AnalysisTaskType,
    pub resolved_scope: DomainScopeV1,
    pub playbook: AnalysisPlaybookV1,
    pub routing_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgePrefetchV1 {
    pub query: String,
    pub version_scope: String,
    pub season: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSufficiency {
    Sufficient,
    Partial,
    Insufficient,
}

impl EvidenceSufficiency {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sufficient => "sufficient",
            Self::Partial => "partial",
            Self::Insufficient => "insufficient",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceCoverageV1 {
    pub required_dimensions: Vec<String>,
    pub satisfied_dimensions: Vec<String>,
    pub missing_dimensions: Vec<String>,
    pub sufficiency: EvidenceSufficiency,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidencePackV1 {
    pub schema_version: String,
    pub playbook_id: String,
    pub resolved_scope: DomainScopeV1,
    pub evidence_ids: Vec<String>,
    pub tool_evidence: Vec<String>,
    pub domain_claim_ids: Vec<String>,
    pub boundary_codes: Vec<String>,
    pub coverage: EvidenceCoverageV1,
    pub answer_constraints: Vec<String>,
}

pub struct DomainChunkContext<'a> {
    pub document_id: &'a str,
    pub title: &'a str,
    pub season: &'a str,
    pub heading: &'a str,
    pub text: &'a str,
    pub source_url: &'a str,
    pub yuque_url: &'a str,
    pub source_updated_at: &'a str,
    pub document_hash: &'a str,
    pub chunk_hash: &'a str,
}

struct ClaimSeed {
    id: &'static str,
    title_contains: &'static str,
    anchors: &'static [&'static str],
    subject_kind: &'static str,
    subject: &'static str,
    relation: &'static str,
    object_kind: &'static str,
    object: &'static str,
    statement: &'static str,
    claim_type: DomainClaimType,
    authority: DomainAuthority,
    conditions: &'static [&'static str],
    conflict_status: DomainConflictStatus,
    simulator_support: SimulatorSupport,
    observable_fields: &'static [&'static str],
    applicable_tools: &'static [&'static str],
    boundary_codes: &'static [&'static str],
}

const CLAIM_SEEDS: &[ClaimSeed] = &[
    ClaimSeed {
        id: "fs-yg-001",
        title_contains: "分山劲白皮书",
        anchors: &["盾击", "击破·援戈", "苍雪刀套路"],
        subject_kind: "skill",
        subject: "盾击",
        relation: "grants",
        object_kind: "resource",
        object: "援戈层数",
        statement: "盾击成功触发击破·援戈后获得援戈层数；苍雪刀招式消耗层数并附加援戈伤害。",
        claim_type: DomainClaimType::GameMechanic,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["暗影千机旗舰端分山劲", "选择援戈", "非侠士目标"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::Implemented,
        observable_fields: &["skills", "rage", "buff_coverage", "timeline"],
        applicable_tools: &["simulate_scenario", "analyze_timeline"],
        boundary_codes: &[],
    },
    ClaimSeed {
        id: "fs-yh-001",
        title_contains: "分山劲白皮书",
        anchors: &["获得9层", "麟光甲", "重置"],
        subject_kind: "skill",
        subject: "业火麟光",
        relation: "grants_and_resets",
        object_kind: "rotation_phase",
        object: "业火九刀",
        statement: "业火麟光获得9层麟光甲；苍雪刀消耗层数，每消耗3层可重置一次斩刀。",
        claim_type: DomainClaimType::GameMechanic,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["暗影千机旗舰端分山劲", "选择业火麟光"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::Implemented,
        observable_fields: &["skills", "buff_coverage", "timeline"],
        applicable_tools: &["simulate_scenario", "analyze_timeline"],
        boundary_codes: &[],
    },
    ClaimSeed {
        id: "fs-haste-001",
        title_contains: "分山劲白皮书",
        anchors: &["14156加速", "多覆盖一个技能", "单走一个绝刀"],
        subject_kind: "haste_band",
        subject: "14156",
        relation: "enables",
        object_kind: "rotation_phase",
        object: "额外盾击与单走绝刀",
        statement: "14156加速会改变盾系填充和血怒覆盖，并可在资源允许时加入单走绝刀。",
        claim_type: DomainClaimType::PlayerPractice,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["当前分山循环", "延迟与手动操作允许"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::Implemented,
        observable_fields: &["skills", "rage", "buff_coverage"],
        applicable_tools: &["compare_scenarios", "analyze_timeline"],
        boundary_codes: &[],
    },
    ClaimSeed {
        id: "fs-haste-002",
        title_contains: "分山劲白皮书",
        anchors: &["水特效一键宏"],
        subject_kind: "weapon_and_input",
        subject: "水特效一键宏",
        relation: "recommended_for",
        object_kind: "haste_band",
        object: "206",
        statement: "水特效一键宏因难以利用单走绝刀且堆加速成本较高，可以考虑206加速。",
        claim_type: DomainClaimType::PlayerPractice,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["水特效武器", "一键宏"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::PartiallyObservable,
        observable_fields: &["macro", "skills", "rage"],
        applicable_tools: &["simulate_scenario", "compare_scenarios"],
        boundary_codes: &["operation_method_is_user_context"],
    },
    ClaimSeed {
        id: "fs-haste-003",
        title_contains: "分山劲白皮书",
        anchors: &["30158", "14156", "几乎无差距"],
        subject_kind: "haste_band",
        subject: "30158",
        relation: "competes_with",
        object_kind: "haste_band",
        object: "14156",
        statement: "橙武四段加速与14156的木桩差距很小，主要取舍是血怒容错、延迟区间与实现难度。",
        claim_type: DomainClaimType::PlayerPractice,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["大橙武", "中低延迟", "同口径配装"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::Implemented,
        observable_fields: &["skills", "buff_coverage", "dps"],
        applicable_tools: &["compare_scenarios", "analyze_timeline"],
        boundary_codes: &[],
    },
    ClaimSeed {
        id: "fs-cw-002",
        title_contains: "分山劲白皮书",
        anchors: &["天下宏愿", "持续伤害效果", "最多叠加3层"],
        subject_kind: "equipment_effect",
        subject: "天下宏愿",
        relation: "triggers",
        object_kind: "periodic_effect",
        object: "裂伤",
        statement: "天下宏愿期间的绝刀会添加持续伤害效果，最多叠加3层。",
        claim_type: DomainClaimType::GameMechanic,
        authority: DomainAuthority::CurrentWhitepaper,
        conditions: &["2026-07-13后正式服", "装备大橙武"],
        conflict_status: DomainConflictStatus::ImplementationMismatch,
        simulator_support: SimulatorSupport::Unsupported,
        observable_fields: &[],
        applicable_tools: &[],
        boundary_codes: &["orange_weapon_dot_not_implemented"],
    },
    ClaimSeed {
        id: "fs-cw-002-patch",
        title_contains: "苍云历次技改汇总",
        anchors: &["分山劲稀世神兵特殊效果", "持续伤害效果", "最多叠加3层"],
        subject_kind: "equipment_effect",
        subject: "天下宏愿",
        relation: "triggers",
        object_kind: "periodic_effect",
        object: "裂伤",
        statement: "后续正式武学调整为天下宏愿绝刀新增持续伤害效果，最多叠加3层。",
        claim_type: DomainClaimType::OfficialChange,
        authority: DomainAuthority::OfficialCurrentPatch,
        conditions: &["2026-07-13后正式服", "装备大橙武"],
        conflict_status: DomainConflictStatus::ImplementationMismatch,
        simulator_support: SimulatorSupport::Unsupported,
        observable_fields: &[],
        applicable_tools: &[],
        boundary_codes: &["orange_weapon_dot_not_implemented"],
    },
    ClaimSeed {
        id: "fs-stance-001",
        title_contains: "苍云进阶机制",
        anchors: &["盾飞", "刀魂", "0.625秒"],
        subject_kind: "skill",
        subject: "盾飞",
        relation: "delays_stance",
        object_kind: "stance",
        object: "刀魂",
        statement: "盾飞后的刀魂刷新存在客户端帧延迟，首个刀系技能过早释放可能无法获得刀魂覆盖。",
        claim_type: DomainClaimType::MeasuredMechanic,
        authority: DomainAuthority::CurrentMechanismTest,
        conditions: &["旗舰端", "真实客户端帧时序"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::PartiallyObservable,
        observable_fields: &["timeline", "buff_coverage"],
        applicable_tools: &["analyze_timeline"],
        boundary_codes: &["client_frame_timing_not_fully_modeled"],
    },
    ClaimSeed {
        id: "fs-charge-001",
        title_contains: "苍云进阶机制",
        anchors: &["实际不触发突进的距离", "MAX", "突进保护距离"],
        subject_kind: "movement_mechanic",
        subject: "实际不触发突进距离",
        relation: "derived_from",
        object_kind: "distance_rule",
        object: "MAX（突进保护距离，4尺）",
        statement: "实际不触发突进的距离取突进保护距离与4尺中的较大值。",
        claim_type: DomainClaimType::MeasuredMechanic,
        authority: DomainAuthority::CurrentMechanismTest,
        conditions: &["旗舰端", "适用于突进技能"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::KnowledgeOnly,
        observable_fields: &[],
        applicable_tools: &[],
        boundary_codes: &["target_distance_compensation_not_modeled"],
    },
    ClaimSeed {
        id: "fs-latency-001",
        title_contains: "低延迟",
        anchors: &["按键", "网络", "FPS"],
        subject_kind: "environment",
        subject: "综合延迟",
        relation: "reduces",
        object_kind: "rotation_quality",
        object: "有效技能数量与覆盖",
        statement: "按键、机器帧率、网络和服务器处理造成的延迟会累积，并影响循环技能数与增益覆盖。",
        claim_type: DomainClaimType::MeasuredMechanic,
        authority: DomainAuthority::CurrentMechanismTest,
        conditions: &["真实客户端环境"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::PartiallyObservable,
        observable_fields: &["configured_network_delay", "skills", "buff_coverage"],
        applicable_tools: &["compare_scenarios", "analyze_timeline"],
        boundary_codes: &["real_latency_sources_not_fully_modeled"],
    },
    ClaimSeed {
        id: "fs-raid-001",
        title_contains: "阆风悬城",
        anchors: &["业火", "斩", "阶段"],
        subject_kind: "encounter_window",
        subject: "阆风悬城阶段轴",
        relation: "competes_with",
        object_kind: "rotation_phase",
        object: "业火爆发",
        statement: "副本阶段、转火和目标窗口会改变业火与斩刀爆发的实际交法。",
        claim_type: DomainClaimType::PlayerPractice,
        authority: DomainAuthority::CurrentPractical,
        conditions: &["阆风悬城指定首领与阶段"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::KnowledgeOnly,
        observable_fields: &[],
        applicable_tools: &[],
        boundary_codes: &["encounter_timeline_not_modeled"],
    },
    ClaimSeed {
        id: "fs-parry-001",
        title_contains: "盾压重置率推导",
        anchors: &["3.154", "招架"],
        subject_kind: "derived_formula",
        subject: "盾压重置率",
        relation: "derived_from",
        object_kind: "sample",
        object: "招架等级样本",
        statement: "盾压重置率公式来自面板样本反推，不是官方公开公式。",
        claim_type: DomainClaimType::DerivedFormula,
        authority: DomainAuthority::CurrentDerived,
        conditions: &["130级", "作者样本范围"],
        conflict_status: DomainConflictStatus::Clear,
        simulator_support: SimulatorSupport::PartiallyObservable,
        observable_fields: &["parry_configuration"],
        applicable_tools: &["simulate_scenario"],
        boundary_codes: &["derived_formula_not_official"],
    },
];

pub fn derive_domain_claims(context: DomainChunkContext<'_>) -> Vec<DomainClaimV1> {
    if context.season != "暗影千机（2026）" || context.title.contains('悟') {
        return Vec::new();
    }
    let searchable = format!("{}\n{}", context.heading, context.text);
    CLAIM_SEEDS
        .iter()
        .filter(|seed| context.title.contains(seed.title_contains))
        .filter(|seed| {
            seed.anchors
                .iter()
                .all(|anchor| searchable.contains(anchor))
        })
        .map(|seed| DomainClaimV1 {
            schema_version: DOMAIN_CLAIM_SCHEMA_V1.to_string(),
            claim_id: seed.id.to_string(),
            subject: DomainEntityV1 {
                kind: seed.subject_kind.to_string(),
                name: seed.subject.to_string(),
            },
            relation: seed.relation.to_string(),
            object: DomainEntityV1 {
                kind: seed.object_kind.to_string(),
                name: seed.object.to_string(),
            },
            statement: seed.statement.to_string(),
            claim_type: seed.claim_type,
            authority: seed.authority,
            scope: DomainScopeV1 {
                client: DomainClient::Flagship,
                game_version: "2026_04_anying_qianji".to_string(),
                season: context.season.to_string(),
                mount: "fenshanjin".to_string(),
                mode: "pve".to_string(),
            },
            conditions: seed
                .conditions
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            conflict_status: seed.conflict_status,
            verification: DomainVerificationV1 {
                simulator_support: seed.simulator_support,
                observable_fields: seed
                    .observable_fields
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                applicable_tools: seed
                    .applicable_tools
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                boundary_codes: seed
                    .boundary_codes
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            },
            source: DomainClaimSourceV1 {
                document_id: context.document_id.to_string(),
                document_hash: context.document_hash.to_string(),
                chunk_hash: context.chunk_hash.to_string(),
                title: context.title.to_string(),
                heading: context.heading.to_string(),
                source_url: context.source_url.to_string(),
                yuque_url: context.yuque_url.to_string(),
                source_updated_at: context.source_updated_at.to_string(),
            },
        })
        .collect()
}

pub fn domain_relations(claims: &[DomainClaimV1]) -> Vec<DomainRelationV1> {
    claims
        .iter()
        .map(|claim| DomainRelationV1 {
            from: claim.subject.name.clone(),
            relation: claim.relation.clone(),
            to: claim.object.name.clone(),
            claim_id: claim.claim_id.clone(),
        })
        .collect()
}

pub fn domain_index_hash<'a>(claims: impl Iterator<Item = &'a DomainClaimV1>) -> String {
    let mut identities = claims
        .map(|claim| {
            (
                claim.claim_id.as_str(),
                claim.source.document_hash.as_str(),
                claim.source.chunk_hash.as_str(),
            )
        })
        .collect::<Vec<_>>();
    identities.sort_unstable();
    canonical_sha256(&identities).unwrap_or_else(|_| "unavailable".to_string())
}

pub fn select_analysis_plan(question: &str, scenario: &ScenarioSnapshotV1) -> AnalysisPlanV1 {
    let normalized = question.to_lowercase();
    let client = if contains_any(&normalized, &["无界", "分山劲·悟", "分山劲・悟", "wujie"])
    {
        DomainClient::Wujie
    } else {
        DomainClient::Flagship
    };
    let (task_type, routing_signals) = classify_task(&normalized);
    let scope = DomainScopeV1 {
        client,
        game_version: scenario.game_version.clone(),
        season: season_for_version(&scenario.game_version).to_string(),
        mount: if client == DomainClient::Wujie {
            if normalized.contains("铁骨") {
                "tieguyi_wu"
            } else {
                "fenshanjin_wu"
            }
        } else if normalized.contains("铁骨") {
            "tieguyi"
        } else if normalized.contains("分山") {
            "fenshanjin"
        } else {
            scenario.mount.as_str()
        }
        .to_string(),
        mode: if contains_any(&normalized, &["pvp", "竞技场", "战场"]) {
            "pvp"
        } else {
            "pve"
        }
        .to_string(),
    };
    let mut plan = AnalysisPlanV1 {
        schema_version: ANALYSIS_PLAN_SCHEMA_V1.to_string(),
        task_type,
        resolved_scope: scope,
        playbook: playbook(task_type, client),
        routing_signals,
    };
    let has_macro_input = scenario
        .simulation
        .macro_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    let has_rotation_input = scenario.simulation.sequence.len() >= 6 || has_macro_input;
    if task_type == AnalysisTaskType::HasteDecision && !has_rotation_input {
        demote_required_dimension(&mut plan.playbook, "candidate_comparison");
        retain_knowledge_tools(&mut plan.playbook);
        plan.routing_signals
            .push("scenario_too_short_for_meaningful_ab".to_string());
    }
    if task_type == AnalysisTaskType::OrangeWeaponTiming && scenario.simulation.equipment.is_empty()
    {
        demote_required_dimension(&mut plan.playbook, "timeline");
        demote_required_dimension(&mut plan.playbook, "candidate_comparison");
        retain_knowledge_tools(&mut plan.playbook);
        plan.routing_signals
            .push("scenario_has_no_equipment_context".to_string());
    }
    apply_equipment_contract(&mut plan, &normalized);
    apply_saved_artifact_contract(&mut plan, &normalized);
    if !plan
        .routing_signals
        .iter()
        .any(|signal| signal == "saved_artifact_access_requested")
    {
        apply_rotation_diagnosis_contract(&mut plan, &normalized, scenario);
    }
    plan
}

pub(crate) fn equipment_strategy_comparison_requested(question: &str) -> bool {
    let normalized = question.to_lowercase();
    let mentions_set = contains_any(&normalized, &["四件套", "4件套", "套装四件"]);
    let mentions_qiegao = contains_any(&normalized, &["四切糕", "4切糕", "切糕"]);
    let asks_comparison = contains_any(
        &normalized,
        &["还是", "哪个好", "对比", "比较", "取舍", "差异", " vs ", "vs."],
    );
    mentions_set && mentions_qiegao && asks_comparison
}

pub(crate) fn equipment_focused_comparison_requested(question: &str) -> bool {
    let normalized = question.to_lowercase();
    !equipment_strategy_comparison_requested(&normalized)
        && (contains_any(&normalized, &["换成", "换掉", "替换", "更换", "候选装备"])
            || (contains_any(&normalized, &["这件", "那件", "当前装备", "装备"])
                && contains_any(
                    &normalized,
                    &["换", "对比", "比较", "哪个好", "怎么样", "差异"],
                )))
}

fn apply_equipment_contract(plan: &mut AnalysisPlanV1, normalized_question: &str) {
    if plan.task_type != AnalysisTaskType::EquipmentAnalysis {
        return;
    }

    // Reading the current build is a complete, useful equipment task by itself.
    // A candidate experiment is required only when the user actually asks for one.
    demote_required_dimension(&mut plan.playbook, "candidate_comparison");
    let strategy = equipment_strategy_comparison_requested(normalized_question);
    let focused = equipment_focused_comparison_requested(normalized_question);
    let needs_versioned_explanation = contains_any(
        normalized_question,
        &["特效", "适配", "收益", "取舍", "为什么", "机制"],
    );
    plan.playbook.preferred_tools.retain(|tool| match tool.as_str() {
        "compare_equipment_strategies" => strategy,
        "compare_focused_equipment" => focused,
        "search_equipment_catalog" => strategy || focused,
        _ => true,
    });
    if needs_versioned_explanation || strategy {
        plan.playbook
            .optional_dimensions
            .retain(|dimension| dimension != "versioned_knowledge");
        if !plan
            .playbook
            .required_dimensions
            .iter()
            .any(|dimension| dimension == "versioned_knowledge")
        {
            plan.playbook
                .required_dimensions
                .push("versioned_knowledge".to_string());
        }
        plan.routing_signals
            .push("equipment_versioned_explanation_requested".to_string());
    }

    if strategy || focused {
        plan.playbook
            .required_dimensions
            .push("candidate_comparison".to_string());
        plan.routing_signals.push(
            if strategy {
                "equipment_strategy_comparison_requested"
            } else {
                "equipment_focused_comparison_requested"
            }
            .to_string(),
        );
    } else {
        plan.playbook.goal =
            "读取当前配装、面板、套装与装备特效，解释属性结构及其证据边界。".to_string();
        if let Some(stage) = plan
            .playbook
            .stages
            .iter_mut()
            .find(|stage| stage.stage_id == "compare")
        {
            stage.label = "解释当前配装".to_string();
            stage.purpose = "结合面板、套装组成与当前循环说明已知适配关系，不虚构换装收益。"
                .to_string();
        }
        plan.routing_signals
            .push("equipment_current_build_inspection".to_string());
    }
}

fn apply_saved_artifact_contract(plan: &mut AnalysisPlanV1, normalized_question: &str) {
    let refers_to_saved_data = contains_any(
        normalized_question,
        &[
            "我保存的",
            "已保存",
            "保存的",
            "存档",
            "命名宏",
            "宏配置",
            "战斗广场",
            "广场方案",
        ],
    );
    if !refers_to_saved_data || plan.resolved_scope.client != DomainClient::Flagship {
        return;
    }
    if !plan
        .playbook
        .required_dimensions
        .iter()
        .any(|dimension| dimension == "saved_artifacts")
    {
        plan.playbook
            .required_dimensions
            .push("saved_artifacts".to_string());
    }
    plan.playbook
        .required_dimensions
        .retain(|dimension| dimension != "macro_context");
    for tool in ["list_saved_artifacts", "read_saved_artifact"] {
        if !plan
            .playbook
            .preferred_tools
            .iter()
            .any(|preferred| preferred == tool)
        {
            plan.playbook.preferred_tools.push(tool.to_string());
        }
    }
    let asks_to_compare = contains_any(
        normalized_question,
        &["对比", "比较", "优缺点", "差异", "哪个好", "哪套"],
    );
    if asks_to_compare {
        let comparison_tools: &[&str] = if contains_any(normalized_question, &["宏", "一键宏"])
        {
            &["compare_saved_macros"]
        } else if contains_any(
            normalized_question,
            &["循环", "战斗广场", "广场方案", "配装方案"],
        ) {
            &["compare_saved_scenarios"]
        } else {
            // A display name does not necessarily reveal its artifact kind.
            // Keep both typed comparators available until catalog evidence does.
            &["compare_saved_macros", "compare_saved_scenarios"]
        };
        for comparison_tool in comparison_tools {
            if !plan
                .playbook
                .preferred_tools
                .iter()
                .any(|preferred| preferred == comparison_tool)
            {
                plan.playbook
                    .preferred_tools
                    .push((*comparison_tool).to_string());
            }
        }
        if !plan
            .playbook
            .required_dimensions
            .iter()
            .any(|dimension| dimension == "candidate_comparison")
        {
            plan.playbook
                .required_dimensions
                .push("candidate_comparison".to_string());
        }
    }
    if !plan
        .playbook
        .stages
        .iter()
        .any(|stage| stage.stage_id == "saved")
    {
        let index = usize::from(!plan.playbook.stages.is_empty());
        plan.playbook.stages.insert(
            index,
            AnalysisStageV1 {
                stage_id: "saved".to_string(),
                label: "定位已保存资料".to_string(),
                purpose: "按显示名称列出候选，用不透明资源标识精确读取或对比。".to_string(),
            },
        );
    }
    plan.playbook
        .forbidden_inferences
        .push("名称存在多个匹配时必须列出候选并让用户消歧，不得静默选择".to_string());
    plan.routing_signals
        .push("saved_artifact_access_requested".to_string());
}

fn apply_rotation_diagnosis_contract(
    plan: &mut AnalysisPlanV1,
    normalized_question: &str,
    scenario: &ScenarioSnapshotV1,
) {
    let is_macro = scenario
        .simulation
        .macro_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    let has_any_rotation_input = !scenario.simulation.sequence.is_empty() || is_macro;
    if plan.resolved_scope.client != DomainClient::Flagship
        || !matches!(
            plan.task_type,
            AnalysisTaskType::BaselineAnalysis
                | AnalysisTaskType::RotationStallDiagnosis
                | AnalysisTaskType::MacroAnalysis
        )
        || !has_any_rotation_input
    {
        return;
    }

    plan.playbook
        .required_dimensions
        .retain(|dimension| dimension != "macro_context");
    for dimension in ["rotation_input", "timeline", "rotation_diagnosis"] {
        if !plan
            .playbook
            .required_dimensions
            .iter()
            .any(|value| value == dimension)
        {
            plan.playbook
                .required_dimensions
                .push(dimension.to_string());
        }
    }
    if !plan
        .playbook
        .preferred_tools
        .iter()
        .any(|tool| tool == "analyze_timeline")
    {
        plan.playbook
            .preferred_tools
            .push("analyze_timeline".to_string());
    }
    let has_meaningful_rotation = scenario.simulation.sequence.len() >= 6 || is_macro;
    if has_meaningful_rotation
        && !plan
            .playbook
            .preferred_tools
            .iter()
            .any(|tool| tool == "compare_scenarios")
    {
        plan.playbook
            .preferred_tools
            .push("compare_scenarios".to_string());
    }
    if is_macro {
        plan.routing_signals
            .push("rotation_input_macro".to_string());
        plan.playbook
            .knowledge_search_hints
            .push("当前分山宏语句 条件阈值 技能顺序 延迟适配 循环漏洞".to_string());
        plan.playbook
            .forbidden_inferences
            .push("宏改法必须指出当前原句并同时引用当前攻略与本轮执行证据".to_string());
    } else {
        plan.routing_signals
            .push("rotation_input_manual_sequence".to_string());
        plan.playbook
            .knowledge_search_hints
            .push("当前分山手动循环 技能顺序 操作节点 怒气覆盖 循环漏洞".to_string());
        plan.playbook.forbidden_inferences.push(
            "手动改法必须指出序列技能或时间轴操作点并同时引用当前攻略与本轮执行证据".to_string(),
        );
    }
    plan.routing_signals
        .push("rotation_diagnosis_first".to_string());
    let mentions_candidate = contains_any(
        normalized_question,
        &[
            "修改",
            "改进",
            "优化",
            "漏洞",
            "方案",
            "怎么改",
            "具体宏语句",
            "对照",
            "对比候选",
            "对比",
            "比较",
            "还是",
            "哪个好",
        ],
    );
    let negates_candidate = contains_any(
        normalized_question,
        &[
            "不要改",
            "不改宏",
            "无需改",
            "不需要改",
            "先不改",
            "暂不改",
            "不要在诊断前直接给修改方案",
            "不要直接给修改方案",
        ],
    );
    let explicitly_resumes_candidate = contains_any(
        normalized_question,
        &[
            "再给修改",
            "然后给修改",
            "之后给修改",
            "诊断后给修改",
            "诊断完给修改",
            "再优化",
            "然后优化",
            "之后优化",
        ],
    );
    let asks_for_candidate =
        mentions_candidate && (!negates_candidate || explicitly_resumes_candidate);
    if asks_for_candidate
        && !plan
            .playbook
            .required_dimensions
            .iter()
            .any(|dimension| dimension == "candidate_comparison")
    {
        plan.playbook
            .required_dimensions
            .push("candidate_comparison".to_string());
        plan.routing_signals
            .push("candidate_comparison_explicitly_requested".to_string());
    }
}

/// Resolve a short follow-up against the last server-selected playbook without
/// trusting model prose as routing input. A self-contained current question always wins.
pub fn select_analysis_plan_with_history(
    question: &str,
    prior_playbook_id: Option<&str>,
    scenario: &ScenarioSnapshotV1,
) -> AnalysisPlanV1 {
    let mut plan = select_analysis_plan(question, scenario);
    if plan.task_type != AnalysisTaskType::GeneralAnalysis {
        return plan;
    }
    let Some(task_type) = prior_playbook_id.and_then(task_type_for_playbook) else {
        return plan;
    };
    plan.task_type = task_type;
    plan.playbook = playbook(task_type, plan.resolved_scope.client);
    plan.routing_signals
        .push("inherited_session_playbook".to_string());
    apply_equipment_contract(&mut plan, &question.to_lowercase());
    apply_saved_artifact_contract(&mut plan, &question.to_lowercase());
    if !plan
        .routing_signals
        .iter()
        .any(|signal| signal == "saved_artifact_access_requested")
    {
        apply_rotation_diagnosis_contract(&mut plan, &question.to_lowercase(), scenario);
    }
    plan
}

fn task_type_for_playbook(playbook_id: &str) -> Option<AnalysisTaskType> {
    Some(match playbook_id {
        "current_rotation_baseline" => AnalysisTaskType::BaselineAnalysis,
        "rotation_stall_diagnosis" => AnalysisTaskType::RotationStallDiagnosis,
        "haste_band_decision" => AnalysisTaskType::HasteDecision,
        "orange_weapon_timing" => AnalysisTaskType::OrangeWeaponTiming,
        "macro_and_manual_analysis" => AnalysisTaskType::MacroAnalysis,
        "equipment_build_analysis" => AnalysisTaskType::EquipmentAnalysis,
        "encounter_advice" => AnalysisTaskType::EncounterAdvice,
        "mechanism_explanation" => AnalysisTaskType::MechanismExplanation,
        "reference_lookup" => AnalysisTaskType::ReferenceLookup,
        "general_grounded_analysis" => AnalysisTaskType::GeneralAnalysis,
        _ => return None,
    })
}

fn demote_required_dimension(playbook: &mut AnalysisPlaybookV1, dimension: &str) {
    playbook
        .required_dimensions
        .retain(|required| required != dimension);
    if !playbook
        .optional_dimensions
        .iter()
        .any(|optional| optional == dimension)
    {
        playbook.optional_dimensions.push(dimension.to_string());
    }
}

fn retain_knowledge_tools(playbook: &mut AnalysisPlaybookV1) {
    playbook.preferred_tools.retain(|tool| {
        matches!(
            tool.as_str(),
            "get_current_scenario" | "search_knowledge_base"
        )
    });
}

/// Builds one high-recall, server-owned retrieval query from the selected playbook.
/// The model may refine it once, but it cannot lose the first domain evidence pass
/// through a malformed or overly generic tool query.
pub fn knowledge_prefetch(plan: &AnalysisPlanV1, question: &str) -> Option<KnowledgePrefetchV1> {
    if plan.task_type == AnalysisTaskType::ReferenceLookup {
        return None;
    }
    if !plan
        .playbook
        .required_dimensions
        .iter()
        .any(|dimension| dimension == "versioned_knowledge")
    {
        return None;
    }
    let normalized = question.to_lowercase();
    if contains_any(
        &normalized,
        &[
            "山海源流",
            "太极秘录",
            "雾海寻龙",
            "历史版本",
            "2024",
            "2025",
        ],
    ) {
        return None;
    }
    let question_prefix = question.chars().take(180).collect::<String>();
    let domain_terms = match (plan.task_type, plan.resolved_scope.client) {
        (AnalysisTaskType::ReferenceLookup, _) => unreachable!("reference lookups are model-led"),
        (_, DomainClient::Wujie) => "无界 分山劲·悟 手打血劫循环 盾飞 劫刀×9 补流血 空白期",
        (AnalysisTaskType::BaselineAnalysis, _) => {
            "旗舰端 分山劲白皮书 当前循环 斩刀 绝刀 盾击 援戈 业火"
        }
        (AnalysisTaskType::RotationStallDiagnosis, _) => {
            "旗舰端 分山劲 循环 盾击 斩刀 业火 援戈 怒气 血怒"
        }
        (AnalysisTaskType::HasteDecision, _) => {
            "旗舰端 分山劲 206 14156 30158 加速 水特效 一键宏 单走绝刀"
        }
        (AnalysisTaskType::OrangeWeaponTiming, _) => {
            "旗舰端 分山劲 天下宏愿 裂伤 绝刀 业火 斩刀 血怒 7月13日"
        }
        (AnalysisTaskType::MacroAnalysis, _) => {
            "旗舰端 分山劲 一键宏 判定 延迟 单走绝刀 援戈 手动循环"
        }
        (AnalysisTaskType::EquipmentAnalysis, _) => {
            "旗舰端 分山劲 配装 套装 切糕 属性收益"
        }
        (AnalysisTaskType::EncounterAdvice, _) if normalized.contains("老四") => {
            "阆风悬城 老四 提前倒数10秒 开业火 第二次斩 破 卡轴"
        }
        (AnalysisTaskType::EncounterAdvice, _) => {
            "暗影千机 英雄挑战 阆风悬城 分山实战 业火 阶段 转火 无敌"
        }
        (AnalysisTaskType::MechanismExplanation, _) => {
            "旗舰端 分山劲 机制 公式 实测 推导 盾压重置率 招架 3.154"
        }
        (AnalysisTaskType::GeneralAnalysis, _) => "旗舰端 分山劲 当前版本 白皮书",
    };
    let rotation_terms = if plan
        .routing_signals
        .iter()
        .any(|signal| signal == "rotation_input_macro")
    {
        "宏语句 条件阈值 延迟适配 循环漏洞"
    } else if plan
        .routing_signals
        .iter()
        .any(|signal| signal == "rotation_input_manual_sequence")
    {
        "手动循环 技能顺序 操作节点 循环漏洞"
    } else {
        ""
    };
    Some(KnowledgePrefetchV1 {
        query: format!("{question_prefix} {domain_terms} {rotation_terms}")
            .trim()
            .to_string(),
        version_scope: "current_only".to_string(),
        season: None,
        category: None,
    })
}

fn classify_task(question: &str) -> (AnalysisTaskType, Vec<String>) {
    let routes: &[(AnalysisTaskType, &[&str])] = &[
        (
            AnalysisTaskType::ReferenceLookup,
            &["是谁", "作者", "名字", "昵称"],
        ),
        (
            AnalysisTaskType::EncounterAdvice,
            &[
                "副本",
                "boss",
                "首领",
                "阆风",
                "千机源枢",
                "老一",
                "老二",
                "老三",
                "老四",
            ],
        ),
        (
            AnalysisTaskType::OrangeWeaponTiming,
            &["橙武", "天下宏愿", "裂伤"],
        ),
        (
            AnalysisTaskType::EquipmentAnalysis,
            &["装备", "配装", "换这件", "换那件", "四件套", "4件套", "四切糕", "4切糕", "切糕", "套装"],
        ),
        (
            AnalysisTaskType::HasteDecision,
            &["加速", "14156", "30158", "206档", "206 和", "206和"],
        ),
        (AnalysisTaskType::MacroAnalysis, &["一键宏", "宏", "macro"]),
        (
            AnalysisTaskType::RotationStallDiagnosis,
            &[
                "空转",
                "断档",
                "卡住",
                "没技能",
                "等cd",
                "等 cd",
                "停手",
                "断流血",
                "循环漏洞",
                "循环问题",
            ],
        ),
        (
            AnalysisTaskType::BaselineAnalysis,
            &[
                "输出基线",
                "当前循环",
                "基线",
                "伤害构成",
                "输出分析",
                "调优",
                "优化",
            ],
        ),
        (
            AnalysisTaskType::MechanismExplanation,
            &["机制", "怎么算", "公式", "系数", "重置率", "为什么"],
        ),
    ];
    for (task, signals) in routes {
        let matched = signals
            .iter()
            .filter(|signal| question.contains(**signal))
            .map(|signal| (*signal).to_string())
            .collect::<Vec<_>>();
        if !matched.is_empty() {
            return (*task, matched);
        }
    }
    (AnalysisTaskType::GeneralAnalysis, Vec::new())
}

fn playbook(task: AnalysisTaskType, client: DomainClient) -> AnalysisPlaybookV1 {
    let (id, label, goal, required, optional, tools, hints, forbidden, stages): (
        &str,
        &str,
        &str,
        &[&str],
        &[&str],
        &[&str],
        &[&str],
        &[&str],
        &[(&str, &str, &str)],
    ) = match task {
        AnalysisTaskType::BaselineAnalysis => (
            "current_rotation_baseline",
            "当前循环输出基线",
            "先说明当前输出结构，再把资源、覆盖和稳定性拆成可观察事实与待验证假设。",
            &[
                "scope",
                "scenario",
                "baseline_metrics",
                "timeline",
                "versioned_knowledge",
            ],
            &[],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "analyze_timeline",
                "simulate_scenario",
            ],
            &["当前分山循环 输出结构", "高质量技能 血怒 援戈 业火"],
            &[
                "不能仅凭伤害占比断言循环错误",
                "未观测到的覆盖、漂移或溢出只能作为假设",
            ],
            &[
                (
                    "scope",
                    "锁定分析口径",
                    "确认客户端、版本、心法和冻结场景。",
                ),
                (
                    "baseline",
                    "建立输出基线",
                    "取得总量、主要伤害来源与技能数量。",
                ),
                (
                    "quality",
                    "检查循环质量",
                    "只在证据可见时检查资源、增益覆盖和时间稳定性。",
                ),
                (
                    "decision",
                    "形成最小结论",
                    "区分已测事实、机制解释与下一步实验。",
                ),
            ],
        ),
        AnalysisTaskType::RotationStallDiagnosis => (
            "rotation_stall_diagnosis",
            "循环空转诊断",
            "先定位空档，再交叉检查冷却、资源、姿态和关键增益，最后提出单变量验证。",
            &["scope", "scenario", "timeline", "versioned_knowledge"],
            &["baseline_metrics"],
            &[
                "get_current_scenario",
                "analyze_timeline",
                "search_knowledge_base",
                "compare_scenarios",
            ],
            &["盾击 斩刀 业火 循环 空转", "援戈 怒气 血怒 时间轴"],
            &[
                "怒气触顶样本不等于已测得损失怒气",
                "时间相关不自动证明因果",
                "木桩时间线不能证明真实网络或按键故障",
            ],
            &[
                (
                    "scope",
                    "锁定循环口径",
                    "读取冻结场景，确认版本、心法与输入方式。",
                ),
                ("locate", "定位空档", "检查空档发生位置及其前后技能。"),
                (
                    "cross_check",
                    "交叉检查原因",
                    "核对主动冷却等待、怒气、姿态和可见增益。",
                ),
                (
                    "experiment",
                    "收束为验证实验",
                    "无法证明因果时只提出一个变量的对照实验。",
                ),
            ],
        ),
        AnalysisTaskType::HasteDecision => (
            "haste_band_decision",
            "加速档决策",
            "把武器、操作方式、延迟和属性成本作为条件，对同口径候选比较产出与稳健性。",
            &[
                "scope",
                "scenario",
                "versioned_knowledge",
                "candidate_comparison",
            ],
            &["timeline", "latency_context", "weapon_context"],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "compare_scenarios",
                "analyze_timeline",
            ],
            &[
                "206 14156 30158 加速 武器 宏",
                "加速 血怒 覆盖 单走绝刀 延迟",
            ],
            &[
                "不能把攻略推荐写成无条件最优",
                "不得同时改变多项配装后把差异归因于加速",
                "小差距需要讨论属性成本与操作稳健性",
            ],
            &[
                (
                    "scope",
                    "补齐决策条件",
                    "确认武器、手动或宏、当前属性、延迟和环境。",
                ),
                (
                    "knowledge",
                    "读取档位机制",
                    "取得当前版本各加速档改变循环的条件化资料。",
                ),
                (
                    "compare",
                    "执行同口径对比",
                    "只改变声明的加速或延迟变量，比较产出与覆盖。",
                ),
                (
                    "tradeoff",
                    "给出条件化选择",
                    "综合收益、属性成本、延迟容错和实现难度。",
                ),
            ],
        ),
        AnalysisTaskType::OrangeWeaponTiming => (
            "orange_weapon_timing",
            "橙武爆发时机",
            "先确认橙武条件与实现完整性，再检查天下宏愿、业火、斩刀和血怒的对轴取舍。",
            &[
                "scope",
                "scenario",
                "versioned_knowledge",
                "implementation_boundary",
                "timeline",
            ],
            &["candidate_comparison"],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "analyze_timeline",
                "compare_scenarios",
            ],
            &["天下宏愿 裂伤 绝刀", "橙武 业火 斩刀 血怒 对轴"],
            &[
                "缺少裂伤实现时不能发布完整正式服橙武总伤害",
                "减少冷却空转不等于总次数一定增加",
                "未装备橙武时不能套用该结论",
            ],
            &[
                ("scope", "确认橙武条件", "核对场景装备、版本、加速与延迟。"),
                (
                    "boundary",
                    "核对实现边界",
                    "读取当前技改与白皮书，并检查模拟器缺失项。",
                ),
                (
                    "alignment",
                    "检查爆发对轴",
                    "观察橙武、业火、斩刀、绝刀和血怒的实际间隔与覆盖。",
                ),
                (
                    "tradeoff",
                    "比较错轴取舍",
                    "区分总次数、技能质量、冷却空转和操作稳健性。",
                ),
            ],
        ),
        AnalysisTaskType::MacroAnalysis => (
            "macro_and_manual_analysis",
            "宏与手动循环",
            "解释宏判定服务的循环约束及其牺牲，并只用当前场景做可执行对照。",
            &["scope", "scenario", "versioned_knowledge", "macro_context"],
            &["candidate_comparison", "timeline"],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "simulate_scenario",
                "compare_scenarios",
            ],
            &["当前分山 一键宏 判定 延迟", "宏 单走绝刀 援戈"],
            &[
                "宏是玩家实践而非机制定义",
                "判定阈值不可脱离延迟复制给所有用户",
                "不提供越过游戏规则的自动化建议",
            ],
            &[
                (
                    "scope",
                    "读取宏与场景",
                    "确认当前宏、版本、心法、武器与延迟条件。",
                ),
                (
                    "intent",
                    "解释宏判定目标",
                    "用当前资料还原判定试图满足的循环约束。",
                ),
                (
                    "tradeoff",
                    "识别自动化牺牲",
                    "检查宏无法稳定完成的资源分配与覆盖。",
                ),
                (
                    "experiment",
                    "生成可执行对照",
                    "只修改当前工具允许的字段并保留回滚口径。",
                ),
            ],
        ),
        AnalysisTaskType::EquipmentAnalysis => (
            "equipment_build_analysis",
            "配装与装备取舍",
            "先读取当前配装和候选，再计算换前换后面板，并在同一循环下实测 DPS 与伤害构成。",
            &["scope", "scenario", "equipment_context", "candidate_comparison"],
            &["versioned_knowledge", "baseline_metrics"],
            &["get_current_scenario", "inspect_equipment_workspace", "compare_focused_equipment", "compare_equipment_strategies", "search_equipment_catalog", "search_knowledge_base"],
            &["当前分山 配装 套装 切糕 属性收益", "四件套 四切糕 取舍"],
            &["装备名和装分不能替代同循环实测", "四件套指普通套装四件效果；四切糕指四件切糕装备，不得混为同一套装", "没有明确候选方案时不能虚构完整配装或 DPS"],
            &[
                ("scope", "读取当前配装", "确认心法、版本、当前装备与循环来源。"),
                ("equipment", "识别装备方案", "解析具体装备名、套装件数与切糕等领域黑话。"),
                ("compare", "执行换装对比", "重算两侧面板，并用同一循环实测 DPS 与伤害构成。"),
                ("tradeoff", "解释属性取舍", "区分面板变化、套装特效、循环适配和证据边界。"),
            ],
        ),
        AnalysisTaskType::EncounterAdvice => (
            "encounter_advice",
            "指定副本实战建议",
            "按首领阶段、目标位置、转火和移动条件给建议，并明确木桩模拟不能证明的部分。",
            &["scope", "versioned_knowledge", "encounter_context"],
            &["scenario"],
            &["get_current_scenario", "search_knowledge_base"],
            &["首领 阶段 转火 移动 业火", "目标 无敌 距离 盾飞"],
            &[
                "木桩模拟不能证明Boss几何与阶段窗口",
                "攻略经验不能升级为必然最优",
                "未指定首领或阶段时只给决策框架",
            ],
            &[
                (
                    "scope",
                    "识别副本范围",
                    "确认首领、阶段、难度、心法和客户端。",
                ),
                (
                    "knowledge",
                    "读取实战资料",
                    "检索当前赛季对应首领与阶段的可溯源建议。",
                ),
                (
                    "conditions",
                    "拆出触发条件",
                    "按无敌、转火、移动、距离和团队窗口组织建议。",
                ),
                (
                    "boundary",
                    "声明验证边界",
                    "明确哪些判断来自攻略，哪些环境尚未被模拟。",
                ),
            ],
        ),
        AnalysisTaskType::MechanismExplanation => (
            "mechanism_explanation",
            "机制与公式说明",
            "先确定规则来源和适用范围，再区分官方机制、实测规律、作者推导与模拟器实现。",
            &["scope", "versioned_knowledge"],
            &["scenario", "baseline_metrics"],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "simulate_scenario",
            ],
            &["机制 公式 实测 推导", "当前版本 技改 实现"],
            &[
                "作者推导不能描述为官方公式",
                "知识数字不能冒充本轮模拟指标",
                "冲突或后续技改必须显式处理",
            ],
            &[
                (
                    "scope",
                    "确定机制范围",
                    "锁定客户端、版本、心法和问题对象。",
                ),
                (
                    "authority",
                    "辨别资料权威",
                    "区分正式技改、当前白皮书、机制实测和作者推导。",
                ),
                (
                    "relation",
                    "解释机制关系",
                    "说明触发条件、资源流和与其他技能的关系。",
                ),
                (
                    "boundary",
                    "对齐实现边界",
                    "需要当前数值时再用模拟器验证，并标出不一致。",
                ),
            ],
        ),
        AnalysisTaskType::ReferenceLookup => (
            "reference_lookup",
            "人物与来源查找",
            "用一次精确实体检索识别作者或来源，不把邻近人物扩展为当前玩法证据。",
            &["scope", "versioned_knowledge"],
            &[],
            &["get_current_scenario", "search_knowledge_base"],
            &["distinctive entity only"],
            &[
                "邻近姓名不能自动视为同一身份",
                "人物来源证据不能支撑当前玩法机制",
            ],
            &[
                (
                    "entity",
                    "提取唯一实体",
                    "只保留问题中的显著姓名、昵称或来源名。",
                ),
                (
                    "lookup",
                    "执行单点检索",
                    "读取明确标注的作者、视频作者或引用关系。",
                ),
                ("report", "返回身份与边界", "不围绕邻近姓名继续联想检索。"),
            ],
        ),
        AnalysisTaskType::GeneralAnalysis => (
            "general_grounded_analysis",
            "通用可验证分析",
            "从问题中选择最小证据路径，区分观察、诊断、决策与边界。",
            &["scope", "scenario"],
            &["versioned_knowledge", "baseline_metrics", "timeline"],
            &[
                "get_current_scenario",
                "search_knowledge_base",
                "simulate_scenario",
                "analyze_timeline",
                "compare_scenarios",
            ],
            &["问题中的技能、资源、装备或副本专名"],
            &["没有证据的诊断只能作为假设", "当前数值只由本轮模拟工具建立"],
            &[
                (
                    "scope",
                    "解析问题范围",
                    "锁定客户端、版本、心法和用户目标。",
                ),
                (
                    "evidence",
                    "选择最小证据路径",
                    "按问题需要读取资料或执行一个确定性实验。",
                ),
                (
                    "synthesis",
                    "组织专业结论",
                    "区分观察、诊断、取舍和下一步实验。",
                ),
            ],
        ),
    };

    let mut required_dimensions = strings(required);
    let mut preferred_tools = strings(tools);
    if client == DomainClient::Wujie {
        required_dimensions.retain(|dimension| {
            !matches!(*dimension, ref value if value == "baseline_metrics" || value == "timeline" || value == "candidate_comparison")
        });
        if !required_dimensions
            .iter()
            .any(|value| value == "versioned_knowledge")
        {
            required_dimensions.push("versioned_knowledge".to_string());
        }
        preferred_tools.retain(|tool| {
            matches!(
                tool.as_str(),
                "get_current_scenario" | "search_knowledge_base"
            )
        });
    }

    AnalysisPlaybookV1 {
        playbook_id: id.to_string(),
        label: label.to_string(),
        goal: goal.to_string(),
        required_dimensions,
        optional_dimensions: strings(optional),
        preferred_tools,
        knowledge_search_hints: strings(hints),
        forbidden_inferences: strings(forbidden),
        stages: stages
            .iter()
            .map(|(stage_id, stage_label, purpose)| AnalysisStageV1 {
                stage_id: (*stage_id).to_string(),
                label: (*stage_label).to_string(),
                purpose: (*purpose).to_string(),
            })
            .collect(),
    }
}

pub fn build_evidence_pack(plan: &AnalysisPlanV1, evidence: &EvidenceStore) -> EvidencePackV1 {
    let mut tools = BTreeSet::new();
    let mut claim_ids = BTreeSet::new();
    let mut boundaries = BTreeSet::new();
    let mut has_fact_eligible_knowledge = false;
    for envelope in evidence.values() {
        if let Some(tool) = envelope.get("tool_name").and_then(Value::as_str) {
            tools.insert(tool.to_string());
        }
        if envelope.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base") {
            collect_knowledge_facts(
                envelope,
                &mut claim_ids,
                &mut boundaries,
                &mut has_fact_eligible_knowledge,
            );
        }
    }

    let satisfied = satisfied_dimensions(
        plan,
        evidence,
        &tools,
        has_fact_eligible_knowledge,
        &boundaries,
    );
    let required = plan
        .playbook
        .required_dimensions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = required.difference(&satisfied).cloned().collect::<Vec<_>>();
    let sufficiency = if missing.is_empty() {
        EvidenceSufficiency::Sufficient
    } else if satisfied.is_empty() || (satisfied.len() == 1 && satisfied.contains("scope")) {
        EvidenceSufficiency::Insufficient
    } else {
        EvidenceSufficiency::Partial
    };
    EvidencePackV1 {
        schema_version: EVIDENCE_PACK_SCHEMA_V1.to_string(),
        playbook_id: plan.playbook.playbook_id.clone(),
        resolved_scope: plan.resolved_scope.clone(),
        evidence_ids: evidence.keys().cloned().collect(),
        tool_evidence: tools.into_iter().collect(),
        domain_claim_ids: claim_ids.into_iter().collect(),
        boundary_codes: boundaries.into_iter().collect(),
        coverage: EvidenceCoverageV1 {
            required_dimensions: required.into_iter().collect(),
            satisfied_dimensions: satisfied.into_iter().collect(),
            missing_dimensions: missing,
            sufficiency,
        },
        answer_constraints: plan.playbook.forbidden_inferences.clone(),
    }
}

fn satisfied_dimensions(
    plan: &AnalysisPlanV1,
    evidence: &EvidenceStore,
    tools: &BTreeSet<String>,
    has_fact_eligible_knowledge: bool,
    boundaries: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut dimensions = BTreeSet::from(["scope".to_string()]);
    if tools.contains("get_current_scenario") {
        dimensions.insert("scenario".to_string());
        dimensions.insert("rotation_input".to_string());
    }
    if has_fact_eligible_knowledge {
        dimensions.insert("versioned_knowledge".to_string());
    }
    if tools.contains("simulate_scenario")
        || tools.contains("analyze_timeline")
        || tools.contains("compare_scenarios")
        || tools.contains("compare_saved_macros")
        || tools.contains("compare_saved_scenarios")
        || tools.contains("compare_focused_equipment")
        || tools.contains("compare_equipment_strategies")
    {
        dimensions.insert("baseline_metrics".to_string());
    }
    if tools.contains("analyze_timeline") {
        dimensions.insert("timeline".to_string());
        if evidence.values().any(|item| {
            item.get("tool_name").and_then(Value::as_str) == Some("analyze_timeline")
                && item.pointer("/result/diagnostic_profile").is_some()
        }) {
            dimensions.insert("rotation_diagnosis".to_string());
        }
    }
    if tools.contains("compare_scenarios")
        || tools.contains("compare_saved_macros")
        || tools.contains("compare_saved_scenarios")
        || tools.contains("compare_focused_equipment")
        || tools.contains("compare_equipment_strategies")
    {
        dimensions.insert("candidate_comparison".to_string());
    }
    if tools.contains("list_saved_artifacts") || tools.contains("read_saved_artifact") {
        dimensions.insert("saved_artifacts".to_string());
    }
    if tools.contains("inspect_equipment_workspace") || tools.contains("search_equipment_catalog") {
        dimensions.insert("equipment_context".to_string());
    }
    if !boundaries.is_empty() {
        dimensions.insert("implementation_boundary".to_string());
    }
    if plan.task_type == AnalysisTaskType::EncounterAdvice && !plan.routing_signals.is_empty() {
        dimensions.insert("encounter_context".to_string());
    }
    if plan.task_type == AnalysisTaskType::MacroAnalysis
        && (plan
            .routing_signals
            .iter()
            .any(|signal| signal.contains('宏'))
            || evidence.values().any(|item| {
                item.pointer("/result/simulation/macro_text")
                    .is_some_and(|value| !value.is_null())
            }))
    {
        dimensions.insert("macro_context".to_string());
    }
    dimensions
}

fn collect_knowledge_facts(
    envelope: &Value,
    claim_ids: &mut BTreeSet<String>,
    boundaries: &mut BTreeSet<String>,
    has_fact_eligible: &mut bool,
) {
    let Some(results) = envelope
        .pointer("/result/results")
        .and_then(Value::as_array)
    else {
        return;
    };
    for result in results {
        *has_fact_eligible |= result
            .get("fact_eligible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(claims) = result.get("domain_claims").and_then(Value::as_array) else {
            continue;
        };
        for claim in claims {
            if let Some(id) = claim.get("claim_id").and_then(Value::as_str) {
                claim_ids.insert(id.to_string());
            }
            if let Some(codes) = claim
                .pointer("/verification/boundary_codes")
                .and_then(Value::as_array)
            {
                boundaries.extend(codes.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }
}

pub fn plan_model_context(plan: &AnalysisPlanV1) -> String {
    let json = serde_json::to_string(plan).unwrap_or_else(|_| "{}".to_string());
    format!(
        "<analysis_plan server_generated=\"true\" schema=\"{}\">\n{}\n</analysis_plan>",
        ANALYSIS_PLAN_SCHEMA_V1, json
    )
}

pub fn evidence_pack_model_context(pack: &EvidencePackV1) -> String {
    let json = serde_json::to_string(pack).unwrap_or_else(|_| "{}".to_string());
    format!(
        "<evidence_pack server_generated=\"true\" schema=\"{}\">\n{}\n</evidence_pack>",
        EVIDENCE_PACK_SCHEMA_V1, json
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceAnnotation {
    pub stage_id: String,
    pub label: String,
    pub overview: String,
}

pub fn trace_annotation(
    plan: &AnalysisPlanV1,
    kind: &str,
    tool_name: Option<&str>,
    evidence_count: usize,
) -> TraceAnnotation {
    let stage_id = match (kind, tool_name) {
        ("planning" | "analysis_plan_selected", _) => "scope",
        ("tool_started" | "tool_finished", Some("get_current_scenario")) => "scope",
        ("tool_started" | "tool_finished", Some("search_knowledge_base")) => "knowledge",
        ("tool_started" | "tool_finished", Some("simulate_scenario")) => "baseline",
        ("tool_started" | "tool_finished", Some("analyze_timeline")) => "locate",
        ("tool_started" | "tool_finished", Some("compare_scenarios")) => "compare",
        ("tool_started" | "tool_finished", Some("list_saved_artifacts")) => "saved",
        ("tool_started" | "tool_finished", Some("read_saved_artifact")) => "saved",
        ("tool_started" | "tool_finished", Some("compare_saved_macros")) => "compare",
        ("tool_started" | "tool_finished", Some("compare_saved_scenarios")) => "compare",
        ("tool_started" | "tool_finished", Some("inspect_equipment_workspace")) => "equipment",
        ("tool_started" | "tool_finished", Some("search_equipment_catalog")) => "equipment",
        ("tool_started" | "tool_finished", Some("compare_focused_equipment")) => "compare",
        ("tool_started" | "tool_finished", Some("compare_equipment_strategies")) => "compare",
        ("evidence_gap_requires_tool", Some("analyze_timeline")) => "locate",
        ("evidence_gap_requires_tool", Some("compare_scenarios")) => "compare",
        ("evidence_coverage_checked" | "reasoning_state_updated", _) => "coverage",
        ("reasoning_critique_started" | "reasoning_critique_failed" | "reasoning_critique_passed", _) => "validation",
        ("validating", _) => "validation",
        ("model_started" | "model_finished", _) => "synthesis",
        _ => "result",
    };
    let stage = plan
        .playbook
        .stages
        .iter()
        .find(|stage| stage.stage_id == stage_id)
        .or_else(|| plan.playbook.stages.first());
    let stage_label = stage
        .map(|stage| stage.label.as_str())
        .unwrap_or("可验证分析");
    let purpose = stage
        .map(|stage| stage.purpose.as_str())
        .unwrap_or(plan.playbook.goal.as_str());
    let (label, overview) = match kind {
        "planning" | "analysis_plan_selected" => (
            format!("选择任务路径 · {}", plan.playbook.label),
            plan.playbook.goal.clone(),
        ),
        "tool_started" => (format!("{} · 取得证据", stage_label), purpose.to_string()),
        "tool_finished" => (
            format!("{} · 已取得证据", stage_label),
            if evidence_count > 0 {
                format!(
                    "已登记{evidence_count}份本轮证据；继续按“{}”检查覆盖。",
                    plan.playbook.label
                )
            } else {
                "本阶段没有产生新证据，后续不会据此扩展事实。".to_string()
            },
        ),
        "evidence_coverage_checked" => (
            "检查专业维度覆盖".to_string(),
            "按任务 Playbook 核对必需维度，缺失项只允许有界补证或明确写入边界。".to_string(),
        ),
        "reasoning_state_updated" => (
            "更新问题推导状态".to_string(),
            "逐项记录哪些判断已有证据、哪些可以开始分析、哪些仍需补证。".to_string(),
        ),
        "reasoning_critique_started" => (
            "执行发布前批判检查".to_string(),
            "检查任务完成度、证据归属、因果强度、范围漂移与干预必要性。".to_string(),
        ),
        "reasoning_critique_failed" => (
            "批判检查要求修订".to_string(),
            "报告需要基于已有证据修订，不新增事实或扩大工具范围。".to_string(),
        ),
        "reasoning_critique_passed" => (
            "批判检查通过".to_string(),
            "报告已满足当前任务的证据推导与发布边界。".to_string(),
        ),
        "model_started" => (
            "组织下一阶段".to_string(),
            format!(
                "模型依据“{}”和现有证据选择下一项最小动作，不展示隐藏推理。",
                plan.playbook.label
            ),
        ),
        "model_finished" => (
            "阶段响应已返回".to_string(),
            "系统只解析工具动作或结构化报告，事实仍由已登记证据约束。".to_string(),
        ),
        "validating" => (
            "校验结论与证据".to_string(),
            "核对报告结构、数值、版本范围、资料引用和实现边界。".to_string(),
        ),
        _ => (stage_label.to_string(), purpose.to_string()),
    };
    TraceAnnotation {
        stage_id: stage_id.to_string(),
        label,
        overview,
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

fn season_for_version(version: &str) -> &'static str {
    match version {
        "2025_10_shanhai_yuanliu" => "山海源流（2025）",
        "2026_04_anying_qianji_test" => "体服（2021-2025）",
        _ => "暗影千机（2026）",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentRuntime;

    #[derive(Deserialize)]
    struct DomainEvalFixture {
        schema_version: String,
        cases: Vec<DomainEvalCase>,
    }

    #[derive(Deserialize)]
    struct DomainEvalCase {
        id: String,
        question: String,
        expected_playbook: String,
        required_dimensions: Vec<String>,
        forbid_simulation: bool,
    }

    #[test]
    fn routes_domain_questions_to_distinct_playbooks() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let cases = [
            ("分析当前循环输出基线", "current_rotation_baseline"),
            ("为什么这里空转？", "rotation_stall_diagnosis"),
            ("这个循环应该如何调优？", "current_rotation_baseline"),
            ("206 和 14156 怎么选？", "haste_band_decision"),
            ("橙武为什么少伤害？", "orange_weapon_timing"),
            ("帮我优化一键宏", "macro_and_manual_analysis"),
            ("千机源枢怎么打？", "encounter_advice"),
            ("盾压重置率怎么算？", "mechanism_explanation"),
        ];
        for (question, expected) in cases {
            assert_eq!(
                select_analysis_plan(question, &scenario)
                    .playbook
                    .playbook_id,
                expected
            );
        }
    }

    #[test]
    fn saved_macro_question_enables_catalog_and_server_side_ab_tools() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let plan = select_analysis_plan("对比我保存的绝云宏和武学助手宏，分析优缺点", &scenario);
        for tool in [
            "list_saved_artifacts",
            "read_saved_artifact",
            "compare_saved_macros",
        ] {
            assert!(plan
                .playbook
                .preferred_tools
                .iter()
                .any(|value| value == tool));
        }
        assert!(plan
            .playbook
            .required_dimensions
            .contains(&"saved_artifacts".to_string()));
        assert!(plan
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(!plan
            .routing_signals
            .contains(&"rotation_diagnosis_first".to_string()));
    }

    #[test]
    fn current_equipment_inspection_does_not_require_a_fabricated_candidate() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let plan = select_analysis_plan(
            "分析我当前配装的属性结构、套装与特效，并说明适配当前循环的证据边界。",
            &scenario,
        );

        assert_eq!(plan.task_type, AnalysisTaskType::EquipmentAnalysis);
        assert!(!plan
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(plan
            .playbook
            .required_dimensions
            .contains(&"versioned_knowledge".to_string()));
        assert!(plan
            .routing_signals
            .contains(&"equipment_current_build_inspection".to_string()));
        for unavailable in [
            "compare_focused_equipment",
            "compare_equipment_strategies",
            "search_equipment_catalog",
        ] {
            assert!(!plan
                .playbook
                .preferred_tools
                .contains(&unavailable.to_string()));
        }
    }

    #[test]
    fn explicit_equipment_comparisons_keep_only_the_matching_experiment() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let strategy = select_analysis_plan("穿四件套好还是穿四切糕好？", &scenario);
        assert!(strategy
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(strategy
            .playbook
            .preferred_tools
            .contains(&"compare_equipment_strategies".to_string()));
        assert!(!strategy
            .playbook
            .preferred_tools
            .contains(&"compare_focused_equipment".to_string()));

        let focused = select_analysis_plan("这件装备换成候选装备怎么样？", &scenario);
        assert!(focused
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(focused
            .playbook
            .preferred_tools
            .contains(&"compare_focused_equipment".to_string()));
        assert!(!focused
            .playbook
            .preferred_tools
            .contains(&"compare_equipment_strategies".to_string()));
    }

    #[test]
    fn saved_comparison_by_display_names_keeps_both_typed_comparators_available() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let plan = select_analysis_plan("对比我保存的分山绝云和分山武学助手", &scenario);

        for tool in ["compare_saved_macros", "compare_saved_scenarios"] {
            assert!(plan
                .playbook
                .preferred_tools
                .iter()
                .any(|value| value == tool));
        }
    }

    #[test]
    fn rotation_plan_uses_server_detected_input_mode() {
        let runtime = AgentRuntime::fixture();
        let manual = runtime.fixture_scenario();
        let manual_plan = select_analysis_plan("分析当前循环输出基线", &manual);
        assert!(manual_plan
            .routing_signals
            .contains(&"rotation_input_manual_sequence".to_string()));
        assert!(manual_plan
            .playbook
            .required_dimensions
            .contains(&"rotation_input".to_string()));
        assert!(manual_plan
            .playbook
            .required_dimensions
            .contains(&"timeline".to_string()));
        assert!(manual_plan
            .playbook
            .required_dimensions
            .contains(&"rotation_diagnosis".to_string()));
        assert!(manual_plan
            .routing_signals
            .contains(&"rotation_diagnosis_first".to_string()));
        assert!(!manual_plan
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));

        let mut macro_request = manual.simulation.clone();
        macro_request.macro_text = Some("/cast 盾击".to_string());
        let macro_scenario = ScenarioSnapshotV1::capture(
            crate::GameVersion::AnYingQianJi,
            crate::Mount::FenShanJin,
            macro_request,
        )
        .unwrap();
        let macro_plan = select_analysis_plan("分析当前循环输出基线", &macro_scenario);
        assert!(macro_plan
            .routing_signals
            .contains(&"rotation_input_macro".to_string()));
        assert!(!macro_plan
            .routing_signals
            .contains(&"rotation_input_manual_sequence".to_string()));

        let edit_plan = select_analysis_plan("找出宏循环漏洞并给出修改对照", &macro_scenario);
        assert!(edit_plan
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(edit_plan
            .routing_signals
            .contains(&"candidate_comparison_explicitly_requested".to_string()));

        let diagnosis_only = select_analysis_plan(
            "先分析当前循环已经做得好的地方，再找出有证据支持的主要风险；不要在诊断前直接给修改方案。",
            &macro_scenario,
        );
        assert!(!diagnosis_only
            .playbook
            .required_dimensions
            .contains(&"candidate_comparison".to_string()));
        assert!(!diagnosis_only
            .routing_signals
            .contains(&"candidate_comparison_explicitly_requested".to_string()));
    }

    #[test]
    fn short_follow_up_inherits_the_last_server_selected_playbook() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let inherited =
            select_analysis_plan_with_history("继续", Some("current_rotation_baseline"), &scenario);
        assert_eq!(inherited.playbook.playbook_id, "current_rotation_baseline");
        assert!(inherited
            .routing_signals
            .contains(&"inherited_session_playbook".to_string()));
        assert!(inherited
            .routing_signals
            .contains(&"rotation_diagnosis_first".to_string()));
        assert!(inherited
            .playbook
            .required_dimensions
            .contains(&"rotation_diagnosis".to_string()));

        let explicit = select_analysis_plan_with_history(
            "绝刀公式怎么算？",
            Some("current_rotation_baseline"),
            &scenario,
        );
        assert_eq!(explicit.playbook.playbook_id, "mechanism_explanation");
    }

    #[test]
    fn domain_eval_fixture_routes_and_declares_required_dimensions() {
        let fixture: DomainEvalFixture =
            serde_json::from_str(include_str!("../../tests/agent_domain_eval/cases.json")).unwrap();
        assert_eq!(fixture.schema_version, "agent-domain-eval-cases/v1");
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        for case in fixture.cases {
            let plan = select_analysis_plan(&case.question, &scenario);
            assert_eq!(
                plan.playbook.playbook_id, case.expected_playbook,
                "{} routed to the wrong playbook",
                case.id
            );
            assert!(case
                .required_dimensions
                .iter()
                .all(|dimension| { plan.playbook.required_dimensions.contains(dimension) }));
            if case.forbid_simulation {
                assert!(plan.playbook.preferred_tools.iter().all(|tool| {
                    matches!(
                        tool.as_str(),
                        "get_current_scenario" | "search_knowledge_base"
                    )
                }));
            }
        }
    }

    #[test]
    fn wujie_plan_removes_simulation_requirements() {
        let runtime = AgentRuntime::fixture();
        let plan = select_analysis_plan("无界分山循环怎么打？", &runtime.fixture_scenario());
        assert_eq!(plan.resolved_scope.client, DomainClient::Wujie);
        assert_eq!(plan.resolved_scope.mount, "fenshanjin_wu");
        assert!(plan.playbook.preferred_tools.iter().all(|tool| {
            matches!(
                tool.as_str(),
                "get_current_scenario" | "search_knowledge_base"
            )
        }));
    }

    #[test]
    fn domain_prefetch_is_task_specific_and_skips_noncurrent_or_reference_scope() {
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let orange = select_analysis_plan("橙武天下宏愿为什么少伤害？", &scenario);
        let orange_query = knowledge_prefetch(&orange, "橙武天下宏愿为什么少伤害？").unwrap();
        assert!(orange_query.query.contains("天下宏愿"));
        assert!(orange_query.query.contains("裂伤"));
        assert_eq!(orange_query.version_scope, "current_only");

        let wujie = select_analysis_plan("无界分山劲·悟循环怎么打？", &scenario);
        assert!(knowledge_prefetch(&wujie, "无界分山劲·悟循环怎么打？")
            .unwrap()
            .query
            .contains("无界"));

        let historical = select_analysis_plan("太极秘录（2025）的循环", &scenario);
        assert!(knowledge_prefetch(&historical, "太极秘录（2025）的循环").is_none());
        let reference = select_analysis_plan("世一苍是谁？", &scenario);
        assert!(knowledge_prefetch(&reference, "世一苍是谁？").is_none());
    }

    #[test]
    fn derives_only_source_bound_current_flagship_claims() {
        let claims = derive_domain_claims(DomainChunkContext {
            document_id: "doc",
            title: "⭐暗影千机_ 分山劲白皮书",
            season: "暗影千机（2026）",
            heading: "3.3.1 基础－橙武循环",
            text: "天下宏愿持续期间绝刀为目标添加持续伤害效果，最多叠加3层。",
            source_url: "https://example.com",
            yuque_url: "https://example.com",
            source_updated_at: "2026-08-15",
            document_hash: "document",
            chunk_hash: "chunk",
        });
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_id, "fs-cw-002");
        assert_eq!(claims[0].source.chunk_hash, "chunk");
        assert_eq!(
            claims[0].conflict_status,
            DomainConflictStatus::ImplementationMismatch
        );
    }

    #[test]
    fn corrected_charge_distance_formula_is_a_clear_source_bound_claim() {
        let claims = derive_domain_claims(DomainChunkContext {
            document_id: "advanced",
            title: "苍云进阶机制（2025）",
            season: "暗影千机（2026）",
            heading: "赴敌",
            text: "实际不触发突进的距离以这两个距离的最大值为准：实际不触发突进的距离 = MAX（突进保护距离，4尺）。",
            source_url: "https://www.yuque.com/sgyxy/cangyun/advanced",
            yuque_url: "https://www.yuque.com/sgyxy/cangyun/advanced",
            source_updated_at: "2026-08-28T06:51:23Z",
            document_hash: "document",
            chunk_hash: "chunk",
        });
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_id, "fs-charge-001");
        assert_eq!(claims[0].conflict_status, DomainConflictStatus::Clear);
        assert_eq!(claims[0].source.chunk_hash, "chunk");
    }

    #[test]
    fn evidence_pack_reports_missing_task_dimensions() {
        let runtime = AgentRuntime::fixture();
        let plan = select_analysis_plan("为什么这里空转？", &runtime.fixture_scenario());
        let pack = build_evidence_pack(&plan, &EvidenceStore::new());
        assert_eq!(pack.coverage.sufficiency, EvidenceSufficiency::Insufficient);
        assert!(pack
            .coverage
            .missing_dimensions
            .contains(&"timeline".to_string()));
    }
}
