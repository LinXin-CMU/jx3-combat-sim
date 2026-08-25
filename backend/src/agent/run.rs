use axum::{
    extract::{rejection::JsonRejection, Json, Path, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures_util::{stream, Stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, Mutex, Notify};

use crate::{SharedState, SimulateRequest};

use super::{
    orchestrator::{
        run_agent_observed, AgentCancellation, AgentRunInput, AgentRunLimits, AgentRunResultV1,
        AgentRunStatus, AgentTraceEventV1, AgentTraceSink,
    },
    provider::LlmProvider,
    AgentRuntime, ScenarioSnapshotV1,
};

pub const AGENT_RUN_CREATED_SCHEMA_V1: &str = "agent-run-created/v1";
pub const AGENT_RUN_STATUS_SCHEMA_V1: &str = "agent-run-status/v1";
pub const AGENT_RUN_STREAM_EVENT_SCHEMA_V1: &str = "agent-run-stream-event/v1";
pub const AGENT_RUN_ERROR_SCHEMA_V1: &str = "agent-run-error/v1";
const EVENT_CAPACITY: usize = 64;
const MAX_TRANSIENT_RUNS: usize = 16;
const MAX_QUESTION_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRunRequest {
    pub question: String,
    pub provider_profile: String,
    pub simulation: SimulateRequest,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRunResponse {
    pub schema_version: &'static str,
    pub run_id: String,
    pub scenario_hash: String,
    pub status: &'static str,
    pub stream_url: String,
    pub status_url: String,
    pub cancel_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentRunStreamEventV1 {
    pub schema_version: String,
    pub run_id: String,
    pub sequence: u32,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AgentRunResultV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunStatusResponseV1 {
    pub schema_version: &'static str,
    pub run_id: String,
    pub scenario_hash: String,
    pub running: bool,
    pub cancellation_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentRunStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AgentRunResultV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelAgentRunResponseV1 {
    pub schema_version: &'static str,
    pub run_id: String,
    pub accepted: bool,
    pub already_terminal: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentRunErrorEnvelope {
    schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    error: AgentRunApiError,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentRunApiError {
    code: &'static str,
    message: &'static str,
}

#[derive(Default)]
struct RunLifecycle {
    cancellation_requested: bool,
    result: Option<AgentRunResultV1>,
}

struct AgentRunRecord {
    run_id: String,
    scenario_hash: String,
    cancellation: AgentCancellation,
    lifecycle: StdMutex<RunLifecycle>,
    events: StdMutex<Vec<AgentRunStreamEventV1>>,
    broadcaster: broadcast::Sender<AgentRunStreamEventV1>,
    terminal_notify: Notify,
}

impl AgentRunRecord {
    fn new(run_id: String, scenario_hash: String) -> Arc<Self> {
        let (broadcaster, _) = broadcast::channel(EVENT_CAPACITY);
        Arc::new(Self {
            run_id,
            scenario_hash,
            cancellation: AgentCancellation::default(),
            lifecycle: StdMutex::new(RunLifecycle::default()),
            events: StdMutex::new(Vec::new()),
            broadcaster,
            terminal_notify: Notify::new(),
        })
    }

    fn publish_trace(&self, event: AgentTraceEventV1) {
        self.publish(AgentRunStreamEventV1 {
            schema_version: AGENT_RUN_STREAM_EVENT_SCHEMA_V1.to_string(),
            run_id: self.run_id.clone(),
            sequence: 0,
            kind: event.kind,
            tool_name: event.tool_name,
            evidence_ids: event.evidence_ids,
            code: event.code,
            result: None,
        });
    }

    fn publish_control(&self, kind: &str) {
        self.publish(AgentRunStreamEventV1 {
            schema_version: AGENT_RUN_STREAM_EVENT_SCHEMA_V1.to_string(),
            run_id: self.run_id.clone(),
            sequence: 0,
            kind: kind.to_string(),
            tool_name: None,
            evidence_ids: Vec::new(),
            code: None,
            result: None,
        });
    }

    fn publish(&self, mut event: AgentRunStreamEventV1) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event.sequence = events.len() as u32 + 1;
        events.push(event.clone());
        let _ = self.broadcaster.send(event);
    }

    fn complete(&self, result: AgentRunResultV1) {
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if lifecycle.result.is_some() {
                return;
            }
            lifecycle.result = Some(result.clone());
        }
        self.publish(AgentRunStreamEventV1 {
            schema_version: AGENT_RUN_STREAM_EVENT_SCHEMA_V1.to_string(),
            run_id: self.run_id.clone(),
            sequence: 0,
            kind: "run_result".to_string(),
            tool_name: None,
            evidence_ids: result
                .report
                .as_ref()
                .map(|report| report.evidence_ids.clone())
                .unwrap_or_default(),
            code: result.error.as_ref().map(|error| error.code.clone()),
            result: Some(result),
        });
        self.terminal_notify.notify_waiters();
    }

    fn request_cancel(&self) -> bool {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lifecycle.result.is_some() {
            return false;
        }
        if !lifecycle.cancellation_requested {
            lifecycle.cancellation_requested = true;
            self.cancellation.cancel();
            self.publish_control("cancel_requested");
        }
        true
    }

    fn status(&self) -> AgentRunStatusResponseV1 {
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        AgentRunStatusResponseV1 {
            schema_version: AGENT_RUN_STATUS_SCHEMA_V1,
            run_id: self.run_id.clone(),
            scenario_hash: self.scenario_hash.clone(),
            running: lifecycle.result.is_none(),
            cancellation_requested: lifecycle.cancellation_requested,
            status: lifecycle
                .result
                .as_ref()
                .map(|result| result.status.clone()),
            result: lifecycle.result.clone(),
        }
    }

    fn snapshot_and_subscribe(
        &self,
    ) -> (
        Vec<AgentRunStreamEventV1>,
        broadcast::Receiver<AgentRunStreamEventV1>,
    ) {
        let receiver = self.broadcaster.subscribe();
        let snapshot = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        (snapshot, receiver)
    }

    #[cfg(test)]
    async fn wait_result(&self) -> AgentRunResultV1 {
        loop {
            let notified = self.terminal_notify.notified();
            if let Some(result) = self.status().result {
                return result;
            }
            notified.await;
        }
    }
}

#[derive(Default)]
struct RunManagerState {
    active_run_id: Option<String>,
    runs: HashMap<String, Arc<AgentRunRecord>>,
    order: VecDeque<String>,
}

pub struct AgentRunManager {
    state: Mutex<RunManagerState>,
    counter: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartRunError {
    pub code: &'static str,
    pub message: &'static str,
}

impl AgentRunManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RunManagerState::default()),
            counter: AtomicU64::new(0),
        })
    }

    pub fn next_run_id(&self) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("run-{millis:x}-{counter:x}")
    }

    async fn start(
        self: &Arc<Self>,
        provider: Box<dyn LlmProvider>,
        runtime: AgentRuntime,
        input: AgentRunInput,
        limits: AgentRunLimits,
    ) -> Result<Arc<AgentRunRecord>, StartRunError> {
        let record =
            AgentRunRecord::new(input.run_id.clone(), input.scenario.scenario_hash.clone());
        {
            let mut state = self.state.lock().await;
            if state.active_run_id.is_some() {
                return Err(StartRunError {
                    code: "agent_run_conflict",
                    message: "another Agent run is already active for this worker",
                });
            }
            state.active_run_id = Some(input.run_id.clone());
            state.order.push_back(input.run_id.clone());
            state.runs.insert(input.run_id.clone(), record.clone());
        }

        let manager = self.clone();
        let task_record = record.clone();
        tokio::spawn(async move {
            let sink_record = task_record.clone();
            let sink: AgentTraceSink = Arc::new(move |event| sink_record.publish_trace(event));
            let result = run_agent_observed(
                provider.as_ref(),
                &runtime,
                input,
                limits,
                task_record.cancellation.clone(),
                Some(sink),
            )
            .await;
            let run_id = result.run_id.clone();
            task_record.complete(result);
            manager.finish(&run_id).await;
        });
        Ok(record)
    }

    async fn finish(&self, run_id: &str) {
        let mut state = self.state.lock().await;
        if state.active_run_id.as_deref() == Some(run_id) {
            state.active_run_id = None;
        }
        while state.runs.len() > MAX_TRANSIENT_RUNS {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if state.active_run_id.as_deref() == Some(oldest.as_str()) {
                state.order.push_back(oldest);
                break;
            }
            state.runs.remove(&oldest);
        }
    }

    async fn get(&self, run_id: &str) -> Option<Arc<AgentRunRecord>> {
        self.state.lock().await.runs.get(run_id).cloned()
    }
}

pub async fn create_run_handler(
    State(state): State<SharedState>,
    payload: Result<Json<CreateAgentRunRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(request) => request,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                None,
                "invalid_json",
                "request body is invalid",
            )
        }
    };
    if !valid_question(&request.question) {
        return error_response(
            StatusCode::BAD_REQUEST,
            None,
            "invalid_question",
            "question must be non-empty and within the configured limit",
        );
    }
    let provider = match state
        .agent_providers
        .create_provider(&request.provider_profile)
    {
        Ok(provider) => provider,
        Err(error) => {
            let status = if error.code == "provider_key_unavailable" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::BAD_REQUEST
            };
            return error_response(status, None, error.code, error.message);
        }
    };
    let runtime = AgentRuntime::load(&state).await;
    let scenario = match ScenarioSnapshotV1::capture(
        runtime.game_version(),
        runtime.mount(),
        request.simulation,
    ) {
        Ok(scenario) => scenario,
        Err(_) => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                None,
                "invalid_scenario",
                "simulation cannot be captured as an immutable Agent scenario",
            )
        }
    };
    let run_id = state.agent_runs.next_run_id();
    let input = AgentRunInput {
        run_id: run_id.clone(),
        question: request.question,
        scenario: scenario.clone(),
    };
    match state
        .agent_runs
        .start(provider, runtime, input, AgentRunLimits::default())
        .await
    {
        Ok(_) => json_response(
            StatusCode::ACCEPTED,
            CreateAgentRunResponse {
                schema_version: AGENT_RUN_CREATED_SCHEMA_V1,
                run_id: run_id.clone(),
                scenario_hash: scenario.scenario_hash,
                status: "accepted",
                stream_url: format!("/api/agent/runs/{run_id}/stream"),
                status_url: format!("/api/agent/runs/{run_id}"),
                cancel_url: format!("/api/agent/runs/{run_id}/cancel"),
            },
        ),
        Err(error) => error_response(StatusCode::CONFLICT, None, error.code, error.message),
    }
}

