# Agent 延迟与逐条校验改造基线

日期：2026-08-26  
范围：`agent-system/v3`、可信场景预取、逐条报告校验、DeepSeek V4 Flash Profile  
公网：关闭

## 问题证据

改造前对本地保存的在线会话进行只读统计：

- 在线运行 43 次，中位耗时 19,145 ms；近期失败案例为 30–42 秒。
- 31 / 43 次触发额外的 `report_repair_requested` 模型轮次。
- 排除安全拒绝后，内容型运行中 14 / 23 次落入 `evidence_insufficient`。
- 高频校验码为 `missing_metric_source`、`numeric_prose_claim`、`uncited_metric` 和 `metric_value_mismatch`。

旧流程把单项 metric 或正文数字错误升级为整份报告失败，并通常再消耗一次模型请求，延迟和可用性同时受损。

## 设计改动

1. 可信编排器在首次模型调用前执行 `get_current_scenario`，通过合法的工具 transcript 注入结果；模型不再为必然读取额外往返。
2. 普通基线从“读取场景 → 模拟 → 报告”三次模型调用缩减为“模拟 → 报告”两次。
3. 严格报告失败后先执行本地逐条校验：保留合法 evidence、finding 和 metric，删除错误 metric，并把无来源正文数字替换为“未验证数值”。
4. 若模型条目全部无效，但本轮存在确定性模拟证据，则生成只包含模拟器直接指标的最小报告。
5. 部分保留结果使用 `partially_verified` 状态并写入正常会话，可继续追问。
6. 仅当 JSON 结构无法解析、本地无法安全降级时，才允许一次无工具模型修复。

## 离线验收结果

- Agent Rust 测试 122 / 122 通过，前端语法检查和 release 构建通过。
- 离线基线报告保持 `completed`，工具调用仍为 2 次、模拟 1 次，模型轮次从 3 降为 2。
- 单项错误报告返回 `partially_verified`，保留模拟器 DPS，不触发 `report_repair_requested`。
- 非法 JSON 仍只允许一次有界修复。
- Phase 1 工具评测 20 / 20，非法写入 0。
- Phase 2 离线模型层评测 12 / 12，evidence citation 和 tool boundary 均为 100%，token 与费用均为 0。
- 隔离验证前后真实 userdata 指纹一致，未改动已有会话。
- 真实模型费用测试单独等待用户确认，不用离线结果冒充真实延迟改善。
