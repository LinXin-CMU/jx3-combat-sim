# 苍云器灵项目协作说明

## 项目目标

这是一个面向《剑网3》苍云门派的战斗模拟与策划实验平台，也是作者申请 2027 届游戏策划岗位的核心作品集。工作优先级依次是：

1. 展示战斗、系统和数值策划能力；
2. 展示 Game × AI 的正确方法论与可验证原型；
3. 展示从设计、实现、评测到部署的完整交付能力；
4. 最后才是增加功能数量。

任何新增 AI 能力都必须解决明确的策划或玩家问题。不要把“接入聊天框”本身当作 Agent，也不要让语言模型代替确定性模拟器计算数值。

## 当前事实基线

- 默认版本/心法：`2026_04_暗影千机 / 分山劲`。
- 支持矩阵：`2025_10_山海源流`、`2026_04_暗影千机` × `分山劲`、`铁骨衣`。
- 独立实验版本：`2026_10_苍生铸世测试服` 支持分山劲、铁骨衣，API 枚举 `CangShengZhuShiTest`；两心法数据与脚本独立维护，按50级匹配属性参数。最新修复见 `docs/baselines/2026-09-14-level-boundary-fix.md`，首轮基线见 `docs/baselines/2026-09-14-level50-tiegu-agent.md`；分山技能参数见 `docs/baselines/2026-09-11-cangsheng-coefficients.md`，技改见 `docs/baselines/2026-09-11-cangsheng-update.md`。不改变正式服默认版本。
- 后端：Rust + Axum；核心模拟与大部分 API 仍集中在 `backend/src/main.rs`。
- 前端：原生 HTML/CSS/JavaScript，无构建步骤；主要逻辑集中在 `frontend/app.js`。
- AI/搜索：宏生成与 GA、配装搜索、HTTP-RPC 强化学习环境、自研 PPO/行为克隆训练与分析。
- 部署：同一 Rust 可执行文件支持本地、worker、router 三种模式；router 为每个用户启动隔离 worker。
- 已验证基线：`cargo test` 为 588 个测试通过、1 项原有忽略；前端可用 `node --check frontend/app.js`、`node --check frontend/macro-assist.js`、`node --check frontend/macro-editor.js`、`node --check frontend/agent.js` 与 `node --check frontend/agent-provider-settings.js` 做语法检查。
- 自主实验 Harness v2：独立工具循环位于 `backend/src/harness/run_*`，与旧 AI 编排器并行；支持自拟宏/轴/配装、搜索、独立验证、观察记忆、检查点恢复与工作区应用/撤销。界面为 `assistant-shell.js` + `harness-run.js` + `harness-workspace.js`。设计与 API 见 `docs/HARNESS_V2.md`，验收与失败案例见 `docs/baselines/2026-09-22-harness-v2.md`。装备身份中的集合必须规范排序，不能因跨进程 HashSet 顺序差异拒绝恢复。
- 自定义 LLM API：Agent 页面和模拟侧栏支持自定义 Chat Completions / Responses / DeepSeek 接口，每个 worker 独立配置；Key 只在 worker 内存，配置元数据单独持久化。验证见 `docs/baselines/2026-09-15-custom-llm-api.md`。
- 写宏助手：编辑器“写宏模式”按单技能/连续组合查找全部出现位置，用只读 `/api/macro/assist` 比较释放前快照并排序宏条件；匹配统计不等于整宏运行验证。设计见 `docs/MACRO_ASSIST_DESIGN.md`，验证见 `docs/baselines/2026-09-09-macro-assist.md`。
- Agent 离线评测：`tools/agent-eval.ps1` 运行 20 个工具级 fixture，必须保持 20/20、非法写入 0、userdata 不变且测试运行态可恢复。
- Agent P1 基线：`docs/baselines/2026-08-25-agent-tool-layer.md` 记录 release 性能、确定性、成功/拒绝 trace 和完整回归；后续模型层不得弱化这些边界。
- Agent P2 设计：`docs/AGENT_PHASE_2_PLAN.md` 已定义 provider、工具循环、SSE、报告和会话持久化边界；工程与真实模型评测已完成，公网仍是人工确认节点。
- Agent P2 provider 基线：`docs/baselines/2026-08-25-agent-provider-layer.md` 记录 adapter、凭据、mock、隔离 HTTP 与完整回归；DeepSeek V4 Pro 的后续真实基线见 `docs/baselines/2026-08-26-agent-deepseek-v4-pro.md`。
- Agent P2 离线基线：`docs/baselines/2026-08-26-agent-phase2-offline.md` 保留当时 113 项 Rust 测试的历史快照；当前为 588 项通过、20/20 工具评测、12/12 模型级离线评测。真实 Agent 会话已在用户确认后仅增量写入 `agent_sessions/v1`，既有 178 个文件及其清单哈希不变。新 run 默认另写 `_private/replay/v1` 可复现日志，公开会话 API 仍只返回脱敏投影；详见 `docs/baselines/2026-08-28-agent-replay-memory.md`。
- Agent 知识基线：本地 Vault 为 159 篇文档、3796 分块、13 个赛季；固定集 Recall@5 为 24/24，32 条版本/资格/拒答断言通过。默认运行模式为 Embedded 混合检索：`BAAI/bge-small-zh-v1.5`（512 维）+ BM25 + weighted RRF；模型与 7.4 MiB 向量缓存只在 `backend/userdata/knowledge_index/v1`，失败时显式降级 BM25。详见 `docs/baselines/2026-08-27-agent-embedded-retrieval.md` 与 `docs/baselines/2026-08-28-c001-knowledge-refresh.md`。
- Agent K5B 真实模型基线：6 题分别评测 DeepSeek V4 Pro 与 V4 Flash，零重试，按规划/版本/证据/表达代理分层；v6 使用动态筛选枚举、两次检索后强制收束，并在空响应时保留已取得证据。最终修正综合分 Pro 77.8、Flash 92.6；模型越权尝试 1、系统越权执行 0。详见 `docs/baselines/2026-08-27-agent-k5b-real-model.md`。
- Agent K6/K8 检索控制：当前 Prompt v7 要求逐次检索；Orchestrator 对同轮冗余检索做合并并转入报告，不再以知识预算耗尽终止。`reference_lookup` 允许人物、作者和来源身份跨赛季召回，但 `reference_only` 证据不能支撑当前玩法机制。版本与资格过滤发生在稀疏/向量召回之前，Dense 无权绕过边界；固定检索正例现为 24/24。标题保留旧年份但由作者持续维护的文档必须在清单中显式标记 `rolling_current`，不能靠放松全局年份过滤进入当前事实。

