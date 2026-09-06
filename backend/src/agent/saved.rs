//! Bounded, read-only access to simulator artifacts saved in the current worker's userdata.
//!
//! The Agent never receives filesystem paths and cannot choose filenames. Public artifact IDs
//! are content-addressed handles rebuilt from the allow-listed catalog on every access.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::{
    Attributes, FormationSelection, GameVersion, Mount, PreReleaseSpec, SimulateRequest,
    TargetConfig, TeamBuffSelection,
};

use super::compare::{configure_macro_rotation, CandidatePatchV1, PatchValueV1, ScenarioPatchV1};
use super::hash::canonical_sha256;
use super::schema::ScenarioSnapshotV1;

pub const LIST_SAVED_ARTIFACTS: &str = "list_saved_artifacts";
pub const READ_SAVED_ARTIFACT: &str = "read_saved_artifact";
pub const COMPARE_SAVED_MACROS: &str = "compare_saved_macros";
pub const COMPARE_SAVED_SCENARIOS: &str = "compare_saved_scenarios";
pub const MAX_SAVED_RESULTS: usize = 24;
const MAX_ARTIFACT_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SavedArtifactKind {
    Macro,
    Loop,
    Equipment,
    Attributes,
    Plaza,
}

impl SavedArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Macro => "macro",
            Self::Loop => "loop",
            Self::Equipment => "equipment",
            Self::Attributes => "attributes",
            Self::Plaza => "plaza",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedArtifactSummary {
    pub artifact_id: String,
    pub kind: SavedArtifactKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedArtifactCatalog {
    pub query: String,
    pub total_matches: usize,
    pub items: Vec<SavedArtifactSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedArtifactDocument {
    pub artifact: SavedArtifactSummary,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedComparisonContext {
    pub comparison_mode: String,
    pub left: SavedArtifactSummary,
    pub right: SavedArtifactSummary,
    pub controlled_environment: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedArtifactError {
    InvalidQuery,
    InvalidKind,
    NotFound,
    Unreadable,
    InvalidFormat,
    UnsupportedComparison,
    ScopeMismatch,
}

impl SavedArtifactError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidQuery => "invalid_saved_query",
            Self::InvalidKind => "invalid_saved_kind",
            Self::NotFound => "saved_artifact_not_found",
            Self::Unreadable => "saved_artifact_unreadable",
            Self::InvalidFormat => "invalid_saved_artifact",
            Self::UnsupportedComparison => "unsupported_saved_comparison",
            Self::ScopeMismatch => "saved_artifact_scope_mismatch",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::InvalidQuery => "saved artifact query is outside configured bounds",
            Self::InvalidKind => "saved artifact kind is not allowed",
            Self::NotFound => "saved artifact id is missing or stale; list artifacts again",
            Self::Unreadable => "saved artifact could not be read within configured bounds",
            Self::InvalidFormat => "saved artifact does not match a supported simulator format",
            Self::UnsupportedComparison => "these saved artifacts do not support this comparison",
            Self::ScopeMismatch => {
                "saved artifact version or mount does not match the current runtime"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SavedArtifactRecord {
    pub summary: SavedArtifactSummary,
    pub content: Value,
    macro_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedSavedComparison {
    pub context: SavedComparisonContext,
    pub baseline: ScenarioSnapshotV1,
    pub candidate: CandidatePatchV1,
}

pub fn list_saved_artifacts(
    root: &Path,
    query: &str,
    kinds: &[SavedArtifactKind],
) -> Result<SavedArtifactCatalog, SavedArtifactError> {
    let query = query.trim();
    if query.chars().count() > 128 || query.chars().any(char::is_control) {
        return Err(SavedArtifactError::InvalidQuery);
    }
    let mut records = scan_catalog(root);
    if !kinds.is_empty() {
        records.retain(|record| kinds.contains(&record.summary.kind));
    }
    if !query.is_empty() {
        let needles = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        records.retain(|record| {
            let haystack = format!(
                "{} {} {} {}",
                record.summary.name,
                record.summary.kind.as_str(),
                record.summary.mount.as_deref().unwrap_or_default(),
                record.summary.game_version.as_deref().unwrap_or_default()
            )
            .to_lowercase();
            needles.iter().all(|needle| haystack.contains(needle))
        });
    }
    records.sort_by(|left, right| {
        right
            .summary
            .updated_at_ms
            .cmp(&left.summary.updated_at_ms)
            .then_with(|| left.summary.name.cmp(&right.summary.name))
    });
    let total_matches = records.len();
    Ok(SavedArtifactCatalog {
        query: query.to_string(),
        total_matches,
        items: records
            .into_iter()
            .take(MAX_SAVED_RESULTS)
            .map(|record| record.summary)
            .collect(),
    })
}

pub fn read_saved_artifact(
    root: &Path,
    artifact_id: &str,
) -> Result<SavedArtifactDocument, SavedArtifactError> {
    let record = resolve(root, artifact_id)?;
    let content = public_artifact_content(
        record.summary.kind,
        &record.content,
        record.macro_text.as_deref(),
    );
    Ok(SavedArtifactDocument {
        artifact: record.summary,
        content,
    })
}

fn public_artifact_content(
    kind: SavedArtifactKind,
    content: &Value,
    macro_text: Option<&str>,
) -> Value {
    if kind != SavedArtifactKind::Macro {
        return content.clone();
    }

    let mode = content
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("general");
    let mut public = serde_json::Map::new();
    for key in ["_mount", "_version", "mode"] {
        if let Some(value) = content.get(key) {
            public.insert(key.to_string(), value.clone());
        }
    }
    public.insert(
        "active_macro_text".to_string(),
        Value::String(macro_text.unwrap_or_default().to_string()),
    );
    if mode == "stance" {
        public.insert("active_sections".to_string(), json!(["shield", "blade"]));
        for key in ["shield", "blade"] {
            if let Some(value) = content.get(key) {
                public.insert(key.to_string(), value.clone());
            }
        }
    } else {
        public.insert("active_sections".to_string(), json!(["general"]));
        if let Some(value) = content.get("general") {
            public.insert("general".to_string(), value.clone());
        }
    }
    Value::Object(public)
}

pub fn prepare_saved_macro_comparison(
    root: &Path,
    left_id: &str,
    right_id: &str,
    current: &ScenarioSnapshotV1,
    version: GameVersion,
    mount: Mount,
) -> Result<PreparedSavedComparison, SavedArtifactError> {
    let left = resolve(root, left_id)?;
    let right = resolve(root, right_id)?;
    ensure_scope(&left, current)?;
    ensure_scope(&right, current)?;
    let left_macro = left
        .macro_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .ok_or(SavedArtifactError::UnsupportedComparison)?;
    let right_macro = right
        .macro_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .ok_or(SavedArtifactError::UnsupportedComparison)?;
    if left.summary.artifact_id == right.summary.artifact_id || left_macro == right_macro {
        return Err(SavedArtifactError::UnsupportedComparison);
    }

    let mut simulation = current.simulation.clone();
    configure_macro_rotation(&mut simulation, left_macro.to_string());
    let baseline = ScenarioSnapshotV1::capture(version, mount, simulation)
        .map_err(|_| SavedArtifactError::InvalidFormat)?;
    let candidate = CandidatePatchV1 {
        label: right.summary.name.clone(),
        patch: ScenarioPatchV1 {
            macro_text: Some(PatchValueV1::Set(right_macro.to_string())),
            ..ScenarioPatchV1::default()
        },
    };
    Ok(PreparedSavedComparison {
        context: SavedComparisonContext {
            comparison_mode: "macro_only_same_frozen_environment".to_string(),
            left: left.summary,
            right: right.summary,
            controlled_environment: vec![
                "attributes".to_string(),
                "equipment".to_string(),
                "talents".to_string(),
                "recipes".to_string(),
                "target".to_string(),
                "network_delay".to_string(),
                "team_buffs".to_string(),
                "formation".to_string(),
            ],
        },
        baseline,
        candidate,
    })
}

pub fn prepare_saved_scenario_comparison(
    root: &Path,
    left_id: &str,
    right_id: &str,
    current: &ScenarioSnapshotV1,
    version: GameVersion,
    mount: Mount,
) -> Result<PreparedSavedComparison, SavedArtifactError> {
    let left = resolve(root, left_id)?;
    let right = resolve(root, right_id)?;
    if !matches!(
        left.summary.kind,
        SavedArtifactKind::Loop | SavedArtifactKind::Plaza
    ) || !matches!(
        right.summary.kind,
        SavedArtifactKind::Loop | SavedArtifactKind::Plaza
    ) || left.summary.artifact_id == right.summary.artifact_id
    {
        return Err(SavedArtifactError::UnsupportedComparison);
    }
    ensure_scope(&left, current)?;
    ensure_scope(&right, current)?;
    let left_simulation = merge_saved_scenario(&left, &current.simulation)?;
    let right_simulation = merge_saved_scenario(&right, &current.simulation)?;
    let baseline = ScenarioSnapshotV1::capture(version, mount, left_simulation)
        .map_err(|_| SavedArtifactError::InvalidFormat)?;
    let candidate = CandidatePatchV1 {
        label: right.summary.name.clone(),
        patch: complete_patch(&right_simulation),
    };
    Ok(PreparedSavedComparison {
        context: SavedComparisonContext {
            comparison_mode: "complete_saved_scenarios".to_string(),
            left: left.summary,
            right: right.summary,
            controlled_environment: vec![
                "current runtime version and mount".to_string(),
                "missing legacy fields inherit the frozen current scenario and are reported as a boundary"
                    .to_string(),
            ],
        },
        baseline,
        candidate,
    })
}

fn resolve(root: &Path, artifact_id: &str) -> Result<SavedArtifactRecord, SavedArtifactError> {
    if artifact_id.len() != 67
        || !artifact_id.starts_with("sa_")
        || !artifact_id[3..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SavedArtifactError::NotFound);
    }
    scan_catalog(root)
        .into_iter()
        .find(|record| record.summary.artifact_id == artifact_id)
        .ok_or(SavedArtifactError::NotFound)
}

fn scan_catalog(root: &Path) -> Vec<SavedArtifactRecord> {
    let mut records = Vec::new();
    add_file_record(
        &mut records,
        root.join("macros.json"),
        SavedArtifactKind::Macro,
        "默认宏".to_string(),
        "macro:default".to_string(),
    );
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if let Some(name) = file_name
                .strip_prefix("macros_")
                .and_then(|value| value.strip_suffix(".json"))
            {
                add_file_record(
                    &mut records,
                    path,
                    SavedArtifactKind::Macro,
                    name.to_string(),
                    format!("macro:{name}"),
                );
            } else if file_name == "attrs.json" {
                add_file_record(
                    &mut records,
                    path,
                    SavedArtifactKind::Attributes,
                    "旧版默认属性".to_string(),
                    "attributes:legacy".to_string(),
                );
            } else if let Some(name) = file_name
                .strip_prefix("attrs_")
                .and_then(|value| value.strip_suffix(".json"))
            {
                add_file_record(
                    &mut records,
                    path,
                    SavedArtifactKind::Attributes,
                    attribute_display_name(name),
                    format!("attributes:{name}"),
                );
            }
        }
    }
    scan_directory(&mut records, root, "loops", SavedArtifactKind::Loop);
    scan_directory(&mut records, root, "equips", SavedArtifactKind::Equipment);
    scan_plaza(&mut records, root);
    records
}

fn scan_directory(
    records: &mut Vec<SavedArtifactRecord>,
    root: &Path,
    directory: &str,
    kind: SavedArtifactKind,
) {
    let dir = root.join(directory);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = read_json(&path) else {
            continue;
        };
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名")
            .to_string();
        let name = content
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&fallback)
            .to_string();
        let locator = format!("{}:{}", kind.as_str(), fallback);
        if let Some(record) = record_from_value(kind, name, locator, content, modified_ms(&path)) {
            records.push(record);
        }
    }
}

fn scan_plaza(records: &mut Vec<SavedArtifactRecord>, root: &Path) {
    let path = root.join("settings.json");
    let Ok(settings) = read_json(&path) else {
        return;
    };
    let Some(raw_store) = settings.get("plaza_builds_v1") else {
        return;
    };
    let store = if let Some(encoded) = raw_store.as_str() {
        serde_json::from_str::<Value>(encoded).ok()
    } else {
        Some(raw_store.clone())
    };
    let Some(builds) = store
        .as_ref()
        .and_then(|value| value.get("builds"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for (index, build) in builds.iter().enumerate() {
        let name = build
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("方案{}", index + 1));
        let build_id = build.get("id").and_then(Value::as_str).unwrap_or("legacy");
        if let Some(record) = record_from_value(
            SavedArtifactKind::Plaza,
            name,
            format!("plaza:{build_id}:{index}"),
            build.clone(),
            build
                .get("updated_at")
                .and_then(Value::as_u64)
                .or_else(|| modified_ms(&path)),
        ) {
            records.push(record);
        }
    }
}

fn add_file_record(
    records: &mut Vec<SavedArtifactRecord>,
    path: PathBuf,
    kind: SavedArtifactKind,
    name: String,
    locator: String,
) {
    let Ok(content) = read_json(&path) else {
        return;
    };
    if let Some(record) = record_from_value(kind, name, locator, content, modified_ms(&path)) {
        records.push(record);
    }
}

fn record_from_value(
    kind: SavedArtifactKind,
    name: String,
    locator: String,
    content: Value,
    updated_at_ms: Option<u64>,
) -> Option<SavedArtifactRecord> {
    let content = sanitize_artifact(kind, &content)?;
    let macro_text = extract_macro_text(kind, &content);
    let capabilities = match kind {
        SavedArtifactKind::Macro => vec!["read", "compare_macro"],
        SavedArtifactKind::Loop => vec!["read", "compare_macro", "compare_scenario"],
        SavedArtifactKind::Plaza => vec!["read", "compare_macro", "compare_scenario"],
        SavedArtifactKind::Equipment | SavedArtifactKind::Attributes => vec!["read"],
    }
    .into_iter()
    .filter(|capability| *capability != "compare_macro" || macro_text.is_some())
    .map(str::to_string)
    .collect::<Vec<_>>();
    let mount = artifact_mount(kind, &content);
    let game_version = artifact_version(kind, &content);
    let content_hash = canonical_sha256(&content).ok()?;
    let artifact_id = canonical_sha256(&json!({
        "kind": kind.as_str(),
        "locator": locator,
        "content_hash": content_hash,
    }))
    .ok()
    .map(|hash| format!("sa_{hash}"))?;
    Some(SavedArtifactRecord {
        summary: SavedArtifactSummary {
            artifact_id,
            kind,
            name,
            mount,
            game_version,
            updated_at_ms,
            capabilities,
        },
        content,
        macro_text,
    })
}

fn sanitize_artifact(kind: SavedArtifactKind, value: &Value) -> Option<Value> {
    let keys: &[&str] = match kind {
        SavedArtifactKind::Macro => &["_mount", "_version", "mode", "general", "shield", "blade"],
        SavedArtifactKind::Loop => &[
            "version",
            "exported_at",
            "target",
            "talents",
            "recipes",
            "sequence",
            "macro",
            "network_delay",
            "initial_rage",
            "boss_attack_interval",
            "macro_duration",
            "hanjia_expectation",
            "tiegu_mode",
            "team_buffs",
            "formation",
        ],
        SavedArtifactKind::Equipment => &[
            "name",
            "mount",
            "created_at",
            "updated_at",
            "stoneId",
            "stoneName",
            "stone_id",
            "stone_name",
            "slots",
        ],
        SavedArtifactKind::Attributes => &[
            "vitality",
            "li_dao",
            "gen_gu",
            "yuan_qi",
            "shen_fa",
            "base_attack",
            "base_magical_attack",
            "weapon_damage",
            "surplus_value",
            "crit_level",
            "crit_effect_level",
            "overcome_level",
            "strain_level",
            "haste_level",
            "parry_value",
            "parry_level",
        ],
        SavedArtifactKind::Plaza => &[
            "id",
            "name",
            "mount",
            "mount_label",
            "version",
            "version_label",
            "equipment",
            "talents",
            "recipes",
            "rotation",
            "attributes",
            "target",
            "team_buffs",
            "formation",
            "snapshot",
            "created_at",
            "updated_at",
        ],
    };
    let source = value.as_object()?;
    let mut safe = serde_json::Map::new();
    for key in keys {
        if let Some(value) = source.get(*key) {
            let value = if kind == SavedArtifactKind::Plaza && *key == "snapshot" {
                sanitize_plaza_snapshot(value)
            } else {
                value.clone()
            };
            safe.insert((*key).to_string(), value);
        }
    }
    Some(Value::Object(safe))
}

fn sanitize_plaza_snapshot(value: &Value) -> Value {
    let Some(source) = value.as_object() else {
        return Value::Null;
    };
    let mut safe = serde_json::Map::new();
    for key in [
        "dps",
        "fight_time",
        "total_damage",
        "skill_count",
        "fingerprint",
        "panel",
        "input_hash",
        "simulated_at",
    ] {
        if let Some(value) = source.get(key) {
            safe.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(safe)
}

fn read_json(path: &Path) -> Result<Value, SavedArtifactError> {
    let metadata = fs::metadata(path).map_err(|_| SavedArtifactError::Unreadable)?;
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(SavedArtifactError::Unreadable);
    }
    let source = fs::read_to_string(path).map_err(|_| SavedArtifactError::Unreadable)?;
    serde_json::from_str(&source).map_err(|_| SavedArtifactError::InvalidFormat)
}

fn modified_ms(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn attribute_display_name(stem: &str) -> String {
    match stem.split_once('_') {
        Some((mount, profile)) => format!("{mount}属性 · {profile}"),
        None => format!("{stem}默认属性"),
    }
}

fn artifact_mount(kind: SavedArtifactKind, value: &Value) -> Option<String> {
    let raw = match kind {
        SavedArtifactKind::Macro => value.get("_mount"),
        SavedArtifactKind::Equipment | SavedArtifactKind::Plaza => value.get("mount"),
        SavedArtifactKind::Loop | SavedArtifactKind::Attributes => None,
    }
    .and_then(Value::as_str)?;
    normalize_mount(raw).map(str::to_string)
}

fn artifact_version(kind: SavedArtifactKind, value: &Value) -> Option<String> {
    let raw = match kind {
        SavedArtifactKind::Macro => value.get("_version"),
        SavedArtifactKind::Plaza => value.get("version"),
        _ => None,
    }
    .and_then(Value::as_str)?;
    normalize_version(raw).map(str::to_string)
}

fn normalize_mount(raw: &str) -> Option<&'static str> {
    match raw {
        "FenShanJin" | "fenshanjin" | "分山劲" => Some("fenshanjin"),
        "TieGuYi" | "tieguyi" | "铁骨衣" => Some("tieguyi"),
        _ => None,
    }
}

fn normalize_version(raw: &str) -> Option<&'static str> {
    match raw {
        "AnYingQianJi" | "2026_04_anying_qianji" => Some("2026_04_anying_qianji"),
        "AnYingQianJiTest" | "2026_04_anying_qianji_test" => Some("2026_04_anying_qianji_test"),
        "ShanHaiYuanLiu" | "2025_10_shanhai_yuanliu" => Some("2025_10_shanhai_yuanliu"),
        _ => None,
    }
}

fn ensure_scope(
    artifact: &SavedArtifactRecord,
    current: &ScenarioSnapshotV1,
) -> Result<(), SavedArtifactError> {
    if artifact
        .summary
        .mount
        .as_deref()
        .is_some_and(|mount| mount != current.mount)
        || artifact
            .summary
            .game_version
            .as_deref()
            .is_some_and(|version| version != current.game_version)
    {
        Err(SavedArtifactError::ScopeMismatch)
    } else {
        Ok(())
    }
}

fn extract_macro_text(kind: SavedArtifactKind, value: &Value) -> Option<String> {
    let macro_value = match kind {
        SavedArtifactKind::Macro => value,
        SavedArtifactKind::Loop => value.get("macro")?,
        SavedArtifactKind::Plaza => value.get("rotation")?.get("macro")?,
        _ => return None,
    };
    macro_object_to_text(macro_value)
}

fn macro_object_to_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.trim().is_empty()).then(|| text.trim().to_string());
    }
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("general");
    if mode == "stance" {
        let shield = value.get("shield").and_then(Value::as_str).unwrap_or("");
        let blade = value.get("blade").and_then(Value::as_str).unwrap_or("");
        let mut text = String::new();
        if !shield.trim().is_empty() {
            text.push_str("#page shield\n");
            text.push_str(shield.trim());
            text.push('\n');
        }
        if !blade.trim().is_empty() {
            text.push_str("#page blade\n");
            text.push_str(blade.trim());
            text.push('\n');
        }
        (!text.trim().is_empty()).then(|| text.trim().to_string())
    } else {
        value
            .get("general")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    }
}

