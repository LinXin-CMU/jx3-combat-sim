//! RL 训练子进程管理 + SSE 事件转发。
//!
//! 设计：
//! - 全局 TrainState 持有 Option<ActiveTrain>（同时只允许一个训练运行）。
//! - start_handler：写 spec.json 临时文件 → spawn `python train.py --spec-json ...`
//!   → 读 stdout 行（每行一个 JSON 事件）→ 广播给 SSE 订阅者。
//! - stop_handler：kill child（Windows: TerminateProcess）。
//! - stream_handler：SSE，订阅 broadcaster；客户端可断线重连。
//! - status_handler：返回当前是否在跑 + run_id + 最新事件摘要。
//!
//! Python 解释器：默认 `python`（环境变量 `JX3_PYTHON` 覆盖）。
//! train.py 路径：默认 `<cwd>/../python/train.py`（相对 backend 工作目录）。

use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    extract::State,
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, Mutex};

use crate::{Attributes, SharedState, TargetConfig};

const EVENT_CHANNEL_CAPACITY: usize = 1024;

// ─────────────────────────────────────────────────────────────────────────────
// 全局状态
// ─────────────────────────────────────────────────────────────────────────────

pub struct TrainState {
    pub current: Mutex<Option<ActiveTrain>>,
    pub broadcaster: broadcast::Sender<JsonValue>,
    /// 最近一次 update 事件的快照（前端首次连接 SSE 时也能立即看到当前状态）
    pub last_event: Mutex<Option<JsonValue>>,
    /// 运行时可调参数（Python 每 update 轮询一次拉新值）
    pub runtime_params: tokio::sync::RwLock<RuntimeParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeParams {
    pub eval_every: u32,
    pub eval_duration: f64,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self { eval_every: 10, eval_duration: 60.0 }
    }
}

pub struct ActiveTrain {
    pub run_id: String,
    pub stop: Arc<AtomicBool>,
    pub child: Arc<Mutex<Option<Child>>>,
}

impl TrainState {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            current: Mutex::new(None),
            broadcaster: tx,
            last_event: Mutex::new(None),
            runtime_params: tokio::sync::RwLock::new(RuntimeParams::default()),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 请求 / 响应
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TrainStartRequest {
    pub attributes: Attributes,
    pub target: TargetConfig,
    #[serde(default)]
    pub talents: Vec<u32>,
    #[serde(default)]
    pub recipes: Vec<u32>,
    pub duration: f64,
    pub haste_level: u32,
    #[serde(default)]
    pub initial_rage: Option<i32>,
    #[serde(default)]
    pub network_delay: u32,
    #[serde(default)]
    pub baseline_dps: Option<f64>,

    // PPO 超参
    pub total_steps: u64,
    #[serde(default = "default_n_envs")]
    pub n_envs: u32,
    #[serde(default = "default_n_steps")]
    pub n_steps: u32,
    #[serde(default = "default_n_epochs")]
    pub n_epochs: u32,
    #[serde(default = "default_minibatch")]
    pub minibatch_size: u32,
    #[serde(default = "default_lr")]
    pub lr: f64,
    #[serde(default = "default_gamma")]
    pub gamma: f64,
    #[serde(default = "default_ent_coef")]
    pub ent_coef: f64,
    #[serde(default = "default_device")]
    pub device: String,
    #[serde(default = "default_save_every")]
    pub save_every: u32,
    #[serde(default)]
    pub resume: Option<String>,
    #[serde(default)]
    pub reset_optimizer: bool,
    /// 课程学习阶段：每阶段可覆盖 duration / total_steps / lr / ent_coef / baseline_dps / n_steps
    /// 若非空则按顺序链式执行，前一阶段 final ckpt 自动成为下一阶段 resume
    #[serde(default)]
    pub stages: Vec<serde_json::Value>,
    /// 每 N 次 update 跑一局确定性策略 rollout 给前端实时观察；0=关闭
    #[serde(default = "default_eval_every")]
    pub eval_every: u32,
    /// eval rollout 的 episode 时长（秒）
    #[serde(default = "default_eval_duration")]
    pub eval_duration: f64,
    /// 允许模型选择的动作 id 列表（None=不限制；0 等待会自动允许）
    #[serde(default)]
    pub allowed_actions: Option<Vec<u32>>,
}

fn default_eval_every() -> u32 { 10 }
fn default_eval_duration() -> f64 { 60.0 }

fn default_n_envs() -> u32 { 8 }
fn default_n_steps() -> u32 { 2048 }
fn default_n_epochs() -> u32 { 10 }
fn default_minibatch() -> u32 { 512 }
fn default_lr() -> f64 { 3e-4 }
fn default_gamma() -> f64 { 0.999 }
fn default_ent_coef() -> f64 { 0.01 }
fn default_device() -> String { "cuda".into() }
fn default_save_every() -> u32 { 20 }

#[derive(Debug, Serialize)]
pub struct TrainStartResponse {
    pub run_id: String,
    pub spec_path: String,
    pub log_dir: String,
    pub save_dir: String,
}

#[derive(Debug, Serialize)]
pub struct TrainStatusResponse {
    pub running: bool,
    pub run_id: Option<String>,
    pub last_event: Option<JsonValue>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器
// ─────────────────────────────────────────────────────────────────────────────

pub async fn start_handler(
    State(shared): State<SharedState>,
    Json(req): Json<TrainStartRequest>,
) -> Response {
    let train = shared.rl_train.clone();
    let mut guard = train.current.lock().await;
    if guard.is_some() {
        return (StatusCode::CONFLICT, Json(err("已有正在运行的训练任务"))).into_response();
    }

    let run_id = format!(
        "rl_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    // 初始化运行时参数（之后可通过 /api/rl/train/params 实时覆盖）
    {
        let mut rp = train.runtime_params.write().await;
        rp.eval_every = req.eval_every;
        rp.eval_duration = req.eval_duration;
    }

    // 路径解析
    let python_dir = resolve_python_dir();
    let python_bin = resolve_python_bin(&python_dir);
    let train_py = python_dir.join("train.py");
    if !train_py.exists() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err(&format!("找不到 train.py: {}", train_py.display()))),
        )
            .into_response();
    }

