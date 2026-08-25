//! 月照连营 (ID: 30855) 阵云结晦二段
//! emit 破·月照连营

use crate::*;

pub fn cast_skill(_player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    em.emit("破·月照连营", 30855901, t);
}
