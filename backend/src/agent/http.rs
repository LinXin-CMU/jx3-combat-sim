use axum::{
    extract::{rejection::JsonRejection, Json, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{SharedState, SimulateRequest};

use super::evidence::EvidenceError;
use super::{
    analyze_timeline, compare_scenarios, get_current_scenario, simulate_scenario, AgentRuntime,
    CandidatePatchV1, ComparisonExecution, EvidenceEnvelopeV1, ScenarioError, ScenarioSnapshotV1,
    ScenarioSummary, SimulationSummary, TimelineAnalysis, ToolBudget, ToolError,
};

pub const AGENT_ERROR_SCHEMA_V1: &str = "agent-tool-error/v1";
const SIMULATE_MAX_SIMULATIONS: u32 = 1;
const COMPARE_MAX_SIMULATIONS: u32 = 4;
const TIMELINE_MAX_SIMULATIONS: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioToolRequest {
    pub trace_id: String,
    pub simulation: SimulateRequest,
}

#[derive(Debug, Serialize)]
pub struct ScenarioToolResponse {
    pub scenario: ScenarioSnapshotV1,
    pub evidence: EvidenceEnvelopeV1<ScenarioSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulateToolRequest {
    pub trace_id: String,
    pub scenario: ScenarioSnapshotV1,
    #[serde(default = "default_simulate_budget")]
    pub max_simulations: u32,
}

#[derive(Debug, Serialize)]
pub struct SimulateToolResponse {
    pub evidence: EvidenceEnvelopeV1<SimulationSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareToolRequest {
    pub trace_id: String,
    pub baseline: ScenarioSnapshotV1,
    pub candidates: Vec<CandidatePatchV1>,
    #[serde(default = "default_compare_budget")]
    pub max_simulations: u32,
}

#[derive(Debug, Serialize)]
pub struct CompareToolResponse {
    pub evidence: EvidenceEnvelopeV1<super::ScenarioComparison>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineToolRequest {
    pub trace_id: String,
    pub scenario: ScenarioSnapshotV1,
    #[serde(default = "default_timeline_budget")]
    pub max_simulations: u32,
}

#[derive(Debug, Serialize)]
pub struct TimelineToolResponse {
    pub simulation: EvidenceEnvelopeV1<SimulationSummary>,
    pub timeline: EvidenceEnvelopeV1<TimelineAnalysis>,
}

#[derive(Debug, Serialize)]
pub struct AgentToolErrorEnvelope {
    pub schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub tool_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_hash: Option<String>,
    pub error: AgentToolError,
}

#[derive(Debug, Serialize)]
pub struct AgentToolError {
    pub code: &'static str,
    pub message: String,
}

pub async fn scenario_handler(
    State(state): State<SharedState>,
    payload: Result<Json<ScenarioToolRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(request) => request,
        Err(_) => return invalid_json_response("get_current_scenario"),
    };
    let runtime = AgentRuntime::load(&state).await;
    let scenario = match ScenarioSnapshotV1::capture(
        runtime.game_version(),
        runtime.mount(),
        request.simulation,
    ) {
        Ok(scenario) => scenario,
        Err(error) => {
            return tool_error_response(
                "get_current_scenario",
                &request.trace_id,
                None,
                ToolError::Scenario(error),
            )
        }
    };
    match get_current_scenario(&request.trace_id, &scenario, &runtime.context(), runtime.provenance()) {
        Ok(evidence) => json_response(ScenarioToolResponse { scenario, evidence }),
        Err(error) => tool_error_response(
            "get_current_scenario",
            &request.trace_id,
            Some(&scenario.scenario_hash),
            error,
        ),
    }
}

pub async fn simulate_handler(
    State(state): State<SharedState>,
    payload: Result<Json<SimulateToolRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(request) => request,
        Err(_) => return invalid_json_response("simulate_scenario"),
    };
    if request.max_simulations > SIMULATE_MAX_SIMULATIONS {
        return invalid_budget_response(
            "simulate_scenario",
            &request.trace_id,
            Some(&request.scenario.scenario_hash),
            SIMULATE_MAX_SIMULATIONS,
        );
    }
    let runtime = AgentRuntime::load(&state).await;
    let context = runtime.context();
    let mut budget = ToolBudget::new(request.max_simulations);
    match simulate_scenario(
        &request.trace_id,
        &request.scenario,
        &context,
        runtime.provenance(),
        &mut budget,
    ) {
        Ok(execution) => json_response(SimulateToolResponse {
            evidence: execution.evidence,
        }),
        Err(error) => tool_error_response(
            "simulate_scenario",
            &request.trace_id,
            Some(&request.scenario.scenario_hash),
            error,
        ),
    }
}

pub async fn compare_handler(
    State(state): State<SharedState>,
    payload: Result<Json<CompareToolRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(request) => request,
        Err(_) => return invalid_json_response("compare_scenarios"),
    };
    if request.max_simulations > COMPARE_MAX_SIMULATIONS {
        return invalid_budget_response(
            "compare_scenarios",
            &request.trace_id,
            Some(&request.baseline.scenario_hash),
            COMPARE_MAX_SIMULATIONS,
        );
    }
    let runtime = AgentRuntime::load(&state).await;
    let context = runtime.context();
    let mut budget = ToolBudget::new(request.max_simulations);
    let result: Result<ComparisonExecution, ToolError> = compare_scenarios(
        &request.trace_id,
        &request.baseline,
        &request.candidates,
        &context,
        runtime.provenance(),
        &mut budget,
    );
    match result {
        Ok(execution) => json_response(CompareToolResponse {
            evidence: execution.evidence,
        }),
        Err(error) => tool_error_response(
            "compare_scenarios",
            &request.trace_id,
            Some(&request.baseline.scenario_hash),
            error,
        ),
    }
}

pub async fn timeline_handler(
    State(state): State<SharedState>,
    payload: Result<Json<TimelineToolRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match payload {
        Ok(request) => request,
        Err(_) => return invalid_json_response("analyze_timeline"),
    };
    if request.max_simulations > TIMELINE_MAX_SIMULATIONS {
        return invalid_budget_response(
            "analyze_timeline",
            &request.trace_id,
            Some(&request.scenario.scenario_hash),
            TIMELINE_MAX_SIMULATIONS,
        );
    }
    let runtime = AgentRuntime::load(&state).await;
    let context = runtime.context();
    let mut budget = ToolBudget::new(request.max_simulations);
    let simulation = match simulate_scenario(
        &request.trace_id,
        &request.scenario,
        &context,
        runtime.provenance(),
        &mut budget,
    ) {
        Ok(execution) => execution,
        Err(error) => {
            return tool_error_response(
                "analyze_timeline",
                &request.trace_id,
                Some(&request.scenario.scenario_hash),
                error,
            )
        }
    };
    match analyze_timeline(&request.trace_id, &simulation, runtime.provenance()) {
        Ok(timeline) => json_response(TimelineToolResponse {
            simulation: simulation.evidence,
            timeline: timeline.evidence,
        }),
        Err(error) => tool_error_response(
            "analyze_timeline",
            &request.trace_id,
            Some(&request.scenario.scenario_hash),
            error,
        ),
    }
}

fn default_simulate_budget() -> u32 {
    SIMULATE_MAX_SIMULATIONS
}

fn default_compare_budget() -> u32 {
    COMPARE_MAX_SIMULATIONS
}

fn default_timeline_budget() -> u32 {
    TIMELINE_MAX_SIMULATIONS
}

fn invalid_json_response(tool_name: &'static str) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        tool_name,
        None,
        None,
        "invalid_json",
        "request body must be valid JSON matching the tool schema".to_string(),
    )
}

fn invalid_budget_response(
    tool_name: &'static str,
    trace_id: &str,
    scenario_hash: Option<&str>,
    hard_limit: u32,
) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        tool_name,
        safe_trace_id(trace_id),
        safe_scenario_hash(scenario_hash),
        "invalid_budget_limit",
        format!("max_simulations cannot exceed {hard_limit} for this endpoint"),
    )
}

