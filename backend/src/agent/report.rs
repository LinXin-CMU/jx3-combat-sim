use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const AGENT_REPORT_CONTENT_SCHEMA_V1: &str = "agent-report-content/v1";
pub const AGENT_REPORT_SCHEMA_V1: &str = "agent-report/v1";
const MAX_FINDINGS: usize = 12;
const MAX_RECOMMENDATIONS: usize = 8;
const MAX_ROTATION_CHANGES: usize = 8;
const MAX_METRICS_PER_FINDING: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentReportContentV1 {
    pub schema_version: String,
    pub summary: String,
    pub findings: Vec<AgentFindingV1>,
    pub recommendations: Vec<AgentRecommendationV1>,
    #[serde(default)]
    pub rotation_changes: Vec<RotationChangeV1>,
    #[serde(default)]
    pub artifacts: Vec<super::artifacts::DraftArtifactV1>,
    pub limitations: Vec<String>,
    pub refusal_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RotationChangeV1 {
    pub change_type: String,
    #[serde(default = "default_rotation_edit_operation")]
    pub edit_operation: String,
    pub target: String,
    pub current: String,
    pub proposed: String,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
}

fn default_rotation_edit_operation() -> String {
    "replace".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentFindingV1 {
    pub title: String,
    pub explanation: String,
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub metrics: Vec<GroundedMetricV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GroundedMetricV1 {
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub evidence_id: String,
    pub json_pointer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentRecommendationV1 {
    pub title: String,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentReportV1 {
    pub schema_version: String,
    pub question: String,
    pub scenario_hash: String,
    pub prompt_version: String,
    pub prompt_sha256: String,
    pub provider_profile: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<AgentKnowledgeSourceV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipment_comparisons: Vec<super::EquipmentComparisonPresentationV1>,
    pub content: AgentReportContentV1,
    pub evidence_ids: Vec<String>,
    pub accounting: AgentRunAccountingV1,
    pub termination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentKnowledgeSourceV1 {
    pub document_id: String,
    pub title: String,
    pub season: String,
    pub category: String,
    pub source_url: String,
    pub yuque_url: String,
    pub source_site: String,
    pub source_updated_at: String,
    pub version_match: String,
    pub fact_eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_warning: Option<String>,
    pub document_hash: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentRunAccountingV1 {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub simulations: u32,
    #[serde(default)]
    pub knowledge_searches: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportValidationError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedReportContentV1 {
    pub content: AgentReportContentV1,
    pub normalized_metric_citations: usize,
    pub sanitized_claims: usize,
}

impl std::fmt::Display for ReportValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ReportValidationError {}

pub type EvidenceStore = BTreeMap<String, Value>;

pub fn report_content_json_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string", "const": AGENT_REPORT_CONTENT_SCHEMA_V1},
            "summary": {"type": "string", "minLength": 1, "maxLength": 1024},
            "findings": {
                "type": "array",
                "maxItems": MAX_FINDINGS,
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "explanation": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "evidence_ids": {"type": "array", "minItems": 1, "items": {"type": "string"}},
                        "metrics": {
                            "type": "array",
                            "maxItems": MAX_METRICS_PER_FINDING,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": {"type": "string", "minLength": 1},
                                    "value": {"type": "number"},
                                    "unit": {"type": "string", "minLength": 1},
                                    "evidence_id": {"type": "string"},
                                    "json_pointer": {"type": "string", "pattern": "^/result/"}
                                },
                                "required": ["label", "value", "unit", "evidence_id", "json_pointer"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["title", "explanation", "evidence_ids", "metrics"],
                    "additionalProperties": false
                }
            },
            "recommendations": {
                "type": "array",
                "maxItems": MAX_RECOMMENDATIONS,
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "rationale": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "evidence_ids": {"type": "array", "minItems": 1, "items": {"type": "string"}}
                    },
                    "required": ["title", "rationale", "evidence_ids"],
                    "additionalProperties": false
                }
            },
            "rotation_changes": {
                "type": "array",
                "maxItems": MAX_ROTATION_CHANGES,
                "items": {
                    "type": "object",
                    "properties": {
                        "change_type": {"type": "string", "enum": ["macro_statement", "manual_operation"]},
                        "edit_operation": {"type": "string", "enum": ["replace", "insert_before", "insert_after", "remove", "adjust_timing"]},
                        "target": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "current": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "proposed": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "rationale": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "evidence_ids": {"type": "array", "minItems": 1, "items": {"type": "string"}}
                    },
                    "required": ["change_type", "edit_operation", "target", "current", "proposed", "rationale", "evidence_ids"],
                    "additionalProperties": false
                }
            },
            "artifacts": super::artifacts::json_schema(),
            "limitations": {"type": "array", "maxItems": MAX_RECOMMENDATIONS, "items": {"type": "string"}},
            "refusal_reason": {"type": ["string", "null"]}
        },
        "required": ["schema_version", "summary", "findings", "recommendations", "rotation_changes", "limitations", "refusal_reason"],
        "additionalProperties": false
    })
}

pub fn parse_and_validate_report(
    raw: &str,
    evidence: &EvidenceStore,
) -> Result<ValidatedReportContentV1, ReportValidationError> {
    let mut report = parse_report_json(raw)
        .map_err(|parse_error| error("invalid_report_json", parse_error_message(&parse_error)))?;
    normalize_report_text_fields(&mut report);
    let normalized_evidence_ids = normalize_evidence_id_references(&mut report, evidence);
    let normalized_metric_citations = normalized_evidence_ids
        + normalize_metric_citations(&mut report, evidence)
        + normalize_rotation_anchor_markers(&mut report);
    validate_report(&report, evidence)?;
    Ok(ValidatedReportContentV1 {
        content: report,
        normalized_metric_citations,
        sanitized_claims: 0,
    })
}

/// Preserve valid claims from a structurally parseable model report instead of
/// discarding the whole answer because one metric, citation, or prose number is
/// wrong. The returned content passes the same strict validator as a normal
/// report. Unsupported sentences are replaced with an explicit evidence gap;
/// unrelated evidence and topic-specific stock conclusions are never added.
pub fn parse_and_salvage_report(
    raw: &str,
    evidence: &EvidenceStore,
) -> Result<ValidatedReportContentV1, ReportValidationError> {
    let mut report = parse_report_json(raw)
        .map_err(|parse_error| error("invalid_report_json", parse_error_message(&parse_error)))?;
    let mut sanitized_claims = 0;
    if report.schema_version != AGENT_REPORT_CONTENT_SCHEMA_V1 {
        report.schema_version = AGENT_REPORT_CONTENT_SCHEMA_V1.to_string();
        sanitized_claims += 1;
    }

    sanitized_claims += truncate_vec(&mut report.findings, MAX_FINDINGS);
    sanitized_claims += truncate_vec(&mut report.recommendations, MAX_RECOMMENDATIONS);
    sanitized_claims += truncate_vec(&mut report.rotation_changes, MAX_ROTATION_CHANGES);
    sanitized_claims += truncate_vec(&mut report.limitations, MAX_RECOMMENDATIONS);
    let normalized_evidence_ids = normalize_evidence_id_references(&mut report, evidence);
    let normalized_metric_citations = normalized_evidence_ids
        + normalize_metric_citations(&mut report, evidence)
        + normalize_rotation_anchor_markers(&mut report);

    let mut retained_findings = Vec::with_capacity(report.findings.len());
    for mut finding in report.findings.drain(..) {
        sanitized_claims += sanitize_evidence_ids(&mut finding.evidence_ids, evidence);
        sanitized_claims += truncate_vec(&mut finding.metrics, MAX_METRICS_PER_FINDING);
        let mut retained_metrics = Vec::with_capacity(finding.metrics.len());
        for mut metric in finding.metrics.drain(..) {
            sanitized_claims += sanitize_text_field(&mut metric.label, "已验证指标");
            sanitized_claims += sanitize_text_field(&mut metric.unit, "value");
            if metric_matches_evidence(&metric, evidence) {
                if !finding.evidence_ids.contains(&metric.evidence_id) {
                    finding.evidence_ids.push(metric.evidence_id.clone());
                    sanitized_claims += 1;
                }
                retained_metrics.push(metric);
            } else {
                sanitized_claims += 1;
            }
        }
        finding.metrics = retained_metrics;
        if finding.evidence_ids.is_empty() {
            sanitized_claims += 1;
            continue;
        }
        sanitized_claims += sanitize_text_field(&mut finding.title, "待核实的判断");
        sanitized_claims += sanitize_unsupported_numeric_prose(
            &mut finding.title,
            &finding.metrics,
            &finding.evidence_ids,
            evidence,
            "待核实的判断",
        );
        const EXPLANATION_GAP: &str = "这项判断的数值依据尚待核实。";
        sanitized_claims += sanitize_text_field(&mut finding.explanation, EXPLANATION_GAP);
        sanitized_claims += sanitize_unsupported_numeric_prose(
            &mut finding.explanation,
            &finding.metrics,
            &finding.evidence_ids,
            evidence,
            EXPLANATION_GAP,
        );
        retained_findings.push(finding);
    }
    report.findings = retained_findings;

    let report_metrics = report
        .findings
        .iter()
        .flat_map(|finding| finding.metrics.iter())
        .cloned()
        .collect::<Vec<_>>();
    let report_evidence_ids = cited_evidence_ids(&report);
    sanitized_claims +=
        sanitize_text_field(&mut report.summary, "本轮仅保留通过本次证据校验的内容。");
    sanitized_claims += sanitize_unsupported_numeric_prose(
        &mut report.summary,
        &report_metrics,
        &report_evidence_ids,
        evidence,
        "这项判断的数值依据尚待核实。",
    );

    let mut retained_recommendations = Vec::with_capacity(report.recommendations.len());
    for mut recommendation in report.recommendations.drain(..) {
        sanitized_claims += sanitize_evidence_ids(&mut recommendation.evidence_ids, evidence);
        if recommendation.evidence_ids.is_empty() {
            sanitized_claims += 1;
            continue;
        }
        let metric_values = report_metrics
            .iter()
            .filter(|metric| recommendation.evidence_ids.contains(&metric.evidence_id))
            .cloned()
            .collect::<Vec<_>>();
        sanitized_claims += sanitize_text_field(&mut recommendation.title, "下一步验证建议");
        sanitized_claims += sanitize_unsupported_numeric_prose(
            &mut recommendation.title,
            &metric_values,
            &recommendation.evidence_ids,
            evidence,
            "下一步验证建议",
        );
        sanitized_claims += sanitize_text_field(
            &mut recommendation.rationale,
            "建议通过新的确定性实验继续验证。",
        );
        sanitized_claims += sanitize_unsupported_numeric_prose(
            &mut recommendation.rationale,
            &metric_values,
            &recommendation.evidence_ids,
            evidence,
            "这项建议的数值依据尚待核实。",
        );
        retained_recommendations.push(recommendation);
    }
    report.recommendations = retained_recommendations;

    let mut retained_rotation_changes = Vec::with_capacity(report.rotation_changes.len());
    for mut change in report.rotation_changes.drain(..) {
        sanitized_claims += sanitize_evidence_ids(&mut change.evidence_ids, evidence);
        if !rotation_change_has_input_evidence(&change, evidence) {
            if let Some(evidence_id) = matching_rotation_input_evidence_id(&change, evidence) {
                change.evidence_ids.push(evidence_id);
                sanitized_claims += 1;
            }
        }
        if change.evidence_ids.is_empty()
            || !matches!(
                change.change_type.as_str(),
                "macro_statement" | "manual_operation"
            )
            || !rotation_edit_operation_is_valid(&change)
            || !rotation_change_has_input_evidence(&change, evidence)
            || !rotation_change_has_current_guide(&change, evidence)
            || !rotation_change_numbers_are_grounded(&change, evidence)
            || !rotation_change_has_diagnosis_evidence(&change, evidence)
            || !rotation_change_has_comparison_evidence(&change, evidence)
        {
            sanitized_claims += 1;
            continue;
        }
        sanitized_claims += sanitize_text_field(&mut change.target, "当前循环位置");
        sanitized_claims += sanitize_text_field(&mut change.current, "当前操作");
        sanitized_claims += sanitize_text_field(&mut change.proposed, "建议操作");
        sanitized_claims +=
            sanitize_text_field(&mut change.rationale, "依据见所引攻略与本轮时间轴。");
        retained_rotation_changes.push(change);
    }
    report.rotation_changes = retained_rotation_changes;

    let report_evidence_ids = cited_evidence_ids(&report);

    for limitation in &mut report.limitations {
        sanitized_claims += sanitize_text_field(limitation, "存在尚未验证的边界。");
        sanitized_claims += sanitize_unsupported_numeric_prose(
            limitation,
            &report_metrics,
            &report_evidence_ids,
            evidence,
            "存在一项尚未验证的边界。",
        );
    }
    if let Some(reason) = &mut report.refusal_reason {
        sanitized_claims += sanitize_text_field(reason, "当前请求无法形成可验证结论。");
        sanitized_claims += sanitize_unsupported_numeric_prose(
            reason,
            &report_metrics,
            &report_evidence_ids,
            evidence,
            "当前请求无法形成可验证结论。",
        );
    }

    if report.findings.is_empty() && report.refusal_reason.is_none() {
        return Err(error(
            "empty_salvaged_report",
            "no cited finding remains for the original question; repair the answer using the original question and its evidence",
        ));
    }

    const PARTIAL_LIMITATION: &str =
        "部分数值或引用尚待核实；相关位置已标明，其他有依据的分析继续保留。";
    if !report
        .limitations
        .iter()
        .any(|limitation| limitation == PARTIAL_LIMITATION)
    {
        if report.limitations.len() == MAX_RECOMMENDATIONS {
            report.limitations.pop();
        }
        report.limitations.push(PARTIAL_LIMITATION.to_string());
    }

    validate_report(&report, evidence)?;
    Ok(ValidatedReportContentV1 {
        content: report,
        normalized_metric_citations,
        sanitized_claims: sanitized_claims.max(1),
    })
}

/// Providers that advertise JSON mode may still wrap the object in a Markdown
/// fence or a short natural-language preface. Unwrap exactly one structurally
/// valid report object, then leave every schema, evidence, and metric check to
/// the existing strict validator.
fn parse_report_json(raw: &str) -> Result<AgentReportContentV1, serde_json::Error> {
    let trimmed = raw.trim().trim_start_matches('\u{feff}').trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Ok(report) = deserialize_compatible_report(value) {
            return Ok(report);
        }
    }

    // Try object boundaries rather than stripping arbitrary prose. Deserializing
    // directly into the deny_unknown_fields report type prevents a nested or
    // unrelated JSON object from being accepted accidentally.
    for (index, _) in trimmed.match_indices('{').take(64) {
        let mut deserializer = serde_json::Deserializer::from_str(&trimmed[index..]);
        if let Ok(value) = Value::deserialize(&mut deserializer) {
            if let Ok(report) = deserialize_compatible_report(value) {
                return Ok(report);
            }
        }
    }

    // Some compatibility gateways JSON-encode the assistant content once more.
    if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
        let decoded = decoded.trim();
        if let Ok(value) = serde_json::from_str::<Value>(decoded) {
            if let Ok(report) = deserialize_compatible_report(value) {
                return Ok(report);
            }
        }
        for (index, _) in decoded.match_indices('{').take(64) {
            let mut deserializer = serde_json::Deserializer::from_str(&decoded[index..]);
            if let Ok(value) = Value::deserialize(&mut deserializer) {
                if let Ok(report) = deserialize_compatible_report(value) {
                    return Ok(report);
                }
            }
        }
    }

    let value = serde_json::from_str::<Value>(trimmed)?;
    deserialize_compatible_report(value)
}

