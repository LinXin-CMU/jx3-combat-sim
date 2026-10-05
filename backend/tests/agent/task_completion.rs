use super::*;
use crate::agent::{
    compare_scenarios, simulate_scenario, AgentRuntime, CandidatePatchV1, PatchValueV1,
    ScenarioPatchV1, ToolBudget, ToolProvenance,
};

fn report(text: Option<&str>) -> AgentReportContentV1 {
    serde_json::from_value(json!({
        "schema_version": "agent-report-content/v1", "summary": "讨论方案取舍。",
        "body_markdown": "假设资源不变，连续动作会影响应变机会。",
        "findings": [], "recommendations": [], "rotation_changes": [],
        "artifacts": text.map(|text| vec![json!({
            "title": "最终宏", "language": "jx3_macro", "content": text,
            "syntax": "verified"
        })]).unwrap_or_default(),
        "limitations": [], "refusal_reason": null
    }))
    .unwrap()
}

fn macro_patch(label: &str, text: &str) -> CandidatePatchV1 {
    CandidatePatchV1 {
        label: label.into(),
        patch: ScenarioPatchV1 {
            macro_text: Some(PatchValueV1::Set(text.into())),
            macro_duration: Some(PatchValueV1::Set(10.0)),
            ..Default::default()
        },
    }
}

fn compared(texts: &[&str]) -> (ScenarioSnapshotV1, EvidenceStore) {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    let patches = texts
        .iter()
        .enumerate()
        .map(|(index, text)| macro_patch(&format!("candidate-{index}"), text))
        .collect::<Vec<_>>();
    let execution = compare_scenarios(
        "completion-test",
        &scenario,
        &patches,
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(4),
    )
    .unwrap();
    let value = serde_json::to_value(execution.evidence).unwrap();
    (scenario, store(value))
}

fn store(envelope: Value) -> EvidenceStore {
    [(
        envelope["evidence_id"].as_str().unwrap().to_string(),
        envelope,
    )]
    .into_iter()
    .collect()
}

// Mutations below model structurally valid older/different tool records. Renew
// their content identity so checks are exercised beyond the corruption guard.
fn renew(value: Value) -> Value {
    let envelope: EvidenceEnvelopeV1<Value> = serde_json::from_value(value).unwrap();
    serde_json::to_value(
        EvidenceEnvelopeV1::new(
            envelope.trace_id,
            envelope.tool_name,
            envelope.scenario_hash,
            envelope.args,
            envelope.result,
            &ToolProvenance {
                engine_version: envelope.engine_version,
                engine_commit: envelope.engine_commit,
                data_hash: envelope.data_hash,
            },
            envelope.duration_ms,
        )
        .unwrap(),
    )
    .unwrap()
}

fn acceptance(count: u8) -> TaskAcceptanceV1 {
    TaskAcceptanceV1 {
        min_comparison_candidates: count,
        ..Default::default()
    }
}

fn check_status<'a>(completion: &'a TaskCompletionV1, id: &str) -> &'a str {
    &completion
        .checks
        .iter()
        .find(|check| check.id == id)
        .unwrap()
        .status
}

#[test]
fn final_macro_matches_a_real_comparison_without_another_simulation() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let content = report(Some("\r\n/cast 盾击\r\n"));
    let before = content.clone();
    let requested = TaskAcceptanceV1 {
        min_comparison_candidates: 1,
        require_macro_artifact: true,
        require_simulation: true,
    };
    let completion = assess(
        "交付宏并完成对照",
        &scenario,
        &content,
        &evidence,
        Some(&requested),
    );
    assert_eq!(completion.status, "checks_passed");
    assert!(completion.semantic_review_required);
    assert_eq!(completion.artifacts[0].status, "simulation_matched");
    assert_eq!(
        completion.artifacts[0].evidence_ids,
        evidence.keys().cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        completion.artifacts[0].content_sha256,
        format!(
            "{:x}",
            Sha256::digest(content.artifacts[0].content.as_bytes())
        )
    );
    assert_eq!(
        content, before,
        "assessment must retain the complete final text"
    );
}

