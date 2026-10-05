//! 寒啸千军（15072）：消耗20格挡值，每95基础体质提高3无双等级，持续15秒。
use crate::{Player, ScriptEmitter, BUFF_HAN_XIAO};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_block_value(-20);
    super::super::apply_vitality_buff(player, BUFF_HAN_XIAO, player.current_stats().base_vitality, 15);
}
