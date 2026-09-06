//! 寒甲 buff (8437) 期望传播 + A/B 双层数 carry-forward
//!
//! 核心难点：A/B 双 buff 的层数由**刷新瞬间**的拆招值决定，
//! 而非当前帧。如果状态加一维"刷新时刻"会爆炸。
//!
//! 解法：carry-forward —— 传播两个加权累计量 `A_carry(τ)` / `B_carry(τ)`
//!       与 `P(τ)` 并行，衰减时跟着左移，刷新时把当前帧的 (A,B) 注入 τ=192。
//!
//! API：
//! - `tick(q, cz)` 每帧调用，传入：
//!     - q   ：本帧成功招架概率（来自坚铁 `JiantieFrameStats.q`）
//!     - cz  ：本帧拆招值（动态，可受其他 buff 影响）
//! - 返回 `HanjiaFrameStats { p_alive, e_a, e_b }`

use super::FPS;

#[derive(Debug, Clone, Copy, Default)]
pub struct HanjiaFrameStats {
    /// 寒甲 buff 存活概率
    pub p_alive: f64,
    /// A 期望层数（×30000 攻击力/层）
    pub e_a: f64,
    /// B 期望层数（×300 攻击力/层）
    pub e_b: f64,
}

#[derive(Debug, Clone)]
pub struct HanjiaCarry {
    /// buff 持续帧数（默认 12s × 16fps = 192）
    duration_frames: usize,
    /// 各 τ 切片的存活概率，长度 duration_frames + 1
    p_tau: Vec<f64>,
    /// A_carry(τ) = P(τ) × E[A | τ]，长度 duration_frames + 1
    a_carry: Vec<f64>,
    /// B_carry(τ) = P(τ) × E[B | τ]，长度 duration_frames + 1
    b_carry: Vec<f64>,
    /// 临时缓冲区
    buf_p: Vec<f64>,
    buf_a: Vec<f64>,
    buf_b: Vec<f64>,
}

impl HanjiaCarry {
    /// 构造（duration_sec：buff 持续秒数；寒甲为 12.0）
    pub fn new(duration_sec: f64) -> Self {
        let n = (duration_sec * FPS as f64).round() as usize;
        let mut p_tau = vec![0.0; n + 1];
        p_tau[0] = 1.0; // 初始无 buff
        Self {
            duration_frames: n,
            p_tau,
            a_carry: vec![0.0; n + 1],
            b_carry: vec![0.0; n + 1],
            buf_p: vec![0.0; n + 1],
            buf_a: vec![0.0; n + 1],
            buf_b: vec![0.0; n + 1],
        }
    }

    pub fn reset(&mut self) {
        for v in self.p_tau.iter_mut() {
            *v = 0.0;
        }
        for v in self.a_carry.iter_mut() {
            *v = 0.0;
        }
        for v in self.b_carry.iter_mut() {
            *v = 0.0;
        }
        self.p_tau[0] = 1.0;
    }

