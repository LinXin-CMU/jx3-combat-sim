//! Run ownership, immutable checkpoints and latest-snapshot event delivery.
use super::run_schema::*;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::watch;

const FILE_LIMIT: usize = 32 * 1024 * 1024;
const STORE_LIMIT: u64 = 512 * 1024 * 1024;

// Evidence is immutable and stored once. Checkpoints retain only references to
// it, avoiding quadratic disk growth as a long experiment accumulates traces.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > FILE_LIMIT {
        return Err(std::io::Error::other("evidence size limit"));
    }
    if path.exists() {
        return Ok(());
    }
    let temp = path.with_extension(format!("{:016x}.pending", rand::random::<u64>()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn store_size(root: &Path) -> std::io::Result<u64> {
    let mut total = 0_u64;
    for directory in fs::read_dir(root)? {
        let directory = directory?;
        if !directory.file_type()?.is_dir() {
            continue;
        }
        for file in fs::read_dir(directory.path())? {
            let file = file?;
            if file.file_type()?.is_file() {
                total = total.saturating_add(file.metadata()?.len());
            }
        }
    }
    Ok(total)
}
fn read_checkpoint(path: &Path) -> Option<Checkpoint> {
    let mut value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if let Some(references) = value.get("artifact_files").and_then(Value::as_array) {
        let mut artifacts = Vec::new();
        for reference in references {
            let file = reference.as_str()?;
            if !file.starts_with("artifact-evidence-")
                || !file.ends_with(".json")
                || !file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            {
                return None;
            }
            let artifact_path = path.parent()?.join(file);
            if fs::metadata(&artifact_path).ok()?.len() > FILE_LIMIT as u64 {
                return None;
            }
            let artifact: Value = serde_json::from_slice(&fs::read(artifact_path).ok()?).ok()?;
            artifacts.push(artifact);
        }
        value["artifacts"] = json!(artifacts);
        value.as_object_mut()?.remove("artifact_files");
    }
    serde_json::from_value(value).ok()
}

pub struct RunRecord {
    pub checkpoint: Mutex<Checkpoint>,
    pub cancel: AtomicBool,
    pub updates: watch::Sender<Value>,
    root: Option<PathBuf>,
    persistence: Mutex<()>,
}
impl RunRecord {
    pub fn new(checkpoint: Checkpoint, root: Option<PathBuf>) -> Arc<Self> {
        let (updates, _) = watch::channel(snapshot_of(&checkpoint));
        Arc::new(Self {
            checkpoint: Mutex::new(checkpoint),
            cancel: AtomicBool::new(false),
            updates,
            root,
            persistence: Mutex::new(()),
        })
    }
    pub fn read(&self) -> Checkpoint {
        self.checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn snapshot(&self) -> Value {
        snapshot_of(&self.checkpoint.lock().unwrap_or_else(|e| e.into_inner()))
    }
    pub fn active(&self) -> bool {
        self.checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            == "running"
    }
    pub fn mutate(&self, durable: bool, change: impl FnOnce(&mut Checkpoint)) {
        {
            let mut state = self.checkpoint.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut state);
            state.sequence += 1;
            self.updates.send_replace(snapshot_of(&state));
        }
        if durable {
            self.persist();
        }
    }
    pub fn event(
        &self,
        kind: &str,
        message: &str,
        tool: Option<&str>,
        artifact_id: Option<&str>,
        data: Option<Value>,
        durable: bool,
    ) {
        self.mutate(durable, |state| {
            state.events.push(RunEvent {
                sequence: state.sequence + 1,
                kind: kind.into(),
                message: public_text(message, 4000),
                tool: tool.map(str::to_owned),
                artifact_id: artifact_id.map(str::to_owned),
                data,
            });
            if state.events.len() > 160 {
                state.events.remove(0);
            }
            state.message = public_text(message, 1000);
            state.phase = kind.into();
        });
    }
    pub fn cancel(&self) -> bool {
        if !self.active() {
            return false;
        }
        self.cancel.store(true, Ordering::SeqCst);
        self.event(
            "cancelling",
            "已请求停止，当前实验结束后保留已验证证据。",
            None,
            None,
            None,
            true,
        );
        true
    }
    pub fn persist(&self) {
        let Some(root) = &self.root else {
            return;
        };
        let _guard = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.read();
        state.persistence_error = false;
        let write = || -> std::io::Result<()> {
            let directory = root.join(&state.run_id);
            fs::create_dir_all(&directory)?;
            let path = directory.join(format!("checkpoint-{:08}.json", state.sequence));
            if path.exists() {
                return Ok(());
            }
            let mut total = store_size(root)?;
            let mut value = serde_json::to_value(&state)?;
            let mut references = Vec::new();
            for artifact in &state.artifacts {
                // A crash may leave an orphan artifact before its checkpoint.
                // Content identity prevents a reused evidence-N from resolving
                // to that different old candidate after recovery.
                let hash = crate::agent::hash::canonical_sha256(artifact)
                    .map_err(std::io::Error::other)?;
                let file = format!("artifact-{}-{hash}.json", artifact.id);
                let artifact_path = directory.join(&file);
                if !artifact_path.exists() {
                    let bytes = serde_json::to_vec(artifact)?;
                    total = total.saturating_add(bytes.len() as u64);
                    if total > STORE_LIMIT {
                        return Err(std::io::Error::other("experiment store quota"));
                    }
                    atomic_write(&artifact_path, &bytes)?;
                }
                references.push(file);
            }
            value["artifacts"] = json!([]);
            value["artifact_files"] = json!(references);
            let bytes = serde_json::to_vec(&value)?;
            if total.saturating_add(bytes.len() as u64) > STORE_LIMIT {
                return Err(std::io::Error::other("experiment store quota"));
            }
            atomic_write(&path, &bytes)?;
            Ok(())
        };
        if write().is_err() {
            self.mutate(false, |state| {
                state.persistence_error = true;
            });
        } else if self.read().persistence_error {
            self.mutate(false, |state| state.persistence_error = false);
        }
    }
}

pub struct RunManager {
    records: Mutex<VecDeque<Arc<RunRecord>>>,
    root: Option<PathBuf>,
}
impl RunManager {
    pub fn new(userdata: PathBuf) -> Arc<Self> {
        Self::open(Some(userdata.join("harness_runs/v2")))
    }
    pub fn open(root: Option<PathBuf>) -> Arc<Self> {
        let mut records = VecDeque::new();
        if let Some(root) = &root {
            if let Ok(entries) = fs::read_dir(root) {
                let mut dirs = entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .collect::<Vec<_>>();
                dirs.sort_by_key(|e| e.file_name());
                for dir in dirs.into_iter().rev().take(24) {
                    let Ok(entries) = fs::read_dir(dir.path()) else {
                        continue;
                    };
                    let mut paths = entries
                        .flatten()
                        .filter(|e| {
                            e.file_name().to_string_lossy().starts_with("checkpoint-")
                                && e.file_name().to_string_lossy().ends_with(".json")
                                && e.metadata().is_ok_and(|m| m.len() <= FILE_LIMIT as u64)
                        })
                        .map(|e| e.path())
                        .collect::<Vec<_>>();
                    paths.sort();
                    let newest_sequence = paths
                        .last()
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.strip_prefix("checkpoint-"))
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    // Atomic rename means an incomplete pending file is never a checkpoint.
                    for path in paths.iter().rev() {
                        if let Some(mut state) = read_checkpoint(path) {
                            if !valid_run_id(&state.run_id)
                                || dir.file_name().to_string_lossy() != state.run_id
                            {
                                continue;
                            }
                            state.sequence = state.sequence.max(newest_sequence).saturating_add(1);
                            if state.status == "running" {
                                state.status = "interrupted".into();
                                state.phase = "interrupted".into();
                                state.message =
                                    "服务曾中断，可从已保存证据继续；未完成的实验预算已保守计入。"
                                        .into();
                                state.usage.simulations = state
                                    .usage
                                    .simulations
                                    .saturating_add(state.reserved_simulations);
                                state.reserved_simulations = 0;
                                state.sequence += 1;
                            }
                            records.push_back(RunRecord::new(state, Some(root.clone())));
                            break;
                        }
                    }
                }
            }
        }
        Arc::new(Self {
            records: Mutex::new(records),
            root,
        })
    }
    pub fn get(&self, id: &str) -> Option<Arc<RunRecord>> {
        if !valid_run_id(id) {
            return None;
        }
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.read().run_id == id)
            .cloned()
    }
    pub fn active(&self) -> bool {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|r| r.active())
    }
    pub fn list(&self) -> Vec<Value> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|r| {
                let mut s = r.snapshot();
                s["events"] = json!([]);
                s["artifacts"] = json!([]);
                s
            })
            .collect()
    }
    pub fn insert(&self, state: Checkpoint) -> Result<Arc<RunRecord>, String> {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.iter().any(|r| r.active()) {
            return Err("已有自主实验正在执行。".into());
        }
        let record = RunRecord::new(state, self.root.clone());
        record.persist();
        if record.read().persistence_error {
            return Err("实验记录无法保存，请检查用户数据目录。".into());
        }
        records.push_front(record.clone());
        while records.len() > 24 {
            records.pop_back();
        }
        Ok(record)
    }
}

