//! 宏文本解析器
//!
//! 格式: /cast [条件] 技能名  或  /fcast [条件] 技能名
//! 条件可省略中括号: /cast rage>49 绝刀
//! 多页用 #page shield / #page blade 分隔

use crate::macro_engine::*;
use crate::Stance;

/// 解析完整宏文本（可含多页）
pub fn parse_macro_text(text: &str) -> Result<MacroConfig, MacroParseError> {
    let mut pages = Vec::new();
    let mut current_lines = Vec::new();
    let mut current_stance: Option<Stance> = None;
    let mut global_line = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();

        // 空行和注释跳过
        if trimmed.is_empty() || trimmed.starts_with("//") {
            global_line += 1;
            continue;
        }

        // 页分隔符: #page shield / #page blade / #page
        if trimmed.starts_with("#page") {
            // 保存当前页
            if !current_lines.is_empty() {
                pages.push(MacroPage {
                    stance_filter: current_stance,
                    lines: current_lines,
                });
                current_lines = Vec::new();
            }
            // 解析新页体态
            let rest = trimmed["#page".len()..].trim();
            current_stance = match rest {
                "shield" | "擎盾" => Some(Stance::Shield),
                "blade" | "擎刀" => Some(Stance::Blade),
                "wall" | "盾墙" => Some(Stance::Wall),
                "" => None,
                _ => {
                    return Err(MacroParseError {
                        line: global_line,
                        message: format!("未知体态: {}", rest),
                    })
                }
            };
            global_line += 1;
            continue;
        }

        // 解析宏行
        if let Some(ml) = parse_line(trimmed, global_line)? {
            current_lines.push(ml);
        }
        global_line += 1;
    }

    // 最后一页
    if !current_lines.is_empty() {
        pages.push(MacroPage {
            stance_filter: current_stance,
            lines: current_lines,
        });
    }

    if pages.is_empty() {
        return Err(MacroParseError {
            line: 0,
            message: "宏文本为空".into(),
        });
    }

    Ok(MacroConfig { pages })
}

/// 解析单行宏命令
fn parse_line(line: &str, line_num: usize) -> Result<Option<MacroLine>, MacroParseError> {
    let err = |msg: &str| MacroParseError {
        line: line_num,
        message: msg.to_string(),
    };

    // 匹配 /cast 或 /fcast
    let (is_fcast, rest) = if line.starts_with("/fcast") {
        (true, line["/fcast".len()..].trim_start())
    } else if line.starts_with("/cast") {
        (false, line["/cast".len()..].trim_start())
    } else {
        return Err(err("行必须以 /cast 或 /fcast 开头"));
    };

    if rest.is_empty() {
        return Err(err("缺少技能名"));
    }

    // 提取条件和技能名
    let (condition, skill_name) = if rest.starts_with('[') {
        // [条件] 技能名
        let end = rest.find(']').ok_or_else(|| err("缺少 ]"))?;
        let cond_str = &rest[1..end];
        let name = rest[end + 1..].trim();
        if name.is_empty() {
            return Err(err("缺少技能名"));
        }
        let cond = parse_condition(cond_str, line_num)?;
        (Some(cond), name.to_string())
    } else {
        // 可能是 条件 技能名（空格分隔）或 纯技能名
        // 尝试找最后一个空格，前面是条件后面是技能名
        // 但条件中也可能有空格... 简化：如果包含条件关键字就解析条件
        let has_condition = rest.contains('>')
            || rest.contains('<')
            || rest.contains("bufftime:")
            || rest.contains("tbufftime:")
            || rest.contains("buff:")
            || rest.contains("nobuff:")
            || rest.contains("tbuff:")
            || rest.contains("tnobuff:")
            || rest.contains("skill_notin_cd:")
            || rest.contains("skill_energy:")
            || rest.contains("last_skill")
            || rest.contains("skill:")
            || rest.contains("noskill:")
            || rest.contains("life")
            || rest.contains("rage")
            || rest.contains("nearby_enemy");

        if has_condition {
            // 最后一个空格分隔条件和技能名
            let last_space = rest
                .rfind(' ')
                .ok_or_else(|| err("条件和技能名之间需要空格"))?;
            let cond_str = &rest[..last_space];
            let name = rest[last_space + 1..].trim();
            if name.is_empty() {
                return Err(err("缺少技能名"));
            }
            let cond = parse_condition(cond_str, line_num)?;
            (Some(cond), name.to_string())
        } else {
            // 纯技能名
            (None, rest.to_string())
        }
    };

    let action = if is_fcast {
        MacroAction::FCast(skill_name)
    } else {
        MacroAction::Cast(skill_name)
    };

    Ok(Some(MacroLine { condition, action }))
}

