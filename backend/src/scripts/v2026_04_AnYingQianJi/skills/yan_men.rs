//! 雁门迢递 (ID: 30856) 阵云结晦三段
//! emit 破·雁门迢递

use crate::*;

pub fn cast_skill(_player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    em.emit("破·雁门迢递", 30856901, t);
}
