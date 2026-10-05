use super::{
    contract::{self, MacroCompileRequestV1},
    job::JobManager,
};
use crate::{agent::AgentRuntime, SharedState};
use axum::{
    extract::{rejection::JsonRejection, Path, Request, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    middleware::Next,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures_util::stream;
use serde::Serialize;
use serde_json::json;
use std::{convert::Infallible, time::Duration};

fn json_response(status: StatusCode, body: impl Serialize) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(
        status,
        json!({"schema_version":"harness-error/v1","error":{"code":code,"message":message}}),
    )
}
fn not_found() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "job_not_found",
        "任务不存在或已过期。任务保存在当前 worker 内存，重启后需使用结果包重新运行。",
    )
}

pub async fn create(
    State(shared): State<SharedState>,
    body: Result<Json<MacroCompileRequestV1>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(req)) => req,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "任务输入格式无效。",
            )
        }
    };
    if let Err(message) = request.validate() {
        return error(StatusCode::BAD_REQUEST, "invalid_request", message);
    }
    let _admission = shared.harness_jobs.admission.lock().await;
    if shared.harness_jobs.active() || shared.harness_runs.active() {
        return error(
            StatusCode::CONFLICT,
            "job_conflict",
            "已有武学助手任务正在运行，请等待完成或取消。",
        );
    }
    if legacy_busy(&shared).await {
        return error(
            StatusCode::CONFLICT,
            "compute_busy",
            "当前有配装搜索、宏优化或训练任务，请先结束该计算任务。",
        );
    }
    let runtime = AgentRuntime::load(&shared).await;
    if request.version != runtime.game_version() || request.mount != runtime.mount() {
        return error(
            StatusCode::CONFLICT,
            "runtime_mismatch",
            "版本或心法已变化，请重新读取当前技能轴。",
        );
    }
    let scenario = match request.snapshot(&runtime) {
        Ok(s) => s,
        Err(msg) => return error(StatusCode::BAD_REQUEST, "invalid_scenario", msg),
    };
    // Executable hashing is cached and runs off the async I/O executor.
    let frozen = tokio::task::spawn_blocking(move || {
        let hash = contract::runtime_hash(&runtime)?;
        Ok::<_, &'static str>((runtime, hash))
    })
    .await;
    let (runtime, runtime_hash) = match frozen {
        Ok(Ok(value)) => value,
        _ => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "runtime_identity_failed",
                "无法建立运行环境身份。",
            )
        }
    };
    let experiment_hash = match contract::experiment_hash(&request, &scenario, &runtime_hash) {
        Ok(hash) => hash,
        Err(msg) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "experiment_identity_failed",
                msg,
            )
        }
    };
    let engine = json!({"algorithm":contract::ALGORITHM_VERSION,"provenance":runtime.provenance(),"executable_hash":contract::executable_hash()});
    let record =
        match shared
            .harness_jobs
            .insert(request, scenario, runtime_hash, experiment_hash, engine)
        {
            Ok(record) => record,
            Err(msg) => return error(StatusCode::CONFLICT, "job_conflict", msg),
        };
    let snapshot = record.snapshot();
    let base = format!("/api/harness/jobs/{}", record.id);
    let body = json!({"schema_version":"harness-job-created/v1","job_id":record.id,
        "scenario_hash":snapshot.scenario_hash,"experiment_hash":snapshot.experiment_hash,"status":snapshot.status,
        "status_url":base,"events_url":format!("{base}/events"),"cancel_url":format!("{base}/cancel"),"artifacts_url":format!("{base}/artifacts")});
    JobManager::execute(record, runtime);
    json_response(StatusCode::ACCEPTED, body)
}

pub async fn list(State(shared): State<SharedState>) -> Response {
    json_response(
        StatusCode::OK,
        json!({"schema_version":"harness-jobs/v1","jobs":shared.harness_jobs.list()}),
    )
}
pub async fn status(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = shared.harness_jobs.get(&id) else {
        return not_found();
    };
    json_response(StatusCode::OK, record.snapshot())
}
pub async fn cancel(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = shared.harness_jobs.get(&id) else {
        return not_found();
    };
    let accepted = record.request_cancel();
    json_response(
        StatusCode::OK,
        json!({"job_id":id,"accepted":accepted,"already_terminal":!accepted}),
    )
}
pub async fn artifacts(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = shared.harness_jobs.get(&id) else {
        return not_found();
    };
    let snapshot = record.snapshot();
    json_response(
        StatusCode::OK,
        json!({"schema_version":"harness-artifacts/v1","job_id":id,
        "status":snapshot.status,"scenario_hash":snapshot.scenario_hash,"experiment_hash":snapshot.experiment_hash,
        "request":record.request,"scenario":record.scenario,"runtime_hash":record.runtime_hash,"engine":record.engine,
        "result":snapshot.result,"last_progress":snapshot.progress,"error":snapshot.error,
        "scope":"current_frozen_scenario","storage":"worker_memory","game_verified":false}),
    )
}
pub async fn events(State(shared): State<SharedState>, Path(id): Path<String>) -> Response {
    let Some(record) = shared.harness_jobs.get(&id) else {
        return not_found();
    };
    let receiver = record.subscribe();
    // Each item is a complete latest snapshot. Reconnection does not require
    // replaying an unbounded log, and slow clients can skip intermediate frames.
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
            let terminal = !snapshot.running;
            let data = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into());
            let event = Event::default()
                .id(snapshot.sequence.to_string())
                .event(if terminal { "completed" } else { "progress" })
                .data(data);
            Some((Ok::<_, Infallible>(event), (receiver, false, terminal)))
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

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
    request: Request,
    next: Next,
) -> Response {
    let _admission = shared.harness_jobs.admission.lock().await;
    if shared.harness_jobs.active() || shared.harness_runs.active() || legacy_busy(&shared).await {
        // Consume a bounded POST body before an early HTTP/1 rejection. Dropping
        // an unread request can reset the connection on Windows instead of
        // delivering the structured 409 to the client.
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024),
        )
        .await;
        return error(
            StatusCode::CONFLICT,
            "compute_busy",
            "已有计算任务正在运行，请先完成或取消该任务。",
        );
    }
    next.run(request).await
}
