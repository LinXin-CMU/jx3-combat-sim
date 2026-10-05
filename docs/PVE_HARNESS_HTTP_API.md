# 苍云 PVE 武学助手 Harness HTTP API

日期：2026-09-21。本文对应 `backend/src/harness/` 当前首版实现，算法标识为 `macro-compile/v1`。它提供“手动技能轴 → 宏候选 → 完整回放 → 首次分歧诊断 → 有界修复”，与旧 AI 助手并行，不调用模型 API，也不创建 Agent 会话。自动配装与循环搜索尚未接入。

## 1. 作用域和任务生命周期

所有路径相对于当前 worker 的服务地址；沿用服务已有认证与用户隔离，不新增凭据协议。任务只保存在当前 worker 内存，按创建顺序保留最近最多 8 个；重启或淘汰后返回 `job_not_found`。下载结果包是客户端导出，不是服务端持久化、检查点或续跑。

每个 worker 同时只接纳一个 Harness 任务，并与现有宏优化、RL 训练/分析/预训练、自动配装重任务互斥。没有排队接口。已接纳任务冻结版本、心法、完整模拟输入和运行时表；之后切换页面或工作区配置不会修改它。断开 HTTP/SSE 或关闭页面不会取消计算。

首版验收范围为 `active_action_identity_timing_resources`：共同窗口内的主动动作身份与顺序、释放时间、档位、引导跳数、释放前怒气/暴怒值/暴怒上限/格挡值。缺少或不适用的资源不按零处理。它不证明所有 Buff、冷却或战斗状态等价，也不证明跨场景稳定性、全局最优或游戏内可用。

## 2. 创建任务

`POST /api/harness/jobs`，JSON 请求体上限 2 MiB。顶层不接受未知字段。

| 字段 | 要求与默认值 |
| --- | --- |
| `version` | 必填，当前 `GameVersion` API 枚举，例如 `AnYingQianJi`；必须等于 worker 当前版本 |
| `mount` | 必填，`FenShanJin` 或 `TieGuYi`；必须等于 worker 当前心法 |
| `simulation` | 必填，完整 `SimulateRequest`；必须是手动技能轴，包含属性与目标 |
| `initial_macro` | 可省略、`null` 或空白；此时从真实目标轴回放生成宏。非空时用它作为首个候选 |
| `max_simulations` | 默认 96，范围 2～256；基线、候选、诊断重放全部计数 |
| `wall_time_ms` | 默认 60000，范围 1000～120000；协作检查的墙钟预算 |
| `max_rounds` | 默认 6，范围 1～12；首个候选在第 0 轮，之后最多执行指定轮数的修复 |
| `max_pages` | 默认 2，范围 1～6；合格候选的宏页数上限 |
| `time_tolerance` | 默认 `0.0625` 秒，范围 0～1；主动动作释放时间允许误差 |

没有 `task`、`objective` 或搜索 seed 顶层字段。当前目标固定为 `faithful_reproduction`；战斗随机参数如 `simulation.dunya_reset_seed` 随完整场景冻结。代码按固定候选顺序执行，但墙钟停止可能受机器负载影响，实验身份相同并不保证每次搜索都能评估同样多的候选。

`simulation.sequence` 需要 1～2048 个操作；操作名不得为空、以 `__` 开头或超过 128 UTF-8 字节。`simulation.macro_text` 必须为空/空白/`null`，不能用宏占位轴作为目标。初始宏最多 8192 UTF-8 字节，需解析通过，最多 16 个解析页、128 条语句；初始宏可以暂不满足最终页数/长度约束，结果须明确标记。

属性须为有限非负数且不超过 `1e9`。目标等级范围 1～200，初始怒气如提供须在 0～100，网络延迟最多 10000 毫秒。停手段最多 32 个，预释放最多 64 个，其时长/位置受到 1200 秒边界约束；其他集合和参数也有上限，完整实现见 `MacroCompileRequestV1::validate`。目标实际回放须无跳过动作；共同窗口必须大于零且不超过 1200 秒，每侧窗口内主动动作不得超过 2048 个。后两项需在任务开始后根据模拟结果检查。

