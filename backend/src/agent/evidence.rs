use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FormationEntry, GameVersion, Mount, MountConstants, RecipeEntry, SkillSpec, TeamBuffEntry,
};

use super::hash::canonical_sha256;
use super::schema::{game_version_id, mount_id};

pub const EVIDENCE_SCHEMA_V1: &str = "agent-evidence/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolProvenance {
    pub engine_version: String,
    pub engine_commit: String,
    pub data_hash: String,
}

impl ToolProvenance {
    /// Build provenance without reading Git or data files during a tool call.
    /// The application startup path is responsible for computing/injecting the
    /// data hash once and may provide `JX3_BUILD_COMMIT` at compile time.
    pub fn from_build(data_hash: impl Into<String>) -> Self {
        Self {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            engine_commit: option_env!("JX3_BUILD_COMMIT")
                .unwrap_or("unknown")
                .to_string(),
            data_hash: data_hash.into(),
        }
    }

    /// Hash the immutable runtime tables once when a worker starts or its
    /// version/mount is explicitly reloaded. Tool calls only clone the result.
    pub fn from_runtime_data(
        game_version: GameVersion,
        mount: Mount,
        constants: MountConstants,
        skills: &[SkillSpec],
        recipes: &[RecipeEntry],
        team_buffs: &[TeamBuffEntry],
        formations: &[FormationEntry],
    ) -> Self {
        let identity = RuntimeDataIdentity {
            schema_version: "agent-runtime-data/v1",
            game_version: game_version_id(game_version),
            mount: mount_id(mount),
            constants,
            skills,
            recipes,
            team_buffs,
            formations,
        };
        let data_hash = canonical_sha256(&identity).unwrap_or_else(|_| "unknown".to_string());
        Self::from_build(data_hash)
    }

    #[cfg(test)]
    pub fn fixture() -> Self {
        Self {
            engine_version: "test-engine".to_string(),
            engine_commit: "test-commit".to_string(),
            data_hash: "test-data".to_string(),
        }
    }

    fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.engine_commit.trim().is_empty() || self.engine_commit == "unknown" {
            warnings.push("engine_commit_unavailable".to_string());
        }
        if self.data_hash.trim().is_empty() || self.data_hash == "unknown" {
            warnings.push("data_hash_unavailable".to_string());
        }
        warnings
    }
}

#[derive(Serialize)]
struct RuntimeDataIdentity<'a> {
    schema_version: &'static str,
    game_version: &'static str,
    mount: &'static str,
    constants: MountConstants,
    skills: &'a [SkillSpec],
    recipes: &'a [RecipeEntry],
    team_buffs: &'a [TeamBuffEntry],
    formations: &'a [FormationEntry],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceEnvelopeV1<T> {
    pub schema_version: String,
    pub trace_id: String,
    pub evidence_id: String,
    pub tool_name: String,
    pub scenario_hash: String,
    pub engine_version: String,
    pub engine_commit: String,
    pub data_hash: String,
    pub args: Value,
    pub result: T,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    InvalidTraceId,
    Serialization(String),
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTraceId => {
                write!(f, "trace_id must match [A-Za-z0-9_-] and be 1..64 bytes")
            }
            Self::Serialization(error) => write!(f, "cannot build evidence envelope: {error}"),
        }
    }
}

impl std::error::Error for EvidenceError {}

#[derive(Serialize)]
struct EvidenceIdentity<'a, T> {
    schema_version: &'static str,
    tool_name: &'a str,
    scenario_hash: &'a str,
    engine_version: &'a str,
    engine_commit: &'a str,
    data_hash: &'a str,
    args: &'a Value,
    result: &'a T,
    warnings: &'a [String],
}

