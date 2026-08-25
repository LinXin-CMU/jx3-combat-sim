//! Strongly typed, read-only tools used by the future combat-analysis Agent.
//!
//! The deterministic simulator remains the source of truth. This module owns
//! scenario identity and evidence contracts; it must not duplicate combat
//! formulas or expose arbitrary filesystem/process access.

pub mod compare;
pub mod evidence;
pub mod hash;
pub mod schema;
pub mod timeline;
pub mod tools;

pub use compare::{
    compare_scenarios, CandidatePatchV1, ComparisonCandidate, ComparisonExecution,
    ComparisonMetrics, FieldChange, PatchValueV1, ScenarioComparison, ScenarioPatchV1,
};
pub use evidence::{EvidenceEnvelopeV1, ToolProvenance, EVIDENCE_SCHEMA_V1};
pub use schema::{ScenarioError, ScenarioSnapshotV1, SCENARIO_SCHEMA_V1};
pub use timeline::{
    analyze_timeline, BuffCoverage, GcdGapEvidence, RageObservation, TimeInterval,
    TimelineAnalysis, TimelineExecution, WaitEvidence,
};
pub use tools::{
    get_current_scenario, simulate_scenario, ScenarioSummary, SimulationExecution,
    SimulationSummary, SimulatorContext, ToolBudget, ToolError,
};
