# v41 用户澄清职责

## 依据与范围

- Claude Code 工具说明：https://code.claude.com/docs/en/tools-reference#askuserquestion-tool-behavior
- Agent SDK 用户交互：https://code.claude.com/docs/en/agent-sdk/user-input#handle-clarifying-questions
- 官方职责是收集需求、消除歧义和选择方向；工具授权独立处理。
- 本项目将查资料、定位事件、任务范围内模拟和 A/B 的安排交给模型，用户提供目标、偏好和工具无法获得的独有信息。

## 修改

- 新增 v41 prompt，保留 v40 以便历史溯源。
- 同步 ask_user_question 工具说明与 reason 字段说明。
- 删除解析器的中文措辞黑名单及对应六项关键词测试。保留结构、选项和凭据安全校验。
- 用真实偏好问题“你更关心哪一种：极限输出还是一键容错？”验证旧黑名单不再误伤。
- 保留原会话追问和选项投影；提示模型将用户答案更新为原目标的约束继续处理。
- 未增加语义硬路由或额外分类模型。模型实际追问质量仍需真实会话验收，单测不证明模型行为。

## 验证

- cargo test clarification：2/2 通过，覆盖选项协议、历史兼容、偏好措辞与会话上下文。
- cargo test：304 通过、2 失败；分别单独重跑仍失败。
  - bounded_session_context_precedes_current_question_as_untrusted_data：消息数量实际 5，断言 4。
  - offline_provider_demonstrates_versioned_knowledge_end_to_end：知识检索次数实际 0，断言 1。
- node --check frontend/agent.js 与 frontend/app.js 通过。
- git diff --check 通过（已有换行格式提示）。
- 无真实模型请求；未部署或重启 3005，运行版本仍为 v40。

## 应用验收

1. “哪些绝刀打亏？”：模型按需要自行定位、检查或验证。
2. 用户目标存在实质取舍：提供相关方向选项和自定义回答。
3. 回答偏好后：继续原任务，采用用户选择的约束。
4. 能力不足：解释当前可判断的部分与局限。