#[test]
fn edits_to_lines_conditions_pages_or_order_are_not_the_tested_macro() {
    let original = "/cast 盾击\n/cast 盾压";
    let (scenario, evidence) = compared(&[original]);
    for edited in [
        "/cast 盾击",
        "/cast [rage>50] 盾击\n/cast 盾压",
        "#page shield\n/cast 盾击\n/cast 盾压",
        "/cast 盾压\n/cast 盾击",
    ] {
        let completion = assess("交付宏", &scenario, &report(Some(edited)), &evidence, None);
        assert_eq!(completion.status, "checks_incomplete", "{edited}");
        assert_eq!(completion.artifacts[0].status, "not_tested", "{edited}");
        assert!(completion.artifacts[0].evidence_ids.is_empty());
    }
}

#[test]
fn another_scenario_cannot_satisfy_artifact_or_comparison_requirements() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let mut value = evidence.values().next().unwrap().clone();
    value["scenario_hash"] = json!("b".repeat(64));
    let completion = assess(
        "对照并交付宏",
        &scenario,
        &report(Some("/cast 盾击")),
        &store(renew(value)),
        Some(&acceptance(1)),
    );
    assert_eq!(
        check_status(&completion, "comparison_candidates"),
        "missing"
    );
    assert_eq!(completion.artifacts[0].status, "not_tested");
}

#[test]
fn rejected_calls_and_unrelated_tools_cannot_supply_completion_evidence() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let original = evidence.values().next().unwrap().clone();
    let rejected: EvidenceStore = [(
        "failed-call".into(),
        json!({
            "schema_version": "agent-tool-result/v1", "ok": false,
            "tool_name": "compare_scenarios", "evidence": [original.clone()]
        }),
    )]
    .into_iter()
    .collect();
    let mut unrelated = original;
    unrelated["tool_name"] = json!("get_current_scenario");
    for evidence in [rejected, store(renew(unrelated))] {
        let requested = TaskAcceptanceV1 {
            require_simulation: true,
            ..acceptance(1)
        };
        let result = assess(
            "完成对照",
            &scenario,
            &report(None),
            &evidence,
            Some(&requested),
        );
        assert_eq!(check_status(&result, "comparison_candidates"), "missing");
        assert_eq!(check_status(&result, "simulation"), "missing");
    }
}

#[test]
fn duplicate_candidate_scenarios_count_once_even_with_different_labels() {
    let (scenario, evidence) = compared(&["/cast 盾击", "/cast 盾击"]);
    let result = assess(
        "比较两案",
        &scenario,
        &report(None),
        &evidence,
        Some(&acceptance(2)),
    );
    assert_eq!(result.status, "checks_incomplete");
    assert!(result.checks[0].detail.contains("1 个不同候选"));
    assert_eq!(result.checks[0].evidence_ids.len(), 1);
    let passed = assess(
        "比较一案",
        &scenario,
        &report(None),
        &evidence,
        Some(&acceptance(1)),
    );
    assert_eq!(passed.status, "checks_passed");
}

#[test]
fn distinct_real_candidate_scenarios_satisfy_the_requested_count() {
    let (scenario, evidence) = compared(&["/cast 盾击", "/cast 盾压"]);
    let result = assess(
        "比较两案",
        &scenario,
        &report(None),
        &evidence,
        Some(&acceptance(2)),
    );
    assert_eq!(result.status, "checks_passed");
    assert!(result.checks[0].detail.contains("2 个不同候选"));
}

#[test]
fn baseline_and_unchanged_candidates_are_not_alternative_candidates() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let original = evidence.values().next().unwrap().clone();
    for mutation in ["baseline", "no_changes"] {
        let mut value = original.clone();
        if mutation == "baseline" {
            value["result"]["candidates"][0]["metrics"]["scenario_hash"] =
                json!(scenario.scenario_hash);
        } else {
            value["result"]["candidates"][0]["changes"] = json!([]);
        }
        let result = assess(
            "比较一案",
            &scenario,
            &report(None),
            &store(renew(value)),
            Some(&acceptance(1)),
        );
        assert_eq!(result.status, "checks_incomplete");
    }
}

