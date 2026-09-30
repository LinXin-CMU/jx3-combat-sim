//! Local-only execution oracle for finite, exact macro synthesis.
//! No solver, filesystem writes, userdata or alternative combat rules live here.
use crate::macro_eval::{evaluate_condition_truths, evaluate_phase2, CastOutcome, PoolEntry};
use crate::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, Write};

pub const EPS: f64 = 1e-7;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Acceptance {
    #[default]
    SkillsAndState,
    SkillsAndTime,
}

#[derive(Clone, Debug, Serialize)]
pub struct Action {
    pub name: String,
    pub fcast: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub time: f64,
    pub cursor: usize,
    pub last_skill: Option<String>,
    pub state: Value,
    pub truth: Vec<bool>,
    pub executable: Vec<bool>,
    pub outcomes: Vec<Option<(u32, f64)>>,
    pub allowed: Vec<usize>,
    pub wait_allowed: bool,
    pub decision_latest: Option<f64>,
    pub wait_next_time: Option<f64>,
    pub wake_atoms: Vec<usize>,
    pub rejected_actions: Vec<usize>,
}

/// Teacher is an instrumentation mode, not a macro feature. Its output must first
/// agree with the independent manual executor. Candidate verification never uses it.
pub struct Probe {
    teacher: bool,
    reference: Vec<CastEvent>,
    actions: Vec<Action>,
    atoms: macro_engine::MacroConfig,
    atom_thresholds: Vec<Vec<(u32, f64, bool)>>,
    pub rows: Vec<Row>,
    cursor: usize,
    aligned: bool,
    pub failure: Option<Value>,
    pub truncated: bool,
    time_tolerance: f64,
    stop_on_divergence: bool,
    stopped_early: bool,
    require_state_match: bool,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactProbe")
            .field("teacher", &self.teacher)
            .finish()
    }
}

impl Probe {
    fn new(
        teacher: bool,
        reference: Vec<CastEvent>,
        actions: Vec<Action>,
        atoms: &[String],
        time_tolerance: f64,
    ) -> Result<Self, String> {
        let text = atoms
            .iter()
            .map(|s| format!("/cast [{s}] 盾刀"))
            .collect::<Vec<_>>()
            .join("\n");
        let atoms = macro_parser::parse_macro_text(if text.is_empty() {
            "/cast 盾刀"
        } else {
            &text
        })
        .map_err(|e| e.to_string())?;
        let atom_thresholds = atoms.pages[0].lines.iter().map(|line| {
            let mut thresholds = Vec::new();
            if let Some(condition) = &line.condition {
                condition.collect_bufftime_thresholds(&mut thresholds);
            }
            thresholds.into_iter().filter_map(|(name, value, target)|
                macro_eval::buff_name_to_id(&name).map(|id| (id, value, target))).collect()
        }).collect();
        Ok(Self {
            teacher,
            reference,
            actions,
            atoms,
            atom_thresholds,
            rows: vec![],
            cursor: 0,
            aligned: true,
            failure: None,
            truncated: false,
            time_tolerance,
            stop_on_divergence: false,
            stopped_early: false,
            require_state_match: true,
        })
    }