即使是手动技能轴，`simulation.macro_duration` 也可能延长主模拟结算时间，因此只允许省略/`null` 或 0～1200 的有限秒数。`boss_attack_interval` 如提供，须是 `1/16`～1200 的有限秒数，不能使用接近零的间隔生成无界攻击事件。

`simulation.team_buffs` 最多 128 项，以下限制适用于每项（包括未启用项），并在进入模拟前检查：

| 团辅字段 | 上限/范围 |
| --- | --- |
| `key` | 最多 128 UTF-8 字节 |
| `stacks` | 0～1024 的整数 |
| `first_release`、`duration` | 0～1200 的有限秒数 |
| `period` | 0，或 `1/16`～1200 的有限秒数 |
| `release_times` | 可省略/`null`；每项为 0～1200 的有限秒数，每个团辅最多 2048 项 |

所有团辅的 `release_times` 数量合计不得超过 8192。显式排程与周期字段都接受校验，不能因某字段当前被另一种排程覆盖而绕过上限。

应从工作区捕获完整环境，保留装备效果、团辅、阵法、预释放、属性、目标、奇穴、秘籍、延迟、资源与停手设置。服务端有默认值不表示省略后的环境与用户原场景相同。如下是独立测试场景，不是推荐实战配装：

```json
{
  "version": "AnYingQianJi",
  "mount": "FenShanJin",
  "simulation": {
    "haste_level": 10492,
    "sequence": ["盾刀", "盾刀", "盾刀", "盾刀", "盾刀", "盾刀"],
    "attributes": {
      "vitality": 202700, "li_dao": 44, "gen_gu": 44, "yuan_qi": 44,
      "shen_fa": 16706, "base_attack": 52873, "base_magical_attack": 0,
      "weapon_damage": 10986, "surplus_value": 10448, "crit_level": 67747,
      "crit_effect_level": 3419, "overcome_level": 97462,
      "strain_level": 85868, "haste_level": 10492,
      "parry_value": 16706, "parry_level": 5599
    },
    "target": {"level": 134, "defense_bonus": 0, "damage_cof": 0},
    "talents": [], "recipes": [], "equipment": {},
    "channel_ticks": {}, "timing_offsets": {}, "qijin_buffs": {},
    "network_delay": 0, "initial_rage": 50, "dunya_reset_seed": 42,
    "macro_text": null, "macro_duration": null, "pauses": [],
    "boss_attack_interval": 2, "hanjia_expectation": false,
    "tiegu_mode": 2, "experimental": false,
    "lite": false, "lite_keep_timeline": false,
    "team_buffs": [], "formation": null, "pre_releases": []
  },
  "initial_macro": "/cast [rage>100] 盾刀",
  "max_simulations": 32,
  "wall_time_ms": 60000,
  "max_rounds": 3,
  "max_pages": 2,
  "time_tolerance": 0.0625
}
```

接纳成功返回 HTTP `202`，schema 为 `harness-job-created/v1`，包含 `job_id`、`scenario_hash`、`experiment_hash`、初始 `status: "running"` 和四个相对地址：`status_url`、`events_url`、`cancel_url`、`artifacts_url`。接纳不代表模板合法或候选合格；后续应查询终态。

三种身份用途不同：

- `scenario_hash`：规范化版本、心法及完整模拟输入；`lite` 输出选项不改变它。
- `runtime_hash`：冻结技能/奇穴/秘籍/团辅/阵法/常量、版本/心法和运行可执行文件摘要。它在结果包中提供，不是未来装备搜索池的通用身份。
- `experiment_hash`：算法版本、固定目标、规范化请求、场景与运行环境身份。改变预算或起始宏也会改变它。读取可执行文件摘要失败时 `engine.executable_hash` 为 `unknown`，此时不能据其断言构建一致。

## 3. 查询、状态与验收

