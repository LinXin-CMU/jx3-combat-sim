//! 循环配置 JSON schema（与前端 buildLoopConfig / applyLoopConfig 对齐）
//!
//! 此类型用于 GA 的存档输出 + 未来"从前端一键启动"时的适应度输入。
//! 属性面板（Attributes）**不**包含在内——保持"循环不含属性"的约定。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const LOOP_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoopConfig {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported_at: Option<String>,
    pub target: LoopTarget,
    pub talents: LoopTalents,
    pub recipes: HashMap<String, Vec<u32>>,
    pub sequence: Vec<SequenceEntry>,
    #[serde(rename = "macro")]
    pub macro_pages: LoopMacro,
    #[serde(default)]
    pub network_delay: u32,
    #[serde(default)]
    pub initial_rage: Option<i32>,
    /// 可选：宏模拟时长上限（秒）。GA 归档的 LoopConfig 会带此字段，
    /// 前端导入时用 runMacroSimulate(duration) 而非 runSimulate，保证 DPS 与优化器一致
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macro_duration: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoopTarget {
    pub level: u32,
    #[serde(default)]
    pub defense_bonus: f64,
}

/// 奇穴选择：tier 1~7 单选 + mixed 多选
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoopTalents {
    #[serde(rename = "1", default, skip_serializing_if = "Option::is_none")]
    pub tier1: Option<u32>,
    #[serde(rename = "2", default, skip_serializing_if = "Option::is_none")]
    pub tier2: Option<u32>,
    #[serde(rename = "3", default, skip_serializing_if = "Option::is_none")]
    pub tier3: Option<u32>,
    #[serde(rename = "4", default, skip_serializing_if = "Option::is_none")]
    pub tier4: Option<u32>,
    #[serde(rename = "5", default, skip_serializing_if = "Option::is_none")]
    pub tier5: Option<u32>,
    #[serde(rename = "6", default, skip_serializing_if = "Option::is_none")]
    pub tier6: Option<u32>,
    #[serde(rename = "7", default, skip_serializing_if = "Option::is_none")]
    pub tier7: Option<u32>,
    #[serde(default)]
    pub mixed: Vec<u32>,
}

impl LoopTalents {
    /// 展平成 id 列表（GA 传给 Player 用）
    pub fn flat_ids(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        for t in [
            self.tier1, self.tier2, self.tier3, self.tier4, self.tier5, self.tier6, self.tier7,
        ] {
            if let Some(id) = t {
                ids.push(id);
            }
        }
        ids.extend(self.mixed.iter().copied());
        ids
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoopMacro {
    #[serde(default = "default_macro_mode")]
    pub mode: String, // "general" | "stance"
    #[serde(default)]
    pub general: String,
    #[serde(default)]
    pub shield: String,
    #[serde(default)]
    pub blade: String,
}

fn default_macro_mode() -> String {
    "general".into()
}

impl LoopMacro {
    /// 构建完整宏文本（general → 单页；stance → 用 #page 分隔 shield/blade）
    pub fn build_text(&self) -> String {
        if self.mode == "general" {
            return self.general.trim().to_string();
        }
        let mut s = String::new();
        if !self.shield.is_empty() {
            s.push_str("#page shield\n");
            s.push_str(&self.shield);
            s.push('\n');
        }
        if !self.blade.is_empty() {
            s.push_str("#page blade\n");
            s.push_str(&self.blade);
            s.push('\n');
        }
        s.trim().to_string()
    }
}

/// 序列条目（type 字段作为 serde tag）
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SequenceEntry {
    Skill {
        skill: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_ticks: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qijin_buff: Option<u32>,
    },
    Macro {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    WaitStance {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    Break {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
}

impl SequenceEntry {
    fn count(&self) -> u32 {
        let c = match self {
            SequenceEntry::Skill { count, .. } => *count,
            SequenceEntry::Macro { count } => *count,
            SequenceEntry::WaitStance { count } => *count,
            SequenceEntry::Break { count } => *count,
        };
        c.unwrap_or(1).max(1)
    }

    fn without_count(&self) -> SequenceEntry {
        match self {
            SequenceEntry::Skill {
                skill,
                channel_ticks,
                offset,
                qijin_buff,
                ..
            } => SequenceEntry::Skill {
                skill: skill.clone(),
                count: None,
                channel_ticks: *channel_ticks,
                offset: *offset,
                qijin_buff: *qijin_buff,
            },
            SequenceEntry::Macro { .. } => SequenceEntry::Macro { count: None },
            SequenceEntry::WaitStance { .. } => SequenceEntry::WaitStance { count: None },
            SequenceEntry::Break { .. } => SequenceEntry::Break { count: None },
        }
    }
}

/// 展开 `count: N` 为 N 份独立条目
pub fn expand_counts(entries: &[SequenceEntry]) -> Vec<SequenceEntry> {
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let n = e.count();
        let flat = e.without_count();
        for _ in 0..n {
            out.push(flat.clone());
        }
    }
    out
}
