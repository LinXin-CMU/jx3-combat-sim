# 宏合成 Python 性能诊断

`tools/profile-exact-macro.py` 是按需运行的本地 VizTracer 工具，支持第一阶段提取、第二阶段压缩、完整流水线。正常 worker 不导入 VizTracer。复用既有 runner 和真实 Rust 模拟器，不更换验收标准，不读取参照宏答案。

## 安装与运行

在仓库根目录执行；沿用已有安装 Z3 的 Python 环境：

```powershell
.venv/exact-macro/Scripts/python.exe -m pip install -r tools/requirements-exact-macro-profile.txt

# 压缩：baseline 内须有 compression-contract.json、atoms.json、actions.json、exact.txt。
.venv/exact-macro/Scripts/python.exe tools/profile-exact-macro.py compression `
  --out backend/target/exact-macro-records/profile-compression -- `
  --baseline <baseline目录> --screening --seconds 600

# 提取：接收完整的场景合同，默认使用 runner 指定的可执行文件。
.venv/exact-macro/Scripts/python.exe tools/profile-exact-macro.py extraction `
  --out backend/target/exact-macro-records/profile-extraction -- `
  <scene.json> --exe backend/target/release/jx3-combat-sim.exe

# 两阶段：将上例 extraction 改成 pipeline。
```

`--` 后是原 runner 的参数；输出目录由工具自动放入 `run/`，不能重复传 `--out`。压缩可传 `--macro` 改变认证起点，或 `--module` 选择独立算法快照。不要同时开启 cProfile。每次使用新目录，防止覆盖证据。

关闭追踪做 A/B 时直接运行 `backend/tests/macro_exact/compression_benchmark.py`。`--archive-equal` 可恢复为等长中间候选写完整原始状态包的旧留档方式，用来隔离序列化与写盘成本；它不改变真实回放或验收。默认省掉这些中间大包，每份更短的认证最佳宏仍写完整包，所有候选保留精简记录。每轮记录 `candidate_sha256`，可核对优化前后是否试了同一批宏。

上例 `--seconds 600` 只是离线对照的观察上限，不是产品任务总时限。省略它时，压缩 benchmark 保留原有 90 秒观察上限；提取/完整流水线保留原有默认不设总时限。观察结束不等于搜索完成，以 `run/benchmark.json` 或 `run/report.json` 的状态为准。

## 输出与读法

- `trace.html`：可直接用浏览器打开的时间线；`trace.json`：原始 Chrome Trace 格式。
- `summary.md` / `summary.json`：按函数汇总、工具版本、源码摘要、任务结果、是否溢出。
- `run/`：真实候选、认证结果和原 runner 的记录。保留在本地忽略目录，不经浏览器或 VPS 传送完整结果。

若 HTML 查看器的外部脚本加载受限，可使用已安装的本地查看器打开 JSON：

```powershell
.venv/exact-macro/Scripts/vizviewer.exe --server_only --port 9001 <目录>/trace.json
```

该命令仅供本机交互诊断，查看结束后在终端按 Ctrl+C 关闭；不接入部署服务。

默认追踪项目 Python 文件，以及线程入口、队列和 JSON 编解码。线程入口必须纳入过滤范围，否则当前 Python 3.14 的过滤会连同接收线程回调一起跳过。忽略 C 调用，保留耗时至少 **100 微秒**的调用；不记录实参、返回值、print 内容或源码正文。函数名和本地源码路径仍在追踪内。`--min-us 0` 可保留短调用，`--include-c` 可纳入 C 调用，但开销和文件会增大。`--entries` 控制环形缓冲容量；发生覆盖时明确记录 `overflow=true`，工具返回 2；空追踪也返回 2，不能把缺失时间线当成完整数据。

`inclusive_ms` 含子调用；`residual_ms` 是减去已记录子调用后的墙钟耗时。过滤掉的短调用与未记录的库调用仍计在父函数中，所以它不是严格的 Python 自耗时，更不是 CPU 时间。`Oracle.run` 包含 Rust 执行、管道传输和等待；不能把它全算作 Python 计算。接收线程可能与主线程重叠，不把各线程总时间相加当成端到端时间。

追踪本身会增加开销。用它找热点，提速结论使用关闭追踪后的同场景 A/B；同时比较候选摘要、最佳宏和完整回放合同，不能用减少搜索或放宽验收冒充纯性能优化。

过滤与计时单位依据 [VizTracer Filter](https://viztracer.readthedocs.io/en/latest/filter.html)，API 与记录开关依据 [VizTracer API](https://viztracer.readthedocs.io/en/latest/viztracer.html)。当前可选依赖固定为 1.1.1。
