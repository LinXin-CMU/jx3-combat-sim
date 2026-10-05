use super::*;

#[test]
fn final_task_context_keeps_actual_user_goal_after_generated_diagnosis() {
    let runtime = AgentRuntime::fixture();
    let input = AgentRunInput {
        acceptance: None,
        run_id:"task-anchor".into(), question:"你自己查数据库".into(),
        scenario:runtime.fixture_scenario(), resume_tools:Vec::new(),
        session_context:Some(serde_json::json!({
            "user_messages":["假设把第一重奇穴换成另一心法的技能，写设计分析"],
            "turns":[{"summary":"宏替换就是用户目标".repeat(10000)}]
        }).to_string()), session_playbook_id:None, task_hint:None,
        analysis_surface:None, equipment_workspace:None,
    };
    let mut messages = vec![ModelMessage::User {content:"<diagnostic_state>旧宏没有释放</diagnostic_state>".into()}];
    append_current_task(&mut messages, &input);
    let ModelMessage::User {content} = messages.last().unwrap() else { panic!("missing task") };
    assert!(content.contains("假设把第一重奇穴换成另一心法的技能，写设计分析"));
    assert!(content.contains("你自己查数据库"));
    assert!(!content.contains("宏替换就是用户目标"));
    assert!(content.len() < 12_000);
}

#[test]
fn shrinking_definition_evidence_keeps_resolvable_citation_identity() {
    let id = "a".repeat(64);
    let item = serde_json::json!({"evidence_id":id, "tool_name":"lookup_skill_definitions", "result":{
        "query":"阵云", "matches":[{"kind":"skill", "scope":{"mount":"fenshanjin", "definition_hash":"b".repeat(64)},
        "definition":{"name":"阵云结晦", "description":"技能定义说明".repeat(500)}}]}});
    let compact = shrink_model_evidence_item(&item, 1500);
    assert_eq!(compact["evidence_id"],id);
    assert!(model_json_bytes(&compact) <= 1500);
}

#[test]
fn future_validation_in_a_design_article_is_not_an_executable_macro_plan() {
    let report = serde_json::json!({"summary":"按假设完成设计分析。", "findings":[],
        "recommendations":[{"title":"后续验证", "rationale":"实现候选规则后用 compare_scenarios 做对照。"}],
        "artifacts":[{"title":"策划案", "language":"markdown", "content":"资源与循环设计"}]}).to_string();
    assert_eq!(unexecuted_plan_marker(Some(&report), &HashSet::new(), &EvidenceStore::new()), None);
}

#[test]
fn initial_context_does_not_preselect_macro_diagnosis_or_discard_full_evidence() {
    let source = serde_json::json!({"ok":true,"evidence":[{"tool_name":"get_current_scenario","result":{
        "game_version":"test", "mount":"tieguyi", "rotation_mode":"macro",
        "selected_talents":[{"id":123,"name":"示例奇穴"}],
        "rotation_input":{"macro_statements":[{"statement":"/cast 盾击"}]},
        "mechanics_context":{"macro_runtime_reference":{"example":"详细语义"}}
    }}]});
    let initial = initial_scenario_context(&source);
    let result = &initial["evidence"][0]["result"];
    assert_eq!(result["mount"], "tieguyi");
    assert_eq!(result["selected_talents"][0]["id"],123);
    assert!(result.get("rotation_input").is_none());
    assert!(result["context_purpose"].as_str().unwrap().contains("get_current_scenario"));
    assert!(model_tool_output(&source)["evidence"][0]["result"].get("rotation_input").is_some());
}

#[test]
fn source_mount_mention_does_not_relabel_frozen_target() {
    let runtime = AgentRuntime::fixture_for(crate::GameVersion::CangShengZhuShiTest, crate::Mount::TieGuYi);
    let plan = select_model_led_analysis_plan("把第一重换成分山测试服的奇穴", None, &runtime.fixture_scenario());
    assert_eq!(plan.resolved_scope.mount, "tieguyi");
    assert_eq!(plan.resolved_scope.season, "苍生铸世测试服（2026）");
}

#[test]
fn skill_definitions_survive_normal_and_compacted_model_projection() {
    let runtime = AgentRuntime::fixture_for(crate::GameVersion::CangShengZhuShiTest, crate::Mount::TieGuYi);
    let envelope = super::super::skill_catalog::lookup("definition-projection", super::super::skill_catalog::DefinitionQuery {
        query:"阵云".into(), ..Default::default()
    }, &runtime.fixture_scenario(), &runtime).unwrap();
    let value = serde_json::to_value(envelope).unwrap();
    assert!(tool_result_cache_key("lookup_skill_definitions", &serde_json::json!({"query":"阵云"})).is_some());
    let normal = model_tool_output(&serde_json::json!({"ok":true,"evidence":[value.clone()]}));
    let compact = compact_handoff_evidence(&value);
    for result in [&normal["evidence"][0]["result"], &compact["result"]] {
        assert_eq!(result["applies_rules_to_scenario"], false);
        assert!(result["matches"].as_array().unwrap().iter().any(|item|
            item["definition"]["description"].as_str().is_some_and(|text| text.contains("暴怒"))
            && item["scope"]["mount"] == "fenshanjin"));
    }
}

#[test]
fn compacted_history_keeps_user_goal_separate_from_assistant_direction() {
    let history = serde_json::json!({"user_messages":["写奇穴替换的策划案", "换成另一心法的第一重", "你自己查数据库"],
        "turns":[{"summary":"旧助手误解".repeat(3000)}, {"question":"你自己查数据库", "summary":"不应继承为用户目标的宏替换建议".repeat(3000)}]}).to_string();
    let compact = compact_session_context(&history, 2000);
    assert!(compact.chars().count() <= 2000);
    let value: Value = serde_json::from_str(&compact).unwrap();
    assert_eq!(value["user_messages"][0], "写奇穴替换的策划案");
    assert_eq!(value["user_messages"][2], "你自己查数据库");
    let wrapped = bounded_session_message(&wrap_session_context(&history), 2000);
    assert!(wrapped.contains("写奇穴替换的策划案"));
    assert!(wrapped.contains("user_messages"));
}
