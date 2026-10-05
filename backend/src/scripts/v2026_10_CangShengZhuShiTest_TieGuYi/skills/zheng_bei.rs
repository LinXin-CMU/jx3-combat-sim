//! 征北（40721）：自身威胁值提高100%，持续8秒。
use crate::{Player, ScriptEmitter};
use super::super::buffs::defs::BUFF_ZHENG_BEI;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_buff(BUFF_ZHENG_BEI);
}
