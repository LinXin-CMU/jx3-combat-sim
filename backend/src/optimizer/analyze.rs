//! 宏文本分析器：抽取可调阈值 (TunableParam)
//!
//! 扫描解析后的 MacroConfig，遍历每行条件树，找出形如 `key <op> NUMBER` 的
//! 数值比较，转换成一条 TunableParam。`=` / `~=` 视为"固定意图"，不作可调。

use serde::Serialize;

use crate::macro_engine::{CmpOp, MacroAction, MacroCondition, MacroLine};
use crate::macro_parser::parse_macro_text;
use crate::Stance;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct TunableParam {
    /// 唯一 id，形如 "shield_L3_V0"（页名 + 行号 + 访问序）
    pub id: String,
    pub page: String, // "general" / "shield" / "blade" / "wall" / "any" / "not_wall"
    pub line: usize,  // 1-based
    pub rule_preview: String,
    pub key: String, // "rage" / "life" / "bufftime:嗜血" / "tbufftime:虚弱" / "skill_energy:X" / "nearby_enemy"
    pub op: String,  // ">" "<" ">=" "<="
    pub original: f64,
    pub suggested_min: f64,
    pub suggested_max: f64,
    pub suggested_step: f64,
    pub kind: String, // "int" | "float"
    // ── 内部定位字段（GA 用来更新 MacroConfig；前端不关心） ──
    pub page_idx: usize,     // MacroConfig.pages 的索引
    pub line_idx: usize,     // MacroPage.lines 的索引（0-based）
    pub visit_idx: usize,    // 该行内可调 leaf 的访问序（0-based）
    pub leaf_kind: LeafKind, // Rage/Life/BuffTime/... 用于类型化写入
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum LeafKind {
    Rage,
    Life,
    BuffTime,
    TBuffTime,
    SkillEnergy,
    NearbyEnemy,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeResult {
    pub params: Vec<TunableParam>,
    pub warnings: Vec<String>,
}

/// 主入口：从宏文本抽取可调阈值
pub fn extract_tunables(macro_text: &str) -> Result<AnalyzeResult, String> {
    let cfg = parse_macro_text(macro_text).map_err(|e| e.to_string())?;

    let mut params = Vec::new();
    let warnings = Vec::new();

    for (page_idx, page) in cfg.pages.iter().enumerate() {
        let page_name = page_name(page.stance_filter);

        for (line_idx, line) in page.lines.iter().enumerate() {
            let line_no = line_idx + 1;
            let preview = format_line(line);

            if let Some(cond) = &line.condition {
                let mut leaves = Vec::new();
                walk_conditions(cond, &mut leaves);

                for (visit_idx, leaf) in leaves.iter().enumerate() {
                    match leaf.to_tunable(page_name, line_no, visit_idx, &preview) {
                        ToTunable::Ok(mut p) => {
                            p.page_idx = page_idx;
                            p.line_idx = line_idx;
                            params.push(p);
                        }
                        ToTunable::Skip => {}
                    }
                }
            }
        }
    }

    Ok(AnalyzeResult { params, warnings })
}

fn page_name(s: Option<Stance>) -> &'static str {
    match s {
        None => "general",
        Some(Stance::Shield) => "shield",
        Some(Stance::Blade) => "blade",
        Some(Stance::Wall) => "wall",
        Some(Stance::NotWall) => "not_wall",
        Some(Stance::Any) => "any",
    }
}

fn format_line(line: &MacroLine) -> String {
    let prefix = if line.action.is_fcast() {
        "/fcast"
    } else {
        "/cast"
    };
    let cond = line
        .condition
        .as_ref()
        .map(|c| format!(" [{}]", c.display_string()))
        .unwrap_or_default();
    let skill = match &line.action {
        MacroAction::Cast(n) | MacroAction::FCast(n) => n.as_str(),
    };
    format!("{}{} {}", prefix, cond, skill)
}

// ─────────────────────────────────────────────────────────────────────────────
// 条件树遍历
// ─────────────────────────────────────────────────────────────────────────────

enum Leaf {
    Rage(CmpOp, i32),
    Life(CmpOp, f64),
    BuffTime(String, CmpOp, f64),
    TBuffTime(String, CmpOp, f64),
    SkillEnergy(String, CmpOp, u32),
    NearbyEnemy(CmpOp, u32),
}

fn walk_conditions(cond: &MacroCondition, out: &mut Vec<Leaf>) {
    match cond {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            walk_conditions(a, out);
            walk_conditions(b, out);
        }
        MacroCondition::Rage(op, v) => out.push(Leaf::Rage(*op, *v)),
        MacroCondition::Life(op, v) => out.push(Leaf::Life(*op, *v)),
        MacroCondition::BuffTime(n, op, v) => out.push(Leaf::BuffTime(n.clone(), *op, *v)),
        MacroCondition::TBuffTime(n, op, v) => out.push(Leaf::TBuffTime(n.clone(), *op, *v)),
        MacroCondition::SkillEnergy(n, op, v) => out.push(Leaf::SkillEnergy(n.clone(), *op, *v)),
        MacroCondition::NearbyEnemy(op, v) => out.push(Leaf::NearbyEnemy(*op, *v)),
        // 这些不含可调数值：Buff/NoBuff/TBuff/TnoBuff/SkillNotInCd/SkillExists/SkillNotExists/LastSkill*
        _ => {}
    }
}

enum ToTunable {
    Ok(TunableParam),
    Skip,
}

