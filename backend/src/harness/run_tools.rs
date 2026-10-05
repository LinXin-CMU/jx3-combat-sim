//! Composable laboratory tools. No tool advances a prescribed workflow.
use super::run_schema::*;
use crate::{
    agent::{hash::canonical_sha256, provider::ToolDefinition, AgentRuntime, ScenarioSnapshotV1},
    SimulateRequest, SimulateResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {name:"inspect".into(),description:"按需读取冻结现场与证据，不触发模拟。skills默认给分页紧凑目录，query为完整技能名/ID时才返回详细定义，模糊名称只筛选目录。artifact默认给指标与首分歧摘要；query=simulation/equipment/diagnosis分别读完整应用场景/配装/诊断，baseline_simulation/evaluation_simulation读取对照/实际验证场景。timeline独立分页，query=reference读取对照。evidence按创建顺序分页查找任意历史分支；用offset/limit避免重复读取整包。".into(),parameters:json!({"type":"object","additionalProperties":false,"properties":{"section":{"type":"string","enum":["scene","skills","artifact","timeline","equipment_catalog","macro_language","evidence"]},"query":{"type":"string","description":"skills:完整名称/ID取详情，部分名称筛目录；artifact:省略取摘要，或simulation/equipment/diagnosis/baseline_simulation/evaluation_simulation；timeline:reference为对照，省略为候选。"},"artifact_id":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":24}},"required":["section"]})},
        ToolDefinition {name:"experiment".into(),description:"执行一次可组合实验：evaluate直接测试你自由撰写的宏/技能序列；compile_macro局部宏求解；search_rotation搜索输出宏；optimize_equipment搜索配装；validate独立回放已有候选。通过parent_id继承任何已测候选，实现联合实验。每次必须写可证伪假设，失败后可以直接修改宏或换策略，无固定流程。".into(),parameters:json!({"type":"object","additionalProperties":false,"properties":{
            "kind":{"type":"string","enum":["evaluate","compile_macro","search_rotation","optimize_equipment","validate"]},
            "hypothesis":{"type":"string"},"parent_id":{"type":"string"},"macro_text":{"type":"string"},
            "sequence":{"type":"array","items":{"type":"string"},"maxItems":2048},
            "max_simulations":{"type":"integer","minimum":2,"maximum":256},
            "network_delay":{"type":"integer","minimum":0,"maximum":10000,"description":"仅kind=validate可提供；其他实验请省略此字段，保持冻结环境。"},"seed":{"type":"integer","minimum":0,"description":"仅kind=validate可提供；evaluate/search/compile不得填写（包括0）。"},
            "equipment":{"type":"object","additionalProperties":false,"description":"完整12槽快照；锁定部位必须与原始配装一致。每个slot含equip_id/strength/embedding/enhance_id/enchant_id。","properties":{"slots":{"type":"object","additionalProperties":{"type":"object","additionalProperties":false,"properties":{"equip_id":{"type":"integer","minimum":0},"strength":{"type":"integer","minimum":0,"maximum":8},"embedding":{"type":"array","items":{"type":"integer","minimum":0,"maximum":8},"maxItems":3},"enhance_id":{"type":"integer","minimum":0},"enchant_id":{"type":"integer","minimum":0}},"required":["equip_id","strength","embedding","enhance_id","enchant_id"]},"required":["HAT","JACKET","BELT","WRIST","BOTTOMS","SHOES","NECKLACE","PENDANT","RING_1","RING_2","PRIMARY_WEAPON","SECONDARY_WEAPON"]},"stone_id":{"type":"integer","minimum":0},"source_label":{"type":"string"}},"required":["slots"]}
        },"required":["kind","hypothesis"]})},
        ToolDefinition {name:"record_learning".into(),description:"记录有证据支撑的失败原因或下一实验决策，保留到本任务恢复上下文。不是修改全局提示词或武学数据。".into(),parameters:json!({"type":"object","additionalProperties":false,"properties":{"observation":{"type":"string"},"evidence_ids":{"type":"array","items":{"type":"string"},"maxItems":8}},"required":["observation","evidence_ids"]})},
        ToolDefinition {name:"finish".into(),description:"交付已有实测候选或明确未解决原因。artifact_id只能选现有证据，不能用文字宣称不存在的验证。summary解释定性结论，具体数值由界面从证据显示。可交付部分方案，不能把失败/缺少验证称成功。".into(),parameters:json!({"type":"object","additionalProperties":false,"properties":{"artifact_id":{"type":"string"},"summary":{"type":"string"},"limitations":{"type":"array","items":{"type":"string"},"maxItems":12}},"required":["summary"]})},
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Experiment {
    pub kind: String,
    pub hypothesis: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub macro_text: Option<String>,
    #[serde(default)]
    pub sequence: Option<Vec<String>>,
    #[serde(default)]
    pub equipment: Option<super::equipment::EquipmentSnapshot>,
    #[serde(default = "default_calls")]
    pub max_simulations: u32,
    #[serde(default)]
    pub network_delay: Option<u32>,
    #[serde(default)]
    pub seed: Option<u32>,
}
fn default_calls() -> u32 {
    24
}

pub fn inspect(state: &Checkpoint, runtime: &AgentRuntime, args: &Value) -> Result<Value, String> {
    let section = args["section"].as_str().ok_or("缺少 section")?;
    let query = args["query"].as_str().unwrap_or("").trim();
    let limit = args["limit"].as_u64().unwrap_or(12).clamp(1, 24) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0).min(2048) as usize;
    match section {
        "evidence" => Ok(
            json!({"artifacts":state.artifacts.iter().skip(offset).take(limit).map(compact_artifact).collect::<Vec<_>>(),
            "total":state.artifacts.len(),"offset":offset,"limit":limit,"order":"creation_id"}),
        ),
        "macro_language" => {
            let examples = [
                "/cast [rage>49] 盾刀",
                "/cast [energy>=10] 盾刀",
                "/cast [sun<40] 盾刀",
                "/cast [buff:血怒&nobuff:盾飞] 盾刀",
                "/cast [bufftime:血怒<1|skill_energy:血怒>1] 血怒",
                "/cast [buff:血怒=2] 血怒",
                "/cast [tbuff:流血&tnobuff:虚弱] 盾刀",
                "/cast [tbufftime:流血<2] 盾刀",
                "/cast [skill_notin_cd:盾猛] 盾猛",
                "/cast [skill:13044&noskill:90001] 盾刀",
                "/cast [last_skill~=盾猛] 盾刀",
                "/fcast [life<0.5&nearby_enemy>0] 盾壁",
            ];
            Ok(
                json!({"source":"current macro_parser + macro_engine","commands":["/cast 技能名","/cast [条件] 技能名","/fcast [条件] 技能名"],
                "pages":["#page","#page shield","#page blade","#page wall"],"page_limit_utf16":128,
                "max_pages":state.request.constraints.max_pages,"comparisons":[">","<","=",">=","<=","~="],
                "boolean":"& 与 | 同优先级，右结合；当前解析器不支持括号分组。","examples":examples,
                "notes":["energy为格挡值；sun/berserk/baonu为暴怒值。","buff:X=N是自身气劲层数条件；skill_energy:X>N是充能层数。","不同姿态页需按姿态手动切换使用。","语法被解析器支持不等于当前版本技能可用；请inspect skills并完整回放。"]}),
            )
        }
        "scene" => Ok(
            json!({"goal":state.request.goal,"version":state.request.version,"mount":state.request.mount,
            "scenario_hash":state.scenario.scenario_hash,"simulation":state.scenario.simulation,
            "equipment":state.request.equipment,"constraints":state.request.constraints,"budget":state.request.budget,"usage":state.usage,
            "artifacts":state.artifacts.iter().map(compact_artifact).collect::<Vec<_>>() }),
        ),
        "skills" => {
            let context = runtime.context();
            let matching = context
                .skills
                .iter()
                .filter(|s| {
                    query.is_empty() || s.name.contains(query) || s.skill_id.to_string() == query
                })
                .filter(|s| {
                    s.requires_talent
                        .is_none_or(|id| state.request.simulation.talents.contains(&id))
                })
                .collect::<Vec<_>>();
            let exact = !query.is_empty()
                && matching
                    .iter()
                    .any(|s| s.name == query || s.skill_id.to_string() == query);
            let skills = matching
                .iter()
                .skip(offset)
                .take(limit)
                .map(|s| {
                    if exact && (s.name == query || s.skill_id.to_string() == query) {
                        serde_json::to_value(s).expect("serializable skill definition")
                    } else {
                        skill_digest(s)
                    }
                })
                .collect::<Vec<_>>();
            let talents = context
                .talents
                .iter()
                .filter(|t| {
                    if query.is_empty() {
                        state.request.simulation.talents.contains(&t.id)
                    } else {
                        t.name == query || t.id.to_string() == query
                    }
                })
                .map(|t| {
                    if query.is_empty() {
                        json!({"id":t.id,"name":t.name,"tier":t.tier})
                    } else {
                        serde_json::to_value(t).expect("serializable talent definition")
                    }
                })
                .collect::<Vec<_>>();
            Ok(
                json!({"skills":skills,"talents":talents,"offset":offset,"limit":limit,"total":matching.len(),
                    "next_offset":(offset.saturating_add(limit)<matching.len()).then_some(offset.saturating_add(limit)),
                    "version":state.request.version,"mount":state.request.mount,
                    "view":if exact {"matched_definitions"} else {"compact_catalog"},
                    "note":"目录保留当前天赋解锁技能（含被动标记）的基础CD、资源、姿态、引导与连招字段；不等于运行时可释放性。空query仅列已选奇穴摘要；完整技能/奇穴名称或ID取详细定义，部分技能名筛目录。未列出伤害系数或脚本修正不表示为零；allowed_skills硬约束见scene。offset/limit继续翻页。"}),
            )
        }
        "artifact" | "timeline" => {
            let artifact = find_artifact(
                state,
                args["artifact_id"].as_str().ok_or("缺少 artifact_id")?,
            )?;
            if section == "artifact" {
                let mut value = match query {
                    "" => compact_artifact(artifact),
                    "simulation" => json!({"id":artifact.id,"scenario_hash":artifact.scenario_hash,
                        "simulation":artifact.simulation,"scope":"application_scene"}),
                    "baseline_simulation" | "evaluation_simulation" => json!({"id":artifact.id,
                        "simulation":artifact.result.pointer(if query=="baseline_simulation" {"/baseline/simulation"} else {"/best/simulation"}),"scope":query}),
                    "equipment" => json!({"id":artifact.id,"equipment":artifact.equipment,"slot_diff":artifact.result.get("slot_diff")}),
                    "diagnosis" => json!({"id":artifact.id,"diagnosis":artifact.result.pointer("/best/diagnosis"),
                        "first_difference":artifact.result.pointer("/best/first_difference"),
                        "alignment_summary":artifact.result.pointer("/best/alignment/summary"),
                        "constraint_error":artifact.result.pointer("/best/constraint_error"),
                        "alignment_error":artifact.result.pointer("/best/alignment_error"),
                        "skipped":artifact.result.pointer("/best/skipped"),"error":artifact.result.get("error")}),
                    _ => return Err("artifact query仅支持simulation/equipment/diagnosis/baseline_simulation/evaluation_simulation；省略query读取摘要。时间轴使用section=timeline分页。".into()),
                };
                value["read_more"] = json!({"section":"artifact","artifact_id":artifact.id,
                    "queries":["simulation","equipment","diagnosis","baseline_simulation","evaluation_simulation"],
                    "timeline":"section=timeline，query=reference为对照；offset/limit分页。simulation是可应用场景；evaluation_simulation可能是不同延迟/种子的验证场景。"});
                return Ok(value);
            }
            let timeline = artifact
                .result
                .get(if query == "reference" {
                    "reference_timeline"
                } else {
                    "timeline"
                })
                .and_then(Value::as_array);
            Ok(
                json!({"artifact_id":artifact.id,"timeline":timeline.map(|t|t.iter().skip(offset).take(limit).collect::<Vec<_>>()),
                "offset":offset,"limit":limit,"total":timeline.map(Vec::len),"truncated":artifact.result.get("timeline_truncated"),
                "note":"query=reference读取对照轴；否则读取候选轴。offset是已记录主动事件列表位置；每行index是原始完整timeline索引。诊断使用section=artifact/query=diagnosis，避免每页重复整包。"}),
            )
        }
        "equipment_catalog" => {
            let original = state
                .request
                .equipment
                .clone()
                .ok_or("缺少完整原始配装；目录候选不能代替真实装备基线。")?;
            let request = equipment_request(
                state,
                macro_request(
                    &state.scenario.simulation,
                    "/cast 盾刀",
                    state.request.constraints.duration_seconds,
                ),
                original,
                2,
                1000,
            )?;
            request.validate()?;
            let mut items = Vec::new();
            for position in super::equipment::POSITIONS {
                if request.locked_slots.iter().any(|p| p == position) {
                    continue;
                }
                for item in runtime.equipment_items() {
                    if !query.is_empty()
                        && query != position
                        && !item.name.contains(query)
                        && item.id.to_string() != query
                    {
                        continue;
                    }
                    if super::equipment::eligible(&request, runtime, position, item)
                        && (request.candidate_source == super::equipment::CandidateSource::Catalog
                            || request
                                .candidate_ids
                                .get(position)
                                .is_some_and(|ids| ids.contains(&item.id)))
                    {
                        items.push((position, item));
                    }
                }
            }
            items.sort_by_key(|(position, item)| (item.sub_type, item.id, *position));
            let total = items.len();
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|(position, item)| json!({"position":position,"item":item}))
                .collect::<Vec<_>>();
            Ok(
                json!({"items":items,"offset":offset,"limit":limit,"total":total,"note":"已过滤当前版本/心法、部位、锁定、来源、品级和ID白名单；保留原槽强化附魔。全配装加速约束在evaluate再次核验。query可填HAT等部位名。"}),
            )
        }
        _ => Err("未知观察范围。".into()),
    }
}

fn skill_digest(skill: &crate::SkillSpec) -> Value {
    let mut value = json!({"skill_id":skill.skill_id,"name":skill.name,"passive":skill.passive,
        "stance":skill.stance,"requires_talent":skill.requires_talent,
        "rage_cost":skill.rage_cost,"rage_gain":skill.rage_gain,"cooldowns":skill.cooldowns,
        "stance_change":skill.stance_change,"max_charges":skill.max_charges,"charge_cd":skill.charge_cd,
        "channel_frame":skill.channel_frame,"channel_interval":skill.channel_interval,"first_tick_frame":skill.first_tick_frame,
        "requires_combo":skill.requires_combo,"grants_combo":skill.grants_combo,"combo_duration":skill.combo_duration,
        "combo_follow":skill.combo_follow});
    value
        .as_object_mut()
        .unwrap()
        .retain(|_, v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty));
    value
}

pub fn find_artifact<'a>(state: &'a Checkpoint, id: &str) -> Result<&'a Artifact, String> {
    state
        .artifacts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "候选证据不存在，不能引用未测试方案。".into())
}
pub fn compact_artifact(a: &Artifact) -> Value {
    json!({"id":a.id,"parent_id":a.parent_id,"kind":a.kind,"summary":a.summary,"scenario_hash":a.scenario_hash,
        "baseline":a.result.get("baseline").map(candidate_digest),"best":a.result.get("best").map(candidate_digest),
        "validation":a.result.get("validation").map(validation_digest),"slot_diff":a.result.get("slot_diff"),"stop_reason":a.result.get("stop_reason"),
        "dps_comparable":a.result.get("dps_comparable"),"improvement_pct":a.result.get("improvement_pct"),
        "dps_delta":a.result.get("dps_delta"),"comparison_scope":a.result.get("comparison_scope"),
        "comparison_reference_hash":a.result.get("comparison_reference_hash"),
        "baseline_policy":a.result.get("baseline_policy"),"candidate_policy":a.result.get("candidate_policy"),
        "limitations":a.result.get("limitations"),"ok":a.result.get("ok"),"error":a.result.get("error"),"simulations":a.result.get("simulations")})
}
/// Validation replays can carry complete candidates with hundreds of macro
/// sequence placeholders. The semantic ledger needs outcomes and identities,
/// while full replay inputs remain accessible in the immutable artifact.
fn validation_digest(validation: &Value) -> Value {
    let Some(object) = validation.as_object() else {
        return validation.clone();
    };
    let mut digest = serde_json::Map::new();
    for (key, value) in object {
        if matches!(key.as_str(), "baseline" | "best") {
            digest.insert(
                key.clone(),
                if value.is_object() {
                    candidate_digest(value)
                } else {
                    Value::Null
                },
            );
        } else if !value.is_object()
            && !value.is_array()
            && !matches!(
                key.as_str(),
                "simulation"
                    | "pages"
                    | "timeline"
                    | "trajectory"
                    | "reference_timeline"
                    | "macro_trace"
            )
        {
            digest.insert(key.clone(), value.clone());
        }
    }
    Value::Object(digest)
}
fn candidate_digest(c: &Value) -> Value {
    let metrics=c.get("metrics").cloned().unwrap_or_else(||json!({"dps":c.get("dps"),"total_damage":c.get("total_damage"),"fight_time":c.get("fight_time"),"skipped_count":c.get("skipped_count")}));
    json!({"macro_text":c.get("macro_text"),"metrics":metrics,"fingerprint":c.get("fingerprint"),"policy":c.get("policy"),
        "reproduced":c.get("reproduced"),"verified":c.get("verified"),"constraints_passed":c.get("constraints_passed"),
        "page_constraints_passed":c.get("page_constraints_passed"),"alignment":c.pointer("/alignment/summary"),
        "first_difference":c.get("first_difference").map(first_difference_digest)})
}

