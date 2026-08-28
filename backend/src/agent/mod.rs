//! Strongly typed, read-only tools used by the future combat-analysis Agent.
//!
//! The deterministic simulator remains the source of truth. This module owns
//! scenario identity and evidence contracts; it must not duplicate combat
//! formulas or expose arbitrary filesystem/process access.

pub mod compare;
pub mod domain;
pub mod evidence;
pub mod hash;
pub mod http;
pub mod knowledge;
mod knowledge_dense;
pub mod orchestrator;
pub mod prompt;
pub mod provider;
pub mod registry;
pub mod report;
pub mod run;
pub mod runtime;
pub mod schema;
pub mod session;
pub mod timeline;
pub mod tools;

pub use compare::{
    compare_scenarios, CandidatePatchV1, ComparisonCandidate, ComparisonExecution,
    ComparisonMetrics, FieldChange, PatchValueV1, ScenarioComparison, ScenarioPatchV1,
};
pub use domain::{
    build_evidence_pack, derive_domain_claims, domain_index_hash, domain_relations,
    evidence_pack_model_context, knowledge_prefetch, plan_model_context, select_analysis_plan,
    AnalysisPlanV1, AnalysisPlaybookV1, AnalysisTaskType, DomainChunkContext, DomainClaimV1,
    DomainRelationV1, EvidencePackV1, EvidenceSufficiency, KnowledgePrefetchV1,
};
pub use evidence::{EvidenceEnvelopeV1, ToolProvenance, EVIDENCE_SCHEMA_V1};
pub use knowledge::{
    KnowledgeAudience, KnowledgeClientScope, KnowledgeIndex, KnowledgeIndexError,
    KnowledgeMountScope, KnowledgeQuality, KnowledgeRetrievalInfo, KnowledgeSearchQuery,
    KnowledgeSearchResponse, KnowledgeSearchResult, KnowledgeVersionContext, KnowledgeVersionMatch,
    KnowledgeVersionScope, MAX_KNOWLEDGE_RESULTS,
};
pub use runtime::AgentRuntime;
pub use schema::{ScenarioError, ScenarioSnapshotV1, SCENARIO_SCHEMA_V1};
pub use timeline::{
    analyze_timeline, BuffCoverage, GcdGapEvidence, RageObservation, TimeInterval,
    TimelineAnalysis, TimelineExecution, WaitEvidence,
};
pub use tools::{
    get_current_scenario, simulate_scenario, ScenarioSummary, SimulationExecution,
    SimulationSummary, SimulatorContext, ToolBudget, ToolError,
};