#[test]
fn a_qualitative_answer_has_no_implicit_simulation_requirement() {
    let runtime = AgentRuntime::fixture();
    let result = assess(
        "讨论玩家体验取舍",
        &runtime.fixture_scenario(),
        &report(None),
        &EvidenceStore::new(),
        None,
    );
    assert_eq!(result.status, "needs_review");
    assert!(result.semantic_review_required);
    assert_eq!(result.checks.len(), 1);
    assert_eq!(result.checks[0].status, "needs_review");
    assert!(!result.checks[0].detail.contains("必须模拟"));
}

#[test]
fn historical_acceptance_defaults_and_optional_comparison_fields_remain_compatible() {
    let empty: TaskAcceptanceV1 = serde_json::from_value(json!({})).unwrap();
    assert!(empty.is_empty());
    assert!(empty.validate().is_ok());
    assert!(serde_json::from_value::<TaskAcceptanceV1>(json!({"write": true})).is_err());
    assert!(acceptance(8).validate().is_ok());
    assert!(acceptance(9).validate().is_err());
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let mut historical = evidence.values().next().unwrap().clone();
    let candidate = historical["result"]["candidates"][0]
        .as_object_mut()
        .unwrap();
    candidate.remove("macro_pages");
    candidate.remove("condition_semantics");
    candidate["metrics"]
        .as_object_mut()
        .unwrap()
        .remove("macro_line_stats");
    candidate["metrics"]
        .as_object_mut()
        .unwrap()
        .remove("skills");
    let result = assess(
        "交付宏",
        &scenario,
        &report(Some("/cast 盾击")),
        &store(renew(historical)),
        None,
    );
    assert_eq!(result.artifacts[0].status, "simulation_matched");
}

#[test]
fn explicit_unmet_acceptance_is_reported_without_deleting_the_answer() {
    let runtime = AgentRuntime::fixture();
    let content = report(None);
    let requested = TaskAcceptanceV1 {
        min_comparison_candidates: 2,
        require_macro_artifact: true,
        require_simulation: true,
    };
    let result = assess(
        "比较两案并交付宏",
        &runtime.fixture_scenario(),
        &content,
        &EvidenceStore::new(),
        Some(&requested),
    );
    assert_eq!(result.status, "checks_incomplete");
    assert_eq!(result.checks.len(), 3);
    assert!(result.checks.iter().all(|item| item.status == "missing"));
    assert!(!content.body_markdown.is_empty());
}

#[test]
fn invalid_macro_syntax_cannot_be_overridden_by_the_model() {
    let runtime = AgentRuntime::fixture();
    let content = report(Some("/cast [rage>] 绝刀"));
    let result = assess(
        "交付宏",
        &runtime.fixture_scenario(),
        &content,
        &EvidenceStore::new(),
        None,
    );
    assert_eq!(result.status, "checks_incomplete");
    assert_eq!(result.artifacts[0].status, "invalid_syntax");
    assert_eq!(content.artifacts[0].syntax, "verified");
}

#[test]
fn missing_fingerprints_zero_casts_and_inconsistent_candidate_hashes_are_not_verified() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let original = evidence.values().next().unwrap().clone();
    for field in [
        "fingerprint",
        "active_casts",
        "candidate_hash",
        "before_value",
    ] {
        let mut value = original.clone();
        let candidate = &mut value["result"]["candidates"][0];
        match field {
            "fingerprint" => {
                candidate["metrics"]
                    .as_object_mut()
                    .unwrap()
                    .remove("fingerprint");
            }
            "active_casts" => candidate["metrics"]["diagnostics"]["active_event_count"] = json!(0),
            "candidate_hash" => candidate["metrics"]["scenario_hash"] = json!("c".repeat(64)),
            _ => candidate["changes"][0]["before"] = json!("not the baseline value"),
        }
        let result = assess(
            "交付宏并对照",
            &scenario,
            &report(Some("/cast 盾击")),
            &store(renew(value)),
            Some(&acceptance(1)),
        );
        assert_eq!(result.artifacts[0].status, "not_tested", "{field}");
        assert_eq!(
            check_status(&result, "comparison_candidates"),
            "missing",
            "{field}"
        );
    }
}