pub async fn run_status_handler(
    State(state): State<SharedState>,
    Path(run_id): Path<String>,
) -> Response {
    match state.agent_runs.get(&run_id).await {
        Some(record) => json_response(StatusCode::OK, record.status()),
        None => run_not_found(&run_id),
    }
}

pub async fn cancel_run_handler(
    State(state): State<SharedState>,
    Path(run_id): Path<String>,
) -> Response {
    let Some(record) = state.agent_runs.get(&run_id).await else {
        return run_not_found(&run_id);
    };
    let accepted = record.request_cancel();
    json_response(
        StatusCode::OK,
        CancelAgentRunResponseV1 {
            schema_version: AGENT_RUN_STATUS_SCHEMA_V1,
            run_id,
            accepted,
            already_terminal: !accepted,
        },
    )
}

pub async fn run_stream_handler(
    State(state): State<SharedState>,
    Path(run_id): Path<String>,
) -> Response {
    let Some(record) = state.agent_runs.get(&run_id).await else {
        return run_not_found(&run_id);
    };
    let stream = event_stream(record);
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

fn event_stream(record: Arc<AgentRunRecord>) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    let (snapshot, receiver) = record.snapshot_and_subscribe();
    let replay = VecDeque::from(snapshot);
    stream::unfold(
        (record, replay, receiver, 0_u32, false),
        |(record, mut replay, mut receiver, mut last_sequence, done)| async move {
            if done {
                return None;
            }
            loop {
                let event = if let Some(event) = replay.pop_front() {
                    event
                } else {
                    match receiver.recv().await {
                        Ok(event) => event,
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let fresh = record
                                .events
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .iter()
                                .filter(|event| event.sequence > last_sequence)
                                .cloned()
                                .collect::<VecDeque<_>>();
                            replay = fresh;
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                };
                if event.sequence <= last_sequence {
                    continue;
                }
                last_sequence = event.sequence;
                let terminal = event.kind == "run_result";
                let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                let sse = SseEvent::default()
                    .id(event.sequence.to_string())
                    .event(event.kind.clone())
                    .data(data);
                return Some((Ok(sse), (record, replay, receiver, last_sequence, terminal)));
            }
        },
    )
}

fn valid_question(question: &str) -> bool {
    !question.trim().is_empty()
        && question.len() <= MAX_QUESTION_BYTES
        && !question
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn run_not_found(run_id: &str) -> Response {
    let safe_id = run_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        .then_some(run_id)
        .filter(|value| value.len() <= 64)
        .map(|value| value.to_string());
    error_response(
        StatusCode::NOT_FOUND,
        safe_id,
        "agent_run_not_found",
        "Agent run was not found; transient runs do not survive worker restart",
    )
}

fn error_response(
    status: StatusCode,
    run_id: Option<String>,
    code: &'static str,
    message: &'static str,
) -> Response {
    json_response(
        status,
        AgentRunErrorEnvelope {
            schema_version: AGENT_RUN_ERROR_SCHEMA_V1,
            run_id,
            error: AgentRunApiError { code, message },
        },
    )
}

fn json_response<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::FakeProvider;

    #[tokio::test]
    async fn manager_completes_fake_run_and_replays_terminal_stream() {
        let manager = AgentRunManager::new();
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let run_id = "run-manager-success".to_string();
        let input = AgentRunInput {
            run_id: run_id.clone(),
            question: "分析当前循环。".to_string(),
            scenario,
        };
        let record = manager
            .start(
                Box::new(FakeProvider::new(
                    "offline".to_string(),
                    "fixture-v1".to_string(),
                )),
                runtime,
                input,
                AgentRunLimits::default(),
            )
            .await
            .unwrap();
        let result = record.wait_result().await;
        assert_eq!(result.status, AgentRunStatus::Completed);
        let snapshot = record.snapshot_and_subscribe().0;
        assert_eq!(snapshot.first().unwrap().kind, "planning");
        assert_eq!(snapshot.last().unwrap().kind, "run_result");
        assert!(snapshot
            .windows(2)
            .all(|pair| pair[0].sequence + 1 == pair[1].sequence));
    }

    #[tokio::test]
    async fn manager_rejects_a_second_active_run_and_cancel_is_idempotent() {
        use crate::agent::provider::{ModelRequest, ModelResponse, ProviderError};
        use async_trait::async_trait;

        struct PendingProvider;
        #[async_trait]
        impl LlmProvider for PendingProvider {
            fn profile_id(&self) -> &str {
                "pending"
            }
            fn model(&self) -> &str {
                "pending-v1"
            }
            async fn complete(
                &self,
                _request: &ModelRequest,
            ) -> Result<ModelResponse, ProviderError> {
                std::future::pending().await
            }
        }

        let manager = AgentRunManager::new();
        let runtime = AgentRuntime::fixture();
        let first = AgentRunInput {
            run_id: "run-active-first".to_string(),
            question: "等待取消。".to_string(),
            scenario: runtime.fixture_scenario(),
        };
        let record = manager
            .start(
                Box::new(PendingProvider),
                runtime,
                first,
                AgentRunLimits::default(),
            )
            .await
            .unwrap();

        let second_runtime = AgentRuntime::fixture();
        let second = AgentRunInput {
            run_id: "run-active-second".to_string(),
            question: "不应启动。".to_string(),
            scenario: second_runtime.fixture_scenario(),
        };
        let error = match manager
            .start(
                Box::new(PendingProvider),
                second_runtime,
                second,
                AgentRunLimits::default(),
            )
            .await
        {
            Ok(_) => panic!("second active run must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, "agent_run_conflict");
        assert!(record.request_cancel());
        assert!(record.request_cancel());
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), record.wait_result())
            .await
            .unwrap();
        assert_eq!(result.status, AgentRunStatus::Cancelled);
        assert_eq!(
            record
                .snapshot_and_subscribe()
                .0
                .iter()
                .filter(|event| event.kind == "cancel_requested")
                .count(),
            1
        );
    }

    #[test]
    fn question_validation_allows_multiline_but_rejects_unsafe_controls() {
        assert!(valid_question("比较当前场景。\n说明局限。"));
        assert!(!valid_question("bad\u{0000}question"));
    }
}
