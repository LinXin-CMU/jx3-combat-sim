//! Test-server shield cooldown procs. Legacy expectation cooldowns stay separate.
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    #[default]
    Cumulative,
    Random,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResetOptions {
    pub mode: ResetMode,
    /// Fixed within a comparison, recorded in the scenario for replay.
    pub seed: u32,
}

pub fn is_zero_seed(seed: &u32) -> bool {
    *seed == 0
}

pub struct ShieldResetProc {
    mode: ResetMode,
    credit: f64,
    rng: StdRng,
}

impl ShieldResetProc {
    pub fn new(options: ResetOptions) -> Self {
        Self {
            mode: options.mode,
            credit: 0.0,
            rng: StdRng::seed_from_u64(options.seed as u64),
        }
    }

    pub fn trigger(&mut self, chance: f64) -> bool {
        let chance = chance.clamp(0.0, 1.0);
        match self.mode {
            ResetMode::Random => self.rng.gen_bool(chance),
            ResetMode::Cumulative => {
                self.credit += chance;
                if self.credit + 1e-9 >= 1.0 {
                    self.credit = (self.credit - 1.0).max(0.0);
                    true
                } else {
                    false
                }
            }
        }
    }
}

impl crate::Player {
    /// One trial per successful 云城盾 damaging cast while 盾压 is cooling down.
    /// Uses the CD setter so macro decisions and snapshots see the reset immediately.
    pub fn try_reset_test_dunya(&mut self) {
        if self.version != crate::GameVersion::CangShengZhuShiTest
            || self.mount != crate::Mount::FenShanJin
            || self.has_talent(91004)
            || self.active_cds.get("cd_盾压").copied().unwrap_or(0.0) <= self.current_time
        {
            return;
        }
        let chance = 0.35
            + if self.has_recipe(4007) { 0.05 } else { 0.0 }
            + if self.has_recipe(4008) { 0.05 } else { 0.0 };
        if self.shield_reset_proc.trigger(chance) {
            self.reset_cd("cd_盾压");
        }
    }
}
