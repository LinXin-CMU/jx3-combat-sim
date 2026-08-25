//! 运行时：HTTP 处理器 + 单实例运行状态 + SSE 广播

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{broadcast, Mutex};

use super::analyze::extract_tunables;
use super::archive::{list_runs, list_archive_subdir, read_archive_file, read_meta, ArchiveWriter};
use super::ga::{EffectiveTunable, Event as GaEvent, FitnessCtx, GaParams, GaRun, ProgressSink};
use super::loop_config::LoopConfig;
use super::rule_pool::{build_pool, CandidateRuleInput};
use super::struct_ga::{StructCtx, StructGaRun};

use crate::{Attributes, TargetConfig, SharedState};

// ─────────────────────────────────────────────────────────────────────────────
// 全局状态（在 main.rs 中构造并放入 AppState）
// ─────────────────────────────────────────────────────────────────────────────

pub struct OptState {
    pub current: Mutex<Option<ActiveRun>>,
    pub broadcaster: broadcast::Sender<GaEvent>,
}

pub struct ActiveRun {
    pub run_id: String,
    pub stop: Arc<AtomicBool>,
    pub total_gens: usize,
}

impl OptState {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self {
            current: Mutex::new(None),
            broadcaster: tx,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 请求/响应类型
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub macro_text: String,
    #[serde(default)]
    pub tunable_overrides: Vec<TunableOverride>,
    pub base_loop: LoopConfig,
    pub attrs: Attributes,
    pub target: TargetConfig,
    #[serde(default)]
    pub ga_params: Option<GaParamsReq>,
    pub duration: f64,
    #[serde(default)]
    pub duration_min: Option<f64>,
    #[serde(default)]
    pub duration_max: Option<f64>,
    pub haste_level: u32,
    #[serde(default)]
    pub talents: Vec<u32>,
    #[serde(default)]
    pub recipes: Vec<u32>,
    #[serde(default)]
    pub initial_rage: Option<i32>,
    #[serde(default)]
    pub network_delay: u32,
    /// 每个个体随机采样评估次数（fitness = mean - 0.5*std）。默认 3
    #[serde(default)]
    pub samples_per_eval: Option<usize>,
    /// Phase 3：启用规则结构搜索
    #[serde(default)]
    pub struct_search: bool,
    /// Phase 3：候选规则池（仅 struct_search=true 时使用）
    #[serde(default)]
    pub candidates: Vec<CandidateRuleInput>,
    /// Phase 3：首行锁定（默认 true）
    #[serde(default = "default_true")]
    pub lock_first: bool,
    /// 显式场景集（Step 2 场景笛卡尔积）——非空时取代 K 次随机时长采样
    #[serde(default)]
    pub scenarios: Option<Vec<crate::optimizer::ga::ScenarioSpec>>,
}

fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct TunableOverride {
    pub id: String,
    pub enabled: bool,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GaParamsReq {
    #[serde(default)]
    pub pop_size: Option<usize>,
    #[serde(default)]
    pub generations: Option<usize>,
    #[serde(default)]
    pub top_n: Option<usize>,
    #[serde(default = "default_fitness_dps_mode")]
    pub fitness_dps_mode: bool,
}

fn default_fitness_dps_mode() -> bool { true }

#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub run_id: String,
    pub n_params: usize,
    pub pop_size: usize,
    pub generations: usize,
    pub enabled_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub running: bool,
    pub run_id: Option<String>,
    pub total_gens: Option<usize>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/start
// ─────────────────────────────────────────────────────────────────────────────

pub async fn start_handler(
    State(shared): State<SharedState>,
    Json(req): Json<StartRequest>,
) -> Response {
    let opt = shared.optimizer.clone();
    // 1. 独占锁：同时只允许一个运行
    let mut guard = opt.current.lock().await;
    if guard.is_some() {
        return (StatusCode::CONFLICT, Json(err_body("已有正在运行的优化任务"))).into_response();
    }

    // Phase 3 分支：规则结构搜索
    if req.struct_search {
        drop(guard);
        return start_struct_handler(shared, req).await;
    }

    // 2. 分析宏
    let analyze_res = match extract_tunables(&req.macro_text) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(err_body(&format!("宏解析失败: {}", e)))).into_response(),
    };
    let all_tunables = analyze_res.params;