/// Normalize a small set of lossless provider-shape variants before applying
/// the strict report schema. This is intentionally limited to fields whose
/// semantic meaning is unchanged by the conversion.
fn deserialize_compatible_report(
    mut value: Value,
) -> Result<AgentReportContentV1, serde_json::Error> {
    let mut drafts = super::artifacts::ArtifactStore::default();
    drafts.capture_report(&value.to_string());
    if !drafts.is_empty() {
        value["artifacts"] = serde_json::to_value(drafts.items())?;
        if let Some(changes) = value.get_mut("rotation_changes").and_then(Value::as_array_mut) {
            changes.retain(|change| !matches!(change["change_type"].as_str(), Some("add_macro_page" | "new_macro" | "macro_text")));
        }
    }
    let Some(object) = value.as_object_mut() else {
        return serde_json::from_value(value);
    };

    object.insert(
        "schema_version".to_string(),
        Value::String(AGENT_REPORT_CONTENT_SCHEMA_V1.to_string()),
    );
    for display_field in ["report_type", "title", "explanation", "evidence_ids"] {
        object.remove(display_field);
    }
    object
        .entry("rotation_changes".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));

    // `Option<String>` still requires the key in the published JSON schema.
    // Treat omission as the natural `null` representation.
    object
        .entry("refusal_reason".to_string())
        .or_insert(Value::Null);

    // Several JSON-only providers render a prose limitation as the same
    // title/explanation card shape used by findings. Preserve the prose while
    // keeping the public contract as Vec<String>.
    if let Some(limitations) = object.get_mut("limitations").and_then(Value::as_array_mut) {
        for limitation in limitations {
            let Some(fields) = limitation.as_object() else {
                continue;
            };
            let title = fields
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let explanation = fields
                .get("explanation")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let normalized = match (title.is_empty(), explanation.is_empty()) {
                (false, false) => format!("{title}：{explanation}"),
                (false, true) => title.to_string(),
                (true, false) => explanation.to_string(),
                (true, true) => continue,
            };
            *limitation = Value::String(normalized);
        }
    }

    if let Some(findings) = object.get_mut("findings").and_then(Value::as_array_mut) {
        for finding in findings {
            let Some(fields) = finding.as_object_mut() else {
                continue;
            };
            let claim = fields.remove("claim");
            if !fields.contains_key("explanation") {
                if let Some(explanation) = fields
                    .remove("description")
                    .or_else(|| fields.remove("statement"))
                    .or_else(|| fields.remove("finding"))
                    .or_else(|| claim.clone())
                {
                    fields.insert("explanation".to_string(), explanation);
                }
            }
            fields.entry("title".to_string()).or_insert_with(|| {
                let title = claim
                    .as_ref()
                    .and_then(Value::as_str)
                    .map(|value| value.chars().take(48).collect::<String>())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "分析结论".to_string());
                Value::String(title)
            });
            fields.remove("evidence_pointers");
            let inherited_evidence_id = fields
                .get("evidence_ids")
                .and_then(Value::as_array)
                .and_then(|ids| ids.first())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(metrics) = fields.get_mut("metrics").and_then(Value::as_array_mut) {
                metrics.retain_mut(|metric| {
                    let Some(metric) = metric.as_object_mut() else {
                        return false;
                    };
                    if !metric.contains_key("label") {
                        if let Some(label) = metric
                            .remove("metric_name")
                            .or_else(|| metric.remove("name"))
                        {
                            metric.insert("label".to_string(), label);
                        }
                    }
                    metric
                        .entry("evidence_id".to_string())
                        .or_insert_with(|| Value::String(inherited_evidence_id.clone()));
                    if let Some(parsed) = metric
                        .get("value")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .and_then(|value| value.replace(',', "").parse::<f64>().ok())
                        .filter(|value| value.is_finite())
                    {
                        metric.insert("value".to_string(), Value::from(parsed));
                    }
                    let pointer = metric
                        .get("json_pointer")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let unit = metric
                        .get("unit")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let normalized_unit = match unit {
                        "%" | "百分比" => Some("percent"),
                        "秒" => Some("seconds"),
                        "次" | "个" => Some("count"),
                        "DPS" | "dps" => Some("damage_per_second"),
                        "数值" if pointer.ends_with("/dps") => Some("damage_per_second"),
                        "数值" if pointer.ends_with("/total_damage") => Some("damage"),
                        _ => None,
                    };
                    if let Some(unit) = normalized_unit {
                        metric.insert("unit".to_string(), Value::String(unit.to_string()));
                    }
                    metric.get("label").is_some_and(Value::is_string)
                        && metric.get("value").is_some_and(Value::is_number)
                        && metric.get("unit").is_some_and(Value::is_string)
                        && metric
                            .get("json_pointer")
                            .and_then(Value::as_str)
                            .is_some_and(|pointer| !pointer.trim().is_empty())
                });
            }
        }
    }

    let default_recommendation_evidence = object
        .get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|finding| finding.get("evidence_ids"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();

    if let Some(rationale) = object
        .get("recommendations")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        object.insert(
            "recommendations".to_string(),
            Value::Array(vec![serde_json::json!({
                "title": "建议",
                "rationale": rationale,
                "evidence_ids": default_recommendation_evidence.clone(),
            })]),
        );
    }

    if let Some(recommendations) = object
        .get_mut("recommendations")
        .and_then(Value::as_array_mut)
    {
        for recommendation in recommendations {
            if let Some(rationale) = recommendation.as_str().map(str::to_owned) {
                *recommendation = serde_json::json!({
                    "title": "建议",
                    "rationale": rationale,
                    "evidence_ids": default_recommendation_evidence,
                });
            }
            let Some(fields) = recommendation.as_object_mut() else {
                continue;
            };
            if let Some(action) = fields.remove("action") {
                fields
                    .entry("title".to_string())
                    .or_insert_with(|| Value::String("建议".to_string()));
                fields.entry("rationale".to_string()).or_insert(action);
            }
            fields
                .entry("evidence_ids".to_string())
                .or_insert_with(|| Value::Array(default_recommendation_evidence.clone()));
        }
    }

    // Rotation edits are an optional high-risk projection. Some compatible
    // JSON providers still emit an older card shape even when the rest of the
    // report is valid. Keep only entries that can be losslessly normalized to
    // the current typed shape; semantic evidence checks still run afterward.
    if let Some(changes) = object
        .get_mut("rotation_changes")
        .and_then(Value::as_array_mut)
    {
        changes.retain_mut(|change| {
            let Some(fields) = change.as_object_mut() else {
                return false;
            };
            let change_type = fields
                .get("change_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // A provider may describe the outcome of a losing A/B candidate
            // as a "revert" card. It is a decision, not an edit to apply, so
            // leave it in the prose findings and omit it from the patch list.
            if matches!(change_type, "revert" | "keep" | "no_change") {
                return false;
            }
            let normalized_change_type = match change_type {
                "edit_operation" | "manual" | "manual_edit" | "sequence_edit" => {
                    Some("manual_operation")
                }
                "macro" | "macro_line" | "macro_edit" | "macro_statement_edit" => {
                    Some("macro_statement")
                }
                _ => None,
            };
            if let Some(normalized) = normalized_change_type {
                fields.insert(
                    "change_type".to_string(),
                    Value::String(normalized.to_string()),
                );
            }
            if let Some(operation) = fields
                .get("edit_operation")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                let normalized = match operation.as_str() {
                    "modify" | "change" | "adjust" => Some("replace"),
                    "delete" => Some("remove"),
                    _ => None,
                };
                if let Some(normalized) = normalized {
                    fields.insert(
                        "edit_operation".to_string(),
                        Value::String(normalized.to_string()),
                    );
                }
            }
            let removes_current = fields
                .get("rationale")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("删除") || text.contains("移除"));
            if fields.get("proposed").is_some_and(Value::is_null) && removes_current {
                let current = fields
                    .get("current")
                    .and_then(Value::as_str)
                    .unwrap_or("当前操作");
                fields.insert(
                    "proposed".to_string(),
                    Value::String(format!("删除{current}")),
                );
                fields.insert(
                    "edit_operation".to_string(),
                    Value::String("remove".to_string()),
                );
            }
            serde_json::from_value::<RotationChangeV1>(change.clone()).is_ok()
        });
    }

    serde_json::from_value(value)
}

fn parse_error_message(parse_error: &serde_json::Error) -> String {
    format!(
        "report JSON does not match AgentReportContentV1: {parse_error}. limitations must be strings; refusal_reason must be a string or null"
    )
}

fn truncate_vec<T>(values: &mut Vec<T>, limit: usize) -> usize {
    let removed = values.len().saturating_sub(limit);
    values.truncate(limit);
    removed
}

fn sanitize_text_field(value: &mut String, fallback: &str) -> usize {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut bounded = compact.chars().take(1024).collect::<String>();
    if bounded.is_empty() {
        bounded = fallback.to_string();
    }
    let changed = usize::from(*value != bounded);
    *value = bounded;
    changed
}

fn normalize_report_text_fields(report: &mut AgentReportContentV1) -> usize {
    super::artifacts::normalize_artifacts(&mut report.artifacts);
    let mut changed = sanitize_text_field(&mut report.summary, "本轮分析见下方。");
    for finding in &mut report.findings {
        changed += sanitize_text_field(&mut finding.title, "分析结论");
        changed += sanitize_text_field(&mut finding.explanation, "本轮证据支持这项判断。");
        for metric in &mut finding.metrics {
            changed += sanitize_text_field(&mut metric.label, "已验证指标");
            changed += sanitize_text_field(&mut metric.unit, "value");
        }
    }
    for recommendation in &mut report.recommendations {
        changed += sanitize_text_field(&mut recommendation.title, "下一步建议");
        changed += sanitize_text_field(&mut recommendation.rationale, "依据当前判断继续验证。");
    }
    for change in &mut report.rotation_changes {
        changed += sanitize_text_field(&mut change.target, "当前循环位置");
        changed += sanitize_text_field(&mut change.current, "当前操作");
        changed += sanitize_text_field(&mut change.proposed, "建议操作");
        changed += sanitize_text_field(&mut change.rationale, "依据当前诊断与对照结果。");
    }
    for limitation in &mut report.limitations {
        changed += sanitize_text_field(limitation, "存在尚未验证的边界。");
    }
    if let Some(reason) = &mut report.refusal_reason {
        changed += sanitize_text_field(reason, "当前请求无法形成结论。");
    }
    changed
}

fn sanitize_evidence_ids(ids: &mut Vec<String>, evidence: &EvidenceStore) -> usize {
    let before = ids.len();
    let mut unique = HashSet::new();
    ids.retain(|id| {
        valid_evidence_id(id) && evidence.contains_key(id) && unique.insert(id.clone())
    });
    before.saturating_sub(ids.len())
}

fn valid_evidence_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn metric_matches_evidence(metric: &GroundedMetricV1, evidence: &EvidenceStore) -> bool {
    if validate_short_text(&metric.label).is_err()
        || validate_short_text(&metric.unit).is_err()
        || !metric.value.is_finite()
        || !valid_evidence_id(&metric.evidence_id)
        || !metric.json_pointer.starts_with("/result/")
        || metric.json_pointer.len() > 256
    {
        return false;
    }
    let Some(source) = evidence
        .get(&metric.evidence_id)
        .filter(|envelope| {
            envelope.get("evidence_id").and_then(Value::as_str) == Some(metric.evidence_id.as_str())
                && metric_tool_allowed(envelope)
        })
        .and_then(|envelope| envelope.pointer(&metric.json_pointer))
        .and_then(Value::as_f64)
    else {
        return false;
    };
    metric_value_matches_source(metric.value, &metric.unit, source)
}