    pub fn observe<'a>(
        &mut self,
        player: &'a Player,
        map: &'a HashMap<&str, Vec<&'a SkillSpec>>,
        ids: &'a HashMap<u32, &'a SkillSpec>,
        last: Option<&str>,
        first: bool,
        delay: f64,
        pool: &mut Vec<PoolEntry>,
    ) {
        if !self.aligned {
            if self.teacher {
                pool.clear();
            }
            return;
        }
        // Never label states after the target should already have cast. Such a
        // state has diverged in time even if no wrong skill has yet been cast.
        if let Some(event) = self.reference.get(self.cursor) {
            let deadline = event.cast_time - if event.is_main && !first { delay } else { 0.0 };
            let tolerance = if self.teacher { EPS } else { self.time_tolerance };
            if player.current_time > deadline + tolerance + EPS {
                self.failure = Some(
                    json!({"index":self.cursor,"kind":"missed_decision_time","deadline":deadline,"time":player.current_time}),
                );
                self.aligned = false;
                self.stopped_early = self.stop_on_divergence && !self.teacher;
                if self.teacher {
                    pool.clear();
                }
                return;
            }
        }
        if self.rows.len() >= 50000 {
            self.truncated = true;
            return;
        }
        let truth = evaluate_condition_truths(
            &self.atoms.pages[0],
            player,
            map,
            ids,
            last.map(str::to_owned),
        );
        let mut outcomes = Vec::new();
        for action in &self.actions {
            let entry = PoolEntry {
                line: 1,
                skill_name: action.name.clone(),
                is_fcast: action.fcast,
            };
            let (result, _) = evaluate_phase2(&[entry], player, map, ids, false);
            outcomes.push(result.map(|r| {
                let actual = resolve_combo_follow(r.skill, player, ids).unwrap_or(r.skill);
                (
                    actual.skill_id,
                    player.next_cast_time(actual)
                        + if macro_eval::skill_is_main(actual) && !first {
                            delay
                        } else {
                            0.0
                        },
                )
            }));
        }
        let target = self.reference.get(self.cursor);
        let allowed: Vec<_> = outcomes
            .iter()
            .enumerate()
            .filter_map(|(i, outcome)| match (target, outcome) {
                (Some(t), Some((id, time)))
                    if t.skill_id == *id && (t.cast_time - time).abs() <= self.time_tolerance + EPS =>
                {
                    Some(i)
                }
                _ => None,
            })
            .collect();
        let teacher_action = outcomes.iter().position(|outcome| matches!((target, outcome),
            (Some(t), Some((id, time))) if t.skill_id == *id && (t.cast_time-time).abs() <= EPS));
        self.rows.push(Row {
            time: player.current_time,
            cursor: self.cursor,
            last_skill: last.map(str::to_owned),
            state: serde_json::to_value(snapshot_event_state(player)).unwrap(),
            truth,
            executable: outcomes.iter().map(Option::is_some).collect(),
            outcomes,
            allowed: allowed.clone(),
            wait_allowed: target.is_none_or(|t| {
                let decision = t.cast_time - (if t.is_main && !first { delay } else { 0.0 });
                player.current_time < decision + self.time_tolerance - EPS
            }),
            decision_latest: target.map(|t| t.cast_time
                - (if t.is_main && !first { delay } else { 0.0 }) + self.time_tolerance),
            wait_next_time: None,
            wake_atoms: Vec::new(),
            rejected_actions: Vec::new(),
        });
        if self.teacher {
            pool.clear();
            // Reference collection still releases at the EXACT manual time.
            // Tolerant labels must never change the teacher's own trajectory.
            if let Some(i) = teacher_action {
                pool.push(PoolEntry {
                    line: i + 1,
                    skill_name: self.actions[i].name.clone(),
                    is_fcast: self.actions[i].fcast,
                });
            }
        }
    }

    pub fn next_time(&mut self, player: &Player, next: f64, first: bool, delay: f64) -> f64 {
        let now = player.current_time;
        if self.teacher {
            if let Some(event) = self.reference.get(self.cursor) {
                let decision = event.cast_time - if event.is_main && !first { delay } else { 0.0 };
                if decision > now + EPS {
                    return next.min(decision);
                }
            }
        }
        // Observe the actual outgoing WAIT edge without changing its timing.
        // A failed wait does NOT imply that every macro must cast here: another
        // legal threshold can wake it before the deadline. Record those options
        // using the SAME scheduler as the executor, for a reversible search hint.
        if self.aligned {
            if let Some(row) = self.rows.last_mut().filter(|r| r.cursor == self.cursor && (r.time-now).abs() <= EPS) {
                row.wait_next_time = Some(next);
                if let Some(latest) = row.decision_latest.filter(|latest| next > latest + EPS) {
                    row.wake_atoms = self.atom_thresholds.iter().enumerate()
                        .filter_map(|(i, thresholds)| {
                            if thresholds.is_empty() { return None; }
                            let wake = player.next_decision_time(thresholds);
                            (wake > now && wake <= latest + EPS).then_some(i)
                        }).collect();
                }
            }
        }
        next
    }

    pub fn finish(&mut self, result: &CastOutcome, name: &str, fcast: bool) {
        if !self.aligned {
            return;
        }
        if let Some(actual) = result.events.iter().find(|e| !e.triggered) {
            let expected = self.reference.get(self.cursor);
            let tolerance = if self.teacher { EPS } else { self.time_tolerance };
            let agrees = expected.is_some_and(|e| same_cast_with_tolerance(e, actual, tolerance)
                && (!self.require_state_match || same_state_with_tolerance(e, actual, tolerance)));
            if !agrees {
                // Channel counts/duration may still be revised by a later fcast.
                // Failed attempts with no active event are not terminal either.
                let irreversible = expected.is_none_or(|e|
                    e.skill_id != actual.skill_id || e.name != actual.name
                    || (e.cast_time - actual.cast_time).abs() > tolerance + EPS
                    || (self.require_state_match && !same_state_with_tolerance(e, actual, tolerance)));
                self.stopped_early = self.stop_on_divergence && !self.teacher && irreversible;
                self.failure = Some(
                    json!({"index":self.cursor,"kind":if expected.is_some_and(|e|same_cast_with_tolerance(e,actual,tolerance)) {"state_mismatch"} else {"cast_mismatch"},"expected":expected.map(action_view),"actual":action_view(actual)}),
                );
                self.reject_action(name, fcast);
                self.aligned = false;
            } else {
                self.cursor += 1;
            }
        } else {
            self.reject_action(name, fcast);
            // Phase 2 eligibility is necessary, but cast_skill may still reject.
            self.failure = Some(json!({"index":self.cursor,"kind":"selected_cast_rejected"}));
            self.aligned = false;
        }
    }

    pub fn should_stop(&self) -> bool { self.stopped_early }

    fn reject_action(&mut self, name: &str, fcast: bool) {
        if let Some(index) = self.actions.iter().position(|a| a.name == name && a.fcast == fcast) {
            if let Some(row) = self.rows.last_mut() {
                // Phase 2 alone does not certify post-cast states or cast success.
                // Refine only the action actually executed from this aligned row;
                // /cast and /fcast may have different channel effects.
                row.allowed.retain(|a| *a != index);
                row.rejected_actions.push(index);
            }
        }
    }
}

