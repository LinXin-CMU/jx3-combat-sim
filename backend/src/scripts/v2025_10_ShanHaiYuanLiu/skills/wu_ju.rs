//! 无惧脚本 (ID: 13042)
//!
//! 解除自身所有控制效果，免疫控制及恐惧6秒。
//! 仅铁骨衣心法可用。
//!
//! 奇穴 鸿烈 (26897)：触发 8 尺 AoE 外功伤害段 13042901（单目标模拟），CD -5 秒

use crate::{Player, ScriptEmitter, BUFF_WU_JU};

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 获得无惧 buff（免控 6 秒）
    player.add_buff(BUFF_WU_JU);

    if player.has_talent(26897) {
        // 鸿烈：额外 AoE 伤害段
        em.emit("鸿烈", 13042901, t);
        // 鸿烈：CD -5 秒（cast_skill 已插入 30s CD，这里缩减 5s）
        player.reduce_cd("cd_无惧", 5.0);
    }
}
