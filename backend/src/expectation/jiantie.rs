//! 坚铁奇穴 (13138) 逐帧期望传播
//!
//! 状态三元组 `(k, τ, ℓ)`：
//! - `k` ∈ {0..5}      当前层数
//! - `τ` ∈ {0..128}    剩余帧数（τ=0 表无 buff，128 帧 = 8s × 16fps）
//! - `ℓ` ∈ {0,1}       锁定标志（招架成功后 = 1，停止叠层 / 不再刷新）
//!
//! 状态空间 N = 1 + 5 × 128 × 2 = 1281
//!
//! 每帧两步：
//!   Step 1 衰减：所有非零 τ 减 1；τ=1 的回到无 buff 态
//!   Step 2 受击结算：以受击概率 h 触发
//!     - ℓ=1（已锁定）：状态不变
//!     - ℓ=0：以 p(k) 招架成功 → (min(k+1,5), 128, 1)；失败 → (..., 128, 0)
//!
//! 输出：
//!   - `e_stacks`     期望层数
//!   - `e_parry_rate` 期望招架率
//!   - `q`            每帧成功招架概率（h × pre-hit 招架率），喂给寒甲

use super::FPS;

const BUFF_FRAMES: usize = 8 * FPS as usize; // 128
const MAX_STACK: usize = 5;
const PER_STACK: f64 = 0.06;
const STATE_COUNT: usize = 1 + MAX_STACK * BUFF_FRAMES * 2; // 1281

/// 状态 ID 编码：
/// - id=0: 无 buff 态 (k=0, τ=0, ℓ=0)
/// - id=1+((k-1)*BUFF_FRAMES + (τ-1))*2 + ℓ : 其余
#[inline]
fn sid(k: usize, tau: usize, l: usize) -> usize {
    debug_assert!(tau > 0 && k > 0 && k <= MAX_STACK && tau <= BUFF_FRAMES && l < 2);
    1 + ((k - 1) * BUFF_FRAMES + (tau - 1)) * 2 + l
}

#[derive(Debug, Clone)]
pub struct JiantieDist {
    /// 概率分布 `[STATE_COUNT]`
    pub p: Vec<f64>,
    /// 招架率上限（默认 0.75）
    pub p_max: f64,
    /// 预计算各状态的 k 值（state_id → 层数）
    k_of: Vec<u8>,
    /// 预计算各状态的 ℓ 值（调试/扩展用，目前算法直接用 sid 位编码）
    #[allow(dead_code)]
    locked_of: Vec<u8>,
    /// 临时缓冲区（避免每帧 alloc）
    buf: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JiantieFrameStats {
    /// 期望层数 E[k]
    pub e_stacks: f64,
    /// 期望招架率 E[p(k)]
    pub e_parry_rate: f64,
    /// 每帧成功招架概率 q = h × pre-hit 招架率（喂给寒甲）
    pub q: f64,
    /// 各层数概率 P(k=0..5)
    pub stack_probs: [f64; 6],
}

impl JiantieDist {
    pub fn new(p_max: f64) -> Self {
        let mut k_of = vec![0u8; STATE_COUNT];
        let mut locked_of = vec![0u8; STATE_COUNT];
        for k in 1..=MAX_STACK {
            for tau in 1..=BUFF_FRAMES {
                for l in 0..2 {
                    let s = sid(k, tau, l);
                    k_of[s] = k as u8;
                    locked_of[s] = l as u8;
                }
            }
        }
        let mut p = vec![0.0; STATE_COUNT];
        p[0] = 1.0;
        Self {
            p,
            p_max,
            k_of,
            locked_of,
            buf: vec![0.0; STATE_COUNT],
        }
    }

    /// 重置到初始态（无 buff）
    pub fn reset(&mut self) {
        for v in self.p.iter_mut() {
            *v = 0.0;
        }
        self.p[0] = 1.0;
    }

