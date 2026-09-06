# v40：当前场景的机制上下文

## 交付范围

按用户要求先写实现，暂不开展模型评测。本次只改 Agent 的上下文供给及历史读取投影；保持自主工具选择、任务路由、战斗计算、既有会话文件与部署状态不变。

- `backend/src/agent/mechanics.rs`：生成 `agent-mechanics-context/v1`。技能名、已选奇穴描述、秘籍名称取自本轮冻结运行表。
- `get_current_scenario.result.mechanics_context`：将解释上下文登记在场景 Evidence 中，保留版本、心法、来源和可复现身份。
- 当前正式服暗影千机分山劲提供源码核对的盾刀切换、流血、绝刀计费档位、狂绝返还、援戈、血怒、嗜血与麟光关系；按已选奇穴/秘籍展开。其他版本、心法及实验模式只带对应运行表与通用观测定义。
- 观测定义区分 `rage_cost`、`rage_spent`、回复/返还、触顶与实际溢出；`coverage_percent` 已为百分数，`average_stacks_while_active` 的分母为 Buff 生效时间。
- 每次模型请求从当前场景 Evidence 重建独立的 `mechanics_context` 消息。首轮、继续分析、上下文交接、报告修复均携带同一完整对象；普通工具投影省去重复副本。仍计入原有请求总字节限制。
- 历史会话读取投影增加历史解释标识、run/scenario/prompt 身份及历史引用 ID。压缩包装保留其用途。既有 userdata 文件未改写。
- Prompt v40 只增加一段上下文使用说明，保持 v39 的自主判断与按需工具循环。

## 来源与发现的差异

摘要依据当前 `backend/src/scripts/v2026_04_AnYingQianJi/`、版本化 TOML 与本地已同步的[当前白皮书](https://www.yuque.com/sgyxy/cangyun/whitepaper-23)。来源别名在上下文的 `sources` 中解析为仓库相对路径或原始链接。

白皮书对应章节：1.1.3（绝刀/斩刀）、1.1.4（盾飞/盾回）、1.1.5（血怒）、1.2.1（绝返、援戈、业火麟光、惊涌、嗜血），3.2（援戈分配）。本次没有重新抓取攻略。

核对中发现的差异放入 `known_differences`，不据此自动改变模拟器：

1. 惊涌：白皮书描述血怒期间的最高档绝刀触发额外伤害；当前 `skills/jue_dao.rs` 检查奇穴与最高档，未检查血怒 Buff。
2. 自然盾回：白皮书区分主动/自然盾回的保护冷却；`buffs/buff_dun_fei.rs` 自然结束也添加 1 秒保护。
3. 援戈：本地白皮书存在血怒期间增加 50% 与 100% 的不同段落；`skills/yuan_ge.rs` 与秘籍 99230 使用 50%。
4. 嗜血：白皮书把额外招式增伤限定于最高怒气档；当前隐藏秘籍 99240 按绝刀技能 ID 生效，未按档位筛选。

这些是源码/资料差异记录，游戏规则的最终修订仍需另行核对，不把实现值直接当成统一攻略事实。

## 验证与部署状态

- 只做 `cargo check --tests` 类型检查（包括测试代码编译检查），成功；未执行测试用例、HTTP 评测或付费模型调用。
- 不宣称已消除玩法细节错误；真实效果留待后续直接使用反馈。
- 2026-09-05 编写完成时未构建发布包或替换 3005 的运行程序；后续部署记录见下节。

## 2026-09-06 本地部署

- `cargo build --release --target-dir target/agent-routing-build` 成功，未运行模型评测。
- 发布 EXE SHA-256：`5A711DEBC2C98AE4146D69F715BDAC7B4908BBFA1286307CDDE04174A3086BE3`；复制到 `backend/target/release/jx3-combat-sim.exe` 后核对一致。
- 原 release EXE 保留于 `backend/target/deployment-backups/v40-20260906-070428/jx3-combat-sim.exe`。
- 校验原进程身份后停止旧 release/debug 两个 3005 监听进程，通过现有 `tools/start-agent-secure.ps1` 启动。部署时新 PID 为 33180，监听仅 `127.0.0.1:3005`。
- 保持原 provider 配置、User 环境凭据、`backend/userdata` 和原知识 Vault。Pro/Flash 的安全配置列表均显示可用；本次未发起模型请求。
- 本地向量缓存因身份不匹配重建，最终加载 159 篇文档、3027 个分块，实际检索模式为 `hybrid_rrf`，缓存状态 `built`。
- `/health` 返回 OK；只读场景接口返回 `agent-mechanics-context/v1`，版本为暗影千机、心法为分山劲。未创建 Agent 会话、未执行模拟或改写配装/宏。
- userdata 排除 `knowledge_index`、`icon_cache` 后共 27,559 个文件，排序相对路径与逐文件 SHA-256 的聚合校验值前后同为 `668006C627E27A359FAAF1DAD0A888C1D8260469BD0D19519B59B06B79C2E8EC`。

## 设计参考

通过 OpenAI Docs 核对领域事实作为模型上下文的做法：[Agent definitions](https://developers.openai.com/api/docs/guides/agents/define-agents)。本项目采用独立的场景解释消息与按需知识检索，不引入新的预设任务路径。
