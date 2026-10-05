//! Free prose uses the existing evidence checks without a fixed article layout.
use super::*;

pub(super) fn adopt_surrounding_text(prefix: &str, suffix: &str, report: &mut AgentReportContentV1) {
    let prefix = prefix.trim().trim_end_matches("```json").trim_end_matches("```JSON").trim_end_matches("```").trim();
    let suffix = suffix.trim().trim_start_matches("```").trim();
    let surrounding = [prefix, suffix].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n\n");
    // A short wrapper remains a wrapper; a substantive article must not disappear
    // when the provider places the evidence object after its visible answer.
    if surrounding.chars().count() >= 128 && surrounding.chars().count() > report.body_markdown.chars().count() {
        report.body_markdown = surrounding;
    }
}

pub(super) fn normalize(body: &mut String) -> usize {
    let mut unwrapped = 0;
    // A report accidentally serialized into its own article field is a
    // transport wrapper, not prose to display or an extra evidence source.
    for _ in 0..2 {
        let Some(inner) = serde_json::from_str::<Value>(body.trim()).ok()
            .filter(|value| value["schema_version"].as_str().is_some_and(|schema| schema.starts_with("agent-report")))
            .and_then(|value| value["body_markdown"].as_str().map(str::to_string)) else { break; };
        *body = inner;
        unwrapped += 1;
    }
    let normalized = body.replace("\r\n", "\n").chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(16_000).collect::<String>().trim().to_string();
    let changed = usize::from(normalized != *body);
    *body = normalized;
    changed + unwrapped
}

pub(super) fn validate(body: &str, metrics: &[&GroundedMetricV1], ids: &[String], evidence: &EvidenceStore) -> Result<(), ReportValidationError> {
    if body.is_empty() { return Ok(()); }
    if body.chars().count() > 16_000 || body.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t')) {
        return Err(error("invalid_report_body", "body text exceeds the size or character limit"));
    }
    validate_numeric_prose(body, metrics.iter().copied(), ids, evidence)
}

pub(super) fn salvage(body: &mut String, metrics: &[GroundedMetricV1], ids: &[String], evidence: &EvidenceStore) -> usize {
    let mut changed = normalize(body);
    if validate_numeric_prose(body, metrics, ids, evidence).is_ok() { return changed; }
    let mut retained = String::new();
    for sentence in body.split_inclusive(['\n', '。', '！', '？', '；']) {
        if validate_numeric_prose(sentence, metrics, ids, evidence).is_ok() {
            retained.push_str(sentence);
        } else {
            changed += 1;
            // Unsupported numbers belong in the report's validation status,
            // not as repeated filler interrupting the user's article.
            if sentence.ends_with('\n') { retained.push('\n'); }
        }
    }
    *body = retained;
    changed
}

#[cfg(test)]
#[path = "../../../tests/agent/free_prose.rs"]
mod tests;
