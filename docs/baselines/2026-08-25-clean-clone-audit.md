# Phase 0 全新目录复现审计

审计日期：2026-08-25  
验证提交：`a02ee00`  
平台：Windows 11，Rust/Cargo 1.94.1，Node.js 24.14.1，Python 3.14.3

## 验证范围

从本地 Git 仓库克隆到系统临时目录，不复制原工作区的 `backend/userdata/`、构建产物或 Python 环境。验证副本最初不存在 `backend/userdata/`，工作区无跟踪文件改动。

## 审计中发现并修复的问题

首次在提交 `6e24af0` 的全新 LF 工作区执行 Golden v2 时，8 个场景的战斗结果全部一致，但两个版本的 `data_sha256` 与原 CRLF 工作区不同。根因是 `tree_sha256()` 直接哈希工作区换行字节。

提交 `098bbea` 将参与数据/脚本 provenance hash 的文本规范化为 LF，并增加 CRLF/LF 等价单元测试；提交 `a02ee00` 刷新可移植 Golden 元数据。该修复没有改变 fingerprint、DPS、总伤害、事件数或战斗时长。

## 最终结果

| 检查 | 结果 |
|---|---|
| `node --check frontend/app.js` | 通过 |
| `python -m unittest discover -s tests -p "test_*.py" -v` | 1/1 通过 |
| `bash tests/check_no_direct_writes.sh` | 通过，0 处违规 |
| `cargo test` | 34/34 通过；19 条既有 warning |
| `cargo build --release` | 通过 |
| `tools/smoke.ps1` | `/health`、首页、5 秒模拟均通过 |
| 2025.10 Golden v2 | 确定性 4/4、Lite/Full 4/4、Golden 4/4 |
| 2026.04 Golden v2 | 确定性 4/4、Lite/Full 4/4、Golden 4/4 |

Smoke 输出 fingerprint 为 `2823599183917862576`，技能数为 4。测试服务仅绑定隔离端口，验证后已关闭。

最终可移植数据/脚本哈希：

- 2025.10：`9e3569e7ae382b249468d61d8efd2c9b5f60c7070ebb5b5ec452f7fd7f48781c`；
- 2026.04：`2ea69b443f3695c9c97e66a337be3eb018c76e3636bbd79a936aa240c37093de`。

## 结论

当前本地启动、核心测试和双版本 Golden 可以从 Git 跟踪内容独立重建，不依赖原工作区会话数据或历史换行格式。公开发布仍受许可证、数据来源边界、作品集素材和外部部署安全项约束。
