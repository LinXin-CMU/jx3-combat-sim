//! 矢尽兵穷：公告已确认的15秒角斗场。
//! 当前单目标模拟假设自身与目标保持在场内；公告未给CD，暂不开放主动按钮。

use super::super::buffs::defs::BUFF_JIAO_DOU_CHANG;
use crate::{Player, ScriptEmitter};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    if player.has_talent(29066) {
        player.add_buff(BUFF_JIAO_DOU_CHANG);
    }
}