fn same_cast(a: &CastEvent, b: &CastEvent) -> bool {
    same_cast_with_tolerance(a, b, EPS)
}

fn same_cast_with_tolerance(a: &CastEvent, b: &CastEvent, tolerance: f64) -> bool {
    a.skill_id == b.skill_id
        && a.name == b.name
        && (a.cast_time - b.cast_time).abs() <= tolerance + EPS
        && a.channel_ticks == b.channel_ticks
        && match (a.channel_duration, b.channel_duration) {
            (Some(a), Some(b)) => (a - b).abs() <= tolerance + EPS,
            (None, None) => true,
            _ => false,
        }
}

fn same_state(a: &CastEvent, b: &CastEvent) -> bool {
    same_state_with_tolerance(a, b, EPS)
}

fn same_state_with_tolerance(a: &CastEvent, b: &CastEvent, tolerance: f64) -> bool {
    fn canonical(v: &mut Value) {
        match v {
            Value::Array(a) => {
                for v in a.iter_mut() {
                    canonical(v);
                }
                // Stable identity, not mutable remaining time, determines pairing.
                a.sort_by_key(|v| v.get("buff_id").or_else(|| v.get("skill_id"))
                    .or_else(|| v.get("name")).map(Value::to_string).unwrap_or_else(|| v.to_string()));
            }
            Value::Object(m) => {
                for v in m.values_mut() {
                    canonical(v);
                }
            }
            _ => (),
        }
    }
    fn equal(a: &Value, b: &Value, key: &str, tolerance: f64) -> bool {
        match (a, b) {
            (Value::Number(a), Value::Number(b)) => {
                let limit = if matches!(key, "time" | "remaining") { tolerance } else { EPS };
                (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= limit + EPS
            }
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b, key, tolerance))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len() && a.iter().all(|(k, a)| b.get(k).is_some_and(|b| equal(a, b, k, tolerance)))
            }
            _ => a == b,
        }
    }
    let mut a = serde_json::to_value(&a.state_after).unwrap();
    let mut b = serde_json::to_value(&b.state_after).unwrap();
    canonical(&mut a);
    canonical(&mut b);
    equal(&a, &b, "", tolerance)
}

