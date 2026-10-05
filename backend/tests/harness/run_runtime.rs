//! Offline model-policy fixtures that drive the real autonomous runtime and simulator.
use crate::agent::provider::{
    FinishReason, LlmProvider, ModelMessage, ModelRequest, ModelResponse, ProviderError,
    ProviderToolCall, TokenUsage,
};
use crate::agent::AgentRuntime;
use crate::harness::{
    run_loop::drive,
    run_schema::{Checkpoint, RunBudget, RunConstraints, RunRequest, RunUsage, RUNTIME_VERSION},
    run_store::{RunManager, RunRecord},
};
use crate::{GameVersion, Mount, SimulateRequest};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

enum Step {
    Reply(Result<ModelResponse, ProviderError>),
    Wait,
}

struct ScriptedProvider {
    steps: Mutex<VecDeque<Step>>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    waiting: Arc<Notify>,
}

impl ScriptedProvider {
    fn new(steps: Vec<Step>) -> (Self, Arc<Mutex<Vec<ModelRequest>>>, Arc<Notify>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let waiting = Arc::new(Notify::new());
        (
            Self {
                steps: Mutex::new(steps.into()),
                requests: requests.clone(),
                waiting: waiting.clone(),
            },
            requests,
            waiting,
        )
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    fn profile_id(&self) -> &str {
        "scripted-offline"
    }
    fn model(&self) -> &str {
        "harness-runtime-test-policy"
    }
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        request
            .validate()
            .expect("runtime emitted a valid model protocol");
        self.requests.lock().unwrap().push(request.clone());
        let step = self.steps.lock().unwrap().pop_front();
        match step.expect("runtime requested an unexpected model step") {
            Step::Reply(response) => response,
            Step::Wait => {
                self.waiting.notify_one();
                std::future::pending().await
            }
        }
    }
}

fn response(name: &str, arguments: Value) -> ModelResponse {
    ModelResponse {
        assistant_text: Some("根据实测证据选择下一次实验。".into()),
        reasoning_content: None,
        tool_calls: vec![ProviderToolCall {
            call_id: format!("call-{:x}", rand::random::<u64>()),
            name: name.into(),
            arguments,
        }],
        finish_reason: FinishReason::ToolCalls,
        usage: TokenUsage {
            input_tokens: 2,
            output_tokens: 1,
            total_tokens: 3,
        },
    }
}
fn tool(name: &str, arguments: Value) -> Step {
    Step::Reply(Ok(response(name, arguments)))
}
fn evaluate(text: &str, parent: Option<&str>, hypothesis: &str) -> Step {
    let mut args = json!({"kind":"evaluate","macro_text":text,"hypothesis":hypothesis});
    if let Some(parent) = parent {
        args["parent_id"] = json!(parent);
    }
    tool("experiment", args)
}
fn finish(id: Option<&str>) -> Step {
    let mut args = json!({"summary":"只交付所列实测结果，未证明全局最优。"});
    if let Some(id) = id {
        args["artifact_id"] = json!(id);
    }
    tool("finish", args)
}
fn failure() -> ProviderError {
    ProviderError {
        code: "offline_transport_failure",
        message: "injected transport failure",
        retryable: true,
        upstream_status: Some(503),
        usage: TokenUsage {
            input_tokens: 7,
            output_tokens: 3,
            total_tokens: 10,
        },
        private_detail: Some("PRIVATE_PROVIDER_DETAIL_SENTINEL".into()),
    }
}

