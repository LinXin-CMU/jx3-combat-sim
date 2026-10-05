#[tokio::test]
async fn model_can_finish_with_free_prose_without_forced_simulation() {
    let runtime = AgentRuntime::fixture();
    let body = "# 操作节奏的变化\n\n假设保留原有资源规则，额外连招会增加集中操作的时间。\n\n玩家需要在保留资源与完成连招之间作出选择。";
    let provider = ScriptedProvider::new(vec![Ok(ModelResponse {
        assistant_text: Some(json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,
            "summary":"分析操作节奏与资源取舍。", "body_markdown":body,
            "findings":[],"recommendations":[],"rotation_changes":[],"limitations":[],"refusal_reason":null}).to_string()),
        reasoning_content:None, tool_calls:vec![], finish_reason:FinishReason::Stop, usage:TokenUsage::default(),
    })]);
    let mut request = input(&runtime, "free-prose-without-simulation");
    request.question = "假设资源机制保持不变，增加一段连招会如何改变玩家的操作决策？写成连续的分析。".into();
    let result = run_agent(&provider, &runtime, request, AgentRunLimits::default(), AgentCancellation::default()).await;
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(result.accounting.simulations, 0);
    assert_eq!(result.accounting.knowledge_searches, 0);
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(result.report.unwrap().content.body_markdown, body);
}