fn action_view(e: &CastEvent) -> Value {
    json!({"skill_id":e.skill_id,"name":e.name,"time":e.cast_time,"is_main":e.is_main,
        "channel_ticks":e.channel_ticks,"channel_duration":e.channel_duration,"sequence_index":e.sequence_index})
}

fn compare(reference: &[CastEvent], actual: &[CastEvent]) -> Value {
    compare_with_tolerance(reference, actual, EPS)
}

fn compare_with_tolerance(reference: &[CastEvent], actual: &[CastEvent], tolerance: f64) -> Value {
    compare_with_policy(reference, actual, tolerance, Acceptance::SkillsAndState)
}

fn compare_with_policy(reference: &[CastEvent], actual: &[CastEvent], tolerance: f64, acceptance: Acceptance) -> Value {
    let order = reference
        .iter()
        .zip(actual)
        .take_while(|(a, b)| a.skill_id == b.skill_id && a.name == b.name)
        .count();
    let exact = reference
        .iter()
        .zip(actual)
        .take_while(|(a, b)| same_cast_with_tolerance(a, b, tolerance))
        .count();
    let states = reference
        .iter()
        .zip(actual)
        .take_while(|(a, b)| same_cast_with_tolerance(a, b, tolerance) && same_state_with_tolerance(a, b, tolerance))
        .count();
    let matched = if acceptance == Acceptance::SkillsAndState { exact.min(states) } else { exact };
    let reproduced = matched == reference.len() && matched == actual.len();
    let error = reference
        .iter()
        .zip(actual)
        .take(order)
        .map(|(a, b)| (a.cast_time - b.cast_time).abs())
        .fold(0.0, f64::max);
    let strict = reference.len() == actual.len() && reference.iter().zip(actual)
        .all(|(a,b)| same_cast(a,b) && same_state(a,b));
    json!({"reproduced":reproduced,"target_count":reference.len(),"actual_count":actual.len(),
        "order_prefix":order,"exact_prefix":exact,"state_prefix":states,"acceptance_prefix":matched,"acceptance":acceptance,
        "state_reproduced":states == reference.len() && states == actual.len(),
        "time_tolerance_seconds":tolerance,"max_time_error_on_order_prefix":error,"strict_reproduced":strict,
        "state_first_difference":if states == reference.len() && states == actual.len() { Value::Null } else {json!({"index":states})},
        "first_difference":if reproduced {Value::Null} else {json!({"index":matched,"expected":reference.get(matched).map(action_view),"actual":actual.get(matched).map(action_view)})}})
}

fn numeric(out: &mut BTreeSet<String>, field: &str, value: i64) {
    for op in ["=", "<", ">"] {
        out.insert(format!("{field}{op}{value}"));
    }
}

