//! 盾壁（13070）— 铁骨衣生存技
//!
//! 主效果（模拟器不实现）：8秒吸收盾（每秒 +2%，上限 20%）
//! 奇穴关联：
//! - 返生 (18723) — 施展后立即 +20% HP + 30% 格挡值（仅格挡值参与模拟）

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 返生奇穴 18723：仅铁骨衣有格挡值资源，回复 max 的 30%
    // 基础 100 → +30；坚韧 13363 下 max=200 → +60
    if player.mount == Mount::TieGuYi && player.has_talent(18723) {
        let max_bv = player.max_block_value();
        let gain = max_bv * 30 / 100;
        player.add_block_value(gain);
    }
}
