//! 盾刀脚本 (ID: 13044)
//!
//! 每段命中给目标添加卷云（减速60%，8秒）
//! 秘籍5007：三段额外回复5点怒气
//! 判断方式：三段释放后无 grants_combo，combo "盾刀_2" 和 "盾刀_3" 都不存在

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 盾压 CD 期望重置：标记本帧施放了非盾压盾系技能
    player.last_cast_shield_non_dunya = true;

    // 每段命中：给目标添加/刷新卷云
    player.add_target_buff(BUFF_JUAN_YUN);

    // 秘籍：三段额外+5怒（三段后不再有 盾刀_2/3 combo buff）
    if player.has_recipe(5007) {
        // 如果刚释放的是三段：盾刀_2 不存在（二段授予的已被三段消耗）
        // 且盾刀_3 不存在（三段不授予新 combo）
        // 但一段后 盾刀_2 存在 → 排除
        let combo_2 = crate::combo_buff_id("盾刀_2");
        let combo_3 = crate::combo_buff_id("盾刀_3");
        if !player.has_buff(combo_2) && !player.has_buff(combo_3) {
            // 可能是一段或三段；检查一段刚授予的 combo_2 是否刚被添加
            // 一段 grants_combo = "盾刀_2"，所以一段后 combo_2 存在
            // 只有三段后两个都不存在
            // 实际上上面已经排除了一段（一段后 combo_2 存在）
            player.add_rage(5);
        }
    }
}
