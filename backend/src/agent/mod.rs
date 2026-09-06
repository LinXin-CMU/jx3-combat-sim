//! Strongly typed, read-only tools used by the future combat-analysis Agent.
//!
//! The deterministic simulator remains the source of truth. This module owns
//! scenario identity and evidence contracts; it must not duplicate combat
//! formulas or expose arbitrary filesystem/process access.

pub mod artifacts;
pub mod compare;
pub mod domain;
pub mod distillation;
mod macro_tuning;
pub mod equipment;
pub mod evidence;
pub mod hash;
pub mod http;
pub mod knowledge;
mod knowledge_dense;
pub mod mechanics;
pub mod orchestrator;
pub mod prompt;
pub mod provider;
pub mod registry;
pub mod report;
pub mod run;
pub mod runtime;
pub mod saved;
pub mod schema;
pub mod session;
pub mod timeline;
mod time_budget;
pub mod terminology;
pub mod tools;

pub use compare::{
    compare_scenarios, compare_scenarios_with_baseline, CandidatePatchV1, ComparisonCandidate,
    ComparisonExecution, ComparisonMetrics, FieldChange, PatchValueV1, ScenarioComparison,
    ScenarioPatchV1,
};
pub use domain::{
    build_evidence_pack, derive_domain_claims, domain_index_hash, domain_relations,
    knowledge_prefetch, select_model_led_analysis_plan, AnalysisPlanV1, AnalysisPlaybookV1,
    AnalysisSurface, AnalysisTaskType, DomainChunkContext, DomainClaimV1, DomainRelationV1,
    EvidencePackV1, EvidenceSufficiency, KnowledgePrefetchV1,
};
pub use equipment::{EquipmentComparisonPresentationV1, EquipmentWorkspaceV1};
pub use evidence::{EvidenceEnvelopeV1, ToolProvenance, EVIDENCE_SCHEMA_V1};
pub use knowledge::{
    KnowledgeAudience, KnowledgeClientScope, KnowledgeIndex, KnowledgeIndexError,
    KnowledgeMountScope, KnowledgeQuality, KnowledgeRetrievalInfo, KnowledgeSearchQuery,
    KnowledgeSearchResponse, KnowledgeSearchResult, KnowledgeVersionContext, KnowledgeVersionMatch,
    KnowledgeVersionScope, MAX_KNOWLEDGE_RESULTS,
};
pub use runtime::AgentRuntime;
pub use saved::{
    list_saved_artifacts, prepare_saved_macro_comparison, prepare_saved_scenario_comparison,
    read_saved_artifact, PreparedSavedComparison, SavedArtifactCatalog, SavedArtifactDocument,
    SavedArtifactError, SavedArtifactKind, SavedArtifactSummary, SavedComparisonContext,
    COMPARE_SAVED_MACROS, COMPARE_SAVED_SCENARIOS, LIST_SAVED_ARTIFACTS, READ_SAVED_ARTIFACT,
};
pub use schema::{ScenarioError, ScenarioSnapshotV1, SCENARIO_SCHEMA_V1};
pub use timeline::{
    analyze_timeline, comparison_timeline_diagnostics, inspect_timeline_events,
    AbsoluteKnifeObservationV1, BuffCoverage, ComparisonBuffObservationV1,
    ComparisonTimelineDiagnosticsV1, GcdGapEvidence, PauseRecoveryObservationV1, RageObservation,
    RotationCycleObservationV1, RotationCycleProfileV1, RotationCycleSkillV1, TimeInterval,
    TimelineAnalysis, TimelineEventInspectionExecution, TimelineEventInspectionV1,
    TimelineEventQueryV1, TimelineEventSelector, TimelineEventViewV1, TimelineEventWindowV1,
    TimelineExecution, TimelineStateViewV1, WaitEvidence, INSPECT_TIMELINE_EVENTS,
};
pub use terminology::{
    DomainTermCardV1, DomainTermChunkContext, DomainTermKindV1, DomainTerminologyIndexV1,
    ResolvedDomainTermV1, DOMAIN_TERM_SCHEMA_V1,
};
pub use tools::{
    get_current_scenario, inspect_rotation_input, simulate_scenario, RotationInputEntryV1,
    RotationInputInspectionV1, RotationInputMatchV1, ScenarioSummary, SimulationExecution,
    SimulationSummary, SimulatorContext, ToolBudget, ToolError, INSPECT_ROTATION_INPUT,
};