#[test]
fn modified_envelope_content_is_rejected_without_trusting_its_old_id() {
    let (scenario, evidence) = compared(&["/cast 盾击"]);
    let mut altered = evidence.values().next().unwrap().clone();
    altered["result"]["candidates"][0]["changes"][0]["after"] = json!("/cast 盾压");
    let result = assess(
        "交付宏",
        &scenario,
        &report(Some("/cast 盾压")),
        &store(altered),
        Some(&acceptance(1)),
    );
    assert_eq!(result.artifacts[0].status, "not_tested");
    assert_eq!(check_status(&result, "comparison_candidates"), "missing");
}

#[test]
fn the_current_simulated_macro_can_match_without_a_comparison() {
    let runtime = AgentRuntime::fixture();
    let mut request = runtime.fixture_scenario().simulation;
    request.macro_duration = Some(10.0);
    crate::agent::compare::configure_macro_rotation(&mut request, "/cast 盾击".into());
    let scenario =
        ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), request).unwrap();
    let simulation = simulate_scenario(
        "completion-test",
        &scenario,
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(1),
    )
    .unwrap();
    let evidence = store(serde_json::to_value(simulation.evidence).unwrap());
    let result = assess(
        "交付当前宏",
        &scenario,
        &report(Some("/cast 盾击")),
        &evidence,
        Some(&TaskAcceptanceV1 {
            require_simulation: true,
            ..Default::default()
        }),
    );
    assert_eq!(result.status, "checks_passed");
    assert_eq!(result.artifacts[0].status, "simulation_matched");
    let comparison = assess(
        "还要比较替代方案",
        &scenario,
        &report(None),
        &evidence,
        Some(&acceptance(1)),
    );
    assert_eq!(comparison.status, "checks_incomplete");
}

#[test]
fn an_unchanged_combat_effect_is_tested_instead_of_mislabeled_untested() {
    let runtime = AgentRuntime::fixture();
    let mut request = runtime.fixture_scenario().simulation;
    request.macro_duration = Some(10.0);
    crate::agent::compare::configure_macro_rotation(&mut request, "/cast 盾击".into());
    let scenario =
        ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), request).unwrap();
    let text = "/cast 盾击\n// 保留同一动作";
    let execution = compare_scenarios(
        "completion-test",
        &scenario,
        &[macro_patch("same-effect", text)],
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(2),
    )
    .unwrap();
    assert!(execution.evidence.result.candidates[0].same_fingerprint);
    let result = assess(
        "检查此宏",
        &scenario,
        &report(Some(text)),
        &store(serde_json::to_value(execution.evidence).unwrap()),
        None,
    );
    assert_eq!(result.artifacts[0].status, "simulation_matched");
    assert!(result.artifacts[0].detail.contains("基线指纹相同"));
    assert!(result.artifacts[0]
        .detail
        .contains("不证明精确复刻或全局最优"));
}

#[test]
fn other_deliverables_remain_reviewable_and_do_not_gain_macro_verification() {
    let runtime = AgentRuntime::fixture();
    let mut content = report(Some("const design = 'proposed';"));
    content.artifacts[0].language = "javascript".into();
    let result = assess(
        "给出设计草案",
        &runtime.fixture_scenario(),
        &content,
        &EvidenceStore::new(),
        None,
    );
    assert_eq!(result.status, "needs_review");
    assert_eq!(result.artifacts[0].status, "not_executable");
    content.artifacts[0].content = "/cast 盾击".into();
    let mislabeled = assess(
        "交付宏",
        &runtime.fixture_scenario(),
        &content,
        &EvidenceStore::new(),
        None,
    );
    assert_eq!(mislabeled.artifacts[0].status, "not_tested");
}

