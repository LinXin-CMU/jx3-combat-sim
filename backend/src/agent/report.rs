use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub const AGENT_REPORT_CONTENT_SCHEMA_V1: &str = "agent-report-content/v1";
pub const AGENT_REPORT_SCHEMA_V1: &str = "agent-report/v1";
const MAX_FINDINGS: usize = 12;
const MAX_RECOMMENDATIONS: usize = 8;
const MAX_METRICS_PER_FINDING: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentReportContentV1 {
    pub schema_version: String,
    pub summary: String,
    pub findings: Vec<AgentFindingV1>,
    pub recommendations: Vec<AgentRecommendationV1>,
    pub limitations: Vec<String>,
    pub refusal_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentFindingV1 {
    pub title: String,
    pub explanation: String,
    pub evidence_ids: Vec<String>,
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
    pub content: AgentReportContentV1,
    pub evidence_ids: Vec<String>,
    pub accounting: AgentRunAccountingV1,
    pub termination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentRunAccountingV1 {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub simulations: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportValidationError {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedReportContentV1 {
    pub content: AgentReportContentV1,
    pub normalized_metric_citations: usize,
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
            "limitations": {"type": "array", "maxItems": MAX_RECOMMENDATIONS, "items": {"type": "string"}},
            "refusal_reason": {"type": ["string", "null"]}
        },
        "required": ["schema_version", "summary", "findings", "recommendations", "limitations", "refusal_reason"],
        "additionalProperties": false
    })
}

pub fn parse_and_validate_report(
    raw: &str,
    evidence: &EvidenceStore,
) -> Result<ValidatedReportContentV1, ReportValidationError> {
    let mut report: AgentReportContentV1 = serde_json::from_str(raw).map_err(|_| {
        error(
            "invalid_report_json",
            "final answer must be valid AgentReportContentV1 JSON",
        )
    })?;
    let normalized_metric_citations = normalize_metric_citations(&mut report, evidence);
    validate_report(&report, evidence)?;
    Ok(ValidatedReportContentV1 {
        content: report,
        normalized_metric_citations,
    })
}

fn normalize_metric_citations(
    report: &mut AgentReportContentV1,
    evidence: &EvidenceStore,
) -> usize {
    let mut normalized = 0;
    for finding in &mut report.findings {
        for metric in &finding.metrics {
            if evidence.contains_key(&metric.evidence_id)
                && !finding.evidence_ids.contains(&metric.evidence_id)
            {
                finding.evidence_ids.push(metric.evidence_id.clone());
                normalized += 1;
            }
        }
    }
    normalized
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
    validate_prose(&report.summary)?;
    if report.findings.len() > MAX_FINDINGS
        || report.recommendations.len() > MAX_RECOMMENDATIONS
        || report.limitations.len() > MAX_RECOMMENDATIONS
    {
        return Err(error(
            "report_too_large",
            "report collection limit exceeded",
        ));
    }
    if report.findings.is_empty() && report.refusal_reason.is_none() {
        return Err(error(
            "empty_report",
            "report requires a verified finding or a refusal reason",
        ));
    }
    if let Some(reason) = &report.refusal_reason {
        validate_prose(reason)?;
    }
    for limitation in &report.limitations {
        validate_prose(limitation)?;
    }

    for finding in &report.findings {
        validate_prose(&finding.title)?;
        validate_prose(&finding.explanation)?;
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

    for recommendation in &report.recommendations {
        validate_prose(&recommendation.title)?;
        validate_prose(&recommendation.rationale)?;
        if recommendation.evidence_ids.is_empty() {
            return Err(error(
                "recommendation_without_evidence",
                "every recommendation must cite evidence",
            ));
        }
        validate_evidence_ids(&recommendation.evidence_ids, evidence)?;
    }
    Ok(())
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
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
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
    validate_prose(&metric.label)?;
    validate_prose(&metric.unit)?;
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
        })
        .and_then(|envelope| envelope.pointer(&metric.json_pointer))
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            error(
                "missing_metric_source",
                "metric source is not a numeric evidence value",
            )
        })?;
    let tolerance = 1e-9_f64.max(source.abs() * 1e-9);
    if (source - metric.value).abs() > tolerance {
        return Err(error(
            "metric_value_mismatch",
            "metric value does not match cited evidence",
        ));
    }
    Ok(())
}

fn validate_prose(value: &str) -> Result<(), ReportValidationError> {
    validate_short_text(value)?;
    if value.chars().any(|character| character.is_numeric()) {
        return Err(error(
            "numeric_prose_claim",
            "numeric literals are only allowed in grounded metrics",
        ));
    }
    Ok(())
}

fn validate_short_text(value: &str) -> Result<(), ReportValidationError> {
    let count = value.chars().count();
    if !(1..=1024).contains(&count) || value.chars().any(char::is_control) {
        Err(error("invalid_report_text", "report text is invalid"))
    } else {
        Ok(())
    }
}

fn error(code: &'static str, message: &'static str) -> ReportValidationError {
    ReportValidationError { code, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evidence() -> EvidenceStore {
        BTreeMap::from([(
            "a".repeat(64),
            json!({"evidence_id": "a".repeat(64), "result": {"dps": 123.5}}),
        )])
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
            limitations: vec!["只验证了当前场景。".to_string()],
            refusal_reason: None,
        }
    }

    #[test]
    fn grounded_metric_passes() {
        validate_report(&report(), &evidence()).unwrap();
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
    fn arabic_numeric_prose_is_rejected_but_natural_language_counts_are_allowed() {
        let mut value = report();
        value.summary = "DPS 为 123.5。".to_string();
        assert_eq!(
            validate_report(&value, &evidence()).unwrap_err().code,
            "numeric_prose_claim"
        );

        let mut natural_count = report();
        natural_count.summary = "循环包含两个技能事件。".to_string();
        validate_report(&natural_count, &evidence()).unwrap();
    }
}
