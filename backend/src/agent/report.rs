use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<AgentKnowledgeSourceV1>,
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
    pub message: &'static str,
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
    let mut report = parse_report_json(raw).map_err(|_| {
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
        sanitized_claims: 0,
    })
}

/// Preserve valid claims from a structurally parseable model report instead of
/// discarding the whole answer because one metric, citation, or prose number is
/// wrong. The returned content passes the same strict validator as a normal
/// report; unsupported pieces are removed or visibly redacted first.
pub fn parse_and_salvage_report(
    raw: &str,
    evidence: &EvidenceStore,
) -> Result<ValidatedReportContentV1, ReportValidationError> {
    let mut report = parse_report_json(raw).map_err(|_| {
        error(
            "invalid_report_json",
            "final answer must be valid AgentReportContentV1 JSON",
        )
    })?;
    let mut sanitized_claims = 0;
    if report.schema_version != AGENT_REPORT_CONTENT_SCHEMA_V1 {
        report.schema_version = AGENT_REPORT_CONTENT_SCHEMA_V1.to_string();
        sanitized_claims += 1;
    }

    sanitized_claims += truncate_vec(&mut report.findings, MAX_FINDINGS);
    sanitized_claims += truncate_vec(&mut report.recommendations, MAX_RECOMMENDATIONS);
    sanitized_claims += truncate_vec(&mut report.limitations, MAX_RECOMMENDATIONS);
    let normalized_metric_citations = normalize_metric_citations(&mut report, evidence);

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
        sanitized_claims += sanitize_text_field(&mut finding.title, "已验证结论");
        sanitized_claims +=
            sanitize_unsupported_numeric_prose(
                &mut finding.title,
                &finding.metrics,
                &finding.evidence_ids,
                evidence,
                "已验证结论",
            );
        sanitized_claims += sanitize_text_field(
            &mut finding.explanation,
            "该结论仅保留通过本次证据校验的部分。",
        );
        sanitized_claims += sanitize_unsupported_numeric_prose(
            &mut finding.explanation,
            &finding.metrics,
            &finding.evidence_ids,
            evidence,
            "以下指标已通过本次模拟证据校验。",
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
        "当前基线的可信指标与主要结论见下方。",
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
            "建议通过新的确定性实验继续验证。",
        );
        retained_recommendations.push(recommendation);
    }
    report.recommendations = retained_recommendations;

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
        let Some(fallback) = direct_baseline_finding(evidence) else {
            return Err(error(
                "empty_salvaged_report",
                "no verifiable claim remains after report sanitization",
            ));
        };
        report.findings.push(fallback);
        report.summary = "模型报告仅部分通过校验；以下保留模拟器直接验证的基线指标。".to_string();
        sanitized_claims += 1;
    }

    const PARTIAL_LIMITATION: &str =
        "部分模型表述或指标未通过逐项证据校验，已自动隐藏；保留内容均可追溯到本次运行证据。";
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
    if let Ok(report) = serde_json::from_str(trimmed) {
        return Ok(report);
    }

    // Try object boundaries rather than stripping arbitrary prose. Deserializing
    // directly into the deny_unknown_fields report type prevents a nested or
    // unrelated JSON object from being accepted accidentally.
    for (index, _) in trimmed.match_indices('{').take(64) {
        let mut deserializer = serde_json::Deserializer::from_str(&trimmed[index..]);
        if let Ok(report) = AgentReportContentV1::deserialize(&mut deserializer) {
            return Ok(report);
        }
    }

    // Some compatibility gateways JSON-encode the assistant content once more.
    if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
        let decoded = decoded.trim();
        if let Ok(report) = serde_json::from_str(decoded) {
            return Ok(report);
        }
        for (index, _) in decoded.match_indices('{').take(64) {
            let mut deserializer = serde_json::Deserializer::from_str(&decoded[index..]);
            if let Ok(report) = AgentReportContentV1::deserialize(&mut deserializer) {
                return Ok(report);
            }
        }
    }

    serde_json::from_str::<AgentReportContentV1>(trimmed)
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
    let tolerance = 1e-9_f64.max(source.abs() * 1e-9);
    (source - metric.value).abs() <= tolerance
}