fn merge_saved_scenario(
    record: &SavedArtifactRecord,
    current: &SimulateRequest,
) -> Result<SimulateRequest, SavedArtifactError> {
    let mut simulation = current.clone();
    let (root, rotation) = match record.summary.kind {
        SavedArtifactKind::Loop => (&record.content, &record.content),
        SavedArtifactKind::Plaza => (
            &record.content,
            record
                .content
                .get("rotation")
                .ok_or(SavedArtifactError::InvalidFormat)?,
        ),
        _ => return Err(SavedArtifactError::UnsupportedComparison),
    };

    if let Some(attributes) = root.get("attributes") {
        simulation.attributes = Some(
            serde_json::from_value::<Attributes>(attributes.clone())
                .map_err(|_| SavedArtifactError::InvalidFormat)?,
        );
        if let Some(haste) = simulation
            .attributes
            .as_ref()
            .map(|value| value.haste_level)
        {
            if haste.is_finite() && haste >= 0.0 {
                simulation.haste_level = haste.round() as u32;
            }
        }
    }
    if let Some(target) = root.get("target") {
        simulation.target = Some(
            serde_json::from_value::<TargetConfig>(target.clone())
                .map_err(|_| SavedArtifactError::InvalidFormat)?,
        );
    }
    if let Some(talents) = root.get("talents") {
        simulation.talents = flatten_ids(talents);
    }
    if let Some(recipes) = root.get("recipes") {
        simulation.recipes = flatten_ids(recipes);
    }
    if let Some(equipment) = root.get("equipment") {
        simulation.equipment = equipment_ids(equipment);
    }
    if let Some(team_buffs) = root.get("team_buffs") {
        simulation.team_buffs =
            serde_json::from_value::<Vec<TeamBuffSelection>>(team_buffs.clone())
                .map_err(|_| SavedArtifactError::InvalidFormat)?;
    }
    if root.get("formation").is_some() {
        simulation.formation = if root.get("formation").is_some_and(Value::is_null) {
            None
        } else {
            Some(
                serde_json::from_value::<FormationSelection>(root["formation"].clone())
                    .map_err(|_| SavedArtifactError::InvalidFormat)?,
            )
        };
    }

    let delay = rotation
        .get("network_delay")
        .or_else(|| rotation.get("delay"))
        .and_then(Value::as_u64);
    if let Some(delay) = delay {
        simulation.network_delay = delay
            .try_into()
            .map_err(|_| SavedArtifactError::InvalidFormat)?;
    }
    if let Some(value) = rotation.get("initial_rage") {
        simulation.initial_rage = value.as_i64().and_then(|rage| rage.try_into().ok());
    }
    if let Some(value) = rotation.get("boss_attack_interval").and_then(Value::as_f64) {
        simulation.boss_attack_interval = Some(value);
    }
    if let Some(value) = rotation.get("macro_duration").and_then(Value::as_f64) {
        simulation.macro_duration = Some(value);
    }
    if let Some(value) = root.get("hanjia_expectation").and_then(Value::as_bool) {
        simulation.hanjia_expectation = Some(value);
    }
    if let Some(value) = root.get("tiegu_mode").and_then(Value::as_u64) {
        simulation.tiegu_mode = value
            .try_into()
            .map_err(|_| SavedArtifactError::InvalidFormat)?;
    }

    let sequence_value = rotation.get("sequence").or_else(|| root.get("sequence"));
    if let Some(sequence_value) = sequence_value {
        let flattened = flatten_sequence(sequence_value)?;
        simulation.sequence = flattened.sequence;
        simulation.channel_ticks = flattened.channel_ticks;
        simulation.timing_offsets = flattened.timing_offsets;
        simulation.qijin_buffs = flattened.qijin_buffs;
        simulation.pre_releases = flattened.pre_releases;
    }
    simulation.macro_text = record.macro_text.clone();
    if simulation.macro_text.is_some() {
        if simulation.sequence.is_empty() {
            configure_macro_rotation(
                &mut simulation,
                record.macro_text.clone().expect("checked macro"),
            );
        } else if simulation.sequence.iter().all(|value| value == "__macro__") {
            let macro_text = record.macro_text.clone().expect("checked macro");
            configure_macro_rotation(&mut simulation, macro_text);
        }
    }
    Ok(simulation)
}