pub fn valid_run_id(id: &str) -> bool {
    id.starts_with("experiment-")
        && id.len() <= 80
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn snapshot_of(c: &Checkpoint) -> Value {
    let running = c.status == "running";
    let resumable = !running
        && c.status != "completed"
        && c.usage.model_calls < c.request.budget.max_model_calls
        && c.usage.simulations < c.request.budget.max_simulations
        && c.usage.elapsed_ms < c.request.budget.wall_time_ms
        && c.usage.total_tokens < c.request.budget.max_total_tokens;
    json!({"schema_version":"harness-run/v1","run_id":c.run_id,"sequence":c.sequence,"status":c.status,"running":running,
        "phase":c.phase,"message":c.message,"goal":c.request.goal,"version":c.request.version,"mount":c.request.mount,
        "scenario_hash":c.scenario.scenario_hash,"experiment_hash":c.experiment_hash,
        "provider_profile":c.request.provider_profile,"model":c.model,"usage":c.usage,"budget":c.request.budget,
        "events":c.events,"artifacts":c.artifacts.iter().map(public_artifact).collect::<Vec<_>>(),
        "result":c.result,"resumable":resumable,"persistence_error":c.persistence_error})
}

pub fn public_artifact(a: &Artifact) -> Value {
    let mut result = a.result.clone();
    if let Some(object) = result.as_object_mut() {
        for key in ["baseline", "best"] {
            if let Some(candidate) = object.get_mut(key).and_then(Value::as_object_mut) {
                candidate.remove("simulation");
            }
        }
        object.remove("timeline");
    }
    json!({"id":a.id,"parent_id":a.parent_id,"kind":a.kind,"scenario_hash":a.scenario_hash,"summary":a.summary,"result":result})
}

pub struct RunClock {
    started: Instant,
    prior: u64,
}
impl RunClock {
    pub fn new(prior: u64) -> Self {
        Self {
            started: Instant::now(),
            prior,
        }
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.prior + self.started.elapsed().as_millis() as u64
    }
}
