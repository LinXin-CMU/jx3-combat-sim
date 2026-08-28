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
    let has_rotation_input = scenario.simulation.sequence.len() >= 6
        || scenario
            .simulation
            .macro_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty());
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
    plan
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
    Some(KnowledgePrefetchV1 {
        query: format!("{question_prefix} {domain_terms}")
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
            ],
        ),
        (
            AnalysisTaskType::BaselineAnalysis,
            &["输出基线", "当前循环", "基线", "伤害构成", "输出分析"],
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
    }
    if has_fact_eligible_knowledge {
        dimensions.insert("versioned_knowledge".to_string());
    }
    if tools.contains("simulate_scenario")
        || tools.contains("analyze_timeline")
        || tools.contains("compare_scenarios")
    {
        dimensions.insert("baseline_metrics".to_string());
    }
    if tools.contains("analyze_timeline") {
        dimensions.insert("timeline".to_string());
    }
    if tools.contains("compare_scenarios") {
        dimensions.insert("candidate_comparison".to_string());
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
        ("evidence_coverage_checked", _) => "coverage",
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