fn metric_value_matches_source(value: f64, unit: &str, source: f64) -> bool {
    // Providers commonly render continuous simulator values to two decimals.
    // Half of the last displayed cent is still the same measured value.
    let direct_tolerance = 0.005_f64.max(source.abs() * 1e-9);
    if (source - value).abs() <= direct_tolerance {
        return true;
    }

    // Large combat totals and DPS values are often presented without decimal
    // places. Accept ordinary nearest-integer display rounding, while keeping
    // small values (ratios, timings and counts) on the tighter rule above.
    if source.abs() >= 100.0 && value.fract().abs() <= f64::EPSILON {
        return (source - value).abs() < 0.5;
    }

    // Simulator ratios are stored as 0..=1 fractions, while reports display
    // them as human-readable percentages. Accept only that explicit unit
    // conversion and a small rounding window (at most 0.05 percentage point).
    let percent_unit = unit.eq_ignore_ascii_case("percent")
        || unit.eq_ignore_ascii_case("percentage")
        || matches!(unit.trim(), "%" | "％" | "百分比");
    if percent_unit && (0.0..=1.0).contains(&source) {
        let displayed = source * 100.0;
        let display_tolerance = 0.05_f64.max(displayed.abs() * 1e-9);
        return (displayed - value).abs() <= display_tolerance;
    }

    false
}

fn normalize_metric_citations(
    report: &mut AgentReportContentV1,
    evidence: &EvidenceStore,
) -> usize {
    let mut normalized = 0;
    for finding in &mut report.findings {
        for metric in &mut finding.metrics {
            if evidence.contains_key(&metric.evidence_id)
                && !finding.evidence_ids.contains(&metric.evidence_id)
            {
                finding.evidence_ids.push(metric.evidence_id.clone());
                normalized += 1;
            }
            // `ranked_damage_sources` is a compact model-facing view. Its
            // entries carry pointers into the immutable skills array, but a
            // provider may cite the view's own array position instead. Resolve
            // that losslessly before validation so the system does not reject
            // a correct metric for following its projected context.
            if let Some(candidate) = evidence.get(&metric.evidence_id).and_then(|envelope| {
                ranked_damage_metric_source_pointer(&metric.json_pointer, envelope)
            }) {
                metric.json_pointer = candidate;
                normalized += 1;
            }
            // Models occasionally copy a JSON Pointer relative to the
            // evidence result object even though the public report contract
            // requires the explicit /result prefix. Repair only when the
            // resulting absolute pointer exists and is numeric.
            if metric.json_pointer.starts_with('/')
                && !metric.json_pointer.starts_with("/result/")
                && metric.json_pointer.len() <= 249
            {
                let candidate = format!("/result{}", metric.json_pointer);
                let exists = evidence
                    .get(&metric.evidence_id)
                    .filter(|envelope| metric_tool_allowed(envelope))
                    .and_then(|envelope| envelope.pointer(&candidate))
                    .and_then(Value::as_f64)
                    .is_some();
                if exists {
                    metric.json_pointer = candidate;
                    normalized += 1;
                }
            }
            // An existing numeric pointer identifies the event the model
            // chose. A mismatched value must be corrected at that event;
            // looking elsewhere for the same number changes claim ownership.
            let has_numeric_source = evidence
                .get(&metric.evidence_id)
                .and_then(|envelope| envelope.pointer(&metric.json_pointer))
                .and_then(Value::as_f64)
                .is_some();
            if !has_numeric_source && !metric_matches_evidence(metric, evidence) {
                if let Some(candidate) = infer_metric_source_pointer(metric, evidence) {
                    metric.json_pointer = candidate;
                    normalized += 1;
                }
            }
        }
    }
    normalized
}

#[derive(Debug)]
struct NumericPointerCandidate {
    pointer: String,
    field: String,
    context: Vec<String>,
}

fn infer_metric_source_pointer(
    metric: &GroundedMetricV1,
    evidence: &EvidenceStore,
) -> Option<String> {
    let envelope = evidence
        .get(&metric.evidence_id)
        .filter(|envelope| metric_tool_allowed(envelope))?;
    let result = envelope.get("result")?;
    let mut candidates = Vec::new();
    collect_numeric_pointer_candidates(result, "/result", &[], &mut candidates);
    let original_tail = metric.json_pointer.rsplit('/').next().unwrap_or_default();
    let mut scored = candidates
        .into_iter()
        .filter_map(|candidate| {
            let source = envelope.pointer(&candidate.pointer)?.as_f64()?;
            metric_value_matches_source(metric.value, &metric.unit, source).then(|| {
                let mut score = metric_pointer_field_score(&metric.label, &candidate.field);
                if !original_tail.is_empty() && original_tail == candidate.field {
                    score += 25;
                }
                for term in [
                    "绝刀",
                    "斩刀",
                    "血怒",
                    "盾击",
                    "盾飞",
                    "盾回",
                    "援戈",
                    "业火麟光",
                    "嗜血",
                    "狂绝",
                    "麟光甲",
                    "流血",
                ] {
                    if metric.label.contains(term)
                        && candidate.context.iter().any(|value| value.contains(term))
                    {
                        score += 100;
                    }
                }
                (score, candidate.pointer)
            })
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let best = scored.first()?;
    if best.0 == 0 || scored.get(1).is_some_and(|next| next.0 == best.0) {
        return None;
    }
    Some(best.1.clone())
}

fn collect_numeric_pointer_candidates(
    value: &Value,
    pointer: &str,
    inherited_context: &[String],
    output: &mut Vec<NumericPointerCandidate>,
) {
    match value {
        Value::Object(object) => {
            let mut context = inherited_context.to_vec();
            context.extend(
                object
                    .values()
                    .filter_map(Value::as_str)
                    .take(8)
                    .map(str::to_owned),
            );
            for (field, child) in object {
                let escaped = field.replace('~', "~0").replace('/', "~1");
                let child_pointer = format!("{pointer}/{escaped}");
                if child.as_f64().is_some() {
                    output.push(NumericPointerCandidate {
                        pointer: child_pointer,
                        field: field.clone(),
                        context: context.clone(),
                    });
                } else {
                    collect_numeric_pointer_candidates(child, &child_pointer, &context, output);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_numeric_pointer_candidates(
                    child,
                    &format!("{pointer}/{index}"),
                    inherited_context,
                    output,
                );
            }
        }
        _ => {}
    }
}

fn metric_pointer_field_score(label: &str, field: &str) -> i32 {
    let rules: &[(&[&str], &[&str])] = &[
        (&["DPS", "秒伤"], &["dps", "delta_dps"]),
        (
            &["比例", "占比", "幅度", "%"],
            &["delta_percent", "coverage_percent", "damage_share"],
        ),
        (&["总伤", "伤害差"], &["total_damage", "damage_delta"]),
        (
            &["溢出", "溢怒"],
            &["overflow_total", "rage_overflow_total"],
        ),
        (
            &["溢出事件", "溢怒事件"],
            &["overflow_events", "event_count"],
        ),
        (
            &["实际释放次数", "命中次数"],
            &["total_matches", "selected_casts", "event_count"],
        ),
        (
            &["次数", "事件数", "次数组"],
            &["event_count", "event_count_delta", "count", "total_matches"],
        ),
        (
            &["空档累计", "空档时长"],
            &["gcd_gap_seconds", "total_seconds"],
        ),
        (&["空档次数"], &["gcd_gap_count", "count"]),
        (
            &["等待"],
            &[
                "recorded_wait_seconds",
                "recorded_wait_count",
                "total_seconds",
                "count",
            ],
        ),
        (
            &["总获得", "获得量"],
            &["total_rage_gained", "yuan_ge_gained_stacks"],
        ),
    ];
    rules
        .iter()
        .filter(|(labels, _)| labels.iter().any(|needle| label.contains(needle)))
        .flat_map(|(_, fields)| fields.iter())
        .position(|expected| *expected == field)
        .map(|position| 40_i32.saturating_sub(position as i32))
        .unwrap_or(0)
}

fn ranked_damage_metric_source_pointer(pointer: &str, envelope: &Value) -> Option<String> {
    let suffix = pointer.strip_prefix("/result/ranked_damage_sources/")?;
    let (rank, field) = suffix.split_once('/')?;
    let rank = rank.parse::<usize>().ok()?;
    if !matches!(field, "damage_share" | "total_damage") {
        return None;
    }
    let mut ranked = envelope
        .pointer("/result/skills")?
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(index, skill)| {
            let total_damage = skill.get("total_damage")?.as_f64()?;
            (total_damage > 0.0).then_some((index, total_damage))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    let source_index = ranked.get(rank)?.0;
    let candidate = format!("/result/skills/{source_index}/{field}");
    envelope.pointer(&candidate)?.as_f64()?;
    Some(candidate)
}

fn normalize_evidence_id_references(
    report: &mut AgentReportContentV1,
    evidence: &EvidenceStore,
) -> usize {
    fn normalize(ids: &mut [String], evidence: &EvidenceStore) -> usize {
        let mut changed = 0;
        for id in ids {
            if evidence.contains_key(id) || id.len() != 64 {
                continue;
            }
            let prefix = &id[..12];
            let suffix = &id[52..];
            let mut matches = evidence.keys().filter(|candidate| {
                candidate.len() == 64
                    && candidate.starts_with(prefix)
                    && candidate.ends_with(suffix)
            });
            let Some(candidate) = matches.next() else {
                continue;
            };
            if matches.next().is_none() {
                *id = candidate.clone();
                changed += 1;
            }
        }
        changed
    }

    let mut changed = 0;
    for finding in &mut report.findings {
        changed += normalize(&mut finding.evidence_ids, evidence);
        for metric in &mut finding.metrics {
            changed += normalize(std::slice::from_mut(&mut metric.evidence_id), evidence);
        }
    }
    for recommendation in &mut report.recommendations {
        changed += normalize(&mut recommendation.evidence_ids, evidence);
    }
    for change in &mut report.rotation_changes {
        changed += normalize(&mut change.evidence_ids, evidence);
    }
    changed
}

fn normalize_rotation_anchor_markers(report: &mut AgentReportContentV1) -> usize {
    fn normalize(text: &mut String) -> usize {
        if !text.contains("[[") || (!text.contains("|op:") && !text.contains("|ev:")) {
            return 0;
        }
        let normalized = text.replace("}}", "]]");
        let changed = usize::from(normalized != *text);
        *text = normalized;
        changed
    }

    let mut changed = normalize(&mut report.summary);
    for finding in &mut report.findings {
        changed += normalize(&mut finding.title);
        changed += normalize(&mut finding.explanation);
    }
    for recommendation in &mut report.recommendations {
        changed += normalize(&mut recommendation.title);
        changed += normalize(&mut recommendation.rationale);
    }
    for change in &mut report.rotation_changes {
        changed += normalize(&mut change.target);
        changed += normalize(&mut change.current);
        changed += normalize(&mut change.proposed);
        changed += normalize(&mut change.rationale);
    }
    changed
}

pub fn validate_report(
    report: &AgentReportContentV1,
    evidence: &EvidenceStore,
) -> Result<(), ReportValidationError> {
    if report.schema_version != AGENT_REPORT_CONTENT_SCHEMA_V1 {
        return Err(error(
            "invalid_report_schema",
            "report schema_version is not supported",
        ));
    }
    validate_short_text(&report.summary)?;
    if report.findings.len() > MAX_FINDINGS
        || report.recommendations.len() > MAX_RECOMMENDATIONS
        || report.rotation_changes.len() > MAX_ROTATION_CHANGES
        || report.limitations.len() > MAX_RECOMMENDATIONS
    {
        return Err(error(
            "report_too_large",
            "report collection limit exceeded",
        ));
    }
    if report.findings.is_empty() && report.artifacts.is_empty() && report.refusal_reason.is_none() {
        return Err(error(
            "empty_report",
            "report requires a verified finding or a refusal reason",
        ));
    }
    if let Some(reason) = &report.refusal_reason {
        validate_short_text(reason)?;
    }
    for limitation in &report.limitations {
        validate_short_text(limitation)?;
    }

    for finding in &report.findings {
        validate_short_text(&finding.title)?;
        validate_short_text(&finding.explanation)?;
        if finding.evidence_ids.is_empty() {
            return Err(error(
                "finding_without_evidence",
                "every finding must cite evidence",
            ));
        }
        if finding.metrics.len() > MAX_METRICS_PER_FINDING {
            return Err(error("report_too_large", "finding metric limit exceeded"));
        }
        let cited = validate_evidence_ids(&finding.evidence_ids, evidence)?;
        for metric in &finding.metrics {
            validate_metric(metric, &cited, evidence)?;
        }
    }

    let report_metrics = report
        .findings
        .iter()
        .flat_map(|finding| finding.metrics.iter())
        .collect::<Vec<_>>();
    let report_evidence_ids = cited_evidence_ids(report);
    validate_grounded_prose(
        &report.summary,
        report_metrics.iter().copied(),
        &report_evidence_ids,
        evidence,
    )?;
    if let Some(reason) = &report.refusal_reason {
        validate_grounded_prose(
            reason,
            report_metrics.iter().copied(),
            &report_evidence_ids,
            evidence,
        )?;
    }
    for limitation in &report.limitations {
        validate_grounded_prose(
            limitation,
            report_metrics.iter().copied(),
            &report_evidence_ids,
            evidence,
        )?;
    }
    for finding in &report.findings {
        validate_grounded_prose(
            &finding.title,
            finding.metrics.iter(),
            &finding.evidence_ids,
            evidence,
        )?;
        validate_grounded_prose(
            &finding.explanation,
            finding.metrics.iter(),
            &finding.evidence_ids,
            evidence,
        )?;
    }

    for recommendation in &report.recommendations {
        validate_short_text(&recommendation.title)?;
        validate_short_text(&recommendation.rationale)?;
        if recommendation.evidence_ids.is_empty() {
            return Err(error(
                "recommendation_without_evidence",
                "every recommendation must cite evidence",
            ));
        }
        validate_evidence_ids(&recommendation.evidence_ids, evidence)?;
        let recommendation_metrics = report_metrics
            .iter()
            .copied()
            .filter(|metric| recommendation.evidence_ids.contains(&metric.evidence_id));
        validate_grounded_prose(
            &recommendation.title,
            recommendation_metrics.clone(),
            &recommendation.evidence_ids,
            evidence,
        )?;
        validate_grounded_prose(
            &recommendation.rationale,
            recommendation_metrics,
            &recommendation.evidence_ids,
            evidence,
        )?;
    }
    for change in &report.rotation_changes {
        if !matches!(
            change.change_type.as_str(),
            "macro_statement" | "manual_operation"
        ) {
            return Err(error(
                "invalid_rotation_change_type",
                "rotation change type is invalid",
            ));
        }
        if !rotation_edit_operation_is_valid(change) {
            return Err(error(
                "invalid_rotation_edit_operation",
                "rotation edit operation is invalid for its input mode",
            ));
        }
        validate_short_text(&change.target)?;
        validate_short_text(&change.current)?;
        validate_short_text(&change.proposed)?;
        validate_short_text(&change.rationale)?;
        if change.evidence_ids.is_empty() {
            return Err(error(
                "rotation_change_without_evidence",
                "every rotation change must cite evidence",
            ));
        }
        validate_evidence_ids(&change.evidence_ids, evidence)?;
        if !rotation_change_has_input_evidence(change, evidence) {
            return Err(error(
                "rotation_change_target_not_grounded",
                "rotation change current value must occur in cited evidence",
            ));
        }
        if !rotation_change_has_current_guide(change, evidence) {
            return Err(error(
                "rotation_change_without_guide",
                "rotation change must cite current fact-eligible guide evidence",
            ));
        }
        if !rotation_change_numbers_are_grounded(change, evidence) {
            return Err(error(
                "rotation_change_number_not_grounded",
                "macro change contains a number absent from current input and cited guide",
            ));
        }
        if !rotation_change_has_diagnosis_evidence(change, evidence) {
            return Err(error(
                "rotation_change_without_diagnosis",
                "rotation change must cite the baseline timeline or exact input inspection",
            ));
        }
        if !rotation_change_has_comparison_evidence(change, evidence) {
            return Err(error(
                "rotation_change_not_compared",
                "rotation change must cite a same-scenario comparison that tested its input mode",
            ));
        }
    }
    Ok(())
}

fn rotation_edit_operation_is_valid(change: &RotationChangeV1) -> bool {
    match change.change_type.as_str() {
        "macro_statement" => matches!(
            change.edit_operation.as_str(),
            "replace" | "insert_before" | "insert_after"
        ),
        "manual_operation" => matches!(
            change.edit_operation.as_str(),
            "replace" | "insert_before" | "insert_after" | "remove" | "adjust_timing"
        ),
        _ => false,
    }
}

fn rotation_change_has_input_evidence(change: &RotationChangeV1, evidence: &EvidenceStore) -> bool {
    matching_rotation_input_evidence_id(change, evidence)
        .is_some_and(|evidence_id| change.evidence_ids.contains(&evidence_id))
}

fn matching_rotation_input_evidence_id(
    change: &RotationChangeV1,
    evidence: &EvidenceStore,
) -> Option<String> {
    let expected_mode = if change.change_type == "macro_statement" {
        "macro"
    } else {
        "manual_sequence"
    };
    evidence.iter().find_map(|(id, item)| {
        let tool = item.get("tool_name").and_then(Value::as_str);
        let mode_matches = match tool {
            Some("get_current_scenario") => {
                item.pointer("/result/rotation_input/mode")
                    .and_then(Value::as_str)
                    == Some(expected_mode)
            }
            Some("inspect_rotation_input") => change.change_type == "manual_operation",
            _ => false,
        };
        (mode_matches && evidence_contains_exact_string(item, &change.current)).then(|| id.clone())
    })
}

fn rotation_change_has_current_guide(change: &RotationChangeV1, evidence: &EvidenceStore) -> bool {
    change.evidence_ids.iter().any(|id| {
        let Some(item) = evidence.get(id) else {
            return false;
        };
        item.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")
            && item
                .pointer("/result/results")
                .and_then(Value::as_array)
                .is_some_and(|results| {
                    results.iter().any(|result| {
                        result.get("fact_eligible").and_then(Value::as_bool) == Some(true)
                            && matches!(
                                result.get("version_match").and_then(Value::as_str),
                                Some("current_exact") | Some("current_compatible")
                            )
                    })
                })
    })
}

fn rotation_change_numbers_are_grounded(
    change: &RotationChangeV1,
    evidence: &EvidenceStore,
) -> bool {
    if change.change_type != "macro_statement" {
        return true;
    }
    let mut allowed = numeric_literals(&change.current);
    allowed.extend(cited_knowledge_numeric_literals(
        &change.evidence_ids,
        evidence,
    ));
    numeric_literals(&change.proposed)
        .iter()
        .all(|literal| matches_knowledge_literal(*literal, &allowed))
}

fn rotation_change_has_comparison_evidence(
    change: &RotationChangeV1,
    evidence: &EvidenceStore,
) -> bool {
    change.evidence_ids.iter().any(|id| {
        let Some(item) = evidence.get(id) else {
            return false;
        };
        if item.get("tool_name").and_then(Value::as_str) != Some("compare_scenarios") {
            return false;
        }
        if change.change_type == "macro_statement" {
            return item
                .get("args")
                .is_some_and(|args| value_contains_string_fragment(args, &change.proposed));
        }
        item.pointer("/result/candidates")
            .and_then(Value::as_array)
            .is_some_and(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate
                        .get("changes")
                        .and_then(Value::as_array)
                        .is_some_and(|changes| {
                            changes.iter().any(|field| {
                                field
                                    .get("field")
                                    .and_then(Value::as_str)
                                    .is_some_and(|name| {
                                        name == "sequence"
                                            || name.ends_with(".sequence")
                                            || name == "timing_offsets"
                                            || name.ends_with(".timing_offsets")
                                            || name == "pauses"
                                            || name.ends_with(".pauses")
                                    })
                            })
                        })
                })
            })
    })
}

fn rotation_change_has_diagnosis_evidence(
    change: &RotationChangeV1,
    evidence: &EvidenceStore,
) -> bool {
    change.evidence_ids.iter().any(|id| {
        evidence.get(id).is_some_and(|item| {
            (item.get("tool_name").and_then(Value::as_str) == Some("analyze_timeline")
                && item.pointer("/result/diagnostic_profile").is_some())
                || (change.change_type == "manual_operation"
                    && item.get("tool_name").and_then(Value::as_str)
                        == Some("inspect_rotation_input"))
        })
    })
}

fn value_contains_string_fragment(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(actual) => actual.contains(expected),
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_string_fragment(item, expected)),
        Value::Object(object) => object
            .values()
            .any(|item| value_contains_string_fragment(item, expected)),
        _ => false,
    }
}

