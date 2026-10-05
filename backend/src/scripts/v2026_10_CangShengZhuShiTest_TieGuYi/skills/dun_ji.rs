//! 盾击脚本 (ID: 13047)
//!
//! 每次盾击命中降低盾飞 2 秒调息时间
//! 陷阵奇穴 (41834)：15s ICD 内连发两次"地坼"(41902，视为盾击)
//! 天下宏愿橙武装备：1024 制累计 prob=307，触发"盾击·神兵"(25780)

use super::ji_po_yuan_ge;
use crate::equip_effects::TIANXIA_HONGYUAN_WEAPON_IDS;
use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 盾压 CD 期望重置：标记本帧施放了非盾压盾系技能
    player.last_cast_shield_non_dunya = true;

    // 降低盾飞 2 秒调息时间
    player.reduce_charge_cd(13050, 2.0);

    // 援戈奇穴：触发击破·援戈
    if player.has_talent(36058) {
        ji_po_yuan_ge::cast_skill(player, em, t);
    }

    // 陷阵奇穴：盾击触发两次"地坼"；15s 内置 CD 门控
    if player.has_talent(41834) && !player.has_buff(BUFF_XIAN_ZHEN_CD) {
        em.emit("地坼", 41902, t);
        em.emit("地坼", 41902, t);
        player.add_buff(BUFF_XIAN_ZHEN_CD);
    }

    // 天下宏愿装备特效：盾击·神兵 期望累计触发（prob=307/1024 ≈ 30%）
    if player.has_equip_in("PRIMARY_WEAPON", TIANXIA_HONGYUAN_WEAPON_IDS)
        && player.accum_equip_effect(25780, 307, 1024)
    {
        em.emit("盾击·神兵", 25780, t);
    }
}
