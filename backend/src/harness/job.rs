use super::{
    compiler::{self, CompileProgress},
    contract::MacroCompileRequestV1,
};
use crate::agent::{AgentRuntime, ScenarioSnapshotV1};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::watch;

const RETAINED_JOBS: usize = 8;

#[derive(Clone, Serialize)]
pub struct JobStatus {
    pub schema_version: &'static str,
    pub job_id: String,
    pub sequence: u64,
    pub status: String,
    pub running: bool,
    pub phase: String,
    pub message: String,
    pub simulations: u32,
    pub max_simulations: u32,
    pub elapsed_ms: u64,
    pub scenario_hash: String,
    pub experiment_hash: String,
    pub cancellation_requested: bool,
    pub progress: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<Value>,
}

pub struct JobRecord {
    pub id: String,
    pub request: MacroCompileRequestV1,
    pub scenario: ScenarioSnapshotV1,
    pub runtime_hash: String,
    pub engine: Value,
    pub cancel: AtomicBool,
    started: Instant,
    status: Mutex<JobStatus>,
    updates: watch::Sender<JobStatus>,
}

impl JobRecord {
    pub fn snapshot(&self) -> JobStatus {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if status.running {
            status.elapsed_ms = self.started.elapsed().as_millis() as u64;
        }
        status
    }
    pub fn subscribe(&self) -> watch::Receiver<JobStatus> {
        self.updates.subscribe()
    }
    fn update(&self, change: impl FnOnce(&mut JobStatus)) {
        let mut state = self.status.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut state);
        state.sequence += 1;
        state.elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.updates.send_replace(state.clone());
    }
    pub fn request_cancel(&self) -> bool {
        let mut state = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if !state.running {
            return false;
        }
        if !state.cancellation_requested {
            self.cancel.store(true, Ordering::SeqCst);
            state.cancellation_requested = true;
            state.message = "已请求取消，正在保留已验证结果。".into();
            state.sequence += 1;
            state.elapsed_ms = self.started.elapsed().as_millis() as u64;
            self.updates.send_replace(state.clone());
        }
        true
    }
    fn progress(&self, progress: CompileProgress) {
        self.update(|state| {
            state.phase = progress.phase.clone();
            state.message = if state.cancellation_requested {
                "正在结束当前计算并保留已验证结果。".into()
            } else {
                progress.message.clone()
            };
            state.simulations = progress.simulations;
            state.progress = serde_json::to_value(progress).ok();
        });
    }
    fn finish(&self, result: Result<Value, String>) {
        self.update(|state| {
            state.running = false;
            match result {
                Ok(result) => {
                    let reason = result["stop_reason"].as_str().unwrap_or("completed");
                    state.status = if state.cancellation_requested || reason == "cancelled" {
                        "cancelled"
                    } else if matches!(
                        reason,
                        "budget_exhausted"
                            | "simulation_budget"
                            | "time_budget"
                            | "timeout"
                            | "simulation_limit"
                            | "time_limit"
                            | "max_rounds"
                            | "round_limit"
                    ) {
                        "budget_exhausted"
                    } else {
                        "completed"
                    }
                    .into();
                    state.phase = state.status.clone();
                    state.message = match state.status.as_str() {
                        "cancelled" => "任务已取消，保留已验证结果。",
                        "budget_exhausted" => "已达到预算，保留最好已测方案。",
                        _ => "搜索已结束，请查看还原结果与剩余差异。",
                    }
                    .into();
                    if let Some(n) = result["simulations"].as_u64() {
                        state.simulations = n as u32;
                    }
                    state.result = Some(result);
                }
                Err(message) => {
                    state.status = if state.cancellation_requested {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    state.phase = state.status.clone();
                    state.message = message.clone();
                    state.error = Some(json!({"code":"compile_failed","message":message}));
                }
            }
        });
    }
}

pub struct JobManager {
    records: Mutex<VecDeque<Arc<JobRecord>>>,
    counter: AtomicU64,
    nonce: u64,
    /// Serializes admission with legacy compute start handlers, not computation.
    pub admission: tokio::sync::Mutex<()>,
}
impl JobManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(VecDeque::new()),
            counter: AtomicU64::new(1),
            nonce: rand::random(),
            admission: tokio::sync::Mutex::new(()),
        })
    }
    pub fn active(&self) -> bool {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|r| r.snapshot().running)
    }
    pub fn list(&self) -> Vec<JobStatus> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|r| {
                let mut status = r.snapshot();
                status.result = None;
                status.progress = None;
                status
            })
            .collect()
    }
    pub fn get(&self, id: &str) -> Option<Arc<JobRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }
    pub fn insert(
        &self,
        request: MacroCompileRequestV1,
        scenario: ScenarioSnapshotV1,
        runtime_hash: String,
        experiment_hash: String,
        engine: Value,
    ) -> Result<Arc<JobRecord>, &'static str> {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.iter().any(|r| r.snapshot().running) {
            return Err("已有武学助手任务正在运行。");
        }
        let id = format!(
            "harness-{:016x}-{}",
            self.nonce,
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        let status = JobStatus {
            schema_version: "harness-job/v1",
            job_id: id.clone(),
            sequence: 1,
            status: "running".into(),
            running: true,
            phase: "baseline".into(),
            message: "正在建立目标轴基线。".into(),
            simulations: 0,
            max_simulations: request.max_simulations,
            elapsed_ms: 0,
            scenario_hash: scenario.scenario_hash.clone(),
            experiment_hash,
            cancellation_requested: false,
            progress: None,
            result: None,
            error: None,
        };
        let (updates, _) = watch::channel(status.clone());
        let record = Arc::new(JobRecord {
            id,
            request,
            scenario,
            runtime_hash,
            engine,
            cancel: AtomicBool::new(false),
            started: Instant::now(),
            status: Mutex::new(status),
            updates,
        });
        records.push_front(record.clone());
        while records.len() > RETAINED_JOBS {
            records.pop_back();
        }
        Ok(record)
    }
    pub fn execute(record: Arc<JobRecord>, runtime: AgentRuntime) {
        tokio::spawn(async move {
            let work = record.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                compiler::compile(
                    &work.request,
                    &runtime,
                    &work.scenario,
                    &work.cancel,
                    |progress| work.progress(progress),
                )
                .and_then(|result| {
                    serde_json::to_value(result).map_err(|_| "结果序列化失败。".into())
                })
            })
            .await;
            record.finish(
                outcome.unwrap_or_else(|_| Err("计算任务异常结束，已保留最近进度。".into())),
            );
        });
    }
}

#[cfg(test)]
#[path = "../../tests/harness/job.rs"]
mod tests;