    let runs_root = python_dir.join("runs").join(&run_id);
    let save_dir = runs_root.join("ckpt");
    let log_dir = runs_root.join("tb");
    let _ = std::fs::create_dir_all(&save_dir);
    let _ = std::fs::create_dir_all(&log_dir);

    // 拼 spec.json
    let spec = serde_json::json!({
        "base_url": "http://localhost:3005",
        "run_id": run_id,
        "attributes": req.attributes,
        "target": req.target,
        "talents": req.talents,
        "recipes": req.recipes,
        "duration": req.duration,
        "initial_rage": req.initial_rage,
        "network_delay": req.network_delay,
        "baseline_dps": req.baseline_dps,
        "total_steps": req.total_steps,
        "n_envs": req.n_envs,
        "n_steps": req.n_steps,
        "n_epochs": req.n_epochs,
        "minibatch_size": req.minibatch_size,
        "lr": req.lr,
        "gamma": req.gamma,
        "ent_coef": req.ent_coef,
        "device": req.device,
        "save_dir": save_dir.to_string_lossy(),
        "log_dir": log_dir.to_string_lossy(),
        "save_every": req.save_every,
        "resume": req.resume,
        "reset_optimizer": req.reset_optimizer,
        "stages": req.stages,
        "eval_every": req.eval_every,
        "eval_duration": req.eval_duration,
        "allowed_actions": req.allowed_actions,
    });
    let spec_path = runs_root.join("spec.json");
    if let Err(e) = std::fs::write(&spec_path, serde_json::to_string_pretty(&spec).unwrap_or_default()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err(&format!("写 spec.json 失败: {}", e))),
        )
            .into_response();
    }

    // spawn
    let mut cmd = Command::new(&python_bin);
    cmd.current_dir(&python_dir)
        .arg(train_py.file_name().unwrap_or_default())
        .arg("--spec-json")
        .arg(spec_path.to_string_lossy().to_string())
        // 强制子进程 stdio 用 UTF-8（避免 Windows GBK 编码中文 JSON 时 UnicodeEncodeError）
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(err(&format!("启动 python 失败: {} ({})", python_bin, e))),
            )
                .into_response();
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let stop = Arc::new(AtomicBool::new(false));
    let child_arc: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));

    let active = ActiveTrain {
        run_id: run_id.clone(),
        stop: stop.clone(),
        child: child_arc.clone(),
    };
    *guard = Some(active);
    drop(guard);

    // 监听 stdout：每行一个 JSON 事件
    let tx = train.broadcaster.clone();
    let train_state_clone = train.clone();
    let train_for_last = train.clone();
    let run_id_for_cleanup = run_id.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    match serde_json::from_str::<JsonValue>(trimmed) {
                        Ok(ev) => {
                            *train_for_last.last_event.lock().await = Some(ev.clone());
                            let _ = tx.send(ev);
                        }
                        Err(_) => {
                            // 非 JSON 行（python 偶尔写到 stdout 的）
                            let ev = serde_json::json!({"event": "raw", "line": trimmed});
                            let _ = tx.send(ev);
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        // 进程退出
        let exit_code = {
            let mut g = child_arc.lock().await;
            if let Some(mut c) = g.take() {
                c.wait().await.ok().and_then(|s| s.code())
            } else {
                None
            }
        };
        let _ = tx.send(serde_json::json!({
            "event": "exit",
            "run_id": run_id_for_cleanup,
            "exit_code": exit_code,
        }));
        // 释放当前任务槽
        let mut g = train_state_clone.current.lock().await;
        if g.as_ref().map_or(false, |a| a.run_id == run_id_for_cleanup) {
            *g = None;
        }
    });

    // 转发 stderr 为 log 事件（不影响主流程）
    let tx_err = train.broadcaster.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_err.send(serde_json::json!({"event": "log", "line": line}));
        }
    });

    Json(TrainStartResponse {
        run_id,
        spec_path: spec_path.to_string_lossy().to_string(),
        log_dir: log_dir.to_string_lossy().to_string(),
        save_dir: save_dir.to_string_lossy().to_string(),
    })
    .into_response()
}