impl<T: Serialize> EvidenceEnvelopeV1<T> {
    pub fn new(
        trace_id: impl Into<String>,
        tool_name: impl Into<String>,
        scenario_hash: impl Into<String>,
        args: Value,
        result: T,
        provenance: &ToolProvenance,
        duration_ms: u64,
    ) -> Result<Self, EvidenceError> {
        let trace_id = trace_id.into();
        validate_trace_id(&trace_id)?;

        let tool_name = tool_name.into();
        let scenario_hash = scenario_hash.into();
        let warnings = provenance.warnings();
        let evidence_id = canonical_sha256(&EvidenceIdentity {
            schema_version: EVIDENCE_SCHEMA_V1,
            tool_name: &tool_name,
            scenario_hash: &scenario_hash,
            engine_version: &provenance.engine_version,
            engine_commit: &provenance.engine_commit,
            data_hash: &provenance.data_hash,
            args: &args,
            result: &result,
            warnings: &warnings,
        })
        .map_err(|error| EvidenceError::Serialization(error.to_string()))?;

        Ok(Self {
            schema_version: EVIDENCE_SCHEMA_V1.to_string(),
            trace_id,
            evidence_id,
            tool_name,
            scenario_hash,
            engine_version: provenance.engine_version.clone(),
            engine_commit: provenance.engine_commit.clone(),
            data_hash: provenance.data_hash.clone(),
            args,
            result,
            warnings,
            duration_ms,
        })
    }
}

pub(super) fn validate_trace_id(trace_id: &str) -> Result<(), EvidenceError> {
    let valid = !trace_id.is_empty()
        && trace_id.len() <= 64
        && trace_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(EvidenceError::InvalidTraceId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_identity_is_stable_and_excludes_trace_and_duration() {
        let provenance = ToolProvenance::fixture();
        let first = EvidenceEnvelopeV1::new(
            "trace-1",
            "simulate_scenario",
            "scenario-1",
            serde_json::json!({"mode": "full"}),
            serde_json::json!({"dps": 123.0}),
            &provenance,
            10,
        )
        .unwrap();
        let second = EvidenceEnvelopeV1::new(
            "trace-2",
            "simulate_scenario",
            "scenario-1",
            serde_json::json!({"mode": "full"}),
            serde_json::json!({"dps": 123.0}),
            &provenance,
            999,
        )
        .unwrap();

        assert_eq!(first.evidence_id, second.evidence_id);
        assert_ne!(first.trace_id, second.trace_id);
        assert_ne!(first.duration_ms, second.duration_ms);
    }

    #[test]
    fn evidence_rejects_unsafe_trace_ids() {
        let error = EvidenceEnvelopeV1::new(
            "../../userdata",
            "get_current_scenario",
            "scenario-1",
            serde_json::json!({}),
            serde_json::json!({}),
            &ToolProvenance::fixture(),
            0,
        )
        .unwrap_err();

        assert_eq!(error, EvidenceError::InvalidTraceId);
    }

    #[test]
    fn missing_build_provenance_is_explicitly_warned() {
        let provenance = ToolProvenance {
            engine_version: "2.0.10".to_string(),
            engine_commit: "unknown".to_string(),
            data_hash: "unknown".to_string(),
        };
        let evidence = EvidenceEnvelopeV1::new(
            "trace-1",
            "get_current_scenario",
            "scenario-1",
            serde_json::json!({}),
            serde_json::json!({}),
            &provenance,
            0,
        )
        .unwrap();

        assert_eq!(
            evidence.warnings,
            vec!["engine_commit_unavailable", "data_hash_unavailable"]
        );
    }

    #[test]
    fn runtime_data_hash_is_stable_and_bound_to_runtime_identity() {
        let first = ToolProvenance::from_runtime_data(
            GameVersion::AnYingQianJi,
            Mount::FenShanJin,
            MountConstants::fenshanjin_default(),
            &[],
            &[],
            &[],
            &[],
        );
        let repeated = ToolProvenance::from_runtime_data(
            GameVersion::AnYingQianJi,
            Mount::FenShanJin,
            MountConstants::fenshanjin_default(),
            &[],
            &[],
            &[],
            &[],
        );
        let other_mount = ToolProvenance::from_runtime_data(
            GameVersion::AnYingQianJi,
            Mount::TieGuYi,
            MountConstants::tieguyi_default(),
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(first.data_hash, repeated.data_hash);
        assert_ne!(first.data_hash, other_mount.data_hash);
        assert_eq!(first.data_hash.len(), 64);
    }
}
