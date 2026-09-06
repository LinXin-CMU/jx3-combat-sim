//! Workflow A step 6, executed server-side using the existing candidate algorithms.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize)]
pub struct Score {
    pub dps: f64,
    pub counts: BTreeMap<String, u32>,
    pub weighted_cast_diff: u64,
}

pub fn counts(response: &crate::SimulateResponse) -> BTreeMap<String, u32> {
    let mut counts = BTreeMap::new();
    for event in response.timeline.iter().filter(|event| !event.triggered) {
        let full = event.name.as_str();
        let key = if matches!(full, "阵云结晦·雾海" | "月照连营·雾海" | "雁门迢递·雾海")
        {
            full
        } else {
            full.split('·').next().unwrap_or(full)
        };
        let key = match key {
            "月照连营" | "雁门迢递" => "阵云结晦",
            "隐刀" => "闪刀",
            "惊沙" => "盾毅",
            _ => key,
        };
        *counts.entry(key.to_string()).or_insert(0) += 1;
    }
    counts
}

fn sensitive(skill: &str) -> bool {
    matches!(skill, "血怒" | "业火麟光" | "偃守孤旌")
}
fn diff(a: &BTreeMap<String, u32>, b: &BTreeMap<String, u32>, weighted: bool) -> u64 {
    a.keys()
        .chain(b.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|key| {
            let delta = a
                .get(key)
                .copied()
                .unwrap_or(0)
                .abs_diff(b.get(key).copied().unwrap_or(0));
            u64::from(delta) * if weighted && sensitive(key) { 5 } else { 1 }
        })
        .sum()
}

#[derive(Clone, Copy, Debug)]
enum Strategy {
    CastDiff,
    Prune,
    Swap,
    Tighten,
}
impl Strategy {
    fn name(self) -> &'static str {
        match self {
            Self::CastDiff => "cast_diff",
            Self::Prune => "prune",
            Self::Swap => "swap",
            Self::Tighten => "tighten",
        }
    }
    fn rounds(self) -> usize {
        if matches!(self, Self::Swap) {
            8
        } else {
            5
        }
    }
}

fn candidates(text: &str, strategy: Strategy) -> Vec<String> {
    match strategy {
        Strategy::Prune | Strategy::CastDiff => crate::macro_prune::list_prune_candidates(text)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !matches!(strategy, Strategy::Prune) || !sensitive(&c.skill))
            .map(|c| c.after_macro)
            .collect(),
        Strategy::Swap => crate::macro_prune::list_swap_candidates(text)
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.after_macro)
            .collect(),
        Strategy::Tighten => crate::macro_prune::list_tighten_candidates(text)
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.after_macro)
            .collect(),
    }
}

fn acceptable(strategy: Strategy, current: &Score, next: &Score, stage_start: &Score) -> bool {
    let drop = (current.dps - next.dps) / current.dps.max(1.0);
    match strategy {
        Strategy::CastDiff => next.weighted_cast_diff < current.weighted_cast_diff && drop <= 0.005,
        Strategy::Swap => (next.dps - current.dps) / current.dps.max(1.0) >= 0.002,
        Strategy::Tighten => drop <= 0.002,
        // Match Workflow A: protect counts of skills actually present at stage start.
        // Including newly recovered skills here freezes a deadlocked initial macro.
        Strategy::Prune => {
            drop <= 0.002
                && stage_start.counts.keys().all(|key| {
                    stage_start
                        .counts
                        .get(key)
                        .copied()
                        .unwrap_or(0)
                        .abs_diff(next.counts.get(key).copied().unwrap_or(0))
                        <= if sensitive(key) { 1 } else { 2 }
                })
        }
    }
}

fn overflow(text: &str) -> u64 {
    super::distillation::page_lengths(text)
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["chars"].as_u64().unwrap_or(0).saturating_sub(128))
        .sum()
}