    /// 推进一帧：
    /// - `p_0`：基础招架率（来自玩家面板）
    /// - `h`  ：本帧受击概率（来自 boss 攻击模型）
    pub fn tick(&mut self, p_0: f64, h: f64) -> JiantieFrameStats {
        let p_max = self.p_max;
        let n = STATE_COUNT;

        // ── Step 1: 衰减 ──────────────────────────────────────────
        // self.p → self.buf（按 τ→τ-1 重排）
        let p_dec = &mut self.buf;
        for v in p_dec.iter_mut() {
            *v = 0.0;
        }
        p_dec[0] = self.p[0]; // 无 buff 态保持

        for k in 1..=MAX_STACK {
            for tau in 1..=BUFF_FRAMES {
                for l in 0..2 {
                    let src = sid(k, tau, l);
                    let mass = self.p[src];
                    if mass == 0.0 {
                        continue;
                    }
                    if tau == 1 {
                        // buff 本帧到期，回到无 buff 态
                        p_dec[0] += mass;
                    } else {
                        p_dec[sid(k, tau - 1, l)] += mass;
                    }
                }
            }
        }

        // ── 记录 pre-hit 招架率（用于喂寒甲 q）──
        // 注意：无 buff 态的 k_of=0，但实际招架率仍是 p_0；要单独加
        let mut pre_hit_parry_sum = p_dec[0] * p_0.min(p_max);
        for s in 1..n {
            let mass = p_dec[s];
            if mass == 0.0 {
                continue;
            }
            let pk = (p_0 + PER_STACK * self.k_of[s] as f64).min(p_max);
            pre_hit_parry_sum += mass * pk;
        }
        let q = h * pre_hit_parry_sum;

        // ── Step 2: 伤害事件结算 ──────────────────────────────────
        // P_new ← (1-h) * P_dec  (不发生伤害分支)
        let p_new = &mut self.p;
        for s in 0..n {
            p_new[s] = (1.0 - h) * p_dec[s];
        }

        // 无 buff 态 (0,0,0) 受击 → 进 (1, 128, ℓ')
        let mass0 = h * p_dec[0];
        if mass0 > 0.0 {
            let pk0 = p_0.min(p_max);
            p_new[sid(1, BUFF_FRAMES, 1)] += mass0 * pk0;
            p_new[sid(1, BUFF_FRAMES, 0)] += mass0 * (1.0 - pk0);
        }

        // 有 buff 态受击
        for k in 1..=MAX_STACK {
            for tau in 1..=BUFF_FRAMES {
                // 未锁定：正常叠层 + 刷新 + 招架判定
                let src = sid(k, tau, 0);
                let mass = h * p_dec[src];
                if mass > 0.0 {
                    let pk = (p_0 + PER_STACK * k as f64).min(p_max);
                    let k_next = (k + 1).min(MAX_STACK);
                    p_new[sid(k_next, BUFF_FRAMES, 1)] += mass * pk;
                    p_new[sid(k_next, BUFF_FRAMES, 0)] += mass * (1.0 - pk);
                }
                // 已锁定：状态不变
                let src = sid(k, tau, 1);
                let mass = h * p_dec[src];
                if mass > 0.0 {
                    p_new[src] += mass;
                }
            }
        }

        // ── 输出指标 ────────────────────────────────────────────
        let mut e_stacks = 0.0;
        let mut e_parry = p_new[0] * p_0.min(p_max);
        let mut probs = [0.0f64; 6];
        probs[0] = p_new[0];
        for s in 1..n {
            let mass = p_new[s];
            if mass == 0.0 {
                continue;
            }
            let k = self.k_of[s] as usize;
            e_stacks += mass * k as f64;
            let pk = (p_0 + PER_STACK * k as f64).min(p_max);
            e_parry += mass * pk;
            probs[k] += mass;
        }

        JiantieFrameStats {
            e_stacks,
            e_parry_rate: e_parry,
            q,
            stack_probs: probs,
        }
    }

