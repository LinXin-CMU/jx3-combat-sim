# v42 连续诊断修复

## 反馈根因

用户同一会话四轮 Flash/v40 分别运行约 131、107、180、180 秒。前两轮停止于验证意愿追问，后两轮达到总运行时限；没有调用 compare_scenarios。旧续接仅带历史解释，未恢复工具检查。事件未携带手动输入来源，模型反复查 ev137 对应操作。

## 已实现

- 可信会话存储从最近三份同场景结果提取最多 24 项去重只读检查参数。白名单仅 simulate_scenario、analyze_timeline、inspect_timeline_events、inspect_rotation_input。
- 新 run 在当前运行环境重做这些确定性查询，将实际结果登记到 EvidenceStore 和本轮工具缓存，并写入新 run 的私有复现记录；不直接把旧解释或旧数值提升为新证据。模拟预算仍按实际执行累计。
- 模型首轮可见恢复后的工具结果，重复同参数查询走本轮缓存。恢复查询为 server_initiated，可调阅。
- 手动主事件记录原始 sequence_index，Agent 输出一基 operation_number。被跳过输入不再需要模型自行对齐；宏事件保持独立宏页/行语义。
- 事件工具增加通用 event_number 精确定位和 rage_cost_below 正耗怒筛选，在分页前生效；免耗技能不匹配正耗怒筛选。
- 追问结果保留供应商公开 assistant_text 为 analysis_text；脱敏、限长，前端标为尚未完成报告校验。私有 reasoning_content 不进入该字段。
- 剩余时间进入收尾窗口时转报告；默认窗口最多 45 秒，小预算为总时限三分之一。这是回合边界收尾，不能抢救已在途且用完总时限的响应。
- 区分外层 run_timeout 与供应商 provider_timeout。
- Pro 调查默认 high，Flash 默认 low；纯报告/修复默认 low。可用服务端环境变量 JX3_AGENT_PRO_REASONING_EFFORT、JX3_AGENT_FLASH_REASONING_EFFORT、JX3_AGENT_REPORT_REASONING_EFFORT 配置 low/high。
- 修正两项过时测试：机制上下文插入后消息数量断言；FakeProvider 从机制上下文误识别用户问题。

## 验证

- cargo test：308/308 通过，19 项既有 warnings。
- 同场景重建及缓存复用、跨场景隔离、事件筛选和输入来源、公开正文保留及隐藏推理隔离均有离线断言。
- node --check frontend/agent.js、frontend/app.js 通过。
- tools/agent-clarification-ui-smoke.py：大页面与侧栏通过，3 次请求全部拦截，0 次模型调用；确认正文可见。
- check_no_direct_writes.sh 通过；git diff --check 通过。
- cargo build 被另一运行中的 debug 可执行文件锁阻止，未停止该进程。未完成新二进制 Golden 指纹回归，不更新 Golden。
- 未修改既有用户会话文件，未部署，3005 仍运行旧 release。

## 尚需验证与后续能力

- 独立构建、战斗指纹回归、部署和原反馈多轮真实模型验收。单元测试通过不等于已证明真实模型会完成候选实验。
- 目前是按会话参数重建只读证据，不是完整持久化执行栈：尚不恢复未执行的结构化候选、供应商推理状态、比较结果或跨重启模型请求。
- 未实现流式供应商响应与首 token / 推理 / 正文分段计时；当前仍以完整响应为单位。
- 非必要追问由 v41/v42 工具职责指导模型选择，未增加关键词黑名单或语义硬路由。

## 2026-09-06 部署与真实验收补记

- 独立 release 构建通过，309/309 单元测试通过。
- 最终二进制 SHA256：93835CE09D3F7484D8F2EC292C856102D438318054D77D828C870F4E5AB47EB8。
- 3005 已部署，PID 34944，仅监听 127.0.0.1。旧版保留于 backend/target/deployment-backups/v42-20260906-124548/jx3-combat-sim.exe。
- 切换时 Windows 暂未释放 exe 占用，前两次自动回滚；加入短暂复制重试后成功。核对 28,157 个既有非知识缓存数据文件，变化 0。
- Golden 两版本共 8 案例的重复确定性、Lite/Full、指纹和数值相同；严格 Golden 检查仍因源码 data_sha256 与旧快照不一致而返回失败。未更新 Golden，不能称严格 Golden 全通过。
- 用户原始 313 操作场景，新旧二进制 fingerprint 均为 12238056427503132456，DPS 均为 2968513.3469472807；新版 313 个主动事件均有输入来源编号。

### 真实模型结果与新增修复

隔离端口 3018、临时 userdata，实际使用原反馈冻结场景和 Flash；未写入生产会话。结果文件位于 backend/target/v42-acceptance-20260906-122004。

1. 解释核心思路：73.7 秒，partially_verified。
2. 好处与主要问题：179.4 秒，恢复 6 项检查，partially_verified。
3. 续接测试：180 秒超时，恢复 13 项检查，2 次比较尝试均失败。
4. “跑”：166.2 秒，恢复 17 项检查，5 次比较尝试中 3 次成功，partially_verified；最终报告根据删除操作后的实测下降否定删除建议。

真实测试发现并修复三个参数/工具问题：

- 接受 selector=rage_cost_below 且同时提供 skill_name 与有效阈值的明确组合，规范化为相同 skill 查询；回归验证证据相同。
- sequence_edits 从最多 4 处放宽至 32 处，允许一项假设涉及 5 个位置，保留总模拟预算和逐行验证。
- remove 可附带预期技能名，用于核对删除目标；名字不匹配仍拒绝。此前带名字会无条件拒绝，导致模型改写参数重试。

最后两项参数修复已通过单元测试并编入部署版本；未再次完整跑五轮付费对话。真实模型共消耗至少 495,015 个已返回 usage tokens，另有一次主动取消请求可能未回传 usage。200,000 的续接测试 token guard 是轮间检查，实际结束于 284,264，非严格费用上限。

结论：工具续接与真实 A/B 链路已经工作，但时延、重复调用、完整候选/比较状态持久化和流式输出尚未全部解决，不能标记多轮可靠性验收全通过。
