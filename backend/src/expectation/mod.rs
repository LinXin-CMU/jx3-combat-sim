//! 坚铁 / 寒甲 期望分布传播子系统
//!
//! 将随机受击事件 + 招架判定 转换为**确定性**的概率分布演化，
//! 输出每帧的 buff 期望层数、存活率等指标，再注入主伤害链。
//!
//! 算法详见：
//! - `.claude/jiantie_expectation_simulation.md`
//! - `.claude/hanjia_expectation_simulation.md`

pub mod jiantie;
pub mod hanjia;

pub use jiantie::{JiantieDist, JiantieFrameStats};
pub use hanjia::{HanjiaCarry, HanjiaFrameStats, encode_hanjia};

/// 帧率（与主模拟器一致）
pub const FPS: u32 = 16;

/// Poisson 模式：把"平均攻击间隔"翻译成每帧受击概率
///
/// `boss_attack_interval_sec`：boss 平均攻击间隔（秒）
/// 返回：`h = 1 - exp(-1 / (16Δ))`
pub fn poisson_h(boss_attack_interval_sec: f64) -> f64 {
    if boss_attack_interval_sec <= 0.0 {
        return 0.0;
    }
    let lam_per_frame = 1.0 / (boss_attack_interval_sec * FPS as f64);
    1.0 - (-lam_per_frame).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisson_h_basic() {
        // Δ = 2s → h ≈ 1/32 ≈ 0.03125
        let h = poisson_h(2.0);
        assert!((h - (1.0 - (-1.0/32.0_f64).exp())).abs() < 1e-12);
        assert!((h - 0.0307_f64).abs() < 1e-3);
    }

    #[test]
    fn poisson_h_edge_zero() {
        assert_eq!(poisson_h(0.0), 0.0);
        assert_eq!(poisson_h(-1.0), 0.0);
    }
}
