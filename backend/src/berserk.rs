//! 暴怒资源流水；独立于普通怒气，也记录技能之间的整秒回复。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BerserkTransaction {
    pub time_seconds: f64,
    pub source: String,
    pub requested_delta: i32,
    pub applied_delta: i32,
    pub before: i32,
    pub after: i32,
    pub cap: i32,
    pub overflow: u32,
}

impl crate::Player {
    pub fn add_berserk_value_from(&mut self, delta: i32, source: &str) {
        self.apply_berserk_delta(delta, source, self.current_time);
    }

    pub(crate) fn apply_berserk_delta(&mut self, delta: i32, source: &str, time: f64) {
        if !self.uses_berserk() {
            return;
        }
        let before = self.berserk_value;
        let cap = self.max_berserk_value();
        let requested = before.saturating_add(delta);
        self.set_berserk_value(requested);
        if !self.lite_mode && delta != 0 {
            self.berserk_transactions.push(BerserkTransaction {
                time_seconds: time,
                source: source.to_string(),
                requested_delta: delta,
                applied_delta: self.berserk_value - before,
                before,
                after: self.berserk_value,
                cap,
                overflow: requested.saturating_sub(cap).max(0) as u32,
            });
        }
    }
}