fn selected_fields(value: &Value, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|key| value.get(*key).map(|v| ((*key).to_string(), v.clone())))
            .collect(),
    )
}

/// Preserve the counterexample needed for the next experiment, not the entire
/// skill-state/buff catalog duplicated inside each event snapshot. Full recorded
/// states remain available through explicit diagnosis or paginated timeline.
fn first_difference_digest(value: &Value) -> Value {
    if !value.is_object() {
        return value.clone();
    }
    let mut digest = selected_fields(
        value,
        &[
            "kind",
            "reference_index",
            "actual_index",
            "reference_offset",
            "actual_offset",
            "time_delta",
        ],
    );
    if let Some(diffs) = value.get("resource_diffs").and_then(Value::as_array) {
        digest["resource_diffs"] = json!(diffs
            .iter()
            .take(8)
            .map(|diff| selected_fields(diff, &["field", "reference", "actual"]))
            .collect::<Vec<_>>());
    }
    for side in ["reference", "actual"] {
        let Some(event) = value.get(side) else {
            continue;
        };
        if event.is_null() {
            digest[side] = Value::Null;
            continue;
        }
        let mut action = selected_fields(
            event,
            &[
                "name",
                "skill_id",
                "cast_time",
                "channel_ticks",
                "rage_after",
                "rage_delta",
            ],
        );
        if let Some(state) = event.get("state_before") {
            action["state_before"] = if state.is_null() {
                Value::Null
            } else {
                selected_fields(
                    state,
                    &[
                        "time",
                        "stance",
                        "rage",
                        "block_value",
                        "berserk_value",
                        "max_berserk_value",
                    ],
                )
            };
        }
        digest[side] = action;
    }
    digest
}