fn direct_baseline_finding(evidence: &EvidenceStore) -> Option<AgentFindingV1> {
    const PATHS: [(&str, &str, &str); 3] = [
        ("/result/dps", "DPS", "damage_per_second"),
        ("/result/total_damage", "总伤害", "damage"),
        ("/result/duration", "战斗时长", "second"),
    ];
    for (evidence_id, envelope) in evidence {
        if envelope.get("tool_name").and_then(Value::as_str) != Some("simulate_scenario") {
            continue;
        }
        let metrics = PATHS
            .iter()
            .filter_map(|(pointer, label, unit)| {
                envelope
                    .pointer(pointer)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                    .map(|value| GroundedMetricV1 {
                        label: (*label).to_string(),
                        value,
                        unit: (*unit).to_string(),
                        evidence_id: evidence_id.clone(),
                        json_pointer: (*pointer).to_string(),
                    })
            })
            .collect::<Vec<_>>();
        if !metrics.is_empty() {
            return Some(AgentFindingV1 {
                title: "模拟器直接验证的基线".to_string(),
                explanation: "原始模型表述未全部通过逐项校验，具体结论已缩减为可复现指标。"
                    .to_string(),
                evidence_ids: vec![evidence_id.clone()],
                metrics,
            });
        }
    }
    None
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
    validate_short_text(&report.summary)?;
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

fn metric_tool_allowed(envelope: &Value) -> bool {
    matches!(
        envelope.get("tool_name").and_then(Value::as_str),
        Some("simulate_scenario" | "compare_scenarios" | "analyze_timeline")
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NumericLiteral {
    value: f64,
    decimal_places: u32,
    percent: bool,
    ordinary_count: bool,
    start: usize,
    end: usize,
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
    if numeric_literals(value).iter().any(|literal| {
        !literal.ordinary_count
            && !matches_metric(*literal, &metric_values)
            && !matches_metric_label_literal(value, *literal, &metrics)
            && !matches_knowledge_literal(*literal, &knowledge_literals)
    }) {
        return Err(error(
            "numeric_prose_claim",
            "numeric prose must restate a grounded metric value",
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
                && !value[start..number_end].contains(',')
                && (0.0..=12.0).contains(&parsed);
            literals.push(NumericLiteral {
                value: parsed,
                decimal_places,
                percent,
                ordinary_count,
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
    let unsupported = numeric_literals(value)
        .into_iter()
        .filter(|literal| {
            !literal.ordinary_count
                && !matches_metric(*literal, &metric_values)
                && !matches_metric_label_literal(value, *literal, &metric_refs)
                && !matches_knowledge_literal(*literal, &knowledge_literals)
        })
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return 0;
    }
    *value = fallback.to_string();
    unsupported.len()
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
        let tolerance = literal.value.abs().max(source.value.abs()).max(1.0) * 1e-9;
        (literal.value - source.value).abs() <= tolerance
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
                    || (literal.value - label_literal.value).abs() > f64::EPSILON
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
            let display_tolerance = if literal.decimal_places == 0 {
                0.5
            } else {
                0.5 * 10_f64.powi(-(literal.decimal_places as i32))
            };
            let floating_tolerance = candidate.abs().max(1.0) * 1e-9;
            (literal.value - candidate).abs() <= display_tolerance.max(floating_tolerance)
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
                            "fact_eligible": true
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
            limitations: vec!["只验证了当前场景。".to_string()],
            refusal_reason: None,
        }
    }

    #[test]
    fn grounded_metric_passes() {
        validate_report(&report(), &evidence()).unwrap();
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
    fn markdown_fenced_report_is_unwrapped_before_strict_validation() {
        let raw = format!(
            "下面是报告：\n```json\n{}\n```\n",
            serde_json::to_string(&report()).unwrap()
        );
        let parsed = parse_and_validate_report(&raw, &evidence()).unwrap();

        assert_eq!(parsed.content.schema_version, AGENT_REPORT_CONTENT_SCHEMA_V1);
        assert_eq!(parsed.content.findings.len(), 1);
    }

    #[test]
    fn double_encoded_report_is_unwrapped_before_strict_validation() {
        let encoded = serde_json::to_string(&serde_json::to_string(&report()).unwrap()).unwrap();
        let parsed = parse_and_validate_report(&encoded, &evidence()).unwrap();

        assert_eq!(parsed.content.schema_version, AGENT_REPORT_CONTENT_SCHEMA_V1);
        assert_eq!(parsed.content.findings.len(), 1);
    }

    #[test]
    fn unrelated_wrapped_json_is_not_accepted_as_a_report() {
        let raw = r#"analysis {\"message\":\"not a report\"} done"#;
        assert_eq!(
            parse_and_validate_report(raw, &evidence()).unwrap_err().code,
            "invalid_report_json"
        );
    }

    #[test]
    fn grounded_numeric_prose_and_ordinary_counts_are_allowed() {
        let mut value = report();
        value.summary = "当前 DPS 为 123.5，循环包含 2 个技能事件。".to_string();
        value.findings[0].explanation = "确定性模拟得到 DPS 约 124。".to_string();
        validate_report(&value, &evidence()).unwrap();

        let mut percent = report();
        percent.findings[0].metrics.push(GroundedMetricV1 {
            label: "伤害占比".to_string(),
            value: 0.3,
            unit: "ratio".to_string(),
            evidence_id: "a".repeat(64),
            json_pointer: "/result/ratio".to_string(),
        });
        percent.summary = "核心技能伤害占比为 30%。".to_string();
        percent.findings[0].title = "前 5 秒输出".to_string();
        validate_report(&percent, &evidence()).unwrap();

        let mut formatted = report();
        formatted.findings[0].metrics[0].value = 80_596.56;
        formatted.summary = "当前 DPS 为 80,596.56。".to_string();
        let mut formatted_evidence = evidence();
        formatted_evidence.get_mut(&"a".repeat(64)).unwrap()["result"]["dps"] = json!(80_596.56);
        validate_report(&formatted, &formatted_evidence).unwrap();

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
            "原始资料给出的示例范围是 0.15 到 0.20；这里只复述攻略，不表示模拟收益。"
                .to_string();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics.clear();
        value.limitations = vec!["这是 2026 赛季知识资料，未运行战斗模拟。".to_string()];

        validate_report(&value, &knowledge_evidence()).unwrap();
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
        ineligible.get_mut(&"b".repeat(64)).unwrap()["result"]["results"][0]
            ["fact_eligible"] = json!(false);
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
    fn salvage_keeps_grounded_metrics_and_redacts_only_unsupported_numbers() {
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
            "当前基线的可信指标与主要结论见下方。"
        );
        assert!(!salvaged.content.summary.contains("999"));
        assert_eq!(
            salvaged.content.findings[0].explanation,
            "以下指标已通过本次模拟证据校验。"
        );
        validate_report(&salvaged.content, &evidence()).unwrap();
    }

    #[test]
    fn salvage_falls_back_to_direct_simulator_metrics_when_all_claims_are_bad() {
        let mut baseline_evidence = evidence();
        baseline_evidence.get_mut(&"a".repeat(64)).unwrap()["tool_name"] =
            json!("simulate_scenario");
        let mut value = report();
        value.findings[0].evidence_ids = vec!["b".repeat(64)];
        value.findings[0].metrics[0].evidence_id = "b".repeat(64);

        let salvaged =
            parse_and_salvage_report(&serde_json::to_string(&value).unwrap(), &baseline_evidence)
                .unwrap();
        assert_eq!(salvaged.content.findings.len(), 1);
        assert_eq!(
            salvaged.content.findings[0].metrics[0].json_pointer,
            "/result/dps"
        );
        assert_eq!(
            salvaged.content.findings[0].evidence_ids,
            vec!["a".repeat(64)]
        );
        validate_report(&salvaged.content, &baseline_evidence).unwrap();
    }
}