/// Only names understood by the actual evaluator are emitted; synthetic combo
/// IDs and internal state fields are deliberately excluded from the language.
fn candidate_atoms(events: &[CastEvent], actions: &[Action]) -> Vec<String> {
    let mut atoms = BTreeSet::new();
    for action in actions.iter().filter(|a| !a.fcast) {
        atoms.insert(format!("last_skill={}", action.name));
        atoms.insert(format!("last_skill~={}", action.name));
        atoms.insert(format!("skill_notin_cd:{}", action.name));
    }
    for e in events {
        for state in [e.state_before.as_ref(), e.state_after.as_ref()]
            .into_iter()
            .flatten()
        {
            numeric(&mut atoms, "rage", state.rage as i64);
            let v = serde_json::to_value(state).unwrap();
            for (name, key) in [("energy", "block_value"), ("sun", "berserk_value")] {
                if let Some(n) = v[key].as_i64() {
                    numeric(&mut atoms, name, n);
                }
            }
            if let Some(skills) = v["skill_states"].as_array() {
                for skill in skills {
                    if let (Some(name), Some(n)) =
                        (skill["name"].as_str(), skill["charges"].as_i64())
                    {
                        numeric(&mut atoms, &format!("skill_energy:{name}"), n);
                    }
                }
            }
            for (target, buffs) in [(false, &state.buffs), (true, &state.target_buffs)] {
                for buff in buffs {
                    if macro_eval::buff_name_to_id(&buff.name) != Some(buff.buff_id) {
                        continue;
                    }
                    let prefix = if target { "t" } else { "" };
                    atoms.insert(format!("{prefix}buff:{}", buff.name));
                    atoms.insert(format!("{prefix}nobuff:{}", buff.name));
                    if !target {
                        numeric(
                            &mut atoms,
                            &format!("buff:{}", buff.name),
                            buff.stacks as i64,
                        );
                    }
                    if buff.remaining > 0.0 {
                        for n in [
                            (buff.remaining * 10.0).floor() / 10.0,
                            (buff.remaining * 10.0).ceil() / 10.0,
                            buff.remaining.floor(),
                        ] {
                            for op in ["<", ">"] {
                                atoms.insert(format!("{prefix}bufftime:{}{op}{n:.1}", buff.name));
                            }
                        }
                    }
                }
            }
        }
    }
    atoms.into_iter().collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: GameVersion,
    mount: Mount,
    simulation: SimulateRequest,
    horizon: f64,
    #[serde(default)]
    candidate: Option<String>,
    #[serde(default)]
    atoms: Option<Vec<String>>,
    #[serde(default = "strict_tolerance")]
    time_tolerance_seconds: f64,
    #[serde(default)]
    acceptance: Acceptance,
    #[serde(default)]
    compact_result: bool,
    #[serde(default)]
    stop_on_divergence: bool,
    /// Local CLI only; never accepted by the public job request.
    #[serde(default)]
    archive_path: Option<std::path::PathBuf>,
}

fn strict_tolerance() -> f64 { EPS }

struct Prepared {
    key: String,
    constants: MountConstants,
    skills: Vec<SkillSpec>,
    recipes: Vec<RecipeEntry>,
    team: Vec<TeamBuffEntry>,
    formations: Vec<FormationEntry>,
    reference: SimulateResponse,
    active: Vec<CastEvent>,
}

#[cfg(test)]
fn run(req: Request) -> Result<Value, String> { run_cached(req, &mut None) }

