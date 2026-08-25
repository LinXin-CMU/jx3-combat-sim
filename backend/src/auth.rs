//! 公网发布鉴权：首次登入用「用户名 + 主密码」加入白名单；
//! 白名单内用户名再次登入无需密码；登入后下发持久 cookie，下次自动放行。
//!
//! 启用条件：环境变量 `JX3_AUTH_PASSWORD` 非空（本地 `cargo run` 不设则完全不鉴权）。
//! 白名单文件：`userdata/whitelist.json`（gitignored），可手动编辑（预加用户名 / 删除撤权后调 `/api/auth/reload`）。

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Whitelist {
    pub members: Vec<Member>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Member {
    pub username: String,
    #[serde(default)]
    pub tokens: Vec<String>,
    #[serde(default)]
    pub created: u64,
    /// 用户可选的专属密码（salt:sha256 哈希）。Some 时再次登入需提供该密码；None 则免密。
    #[serde(default)]
    pub password: Option<String>,
}

struct AuthState {
    master_pw: String,
    path: PathBuf,
    wl: RwLock<Whitelist>,
}

static AUTH: OnceLock<AuthState> = OnceLock::new();

/// 初始化鉴权（主密码 + 白名单文件路径）。只在 `JX3_AUTH_PASSWORD` 非空时调用。
pub fn init(master_pw: String, path: PathBuf) {
    let wl = load(&path);
    let n = wl.members.len();
    let _ = AUTH.set(AuthState { master_pw, path, wl: RwLock::new(wl) });
    println!("[auth] 鉴权已启用，白名单成员 {} 人，文件: {}", n, AUTH.get().unwrap().path.display());
}

fn load(path: &PathBuf) -> Whitelist {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn persist(st: &AuthState) {
    if let Some(parent) = st.path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(wl) = st.wl.read() {
        if let Ok(s) = serde_json::to_string_pretty(&*wl) {
            let _ = std::fs::write(&st.path, s);
        }
    }
}

fn gen_hex(len: usize) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let n: u8 = rng.gen_range(0..16);
            std::char::from_digit(n as u32, 16).unwrap()
        })
        .collect()
}

fn gen_token() -> String {
    gen_hex(32)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 把明文密码哈希成 `salt:sha256(salt:pw)`（不存明文）。
fn hash_pw(pw: &str) -> String {
    use sha2::{Digest, Sha256};
    let salt = gen_hex(12);
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b":");
    h.update(pw.as_bytes());
    format!("{}:{}", salt, to_hex(&h.finalize()))
}

fn verify_pw(pw: &str, stored: &str) -> bool {
    let Some((salt, digest)) = stored.split_once(':') else {
        return false;
    };
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b":");
    h.update(pw.as_bytes());
    to_hex(&h.finalize()) == digest
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        if let Some(v) = part.trim().strip_prefix("jx3_session=") {
            return Some(v.to_string());
        }
    }
    None
}

/// 给 Router 用：从请求头的 cookie 解析出已认证用户名（无效/未启用返回 None）。
pub fn authed_user(headers: &HeaderMap) -> Option<String> {
    let st = AUTH.get()?;
    let tok = cookie_token(headers)?;
    token_user(st, &tok)
}

fn token_user(st: &AuthState, token: &str) -> Option<String> {
    let wl = st.wl.read().ok()?;
    wl.members
        .iter()
        .find(|m| m.tokens.iter().any(|t| t == token))
        .map(|m| m.username.clone())
}

/// 无需鉴权即可访问的路径（登入页 + 登入相关接口 + 健康检查）。
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/health"
            | "/favicon.ico"
            | "/login"
            | "/login.html"
            | "/api/auth/login"
            | "/api/auth/me"
            | "/api/auth/logout"
    )
}

/// 全局鉴权中间件。未启用鉴权时直接放行。
pub async fn middleware(req: Request, next: Next) -> Response {
    let Some(st) = AUTH.get() else {
        return next.run(req).await;
    };
    // CORS 预检不拦
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if is_public(path) {
        return next.run(req).await;
    }
    if let Some(tok) = cookie_token(req.headers()) {
        if token_user(st, &tok).is_some() {
            return next.run(req).await;
        }
    }
    // 未通过：HTML 导航跳登入页，API 返回 401
    let wants_html = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/html"))
        .unwrap_or(false);
    if wants_html {
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/login.html")
            .body(Body::empty())
            .unwrap();
    }
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    /// 登入用：新账号填发布主密码；已设专属密码的账号填该专属密码；免密账号留空。
    #[serde(default)]
    pub password: String,
    /// 可选：设置/修改本账号的专属密码（非空时在本次登入成功后写入）。
    #[serde(default)]
    pub set_password: Option<String>,
}

