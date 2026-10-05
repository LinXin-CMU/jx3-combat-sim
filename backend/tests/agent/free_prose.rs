use super::*;

#[test]
fn nested_serialized_report_renders_article_and_still_checks_its_claims() {
    let nested = serde_json::json!({"schema_version":"agent-report/v1", "body_markdown":"# 取舍\n\n新资源会改变循环安排。", "findings":[]}).to_string();
    let parsed = parse_and_validate_report(&prose_report(&nested), &EvidenceStore::new()).unwrap();
    assert_eq!(parsed.content.body_markdown, "# 取舍\n\n新资源会改变循环安排。");
    let nested = serde_json::json!({"schema_version":"agent-report/v1", "body_markdown":"实测提升37.42%。", "findings":[]}).to_string();
    assert_eq!(parse_and_validate_report(&prose_report(&nested), &EvidenceStore::new()).unwrap_err().code, "numeric_prose_claim");
}

#[test]
fn cited_definition_numbers_are_supported_without_becoming_simulated_gains() {
    let runtime = crate::agent::AgentRuntime::fixture_for(crate::GameVersion::CangShengZhuShiTest, crate::Mount::TieGuYi);
    let scenario = runtime.fixture_scenario();
    let envelope = crate::agent::skill_catalog::lookup("prose-definitions", crate::agent::skill_catalog::DefinitionQuery {
        query:"阵云".into(), ..Default::default()
    }, &scenario, &runtime).unwrap();
    let id = envelope.evidence_id.clone();
    let mut evidence = EvidenceStore::new();
    evidence.insert(id.clone(), serde_json::to_value(envelope).unwrap());
    let mut report: Value = serde_json::from_str(&prose_report("分山定义的暴怒上限为120点，消耗50点；达到100点时改为消耗100点，伤害额外提高75%。移植时需要决定如何保留铁骨的格挡用途。" )).unwrap();
    report["findings"] = serde_json::json!([{"title":"源定义", "explanation":"按分山奇穴定义分析移植取舍。", "evidence_ids":[id], "metrics":[]}]);
    parse_and_validate_report(&report.to_string(), &evidence).unwrap();
    report["body_markdown"] = Value::String("实测移植后DPS提高37.42%。".into());
    assert_eq!(parse_and_validate_report(&report.to_string(), &evidence).unwrap_err().code, "numeric_prose_claim");
}

fn prose_report(body: &str) -> String {
    serde_json::json!({"schema_version": AGENT_REPORT_CONTENT_SCHEMA_V1,
        "summary":"围绕用户提出的替换方案展开讨论。", "body_markdown":body,
        "findings":[],"recommendations":[],"rotation_changes":[],"limitations":[],"refusal_reason":null}).to_string()
}

#[test]
fn qualitative_answer_can_have_no_simulation_or_finding_cards() {
    let body = "先假设保留原有防守资源，只替换主动招式。\n\n这样可以讨论操作节奏的变化；额外连招会与原有技能竞争施放时间。";
    let parsed = parse_and_validate_report(&prose_report(body), &EvidenceStore::new()).unwrap();
    assert_eq!(parsed.content.body_markdown, body);
    assert!(parsed.content.findings.is_empty());
}

#[test]
fn long_article_preserves_paragraphs_table_and_model_chosen_headings() {
    let body = format!("# 一种可能的解释\n\n{}\n\n| 选择 | 代价 |\n| --- | --- |\n| 保留 | 占用时间 |", "改变技能会改变玩家需要作出的选择。\n\n".repeat(90));
    let parsed = parse_and_validate_report(&prose_report(&body), &EvidenceStore::new()).unwrap();
    assert_eq!(parsed.content.body_markdown, body);
    assert!(parsed.content.body_markdown.chars().count() > 1024);
}

#[test]
fn free_prose_cannot_bypass_numeric_evidence_validation() {
    let error = parse_and_validate_report(&prose_report("实测伤害提高37.42%。"), &EvidenceStore::new()).unwrap_err();
    assert_eq!(error.code, "numeric_prose_claim");
}