impl Leaf {
    fn to_tunable(&self, page: &str, line: usize, vi: usize, preview: &str) -> ToTunable {
        let op = match self.op() {
            CmpOp::Gt | CmpOp::Lt | CmpOp::GtEq | CmpOp::LtEq => self.op().symbol().to_string(),
            CmpOp::Eq | CmpOp::Neq => return ToTunable::Skip, // "固定意图"
        };

        let id = format!("{}_L{}_V{}", page, line, vi);

        let (key, original, min, max, step, kind, leaf_kind) = match self {
            Leaf::Rage(_, v) => {
                let fv = *v as f64;
                let (mn, mx) = clamp_int_range(fv, 0.0, 100.0, 30.0);
                ("rage".to_string(), fv, mn, mx, 1.0, "int", LeafKind::Rage)
            }
            Leaf::Life(_, v) => {
                let (mn, mx) = clamp_int_range(*v, 0.0, 100.0, 30.0);
                ("life".to_string(), *v, mn, mx, 1.0, "int", LeafKind::Life)
            }
            Leaf::BuffTime(n, _, v) => {
                let mn = round_to_step((*v - 5.0).max(0.0), 0.1);
                let mx = round_to_step(*v + 5.0, 0.1);
                (
                    format!("bufftime:{}", n),
                    *v,
                    mn,
                    mx,
                    0.1,
                    "float",
                    LeafKind::BuffTime,
                )
            }
            Leaf::TBuffTime(n, _, v) => {
                let mn = round_to_step((*v - 5.0).max(0.0), 0.1);
                let mx = round_to_step(*v + 5.0, 0.1);
                (
                    format!("tbufftime:{}", n),
                    *v,
                    mn,
                    mx,
                    0.1,
                    "float",
                    LeafKind::TBuffTime,
                )
            }
            Leaf::SkillEnergy(n, _, v) => {
                let fv = *v as f64;
                let (mn, mx) = clamp_int_range(fv, 0.0, 10.0, 3.0);
                (
                    format!("skill_energy:{}", n),
                    fv,
                    mn,
                    mx,
                    1.0,
                    "int",
                    LeafKind::SkillEnergy,
                )
            }
            Leaf::NearbyEnemy(_, v) => {
                let fv = *v as f64;
                let (mn, mx) = clamp_int_range(fv, 0.0, 30.0, 5.0);
                (
                    "nearby_enemy".to_string(),
                    fv,
                    mn,
                    mx,
                    1.0,
                    "int",
                    LeafKind::NearbyEnemy,
                )
            }
        };

        ToTunable::Ok(TunableParam {
            id,
            page: page.to_string(),
            line,
            rule_preview: preview.to_string(),
            key,
            op,
            original,
            suggested_min: min,
            suggested_max: max,
            suggested_step: step,
            kind: kind.to_string(),
            page_idx: 0,
            line_idx: 0,
            visit_idx: vi,
            leaf_kind,
        })
    }

    fn op(&self) -> CmpOp {
        match self {
            Leaf::Rage(op, _)
            | Leaf::Life(op, _)
            | Leaf::BuffTime(_, op, _)
            | Leaf::TBuffTime(_, op, _)
            | Leaf::SkillEnergy(_, op, _)
            | Leaf::NearbyEnemy(op, _) => *op,
        }
    }
}

/// 整数范围裁剪到 [hard_min, hard_max]，以原值为中心左右扩展 spread
fn clamp_int_range(v: f64, hard_min: f64, hard_max: f64, spread: f64) -> (f64, f64) {
    let mn = (v - spread).max(hard_min);
    let mx = (v + spread).min(hard_max);
    (mn, mx)
}

/// 按步长对齐，消除 10.8-5.0=5.800000000000001 这类 f64 误差
pub fn round_to_step(v: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return v;
    }
    let scale = (1.0 / step).round();
    (v * scale).round() / scale
}

/// 把一组阈值写入 MacroConfig 对应位置（GA 每次评估都会调用）
/// `params[i]` 定位第 i 个阈值, `values[i]` 是要写入的新值
pub fn apply_values(
    cfg: &mut crate::macro_engine::MacroConfig,
    params: &[TunableParam],
    values: &[f64],
) {
    for (p, &v) in params.iter().zip(values.iter()) {
        if let Some(page) = cfg.pages.get_mut(p.page_idx) {
            if let Some(line) = page.lines.get_mut(p.line_idx) {
                if let Some(cond) = &mut line.condition {
                    let mut counter = 0usize;
                    set_nth_leaf(cond, p.visit_idx, &mut counter, v);
                }
            }
        }
    }
}

fn set_nth_leaf(cond: &mut MacroCondition, target: usize, counter: &mut usize, v: f64) -> bool {
    match cond {
        MacroCondition::And(a, b) | MacroCondition::Or(a, b) => {
            if set_nth_leaf(a, target, counter, v) {
                return true;
            }
            set_nth_leaf(b, target, counter, v)
        }
        MacroCondition::Rage(_, val) => {
            if *counter == target {
                *val = v.round() as i32;
                return true;
            }
            *counter += 1;
            false
        }
        MacroCondition::Life(_, val) => {
            if *counter == target {
                *val = v;
                return true;
            }
            *counter += 1;
            false
        }
        MacroCondition::BuffTime(_, _, val) | MacroCondition::TBuffTime(_, _, val) => {
            if *counter == target {
                *val = v;
                return true;
            }
            *counter += 1;
            false
        }
        MacroCondition::SkillEnergy(_, _, val) => {
            if *counter == target {
                *val = v.max(0.0).round() as u32;
                return true;
            }
            *counter += 1;
            false
        }
        MacroCondition::NearbyEnemy(_, val) => {
            if *counter == target {
                *val = v.max(0.0).round() as u32;
                return true;
            }
            *counter += 1;
            false
        }
        _ => false, // 非可调 leaf 不参与计数
    }
}