#[test]
fn a_real_comparison_can_change_a_serialization_omitted_default() {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    assert_eq!(scenario.simulation.dunya_reset_seed, 0);
    assert!(serde_json::to_value(&scenario.simulation)
        .unwrap()
        .get("dunya_reset_seed")
        .is_none());
    let candidate = CandidatePatchV1 {
        label: "seed-change".into(),
        patch: ScenarioPatchV1 {
            dunya_reset_seed: Some(7),
            ..Default::default()
        },
    };
    let comparison = compare_scenarios(
        "completion-test",
        &scenario,
        &[candidate],
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(2),
    )
    .unwrap();
    let evidence = store(serde_json::to_value(comparison.evidence).unwrap());
    let result = assess(
        "比较另一个种子",
        &scenario,
        &report(None),
        &evidence,
        Some(&acceptance(1)),
    );
    assert_eq!(result.status, "checks_passed");
}

#[test]
fn timeline_analysis_alone_proves_a_real_simulation_and_current_macro() {
    let runtime = AgentRuntime::fixture();
    let mut request = runtime.fixture_scenario().simulation;
    request.macro_duration = Some(10.0);
    crate::agent::compare::configure_macro_rotation(&mut request, "/cast 盾击".into());
    let scenario =
        ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), request).unwrap();
    let simulation = simulate_scenario(
        "completion-test",
        &scenario,
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(1),
    )
    .unwrap();
    let timeline =
        crate::agent::analyze_timeline("completion-test", &simulation, runtime.provenance())
            .unwrap();
    let evidence = store(serde_json::to_value(timeline.evidence).unwrap());
    let requested = TaskAcceptanceV1 {
        require_simulation: true,
        ..Default::default()
    };
    let result = assess(
        "分析当前宏",
        &scenario,
        &report(Some("/cast 盾击")),
        &evidence,
        Some(&requested),
    );
    assert_eq!(result.status, "checks_passed");
    assert_eq!(result.artifacts[0].status, "simulation_matched");
    let mut no_casts = evidence.values().next().unwrap().clone();
    no_casts["result"]["active_event_count"] = json!(0);
    let rejected = assess(
        "分析当前宏",
        &scenario,
        &report(None),
        &store(renew(no_casts)),
        Some(&requested),
    );
    assert_eq!(rejected.status, "checks_incomplete");
}

#[test]
fn the_public_objective_is_redacted_and_bounded_even_for_direct_callers() {
    let runtime = AgentRuntime::fixture();
    let objective = format!(
        "api_key = this-is-an-inert-test-value\n{}",
        "分析当前场景".repeat(5000)
    );
    let result = assess(
        &objective,
        &runtime.fixture_scenario(),
        &report(None),
        &EvidenceStore::new(),
        None,
    );
    assert!(!result.objective.contains("inert-test-value"));
    assert!(result.objective.starts_with("[REDACTED SENSITIVE VALUE]"));
    assert!(result.objective.len() <= 16 * 1024);
    assert!(result.objective.ends_with('…'));
}

#[test]
fn a_manual_sequence_with_unused_macro_text_does_not_verify_the_macro() {
    let runtime = AgentRuntime::fixture();
    let mut request = runtime.fixture_scenario().simulation;
    request.macro_text = Some("/cast 盾击".into());
    let scenario =
        ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), request).unwrap();
    let simulation = simulate_scenario(
        "completion-test",
        &scenario,
        &runtime.context(),
        runtime.provenance(),
        &mut ToolBudget::new(1),
    )
    .unwrap();
    assert!(simulation.evidence.result.skill_count > 0);
    let evidence = store(serde_json::to_value(simulation.evidence).unwrap());
    let requested = TaskAcceptanceV1 {
        require_simulation: true,
        ..Default::default()
    };
    let result = assess(
        "验证宏",
        &scenario,
        &report(Some("/cast 盾击")),
        &evidence,
        Some(&requested),
    );
    assert_eq!(check_status(&result, "simulation"), "passed");
    assert_eq!(result.artifacts[0].status, "not_tested");
}
