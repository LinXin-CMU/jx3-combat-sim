use crate::{
    agent::{hash::canonical_sha256, AgentRuntime, ScenarioSnapshotV1},
    GameVersion, Mount, SimulateRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Read, sync::OnceLock};

pub const ALGORITHM_VERSION: &str = "macro-compile/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroCompileRequestV1 {
    pub simulation: SimulateRequest,
    pub version: GameVersion,
    pub mount: Mount,
    #[serde(default)]
    pub initial_macro: Option<String>,
    #[serde(default = "default_simulations")]
    pub max_simulations: u32,
    #[serde(default = "default_wall_time")]
    pub wall_time_ms: u64,
    #[serde(default = "default_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_pages")]
    pub max_pages: usize,
    #[serde(default = "default_tolerance")]
    pub time_tolerance: f64,
}
fn default_simulations() -> u32 {
    96
}
fn default_wall_time() -> u64 {
    60_000
}
fn default_rounds() -> u32 {
    6
}
fn default_pages() -> usize {
    2
}
fn default_tolerance() -> f64 {
    1.0 / 16.0
}

impl MacroCompileRequestV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        let sim = &self.simulation;
        if !(2..=256).contains(&self.max_simulations)
            || !(1000..=120_000).contains(&self.wall_time_ms)
            || !(1..=12).contains(&self.max_rounds)
            || !(1..=6).contains(&self.max_pages)
            || !self.time_tolerance.is_finite()
            || !(0.0..=1.0).contains(&self.time_tolerance)
        {
            return Err("预算或宏限制超出范围。");
        }
        if sim.sequence.is_empty()
            || sim.sequence.len() > 2048
            || sim
                .sequence
                .iter()
                .any(|s| s.is_empty() || s.len() > 128 || s.starts_with("__"))
            || sim
                .macro_text
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return Err("请选择包含 1～2048 个操作的手动技能轴，宏模式不能作为目标轴。");
        }
        if sim.attributes.is_none() || sim.target.is_none() {
            return Err("缺少完整属性或目标配置。");
        }
        let attrs =
            serde_json::to_value(sim.attributes.as_ref().unwrap()).map_err(|_| "属性配置无效。")?;
        if attrs.as_object().unwrap().values().any(|v| {
            v.as_f64()
                .is_none_or(|n| !n.is_finite() || n < 0.0 || n > 1e9)
        }) {
            return Err("属性必须是有限的非负数。");
        }
        let target = sim.target.as_ref().unwrap();
        if target.level == 0
            || target.level > 200
            || !target.defense_bonus.is_finite()
            || !target.damage_cof.is_finite()
            || target.defense_bonus.abs() > 10_000.0
            || target.damage_cof.abs() > 10_000.0
            || sim.network_delay > 10_000
            || sim.initial_rage.is_some_and(|r| !(0..=100).contains(&r))
        {
            return Err("目标、延迟或初始资源配置无效。");
        }
        if sim.pauses.len() > 32
            || sim.pre_releases.len() > 64
            || sim.talents.len() > 32
            || sim.recipes.len() > 256
            || sim.channel_ticks.len() > 2048
            || sim.timing_offsets.len() > 2048
            || sim.qijin_buffs.len() > 2048
            || sim.team_buffs.len() > 128
            || sim.equipment.len() > 32
            || sim.pauses.iter().any(|(t, d)| {
                !t.is_finite() || !d.is_finite() || *t < 0.0 || *d < 0.0 || *t + *d > 1200.0
            })
            || sim.pre_releases.iter().any(|p| {
                !p.time_before.is_finite()
                    || p.time_before <= 0.0
                    || p.time_before > 1200.0
                    || p.skill.len() > 128
            })
            || sim
                .timing_offsets
                .values()
                .any(|v| !v.is_finite() || v.abs() > 1200.0)
            || sim.channel_ticks.values().any(|v| *v > 256)
            || sim
                .macro_duration
                .is_some_and(|v| !v.is_finite() || !(0.0..=1200.0).contains(&v))
            || sim
                .boss_attack_interval
                .is_some_and(|v| !v.is_finite() || !(1.0 / 16.0..=1200.0).contains(&v))
        {
            return Err("场景配置超出有界实验范围。");
        }
        // A simulation cannot be interrupted mid-step. Bound event density before
        // entering the engine, including nested user-supplied buff schedules.
        if sim.team_buffs.iter().any(|buff| {
            buff.key.len() > 128
                || buff.stacks > 1024
                || !buff.first_release.is_finite()
                || !(0.0..=1200.0).contains(&buff.first_release)
                || !buff.duration.is_finite()
                || !(0.0..=1200.0).contains(&buff.duration)
                || !buff.period.is_finite()
                || !(buff.period == 0.0 || (1.0 / 16.0..=1200.0).contains(&buff.period))
                || buff.release_times.as_ref().is_some_and(|times| {
                    times.len() > 2048
                        || times
                            .iter()
                            .any(|t| !t.is_finite() || !(0.0..=1200.0).contains(t))
                })
        }) || sim
            .team_buffs
            .iter()
            .map(|b| b.release_times.as_ref().map_or(0, Vec::len))
            .sum::<usize>()
            > 8192
        {
            return Err("团辅释放排程或参数超出有界实验范围。");
        }
        if let Some(text) = &self.initial_macro {
            if text.len() > 8192 {
                return Err("起始宏最多 8192 字节。");
            }
            if !text.trim().is_empty() {
                let parsed = crate::macro_parser::parse_macro_text(text)
                    .map_err(|_| "起始宏语法无效，请先修正。")?;
                if parsed.pages.len() > 16
                    || parsed.pages.iter().map(|p| p.lines.len()).sum::<usize>() > 128
                {
                    return Err("起始宏页数或语句数量超出范围。");
                }
            }
        }
        Ok(())
    }

    pub fn snapshot(&self, runtime: &AgentRuntime) -> Result<ScenarioSnapshotV1, &'static str> {
        self.validate()?;
        if self.version != runtime.game_version() || self.mount != runtime.mount() {
            return Err("版本或心法已变化，请重新读取当前技能轴。");
        }
        ScenarioSnapshotV1::capture(self.version, self.mount, self.simulation.clone())
            .map_err(|_| "无法冻结场景，请检查完整模拟输入。")
    }
}