fn tool_error_response(
    tool_name: &'static str,
    trace_id: &str,
    scenario_hash: Option<&str>,
    error: ToolError,
) -> Response {
    let (status, code) = match &error {
        ToolError::Scenario(ScenarioError::HashMismatch { .. }) => {
            (StatusCode::CONFLICT, "scenario_hash_mismatch")
        }
        ToolError::Scenario(ScenarioError::Serialization(_))
        | ToolError::Evidence(EvidenceError::Serialization(_)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
        ToolError::Scenario(_) => (StatusCode::BAD_REQUEST, "invalid_scenario"),
        ToolError::Evidence(EvidenceError::InvalidTraceId) => {
            (StatusCode::BAD_REQUEST, "invalid_trace_id")
        }
        ToolError::RuntimeMismatch { .. } => (StatusCode::CONFLICT, "runtime_mismatch"),
        ToolError::BudgetExceeded { .. } => (StatusCode::TOO_MANY_REQUESTS, "budget_exceeded"),
        ToolError::InvalidCandidateCount { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_candidate_count")
        }
        ToolError::InvalidCandidateLabel => (StatusCode::BAD_REQUEST, "invalid_candidate_label"),
        ToolError::DuplicateCandidateLabel { .. } => {
            (StatusCode::BAD_REQUEST, "duplicate_candidate_label")
        }
        ToolError::NoScenarioChanges { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "no_scenario_changes")
        }
        ToolError::TimelineDetailsUnavailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "timeline_details_unavailable",
        ),
        ToolError::EquipmentFocusUnavailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "equipment_focus_unavailable",
        ),
        ToolError::EquipmentStrategyUnavailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "equipment_strategy_unavailable",
        ),
    };
    error_response(
        status,
        tool_name,
        safe_trace_id(trace_id),
        safe_scenario_hash(scenario_hash),
        code,
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            "internal tool error".to_string()
        } else if code == "scenario_hash_mismatch" {
            "scenario hash does not match canonical scenario contents".to_string()
        } else {
            error.to_string()
        },
    )
}

