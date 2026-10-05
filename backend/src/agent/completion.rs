//! Objective completion checks over already registered tool evidence.
//! These checks neither execute tools nor decide whether an explanation answers
//! the user's intent. A passed check always retains the semantic-review boundary.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::evidence::{EvidenceEnvelopeV1, EVIDENCE_SCHEMA_V1};
use super::report::{AgentReportContentV1, EvidenceStore};
use super::schema::ScenarioSnapshotV1;

pub const TASK_COMPLETION_SCHEMA_V1: &str = "agent-task-completion/v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TaskAcceptanceV1 {
    pub min_comparison_candidates: u8,
    pub require_macro_artifact: bool,
    pub require_simulation: bool,
}

impl TaskAcceptanceV1 {
    pub fn is_empty(&self) -> bool {
        self.min_comparison_candidates == 0
            && !self.require_macro_artifact
            && !self.require_simulation
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.min_comparison_candidates > 8 {
            Err("min_comparison_candidates must be within 0..=8")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskCompletionV1 {
    pub schema_version: String,
    pub objective: String,
    pub status: String,
    pub semantic_review_required: bool,
    pub checks: Vec<TaskCompletionCheckV1>,
    pub artifacts: Vec<TaskArtifactVerificationV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskCompletionCheckV1 {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskArtifactVerificationV1 {
    pub title: String,
    pub language: String,
    pub content_sha256: String,
    pub status: String,
    pub evidence_ids: Vec<String>,
    pub detail: String,
}

struct TestedMacro {
    text: String,
    evidence_id: String,
    detail: String,
}

#[derive(Default)]
struct Observations {
    simulations: BTreeSet<String>,
    candidates: BTreeMap<String, BTreeSet<String>>,
    macros: Vec<TestedMacro>,
}

pub fn assess(
    objective: &str,
    scenario: &ScenarioSnapshotV1,
    content: &AgentReportContentV1,
    evidence: &EvidenceStore,
    acceptance: Option<&TaskAcceptanceV1>,
) -> TaskCompletionV1 {
    let observed = observe(scenario, evidence);
    let mut checks = Vec::new();
    if let Some(acceptance) = acceptance {
        if let Err(detail) = acceptance.validate() {
            checks.push(check(
                "acceptance",
                "验收条件",
                false,
                detail.into(),
                Vec::new(),
            ));
        }
        if acceptance.min_comparison_candidates > 0 {
            let count = observed.candidates.len();
            let ids = observed
                .candidates
                .values()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>();
            checks.push(check(
                "comparison_candidates", "同场景候选对照",
                count >= usize::from(acceptance.min_comparison_candidates),
                format!("已核对 {count} 个不同候选场景，需要至少 {} 个；基线、重复候选和没有实际施放的记录不计入。", acceptance.min_comparison_candidates),
                ids.into_iter().collect(),
            ));
        }
        if acceptance.require_simulation {
            checks.push(check(
                "simulation",
                "实际模拟证据",
                !observed.simulations.is_empty(),
                if observed.simulations.is_empty() {
                    "未找到当前场景下含有效指纹及实际施放的模拟证据；读取场景不算模拟。"
                } else {
                    "当前场景存在有效指纹及实际施放记录；这里只确认模拟发生，不评价方案优劣。"
                }
                .into(),
                observed.simulations.iter().cloned().collect(),
            ));
        }
    }

    let mut artifacts = Vec::new();
    let mut valid_macro_count = 0;
    for (index, artifact) in content.artifacts.iter().enumerate() {
        let is_macro = is_macro(&artifact.language, &artifact.content);
        let (status, detail, ids) = if !is_macro {
            (
                "not_executable",
                "该交付物不属于本地宏执行契约，需人工审阅其内容。".into(),
                Vec::new(),
            )
        } else if crate::macro_parser::parse_macro_text(&artifact.content).is_err() {
            // Never accept the model-provided `syntax` field as a parser result.
            (
                "invalid_syntax",
                "完整宏未通过本地语法解析；保留原文供修订。".into(),
                Vec::new(),
            )
        } else {
            valid_macro_count += 1;
            let normalized = normalize_macro(&artifact.content);
            let matches = observed
                .macros
                .iter()
                .filter(|tested| tested.text == normalized)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                (
                    "not_tested",
                    "未找到与最终完整宏文本一致的同场景运行证据；语法通过不表示已经模拟。".into(),
                    Vec::new(),
                )
            } else {
                let ids = matches
                    .iter()
                    .map(|tested| tested.evidence_id.clone())
                    .collect::<BTreeSet<_>>();
                let details = matches
                    .iter()
                    .map(|tested| tested.detail.as_str())
                    .collect::<BTreeSet<_>>();
                (
                    "simulation_matched",
                    details.into_iter().collect::<Vec<_>>().join("；"),
                    ids.into_iter().collect(),
                )
            }
        };
        if is_macro {
            checks.push(check(
                &format!("macro_artifact_{}", index + 1),
                &format!("最终宏：{}", artifact.title),
                status == "simulation_matched",
                detail.clone(),
                ids.clone(),
            ));
        }
        artifacts.push(TaskArtifactVerificationV1 {
            title: artifact.title.clone(),
            language: artifact.language.clone(),
            content_sha256: format!("{:x}", Sha256::digest(artifact.content.as_bytes())),
            status: status.into(),
            evidence_ids: ids,
            detail,
        });
    }
    if acceptance.is_some_and(|acceptance| acceptance.require_macro_artifact) {
        checks.push(check(
            "macro_artifact",
            "完整宏交付物",
            valid_macro_count > 0,
            if valid_macro_count > 0 {
                "已交付可解析的完整宏；每份最终宏的模拟匹配另行核验。"
            } else {
                "尚未交付可解析的完整宏；正文讨论或历史候选不能代替最终交付物。"
            }
            .into(),
            Vec::new(),
        ));
    }
    let status = if checks.iter().any(|item| item.status == "missing") {
        "checks_incomplete"
    } else if checks.is_empty() {
        checks.push(TaskCompletionCheckV1 {
            id: "semantic_review".into(),
            label: "任务内容审阅".into(),
            status: "needs_review".into(),
            detail: "本轮没有客观执行验收项；定性分析可以直接交付，是否充分回答目标仍需审阅。"
                .into(),
            evidence_ids: Vec::new(),
        });
        "needs_review"
    } else {
        "checks_passed"
    };
    TaskCompletionV1 {
        schema_version: TASK_COMPLETION_SCHEMA_V1.into(),
        objective: public_objective(objective),
        status: status.into(),
        semantic_review_required: true,
        checks,
        artifacts,
    }
}

fn public_objective(objective: &str) -> String {
    // Match the run API's maximum question byte size while keeping UTF-8 whole.
    const MAX_OBJECTIVE_BYTES: usize = 16 * 1024;
    let mut output = super::session::redact_sensitive_text(objective);
    if output.len() > MAX_OBJECTIVE_BYTES {
        let mut end = MAX_OBJECTIVE_BYTES - "…".len();
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push('…');
    }
    output
}

fn check(
    id: &str,
    label: &str,
    passed: bool,
    detail: String,
    evidence_ids: Vec<String>,
) -> TaskCompletionCheckV1 {
    TaskCompletionCheckV1 {
        id: id.into(),
        label: label.into(),
        status: if passed { "passed" } else { "missing" }.into(),
        detail,
        evidence_ids,
    }
}

fn is_macro(language: &str, text: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "macro" | "jx3_macro"
    ) || text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("/cast ") || line.starts_with("/fcast ") || line.starts_with("#page")
    })
}

fn normalize_macro(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_string()
}

fn observe(scenario: &ScenarioSnapshotV1, evidence: &EvidenceStore) -> Observations {
    let mut observed = Observations::default();
    if scenario.verify_hash().is_err() {
        return observed;
    }
    for (id, value) in evidence {
        let Some(envelope) = trusted_envelope(id, value, scenario) else {
            continue;
        };
        match envelope.tool_name.as_str() {
            "simulate_scenario" | "analyze_timeline"
                if (envelope.tool_name == "simulate_scenario"
                    && valid_simulation(&envelope.result))
                    || (envelope.tool_name == "analyze_timeline"
                        && valid_timeline(&envelope.result)) =>
            {
                observed.simulations.insert(id.clone());
                if let Some(text) = scenario
                    .simulation
                    .macro_text
                    .as_deref()
                    .filter(|_| macro_execution_observed(scenario, &envelope.result))
                {
                    observed.macros.push(TestedMacro {
                        text: normalize_macro(text), evidence_id: id.clone(),
                        detail: "最终完整宏与当前冻结场景中的已模拟宏一致，且有实际施放；这不证明收益、精确复刻或全局最优。".into(),
                    });
                }
            }
            "compare_scenarios" => {
                let baseline = &envelope.result["baseline"];
                if baseline["scenario_hash"].as_str() != Some(&scenario.scenario_hash)
                    || !valid_comparison_metrics(baseline)
                {
                    continue;
                }
                observed.simulations.insert(id.clone());
                let Some(candidates) = envelope.result["candidates"].as_array() else {
                    continue;
                };
                for candidate in candidates {
                    let metrics = &candidate["metrics"];
                    if !valid_comparison_metrics(metrics) {
                        continue;
                    }
                    let Some(snapshot) = reconstruct_candidate(scenario, candidate) else {
                        continue;
                    };
                    observed
                        .candidates
                        .entry(snapshot.scenario_hash.clone())
                        .or_default()
                        .insert(id.clone());
                    // Only a full changed macro, not a partial replacement or a
                    // similarly named draft, proves this final artifact was run.
                    let changed_macro = candidate["changes"].as_array().is_some_and(|changes| {
                        changes.iter().any(|change| {
                            change["field"] == "simulation.macro_text"
                                && change["after"].is_string()
                        })
                    });
                    if changed_macro && macro_execution_observed(&snapshot, metrics) {
                        if let Some(text) = snapshot.simulation.macro_text.as_deref() {
                            let outcome = if metrics["fingerprint"] == baseline["fingerprint"] {
                                "该候选与基线指纹相同，未观察到战斗轨迹变化"
                            } else {
                                "该候选与基线指纹不同，不能仅据此认定收益改善"
                            };
                            observed.macros.push(TestedMacro {
                                text: normalize_macro(text), evidence_id: id.clone(),
                                detail: format!("最终完整宏匹配同场景对照中的已执行候选；{outcome}；匹配不证明精确复刻或全局最优。"),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    observed
}

fn trusted_envelope(
    id: &str,
    value: &Value,
    scenario: &ScenarioSnapshotV1,
) -> Option<EvidenceEnvelopeV1<Value>> {
    // EvidenceStore is supplied by the trusted registry, never from the report.
    // Recheck its envelope identity so mismatched, truncated and failure records
    // cannot accidentally satisfy a completion check.
    let envelope: EvidenceEnvelopeV1<Value> = serde_json::from_value(value.clone()).ok()?;
    if envelope.schema_version != EVIDENCE_SCHEMA_V1
        || envelope.evidence_id != id
        || envelope.scenario_hash != scenario.scenario_hash
        || envelope.engine_version.is_empty()
        || envelope.data_hash.is_empty()
        || super::evidence::validate_trace_id(&envelope.trace_id).is_err()
    {
        return None;
    }
    let expected = super::hash::canonical_sha256(&json!({
        "schema_version": EVIDENCE_SCHEMA_V1, "tool_name": envelope.tool_name,
        "scenario_hash": envelope.scenario_hash, "engine_version": envelope.engine_version,
        "engine_commit": envelope.engine_commit, "data_hash": envelope.data_hash,
        "args": envelope.args, "result": envelope.result, "warnings": envelope.warnings,
    }))
    .ok()?;
    (expected == id).then_some(envelope)
}

fn valid_metrics(metrics: &Value) -> bool {
    valid_fingerprint_and_time(metrics)
        && metrics["skill_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
        && ["dps", "total_damage"].iter().all(|field| {
            metrics[field]
                .as_f64()
                .is_some_and(|number| number.is_finite() && number >= 0.0)
        })
}

fn valid_fingerprint_and_time(metrics: &Value) -> bool {
    let Some(fingerprint) = metrics["fingerprint"].as_u64() else {
        return false;
    };
    metrics["fingerprint_hex"].as_str() == Some(format!("{fingerprint:016x}").as_str())
        && metrics["fight_time"]
            .as_f64()
            .is_some_and(|time| time.is_finite() && time > 0.0)
}

fn valid_simulation(result: &Value) -> bool {
    valid_metrics(result) && has_active_skill(result)
}

fn valid_timeline(result: &Value) -> bool {
    valid_fingerprint_and_time(result)
        && result["active_event_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
        && has_active_skill(result)
}

fn has_active_skill(result: &Value) -> bool {
    result["skills"].as_array().is_some_and(|skills| {
        skills.iter().any(|skill| {
            skill["triggered"] == false
                && skill["event_count"].as_u64().is_some_and(|count| count > 0)
        })
    })
}

fn valid_comparison_metrics(metrics: &Value) -> bool {
    valid_metrics(metrics)
        && metrics
            .pointer("/diagnostics/active_event_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
}

fn macro_execution_observed(scenario: &ScenarioSnapshotV1, result: &Value) -> bool {
    let sequence = &scenario.simulation.sequence;
    if !sequence.iter().any(|action| action == "__macro__") {
        return false;
    }
    if let Some(lines) = result.get("macro_line_stats").and_then(Value::as_array) {
        return lines.iter().any(|line| {
            line["selected_casts"]
                .as_u64()
                .is_some_and(|casts| casts > 0)
        });
    }
    // Simulation summaries (and older comparison records) have no per-line
    // execution counts. Actual active casts in a pure macro sequence suffice;
    // mixed manual/macro sequences need the explicit successful-macro counts.
    sequence.iter().all(|action| action == "__macro__")
}

fn reconstruct_candidate(
    scenario: &ScenarioSnapshotV1,
    candidate: &Value,
) -> Option<ScenarioSnapshotV1> {
    let hash = candidate.pointer("/metrics/scenario_hash")?.as_str()?;
    if hash == scenario.scenario_hash {
        return None;
    }
    let changes = candidate["changes"].as_array()?;
    if changes.is_empty() {
        return None;
    }
    let mut encoded = serde_json::to_value(scenario).ok()?;
    let simulation = encoded["simulation"].as_object_mut()?;
    // These typed defaults are omitted by SimulateRequest's serialization. A
    // legitimate comparison can still change them, e.g. seed 0 -> 7.
    simulation.insert(
        "dunya_reset_seed".into(),
        json!(scenario.simulation.dunya_reset_seed),
    );
    simulation.insert(
        "solidified_casts".into(),
        serde_json::to_value(&scenario.simulation.solidified_casts).ok()?,
    );
    for change in changes {
        let field = change["field"].as_str()?.strip_prefix("simulation.")?;
        // Current comparison patches expose direct simulation fields only.
        let current = simulation.get(field)?;
        let before = change.get("before")?;
        let after = change.get("after")?;
        if current != before || before == after {
            return None;
        }
        simulation.insert(field.into(), after.clone());
    }
    encoded["scenario_hash"] = Value::String(hash.into());
    let snapshot: ScenarioSnapshotV1 = serde_json::from_value(encoded).ok()?;
    snapshot.verify_hash().ok()?;
    Some(snapshot)
}

#[cfg(test)]
#[path = "../../tests/agent/task_completion.rs"]
mod tests;