#[derive(Default)]
struct FlattenedSequence {
    sequence: Vec<String>,
    channel_ticks: HashMap<String, u32>,
    timing_offsets: HashMap<String, f64>,
    qijin_buffs: HashMap<String, u32>,
    pre_releases: Vec<PreReleaseSpec>,
}

fn flatten_sequence(value: &Value) -> Result<FlattenedSequence, SavedArtifactError> {
    let entries = value.as_array().ok_or(SavedArtifactError::InvalidFormat)?;
    let mut output = FlattenedSequence::default();
    for entry in entries {
        let count = entry
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .clamp(1, 10_000);
        for _ in 0..count {
            let kind = entry.get("type").and_then(Value::as_str).unwrap_or("skill");
            if kind == "pre_release" {
                let skill = entry
                    .get("skill")
                    .and_then(Value::as_str)
                    .ok_or(SavedArtifactError::InvalidFormat)?;
                output.pre_releases.push(PreReleaseSpec {
                    skill: skill.to_string(),
                    time_before: entry.get("pre_time").and_then(Value::as_f64).unwrap_or(5.0),
                });
                continue;
            }
            if kind == "break" {
                continue;
            }
            let token = match kind {
                "macro" => "__macro__".to_string(),
                "wait_stance" => "__切体态延迟中__".to_string(),
                "wait_zhan_jue" => "__战绝回怒__".to_string(),
                "skill" => entry
                    .get("skill")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or(SavedArtifactError::InvalidFormat)?
                    .to_string(),
                _ => return Err(SavedArtifactError::InvalidFormat),
            };
            let index = output.sequence.len().to_string();
            if let Some(ticks) = entry.get("channel_ticks").and_then(Value::as_u64) {
                output.channel_ticks.insert(
                    index.clone(),
                    ticks
                        .try_into()
                        .map_err(|_| SavedArtifactError::InvalidFormat)?,
                );
            }
            if entry.get("offset_max").and_then(Value::as_bool) == Some(true) {
                output.timing_offsets.insert(index.clone(), -1.0);
            } else if let Some(offset) = entry.get("offset").and_then(Value::as_f64) {
                output.timing_offsets.insert(index.clone(), offset);
            }
            if let Some(buff) = entry.get("qijin_buff").and_then(Value::as_u64) {
                output.qijin_buffs.insert(
                    index,
                    buff.try_into()
                        .map_err(|_| SavedArtifactError::InvalidFormat)?,
                );
            }
            output.sequence.push(token);
        }
    }
    Ok(output)
}