    /// 推进一帧
    /// - `q`  ：本帧成功招架概率（喂自坚铁）
    /// - `cz` ：本帧拆招值（最终面板值，含动态 buff 加成）
    pub fn tick(&mut self, q: f64, cz: f64) -> HanjiaFrameStats {
        let n = self.duration_frames;
        let (a_now, b_now) = encode_hanjia(cz);

        // ── Step 1: 衰减（τ → τ-1，τ=0 保持） ──
        for v in self.buf_p.iter_mut() {
            *v = 0.0;
        }
        for v in self.buf_a.iter_mut() {
            *v = 0.0;
        }
        for v in self.buf_b.iter_mut() {
            *v = 0.0;
        }
        self.buf_p[0] = self.p_tau[0]; // τ=0 (无 buff) 保持
                                       // τ=1..n 的概率质量左移到 buf[0..n-1]
        for tau in 1..=n {
            self.buf_p[tau - 1] += self.p_tau[tau];
            self.buf_a[tau - 1] += self.a_carry[tau];
            self.buf_b[tau - 1] += self.b_carry[tau];
        }

        // ── Step 2: 刷新事件 ──
        // 不刷新分支 ×(1-q)
        let one_minus_q = 1.0 - q;
        for tau in 0..=n {
            self.p_tau[tau] = one_minus_q * self.buf_p[tau];
            self.a_carry[tau] = one_minus_q * self.buf_a[tau];
            self.b_carry[tau] = one_minus_q * self.buf_b[tau];
        }
        // 刷新分支：质量 collapse 到 τ=n（满），带本帧 (A, B)
        self.p_tau[n] += q;
        self.a_carry[n] += q * a_now as f64;
        self.b_carry[n] += q * b_now as f64;

        let p_alive = 1.0 - self.p_tau[0];
        let e_a: f64 = self.a_carry.iter().sum();
        let e_b: f64 = self.b_carry.iter().sum();

        HanjiaFrameStats { p_alive, e_a, e_b }
    }

    pub fn total_prob(&self) -> f64 {
        self.p_tau.iter().sum()
    }
}

/// 拆招值 → (A_stacks, B_stacks)
///
/// A：30000/层 高位（最大 125 层）
/// B：300/层  低位（最大 125 层；编码 raw=cz×7%，量化到 300 倍数后取余）
pub fn encode_hanjia(cz: f64) -> (u32, u32) {
    let raw = cz * 0.07;
    if raw < 0.0 {
        return (0, 0);
    }
    let quantized = ((raw / 300.0).round() * 300.0) as i64;
    let quantized = quantized.max(0);
    let a = (quantized / 30000).min(125) as u32;
    let b = ((quantized - a as i64 * 30000) / 300).min(125) as u32;
    (a, b)
}

