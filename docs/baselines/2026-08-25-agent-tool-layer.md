# P1 Agent 工具层验收与性能基线

记录日期：2026-08-25

证据构建提交：`b887aa331df5674f88df00a11a7aa67480f97ab7`

结论：第 1 阶段的四个强类型只读工具、证据链、20 题无模型评测和阶段回归全部通过；这仍是工具层，不声称语言模型回答质量或 Agent MVP 已完成。

## 1. 环境与方法

| 项目 | 值 |
|---|---|
| 操作系统 | Windows 11 专业版 |
| CPU | Intel Core i5-13600KF，14 核 / 20 线程 |
| 内存 | 95.8 GiB |
| Rust | `rustc 1.94.1 (e408947bf 2026-03-25)` |
| Python | 3.14.3 |
| Node.js | v24.14.1 |
| 构建 | release，编译时注入 `JX3_BUILD_COMMIT=b887aa3...` |
| 服务 | `127.0.0.1:3016` 隔离测试进程；原 3005 服务未停止 |

基准输入为 2026.04 分山劲绝云宏、300 秒战斗、1220 个宏序列槽、9 个奇穴和 36 个秘籍。每个工具预热 3 次，再串行请求 30 次。计时包含本机 HTTP 往返、完整响应读取、UTF-8 解码和 Python JSON 解析；P50/P95 使用有序样本线性插值。

trace 中的 `data_hash=c772...` 是 Agent 对已加载运行时表做 canonical serialization 后的身份；Golden 报告中的 `data_sha256=2ea...` 是版本数据与脚本文件树哈希。两者用途和覆盖范围不同，不应互相比较。

复现命令：

```powershell
$env:JX3_BUILD_COMMIT = (git rev-parse HEAD).Trim()
Set-Location backend
cargo build --release
python tests/benchmark_agent_tools.py --backend http://127.0.0.1:3016 --warmups 3 --samples 30
```

## 2. 工具性能与响应体积

| 工具 | 模拟次数/调用 | 响应 P50 | HTTP P50 | HTTP P95 | 服务内 P50 | 30 次证据一致 |
|---|---:|---:|---:|---:|---:|---|
| `get_current_scenario` | 0 | 17,124 B | 1.348 ms | 11.910 ms | < 1 ms | 是 |
| `simulate_scenario` | 1 | 3,465 B | 15.100 ms | 25.563 ms | 12 ms | 是 |
| `compare_scenarios` | 2 | 1,636 B | 27.779 ms | 39.722 ms | 24 ms | 是 |
| `analyze_timeline` | 1 | 20,423 B | 14.416 ms | 24.771 ms | 11 ms | 是 |

服务内 `duration_ms` 当前以整数毫秒记录，所以场景捕获和时间线聚合出现 0 ms 代表“小于 1 ms”，不是零成本。`analyze_timeline` 的 HTTP 响应较大，是因为它返回技能聚合、等待证据、GCD gap 和 Buff 区间；仍远小于原始 300 秒完整 timeline。

四个工具在全部 30 次采样中均保持相同 scenario hash、evidence ID 和适用的 fingerprint。`compare_scenarios` 的 P95 最高，但一次完成基线与候选两次 300 秒模拟，仍低于 40 ms；这为后续模型循环预留了远大于工具计算本身的网络与推理预算。

## 3. 成功与拒绝证据

成功 trace：[`agent-traces/p1-07-success.json`](agent-traces/p1-07-success.json)。

- 基线 scenario hash：`fef0b24b904634f9cf06d5399445058e40d3d8207b96c75d5aa5126639257850`；
- 基线 fingerprint：`e43e87d24ff9aed2`，DPS `949319.267`；
- 单变量候选把网络延迟从 0 调到 25 ms，candidate fingerprint 为 `d2291b09efd6a6b4`，DPS 下降 `10923.103`，即 `-1.151%`；
- 时间线 evidence 显式引用同一次模拟 evidence ID；重复模拟 evidence ID 一致；
- 时间线只陈述可观察的等待、gap、资源和 Buff 覆盖，不把相关性直接写成因果。

拒绝 trace：[`agent-traces/p1-07-rejected.json`](agent-traces/p1-07-rejected.json)。候选把已经为 0 的网络延迟再次设为 0，工具在消耗模拟预算前返回 HTTP 422 与稳定错误码 `no_scenario_changes`。这展示系统会拒绝不可比较实验，而不是编造一个“无提升”结论。

提交的成功 trace 是可读投影：保留 engine commit、data hash、scenario hash、evidence ID、关键数值和链路检查，省略大段逐事件数组。evidence ID 仍绑定完整原始工具结果；运行基准脚本可重新导出完整投影。

## 4. 评测先发现的问题

第一次 30 次基准没有通过：战斗 fingerprint 固定，但 `analyze_timeline` 的 evidence ID 偶发变化。根因是模拟器从 HashMap 生成 Buff 轨道；相同 UI 优先级下的轨道顺序不稳定，而时间线证据此前沿用了该顺序。

修复在 Agent 分析层按稳定 `buff_id` 排序，并增加“源 Buff 轨道倒序后 evidence ID 仍相同”的测试。修复没有改伤害公式、事件 fingerprint 或 Golden 数值。这个失败记录保留在报告中，因为它证明评测在模型接入前实际拦截了证据层 silent drift。

## 5. 完整阶段回归

| 验证项 | 结果 |
|---|---|
| Rust | 69/69，0 失败；19 个既有 warning |
| Python fixture/benchmark 测试 | 9/9 |
| Agent 无模型评测 | 20/20；写尝试 0；userdata 不变；运行态已恢复 |
| Versioned Golden v2 | 2025.10 + 2026.04 共 8/8 |
| 普通 smoke | health、首页、模拟通过 |
| Agent HTTP smoke | 5/5，通过场景、模拟、A/B、时间线和预算拒绝 |
| 状态写入守卫 | 0 个脚本直写违规 |
| 前端语法 | `node --check frontend/app.js` 通过 |
| 秘密扫描 | 8 个已解释 provenance/fingerprint 候选，0 个秘密 |

所有工具只读，未新增会话或 trace 持久化，仓库内 userdata 的文件集合和 SHA-256 在评测前后完全一致。测试结束后只关闭 3016 隔离进程，既有 3005 服务保持运行。

## 6. 阶段边界

P1 已证明“模型未来能调用什么、数值从哪里来、证据怎样复验、错误怎样拒绝”。它还没有证明模型会正确规划、选择工具或撰写解释。因此下一阶段需要在正式实现前确认 provider adapter、首批模型、开发预算，以及会话数据的保留与脱敏方案；公网继续关闭。
