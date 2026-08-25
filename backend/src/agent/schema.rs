use serde::{Deserialize, Serialize};
use std::fmt;

use crate::{GameVersion, Mount, SimulateRequest};

use super::hash::canonical_sha256;

pub const SCENARIO_SCHEMA_V1: &str = "agent-scenario/v1";

/// Immutable snapshot captured when an Agent analysis starts.
///
/// The frontend supplies the complete simulation request while the worker
/// binds its current version and mount. `scenario_hash` identifies gameplay
/// inputs; transport-only lite flags are deliberately excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSnapshotV1 {
    pub schema_version: String,
    pub game_version: String,
    pub mount: String,
    pub simulation: SimulateRequest,
    pub scenario_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioError {
    MissingField(&'static str),
    InvalidField(&'static str),
    Serialization(String),
    HashMismatch { expected: String, actual: String },
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing required scenario field: {field}"),
            Self::InvalidField(field) => write!(f, "invalid scenario field: {field}"),
            Self::Serialization(error) => write!(f, "cannot canonicalize scenario: {error}"),
            Self::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "scenario hash mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ScenarioError {}

#[derive(Serialize)]
struct ScenarioHashPayload<'a> {
    schema_version: &'static str,
    game_version: &'a str,
    mount: &'a str,
    simulation: &'a SimulateRequest,
}

impl ScenarioSnapshotV1 {
    pub fn capture(
        game_version: GameVersion,
        mount: Mount,
        mut simulation: SimulateRequest,
    ) -> Result<Self, ScenarioError> {
        validate_simulation(&simulation)?;

        // These only control response size. They must not create a different
        // gameplay identity for the same scenario.
        simulation.lite = false;
        simulation.lite_keep_timeline = false;

        let game_version = game_version_id(game_version).to_string();
        let mount = mount_id(mount).to_string();
        let scenario_hash = hash_payload(&game_version, &mount, &simulation)?;

        Ok(Self {
            schema_version: SCENARIO_SCHEMA_V1.to_string(),
            game_version,
            mount,
            simulation,
            scenario_hash,
        })
    }

    pub fn verify_hash(&self) -> Result<(), ScenarioError> {
        if self.schema_version != SCENARIO_SCHEMA_V1 {
            return Err(ScenarioError::InvalidField("schema_version"));
        }
        validate_identity(&self.game_version, &self.mount)?;
        validate_simulation(&self.simulation)?;
        let actual = hash_payload(&self.game_version, &self.mount, &self.simulation)?;
        if actual == self.scenario_hash {
            Ok(())
        } else {
            Err(ScenarioError::HashMismatch {
                expected: self.scenario_hash.clone(),
                actual,
            })
        }
    }
}

fn validate_identity(game_version: &str, mount: &str) -> Result<(), ScenarioError> {
    if !matches!(
        game_version,
        "2025_10_shanhai_yuanliu" | "2026_04_anying_qianji" | "2026_04_anying_qianji_test"
    ) {
        return Err(ScenarioError::InvalidField("game_version"));
    }
    if !matches!(mount, "fenshanjin" | "tieguyi") {
        return Err(ScenarioError::InvalidField("mount"));
    }
    Ok(())
}

fn hash_payload(
    game_version: &str,
    mount: &str,
    simulation: &SimulateRequest,
) -> Result<String, ScenarioError> {
    let mut normalized = simulation.clone();
    normalized.lite = false;
    normalized.lite_keep_timeline = false;
    canonical_sha256(&ScenarioHashPayload {
        schema_version: SCENARIO_SCHEMA_V1,
        game_version,
        mount,
        simulation: &normalized,
    })
    .map_err(|error| ScenarioError::Serialization(error.to_string()))
}

fn validate_simulation(simulation: &SimulateRequest) -> Result<(), ScenarioError> {
    if simulation.attributes.is_none() {
        return Err(ScenarioError::MissingField("simulation.attributes"));
    }
    if simulation.target.is_none() {
        return Err(ScenarioError::MissingField("simulation.target"));
    }

    let has_macro = simulation
        .macro_text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if simulation.sequence.is_empty() && !has_macro {
        return Err(ScenarioError::MissingField(
            "simulation.sequence_or_macro_text",
        ));
    }
    if simulation
        .macro_duration
        .is_some_and(|duration| !duration.is_finite() || duration <= 0.0)
    {
        return Err(ScenarioError::InvalidField("simulation.macro_duration"));
    }
    Ok(())
}

fn game_version_id(version: GameVersion) -> &'static str {
    match version {
        GameVersion::ShanHaiYuanLiu => "2025_10_shanhai_yuanliu",
        GameVersion::AnYingQianJi => "2026_04_anying_qianji",
        GameVersion::AnYingQianJiTest => "2026_04_anying_qianji_test",
    }
}

fn mount_id(mount: Mount) -> &'static str {
    match mount {
        Mount::FenShanJin => "fenshanjin",
        Mount::TieGuYi => "tieguyi",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Attributes, TargetConfig};
    use std::collections::HashMap;

    fn request() -> SimulateRequest {
        SimulateRequest {
            haste_level: 0,
            sequence: vec!["盾击".to_string(), "盾压".to_string()],
            talents: Vec::new(),
            channel_ticks: HashMap::new(),
            timing_offsets: HashMap::new(),
            network_delay: 0,
            recipes: Vec::new(),
            qijin_buffs: HashMap::new(),
            macro_text: None,
            macro_duration: None,
            attributes: Some(Attributes::default()),
            target: Some(TargetConfig {
                level: 133,
                defense_bonus: 0.0,
                damage_cof: 0.0,
            }),
            initial_rage: None,
            pauses: Vec::new(),
            boss_attack_interval: None,
            hanjia_expectation: None,
            tiegu_mode: 2,
            experimental: false,
            lite: false,
            lite_keep_timeline: false,
            equipment: HashMap::new(),
            team_buffs: Vec::new(),
            formation: None,
            pre_releases: Vec::new(),
        }
    }

    #[test]
    fn capture_is_stable_and_verifiable() {
        let first =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();
        let second =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();

        assert_eq!(first.scenario_hash, second.scenario_hash);
        assert_eq!(first.scenario_hash.len(), 64);
        first.verify_hash().unwrap();
    }

    #[test]
    fn transport_flags_do_not_change_scenario_identity() {
        let normal = request();
        let mut lite = request();
        lite.lite = true;
        lite.lite_keep_timeline = true;

        let normal =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, normal)
                .unwrap();
        let lite = ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, lite)
            .unwrap();

        assert_eq!(normal.scenario_hash, lite.scenario_hash);
        assert!(!lite.simulation.lite);
        assert!(!lite.simulation.lite_keep_timeline);
    }

    #[test]
    fn map_insertion_order_does_not_change_hash() {
        let mut left = request();
        left.equipment.insert("HAT".into(), 10);
        left.equipment.insert("PRIMARY_WEAPON".into(), 20);

        let mut right = request();
        right.equipment.insert("PRIMARY_WEAPON".into(), 20);
        right.equipment.insert("HAT".into(), 10);

        let left = ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, left)
            .unwrap();
        let right =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, right)
                .unwrap();

        assert_eq!(left.scenario_hash, right.scenario_hash);
    }

    #[test]
    fn gameplay_version_and_mount_changes_change_hash() {
        let base =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();

        let mut changed_request = request();
        changed_request.network_delay = 100;
        let changed = ScenarioSnapshotV1::capture(
            GameVersion::AnYingQianJi,
            Mount::FenShanJin,
            changed_request,
        )
        .unwrap();
        let old_version =
            ScenarioSnapshotV1::capture(GameVersion::ShanHaiYuanLiu, Mount::FenShanJin, request())
                .unwrap();
        let tank =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::TieGuYi, request())
                .unwrap();

        assert_ne!(base.scenario_hash, changed.scenario_hash);
        assert_ne!(base.scenario_hash, old_version.scenario_hash);
        assert_ne!(base.scenario_hash, tank.scenario_hash);
    }

    #[test]
    fn verify_hash_detects_mutation() {
        let mut snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();
        snapshot.simulation.network_delay = 25;

        assert!(matches!(
            snapshot.verify_hash(),
            Err(ScenarioError::HashMismatch { .. })
        ));
    }

    #[test]
    fn serialized_snapshot_round_trips_and_rejects_unknown_identity() {
        let snapshot =
            ScenarioSnapshotV1::capture(GameVersion::AnYingQianJi, Mount::FenShanJin, request())
                .unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let mut decoded: ScenarioSnapshotV1 = serde_json::from_str(&encoded).unwrap();
        decoded.verify_hash().unwrap();

        decoded.game_version = "future_version".to_string();
        assert_eq!(
            decoded.verify_hash().unwrap_err(),
            ScenarioError::InvalidField("game_version")
        );
    }

    #[test]
    fn capture_rejects_incomplete_or_invalid_scenarios() {
        let mut missing_attributes = request();
        missing_attributes.attributes = None;
        assert_eq!(
            ScenarioSnapshotV1::capture(
                GameVersion::AnYingQianJi,
                Mount::FenShanJin,
                missing_attributes,
            )
            .unwrap_err(),
            ScenarioError::MissingField("simulation.attributes")
        );

        let mut missing_rotation = request();
        missing_rotation.sequence.clear();
        assert_eq!(
            ScenarioSnapshotV1::capture(
                GameVersion::AnYingQianJi,
                Mount::FenShanJin,
                missing_rotation,
            )
            .unwrap_err(),
            ScenarioError::MissingField("simulation.sequence_or_macro_text")
        );

        let mut invalid_duration = request();
        invalid_duration.macro_duration = Some(0.0);
        assert_eq!(
            ScenarioSnapshotV1::capture(
                GameVersion::AnYingQianJi,
                Mount::FenShanJin,
                invalid_duration,
            )
            .unwrap_err(),
            ScenarioError::InvalidField("simulation.macro_duration")
        );
    }
}