fn flatten_ids(value: &Value) -> Vec<u32> {
    let mut ids = Vec::new();
    collect_ids(value, &mut ids);
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn collect_ids(value: &Value, ids: &mut Vec<u32>) {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64().and_then(|value| value.try_into().ok()) {
                if value != 0 {
                    ids.push(value);
                }
            }
        }
        Value::Array(values) => values.iter().for_each(|value| collect_ids(value, ids)),
        Value::Object(values) => values.values().for_each(|value| collect_ids(value, ids)),
        _ => {}
    }
}

fn equipment_ids(value: &Value) -> HashMap<String, u32> {
    let slots = value.get("slots").unwrap_or(value);
    slots
        .as_object()
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter_map(|(position, item)| {
            item.get("equip_id")
                .and_then(Value::as_u64)
                .and_then(|value| value.try_into().ok())
                .filter(|value| *value != 0)
                .map(|id| (position.clone(), id))
        })
        .collect()
}

fn complete_patch(simulation: &SimulateRequest) -> ScenarioPatchV1 {
    ScenarioPatchV1 {
        haste_level: Some(simulation.haste_level),
        sequence: Some(simulation.sequence.clone()),
        talents: Some(simulation.talents.clone()),
        channel_ticks: Some(simulation.channel_ticks.clone()),
        timing_offsets: Some(simulation.timing_offsets.clone()),
        network_delay: Some(simulation.network_delay),
        recipes: Some(simulation.recipes.clone()),
        qijin_buffs: Some(simulation.qijin_buffs.clone()),
        macro_text: Some(match &simulation.macro_text {
            Some(value) => PatchValueV1::Set(value.clone()),
            None => PatchValueV1::Clear,
        }),
        macro_duration: Some(match simulation.macro_duration {
            Some(value) => PatchValueV1::Set(value),
            None => PatchValueV1::Clear,
        }),
        attributes: simulation.attributes.clone(),
        target: simulation.target.clone(),
        initial_rage: Some(match simulation.initial_rage {
            Some(value) => PatchValueV1::Set(value),
            None => PatchValueV1::Clear,
        }),
        pauses: Some(simulation.pauses.clone()),
        boss_attack_interval: Some(match simulation.boss_attack_interval {
            Some(value) => PatchValueV1::Set(value),
            None => PatchValueV1::Clear,
        }),
        hanjia_expectation: Some(match simulation.hanjia_expectation {
            Some(value) => PatchValueV1::Set(value),
            None => PatchValueV1::Clear,
        }),
        tiegu_mode: Some(simulation.tiegu_mode),
        experimental: Some(simulation.experimental),
        equipment: Some(simulation.equipment.clone()),
        team_buffs: Some(simulation.team_buffs.clone()),
        formation: Some(match &simulation.formation {
            Some(value) => PatchValueV1::Set(value.clone()),
            None => PatchValueV1::Clear,
        }),
        pre_releases: Some(simulation.pre_releases.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_root() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("jx3-agent-saved-{}-{}", std::process::id(), nonce));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("loops")).unwrap();
        fs::create_dir_all(root.join("equips")).unwrap();
        root
    }

    #[test]
    fn catalog_exposes_opaque_ids_without_settings_or_paths() {
        let root = fixture_root();
        fs::write(
            root.join("macros_alpha.json"),
            r#"{"_mount":"FenShanJin","mode":"general","general":"/cast 盾击"}"#,
        )
        .unwrap();
        fs::write(
            root.join("settings.json"),
            r#"{"openai_api_key":"secret","ui_theme":"light"}"#,
        )
        .unwrap();
        let catalog = list_saved_artifacts(&root, "alpha", &[SavedArtifactKind::Macro]).unwrap();
        assert_eq!(catalog.items.len(), 1);
        assert!(catalog.items[0].artifact_id.starts_with("sa_"));
        let encoded = serde_json::to_string(&catalog).unwrap();
        assert!(!encoded.contains("settings"));
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains(root.to_string_lossy().as_ref()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stance_macro_is_restored_with_page_markers() {
        let value = json!({
            "mode": "stance",
            "shield": "/cast 盾击",
            "blade": "/cast 绝刀"
        });
        assert_eq!(
            macro_object_to_text(&value).unwrap(),
            "#page shield\n/cast 盾击\n#page blade\n/cast 绝刀"
        );
    }

    #[test]
    fn general_macro_reader_hides_inactive_stance_storage_fields() {
        let root = fixture_root();
        fs::write(
            root.join("macros_helper.json"),
            r#"{"_mount":"FenShanJin","mode":"general","general":"/cast 盾击","shield":"/cast 盾猛","blade":"/cast 绝刀"}"#,
        )
        .unwrap();
        let catalog = list_saved_artifacts(&root, "helper", &[SavedArtifactKind::Macro]).unwrap();
        let document = read_saved_artifact(&root, &catalog.items[0].artifact_id).unwrap();

        assert_eq!(document.content["mode"], "general");
        assert_eq!(document.content["active_sections"], json!(["general"]));
        assert_eq!(document.content["active_macro_text"], "/cast 盾击");
        assert!(document.content.get("shield").is_none());
        assert!(document.content.get("blade").is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn plaza_reader_only_extracts_builds_from_settings() {
        let root = fixture_root();
        let store = json!({"version": 1, "builds": [{
            "id": "b1", "name": "毕业方案", "mount": "FenShanJin",
            "version": "AnYingQianJi", "rotation": {"macro": {"mode":"general", "general":"/cast 盾击"}},
            "unexpected_secret": "nested-must-not-leak"
        }]});
        fs::write(
            root.join("settings.json"),
            serde_json::to_string(&json!({
                "plaza_builds_v1": serde_json::to_string(&store).unwrap(),
                "password": "must-not-leak"
            }))
            .unwrap(),
        )
        .unwrap();
        let catalog = list_saved_artifacts(&root, "毕业", &[SavedArtifactKind::Plaza]).unwrap();
        assert_eq!(catalog.items.len(), 1);
        let document = read_saved_artifact(&root, &catalog.items[0].artifact_id).unwrap();
        let encoded = serde_json::to_string(&document).unwrap();
        assert!(!encoded.contains("must-not-leak"));
        assert!(!encoded.contains("nested-must-not-leak"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_macro_comparison_runs_both_macros_in_one_frozen_environment() {
        let root = fixture_root();
        for (name, body) in [
            ("alpha", "/cast 盾击\n/cast 盾猛"),
            ("beta", "/cast 盾猛\n/cast 盾击"),
        ] {
            fs::write(
                root.join(format!("macros_{name}.json")),
                serde_json::to_string(&json!({
                    "_mount": "FenShanJin",
                    "_version": "AnYingQianJi",
                    "mode": "general",
                    "general": body
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let catalog = list_saved_artifacts(&root, "", &[SavedArtifactKind::Macro]).unwrap();
        let left = catalog
            .items
            .iter()
            .find(|item| item.name == "alpha")
            .unwrap();
        let right = catalog
            .items
            .iter()
            .find(|item| item.name == "beta")
            .unwrap();
        let runtime = crate::agent::AgentRuntime::fixture();
        let current = runtime.fixture_scenario();
        let prepared = prepare_saved_macro_comparison(
            &root,
            &left.artifact_id,
            &right.artifact_id,
            &current,
            runtime.game_version(),
            runtime.mount(),
        )
        .unwrap();
        assert_eq!(
            prepared.context.comparison_mode,
            "macro_only_same_frozen_environment"
        );
        assert_eq!(
            prepared
                .baseline
                .simulation
                .attributes
                .as_ref()
                .map(|value| value.base_attack),
            current
                .simulation
                .attributes
                .as_ref()
                .map(|value| value.base_attack)
        );
        assert_eq!(
            prepared.baseline.simulation.equipment,
            current.simulation.equipment
        );
        let context = runtime.context();
        let mut budget = crate::agent::ToolBudget::new(2);
        let execution = crate::agent::compare_scenarios(
            "saved-macro-test",
            &prepared.baseline,
            &[prepared.candidate],
            &context,
            runtime.provenance(),
            &mut budget,
        )
        .unwrap();
        assert_eq!(budget.used_simulations, 2);
        assert_eq!(execution.evidence.result.candidates.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