pub struct ExperimentOutput {
    pub result: Value,
    pub simulation: SimulateRequest,
    pub equipment: Option<super::equipment::EquipmentSnapshot>,
    pub simulations: u32,
}

pub fn execute(
    state: &Checkpoint,
    runtime: &AgentRuntime,
    experiment: &Experiment,
    cancel: &AtomicBool,
    wall_time_ms: u64,
    mut progress: impl FnMut(Value),
) -> Result<ExperimentOutput, String> {
    let charged = Cell::new(0_u32);
    let mut emit = |value: Value| {
        if let Some(n) = value["simulations"].as_u64() {
            charged.set(charged.get().max(n.min(u64::from(u32::MAX)) as u32));
        }
        progress(value);
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_inner(state, runtime, experiment, cancel, wall_time_ms, &mut emit)
    }));
    match outcome {
        Ok(Ok(output)) => Ok(output),
        other => {
            let error = match other {
                Ok(Err(error)) => error,
                _ => "模拟器实验异常，已保留此前证据和实际进入模拟的次数。".into(),
            };
            let parent = experiment
                .parent_id
                .as_deref()
                .and_then(|id| find_artifact(state, id).ok());
            let reason = if cancel.load(Ordering::Relaxed) {
                "cancelled"
            } else if charged.get() == 0 {
                "rejected"
            } else {
                "evaluation_failed"
            };
            Ok(ExperimentOutput {
                result: json!({"ok":false,"task":experiment.kind,"stop_reason":reason,"simulations":charged.get(),
                    "best":null,"error":error,"limitations":["本次没有产生可接受的新候选；失败证据仍可用于下一次实验。"]}),
                simulation: parent.map_or_else(
                    || state.scenario.simulation.clone(),
                    |a| a.simulation.clone(),
                ),
                equipment: parent
                    .and_then(|a| a.equipment.clone())
                    .or_else(|| state.request.equipment.clone()),
                simulations: charged.get(),
            })
        }
    }
}

