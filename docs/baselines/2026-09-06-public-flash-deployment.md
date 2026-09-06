# 公网恢复：Flash 专用部署

## 确认范围

用户批准恢复腾讯云 VPS + frp，并要求公网仅开放 Flash、分析次数不限。未设置每日、累计或用户次数配额；单次任务超时、资源预算、并发和请求频率保护继续生效。本地 3005 的配置、进程和数据不变。

## 链路

浏览器 HTTPS → VPS Nginx → VPS 回环 frp 端口 → 校验服务端证书的加密隧道 → 本机回环 3006 Router → 每账号独立 worker。

- 沿用旧入口端口 2014，升级 HTTPS。80 仅放行 ACME 验证，其他请求返回维护状态。443 当前云侧不通，发布不依赖它。
- Let's Encrypt 短期 IP 证书：独立 systemd timer 每日两次检查续签，部署钩子重载 Nginx。
- frp 使用全新 token 和专用 TLS 证书；客户端固定信任该证书并校验名称。隧道接收端口仅绑定 VPS 回环地址。专用 frp 证书有效期 825 天，到期前需同步更换客户端信任文件。
- 按用户 2026-09-07 要求，公网同时最多 100 个 worker（原为 4）。此为配置上限，未进行 100 人并发负载验收。Nginx 保护登录频率、瞬时请求和连接数，不限制累计分析次数。
- 独立 `config/agent.public.toml` 只包含 Flash；直接请求 Pro 在服务端返回 `provider_not_found`。
- 模型 key 从 Windows 用户环境读取后随子进程环境注入，未写入公开配置或浏览器。

## 账号与持久化

原 `backend/userdata/whitelist.json` 未覆盖。新的公网认证文件在 Windows 用户目录 `.jx3-public` 下，目录 ACL 仅允许当前用户访问。

- 保留原 6 个用户名和存档目录映射。
- 2 个原个人密码继续有效；按用户后续明确要求，4 个原免密账号恢复只填用户名登录。初次发布生成的临时密码已撤销。
- 新认证文件不继承旧登录 token；原认证文件保持不变。
- 既有免密账号保留免密；新账号仍需要新邀请码和至少 12 字符个人密码。免密用户名相当于公开入口，知道用户名的人可访问该账号的存档并使用 AI，此风险已告知用户。
- 新密码使用 Argon2id；旧哈希成功登录后升级。Cookie 为 Secure / HttpOnly / SameSite=Lax。
- worker 日志改为追加，避免重启覆盖原日志。
- 部署冒烟使用独立测试账号，未向原用户会话写测试问题。

## 操作与回滚

启动：`pwsh -NoProfile -File publish-frp.ps1`。

停止：`pwsh -NoProfile -File tools/stop-public.ps1`。

启动器使用独立 `backend/target/public-runtime/jx3-combat-sim.exe`，拒绝端口占用，不自动终止其他程序。停止器核对 PID、可执行文件和启动时间，只停止记录中的公网 Router、worker 和 frpc。

地址、账号交接和公网快捷入口位于私密目录。勿将整份交接文件分享给访客；只分发对应个人密码或邀请码。

VPS 保留 Nginx 维护页配置备份；需要暂时下线可恢复该配置并校验、重载 Nginx。旧 frp 配置仅作私密历史记录，禁止重新启用退役凭据。

电脑关机、休眠或断网时应用不可用，VPS 不运行模拟器本体。恢复电脑后需启动公网脚本；此次未新增开机自启或更改休眠设置。

## 验证结果

- Rust：337 passed、1 ignored；release 编译成功。
- 回环及真实 HTTPS 冒烟覆盖：匿名 API 拦截、既有免密账号登录、Cookie 标志、Flash 唯一可用、Pro 服务端拒绝、前端加载、注销撤销。
- HTTPS 使用系统证书验证，未跳过校验。Nginx 配置校验通过，续签 timer 已启用。
- 新 frpc 成功登录并建立代理。
- 真实 Flash 请求 `run-1a077602e1a-0` 已 completed；通过公网状态 API 二次确认。首次冒烟监测遇到一次 HTTP 连接异常，未重复创建任务；后续读取正常。冒烟脚本仅对 GET 增加有界重试，POST 不自动重发。
- 本地 3005 原进程保持运行，未替换其 exe；此次未改模拟公式、未更新 golden。

配置依据：[frp TLS 身份验证](https://gofrp.org/en/docs/features/common/network/network-tls/)、[Let's Encrypt IP 证书与 Certbot](https://letsencrypt.org/2026/03/11/shorter-certs-certbot)。