pub async fn stop_handler(State(shared): State<SharedState>) -> Response {
    let train = shared.rl_train.clone();
    let guard = train.current.lock().await;
    let Some(active) = guard.as_ref() else {
        return Json(serde_json::json!({"stopped": false, "reason": "无进行中的任务"})).into_response();
    };
    active.stop.store(true, Ordering::Relaxed);
    let run_id = active.run_id.clone();
    let child_arc = active.child.clone();
    drop(guard);

    // 第一轮：tokio start_kill + 3 秒 wait
    let (pid, killed_first) = {
        let mut child_guard = child_arc.lock().await;
        let pid = child_guard.as_ref().and_then(|c| c.id());
        let killed = if let Some(c) = child_guard.as_mut() {
            let _ = c.start_kill();
            tokio::time::timeout(std::time::Duration::from_secs(3), c.wait()).await.ok().is_some()
        } else {
            true
        };
        (pid, killed)
    };

    // 第二轮：如果 tokio kill 没生效，调 OS 级 taskkill /F /T
    // /F 强制；/T 杀子进程树；防止 python ThreadPoolExecutor 残留
    let mut force_killed = false;
    if !killed_first {
        if let Some(p) = pid {
            #[cfg(windows)]
            let result = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &p.to_string()])
                .output();
            #[cfg(not(windows))]
            let result = std::process::Command::new("kill")
                .args(["-9", &p.to_string()])
                .output();
            force_killed = result.map(|r| r.status.success()).unwrap_or(false);
        }
    }

    let _ = train.broadcaster.send(serde_json::json!({
        "event": "stop_requested",
        "run_id": run_id,
        "pid": pid,
        "killed_within_timeout": killed_first,
        "force_killed": force_killed,
    }));

    Json(serde_json::json!({
        "stopped": true,
        "run_id": run_id,
        "pid": pid,
        "killed": killed_first,
        "force_killed": force_killed,
    })).into_response()
}

/// 扫 python/runs/ 目录，列出历史 run 及其 ckpt 文件（递归查找 *.pt）
pub async fn list_runs_handler() -> Json<serde_json::Value> {
    let root = resolve_python_dir().join("runs");
    let mut runs: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue; };
            if !ft.is_dir() { continue; }
            let run_id = e.file_name().to_string_lossy().to_string();
            let run_dir = e.path();
            let spec_path = run_dir.join("spec.json");
            let mut ckpts: Vec<serde_json::Value> = Vec::new();
            collect_ckpts(&run_dir, &run_dir, &mut ckpts);
            ckpts.sort_by(|a, b| b["mtime"].as_u64().cmp(&a["mtime"].as_u64()));
            let spec = std::fs::read_to_string(&spec_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
            runs.push(serde_json::json!({
                "run_id": run_id,
                "ckpts": ckpts,
                "spec": spec,
            }));
        }
    }
    runs.sort_by(|a, b| b["run_id"].as_str().cmp(&a["run_id"].as_str()));
    Json(serde_json::json!({ "runs": runs }))
}

