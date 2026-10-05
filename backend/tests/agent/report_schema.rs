use super::*;
use serde_json::json;
use std::collections::BTreeSet;

fn qualitative_report() -> Value {
    json!({
        "schema_version": AGENT_REPORT_CONTENT_SCHEMA_V1,
        "summary": "按设计假设讨论连招的投入与退出。",
        "body_markdown": "假设资源规则不变，连续动作会增加时间承诺；可以设计退出机会供玩家权衡。",
        "findings": [{
            "title": "时间承诺", "explanation": "连续投入会影响玩家应对变化的机会。",
            "evidence_ids": [], "metrics": []
        }],
        "recommendations": [{
            "title": "保留退出机会", "rationale": "可以让玩家主动结束连招，选择保留应对空间。",
            "evidence_ids": []
        }],
        "rotation_changes": [], "artifacts": [], "limitations": [], "refusal_reason": null
    })
}

// A focused wire-contract check, not a second semantic evidence validator.
// Walk real report fixtures so minItems/required drift is caught at any depth.
fn check_wire_shape(schema: &Value, instance: &Value) -> Result<(), String> {
    let matches_type = |kind: &str| match kind {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "null" => instance.is_null(),
        _ => false,
    };
    let valid_type = match &schema["type"] {
        Value::String(kind) => matches_type(kind),
        Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).any(matches_type),
        _ => false,
    };
    if !valid_type {
        return Err("type mismatch".into());
    }
    if let Some(expected) = schema.get("const") {
        if instance != expected {
            return Err("const mismatch".into());
        }
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
        if !choices.contains(instance) {
            return Err("enum mismatch".into());
        }
    }
    if let Some(object) = instance.as_object() {
        let properties = schema["properties"]
            .as_object()
            .ok_or("missing properties")?;
        for name in schema["required"].as_array().ok_or("missing required")? {
            if !object.contains_key(name.as_str().ok_or("invalid required")?) {
                return Err(format!("required field missing: {name}"));
            }
        }
        for (name, value) in object {
            match properties.get(name) {
                Some(child) => check_wire_shape(child, value)?,
                None if schema["additionalProperties"] == false => {
                    return Err(format!("unknown field: {name}"))
                }
                None => {}
            }
        }
    }
    if let Some(items) = instance.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|min| items.len() < min as usize)
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|max| items.len() > max as usize)
        {
            return Err("array length mismatch".into());
        }
        for item in items {
            check_wire_shape(&schema["items"], item)?;
        }
    }
    Ok(())
}

fn assert_strict_objects(schema: &Value) {
    if schema["type"] == "object" {
        let properties = schema["properties"].as_object().unwrap();
        let required = schema["required"].as_array().unwrap();
        let required_names = required
            .iter()
            .map(|item| item.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let property_names = properties
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required_names, property_names,
            "strict output requires every declared property"
        );
        assert_eq!(required.len(), required_names.len());
        assert_eq!(schema["additionalProperties"], false);
        for child in properties.values() {
            assert_strict_objects(child);
        }
    }
    if let Some(items) = schema.get("items") {
        assert_strict_objects(items);
    }
}

#[test]
fn qualitative_empty_references_agree_between_wire_schema_and_runtime() {
    let value = qualitative_report();
    check_wire_shape(&report_content_json_schema(), &value).unwrap();
    let parsed = parse_and_validate_report(&value.to_string(), &EvidenceStore::new()).unwrap();
    assert!(parsed.content.findings[0].evidence_ids.is_empty());
    assert!(parsed.content.recommendations[0].evidence_ids.is_empty());
}

#[test]
fn strict_output_uses_explicit_empty_values_and_keeps_legacy_reports_readable() {
    let schema = report_content_json_schema();
    assert_strict_objects(&schema);
    let mut value = qualitative_report();
    value["body_markdown"] = json!("");
    value["findings"] = json!([]);
    value["refusal_reason"] = json!("当前信息不足，无法形成结论。");
    check_wire_shape(&schema, &value).unwrap();
    parse_and_validate_report(&value.to_string(), &EvidenceStore::new()).unwrap();
    value.as_object_mut().unwrap().remove("body_markdown");
    value.as_object_mut().unwrap().remove("artifacts");
    assert!(check_wire_shape(&schema, &value).is_err());
    let legacy = parse_and_validate_report(&value.to_string(), &EvidenceStore::new()).unwrap();
    assert!(legacy.content.body_markdown.is_empty());
    assert!(legacy.content.artifacts.is_empty());
}

#[test]
fn qualitative_wire_shape_does_not_relax_numeric_evidence_validation() {
    let mut value = qualitative_report();
    value["findings"][0]["metrics"] = json!([{
        "label": "DPS", "value": 123.5, "unit": "damage_per_second",
        "evidence_id": "a".repeat(64), "json_pointer": "/result/dps"
    }]);
    check_wire_shape(&report_content_json_schema(), &value).unwrap();
    assert_eq!(
        parse_and_validate_report(&value.to_string(), &EvidenceStore::new())
            .unwrap_err()
            .code,
        "finding_without_evidence"
    );
    value["findings"][0]["metrics"] = json!([]);
    value["recommendations"][0]["rationale"] = json!("修改后实测伤害提高37.42%。");
    assert_eq!(
        parse_and_validate_report(&value.to_string(), &EvidenceStore::new())
            .unwrap_err()
            .code,
        "numeric_prose_claim"
    );
}

#[test]
fn empty_references_still_require_prose_or_verified_rotation_evidence() {
    let mut value = qualitative_report();
    value["body_markdown"] = json!("");
    assert_eq!(
        parse_and_validate_report(&value.to_string(), &EvidenceStore::new())
            .unwrap_err()
            .code,
        "finding_without_evidence"
    );
    let schema = report_content_json_schema();
    let mut value = qualitative_report();
    value["rotation_changes"] = json!([{
        "change_type": "macro_statement", "edit_operation": "replace", "target": "起手",
        "current": "/cast 盾击", "proposed": "/cast 盾压", "rationale": "尝试不同技能。",
        "evidence_ids": []
    }]);
    assert!(check_wire_shape(&schema, &value).is_err());
    assert_eq!(
        parse_and_validate_report(&value.to_string(), &EvidenceStore::new())
            .unwrap_err()
            .code,
        "rotation_change_without_evidence"
    );
}
