//! Agent 暴怒证据：只汇总模拟器流水，不以普通怒气推算暴怒。
use crate::{SimulateResponse, berserk::BerserkTransaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BerserkObservation {
    pub modeled: bool,
    pub overflow_tracking_available: bool,
    pub cap: i32,
    pub ending: i32,
    pub generated_before_cap: u64,
    pub gained_after_cap: u64,
    pub spent: u64,
    pub overflow_events: usize,
    pub overflow_total: u64,
    pub overflow_by_source: BTreeMap<String, u64>,
}

pub fn observe(response: &SimulateResponse) -> Option<BerserkObservation> {
    let ending = response.berserk_value?;
    let transactions = response.berserk_transactions.as_deref().unwrap_or_default();
    let mut sources = BTreeMap::new();
    for transaction in transactions.iter().filter(|t| t.overflow > 0) {
        *sources.entry(transaction.source.clone()).or_insert(0) += transaction.overflow as u64;
    }
    Some(BerserkObservation {
        modeled: true,
        overflow_tracking_available: response.berserk_transactions.is_some(),
        cap: response.max_berserk_value.unwrap_or(120),
        ending,
        generated_before_cap: transactions
            .iter()
            .map(|t| t.requested_delta.max(0) as u64)
            .sum(),
        gained_after_cap: transactions
            .iter()
            .map(|t| t.applied_delta.max(0) as u64)
            .sum(),
        spent: transactions
            .iter()
            .map(|t| (-t.applied_delta).max(0) as u64)
            .sum(),
        overflow_events: transactions.iter().filter(|t| t.overflow > 0).count(),
        overflow_total: transactions.iter().map(|t| t.overflow as u64).sum(),
        overflow_by_source: sources,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BerserkEvent {
    pub transaction_number: usize,
    #[serde(flatten)]
    pub transaction: BerserkTransaction,
    /// Only a same-time blood-rage cast is the actual source. Passive ticks have no cast owner.
    pub event_number: Option<usize>,
    pub macro_page: Option<usize>,
    pub macro_line: Option<usize>,
    pub previous_event_number: Option<usize>,
    pub next_event_number: Option<usize>,
}

pub fn events(response: &SimulateResponse, overflow_only: bool) -> Vec<BerserkEvent> {
    let active = response
        .timeline
        .iter()
        .filter(|e| !e.triggered)
        .collect::<Vec<_>>();
    response
        .berserk_transactions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            if overflow_only {
                t.overflow > 0
            } else {
                t.after == t.cap
            }
        })
        .map(|(index, t)| {
            let owner = active.iter().enumerate().find(|(_, e)| {
                t.source == "不归·血怒回复"
                    && e.skill_id == 13040
                    && (e.cast_time - t.time_seconds).abs() < 0.001
            });
            BerserkEvent {
                transaction_number: index + 1,
                transaction: t.clone(),
                event_number: owner.map(|(i, _)| i + 1),
                macro_page: owner.and_then(|(_, e)| e.macro_page),
                macro_line: owner.and_then(|(_, e)| e.macro_line),
                previous_event_number: active
                    .iter()
                    .rposition(|e| e.cast_time <= t.time_seconds)
                    .map(|i| i + 1),
                next_event_number: active
                    .iter()
                    .position(|e| e.cast_time > t.time_seconds)
                    .map(|i| i + 1),
            }
        })
        .collect()
}