/// 递归收集 *.pt 文件（相对 run_root 做 name，显示时便于区分阶段）
fn collect_ckpts(dir: &std::path::Path, run_root: &std::path::Path, out: &mut Vec<serde_json::Value>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for f in entries.flatten() {
        let path = f.path();
        let Ok(ft) = f.file_type() else { continue; };
        if ft.is_dir() {
            collect_ckpts(&path, run_root, out);
            continue;
        }
        let name = f.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pt") { continue; }
        let rel = path.strip_prefix(run_root).ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| name.clone());
        let meta = std::fs::metadata(&path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(serde_json::json!({
            "name": rel,       // 形如 "warmup_30s/ppo_final.pt"
            "path": path.to_string_lossy(),
            "size": size,
            "mtime": mtime,
        }));
    }
}

/// GET /api/rl/train/params — Python 训练循环每个 update 轮询一次拉新值
pub async fn get_runtime_params_handler(State(shared): State<SharedState>) -> Json<RuntimeParams> {
    let p = shared.rl_train.runtime_params.read().await.clone();
    Json(p)
}

/// POST /api/rl/train/params — 前端实时改 eval_every / eval_duration
pub async fn set_runtime_params_handler(
    State(shared): State<SharedState>,
    Json(req): Json<RuntimeParams>,
) -> Json<RuntimeParams> {
    let mut g = shared.rl_train.runtime_params.write().await;
    *g = req.clone();
    Json(req)
}

pub async fn status_handler(State(shared): State<SharedState>) -> Json<TrainStatusResponse> {
    let train = shared.rl_train.clone();
    let guard = train.current.lock().await;
    let last = train.last_event.lock().await.clone();
    Json(TrainStatusResponse {
        running: guard.is_some(),
        run_id: guard.as_ref().map(|a| a.run_id.clone()),
        last_event: last,
    })
}

pub async fn stream_handler(
    State(shared): State<SharedState>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let rx = shared.rl_train.broadcaster.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        |res: Result<JsonValue, _>| async move {
            match res {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    Some(Ok::<_, std::convert::Infallible>(SseEvent::default().data(data)))
                }
                Err(_) => None,
            }
        },
    );
    use futures_util::StreamExt;
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ─────────────────────────────────────────────────────────────────────────────
// 工具
// ─────────────────────────────────────────────────────────────────────────────

fn err(msg: &str) -> JsonValue {
    serde_json::json!({"error": msg})
}

/// 供 analysis.rs 复用的包装
pub fn resolve_python_dir_pub() -> std::path::PathBuf { resolve_python_dir() }
pub fn resolve_python_bin_pub(dir: &std::path::Path) -> String { resolve_python_bin(dir) }

/// 解析 python 目录：优先 ./python；其次 ../python（相对 backend 工作目录）
fn resolve_python_dir() -> std::path::PathBuf {
    let candidates = [
        std::path::PathBuf::from("./python"),
        std::path::PathBuf::from("../python"),
    ];
    for c in &candidates {
        if c.join("train.py").exists() {
            return c.clone();
        }
    }
    candidates[0].clone()
}

/// 解析 python 解释器：依次查 python_dir/.venv → python_dir/../.venv（项目根）→ system `python`
/// JX3_PYTHON 环境变量已被用户注释掉（保留注释作为后续开关）
fn resolve_python_bin(python_dir: &std::path::Path) -> String {
    // if let Ok(p) = std::env::var("JX3_PYTHON") {
    //     return p;
    // }
    #[cfg(windows)]
    let rel = ["Scripts", "python.exe"];
    #[cfg(not(windows))]
    let rel = ["bin", "python"];

    let candidates = [
        python_dir.join(".venv").join(rel[0]).join(rel[1]),
        python_dir.join("..").join(".venv").join(rel[0]).join(rel[1]),
    ];
    for c in &candidates {
        if c.exists() {
            // 规范化路径（去掉 ../）避免 child process 的 cwd 相对解析歧义
            let abs = std::fs::canonicalize(c).unwrap_or_else(|_| c.clone());
            return abs.to_string_lossy().to_string();
        }
    }
    "python".to_string()
}
