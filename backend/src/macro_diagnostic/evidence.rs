//! Read-only target probes at the executor's decision state. Never add to its pool.
use super::*;

#[derive(Debug, Serialize)]
pub struct AtomEvidence {
    pub condition: String,
    pub actual: String,
    pub passed: bool,
}

#[derive(Debug, Serialize)]
pub struct TargetLine {
    pub line: usize,
    pub condition: String,
    pub passed: bool,
    pub atoms: Vec<AtomEvidence>,
    pub truncated: bool,
    pub probe: Phase2EntryDebug,
}

#[derive(Debug, Serialize)]
pub struct TargetEvidence {
    pub skill: String,
    pub lines: Vec<TargetLine>,
    pub other_pages: Vec<usize>,
    pub probe: Phase2EntryDebug,
}

fn skill_group<'a>(name: &str, map: &'a HashMap<&str, Vec<&'a SkillSpec>>, ids: &'a HashMap<u32, &'a SkillSpec>) -> Option<&'a Vec<&'a SkillSpec>> {
    map.get(name).or_else(|| name.parse::<u32>().ok().and_then(|id| ids.get(&id))
        .and_then(|spec| map.get(spec.name.split('·').next().unwrap_or(&spec.name))))
}

fn same_action(a: &str, b: &str, map: &HashMap<&str, Vec<&SkillSpec>>, ids: &HashMap<u32, &SkillSpec>) -> bool {
    a == b || match (skill_group(a, map, ids), skill_group(b, map, ids)) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    }
}

fn value(cond: &MacroCondition, state: &Phase1State) -> String {
    use MacroCondition::*;
    match cond {
        Rage(..) => format!("怒气 {}", state.player.rage),
        Energy(..) => format!("格挡 {}", state.player.block_value),
        Berserk(..) => format!("暴怒 {}", state.player.berserk_value),
        Life(..) => "血量比例 1（模拟满血）".into(),
        NearbyEnemy(..) => "附近目标 1（模拟设定）".into(),
        LastSkill(..) | LastSkillNot(..) => format!("上次成功技能：{}", state.last_skill.as_deref().unwrap_or("无")),
        SkillExists(id) | SkillNotExists(id) => format!("奇穴 {}：{}", id, if state.player.has_talent(*id) {"已选"} else {"未选"}),
        SkillEnergy(name, ..) | SkillNotInCd(name) => {
            match skill_group(name, state.skill_map, state.skill_by_id).and_then(|group| group.first()) {
                Some(skill) if matches!(cond, SkillEnergy(..)) => format!("{} 充能 {}", name, state.player.get_charge_count(skill)),
                Some(skill) => format!("{}：{}", name, if state.player.is_skill_not_in_cd(skill) {"无冷却"} else {"冷却中"}),
                None => format!("未知技能 {}", name),
            }
        }
        Buff(name) | NoBuff(name) | BuffTime(name, ..) | BuffStack(name, ..) | TBuff(name) | TnoBuff(name) | TBuffTime(name, ..) => {
            let target = matches!(cond, TBuff(..) | TnoBuff(..) | TBuffTime(..));
            let lookup = if target { &state.target_buff_lookup } else { &state.buff_lookup };
            let buffs = if target { &state.player.target_buffs } else { &state.player.active_buffs };
            let Some(id) = buff_name_to_id(name) else { return format!("未知气劲 {}", name) };
            match lookup.get(&id).map(|&index| &buffs[index]) {
                None => format!("{} 不存在", name),
                Some(buff) => format!("{} · {} 层 · {}", name, buff.stacks, if buff.expires_at == 0.0 {"永久".into()} else {format!("剩余 {:.3}s", (buff.expires_at - state.player.current_time).max(0.0))}),
            }
        }
        And(..) | Or(..) => unreachable!(),
    }
}

fn atoms(cond: &MacroCondition, state: &Phase1State, out: &mut Vec<AtomEvidence>, truncated: &mut bool) {
    if out.len() >= 32 { *truncated = true; return; }
    match cond {
        MacroCondition::And(a,b) | MacroCondition::Or(a,b) => { atoms(a,state,out,truncated); atoms(b,state,out,truncated); }
        _ => out.push(AtomEvidence {condition:cond.display_string(), actual:value(cond,state), passed:eval_condition(cond,state)}),
    }
}

pub(crate) fn inspect<'a>(target: &str, config: &MacroConfig, page: usize, player: &'a Player,
    map: &'a HashMap<&'a str, Vec<&'a SkillSpec>>, ids: &'a HashMap<u32, &'a SkillSpec>, last: Option<&str>) -> TargetEvidence {
    let state = Phase1State { player, skill_map:map, skill_by_id:ids, last_skill:last.map(str::to_owned),
        buff_lookup:player.buff_idx_lookup(), target_buff_lookup:player.target_idx_lookup() };
    let probe = |name: &str, line: usize, is_fcast: bool| {
        let entry = PoolEntry {line, skill_name:name.to_owned(), is_fcast};
        let mut observation = evaluate_phase2(&[entry],player,map,ids,true).1.remove(0);
        if observation.reason == "体态/条件不满足" {
            // Use the same rejection explanations as manual simulation. Some
            // special restrictions have no detailed text, so retain fallback.
            if let Some(ranks) = skill_group(name, map, ids) {
                let mut reasons: Vec<_> = ranks.iter().filter_map(|rank| player.reject_reason(rank)).collect();
                reasons.sort(); reasons.dedup();
                if !reasons.is_empty() { observation.reason = reasons.into_iter().take(4).collect::<Vec<_>>().join("；"); }
            }
        }
        observation
    };
    let mut lines = Vec::new(); let mut other_pages = Vec::new();
    for (p, macro_page) in config.pages.iter().enumerate() {
        for (index, line) in macro_page.lines.iter().enumerate() {
            if !same_action(target,line.action.skill_name(),map,ids) {continue;}
            if p != page { if !other_pages.contains(&(p+1)) {other_pages.push(p+1);} continue; }
            let mut observations = Vec::new(); let mut truncated = false;
            if let Some(cond) = &line.condition {atoms(cond,&state,&mut observations,&mut truncated);}
            lines.push(TargetLine {line:index+1, condition:line.condition.as_ref().map(|c| c.semantic_string()).unwrap_or_else(|| "无条件".into()),
                passed:line.condition.as_ref().map(|c| eval_condition(c,&state)).unwrap_or(true), atoms:observations, truncated,
                probe:probe(line.action.skill_name(), index+1, line.action.is_fcast())});
        }
    }
    TargetEvidence {skill:target.into(), lines, other_pages, probe:probe(target,0,false)}
}