fn available_commands(runtime: &AgentRuntime, simulation: &SimulateRequest) -> BTreeSet<String> {
    runtime
        .context()
        .skills
        .iter()
        .filter(|s| {
            !s.passive
                && s.skill_id != 90001
                && s.requires_talent
                    .is_none_or(|id| simulation.talents.contains(&id))
        })
        .map(|s| {
            if (90010..=90012).contains(&s.skill_id) {
                s.name.clone()
            } else {
                s.name.split('·').next().unwrap_or(&s.name).into()
            }
        })
        .collect()
}

fn rotation_constraints(
    sim: &SimulateRequest,
    state: &Checkpoint,
    runtime: &AgentRuntime,
) -> Result<(), String> {
    let available = available_commands(runtime, sim);
    let allowed: BTreeSet<_> = state
        .request
        .constraints
        .allowed_skills
        .iter()
        .cloned()
        .collect();
    let commands: Vec<String> =
        if let Some(text) = sim.macro_text.as_deref().filter(|m| !m.trim().is_empty()) {
            let config = crate::macro_parser::parse_macro_text(text).map_err(|e| e.to_string())?;
            let pages = super::compiler::pages(text);
            if pages.is_empty()
                || pages.len() > state.request.constraints.max_pages
                || pages.iter().any(|p| !p.within_limit)
            {
                return Err("宏超过允许页数或每页 128 字符限制。".into());
            }
            config
                .pages
                .iter()
                .flat_map(|p| p.lines.iter().map(|l| l.action.skill_name().to_owned()))
                .collect()
        } else {
            sim.sequence.clone()
        };
    if commands.is_empty() || commands.iter().any(|s| !available.contains(s)) {
        return Err("候选含当前版本、心法或奇穴下不可用的动作，或使用模拟器专用操作。".into());
    }
    if !allowed.is_empty() && commands.iter().any(|s| !allowed.contains(s)) {
        return Err("候选动作超出本任务 allowed_skills 硬约束。".into());
    }
    Ok(())
}

fn equipment_request(
    state: &Checkpoint,
    simulation: SimulateRequest,
    equipment: super::equipment::EquipmentSnapshot,
    simulations: u32,
    wall: u64,
) -> Result<super::equipment::EquipmentRequest, String> {
    let mut value = serde_json::to_value(&state.request.constraints).map_err(|e| e.to_string())?;
    let object = value.as_object_mut().ok_or("约束必须为对象")?;
    object.remove("allowed_skills");
    object.remove("max_pages");
    object.insert("simulation".into(), json!(simulation));
    object.insert("equipment".into(), json!(equipment));
    object.insert("version".into(), json!(state.request.version));
    object.insert("mount".into(), json!(state.request.mount));
    object.insert("max_simulations".into(), json!(simulations));
    object.insert("wall_time_ms".into(), json!(wall.clamp(1000, 120000)));
    object.insert("max_rounds".into(), json!(6));
    serde_json::from_value(value).map_err(|e| format!("配装约束无效：{e}"))
}

/// Zero-replay validation always anchors locks and acquisition constraints to the
/// original Run equipment, even after an arbitrary chain of parent artifacts.
fn rebuild_equipment(
    state: &Checkpoint,
    runtime: &AgentRuntime,
    simulation: &SimulateRequest,
    equipment: &super::equipment::EquipmentSnapshot,
    baseline: bool,
) -> Result<SimulateRequest, String> {
    let original = state
        .request
        .equipment
        .clone()
        .ok_or("缺少原始完整配装，不能从模型候选伪造配装基线。")?;
    // Attribute calculation is independent of rotation. A validation-only macro
    // adapts the equipment operator without forcing a user axis through compile.
    let probe = macro_request(
        simulation,
        "/cast 盾刀",
        state.request.constraints.duration_seconds,
    );
    let mut request = equipment_request(state, probe.clone(), original, 2, 1000)?;
    if baseline {
        request.haste_min = None;
        request.haste_max = None;
    }
    let scenario = ScenarioSnapshotV1::capture(request.version, request.mount, probe)
        .map_err(|e| e.to_string())?;
    let rebuilt = super::equipment::evaluate_snapshot(&request, runtime, &scenario, equipment)?;
    let mut result = simulation.clone();
    result.attributes = rebuilt.attributes;
    result.haste_level = rebuilt.haste_level;
    result.equipment = rebuilt.equipment;
    Ok(result)
}

fn baseline_equipment(
    state: &Checkpoint,
    parent: Option<&Artifact>,
) -> Option<super::equipment::EquipmentSnapshot> {
    parent
        .and_then(|a| a.equipment.clone())
        .or_else(|| state.request.equipment.clone())
}