// ── 条件解析器（递归下降，& | 等优先级右结合）──

/// 条件 token
#[derive(Debug, Clone)]
enum CondToken {
    And,
    Or,
    Atom(String), // 原子条件字符串
}

/// 分词：按 & 和 | 切割，保留原子条件
fn tokenize_condition(input: &str) -> Vec<CondToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        match ch {
            '&' => {
                let s = current.trim().to_string();
                if !s.is_empty() {
                    tokens.push(CondToken::Atom(s));
                }
                current.clear();
                tokens.push(CondToken::And);
            }
            '|' => {
                let s = current.trim().to_string();
                if !s.is_empty() {
                    tokens.push(CondToken::Atom(s));
                }
                current.clear();
                tokens.push(CondToken::Or);
            }
            _ => current.push(ch),
        }
    }
    let s = current.trim().to_string();
    if !s.is_empty() {
        tokens.push(CondToken::Atom(s));
    }
    tokens
}

/// 解析条件表达式
fn parse_condition(input: &str, line_num: usize) -> Result<MacroCondition, MacroParseError> {
    let tokens = tokenize_condition(input);
    if tokens.is_empty() {
        return Err(MacroParseError {
            line: line_num,
            message: "空条件".into(),
        });
    }
    let (cond, rest) = parse_expr(&tokens, line_num)?;
    if !rest.is_empty() {
        return Err(MacroParseError {
            line: line_num,
            message: "条件解析未完成".into(),
        });
    }
    Ok(cond)
}

/// 递归下降：expr = primary ( ('&'|'|') expr )?
fn parse_expr<'a>(
    tokens: &'a [CondToken],
    line_num: usize,
) -> Result<(MacroCondition, &'a [CondToken]), MacroParseError> {
    let (left, rest) = parse_primary(tokens, line_num)?;
    match rest.first() {
        Some(CondToken::And) => {
            let (right, rest2) = parse_expr(&rest[1..], line_num)?;
            Ok((MacroCondition::And(Box::new(left), Box::new(right)), rest2))
        }
        Some(CondToken::Or) => {
            let (right, rest2) = parse_expr(&rest[1..], line_num)?;
            Ok((MacroCondition::Or(Box::new(left), Box::new(right)), rest2))
        }
        _ => Ok((left, rest)),
    }
}

/// 解析原子条件
fn parse_primary<'a>(
    tokens: &'a [CondToken],
    line_num: usize,
) -> Result<(MacroCondition, &'a [CondToken]), MacroParseError> {
    let err = |msg: &str| MacroParseError {
        line: line_num,
        message: msg.to_string(),
    };

    match tokens.first() {
        Some(CondToken::Atom(s)) => {
            let cond = parse_atom(s, line_num)?;
            Ok((cond, &tokens[1..]))
        }
        _ => Err(err("期望条件表达式")),
    }
}

