//! Versioned experiment protocol. The model chooses actions; this contract bounds effects.
use crate::{
    agent::{session::contains_likely_secret, ScenarioSnapshotV1},
    GameVersion, Mount, SimulateRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const RUNTIME_VERSION: &str = "pve-experiment-runtime/v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunBudget {
    pub max_model_calls: u32,
    pub max_simulations: u32,
    pub wall_time_ms: u64,
    pub max_output_tokens: u32,
    pub max_total_tokens: u64,
}
impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_model_calls: 16,
            max_simulations: 192,
            wall_time_ms: 240_000,
            max_output_tokens: 8192,
            max_total_tokens: 192_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunConstraints {
    pub max_pages: usize,
    pub duration_seconds: f64,
    pub allowed_skills: Vec<String>,
    pub locked_slots: Vec<String>,
    pub candidate_source: String,
    pub candidate_ids: HashMap<String, Vec<u32>>,
    pub min_item_level: Option<u32>,
    pub max_item_level: Option<u32>,
    pub allowed_sources: Vec<String>,
    pub haste_min: Option<u32>,
    pub haste_max: Option<u32>,
    pub max_candidates_per_slot: usize,
}
impl Default for RunConstraints {
    fn default() -> Self {
        Self {
            max_pages: 2,
            duration_seconds: 120.0,
            allowed_skills: vec![],
            locked_slots: vec![],
            candidate_source: "catalog".into(),
            candidate_ids: HashMap::new(),
            min_item_level: None,
            max_item_level: None,
            allowed_sources: vec![],
            haste_min: None,
            haste_max: None,
            max_candidates_per_slot: 12,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub goal: String,
    #[serde(default)]
    pub provider_profile: String,
    pub simulation: SimulateRequest,
    pub version: GameVersion,
    pub mount: Mount,
    #[serde(default)]
    pub equipment: Option<super::equipment::EquipmentSnapshot>,
    #[serde(default)]
    pub constraints: RunConstraints,
    #[serde(default)]
    pub budget: RunBudget,
}

impl RunRequest {
    pub fn validate(&self) -> Result<(), String> {
        if contains_secret_value(&serde_json::to_value(self).map_err(|_| "实验输入无法序列化")?)
        {
            return Err("实验输入包含疑似凭据，请只在专用模型设置中填写 Key。".into());
        }
        if self.goal.trim().is_empty()
            || self.goal.len() > 8192
            || contains_likely_secret(&self.goal)
        {
            return Err("请输入目标（最多 8192 字节），不要在目标中填写凭据。".into());
        }
        let b = &self.budget;
        if !(1..=32).contains(&b.max_model_calls)
            || !(4..=1024).contains(&b.max_simulations)
            || !(10_000..=900_000).contains(&b.wall_time_ms)
            || !(1024..=8192).contains(&b.max_output_tokens)
            || !(4096..=256_000).contains(&b.max_total_tokens)
        {
            return Err("模型、模拟或时间预算超出范围。".into());
        }
        let c = &self.constraints;
        if !(1..=6).contains(&c.max_pages)
            || !c.duration_seconds.is_finite()
            || !(10.0..=600.0).contains(&c.duration_seconds)
            || c.allowed_skills.len() > 64
            || c.allowed_skills
                .iter()
                .any(|s| s.is_empty() || s.len() > 128)
            || c.locked_slots.len() > 16
            || c.locked_slots.iter().any(|s| s.len() > 64)
            || !["catalog", "provided_ids"].contains(&c.candidate_source.as_str())
            || c.candidate_ids.len() > 16
            || c.candidate_ids.values().any(|ids| ids.len() > 256)
            || c.allowed_sources.len() > 32
            || c.allowed_sources.iter().any(|s| s.len() > 128)
            || !(1..=32).contains(&c.max_candidates_per_slot)
            || c.min_item_level
                .zip(c.max_item_level)
                .is_some_and(|(a, b)| a > b)
            || c.haste_min.zip(c.haste_max).is_some_and(|(a, b)| a > b)
        {
            return Err("实验约束超出范围或上下限矛盾。".into());
        }
        if self.provider_profile.len() > 64 {
            return Err("模型配置标识无效。".into());
        }
        validate_environment(&self.simulation, self.version, self.mount)?;
        Ok(())
    }
    pub fn scenario(&self) -> Result<ScenarioSnapshotV1, String> {
        ScenarioSnapshotV1::capture(self.version, self.mount, self.simulation.clone())
            .map_err(|e| e.to_string())
    }
}

pub fn validate_environment(
    sim: &SimulateRequest,
    version: GameVersion,
    mount: Mount,
) -> Result<(), String> {
    let mut probe = sim.clone();
    let initial_macro = probe.macro_text.take().filter(|s| !s.trim().is_empty());
    if initial_macro.is_some() {
        probe.sequence = vec!["环境校验".into()];
    }
    super::contract::MacroCompileRequestV1 {
        simulation: probe,
        version,
        mount,
        initial_macro,
        max_simulations: 2,
        wall_time_ms: 1000,
        max_rounds: 1,
        max_pages: 6,
        time_tolerance: 1.0 / 16.0,
    }
    .validate()
    .map_err(str::to_owned)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunUsage {
    pub model_calls: u32,
    pub simulations: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub last_input_tokens: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub sequence: u64,
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub scenario_hash: String,
    pub request_hash: String,
    pub summary: String,
    pub result: Value,
    pub simulation: SimulateRequest,
    pub equipment: Option<super::equipment::EquipmentSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub summary: String,
    pub selected_artifact_id: Option<String>,
    pub evidence_ids: Vec<String>,
    pub completion: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub schema_version: String,
    pub run_id: String,
    pub sequence: u64,
    pub status: String,
    pub phase: String,
    pub message: String,
    pub request: RunRequest,
    pub scenario: ScenarioSnapshotV1,
    pub runtime_hash: String,
    pub experiment_hash: String,
    pub model: String,
    pub usage: RunUsage,
    pub events: Vec<RunEvent>,
    pub artifacts: Vec<Artifact>,
    pub attempts: HashMap<String, Value>,
    pub result: Option<RunResult>,
    pub persistence_error: bool,
    pub reserved_simulations: u32,
}

pub fn public_text(text: &str, limit: usize) -> String {
    if contains_likely_secret(text) {
        return "[疑似凭据内容已隐藏]".into();
    }
    text.chars().take(limit).collect()
}

pub fn contains_secret_value(value: &Value) -> bool {
    match value {
        Value::String(text) => contains_likely_secret(text),
        Value::Array(items) => items.iter().any(contains_secret_value),
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| contains_likely_secret(key) || contains_secret_value(value)),
        _ => false,
    }
}
