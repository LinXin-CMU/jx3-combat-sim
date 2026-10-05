fn completion_prose_response(body: &str) -> ModelResponse {
    ModelResponse {
        assistant_text: Some(json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,
            "summary":"讨论当前方案。", "body_markdown":body,
            "findings":[],"recommendations":[],"rotation_changes":[],"artifacts":[],
            "limitations":[],"refusal_reason":null}).to_string()),
        reasoning_content: None, tool_calls: vec![], finish_reason: FinishReason::Stop,
        usage: TokenUsage::default(),
    }
}

#[tokio::test]
async fn completion_missing_evidence_preserves_answer_and_does_not_force_more_turns() {
    let runtime = AgentRuntime::fixture();
    let body = "可以先讨论资源保留与释放时机；该回答尚未包含对照实验。";
    let provider = ScriptedProvider::new(vec![Ok(completion_prose_response(body))]);
    let mut request = input(&runtime, "completion-missing-comparison");
    request.acceptance = Some(super::super::completion::TaskAcceptanceV1 {
        min_comparison_candidates: 1, ..Default::default()
    });
    let result = run_agent(&provider, &runtime, request, AgentRunLimits::default(), AgentCancellation::default()).await;
    assert_eq!(result.status, AgentRunStatus::Completed, "run lifecycle is separate from acceptance");
    assert_eq!(result.report.as_ref().unwrap().content.body_markdown, body);
    assert_eq!(result.task_completion.as_ref().unwrap().status, "checks_incomplete");
    assert!(result.task_completion.as_ref().unwrap().semantic_review_required);
    assert_eq!(result.accounting.simulations, 0);
    assert_eq!(provider.requests().len(), 1, "completion must not add hidden model calls");
    let sent = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(sent.contains("explicit_acceptance"));
}

#[tokio::test]
async fn completion_qualitative_answer_needs_review_without_forced_simulation() {
    let runtime = AgentRuntime::fixture();
    let body = "假设保留防守资源，中途退出能降低连续操作负担，同时需要控制反复退出的收益。";
    let provider = ScriptedProvider::new(vec![Ok(completion_prose_response(body))]);
    let result = run_agent(&provider, &runtime, input(&runtime, "completion-qualitative"), AgentRunLimits::default(), AgentCancellation::default()).await;
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.task_completion.as_ref().unwrap().status, "needs_review");
    assert_eq!(result.report.as_ref().unwrap().content.body_markdown, body);
    assert_eq!(result.accounting.simulations, 0);
    assert_eq!(provider.requests().len(), 1);
    let mut historical = serde_json::to_value(&result).unwrap();
    historical.as_object_mut().unwrap().remove("task_completion");
    let restored: AgentRunResultV1 = serde_json::from_value(historical).unwrap();
    assert!(restored.task_completion.is_none(), "historical reports remain readable");
}

#[tokio::test]
async fn completion_recognizes_real_simulation_but_does_not_certify_semantics() {
    let runtime = AgentRuntime::fixture();
    let provider = ScriptedProvider::new(vec![
        Ok(tool_call("verify", "simulate_scenario", json!({}))),
        Ok(completion_prose_response("已取得当前场景模拟；如何取舍仍须结合用户目标。")),
    ]);
    let mut request = input(&runtime, "completion-real-simulation");
    request.acceptance = Some(super::super::completion::TaskAcceptanceV1 {
        require_simulation: true, ..Default::default()
    });
    let result = run_agent(&provider, &runtime, request, AgentRunLimits::default(), AgentCancellation::default()).await;
    assert_eq!(result.task_completion.as_ref().unwrap().status, "checks_passed");
    assert!(result.task_completion.as_ref().unwrap().semantic_review_required);
    assert_eq!(result.accounting.simulations, 1);
    assert_eq!(provider.requests().len(), 2);
    let restored: AgentRunResultV1 = serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
    assert_eq!(restored.task_completion, result.task_completion);
}

#[tokio::test]
async fn completion_invalid_acceptance_is_rejected_before_model_call() {
    let runtime = AgentRuntime::fixture();
    let provider = ScriptedProvider::new(vec![]);
    let mut request = input(&runtime, "completion-invalid-acceptance");
    request.acceptance = Some(super::super::completion::TaskAcceptanceV1 {
        min_comparison_candidates: 9, ..Default::default()
    });
    let result = run_agent(&provider, &runtime, request, AgentRunLimits::default(), AgentCancellation::default()).await;
    assert_eq!(result.status, AgentRunStatus::ProtocolFailed);
    assert!(provider.requests().is_empty());
}
