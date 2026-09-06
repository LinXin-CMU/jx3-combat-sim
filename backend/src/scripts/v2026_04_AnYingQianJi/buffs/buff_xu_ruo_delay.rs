//! 虚弱添加延迟 Buff 事件脚本 — 暗影千机版
//!
//! on_expire: 0.125秒后给目标添加虚弱
//! level 1 (默认): -5%（走 BUFF_XU_RUO_DEF.effects 内置 -51，extra_effects 不填）
//! level 2 (戍边44566): -7%（extra_effects 填差额 -21，与 def 累加得 -72）

use crate::*;

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    let (lv, extra_diff) = if player.has_talent(44566) {
        (2u32, -21.0)
    } else {
        (1u32, 0.0)
    };
    player.add_target_buff((BUFF_XU_RUO, lv));
    if let Some(inst) = player
        .target_buffs
        .iter_mut()
        .find(|b| b.buff_id == BUFF_XU_RUO)
    {
        inst.extra_effects = if extra_diff != 0.0 {
            vec![EffectEntry {
                field: AttribField::TargetPhysicsShieldPercent,
                value: extra_diff,
            }]
        } else {
            Vec::new()
        };
    }
}
