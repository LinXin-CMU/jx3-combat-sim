//! 进程隔离 Router 模式（`JX3_ROUTER=1` 启用）。
//!
//! 公网边缘：负责认证（登入页 + 白名单 cookie）+ 反向代理。
//! 每个登入用户名分配一个**独立的 worker 进程**（自身 exe 的普通模式，绑定 127.0.0.1 私有端口、
//! 独立 userdata 目录、关认证、不开浏览器），该用户所有请求反代到他自己的 worker。
//! → A / B 从内存到磁盘物理隔离，不存在共享可变状态，原理上不可能互相影响。
//!
//! SSE（优化器/RL 进度流）通过流式透传响应体支持。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{header, HeaderMap, HeaderName, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const MAX_BODY: usize = 64 * 1024 * 1024; // 64MB 请求体上限
const IDLE_SECS: u64 = 30 * 60; // worker 空闲 30 分钟回收
const SPAWN_WAIT_RETRIES: u32 = 400; // 冷启动 /health 轮询次数（×300ms ≈ 120s，容忍首启杀软扫描+冷盘）

struct WorkerEntry {
    port: u16,
    child: Child,
    last_seen: Instant,
}

pub struct Manager {
    workers: Mutex<HashMap<String, WorkerEntry>>,
    client: reqwest::Client,
    exe: PathBuf,
    cwd: PathBuf,
    userdata_root: PathBuf,
    next_port: AtomicU16,
    login_html: String,
    max_workers: usize,
}

impl Manager {
    /// 为用户名生成安全的目录名（中文/特殊字符替换 + 附加哈希防碰撞）。
    fn userdata_dir_for(&self, user: &str) -> PathBuf {
        let mut safe: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(24)
            .collect();
        if safe.is_empty() {
            safe.push('u');
        }
        // FNV-1a 32bit 哈希
        let mut h: u32 = 2166136261;
        for b in user.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        self.userdata_root
            .join("users")
            .join(format!("{}_{:08x}", safe, h))
    }