/// 解析单个原子条件字符串
fn parse_atom(s: &str, line_num: usize) -> Result<MacroCondition, MacroParseError> {
    let err = |msg: String| MacroParseError {
        line: line_num,
        message: msg,
    };

    // rage>N / rage<N / rage=N / rage>=N / rage<=N
    if s.starts_with("rage") {
        let (op, val) = parse_cmp_i32(&s[4..], line_num)?;
        return Ok(MacroCondition::Rage(op, val));
    }

    // life>N / life<N
    if s.starts_with("life") {
        let (op, val) = parse_cmp_f64(&s[4..], line_num)?;
        return Ok(MacroCondition::Life(op, val));
    }

    // nearby_enemy>N
    if s.starts_with("nearby_enemy") {
        let (op, val) = parse_cmp_i32(&s[12..], line_num)?;
        return Ok(MacroCondition::NearbyEnemy(op, val as u32));
    }

    // bufftime:名字>N / bufftime:名字<N
    if s.starts_with("bufftime:") {
        let rest = &s[9..];
        let (name, op, val) = parse_name_cmp_f64(rest, line_num)?;
        return Ok(MacroCondition::BuffTime(name, op, val));
    }

    // tbufftime:名字>N
    if s.starts_with("tbufftime:") {
        let rest = &s[10..];
        let (name, op, val) = parse_name_cmp_f64(rest, line_num)?;
        return Ok(MacroCondition::TBuffTime(name, op, val));
    }

    // nobuff:名字
    if s.starts_with("nobuff:") {
        return Ok(MacroCondition::NoBuff(s[7..].trim().to_string()));
    }

    // buff:名字=N / buff:名字>N / buff:名字<N（层数检查）
    // buff:名字（存在检查）
    if s.starts_with("buff:") {
        let rest = &s[5..];
        if rest.contains(|c: char| c == '>' || c == '<' || c == '=') {
            let (name, op, val) = parse_name_cmp_f64(rest, line_num)?;
            return Ok(MacroCondition::BuffStack(name, op, val as u32));
        }
        return Ok(MacroCondition::Buff(rest.trim().to_string()));
    }

    // tnobuff:名字
    if s.starts_with("tnobuff:") {
        return Ok(MacroCondition::TnoBuff(s[8..].trim().to_string()));
    }

    // tbuff:名字
    if s.starts_with("tbuff:") {
        return Ok(MacroCondition::TBuff(s[6..].trim().to_string()));
    }

    // skill_notin_cd:名字
    if s.starts_with("skill_notin_cd:") {
        return Ok(MacroCondition::SkillNotInCd(s[15..].trim().to_string()));
    }

    // skill_energy:名字>N
    if s.starts_with("skill_energy:") {
        let rest = &s[13..];
        let (name, op, val) = parse_name_cmp_f64(rest, line_num)?;
        return Ok(MacroCondition::SkillEnergy(name, op, val as u32));
    }

    // noskill:数字ID
    if s.starts_with("noskill:") {
        let id: u32 = s[8..]
            .trim()
            .parse()
            .map_err(|_| err(format!("noskill ID 不是数字: {}", &s[8..])))?;
        return Ok(MacroCondition::SkillNotExists(id));
    }

    // skill:数字ID
    if s.starts_with("skill:") {
        let id: u32 = s[6..]
            .trim()
            .parse()
            .map_err(|_| err(format!("skill ID 不是数字: {}", &s[6..])))?;
        return Ok(MacroCondition::SkillExists(id));
    }

    // last_skill~=名字
    if s.starts_with("last_skill~=") {
        return Ok(MacroCondition::LastSkillNot(s[12..].trim().to_string()));
    }

    // last_skill=名字
    if s.starts_with("last_skill=") {
        return Ok(MacroCondition::LastSkill(s[11..].trim().to_string()));
    }

    Err(err(format!("无法识别的条件: {}", s)))
}

/// 解析比较运算符 + 整数值，如 ">49"
fn parse_cmp_i32(s: &str, line_num: usize) -> Result<(CmpOp, i32), MacroParseError> {
    let err = |msg: String| MacroParseError {
        line: line_num,
        message: msg,
    };
    let (op, rest) = parse_cmp_op(s, line_num)?;
    let val: i32 = rest
        .trim()
        .parse()
        .map_err(|_| err(format!("无法解析数值: {}", rest)))?;
    Ok((op, val))
}

/// 解析比较运算符 + 浮点值
fn parse_cmp_f64(s: &str, line_num: usize) -> Result<(CmpOp, f64), MacroParseError> {
    let err = |msg: String| MacroParseError {
        line: line_num,
        message: msg,
    };
    let (op, rest) = parse_cmp_op(s, line_num)?;
    let val: f64 = rest
        .trim()
        .parse()
        .map_err(|_| err(format!("无法解析数值: {}", rest)))?;
    Ok((op, val))
}

