# 自定义 LLM API（2026-09-15）

发布版本：`2.0.14 / 20260915`。

本地与公网运行程序均已更新，文件哈希与本次 release 一致；本地设置端点及静态资源版本已验证，公网鉴权、健康检查和隧道连接正常。保留更新前的程序副本用于回滚。

## 使用方式

Agent 独立页面和循环模拟侧栏的模型选择旁均有“自定义接口”。填写显示名称、API 地址、模型 ID 和 API Key，选择协议后可测试连接，或直接“保存并使用”。保存后两个入口同步选中自定义模型；移除后回到可用的预置模型。

协议复用现有 Chat Completions、Responses、DeepSeek 三种适配模式。地址需包含服务商要求的前缀，例如 `/v1`；粘贴完整的 `/chat/completions` 或 `/responses` 地址时自动去掉末尾端点。测试只发送固定的连接/工具调用指令，不携带模拟场景，也不自动保存配置。

每个 worker 支持一个独立自定义配置。显示名称、地址、模型和协议保存在该 worker 的 `agent_custom_provider.json`；Key 仅保存在 worker 内存。页面刷新后可以继续使用，服务或 worker 重启后需重新填写 Key。同一地址留空 Key 可沿用当前凭据；更改地址必须重新填写。

## 实现边界

- 专用 `GET/PUT/DELETE /api/agent/providers/custom` 与 `POST /api/agent/providers/custom/test`；Agent run 仍只传 `provider_profile=user-custom`，不接受凭据或任意网络工具。
- 预置 provider 配置和凭据规则保持原样。自定义模型也经过原有工具白名单、预算、证据校验、报告和会话链路。
- 设置需要同源专用请求头；它不在跨域允许列表中。浏览器通过 HTTPS 或本机地址提交 Key，远程上游必须使用 HTTPS。
- 自定义上游禁用代理和重定向，连接时检查实际 DNS 结果，拒绝内网、链路本地等地址；本地运行允许显式 loopback 地址，公网 worker 不允许。
- Key 不进入本地浏览器存储、通用设置、配置文件、Agent 会话、trace 或 replay。关闭/保存窗口清空密码框；上游错误使用固定分类，自定义上游诊断不写 replay；响应回显凭据时拒绝该响应。
- 测试每个 worker 同时最多一个，最长 40 秒；上游响应按实际接收大小限制为 1 MiB，包含无 Content-Length 的分块响应。

## 验证

- Rust：486 项通过，1 项原有忽略。新增 9 项回归覆盖配置恢复、Key 生命周期、地址变更、worker 隔离、三种协议工具往返、私网和重定向拒绝、错误/凭据回显处理、分块响应大小限制。
- `tools/agent-custom-provider-smoke.py` 在隔离 worker 和本地 mock 上游验证保存、刷新、编辑、移除、三协议连接测试、错误显示和完整 Agent 工具循环；桌面/390px 手机布局、两处入口、Escape 行为及凭据不落盘检查通过。使用合成凭据，不调用真实服务商。
- Agent 工具评测 20/20，非法写入 0、userdata 不变、运行态恢复；模型离线评测 12/12。
- JavaScript 语法及脚本 setter 静态检查通过。
- 正式服两个版本各 4 个 Golden 场景的重复运行、Lite/Full、伤害数值和 fingerprint 一致。Golden v2 整体仍因既有 `data_sha256` 元数据漂移返回失败；没有更新 Golden。

复现：在一次性本地 worker 上运行 `python tools/agent-custom-provider-smoke.py --backend http://127.0.0.1:3039 --userdata <隔离 userdata 路径>`。脚本拒绝默认真实服务端口，会新建测试 Agent 会话；测试结束停止隔离 worker 并清理临时 userdata。

实际服务商的可用模型、兼容字段和工具能力需要用用户自己的配置测试；本次不宣称经过真实服务商兼容性认证。

测试 worker 已停止。自动审批以 `blocked by policy` 拒绝删除本次测试的临时目录，因此 `backend/target/custom-api-runtime` 与 `backend/target/custom-api-userdata` 保留在忽略目录下，没有换用其他方式删除。
