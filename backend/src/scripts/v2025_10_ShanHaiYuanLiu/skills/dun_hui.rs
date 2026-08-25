//! 盾回脚本 (ID: 13051)
//!
//! 1. 移除盾飞 buff（截断后续每跳）
//! 2. 移除盾飞延迟切换 buff（如果还在）
//! 3. 切换擎盾姿态（通过 TOML stance_change 已处理）
//! 4. 获得[坚定] buff（受伤-10%，8秒）

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 移除盾飞相关 buff
    player.remove_buff(BUFF_DUN_FEI);
    player.remove_buff(BUFF_DUN_FEI_DELAY);

    // 获得坚定（盾威奇穴：-25%、18s；否则 -10%、8s）
    if player.has_talent(13320) {
        // 18s = 288 帧；原 duration 128 帧，额外 +160 帧
        player.add_buff_extended(BUFF_JIAN_DING, 160);
    } else {
        player.add_buff(BUFF_JIAN_DING);
    }
}
