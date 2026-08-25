# 苍云器灵 · MMO 战斗策划实验平台

> 将《剑网3》苍云职业的战斗规则抽象为确定性模拟环境，并用搜索、强化学习与可解释分析辅助战斗、系统和数值策划决策。

这是我面向 2027 届游戏策划岗位准备的个人作品集项目。项目重点不是复刻一个游戏界面，而是展示如何把复杂战斗规则转化为可以配置、验证、对比和部署的策划工具。

当前已经实现战斗模拟、宏优化、配装搜索和强化学习环境；工具调用 Agent 正处于设计与开发阶段，不在本文中冒充已完成功能。

## 90 秒了解项目

1. 运行 `start.bat`，浏览器访问 `http://localhost:3005`；
2. 选择游戏版本与苍云心法，配置奇穴、秘籍、属性和团队环境；
3. 用手动循环或宏运行确定性模拟，查看 DPS、技能占比和战斗时间轴；
4. 使用宏生成/剪枝/遗传优化、配装搜索或 RL 页面比较不同策略；
5. 通过 fingerprint、golden 和对比工具验证改动是否造成预期外行为漂移。

作品集主线可以概括为：

```text
策划问题 → 规则建模 → 确定性仿真 → 搜索/学习 → 证据解释 → 回归验证
```

## 核心能力

### 战斗规则与数值建模

- 支持 `2025_10_山海源流`、`2026_04_暗影千机` 两套版本数据；
- 支持分山劲与铁骨衣心法、技能、奇穴、秘籍、Buff、团队增益和阵法；
- 使用数据配置承载基础规则，使用版本化 Rust 脚本表达条件机制；
- 共用九阶段伤害链，避免页面、宏、优化器和 RL 各算一套数值；
- 时间轴与 fingerprint 用于解释输出和检测 silent regression。

### 宏与策略优化

- 剑网3风格宏解析和条件求值；
- 宏执行、生成、剪枝和候选规则池；
- 结构化遗传算法与数值参数优化；
- 在表达式、姿态页和宏长度约束内寻找可执行策略，而不只给理论 DPS 上限。

### 配装与系统分析

- 装备、附魔、五彩石与套装效果建模；
- 属性收益、局部替换和 DPS 预览；
- 自动配装、Pareto 筛选与属性拟合；
- 支持将搜索结果送回同一个模拟器复验。

### 强化学习

- 82 维观测、18 个动作槽位的 HTTP-RPC Gym 风格环境；
- 自研 PPO 与行为克隆训练流程；
- 支持训练、checkpoint、rollout 和策略分析；
- 当前重点覆盖分山劲离散技能决策，尚未声称覆盖完整团队战斗或 Boss AI。

### 工程与部署

- Rust + Axum 后端同时提供 API 和静态前端；
- 原生 HTML/CSS/JavaScript 前端；
- Router + per-user worker 的多用户进程隔离；
- 本地、Cloudflare Quick Tunnel、自建 VPS + frp 三种运行路径；
- 真实部署配置、用户数据、模型权重和日志不进入 Git。

## 系统架构

```mermaid
flowchart LR
    UI[原生 Web 前端] --> API[Axum API]
    API --> SIM[确定性战斗模拟器]
    API --> MACRO[宏解析与优化]
    API --> EQUIP[配装与属性搜索]
    API --> RL[RL 环境与训练管理]
    SIM --> DATA[版本化 TOML / JSON 数据]
    SIM --> SCRIPT[版本化 Rust 机制脚本]
    MACRO --> SIM
    EQUIP --> SIM
    RL --> SIM
    SIM --> EVIDENCE[时间轴 / fingerprint / 对比证据]
```

最重要的架构约束是：模拟器是数值事实源。搜索算法、强化学习和未来 Agent 可以提出方案，但最终结果必须由确定性模拟器验证。

## 快速开始

### 环境

- Rust stable toolchain；
- Windows 可直接使用 `start.bat`；
- Python 仅在运行 RL 训练/分析功能时需要，依赖见 `python/requirements.txt`。

### Windows 一键启动

```powershell
.\start.bat
```

默认只监听 `127.0.0.1:3005`。Rust 后端已经托管前端，不需要再启动 8080/8081 的独立静态服务器。

### 手动启动

```powershell
Set-Location backend
cargo run
```