`GET /api/harness/jobs` 返回 `{"schema_version":"harness-jobs/v1","jobs":[...]}`，按新到旧列出摘要。列表中的 `progress`、`result` 均为 `null`，应进一步查询单个任务。

`GET /api/harness/jobs/:id` 返回 `harness-job/v1`：

| 字段 | 含义 |
| --- | --- |
| `job_id`、`scenario_hash`、`experiment_hash` | 当前任务及实验身份 |
| `sequence` | 状态更新序号，用于识别较新快照 |
| `status`、`running` | `running`，或终态 `completed` / `cancelled` / `budget_exhausted` / `failed` |
| `phase`、`message` | 当前阶段和展示文本；运行阶段包括 `baseline`、`replay`、`candidate`、`repair`、`finished` |
| `simulations`、`max_simulations`、`elapsed_ms` | 已发起模拟数量、次数预算和任务耗时；正在执行的一次模拟在开始前计数 |
| `cancellation_requested` | 是否已受理取消，不能据此判定已停止 |
| `progress` | 最近 `CompileProgress`：`phase`、`message`、`simulations`、`best` |
| `result` | 正常收束后的 `pve-macro-compile/v1` 结果；运行中通常为 `null` |
| `error` | 任务失败详情，形如 `{"code":"compile_failed","message":"..."}` |

正常收束结果含 `stop_reason`、预算用量、`rounds`、`acceptance_scope`、`window_seconds`、`baseline`、`baseline_fingerprint`、`best`、`history`、`failed_candidates` 和 `limitations`。当前停止原因是：

| `stop_reason` | 终态与含义 |
| --- | --- |
| `target_reproduced` | `completed`：本场景验收范围内通过 |
| `no_improvement` | `completed`：局部搜索没有继续改善；不等于还原成功 |
| `budget_exhausted` | `budget_exhausted`：模拟次数用尽 |
| `time_budget` | `budget_exhausted`：检测到墙钟预算到期 |
| `max_rounds` | `budget_exhausted`：达到修复轮数上限 |
| `cancelled` | `cancelled`：用户取消；已有结果可保留 |

`best` 可以为 `null`，例如基线后取消且尚未验证候选。候选包含宏正文、分页长度、`verified`、`full_snapshots`、`reproduced`、动作对齐、首次分歧、动作索引视图、可选诊断、模拟指标、指纹和来源。`verified=true` 仅说明执行过完整模拟；`reproduced=true` 还要求页面约束通过、窗口内完整状态快照、无跳过动作且 missing/extra/changed 都为零。候选指纹和基线指纹是十进制字符串，避免浏览器丢失 `u64` 精度。

窗口为 `max(基线 fight_time, 末次目标主动释放时间 + 1/16)`，对齐使用 `0 <= cast_time < window_seconds`，以免遗漏目标末次释放。先最大化同技能配对数量，同分再最小化时间距离；月照/雁门与雾海保留独立身份。`alignment.rows` 的 `reference_index`、`actual_index` 指向原始完整 timeline，`summary.first_difference` 则是 rows 索引。`summary.time_error` 为所有配对的绝对时间差总和，容差内差值仍计入。

`changed` 包括名称档位、skill_id、双方均已记录的引导跳数、资源或超过容差的时间变化。`same` 只表示这些已定义字段通过。分页长度按 UTF-16 单元计数，含页内换行、不含 `#page` 标记，去除正文末尾空白，每页上限 128；这与现有编辑器一致，不替代游戏内人工确认。

模拟器允许省略条件方括号的历史文本，Harness 会将起始宏和全部候选规范为 `[条件]` 形式，保留注释与页分隔；规范化之后再去重、回放和计算长度。返回的 `best.macro_text` 就是被测试的文本，补括号后超长的页不能通过约束检查。

`baseline` 与 `best.metrics` 中的 DPS、总伤害、fight_time 是各自完整模拟的观测值，结算尾段可能不同；首版没有把它们作为同窗口收益证明，也不以 DPS 排序覆盖轴还原目标。诊断失败或某候选无法验收会保留已测试最好结果，并在 `limitations` / `failed_candidates` 反映边界；不是每个历史候选都包含完整文本或完整时间轴。