    fn pick_port(&self) -> u16 {
        for _ in 0..200 {
            let p = self.next_port.fetch_add(1, Ordering::Relaxed);
            let p = if p < 4001 { 4001 } else { p };
            if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
                return p;
            }
        }
        // 兜底
        4001
    }

    /// 确保该用户的 worker 在运行且就绪，返回其端口。
    /// 新建时仅在锁内 spawn（快），随后**在锁外轮询 /health** 直到就绪才返回。
    async fn ensure_worker(&self, user: &str) -> Result<u16, String> {
        let port;
        {
            let mut map = self.workers.lock().await;
            let mut dead = false;
            if let Some(w) = map.get_mut(user) {
                if matches!(w.child.try_wait(), Ok(Some(_))) {
                    dead = true; // 进程已退出，需重建
                } else {
                    w.last_seen = Instant::now();
                    return Ok(w.port);
                }
            }
            if dead {
                map.remove(user);
            }
            if map.len() >= self.max_workers {
                return Err("服务繁忙，请稍后重试".into());
            }
            let p = self.pick_port();
            let dir = self.userdata_dir_for(user);
            let _ = std::fs::create_dir_all(&dir);
            let icon_dir = self.userdata_root.join("icon_cache");
            let _ = std::fs::create_dir_all(&icon_dir);
            // worker 的 stdout/stderr 重定向到各自日志文件，避免继承 router 管道被启动期海量日志写满阻塞
            let (out, err) = match std::fs::OpenOptions::new().create(true).append(true).open(dir.join("worker.log")) {
                Ok(f) => match f.try_clone() {
                    Ok(f2) => (Stdio::from(f), Stdio::from(f2)),
                    Err(_) => (Stdio::null(), Stdio::null()),
                },
                Err(_) => (Stdio::null(), Stdio::null()),
            };
            let mut command = Command::new(&self.exe);
            command.kill_on_drop(true);
            let child = command
                .env_remove("JX3_ROUTER")
                .env_remove("JX3_AUTH_PASSWORD")
                .env("JX3_PORT", p.to_string())
                .env("JX3_BIND", "127.0.0.1")
                .env("JX3_NO_BROWSER", "1")
                .env("JX3_USERDATA_DIR", dir.to_string_lossy().to_string())
                .env("JX3_ICON_CACHE_DIR", icon_dir.to_string_lossy().to_string())
                .current_dir(&self.cwd)
                .stdout(out)
                .stderr(err)
                .spawn()
                .map_err(|e| format!("spawn worker 失败: {e}"))?;
            println!(
                "[router] 为用户 {:?} 启动 worker，端口 {}，数据目录 {}",
                user,
                p,
                dir.display()
            );
            map.insert(
                user.to_string(),
                WorkerEntry {
                    port: p,
                    child,
                    last_seen: Instant::now(),
                },
            );
            port = p;
        }

        // 锁外轮询 worker 就绪（worker 是绑定端口后才 serve /health，就绪即数据已全部加载）
        let health = format!("http://127.0.0.1:{}/health", port);
        for _ in 0..SPAWN_WAIT_RETRIES {
            if let Ok(r) = self
                .client
                .get(&health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                if r.status().is_success() {
                    return Ok(port);
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        // 超时：清理
        let mut map = self.workers.lock().await;
        if let Some(mut w) = map.remove(user) {
            let _ = w.child.start_kill();
        }
        Err(format!("worker 启动超时（端口 {port}）"))
    }
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/html"))
        .unwrap_or(false)
}

/// 北京时间 HH:MM:SS（不引 chrono：UNIX 秒 +8h 取模）。
fn now_hms() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + 8 * 3600;
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

/// 是否在控制台记录此请求。只记有意义的 API 调用，过滤静态资源 + 高频低信号噪声
/// （icon 代理 / 设置自动同步 / 鉴权探测 / 健康检查）。
fn should_log_path(path: &str) -> bool {
    if !path.starts_with("/api/") {
        return false; // 静态文件 / 页面不记
    }
    if path.starts_with("/api/icon") || path == "/api/settings" || path == "/api/auth/me" {
        return false; // 高频噪声
    }
    true
}

async fn serve_login(State(mgr): State<Arc<Manager>>) -> Html<String> {
    Html(mgr.login_html.clone())
}

/// 反向代理 fallback：认证 → 找/起 worker → 流式转发。
async fn proxy(State(mgr): State<Arc<Manager>>, req: Request) -> Response {
    // 1) 认证
    let user = match crate::auth::authed_user(req.headers()) {
        Some(u) => u,
        None => {
            return if wants_html(req.headers()) {
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "/login.html")
                    .body(Body::empty())
                    .unwrap()
            } else {
                (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
            };
        }
    };

    // 2) 确保 worker
    let port = match mgr.ensure_worker(&user).await {
        Ok(p) => p,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "服务正在准备或繁忙，请稍后重试").into_response(),
    };

    // 3) 拆请求
    let (parts, body) = req.into_parts();
    let path_q = parts
        .uri
        .path_and_query()
        .map(|x| x.as_str())
        .unwrap_or("/")
        .to_string();
    let url = format!("http://127.0.0.1:{}{}", port, path_q);

    // 控制台记录：哪个用户调了什么 API（过滤静态/高频噪声）
    if should_log_path(parts.uri.path()) {
        println!(
            "[router] {} 用户 {:?} → {} {}",
            now_hms(),
            user,
            parts.method,
            parts.uri.path()
        );
    }

    let body_bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };

    // 4) 转发（冷启动期间连接失败会重试）
    let mut attempt = 0u32;
    let upstream = loop {
        let mut rb = mgr
            .client
            .request(parts.method.clone(), &url)
            .body(body_bytes.clone());
        for (k, v) in parts.headers.iter() {
            if is_hop_by_hop(k) || k == header::HOST || k == header::CONTENT_LENGTH {
                continue;
            }
            rb = rb.header(k, v);
        }
        match rb.send().await {
            Ok(r) => break r,
            Err(e) => {
                if e.is_connect() && attempt < SPAWN_WAIT_RETRIES {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                return (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")).into_response();
            }
        }
    };

    // 5) 流式回传（含 SSE）
    let status = upstream.status();
    let mut out_headers = HeaderMap::new();
    for (k, v) in upstream.headers().iter() {
        if is_hop_by_hop(k) || k == header::CONTENT_LENGTH {
            continue;
        }
        out_headers.insert(k.clone(), v.clone());
    }
    let stream = upstream.bytes_stream();
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    *response.headers_mut() = out_headers;
    response
}

pub async fn run() {
    // 认证主密码（必须）
    let master = std::env::var("JX3_AUTH_PASSWORD").unwrap_or_default();
    if master.trim().is_empty() {
        eprintln!("[router] 错误：router 模式必须设置 JX3_AUTH_PASSWORD");
        std::process::exit(1);
    }
    let userdata_root =
        std::fs::canonicalize(crate::userdata_base()).unwrap_or_else(|_| crate::userdata_base());
    let auth_file = std::env::var_os("JX3_AUTH_FILE").map(PathBuf::from)
        .unwrap_or_else(|| userdata_root.join("whitelist.json"));
    crate::auth::init(master, auth_file);

    // 登入页（router 不经过 ServeDir，直接读文件）
    let login_html = std::fs::read_to_string("../frontend/login.html")
        .or_else(|_| std::fs::read_to_string("frontend/login.html"))
        .unwrap_or_else(|_| "<h1>登入页缺失 (frontend/login.html)</h1>".into());

    let exe = std::env::current_exe().expect("current_exe");
    let cwd = std::env::current_dir().expect("current_dir");

    let mgr = Arc::new(Manager {
        workers: Mutex::new(HashMap::new()),
        // 关键：no_proxy()。否则 router→worker 的 127.0.0.1 连接会被系统/环境代理
        // （如 Clash 的 HTTP_PROXY=127.0.0.1:7897）劫持导致连不上 worker。
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("reqwest client"),
        exe,
        cwd,
        userdata_root,
        next_port: AtomicU16::new(4001),
        login_html,
        max_workers: std::env::var("JX3_MAX_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
    });

    // 空闲回收
    {
        let mgr2 = mgr.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut map = mgr2.workers.lock().await;
                let dead: Vec<String> = map
                    .iter()
                    .filter(|(_, w)| w.last_seen.elapsed().as_secs() > IDLE_SECS)
                    .map(|(u, _)| u.clone())
                    .collect();
                for u in dead {
                    if let Some(mut w) = map.remove(&u) {
                        let _ = w.child.start_kill();
                        println!("[router] 回收空闲 worker：用户 {:?}，端口 {}", u, w.port);
                    }
                }
            }
        });
    }

    // 启动预热：先拉起一个临时 worker 把 OS 磁盘缓存 + 杀软首扫的代价付掉，再杀掉，
    // 这样第一个真实用户的 worker 冷启动就快了（避免首个用户等几十秒）。
    {
        let mgr3 = mgr.clone();
        tokio::spawn(async move {
            println!("[router] 预热中（首启加载磁盘缓存）...");
            let t = std::time::Instant::now();
            let _ = mgr3.ensure_worker("__warmup__").await;
            {
                let mut map = mgr3.workers.lock().await;
                if let Some(mut w) = map.remove("__warmup__") {
                    let _ = w.child.start_kill();
                }
            }
            println!(
                "[router] 预热完成，用时 {:.1}s（首个用户将走热缓存）",
                t.elapsed().as_secs_f64()
            );
        });
    }

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/login", get(serve_login))
        .route("/login.html", get(serve_login))
        .route("/api/auth/login", post(crate::auth::login))
        .route("/api/auth/logout", post(crate::auth::logout))
        .route("/api/auth/me", get(crate::auth::me))
        .route("/api/auth/reload", post(crate::auth::reload))
        .fallback(proxy)
        .with_state(mgr.clone());

    let bind_addr = std::env::var("JX3_BIND").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("JX3_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3005);
    let listener = tokio::net::TcpListener::bind(format!("{bind_addr}:{port}"))
        .await
        .unwrap();
    println!("[router] 进程隔离 Router 运行于 http://{bind_addr}:{port}（每用户独立 worker）");

    let server = axum::serve(listener, app);
    tokio::select! {
        r = server => { if let Err(e) = r { eprintln!("[router] serve 退出: {e}"); } }
        _ = tokio::signal::ctrl_c() => {
            println!("[router] 收到 Ctrl+C，正在关闭所有 worker ...");
            let mut map = mgr.workers.lock().await;
            for (u, w) in map.iter_mut() {
                let _ = w.child.start_kill();
                println!("[router]   关闭 worker：用户 {:?}，端口 {}", u, w.port);
            }
        }
    }
}
