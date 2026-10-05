//! 盾猛（13046 / 25204）：激昂使用最终体质，持续15秒。
use crate::{Player, ScriptEmitter};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.last_cast_shield_non_dunya = true;
    super::super::apply_ji_ang(player);
}