// ─────────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expectation::poisson_h;

    /// 编码：500000 × 7% = 35000，量化到 35100 → A=1, B=17 (合计 35100)
    #[test]
    fn encode_basic() {
        let (a, b) = encode_hanjia(500000.0);
        assert_eq!(a, 1);
        assert_eq!(b, 17);
        assert_eq!(a * 30000 + b * 300, 35100);
    }

    /// 编码：700000 × 7% = 49000，量化到 48900 → A=1, B=63
    #[test]
    fn encode_large() {
        let (a, b) = encode_hanjia(700000.0);
        assert_eq!(a, 1);
        assert_eq!(b, 63);
        assert_eq!(a * 30000 + b * 300, 48900);
    }

    /// 边界：cz = 0 → (0, 0)
    #[test]
    fn encode_zero() {
        assert_eq!(encode_hanjia(0.0), (0, 0));
        assert_eq!(encode_hanjia(-100.0), (0, 0));
    }

    /// 概率守恒：P(τ=0..192) 之和 = 1
    #[test]
    fn probability_conservation() {
        let mut hj = HanjiaCarry::new(12.0);
        for _ in 0..2000 {
            hj.tick(0.05, 500000.0);
            let total = hj.total_prob();
            assert!((total - 1.0).abs() < 1e-10, "sum P(τ) = {}", total);
        }
    }

    /// 稳态：q 恒定时 P(alive) 收敛到 1 - (1-q)^N（N=BUFF_FRAMES）
    /// q=0.0204（坚铁文档算例），P(alive) ≈ 0.981
    #[test]
    fn steady_state_alive() {
        let mut hj = HanjiaCarry::new(12.0);
        let q = 0.0204;
        for _ in 0..1000 {
            hj.tick(q, 500000.0);
        }
        let stats = hj.tick(q, 500000.0);
        assert!(
            stats.p_alive > 0.97 && stats.p_alive < 0.99,
            "稳态 P(alive) = {} (期望 ~0.98)",
            stats.p_alive
        );
    }

    /// 稳态：基础 CZ=500000，E[B] ≈ 17 × P(alive) ≈ 16.7
    /// 文档算例
    #[test]
    fn steady_state_e_b_basic() {
        let mut hj = HanjiaCarry::new(12.0);
        let q = 0.0204;
        for _ in 0..1000 {
            hj.tick(q, 500000.0);
        }
        let stats = hj.tick(q, 500000.0);
        assert!(
            (stats.e_b - 16.7).abs() < 0.5,
            "E[B] = {} (期望 ~16.7)",
            stats.e_b
        );
        assert!(
            (stats.e_a - 0.98).abs() < 0.05,
            "E[A] = {} (期望 ~0.98)",
            stats.e_a
        );
    }

    /// 动态拆招值：t<20s 用 500000，t∈[20,40) 用 700000，t≥40 回 500000
    /// E[B] 应该指数过渡，不是瞬间跳变
    #[test]
    fn dynamic_cz_response() {
        let mut hj = HanjiaCarry::new(12.0);
        let q = 0.0204;
        let frames = |t_sec: f64| (t_sec * FPS as f64) as usize;

        // t=0..20s 稳态化
        for _ in 0..frames(20.0) {
            hj.tick(q, 500000.0);
        }
        let stats_pre_jump = hj.tick(q, 500000.0);
        let pre_b = stats_pre_jump.e_b;
        assert!((pre_b - 16.7).abs() < 0.5);

        // t=20..40s 用 700000，应渐变上升
        let mut last_stats = HanjiaFrameStats::default();
        for _ in 0..frames(20.0) {
            last_stats = hj.tick(q, 700000.0);
        }
        // 跳变后 20s 应该已基本稳态在 ~62
        assert!(
            (last_stats.e_b - 61.9).abs() < 1.5,
            "20s 后 E[B] = {} (期望 ~62)",
            last_stats.e_b
        );

        // 关键验证：跳变后立即不会瞬间跳到 62
        let mut hj2 = HanjiaCarry::new(12.0);
        for _ in 0..frames(20.0) {
            hj2.tick(q, 500000.0);
        }
        // 在 t=20s 那一帧就跳变 CZ=700000
        let immediately_after_jump = hj2.tick(q, 700000.0);
        // E[B] 应该接近跳变前的 16.7（一帧不可能立即收敛）
        assert!(
            immediately_after_jump.e_b < 25.0,
            "跳变后 1 帧 E[B] = {}（不应瞬间跳到 62）",
            immediately_after_jump.e_b
        );
    }

    /// reset 后回到初始态
    #[test]
    fn reset_works() {
        let mut hj = HanjiaCarry::new(12.0);
        for _ in 0..500 {
            hj.tick(0.05, 500000.0);
        }
        hj.reset();
        assert_eq!(hj.p_tau[0], 1.0);
        assert!((hj.total_prob() - 1.0).abs() < 1e-12);
    }

    /// 完整接入测试：与坚铁联动
    /// 验证 q 来自坚铁的 mitigation_weight，传给寒甲产出合理的 E_atk_bonus
    #[test]
    fn integration_with_jiantie() {
        use crate::expectation::JiantieDist;
        let mut jt = JiantieDist::new(0.75);
        let mut hj = HanjiaCarry::new(12.0);
        let h = poisson_h(2.0);
        let p_0 = 0.20;
        let cz = 500000.0;

        let mut last = HanjiaFrameStats::default();
        for _ in 0..(16 * 60) {
            let jt_stats = jt.tick(p_0, h);
            last = hj.tick(jt_stats.q, cz);
        }
        // 期望攻击力加成：E[A] × 30000 + E[B] × 300
        let atk_bonus = last.e_a * 30000.0 + last.e_b * 300.0;
        assert!(
            atk_bonus > 5000.0 && atk_bonus < 50000.0,
            "联动 E[atk_bonus] = {} (期望 5K~50K)",
            atk_bonus
        );
    }
}
