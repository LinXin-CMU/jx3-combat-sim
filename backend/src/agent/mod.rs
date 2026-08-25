//! Strongly typed, read-only tools used by the future combat-analysis Agent.
//!
//! The deterministic simulator remains the source of truth. This module owns
//! scenario identity and evidence contracts; it must not duplicate combat
//! formulas or expose arbitrary filesystem/process access.

pub mod hash;
pub mod schema;

pub use schema::{ScenarioError, ScenarioSnapshotV1, SCENARIO_SCHEMA_V1};
