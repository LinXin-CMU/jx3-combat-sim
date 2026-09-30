//! One exact synthesis job per worker. Evidence stays in server-local records.
use crate::{GameVersion, Mount, SharedState, SimulateRequest};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Notify,
};

#[derive(Default)]
pub struct Manager {
    current: Mutex<Option<Arc<Job>>>,
    admission: tokio::sync::Mutex<()>,
}
struct Job {
    id: String,
    source: Value,
    view: Mutex<Value>,
    cancel: AtomicBool,
    paused: AtomicBool,
    done: AtomicBool,
    started: Instant,
    clock: Mutex<RunClock>,
    revision: AtomicU64,
    wake: Notify,
}
struct RunClock {
    elapsed: Duration,
    since: Option<Instant>,
}
impl RunClock {
    fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
            since: Some(Instant::now()),
        }
    }
    fn set_running(&mut self, running: bool) {
        if running {
            if self.since.is_none() {
                self.since = Some(Instant::now());
            }
        } else if let Some(since) = self.since.take() {
            self.elapsed += since.elapsed();
        }
    }
    fn elapsed_ms(&self) -> f64 {
        (self.elapsed + self.since.map(|s| s.elapsed()).unwrap_or_default()).as_secs_f64() * 1000.0
    }
}
#[derive(Default, Deserialize)]
pub struct SnapshotQuery {
    #[serde(default)]
    compact: bool,
    revision: Option<u64>,
    job_id: Option<String>,
    best_macro: Option<String>,
    candidate_macro: Option<String>,
    result_macro: Option<String>,
}

fn macro_revision(value: &Value) -> Option<String> {
    value["macro"].as_str().map(|text| format!("{:x}", Sha256::digest(text.as_bytes())))
}

