#[tokio::test]
async fn unsupported_prose_gets_one_rewrite_before_bounded_salvage() {
    let invalid = "候选方案实测提升37.42%。这一收益来自新资源。\n\n额外连招会挤占原有操作时间。";
    let corrected = "候选方案引入新资源，额外连招会挤占原有操作时间。尚无候选实测，以下讨论设计取舍。";
    for body in [corrected, invalid] {
        let runtime = AgentRuntime::fixture();
        let response = |body: &str| Ok(ModelResponse {
            assistant_text:Some(json!({"schema_version":AGENT_REPORT_CONTENT_SCHEMA_V1,
                "summary":"分析候选规则的取舍。", "body_markdown":body,
                "findings":[], "recommendations":[], "rotation_changes":[],
                "limitations":[], "refusal_reason":null}).to_string()),
            reasoning_content:None,tool_calls:Vec::new(),finish_reason:FinishReason::Stop,
            usage:TokenUsage::default(),
        });
        let provider = ScriptedProvider::new(vec![response(invalid),response(body)]);
        let mut task = input(&runtime,"prose-rewrite");
        task.question = "按假设分析候选规则".into();
        let result = run_agent(&provider,&runtime,task,AgentRunLimits::default(),AgentCancellation::default()).await;
        let requests = provider.requests();
        assert_eq!(requests.len(),2);
        assert!(requests[1].tools.is_empty());
        assert!(serde_json::to_string(&requests[1].messages).unwrap().contains("按假设分析候选规则"));
        let content = &result.report.unwrap().content;
        assert!(!content.body_markdown.contains("37.42"));
        if body == corrected {
            assert_eq!(result.status,AgentRunStatus::Completed);
            assert_eq!(content.body_markdown,corrected);
        } else {
            assert_eq!(result.status,AgentRunStatus::PartiallyVerified);
            assert!(content.body_markdown.contains("额外连招会挤占原有操作时间"));
        }
    }
}
