use super::*;

#[test]
fn original_user_request_survives_many_clarifications_and_report_turn_limit() {
    let mut events = vec![AgentSessionEventV1::user_message("original", "假设换第一重奇穴，写策划分析")];
    for index in 0..9 {
        events.push(AgentSessionEventV1::user_message(&format!("run-{index}"),
            &format!("针对你的问题：是否改宏第六行？\n我的回答：补充约束{index}")));
    }
    let context: Value = serde_json::from_str(&build_prior_context(&events).unwrap()).unwrap();
    let messages = context["user_messages"].as_array().unwrap();
    assert_eq!(messages.len(), 7);
    assert_eq!(messages[0], "假设换第一重奇穴，写策划分析");
    assert_eq!(messages.last().unwrap(), "补充约束8");
    assert!(!serde_json::to_string(messages).unwrap().contains("是否改宏"));
}

#[test]
fn legacy_reply_wrapper_does_not_turn_assistant_question_into_user_intent() {
    assert_eq!(clarification_answer("针对你的问题：提供全部系数\n我的回答：你自己看模拟器数据库"), "你自己看模拟器数据库");
    assert_eq!(clarification_answer("新任务：请解释我的回答：这几个字"), "新任务：请解释我的回答：这几个字");
}
