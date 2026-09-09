//! 偃守孤旌（41982）：施加战绝 buff（擎盾释放，脚本切换擎刀由 stance_change 处理）

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_buff(BUFF_ZHAN_JUE);
    // 首跳立即触发：立即回100怒（后续 t=3,6,9 由 buff on_tick 处理，共4次×100=400怒）
    player.add_rage_from(100, "偃守孤旌回怒");
}