/// 登入：白名单内用户名免密；新用户名需主密码，成功后加入白名单。下发持久 cookie。
pub async fn login(Json(req): Json<LoginReq>) -> Response {
    let Some(st) = AUTH.get() else {
        return json_err(StatusCode::SERVICE_UNAVAILABLE, "鉴权未启用");
    };
    let username = req.username.trim().to_string();
    if username.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "请填写用户名");
    }
    if username.len() > 40 {
        return json_err(StatusCode::BAD_REQUEST, "用户名过长");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let token = gen_token();
    let set_pw = req
        .set_password
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    {
        let mut wl = st.wl.write().unwrap();
        match wl.members.iter_mut().find(|m| m.username == username) {
            Some(m) => {
                // 已在白名单：有专属密码则校验，否则免密
                if let Some(hash) = &m.password {
                    if !verify_pw(&req.password, hash) {
                        return json_err(StatusCode::UNAUTHORIZED, "密码错误");
                    }
                }
                // 可选设置/修改专属密码
                if let Some(np) = &set_pw {
                    m.password = Some(hash_pw(np));
                }
                m.tokens.push(token.clone());
                // 限制每个成员保留的 token 数（多设备登入），防止无限增长
                let len = m.tokens.len();
                if len > 10 {
                    m.tokens.drain(0..len - 10);
                }
            }
            None => {
                // 新用户名：校验发布主密码（创建账号的门槛）
                if req.password != st.master_pw {
                    return json_err(StatusCode::UNAUTHORIZED, "密码错误");
                }
                wl.members.push(Member {
                    username: username.clone(),
                    tokens: vec![token.clone()],
                    created: now,
                    password: set_pw.as_deref().map(hash_pw),
                });
            }
        }
    }
    persist(st);
    let cookie = format!(
        "jx3_session={}; Path=/; HttpOnly; Max-Age=31536000; SameSite=Lax",
        token
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::SET_COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            "{{\"ok\":true,\"username\":{}}}",
            serde_json::to_string(&username).unwrap()
        )))
        .unwrap()
}

/// 登出：作废当前 cookie token。
pub async fn logout(headers: HeaderMap) -> Response {
    if let (Some(st), Some(tok)) = (AUTH.get(), cookie_token(&headers)) {
        {
            let mut wl = st.wl.write().unwrap();
            for m in &mut wl.members {
                m.tokens.retain(|t| t != &tok);
            }
        }
        persist(st);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::SET_COOKIE,
            "jx3_session=; Path=/; HttpOnly; Max-Age=0; SameSite=Lax",
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"ok\":true}"))
        .unwrap()
}

/// 当前登入状态（前端可用于显示「当前用户」）。
pub async fn me(headers: HeaderMap) -> Response {
    let Some(st) = AUTH.get() else {
        return json_body("{\"enabled\":false,\"authed\":true}");
    };
    if let Some(tok) = cookie_token(&headers) {
        if let Some(u) = token_user(st, &tok) {
            return json_body(&format!(
                "{{\"enabled\":true,\"authed\":true,\"username\":{}}}",
                serde_json::to_string(&u).unwrap()
            ));
        }
    }
    json_body("{\"enabled\":true,\"authed\":false}")
}

/// 重载白名单文件（手动编辑 whitelist.json 后调用，免重启）。需已登入（受中间件保护）。
pub async fn reload() -> Response {
    let Some(st) = AUTH.get() else {
        return json_body("{\"ok\":false,\"error\":\"鉴权未启用\"}");
    };
    let fresh = load(&st.path);
    let n = fresh.members.len();
    *st.wl.write().unwrap() = fresh;
    json_body(&format!("{{\"ok\":true,\"members\":{}}}", n))
}

fn json_body(s: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(s.to_string()))
        .unwrap()
}

fn json_err(code: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            "{{\"ok\":false,\"error\":{}}}",
            serde_json::to_string(msg).unwrap()
        )))
        .unwrap()
}