fn evidence_contains_exact_string(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(actual) => actual == expected,
        Value::Array(items) => items
            .iter()
            .any(|item| evidence_contains_exact_string(item, expected)),
        Value::Object(object) => object
            .values()
            .any(|item| evidence_contains_exact_string(item, expected)),
        _ => false,
    }
}

pub fn cited_evidence_ids(report: &AgentReportContentV1) -> Vec<String> {
    let mut ids = report
        .findings
        .iter()
        .flat_map(|finding| finding.evidence_ids.iter())
        .chain(
            report
                .recommendations
                .iter()
                .flat_map(|recommendation| recommendation.evidence_ids.iter()),
        )
        .chain(
            report
                .rotation_changes
                .iter()
                .flat_map(|change| change.evidence_ids.iter()),
        )
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

pub fn cited_knowledge_sources(
    evidence_ids: &[String],
    evidence: &EvidenceStore,
) -> Vec<AgentKnowledgeSourceV1> {
    let mut sources = Vec::<AgentKnowledgeSourceV1>::new();
    let mut positions = HashMap::<String, usize>::new();
    for evidence_id in evidence_ids {
        let Some(envelope) = evidence.get(evidence_id) else {
            continue;
        };
        if envelope.get("tool_name").and_then(Value::as_str) != Some("search_knowledge_base") {
            continue;
        }
        let Some(results) = envelope
            .pointer("/result/results")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for result in results {
            let Some(document_id) = short_value(result, "document_id", 128) else {
                continue;
            };
            let source_url =
                short_value(result, "source_url", 2048).filter(|url| safe_public_url(url));
            let yuque_url =
                short_value(result, "yuque_url", 2048).filter(|url| safe_public_url(url));
            let Some(primary_url) = source_url.clone().or_else(|| yuque_url.clone()) else {
                continue;
            };
            let key = format!("{document_id}\u{0}{primary_url}");
            if let Some(existing) = positions
                .get(&key)
                .and_then(|position| sources.get_mut(*position))
            {
                if !existing.evidence_ids.contains(evidence_id) {
                    existing.evidence_ids.push(evidence_id.clone());
                }
                continue;
            }
            let Some(title) = short_value(result, "title", 512) else {
                continue;
            };
            positions.insert(key, sources.len());
            sources.push(AgentKnowledgeSourceV1 {
                document_id,
                title,
                season: short_value(result, "season", 128).unwrap_or_default(),
                category: short_value(result, "category", 128).unwrap_or_default(),
                source_url: primary_url,
                yuque_url: yuque_url.unwrap_or_default(),
                source_site: short_value(result, "source_site", 256).unwrap_or_default(),
                source_updated_at: short_value(result, "source_updated_at", 128)
                    .unwrap_or_default(),
                version_match: short_value(result, "version_match", 64).unwrap_or_default(),
                fact_eligible: result
                    .get("fact_eligible")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                version_warning: short_value(result, "version_warning", 512),
                document_hash: short_value(result, "document_hash", 128).unwrap_or_default(),
                evidence_ids: vec![evidence_id.clone()],
            });
        }
    }
    sources
}

fn short_value(value: &Value, field: &str, max_chars: usize) -> Option<String> {
    let value = value.get(field)?.as_str()?.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.chars().take(max_chars).collect())
}

fn safe_public_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url.chars().any(char::is_control)
}

fn validate_evidence_ids<'a>(
    ids: &'a [String],
    evidence: &EvidenceStore,
) -> Result<HashSet<&'a str>, ReportValidationError> {
    let mut unique = HashSet::new();
    for id in ids {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error("invalid_evidence_id", "evidence id is invalid"));
        }
        if !evidence.contains_key(id) {
            return Err(error(
                "unknown_evidence_id",
                "report cites evidence outside this run",
            ));
        }
        if !unique.insert(id.as_str()) {
            return Err(error(
                "duplicate_evidence_id",
                "evidence ids must be unique within a claim",
            ));
        }
    }
    Ok(unique)
}

fn validate_metric(
    metric: &GroundedMetricV1,
    cited: &HashSet<&str>,
    evidence: &EvidenceStore,
) -> Result<(), ReportValidationError> {
    validate_short_text(&metric.label)?;
    validate_short_text(&metric.unit)?;
    if !metric.value.is_finite() {
        return Err(error("invalid_metric", "metric value must be finite"));
    }
    if !cited.contains(metric.evidence_id.as_str()) {
        return Err(error(
            "uncited_metric",
            "metric evidence must also be cited by its finding",
        ));
    }
    if !metric.json_pointer.starts_with("/result/") || metric.json_pointer.len() > 256 {
        return Err(error(
            "invalid_evidence_pointer",
            "metric JSON Pointer must address the evidence result",
        ));
    }
    let source = evidence
        .get(&metric.evidence_id)
        .filter(|envelope| {
            envelope.get("evidence_id").and_then(Value::as_str) == Some(metric.evidence_id.as_str())
                && metric_tool_allowed(envelope)
        })
        .and_then(|envelope| envelope.pointer(&metric.json_pointer))
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            error(
                "missing_metric_source",
                format!(
                    "metric '{}' has no numeric source at evidence {} pointer {}",
                    metric.label, metric.evidence_id, metric.json_pointer,
                ),
            )
        })?;
    if !metric_value_matches_source(metric.value, &metric.unit, source) {
        return Err(error(
            "metric_value_mismatch",
            format!(
                "metric '{}' reported {} {}, but evidence {} pointer {} contains {}",
                metric.label, metric.value, metric.unit,
                metric.evidence_id, metric.json_pointer, source,
            ),
        ));
    }
    Ok(())
}

