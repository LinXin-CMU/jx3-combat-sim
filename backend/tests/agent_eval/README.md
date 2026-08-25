# P1 Agent 离线评测集

这套评测先验收工具、证据与安全边界，不评价模型文风。正式接入模型后沿用同一批问题，增加回答级评分器，而不改写底层事实期望。

## 组成

- 4 题事实读取；
- 6 题单变量 A/B；
- 6 题时间线诊断；
- 2 题缺字段或不可比较；
- 2 题越权请求。

每题必须包含：

```text
scenario
question
allowed_tools
expected_evidence
forbidden_claims
pass_rule
```

`expected_evidence` 描述应出现的证据，不保存一段预设回答。`pass_rule.assertions` 只验证强类型字段、关系、错误码和 provenance；涉及数值的成功题必须同时断言 fingerprint。

越权题在无模型阶段验证当前工具集合没有写能力、runner 不发出工具请求且 `write_attempts=0`。接入模型后再对实际回答是否拒绝越权、是否出现 forbidden claim 评分。

## 文件

- `scenarios.json`：完整、可复现的模拟输入及其版本/心法；
- `cases.json`：20 道问题、允许工具、证据期望、禁止结论与机器 pass rule；
- `../run_agent_eval.py`：无模型 HTTP runner。

## 运行

先在隔离端口启动当前后端，再执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\agent-eval.ps1 -BaseUrl http://127.0.0.1:3016
```

也可以在 `backend` 目录直接运行 `python tests/run_agent_eval.py --backend http://127.0.0.1:3016`。

成功摘要：

```text
fixtures=20/20 write_attempts=0 userdata_unchanged=true runtime_restored=true
[OK] Agent model-free evaluation passed.
```

runner 会用 `persist=false` 切换 fixture 所需版本，并在结束时恢复运行前的版本/心法；同时对仓库内 userdata 做运行前后 SHA-256 对比。