现有引擎的 `pauses` 只作用于宏执行，手动轴不消费该停手调度。Harness 保留完整字段并在这类结果的 `limitations` 提示；由此产生的时间或动作差异仍按实际回放报告，不视作已还原。

## 4. SSE 和取消

`GET /api/harness/jobs/:id/events` 返回 `text/event-stream`，建立连接时立即发送最新状态，之后推送更新，约每 10 秒发送保活。

```text
id: 7
event: progress
data: {"schema_version":"harness-job/v1", ...完整 JobStatus...}

id: 8
event: completed
data: {"schema_version":"harness-job/v1", ...完整终态 JobStatus...}
```

以上 `data` 是结构示意。实际每条 `data` 都是完整 JSON 状态快照；终态事件统一叫 `completed`，包括取消、预算停止和失败，必须读取 `data.status`。终态发出后流结束。该流使用最新值通道，慢客户端可能跳过中间快照；`Last-Event-ID` 不触发历史补发，重连直接得到当前快照，已经结束的任务也会立即发出终态。

`POST /api/harness/jobs/:id/cancel` 无业务请求体，发送空对象亦可。返回 HTTP `200`：

```json
{"job_id":"harness-...","accepted":true,"already_terminal":false}
```

重复取消运行任务仍返回 `accepted=true`；已经结束时为 `accepted=false, already_terminal=true`。取消在模拟/候选生成阶段之间检查，不强行中止正在执行的一次计算；继续查询，直到 `running=false`。取消受理后、真正结束前仍占用任务名额。已验证候选可以保留，尚未完成的候选不会被标为验证通过。

## 5. 错误与计算互斥

HTTP 层失败使用 `harness-error/v1`：

```json
{"schema_version":"harness-error/v1","error":{"code":"runtime_mismatch","message":"版本或心法已变化，请重新读取当前技能轴。"}}
```

| HTTP | `error.code` | 含义 |
| --- | --- | --- |
| 400 | `invalid_request` | JSON/字段/输入上限/初始宏解析不合法；请求体读取拒绝也归此类 |
| 400 | `invalid_scenario` | 不能建立合法完整场景快照 |
| 404 | `job_not_found` | ID 不存在、已被淘汰或 worker 已重启 |
| 409 | `runtime_mismatch` | 提交版本/心法与 worker 当前状态不一致 |
| 409 | `job_conflict` | 当前 worker 已有运行中的 Harness 任务 |
| 409 | `compute_busy` | 与旧重任务冲突，或旧重任务启动被运行中的 Harness 拒绝 |
| 500 | `runtime_identity_failed` / `experiment_identity_failed` | 建立运行环境/实验身份失败 |

任务被接纳后才发现的模板跳过动作、窗口超限等计算错误，通过查询接口的 `status=failed` 和 `error.code=compile_failed` 表示；查询本身仍返回 HTTP `200`。取消、预算耗尽也不是 HTTP 请求错误。

互斥的旧启动入口为 `/api/optimizer/start`、`/api/rl/train/start`、`/api/rl/analyze/start`、`/api/rl/pretrain/start`、`/api/equip/auto_optimize`。这些入口既检查 Harness，也检查正在执行的旧重任务，因而旧重任务之间也互斥，包括防止自动配装搜索重复进入；各入口共用接纳锁，避免“检查空闲后同时启动”。这不是整个服务的全局锁：普通模拟、只读查询、旧 AI 页面等仍保留。

## 6. 导出与可执行复跑

`GET /api/harness/jobs/:id/artifacts` 返回 `harness-artifacts/v1`，包含：

- `job_id`、任务 `status`、场景/实验/运行环境哈希；
- 原始 `request`、规范化 `scenario`、`engine`（算法、旧 provenance、可执行文件摘要）；
- `result`、`last_progress`、`error`；
- `scope: "current_frozen_scenario"`、`storage: "worker_memory"`、`game_verified: false`。

