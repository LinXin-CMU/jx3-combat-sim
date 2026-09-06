//! 序列 → 宏 转换器（P0 MVP + 质量改进）
//!
//! 改进点（相对初版）：
//! 1. 负样本清洗：只把"当时 S 本身可释放但玩家选别的技能"的状态当 S 的负样本
//! 2. 候选扩充：skill_notin_cd / skill_energy / last_skill / bufftime>N / tbufftime>N
//! 3. OR 多子句：贪心抽完一条 AND 后，把剩余未覆盖的正样本再抽一条，最终用 OR 合并
//! 4. 数值阈值搜索：基于正/负样本实际分布的分位点选阈值
//! 5. F1 评分代替 recall - α·fp
//! 6. 主/非主 GCD 分桶：学 S 规则时只用同 is_main 段的负样本
//! 7. 连招后续段合并到主技能名（月照/雁门→阵云结晦，隐刀→闪刀，惊沙→盾毅）

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::macro_engine::{CmpOp, MacroCondition};
use crate::Stance;

// ─────────────────────────────────────────────────────────────────────────────
// 输入
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct InputBuff {
    pub name: String,
    #[serde(default)]
    pub remaining: f64,
    #[serde(default)]
    pub stacks: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InputSkillCd {
    pub name: String,
    #[serde(default)]
    pub remaining: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InputState {
    #[serde(default)]
    pub rage: i32,
    #[serde(default)]
    pub stance: Stance,
    #[serde(default)]
    pub buffs: Vec<InputBuff>,
    #[serde(default)]
    pub target_buffs: Vec<InputBuff>,
    #[serde(default)]
    pub skill_cds: Vec<InputSkillCd>,
}

#[derive(Debug, Deserialize)]
pub struct InputCast {
    pub name: String,
    #[serde(default)]
    pub triggered: bool,
    #[serde(default = "default_is_main")]
    pub is_main: bool,
    #[serde(default)]
    pub state_before: Option<InputState>,
}
fn default_is_main() -> bool {
    true
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GenOptions {
    #[serde(default = "default_page_chars")]
    pub max_chars_per_page: usize,
    #[serde(default = "default_max_terms")]
    pub max_terms: usize,
    #[serde(default = "default_max_clauses")]
    pub max_clauses: usize,
    #[serde(default = "default_min_gain")]
    pub min_gain: f64,
    #[serde(default = "default_min_cover")]
    pub min_cover_pct: f64,

    /// 一致性敏感技能清单：默认 ["血怒", "业火麟光"]
    /// 这些技能走"严格模式"：门槛提高、不允许部分覆盖降级
    #[serde(default = "default_consistency_skills")]
    pub consistency_sensitive_skills: Vec<String>,

    /// 一致性严格模式下 min_gain 的倍率（默认 3.0）
    #[serde(default = "default_consistency_gain_mult")]
    pub consistency_min_gain_mult: f64,

    /// 一致性严格模式下 max_terms 的增量（默认 +2）
    #[serde(default = "default_consistency_terms_bonus")]
    pub consistency_max_terms_bonus: usize,

    /// 一致性敏感技能在后续 cast-diff 迭代中的优先级倍率（默认 5.0）
    /// 当前版本尚未接入 cast-diff 迭代，暂作为 API 占位字段
    #[serde(default = "default_consistency_priority")]
    pub consistency_cast_diff_priority: f64,

    /// 是否启用 Copeland 优先级排序（默认 true）
    #[serde(default = "default_true")]
    pub use_copeland_priority: bool,

    /// 是否对 off-GCD 技能走特殊负样本池（默认 true）
    #[serde(default = "default_true")]
    pub use_off_gcd_treatment: bool,

    /// v2 算法开关：启用开场裁剪 + 精细阈值搜索
    #[serde(default)]
    pub use_v2: bool,
}
fn default_page_chars() -> usize {
    128
}
fn default_max_terms() -> usize {
    2
}
fn default_max_clauses() -> usize {
    3
}
fn default_min_gain() -> f64 {
    0.05
}
fn default_min_cover() -> f64 {
    1.0
}
fn default_consistency_skills() -> Vec<String> {
    vec!["血怒".into(), "业火麟光".into()]
}
fn default_consistency_gain_mult() -> f64 {
    3.0
}
fn default_consistency_terms_bonus() -> usize {
    2
}
fn default_consistency_priority() -> f64 {
    5.0
}
fn default_true() -> bool {
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// 输出
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PageSummary {
    pub stance: String,
    pub chars: usize,
    pub limit: usize,
    pub rule_count: usize,
}

#[derive(Debug, Serialize)]
pub struct DroppedRule {
    pub rule: String,
    pub reason: String,
    pub coverage_pct: f64,
}

#[derive(Debug, Serialize, Default)]
pub struct GenStats {
    pub total_main_casts: usize,
    pub used_casts: usize,
    pub skill_count: usize,
    /// v2: 开场裁剪掉的样本数（0 = 未启用或无裁剪）
    #[serde(skip_serializing_if = "is_zero")]
    pub opener_trimmed: usize,
    /// v2-2.8: 多起点择优的尝试次数（0 = 未启用）
    #[serde(skip_serializing_if = "is_zero")]
    pub multi_start_trials: usize,
    /// v2-2.9: 跨页出现的同技能数量
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cross_page_skills: Vec<String>,
}
fn is_zero(v: &usize) -> bool {
    *v == 0
}

#[derive(Debug, Serialize)]
pub struct RuleDiagnostic {
    pub stance: String,
    pub skill: String,
    pub condition: String, // 渲染后的条件文本，"(无)" 表示无条件
    pub positives: usize,  // 该技能的正样本数
    pub negatives: usize,  // 当时的负样本数
    pub f1: f64,           // 最终条件在正负样本上的 F1
    pub copeland: i32,     // Copeland 优先级分
    pub is_main: bool,
    pub off_gcd: bool,
    pub stance_switcher: bool,
    pub consistency_sensitive: bool,
    pub candidates_top: Vec<CandidateInfo>, // top-N 候选条件（含未选中）
    pub notes: Vec<String>,                 // 例如 "OR 覆盖率<70% 降级为无条件" 等
}

#[derive(Debug, Serialize)]
pub struct CandidateInfo {
    pub expr: String,
    pub f1: f64,
    pub selected: bool,
}

#[derive(Debug, Serialize)]
pub struct GenResult {
    pub macro_text: String,
    pub pages: Vec<PageSummary>,
    pub dropped_rules: Vec<DroppedRule>,
    pub stats: GenStats,
    #[serde(default)]
    pub rule_diagnostics: Vec<RuleDiagnostic>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 内部样本
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Sample<'a> {
    state: &'a InputState,
    skill_base: String,
    is_main: bool,
    last_main_base: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 主入口
// ─────────────────────────────────────────────────────────────────────────────

pub fn generate(timeline: &[InputCast], opts: &GenOptions) -> GenResult {
    // 1. 预处理：过滤有效 cast，计算每个事件释放前的 last_main_base
    let mut last_main: Option<String> = None;
    let mut samples: Vec<Sample> = Vec::new();
    for e in timeline {
        if e.triggered {
            continue;
        }
        let Some(st) = e.state_before.as_ref() else {
            continue;
        };
        let base = base_skill_name(&e.name).to_string();
        samples.push(Sample {
            state: st,
            skill_base: base.clone(),
            is_main: e.is_main,
            last_main_base: last_main.clone(),
        });
        // last_main 在本次 cast 之后更新（供下一次）
        if e.is_main {
            last_main = Some(base);
        }
    }

    // v2: 开场裁剪 — 跳过稳态前的开场序列，避免开场堆怒气的数据污染阈值
    let opener_trimmed = if opts.use_v2 {
        let steady_start = detect_steady_start(&samples);
        if steady_start > 0 {
            samples.drain(..steady_start);
            steady_start
        } else {
            0
        }
    } else {
        0
    };

    let total_main_casts = samples.iter().filter(|s| s.is_main).count();

    // 2. 按姿态分页
    let mut by_stance: BTreeMap<u8, Vec<&Sample>> = BTreeMap::new();
    for s in &samples {
        by_stance
            .entry(stance_key(s.state.stance))
            .or_default()
            .push(s);
    }

    let mut pages_text: Vec<(Stance, String, usize)> = Vec::new();
    let mut dropped: Vec<DroppedRule> = Vec::new();
    let mut all_skills: BTreeSet<String> = BTreeSet::new();
    let mut all_diagnostics: Vec<RuleDiagnostic> = Vec::new();

    if opts.use_v2 {
        // v2-2.8: 多起点择优 — 跑 N 次生成（扰动 min_gain），选总 F1 最高的
        let n_starts = 5;
        for (sk, casts) in &by_stance {
            let stance = unkey_stance(*sk);
            let mut best_result: Option<(
                String,
                usize,
                Vec<DroppedRule>,
                Vec<String>,
                Vec<RuleDiagnostic>,
                f64,
            )> = None;
            for trial in 0..n_starts {
                let mut trial_opts = opts.clone();
                // 扰动 min_gain: trial 0 用原值，其余 ×0.5~1.5
                if trial > 0 {
                    let mult = 0.5 + (trial as f64) * 0.25;
                    trial_opts.min_gain *= mult;
                }
                let (text, rc, dr, sk_list, diags) =
                    build_page(&casts, &trial_opts, stance_label(stance));
                let total_f1: f64 = diags.iter().map(|d| d.f1).sum();
                let chars: usize = text.chars().count();
                let score = if chars <= trial_opts.max_chars_per_page {
                    total_f1
                } else {
                    total_f1 * 0.5 // 超字数惩罚
                };
                if best_result
                    .as_ref()
                    .map_or(true, |(_, _, _, _, _, bs)| score > *bs)
                {
                    best_result = Some((text, rc, dr, sk_list, diags, score));
                }
            }
            if let Some((text, rc, dr, sk_list, diags, _)) = best_result {
                all_skills.extend(sk_list);
                dropped.extend(dr);
                all_diagnostics.extend(diags);
                pages_text.push((stance, text, rc));
            }
        }
    } else {
        for (sk, casts) in &by_stance {
            let stance = unkey_stance(*sk);
            let (text, rule_count, local_dropped, skills, diags) =
                build_page(&casts, opts, stance_label(stance));
            all_skills.extend(skills);
            dropped.extend(local_dropped);
            all_diagnostics.extend(diags);
            pages_text.push((stance, text, rule_count));
        }
    }

    // 3. 拼最终文本
    let mut out = String::new();
    let mut summaries: Vec<PageSummary> = Vec::new();
    for (stance, body, rule_count) in &pages_text {
        let header = match stance {
            Stance::Shield => "#page shield\n",
            Stance::Blade => "#page blade\n",
            Stance::Wall => "#page wall\n",
            _ => "#page\n",
        };
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(header);
        out.push_str(body);
        let chars = body.chars().count();
        summaries.push(PageSummary {
            stance: stance_label(*stance).to_string(),
            chars,
            limit: opts.max_chars_per_page,
            rule_count: *rule_count,
        });
    }

    // v2-2.9: 跨页一致性检测 — 找出在多个姿态页出现的技能
    let cross_page_skills = if opts.use_v2 {
        let mut skill_pages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for d in &all_diagnostics {
            skill_pages
                .entry(d.skill.clone())
                .or_default()
                .insert(d.stance.clone());
        }
        skill_pages
            .into_iter()
            .filter(|(_, pages)| pages.len() > 1)
            .map(|(skill, _)| skill)
            .collect()
    } else {
        Vec::new()
    };

    let multi_start_trials = if opts.use_v2 { 5 } else { 0 };

    GenResult {
        macro_text: out,
        pages: summaries,
        dropped_rules: dropped,
        stats: GenStats {
            total_main_casts,
            used_casts: samples.len(),
            skill_count: all_skills.len(),
            opener_trimmed,
            multi_start_trials,
            cross_page_skills,
        },
        rule_diagnostics: all_diagnostics,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 单页生成
// ─────────────────────────────────────────────────────────────────────────────

fn build_page<'a>(
    page_samples: &[&Sample<'a>],
    opts: &GenOptions,
    stance_label: &str,
) -> (
    String,
    usize,
    Vec<DroppedRule>,
    Vec<String>,
    Vec<RuleDiagnostic>,
) {
    // 按基础名聚类
    let mut by_base: BTreeMap<String, Vec<&Sample<'a>>> = BTreeMap::new();
    for s in page_samples {
        by_base.entry(s.skill_base.clone()).or_default().push(*s);
    }
    let total_casts = page_samples.len() as f64;

    struct Rule {
        skill: String,
        cond: Option<MacroCondition>,
        #[allow(dead_code)]
        cover: usize,
        n_cond: usize,
        #[allow(dead_code)]
        is_main: bool,
        copeland: i32,
        off_gcd: bool,
    }

    let mut rules: Vec<Rule> = Vec::new();
    let mut dropped = Vec::new();
    let mut skills_seen = Vec::new();
    let mut diagnostics: Vec<RuleDiagnostic> = Vec::new();

    // 按基础名收集每个技能的最小观察到的 rage（近似 rage_cost 下界）
    let mut min_rage: BTreeMap<String, i32> = BTreeMap::new();
    for (skill, positives) in &by_base {
        let mr = positives.iter().map(|s| s.state.rage).min().unwrap_or(0);
        min_rage.insert(skill.clone(), mr);
    }

    // Copeland：对每个样本 i，X = 它的 skill_base；对每个 Y ≠ X，若 Y 在 state_before 下可释放
    // 则 wins[X][Y] += 1（X 战胜 Y 的一次证据）
    let mut wins: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    for s in page_samples {
        if !s.is_main {
            continue;
        } // 非主 GCD 不竞争槽位
        let x = &s.skill_base;
        for (y, _) in &by_base {
            if y == x {
                continue;
            }
            let y_main = by_base[y].iter().filter(|s2| s2.is_main).count() * 2 >= by_base[y].len();
            if !y_main {
                continue;
            }
            let mr = *min_rage.get(y).unwrap_or(&0);
            if !skill_feasible(y, s.state) {
                continue;
            }
            if s.state.rage < mr {
                continue;
            }
            *wins
                .entry(x.clone())
                .or_default()
                .entry(y.clone())
                .or_insert(0) += 1;
        }
    }
    let copeland_score = |skill: &str| -> i32 {
        let mut sc = 0i32;
        for other in by_base.keys() {
            if other == skill {
                continue;
            }
            let a = wins
                .get(skill)
                .and_then(|m| m.get(other))
                .copied()
                .unwrap_or(0) as i32;
            let b = wins
                .get(other)
                .and_then(|m| m.get(skill))
                .copied()
                .unwrap_or(0) as i32;
            sc += a - b;
        }
        sc
    };

    for (skill, positives) in &by_base {
        skills_seen.push(skill.clone());
        let cover_pct = positives.len() as f64 / total_casts * 100.0;
        if cover_pct < opts.min_cover_pct {
            dropped.push(DroppedRule {
                rule: format!("/cast {}", skill),
                reason: "low_coverage".into(),
                coverage_pct: cover_pct,
            });
            continue;
        }

        let skill_is_main = positives.iter().filter(|s| s.is_main).count() * 2 >= positives.len();
        let off_gcd = is_off_gcd(skill);
        let stance_switch = is_stance_switcher(skill);
        let consistency_sensitive = opts.consistency_sensitive_skills.iter().any(|s| s == skill);

        // 负样本池
        //  - 占 GCD 的主技能：同 is_main 桶、不同技能、且当时 S 本身可释放
        //  - off-GCD：本页里**所有**"S 当时可释放但玩家没放"的时刻
        let mut negatives: Vec<&Sample<'a>> = Vec::new();
        if off_gcd && opts.use_off_gcd_treatment {
            for s in page_samples {
                if s.skill_base == *skill {
                    continue;
                }
                if !skill_feasible(skill, s.state) {
                    continue;
                }
                negatives.push(*s);
            }
        } else {
            for (other, ns) in &by_base {
                if other == skill {
                    continue;
                }
                for s in ns {
                    if s.is_main != skill_is_main {
                        continue;
                    }
                    if !skill_feasible(skill, s.state) {
                        continue;
                    }
                    negatives.push(s);
                }
            }
        }

        // 条件抽取门槛：
        //   consistency_sensitive：严格模式（门槛 × mult、terms + bonus、不允许部分覆盖）
        //   stance_switcher：宽松模式（低门槛，允许部分覆盖）
        //   off_gcd：中等宽松
        //   默认：opts 里给的值
        let (min_gain, max_terms, max_clauses, allow_partial) = if consistency_sensitive {
            (
                opts.min_gain * opts.consistency_min_gain_mult,
                opts.max_terms + opts.consistency_max_terms_bonus,
                opts.max_clauses,
                false,
            )
        } else if stance_switch {
            (opts.min_gain * 0.2, 4, 4, true)
        } else if off_gcd {
            (opts.min_gain * 0.5, 3, 3, true)
        } else {
            (opts.min_gain, opts.max_terms, opts.max_clauses, false)
        };
        let cond = if negatives.is_empty() {
            None
        } else {
            extract_rule(
                positives,
                &negatives,
                opts,
                min_gain,
                allow_partial,
                max_terms,
                max_clauses,
            )
        };
        let n_cond = cond.as_ref().map(count_atoms).unwrap_or(0);
        let cp = if opts.use_copeland_priority {
            copeland_score(skill)
        } else {
            0
        };

        // ── 诊断：top-N 候选 + 最终 F1 ──
        let final_f1 = cond
            .as_ref()
            .map(|c| score_f1(c, positives, &negatives))
            .unwrap_or(0.0);
        let cond_text = cond
            .as_ref()
            .map(|c| c.display_string())
            .unwrap_or_else(|| "(无)".to_string());

        let mut selected_atoms: BTreeSet<String> = BTreeSet::new();
        if let Some(c) = cond.as_ref() {
            collect_atom_exprs(c, &mut selected_atoms);
        }

        let mut cand_scored: Vec<(String, f64, bool)> =
            enumerate_candidates(positives, &negatives, opts.use_v2)
                .into_iter()
                .map(|c| {
                    let f1 = score_f1(&c, positives, &negatives);
                    let expr = c.display_string();
                    let sel = selected_atoms.contains(&expr);
                    (expr, f1, sel)
                })
                .collect();
        cand_scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        cand_scored.truncate(12);
        let candidates_top = cand_scored
            .into_iter()
            .map(|(expr, f1, sel)| CandidateInfo {
                expr,
                f1,
                selected: sel,
            })
            .collect();

        let mut notes = Vec::new();
        if negatives.is_empty() {
            notes.push("无负样本 → 无条件规则".into());
        }
        if cond.is_none() && !negatives.is_empty() {
            notes.push("未找到 F1 达标的条件 → 降级为无条件".into());
        }

        diagnostics.push(RuleDiagnostic {
            stance: stance_label.to_string(),
            skill: skill.clone(),
            condition: cond_text,
            positives: positives.len(),
            negatives: negatives.len(),
            f1: final_f1,
            copeland: cp,
            is_main: skill_is_main,
            off_gcd,
            stance_switcher: stance_switch,
            consistency_sensitive,
            candidates_top,
            notes,
        });

        rules.push(Rule {
            skill: skill.clone(),
            cond,
            cover: positives.len(),
            n_cond,
            is_main: skill_is_main,
            copeland: cp,
            off_gcd,
        });
    }

    // 排序：
    //  1) off-GCD 技能（盾飞/盾回/血怒）排在主技能之前，它们不占主 GCD 槽
    //     phase 2 会先跑这些再跑主输出；顺序在 off-GCD 内部不重要
    //  2) 无条件兜底放最后 —— §5.4
    //  3) 按 Copeland 分数降序（战胜多的主技能优先级高）
    //  4) 同分按条件数降序
    rules.sort_by(|a, b| {
        b.off_gcd
            .cmp(&a.off_gcd)
            .then(a.cond.is_none().cmp(&b.cond.is_none()))
            .then(b.copeland.cmp(&a.copeland))
            .then(b.n_cond.cmp(&a.n_cond))
    });

    // 渲染规则为文本行
    let rendered: Vec<String> = rules
        .iter()
        .map(|r| match &r.cond {
            Some(c) => {
                let reordered = crate::macro_prune::reorder_last_skill_first(c);
                format!("/cast {} {}\n", reordered.display_string(), r.skill)
            }
            None => format!("/cast {}\n", r.skill),
        })
        .collect();

    // v2-2.4: 字数感知 — 用贪心背包选最优规则子集（字数预算内 F1×覆盖率 最大化）
    let selected = if opts.use_v2
        && rendered.iter().map(|l| l.chars().count()).sum::<usize>() > opts.max_chars_per_page
    {
        let char_limit = opts.max_chars_per_page;
        // off-GCD 规则必选（盾飞/盾回/血怒，不占主 GCD 槽）
        let mut must: Vec<usize> = Vec::new();
        let mut optional: Vec<usize> = Vec::new();
        for (i, r) in rules.iter().enumerate() {
            if r.off_gcd {
                must.push(i);
            } else {
                optional.push(i);
            }
        }
        let mut used_chars: usize = must.iter().map(|&i| rendered[i].chars().count()).sum();
        let mut picked: Vec<usize> = must;
        // 按 F1 × 覆盖率 / 字数 的性价比排序
        let mut scored: Vec<(usize, f64)> = optional
            .iter()
            .map(|&i| {
                let chars = rendered[i].chars().count().max(1) as f64;
                let diag = diagnostics.iter().find(|d| d.skill == rules[i].skill);
                let f1 = diag.map_or(0.5, |d| d.f1);
                let cover = rules[i].cover as f64 / total_casts.max(1.0);
                (i, f1 * cover / chars)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (idx, _) in scored {
            let chars = rendered[idx].chars().count();
            if used_chars + chars <= char_limit {
                picked.push(idx);
                used_chars += chars;
            }
        }
        picked.sort(); // 保持原顺序
        picked
    } else {
        (0..rules.len()).collect()
    };

    let mut text = String::new();
    let mut count = 0;
    for &i in &selected {
        text.push_str(&rendered[i]);
        count += 1;
    }
    // 被字数预算截掉的规则
    for (i, r) in rules.iter().enumerate() {
        if !selected.contains(&i) {
            let cover_pct = r.cover as f64 / total_casts * 100.0;
            dropped.push(DroppedRule {
                rule: rendered[i].trim().to_string(),
                reason: "char_budget".into(),
                coverage_pct: cover_pct,
            });
        }
    }

    (text, count, dropped, skills_seen, diagnostics)
}

/// 深度遍历条件树，把所有叶子节点（原子）的 display_string 收集起来
fn collect_atom_exprs(cond: &MacroCondition, out: &mut BTreeSet<String>) {
    match cond {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            collect_atom_exprs(a, out);
            collect_atom_exprs(b, out);
        }
        _ => {
            out.insert(cond.display_string());
        }
    }
}

/// 收集 bufftime / tbufftime 类叶子对应的 buff 名字（附是否 target）
fn collect_bufftime_atoms(cond: &MacroCondition, out: &mut BTreeSet<(String, bool)>) {
    match cond {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            collect_bufftime_atoms(a, out);
            collect_bufftime_atoms(b, out);
        }
        MacroCondition::BuffTime(n, _, _) => {
            out.insert((n.clone(), false));
        }
        MacroCondition::TBuffTime(n, _, _) => {
            out.insert((n.clone(), true));
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 多子句 OR 抽取（greedy-subtract）
// ─────────────────────────────────────────────────────────────────────────────

fn extract_rule<'a>(
    positives: &[&Sample<'a>],
    negatives: &[&Sample<'a>],
    opts: &GenOptions,
    min_gain: f64,
    allow_partial: bool,
    max_terms: usize,
    max_clauses: usize,
) -> Option<MacroCondition> {
    let mut clauses: Vec<MacroCondition> = Vec::new();
    let mut remaining_pos: Vec<&Sample<'a>> = positives.to_vec();
    let neg_all = negatives.to_vec();

    // 每个子句至少要命中 ≥ N 个剩余正样本。放宽下限以允许少数关键分支（如
    // "首次施加 debuff"这种占比小但不可丢的 apply 分支）被保留。
    let chunk_ratio = if allow_partial { 0.03 } else { 0.05 };
    let min_chunk = (positives.len() as f64 * chunk_ratio).ceil() as usize;
    let min_chunk = min_chunk.max(2);
    for _ in 0..max_clauses {
        if remaining_pos.len() < min_chunk {
            break;
        }
        let clause = greedy_and(&remaining_pos, &neg_all, opts, min_gain, max_terms);
        let Some(cl) = clause else {
            break;
        };
        let before = remaining_pos.len();
        remaining_pos.retain(|s| !eval_cond(&cl, s));
        let covered = before - remaining_pos.len();
        if covered < min_chunk {
            break;
        } // 这条子句覆盖太少，不要了
        clauses.push(cl);
        if remaining_pos.is_empty() {
            break;
        }
    }
    // 如果 OR 合并后漏掉的正样本占比仍 > 30%，说明单靠这些子句覆盖不全
    // → 降级为无条件兜底（"该技能没稳定触发条件"）
    // 但姿态切换类技能不走此降级 —— 宁可漏放也不要过放
    if !allow_partial {
        let covered_ratio =
            (positives.len() - remaining_pos.len()) as f64 / positives.len().max(1) as f64;
        // 放宽到 50%：剩下的未覆盖正样本可以靠下面的"语义配对补全"救回来
        if covered_ratio < 0.5 && !clauses.is_empty() {
            return None;
        }
    }

    // ─── 语义配对补全 ──────────────────────────────────────────────
    // 对已选出的 clauses 里每个 (t)bufftime:X op N 原子，尝试以 (t)nobuff:X 作为 OR 分支。
    // 逻辑：
    //   - bufftime 类判断在 absent→false 语义下不覆盖 "buff 缺席" 场景
    //   - 如果训练数据里 skill 也在 buff 缺席时释放过（apply 分支），
    //     greedy 可能因为 F1 低没挑中这个配对 → 这里兜底补上
    //   - 只有当该 nobuff 能覆盖若干尚未被 clauses 覆盖的正样本 + 对负样本不显著误伤
    //     才真正追加
    let mut bufftime_buff_names: std::collections::BTreeSet<(String, bool)> =
        std::collections::BTreeSet::new();
    for cl in &clauses {
        collect_bufftime_atoms(cl, &mut bufftime_buff_names);
    }
    for (name, is_target) in bufftime_buff_names {
        let pair_atom = if is_target {
            MacroCondition::TnoBuff(name.clone())
        } else {
            MacroCondition::NoBuff(name.clone())
        };
        // 计算加了配对之后对正负样本的影响
        let covered_by_existing = |s: &&Sample<'_>| clauses.iter().any(|c| eval_cond(c, s));
        let extra_pos = positives
            .iter()
            .filter(|s| !covered_by_existing(s) && eval_cond(&pair_atom, s))
            .count();
        if extra_pos < 2 {
            continue;
        }
        let extra_neg = negatives
            .iter()
            .filter(|s| !covered_by_existing(s) && eval_cond(&pair_atom, s))
            .count();
        // 新增覆盖正样本 / 新增误伤负样本 的比值要足够好
        //（至少正样本新增比负样本新增多一些，否则说明 "absent" 场景对本技能不是真的 apply 时机）
        let neg_ratio = extra_neg as f64 / extra_pos as f64;
        if neg_ratio > 1.5 {
            continue;
        }
        clauses.push(pair_atom);
    }
    if clauses.is_empty() {
        return None;
    }
    let mut it = clauses.into_iter();
    let first = it.next().unwrap();
    let cond = it.fold(first, |acc, c| {
        MacroCondition::Or(Box::new(acc), Box::new(c))
    });
    Some(guard_virtual_weak(cond))
}

/// 虚弱获取有延迟：如果卡 GCD 后半程释放盾飞，紧接着用 tnobuff:虚弱 会误判为真
/// 导致绝刀等直接吃不到虚弱。对每个 tnobuff:虚弱 叶子加 AND last_skill!=盾飞。
fn guard_virtual_weak(cond: MacroCondition) -> MacroCondition {
    match cond {
        MacroCondition::And(a, b) => MacroCondition::And(
            Box::new(guard_virtual_weak(*a)),
            Box::new(guard_virtual_weak(*b)),
        ),
        MacroCondition::Or(a, b) => MacroCondition::Or(
            Box::new(guard_virtual_weak(*a)),
            Box::new(guard_virtual_weak(*b)),
        ),
        MacroCondition::TnoBuff(ref name) if name == "虚弱" => MacroCondition::And(
            Box::new(MacroCondition::TnoBuff(name.clone())),
            Box::new(MacroCondition::LastSkillNot("盾飞".to_string())),
        ),
        other => other,
    }
}

fn greedy_and<'a>(
    positives: &[&Sample<'a>],
    negatives: &[&Sample<'a>],
    opts: &GenOptions,
    min_gain: f64,
    max_terms: usize,
) -> Option<MacroCondition> {
    if positives.is_empty() {
        return None;
    }
    let mut pos = positives.to_vec();
    let mut neg = negatives.to_vec();
    let mut conds: Vec<MacroCondition> = Vec::new();
    for _ in 0..max_terms {
        let cands = enumerate_candidates(&pos, &neg, opts.use_v2);

        // v2-2.5: 联合条件搜索 — 尝试 AND 二元组合，看是否比单条件 F1 更高
        let mut best: Option<(MacroCondition, f64)> = None;

        if opts.use_v2 && cands.len() >= 2 {
            // 先找 top-K 单条件
            let mut singles: Vec<(MacroCondition, f64)> = cands
                .iter()
                .map(|c| (c.clone(), score_f1(c, &pos, &neg)))
                .collect();
            singles.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            singles.truncate(10);
            // 最佳单条件
            if let Some((ref c, s)) = singles.first() {
                best = Some((c.clone(), *s));
            }
            // 枚举 top-K 二元 AND 组合
            let k = singles.len();
            for i in 0..k {
                for j in (i + 1)..k {
                    let pair = MacroCondition::And(
                        Box::new(singles[i].0.clone()),
                        Box::new(singles[j].0.clone()),
                    );
                    let s = score_f1(&pair, &pos, &neg);
                    if best.as_ref().map_or(true, |(_, bs)| s > *bs) {
                        best = Some((pair, s));
                    }
                }
            }
        } else {
            for c in cands {
                let s = score_f1(&c, &pos, &neg);
                if best.as_ref().map_or(true, |(_, bs)| s > *bs) {
                    best = Some((c, s));
                }
            }
        }

        let Some((c, s)) = best else {
            break;
        };
        if s < min_gain {
            break;
        }
        pos.retain(|st| eval_cond(&c, st));
        neg.retain(|st| eval_cond(&c, st));
        conds.push(c);
        if neg.is_empty() {
            break;
        }
    }
    if conds.is_empty() {
        return None;
    }
    let mut it = conds.into_iter();
    let first = it.next().unwrap();
    Some(it.fold(first, |acc, c| {
        MacroCondition::And(Box::new(acc), Box::new(c))
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// 候选条件
// ─────────────────────────────────────────────────────────────────────────────

fn enumerate_candidates<'a>(
    pos: &[&Sample<'a>],
    neg: &[&Sample<'a>],
    use_v2: bool,
) -> Vec<MacroCondition> {
    let mut out = Vec::new();

    // —— 怒气：分位 + 固定点（v2: 加入所有正/负样本实际值）——
    {
        let pr: Vec<i32> = pos.iter().map(|s| s.state.rage).collect();
        let nr: Vec<i32> = neg.iter().map(|s| s.state.rage).collect();
        let mut ts: BTreeSet<i32> = BTreeSet::new();
        for &q in &[25, 35, 45, 55, 65, 75] {
            ts.insert(q);
        }
        for v in quantiles_i32(&pr, &[0.25, 0.5, 0.75]) {
            ts.insert(v);
        }
        for v in quantiles_i32(&nr, &[0.25, 0.5, 0.75]) {
            ts.insert(v);
        }
        if use_v2 {
            // 枚举所有正/负样本出现过的怒气值作为候选阈值
            for &v in &pr {
                ts.insert(v);
            }
            for &v in &nr {
                ts.insert(v);
            }
            // 如果候选过多（>30），退化为 10 分位点
            if ts.len() > 30 {
                ts.clear();
                for &q in &[25, 35, 45, 55, 65, 75] {
                    ts.insert(q);
                }
                let all: Vec<i32> = pr.iter().chain(nr.iter()).copied().collect();
                for v in quantiles_i32(&all, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]) {
                    ts.insert(v);
                }
            }
        }
        for v in ts {
            if v >= 0 && v <= 100 {
                out.push(MacroCondition::Rage(CmpOp::GtEq, v));
                out.push(MacroCondition::Rage(CmpOp::Lt, v));
            }
        }
    }

    // —— 自身 buff（存在/不存在）——
    let mut self_names: BTreeSet<String> = BTreeSet::new();
    for s in pos.iter().chain(neg.iter()) {
        for b in &s.state.buffs {
            self_names.insert(b.name.clone());
        }
    }
    for n in &self_names {
        out.push(MacroCondition::Buff(n.clone()));
        out.push(MacroCondition::NoBuff(n.clone()));
    }

    // —— 自身 bufftime：正样本中出现的 buff，thresh 从分布取（v2: 加入实际值）——
    let mut self_times: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for s in pos.iter() {
        for b in &s.state.buffs {
            self_times
                .entry(b.name.clone())
                .or_default()
                .push(b.remaining);
        }
    }
    if use_v2 {
        for s in neg.iter() {
            for b in &s.state.buffs {
                self_times
                    .entry(b.name.clone())
                    .or_default()
                    .push(b.remaining);
            }
        }
    }
    for (name, times) in self_times {
        let qs = quantiles_f64(&times, &[0.25, 0.5, 0.75]);
        let mut ths: BTreeSet<i64> = BTreeSet::new();
        for &t in &[1.0f64, 2.0, 3.0, 5.0] {
            ths.insert(t.round() as i64);
        }
        for v in qs {
            ths.insert(v.round() as i64);
        }
        if use_v2 {
            // 枚举正/负样本中实际出现的时间值（取整）
            for &v in &times {
                let r = v.round() as i64;
                ths.insert(r);
                // 也试 ±1 秒偏移
                ths.insert(r.saturating_sub(1));
                ths.insert(r.saturating_add(1));
            }
            // 候选过多时截断
            if ths.len() > 20 {
                let mut sorted: Vec<i64> = ths.into_iter().filter(|&t| t > 0 && t < 60).collect();
                sorted.sort();
                let step = sorted.len() / 10;
                ths = BTreeSet::new();
                for &t in &[1i64, 2, 3, 5] {
                    ths.insert(t);
                }
                if step > 0 {
                    for i in (0..sorted.len()).step_by(step.max(1)) {
                        ths.insert(sorted[i]);
                    }
                }
            }
        }
        for t in ths {
            if t > 0 && t < 60 {
                let tv = t as f64;
                out.push(MacroCondition::BuffTime(name.clone(), CmpOp::Lt, tv));
                out.push(MacroCondition::BuffTime(name.clone(), CmpOp::Gt, tv));
            }
        }
    }

    // —— 目标 buff ——
    let mut tgt_names: BTreeSet<String> = BTreeSet::new();
    for s in pos.iter().chain(neg.iter()) {
        for b in &s.state.target_buffs {
            tgt_names.insert(b.name.clone());
        }
    }
    for n in &tgt_names {
        out.push(MacroCondition::TBuff(n.clone()));
        out.push(MacroCondition::TnoBuff(n.clone()));
    }
    let mut tgt_times: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for s in pos.iter() {
        for b in &s.state.target_buffs {
            tgt_times
                .entry(b.name.clone())
                .or_default()
                .push(b.remaining);
        }
    }
    if use_v2 {
        for s in neg.iter() {
            for b in &s.state.target_buffs {
                tgt_times
                    .entry(b.name.clone())
                    .or_default()
                    .push(b.remaining);
            }
        }
    }
    for (name, times) in tgt_times {
        let qs = quantiles_f64(&times, &[0.25, 0.5, 0.75]);
        let mut ths: BTreeSet<i64> = BTreeSet::new();
        for &t in &[1.0f64, 2.0, 3.0, 5.0] {
            ths.insert(t.round() as i64);
        }
        for v in qs {
            ths.insert(v.round() as i64);
        }
        if use_v2 {
            for &v in &times {
                let r = v.round() as i64;
                ths.insert(r);
                ths.insert(r.saturating_sub(1));
                ths.insert(r.saturating_add(1));
            }
            if ths.len() > 20 {
                let mut sorted: Vec<i64> = ths.into_iter().filter(|&t| t > 0 && t < 60).collect();
                sorted.sort();
                let step = sorted.len() / 10;
                ths = BTreeSet::new();
                for &t in &[1i64, 2, 3, 5] {
                    ths.insert(t);
                }
                if step > 0 {
                    for i in (0..sorted.len()).step_by(step.max(1)) {
                        ths.insert(sorted[i]);
                    }
                }
            }
        }
        for t in ths {
            if t > 0 && t < 60 {
                let tv = t as f64;
                out.push(MacroCondition::TBuffTime(name.clone(), CmpOp::Lt, tv));
                out.push(MacroCondition::TBuffTime(name.clone(), CmpOp::Gt, tv));
            }
        }
    }

    // —— 技能充能 / CD ——
    // skill_cds 里条目两种形态："盾刀"(单 CD) 或 "盾击(2层)"(充能，括号里为当前层数)
    let mut charge_names: BTreeSet<(String, u32)> = BTreeSet::new();
    let mut plain_cd_names: BTreeSet<String> = BTreeSet::new();
    for s in pos.iter().chain(neg.iter()) {
        for cd in &s.state.skill_cds {
            if let Some((base, layers)) = parse_charge_name(&cd.name) {
                charge_names.insert((base, layers));
            } else {
                plain_cd_names.insert(cd.name.clone());
            }
        }
    }
    // skill_energy 候选：技能 X 层数 ≥ 1/2/3（通过 parse 推断最大层）
    let mut per_skill_max: BTreeMap<String, u32> = BTreeMap::new();
    for (name, layer) in &charge_names {
        let e = per_skill_max.entry(name.clone()).or_insert(0);
        if *layer > *e {
            *e = *layer;
        }
    }
    for (name, max_layer) in per_skill_max {
        for k in 1..=max_layer.saturating_add(1).min(4) {
            out.push(MacroCondition::SkillEnergy(name.clone(), CmpOp::GtEq, k));
        }
    }
    // skill_notin_cd 候选
    for name in plain_cd_names {
        out.push(MacroCondition::SkillNotInCd(name));
    }
    // 充能技能的 skill_notin_cd 也给一个（语义：至少 1 层）
    for (name, _) in charge_names.iter().collect::<Vec<_>>() {
        out.push(MacroCondition::SkillNotInCd(name.clone()));
    }

    // —— last_skill ——
    let mut last_names: BTreeSet<String> = BTreeSet::new();
    for s in pos.iter().chain(neg.iter()) {
        if let Some(ls) = &s.last_main_base {
            last_names.insert(ls.clone());
        }
    }
    for n in last_names {
        out.push(MacroCondition::LastSkill(n.clone()));
        out.push(MacroCondition::LastSkillNot(n));
    }

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// 评分 & 求值
// ─────────────────────────────────────────────────────────────────────────────

fn score_f1<'a>(c: &MacroCondition, pos: &[&Sample<'a>], neg: &[&Sample<'a>]) -> f64 {
    let tp = pos.iter().filter(|s| eval_cond(c, s)).count() as f64;
    let fp = neg.iter().filter(|s| eval_cond(c, s)).count() as f64;
    let fn_ = pos.len() as f64 - tp;
    if tp == 0.0 {
        return 0.0;
    }
    let precision = tp / (tp + fp).max(1e-9);
    let recall = tp / (tp + fn_).max(1e-9);
    2.0 * precision * recall / (precision + recall).max(1e-9)
}

fn eval_cond<'a>(c: &MacroCondition, s: &Sample<'a>) -> bool {
    let st = s.state;
    match c {
        MacroCondition::Rage(op, v) => op.compare_i32(st.rage, *v),
        MacroCondition::Buff(n) => st.buffs.iter().any(|b| &b.name == n),
        MacroCondition::NoBuff(n) => !st.buffs.iter().any(|b| &b.name == n),
        MacroCondition::BuffTime(n, op, v) => st
            .buffs
            .iter()
            .find(|b| &b.name == n)
            .map_or(false, |b| op.compare_f64(b.remaining, *v)),
        MacroCondition::TBuff(n) => st.target_buffs.iter().any(|b| &b.name == n),
        MacroCondition::TnoBuff(n) => !st.target_buffs.iter().any(|b| &b.name == n),
        MacroCondition::TBuffTime(n, op, v) => st
            .target_buffs
            .iter()
            .find(|b| &b.name == n)
            .map_or(false, |b| op.compare_f64(b.remaining, *v)),
        MacroCondition::SkillNotInCd(n) => skill_feasible(n, st),
        MacroCondition::SkillEnergy(n, op, v) => {
            let layers = current_charge_layers(n, st);
            op.compare_u32(layers, *v)
        }
        MacroCondition::LastSkill(n) => s.last_main_base.as_deref() == Some(n.as_str()),
        MacroCondition::LastSkillNot(n) => s.last_main_base.as_deref() != Some(n.as_str()),
        MacroCondition::And(a, b) => eval_cond(a, s) && eval_cond(b, s),
        MacroCondition::Or(a, b) => eval_cond(a, s) || eval_cond(b, s),
        _ => true,
    }
}

fn count_atoms(c: &MacroCondition) -> usize {
    match c {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => count_atoms(a) + count_atoms(b),
        _ => 1,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 可行性 & 工具
// ─────────────────────────────────────────────────────────────────────────────

/// skill S 在 state 能否释放（仅看 CD/充能；忽略怒气/姿态，姿态在分页时已处理）
fn skill_feasible(base: &str, st: &InputState) -> bool {
    for cd in &st.skill_cds {
        if let Some((n, layers)) = parse_charge_name(&cd.name) {
            if n == base {
                return layers >= 1;
            }
        } else if cd.name == base && cd.remaining > 0.0 {
            return false;
        }
    }
    true
}

/// 当前充能层数：状态里找 "X(N层)"，找不到视为满层（用 2 做默认，够表达 ≥1/≥2）
fn current_charge_layers(base: &str, st: &InputState) -> u32 {
    for cd in &st.skill_cds {
        if let Some((n, layers)) = parse_charge_name(&cd.name) {
            if n == base {
                return layers;
            }
        }
    }
    // 该技能不在 skill_cds → 满层；但也可能根本不是充能技能，返回一个足够大的值
    99
}

/// "盾击(2层)" → Some(("盾击", 2))；其他返回 None
fn parse_charge_name(s: &str) -> Option<(String, u32)> {
    let open = s.rfind('(')?;
    let close = s.rfind("层)")?;
    if close <= open {
        return None;
    }
    let num = &s[open + 1..close];
    let n: u32 = num.parse().ok()?;
    Some((s[..open].to_string(), n))
}

fn quantiles_i32(xs: &[i32], qs: &[f64]) -> Vec<i32> {
    if xs.is_empty() {
        return vec![];
    }
    let mut v = xs.to_vec();
    v.sort_unstable();
    qs.iter()
        .map(|q| {
            let idx = ((v.len() as f64 - 1.0) * q).round() as usize;
            v[idx.min(v.len() - 1)]
        })
        .collect()
}

fn quantiles_f64(xs: &[f64], qs: &[f64]) -> Vec<f64> {
    if xs.is_empty() {
        return vec![];
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    qs.iter()
        .map(|q| {
            let idx = ((v.len() as f64 - 1.0) * q).round() as usize;
            v[idx.min(v.len() - 1)]
        })
        .collect()
}

/// 不占主 GCD 的技能：盾飞/盾回（切姿态、影响循环）、血怒（自给怒气）。
/// 这类技能：
///  - 不与主技能争抢 GCD 槽，负样本池应独立构造（任何"可释放但没放"的时刻都算负）
///  - 条件学习应更激进（更多 term / 更低 min_gain / 允许部分覆盖），宁可多条件不要无条件
///  - 排序上不受主技能顺序约束
fn is_off_gcd(base: &str) -> bool {
    matches!(base, "盾飞" | "盾回" | "血怒")
}
/// 其中 盾飞/盾回 还额外会切姿态，重要性最高
fn is_stance_switcher(base: &str) -> bool {
    matches!(base, "盾飞" | "盾回")
}

fn base_skill_name(name: &str) -> &str {
    let head = name.split('·').next().unwrap_or(name);
    match head {
        "月照连营" | "雁门迢递" => "阵云结晦",
        "隐刀" => "闪刀",
        "惊沙" => "盾毅",
        other => other,
    }
}

fn stance_key(s: Stance) -> u8 {
    match s {
        Stance::Shield => 1,
        Stance::Blade => 2,
        Stance::Wall => 3,
        _ => 0,
    }
}
fn unkey_stance(k: u8) -> Stance {
    match k {
        1 => Stance::Shield,
        2 => Stance::Blade,
        3 => Stance::Wall,
        _ => Stance::Any,
    }
}
fn stance_label(s: Stance) -> &'static str {
    match s {
        Stance::Shield => "shield",
        Stance::Blade => "blade",
        Stance::Wall => "wall",
        _ => "any",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// v2: 开场裁剪
// ─────────────────────────────────────────────────────────────────────────────

/// 检测稳态起点索引。策略：
/// 1. 找到怒气首次 ≥50 且发生了一次姿态"回切"（与首个主 GCD 同姿态）的位置
/// 2. 如果没有回切信号，回退为 skip 前 min(8, len/4) 个主 GCD
fn detect_steady_start(samples: &[Sample]) -> usize {
    if samples.len() < 8 {
        return 0;
    }
    let first_stance = samples
        .iter()
        .find(|s| s.is_main)
        .map(|s| stance_key(s.state.stance));
    let Some(first_st) = first_stance else {
        return 0;
    };

    // 记录是否离开过初始姿态
    let mut left_first = false;
    let mut main_count = 0usize;
    for (i, s) in samples.iter().enumerate() {
        if !s.is_main {
            continue;
        }
        main_count += 1;
        let sk = stance_key(s.state.stance);
        if sk != first_st {
            left_first = true;
        }
        // 条件：离开过初始姿态 → 回来 + 怒气 ≥ 50
        if left_first && sk == first_st && s.state.rage >= 50 {
            return i;
        }
    }
    // 没有回切，回退策略
    let skip_main = main_count.min(8).min(samples.len() / 4);
    let mut counted = 0usize;
    for (i, s) in samples.iter().enumerate() {
        if s.is_main {
            counted += 1;
        }
        if counted >= skip_main {
            return i + 1;
        }
    }
    0
}
