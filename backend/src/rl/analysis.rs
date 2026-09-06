//! RL 不一致分析子进程管理 + SSE（镜像 training.rs 的模式）。
//!
//! 只允许同时一个 analyze 任务运行（占用 GPU 推理 + HTTP env）。

use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

const EVENT_CAPACITY: usize = 512;

pub struct AnalyzeState {
    pub current: Mutex<Option<ActiveAnalyze>>,
    pub broadcaster: broadcast::Sender<JsonValue>,
    pub last_event: Mutex<Option<JsonValue>>,
    pub last_result: Mutex<Option<JsonValue>>,
}

pub struct ActiveAnalyze {
    pub run_id: String,
    pub stop: Arc<AtomicBool>,
    pub child: Arc<Mutex<Option<Child>>>,
}

impl AnalyzeState {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(EVENT_CAPACITY);
        Arc::new(Self {
            current: Mutex::new(None),
            broadcaster: tx,
            last_event: Mutex::new(None),
            last_result: Mutex::new(None),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct AnalyzeStartRequest {
    pub ckpt: String,
    pub macro_text: String,
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
    #[serde(default = "default_device")]
    pub device: String,
}

fn default_device() -> String {
    "cuda".into()
}

#[derive(Debug, Serialize)]
pub struct AnalyzeStartResponse {
    pub run_id: String,
    pub out_dir: String,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeStatusResponse {
    pub running: bool,
    pub run_id: Option<String>,
    pub last_event: Option<JsonValue>,
    pub last_result: Option<JsonValue>,
}

pub async fn start_handler(
    State(shared): State<SharedState>,
    Json(req): Json<AnalyzeStartRequest>,
) -> Response {
    let state = shared.rl_analyze.clone();
    let mut guard = state.current.lock().await;
    if guard.is_some() {
        return (StatusCode::CONFLICT, Json(err("已有正在运行的分析任务"))).into_response();
    }

    let run_id = format!(
        "ana_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    let python_dir = super::training::resolve_python_dir_pub();
    let python_bin = super::training::resolve_python_bin_pub(&python_dir);
    let analyze_py = python_dir.join("analyze.py");
    if !analyze_py.exists() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err(&format!("找不到 analyze.py: {}", analyze_py.display()))),
        )
            .into_response();
    }

    let runs_root = python_dir.join("analyses").join(&run_id);
    let _ = std::fs::create_dir_all(&runs_root);

    let spec = serde_json::json!({
        "base_url": "http://localhost:3005",
        "run_id": run_id,
        "ckpt": req.ckpt,
        "macro_text": req.macro_text,
        "duration": req.duration,
        "initial_rage": req.initial_rage,
        "network_delay": req.network_delay,
        "device": req.device,
        "out_dir": runs_root.to_string_lossy(),
        "attributes": req.attributes,
        "target": req.target,
        "talents": req.talents,
        "recipes": req.recipes,
    });
    let spec_path = runs_root.join("spec.json");
    if let Err(e) = std::fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec).unwrap_or_default(),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err(&format!("写 spec.json 失败: {}", e))),
        )
            .into_response();
    }

    let mut cmd = Command::new(&python_bin);
    cmd.current_dir(&python_dir)
        .arg(analyze_py.file_name().unwrap_or_default())
        .arg("--spec-json")
        .arg(spec_path.to_string_lossy().to_string())
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

    *guard = Some(ActiveAnalyze {
        run_id: run_id.clone(),
        stop: stop.clone(),
        child: child_arc.clone(),
    });
    drop(guard);

    // 清空上一次的结果快照
    *state.last_result.lock().await = None;
    *state.last_event.lock().await = None;

    let tx = state.broadcaster.clone();
    let state_clone = state.clone();
    let run_id_for_cleanup = run_id.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<JsonValue>(trimmed) {
                        Ok(ev) => {
                            if ev.get("event").and_then(|v| v.as_str()) == Some("done") {
                                *state_clone.last_result.lock().await = Some(ev.clone());
                            }
                            *state_clone.last_event.lock().await = Some(ev.clone());
                            let _ = tx.send(ev);
                        }
                        Err(_) => {
                            let _ = tx.send(serde_json::json!({"event": "raw", "line": trimmed}));
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
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
        let mut g = state_clone.current.lock().await;
        if g.as_ref().map_or(false, |a| a.run_id == run_id_for_cleanup) {
            *g = None;
        }
    });
    let tx_err = state.broadcaster.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_err.send(serde_json::json!({"event": "log", "line": line}));
        }
    });

    Json(AnalyzeStartResponse {
        run_id,
        out_dir: runs_root.to_string_lossy().to_string(),
    })
    .into_response()
}

pub async fn stop_handler(State(shared): State<SharedState>) -> Response {
    let state = shared.rl_analyze.clone();
    let guard = state.current.lock().await;
    let Some(active) = guard.as_ref() else {
        return Json(serde_json::json!({"stopped": false, "reason": "无进行中的任务"}))
            .into_response();
    };
    active
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let run_id = active.run_id.clone();
    let child_arc = active.child.clone();
    drop(guard);
    let mut cg = child_arc.lock().await;
    let killed = if let Some(c) = cg.as_mut() {
        let _ = c.start_kill();
        tokio::time::timeout(std::time::Duration::from_secs(3), c.wait())
            .await
            .ok()
            .is_some()
    } else {
        true
    };
    drop(cg);
    let _ = state.broadcaster.send(serde_json::json!({
        "event": "stop_requested", "run_id": run_id, "killed_within_timeout": killed,
    }));
    Json(serde_json::json!({"stopped": true, "run_id": run_id, "killed": killed})).into_response()
}

pub async fn status_handler(State(shared): State<SharedState>) -> Json<AnalyzeStatusResponse> {
    let state = shared.rl_analyze.clone();
    let guard = state.current.lock().await;
    let last = state.last_event.lock().await.clone();
    let last_result = state.last_result.lock().await.clone();
    Json(AnalyzeStatusResponse {
        running: guard.is_some(),
        run_id: guard.as_ref().map(|a| a.run_id.clone()),
        last_event: last,
        last_result,
    })
}

pub async fn stream_handler(
    State(shared): State<SharedState>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let rx = shared.rl_analyze.broadcaster.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        |res: Result<JsonValue, _>| async move {
            match res {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    Some(Ok::<_, std::convert::Infallible>(
                        SseEvent::default().data(data),
                    ))
                }
                Err(_) => None,
            }
        },
    );
    use futures_util::StreamExt;
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 返回最近一次分析的 rl_actions.json 内容（前端一键导入回放 tab 用）
pub async fn latest_actions_handler(State(shared): State<SharedState>) -> Response {
    let state = shared.rl_analyze.clone();
    let last_result = state.last_result.lock().await.clone();
    let Some(result) = last_result else {
        return (StatusCode::NOT_FOUND, Json(err("暂无分析结果"))).into_response();
    };
    let Some(path) = result.get("actions_path").and_then(|v| v.as_str()) else {
        return (StatusCode::NOT_FOUND, Json(err("结果未包含 actions_path"))).into_response();
    };
    match std::fs::read_to_string(path) {
        Ok(text) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(err(&format!("读取失败: {}", e))),
        )
            .into_response(),
    }
}

fn err(msg: &str) -> JsonValue {
    serde_json::json!({"error": msg})
}
