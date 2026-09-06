//! Agent skill adapter: reuse the existing sequence-to-macro generator unchanged.
use super::tools::{SimulationExecution, ToolError};
use serde::Deserialize;
use serde_json::{json, Value};

pub const DISTILL_MACRO: &str = "distill_macro";
pub const INSTRUCTIONS: &str = include_str!("../../agent_skills/macro_distillation/SKILL.md");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arguments {
    #[serde(default)]
    pub macro_text: Option<String>,
    #[serde(default)]
    pub max_evaluations: Option<usize>,
}

pub fn arguments_schema() -> Value {
    json!({"type":"object","properties":{
        "macro_text":{"type":["string","null"],"maxLength":8192,"description":"Omit to generate from the frozen rotation; provide a complete candidate to algorithmically tune an Agent revision."},
        "max_evaluations":{"type":["integer","null"],"minimum":1,"maximum":512,"description":"Algorithm simulation allowance; default 384. Repeats Workflow A from the best tested candidate, charging only actual simulations."}
    },"additionalProperties":false})
}

pub fn generate(execution: &SimulationExecution) -> Result<Value, ToolError> {
    // Match the existing frontend /api/macro/from_sequence input exactly.
    let casts: Vec<crate::macro_gen::InputCast> = execution
        .response
        .timeline
        .iter()
        .map(|event| {
            serde_json::from_value(json!({
                "name": event.name, "triggered": event.triggered,
                "is_main": event.is_main, "state_before": event.state_before,
            }))
        })
        .collect::<Result<_, _>>()
        .map_err(|_| ToolError::TimelineDetailsUnavailable)?;
    if !casts
        .iter()
        .any(|event| !event.triggered && event.state_before.is_some())
    {
        return Err(ToolError::TimelineDetailsUnavailable);
    }
    // Deserialize {} to use serde defaults, not GenOptions::default()'s zero fields.
    let options = serde_json::from_value::<crate::macro_gen::GenOptions>(json!({}))
        .expect("generator defaults are valid");
    let generated = crate::macro_gen::generate(&casts, &options);
    Ok(json!({
        "skill_id": "macro-distillation/v2", "generator": "macro_gen::generate",
        "baseline": execution.evidence.result,
        "macro_text": generated.macro_text,
        "pages": page_lengths(&generated.macro_text),
        "stats": generated.stats, "rule_diagnostics": generated.rule_diagnostics,
        "dropped_rules": generated.dropped_rules,
        "candidate_status": "generated_uncompared",
    }))
}

/// Same counting convention as calcPagesFromText in the existing editor.
pub fn page_lengths(text: &str) -> Value {
    let mut pages = Vec::new();
    let mut stance = "any".to_string();
    let mut body = String::new();
    let flush = |pages: &mut Vec<Value>, stance: &str, body: &str| {
        let chars = body.trim_end().encode_utf16().count();
        if chars > 0 {
            pages
                .push(json!({"stance":stance,"chars":chars,"limit":128,"within_limit":chars<=128}));
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("#page") {
            flush(&mut pages, &stance, &body);
            body.clear();
            stance = if rest.trim().is_empty() {
                "any"
            } else {
                rest.trim()
            }
            .into();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut pages, &stance, &body);
    Value::Array(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_each_body_with_newlines_and_without_page_markers() {
        let pages = page_lengths("#page shield\n/cast 盾击\n/cast 盾飞\n#page blade\n/cast 斩刀\n");
        assert_eq!(pages[0]["chars"], 17);
        assert_eq!(pages[1]["chars"], 8);
        assert_eq!(pages[1]["within_limit"], true);
        assert_eq!(page_lengths(&"x".repeat(129))[0]["within_limit"], false);
    }
}
