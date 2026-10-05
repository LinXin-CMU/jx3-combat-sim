//! Worker-scoped autonomous runs. All writes are typed, additive experiment records.
use super::{run_loop, run_schema::*, run_store::RunRecord, run_tools};
use crate::{
    agent::{hash::canonical_sha256, provider::LlmProvider, AgentRuntime},
    SharedState,
};
use axum::{
    extract::{rejection::JsonRejection, Json, Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures_util::stream;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    sync::{atomic::Ordering, Arc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"schema_version":"harness-error/v1","error":{"code":code,"message":message}})),
    )
        .into_response()
}
fn missing() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "run_not_found",
        "实验不存在或不在当前用户的可恢复记录中。",
    )
}

fn provider(state: &SharedState, profile: &str) -> Result<Box<dyn LlmProvider>, String> {
    if profile == "offline" {
        return Ok(Box::new(run_loop::OfflineLaboratory));
    }
    state
        .agent_providers
        .create_provider(profile)
        .map_err(|e| format!("{}: {}", e.code, e.message))
}
fn select_provider(state: &SharedState, profile: &str) -> Result<String, String> {
    if !profile.is_empty() {
        return Ok(profile.into());
    }
    let profiles = state.agent_providers.safe_list().profiles;
    profiles
        .iter()
        .find(|p| {
            p.available
                && p.model.to_ascii_lowercase().contains("deepseek")
                && p.model.to_ascii_lowercase().contains("flash")
        })
        .or_else(|| {
            profiles
                .iter()
                .find(|p| p.available && p.model.to_ascii_lowercase().contains("deepseek"))
        })
        .or_else(|| profiles.iter().find(|p| p.available && p.id != "offline"))
        .map(|p| p.id.clone())
        .ok_or_else(|| "请先配置可用的 DeepSeek 接口。".into())
}
pub async fn create(
    State(state): State<SharedState>,
    body: Result<Json<RunRequest>, JsonRejection>,
) -> Response {
    let mut request = match body {
        Ok(Json(r)) => r,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "实验输入格式无效。",
            )
        }
    };
    if let Err(e) = request.validate() {
        return error(StatusCode::BAD_REQUEST, "invalid_request", &e);
    }
    request.provider_profile = match select_provider(&state, &request.provider_profile) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, "provider_unavailable", &e),
    };
    let provider = match provider(&state, &request.provider_profile) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, "provider_unavailable", &e),
    };
    let _admission = state.harness_jobs.admission.lock().await;
    if state.harness_runs.active()
        || state.harness_jobs.active()
        || super::http::legacy_busy(&state).await
    {
        return error(
            StatusCode::CONFLICT,
            "compute_busy",
            "已有计算任务正在执行，请等待或停止它。",
        );
    }
    let runtime = AgentRuntime::load(&state).await;
    if runtime.game_version() != request.version || runtime.mount() != request.mount {
        return error(
            StatusCode::CONFLICT,
            "runtime_mismatch",
            "版本或心法已变化，请重新读取现场。",
        );
    }
    let scenario = match request.scenario() {
        Ok(s) => s,
        Err(e) => return error(StatusCode::BAD_REQUEST, "invalid_scenario", &e),
    };
    let frozen = tokio::task::spawn_blocking(move || {
        identity(&runtime).map(|hash| (Arc::new(runtime), hash))
    })
    .await;
    let (runtime, runtime_hash) = match frozen {
        Ok(Ok(r)) => r,
        _ => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "runtime_identity_failed",
                "无法建立实验构建身份。",
            )
        }
    };
    let experiment_hash = match canonical_sha256(
        &json!({"runtime":RUNTIME_VERSION,"request":request,"scenario_hash":scenario.scenario_hash,"runtime_hash":runtime_hash}),
    ) {
        Ok(h) => h,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "场景不能规范化。",
            )
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let run_id = format!("experiment-{now:016x}-{:016x}", rand::random::<u64>());
    let checkpoint = Checkpoint {
        schema_version: RUNTIME_VERSION.into(),
        run_id: run_id.clone(),
        sequence: 1,
        status: "running".into(),
        phase: "created".into(),
        message: "已冻结当前场景，准备自主实验。".into(),
        request,
        scenario,
        runtime_hash,
        experiment_hash,
        model: provider.model().into(),
        usage: RunUsage::default(),
        events: vec![],
        artifacts: vec![],
        attempts: Default::default(),
        result: None,
        persistence_error: false,
        reserved_simulations: 0,
    };
    let record = match state.harness_runs.insert(checkpoint) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, "run_store_failed", &e),
    };
    spawn(record, runtime, provider);
    created(&run_id)
}
fn spawn(record: Arc<RunRecord>, runtime: Arc<AgentRuntime>, provider: Box<dyn LlmProvider>) {
    let owner = record.clone();
    tokio::spawn(async move {
        let execution = tokio::spawn(run_loop::drive(record, runtime, provider));
        if execution.await.is_err() {
            owner.mutate(true, |s| {
                s.status = "interrupted".into();
                s.phase = "interrupted".into();
                s.message = "实验调度异常结束，已保存此前证据，可恢复。".into();
                s.usage.simulations += s.reserved_simulations;
                s.reserved_simulations = 0;
            });
        }
    });
}
fn created(id: &str) -> Response {
    let base = format!("/api/harness/runs/{id}");
    (StatusCode::ACCEPTED,Json(json!({"schema_version":"harness-run-created/v1","run_id":id,"status_url":base,"events_url":format!("{base}/events"),"cancel_url":format!("{base}/cancel"),"artifacts_url":format!("{base}/artifacts"),"resume_url":format!("{base}/resume")}))).into_response()
}
pub async fn list(State(state): State<SharedState>) -> Response {
    Json(json!({"schema_version":"harness-runs/v1","runs":state.harness_runs.list()}))
        .into_response()
}
pub async fn status(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    match state.harness_runs.get(&id) {
        Some(r) => Json(r.snapshot()).into_response(),
        None => missing(),
    }
}
pub async fn cancel(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    match state.harness_runs.get(&id) {
        Some(r) => Json(json!({"run_id":id,"accepted":r.cancel()})).into_response(),
        None => missing(),
    }
}
pub async fn artifacts(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = state.harness_runs.get(&id) else {
        return missing();
    };
    Json(record.read()).into_response()
}
pub async fn resume(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = state.harness_runs.get(&id) else {
        return missing();
    };
    let _admission = state.harness_jobs.admission.lock().await;
    if state.harness_runs.active()
        || state.harness_jobs.active()
        || super::http::legacy_busy(&state).await
    {
        return error(
            StatusCode::CONFLICT,
            "compute_busy",
            "已有计算任务正在执行。",
        );
    }
    if record.snapshot()["resumable"] != true {
        return error(
            StatusCode::CONFLICT,
            "not_resumable",
            "任务已结束或原预算已耗尽；可基于结果创建新的目标。",
        );
    }
    let checkpoint = record.read();
    let runtime = Arc::new(AgentRuntime::load(&state).await);
    if runtime.game_version() != checkpoint.request.version
        || runtime.mount() != checkpoint.request.mount
    {
        return error(
            StatusCode::CONFLICT,
            "runtime_mismatch",
            "请先切回实验的版本和心法。",
        );
    }
    let frozen = runtime.clone();
    let hash = tokio::task::spawn_blocking(move || identity(&frozen)).await;
    if !matches!(hash,Ok(Ok(ref hash)) if hash==&checkpoint.runtime_hash) {
        return error(
            StatusCode::CONFLICT,
            "runtime_changed",
            "模拟器或数据构建已变化，不能混用原证据续跑；请创建新实验。",
        );
    }
    let provider = match provider(&state, &checkpoint.request.provider_profile) {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, "provider_unavailable", &e),
    };
    record.cancel.store(false, Ordering::SeqCst);
    record.mutate(true, |s| {
        s.status = "running".into();
        s.result = None;
    });
    if record.read().persistence_error {
        record.mutate(false, |s| s.status = "interrupted".into());
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "run_store_failed",
            "检查点暂时无法保存，未开始恢复计算。",
        );
    }
    record.event(
        "resumed",
        "从已保存证据和剩余预算继续，不重跑已完成实验。",
        None,
        None,
        None,
        true,
    );
    spawn(record, runtime, provider);
    created(&id)
}
pub async fn events(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = state.harness_runs.get(&id) else {
        return missing();
    };
    let receiver = record.updates.subscribe();
    let stream = stream::unfold(
        (receiver, true, false),
        |(mut receiver, first, done)| async move {
            if done {
                return None;
            }
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let snapshot = receiver.borrow_and_update().clone();
            let terminal = snapshot["running"] != true;
            let event = Event::default()
                .id(snapshot["sequence"].to_string())
                .event(if terminal { "completed" } else { "progress" })
                .data(snapshot.to_string());
            Some((Ok::<_, Infallible>(event), (receiver, false, terminal)))
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}
pub async fn capabilities() -> Response {
    Json(json!({"schema_version":"harness-capabilities/v2","runtime":RUNTIME_VERSION,"tools":run_tools::definitions(),"persistence":"immutable_worker_checkpoints","workflow":"model_selected_experiments","supports":["macro_reproduction","rotation_search","equipment_search","direct_candidate_evaluation","branching","independent_validation","resume","workspace_apply_undo"]})).into_response()
}

pub fn identity(runtime: &AgentRuntime) -> Result<String, String> {
    let base = super::contract::runtime_hash(runtime).map_err(str::to_owned)?;
    let equipment = runtime.equipment_identity()?;
    canonical_sha256(&json!({"runtime":RUNTIME_VERSION,"engine":base,"equipment":equipment}))
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    artifact_id: String,
    expected_scenario_hash: String,
}
pub async fn apply(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    body: Result<Json<ApplyRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_request", "应用输入无效。");
    };
    let _admission = state.harness_jobs.admission.lock().await;
    if state.harness_runs.active()
        || state.harness_jobs.active()
        || super::http::legacy_busy(&state).await
    {
        return error(
            StatusCode::CONFLICT,
            "compute_busy",
            "请先结束正在执行的计算任务再应用方案。",
        );
    }
    let Some(record) = state.harness_runs.get(&id) else {
        return missing();
    };
    let checkpoint = record.read();
    if checkpoint.status == "running" {
        return error(
            StatusCode::CONFLICT,
            "run_active",
            "请先结束当前实验再应用候选。",
        );
    }
    if *state.version.read().await != checkpoint.request.version
        || *state.mount.read().await != checkpoint.request.mount
    {
        return error(
            StatusCode::CONFLICT,
            "runtime_mismatch",
            "请先切回方案的版本与心法。",
        );
    }
    let runtime = AgentRuntime::load(&state).await;
    let expected_runtime = checkpoint.runtime_hash.clone();
    if !matches!(tokio::task::spawn_blocking(move||identity(&runtime)).await,Ok(Ok(hash)) if hash==expected_runtime)
    {
        return error(
            StatusCode::CONFLICT,
            "runtime_changed",
            "模拟器或数据构建已变化，请重新验证方案后应用。",
        );
    }
    if request.expected_scenario_hash != checkpoint.scenario.scenario_hash {
        return error(
            StatusCode::CONFLICT,
            "scene_changed",
            "现场快照不匹配，请重新预览。",
        );
    }
    let artifact = match run_tools::find_artifact(&checkpoint, &request.artifact_id) {
        Ok(a) => a,
        Err(e) => return error(StatusCode::BAD_REQUEST, "artifact_not_found", &e),
    };
    let best = &artifact.result["best"];
    if best["verified"] != true
        || best.get("page_constraints_passed") == Some(&Value::Bool(false))
        || best.get("constraints_passed") == Some(&Value::Bool(false))
    {
        return error(
            StatusCode::CONFLICT,
            "candidate_unverified",
            "该候选未通过基本约束，不能写入。",
        );
    }
    let transaction_id = format!("transaction-{:016x}", rand::random::<u64>());
    let transaction = json!({"transaction_id":transaction_id,"artifact_id":artifact.id,"version":checkpoint.request.version,"mount":checkpoint.request.mount,
        "before":{"simulation":checkpoint.request.simulation,"equipment":checkpoint.request.equipment},
        "after":{"simulation":artifact.simulation,"equipment":artifact.equipment},
        "diff":{"macro_before":checkpoint.request.simulation.macro_text,"macro_after":artifact.simulation.macro_text,"slot_diff":artifact.result.get("slot_diff")},"status":"prepared"});
    record.event(
        "workspace_prepared",
        "已生成应用事务与撤销点，工作区将校验当前内容后写入。",
        None,
        Some(&artifact.id),
        Some(transaction.clone()),
        true,
    );
    if record.read().persistence_error {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "run_store_failed",
            "应用事务无法保存，工作区尚未修改。",
        );
    }
    Json(transaction).into_response()
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    transaction_id: String,
}
pub async fn undo(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    body: Result<Json<UndoRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_request", "撤销输入无效。");
    };
    let Some(record) = state.harness_runs.get(&id) else {
        return missing();
    };
    let checkpoint = record.read();
    let Some(event) = checkpoint.events.iter().rev().find(|e| {
        e.kind == "workspace_prepared"
            && e.data
                .as_ref()
                .is_some_and(|v| v["transaction_id"] == request.transaction_id)
    }) else {
        return error(
            StatusCode::NOT_FOUND,
            "transaction_not_found",
            "撤销记录不存在。",
        );
    };
    let transaction = event.data.as_ref().unwrap();
    let reverse = json!({"transaction_id":request.transaction_id,"version":checkpoint.request.version,"mount":checkpoint.request.mount,"before":transaction["after"],"after":transaction["before"],"status":"undo_prepared"});
    record.event(
        "workspace_undo_prepared",
        "已生成撤销事务，工作区将核对应用后状态再恢复。",
        None,
        None,
        Some(reverse.clone()),
        true,
    );
    if record.read().persistence_error {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "run_store_failed",
            "撤销事务无法保存，工作区尚未修改。",
        );
    }
    Json(reverse).into_response()
}
