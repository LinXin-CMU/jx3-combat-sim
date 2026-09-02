use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::domain::{AnalysisPlanV1, AnalysisTaskType, EvidencePackV1};
use super::report::{AgentReportContentV1, EvidenceStore, ReportValidationError};

pub const REASONING_STATE_SCHEMA_V1: &str = "agent-reasoning-state/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningCheckpointStatus {
    Complete,
    Ready,
    Pending,
    NotRequired,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningCheckpointV1 {
    pub checkpoint_id: String,
    pub label: String,
    pub question: String,
    pub required_evidence: Vec<String>,
    pub status: ReasoningCheckpointStatus,
    pub evidence_ids: Vec<String>,
    pub decision_rule: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundedHypothesisSearchV1 {
    pub enabled: bool,
    pub max_hypotheses: u8,
    pub max_experiments: u8,
    pub ranking_criteria: Vec<String>,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningStateV1 {
    pub schema_version: String,
    pub playbook_id: String,
    pub objective: String,
    pub checkpoints: Vec<ReasoningCheckpointV1>,
    pub next_checkpoint: String,
    pub hypothesis_search: BoundedHypothesisSearchV1,
    pub publication_checks: Vec<String>,
    pub stopping_condition: String,
    pub public_summary: String,
}

pub fn build_reasoning_state(
    question: &str,
    plan: &AnalysisPlanV1,
    pack: &EvidencePackV1,
    evidence: &EvidenceStore,
) -> ReasoningStateV1 {
    let is_rotation = is_rotation_task(plan.task_type);
    let wants_experiment = plan
        .routing_signals
        .iter()
        .any(|signal| signal == "candidate_comparison_explicitly_requested");
    let scenario_ids = evidence_ids_for_tools(evidence, &["get_current_scenario"]);
    let timeline_ids = evidence_ids_for_tools(evidence, &["analyze_timeline"]);
    let baseline_ids = evidence_ids_for_tools(
        evidence,
        &["simulate_scenario", "analyze_timeline", "compare_scenarios"],
    );
    let knowledge_ids = fact_eligible_knowledge_ids(evidence);
    let comparison_ids = evidence_ids_for_tools(
        evidence,
        &[
            "compare_scenarios",
            "compare_saved_macros",
            "compare_saved_scenarios",
            "compare_focused_equipment",
            "compare_equipment_strategies",
        ],
    );
    let equipment_ids = evidence_ids_for_tools(evidence, &["inspect_equipment_workspace"]);

    let mut checkpoints = if plan.task_type == AnalysisTaskType::PracticalAdaptation {
        practical_adaptation_checkpoints(&scenario_ids, &timeline_ids, &knowledge_ids)
    } else if is_rotation {
        rotation_checkpoints(
            wants_experiment,
            &scenario_ids,
            &baseline_ids,
            &timeline_ids,
            &knowledge_ids,
            &comparison_ids,
        )
    } else if plan.task_type == AnalysisTaskType::EquipmentAnalysis {
        equipment_checkpoints(
            &scenario_ids,
            &equipment_ids,
            &knowledge_ids,
            &comparison_ids,
            plan.playbook
                .required_dimensions
                .iter()
                .any(|dimension| dimension == "candidate_comparison"),
            plan.playbook
                .required_dimensions
                .iter()
                .any(|dimension| dimension == "versioned_knowledge"),
        )
    } else {
        generic_checkpoints(pack, &scenario_ids, &knowledge_ids, &baseline_ids)
    };

    let required_ready = pack.coverage.missing_dimensions.is_empty();
    checkpoints.push(checkpoint(
        "publish",
        "发布前批判检查",
        "结论是否回答了原问题，并把观察、诊断、实验与边界分开？",
        &pack.coverage.required_dimensions,
        if required_ready {
            ReasoningCheckpointStatus::Ready
        } else {
            ReasoningCheckpointStatus::Pending
        },
        &pack.evidence_ids,
        "逐项检查任务完成度、证据归属、因果强度、版本范围、干预必要性与表达清晰度。",
    ));

    let next_checkpoint = checkpoints
        .iter()
        .find(|item| {
            matches!(
                item.status,
                ReasoningCheckpointStatus::Pending | ReasoningCheckpointStatus::Ready
            )
        })
        .map(|item| item.checkpoint_id.clone())
        .unwrap_or_else(|| "publish".to_string());
    let completed = checkpoints
        .iter()
        .filter(|item| item.status == ReasoningCheckpointStatus::Complete)
        .count();
    let public_summary = format!(
        "已完成 {completed}/{} 个证据检查点；当前节点：{}。下一步只允许执行能改变该判断的最小动作。",
        checkpoints.len(),
        checkpoints
            .iter()
            .find(|item| item.checkpoint_id == next_checkpoint)
            .map(|item| item.label.as_str())
            .unwrap_or("发布前检查")
    );

    ReasoningStateV1 {
        schema_version: REASONING_STATE_SCHEMA_V1.to_string(),
        playbook_id: plan.playbook.playbook_id.clone(),
        objective: format!("回答用户问题：{}", compact_question(question)),
        checkpoints,
        next_checkpoint,
        hypothesis_search: BoundedHypothesisSearchV1 {
            enabled: is_rotation && wants_experiment,
            max_hypotheses: if is_rotation && wants_experiment {
                3
            } else {
                0
            },
            max_experiments: if is_rotation && wants_experiment {
                1
            } else {
                0
            },
            ranking_criteria: vec![
                "现有证据强度".to_string(),
                "能否被同场景实验证伪".to_string(),
                "对用户决策的预期影响".to_string(),
            ],
            rules: vec![
                "候选假设必须源自已观测现象，不得直接从攻略复制改法。".to_string(),
                "先写出什么结果会推翻假设，再执行单变量实验。".to_string(),
                "没有明确弱点时停止优化，不为使用工具而制造候选。".to_string(),
            ],
        },
        publication_checks: vec![
            "回答了用户实际问题，而不是只复述指标或工具结果".to_string(),
            "每个方向性或因果判断都有对应证据，相关性未冒充因果".to_string(),
            "观察、诊断、候选实验和最终决策没有混写".to_string(),
            "循环诊断同时说明已验证优点、风险及解释边界".to_string(),
            "改法只在同场景对照支持后发布，且解释伤害构成与时序变化".to_string(),
            "没有偏离版本、心法、输入方式和用户请求范围".to_string(),
        ],
        stopping_condition: "原问题的必需检查点均有证据或明确边界，继续调用工具不会改变决策。"
            .to_string(),
        public_summary,
    }
}

pub fn reasoning_state_model_context(state: &ReasoningStateV1) -> String {
    let json = serde_json::to_string(state).unwrap_or_else(|_| "{}".to_string());
    format!(
        "<reasoning_state server_generated=\"true\" schema=\"{}\">\n{}\n</reasoning_state>",
        REASONING_STATE_SCHEMA_V1, json
    )
}

/// Applies presentation-only constraints that can be resolved without another
/// model turn. This never rewrites claims or evidence: it only removes surplus
/// sections/metrics and interventions the user explicitly declined.
pub fn normalize_reasoning_contract(
    question: &str,
    content: &mut AgentReportContentV1,
) -> usize {
    let mut changes = 0usize;
    if content.findings.len() > 3 {
        changes += content.findings.len() - 3;
        content.findings.truncate(3);
    }
    if content.recommendations.len() > 1 {
        changes += content.recommendations.len() - 1;
        content.recommendations.truncate(1);
    }
    if content.limitations.len() > 3 {
        changes += content.limitations.len() - 3;
        content.limitations.truncate(3);
    }
    for (index, finding) in content.findings.iter_mut().enumerate() {
        let limit = if index == 0 { 2 } else { 1 };
        if finding.metrics.len() > limit {
            changes += finding.metrics.len() - limit;
            finding.metrics.truncate(limit);
        }
    }
    let normalized_question = question.to_lowercase();
    if contains_any(
        &normalized_question,
        &["不要改", "不改宏", "无需改", "不要修改", "不需要改"],
    ) {
        changes += content.recommendations.len() + content.rotation_changes.len();
        content.recommendations.clear();
        content.rotation_changes.clear();
    }
    changes
}

/// A semantic publication gate. This deliberately grades outcomes rather than
/// enforcing an exact tool-call sequence: the planner may take another valid
/// path, but a current rotation diagnosis still has to cite its timeline and a
/// tested edit still has to cite the comparison that tested it.
pub fn audit_reasoning_contract(
    question: &str,
    plan: &AnalysisPlanV1,
    content: &AgentReportContentV1,
    evidence: &EvidenceStore,
) -> Result<(), ReportValidationError> {
    if content.refusal_reason.is_some() {
        return Ok(());
    }
    let total_metrics = content
        .findings
        .iter()
        .map(|finding| finding.metrics.len())
        .sum::<usize>();
    if content.findings.len() > 3
        || content.recommendations.len() > 1
        || content.limitations.len() > 3
        || total_metrics > 4
    {
        return Err(reasoning_error(
            "report_focus_exceeded",
            "Report exceeded the task-focused finding, recommendation, limitation, or metric budget",
        ));
    }
    let cited = cited_evidence_ids(content);
    if is_rotation_task(plan.task_type) {
        let timeline = evidence_ids_for_tools(evidence, &["analyze_timeline"]);
        if !timeline.is_empty() && !timeline.iter().any(|id| cited.contains(id)) {
            return Err(reasoning_error(
                "rotation_timeline_not_used",
                "Rotation report did not use the available timeline diagnosis",
            ));
        }
        let comparisons = evidence_ids_for_tools(evidence, &["compare_scenarios"]);
        if !content.rotation_changes.is_empty() {
            if comparisons.is_empty() {
                return Err(reasoning_error(
                    "untested_rotation_change",
                    "Rotation change was published without a same-scenario comparison",
                ));
            }
            if content.rotation_changes.iter().any(|change| {
                !comparisons
                    .iter()
                    .any(|id| change.evidence_ids.contains(id))
            }) {
                return Err(reasoning_error(
                    "rotation_comparison_not_used",
                    "A rotation change did not cite the comparison that tested it",
                ));
            }
        }
        if !comparisons.is_empty()
            && content.recommendations.iter().any(|recommendation| {
                comparisons
                    .iter()
                    .any(|id| recommendation.evidence_ids.contains(id))
                    && contains_any(
                        &recommendation.rationale.to_lowercase(),
                        &["再决定是否采用", "再跑", "尚需对比", "需要对比"],
                    )
            })
        {
            return Err(reasoning_error(
                "completed_experiment_repeated",
                "The decision asked to repeat a same-scenario experiment that already completed",
            ));
        }
        let normalized_question = question.to_lowercase();
        if contains_any(
            &normalized_question,
            &["不要改", "不改宏", "无需改", "不要修改", "不需要改"],
        ) && (!content.recommendations.is_empty() || !content.rotation_changes.is_empty())
        {
            return Err(reasoning_error(
                "unrequested_rotation_intervention",
                "Rotation report proposed an intervention after the user explicitly declined one",
            ));
        }
        if contains_any(
            &normalized_question,
            &["好在哪里", "差在哪里", "优缺点", "哪里好", "哪里差"],
        ) {
            let titles = content
                .findings
                .iter()
                .map(|finding| finding.title.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if !contains_any(&titles, &["优点", "稳定", "做得好"])
                || !contains_any(&titles, &["风险", "瓶颈", "待验证", "边界", "未发现"])
            {
                return Err(reasoning_error(
                    "rotation_diagnosis_incomplete",
                    "Rotation diagnosis did not separately cover verified strengths and observed risks",
                ));
            }
        }
        if comparisons.is_empty() {
            let assertive_text = content
                .findings
                .iter()
                .map(|finding| format!("{} {}", finding.title, finding.explanation))
                .chain(std::iter::once(content.summary.clone()))
                .collect::<Vec<_>>()
                .join(" ");
            if [
                "怒气溢出",
                "资源浪费",
                "执行层面的浪费",
                "拖低dps",
                "拖低 dps",
            ]
            .iter()
            .any(|term| contains_unnegated_term(&assertive_text, term))
            {
                return Err(reasoning_error(
                    "rotation_causality_overstated",
                    "Rotation report converted an observed signal into untested loss or waste",
                ));
            }
        }
    }
    if plan.task_type == AnalysisTaskType::EquipmentAnalysis {
        let workspace = evidence_ids_for_tools(evidence, &["inspect_equipment_workspace"]);
        if !workspace.is_empty() && !workspace.iter().any(|id| cited.contains(id)) {
            return Err(reasoning_error(
                "equipment_workspace_not_used",
                "Equipment report did not use the inspected workspace",
            ));
        }
        let normalized = question.to_lowercase();
        let asked_qiegao = normalized.contains("切糕");
        if !asked_qiegao
            && content
                .recommendations
                .iter()
                .any(|item| item.title.contains("切糕") || item.rationale.contains("切糕"))
        {
            return Err(reasoning_error(
                "equipment_task_drift",
                "Equipment report introduced an unrequested strategy branch",
            ));
        }
    }
    Ok(())
}

fn rotation_checkpoints(
    wants_experiment: bool,
    scenario: &[String],
    baseline: &[String],
    timeline: &[String],
    knowledge: &[String],
    comparison: &[String],
) -> Vec<ReasoningCheckpointV1> {
    let diagnosis_ready = !timeline.is_empty() && !knowledge.is_empty();
    vec![
        checkpoint(
            "scope",
            "锁定循环口径",
            "本次分析对应哪个版本、心法、输入方式和冻结环境？",
            &["scenario".to_string(), "rotation_input".to_string()],
            completed_if(!scenario.is_empty()),
            scenario,
            "以服务端场景为准；宏与手动序列必须分开解释。",
        ),
        checkpoint(
            "baseline",
            "建立输出画像",
            "当前循环的伤害来源、技能数量、节奏和资源流是什么？",
            &["baseline_metrics".to_string(), "timeline".to_string()],
            completed_if(!baseline.is_empty() && !timeline.is_empty()),
            &merge_ids(baseline, timeline),
            "先描述整体结构，不用单一 DPS 或伤害占比代替诊断。",
        ),
        checkpoint(
            "strengths",
            "确认已验证优点",
            "循环哪些部分已经稳定工作，证据边界是什么？",
            &["timeline".to_string()],
            completed_if(!timeline.is_empty()),
            timeline,
            "至少检查节奏、冷却等待、跳过输入和资源样本；没有证据的维度不评价。",
        ),
        checkpoint(
            "risks",
            "定位可观察风险",
            "现象层面哪里可能限制循环，哪些只是待验证信号？",
            &["timeline".to_string()],
            completed_if(!timeline.is_empty()),
            timeline,
            "触顶、占比或相关性不是损失量和因果结论。",
        ),
        checkpoint(
            "knowledge",
            "对齐当前攻略约束",
            "当前版本的理想循环约束如何解释已观测现象？",
            &["versioned_knowledge".to_string()],
            completed_if(!knowledge.is_empty()),
            knowledge,
            "攻略用于解释和提出假设，不能冒充本场景实测。",
        ),
        checkpoint(
            "hypothesis",
            "形成可反驳假设",
            "哪一个因果解释最值得测试，什么结果会推翻它？",
            &["timeline".to_string(), "versioned_knowledge".to_string()],
            if !comparison.is_empty() {
                ReasoningCheckpointStatus::Complete
            } else if diagnosis_ready {
                ReasoningCheckpointStatus::Ready
            } else {
                ReasoningCheckpointStatus::Pending
            },
            &merge_ids(timeline, knowledge),
            "最多保留三个候选，只测试证据最强、可证伪且影响最大的一个。",
        ),
        checkpoint(
            "experiment",
            "执行单变量实验",
            "候选是否改善目标指标，并改变了哪些技能、资源或时序？",
            &["candidate_comparison".to_string()],
            if !wants_experiment {
                ReasoningCheckpointStatus::NotRequired
            } else if !comparison.is_empty() {
                ReasoningCheckpointStatus::Complete
            } else if diagnosis_ready {
                ReasoningCheckpointStatus::Ready
            } else {
                ReasoningCheckpointStatus::Pending
            },
            comparison,
            "固定环境并只改变一个声明变量；同时解释 DPS、伤害构成和时序差异。",
        ),
    ]
}

fn practical_adaptation_checkpoints(
    scenario: &[String],
    timeline: &[String],
    knowledge: &[String],
) -> Vec<ReasoningCheckpointV1> {
    vec![
        checkpoint(
            "scope",
            "锁定实战口径",
            "当前版本、心法、宏或手动输入以及冻结环境是什么？",
            &["scenario".to_string(), "rotation_input".to_string()],
            completed_if(!scenario.is_empty()),
            scenario,
            "以冻结场景与服务端解析出的输入方式为准。",
        ),
        checkpoint(
            "runtime",
            "还原停手恢复语义",
            "分体态宏停手后从哪一页继续，盾飞和盾回如何改变体态？",
            &["rotation_input".to_string()],
            completed_if(!scenario.is_empty()),
            scenario,
            "停手本身不重置体态；按恢复时的实际体态选择宏页。",
        ),
        checkpoint(
            "timeline",
            "读取木桩执行事实",
            "当前冻结循环实际记录了哪些节奏、等待和输入跳过？",
            &["timeline".to_string(), "rotation_diagnosis".to_string()],
            completed_if(!timeline.is_empty()),
            timeline,
            "木桩时间轴只证明当前冻结条件，不能冒充移动或转火实验。",
        ),
        checkpoint(
            "knowledge",
            "对齐当前版本资料",
            "攻略如何解释距离、目标状态、盾飞回返和延迟调节？",
            &["versioned_knowledge".to_string()],
            completed_if(!knowledge.is_empty()),
            knowledge,
            "攻略是机制与玩家实践证据，不能冒充本场景实测。",
        ),
        checkpoint(
            "adaptation",
            "逐项回答实战适配",
            "移动、转火、停手、延迟四项分别能确认什么，边界是什么？",
            &[
                "rotation_input".to_string(),
                "timeline".to_string(),
                "versioned_knowledge".to_string(),
            ],
            completed_if(!scenario.is_empty() && !timeline.is_empty() && !knowledge.is_empty()),
            &merge_ids(&merge_ids(scenario, timeline), knowledge),
            "四项分开回答；明确标记运行规则、模拟观测、攻略建议和未模拟条件。",
        ),
    ]
}

fn equipment_checkpoints(
    scenario: &[String],
    equipment: &[String],
    knowledge: &[String],
    comparison: &[String],
    comparison_required: bool,
    knowledge_required: bool,
) -> Vec<ReasoningCheckpointV1> {
    vec![
        checkpoint(
            "scope",
            "锁定配装口径",
            "当前版本、心法、循环和装备工作区是什么？",
            &["scenario".to_string(), "equipment_context".to_string()],
            completed_if(!scenario.is_empty() && !equipment.is_empty()),
            &merge_ids(scenario, equipment),
            "不得用装备名、装分或黑话替代实际工作区。",
        ),
        checkpoint(
            "build",
            "建立当前配装画像",
            "当前面板、套装件数与特效结构实际是什么？",
            &["equipment_context".to_string()],
            completed_if(!equipment.is_empty()),
            equipment,
            "区分装备件数、套装效果和策略名称，避免把五件装备说成五件套效果。",
        ),
        checkpoint(
            "knowledge",
            "解释版本机制",
            "当前版本资料是否提供了属性阈值、套装或特效取舍依据？",
            &["versioned_knowledge".to_string()],
            if !knowledge_required {
                ReasoningCheckpointStatus::NotRequired
            } else if knowledge.is_empty() {
                ReasoningCheckpointStatus::Pending
            } else {
                ReasoningCheckpointStatus::Complete
            },
            knowledge,
            "没有阈值证据时，不得声称属性偏低、溢出或已经达标。",
        ),
        checkpoint(
            "experiment",
            "执行同循环换装实验",
            "候选相对当前方案改变了哪些面板、DPS 和伤害构成？",
            &["candidate_comparison".to_string()],
            if !comparison_required {
                ReasoningCheckpointStatus::NotRequired
            } else if comparison.is_empty() {
                ReasoningCheckpointStatus::Pending
            } else {
                ReasoningCheckpointStatus::Complete
            },
            comparison,
            "候选缺失时明确索取部位和装备，不虚构完整方案。",
        ),
    ]
}

fn generic_checkpoints(
    pack: &EvidencePackV1,
    scenario: &[String],
    knowledge: &[String],
    baseline: &[String],
) -> Vec<ReasoningCheckpointV1> {
    vec![
        checkpoint(
            "scope",
            "锁定问题口径",
            "问题对象、版本、心法和用户决策是什么？",
            &["scope".to_string()],
            completed_if(!scenario.is_empty()),
            scenario,
            "优先回答用户真正要做的决策。",
        ),
        checkpoint(
            "evidence",
            "取得最小充分证据",
            "哪些资料或模拟结果能够直接改变答案？",
            &pack.coverage.required_dimensions,
            if pack.coverage.missing_dimensions.is_empty() {
                ReasoningCheckpointStatus::Complete
            } else {
                ReasoningCheckpointStatus::Pending
            },
            &merge_ids(knowledge, baseline),
            "只补缺失维度，已有证据不足时保留边界而不是反复检索。",
        ),
    ]
}

fn checkpoint(
    checkpoint_id: &str,
    label: &str,
    question: &str,
    required_evidence: &[String],
    status: ReasoningCheckpointStatus,
    evidence_ids: &[String],
    decision_rule: &str,
) -> ReasoningCheckpointV1 {
    ReasoningCheckpointV1 {
        checkpoint_id: checkpoint_id.to_string(),
        label: label.to_string(),
        question: question.to_string(),
        required_evidence: required_evidence.to_vec(),
        status,
        evidence_ids: evidence_ids.to_vec(),
        decision_rule: decision_rule.to_string(),
    }
}

fn completed_if(value: bool) -> ReasoningCheckpointStatus {
    if value {
        ReasoningCheckpointStatus::Complete
    } else {
        ReasoningCheckpointStatus::Pending
    }
}

fn is_rotation_task(task: AnalysisTaskType) -> bool {
    matches!(
        task,
        AnalysisTaskType::BaselineAnalysis
            | AnalysisTaskType::RotationStallDiagnosis
            | AnalysisTaskType::PracticalAdaptation
            | AnalysisTaskType::MacroAnalysis
            | AnalysisTaskType::HasteDecision
            | AnalysisTaskType::OrangeWeaponTiming
    )
}

fn evidence_ids_for_tools(evidence: &EvidenceStore, tools: &[&str]) -> Vec<String> {
    evidence
        .iter()
        .filter(|(_, item)| {
            item.get("tool_name")
                .and_then(Value::as_str)
                .is_some_and(|name| tools.contains(&name))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn fact_eligible_knowledge_ids(evidence: &EvidenceStore) -> Vec<String> {
    evidence
        .iter()
        .filter(|(_, item)| {
            item.get("tool_name").and_then(Value::as_str) == Some("search_knowledge_base")
                && item
                    .pointer("/result/results")
                    .and_then(Value::as_array)
                    .is_some_and(|results| {
                        results.iter().any(|result| {
                            result.get("fact_eligible").and_then(Value::as_bool) == Some(true)
                        })
                    })
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn cited_evidence_ids(content: &AgentReportContentV1) -> Vec<String> {
    let mut ids = content
        .findings
        .iter()
        .flat_map(|item| item.evidence_ids.iter().cloned())
        .chain(
            content
                .recommendations
                .iter()
                .flat_map(|item| item.evidence_ids.iter().cloned()),
        )
        .chain(
            content
                .rotation_changes
                .iter()
                .flat_map(|item| item.evidence_ids.iter().cloned()),
        )
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn merge_ids(left: &[String], right: &[String]) -> Vec<String> {
    let mut result = left.to_vec();
    result.extend_from_slice(right);
    result.sort();
    result.dedup();
    result
}

fn compact_question(question: &str) -> String {
    let compact = question.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut value = compact.chars().take(120).collect::<String>();
    if compact.chars().count() > 120 {
        value.push('…');
    }
    value
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

fn contains_unnegated_term(value: &str, term: &str) -> bool {
    value.match_indices(term).any(|(index, _)| {
        let context = value[..index]
            .chars()
            .rev()
            .take(12)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        !contains_any(
            &context,
            &["不", "未", "无法", "不能", "并非", "没有", "尚未", "不足以"],
        )
    })
}

fn reasoning_error(code: &'static str, message: &'static str) -> ReportValidationError {
    ReportValidationError { code, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{select_analysis_plan, ScenarioSnapshotV1};
    use crate::{Attributes, GameVersion, Mount, SimulateRequest, TargetConfig};
    use std::collections::HashMap;

    fn scenario() -> ScenarioSnapshotV1 {
        ScenarioSnapshotV1::capture(
            GameVersion::AnYingQianJi,
            Mount::FenShanJin,
            SimulateRequest {
                haste_level: 0,
                sequence: vec!["盾击".to_string(); 8],
                talents: Vec::new(),
                channel_ticks: HashMap::new(),
                timing_offsets: HashMap::new(),
                network_delay: 0,
                recipes: Vec::new(),
                qijin_buffs: HashMap::new(),
                macro_text: None,
                macro_duration: None,
                attributes: Some(Attributes::default()),
                target: Some(TargetConfig {
                    level: 133,
                    defense_bonus: 0.0,
                    damage_cof: 0.0,
                }),
                initial_rage: None,
                pauses: Vec::new(),
                boss_attack_interval: None,
                hanjia_expectation: None,
                tiegu_mode: 2,
                experimental: false,
                lite: false,
                lite_keep_timeline: false,
                equipment: HashMap::new(),
                team_buffs: Vec::new(),
                formation: None,
                pre_releases: Vec::new(),
            },
        )
        .unwrap()
    }

    fn evidence(tool: &str, result: Value) -> Value {
        serde_json::json!({"tool_name": tool, "result": result})
    }

    #[test]
    fn optimization_rotation_exposes_hypothesis_before_experiment() {
        let scenario = scenario();
        let plan = select_analysis_plan("分析并优化当前循环", &scenario);
        let mut store = EvidenceStore::new();
        store.insert(
            "scenario".to_string(),
            evidence("get_current_scenario", serde_json::json!({})),
        );
        store.insert(
            "timeline".to_string(),
            evidence(
                "analyze_timeline",
                serde_json::json!({"diagnostic_profile": {}}),
            ),
        );
        store.insert(
            "knowledge".to_string(),
            evidence(
                "search_knowledge_base",
                serde_json::json!({"results": [{"fact_eligible": true}]}),
            ),
        );
        let pack = super::super::domain::build_evidence_pack(&plan, &store);
        let state = build_reasoning_state("分析并优化当前循环", &plan, &pack, &store);
        assert_eq!(state.next_checkpoint, "hypothesis");
        assert!(state.hypothesis_search.enabled);
        assert_eq!(state.hypothesis_search.max_experiments, 1);
        assert_eq!(
            state
                .checkpoints
                .iter()
                .find(|item| item.checkpoint_id == "experiment")
                .unwrap()
                .status,
            ReasoningCheckpointStatus::Ready
        );
    }

    #[test]
    fn semantic_audit_rejects_untested_rotation_edit() {
        let scenario = scenario();
        let plan = select_analysis_plan("优化当前循环", &scenario);
        let mut store = EvidenceStore::new();
        store.insert(
            "timeline".to_string(),
            evidence(
                "analyze_timeline",
                serde_json::json!({"diagnostic_profile": {}}),
            ),
        );
        let content = AgentReportContentV1 {
            schema_version: "agent-report-content/v1".to_string(),
            summary: "存在候选改法。".to_string(),
            findings: vec![super::super::report::AgentFindingV1 {
                title: "时间轴".to_string(),
                explanation: "已读取。".to_string(),
                evidence_ids: vec!["timeline".to_string()],
                metrics: vec![],
            }],
            recommendations: vec![],
            rotation_changes: vec![super::super::report::RotationChangeV1 {
                change_type: "macro".to_string(),
                edit_operation: "replace".to_string(),
                target: "第一行".to_string(),
                current: "旧".to_string(),
                proposed: "新".to_string(),
                rationale: "测试".to_string(),
                evidence_ids: vec!["timeline".to_string()],
            }],
            limitations: vec![],
            refusal_reason: None,
        };
        assert_eq!(
            audit_reasoning_contract("优化当前循环", &plan, &content, &store)
                .unwrap_err()
                .code,
            "untested_rotation_change"
        );
    }

    #[test]
    fn semantic_audit_respects_explicit_diagnosis_only_request() {
        let scenario = scenario();
        let plan = select_analysis_plan("分析当前循环好在哪里、差在哪里，这次不要改宏", &scenario);
        let mut store = EvidenceStore::new();
        store.insert(
            "timeline".to_string(),
            evidence(
                "analyze_timeline",
                serde_json::json!({"diagnostic_profile": {}}),
            ),
        );
        let content = AgentReportContentV1 {
            schema_version: "agent-report-content/v1".to_string(),
            summary: "循环已有清晰画像。".to_string(),
            findings: vec![
                super::super::report::AgentFindingV1 {
                    title: "已验证优点".to_string(),
                    explanation: "时间轴连续。".to_string(),
                    evidence_ids: vec!["timeline".to_string()],
                    metrics: vec![],
                },
                super::super::report::AgentFindingV1 {
                    title: "可观察风险".to_string(),
                    explanation: "仍需进一步实验。".to_string(),
                    evidence_ids: vec!["timeline".to_string()],
                    metrics: vec![],
                },
            ],
            recommendations: vec![super::super::report::AgentRecommendationV1 {
                title: "降低阈值".to_string(),
                rationale: "尝试修改宏。".to_string(),
                evidence_ids: vec!["timeline".to_string()],
            }],
            rotation_changes: vec![],
            limitations: vec![],
            refusal_reason: None,
        };
        assert_eq!(
            audit_reasoning_contract(
                "分析当前循环好在哪里、差在哪里，这次不要改宏",
                &plan,
                &content,
                &store,
            )
            .unwrap_err()
            .code,
            "unrequested_rotation_intervention"
        );
    }

    #[test]
    fn presentation_normalization_focuses_metrics_and_removes_declined_edits() {
        let mut content = AgentReportContentV1 {
            schema_version: "agent-report-content/v1".to_string(),
            summary: "循环画像。".to_string(),
            findings: (0..4)
                .map(|index| super::super::report::AgentFindingV1 {
                    title: format!("结论{index}"),
                    explanation: "说明。".to_string(),
                    evidence_ids: vec![],
                    metrics: (0..4)
                        .map(|metric| super::super::report::GroundedMetricV1 {
                            label: format!("指标{metric}"),
                            value: metric as f64,
                            unit: "count".to_string(),
                            evidence_id: "evidence".to_string(),
                            json_pointer: "/result/value".to_string(),
                        })
                        .collect(),
                })
                .collect(),
            recommendations: vec![super::super::report::AgentRecommendationV1 {
                title: "改宏".to_string(),
                rationale: "调整阈值。".to_string(),
                evidence_ids: vec![],
            }],
            rotation_changes: vec![super::super::report::RotationChangeV1 {
                change_type: "macro".to_string(),
                edit_operation: "replace".to_string(),
                target: "第一行".to_string(),
                current: "旧".to_string(),
                proposed: "新".to_string(),
                rationale: "测试".to_string(),
                evidence_ids: vec![],
            }],
            limitations: vec!["一".to_string(), "二".to_string(), "三".to_string(), "四".to_string()],
            refusal_reason: None,
        };
        assert!(normalize_reasoning_contract("这次不要改宏", &mut content) > 0);
        assert_eq!(content.findings.len(), 3);
        assert_eq!(content.findings.iter().map(|item| item.metrics.len()).sum::<usize>(), 4);
        assert!(content.recommendations.is_empty());
        assert!(content.rotation_changes.is_empty());
        assert_eq!(content.limitations.len(), 3);
    }

    #[test]
    fn semantic_audit_rejects_unverified_resource_waste_claim() {
        let scenario = scenario();
        let plan = select_analysis_plan("分析当前循环", &scenario);
        let mut store = EvidenceStore::new();
        store.insert(
            "timeline".to_string(),
            evidence(
                "analyze_timeline",
                serde_json::json!({"diagnostic_profile": {}}),
            ),
        );
        let content = AgentReportContentV1 {
            schema_version: "agent-report-content/v1".to_string(),
            summary: "当前存在怒气溢出和资源浪费。".to_string(),
            findings: vec![super::super::report::AgentFindingV1 {
                title: "资源风险".to_string(),
                explanation: "怒气触顶直接拖低 DPS。".to_string(),
                evidence_ids: vec!["timeline".to_string()],
                metrics: vec![],
            }],
            recommendations: vec![],
            rotation_changes: vec![],
            limitations: vec![],
            refusal_reason: None,
        };
        assert_eq!(
            audit_reasoning_contract("分析当前循环", &plan, &content, &store)
                .unwrap_err()
                .code,
            "rotation_causality_overstated"
        );
    }

    #[test]
    fn semantic_audit_distinguishes_negated_loss_language() {
        assert!(contains_unnegated_term("当前存在资源浪费", "资源浪费"));
        assert!(!contains_unnegated_term(
            "怒气触顶不能等同于资源浪费",
            "资源浪费"
        ));
        assert!(!contains_unnegated_term(
            "现有证据不足以证明怒气溢出",
            "怒气溢出"
        ));
    }

    #[test]
    fn semantic_audit_rejects_repeating_a_completed_comparison() {
        let scenario = scenario();
        let plan = select_analysis_plan("分析并优化当前循环", &scenario);
        let mut store = EvidenceStore::new();
        store.insert(
            "timeline".to_string(),
            evidence(
                "analyze_timeline",
                serde_json::json!({"diagnostic_profile": {}}),
            ),
        );
        store.insert(
            "comparison".to_string(),
            evidence(
                "compare_scenarios",
                serde_json::json!({"candidates": [{"delta_dps": -1.0}]}),
            ),
        );
        let content = AgentReportContentV1 {
            schema_version: "agent-report-content/v1".to_string(),
            summary: "候选已完成对比。".to_string(),
            findings: vec![super::super::report::AgentFindingV1 {
                title: "时间轴风险".to_string(),
                explanation: "已读取。".to_string(),
                evidence_ids: vec!["timeline".to_string()],
                metrics: vec![],
            }],
            recommendations: vec![super::super::report::AgentRecommendationV1 {
                title: "保留原方案".to_string(),
                rationale: "再跑同场景对比，再决定是否采用。".to_string(),
                evidence_ids: vec!["comparison".to_string()],
            }],
            rotation_changes: vec![],
            limitations: vec![],
            refusal_reason: None,
        };
        assert_eq!(
            audit_reasoning_contract("分析并优化当前循环", &plan, &content, &store)
                .unwrap_err()
                .code,
            "completed_experiment_repeated"
        );
    }
}