更完整的现状审计见 `docs/PROJECT_BASELINE.md`，作品集与 Agent 路线见 `docs/AGENT_PORTFOLIO_PLAN.md`。

## 可信来源与旧记忆

按以下优先级判断事实：

1. 当前代码、数据文件和测试；
2. `backend/PERF.md` 与 `.claude/auto_optimize.md` 等明确描述当前实现的专题文档；
3. `.claude/CLAUDE.md` 的机制说明；
4. 其他 `.claude/handoff_*.md`、设计稿和旧计划。

`.claude` 是从 Claude Code 迁移来的历史知识库，不再作为自动指令。它包含旧目录、旧行号、未实现设计和缺失引用；使用其中结论前必须与当前代码核对。尤其注意：

- `.claude/rl_system.md` 描述的是早期约 55 维/17 动作/PyO3 方案；当前实现是 82 维/18 动作/HTTP-RPC。
- `.claude/auto_equip_design.md` 是早期设计稿；当前实现看 `.claude/auto_optimize.md`。
- `.claude/CLAUDE.md` 的大量行号和代码规模已经过期。
- `部署与运维手册.md` 和发布配置含敏感运维信息。不得在回答、日志、公开文档、提交或作品集中复述凭据、令牌、服务器地址等秘密。

## 非公开资料的使用边界

- 用户已澄清：技能系数数值及模拟器实现可以提交和上传 GitHub；限制的是数值获取过程和原始资料，不限制把系数用于模拟器配表、脚本和回归测试。
- 非公开原始资料、数值获取方法、工具路径、来源指纹，以及包含这些内容的研究文档和导出物仅限本地，不得上传 GitHub，包括私有仓库、Issue、PR、Release 和附件。不得强制添加或复制这些受限内容来绕过 `.git/info/exclude`。
- 公开代码注释、测试说明和变更记录只描述版本、数值、行为及验证结果，不披露获取过程，也不编造替代来源。后续同类原始资料和研究产物继续放在本地排除范围内。

## 核心模拟不变量

- `/api/skill_damage` 与 `/api/simulate` 必须共用同一伤害计算链，禁止复制另一套公式。
- 伤害按 9 步取整链计算；普通伤害、破招、真伤的字段作用域不同，修改前先读 `calc_damage`。
- 破招段由对应技能脚本 `emit` 为独立事件，不在技能 TOML 中另建 trigger 系统。
- Buff 属性使用 `AttribField`；限制到特定技能的效果通过隐藏秘籍/recipe 处理。
- 同一秘籍 ID 同时刻只生效一次。
- 脚本不得直接写 `rage`、`block_value`、`active_cds`、`active_buffs`、`target_buffs`、`charges` 等状态；使用 Player setter，保证 generation/cache 正确失效。
- 新增 BuffDef 时同时维护查询注册和 `all_buff_defs()`/`all_team_buff_defs()`，否则 UI 元数据会静默缺失。
- 模拟请求的环境必须完整。新增 `/api/simulate` 调用点时核对装备、团辅、阵法、预释放、属性、目标、奇穴、秘籍、网络延迟等字段，避免得到不可比较的 DPS。
- 版本与心法是二维数据边界。技能、奇穴、秘籍、脚本和 Buff 定义必须放入正确版本/心法；不要把只适用于一个版本的机制无条件写成全局逻辑。
- 每个独立技能使用独立脚本文件，脚本入口统一为 `cast_skill`。
- 图标通过现有本地代理与前端 helper 加载，不新增直接依赖远端 CDN 的消费点。