/// Includes the running executable (compiled scripts/buffs), not just a possibly
/// absent Git commit. Never reads or emits source documents or credentials.
pub fn executable_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let compute = || -> std::io::Result<String> {
            let mut file = std::fs::File::open(std::env::current_exe()?)?;
            let mut sha = Sha256::new();
            let mut buffer = [0_u8; 65_536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                sha.update(&buffer[..n]);
            }
            Ok(format!("{:x}", sha.finalize()))
        };
        compute().unwrap_or_else(|_| "unknown".into())
    })
    .as_str()
}

pub fn runtime_hash(runtime: &AgentRuntime) -> Result<String, &'static str> {
    let context = runtime.context();
    canonical_sha256(&json!({
        "schema":"harness-runtime/v1", "executable_hash":executable_hash(),
        "version":runtime.game_version(), "mount":runtime.mount(),
        "constants":context.constants, "skills":context.skills, "talents":context.talents,
        "recipes":context.recipes, "team_buffs":context.team_buffs, "formations":context.formations,
    }))
    .map_err(|_| "无法计算运行环境身份。")
}

pub fn experiment_hash(
    request: &MacroCompileRequestV1,
    scenario: &ScenarioSnapshotV1,
    runtime_hash: &str,
) -> Result<String, &'static str> {
    let mut normalized = request.clone();
    normalized.simulation = scenario.simulation.clone();
    canonical_sha256(
        &json!({"algorithm":ALGORITHM_VERSION,"objective":"faithful_reproduction",
        "request":normalized,"scenario_hash":scenario.scenario_hash,"runtime_hash":runtime_hash}),
    )
    .map_err(|_| "无法计算实验身份。")
}

#[cfg(test)]
#[path = "../../tests/harness/contract.rs"]
mod tests;