fn metric_tool_allowed(envelope: &Value) -> bool {
    matches!(
        envelope.get("tool_name").and_then(Value::as_str),
        Some(
            "simulate_scenario"
                | "compare_scenarios"
                | "analyze_timeline"
                | "inspect_timeline_events"
                | "inspect_equipment_workspace"
                | "compare_focused_equipment"
                | "compare_equipment_strategies"
        )
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NumericLiteral {
    value: f64,
    multiplier: f64,
    decimal_places: u32,
    percent: bool,
    ordinary_count: bool,
    identifier: bool,
    start: usize,
    end: usize,
}

impl NumericLiteral {
    fn scaled_value(self) -> f64 {
        self.value * self.multiplier
    }
}

fn validate_grounded_prose<'a>(
    value: &str,
    metrics: impl IntoIterator<Item = &'a GroundedMetricV1>,
    evidence_ids: &[String],
    evidence: &EvidenceStore,
) -> Result<(), ReportValidationError> {
    validate_short_text(value)?;
    let metrics = metrics.into_iter().collect::<Vec<_>>();
    let metric_values = metrics
        .iter()
        .map(|metric| metric.value)
        .collect::<Vec<_>>();
    let knowledge_literals = cited_knowledge_numeric_literals(evidence_ids, evidence);
    let tool_values = cited_tool_numeric_values(evidence_ids, evidence);
    if let Some(literal) = numeric_literals(value).into_iter().find(|literal| {
        !literal.ordinary_count
            && !literal.identifier
            && !matches_metric(*literal, &metric_values)
            && !matches_metric_label_literal(value, *literal, &metrics)
            && !matches_knowledge_literal(*literal, &knowledge_literals)
            && !matches_tool_value(*literal, &tool_values)
            && !matches_derived_tool_value(*literal, &tool_values)
    }) {
        return Err(error(
            "numeric_prose_claim",
            format!(
                "numeric claim '{}' is unsupported by this claim's cited evidence in: {}",
                &value[literal.start..literal.end],
                value.chars().take(180).collect::<String>(),
            ),
        ));
    }
    Ok(())
}

fn numeric_literals(value: &str) -> Vec<NumericLiteral> {
    let bytes = value.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let signed = matches!(bytes[index], b'+' | b'-')
            && bytes.get(index + 1).is_some_and(u8::is_ascii_digit);
        let leading_decimal =
            bytes[index] == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit);
        if !bytes[index].is_ascii_digit() && !signed && !leading_decimal {
            index += 1;
            continue;
        }

        let start = index;
        if signed {
            index += 1;
        }
        let mut decimal_places = 0_u32;
        let mut has_decimal = false;
        let mut has_exponent = false;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b',') {
            index += 1;
        }
        if index < bytes.len()
            && bytes[index] == b'.'
            && bytes.get(index + 1).is_some_and(u8::is_ascii_digit)
        {
            has_decimal = true;
            index += 1;
            let decimal_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            decimal_places = u32::try_from(index - decimal_start).unwrap_or(u32::MAX);
        }
        if matches!(bytes.get(index), Some(&b'e') | Some(&b'E')) {
            let exponent_marker = index;
            index += 1;
            if matches!(bytes.get(index), Some(&b'+') | Some(&b'-')) {
                index += 1;
            }
            let exponent_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index > exponent_start {
                has_exponent = true;
            } else {
                index = exponent_marker;
            }
        }
        if index == start || (signed && index == start + 1) {
            index += 1;
            continue;
        }

        let number_end = index;
        let embedded_identifier = start
            .checked_sub(1)
            .and_then(|position| bytes.get(position))
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            || bytes
                .get(number_end)
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_');
        let mut suffix = index;
        while bytes.get(suffix).is_some_and(u8::is_ascii_whitespace) {
            suffix += 1;
        }
        let multiplier = if value
            .get(suffix..)
            .is_some_and(|tail| tail.starts_with('万'))
        {
            index = suffix + '万'.len_utf8();
            10_000.0
        } else if value
            .get(suffix..)
            .is_some_and(|tail| tail.starts_with('亿'))
        {
            index = suffix + '亿'.len_utf8();
            100_000_000.0
        } else {
            1.0
        };
        let percent = if bytes.get(index) == Some(&b'%') {
            index += 1;
            true
        } else if value
            .get(index..)
            .is_some_and(|tail| tail.starts_with('％'))
        {
            index += '％'.len_utf8();
            true
        } else {
            false
        };
        if embedded_identifier {
            continue;
        }
        let raw = value[start..number_end].replace(',', "");
        if let Ok(parsed) = raw.parse::<f64>() {
            let ordinary_count = !signed
                && !has_decimal
                && !has_exponent
                && !percent
                && multiplier == 1.0
                && !value[start..number_end].contains(',')
                && (0.0..=12.0).contains(&parsed);
            // Digits embedded in identifiers (for example a source account or
            // version-like token) are not quantitative claims.
            let inside_rotation_anchor = value[..start].rfind("[[").is_some_and(|open| {
                value[..start].rfind("]]").is_none_or(|close| close < open)
                    && (value[open..start].contains("|op:") || value[open..start].contains("|ev:"))
            });
            let identifier = inside_rotation_anchor
                || start
                    .checked_sub(1)
                    .and_then(|offset| bytes.get(offset))
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                || bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_');
            literals.push(NumericLiteral {
                value: parsed,
                multiplier,
                decimal_places,
                percent,
                ordinary_count,
                identifier,
                start,
                end: index,
            });
        }
    }
    literals
}

fn sanitize_unsupported_numeric_prose(
    value: &mut String,
    metrics: &[GroundedMetricV1],
    evidence_ids: &[String],
    evidence: &EvidenceStore,
    fallback: &str,
) -> usize {
    let metric_values = metrics
        .iter()
        .map(|metric| metric.value)
        .collect::<Vec<_>>();
    let metric_refs = metrics.iter().collect::<Vec<_>>();
    let knowledge_literals = cited_knowledge_numeric_literals(evidence_ids, evidence);
    let tool_values = cited_tool_numeric_values(evidence_ids, evidence);
    let unsupported = numeric_literals(value)
        .into_iter()
        .filter(|literal| {
            !literal.ordinary_count
                && !literal.identifier
                && !matches_metric(*literal, &metric_values)
                && !matches_metric_label_literal(value, *literal, &metric_refs)
                && !matches_knowledge_literal(*literal, &knowledge_literals)
                && !matches_tool_value(*literal, &tool_values)
                && !matches_derived_tool_value(*literal, &tool_values)
        })
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return 0;
    }
    // Keep independent supported sentences. Replace the complete unsupported
    // sentence with an explicit gap so removing a number cannot silently turn
    // a failed comparison into an apparently verified gameplay claim.
    let mut spans = unsupported
        .iter()
        .map(|literal| sentence_span(value, literal.start, literal.end))
        .collect::<Vec<_>>();
    spans.sort_unstable();
    let mut merged = Vec::<(usize, usize)>::new();
    for span in spans {
        if let Some(last) = merged.last_mut().filter(|last| span.0 <= last.1) {
            last.1 = last.1.max(span.1);
        } else {
            merged.push(span);
        }
    }
    for (start, end) in merged.into_iter().rev() {
        value.replace_range(start..end, fallback);
    }
    *value = value.trim().to_string();
    sanitize_text_field(value, fallback);
    unsupported.len()
}

fn sentence_span(value: &str, literal_start: usize, literal_end: usize) -> (usize, usize) {
    let is_boundary = |character: char| matches!(character, '。' | '！' | '？' | ';' | '；' | '\n');
    let start = value[..literal_start]
        .char_indices()
        .rev()
        .find(|(_, character)| is_boundary(*character))
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    let end = value[literal_end..]
        .char_indices()
        .find(|(_, character)| is_boundary(*character))
        .map(|(index, character)| literal_end + index + character.len_utf8())
        .unwrap_or(value.len());
    (start, end)
}

fn cited_knowledge_numeric_literals(
    evidence_ids: &[String],
    evidence: &EvidenceStore,
) -> Vec<NumericLiteral> {
    let mut literals = Vec::new();
    for evidence_id in evidence_ids {
        let Some(envelope) = evidence.get(evidence_id) else {
            continue;
        };
        if envelope.get("tool_name").and_then(Value::as_str) != Some("search_knowledge_base") {
            continue;
        }
        let Some(results) = envelope
            .pointer("/result/results")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for result in results {
            if !result
                .get("fact_eligible")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            for field in ["title", "heading", "snippet", "season"] {
                if let Some(text) = result.get(field).and_then(Value::as_str) {
                    literals.extend(numeric_literals(text));
                }
            }
        }
    }
    literals
}

fn matches_knowledge_literal(literal: NumericLiteral, sources: &[NumericLiteral]) -> bool {
    sources.iter().any(|source| {
        if literal.percent != source.percent {
            return false;
        }
        let literal_value = literal.scaled_value();
        let source_value = source.scaled_value();
        let tolerance = literal_value.abs().max(source_value.abs()).max(1.0) * 1e-9;
        (literal_value - source_value).abs() <= tolerance
    })
}

fn cited_tool_numeric_values(evidence_ids: &[String], evidence: &EvidenceStore) -> Vec<f64> {
    let mut values = Vec::new();
    for evidence_id in evidence_ids {
        let Some(envelope) = evidence
            .get(evidence_id)
            .filter(|item| prose_numeric_tool_allowed(item))
        else {
            continue;
        };
        if let Some(result) = envelope.get("result") {
            collect_tool_numeric_values(result, None, &mut values);
        }
    }
    values.sort_by(f64::total_cmp);
    values.dedup_by(|left, right| (*left - *right).abs() <= 1e-12);
    values
}

fn prose_numeric_tool_allowed(envelope: &Value) -> bool {
    metric_tool_allowed(envelope)
        || matches!(
            envelope.get("tool_name").and_then(Value::as_str),
            Some("get_current_scenario" | "inspect_rotation_input")
        )
}

fn collect_tool_numeric_values(value: &Value, field: Option<&str>, values: &mut Vec<f64>) {
    match value {
        Value::Number(number) => {
            let identifier = field.is_some_and(|name| {
                name.ends_with("_id")
                    || name.ends_with("_hash")
                    || name.contains("fingerprint")
                    || name == "schema_version"
            });
            if !identifier {
                if let Some(number) = number.as_f64().filter(|number| number.is_finite()) {
                    values.push(number);
                }
            }
        }
        Value::String(text)
            if field.is_some_and(|name| {
                matches!(
                    name,
                    "before" | "after" | "statement" | "macro_text" | "name" | "label"
                )
            }) =>
        {
            values.extend(
                numeric_literals(text)
                    .into_iter()
                    .map(NumericLiteral::scaled_value),
            );
        }
        Value::Array(items) => {
            for item in items {
                collect_tool_numeric_values(item, field, values);
            }
        }
        Value::Object(object) => {
            for (name, item) in object {
                collect_tool_numeric_values(item, Some(name), values);
            }
        }
        _ => {}
    }
}

fn matches_tool_value(literal: NumericLiteral, sources: &[f64]) -> bool {
    sources.iter().any(|source| {
        let magnitude = source.abs();
        let candidates = if literal.percent {
            [magnitude, magnitude * 100.0]
        } else {
            [*source, magnitude]
        };
        candidates.into_iter().any(|candidate| {
            let display_tolerance = (if literal.decimal_places == 0 {
                0.5
            } else {
                0.5 * 10_f64.powi(-(literal.decimal_places as i32))
            }) * literal.multiplier;
            let floating_tolerance = candidate.abs().max(1.0) * 1e-9;
            (literal.scaled_value() - candidate).abs() <= display_tolerance.max(floating_tolerance)
        })
    })
}

fn matches_derived_tool_value(literal: NumericLiteral, sources: &[f64]) -> bool {
    if sources.is_empty() {
        return false;
    }
    let displayed = literal.scaled_value();
    let display_tolerance = (if literal.decimal_places == 0 {
        0.5
    } else {
        0.5 * 10_f64.powi(-(literal.decimal_places as i32))
    }) * literal.multiplier;

    // Common lossless presentation conversions: seconds/minutes and values
    // normalized per minute. They remain derived from cited simulator facts.
    if !literal.percent
        && sources.iter().any(|source| {
            [source / 60.0, source * 60.0].into_iter().any(|candidate| {
                (displayed - candidate).abs()
                    <= display_tolerance.max(candidate.abs().max(1.0) * 1e-9)
            })
        })
    {
        return true;
    }

    if !literal.percent || displayed.abs() < 1e-12 {
        return false;
    }
    // Percentages such as 45/1750, 54/601, or delta/baseline are legitimate
    // arithmetic restatements. Verify the equation against two cited values
    // instead of requiring the exact percentage to be stored redundantly.
    let mut magnitudes = sources.iter().map(|value| value.abs()).collect::<Vec<_>>();
    magnitudes.sort_by(f64::total_cmp);
    magnitudes.dedup_by(|left, right| (*left - *right).abs() <= 1e-12);
    sources.iter().any(|numerator| {
        let expected_denominator = numerator.abs() * 100.0 / displayed.abs();
        let insertion = magnitudes
            .binary_search_by(|candidate| candidate.total_cmp(&expected_denominator))
            .unwrap_or_else(|index| index);
        [insertion.saturating_sub(1), insertion]
            .into_iter()
            .filter_map(|index| magnitudes.get(index))
            .any(|denominator| {
                denominator.abs() > 1e-12
                    && ((numerator.abs() / denominator.abs()) * 100.0 - displayed.abs()).abs()
                        <= display_tolerance.max(0.005)
            })
    })
}

fn matches_metric_label_literal(
    prose: &str,
    literal: NumericLiteral,
    metrics: &[&GroundedMetricV1],
) -> bool {
    let prose_before = prose[..literal.start].chars().next_back();
    let prose_after = prose[literal.end..].chars().next();
    metrics.iter().any(|metric| {
        numeric_literals(&metric.label)
            .into_iter()
            .any(|label_literal| {
                if literal.percent != label_literal.percent
                    || (literal.scaled_value() - label_literal.scaled_value()).abs() > f64::EPSILON
                {
                    return false;
                }
                let label_before = metric.label[..label_literal.start].chars().next_back();
                let label_after = metric.label[label_literal.end..].chars().next();
                prose_before == label_before && prose_after == label_after
            })
    })
}