    // 3. 合并 overrides
    let override_map: HashMap<String, &TunableOverride> =
        req.tunable_overrides.iter().map(|o| (o.id.clone(), o)).collect();
    let mut enabled_tunables: Vec<EffectiveTunable> = Vec::new();
    for p in all_tunables.iter() {
        match override_map.get(&p.id) {
            Some(ov) if !ov.enabled => continue,
            Some(ov) => enabled_tunables.push(EffectiveTunable {
                param: p.clone(),
                min: ov.min, max: ov.max, step: ov.step.max(1e-6),
            }),
            None => enabled_tunables.push(EffectiveTunable {
                param: p.clone(),
                min: p.suggested_min, max: p.suggested_max, step: p.suggested_step.max(1e-6),
            }),
        }
    }
    if enabled_tunables.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(err_body("至少要启用一个可调阈值"))).into_response();
    }

    // 4. 再次解析宏作为 baseline MacroConfig
    let baseline_macro = match crate::macro_parser::parse_macro_text(&req.macro_text) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(err_body(&format!("宏解析失败: {}", e)))).into_response(),
    };

    // 5. GA 参数
    let mut ga_params = GaParams::default();
    if let Some(p) = req.ga_params {
        if let Some(v) = p.pop_size { ga_params.pop_size = v.max(8); }
        if let Some(v) = p.generations { ga_params.generations = v.max(1); }
        if let Some(v) = p.top_n { ga_params.top_n = v.max(1); }
        ga_params.fitness_dps_mode = p.fitness_dps_mode;
    }

    // 6. 生成 run_id
    let run_id = new_run_id("phase2");

    // 7. meta 写盘
    let meta = serde_json::json!({
        "run_id": run_id,
        "phase": "phase2",
        "start_time_unix": now_unix(),
        "duration": req.duration,
        "haste_level": req.haste_level,
        "network_delay": req.network_delay,
        "initial_rage": req.initial_rage,
        "talents": req.talents,
        "recipes": req.recipes,
        "attrs": req.attrs,
        "target": req.target,
        "ga": {
            "pop_size": ga_params.pop_size,
            "generations": ga_params.generations,
            "top_n": ga_params.top_n,
        },
        "n_params": enabled_tunables.len(),
        "n_all_params": all_tunables.len(),
    });

    // 8. 创建存档器 + 存入 baseline
    let mut archive = match ArchiveWriter::new(run_id.clone(), &meta, &req.base_loop) {
        Ok(a) => a,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(err_body(&format!("存档创建失败: {}", e)))).into_response(),
    };

    // 9. FitnessCtx
    let ctx = Arc::new(FitnessCtx {
        baseline_macro,
        tunables: enabled_tunables.clone(),
        attrs: req.attrs,
        target: req.target,
        talents: req.talents.clone(),
        recipes: req.recipes.clone(),
        initial_rage: req.initial_rage,
        haste_level: req.haste_level,
        network_delay_ms: req.network_delay,
        duration_min: req.duration_min.unwrap_or(req.duration).max(1.0),
        duration_max: req.duration_max.unwrap_or(req.duration).max(1.0),
        samples_per_eval: req.samples_per_eval.unwrap_or(3).max(1),
        scenarios: req.scenarios.clone().unwrap_or_default(),
        skills: Arc::new(shared.skills.read().await.clone()),
        recipes_table: Arc::new(shared.recipes.read().await.clone()),
    });

    // 10. 运行句柄
    let stop = Arc::new(AtomicBool::new(false));
    let active = ActiveRun {
        run_id: run_id.clone(),
        stop: stop.clone(),
        total_gens: ga_params.generations,
    };
    *guard = Some(active);
    drop(guard);

    // 11. 广播 Started
    let tx = opt.broadcaster.clone();
    let enabled_ids: Vec<String> = enabled_tunables.iter().map(|t| t.param.id.clone()).collect();
    let _ = tx.send(GaEvent::Started {
        run_id: run_id.clone(),
        total_gens: ga_params.generations,
        pop_size: ga_params.pop_size,
        n_params: enabled_tunables.len(),
        enabled_ids,
    });

    // 12. spawn_blocking 跑 GA
    let opt_state_clone = opt.clone();
    let run_id_clone = run_id.clone();
    let base_loop = req.base_loop.clone();
    let ga_params_clone = ga_params.clone();
    tokio::task::spawn_blocking(move || {
        let sink: ProgressSink = {
            let tx = tx.clone();
            Box::new(move |ev: GaEvent| {
                let _ = tx.send(ev);
            })
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut run = GaRun {
                ctx: &ctx,
                params: &ga_params_clone,
                base_loop: &base_loop,
                archive: &mut archive,
                sink: &sink,
                stop: stop.clone(),
            };
            run.run()
        }));

        let (best_dps, gens_completed) = match result {
            Ok((_ind, best, gens)) => (best, gens),
            Err(_) => {
                let _ = tx.send(GaEvent::Error { message: "GA 运行异常崩溃".into() });
                (0.0, 0)
            }
        };

        let _ = tx.send(GaEvent::Done {
            best_dps,
            total_gens: gens_completed,
        });

        // 释放锁
        let rt = tokio::runtime::Handle::try_current();
        if let Ok(handle) = rt {
            handle.block_on(async {
                let mut g = opt_state_clone.current.lock().await;
                if let Some(r) = g.as_ref() {
                    if r.run_id == run_id_clone { *g = None; }
                }
            });
        } else {
            // 没有当前 tokio 运行时（spawn_blocking 场景外）：启动一个临时 current_thread runtime 清状态
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let mut g = opt_state_clone.current.lock().await;
                if let Some(r) = g.as_ref() {
                    if r.run_id == run_id_clone { *g = None; }
                }
            });
        }
    });

    let enabled_ids_resp: Vec<String> = enabled_tunables.iter().map(|t| t.param.id.clone()).collect();
    Json(StartResponse {
        run_id,
        n_params: enabled_tunables.len(),
        pop_size: ga_params.pop_size,
        generations: ga_params.generations,
        enabled_ids: enabled_ids_resp,
    }).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/stop