fn macro_projection(value: &Value, known: Option<&str>) -> Value {
    let Some(object) = value.as_object() else { return value.clone(); };
    let revision = macro_revision(value);
    let unchanged = revision.as_deref().is_some_and(|r| Some(r) == known);
    let mut projection = serde_json::Map::new();
    for (key, field) in object {
        if key != "macro" || !unchanged { projection.insert(key.clone(), field.clone()); }
    }
    if let Some(revision) = revision { projection.insert("macro_revision".into(), revision.into()); }
    Value::Object(projection)
}
impl Manager {
    pub fn active(&self) -> bool {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|j| !j.done.load(Ordering::Acquire))
    }
    fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .filter(|j| j.id == id)
            .cloned()
    }
}
impl Job {
    fn snapshot(&self) -> Value {
        self.snapshot_options(&SnapshotQuery::default())
    }
    fn snapshot_options(&self, query: &SnapshotQuery) -> Value {
        let view = self.view.lock().unwrap();
        let revision = self.revision.load(Ordering::Acquire);
        let same_job = query.job_id.as_deref() == Some(self.id.as_str());
        let known = |key| if same_job { match key {
            "best" => query.best_macro.as_deref(),
            "candidate" => query.candidate_macro.as_deref(),
            _ => query.result_macro.as_deref(),
        } } else { None };
        // All callers get the UI projection, including legacy compact=false.
        let _ = query.compact;
        let mut v = {
            let mut v = json!({});
            for key in [
                "id",
                "status",
                "phase",
                "pause_requested",
                "iteration",
                "stage",
                "compression",
            ] {
                v[key] = view[key].clone();
            }
            if query.revision != Some(revision) || (query.job_id.is_some() && !same_job) {
                for key in ["reason", "version", "mount", "horizon"] {
                    v[key] = view[key].clone();
                }
                for key in ["best", "candidate"] { v[key] = macro_projection(&view[key], known(key)); }
                if let Some(report) = view["result"]["report"].as_object() {
                    let mut summary = json!({});
                    for key in [
                        "status",
                        "reason",
                        "comparison",
                        "total_solve_ms",
                        "compression",
                        "compression_stop",
                    ] {
                        if let Some(value) = report.get(key) {
                            summary[key] = value.clone();
                        }
                    }
                    v["result"] = macro_projection(&json!({"report":summary,"macro":view["result"]["macro"]}), known("result"));
                }
            }
            // A refreshed/lost client cache needs bodies even if it happens to
            // send the current task revision. Hashes are scoped to this job.
            if query.job_id.is_some() {
                for key in ["best", "candidate", "result"] {
                    if v.get(key).is_none() && macro_revision(&view[key]).as_deref().is_some_and(|r| Some(r) != known(key)) {
                        let mut body = json!({"macro":view[key]["macro"]});
                        if key != "result" { body = view[key].clone(); }
                        v[key] = macro_projection(&body, known(key));
                    }
                }
            }
            v
        };
        v["done"] = self.done.load(Ordering::Acquire).into();
        v["revision"] = revision.into();
        v["elapsed_ms"] = self.clock.lock().unwrap().elapsed_ms().into();
        v["elapsed_excludes_pauses"] = true.into();
        v["wall_elapsed_ms"] = (self.started.elapsed().as_secs_f64() * 1000.0).into();
        v["cancel_requested"] = self.cancel.load(Ordering::Acquire).into();
        if v["done"] != true && self.cancel.load(Ordering::Acquire) {
            v["phase"] = "cancelling".into();
        } else if v["done"] != true && self.paused.load(Ordering::Acquire) && v["phase"] != "paused"
        {
            v["phase"] = "pausing".into();
        }
        v
    }
    fn fail(&self, status: &str, message: &str) {
        let mut v = self.view.lock().unwrap();
        v["status"] = status.into();
        v["reason"] = message.into();
        self.revision.fetch_add(1, Ordering::Release);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    version: GameVersion,
    mount: Mount,
    simulation: SimulateRequest,
    horizon: f64,
    #[serde(default = "auto_compress")]
    compress: bool,
}
fn auto_compress() -> bool { true }
impl Start {
    fn validate(&self) -> Result<(), &'static str> {
        if !self.horizon.is_finite() || self.horizon <= 0.0 || self.horizon > 1200.0 {
            return Err("验证终点须在 0–1200 秒内。");
        }
        let r = &self.simulation;
        if r.sequence.is_empty()
            || r.sequence.len() > 6000
            || r.sequence.iter().any(|s| s.starts_with("__"))
        {
            return Err("请使用包含 1–6000 个主动技能的手动循环，暂不支持宏占位或调试操作。");
        }
        if r.macro_text.is_some() || r.lite || !r.pauses.is_empty() {
            return Err("精确合成需要完整手动循环，暂不支持停手区间。");
        }
        if r.attributes.is_none() || r.target.is_none() {
            return Err("请等待角色属性和目标加载完成。");
        }
        Ok(())
    }
}
fn paths() -> (PathBuf, PathBuf) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let root = if cwd.join("tools/exact-macro-worker.py").is_file() {
        cwd
    } else {
        cwd.parent().unwrap_or(&cwd).to_path_buf()
    };
    let local = root.join(if cfg!(windows) {
        ".venv/exact-macro/Scripts/python.exe"
    } else {
        ".venv/exact-macro/bin/python"
    });
    let python = std::env::var_os("JX3_EXACT_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if local.is_file() {
                local
            } else {
                PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
            }
        });
    (root, python)
}
fn error(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({"error":message}))).into_response()
}

