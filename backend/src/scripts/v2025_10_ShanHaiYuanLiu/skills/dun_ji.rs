//! 盾击脚本 (ID: 13047)
//!
//! 每次盾击命中降低盾飞 2 秒调息时间

use crate::*;
use super::ji_po_yuan_ge;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 盾压 CD 期望重置：标记本帧施放了非盾压盾系技能
    player.last_cast_shield_non_dunya = true;

    // 降低盾飞 2 秒调息时间
    player.reduce_charge_cd(13050, 2.0);

    // 援戈奇穴：触发击破·援戈
    if player.has_talent(36058) {
        ji_po_yuan_ge::cast_skill(player, em, t);
    }
}