fn execute_inner(
    state: &Checkpoint,
    runtime: &AgentRuntime,
    experiment: &Experiment,
    cancel: &AtomicBool,
    wall_time_ms: u64,
    progress: &mut impl FnMut(Value),
) -> Result<ExperimentOutput, String> {
    if experiment.hypothesis.trim().is_empty() || experiment.hypothesis.len() > 2000 {
        return Err("实验须有简短明确的假设。".into());
    }
    if !(2..=256).contains(&experiment.max_simulations) || wall_time_ms < 1000 {
        return Err("本次实验的模拟或剩余时间预算不足。".into());
    }
    if runtime.game_version() != state.request.version || runtime.mount() != state.request.mount {
        return Err("实验运行时与冻结版本或心法不一致。".into());
    }
    state.scenario.verify_hash().map_err(|e| e.to_string())?;
    if state.request.scenario()?.scenario_hash != state.scenario.scenario_hash {
        return Err("任务请求与冻结场景不一致。".into());
    }
    let parent = experiment
        .parent_id
        .as_deref()
        .map(|id| find_artifact(state, id))
        .transpose()?;
    let mut original = parent.map_or_else(
        || state.scenario.simulation.clone(),
        |a| a.simulation.clone(),
    );
    let original_equipment = baseline_equipment(state, parent);
    // Baseline panels are rebuilt from actual slots, never from arbitrary model
    // attribute values; initial equipment remains the permanent constraint anchor.
    if let Some(eq) = &original_equipment {
        original = rebuild_equipment(state, runtime, &original, eq, true)?;
    }
    original.lite = false;
    original.lite_keep_timeline = false;
    let mut simulation = original.clone();
    let mut equipment = original_equipment.clone();
    if experiment.kind != "compile_macro"
        && experiment.macro_text.is_some()
        && experiment.sequence.is_some()
    {
        return Err("单次实验不能同时提供宏和技能序列。".into());
    }
    if experiment.kind == "validate" {
        if parent.is_none() {
            return Err("validate 需要引用已有证据 parent_id。".into());
        }
        if experiment.macro_text.is_some()
            || experiment.sequence.is_some()
            || experiment.equipment.is_some()
        {
            return Err(
                "validate 只能复跑父候选或改变延迟/种子；修改宏、轴或装备应使用 evaluate。".into(),
            );
        }
    } else if experiment.network_delay.is_some() || experiment.seed.is_some() {
        return Err("延迟和种子只能在 validate 中改变，不能伪造同环境优化收益。".into());
    }
    if let Some(text) = &experiment.macro_text {
        simulation = macro_request(
            &simulation,
            &super::compiler::normalize_game_macro(text)?,
            state.request.constraints.duration_seconds,
        );
    }
    if let Some(sequence) = &experiment.sequence {
        simulation.sequence = sequence.clone();
        simulation.macro_text = None;
        simulation.macro_duration = None;
        simulation.channel_ticks.clear();
        simulation.timing_offsets.clear();
        simulation.qijin_buffs.clear();
    }
    if let Some(eq) = &experiment.equipment {
        simulation = rebuild_equipment(
            state,
            runtime,
            &simulation,
            eq,
            experiment.kind == "optimize_equipment",
        )?;
        equipment = Some(eq.clone());
    } else if let Some(eq) = &equipment {
        // Parent candidates cannot turn an earlier out-of-constraint observation
        // into a new valid starting equipment configuration.
        simulation = rebuild_equipment(
            state,
            runtime,
            &simulation,
            eq,
            experiment.kind == "optimize_equipment",
        )?;
    }
    if let Some(delay) = experiment.network_delay {
        simulation.network_delay = delay;
    }
    if let Some(seed) = experiment.seed {
        simulation.dunya_reset_seed = seed;
    }
    if let Some(text) = simulation
        .macro_text
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        let duration = if experiment.kind == "validate" {
            simulation
                .macro_duration
                .unwrap_or(state.request.constraints.duration_seconds)
        } else {
            state.request.constraints.duration_seconds
        };
        simulation = macro_request(
            &simulation,
            &super::compiler::normalize_game_macro(&text)?,
            duration,
        );
    }
    validate_environment(&simulation, state.request.version, state.request.mount)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(ExperimentOutput {
            result: json!({"task":experiment.kind,"stop_reason":"cancelled","simulations":0,"best":null}),
            simulation: original,
            equipment: original_equipment,
            simulations: 0,
        });
    }
    let mut result = match experiment.kind.as_str() {
        "evaluate" | "validate" => {
            return evaluate_atomic(
                state,
                runtime,
                experiment,
                parent,
                original,
                original_equipment,
                simulation,
                equipment,
                cancel,
                wall_time_ms,
                progress,
            )
        }
        "compile_macro" => {
            let mut reference = original.clone();
            if let Some(sequence) = &experiment.sequence {
                reference.sequence = sequence.clone();
                reference.macro_text = None;
                reference.macro_duration = None;
                reference.channel_ticks.clear();
                reference.timing_offsets.clear();
                reference.qijin_buffs.clear();
            }
            if let Some(eq) = &equipment {
                reference = rebuild_equipment(state, runtime, &reference, eq, false)?;
            }
            if reference
                .macro_text
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            {
                return Err("compile_macro 需要手动目标轴；可直接提交 sequence 指定目标，无需切换用户预设。".into());
            }
            rotation_constraints(&reference, state, runtime)?;
            let req = super::contract::MacroCompileRequestV1 {
                simulation: reference.clone(),
                version: state.request.version,
                mount: state.request.mount,
                initial_macro: experiment.macro_text.clone(),
                max_simulations: experiment.max_simulations,
                wall_time_ms: wall_time_ms.clamp(1000, 120000),
                max_rounds: 6,
                max_pages: state.request.constraints.max_pages,
                time_tolerance: 1.0 / 16.0,
            };
            let scenario = req.snapshot(runtime).map_err(str::to_owned)?;
            let compiled = super::compiler::compile(&req, runtime, &scenario, cancel, |p| {
                progress(serde_json::to_value(p).unwrap_or(Value::Null))
            })?;
            if let Some(best) = &compiled.best {
                simulation = macro_request(&reference, &best.macro_text, compiled.window_seconds);
            }
            let mut value = serde_json::to_value(compiled).map_err(|e| e.to_string())?;
            value["task"] = json!("compile_macro");
            if value["baseline"].is_object() {
                let metrics = value["baseline"].clone();
                value["baseline"] = json!({"metrics":metrics,"fingerprint":value["baseline_fingerprint"],"simulation":reference});
            }
            value
        }
        "search_rotation" => {
            let req = super::rotation::RotationRequest {
                simulation: simulation.clone(),
                version: state.request.version,
                mount: state.request.mount,
                initial_macro: simulation.macro_text.clone(),
                allowed_skills: state.request.constraints.allowed_skills.clone(),
                max_simulations: experiment.max_simulations,
                wall_time_ms: wall_time_ms.clamp(1000, 120000),
                max_rounds: 6,
                duration_seconds: state.request.constraints.duration_seconds,
                max_pages: state.request.constraints.max_pages,
            };
            let scenario = req.snapshot(runtime).map_err(str::to_owned)?;
            super::rotation::run(&req, runtime, &scenario, cancel, &mut *progress)?
        }
        "optimize_equipment" => {
            rotation_constraints(&simulation, state, runtime)?;
            let eq = equipment
                .clone()
                .ok_or("缺少完整当前配装，不能证明换装收益。")?;
            let req = equipment_request(
                state,
                simulation.clone(),
                eq,
                experiment.max_simulations,
                wall_time_ms,
            )?;
            req.validate()?;
            let scenario =
                ScenarioSnapshotV1::capture(req.version, req.mount, req.simulation.clone())
                    .map_err(|e| e.to_string())?;
            super::equipment::run(&req, runtime, &scenario, cancel, &mut *progress)?
        }
        _ => return Err("未注册的实验类型。".into()),
    };
    if let Some(value) = result.pointer("/best/simulation").filter(|v| v.is_object()) {
        simulation = serde_json::from_value(value.clone()).map_err(|_| "候选场景无效")?;
    }
    if let Some(value) = result.pointer("/best/equipment").filter(|v| v.is_object()) {
        equipment = Some(serde_json::from_value(value.clone()).map_err(|_| "候选装备无效")?);
    }
    let mut constraint_error = rotation_constraints(&simulation, state, runtime).err();
    if let Some(eq) = &equipment {
        if let Err(error) = rebuild_equipment(state, runtime, &simulation, eq, false) {
            constraint_error = Some(error);
        }
    }
    if result["best"].is_object() {
        result["best"]["simulation"] = json!(simulation);
        result["best"]["macro_text"] = json!(simulation.macro_text);
        result["best"]["pages"] = json!(simulation
            .macro_text
            .as_deref()
            .map(super::compiler::pages)
            .unwrap_or_default());
        result["best"]["constraints_passed"] = json!(
            constraint_error.is_none()
                && result["best"].get("constraints_passed") != Some(&Value::Bool(false))
        );
        if let Some(error) = constraint_error {
            result["best"]["verified"] = json!(false);
            result["best"]["reproduced"] = json!(false);
            result["best"]["constraint_error"] = json!(error);
        }
    }
    let simulations = result["simulations"].as_u64().unwrap_or(0) as u32;
    Ok(ExperimentOutput {
        result,
        simulation,
        equipment,
        simulations,
    })
}

