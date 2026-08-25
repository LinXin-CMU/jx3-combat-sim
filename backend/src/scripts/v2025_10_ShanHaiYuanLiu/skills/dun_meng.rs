//! 盾猛脚本 (ID: 13046)
//!
//! 奇穴 激昂 (13356)：施展后按最终体质叠激昂 buff (8418)
//!   层数 = floor(final_vitality / 3300)，max 100
//!   每层 +66 招架等级 +166 拆招值 +331 破招值，6 秒

use crate::{Player, ScriptEmitter, BUFF_JI_ANG};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 盾压 CD 期望重置：标记本帧施放了非盾压盾系技能
    player.last_cast_shield_non_dunya = true;

    if player.has_talent(13356) {
        let vitality = player.current_stats().vitality;
        let stacks = ((vitality / 3300.0).floor() as u32).min(100);
        if stacks > 0 {
            player.remove_buff(BUFF_JI_ANG);
            for _ in 0..stacks {
                player.add_buff(BUFF_JI_ANG);
            }
        }
    }
}