服务启动后访问：

- 页面：`http://localhost:3005`
- 健康检查：`http://localhost:3005/health`

### 本地 smoke

保持服务运行，在另一个终端执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\smoke.ps1
```

smoke 会验证 `/health`、前端 HTML 和一个实际产生技能事件的 5 秒模拟。

## 验证

### Rust 单元测试

```powershell
Set-Location backend
cargo test
```

第 0 阶段记录的基线为 34 项通过、0 失败；当前仍有 17 条编译 warning，计划作为后续工程卫生任务处理。

### 前端语法

```powershell
node --check frontend/app.js
```

### 确定性与 golden

先以 release 模式启动后端，再运行：

```powershell
Set-Location backend
python tests/diff_baseline.py
```

检查包括：

- 同请求重复运行 fingerprint 一致；
- lite/full 的 fingerprint、DPS 和总伤害一致；
- 4 个代表场景与已知 golden 一致。

除非确认规则行为发生了有意变化，否则不要使用 `--update` 覆盖 golden。

当前 Phase 0 结果是确定性 4/4、Lite/Full 等价 4/4、golden 2/4；仓库因此仍处于发布阻断状态。失败详情和性能数据见 [`docs/baselines/2026-08-25-phase0.md`](docs/baselines/2026-08-25-phase0.md)。

## Game × AI 路线

### 已实现

- 强化学习训练环境与 PPO/BC；
- 遗传算法宏优化；
- Pareto 配装搜索；
- 确定性模拟、时间轴和回归证据。

### 下一案例：战斗分析 Agent

Agent 将围绕明确目标自主调用高层工具：

- `get_current_scenario`
- `simulate_scenario`
- `compare_scenarios`
- `analyze_timeline`

它负责拆解问题、提出假设和组织证据，不直接生成“看起来合理”的伤害数字。第一版会先建立 20–30 条离线评测题，再接模型和聊天界面。

### 差异化案例：RL → 宏蒸馏

让 PPO 策略作为教师，Agent 聚类关键决策与失败分歧，提出人类可读的宏规则，再交给现有宏验证器和 GA 复验。目标不是把神经网络伪装成解释，而是量化“理论策略”到“游戏内可执行宏”的性能差距与表达边界。

详细计划见 [`docs/AGENT_PORTFOLIO_PLAN.md`](docs/AGENT_PORTFOLIO_PLAN.md)。

## 项目边界

- 这是玩家研究与策划实验项目，不是官方客户端、插件或战斗服务器；
- Boss 受击、仇恨、位移和完整团队战斗尚未全部建模；
- RL 当前主要研究分山劲离散技能决策；
- 模拟准确性依赖版本数据和机制资料，跨版本结论必须重新验证；
- 公网演示在完成 HTTPS、限流和最小权限检查前不会作为正式服务开放。

## 文档索引

- [`docs/PROJECT_BASELINE.md`](docs/PROJECT_BASELINE.md)：项目模块、规模、技术债和已验证基线；
- [`docs/AGENT_PORTFOLIO_PLAN.md`](docs/AGENT_PORTFOLIO_PLAN.md)：四套 Agent 方案和六周路线；
- [`docs/PHASE_0_PLAN.md`](docs/PHASE_0_PLAN.md)：公开仓库卫生计划；
- [`docs/baselines/2026-08-25-phase0.md`](docs/baselines/2026-08-25-phase0.md)：Agent 接入前的回归与性能锚点；
- [`docs/RELEASE_CHECKLIST.md`](docs/RELEASE_CHECKLIST.md)：公开仓库与 Demo 的发布阻断项；
- [`docs/security/SECRET_SCAN_REPORT.md`](docs/security/SECRET_SCAN_REPORT.md)：脱敏与秘密扫描结果；
- [`backend/PERF.md`](backend/PERF.md)：模拟器性能记录；
- [`部署与运维手册.md`](部署与运维手册.md)：参数化本地/公网部署流程。

## 作品集与责任说明

本仓库用于展示个人在战斗策划、系统策划、数值分析、Game AI 和工程部署方面的实践。项目中的游戏名称、技能名称与相关知识产权归原权利方所有；本项目与游戏官方及其关联公司无隶属或背书关系。

当前尚未添加开源许可证。在许可证明确前，公开可读不等于授权复制、分发或商用。