struct AtomicBudget<'a> {
    cancel: &'a AtomicBool,
    started: Instant,
    wall: u64,
    maximum: u32,
    used: u32,
}
impl AtomicBudget<'_> {
    fn stop(&self) -> Option<&'static str> {
        if self.cancel.load(Ordering::Relaxed) {
            Some("cancelled")
        } else if self.started.elapsed().as_millis() >= u128::from(self.wall) {
            Some("time_budget")
        } else if self.used >= self.maximum {
            Some("budget_exhausted")
        } else {
            None
        }
    }
    fn replay(
        &mut self,
        runtime: &AgentRuntime,
        simulation: &SimulateRequest,
        trace: Option<&mut crate::macro_diagnostic::Collector>,
        progress: &mut impl FnMut(Value),
    ) -> Result<SimulateResponse, String> {
        if let Some(reason) = self.stop() {
            return Err(reason.into());
        }
        self.used += 1;
        progress(
            json!({"phase":"evaluate","message":"按冻结完整环境回放候选与对照。","simulations":self.used}),
        );
        let c = runtime.context();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::simulate_core_with_trace(
                simulation,
                c.skills,
                c.game_version,
                c.mount,
                c.constants,
                c.recipes,
                c.team_buffs,
                c.formations,
                trace,
            )
        }))
        .map_err(|_| "完整候选回放失败；没有接受该候选。".to_owned())
    }
}

fn observed_timeline(response: &SimulateResponse) -> Vec<Value> {
    response.timeline.iter().enumerate().filter(|(_,e)|super::alignment::is_active(e)).take(super::alignment::MAX_ACTIVE_EVENTS)
        .map(|(index,e)|json!({"index":index,"name":e.name,"skill_id":e.skill_id,"cast_time":e.cast_time,
            "state_before":e.state_before,"damage_total":e.damage_total,"macro_page":e.macro_page,"macro_line":e.macro_line})).collect()
}
fn action_view(response: &SimulateResponse, window: f64) -> Vec<Value> {
    response.timeline.iter().enumerate().filter(|(_,e)|super::alignment::is_active(e)&&super::alignment::within_window(e,window))
        .map(|(index,e)|json!({"index":index,"name":e.name,"skill_id":e.skill_id,"cast_time":e.cast_time})).collect()
}
fn snapshots_complete(response: &SimulateResponse, window: f64) -> bool {
    response
        .timeline
        .iter()
        .filter(|e| super::alignment::is_active(e) && super::alignment::within_window(e, window))
        .all(|e| e.state_before.as_ref().is_some_and(|s| s.time.is_finite()))
}

fn manual_policy_hash(simulation: &SimulateRequest) -> Option<String> {
    if simulation
        .macro_text
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return None;
    }
    canonical_sha256(
        &json!({"scope":"complete_manual_action_policy","sequence":simulation.sequence,
        "channel_ticks":simulation.channel_ticks,"timing_offsets":simulation.timing_offsets,
        "qijin_buffs":simulation.qijin_buffs,"pre_releases":simulation.pre_releases,
        "pauses":simulation.pauses,"macro_duration":simulation.macro_duration}),
    )
    .ok()
}
fn candidate_evidence(
    response: &SimulateResponse,
    simulation: &SimulateRequest,
    state: &Checkpoint,
) -> Value {
    json!({"metrics":metrics(response),"fingerprint":response.fingerprint.to_string(),"simulation":simulation,
        "scenario_hash":artifact_hash(simulation,&state.request),"macro_text":simulation.macro_text,
        "skipped":response.skipped,"replayed":true})
}