pub async fn current(
    State(shared): State<SharedState>,
    Query(query): Query<SnapshotQuery>,
) -> Json<Value> {
    let job = shared
        .exact_jobs
        .current
        .lock()
        .unwrap()
        .as_ref()
        .map(|j| j.snapshot_options(&query));
    Json(json!({"available": paths().0.join("tools/exact-macro-worker.py").is_file(), "job":job}))
}
pub async fn create(State(shared): State<SharedState>, Json(request): Json<Start>) -> Response {
    if let Err(e) = request.validate() {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let _admission = shared.exact_jobs.admission.lock().await;
    if shared.exact_jobs.active()
        || legacy_busy(&shared).await
    {
        return error(
            StatusCode::CONFLICT,
            "已有计算任务正在执行，请等待完成或停止它。",
        );
    }
    let _gate = shared.agent_context_gate.read().await;
    if request.version != *shared.version.read().await
        || request.mount != *shared.mount.read().await
    {
        return error(
            StatusCode::CONFLICT,
            "版本或心法已变化，请重新读取当前循环。",
        );
    }
    if !paths().0.join("tools/exact-macro-worker.py").is_file() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "缺少精确合成运行文件，请按精确合成文档安装。",
        );
    }
    let id = format!(
        "exact-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let job = Arc::new(Job {
        id: id.clone(),
        source: json!({"version":request.version,"mount":request.mount,"simulation":request.simulation,"horizon":request.horizon}),
        view: Mutex::new(
            json!({"id":id,"status":"running","phase":"preparing","progress":[],"version":request.version,"mount":request.mount,"horizon":request.horizon}),
        ),
        cancel: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        done: AtomicBool::new(false),
        started: Instant::now(),
        clock: Mutex::new(RunClock::new()),
        revision: AtomicU64::new(1),
        wake: Notify::new(),
    });
    let response = job.snapshot();
    *shared.exact_jobs.current.lock().unwrap() = Some(job.clone());
    tokio::spawn(async move {
        execute(job, request).await;
    });
    (StatusCode::ACCEPTED, Json(response)).into_response()
}
pub async fn status(
    State(shared): State<SharedState>,
    Path(id): Path<String>,
    Query(query): Query<SnapshotQuery>,
) -> Response {
    match shared.exact_jobs.get(&id) {
        Some(j) => Json(j.snapshot_options(&query)).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "任务已过期。当前 worker 仅保留最近一次任务。",
        ),
    }
}
/// Explicit preview runs fetch only the frozen input, never the solver evidence archive.
pub async fn source(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    match shared.exact_jobs.get(&id) {
        Some(job) => Json(job.source.clone()).into_response(),
        None => error(StatusCode::NOT_FOUND, "任务已过期，请重新读取模板循环。"),
    }
}
pub async fn cancel(State(shared): State<SharedState>, Path(id): Path<String>, Query(mut query): Query<SnapshotQuery>) -> Response {
    let Some(j) = shared.exact_jobs.get(&id) else {
        return error(StatusCode::NOT_FOUND, "任务不存在。");
    };
    if !j.done.load(Ordering::Acquire) {
        j.cancel.store(true, Ordering::Release);
        j.clock.lock().unwrap().set_running(false);
        j.revision.fetch_add(1, Ordering::Release);
        j.wake.notify_one();
    }
    query.revision = None;
    Json(j.snapshot_options(&query))
    .into_response()
}
pub async fn pause(State(shared): State<SharedState>, Path(id): Path<String>, Query(query): Query<SnapshotQuery>) -> Response {
    control(&shared, &id, true, query)
}
pub async fn resume(State(shared): State<SharedState>, Path(id): Path<String>, Query(query): Query<SnapshotQuery>) -> Response {
    control(&shared, &id, false, query)
}
fn control(shared: &SharedState, id: &str, paused: bool, mut query: SnapshotQuery) -> Response {
    let Some(j) = shared.exact_jobs.get(id) else {
        return error(StatusCode::NOT_FOUND, "任务不存在。");
    };
    if !j.done.load(Ordering::Acquire) && !j.cancel.load(Ordering::Acquire) {
        j.paused.store(paused, Ordering::Release);
        let mut v = j.view.lock().unwrap();
        v["pause_requested"] = paused.into();
        v["phase"] = if paused { "pausing" } else { "resuming" }.into();
        j.clock.lock().unwrap().set_running(!paused);
        j.revision.fetch_add(1, Ordering::Release);
        j.wake.notify_one();
    }
    query.revision = None;
    Json(j.snapshot_options(&query))
    .into_response()
}
async fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}
async fn execute(job: Arc<Job>, request: Start) {
    let dir = paths().0.join("backend/target/exact-macro-records").join(&job.id);
    let outcome = run_process(&job, &request, &dir).await;
    job.clock.lock().unwrap().set_running(false);
    if outcome.is_err() {
        job.fail(
            "error",
            "精确合成进程启动或读取失败；请检查 Python / z3-solver 安装。",
        );
    }
    if let Ok(bytes) = tokio::fs::read(dir.join("result.json")).await {
        if let Ok(result) = serde_json::from_slice::<Value>(&bytes) {
            let mut view = job.view.lock().unwrap();
            view["status"] = result["report"]["status"].clone();
            if result["report"]["compression"].is_object() {
                view["compression"] = result["report"]["compression"].clone();
            }
            view["result"] = result;
        }
    }
    {
        let mut v = job.view.lock().unwrap();
        if v["status"] == "running" {
            v["status"] = "error".into();
            v["reason"] = "求解进程未返回完整报告。".into();
        }
        v["elapsed_ms"] = job.clock.lock().unwrap().elapsed_ms().into();
    }
    // Keep local test evidence, never load it into the API response or ZIP memory.
    job.done.store(true, Ordering::Release);
    job.revision.fetch_add(1, Ordering::Release);
}
async fn run_process(
    job: &Job,
    request: &Start,
    dir: &std::path::Path,
) -> Result<(), std::io::Error> {
    tokio::fs::create_dir_all(dir).await?;
    let scene = json!({"version":request.version,"mount":request.mount,"simulation":request.simulation,"horizon":request.horizon});
    tokio::fs::write(dir.join("input.json"), serde_json::to_vec(&scene)?).await?;
    let (root, python) = paths();
    let mut command = Command::new(python);
    command
        .arg(root.join("tools/exact-macro-worker.py"))
        .arg(dir.join("input.json"))
        .arg("--exe")
        .arg(std::env::current_exe()?)
        .arg("--out")
        .arg(dir)
        .current_dir(root.join("backend"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env("PYTHONUTF8", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    command.arg(if request.compress { "--compress" } else { "--no-compress" });
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    let mut cancellation = None;
    let mut oracle_pid = None;
    let mut sent_pause = false;
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                if let Ok(mut event) = serde_json::from_str::<Value>(&line) {
                    if let Some(pid) = event["oracle_pid"].as_u64() { oracle_pid = Some(pid as u32); }
                    event.as_object_mut().map(|m| m.remove("oracle_pid"));
                    let mut v = job.view.lock().unwrap();
                    if let Some(iteration) = event["iteration"].as_u64() { v["iteration"] = iteration.max(v["iteration"].as_u64().unwrap_or(0)).into(); }
                    if event["phase"].is_string() { v["phase"] = event["phase"].clone(); }
                    if event["stage"].is_string() { v["stage"] = event["stage"].clone(); }
                    if event["compression"].is_object() { v["compression"] = event["compression"].clone(); }
                    if event["phase"] == "target" { v["target"] = event["target"].take(); }
                    if event["phase"] == "best" { v["best"] = json!({"macro":event["macro"].take(),"comparison":event["comparison"].clone(),"char_count":event["char_count"].clone()}); }
                    if event["phase"] == "candidate" { v["candidate"] = json!({"macro":event["macro"].take(),"iteration":event["iteration"].clone(),"rule_count":event["rule_count"].clone(),"time_tolerance_seconds":event["time_tolerance_seconds"].clone(),"comparison":null}); }
                    if event["phase"] == "replayed" && event["iteration"] == v["candidate"]["iteration"] { v["candidate"]["comparison"] = event["comparison"].clone(); }
                    if event["phase"] == "failed" { v["status"] = event["status"].clone(); v["reason"] = event["reason"].clone(); }
                    let progress = v["progress"].as_array_mut().unwrap();
                    progress.push(event); if progress.len() > 100 { progress.remove(0); }
                    job.revision.fetch_add(1, Ordering::Release);
                }
            }
            _ = async { tokio::select! { _ = ticker.tick() => {}, _ = job.wake.notified() => {} } } => {
                let paused = job.paused.load(Ordering::Acquire);
                if cancellation.is_none() && paused != sent_pause {
                    let _ = input.write_all(if paused { b"pause\n" } else { b"resume\n" }).await;
                    sent_pause = paused;
                }
                if cancellation.is_none() && job.cancel.load(Ordering::Acquire) {
                    let _ = input.write_all(b"cancel\n").await;
                    cancellation = Some(Instant::now());
                    job.view.lock().unwrap()["phase"] = "cancelling".into();
                }
                if cancellation.is_some_and(|at: Instant| at.elapsed() > Duration::from_secs(2)) {
                    if let Some(pid) = oracle_pid { kill_pid(pid).await; }
                    let _ = child.kill().await;
                    job.fail(if job.cancel.load(Ordering::Acquire) { "cancelled" } else { "budget_exhausted" }, "任务已停止；未取得完整回放证书。");
                    break;
                }
            }
        }
    }
    let _ = child.wait().await;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/macro_exact/http.rs"]
mod tests;

pub(crate) async fn legacy_busy(shared: &SharedState) -> bool {
    if shared.exact_jobs.active() { return true; }
    if shared.optimizer.current.lock().await.is_some()
        || shared.rl_train.current.lock().await.is_some()
        || shared.rl_analyze.current.lock().await.is_some()
        || shared.rl_pretrain.current.lock().await.is_some()
    {
        return true;
    }
    shared
        .auto_search
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .running
}

/// Applied only to the legacy heavy-job start endpoints. Holding the same
/// admission lock until a handler returns closes the check/start race.
pub async fn legacy_admission(
    State(shared): State<SharedState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let _admission = shared.exact_jobs.admission.lock().await;
    if legacy_busy(&shared).await {
        // Consume a bounded POST body before an early HTTP/1 rejection. Dropping
        // an unread request can reset the connection on Windows instead of
        // delivering the structured 409 to the client.
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024),
        )
        .await;
        return error(StatusCode::CONFLICT, "已有计算任务正在运行，请先完成或取消该任务。");
    }
    next.run(request).await
}