/// 解析 "名字>数值" 格式
fn parse_name_cmp_f64(s: &str, line_num: usize) -> Result<(String, CmpOp, f64), MacroParseError> {
    let err = |msg: String| MacroParseError {
        line: line_num,
        message: msg,
    };
    // 找第一个比较运算符位置
    let pos = s
        .find(|c: char| c == '>' || c == '<' || c == '=')
        .ok_or_else(|| err("缺少比较运算符".into()))?;
    let name = s[..pos].trim().to_string();
    let (op, rest) = parse_cmp_op(&s[pos..], line_num)?;
    let val: f64 = rest
        .trim()
        .parse()
        .map_err(|_| err(format!("无法解析数值: {}", rest)))?;
    Ok((name, op, val))
}

/// 解析比较运算符，返回 (运算符, 剩余字符串)
fn parse_cmp_op(s: &str, line_num: usize) -> Result<(CmpOp, &str), MacroParseError> {
    let err = || MacroParseError {
        line: line_num,
        message: "无法解析比较运算符".into(),
    };
    if s.starts_with(">=") {
        return Ok((CmpOp::GtEq, &s[2..]));
    }
    if s.starts_with("<=") {
        return Ok((CmpOp::LtEq, &s[2..]));
    }
    if s.starts_with("~=") {
        return Ok((CmpOp::Neq, &s[2..]));
    }
    if s.starts_with('>') {
        return Ok((CmpOp::Gt, &s[1..]));
    }
    if s.starts_with('<') {
        return Ok((CmpOp::Lt, &s[1..]));
    }
    if s.starts_with('=') {
        return Ok((CmpOp::Eq, &s[1..]));
    }
    Err(err())
}

// ─────────────────────────────────────────────────────────────────────────────
// 渲染器：MacroConfig → 文本（parse_macro_text 的逆操作）
// ─────────────────────────────────────────────────────────────────────────────