#[allow(clippy::too_many_arguments)]
fn evaluate_atomic(
    state: &Checkpoint,
    runtime: &AgentRuntime,
    experiment: &Experiment,
    parent: Option<&Artifact>,
    original: SimulateRequest,
    original_equipment: Option<super::equipment::EquipmentSnapshot>,
    mut simulation: SimulateRequest,
    equipment: Option<super::equipment::EquipmentSnapshot>,
    cancel: &AtomicBool,
    wall_time_ms: u64,
    progress: &mut impl FnMut(Value),
) -> Result<ExperimentOutput, String> {
    let validate = experiment.kind == "validate";
    let changed_environment = validate
        && (original.network_delay != simulation.network_delay
            || original.dunya_reset_seed != simulation.dunya_reset_seed);
    let mut baseline_request = original.clone();
    if validate {
        // The counterfactual belongs to the selected experiment branch. Global
        // initial input can be a different build or sequence after joint search.
        baseline_request = parent
            .and_then(|a| a.result.pointer("/baseline/simulation"))
            .filter(|v| v.is_object())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|_| "父证据的基线场景无效")?
            .unwrap_or_else(|| state.scenario.simulation.clone());
        if parent
            .and_then(|a| a.result.pointer("/baseline/simulation"))
            .is_none()
        {
            if let Some(eq) = &state.request.equipment {
                baseline_request = rebuild_equipment(state, runtime, &baseline_request, eq, true)?;
            }
        }
        baseline_request.network_delay = simulation.network_delay;
        baseline_request.dunya_reset_seed = simulation.dunya_reset_seed;
    }
    baseline_request.lite = false;
    baseline_request.lite_keep_timeline = false;
    if let Some(text) = baseline_request
        .macro_text
        .clone()
        .filter(|m| !m.trim().is_empty())
    {
        let duration = if validate {
            simulation
                .macro_duration
                .unwrap_or(state.request.constraints.duration_seconds)
        } else {
            state.request.constraints.duration_seconds
        };
        baseline_request = macro_request(
            &baseline_request,
            &super::compiler::normalize_game_macro(&text)?,
            duration,
        );
    }
    validate_environment(
        &baseline_request,
        state.request.version,
        state.request.mount,
    )?;
    let constraint_error = rotation_constraints(&simulation, state, runtime).err();
    let mut budget = AtomicBudget {
        cancel,
        started: Instant::now(),
        wall: wall_time_ms,
        maximum: experiment.max_simulations,
        used: 0,
    };
    let baseline = budget.replay(runtime, &baseline_request, None, progress)?;
    let mut result = json!({"task":experiment.kind,"stop_reason":"evaluated","simulations":budget.used,
        "baseline":candidate_evidence(&baseline,&baseline_request,state),"best":null,
        "comparison_scope":if validate {"parent_experiment_baseline"}else if parent.is_some(){"parent_candidate"}else{"original_scene"},
        "comparison_reference_hash":artifact_hash(&baseline_request,&state.request),
        "limitations":["比较保留完整结算时间；DPS仅在实际结算时长相同，或完整手动动作策略相同时可比。同策略下加速改变实际用时，不代表同时间窗口收益。",
            "动作还原仅验证窗口内动作身份、时间和已采集资源，不证明全部气劲、冷却或全局最优。"]});
    let baseline_macro = baseline_request
        .macro_text
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty());
    let window = if baseline_macro {
        baseline_request
            .macro_duration
            .unwrap_or(state.request.constraints.duration_seconds)
    } else {
        baseline.fight_time.max(
            baseline
                .timeline
                .iter()
                .filter(|e| super::alignment::is_active(e))
                .map(|e| e.cast_time + 1.0 / 16.0)
                .fold(0.0, f64::max),
        )
    };
    if !window.is_finite() || window <= 0.0 || window > 1200.0 {
        result["stop_reason"] = json!("evaluation_failed");
        result["error"] = json!("参考场景实际窗口必须大于 0 且不超过 1200 秒。");
        return Ok(ExperimentOutput {
            result,
            simulation: original,
            equipment: original_equipment,
            simulations: budget.used,
        });
    }
    if let Some(reason) = budget.stop() {
        result["stop_reason"] = json!(reason);
        return Ok(ExperimentOutput {
            result,
            simulation: original,
            equipment: original_equipment,
            simulations: budget.used,
        });
    }
    if let Some(text) = simulation
        .macro_text
        .clone()
        .filter(|m| !m.trim().is_empty())
    {
        simulation = macro_request(&simulation, &text, window);
    }
    let mut trace = crate::macro_diagnostic::Collector::new(0.0, window);
    let candidate = match budget.replay(runtime, &simulation, Some(&mut trace), progress) {
        Ok(response) => response,
        Err(error) => {
            result["ok"] = json!(false);
            result["error"] = json!(error);
            result["stop_reason"] = json!(budget.stop().unwrap_or("evaluation_failed"));
            result["simulations"] = json!(budget.used);
            return Ok(ExperimentOutput {
                result,
                simulation: original,
                equipment: original_equipment,
                simulations: budget.used,
            });
        }
    };
    result["simulations"] = json!(budget.used);
    result["timeline"] = json!(observed_timeline(&candidate));
    result["reference_timeline"] = json!(observed_timeline(&baseline));
    result["timeline_truncated"] = json!([&baseline, &candidate].into_iter().any(|r| r
        .timeline
        .iter()
        .filter(|e| super::alignment::is_active(e))
        .count()
        > super::alignment::MAX_ACTIVE_EVENTS));
    result["macro_trace"] = serde_json::to_value(&trace).map_err(|e| e.to_string())?;
    let active = action_view(&candidate, window);
    let reference = action_view(&baseline, window);
    let full_snapshots =
        snapshots_complete(&baseline, window) && snapshots_complete(&candidate, window);
    let pages = simulation
        .macro_text
        .as_deref()
        .map(super::compiler::pages)
        .unwrap_or_default();
    let page_ok = simulation.macro_text.is_none()
        || (!pages.is_empty()
            && pages.len() <= state.request.constraints.max_pages
            && pages.iter().all(|p| p.within_limit));
    let finite = candidate.dps.is_finite()
        && candidate.total_damage.is_finite()
        && candidate.fight_time.is_finite()
        && candidate.fight_time > 0.0;
    let constraints_passed = constraint_error.is_none() && page_ok;
    let candidate_tail = candidate
        .timeline
        .iter()
        .filter(|e| super::alignment::is_active(e) && !super::alignment::within_window(e, window))
        .count();
    let mut best = candidate_evidence(&candidate, &simulation, state);
    best["pages"] = json!(pages);
    best["page_constraints_passed"] = json!(page_ok);
    best["allowed_skills_passed"] = json!(constraint_error.is_none());
    best["constraints_passed"] = json!(constraints_passed);
    best["constraint_error"] = json!(constraint_error);
    best["full_snapshots"] = json!(full_snapshots);
    best["verified"] = json!(
        finite
            && constraints_passed
            && full_snapshots
            && !active.is_empty()
            && candidate.skipped.is_empty()
    );
    best["reference_actions"] = json!(reference);
    best["actual_actions"] = json!(active);
    best["outside_window_actions"] = json!(candidate_tail);
    best["equipment"] = json!(equipment);
    let mut reproduced = false;
    match super::alignment::align(&baseline.timeline, &candidate.timeline, window, 1.0 / 16.0) {
        Ok(alignment) => {
            reproduced = best["verified"] == true
                && !reference.is_empty()
                && baseline.skipped.is_empty()
                && candidate_tail == 0
                && alignment.summary.missing == 0
                && alignment.summary.extra == 0
                && alignment.summary.changed == 0;
            if let Some(index) = alignment.summary.first_difference {
                let row = &alignment.rows[index];
                let event = |e: &crate::CastEvent| {
                    json!({"name":e.name,"skill_id":e.skill_id,"cast_time":e.cast_time,
                    "channel_ticks":e.channel_ticks,"rage_after":e.rage_after,"rage_delta":e.rage_delta,"state_before":e.state_before})
                };
                let actual = row.actual_index.and_then(|i| candidate.timeline.get(i));
                let offset = |response: &SimulateResponse, index: usize| {
                    response
                        .timeline
                        .iter()
                        .take(index)
                        .filter(|e| super::alignment::is_active(e))
                        .count()
                };
                best["first_difference"] = json!({"kind":row.kind,"reference_index":row.reference_index,"actual_index":row.actual_index,
                    "reference_offset":row.reference_index.map(|i|offset(&baseline,i)),"actual_offset":row.actual_index.map(|i|offset(&candidate,i)),
                    "time_delta":row.time_delta,"resource_diffs":row.resource_diffs,
                    "reference":row.reference_index.and_then(|i|baseline.timeline.get(i)).map(event),"actual":actual.map(event)});
                let observed = actual.and_then(|e| {
                    trace.decisions.iter().find(|d| {
                        d.cast_success == Some(true)
                            && d.cast_time.is_some_and(|t| (t - e.cast_time).abs() < 1e-6)
                            && d.selected
                                .as_deref()
                                .is_some_and(|s| s == super::alignment::skill_key(e))
                    })
                });
                best["diagnosis"] = json!({"kind":if observed.is_some(){"observed_decision"}else{"unobserved"},
                    "message":if observed.is_some(){"首分歧实际释放对应的真实宏执行观察；不据附近状态推断因果。"}else{"首分歧没有精确匹配的宏执行观察，请查看双方释放前快照。"},
                    "decision":observed,"trace_truncated":trace.truncated});
            }
            best["alignment"] = json!(alignment);
        }
        Err(error) => {
            best["alignment_error"] = json!(error);
            best["verified"] = json!(false);
        }
    }
    best["reproduced"] = json!(reproduced);
    let same_time_comparable = finite
        && baseline.dps.is_finite()
        && (candidate.fight_time - baseline.fight_time).abs() < 1e-6;
    let reference_policy = manual_policy_hash(&baseline_request);
    let candidate_policy = manual_policy_hash(&simulation);
    let policy_comparable = finite
        && baseline.dps.is_finite()
        && baseline.fight_time.is_finite()
        && baseline.fight_time > 0.0
        && baseline.skipped.is_empty()
        && candidate.skipped.is_empty()
        && reference_policy.is_some()
        && reference_policy == candidate_policy;
    let comparable = same_time_comparable || policy_comparable;
    result["policy_comparable"] = json!(policy_comparable);
    result["same_time_comparable"] = json!(same_time_comparable);
    result["dps_comparison_scope"] = json!(if policy_comparable {
        "same_complete_manual_policy"
    } else if same_time_comparable {
        "same_actual_fight_time"
    } else {
        "not_comparable"
    });
    result["comparison_policy"] =
        json!({"reference_hash":reference_policy,"candidate_hash":candidate_policy});
    result["dps_comparable"] = json!(comparable);
    result["improvement_pct"] = if comparable && baseline.dps > 0.0 {
        json!((candidate.dps / baseline.dps - 1.0) * 100.0)
    } else {
        Value::Null
    };
    if let (Some(a), Some(b)) = (&original_equipment, &equipment) {
        result["slot_diff"] = super::equipment::equipment_diff(runtime, a, b);
    }
    if validate {
        let parent_fingerprint = parent
            .and_then(|a| {
                a.result
                    .pointer("/validation/application_fingerprint")
                    .or_else(|| a.result.pointer("/best/fingerprint"))
            })
            .and_then(Value::as_str);
        result["validation"] = json!({"status":"completed","independent":changed_environment,
            "scope":if changed_environment{"held_out_delay_or_seed"}else{"same_scenario_replay"},
            "network_delay":simulation.network_delay,"seed":simulation.dunya_reset_seed,
            "application_scenario_hash":artifact_hash(&original,&state.request),"application_fingerprint":parent_fingerprint,
            "evaluation_scenario_hash":artifact_hash(&simulation,&state.request),
            "baseline_evaluation_scenario_hash":artifact_hash(&baseline_request,&state.request),
            "fingerprint_matches_parent":if changed_environment {None}else{parent_fingerprint.map(|f|f==candidate.fingerprint.to_string())},
            "dps_comparable":comparable,"same_time_comparable":same_time_comparable,"policy_comparable":policy_comparable,
            "dps_comparison_scope":result["dps_comparison_scope"],
            "improved":changed_environment&&comparable&&best["verified"]==true&&candidate.dps>baseline.dps+1e-8,
            "reproduced":reproduced});
    }
    result["best"] = best;
    result["window_seconds"] = json!(window);
    if cancel.load(Ordering::Relaxed) {
        result["stop_reason"] = json!("cancelled");
    }
    Ok(ExperimentOutput {
        result,
        simulation: if validate { original } else { simulation },
        equipment,
        simulations: budget.used,
    })
}