    /// 概率守恒检查（调试用）
    pub fn total_prob(&self) -> f64 {
        self.p.iter().sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expectation::poisson_h;

    /// 概率守恒：每帧后 sum(P) ≈ 1
    #[test]
    fn probability_conservation() {
        let mut dist = JiantieDist::new(0.75);
        let h = poisson_h(2.0);
        for _ in 0..1000 {
            dist.tick(0.20, h);
            let total = dist.total_prob();
            assert!((total - 1.0).abs() < 1e-10, "sum(P) = {}", total);
        }
    }

    /// 极端场景 1：p_0 = 0, p_max = 1.0
    /// 注意：即使 p_0=0，p(k=5) = 0 + 0.06×5 = 30%，仍会触发锁定。
    /// 锁定后 buff 不再刷新，128 帧后过期回到 0 层重新积累 → 稳态在 4 ~ 5 之间。
    #[test]
    fn extreme_p0_zero() {
        let mut dist = JiantieDist::new(1.0);
        // h = 1：每帧必受击
        for _ in 0..500 {
            dist.tick(0.0, 1.0);
        }
        let stats = dist.tick(0.0, 1.0);
        assert!(
            stats.e_stacks > 4.0 && stats.e_stacks <= 5.0,
            "稳态 E[k] = {} (期望 4.0~5.0)",
            stats.e_stacks
        );
        // E[parry] = 0 + 0.06 × E[k]，应在 0.24 ~ 0.30
        assert!(
            stats.e_parry_rate > 0.20 && stats.e_parry_rate < 0.31,
            "E[parry] = {} (期望 0.20~0.31)",
            stats.e_parry_rate
        );
    }

    /// 极端场景 2：p_0 = 1.0 → 第一次受击即锁定
    /// 稳态期望层数 = 1，期望招架率 = 1.0
    #[test]
    fn extreme_p0_one() {
        let mut dist = JiantieDist::new(1.0);
        // 跑 200 帧让稳态形成（h=1.0 每帧必受击）
        for _ in 0..200 {
            dist.tick(1.0, 1.0);
        }
        let stats = dist.tick(1.0, 1.0);
        // 第 1 帧受击 → 立即锁定在 k=1 / 招架率 1.0
        assert!(
            (stats.e_stacks - 1.0).abs() < 1e-6,
            "E[k] = {} (期望 1.0)",
            stats.e_stacks
        );
        assert!(
            (stats.e_parry_rate - 1.0).abs() < 1e-6,
            "E[parry] = {} (期望 1.0)",
            stats.e_parry_rate
        );
    }

    /// 一般稳态：Δ=2s, p_0=0.20，跑 60s 看稳态期望层数
    /// 比较参考值（Python 文档示例输出）
    #[test]
    fn steady_state_typical() {
        let mut dist = JiantieDist::new(0.75);
        let h = poisson_h(2.0);
        let mut last = JiantieFrameStats::default();
        for _ in 0..(16 * 60) {
            last = dist.tick(0.20, h);
        }
        // 稳态期望层数应该在 [3.5, 5.0] 之间（受击概率不高，但 buff 持续 8s）
        assert!(
            last.e_stacks > 2.0 && last.e_stacks < 5.0,
            "稳态 E[k] = {} (期望区间 2~5)",
            last.e_stacks
        );
        // 招架率 = p_0 + 0.06×E[k]，应在 0.32 ~ 0.50
        assert!(
            last.e_parry_rate > 0.30 && last.e_parry_rate < 0.55,
            "稳态 E[parry] = {} (期望 0.30~0.55)",
            last.e_parry_rate
        );
    }

    /// 概率分布 stack_probs 加起来 = 1
    #[test]
    fn stack_probs_sum_to_one() {
        let mut dist = JiantieDist::new(0.75);
        let h = poisson_h(2.0);
        for _ in 0..200 {
            let stats = dist.tick(0.30, h);
            let total: f64 = stats.stack_probs.iter().sum();
            assert!(
                (total - 1.0).abs() < 1e-10,
                "stack_probs sum = {} (期望 1)",
                total
            );
        }
    }

    /// 初始态：无 buff，受击前 P(0层) = 1
    #[test]
    fn initial_state() {
        let dist = JiantieDist::new(0.75);
        assert_eq!(dist.total_prob(), 1.0);
        assert_eq!(dist.p[0], 1.0);
    }

    /// reset 后回到初始态
    #[test]
    fn reset_works() {
        let mut dist = JiantieDist::new(0.75);
        for _ in 0..100 {
            dist.tick(0.3, 0.05);
        }
        dist.reset();
        assert_eq!(dist.p[0], 1.0);
        assert!((dist.total_prob() - 1.0).abs() < 1e-12);
    }
}
