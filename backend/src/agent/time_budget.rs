//! Runtime deadlines based on observed latency and newly registered evidence.
pub struct AdaptiveTimeBudget {
    deadline_ms: u64,
    ceiling_ms: u64,
    request_ms: u64,
    evidence_count: usize,
}

impl AdaptiveTimeBudget {
    pub fn new(configured_ms: u64, model: &str) -> Self {
        Self {
            deadline_ms: configured_ms,
            // Explicit short/custom deadlines remain strict (including cancellation tests).
            ceiling_ms: if configured_ms == 180_000 { 600_000 } else { configured_ms },
            request_ms: if model.contains("pro") { 150_000 } else { 120_000 },
            evidence_count: 0,
        }
    }

    pub fn advance(&mut self, elapsed_ms: u64, evidence_count: usize) -> bool {
        let old = self.deadline_ms;
        if evidence_count > self.evidence_count && elapsed_ms < self.ceiling_ms {
            self.evidence_count = evidence_count;
            let needed = elapsed_ms.saturating_add(self.request_ms).saturating_add(45_000);
            self.deadline_ms = self.deadline_ms.max(needed.min(self.ceiling_ms));
        }
        old != self.deadline_ms
    }

    pub fn observe_response(&mut self, duration_ms: u64) {
        self.request_ms = duration_ms.saturating_mul(2).saturating_add(15_000)
            .clamp(120_000, 240_000);
    }

    pub fn allow_output(&mut self, tokens: u32) {
        let floor = if tokens >= 32_768 { 240_000 } else if tokens >= 16_384 { 180_000 } else { 120_000 };
        self.request_ms = self.request_ms.max(floor);
    }

    /// Called only for bounded completion retries, not each planning iteration.
    /// A larger answer allowance needs actual time to produce the answer.
    pub fn reserve_completion(&mut self, elapsed_ms: u64) {
        self.request_ms = self.request_ms.max(180_000);
        self.deadline_ms = self.deadline_ms.max(
            elapsed_ms.saturating_add(self.request_ms).min(self.ceiling_ms));
    }

    pub fn deadline_ms(&self) -> u64 { self.deadline_ms }

    pub fn request_ms(&self, elapsed_ms: u64) -> u64 {
        self.request_ms.min(self.deadline_ms.saturating_sub(elapsed_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_retry_gets_time_without_new_evidence() {
        let mut budget = AdaptiveTimeBudget::new(180_000, "flash");
        budget.reserve_completion(182_124);
        assert_eq!(budget.request_ms(182_124), 180_000);
        assert_eq!(budget.deadline_ms(), 362_124);
        let mut explicit = AdaptiveTimeBudget::new(200, "flash");
        explicit.reserve_completion(180);
        assert_eq!(explicit.request_ms(180), 20);
    }
    #[test]
    fn new_evidence_and_observed_latency_extend_a_real_task() {
        let mut budget = AdaptiveTimeBudget::new(180_000, "deepseek-v4-flash");
        assert!(!budget.advance(0, 1));
        budget.observe_response(100_000);
        assert!(budget.advance(100_000, 3));
        assert_eq!(budget.deadline_ms(), 360_000);
        assert_eq!(budget.request_ms(100_000), 215_000);
    }
    #[test]
    fn repeated_checks_do_not_buy_more_time() {
        let mut budget = AdaptiveTimeBudget::new(180_000, "flash");
        budget.advance(0, 2);
        budget.observe_response(100_000);
        assert!(!budget.advance(150_000, 2));
        assert_eq!(budget.deadline_ms(), 180_000);
    }
    #[test]
    fn explicit_limits_and_absolute_ceiling_still_apply() {
        let mut short = AdaptiveTimeBudget::new(200, "pro");
        short.advance(100, 10);
        assert_eq!(short.request_ms(150), 50);
        let mut long = AdaptiveTimeBudget::new(180_000, "pro");
        long.observe_response(230_000);
        long.advance(550_000, 20);
        assert_eq!(long.deadline_ms(), 600_000);
        assert_eq!(long.request_ms(599_000), 1_000);
    }
}
