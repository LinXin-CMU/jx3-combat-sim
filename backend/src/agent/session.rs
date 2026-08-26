use axum::{
    extract::{Path, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path as FsPath, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::SharedState;

use super::orchestrator::{AgentRunResultV1, AgentRunStatus, AgentTraceEventV1};

pub const AGENT_SESSION_META_SCHEMA_V1: &str = "agent-session-meta/v1";
pub const AGENT_SESSION_EVENT_SCHEMA_V1: &str = "agent-session-event/v1";
pub const AGENT_SESSION_LIST_SCHEMA_V1: &str = "agent-session-list/v1";
pub const AGENT_SESSION_DETAIL_SCHEMA_V1: &str = "agent-session-detail/v1";
pub const AGENT_SESSION_ERROR_SCHEMA_V1: &str = "agent-session-error/v1";
const MAX_SESSION_ID_BYTES: usize = 64;
const MAX_TITLE_CHARS: usize = 80;
const MAX_SESSIONS_RETURNED: usize = 200;
const MAX_CONTEXT_TURNS: usize = 2;
const MAX_CONTEXT_FIELD_CHARS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionMetaV1 {
    pub schema_version: String,
    pub session_id: String,
    pub title: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionEventV1 {
    pub schema_version: String,
    pub sequence: u32,
    pub timestamp_ms: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AgentRunResultV1>,
}

impl AgentSessionEventV1 {
    fn empty(kind: &str) -> Self {
        Self {
            schema_version: AGENT_SESSION_EVENT_SCHEMA_V1.to_string(),
            sequence: 0,
            timestamp_ms: now_ms(),
            kind: kind.to_string(),
            run_id: None,
            parent_run_id: None,
            question: None,
            scenario_hash: None,
            provider_profile: None,
            model: None,
            trace_kind: None,
            tool_name: None,
            evidence_ids: Vec::new(),
            code: None,
            result: None,
        }
    }

    pub fn user_message(run_id: &str, question: &str) -> Self {
        let mut event = Self::empty("user_message");
        event.run_id = Some(run_id.to_string());
        event.question = Some(redact_sensitive_text(question));
        event
    }

    pub fn run_started(
        run_id: &str,
        parent_run_id: Option<String>,
        scenario_hash: &str,
        provider_profile: &str,
        model: &str,
    ) -> Self {
        let mut event = Self::empty("run_started");
        event.run_id = Some(run_id.to_string());
        event.parent_run_id = parent_run_id;
        event.scenario_hash = Some(scenario_hash.to_string());
        event.provider_profile = Some(provider_profile.to_string());
        event.model = Some(model.to_string());
        event
    }

    pub fn trace(run_id: &str, trace: &AgentTraceEventV1) -> Self {
        let mut event = Self::empty("run_trace");
        event.run_id = Some(run_id.to_string());
        event.trace_kind = Some(trace.kind.clone());
        event.tool_name = trace.tool_name.clone();
        event.evidence_ids = trace.evidence_ids.clone();
        event.code = trace.code.clone();
        event
    }

    pub fn control(run_id: &str, kind: &str) -> Self {
        let mut event = Self::empty(kind);
        event.run_id = Some(run_id.to_string());
        event
    }

    pub fn run_result(result: &AgentRunResultV1) -> Self {
        let mut event = Self::empty("run_result");
        event.run_id = Some(result.run_id.clone());
        event.scenario_hash = Some(result.scenario_hash.clone());
        event.provider_profile = Some(result.provider_profile.clone());
        event.model = Some(result.model.clone());
        event.evidence_ids = result
            .report
            .as_ref()
            .map(|report| report.evidence_ids.clone())
            .unwrap_or_default();
        event.code = result.error.as_ref().map(|error| error.code.clone());
        event.result = Some(result.clone());
        event
    }

    fn interrupted(run_id: &str) -> Self {
        let mut event = Self::empty("run_interrupted");
        event.run_id = Some(run_id.to_string());
        event.code = Some("worker_restarted".to_string());
        event
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionSummaryV1 {
    pub session_id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_hash: Option<String>,
    pub event_count: usize,
    pub corrupted_event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionDetailV1 {
    pub schema_version: String,
    pub meta: AgentSessionMetaV1,
    pub summary: AgentSessionSummaryV1,
    pub events: Vec<AgentSessionEventV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionRunBinding {
    pub session_id: String,
    pub prior_context: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionListResponseV1 {
    pub schema_version: &'static str,
    pub sessions: Vec<AgentSessionSummaryV1>,
}

#[derive(Debug, Clone)]
pub struct SessionStoreError {
    pub code: &'static str,
    pub message: &'static str,
}

impl std::fmt::Display for SessionStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for SessionStoreError {}

#[derive(Debug)]
pub struct AgentSessionStore {
    root: PathBuf,
    available: bool,
    write_gate: Mutex<()>,
    counter: AtomicU64,
}

impl AgentSessionStore {
    pub fn open(userdata_root: PathBuf) -> Result<Arc<Self>, SessionStoreError> {
        let root = userdata_root.join("agent_sessions").join("v1");
        fs::create_dir_all(&root).map_err(|_| store_unavailable())?;
        let store = Arc::new(Self {
            root,
            available: true,
            write_gate: Mutex::new(()),
            counter: AtomicU64::new(0),
        });
        store.recover_interrupted_sessions()?;
        Ok(store)
    }

    pub fn unavailable(userdata_root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root: userdata_root.join("agent_sessions").join("v1"),
            available: false,
            write_gate: Mutex::new(()),
            counter: AtomicU64::new(0),
        })
    }

    pub fn create_or_resume_run(
        &self,
        requested_session_id: Option<&str>,
        run_id: &str,
        question: &str,
        scenario_hash: &str,
        provider_profile: &str,
        model: &str,
    ) -> Result<AgentSessionRunBinding, SessionStoreError> {
        self.ensure_available()?;
        let _guard = self
            .write_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (session_id, parent_run_id, prior_context) = match requested_session_id {
            Some(session_id) => {
                validate_session_id(session_id)?;
                let detail = self.load_session_unlocked(session_id)?;
                if detail.summary.corrupted_event_count > 0 {
                    return Err(error(
                        "agent_session_corrupted",
                        "session contains corrupted events; start a new session instead",
                    ));
                }
                let prior_context = build_prior_context(&detail.events);
                (
                    session_id.to_string(),
                    detail.summary.last_run_id,
                    prior_context,
                )
            }
            None => {
                let session_id = self.create_session_unlocked(question)?;
                (session_id, None, None)
            }
        };
        self.append_event_unlocked(
            &session_id,
            AgentSessionEventV1::user_message(run_id, question),
        )?;
        self.append_event_unlocked(
            &session_id,
            AgentSessionEventV1::run_started(
                run_id,
                parent_run_id,
                scenario_hash,
                provider_profile,
                model,
            ),
        )?;
        Ok(AgentSessionRunBinding {
            session_id,
            prior_context,
        })
    }

    pub fn append_event(
        &self,
        session_id: &str,
        event: AgentSessionEventV1,
    ) -> Result<(), SessionStoreError> {
        self.ensure_available()?;
        validate_session_id(session_id)?;
        let _guard = self
            .write_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.append_event_unlocked(session_id, event).map(|_| ())
    }

    pub fn list_sessions(&self) -> Result<Vec<AgentSessionSummaryV1>, SessionStoreError> {
        self.ensure_available()?;
        let _guard = self
            .write_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut sessions = Vec::new();
        let entries = fs::read_dir(&self.root).map_err(|_| store_unavailable())?;
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let session_id = entry.file_name().to_string_lossy().to_string();
            if !is_valid_identifier(&session_id) {
                continue;
            }
            if let Ok(detail) = self.load_session_unlocked(&session_id) {
                sessions.push(detail.summary);
            }
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| right.session_id.cmp(&left.session_id))
        });
        sessions.truncate(MAX_SESSIONS_RETURNED);
        Ok(sessions)
    }

    pub fn load_session(
        &self,
        session_id: &str,
    ) -> Result<AgentSessionDetailV1, SessionStoreError> {
        self.ensure_available()?;
        validate_session_id(session_id)?;
        let _guard = self
            .write_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.load_session_unlocked(session_id)
    }

    fn create_session_unlocked(&self, question: &str) -> Result<String, SessionStoreError> {
        for _ in 0..32 {
            let session_id = self.next_session_id();
            let session_dir = self.root.join(&session_id);
            match fs::create_dir(&session_dir) {
                Ok(()) => {
                    fs::create_dir(session_dir.join("events")).map_err(|_| store_unavailable())?;
                    let meta = AgentSessionMetaV1 {
                        schema_version: AGENT_SESSION_META_SCHEMA_V1.to_string(),
                        session_id: session_id.clone(),
                        title: session_title(question),
                        created_at_ms: now_ms(),
                    };
                    write_new_json(&session_dir.join("meta.json"), &meta, &self.counter)?;
                    return Ok(session_id);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(store_unavailable()),
            }
        }
        Err(error(
            "agent_session_id_exhausted",
            "could not allocate a unique session id",
        ))
    }

    fn next_session_id(&self) -> String {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("session-{:x}-{:x}", now_ms(), counter)
    }

    fn append_event_unlocked(
        &self,
        session_id: &str,
        mut event: AgentSessionEventV1,
    ) -> Result<u32, SessionStoreError> {
        let session_dir = self.session_dir(session_id)?;
        let events_dir = session_dir.join("events");
        let sequence = next_event_sequence(&events_dir)?;
        event.sequence = sequence;
        event.timestamp_ms = now_ms();
        let path = events_dir.join(format!("{sequence:06}.json"));
        write_new_json(&path, &event, &self.counter)?;
        Ok(sequence)
    }

    fn load_session_unlocked(
        &self,
        session_id: &str,
    ) -> Result<AgentSessionDetailV1, SessionStoreError> {
        let session_dir = self.session_dir(session_id)?;
        let meta_bytes = fs::read(session_dir.join("meta.json"))
            .map_err(|_| error("agent_session_corrupted", "session metadata cannot be read"))?;
        let meta: AgentSessionMetaV1 = serde_json::from_slice(&meta_bytes)
            .map_err(|_| error("agent_session_corrupted", "session metadata is invalid"))?;
        if meta.schema_version != AGENT_SESSION_META_SCHEMA_V1 || meta.session_id != session_id {
            return Err(error(
                "agent_session_corrupted",
                "session metadata does not match its directory",
            ));
        }
        let (events, corrupted_event_count) = read_events(&session_dir.join("events"))?;
        let summary = summarize(&meta, &events, corrupted_event_count);
        Ok(AgentSessionDetailV1 {
            schema_version: AGENT_SESSION_DETAIL_SCHEMA_V1.to_string(),
            meta,
            summary,
            events,
        })
    }

    fn session_dir(&self, session_id: &str) -> Result<PathBuf, SessionStoreError> {
        validate_session_id(session_id)?;
        let path = self.root.join(session_id);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                session_not_found()
            } else {
                store_unavailable()
            }
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(session_not_found());
        }
        Ok(path)
    }

    fn recover_interrupted_sessions(&self) -> Result<(), SessionStoreError> {
        let _guard = self
            .write_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entries = fs::read_dir(&self.root).map_err(|_| store_unavailable())?;
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let session_id = entry.file_name().to_string_lossy().to_string();
            if !is_valid_identifier(&session_id) {
                continue;
            }
            let Ok(detail) = self.load_session_unlocked(&session_id) else {
                continue;
            };
            if detail.summary.status == "running" {
                if let Some(run_id) = detail.summary.last_run_id {
                    self.append_event_unlocked(
                        &session_id,
                        AgentSessionEventV1::interrupted(&run_id),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn ensure_available(&self) -> Result<(), SessionStoreError> {
        if self.available {
            Ok(())
        } else {
            Err(store_unavailable())
        }
    }
}

pub async fn list_sessions_handler(State(state): State<SharedState>) -> Response {
    match state.agent_sessions.list_sessions() {
        Ok(sessions) => json_response(
            StatusCode::OK,
            AgentSessionListResponseV1 {
                schema_version: AGENT_SESSION_LIST_SCHEMA_V1,
                sessions,
            },
        ),
        Err(error) => session_error_response(error),
    }
}

pub async fn get_session_handler(
    State(state): State<SharedState>,
    Path(session_id): Path<String>,
) -> Response {
    match state.agent_sessions.load_session(&session_id) {
        Ok(session) => json_response(StatusCode::OK, session),
        Err(error) => session_error_response(error),
    }
}

fn read_events(
    events_dir: &FsPath,
) -> Result<(Vec<AgentSessionEventV1>, usize), SessionStoreError> {
    let mut files = fs::read_dir(events_dir)
        .map_err(|_| store_unavailable())?
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() || file_type.is_symlink() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            parse_event_sequence(&name).map(|sequence| (sequence, entry.path()))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(sequence, _)| *sequence);
    let mut events = Vec::new();
    let mut corrupted = 0;
    for (expected_sequence, path) in files {
        let parsed = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AgentSessionEventV1>(&bytes).ok())
            .filter(|event| {
                event.schema_version == AGENT_SESSION_EVENT_SCHEMA_V1
                    && event.sequence == expected_sequence
            });
        match parsed {
            Some(event) => events.push(event),
            None => corrupted += 1,
        }
    }
    Ok((events, corrupted))
}

fn next_event_sequence(events_dir: &FsPath) -> Result<u32, SessionStoreError> {
    let max_sequence = fs::read_dir(events_dir)
        .map_err(|_| store_unavailable())?
        .flatten()
        .filter_map(|entry| parse_event_sequence(&entry.file_name().to_string_lossy()))
        .max()
        .unwrap_or(0);
    max_sequence
        .checked_add(1)
        .filter(|sequence| *sequence <= 999_999)
        .ok_or_else(|| {
            error(
                "agent_session_event_limit",
                "session event limit has been reached",
            )
        })
}

fn parse_event_sequence(name: &str) -> Option<u32> {
    if name.len() != 11 || !name.ends_with(".json") {
        return None;
    }
    let digits = &name[..6];
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| digits.parse::<u32>().ok())
        .flatten()
        .filter(|sequence| *sequence > 0)
}

fn summarize(
    meta: &AgentSessionMetaV1,
    events: &[AgentSessionEventV1],
    corrupted_event_count: usize,
) -> AgentSessionSummaryV1 {
    let mut status = "created".to_string();
    let mut last_run_id = None;
    let mut scenario_hash = None;
    for event in events {
        if let Some(run_id) = &event.run_id {
            last_run_id = Some(run_id.clone());
        }
        if let Some(hash) = &event.scenario_hash {
            scenario_hash = Some(hash.clone());
        }
        match event.kind.as_str() {
            "run_started" => status = "running".to_string(),
            "run_interrupted" => status = "interrupted".to_string(),
            "run_result" => {
                status = event
                    .result
                    .as_ref()
                    .map(|result| status_name(&result.status).to_string())
                    .unwrap_or_else(|| "finished".to_string());
            }
            _ => {}
        }
    }
    AgentSessionSummaryV1 {
        session_id: meta.session_id.clone(),
        title: meta.title.clone(),
        created_at_ms: meta.created_at_ms,
        updated_at_ms: events
            .last()
            .map(|event| event.timestamp_ms)
            .unwrap_or(meta.created_at_ms),
        status,
        last_run_id,
        scenario_hash,
        event_count: events.len(),
        corrupted_event_count,
    }
}

/// Rebuild only the last visible reports, never the raw provider transcript or
/// hidden reasoning. Current-run tools remain the sole source of numeric truth.
fn build_prior_context(events: &[AgentSessionEventV1]) -> Option<String> {
    let turns = events
        .iter()
        .rev()
        .filter_map(|event| event.result.as_ref())
        .filter_map(|result| {
            let report = result.report.as_ref()?;
            Some(serde_json::json!({
                "question": clipped(&report.question),
                "status": status_name(&result.status),
                "summary": clipped(&report.content.summary),
                "findings": report.content.findings.iter().take(4).map(|finding| serde_json::json!({
                    "title": clipped(&finding.title),
                    "explanation": clipped(&finding.explanation),
                    "metrics": finding.metrics.iter().take(6).map(|metric| serde_json::json!({
                        "label": clipped(&metric.label),
                        "value": metric.value,
                        "unit": clipped(&metric.unit),
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "recommendations": report.content.recommendations.iter().take(3).map(|recommendation| serde_json::json!({
                    "title": clipped(&recommendation.title),
                    "rationale": clipped(&recommendation.rationale),
                })).collect::<Vec<_>>(),
                "limitations": report.content.limitations.iter().take(3).map(|value| clipped(value)).collect::<Vec<_>>(),
                "refusal_reason": report.content.refusal_reason.as_deref().map(clipped),
            }))
        })
        .take(MAX_CONTEXT_TURNS)
        .collect::<Vec<_>>();
    if turns.is_empty() {
        return None;
    }
    let mut chronological = turns;
    chronological.reverse();
    serde_json::to_string(&serde_json::json!({
        "schema_version": "agent-session-context/v1",
        "turns": chronological,
    }))
    .ok()
}

fn clipped(value: &str) -> String {
    let mut output = value
        .chars()
        .take(MAX_CONTEXT_FIELD_CHARS)
        .collect::<String>();
    if value.chars().count() > MAX_CONTEXT_FIELD_CHARS {
        output.push('…');
    }
    output
}

fn status_name(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::Refused => "refused",
        AgentRunStatus::EvidenceInsufficient => "evidence_insufficient",
        AgentRunStatus::Cancelled => "cancelled",
        AgentRunStatus::BudgetExhausted => "budget_exhausted",
        AgentRunStatus::ProviderFailed => "provider_failed",
        AgentRunStatus::ProtocolFailed => "protocol_failed",
        AgentRunStatus::TimedOut => "timed_out",
    }
}

fn session_title(question: &str) -> String {
    let redacted = redact_sensitive_text(question);
    let compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = compact.chars().take(MAX_TITLE_CHARS).collect::<String>();
    if compact.chars().count() > MAX_TITLE_CHARS {
        title.push('…');
    }
    if title.is_empty() {
        "未命名分析".to_string()
    } else {
        title
    }
}

pub fn contains_likely_secret(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|token| looks_like_secret_token(trim_token(token)))
        || value.lines().any(line_has_secret_assignment)
}

fn redact_sensitive_text(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            if line_has_secret_assignment(line) {
                return "[REDACTED SENSITIVE VALUE]".to_string();
            }
            line.split_inclusive(char::is_whitespace)
                .map(|part| {
                    let token = trim_token(part);
                    if looks_like_secret_token(token) {
                        part.replacen(token, "[REDACTED]", 1)
                    } else {
                        part.to_string()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_token(token: &str) -> &str {
    token.trim_matches(|character: char| {
        matches!(
            character,
            '"' | '\'' | '`' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    })
}

fn looks_like_secret_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    (lower.starts_with("sk-") && token.len() >= 20)
        || (lower.starts_with("ghp_") && token.len() >= 20)
        || (lower.starts_with("github_pat_") && token.len() >= 24)
        || (lower.starts_with("bearer:") && token.len() >= 24)
}

fn line_has_secret_assignment(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let markers = [
        "api_key",
        "apikey",
        "authorization",
        "access_token",
        "refresh_token",
        "password",
        "client_secret",
    ];
    markers.iter().any(|marker| {
        lower.find(marker).is_some_and(|position| {
            let tail = &line[position + marker.len()..];
            let tail = tail.trim_start();
            (tail.starts_with('=') || tail.starts_with(':'))
                && tail
                    .get(1..)
                    .is_some_and(|value| value.trim().chars().count() >= 8)
        })
    })
}

fn write_new_json<T: Serialize>(
    final_path: &FsPath,
    value: &T,
    counter: &AtomicU64,
) -> Result<(), SessionStoreError> {
    let mut json = serde_json::to_value(value).map_err(|_| store_unavailable())?;
    redact_json_strings(&mut json);
    let bytes = serde_json::to_vec_pretty(&json).map_err(|_| store_unavailable())?;
    let parent = final_path.parent().ok_or_else(store_unavailable)?;
    let temp_name = format!(
        ".tmp-{}-{:x}",
        std::process::id(),
        counter.fetch_add(1, Ordering::Relaxed)
    );
    let temp_path = parent.join(temp_name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|_| store_unavailable())?;
    if file.write_all(&bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(store_unavailable());
    }
    drop(file);
    if fs::rename(&temp_path, final_path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(store_unavailable());
    }
    Ok(())
}

fn redact_json_strings(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_sensitive_text(text),
        Value::Array(items) => items.iter_mut().for_each(redact_json_strings),
        Value::Object(fields) => fields.values_mut().for_each(redact_json_strings),
        _ => {}
    }
}

fn validate_session_id(session_id: &str) -> Result<(), SessionStoreError> {
    if is_valid_identifier(session_id) {
        Ok(())
    } else {
        Err(session_not_found())
    }
}

fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn error(code: &'static str, message: &'static str) -> SessionStoreError {
    SessionStoreError { code, message }
}

fn store_unavailable() -> SessionStoreError {
    error(
        "agent_session_store_unavailable",
        "Agent session storage is unavailable",
    )
}

fn session_not_found() -> SessionStoreError {
    error("agent_session_not_found", "Agent session was not found")
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentSessionErrorEnvelope {
    schema_version: &'static str,
    error: AgentSessionApiError,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentSessionApiError {
    code: &'static str,
    message: &'static str,
}

fn session_error_response(error: SessionStoreError) -> Response {
    let status = match error.code {
        "agent_session_not_found" => StatusCode::NOT_FOUND,
        "agent_session_corrupted" => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    json_response(
        status,
        AgentSessionErrorEnvelope {
            schema_version: AGENT_SESSION_ERROR_SCHEMA_V1,
            error: AgentSessionApiError {
                code: error.code,
                message: error.message,
            },
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

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "jx3-agent-session-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn finish_event(run_id: &str) -> AgentSessionEventV1 {
        let mut event = AgentSessionEventV1::empty("run_result");
        event.run_id = Some(run_id.to_string());
        event
    }

    fn visible_result_event(run_id: &str, question: &str, summary: &str) -> AgentSessionEventV1 {
        let result: AgentRunResultV1 = serde_json::from_value(serde_json::json!({
            "schema_version": "agent-run/v1",
            "run_id": run_id,
            "scenario_hash": "scenario-context",
            "prompt_version": "v1",
            "prompt_sha256": "prompt-hash",
            "provider_profile": "offline",
            "model": "fixture-v1",
            "status": "completed",
            "accounting": {
                "model_turns": 1, "tool_calls": 1, "simulations": 1,
                "input_tokens": 0, "output_tokens": 0, "total_tokens": 0,
                "duration_ms": 1
            },
            "report": {
                "schema_version": "agent-report/v1",
                "question": question,
                "scenario_hash": "scenario-context",
                "prompt_version": "v1",
                "prompt_sha256": "prompt-hash",
                "provider_profile": "offline",
                "model": "fixture-v1",
                "content": {
                    "schema_version": "agent-report-content/v1",
                    "summary": summary,
                    "findings": [], "recommendations": [], "limitations": [],
                    "refusal_reason": null
                },
                "evidence_ids": [],
                "accounting": {
                    "model_turns": 1, "tool_calls": 1, "simulations": 1,
                    "input_tokens": 0, "output_tokens": 0, "total_tokens": 0,
                    "duration_ms": 1
                },
                "termination": "completed"
            },
            "trace": []
        }))
        .unwrap();
        AgentSessionEventV1::run_result(&result)
    }

    #[test]
    fn session_events_are_append_only_and_listable() {
        let root = temp_root("append");
        let store = AgentSessionStore::open(root.clone()).unwrap();
        let session_id = store
            .create_or_resume_run(
                None,
                "run-one",
                "比较当前循环",
                "scenario-one",
                "offline",
                "fixture-v1",
            )
            .unwrap()
            .session_id;
        store
            .append_event(&session_id, finish_event("run-one"))
            .unwrap();

        let detail = store.load_session(&session_id).unwrap();
        assert_eq!(detail.events.len(), 3);
        assert_eq!(detail.events[0].sequence, 1);
        assert_eq!(detail.events[2].sequence, 3);
        assert_eq!(detail.summary.status, "finished");
        assert_eq!(store.list_sessions().unwrap().len(), 1);
        assert!(!detail
            .events
            .iter()
            .any(|event| event.kind.contains("delete")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restart_marks_running_session_interrupted_and_ignores_temp_files() {
        let root = temp_root("recovery");
        let session_id = {
            let store = AgentSessionStore::open(root.clone()).unwrap();
            let session_id = store
                .create_or_resume_run(
                    None,
                    "run-pending",
                    "等待中的任务",
                    "scenario-two",
                    "offline",
                    "fixture-v1",
                )
                .unwrap()
                .session_id;
            fs::write(
                store
                    .root
                    .join(&session_id)
                    .join("events")
                    .join(".tmp-incomplete"),
                b"partial",
            )
            .unwrap();
            session_id
        };
        let reopened = AgentSessionStore::open(root.clone()).unwrap();
        let detail = reopened.load_session(&session_id).unwrap();
        assert_eq!(detail.summary.status, "interrupted");
        assert_eq!(detail.summary.corrupted_event_count, 0);
        assert_eq!(detail.events.last().unwrap().kind, "run_interrupted");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resumed_run_receives_bounded_visible_context_without_provider_transcript() {
        let root = temp_root("context");
        let store = AgentSessionStore::open(root.clone()).unwrap();
        let first = store
            .create_or_resume_run(
                None,
                "run-context-one",
                "先分析基线",
                "scenario-context",
                "offline",
                "fixture-v1",
            )
            .unwrap();
        assert!(first.prior_context.is_none());
        store
            .append_event(
                &first.session_id,
                visible_result_event("run-context-one", "先分析基线", "基线结论可见"),
            )
            .unwrap();

        let second = store
            .create_or_resume_run(
                Some(&first.session_id),
                "run-context-two",
                "刚才的结论是什么？",
                "scenario-context",
                "offline",
                "fixture-v1",
            )
            .unwrap();
        let context = second.prior_context.unwrap();
        assert!(context.contains("先分析基线"));
        assert!(context.contains("基线结论可见"));
        assert!(!context.contains("run_trace"));
        assert!(context.len() <= 16 * 1024);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_event_is_reported_without_overwrite_and_secrets_are_redacted() {
        let root = temp_root("corrupt");
        let store = AgentSessionStore::open(root.clone()).unwrap();
        let session_id = store
            .create_or_resume_run(
                None,
                "run-secret",
                "普通问题",
                "scenario-three",
                "offline",
                "fixture-v1",
            )
            .unwrap()
            .session_id;
        let events_dir = store.root.join(&session_id).join("events");
        fs::write(events_dir.join("000003.json"), b"not-json").unwrap();
        let fake_secret = format!("{}{}", "sk-", "abcdefghijklmnopqrstu");
        let mut event = AgentSessionEventV1::empty("note");
        event.question = Some(format!("api_key=abcdefghijk secret {fake_secret}"));
        store.append_event(&session_id, event).unwrap();

        let detail = store.load_session(&session_id).unwrap();
        assert_eq!(detail.summary.corrupted_event_count, 1);
        assert_eq!(
            fs::read(events_dir.join("000003.json")).unwrap(),
            b"not-json"
        );
        let persisted = fs::read_to_string(events_dir.join("000004.json")).unwrap();
        assert!(!persisted.contains("abcdefghijk"));
        assert!(!persisted.contains(&fake_secret));
        assert!(persisted.contains("REDACTED"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn identifiers_and_sensitive_input_are_rejected() {
        let root = temp_root("validation");
        let store = AgentSessionStore::open(root.clone()).unwrap();
        assert_eq!(
            store.load_session("../escape").unwrap_err().code,
            "agent_session_not_found"
        );
        assert!(contains_likely_secret("Authorization: abcdefghijklmnop"));
        assert!(contains_likely_secret(&format!(
            "{}{}",
            "sk-", "abcdefghijklmnopqrstu"
        )));
        assert!(!contains_likely_secret("如何安全配置 API key？"));
        let _ = fs::remove_dir_all(root);
    }
}