#[test]
fn salvage_preserves_unrelated_analysis_and_layout() {
    let result = parse_and_salvage_report(&prose_report("# 取舍\n\n额外连招会占用原有操作时间。实测伤害提高37.42%。\n\n可以先讨论资源保留与释放时机。"), &EvidenceStore::new()).unwrap();
    assert!(result.content.body_markdown.starts_with("# 取舍\n\n"));
    assert!(result.content.body_markdown.contains("额外连招会占用原有操作时间。"));
    assert!(result.content.body_markdown.contains("\n\n可以先讨论"));
    assert!(!result.content.body_markdown.contains("37.42"));
    assert!(!result.content.body_markdown.contains("这项数值尚待核验"));
    validate_report(&result.content, &EvidenceStore::new()).unwrap();
}

#[test]
fn legacy_reports_still_parse_without_body() {
    let mut value: Value = serde_json::from_str(&prose_report("")).unwrap();
    value.as_object_mut().unwrap().remove("body_markdown");
    value["refusal_reason"] = Value::String("缺少问题中的对象。".into());
    let parsed = parse_and_validate_report(&value.to_string(), &EvidenceStore::new()).unwrap();
    assert!(parsed.content.body_markdown.is_empty());
    assert!(serde_json::to_value(parsed.content).unwrap().get("body_markdown").is_none());
}

#[test]
fn article_outside_json_and_qualitative_suggestions_survive() {
    let body = format!("# 玩家如何选择\n\n{}", "假设防守资源保持不变，增加连续动作会让玩家考虑是否继续投入；退出机会和收益分布可以作为设计变量。\n\n".repeat(5));
    let mut report: Value = serde_json::from_str(&prose_report("正文见上。")).unwrap();
    report["recommendations"] = serde_json::json!([{"title":"考虑退出机会", "rationale":"可以允许玩家中途退出连招，保留应对环境变化的选择。", "evidence_ids":[]}]);
    let raw = format!("{body}\n```json\n{report}\n```");
    let parsed = parse_and_validate_report(&raw, &EvidenceStore::new()).unwrap();
    assert_eq!(parsed.content.body_markdown, body.trim());
    assert_eq!(parsed.content.recommendations.len(), 1);
    assert!(parsed.content.recommendations[0].evidence_ids.is_empty());
    let decoded = parse_and_validate_report(&serde_json::to_string(&raw).unwrap(), &EvidenceStore::new()).unwrap();
    assert_eq!(decoded.content.body_markdown, body.trim());
}

#[test]
fn qualitative_suggestion_does_not_allow_invented_numeric_results() {
    let mut report: Value = serde_json::from_str(&prose_report("可以讨论连招的退出机会。")).unwrap();
    report["recommendations"] = serde_json::json!([{"title":"修改连招", "rationale":"修改后实测伤害提高37.42%。", "evidence_ids":[]}]);
    assert_eq!(parse_and_validate_report(&report.to_string(), &EvidenceStore::new()).unwrap_err().code, "numeric_prose_claim");
}

#[test]
fn optional_qualitative_findings_do_not_force_source_hunting() {
    let mut report: Value = serde_json::from_str(&prose_report("假设原有资源机制不变，连招会增加连续操作的时间承诺。")).unwrap();
    report["findings"] = serde_json::json!([{"title":"时间承诺", "explanation":"玩家需要持续判断是否继续投入。", "evidence_ids":[], "metrics":[]}]);
    let parsed = parse_and_validate_report(&report.to_string(), &EvidenceStore::new()).unwrap();
    assert_eq!(parsed.content.findings.len(), 1);
    report["findings"][0]["metrics"] = serde_json::json!([{"label":"DPS", "value":123.5,"unit":"damage_per_second","evidence_id":"a".repeat(64),"json_pointer":"/result/dps"}]);
    assert_eq!(parse_and_validate_report(&report.to_string(), &EvidenceStore::new()).unwrap_err().code, "finding_without_evidence");
}