/// Every simulation is charged by the caller. Stop with the best tested candidate.
pub fn tune(
    initial: &str,
    target: &crate::SimulateResponse,
    max_evaluations: usize,
    mut simulate: impl FnMut(&str) -> Option<crate::SimulateResponse>,
) -> Value {
    let started = Instant::now();
    let target_counts = counts(target);
    let mut evaluations = 0;
    let mut cache = HashMap::<String, Score>::new();
    let mut evaluate = |text: &str| -> Option<Score> {
        if let Some(score) = cache.get(text) {
            return Some(score.clone());
        }
        if evaluations >= max_evaluations || started.elapsed() >= Duration::from_secs(20) {
            return None;
        }
        let response = simulate(text)?;
        evaluations += 1;
        let counts = counts(&response);
        let score = Score {
            dps: response.dps,
            weighted_cast_diff: diff(&target_counts, &counts, true),
            counts,
        };
        cache.insert(text.into(), score.clone());
        Some(score)
    };
    let Some(initial_score) = evaluate(initial) else {
        return json!({"status":"evaluation_limit","macro_text":initial,"evaluations":0,"history":[],"trials":[]});
    };
    let mut best = initial.to_string();
    let mut best_score = initial_score.clone();
    let mut history = Vec::new();
    let mut trials = Vec::new();
    let mut limited = false;
    let mut accepted_versions = BTreeSet::from([best.clone()]);
    let mut passes = 0;
    let mut used_strategies = BTreeSet::new();
    // Repeated clicks in Workflow A start from the last accepted macro. Keep that
    // continuation inside this tool, sharing actual-simulation cache and budget.
    loop {
        passes += 1;
        let pass_start = best.clone();
        let mut strategies = vec![];
        if diff(&target_counts, &best_score.counts, false) > 5 {
            strategies.push(Strategy::CastDiff);
        }
        strategies.extend([Strategy::Prune, Strategy::Swap, Strategy::Tighten]);
        for &strategy in &strategies {
            used_strategies.insert(strategy.name());
            let stage_start = best_score.clone();
            for round in 1..=strategy.rounds() {
                let mut picked: Option<(String, Score)> = None;
                for candidate in candidates(&best, strategy) {
                    if accepted_versions.contains(&candidate)
                        || overflow(&candidate) > overflow(&best)
                    {
                        continue;
                    }
                    let Some(score) = evaluate(&candidate) else {
                        limited = true;
                        break;
                    };
                    let accepted = acceptable(strategy, &best_score, &score, &stage_start);
                    trials.push(json!({"pass":passes,"strategy":strategy.name(),"round":round,"macro_text":candidate,"score":score,"eligible":accepted}));
                    if !accepted {
                        continue;
                    }
                    let better = picked.as_ref().is_none_or(|(_, previous)| {
                        if matches!(strategy, Strategy::CastDiff)
                            && score.weighted_cast_diff != previous.weighted_cast_diff
                        {
                            score.weighted_cast_diff < previous.weighted_cast_diff
                        } else {
                            score.dps > previous.dps
                        }
                    });
                    if better {
                        picked = Some((candidate, score));
                    }
                }
                let Some((candidate, score)) = picked else {
                    break;
                };
                best = candidate;
                best_score = score;
                accepted_versions.insert(best.clone());
                history.push(json!({"pass":passes,"strategy":strategy.name(),"round":round,"macro_text":best,"score":best_score}));
                if limited {
                    break;
                }
            }
            if limited {
                break;
            }
        }
        if limited || best == pass_start {
            break;
        }
    }
    drop(evaluate);
    json!({"status":if started.elapsed() >= Duration::from_secs(20) { "time_limit" } else if limited { "evaluation_limit" } else { "completed" },
        "macro_text":best,"initial":initial_score,"final":best_score,"evaluations":evaluations,
        "passes":passes,"strategies":used_strategies,"history":history,"trials":trials,
        "target":{"dps":target.dps,"counts":target_counts},
        "dps_ratio_to_target":if target.dps > 0.0 { Some(best_score.dps / target.dps) } else { None },
        "missing_target_skills":target_counts.keys().filter(|key| !best_score.counts.contains_key(*key)).collect::<Vec<_>>(),
        "policy":"workflow-a/v2: cast_diff(5x sensitive, <=0.5% DPS loss), prune(<=0.2%, existing skill drift1/2), swap(>=0.2%), tighten(<=0.2%); shared evaluation budget"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_by_workflow_objective_not_only_dps() {
        let a = Score {
            dps: 1000.,
            counts: BTreeMap::from([("血怒".into(), 10)]),
            weighted_cast_diff: 10,
        };
        let mut b = a.clone();
        b.dps = 999.;
        b.weighted_cast_diff = 9;
        assert!(acceptable(Strategy::CastDiff, &a, &b, &a));
        assert!(!acceptable(Strategy::Swap, &a, &b, &a));
        b.weighted_cast_diff = 11;
        assert!(acceptable(Strategy::Tighten, &a, &b, &a));
        b.counts.insert("血怒".into(), 12);
        assert!(!acceptable(Strategy::Prune, &a, &b, &a));
    }

    #[test]
    fn legacy_prune_can_recover_skills_missing_from_deadlocked_start() {
        let stuck = Score {
            dps: 26446.73,
            counts: BTreeMap::from([("业火麟光".into(), 6)]),
            weighted_cast_diff: 363,
        };
        let recovered = Score {
            dps: 451992.19,
            counts: BTreeMap::from([("业火麟光".into(), 6), ("盾击".into(), 320)]),
            weighted_cast_diff: 467,
        };
        assert!(!acceptable(Strategy::CastDiff, &stuck, &recovered, &stuck));
        assert!(acceptable(Strategy::Prune, &stuck, &recovered, &stuck));
        let mut drifted = recovered.clone();
        drifted.counts.insert("业火麟光".into(), 8);
        assert!(!acceptable(Strategy::Prune, &stuck, &drifted, &stuck));
    }
}
