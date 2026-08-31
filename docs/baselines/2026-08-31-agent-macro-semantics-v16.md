# Agent 宏语义投影与执行兼容回归（v16）

日期：2026-08-31

## 目标

保留玩家宏的既有直接执行行为，同时让 Agent 读取模拟器解析器生成的真实条件树，而不是按通用编程语言优先级猜测宏语句。

## 实现边界

- 未修改宏执行器的逐行选择逻辑。
- 未修改 `&`、`|` 等优先级且右结合的既有解析规则。
- `get_current_scenario` 新增 `condition_ast`、全括号 `condition_semantics` 和宏运行语义元数据。
- v16 提示词要求把“解析含义”和“运行时实际影响”分开；命中率、被前序语句抢占、DPS 变化仍需时间轴或同场景对比证据。
- 修复 renderer 到 parser 的两个兼容缺口：无方括号 `skill_energy:` 条件识别，以及后续无姿态过滤分页的 `#page` 保留。玩家直接提交宏的链路不经过 renderer。

## 回归结果

- Rust：195/195。
- 宏相关：11/11。
- 固定知识检索：24/24 recall cases，32 条安全断言。
- Phase 1 工具评测：20/20。
- Phase 2 离线模型评测：12/12；证据引用与只读工具边界均为 100%。
- 知识/source-card smoke：通过。
- 真实 `backend/userdata`：隔离验收前后指纹一致。

2026.04 Golden v2 在部署前后保持一致：

| 场景 | fingerprint | DPS | events |
| --- | --- | ---: | ---: |
| 绝云宏 standard | `e43e87d24ff9aed2` | 949319.267 | 1648 |
| 简循环 | `099ddb885b081c61` | 227068.133 | 992 |
| 空序列 | `cbf29ce484222325` | -0.000 | 0 |
| 绝云宏 boss=2s | `d91b95325a935c51` | 920311.410 | 1607 |

## 部署验收

- 本机 `127.0.0.1:3005` 健康检查通过。
- Run/Session/SSE smoke 通过。
- 知识来源卡持久化 smoke 通过。
- 复杂宏服务端投影为 `parse_status=valid`、`associativity=right`，并返回根节点为 `and` 的条件 AST。
- 旧 release 二进制保留为 `backend/target/release/jx3-combat-sim.exe.pre-v16-20260831`；会话目录未迁移或清空。

这组结果证明“Agent 获得了更准确的宏语义输入”，不等于已经证明 Agent 能自动找到最优宏。宏行命中、抢占和候选收益仍需后续运行时观测与多轮实验闭环。
