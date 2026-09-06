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
pub enum DomainClient { Flagship, Wujie }

/// Wire-compatible task labels for old sessions. New runs always use
/// `GeneralAnalysis`; the model owns the investigation path.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisTaskType {
    BaselineAnalysis,
    RotationStallDiagnosis,
    RotationEditAnalysis,
    PracticalAdaptation,
    HasteDecision,
    OrangeWeaponTiming,
    MacroAnalysis,
    SavedArtifactAnalysis,
    EquipmentAnalysis,
    EncounterAdvice,
    MechanismExplanation,
    ReferenceLookup,
    GeneralAnalysis,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisSurface { Simulation, Equipment, Agent }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    StructuredHint,
    ComposedIntent,
    HybridSemantic,
    ScoredSignals,
    SessionInheritance,
    GeneralFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingCandidateV1 {
    pub task_type: AnalysisTaskType,
    pub score: u16,
    pub lexical_score: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_similarity_millis: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingDecisionV1 {
    pub strategy: RoutingStrategy,
    pub confidence_basis_points: u16,
    pub candidates: Vec<RoutingCandidateV1>,
}

impl Default for RoutingDecisionV1 {
    fn default() -> Self {
        Self { strategy: RoutingStrategy::GeneralFallback, confidence_basis_points: 0, candidates: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticRouteScoreV1 {
    pub task_type: AnalysisTaskType,
    pub similarity_millis: u16,
}

// Compatibility types for persisted evidence. Runtime no longer authors
// gameplay claims in code; source passages and corpus-derived term cards carry
// domain meaning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainClaimType { OfficialChange, GameMechanic, MeasuredMechanic, DerivedFormula, PlayerPractice, OptimizationHypothesis }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainAuthority { OfficialCurrentPatch, CurrentWhitepaper, CurrentMechanismTest, CurrentPractical, CurrentDerived }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainConflictStatus { Clear, SupersedesOlder, UnresolvedInternal, ImplementationMismatch }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SimulatorSupport { Implemented, PartiallyObservable, Unsupported, KnowledgeOnly }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainEntityV1 { pub kind: String, pub name: String }

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
    pub derivation_method: String,
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
pub struct DomainRelationV1 { pub from: String, pub relation: String, pub to: String, pub claim_id: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisStageV1 { pub stage_id: String, pub label: String, pub purpose: String }

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
    #[serde(default)]
    pub routing_decision: RoutingDecisionV1,
}

/// Deprecated compatibility shape. Model-led runs decide whether retrieval is
/// useful instead of receiving a task-specific prefetch query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgePrefetchV1 {
    pub query: String,
    pub version_scope: String,
    pub season: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSufficiency { Sufficient, Partial, Insufficient }

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

/// Compatibility hook for old knowledge payloads. The former hand-authored
/// answer table has been removed.
pub fn derive_domain_claims(_context: DomainChunkContext<'_>) -> Vec<DomainClaimV1> { Vec::new() }

pub fn domain_relations(claims: &[DomainClaimV1]) -> Vec<DomainRelationV1> {
    claims.iter().map(|claim| DomainRelationV1 {
        from: claim.subject.name.clone(),
        relation: claim.relation.clone(),
        to: claim.object.name.clone(),
        claim_id: claim.claim_id.clone(),
    }).collect()
}

pub fn domain_index_hash<'a>(claims: impl Iterator<Item = &'a DomainClaimV1>) -> String {
    let mut identities = claims.map(|claim| (
        claim.claim_id.as_str(),
        claim.source.document_hash.as_str(),
        claim.source.chunk_hash.as_str(),
    )).collect::<Vec<_>>();
    identities.sort_unstable();
    canonical_sha256(&identities).unwrap_or_else(|_| "unavailable".to_string())
}

/// Resolve only product scope. No task label, similarity prototype, keyword
/// route, or inherited playbook can lock the model to a workflow.
pub fn select_model_led_analysis_plan(
    question: &str,
    surface: Option<AnalysisSurface>,
    scenario: &ScenarioSnapshotV1,
) -> AnalysisPlanV1 {
    let normalized = question.to_lowercase();
    let client = if contains_any(&normalized, &["无界", "分山劲·悟", "分山劲・悟", "wujie"]) { DomainClient::Wujie } else { DomainClient::Flagship };
    let mount = if client == DomainClient::Wujie {
        if normalized.contains("铁骨") { "tieguyi_wu" } else { "fenshanjin_wu" }
    } else if normalized.contains("铁骨") {
        "tieguyi"
    } else if normalized.contains("分山") {
        "fenshanjin"
    } else {
        scenario.mount.as_str()
    };
    let scope = DomainScopeV1 {
        client,
        game_version: scenario.game_version.clone(),
        season: season_for_version(&scenario.game_version).to_string(),
        mount: mount.to_string(),
        mode: if contains_any(&normalized, &["pvp", "竞技场", "战场"]) { "pvp" } else { "pve" }.to_string(),
    };
    let mut routing_signals = vec!["model_led_runtime".to_string()];
    if let Some(surface) = surface { routing_signals.push(format!("analysis_surface:{surface:?}")); }
    AnalysisPlanV1 {
        schema_version: ANALYSIS_PLAN_SCHEMA_V1.to_string(),
        task_type: AnalysisTaskType::GeneralAnalysis,
        resolved_scope: scope,
        playbook: model_led_playbook(client),
        routing_signals,
        routing_decision: RoutingDecisionV1::default(),
    }
}

pub fn select_analysis_plan(question: &str, scenario: &ScenarioSnapshotV1) -> AnalysisPlanV1 {
    select_model_led_analysis_plan(question, None, scenario)
}

pub fn knowledge_prefetch(_plan: &AnalysisPlanV1, _question: &str) -> Option<KnowledgePrefetchV1> { None }

fn model_led_playbook(client: DomainClient) -> AnalysisPlaybookV1 {
    let optional = if client == DomainClient::Wujie {
        vec!["versioned_knowledge"]
    } else {
        vec!["versioned_knowledge", "baseline_metrics", "timeline", "rotation_anchor", "candidate_comparison", "saved_artifacts", "equipment_context", "implementation_boundary"]
    };
    AnalysisPlaybookV1 {
        playbook_id: "model_led_diagnosis".to_string(),
        label: "模型自主诊断".to_string(),
        goal: "依据用户目标和已解析领域实体，自主选择理解、取证、实验、追问或结论。".to_string(),
        required_dimensions: vec!["scope".to_string(), "scenario".to_string()],
        optional_dimensions: optional.into_iter().map(str::to_string).collect(),
        preferred_tools: Vec::new(),
        knowledge_search_hints: Vec::new(),
        forbidden_inferences: Vec::new(),
        stages: vec![
            AnalysisStageV1 { stage_id: "understand".to_string(), label: "理解问题".to_string(), purpose: "形成当前判断并选择信息增益最大的动作。".to_string() },
            AnalysisStageV1 { stage_id: "investigate".to_string(), label: "自主取证".to_string(), purpose: "按需要读取资料、定位事件或运行实验。".to_string() },
            AnalysisStageV1 { stage_id: "answer".to_string(), label: "形成结论".to_string(), purpose: "直接回答问题并标明实测、资料与推断的关系。".to_string() },
        ],
    }
}

pub fn build_evidence_pack(plan: &AnalysisPlanV1, evidence: &EvidenceStore) -> EvidencePackV1 {
    let tools = evidence.values()
        .filter_map(|envelope| envelope.get("tool_name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let satisfied = satisfied_dimensions(&tools, evidence);
    let required = plan.playbook.required_dimensions.iter().cloned().collect::<BTreeSet<_>>();
    let missing = required.difference(&satisfied).cloned().collect::<Vec<_>>();
    let sufficiency = if missing.is_empty() { EvidenceSufficiency::Sufficient } else if satisfied.len() <= 1 { EvidenceSufficiency::Insufficient } else { EvidenceSufficiency::Partial };
    EvidencePackV1 {
        schema_version: EVIDENCE_PACK_SCHEMA_V1.to_string(),
        playbook_id: plan.playbook.playbook_id.clone(),
        resolved_scope: plan.resolved_scope.clone(),
        evidence_ids: evidence.keys().cloned().collect(),
        tool_evidence: tools.into_iter().collect(),
        domain_claim_ids: Vec::new(),
        boundary_codes: Vec::new(),
        coverage: EvidenceCoverageV1 {
            required_dimensions: required.into_iter().collect(),
            satisfied_dimensions: satisfied.into_iter().collect(),
            missing_dimensions: missing,
            sufficiency,
        },
        answer_constraints: Vec::new(),
    }
}

fn satisfied_dimensions(tools: &BTreeSet<String>, evidence: &EvidenceStore) -> BTreeSet<String> {
    let mut dimensions = BTreeSet::from(["scope".to_string()]);
    if tools.contains("get_current_scenario") { dimensions.extend(["scenario".to_string(), "rotation_input".to_string()]); }
    if tools.contains("search_knowledge_base") && evidence.values().any(fact_eligible_knowledge) { dimensions.insert("versioned_knowledge".to_string()); }
    if tools.contains("simulate_scenario") || tools.contains("analyze_timeline") || has_comparison(tools) { dimensions.insert("baseline_metrics".to_string()); }
    if tools.contains("analyze_timeline") || tools.contains("inspect_timeline_events") { dimensions.insert("timeline".to_string()); }
    if tools.contains("inspect_rotation_input") || tools.contains("inspect_timeline_events") { dimensions.insert("rotation_anchor".to_string()); }
    if has_comparison(tools) { dimensions.insert("candidate_comparison".to_string()); }
    if tools.contains("list_saved_artifacts") || tools.contains("read_saved_artifact") { dimensions.insert("saved_artifacts".to_string()); }
    if tools.contains("inspect_equipment_workspace") || tools.contains("search_equipment_catalog") { dimensions.insert("equipment_context".to_string()); }
    dimensions
}

fn fact_eligible_knowledge(envelope: &Value) -> bool {
    envelope.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")
        && envelope.pointer("/result/results").and_then(Value::as_array).is_some_and(|results| {
            results.iter().any(|result| result.get("fact_eligible").and_then(Value::as_bool) == Some(true))
        })
}

fn has_comparison(tools: &BTreeSet<String>) -> bool {
    ["compare_scenarios", "compare_saved_macros", "compare_saved_scenarios", "compare_focused_equipment", "compare_equipment_strategies"]
        .iter().any(|name| tools.contains(*name))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceAnnotation { pub stage_id: String, pub label: String, pub overview: String }

pub fn trace_annotation(_plan: &AnalysisPlanV1, kind: &str, tool_name: Option<&str>, evidence_count: usize) -> TraceAnnotation {
    let (stage_id, stage_label, purpose) = match tool_name {
        Some("get_current_scenario") => ("scope", "确认分析对象", "读取当前版本、心法、环境与循环。"),
        Some("search_knowledge_base") => ("knowledge", "查阅版本资料", "核对当前赛季机制、术语与打法语境。"),
        Some("inspect_rotation_input") => ("locate", "定位循环操作", "定位同名技能的具体操作与相邻结构。"),
        Some("analyze_timeline") => ("diagnose", "建立循环画像", "检查阶段、资源、增益、技能结构与执行质量。"),
        Some("inspect_timeline_events") => ("locate", "核对具体事件", "展开聚合信号对应的时间点与相邻技能。"),
        Some("simulate_scenario") => ("evidence", "运行基线模拟", "取得当前冻结场景的确定性结果。"),
        Some("compare_scenarios" | "compare_saved_macros" | "compare_saved_scenarios" | "compare_focused_equipment" | "compare_equipment_strategies") => ("experiment", "验证候选假设", "保持其余条件一致，比较单变量改动的实际影响。"),
        Some("list_saved_artifacts" | "read_saved_artifact") => ("evidence", "读取已保存方案", "定位用户点名的本地方案与内容。"),
        Some("inspect_equipment_workspace" | "search_equipment_catalog") => ("evidence", "读取配装上下文", "取得当前装备、候选与面板口径。"),
        _ if matches!(kind, "planning" | "analysis_context_prepared" | "analysis_plan_selected") => ("understand", "理解用户目标", "结合问题、会话与页面状态确定当前要解决的事情。"),
        _ => ("decision", "评估当前判断", "依据已有事实决定继续取证、验证、追问或形成结论。"),
    };
    let (label, overview) = match kind {
        "planning" => ("理解分析任务".to_string(), "识别用户目标、当前上下文和需要验证的事实类型。".to_string()),
        "analysis_context_prepared" | "analysis_plan_selected" => ("准备分析上下文".to_string(), "已载入当前会话、页面与场景信息，接下来由模型决定分析步骤。".to_string()),
        "tool_started" => (format!("{stage_label} · 进行中"), purpose.to_string()),
        "tool_finished" => (
            if evidence_count > 0 { format!("{stage_label} · 已取得证据") } else { format!("{stage_label} · 未产生新证据") },
            if evidence_count > 0 { format!("已登记{evidence_count}份新证据；模型将据此更新当前判断。") } else { "本阶段没有产生新证据，后续不会据此扩展事实。".to_string() },
        ),
        "model_started" => ("选择下一步".to_string(), "模型根据用户目标、当前判断和证据缺口选择下一项动作。".to_string()),
        "model_finished" => ("阶段响应已返回".to_string(), "系统只解析工具动作或结构化报告，事实仍由已登记证据约束。".to_string()),
        "validating" => ("校验结论与证据".to_string(), "核对报告结构、数值、版本范围、资料引用和实现边界。".to_string()),
        "provider_empty_retry" => ("重试生成结论".to_string(), "模型正文为空；保留现有证据并进行一次有界重试，不重复调用工具。".to_string()),
        "report_repair_requested" => ("修复报告结构".to_string(), "报告未满足结构或证据协议；仅修复报告，不改变已取得事实。".to_string()),
        "report_structure_evidence_preserved" => ("发布证据兜底报告".to_string(), "模型报告未通过校验；系统直接从已登记证据生成可验证摘要。".to_string()),
        _ => (stage_label.to_string(), purpose.to_string()),
    };
    TraceAnnotation { stage_id: stage_id.to_string(), label, overview }
}

fn contains_any(value: &str, terms: &[&str]) -> bool { terms.iter().any(|term| value.contains(term)) }

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

    #[test]
    fn every_question_uses_the_same_model_led_plan() {
        let scenario = AgentRuntime::fixture().fixture_scenario();
        for question in ["这套循环哪里有问题？", "哪里可以插白刀？", "穿四件套还是四切糕？", "世一苍是谁？"] {
            let plan = select_analysis_plan(question, &scenario);
            assert_eq!(plan.task_type, AnalysisTaskType::GeneralAnalysis);
            assert_eq!(plan.playbook.playbook_id, "model_led_diagnosis");
            assert!(plan.routing_decision.candidates.is_empty());
            assert!(plan.playbook.knowledge_search_hints.is_empty());
        }
    }

    #[test]
    fn code_authored_claim_table_is_empty() {
        let claims = derive_domain_claims(DomainChunkContext {
            document_id: "doc", title: "任意攻略", season: "暗影千机（2026）", heading: "循环",
            text: "白刀指某种循环做法。", source_url: "https://example.com/source",
            yuque_url: "https://example.com/yuque", source_updated_at: "2026-09-05",
            document_hash: "doc-hash", chunk_hash: "chunk-hash",
        });
        assert!(claims.is_empty());
    }

    #[test]
    fn scope_resolution_does_not_create_a_task_route() {
        let scenario = AgentRuntime::fixture().fixture_scenario();
        let plan = select_model_led_analysis_plan("无界铁骨怎么配？", Some(AnalysisSurface::Equipment), &scenario);
        assert_eq!(plan.resolved_scope.client, DomainClient::Wujie);
        assert_eq!(plan.resolved_scope.mount, "tieguyi_wu");
        assert_eq!(plan.task_type, AnalysisTaskType::GeneralAnalysis);
    }
}