fn safe_trace_id(trace_id: &str) -> Option<String> {
    super::evidence::validate_trace_id(trace_id)
        .ok()
        .map(|_| trace_id.to_string())
}

fn safe_scenario_hash(scenario_hash: Option<&str>) -> Option<String> {
    scenario_hash
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

fn error_response(
    status: StatusCode,
    tool_name: &'static str,
    trace_id: Option<String>,
    scenario_hash: Option<String>,
    code: &'static str,
    message: String,
) -> Response {
    let mut response = (
        status,
        Json(AgentToolErrorEnvelope {
            schema_version: AGENT_ERROR_SCHEMA_V1,
            trace_id,
            tool_name,
            scenario_hash,
            error: AgentToolError { code, message },
        }),
    )
        .into_response();
    set_json_utf8(&mut response);
    response
}

fn json_response<T: Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    set_json_utf8(&mut response);
    response
}

fn set_json_utf8(response: &mut Response) {
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn request_schemas_reject_unknown_top_level_fields_and_apply_budgets() {
        let invalid = serde_json::json!({
            "trace_id": "trace-1",
            "scenario": {},
            "unexpected": true
        });
        assert!(serde_json::from_value::<SimulateToolRequest>(invalid).is_err());

        assert_eq!(default_simulate_budget(), 1);
        assert_eq!(default_compare_budget(), 4);
        assert_eq!(default_timeline_budget(), 1);
    }

    #[tokio::test]
    async fn hash_mismatch_maps_to_stable_conflict_envelope() {
        let response = tool_error_response(
            "simulate_scenario",
            "trace-1",
            Some("claimed-hash"),
            ToolError::Scenario(ScenarioError::HashMismatch {
                expected: "claimed-hash".to_string(),
                actual: "actual-hash".to_string(),
            }),
        );
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["schema_version"], AGENT_ERROR_SCHEMA_V1);
        assert_eq!(body["error"]["code"], "scenario_hash_mismatch");
        assert_eq!(
            body["error"]["message"],
            "scenario hash does not match canonical scenario contents"
        );
        assert_eq!(body["trace_id"], "trace-1");
        assert!(body.get("scenario_hash").is_none());
    }

    #[tokio::test]
    async fn unsafe_trace_is_not_reflected_in_error_json() {
        let response = tool_error_response(
            "simulate_scenario",
            "../../userdata",
            None,
            ToolError::Evidence(EvidenceError::InvalidTraceId),
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(body.get("trace_id").is_none());
        assert_eq!(body["error"]["code"], "invalid_trace_id");
    }
}
