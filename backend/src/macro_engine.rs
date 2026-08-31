//! 宏引擎数据结构

/// 宏整体配置（可含多页）
pub struct MacroConfig {
    pub pages: Vec<MacroPage>,
}

/// 单页宏（对应一个体态页）
pub struct MacroPage {
    pub stance_filter: Option<crate::Stance>,
    pub lines: Vec<MacroLine>,
}

/// 单行宏命令
pub struct MacroLine {
    pub condition: Option<MacroCondition>,
    pub action: MacroAction,
}

/// 宏动作
pub enum MacroAction {
    /// /cast — 标准释放，受 GCD 和引导约束
    Cast(String),
    /// /fcast — 强制释放，可打断引导
    FCast(String),
}

impl MacroAction {
    pub fn skill_name(&self) -> &str {
        match self {
            MacroAction::Cast(n) | MacroAction::FCast(n) => n,
        }
    }

    pub fn is_fcast(&self) -> bool {
        matches!(self, MacroAction::FCast(_))
    }
}

/// 条件表达式树
/// 运算符优先级：& 和 | 等优先级，右结合
/// A&B|C = A & (B | C)
#[derive(Clone)]
pub enum MacroCondition {
    // 自身资源
    Rage(CmpOp, i32),
    Life(CmpOp, f64),

    // 自身 buff
    Buff(String),
    NoBuff(String),
    BuffTime(String, CmpOp, f64),
    BuffStack(String, CmpOp, u32),

    // 目标 buff
    TBuff(String),
    TnoBuff(String),
    TBuffTime(String, CmpOp, f64),

    // 技能状态
    /// skill_notin_cd:X — 优先检查独立 CD（充能≥1视为无CD），再检查 GCD
    SkillNotInCd(String),
    /// skill:数字ID — 技能存在（奇穴已点出）
    SkillExists(u32),
    /// noskill:数字ID — 技能不存在
    SkillNotExists(u32),
    /// skill_energy:X>N — 充能层数
    SkillEnergy(String, CmpOp, u32),

    // last_skill
    LastSkill(String),
    LastSkillNot(String),

    // 环境
    NearbyEnemy(CmpOp, u32),

    // 组合
    And(Box<MacroCondition>, Box<MacroCondition>),
    Or(Box<MacroCondition>, Box<MacroCondition>),
}

impl MacroCondition {
    /// 收集本条件树里所有 bufftime / tbufftime 阈值，用于事件驱动跳转。
    /// 返回 (buff_name, threshold_sec, is_target)。
    pub fn collect_bufftime_thresholds(&self, out: &mut Vec<(String, f64, bool)>) {
        match self {
            MacroCondition::BuffTime(n, _, v) => out.push((n.clone(), *v, false)),
            MacroCondition::TBuffTime(n, _, v) => out.push((n.clone(), *v, true)),
            MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
                a.collect_bufftime_thresholds(out);
                b.collect_bufftime_thresholds(out);
            }
            _ => {}
        }
    }
}

impl MacroConfig {
    /// 扫整个宏所有 bufftime/tbufftime 阈值（去重）。
    pub fn bufftime_thresholds(&self) -> Vec<(String, f64, bool)> {
        let mut out = Vec::new();
        for page in &self.pages {
            for line in &page.lines {
                if let Some(ref cond) = line.condition {
                    cond.collect_bufftime_thresholds(&mut out);
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).then(a.2.cmp(&b.2)));
        out.dedup();
        out
    }
}

/// 比较运算符
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CmpOp {
    Gt,   // >
    Lt,   // <
    Eq,   // =
    GtEq, // >=
    LtEq, // <=
    Neq,  // ~=
}

impl CmpOp {
    pub fn compare_f64(self, lhs: f64, rhs: f64) -> bool {
        match self {
            CmpOp::Gt => lhs > rhs,
            CmpOp::Lt => lhs < rhs,
            CmpOp::Eq => (lhs - rhs).abs() < 0.001,
            CmpOp::GtEq => lhs >= rhs - 0.001,
            CmpOp::LtEq => lhs <= rhs + 0.001,
            CmpOp::Neq => (lhs - rhs).abs() >= 0.001,
        }
    }