pub fn render_macro_text(cfg: &MacroConfig) -> String {
    let mut out = String::new();
    for (i, page) in cfg.pages.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if let Some(stance) = &page.stance_filter {
            let s = match stance {
                Stance::Shield => "shield",
                Stance::Blade => "blade",
                Stance::Wall => "wall",
                _ => "",
            };
            if !s.is_empty() {
                out.push_str(&format!("#page {}\n", s));
            } else {
                out.push_str("#page\n");
            }
        } else if i > 0 {
            // A later unfiltered page still needs an explicit delimiter.  Without
            // it, render -> parse merges this page into the preceding stance page.
            out.push_str("#page\n");
        }
        for line in &page.lines {
            let cmd = if line.action.is_fcast() {
                "/fcast"
            } else {
                "/cast"
            };
            let skill = line.action.skill_name();
            match &line.condition {
                Some(c) => out.push_str(&format!("{} {} {}\n", cmd, c.display_string(), skill)),
                None => out.push_str(&format!("{} {}\n", cmd, skill)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_cast() {
        let config = parse_macro_text("/cast 盾击").unwrap();
        assert_eq!(config.pages.len(), 1);
        assert_eq!(config.pages[0].lines.len(), 1);
        assert!(config.pages[0].lines[0].condition.is_none());
        assert_eq!(config.pages[0].lines[0].action.skill_name(), "盾击");
    }

    #[test]
    fn test_condition_rage() {
        let config = parse_macro_text("/cast [rage>49] 绝刀").unwrap();
        let cond = config.pages[0].lines[0].condition.as_ref().unwrap();
        match cond {
            MacroCondition::Rage(CmpOp::Gt, 49) => {}
            _ => panic!("expected Rage(Gt, 49), got {:?}", "other"),
        }
    }

    #[test]
    fn test_right_associative() {
        // A&B|C = A & (B | C)
        let config = parse_macro_text("/cast [rage>49&buff:血怒|nobuff:坚定] 绝刀").unwrap();
        let cond = config.pages[0].lines[0].condition.as_ref().unwrap();
        match cond {
            MacroCondition::And(a, bc) => {
                match a.as_ref() {
                    MacroCondition::Rage(CmpOp::Gt, 49) => {}
                    _ => panic!("expected Rage"),
                }
                match bc.as_ref() {
                    MacroCondition::Or(b, c) => {
                        match b.as_ref() {
                            MacroCondition::Buff(name) if name == "血怒" => {}
                            _ => panic!("expected Buff(血怒)"),
                        }
                        match c.as_ref() {
                            MacroCondition::NoBuff(name) if name == "坚定" => {}
                            _ => panic!("expected NoBuff(坚定)"),
                        }
                    }
                    _ => panic!("expected Or"),
                }
            }
            _ => panic!("expected And"),
        }
    }

    #[test]
    fn current_general_macro_exposes_exact_complex_grouping() {
        let text = concat!(
            "/cast [buff:盾飞&nobuff:血怒·惊涌&buff:麟光甲=9|bufftime:狂绝<4.7&buff:麟光甲|skill_energy:血怒>1] 血怒\n",
            "/cast [buff:天下宏愿|rage>64&bufftime:嗜血<5.3|nobuff:嗜血] 盾飞\n",
            "/cast [skill_energy:阵云结晦=2|nobuff:麟光甲&bufftime:嗜血>8] 阵云结晦"
        );
        let config = parse_macro_text(text).unwrap();
        let semantics = config.pages[0]
            .lines
            .iter()
            .map(|line| line.condition.as_ref().unwrap().semantic_string())
            .collect::<Vec<_>>();

        assert_eq!(
            semantics[0],
            "(buff:盾飞 AND (nobuff:血怒·惊涌 AND (buff:麟光甲=9 OR (bufftime:狂绝<4.7 AND (buff:麟光甲 OR skill_energy:血怒>1)))))"
        );
        assert_eq!(
            semantics[1],
            "(buff:天下宏愿 OR (rage>64 AND (bufftime:嗜血<5.3 OR nobuff:嗜血)))"
        );
        assert_eq!(
            semantics[2],
            "(skill_energy:阵云结晦=2 OR (nobuff:麟光甲 AND bufftime:嗜血>8))"
        );
    }

    #[test]
    fn test_multi_page() {
        let text = "#page shield\n/cast 盾击\n#page blade\n/cast 斩刀";
        let config = parse_macro_text(text).unwrap();
        assert_eq!(config.pages.len(), 2);
        assert_eq!(config.pages[0].stance_filter, Some(Stance::Shield));
        assert_eq!(config.pages[1].stance_filter, Some(Stance::Blade));
    }

    #[test]
    fn test_fcast() {
        let config = parse_macro_text("/fcast [buff:血怒] 血怒").unwrap();
        assert!(config.pages[0].lines[0].action.is_fcast());
    }

    #[test]
    fn test_no_bracket_condition() {
        let config = parse_macro_text("/cast rage>49 绝刀").unwrap();
        assert!(config.pages[0].lines[0].condition.is_some());
        assert_eq!(config.pages[0].lines[0].action.skill_name(), "绝刀");
    }

    #[test]
    fn rendered_skill_energy_equality_round_trips_as_a_condition() {
        let config = parse_macro_text("/cast [skill_energy:阵云结晦=2] 阵云结晦").unwrap();
        let rendered = render_macro_text(&config);
        let reparsed = parse_macro_text(&rendered).unwrap();
        let line = &reparsed.pages[0].lines[0];
        assert_eq!(line.action.skill_name(), "阵云结晦");
        assert!(matches!(
            line.condition,
            Some(MacroCondition::SkillEnergy(ref name, CmpOp::Eq, 2)) if name == "阵云结晦"
        ));
    }

    #[test]
    fn rendered_later_unfiltered_page_keeps_its_boundary() {
        let text = "#page shield\n/cast 盾击\n#page\n/cast 绝刀\n/cast 斩刀";
        let config = parse_macro_text(text).unwrap();
        assert_eq!(config.pages.len(), 2);

        let rendered = render_macro_text(&config);
        let reparsed = parse_macro_text(&rendered).unwrap();
        assert_eq!(reparsed.pages.len(), 2);
        assert_eq!(reparsed.pages[0].stance_filter, Some(Stance::Shield));
        assert_eq!(reparsed.pages[1].stance_filter, None);
        assert_eq!(reparsed.pages[1].lines[0].action.skill_name(), "绝刀");
    }
}