pub fn run_simulation(runtime: &AgentRuntime, request: &SimulateRequest) -> SimulateResponse {
    let c = runtime.context();
    crate::simulate_core(
        request,
        c.skills,
        c.game_version,
        c.mount,
        c.constants,
        c.recipes,
        c.team_buffs,
        c.formations,
    )
}
pub fn macro_request(base: &SimulateRequest, text: &str, duration: f64) -> SimulateRequest {
    let mut sim = base.clone();
    sim.sequence = vec!["__macro__".into(); (duration / 0.25).ceil() as usize + 20];
    sim.macro_text = Some(text.into());
    sim.macro_duration = Some(duration);
    sim.channel_ticks.clear();
    sim.timing_offsets.clear();
    sim.qijin_buffs.clear();
    sim.lite = false;
    sim.lite_keep_timeline = false;
    sim
}
fn metrics(response: &SimulateResponse) -> Value {
    json!({"dps":response.dps,"total_damage":response.total_damage,"fight_time":response.fight_time,"active_casts":response.timeline.iter().filter(|e|super::alignment::is_active(e)).count(),"skipped_count":response.skipped.len()})
}
pub fn artifact_hash(simulation: &SimulateRequest, request: &RunRequest) -> String {
    ScenarioSnapshotV1::capture(request.version, request.mount, simulation.clone())
        .map(|s| s.scenario_hash)
        .unwrap_or_default()
}
pub fn request_hash(experiment: &Experiment) -> Result<String, String> {
    let mut value = serde_json::to_value(experiment).map_err(|e| e.to_string())?;
    value.as_object_mut().unwrap().remove("hypothesis");
    canonical_sha256(&value).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "../../tests/harness/run_tools.rs"]
mod tests;
