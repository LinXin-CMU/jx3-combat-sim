//! 盾飞脚本 (ID: 13050)

use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 延迟0.125秒给目标添加虚弱（通过 buff expire 触发）
    player.add_buff(BUFF_XU_RUO_DELAY);

    // 延迟0.375秒切擎刀（通过 buff expire 触发）
    player.add_buff(BUFF_DUN_FEI_DELAY);

    // 盾飞 buff（持续伤害 + 秘籍加时长）
    let extra_frames: u32 =
        player.has_recipe(9005) as u32 * 80
      + player.has_recipe(9006) as u32 * 80;
    player.add_buff_extended(BUFF_DUN_FEI, extra_frames);

    // 首跳立即生效（skill_id=13463 与 buff_dun_fei::on_tick 保持一致，确保走每跳伤害 spec）
    em.emit("盾飞·每跳", 13463, t);

    // 奇穴锋鸣
    if player.has_talent(22897) {
        player.add_buff(BUFF_FENG_MING);
    }
}