fn checkpoint() -> Checkpoint {
    let raw: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut simulation: SimulateRequest =
        serde_json::from_value(raw["simulation"].clone()).unwrap();
    simulation.sequence = vec!["__macro__".into(); 60];
    simulation.macro_text = Some("/cast 盾刀".into());
    simulation.macro_duration = Some(10.0);
    simulation.talents.clear();
    simulation.recipes.clear();
    simulation.equipment.clear();
    simulation.channel_ticks.clear();
    simulation.timing_offsets.clear();
    simulation.qijin_buffs.clear();
    simulation.network_delay = 0;
    let request = RunRequest {
        goal: "根据完整场景实验、修正候选并交付证据。".into(),
        provider_profile: "scripted-offline".into(),
        simulation,
        version: GameVersion::AnYingQianJi,
        mount: Mount::FenShanJin,
        equipment: None,
        constraints: RunConstraints {
            duration_seconds: 10.0,
            allowed_skills: vec!["盾刀".into()],
            ..Default::default()
        },
        budget: RunBudget {
            max_model_calls: 16,
            max_simulations: 32,
            wall_time_ms: 60_000,
            ..Default::default()
        },
    };
    request.validate().unwrap();
    let scenario = request.scenario().unwrap();
    Checkpoint {
        schema_version: RUNTIME_VERSION.into(),
        run_id: format!("experiment-runtime-test-{:x}", rand::random::<u64>()),
        sequence: 1,
        status: "running".into(),
        phase: "created".into(),
        message: "offline fixture".into(),
        request,
        scenario,
        runtime_hash: "offline-runtime-fixture".into(),
        experiment_hash: "offline-experiment-fixture".into(),
        model: "harness-runtime-test-policy".into(),
        usage: RunUsage::default(),
        events: vec![],
        artifacts: vec![],
        attempts: HashMap::new(),
        result: None,
        persistence_error: false,
        reserved_simulations: 0,
    }
}