    pub fn compare_i32(self, lhs: i32, rhs: i32) -> bool {
        match self {
            CmpOp::Gt => lhs > rhs,
            CmpOp::Lt => lhs < rhs,
            CmpOp::Eq => lhs == rhs,
            CmpOp::GtEq => lhs >= rhs,
            CmpOp::LtEq => lhs <= rhs,
            CmpOp::Neq => lhs != rhs,
        }
    }

    pub fn compare_u32(self, lhs: u32, rhs: u32) -> bool {
        match self {
            CmpOp::Gt => lhs > rhs,
            CmpOp::Lt => lhs < rhs,
            CmpOp::Eq => lhs == rhs,
            CmpOp::GtEq => lhs >= rhs,
            CmpOp::LtEq => lhs <= rhs,
            CmpOp::Neq => lhs != rhs,
        }
    }
}

impl CmpOp {
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Gt => ">", CmpOp::Lt => "<", CmpOp::Eq => "=",
            CmpOp::GtEq => ">=", CmpOp::LtEq => "<=", CmpOp::Neq => "~=",
        }
    }
}

impl MacroCondition {
    /// Fully parenthesized semantic projection of the AST.
    ///
    /// This is intentionally separate from `display_string`: game macro text
    /// has no grouping parentheses, while Agent/debug consumers need to see the
    /// exact tree produced by the right-associative parser.
    pub fn semantic_string(&self) -> String {
        match self {
            MacroCondition::And(a, b) => {
                format!("({} AND {})", a.semantic_string(), b.semantic_string())
            }
            MacroCondition::Or(a, b) => {
                format!("({} OR {})", a.semantic_string(), b.semantic_string())
            }
            _ => self.display_string(),
        }
    }

    pub fn display_string(&self) -> String {
        match self {
            MacroCondition::Rage(op, v) => format!("rage{}{}", op.symbol(), v),
            MacroCondition::Life(op, v) => format!("life{}{}", op.symbol(), v),
            MacroCondition::Buff(n) => format!("buff:{}", n),
            MacroCondition::NoBuff(n) => format!("nobuff:{}", n),
            MacroCondition::BuffTime(n, op, v) => format!("bufftime:{}{}{}", n, op.symbol(), v),
            MacroCondition::BuffStack(n, op, v) => format!("buff:{}{}{}", n, op.symbol(), v),
            MacroCondition::TBuff(n) => format!("tbuff:{}", n),
            MacroCondition::TnoBuff(n) => format!("tnobuff:{}", n),
            MacroCondition::TBuffTime(n, op, v) => format!("tbufftime:{}{}{}", n, op.symbol(), v),
            MacroCondition::SkillNotInCd(n) => format!("skill_notin_cd:{}", n),
            MacroCondition::SkillExists(id) => format!("skill:{}", id),
            MacroCondition::SkillNotExists(id) => format!("noskill:{}", id),
            MacroCondition::SkillEnergy(n, op, v) => format!("skill_energy:{}{}{}", n, op.symbol(), v),
            MacroCondition::LastSkill(n) => format!("last_skill={}", n),
            MacroCondition::LastSkillNot(n) => format!("last_skill~={}", n),
            MacroCondition::NearbyEnemy(op, v) => format!("nearby_enemy{}{}", op.symbol(), v),
            MacroCondition::And(a, b) => format!("{}&{}", a.display_string(), b.display_string()),
            MacroCondition::Or(a, b) => format!("{}|{}", a.display_string(), b.display_string()),
        }
    }
}

/// 解析错误
#[derive(Debug)]
pub struct MacroParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for MacroParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "第{}行: {}", self.line + 1, self.message)
    }
}