运行中下载的是当时快照，`result` 可能仍为空；完整交付应在终态下载。结果包携带用户场景，应由用户决定分享范围。它未内嵌模拟器可执行文件或数据文件，也没有自动还原旧构建；复跑前应使用对应构建与数据。

下列 Python 代码只依赖标准库，可保存为临时脚本后执行 `python replay_harness.py 结果包.json http://127.0.0.1:3028`。目标应是隔离测试 worker。代码重放候选，核对字符串指纹与指标；版本切换仅用 `persist:false`，结束恢复原状态，不创建 Agent 会话或改写已保存宏。

```python
import copy
import json
import math
from pathlib import Path
import sys
import urllib.request

artifact = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
base_url = sys.argv[2].rstrip("/")

def request(path, payload=None):
    encoded = None if payload is None else json.dumps(
        payload, ensure_ascii=False, allow_nan=False
    ).encode("utf-8")
    req = urllib.request.Request(
        base_url + path, data=encoded,
        headers={"Content-Type": "application/json; charset=utf-8"},
        method="GET" if encoded is None else "POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return json.load(response)

result = artifact.get("result")
assert result and result.get("best"), "需要含已验证候选的终态结果包"
best = result["best"]
assert best["verified"]
assert not any(job["running"] for job in request("/api/harness/jobs")["jobs"])
original = request("/api/mounts/current")
try:
    changed = request("/api/mounts/switch", {
        "version": artifact["request"]["version"],
        "mount": artifact["request"]["mount"], "persist": False,
    })
    assert changed["ok"]
    simulation = copy.deepcopy(artifact["scenario"]["simulation"])
    duration = result["window_seconds"]
    simulation.update(
        sequence=["__macro__"] * (math.ceil(duration / 0.25) + 20),
        macro_text=best["macro_text"], macro_duration=duration,
        channel_ticks={}, timing_offsets={}, qijin_buffs={},
        lite=False, lite_keep_timeline=False,
    )
    actual = request("/api/simulate", simulation)
    assert str(actual["fingerprint"]) == best["fingerprint"], "回放指纹不同"
    for field in ("dps", "total_damage", "fight_time"):
        assert math.isclose(actual[field], best["metrics"][field],
                            rel_tol=1e-12, abs_tol=1e-8), field
    print("候选回放指纹与数值一致；不代表完整构建身份或泛化能力相同。")
finally:
    restored = request("/api/mounts/switch", {
        "version": original["version"], "mount": original["mount"],
        "persist": False,
    })
    assert restored["ok"], "恢复原运行态失败"
```

如果要重新执行整个搜索，将结果包的 `request` 原样提交到创建接口；这会产生新的 Job ID，不是续跑。创建前切到请求版本/心法，比较新旧 `experiment_hash` 与结果包 `runtime_hash`；身份不一致时先检查构建、数据和参数，不把不同实验当成同一基线。即使身份一致，也应分别记录墙钟预算下实际模拟数量及停止原因。

## 7. 验证入口

从仓库根目录可运行以下命令。HTTP 脚本要求已启动隔离 worker；文档列出可复跑入口，不声明当前构建已经通过。

```powershell
node tools/harness-alignment-test.js
python tools/harness-smoke.py --base-url http://127.0.0.1:3028
```

Rust 回归位于 `backend/tests/harness/`，由模块测试引用；可在 `backend` 目录执行 `cargo test harness::`。HTTP smoke 涵盖主模拟合法轴、生成/修复、实验身份、artifact 回放、终态 SSE 重连、错误码、事件密度/时长/团辅嵌套排程拒绝、预算、取消及旧任务冲突；始终以 `persist:false` 切换并在 `finally` 恢复运行态，只对自身创建的 Harness 任务做清理。危险输入的拒绝测试额外携带不匹配版本：正确实现先返回 `invalid_request`，缺少对应上限的旧实现也会在版本检查处停止，不进入模拟。测试结果与发布构建记录由本次基线补充。
