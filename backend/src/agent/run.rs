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
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, Mutex, Notify};

use crate::{SharedState, SimulateRequest};

use super::{
    orchestrator::{
        run_agent_recorded, AgentCancellation, AgentReplayEventV1, AgentReplaySink, AgentRunInput,
        AgentRunLimits, AgentRunResultV1, AgentRunStatus, AgentTraceEventV1, AgentTraceSink,
    },
    provider::LlmProvider,
    session::{contains_likely_secret, AgentSessionEventV1, AgentSessionStore},
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
    #[serde(default)]
    pub session_id: Option<String>,
    pub simulation: SimulateRequest,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRunResponse {
    pub schema_version: &'static str,
    pub run_id: String,
    pub session_id: String,
    pub scenario_hash: String,
    pub status: &'static str,
    pub stream_url: String,
    pub status_url: String,
    pub cancel_url: String,
    pub session_url: String,
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
    pub stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playbook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AgentRunResultV1>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub persistence_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunStatusResponseV1 {
    pub schema_version: &'static str,
    pub run_id: String,
    pub session_id: String,
    pub scenario_hash: String,
    pub running: bool,
    pub cancellation_requested: bool,
    pub persistence_error: bool,
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
    session_id: String,
    scenario_hash: String,
    sessions: Arc<AgentSessionStore>,
    cancellation: AgentCancellation,
    persistence_error: AtomicBool,
    lifecycle: StdMutex<RunLifecycle>,
    events: StdMutex<Vec<AgentRunStreamEventV1>>,
    broadcaster: broadcast::Sender<AgentRunStreamEventV1>,
    terminal_notify: Notify,
}

impl AgentRunRecord {
    fn new(
        run_id: String,
        session_id: String,
        scenario_hash: String,
        sessions: Arc<AgentSessionStore>,
    ) -> Arc<Self> {
        let (broadcaster, _) = broadcast::channel(EVENT_CAPACITY);
        Arc::new(Self {
            run_id,
            session_id,
            scenario_hash,
            sessions,
            cancellation: AgentCancellation::default(),
            persistence_error: AtomicBool::new(false),
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
            stage_id: event.stage_id,
            label: event.label,
            overview: event.overview,
            playbook_id: event.playbook_id,
            result: None,
            persistence_error: false,
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
            stage_id: None,
            label: None,
            overview: None,
            playbook_id: None,
            result: None,
            persistence_error: false,
        });
    }

    fn publish_replay(&self, event: AgentReplayEventV1) {
        if self
            .sessions
            .append_replay_event(
                &self.session_id,
                &self.run_id,
                &event.kind,
                event.payload,
            )
            .is_err()
        {
            self.persistence_error.store(true, Ordering::SeqCst);
        }
    }

    fn publish(&self, mut event: AgentRunStreamEventV1) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event.sequence = events.len() as u32 + 1;
        let session_event = match event.result.as_ref() {
            Some(result) => AgentSessionEventV1::run_result(result),
            None if event.kind == "cancel_requested" => {
                AgentSessionEventV1::control(&self.run_id, "cancel_requested")
            }
            None => AgentSessionEventV1::trace(
                &self.run_id,
                &AgentTraceEventV1 {
                    sequence: event.sequence,
                    kind: event.kind.clone(),
                    tool_name: event.tool_name.clone(),
                    evidence_ids: event.evidence_ids.clone(),
                    code: event.code.clone(),
                    stage_id: event.stage_id.clone(),
                    label: event.label.clone(),
                    overview: event.overview.clone(),
                    playbook_id: event.playbook_id.clone(),
                },
            ),
        };
        if self
            .sessions
            .append_event(&self.session_id, session_event)
            .is_err()
        {
            self.persistence_error.store(true, Ordering::SeqCst);
            event.persistence_error = true;
        }
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
        self.publish_replay(AgentReplayEventV1 {
            kind: "run_result".to_string(),
            payload: serde_json::to_value(&result).unwrap_or_else(|_| serde_json::json!({})),
        });
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
            stage_id: None,
            label: None,
            overview: None,
            playbook_id: None,
            result: Some(result),
            persistence_error: false,
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
            session_id: self.session_id.clone(),
            scenario_hash: self.scenario_hash.clone(),
            running: lifecycle.result.is_none(),
            cancellation_requested: lifecycle.cancellation_requested,
            persistence_error: self.persistence_error.load(Ordering::SeqCst),
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
    sessions: Arc<AgentSessionStore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartRunError {
    pub code: &'static str,
    pub message: &'static str,
}

impl AgentRunManager {
    pub fn new(sessions: Arc<AgentSessionStore>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RunManagerState::default()),
            counter: AtomicU64::new(0),
            sessions,
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
        mut input: AgentRunInput,
        limits: AgentRunLimits,
        requested_session_id: Option<&str>,
    ) -> Result<Arc<AgentRunRecord>, StartRunError> {
        let provider_profile = provider.profile_id().to_string();
        let model = provider.model().to_string();
        {
            let mut state = self.state.lock().await;
            if state.active_run_id.is_some() {
                return Err(StartRunError {
                    code: "agent_run_conflict",
                    message: "another Agent run is already active for this worker",
                });
            }
            let binding = self
                .sessions
                .create_or_resume_run(
                    requested_session_id,
                    &input.run_id,
                    &input.question,
                    &input.scenario.scenario_hash,
                    &provider_profile,
                    &model,
                )
                .map_err(|error| StartRunError {
                    code: error.code,
                    message: error.message,
                })?;
            input.session_context = binding.prior_context;
            input.session_playbook_id = binding.prior_playbook_id;
            let record = AgentRunRecord::new(
                input.run_id.clone(),
                binding.session_id,
                input.scenario.scenario_hash.clone(),
                self.sessions.clone(),
            );
            state.active_run_id = Some(input.run_id.clone());
            state.order.push_back(input.run_id.clone());
            state.runs.insert(input.run_id.clone(), record.clone());
            drop(state);

            let manager = self.clone();
            let task_record = record.clone();
            tokio::spawn(async move {
                let sink_record = task_record.clone();
                let sink: AgentTraceSink = Arc::new(move |event| sink_record.publish_trace(event));
                let replay_record = task_record.clone();
                let replay_sink: AgentReplaySink =
                    Arc::new(move |event| replay_record.publish_replay(event));
                let result = run_agent_recorded(
                    provider.as_ref(),
                    &runtime,
                    input,
                    limits,
                    task_record.cancellation.clone(),
                    Some(sink),
                    Some(replay_sink),
                )
                .await;
                let run_id = result.run_id.clone();
                task_record.complete(result);
                manager.finish(&run_id).await;
            });
            return Ok(record);
        }
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
    if contains_likely_secret(&request.question) {
        return error_response(
            StatusCode::BAD_REQUEST,
            None,
            "sensitive_input_rejected",
            "question appears to contain a credential; remove it before starting the Agent",
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
        session_context: None,
        session_playbook_id: None,
    };
    match state
        .agent_runs
        .start(
            provider,
            runtime,
            input,
            AgentRunLimits::default(),
            request.session_id.as_deref(),
        )
        .await
    {
        Ok(record) => json_response(
            StatusCode::ACCEPTED,
            CreateAgentRunResponse {
                schema_version: AGENT_RUN_CREATED_SCHEMA_V1,
                run_id: run_id.clone(),
                session_id: record.session_id.clone(),
                scenario_hash: scenario.scenario_hash,
                status: "accepted",
                stream_url: format!("/api/agent/runs/{run_id}/stream"),
                status_url: format!("/api/agent/runs/{run_id}"),
                cancel_url: format!("/api/agent/runs/{run_id}/cancel"),
                session_url: format!("/api/agent/sessions/{}", record.session_id),
            },
        ),
        Err(error) => {
            let status = match error.code {
                "agent_run_conflict" | "agent_session_corrupted" => StatusCode::CONFLICT,
                "agent_session_not_found" => StatusCode::NOT_FOUND,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            error_response(status, None, error.code, error.message)
        }
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
    use std::path::PathBuf;

    fn test_manager(name: &str) -> (Arc<AgentRunManager>, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "jx3-agent-run-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let sessions = AgentSessionStore::open(root.clone()).unwrap();
        (AgentRunManager::new(sessions), root)
    }

    #[tokio::test]
    async fn manager_completes_fake_run_and_replays_terminal_stream() {
        let (manager, root) = test_manager("success");
        let runtime = AgentRuntime::fixture();
        let scenario = runtime.fixture_scenario();
        let run_id = "run-manager-success".to_string();
        let input = AgentRunInput {
            run_id: run_id.clone(),
            question: "分析当前循环。".to_string(),
            scenario,
            session_context: None,
            session_playbook_id: None,
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
                None,
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
        assert!(!record.status().persistence_error);
        let session = manager.sessions.load_session(&record.session_id).unwrap();
        assert_eq!(session.summary.status, "completed");
        let public_json = serde_json::to_string(&session).unwrap();
        assert!(!public_json.contains("model_request"));
        assert!(!public_json.contains("prompt_instructions"));
        let replay_dir = root
            .join("agent_sessions")
            .join("v1")
            .join(&record.session_id)
            .join("_private")
            .join("replay")
            .join("v1")
            .join(&run_id)
            .join("events");
        let replay = std::fs::read_dir(replay_dir)
            .unwrap()
            .flatten()
            .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        for kind in [
            "run_input",
            "analysis_plan",
            "tool_dispatch",
            "model_request",
            "model_response",
            "report_validation",
            "run_result",
        ] {
            assert!(replay.contains(kind), "missing replay event: {kind}");
        }
        let _ = std::fs::remove_dir_all(root);
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

        let (manager, root) = test_manager("cancel");
        let runtime = AgentRuntime::fixture();
        let first = AgentRunInput {
            run_id: "run-active-first".to_string(),
            question: "等待取消。".to_string(),
            scenario: runtime.fixture_scenario(),
            session_context: None,
            session_playbook_id: None,
        };
        let record = manager
            .start(
                Box::new(PendingProvider),
                runtime,
                first,
                AgentRunLimits::default(),
                None,
            )
            .await
            .unwrap();

        let second_runtime = AgentRuntime::fixture();
        let second = AgentRunInput {
            run_id: "run-active-second".to_string(),
            question: "不应启动。".to_string(),
            scenario: second_runtime.fixture_scenario(),
            session_context: None,
            session_playbook_id: None,
        };
        let error = match manager
            .start(
                Box::new(PendingProvider),
                second_runtime,
                second,
                AgentRunLimits::default(),
                None,
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn question_validation_allows_multiline_but_rejects_unsafe_controls() {
        assert!(valid_question("比较当前场景。\n说明局限。"));
        assert!(!valid_question("bad\u{0000}question"));
    }
}