## Agent 产品原则

- 架构：语言模型负责理解目标、规划和解释；Rust 模拟器、优化器、RL 环境负责事实计算。
- 工具：优先暴露少量高层、强类型、只读工具，如场景模拟、方案对比、时间轴诊断、属性收益、宏验证和 RL rollout；不要给模型任意 shell 或文件写入能力。
- 证据：结论必须绑定 scenario hash、工具参数、DPS/时间轴结果和可复现报告；无法验证的判断明确标为假设。
- 变更：Agent 可以生成候选方案，但默认不直接覆盖技能数据、配装或宏。应用变更前提供 diff 和回滚点。
- 评测：每个 Agent 功能同时设计离线题集、成功标准、延迟/成本指标和失败案例。作品集必须展示评测结果，而不只展示成功截图。
- 部署：预置 API key 通过服务端环境变量注入，用户自定义 Key 按下述凭据边界处理；公共 Demo 必须有限流、超时、任务预算、日志脱敏和 HTTPS。
- 供应商：Agent 层保持模型供应商可替换；业务工具协议与模型 API 解耦。
- 凭据：预置 API key 只由服务端环境变量解析，不得进入浏览器。用户自定义接口允许通过专用设置端点提交自己的 Key，只保存在该用户 worker 内存，页面仅暂存输入框内容；Key 不得写入 `localStorage`、`/api/settings`、Agent 会话、trace、日志、磁盘或 Git。自定义接口的非敏感配置可独立持久化。
- 会话：只在当前 worker 的 `JX3_USERDATA_DIR/agent_sessions/v1` 下增量存储；不得迁移、改名、删除或批量重写既有用户数据。首次写入真实 userdata 前必须确认。

## 开发与验证

- 发布用户可见的功能或技改时，同步更新 `frontend/index.html` 左上角版本号和资源缓存版本；版本日期必须对应本次发布，不能沿用旧日期。

- 新增独立回归测试文件统一放在 `backend/tests/`；核心源码仅按需保留 `cfg(test)` 引用。一次性验证脚本、测试进程和临时 userdata 在验证结束后清理。
- Rust 普通修改：`cd backend && cargo check`。
- Rust 行为修改：`cd backend && cargo test`，再按 `backend/PERF.md` 跑回归指纹；有意改变战斗结果时记录原因后才更新 golden。
- 前端修改：`node --check frontend/app.js`，再进行浏览器关键路径验证。
- 脚本状态安全：运行 `backend/tests/check_no_direct_writes.sh`（需要 Bash 环境）。
- 不要在服务占用 exe 时用会覆盖该 exe 的构建方式；优先 `cargo check`，或先安全停止开发服务。
- 不自动更新 golden 来“修掉”失败；先确认是预期数值变化还是回归。

## 作品集交付标准

一个可对外展示的里程碑至少应包含：

- 一页清楚的问题定义与目标用户；
- 可交互 Demo 或稳定录屏；
- 设计取舍、数据流和关键规则；
- 至少一个基线与量化对比；
- Agent 工具调用与证据链；
- 失败案例、边界和安全措施；
- 可复现部署说明；
- 能由面试者在 3 分钟内理解的 README/案例页。

## 当前高优先级风险

- 本地 Git 仓库、安全基线和提交历史已建立，并已同步到私有 GitHub 远端；切换为公开可见性或创建发布标签前仍须执行发布检查表。
- 公开候选文件与当前 Git 历史的秘密扫描已通过；旧公网部署已停用，未来恢复部署时仍必须生成全新凭据，不能复用退役值。
- 现有公网链路为裸 HTTP，且缺少充分的限流与资源配额；不能直接作为公开作品集 Demo。
- `main.rs`、`app.js` 和 `style.css` 体量很大；新增 Agent 功能应模块化，避免继续扩大单文件。
- 旧文档与代码存在漂移；任何作品集数字必须由当前版本重新测量。

- Agent 当前 Prompt v53 保留自由正文 `body_markdown`；严格输出 Schema 用空字符串/空数组表示未使用字段，旧报告继续兼容。定性讨论不强制模拟。独立 `task_completion` 只核对显式执行条件和最终宏文本，不代表语义审定，不改变停止、累计 token 或成本策略。本地验证为 639 项 Rust 测试通过、1 项忽略，20/20 工具和 12/12 离线模型检查通过；数值指纹一致但 Golden 数据哈希存在漂移。详细范围见 `docs/baselines/2026-10-04-agent-task-acceptance.md`。