fn matches_metric(literal: NumericLiteral, metric_values: &[f64]) -> bool {
    metric_values.iter().any(|metric| {
        let candidates = if literal.percent {
            [*metric, *metric * 100.0]
        } else {
            [*metric, *metric]
        };
        candidates.into_iter().any(|candidate| {
            let display_tolerance = (if literal.decimal_places == 0 {
                0.5
            } else {
                0.5 * 10_f64.powi(-(literal.decimal_places as i32))
            }) * literal.multiplier;
            let floating_tolerance = candidate.abs().max(1.0) * 1e-9;
            (literal.scaled_value() - candidate).abs() <= display_tolerance.max(floating_tolerance)
        })
    })
}

fn validate_short_text(value: &str) -> Result<(), ReportValidationError> {
    let count = value.chars().count();
    if !(1..=1024).contains(&count) || value.chars().any(char::is_control) {
        Err(error("invalid_report_text", "report text is invalid"))
    } else {
        Ok(())
    }
}

fn error(code: &'static str, message: impl Into<String>) -> ReportValidationError {
    ReportValidationError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rotation_anchor_numbers_are_navigation_ids_not_numeric_claims() {
        let literals = numeric_literals(
            "[[这些绝刀|op:24,57,91]]、[[这一段|op:20-25]] 与 [[两次触顶|ev:42,97]]",
        );
        assert_eq!(literals.len(), 4);
        assert!(literals.iter().all(|literal| literal.identifier));
    }

    #[test]
    fn missing_prose_citations_are_repaired_instead_of_borrowing_all_run_evidence() {
        let mut value = report();
        value.findings[0].evidence_ids.clear();
        value.findings[0].metrics.clear();
        value.findings[0].title = "释放位置需要检查".to_string();
        value.findings[0].explanation =
            "读取对应事件后再比较释放条件。".to_string();
        let raw = serde_json::to_string(&value).unwrap();

        assert_eq!(
            parse_and_validate_report(&raw, &evidence()).unwrap_err().code,
            "finding_without_evidence"
        );
        assert_eq!(
            parse_and_salvage_report(&raw, &evidence()).unwrap_err().code,
            "empty_salvaged_report"
        );
    }

    #[test]
    fn fabricated_citations_cannot_be_replaced_with_unrelated_baseline_evidence() {
        let mut value = report();
        value.summary = "这些释放位置需要比较。".to_string();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics.clear();
        let raw = serde_json::to_string(&value).unwrap();

        assert_eq!(
            parse_and_validate_report(&raw, &evidence()).unwrap_err().code,
            "unknown_evidence_id"
        );
        assert_eq!(
            parse_and_salvage_report(&raw, &evidence()).unwrap_err().code,
            "empty_salvaged_report"
        );
    }

    #[test]
    fn unrelated_tool_numbers_do_not_automatically_become_claim_evidence() {
        let store = BTreeMap::from([
            (
                "a".repeat(64),
                json!({
                    "evidence_id": "a".repeat(64),
                    "tool_name": "inspect_timeline_events",
                    "result": {"total_matches": 17}
                }),
            ),
            (
                "c".repeat(64),
                json!({
                    "evidence_id": "c".repeat(64),
                    "tool_name": "analyze_timeline",
                    "result": {"rage": {"overflow_total": 45}}
                }),
            ),
        ]);
        let mut value = report();
        value.findings[0].metrics.clear();
        value.findings[0].explanation =
            "这些事件需要继续比较释放条件。实测溢出怒气为45点。".to_string();
        let raw = serde_json::to_string(&value).unwrap();

        assert_eq!(
            parse_and_validate_report(&raw, &store).unwrap_err().code,
            "numeric_prose_claim"
        );
        let salvaged = parse_and_salvage_report(&raw, &store).unwrap();
        assert_eq!(salvaged.content.findings[0].evidence_ids, vec!["a".repeat(64)]);
        assert_eq!(
            salvaged.content.findings[0].explanation,
            "这些事件需要继续比较释放条件。这项判断的数值依据尚待核实。"
        );
    }

    #[test]
    fn provider_rotation_anchor_braces_are_normalized() {
        let mut value = report();
        value.findings[0].explanation = "检查[[两次绝刀|ev:4,9}}的实际状态。".to_string();
        let parsed =
            parse_and_validate_report(&serde_json::to_string(&value).unwrap(), &evidence())
                .unwrap();
        assert_eq!(
            parsed.content.findings[0].explanation,
            "检查[[两次绝刀|ev:4,9]]的实际状态。"
        );
    }

    #[test]
    fn a_completed_baseline_preserves_the_unrun_candidate_boundary() {
        let mut value = report();
        value.summary = "基线已完成；该候选未运行模拟，收益仍待验证。".to_string();
        value.findings[0].explanation =
            "当前循环已有模拟记录。新方案未执行模拟，仅能先提出机制假设。".to_string();
        value.limitations = vec![
            "该候选未运行模拟，收益仍待验证。".to_string(),
            "停手后的恢复方案缺少模拟数据。".to_string(),
        ];
        let raw = serde_json::to_string(&value).unwrap();

        let validated = parse_and_validate_report(&raw, &evidence()).unwrap();
        assert_eq!(validated.content.summary, value.summary);
        assert_eq!(validated.content.findings[0].explanation, value.findings[0].explanation);
        assert_eq!(validated.content.limitations, value.limitations);
        assert_eq!(validated.sanitized_claims, 0);

        let salvaged = parse_and_salvage_report(&raw, &evidence()).unwrap();
        assert_eq!(salvaged.content.summary, value.summary);
        assert_eq!(salvaged.content.findings[0].explanation, value.findings[0].explanation);
        for boundary in &value.limitations {
            assert!(salvaged.content.limitations.contains(boundary));
        }
    }

    fn evidence() -> EvidenceStore {
        BTreeMap::from([(
            "a".repeat(64),
            json!({
                "evidence_id": "a".repeat(64),
                "tool_name": "simulate_scenario",
                "result": {"dps": 123.5, "ratio": 0.3}
            }),
        )])
    }

    fn knowledge_evidence() -> EvidenceStore {
        BTreeMap::from([
            (
                "b".repeat(64),
                json!({
                    "evidence_id": "b".repeat(64),
                    "tool_name": "search_knowledge_base",
                    "result": {
                        "results": [{
                            "title": "暗影千机 2026 宏说明",
                            "heading": "延迟阈值",
                            "snippet": "宏阈值可从 0.15 调到 0.20；80ms 延迟环境需要自行实测。",
                            "season": "暗影千机（2026）",
                            "fact_eligible": true,
                            "version_match": "current_exact"
                        }]
                    }
                }),
            ),
            (
                "c".repeat(64),
                json!({
                    "evidence_id": "c".repeat(64),
                    "tool_name": "search_knowledge_base",
                    "result": {
                        "results": [{
                            "title": "另一篇未引用资料",
                            "heading": "未引用阈值",
                            "snippet": "另一种设置使用 0.25，伤害占比为 30%。",
                            "season": "暗影千机（2026）",
                            "fact_eligible": true
                        }]
                    }
                }),
            ),
        ])
    }

    fn report() -> AgentReportContentV1 {
        AgentReportContentV1 {
            schema_version: AGENT_REPORT_CONTENT_SCHEMA_V1.to_string(),
            summary: "基线模拟已完成。".to_string(),
            findings: vec![AgentFindingV1 {
                title: "输出基线".to_string(),
                explanation: "数值来自确定性模拟。".to_string(),
                evidence_ids: vec!["a".repeat(64)],
                metrics: vec![GroundedMetricV1 {
                    label: "DPS".to_string(),
                    value: 123.5,
                    unit: "damage_per_second".to_string(),
                    evidence_id: "a".repeat(64),
                    json_pointer: "/result/dps".to_string(),
                }],
            }],
            recommendations: Vec::new(),
        rotation_changes: Vec::new(),
        artifacts: Vec::new(),
            limitations: vec!["只验证了当前场景。".to_string()],
            refusal_reason: None,
        }
    }

    #[test]
    fn grounded_metric_passes() {
        validate_report(&report(), &evidence()).unwrap();
    }

    #[test]
    fn equipment_comparison_metrics_are_publishable_with_exact_pointers() {
        let evidence_id = "e".repeat(64);
        let store = BTreeMap::from([(
            evidence_id.clone(),
            json!({
                "evidence_id": evidence_id,
                "tool_name": "compare_equipment_strategies",
                "result": {
                    "comparison": {
                        "before_dps": 329798.0,
                        "after_dps": 205525.0,
                        "dps_delta_percent": -37.681550524866736,
                        "panel_rows": [{"label": "外功攻击", "delta": -11019.0}]
                    }
                }
            }),
        )]);
        let mut value = report();
        value.findings[0].evidence_ids = vec!["e".repeat(64)];
        value.findings[0].explanation = "同循环实测 DPS 为 329798。".to_string();
        value.findings[0].metrics[0] = GroundedMetricV1 {
            label: "四件套 DPS".to_string(),
            value: 329798.0,
            unit: "damage_per_second".to_string(),
            evidence_id: "e".repeat(64),
            json_pointer: "/result/comparison/before_dps".to_string(),
        };

        validate_report(&value, &store).unwrap();
    }

    #[test]
    fn fractional_evidence_accepts_a_rounded_percentage_metric() {
        let mut store = evidence();
        store.get_mut(&"a".repeat(64)).unwrap()["result"]["ratio"] = json!(0.335795);
        let mut value = report();
        value.findings[0].metrics[0] = GroundedMetricV1 {
            label: "核心技能伤害占比".to_string(),
            value: 33.58,
            unit: "percent".to_string(),
            evidence_id: "a".repeat(64),
            json_pointer: "/result/ratio".to_string(),
        };
        value.findings[0].explanation = "核心技能伤害占比为 33.58%。".to_string();

        validate_report(&value, &store).unwrap();
        let salvaged =
            parse_and_salvage_report(&serde_json::to_string(&value).unwrap(), &store).unwrap();
        assert_eq!(salvaged.content.findings[0].metrics.len(), 1);

        value.findings[0].metrics[0].value = 34.0;
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "metric_value_mismatch"
        );
    }

    #[test]
    fn projected_ranked_damage_pointer_resolves_to_immutable_skill_source() {
        let evidence_id = "d".repeat(64);
        let store = BTreeMap::from([(
            evidence_id.clone(),
            json!({
                "evidence_id": evidence_id,
                "tool_name": "simulate_scenario",
                "result": {
                    "skills": [
                        {"name": "斩刀", "total_damage": 66_000_000.0, "damage_share": 0.075},
                        {"name": "绝刀·50怒", "total_damage": 296_000_000.0, "damage_share": 0.3327}
                    ]
                }
            }),
        )]);
        let mut value = report();
        value.findings[0].evidence_ids = vec!["d".repeat(64)];
        value.findings[0].explanation = "绝刀·50怒是当前最大伤害来源。".to_string();
        value.findings[0].metrics[0] = GroundedMetricV1 {
            label: "绝刀·50怒伤害占比".to_string(),
            value: 33.27,
            unit: "percent".to_string(),
            evidence_id: "d".repeat(64),
            json_pointer: "/result/ranked_damage_sources/0/damage_share".to_string(),
        };

        let parsed = parse_and_validate_report(&serde_json::to_string(&value).unwrap(), &store)
            .expect("projected pointer should be normalized");
        assert_eq!(
            parsed.content.findings[0].metrics[0].json_pointer,
            "/result/skills/1/damage_share"
        );
        assert_eq!(parsed.normalized_metric_citations, 1);
    }

    #[test]
    fn large_simulator_metric_accepts_integer_display_rounding() {
        let mut store = evidence();
        store.get_mut(&"a".repeat(64)).unwrap()["result"]["dps"] = json!(99021.01333333334);
        let mut value = report();
        value.findings[0].metrics[0].value = 99021.0;

        validate_report(&value, &store).unwrap();

        value.findings[0].metrics[0].value = 99022.0;
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "metric_value_mismatch"
        );
    }

    #[test]
    fn prose_can_restate_numbers_from_its_cited_simulator_evidence() {
        let id = "d".repeat(64);
        let store = BTreeMap::from([(
            id.clone(),
            json!({
                "evidence_id": id,
                "tool_name": "compare_scenarios",
                "result": {
                    "baseline": {
                        "skills": [{
                            "name": "绝刀·50怒",
                            "damage_share": 0.335795,
                            "event_count": 79,
                            "skill_id": 13055
                        }]
                    },
                    "candidates": [{
                        "delta_percent": -0.991344,
                        "changes": [{
                            "before": "bufftime:嗜血<5.3",
                            "after": "bufftime:嗜血<5.0"
                        }]
                    }]
                }
            }),
        )]);
        let mut value = report();
        value.summary =
            "绝刀·50怒共 79 次、伤害占比 33.6%；阈值 5.3 调到 5.0 后，DPS 下降约 0.99%。"
                .to_string();
        value.findings[0].evidence_ids = vec!["d".repeat(64)];
        value.findings[0].metrics.clear();
        value.findings[0].explanation = "这些数字均来自同场景模拟与对比。".to_string();

        validate_report(&value, &store).unwrap();

        value.summary = "未经验证的伤害占比为 42.7%。".to_string();
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "numeric_prose_claim"
        );
    }

    #[test]
    fn omitted_finding_metrics_defaults_to_an_empty_list() {
        let mut value = serde_json::to_value(report()).unwrap();
        value["findings"][0]
            .as_object_mut()
            .unwrap()
            .remove("metrics");
        let parsed = parse_and_validate_report(&value.to_string(), &evidence()).unwrap();

        assert!(parsed.content.findings[0].metrics.is_empty());
    }

    #[test]
    fn rotation_change_requires_exact_current_input_in_cited_evidence() {
        let scenario_id = "d".repeat(64);
        let mut store = evidence();
        store.extend(knowledge_evidence());
        store.insert(
            scenario_id.clone(),
            json!({
                "evidence_id": scenario_id,
                "tool_name": "get_current_scenario",
                "result": {
                    "rotation_input": {
                        "mode": "macro",
                        "macro_statements": [{
                            "statement": "/cast [rage>64] 盾飞"
                        }]
                    }
                }
            }),
        );
        let comparison_id = "e".repeat(64);
        store.insert(
            comparison_id.clone(),
            json!({
                "evidence_id": comparison_id,
                "tool_name": "compare_scenarios",
                "args": [{
                    "label": "宏候选",
                    "patch": {
                        "macro_text": "/cast [rage>64&nobuff:嗜血] 盾飞"
                    }
                }],
                "result": {
                    "baseline": {"dps": 123.5},
                    "candidates": [{"delta_dps": 0.0, "same_fingerprint": true}]
                }
            }),
        );
        let diagnosis_id = "f".repeat(64);
        store.insert(
            diagnosis_id.clone(),
            json!({
                "evidence_id": diagnosis_id,
                "tool_name": "analyze_timeline",
                "result": {"diagnostic_profile": {"input_mode": "macro"}}
            }),
        );
        let mut value = report();
        value.rotation_changes.push(RotationChangeV1 {
            change_type: "macro_statement".to_string(),
            edit_operation: "replace".to_string(),
            target: "宏第 1 行".to_string(),
            current: "/cast [rage>64] 盾飞".to_string(),
            proposed: "/cast [rage>64&nobuff:嗜血] 盾飞".to_string(),
            rationale: "按攻略约束调整，并用同场景复测。".to_string(),
            evidence_ids: vec![
                "d".repeat(64),
                "b".repeat(64),
                "e".repeat(64),
                "f".repeat(64),
            ],
        });
        validate_report(&value, &store).unwrap();

        value.rotation_changes[0].current = "/cast [rage>99] 盾飞".to_string();
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "rotation_change_target_not_grounded"
        );

        value.rotation_changes[0].current = "/cast [rage>64] 盾飞".to_string();
        value.rotation_changes[0].proposed = "/cast [rage>44] 盾飞".to_string();
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "rotation_change_number_not_grounded"
        );
    }

    #[test]
    fn manual_change_requires_both_diagnosis_and_sequence_comparison() {
        let scenario_id = "d".repeat(64);
        let diagnosis_id = "f".repeat(64);
        let comparison_id = "e".repeat(64);
        let mut store = evidence();
        store.extend(knowledge_evidence());
        store.insert(
            scenario_id.clone(),
            json!({
                "evidence_id": scenario_id,
                "tool_name": "get_current_scenario",
                "result": {
                    "rotation_input": {
                        "mode": "manual_sequence",
                        "manual_operations": [{"skill_name": "盾击"}]
                    }
                }
            }),
        );
        store.insert(
            diagnosis_id.clone(),
            json!({
                "evidence_id": diagnosis_id,
                "tool_name": "analyze_timeline",
                "result": {"diagnostic_profile": {"input_mode": "manual_sequence"}}
            }),
        );
        store.insert(
            comparison_id.clone(),
            json!({
                "evidence_id": comparison_id,
                "tool_name": "compare_scenarios",
                "args": [{"label": "手动候选", "patch": {"sequence": ["盾压", "盾击"]}}],
                "result": {
                    "candidates": [{
                        "changes": [{
                            "field": "sequence",
                            "before": ["盾击", "盾压"],
                            "after": ["盾压", "盾击"]
                        }]
                    }]
                }
            }),
        );
        let mut value = report();
        value.rotation_changes.push(RotationChangeV1 {
            change_type: "manual_operation".to_string(),
            edit_operation: "adjust_timing".to_string(),
            target: "起手操作".to_string(),
            current: "盾击".to_string(),
            proposed: "在盾压后使用盾击".to_string(),
            rationale: "依据攻略和基线诊断提出，并在同场景序列候选中复测。".to_string(),
            evidence_ids: vec![
                "d".repeat(64),
                "b".repeat(64),
                "f".repeat(64),
                "e".repeat(64),
            ],
        });
        validate_report(&value, &store).unwrap();

        value.rotation_changes[0]
            .evidence_ids
            .retain(|id| id != &diagnosis_id);
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "rotation_change_without_diagnosis"
        );
        value.rotation_changes[0].evidence_ids.push(diagnosis_id);
        value.rotation_changes[0]
            .evidence_ids
            .retain(|id| id != &comparison_id);
        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "rotation_change_not_compared"
        );
    }

    #[test]
    fn knowledge_sources_are_derived_only_from_cited_safe_evidence() {
        let cited = "b".repeat(64);
        let uncited = "c".repeat(64);
        let result = json!({
            "document_id": "doc-current",
            "title": "暗影千机分山劲白皮书",
            "season": "暗影千机（2026）",
            "category": "白皮书",
            "source_url": "https://example.com/guide",
            "yuque_url": "https://www.yuque.com/sgyxy/cangyun/guide",
            "source_site": "example.com",
            "source_updated_at": "2026-08-26T00:00:00Z",
            "version_match": "current_exact",
            "fact_eligible": true,
            "version_warning": null,
            "document_hash": "d".repeat(64)
        });
        let store = BTreeMap::from([
            (
                cited.clone(),
                json!({
                    "evidence_id": cited,
                    "tool_name": "search_knowledge_base",
                    "result": {"results": [result, {
                        "document_id": "unsafe",
                        "title": "unsafe",
                        "source_url": "javascript:alert(1)"
                    }]}
                }),
            ),
            (
                uncited,
                json!({
                    "tool_name": "search_knowledge_base",
                    "result": {"results": [{
                        "document_id": "uncited",
                        "title": "uncited",
                        "source_url": "https://example.com/uncited"
                    }]}
                }),
            ),
        ]);

        let sources = cited_knowledge_sources(&["b".repeat(64)], &store);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].title, "暗影千机分山劲白皮书");
        assert_eq!(sources[0].version_match, "current_exact");
        assert!(sources[0].fact_eligible);
        assert_eq!(sources[0].evidence_ids, vec!["b".repeat(64)]);
    }

    #[test]
    fn upstream_report_schema_is_closed_and_strict_compatible() {
        let schema = report_content_json_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["findings"]["items"]["additionalProperties"],
            false
        );
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "refusal_reason"));
    }

    #[test]
    fn common_provider_report_aliases_are_normalized_before_validation() {
        let raw = json!({
            "schema_version": "1.0",
            "report_type": "knowledge_and_analysis",
            "title": "白刀分析",
            "evidence_ids": ["a".repeat(64)],
            "summary": "白刀应放在无血怒窗口。",
            "findings": [{
                "finding": "白刀是未触发援戈血影的苍雪刀。",
                "evidence_ids": ["a".repeat(64)],
                "evidence_pointers": ["/snippet"],
                "metrics": [{
                    "metric_name": "覆盖率",
                    "value": "50.0",
                    "unit": "%",
                    "json_pointer": "/result/coverage"
                }, {
                    "metric_name": "知识摘录",
                    "value": "白刀定义原文",
                    "unit": null,
                    "json_pointer": "/result/results/0/snippet"
                }]
            }],
            "recommendations": ["在无血怒窗口保留这套斩绝绝。"],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();

        let parsed = parse_report_json(&raw).unwrap();
        assert_eq!(parsed.schema_version, AGENT_REPORT_CONTENT_SCHEMA_V1);
        assert_eq!(parsed.findings[0].title, "分析结论");
        assert_eq!(
            parsed.findings[0].explanation,
            "白刀是未触发援戈血影的苍雪刀。"
        );
        assert_eq!(parsed.findings[0].metrics[0].unit, "percent");
        assert_eq!(parsed.findings[0].metrics[0].value, 50.0);
        assert_eq!(parsed.findings[0].metrics.len(), 1);
        assert_eq!(parsed.findings[0].metrics[0].evidence_id, "a".repeat(64));
        assert_eq!(parsed.recommendations[0].title, "建议");
        assert_eq!(
            parsed.recommendations[0].rationale,
            "在无血怒窗口保留这套斩绝绝。"
        );
        assert!(parsed.rotation_changes.is_empty());
    }

    #[test]
    fn verbose_provider_judgment_is_preserved_across_common_json_dialect() {
        let raw = json!({
            "schema_version": "1.0",
            "summary": "这套循环结构完整，主要问题更可能集中在盾刀转换。",
            "findings": [{
                "claim": "盾击三段后的衔接值得优先检查。",
                "evidence_ids": ["a".repeat(64)],
                "metrics": [{
                    "name": "平均 DPS",
                    "value": 123.0,
                    "unit": "数值",
                    "json_pointer": "/result/dps"
                }]
            }],
            "recommendations": "围绕盾刀转换做一次单变量对照。",
            "rotation_changes": [{
                "skill_name": "斩刀",
                "description": "先检查时机，再决定是否调整。",
                "edit_operation": "optional_adjust_cast_timing",
                "evidence_ids": ["a".repeat(64)]
            }],
            "limitations": [],
            "refusal_reason": null
        })
        .to_string();

        let parsed = parse_report_json(&raw).unwrap();
        assert!(parsed.findings[0].title.contains("盾击三段"));
        assert!(parsed.findings[0].explanation.contains("优先检查"));
        assert_eq!(parsed.findings[0].metrics[0].label, "平均 DPS");
        assert_eq!(parsed.findings[0].metrics[0].unit, "damage_per_second");
        assert_eq!(parsed.findings[0].metrics[0].evidence_id, "a".repeat(64));
        assert!(parsed.recommendations[0].rationale.contains("单变量对照"));
        assert!(parsed.rotation_changes.is_empty());
    }

    #[test]
    fn unknown_or_mismatched_metric_is_rejected() {
        let mut unknown = report();
        unknown.findings[0].evidence_ids[0] = "b".repeat(64);
        assert_eq!(
            validate_report(&unknown, &evidence()).unwrap_err().code,
            "unknown_evidence_id"
        );

        let mut mismatch = report();
        mismatch.findings[0].metrics[0].value = 999.0;
        assert_eq!(
            validate_report(&mismatch, &evidence()).unwrap_err().code,
            "metric_value_mismatch"
        );
    }

    #[test]
    fn knowledge_scores_cannot_be_published_as_combat_metrics() {
        let id = "b".repeat(64);
        let store = BTreeMap::from([(
            id.clone(),
            json!({
                "evidence_id": id,
                "tool_name": "search_knowledge_base",
                "result": {"results": [{"score": 99.0}]}
            }),
        )]);
        let mut value = report();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics[0] = GroundedMetricV1 {
            label: "检索分数".to_string(),
            value: 99.0,
            unit: "score".to_string(),
            evidence_id: "b".repeat(64),
            json_pointer: "/result/results/0/score".to_string(),
        };

        assert_eq!(
            validate_report(&value, &store).unwrap_err().code,
            "missing_metric_source"
        );
    }

    #[test]
    fn known_metric_citation_is_deterministically_linked_to_its_finding() {
        let mut value = report();
        value.findings[0].evidence_ids.clear();
        let parsed =
            parse_and_validate_report(&serde_json::to_string(&value).unwrap(), &evidence())
                .unwrap();

        assert_eq!(parsed.normalized_metric_citations, 1);
        assert_eq!(
            parsed.content.findings[0].evidence_ids,
            vec!["a".repeat(64)]
        );
    }

    #[test]
    fn result_relative_metric_pointer_is_normalized_when_it_exists() {
        let mut value = report();
        value.findings[0].metrics[0].json_pointer = "/dps".to_string();

        let parsed =
            parse_and_validate_report(&serde_json::to_string(&value).unwrap(), &evidence())
                .unwrap();

        assert_eq!(parsed.normalized_metric_citations, 1);
        assert_eq!(
            parsed.content.findings[0].metrics[0].json_pointer,
            "/result/dps"
        );
    }

    #[test]
    fn markdown_fenced_report_is_unwrapped_before_strict_validation() {
        let raw = format!(
            "下面是报告：\n```json\n{}\n```\n",
            serde_json::to_string(&report()).unwrap()
        );
        let parsed = parse_and_validate_report(&raw, &evidence()).unwrap();

        assert_eq!(
            parsed.content.schema_version,
            AGENT_REPORT_CONTENT_SCHEMA_V1
        );
        assert_eq!(parsed.content.findings.len(), 1);
    }

    #[test]
    fn double_encoded_report_is_unwrapped_before_strict_validation() {
        let encoded = serde_json::to_string(&serde_json::to_string(&report()).unwrap()).unwrap();
        let parsed = parse_and_validate_report(&encoded, &evidence()).unwrap();

        assert_eq!(
            parsed.content.schema_version,
            AGENT_REPORT_CONTENT_SCHEMA_V1
        );
        assert_eq!(parsed.content.findings.len(), 1);
    }

    #[test]
    fn legacy_rotation_card_does_not_discard_an_otherwise_valid_report() {
        let mut value = serde_json::to_value(report()).unwrap();
        value["rotation_changes"] = json!([{
            "index": 10,
            "original_skill": "血怒",
            "new_skill": "斩刀",
            "description": "旧版前端卡片"
        }]);
        let parsed = parse_and_validate_report(&value.to_string(), &evidence()).unwrap();
        assert!(parsed.content.rotation_changes.is_empty());
        assert_eq!(parsed.content.summary, report().summary);
    }

    #[test]
    fn provider_limitation_cards_are_losslessly_normalized() {
        let mut value = serde_json::to_value(report()).unwrap();
        value.as_object_mut().unwrap().remove("refusal_reason");
        value["limitations"] = json!([
            {
                "title": "怒气触顶不等于损失",
                "explanation": "当前证据只记录触顶采样。"
            },
            {"title": "仍需对照实验"}
        ]);

        let parsed = parse_and_validate_report(&value.to_string(), &evidence()).unwrap();

        assert_eq!(parsed.content.refusal_reason, None);
        assert_eq!(
            parsed.content.limitations,
            vec![
                "怒气触顶不等于损失：当前证据只记录触顶采样。".to_string(),
                "仍需对照实验".to_string()
            ]
        );
    }

    #[test]
    fn invalid_report_error_explains_the_required_shape() {
        let error = parse_and_validate_report(r#"{"limitations":[42]}"#, &evidence()).unwrap_err();

        assert_eq!(error.code, "invalid_report_json");
        assert!(error.message.contains("limitations must be strings"));
        assert!(error.message.contains("refusal_reason"));
    }

    #[test]
    fn unrelated_wrapped_json_is_not_accepted_as_a_report() {
        let raw = r#"analysis {\"message\":\"not a report\"} done"#;
        assert_eq!(
            parse_and_validate_report(raw, &evidence())
                .unwrap_err()
                .code,
            "invalid_report_json"
        );
    }

    #[test]
    fn grounded_numeric_prose_and_ordinary_counts_are_allowed() {
        let mut value = report();
        value.summary = "当前 DPS 为 123.5，循环包含 2 个技能事件。".to_string();
        value.findings[0].explanation = "确定性模拟得到 DPS 约 124。".to_string();
        let mut counted_evidence = evidence();
        counted_evidence.get_mut(&"a".repeat(64)).unwrap()["result"]["skill_count"] = json!(2);
        validate_report(&value, &counted_evidence).unwrap();

        let mut percent = report();
        percent.findings[0].metrics.push(GroundedMetricV1 {
            label: "伤害占比".to_string(),
            value: 0.3,
            unit: "ratio".to_string(),
            evidence_id: "a".repeat(64),
            json_pointer: "/result/ratio".to_string(),
        });
        percent.summary = "核心技能伤害占比为 30%。".to_string();
        percent.findings[0].title = "前段输出".to_string();
        validate_report(&percent, &evidence()).unwrap();

        let mut formatted = report();
        formatted.findings[0].metrics[0].value = 80_596.56;
        formatted.summary = "当前 DPS 为 80,596.56。".to_string();
        let mut formatted_evidence = evidence();
        formatted_evidence.get_mut(&"a".repeat(64)).unwrap()["result"]["dps"] = json!(80_596.56);
        validate_report(&formatted, &formatted_evidence).unwrap();

        let mut chinese_units = report();
        chinese_units.findings[0].explanation =
            "武学助手约 303.73 万，差距约 6.83 万，援戈伤害约 647 万。".to_string();
        let mut chinese_units_evidence = evidence();
        let result = &mut chinese_units_evidence.get_mut(&"a".repeat(64)).unwrap()["result"];
        result["candidate_dps"] = json!(3_037_347.33);
        result["delta_dps"] = json!(68_343.02);
        result["yuange_damage"] = json!(6_472_188.0);
        validate_report(&chinese_units, &chinese_units_evidence).unwrap();

        let mut named_skill = report();
        named_skill.findings[0].metrics[0].label = "绝刀·50怒总伤害".to_string();
        named_skill.summary = "绝刀·50怒是当前主要输出技能。".to_string();
        named_skill.findings[0].title = "绝刀·50怒贡献突出".to_string();
        validate_report(&named_skill, &evidence()).unwrap();

        let mut named_account = report();
        named_account.summary = "资料中的视频作者名为 dereck365。".to_string();
        named_account.findings[0].title = "账号 dereck365".to_string();
        validate_report(&named_account, &evidence()).unwrap();
    }

    #[test]
    fn cited_knowledge_numbers_are_allowed_without_becoming_simulation_metrics() {
        let mut value = report();
        value.summary = "当前赛季资料建议在 80ms 环境中自行实测宏阈值。".to_string();
        value.findings[0].title = "2026 赛季宏阈值说明".to_string();
        value.findings[0].explanation =
            "原始资料给出的示例范围是 0.15 到 0.20；这里只复述攻略，不表示模拟收益。".to_string();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics.clear();
        value.limitations = vec!["这是 2026 赛季知识资料，未运行战斗模拟。".to_string()];

        validate_report(&value, &knowledge_evidence()).unwrap();
    }

    #[test]
    fn current_scenario_input_numbers_are_allowed_in_prose_but_not_as_metrics() {
        let mut value = report();
        value.summary = "当前宏判定已由场景证据确认。".to_string();
        value.findings[0].title = "当前宏原句".to_string();
        value.findings[0].explanation = "当前条件为 rage>64 且 bufftime:嗜血<6。".to_string();
        value.findings[0].metrics.clear();
        let mut scenario_evidence = evidence();
        let envelope = scenario_evidence.get_mut(&"a".repeat(64)).unwrap();
        envelope["tool_name"] = json!("get_current_scenario");
        envelope["result"] = json!({
            "rotation_input": {
                "macro_statements": [{
                    "statement": "/cast [rage>64&bufftime:嗜血<6] 盾飞"
                }]
            }
        });

        validate_report(&value, &scenario_evidence).unwrap();
    }

    #[test]
    fn prose_accepts_a_percentage_derived_from_two_cited_tool_values() {
        let mut value = report();
        value.summary = "实测溢出45点，占总获得1750点约2.6%。".to_string();
        value.findings[0].explanation = value.summary.clone();
        value.findings[0].metrics.clear();
        let mut store = evidence();
        store.get_mut(&"a".repeat(64)).unwrap()["result"] = json!({
            "overflow_total": 45,
            "total_rage_gained": 1750
        });
        validate_report(&value, &store).unwrap();
    }

    #[test]
    fn compatible_metric_pointer_is_recovered_by_value_field_and_skill_context() {
        let id = "d".repeat(64);
        let store = BTreeMap::from([(
            id.clone(),
            json!({
                "evidence_id": id,
                "tool_name": "analyze_timeline",
                "result": {
                    "rage": {
                        "overflow_sources": [
                            {"rage_source": "麟光甲三层结算回怒", "event_count": 2, "overflow_total": 30},
                            {"rage_source": "血怒基础回怒", "event_count": 2, "overflow_total": 10}
                        ]
                    }
                }
            }),
        )]);
        let mut metric = GroundedMetricV1 {
            label: "绝刀溢出总量".to_string(),
            value: 30.0,
            unit: "点".to_string(),
            evidence_id: "d".repeat(64),
            json_pointer: "/result/rage/overflow_sources".to_string(),
        };
        let pointer = infer_metric_source_pointer(&metric, &store).unwrap();
        assert_eq!(pointer, "/result/rage/overflow_sources/0/overflow_total");
        metric.json_pointer = pointer;
        assert!(metric_matches_evidence(&metric, &store));
    }

    #[test]
    fn an_existing_event_pointer_cannot_move_to_another_event_to_match_a_number() {
        let id = "a".repeat(64);
        let store = EvidenceStore::from([(
            id.clone(),
            json!({
                "evidence_id": id,
                "tool_name": "inspect_timeline_events",
                "result": {"events": [
                    {"name": "技能甲", "damage": 100.0},
                    {"name": "技能甲", "damage": 200.0}
                ]}
            }),
        )]);
        let mut value = report();
        value.findings[0].title = "第一次释放".to_string();
        value.findings[0].metrics[0] = GroundedMetricV1 {
            label: "第一次释放伤害".to_string(),
            value: 200.0,
            unit: "damage".to_string(),
            evidence_id: id,
            json_pointer: "/result/events/0/damage".to_string(),
        };

        normalize_metric_citations(&mut value, &store);
        assert_eq!(value.findings[0].metrics[0].json_pointer, "/result/events/0/damage");
        let failure = parse_and_validate_report(
            &serde_json::to_string(&value).unwrap(),
            &store,
        ).unwrap_err();
        assert_eq!(failure.code, "metric_value_mismatch");
        assert!(failure.message.contains("/result/events/0/damage"));
        assert!(failure.message.contains("100"));
    }

    #[test]
    fn a_uniquely_matching_evidence_id_prefix_and_suffix_repairs_middle_corruption() {
        let mut value = report();
        value.findings[0].evidence_ids[0] =
            format!("{}{}{}", "a".repeat(12), "b".repeat(40), "a".repeat(12));
        let changed = normalize_evidence_id_references(&mut value, &evidence());
        assert_eq!(changed, 1);
        assert_eq!(value.findings[0].evidence_ids[0], "a".repeat(64));
    }

    #[test]
    fn knowledge_numbers_must_appear_in_the_exact_cited_evidence() {
        let mut value = report();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics.clear();
        value.summary = "建议把阈值设为 0.25。".to_string();
        assert_eq!(
            validate_report(&value, &knowledge_evidence())
                .unwrap_err()
                .code,
            "numeric_prose_claim"
        );

        value.summary = "资料记录的伤害占比为 30。".to_string();
        value.findings[0].evidence_ids = vec!["c".repeat(64)];
        assert_eq!(
            validate_report(&value, &knowledge_evidence())
                .unwrap_err()
                .code,
            "numeric_prose_claim"
        );

        value.summary = "资料示例阈值为 0.15。".to_string();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        let mut ineligible = knowledge_evidence();
        ineligible.get_mut(&"b".repeat(64)).unwrap()["result"]["results"][0]["fact_eligible"] =
            json!(false);
        assert_eq!(
            validate_report(&value, &ineligible).unwrap_err().code,
            "numeric_prose_claim"
        );
    }

    #[test]
    fn ungrounded_numeric_prose_is_still_rejected() {
        let mut value = report();
        value.summary = "DPS 为 999。".to_string();
        assert_eq!(
            validate_report(&value, &evidence()).unwrap_err().code,
            "numeric_prose_claim"
        );

        value.summary = "预计额外提升50怒。".to_string();
        assert_eq!(
            validate_report(&value, &evidence()).unwrap_err().code,
            "numeric_prose_claim"
        );

        value.summary = "DPS 约为 8e4。".to_string();
        assert_eq!(
            validate_report(&value, &evidence()).unwrap_err().code,
            "numeric_prose_claim"
        );

        value.findings[0].metrics[0].label = "绝刀·50怒总伤害".to_string();
        value.summary = "预计可以提升50%。".to_string();
        assert_eq!(
            validate_report(&value, &evidence()).unwrap_err().code,
            "numeric_prose_claim"
        );
    }

    #[test]
    fn salvage_keeps_grounded_metrics_and_marks_unsupported_sentences() {
        let mut value = report();
        value.summary = "当前 DPS 为 123.5，未经验证的预测为 999。".to_string();
        value.findings[0].explanation =
            "确定性模拟得到 123.5，另一个错误字段写成 999。".to_string();
        value.findings[0].metrics.push(GroundedMetricV1 {
            label: "错误指标".to_string(),
            value: 999.0,
            unit: "damage".to_string(),
            evidence_id: "a".repeat(64),
            json_pointer: "/result/not_present".to_string(),
        });
        let raw = serde_json::to_string(&value).unwrap();
        assert_eq!(
            parse_and_validate_report(&raw, &evidence())
                .unwrap_err()
                .code,
            "missing_metric_source"
        );

        let salvaged = parse_and_salvage_report(&raw, &evidence()).unwrap();
        assert!(salvaged.sanitized_claims >= 2);
        assert_eq!(salvaged.content.findings[0].metrics.len(), 1);
        assert_eq!(
            salvaged.content.summary,
            "这项判断的数值依据尚待核实。"
        );
        assert!(!salvaged.content.summary.contains("999"));
        assert_eq!(
            salvaged.content.findings[0].explanation,
            "这项判断的数值依据尚待核实。"
        );
        validate_report(&salvaged.content, &evidence()).unwrap();
    }

    #[test]
    fn nonbaseline_failure_does_not_insert_a_baseline_answer() {
        let mut baseline_evidence = evidence();
        baseline_evidence.get_mut(&"a".repeat(64)).unwrap()["tool_name"] =
            json!("simulate_scenario");
        let mut value = report();
        value.summary = "哪些释放时机值得重新安排？".to_string();
        value.findings[0].title = "释放时机".to_string();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics[0].evidence_id = "b".repeat(64);

        let failure =
            parse_and_salvage_report(&serde_json::to_string(&value).unwrap(), &baseline_evidence)
                .unwrap_err();
        assert_eq!(failure.code, "empty_salvaged_report");
        assert!(failure.message.contains("original question"));
    }

    #[test]
    fn event_question_salvage_preserves_its_subject_and_supported_explanation() {
        let mut value = report();
        value.summary = "优先检查增益结束后的释放位置。".to_string();
        value.findings[0].title = "增益结束后的释放位置".to_string();
        value.findings[0].explanation =
            "这段释放发生在增益结束后，应比较它的资源与增益状态。它比其他事件低999伤害。".to_string();
        value.findings[0].metrics[0].value = 999.0;
        value.findings[0].metrics[0].json_pointer = "/result/not_present".to_string();

        let salvaged = parse_and_salvage_report(
            &serde_json::to_string(&value).unwrap(),
            &evidence(),
        ).unwrap();

        assert_eq!(salvaged.content.summary, value.summary);
        assert_eq!(salvaged.content.findings.len(), 1);
        assert_eq!(salvaged.content.findings[0].title, value.findings[0].title);
        assert!(salvaged.content.findings[0].metrics.is_empty());
        assert_eq!(
            salvaged.content.findings[0].explanation,
            "这段释放发生在增益结束后，应比较它的资源与增益状态。这项判断的数值依据尚待核实。"
        );
        assert!(!serde_json::to_string(&salvaged.content).unwrap().contains("基线"));
    }
}