fn run_cached(req: Request, cache: &mut Option<Prepared>) -> Result<Value, String> {
    let started = std::time::Instant::now();
    if !req.time_tolerance_seconds.is_finite() || !(EPS..=0.125).contains(&req.time_tolerance_seconds) {
        return Err("time tolerance must be in [1e-7,0.125] seconds".into());
    }
    if !req.horizon.is_finite() || req.horizon <= 0.0 || req.horizon > 1200.0 {
        return Err("horizon must be in (0,1200]".into());
    }
    if req.simulation.sequence.is_empty()
        || req.simulation.sequence.len() > 6000
        || req.simulation.sequence.iter().any(|s| s.starts_with("__"))
    {
        return Err("target must contain 1..6000 explicit manual skill names; debug/macro operations are unsupported".into());
    }
    if req.simulation.macro_text.is_some()
        || req.simulation.lite
        || !req.simulation.pauses.is_empty()
    {
        return Err(
            "target must be a full manual request without macro_text, lite or pauses".into(),
        );
    }
    let mut base = req.simulation.clone();
    base.macro_duration = Some(req.horizon);
    let key = serde_json::to_string(&json!({"version":req.version,"mount":req.mount,
        "simulation":base,"horizon":req.horizon})).map_err(|e| e.to_string())?;
    let cache_hit = cache.as_ref().is_some_and(|p| p.key == key);
    if !cache_hit {
        let (constants, _, _, _, _) = load_school_toml(req.version, req.mount)?;
        let skills = load_skills(Path::new(&skills_dir(req.version, req.mount)));
        let recipes = load_recipes(Path::new(&recipes_file_for_mount(req.version, req.mount)));
        let team = load_team_buffs(Path::new(&team_buffs_file_for_mount(req.version, req.mount)));
        let formations = load_formations(Path::new(&formations_file(req.version)));
        let reference = simulate_core_with_trace(&base, &skills, req.version, req.mount,
            constants, &recipes, &team, &formations, None);
        let active = reference.timeline.iter().filter(|e| !e.triggered).cloned().collect();
        *cache = Some(Prepared { key, constants, skills, recipes, team, formations, reference, active });
    }
    let prepared = cache.as_ref().unwrap();
    let skills = &prepared.skills;
    let reference = &prepared.reference;
    let active = &prepared.active;
    let preparation_ms = started.elapsed().as_secs_f64() * 1000.0;
    let simulate = |r: &SimulateRequest, trace| {
        simulate_core_with_trace(
            r,
            skills,
            req.version,
            req.mount,
            prepared.constants,
            &prepared.recipes,
            &prepared.team,
            &prepared.formations,
            trace,
        )
    };
    if !reference.skipped.is_empty() || active.len() != base.sequence.len() {
        return Ok(
            json!({"status":"invalid_target","skipped":reference.skipped,"target":active.iter().map(action_view).collect::<Vec<_>>() }),
        );
    }
    if active.iter().any(|e| e.cast_time > req.horizon + EPS) {
        return Err("horizon excludes a target cast".into());
    }
    let mut names = BTreeSet::new();
    for e in active {
        let skill = skills
            .iter()
            .find(|s| s.skill_id == e.skill_id)
            .ok_or("unknown active skill")?;
        names.insert(if (90010..=90012).contains(&skill.skill_id) {
            skill.name.clone()
        } else {
            skill.name.split('·').next().unwrap().into()
        });
    }
    // In a no-channel scene these commands have identical executor semantics.
    // Canonicalize the search alphabet, not a generated macro after acceptance.
    let has_channel = active.iter().any(|e| e.channel_ticks.is_some() || e.channel_duration.is_some()
        || skills.iter().any(|s| s.skill_id == e.skill_id && s.channel_interval.is_some()))
        || base.pre_releases.iter().any(|p| skills.iter().any(|s|
            s.name.split('·').next() == Some(p.skill.as_str()) && s.channel_interval.is_some()));
    let cast_only = req.acceptance == Acceptance::SkillsAndTime && !has_channel;
    let actions: Vec<_> = names
        .into_iter()
        .flat_map(|name| {
            [
                Action {
                    name: name.clone(),
                    fcast: false,
                },
                Action { name, fcast: true },
            ].into_iter().filter(move |a| !cast_only || !a.fcast)
        })
        .collect();
    let atoms = req
        .atoms
        .unwrap_or_else(|| candidate_atoms(&active, &actions));
    if atoms.is_empty() || atoms.len() > 30000 {
        return Err("atom budget exceeded (1..30000)".into());
    }
    let teacher = req.candidate.is_none();
    let text = req.candidate.unwrap_or_else(|| {
        actions
            .iter()
            .map(|a| format!("/{} {}", if a.fcast { "fcast" } else { "cast" }, a.name))
            .collect::<Vec<_>>()
            .join("\n")
    });
    macro_parser::parse_macro_text(&text).map_err(|e| e.to_string())?;
    let mut trace = macro_diagnostic::Collector::new(-2.0, -1.0);
    trace.exact = Some(Probe::new(
        teacher,
        active.clone(),
        actions.clone(),
        &atoms,
        req.time_tolerance_seconds,
    )?);
    trace.exact.as_mut().unwrap().stop_on_divergence = req.stop_on_divergence;
    trace.exact.as_mut().unwrap().require_state_match = teacher || req.acceptance == Acceptance::SkillsAndState;
    let mut candidate = base.clone();
    candidate.sequence = vec!["__macro__".into(); 6000];
    candidate.macro_text = Some(text);
    candidate.channel_ticks.clear();
    candidate.timing_offsets.clear();
    candidate.solidified_casts.clear();
    candidate.qijin_buffs.clear();
    let replay_started = std::time::Instant::now();
    let actual = simulate(&candidate, Some(&mut trace));
    let replay_ms = replay_started.elapsed().as_secs_f64() * 1000.0;
    // The runtime allows a final cast started at the horizon (and delay may place
    // it beyond it). Keep EVERY returned active cast: never crop away extras.
    let casts: Vec<_> = actual
        .timeline
        .iter()
        .filter(|e| !e.triggered)
        .cloned()
        .collect();
    let mut comparison = if teacher { compare(active, &casts) }
        else { compare_with_policy(active, &casts, req.time_tolerance_seconds, req.acceptance) };
    let probe = trace.exact.unwrap();
    comparison["completed_full_replay"] = (!probe.stopped_early).into();
    if probe.stopped_early {
        comparison["reproduced"] = false.into();
        comparison["strict_reproduced"] = false.into();
    }
    // Exhausting placeholders cannot prove terminal silence, even when every
    // requested action has matched. Never certify a cast-count-truncated replay.
    let cast_budget_reached = casts.len() >= candidate.sequence.len();
    let status = if probe.truncated || cast_budget_reached {
        "probe_budget"
    } else if teacher && comparison["reproduced"] != true {
        "semantic_mismatch"
    } else {
        "ok"
    };
    let projection_started = std::time::Instant::now();
    let full = if !req.compact_result || req.archive_path.is_some() { Some(
        json!({"status":status,"teacher":teacher,"comparison":comparison,"actions":actions,"atoms":atoms,
        "target":active,"actual":casts,"rows":probe.rows,"probe_failure":probe.failure,"truncated":probe.truncated || cast_budget_reached,
        "simulation":base,"version":req.version,"mount":req.mount,"horizon":req.horizon,
        "target_fingerprint":reference.fingerprint.to_string(),"actual_fingerprint":actual.fingerprint.to_string()})
    ) } else { None };
    if let Some(path) = req.archive_path {
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        serde_json::to_writer(std::io::BufWriter::new(file), full.as_ref().unwrap()).map_err(|e| e.to_string())?;
    }
    let mut value = if req.compact_result {
        let rows: Vec<_> = probe.rows.iter().enumerate().map(|(i, r)| {
            let bits: String = r.truth.chunks(8).map(|chunk| {
                let byte = chunk.iter().enumerate().fold(0u8, |n,(bit,v)| n | ((*v as u8) << bit));
                format!("{byte:02x}")
            }).collect();
            json!({"time":r.time,"cursor":r.cursor,"last_skill":r.last_skill,
                "truth_hex":bits,"truth_count":r.truth.len(),"executable":r.executable,
                "allowed":r.allowed,"wait_allowed":r.wait_allowed,"decision_latest":r.decision_latest,
                "wait_next_time":r.wait_next_time,"wake_atoms":r.wake_atoms,"rejected_actions":r.rejected_actions,
                "state":if i + 1 == probe.rows.len() { &r.state } else { &Value::Null }})
        }).collect();
        let events: Vec<_> = casts.iter().map(|e| json!({"skill_id":e.skill_id,"name":e.name,
            "cast_time":e.cast_time,"macro_line":e.macro_line,"macro_page":e.macro_page})).collect();
        let mut v = json!({"status":status,"teacher":teacher,"comparison":comparison,"rows":rows,
            "actual":events,"probe_failure":probe.failure,"truncated":probe.truncated || cast_budget_reached,
            "actual_fingerprint":actual.fingerprint.to_string()});
        if teacher {
            v["actions"] = json!(actions); v["atoms"] = json!(atoms); v["simulation"] = json!(base);
            v["target"] = json!(active.iter().map(action_view).collect::<Vec<_>>());
            v["target_fingerprint"] = reference.fingerprint.to_string().into();
        }
        v
    } else { full.unwrap() };
    value["timings_ms"] = json!({"prepare":preparation_ms,"replay_with_probe":replay_ms,
        "projection_and_archive":projection_started.elapsed().as_secs_f64()*1000.0,"scene_cache_hit":cache_hit});
    Ok(value)
}

pub fn cli() {
    let stdin = std::io::stdin();
    let mut cache = None;
    for line in stdin.lock().lines() {
        let result = line
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Request>(&s).map_err(|e| e.to_string()))
            .and_then(|req| run_cached(req, &mut cache));
        let value = result.unwrap_or_else(|error| json!({"status":"error","error":error}));
        println!("EXACT_JSON {}", value);
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
#[path = "../tests/macro_exact/mod.rs"]
mod tests;