// ─────────────────────────────────────────────────────────────────────────────

pub async fn stop_handler(
    State(shared): State<SharedState>,
) -> impl IntoResponse {
    let opt = shared.optimizer.clone();
    let guard = opt.current.lock().await;
    if let Some(r) = guard.as_ref() {
        r.stop.store(true, Ordering::Relaxed);
        Json(serde_json::json!({ "stopped": true, "run_id": r.run_id }))
    } else {
        Json(serde_json::json!({ "stopped": false, "reason": "没有正在运行的任务" }))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/status
// ─────────────────────────────────────────────────────────────────────────────

pub async fn status_handler(
    State(shared): State<SharedState>,
) -> impl IntoResponse {
    let opt = shared.optimizer.clone();
    let guard = opt.current.lock().await;
    match guard.as_ref() {
        Some(r) => Json(StatusResponse {
            running: true,
            run_id: Some(r.run_id.clone()),
            total_gens: Some(r.total_gens),
        }),
        None => Json(StatusResponse {
            running: false, run_id: None, total_gens: None,
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/stream (SSE)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn stream_handler(
    State(shared): State<SharedState>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let opt = shared.optimizer.clone();
    let rx = opt.broadcaster.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|res: Result<GaEvent, _>| async move {
            match res {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    Some(Ok::<_, std::convert::Infallible>(SseEvent::default().data(data)))
                }
                Err(_) => None,
            }
        });
    use futures_util::StreamExt;
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/analyze (已在 main.rs 中，此处不重复)
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// 处理器：/api/optimizer/runs 列表
// ─────────────────────────────────────────────────────────────────────────────

pub async fn list_runs_handler() -> Response {
    match list_runs() {
        Ok(ids) => Json(serde_json::json!({ "runs": ids })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err_body(&e.to_string()))).into_response(),
    }
}

pub async fn run_detail_handler(Path(id): Path<String>) -> Response {
    let meta = match read_meta(&id) {
        Ok(m) => m,
        Err(e) => return (StatusCode::NOT_FOUND, Json(err_body(&e.to_string()))).into_response(),
    };
    let milestones = list_archive_subdir(&id, "milestones").unwrap_or_default();
    let topn = list_archive_subdir(&id, "topN").unwrap_or_default();
    Json(serde_json::json!({
        "run_id": id,
        "meta": meta,
        "milestones": milestones,
        "topN": topn,
    })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

pub async fn run_file_handler(
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> Response {
    match read_archive_file(&id, &q.path) {
        Ok(text) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            text,
        ).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(err_body(&e.to_string()))).into_response(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 工具函数
// ─────────────────────────────────────────────────────────────────────────────

fn err_body(msg: &str) -> serde_json::Value {
    serde_json::json!({ "error": msg })
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_run_id(prefix: &str) -> String {
    format!("{}_{}", prefix, now_unix())
}

// 复用 ga.rs 里的 fitness/EffectiveTunable（重导出避免循环依赖）
pub use super::ga::EffectiveTunable as _ReexportEffective;
pub use super::analyze::TunableParam as _ReexportTunable;

// ─────────────────────────────────────────────────────────────────────────────
// Phase 3：规则结构搜索 start handler
// ─────────────────────────────────────────────────────────────────────────────

async fn start_struct_handler(shared: SharedState, req: StartRequest) -> Response {
    let opt = shared.optimizer.clone();
    let mut guard = opt.current.lock().await;
    if guard.is_some() {
        return (StatusCode::CONFLICT, Json(err_body("已有正在运行的优化任务"))).into_response();
    }

    // 1. 构建规则池
    let pool = match build_pool(&req.macro_text, &req.candidates, req.lock_first) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(err_body(&e))).into_response(),
    };

    // 2. flat param table
    let (param_keys, tunables) = StructCtx::build_param_table(&pool);

    // 3. 合并 overrides（按 tunable id 查找覆盖 min/max/step / 禁用）
    let override_map: HashMap<String, &TunableOverride> =
        req.tunable_overrides.iter().map(|o| (o.id.clone(), o)).collect();
    let mut keep_mask = Vec::with_capacity(tunables.len());
    let mut effective: Vec<EffectiveTunable> = Vec::new();
    let mut kept_keys: Vec<(String, usize)> = Vec::new();
    for (t, k) in tunables.iter().zip(param_keys.iter()) {
        match override_map.get(&t.param.id) {
            Some(ov) if !ov.enabled => { keep_mask.push(false); }
            Some(ov) => {
                effective.push(EffectiveTunable {
                    param: t.param.clone(),
                    min: ov.min, max: ov.max, step: ov.step.max(1e-6),
                });
                kept_keys.push(k.clone());
                keep_mask.push(true);
            }
            None => {
                effective.push(t.clone());
                kept_keys.push(k.clone());
                keep_mask.push(true);
            }
        }
    }

    // 4. GA 参数
    let mut ga_params = GaParams::default();
    if let Some(p) = req.ga_params {
        if let Some(v) = p.pop_size { ga_params.pop_size = v.max(8); }
        if let Some(v) = p.generations { ga_params.generations = v.max(1); }
        if let Some(v) = p.top_n { ga_params.top_n = v.max(1); }
        ga_params.fitness_dps_mode = p.fitness_dps_mode;
    }

    let run_id = new_run_id("phase3");

    // 规则池摘要（写入 meta，前端 TopN 反查时可用）
    let pool_summary: Vec<_> = pool.rules.values().map(|r| serde_json::json!({
        "id": r.id, "name": r.name, "page": r.page,
        "source": r.source, "locked": r.locked, "line_text": r.line_text,
    })).collect();

    let meta = serde_json::json!({
        "run_id": run_id,
        "phase": "phase3",
        "start_time_unix": now_unix(),
        "duration": req.duration,
        "haste_level": req.haste_level,
        "network_delay": req.network_delay,
        "initial_rage": req.initial_rage,
        "talents": req.talents,
        "recipes": req.recipes,
        "attrs": req.attrs,
        "target": req.target,
        "ga": {
            "pop_size": ga_params.pop_size,
            "generations": ga_params.generations,
            "top_n": ga_params.top_n,
        },
        "n_params": effective.len(),
        "n_all_params": tunables.len(),
        "pool": pool_summary,
        "shield_default_order": pool.shield_default_order,
        "blade_default_order": pool.blade_default_order,
    });

    let mut archive = match ArchiveWriter::new(run_id.clone(), &meta, &req.base_loop) {
        Ok(a) => a,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(err_body(&format!("存档创建失败: {}", e)))).into_response(),
    };

    // 5. StructCtx
    let ctx = Arc::new(StructCtx {
        pool,
        param_keys: kept_keys.clone(),
        tunables: effective.clone(),
        attrs: req.attrs,
        target: req.target,
        talents: req.talents,
        recipes: req.recipes,
        initial_rage: req.initial_rage,
        haste_level: req.haste_level,
        network_delay_ms: req.network_delay,
        duration_min: req.duration_min.unwrap_or(req.duration).max(1.0),
        duration_max: req.duration_max.unwrap_or(req.duration).max(1.0),
        samples_per_eval: req.samples_per_eval.unwrap_or(3).max(1),
        skills: Arc::new(shared.skills.read().await.clone()),
        recipes_table: Arc::new(shared.recipes.read().await.clone()),
    });

    let stop = Arc::new(AtomicBool::new(false));
    let active = ActiveRun {
        run_id: run_id.clone(),
        stop: stop.clone(),
        total_gens: ga_params.generations,
    };
    *guard = Some(active);
    drop(guard);

    let tx = opt.broadcaster.clone();
    let enabled_ids: Vec<String> = effective.iter().map(|t| t.param.id.clone()).collect();
    let _ = tx.send(GaEvent::Started {
        run_id: run_id.clone(),
        total_gens: ga_params.generations,
        pop_size: ga_params.pop_size,
        n_params: effective.len(),
        enabled_ids: enabled_ids.clone(),
    });

    let opt_state_clone = opt.clone();
    let run_id_clone = run_id.clone();
    let base_loop = req.base_loop.clone();
    let ga_params_clone = ga_params.clone();
    tokio::task::spawn_blocking(move || {
        let sink: ProgressSink = {
            let tx = tx.clone();
            Box::new(move |ev: GaEvent| { let _ = tx.send(ev); })
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut run = StructGaRun {
                ctx: &ctx,
                params: &ga_params_clone,
                base_loop: &base_loop,
                archive: &mut archive,
                sink: &sink,
                stop: stop.clone(),
            };
            run.run()
        }));

        let (best_dps, gens_completed) = match result {
            Ok((_ind, best, gens)) => (best, gens),
            Err(_) => {
                let _ = tx.send(GaEvent::Error { message: "GA 运行异常崩溃".into() });
                (0.0, 0)
            }
        };

        let _ = tx.send(GaEvent::Done { best_dps, total_gens: gens_completed });

        let rt = tokio::runtime::Handle::try_current();
        if let Ok(handle) = rt {
            handle.block_on(async {
                let mut g = opt_state_clone.current.lock().await;
                if let Some(r) = g.as_ref() {
                    if r.run_id == run_id_clone { *g = None; }
                }
            });
        } else {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let mut g = opt_state_clone.current.lock().await;
                if let Some(r) = g.as_ref() {
                    if r.run_id == run_id_clone { *g = None; }
                }
            });
        }
    });

    Json(StartResponse {
        run_id,
        n_params: effective.len(),
        pop_size: ga_params.pop_size,
        generations: ga_params.generations,
        enabled_ids,
    }).into_response()
}