async fn run_script(state: Checkpoint, steps: Vec<Step>) -> (Arc<RunRecord>, Vec<ModelRequest>) {
    let record = RunRecord::new(state, None);
    let (provider, requests, _) = ScriptedProvider::new(steps);
    drive(
        record.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    )
    .await;
    let observed = requests.lock().unwrap().clone();
    (record, observed)
}
fn outputs(requests: &[ModelRequest]) -> Vec<&Value> {
    requests
        .iter()
        .flat_map(|r| &r.messages)
        .filter_map(|m| match m {
            ModelMessage::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn failed_candidate_guides_a_changed_candidate_to_real_simulator_success() {
    let (record, requests) = run_script(
        checkpoint(),
        vec![
            evaluate("/cast [rage>100] 盾刀", None, "验证不可能满足的怒气门槛"),
            evaluate("/cast 盾刀", None, "去掉反例证实的阻塞条件"),
            finish(Some("evidence-2")),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.status, "completed");
    assert_eq!(state.usage.simulations, 4);
    assert_eq!(state.artifacts.len(), 2);
    assert_eq!(state.artifacts[0].result["best"]["verified"], false);
    assert_eq!(state.artifacts[0].result["best"]["metrics"]["dps"], 0.0);
    assert_eq!(state.artifacts[1].result["best"]["verified"], true);
    assert!(
        state.artifacts[1].result["best"]["metrics"]["dps"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(
        state.result.unwrap().selected_artifact_id.as_deref(),
        Some("evidence-2")
    );
    assert!(outputs(&requests)
        .iter()
        .any(|o| o["id"] == "evidence-1" && o["best"]["verified"] == false));
}

#[tokio::test]
async fn equivalent_experiment_with_new_hypothesis_reuses_evidence_without_more_replays() {
    let (record, requests) = run_script(
        checkpoint(),
        vec![
            evaluate("/cast 盾刀", None, "首次测量"),
            evaluate("/cast 盾刀", None, "换一种文字重复同一实验"),
            finish(Some("evidence-1")),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(state.attempts.len(), 1);
    assert_eq!(state.usage.simulations, 2);
    assert!(state
        .events
        .iter()
        .any(|e| e.kind == "duplicate_suppressed"));
    assert!(outputs(&requests).iter().any(|o| o["cached"] == true));
}

#[tokio::test]
async fn parent_id_can_branch_from_older_evidence_instead_of_latest_candidate() {
    let (record, _) = run_script(checkpoint(), vec![
        evaluate("/cast 盾刀", None, "建立可用分支"),
        evaluate("/cast [rage>100] 盾刀", Some("evidence-1"), "尝试被条件阻塞的新分支"),
        tool("experiment", json!({"kind":"evaluate","parent_id":"evidence-1","hypothesis":"回到较早可用分支复跑"})),
        finish(Some("evidence-3")),
    ]).await;
    let state = record.read();
    assert_eq!(state.usage.simulations, 6);
    assert_eq!(state.artifacts.len(), 3);
    assert_eq!(state.artifacts[2].parent_id.as_deref(), Some("evidence-1"));
    assert_eq!(
        state.artifacts[2].simulation.macro_text,
        state.artifacts[0].simulation.macro_text
    );
    assert_ne!(
        state.artifacts[2].simulation.macro_text,
        state.artifacts[1].simulation.macro_text
    );
    assert_eq!(state.artifacts[2].result["best"]["verified"], true);
    assert_eq!(
        state.artifacts[2].result["best"]["fingerprint"],
        state.artifacts[0].result["best"]["fingerprint"]
    );
}

#[tokio::test]
async fn unknown_tool_is_a_recoverable_counterexample_and_learning_requires_evidence() {
    let (record, requests) = run_script(
        checkpoint(),
        vec![
            tool("invented_tool", json!({})),
            tool(
                "record_learning",
                json!({"observation":"不能引用未测结论","evidence_ids":["not-real"]}),
            ),
            tool("inspect", json!({"section":"scene"})),
            evaluate("/cast 盾刀", None, "根据真实场景运行"),
            tool(
                "record_learning",
                json!({"observation":"该冻结场景已能释放所选技能","evidence_ids":["evidence-1"]}),
            ),
            finish(Some("evidence-1")),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.status, "completed");
    assert_eq!(state.usage.simulations, 2);
    assert_eq!(
        state
            .events
            .iter()
            .filter(|e| e.kind == "tool_rejected")
            .count(),
        2
    );
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "learning").count(),
        1
    );
    assert!(outputs(&requests).iter().any(|o| o["ok"] == false));
}

#[tokio::test]
async fn cancellation_interrupts_an_inflight_provider_and_retains_completed_evidence() {
    let record = RunRecord::new(checkpoint(), None);
    let (provider, requests, waiting) = ScriptedProvider::new(vec![
        evaluate("/cast 盾刀", None, "先保存一个实测候选"),
        Step::Wait,
    ]);
    let task = tokio::spawn(drive(
        record.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    ));
    tokio::time::timeout(Duration::from_secs(10), waiting.notified())
        .await
        .unwrap();
    assert!(record.cancel());
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    let state = record.read();
    assert_eq!(state.status, "cancelled");
    assert_eq!(state.usage.simulations, 2);
    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(!record.active());
}

#[tokio::test]
async fn unsupported_report_numbers_do_not_block_a_verified_candidate_or_trigger_rewrites() {
    let (record, requests) = run_script(checkpoint(), vec![
        evaluate("/cast 盾刀", None, "取得可回放证据"),
        tool("finish", json!({"artifact_id":"evidence-1", "summary":"所选宏已通过本次模拟。实测DPS为987654321.123。请结合实测环境使用。"})),
    ]).await;
    let state = record.read();
    assert_eq!(state.status, "completed");
    assert_eq!(state.usage.simulations, 2);
    assert_eq!(requests.len(), 2);
    let result = state.result.as_ref().unwrap();
    assert_eq!(result.completion, "verified");
    assert_eq!(result.selected_artifact_id.as_deref(), Some("evidence-1"));
    assert!(result.summary.contains("所选宏已通过本次模拟"));
    assert!(!result.summary.contains("987654321"));
    assert!(!result.summary.contains("123"));
    assert!(result.limitations.iter().any(|s|s.contains("已从结论移除")));
    assert!(state.events.iter().any(|e|e.kind=="report_projected"));
    assert!(!state.events.iter().any(|e|e.kind=="tool_rejected"));
}

#[tokio::test]
async fn report_projection_never_upgrades_an_unverified_experiment() {
    let (record, _) = run_script(checkpoint(), vec![
        evaluate("/cast [rage>100] 盾刀", None, "不可运行的对照"),
        tool("finish", json!({"artifact_id":"evidence-1", "summary":"实测DPS为987654321.123。"})),
    ]).await;
    let state = record.read();
    let result = state.result.as_ref().unwrap();
    assert_eq!(state.status, "completed");
    assert_eq!(result.completion, "partial");
    assert!(!result.summary.contains("987654321"));
    assert!(result.summary.contains("已移除"));
    assert_eq!(state.artifacts[0].result["best"]["verified"], false);
}

#[tokio::test]
async fn model_simulation_token_and_wall_budgets_stop_without_an_extra_model_call() {
    let mut state = checkpoint();
    state.request.budget.max_model_calls = 1;
    let (record, requests) =
        run_script(state, vec![tool("inspect", json!({"section":"scene"}))]).await;
    assert_eq!(record.read().status, "budget_exhausted");
    assert_eq!(requests.len(), 1);
    assert_eq!(record.read().usage.simulations, 0);

    let mut state = checkpoint();
    state.request.budget.max_simulations = 4;
    let (record, requests) = run_script(
        state,
        vec![
            evaluate("/cast 盾刀", None, "第一次"),
            evaluate("/cast [rage>100] 盾刀", None, "第二次"),
        ],
    )
    .await;
    assert_eq!(record.read().status, "budget_exhausted");
    assert_eq!(record.read().usage.simulations, 4);
    assert_eq!(requests.len(), 2);
    assert_eq!(record.read().artifacts.len(), 2);

    let mut state = checkpoint();
    state.request.budget.max_total_tokens = 4096;
    let mut full = response("inspect", json!({"section":"scene"}));
    full.usage = TokenUsage {
        input_tokens: 4000,
        output_tokens: 96,
        total_tokens: 4096,
    };
    let (record, requests) = run_script(state, vec![Step::Reply(Ok(full))]).await;
    assert_eq!(record.read().status, "budget_exhausted");
    assert_eq!(requests.len(), 1);
    assert_eq!(record.read().usage.total_tokens, 4096);

    let mut state = checkpoint();
    state.usage.elapsed_ms = state.request.budget.wall_time_ms;
    let (record, requests) = run_script(state, vec![]).await;
    assert_eq!(record.read().status, "budget_exhausted");
    assert!(requests.is_empty());
    assert_eq!(record.read().usage.simulations, 0);
}

#[tokio::test]
async fn provider_failure_preserves_verified_artifacts_and_charges_reported_usage() {
    let (record, _) = run_script(
        checkpoint(),
        vec![
            evaluate("/cast 盾刀", None, "连接中断前取得证据"),
            Step::Reply(Err(failure())),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.status, "interrupted");
    assert_eq!(state.usage.model_calls, 2);
    assert_eq!(state.usage.simulations, 2);
    assert_eq!(state.usage.input_tokens, 9);
    assert_eq!(state.usage.output_tokens, 4);
    assert_eq!(state.usage.total_tokens, 13);
    assert_eq!(state.artifacts[0].result["best"]["verified"], true);
    assert_eq!(record.snapshot()["resumable"], true);
    assert!(!serde_json::to_string(&state)
        .unwrap()
        .contains("PRIVATE_PROVIDER_DETAIL_SENTINEL"));
}

#[tokio::test]
async fn budget_feedback_reserves_validation_and_delivery_before_large_context_exhaustion() {
    let mut state = checkpoint();
    state.request.budget.max_total_tokens = 55000;
    let mut first = response("inspect", json!({"section":"macro_language"}));
    first.usage = TokenUsage {input_tokens:10000, output_tokens:1, total_tokens:10001};
    let mut second = response("inspect", json!({"section":"scene"}));
    second.usage = first.usage.clone();
    let (record, requests) = run_script(state, vec![Step::Reply(Ok(first)), Step::Reply(Ok(second)), finish(None)]).await;
    let feedback = outputs(&requests);
    assert!(feedback.iter().any(|o|o["_remaining_budget"]["should_finish_soon"]==false));
    assert!(feedback.iter().any(|o|o["_remaining_budget"]["should_finish_soon"]==true
        && o["_remaining_budget"]["total_tokens"].as_u64().unwrap_or(0)>20_000));
    assert_eq!(record.read().status,"completed");
    assert_eq!(record.read().usage.simulations,0);
    assert!(requests.last().unwrap().instructions.contains("预算已进入收尾区"));
}

#[tokio::test]
async fn provider_output_limit_is_actionable_and_never_mislabeled_as_a_network_failure() {
    for exhausted in [false, true] {
        let mut state = checkpoint();
        state.request.budget.max_total_tokens = 4096;
        let mut error = failure();
        error.code = "provider_output_limit";
        error.retryable = false;
        error.upstream_status = None;
        if exhausted { error.usage.total_tokens = 4096; }
        let (record, requests) = run_script(state, vec![
            evaluate("/cast 盾刀", None, "输出截断前已测试的候选"),
            Step::Reply(Err(error)),
        ]).await;
        let result = record.read();
        assert_eq!(result.status, if exhausted {"budget_exhausted"} else {"interrupted"});
        assert!(result.message.contains("单轮 Token 上限"));
        assert!(!result.message.contains("连接中断"));
        assert_eq!(result.usage.simulations,2);
        assert_eq!(result.artifacts.len(),1);
        assert_eq!(requests.len(),2);
    }
}

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
        let path = root.join(format!("harness-run-runtime-{:x}", rand::random::<u64>()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn texts(&self) -> Vec<String> {
        fn collect(path: &std::path::Path, output: &mut Vec<String>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    collect(&entry.path(), output);
                } else {
                    output.push(std::fs::read_to_string(entry.path()).unwrap());
                }
            }
        }
        let mut output = Vec::new();
        collect(&self.0, &mut output);
        output
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let expected_parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .canonicalize()
            .unwrap();
        let target = self.0.canonicalize().unwrap();
        assert_eq!(target.parent(), Some(expected_parent.as_path()));
        assert!(target
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("harness-run-runtime-"));
        std::fs::remove_dir_all(target).unwrap();
    }
}

#[tokio::test]
async fn restart_recovers_evidence_without_persisting_or_replaying_private_reasoning() {
    const PRIVATE: &str = "PRIVATE_REASONING_SENTINEL_FOR_THIS_LIVE_PROVIDER_ONLY";
    let directory = TestDirectory::new();
    let state = checkpoint();
    let run_id = state.run_id.clone();
    let record = RunRecord::new(state, Some(directory.0.clone()));
    let mut first = response(
        "experiment",
        json!({"kind":"evaluate","macro_text":"/cast 盾刀","hypothesis":"中断后保留可恢复证据"}),
    );
    first.reasoning_content = Some(PRIVATE.into());
    let (provider, requests, _) =
        ScriptedProvider::new(vec![Step::Reply(Ok(first)), Step::Reply(Err(failure()))]);
    drive(
        record.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    )
    .await;
    let requests = requests.lock().unwrap();
    assert!(requests[1].messages.iter().any(|m| matches!(m, ModelMessage::Assistant { reasoning_content: Some(value), .. } if value == PRIVATE)));
    for text in directory.texts() {
        assert!(!text.contains(PRIVATE));
        assert!(!text.contains("reasoning_content"));
        assert!(!text.contains("PRIVATE_PROVIDER_DETAIL_SENTINEL"));
    }
    drop(requests);
    let restored = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert_eq!(restored.read().artifacts.len(), 1);
    assert_eq!(restored.read().usage.simulations, 2);
    restored.mutate(true, |s| s.status = "running".into());
    let (provider, observed, _) = ScriptedProvider::new(vec![finish(Some("evidence-1"))]);
    drive(
        restored.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    )
    .await;
    let state = restored.read();
    assert_eq!(state.status, "completed");
    assert_eq!(state.usage.simulations, 2);
    assert_eq!(state.usage.model_calls, 3);
    assert_eq!(
        state.result.unwrap().selected_artifact_id.as_deref(),
        Some("evidence-1")
    );
    let observed = observed.lock().unwrap();
    assert_eq!(observed[0].messages.len(), 1);
    let ModelMessage::User { content } = &observed[0].messages[0] else {
        panic!("semantic recovery context expected")
    };
    assert!(content.contains("evidence-1"));
    assert!(!content.contains(PRIVATE));
}

#[tokio::test]
async fn interrupted_reserved_work_is_accounted_once_and_partial_files_are_ignored() {
    let directory = TestDirectory::new();
    let mut state = checkpoint();
    let run_id = state.run_id.clone();
    state.usage.simulations = 2;
    state.reserved_simulations = 3;
    let record = RunRecord::new(state, Some(directory.0.clone()));
    record.persist();
    std::fs::write(
        directory
            .0
            .join(&run_id)
            .join("checkpoint-99999999.pending"),
        "{broken",
    )
    .unwrap();
    let restored = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert_eq!(restored.read().status, "interrupted");
    assert_eq!(restored.read().usage.simulations, 5);
    assert_eq!(restored.read().reserved_simulations, 0);
    restored.mutate(true, |s| s.status = "running".into());
    let (provider, _, _) = ScriptedProvider::new(vec![finish(None)]);
    drive(
        restored.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    )
    .await;
    assert_eq!(restored.read().status, "completed");
    assert_eq!(restored.read().usage.simulations, 5);
    let second = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert_eq!(second.read().usage.simulations, 5);
    assert_eq!(second.read().result.unwrap().completion, "no_solution");
}

#[tokio::test]
async fn orphan_artifact_cannot_replace_new_evidence_reusing_the_same_public_id() {
    let directory = TestDirectory::new();
    let state = checkpoint();
    let run_id = state.run_id.clone();
    let (old, _) = run_script(
        state.clone(),
        vec![
            evaluate("/cast [rage>100] 盾刀", None, "即将成为孤儿的失败证据"),
            finish(Some("evidence-1")),
        ],
    )
    .await;
    let orphan = old.read().artifacts[0].clone();
    assert_eq!(orphan.result["best"]["verified"], false);
    let record = RunRecord::new(state, Some(directory.0.clone()));
    record.persist();
    record.mutate(true, |s| s.artifacts.push(orphan.clone()));
    assert!(!record.read().persistence_error);
    // Remove only our newly created checkpoint to reproduce a crash after the
    // artifact rename but before its referencing checkpoint becomes durable.
    let uncommitted = directory
        .0
        .join(&run_id)
        .join(format!("checkpoint-{:08}.json", record.read().sequence));
    std::fs::remove_file(uncommitted).unwrap();
    let restored = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert!(restored.read().artifacts.is_empty());
    restored.mutate(true, |s| s.status = "running".into());
    let (provider, _, _) = ScriptedProvider::new(vec![
        evaluate("/cast 盾刀", None, "旧checkpoint之后的新实测候选"),
        finish(Some("evidence-1")),
    ]);
    drive(
        restored.clone(),
        Arc::new(AgentRuntime::fixture()),
        Box::new(provider),
    )
    .await;
    let fresh = restored.read();
    assert!(!fresh.persistence_error);
    assert_eq!(fresh.artifacts[0].id, orphan.id);
    assert_ne!(fresh.artifacts[0].request_hash, orphan.request_hash);
    assert_eq!(fresh.artifacts[0].result["best"]["verified"], true);
    let reopened = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&reopened.read().artifacts).unwrap(),
        serde_json::to_value(&fresh.artifacts).unwrap(),
        "an immutable orphan must not silently replace a newer same-ID artifact"
    );
}

#[test]
fn persistence_failure_clears_after_the_store_is_repaired_and_written_successfully() {
    let directory = TestDirectory::new();
    let blocked_root = directory.0.join("temporarily-blocked-store");
    std::fs::write(&blocked_root, "ordinary file prevents directory creation").unwrap();
    let state = checkpoint();
    let run_id = state.run_id.clone();
    let record = RunRecord::new(state, Some(blocked_root.clone()));
    record.persist();
    assert!(record.read().persistence_error);
    assert_eq!(record.snapshot()["persistence_error"], true);
    std::fs::remove_file(&blocked_root).unwrap();
    std::fs::create_dir(&blocked_root).unwrap();
    record.persist();
    assert!(
        !record.read().persistence_error,
        "a repaired store must not leave a sticky failure"
    );
    assert_eq!(record.snapshot()["persistence_error"], false);
    let reopened = RunManager::open(Some(blocked_root)).get(&run_id).unwrap();
    assert!(
        !reopened.read().persistence_error,
        "successful checkpoints must persist the cleared flag"
    );
}

#[test]
fn corrupt_latest_checkpoint_falls_back_and_reserves_a_new_sequence_for_future_writes() {
    let directory = TestDirectory::new();
    let state = checkpoint();
    let run_id = state.run_id.clone();
    let record = RunRecord::new(state, Some(directory.0.clone()));
    record.persist();
    record.mutate(true, |s| {
        s.message = "newer checkpoint before injected corruption".into()
    });
    let damaged_sequence = record.read().sequence;
    let damaged_path = directory
        .0
        .join(&run_id)
        .join(format!("checkpoint-{damaged_sequence:08}.json"));
    std::fs::write(&damaged_path, "{incomplete checkpoint").unwrap();
    let restored = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert!(restored.read().sequence > damaged_sequence);
    restored.mutate(true, |s| {
        s.status = "interrupted".into();
        s.message = "recovered checkpoint persisted beyond damaged sequence".into();
    });
    assert!(!restored.read().persistence_error);
    assert_eq!(
        std::fs::read_to_string(damaged_path).unwrap(),
        "{incomplete checkpoint",
        "immutable damaged files must not be silently overwritten"
    );
    let reopened = RunManager::open(Some(directory.0.clone()))
        .get(&run_id)
        .unwrap();
    assert_eq!(
        reopened.read().message,
        "recovered checkpoint persisted beyond damaged sequence"
    );
    assert!(reopened.read().sequence > damaged_sequence);
}

#[tokio::test]
async fn repeated_skill_inspection_reuses_the_observation_without_simulation_cost() {
    let query = json!({"section":"skills","query":"盾刀","offset":0,"limit":4});
    let (record, requests) = run_script(
        checkpoint(),
        vec![
            tool("inspect", query.clone()),
            tool("inspect", query),
            finish(None),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.status, "completed");
    assert_eq!(state.usage.simulations, 0);
    assert!(state.artifacts.is_empty());
    assert_eq!(
        state
            .attempts
            .keys()
            .filter(|key| key.starts_with("observe:"))
            .count(),
        1
    );
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| event.kind == "observation")
            .count(),
        1
    );
    assert!(state
        .events
        .iter()
        .any(|event| event.kind == "observation_reused"));
    let outputs = outputs(&requests);
    let initial = outputs
        .iter()
        .find(|output| output["skills"].is_array() && output["_cached_observation"] != true)
        .expect("first inspection should return real skill definitions");
    let reused = outputs
        .iter()
        .find(|output| output["_cached_observation"] == true)
        .expect("repeat inspection should explicitly identify cached evidence");
    assert_eq!(initial["skills"], reused["skills"]);
    assert!(reused["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skill| skill["name"]
            .as_str()
            .is_some_and(|name| name.contains("盾刀"))));
}

#[tokio::test]
async fn reasoning_triggered_compaction_keeps_recent_paired_calls_and_latest_counterexample() {
    let observation = response(
        "inspect",
        json!({"section":"skills","query":"盾刀","limit":2}),
    );
    let observation_call = observation.tool_calls[0].call_id.clone();
    let mut experiment = response(
        "experiment",
        json!({
            "kind":"evaluate","macro_text":"/cast [rage>100] 盾刀",
            "hypothesis":"保持不可满足的条件，以实测反馈决定下一步",
        }),
    );
    let experiment_call = experiment.tool_calls[0].call_id.clone();
    // Valid provider-private content below the per-message protocol limit, but
    // deliberately above the live-context compaction threshold.
    experiment.reasoning_content = Some("r".repeat(60 * 1024));
    let (record, requests) = run_script(
        checkpoint(),
        vec![
            Step::Reply(Ok(observation)),
            Step::Reply(Ok(experiment)),
            finish(Some("evidence-1")),
        ],
    )
    .await;
    let state = record.read();
    assert_eq!(state.status, "completed");
    assert!(state
        .events
        .iter()
        .any(|event| event.kind == "context_compacted"));
    let compacted = requests
        .iter()
        .rev()
        .find(|request| {
            request.messages.iter().any(|message| {
        matches!(message, ModelMessage::ToolResult { call_id, .. } if call_id == &experiment_call)
    })
        })
        .expect("the next model request must retain its latest experimental counterexample");
    assert!(
        matches!(compacted.messages.first(), Some(ModelMessage::User { .. })),
        "the compacted context starts with the durable semantic ledger"
    );
    for id in [&observation_call, &experiment_call] {
        let assistant_position = compacted
            .messages
            .iter()
            .position(|message| match message {
                ModelMessage::Assistant { tool_calls, .. } => {
                    tool_calls.iter().any(|call| &call.call_id == id)
                }
                _ => false,
            })
            .expect("recent tool output must retain its corresponding assistant call");
        let result_position = compacted.messages.iter().position(|message| {
            matches!(message, ModelMessage::ToolResult { call_id, .. } if call_id == id)
        }).expect("both recent completed exchanges remain available");
        assert!(assistant_position < result_position);
    }
    let latest = compacted
        .messages
        .iter()
        .find_map(|message| match message {
            ModelMessage::ToolResult { call_id, output } if call_id == &experiment_call => {
                Some(output)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(latest["id"], "evidence-1");
    assert_eq!(latest["best"]["verified"], false);
    assert_eq!(latest["best"]["metrics"]["dps"], 0.0);
    compacted
        .validate()
        .expect("compaction may not create orphan or duplicate calls");
}
