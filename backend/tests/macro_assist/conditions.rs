use super::*;
use serde_json::{json, Value};

fn cast(name: &str, state: Value) -> Value {
    json!({"name": name, "state_before": state, "triggered": false})
}

fn state(rage: i32, sun: i32) -> Value {
    json!({"rage":rage,"berserk_value":sun,"block_value":77,
        "buffs":[],"target_buffs":[],"skill_states":[]})
}

fn request(timeline: Vec<Value>, start: usize, end: usize, step: usize) -> AssistRequest {
    serde_json::from_value(json!({"timeline":timeline,"selection_start":start,
        "selection_end":end,"step":step,"options":{"max_terms":2,"max_candidates":1000}}))
    .unwrap()
}

fn condition(text: &str) -> MacroCondition {
    crate::macro_parser::parse_macro_text(&format!("/cast [{text}] 盾击"))
        .unwrap()
        .pages
        .remove(0)
        .lines
        .remove(0)
        .condition
        .unwrap()
}

fn sample(state: &InputState) -> Sample<'_> {
    Sample {
        index: 0,
        positive: true,
        same_skill_outside_combo: false,
        state: Some(state),
        last_skill: None,
    }
}

fn contains_expression(candidate: &Candidate, expression: &str) -> bool {
    candidate.expression == expression
        || candidate
            .equivalent_expressions
            .iter()
            .any(|item| item == expression)
}

#[test]
fn finds_all_contiguous_occurrences_including_off_gcd_and_overlaps() {
    let timeline = vec![
        cast("盾击·一", state(0, 120)),
        json!({"name":"破招", "triggered":true}),
        cast("血怒", state(10, 120)),
        cast("盾击·二", state(20, 120)),
        cast("血怒", state(30, 120)),
        cast("盾击", state(40, 120)),
    ];
    let result = analyze(&request(timeline, 0, 1, 1)).unwrap();
    assert_eq!(result.positives, vec![1, 3]);
    assert_eq!(result.active_count, 5);
    assert_eq!(result.occurrences[1].start_active_index, 2);
    assert_eq!(result.selection.step_target, "血怒");
    let overlap = analyze(&request(
        vec![
            cast("盾击", state(0, 0)),
            cast("盾击", state(0, 0)),
            cast("盾击", state(0, 0)),
        ],
        0,
        1,
        0,
    ))
    .unwrap();
    assert_eq!(overlap.positives, vec![0, 1]);
    assert_eq!(overlap.negative_count, 1);
}

#[test]
fn combo_followups_remain_separate_and_other_starts_are_negative() {
    let timeline = vec![
        cast("阵云结晦", state(0, 120)),
        cast("月照连营", state(0, 80)),
        cast("雁门迢递", state(0, 40)),
        cast("阵云结晦", state(0, 120)),
        cast("盾击", state(0, 120)),
    ];
    let result = analyze(&request(timeline, 0, 2, 1)).unwrap();
    assert_eq!(result.positives, vec![1]);
    assert_eq!(
        result.selection.skill_names,
        vec!["阵云结晦", "月照连营", "雁门迢递"]
    );
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.macro_text.ends_with(" 月照连营")));
}

#[test]
fn sun_recommendations_are_independent_of_rage_and_block_value() {
    let result = analyze(&request(
        vec![
            cast("阵云结晦", state(50, 120)),
            cast("盾击", state(50, 0)),
            cast("阵云结晦", state(50, 120)),
        ],
        0,
        0,
        0,
    ))
    .unwrap();
    let sun = result
        .candidates
        .iter()
        .find(|candidate| contains_expression(candidate, "sun=120"))
        .unwrap();
    assert_eq!((sun.tp, sun.fp, sun.r#fn), (2, 0, 0));
    assert_eq!(sun.coverage, 1.0);
    assert_eq!(sun.precision, Some(1.0));
    assert!(result.candidates[0].expression.starts_with("sun"));
    let state: InputState = serde_json::from_value(state(12, 120)).unwrap();
    assert_eq!(eval(&condition("energy=77"), &sample(&state)), Truth::True);
    assert_eq!(
        eval(&condition("energy=120"), &sample(&state)),
        Truth::False
    );
    assert_eq!(eval(&condition("sun=120"), &sample(&state)), Truth::True);
    assert_eq!(eval(&condition("rage=12"), &sample(&state)), Truth::True);
}

#[test]
fn last_skill_tracks_successful_off_gcd_casts_and_ignores_triggered_events() {
    let result = analyze(&request(
        vec![
            cast("盾击", state(0, 0)),
            cast("血怒", state(0, 0)),
            json!({"name":"被动", "triggered":true}),
            cast("绝刀", state(0, 0)),
            cast("盾击", state(0, 0)),
        ],
        2,
        2,
        0,
    ))
    .unwrap();
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.expression == "last_skill=血怒")
        .unwrap();
    assert_eq!(candidate.matched_positive_indices, vec![2]);
    assert_eq!(candidate.fp, 0);
}

#[test]
fn missing_resource_charge_and_buff_fields_stay_unknown() {
    let empty = InputState::default();
    for text in [
        "rage=0",
        "sun=0",
        "energy=100",
        "buff:血怒",
        "nobuff:血怒",
        "buff:血怒=0",
        "bufftime:血怒<5",
        "skill_energy:血怒=3",
        "skill_notin_cd:血怒",
        "life=1",
        "nearby_enemy=1",
        "skill:30769",
    ] {
        assert_eq!(
            eval(&condition(text), &sample(&empty)),
            Truth::Unknown,
            "{text}"
        );
    }
    let no_buffs: InputState = serde_json::from_value(json!({"buffs":[]})).unwrap();
    assert_eq!(
        eval(&condition("buff:血怒=0"), &sample(&no_buffs)),
        Truth::True
    );
    assert_eq!(
        eval(&condition("bufftime:血怒<5"), &sample(&no_buffs)),
        Truth::False
    );
}

#[test]
fn permanent_buff_time_aliases_and_duplicate_id_use_runtime_semantics() {
    let id = crate::macro_eval::buff_name_to_id("麟光甲").unwrap();
    let data: InputState = serde_json::from_value(json!({"buffs":[
        {"name":"麟光玄甲", "buff_id":id,"remaining":0,"stacks":2},
        {"name":"麟光玄甲", "buff_id":id,"remaining":0,"stacks":5}],"target_buffs":[]}))
    .unwrap();
    let row = sample(&data);
    assert_eq!(eval(&condition("buff:麟光甲=5"), &row), Truth::True);
    assert_eq!(eval(&condition("buff:麟光甲=2"), &row), Truth::False);
    assert_eq!(eval(&condition("bufftime:麟光甲<5"), &row), Truth::False);
    assert_eq!(eval(&condition("bufftime:麟光甲>5"), &row), Truth::True);
    assert_eq!(eval(&condition("tbufftime:虚弱<5"), &row), Truth::False);
}

#[test]
fn full_charges_and_readiness_are_read_from_structured_fields_only() {
    let data: InputState = serde_json::from_value(json!({"skill_states":[
        {"name":"血怒","skill_id":13040,"charges":3,"max_charges":3,"not_in_cd":true},
        {"name":"盾击","skill_id":13047,"charges":2,"max_charges":3,"not_in_cd":false},
        {"name":"绝刀","skill_id":13054,"charges":null,"max_charges":null,"not_in_cd":true}]}))
    .unwrap();
    let row = sample(&data);
    assert_eq!(eval(&condition("skill_energy:血怒=3"), &row), Truth::True);
    assert_eq!(eval(&condition("skill_energy:盾击=2"), &row), Truth::True);
    assert_eq!(eval(&condition("skill_notin_cd:盾击"), &row), Truth::False);
    assert_eq!(
        eval(&condition("skill_energy:绝刀=0"), &row),
        Truth::Unknown
    );
    let old: InputState =
        serde_json::from_value(json!({"skill_cds":[{"name":"血怒(0层)","remaining":20}]})).unwrap();
    assert_eq!(
        eval(&condition("skill_energy:血怒=0"), &sample(&old)),
        Truth::Unknown
    );
    let (atoms, _) = enumerate_atoms(&[row]);
    assert!(!atoms
        .iter()
        .any(|atom| atom.display_string().starts_with("skill_energy:绝刀")));
}

#[test]
fn searches_an_and_combination_when_neither_atom_separates() {
    let result = analyze(&request(
        vec![
            cast("绝刀", state(50, 120)),
            cast("盾击", state(50, 0)),
            cast("盾击", state(0, 120)),
            cast("绝刀", state(50, 120)),
        ],
        0,
        0,
        0,
    ))
    .unwrap();
    let best = &result.candidates[0];
    assert_eq!(best.terms, 2);
    assert_eq!((best.tp, best.fp), (2, 0));
    assert!(best.expression.contains('&'));
    assert!(result.search.compounds_evaluated > 0);
}

#[test]
fn mixed_conditions_are_scored_after_the_real_right_associative_parse() {
    let data: InputState = serde_json::from_value(state(0, 120)).unwrap();
    let row = sample(&data);
    // rage=50 & (sun=0 | sun=120) is false, unlike (rage=50 & sun=0) | sun=120.
    assert_eq!(
        eval(&condition("rage=50&sun=0|sun=120"), &row),
        Truth::False
    );
    let invalid_shape = MacroCondition::Or(
        Box::new(MacroCondition::And(
            Box::new(MacroCondition::Rage(CmpOp::Eq, 50)),
            Box::new(MacroCondition::Berserk(CmpOp::Eq, 0)),
        )),
        Box::new(MacroCondition::Berserk(CmpOp::Eq, 120)),
    );
    assert!(score(invalid_shape, &[row], "盾击", 3).is_none());
}

#[test]
fn unsupported_name_inequality_is_never_accepted_with_changed_meaning() {
    let data: InputState =
        serde_json::from_value(json!({"buffs":[{"name":"血怒","stacks":2,"remaining":10}]}))
            .unwrap();
    assert!(score(
        MacroCondition::BuffStack("血怒".into(), CmpOp::Neq, 1),
        &[sample(&data)],
        "盾击",
        1
    )
    .is_none());
}

#[test]
fn scores_expose_unknown_samples_and_do_not_manufacture_negative_evidence() {
    let result = analyze(&request(
        vec![
            cast("绝刀", state(50, 120)),
            cast("盾击", json!({})),
            cast("绝刀", json!({})),
            cast("盾击", state(50, 0)),
        ],
        0,
        0,
        0,
    ))
    .unwrap();
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| contains_expression(candidate, "sun=120"))
        .unwrap();
    assert_eq!((candidate.tp, candidate.fp, candidate.r#fn), (1, 0, 0));
    assert_eq!(candidate.unknown_positive_indices, vec![2]);
    assert_eq!(candidate.unknown_negative_indices, vec![1]);
    assert_eq!(candidate.coverage, 0.5);
    assert_eq!(candidate.precision, Some(1.0));
    assert_eq!(candidate.false_match_rate, Some(0.0));
}

#[test]
fn results_are_stable_and_every_macro_round_trips_to_its_reported_semantics() {
    let req = request(
        vec![
            cast("绝刀", state(50, 120)),
            cast("盾击", state(30, 100)),
            cast("绝刀", state(50, 120)),
        ],
        0,
        0,
        0,
    );
    let first = analyze(&req).unwrap();
    let second = analyze(&req).unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    for candidate in first.candidates {
        assert!(candidate.chars <= 128);
        assert_eq!(candidate.chars, candidate.macro_text.encode_utf16().count());
        let config = crate::macro_parser::parse_macro_text(&candidate.macro_text).unwrap();
        let line = &config.pages[0].lines[0];
        assert_eq!(line.action.skill_name(), "绝刀");
        if let Some(condition) = &line.condition {
            assert_eq!(condition.semantic_string(), candidate.semantic_expression);
        } else {
            assert_eq!(candidate.terms, 0);
        }
    }
}

#[test]
fn rejects_virtual_actions_invalid_indices_and_unbounded_search_settings() {
    let mut req = request(vec![json!({"name":"移除气劲", "skill_id":90001})], 0, 0, 0);
    assert!(analyze(&req).unwrap_err().contains("虚拟操作"));
    req = request(vec![cast("盾击", state(0, 0))], 0, 1, 0);
    assert!(analyze(&req).is_err());
    req.selection_end = 0;
    req.options.max_terms = 4;
    assert!(analyze(&req).is_err());
}

#[test]
fn no_negative_sample_is_reported_without_an_undefined_rate() {
    let result = analyze(&request(vec![cast("盾击", state(0, 0))], 0, 0, 0)).unwrap();
    assert_eq!(result.negative_count, 0);
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.false_match_rate.is_none()));
}

#[test]
fn mist_variants_match_their_own_skill_and_preserve_copyable_names() {
    let result = analyze(&request(
        vec![
            json!({"name":"阵云结晦·雾海", "skill_id":90010,"state_before":state(50,120)}),
            json!({"name":"阵云结晦", "skill_id":30769,"state_before":state(50,120)}),
            json!({"name":"阵云结晦·雾海", "skill_id":90010,"state_before":state(50,120)}),
        ],
        0,
        0,
        0,
    ))
    .unwrap();
    assert_eq!(result.positives, vec![0, 2]);
    assert_eq!(result.selection.step_target, "阵云结晦·雾海");
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.macro_text.ends_with(" 阵云结晦·雾海")));
    let snapshot: InputState = serde_json::from_value(json!({"skill_states":[
        {"name":"阵云结晦·雾海", "skill_id":90010,"not_in_cd":true}]}))
    .unwrap();
    assert_eq!(
        eval(
            &condition("skill_notin_cd:阵云结晦·雾海"),
            &sample(&snapshot)
        ),
        Truth::True
    );
    assert_eq!(
        eval(&condition("skill_notin_cd:阵云结晦"), &sample(&snapshot)),
        Truth::Unknown
    );
}

#[test]
fn last_skill_after_mist_variant_matches_runtime_base_name() {
    let result = analyze(&request(
        vec![
            json!({"name":"阵云结晦·雾海", "skill_id":90010,"state_before":state(50,120)}),
            cast("月照连营", state(50, 120)),
            cast("盾击", state(50, 120)),
        ],
        1,
        1,
        0,
    ))
    .unwrap();
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| contains_expression(candidate, "last_skill=阵云结晦"))
        .unwrap();
    assert_eq!(candidate.matched_positive_indices, vec![1]);
    assert_eq!(candidate.fp, 0);
    assert!(!result
        .candidates
        .iter()
        .any(|candidate| { contains_expression(candidate, "last_skill=阵云结晦·雾海") }));
}

#[test]
fn equivalent_conditions_are_collapsed_but_other_spellings_remain_available() {
    let result = analyze(&request(
        vec![cast("阵云结晦", state(50, 120)), cast("盾击", state(50, 0))],
        0,
        0,
        0,
    ))
    .unwrap();
    let group = result
        .candidates
        .iter()
        .find(|candidate| contains_expression(candidate, "sun=120"))
        .unwrap();
    assert!(contains_expression(group, "sun>=120"));
    assert!(group.equivalent_expressions.len() > 1);
    assert!(result.candidates.len() < result.search.atoms_evaluated);
}

#[test]
fn dense_buff_boundaries_cannot_evict_resource_charge_or_last_skill_families() {
    let inputs: Vec<InputState> = (0..80).map(|index| serde_json::from_value(json!({
        "rage":index,"berserk_value":120-index,"block_value":index,
        "buffs":(0..32).map(|buff|json!({"name":PREFERRED_BUFF_NAMES[buff],"buff_id":crate::macro_eval::buff_name_to_id(PREFERRED_BUFF_NAMES[buff]),
            "remaining":(index*32+buff) as f64 / 1000.0 + 1.0,"stacks":index%7})).collect::<Vec<_>>(),
        "target_buffs":(0..32).map(|buff|json!({"name":PREFERRED_BUFF_NAMES[buff],"buff_id":crate::macro_eval::buff_name_to_id(PREFERRED_BUFF_NAMES[buff]),
            "remaining":(index*32+buff) as f64 / 1000.0 + 1.0,"stacks":1})).collect::<Vec<_>>(),
        "skill_states":[{"name":"血怒","charges":index%4,"max_charges":3,"not_in_cd":true}]
    })).unwrap()).collect();
    let rows: Vec<_> = inputs
        .iter()
        .map(|state| Sample {
            last_skill: Some("盾击"),
            ..sample(state)
        })
        .collect();
    let (atoms, truncated) = enumerate_atoms(&rows);
    assert!(truncated);
    assert!(atoms.len() <= MAX_ATOMS);
    for prefix in [
        "sun",
        "rage",
        "energy",
        "skill_energy:",
        "skill_notin_cd:",
        "last_skill",
    ] {
        assert!(
            atoms
                .iter()
                .any(|atom| atom.display_string().starts_with(prefix)),
            "{prefix}"
        );
    }
}

#[test]
fn handles_2048_observations_with_bounded_search_and_output() {
    let timeline: Vec<_> = (0..MAX_EVENTS)
        .map(|index| {
            let mut data = state((index % 101) as i32, (index % 121) as i32);
            data["buffs"] =
                json!([{"name":"血怒","remaining":((index%160)+1) as f64/16.0,"stacks":1}]);
            data["skill_states"] =
                json!([{"name":"血怒","charges":index%4,"max_charges":3,"not_in_cd":index%4>0}]);
            cast(if index % 8 == 0 { "绝刀" } else { "盾击" }, data)
        })
        .collect();
    let mut req = request(timeline, 0, 0, 0);
    req.options.max_candidates = 120;
    let started = std::time::Instant::now();
    let result = analyze(&req).unwrap();
    eprintln!(
        "macro_assist scale: {} observations, {} atoms, {} compounds, {} groups, {:.3}s",
        MAX_EVENTS,
        result.search.atoms_evaluated,
        result.search.compounds_evaluated,
        result.search.total_candidates,
        started.elapsed().as_secs_f64()
    );
    assert_eq!(result.active_count, MAX_EVENTS);
    assert!(result.candidates.len() <= 120);
    assert!(result.search.atoms_evaluated <= MAX_ATOMS);
    assert!(result.search.compounds_evaluated <= BEAM_WIDTH * BEAM_WIDTH * 2);
}

#[test]
fn readable_buff_names_resolve_aliases_by_identity_and_skip_internal_ids() {
    let make = |name: &str, id: Option<u32>| -> InputBuff {
        serde_json::from_value(json!({"name":name,"buff_id":id})).unwrap()
    };
    for (source, preferred) in [
        ("麟光玄甲", "麟光甲"),
        ("橙武", "天下宏愿"),
        ("驭焰", "天下宏愿"),
        ("铁骨·宿敌", "宿敌"),
        ("切换至盾姿态", "擎盾"),
        ("切换至刀姿态", "擎刀"),
    ] {
        let id = crate::macro_eval::buff_name_to_id(source).unwrap();
        let actual = buff_name(&make(source, Some(id))).unwrap();
        assert_eq!(actual, preferred);
        assert_eq!(crate::macro_eval::buff_name_to_id(&actual), Some(id));
        assert_eq!(
            buff_name(&make(&id.to_string(), Some(id))).as_deref(),
            Some(preferred)
        );
        assert_eq!(buff_name(&make(source, None)).as_deref(), Some(preferred));
    }
    let armor = crate::macro_eval::buff_name_to_id("麟光甲").unwrap();
    // A misleading display name must not override an explicit identity.
    assert_eq!(
        buff_name(&make("血怒", Some(armor))).as_deref(),
        Some("麟光甲")
    );
    for name in ["内部秘籍状态", "3489660934", "血怒"] {
        assert!(buff_name(&make(name, Some(3489660934))).is_none());
    }
    assert!(buff_name(&make("无已知宏别名", None)).is_none());
}

#[test]
fn numeric_buff_macros_and_precise_user_thresholds_remain_runtime_compatible() {
    let snapshot: InputState = serde_json::from_value(json!({"buffs":[
        {"name":"内部秘籍状态","buff_id":3489660934u32,"remaining":17.625,"stacks":2}]}))
    .unwrap();
    assert_eq!(
        eval(&condition("buff:3489660934"), &sample(&snapshot)),
        Truth::True
    );
    assert_eq!(
        eval(
            &condition("bufftime:3489660934>=17.625"),
            &sample(&snapshot)
        ),
        Truth::True
    );
    assert!(score(
        condition("buff:3489660934"),
        &[sample(&snapshot)],
        "盾击",
        1
    )
    .is_none());
}

fn timed_state(time: f64) -> Value {
    let mut snapshot = state(50, 120);
    snapshot["buffs"] = json!([{"name":"血怒","remaining":time,"stacks":1}]);
    snapshot["target_buffs"] = json!([{"name":"虚弱","remaining":time,"stacks":1}]);
    snapshot
}

#[test]
fn tenth_second_thresholds_recompute_coverage_from_unrounded_snapshots() {
    let req = request(
        vec![
            cast("绝刀", timed_state(17.625)),
            cast("盾击", timed_state(17.61)),
            cast("绝刀", timed_state(17.875)),
            cast("盾击", timed_state(17.59)),
            cast("盾击", timed_state(17.7)),
        ],
        0,
        0,
        0,
    );
    let result = analyze(&req).unwrap();
    for prefix in ["bufftime:血怒", "tbufftime:虚弱"] {
        let greater = result
            .candidates
            .iter()
            .find(|item| contains_expression(item, &format!("{prefix}>=17.6")))
            .unwrap();
        assert_eq!((greater.tp, greater.fp, greater.r#fn), (2, 2, 0));
        assert_eq!(greater.matched_negative_indices, vec![1, 4]);
        let lesser = result
            .candidates
            .iter()
            .find(|item| contains_expression(item, &format!("{prefix}<17.7")))
            .unwrap();
        assert_eq!((lesser.tp, lesser.fp, lesser.r#fn), (1, 2, 1));
        assert_eq!(lesser.matched_positive_indices, vec![0]);
        assert_eq!(lesser.matched_negative_indices, vec![1, 3]);
        // A mere display truncation of 17.625 would keep different statistics.
        let before = sample(req.timeline[0].state_before.as_ref().unwrap());
        let outside = sample(req.timeline[1].state_before.as_ref().unwrap());
        assert_eq!(
            eval(&condition(&format!("{prefix}<17.625")), &before),
            Truth::False
        );
        assert_eq!(
            eval(&condition(&format!("{prefix}>=17.625")), &outside),
            Truth::False
        );
    }
    assert_eq!(
        req.timeline[0]
            .state_before
            .as_ref()
            .unwrap()
            .buffs
            .as_ref()
            .unwrap()[0]
            .remaining,
        Some(17.625)
    );
    // The engine's >= tolerance is preserved while strict < is still strict.
    let edge: InputState = serde_json::from_value(timed_state(17.5995)).unwrap();
    assert_eq!(
        eval(&condition("bufftime:血怒>=17.6"), &sample(&edge)),
        Truth::True
    );
    assert_eq!(
        eval(&condition("bufftime:血怒<17.6"), &sample(&edge)),
        Truth::True
    );
    let exact: InputState = serde_json::from_value(timed_state(17.6)).unwrap();
    assert_eq!(
        eval(&condition("bufftime:血怒<17.6"), &sample(&exact)),
        Truth::False
    );
}

fn assert_readable_tenth_buff_tree(atom: &MacroCondition) -> (usize, usize) {
    use MacroCondition::*;
    match atom {
        And(left, right) | Or(left, right) => {
            let a = assert_readable_tenth_buff_tree(left);
            let b = assert_readable_tenth_buff_tree(right);
            (a.0 + b.0, a.1 + b.1)
        }
        Buff(name) | NoBuff(name) | BuffStack(name, ..) | TBuff(name) | TnoBuff(name) => {
            assert!(
                name.chars().any(|character| !character.is_ascii_digit()),
                "numeric buff: {name}"
            );
            assert!(crate::macro_eval::buff_name_to_id(name).is_some());
            (1, 0)
        }
        BuffTime(name, _, threshold) | TBuffTime(name, _, threshold) => {
            assert!(
                name.chars().any(|character| !character.is_ascii_digit()),
                "numeric buff: {name}"
            );
            assert!(crate::macro_eval::buff_name_to_id(name).is_some());
            let decimal = threshold.to_string();
            let parts: Vec<_> = decimal.split('.').collect();
            assert!(
                parts.len() == 1 || parts[1].len() == 1,
                "precision: {decimal}"
            );
            assert!((threshold * 10.0 - (threshold * 10.0).round()).abs() < 1e-8);
            (1, 1)
        }
        _ => (0, 0),
    }
}

#[test]
fn all_candidate_and_equivalent_trees_use_names_and_single_decimal_thresholds() {
    let armor_id = crate::macro_eval::buff_name_to_id("麟光甲").unwrap();
    let times = [17.625, 17.61, 17.875, 17.59, 17.7, 17.999, 18.06];
    let timeline = times.iter().enumerate().map(|(index, time)| {
        let mut snapshot = timed_state(*time);
        snapshot["rage"] = json!(index * 10);
        snapshot["buffs"].as_array_mut().unwrap().extend([
            json!({"name":"3489660934","buff_id":3489660934u32,"remaining":*time,"stacks":index+1}),
            json!({"name":armor_id.to_string(),"buff_id":armor_id,"remaining":*time,"stacks":index%3+1}),
        ]);
        snapshot["target_buffs"].as_array_mut().unwrap().push(
            json!({"name":"内部目标状态","buff_id":3489660934u32,"remaining":*time,"stacks":1}));
        cast(if index%2==0 { "绝刀" } else { "盾击" }, snapshot)
    }).collect();
    let mut req = request(timeline, 0, 0, 0);
    req.options.max_terms = 3;
    let result = analyze(&req).unwrap();
    let mut buff_atoms = 0;
    let mut time_atoms = 0;
    let mut compound_expressions = 0;
    for candidate in &result.candidates {
        let copied = crate::macro_parser::parse_macro_text(&candidate.macro_text).unwrap();
        if let Some(atom) = &copied.pages[0].lines[0].condition {
            assert_readable_tenth_buff_tree(atom);
        }
        for expression in std::iter::once(&candidate.expression)
            .chain(&candidate.equivalent_expressions)
            .filter(|text| !text.is_empty())
        {
            let parsed = condition(expression);
            let counts = assert_readable_tenth_buff_tree(&parsed);
            buff_atoms += counts.0;
            time_atoms += counts.1;
            compound_expressions += usize::from(matches!(
                parsed,
                MacroCondition::And(..) | MacroCondition::Or(..)
            ));
            // Every equivalent spelling must actually retain this group's
            // observed match/unknown counts after the real parser round trip.
            let truth: Vec<_> = req
                .timeline
                .iter()
                .enumerate()
                .map(|(index, event)| {
                    eval(
                        &parsed,
                        &Sample {
                            index,
                            positive: result.positives.contains(&index),
                            same_skill_outside_combo: false,
                            state: event.state_before.as_ref(),
                            last_skill: index
                                .checked_sub(1)
                                .map(|previous| base_name(&req.timeline[previous].name)),
                        },
                    )
                })
                .collect();
            assert_eq!(
                truth
                    .iter()
                    .enumerate()
                    .filter(
                        |(index, value)| result.positives.contains(index) && **value == Truth::True
                    )
                    .count(),
                candidate.tp
            );
            assert_eq!(
                truth
                    .iter()
                    .enumerate()
                    .filter(|(index, value)| !result.positives.contains(index)
                        && **value == Truth::True)
                    .count(),
                candidate.fp
            );
            assert!(!expression.contains("3489660934"));
        }
    }
    assert!(buff_atoms > 0 && time_atoms > 0 && compound_expressions > 0);
}

#[test]
fn outside_combo_same_skill_evidence_ranks_a_separator_above_skill_identity_only() {
    let buff_state = |rage, stacks| {
        let mut snapshot = state(rage, 120);
        snapshot["buffs"] = json!([{"name":"血怒","remaining":10,"stacks":stacks}]);
        snapshot
    };
    let mut timeline = vec![
        cast("盾击", buff_state(0, 0)),
        cast("血怒", buff_state(50, 2)),
    ];
    timeline.extend((0..8).map(|_| cast("绝刀", buff_state(0, 2))));
    timeline.extend([
        cast("血怒", buff_state(50, 1)),
        cast("盾击", buff_state(0, 0)),
        cast("血怒", buff_state(50, 2)),
    ]);
    let result = analyze(&request(timeline, 0, 1, 1)).unwrap();
    assert_eq!(result.positives, vec![1, 12]);
    assert_eq!(result.same_skill_outside_combo_indices, vec![10]);
    assert!(result.same_skill_other_step_indices.is_empty());
    let separating_index = result
        .candidates
        .iter()
        .position(|item| contains_expression(item, "buff:血怒=2"))
        .unwrap();
    let identity_index = result
        .candidates
        .iter()
        .position(|item| contains_expression(item, "rage=50"))
        .unwrap();
    let separating = &result.candidates[separating_index];
    let identity = &result.candidates[identity_index];
    assert!(
        separating.f1 < identity.f1,
        "fixture must oppose the global ranking"
    );
    assert!(separating_index < identity_index);
    assert_eq!(separating.same_skill_outside_combo_f1, Some(1.0));
    assert_eq!(identity.same_skill_outside_combo_f1, Some(0.8));
    assert_eq!(identity.same_skill_outside_combo_fp, 1);
    assert_eq!(identity.matched_same_skill_outside_combo_indices, vec![10]);
}

#[test]
fn overlapping_combos_partition_same_skill_other_steps_without_overriding_positives() {
    let timeline: Vec<_> = ["血怒", "盾击", "血怒", "盾击", "血怒", "绝刀", "血怒"]
        .iter()
        .map(|name| cast(name, state(50, 120)))
        .collect();
    for (step, positives, other_steps) in [(0, vec![0, 2], vec![4]), (2, vec![2, 4], vec![0])] {
        let result = analyze(&request(timeline.clone(), 0, 2, step)).unwrap();
        assert_eq!(result.positives, positives);
        assert_eq!(result.same_skill_outside_combo_indices, vec![6]);
        assert_eq!(result.same_skill_other_step_indices, other_steps);
        assert_eq!(result.other_skill_negative_indices, vec![1, 3, 5]);
        let all: Vec<_> = result
            .positives
            .iter()
            .chain(&result.same_skill_outside_combo_indices)
            .chain(&result.same_skill_other_step_indices)
            .chain(&result.other_skill_negative_indices)
            .copied()
            .collect();
        assert_eq!(all.len(), result.active_count);
        assert_eq!(
            all.iter().copied().collect::<BTreeSet<_>>().len(),
            result.active_count
        );
    }
}

#[test]
fn missing_outside_fields_are_penalized_instead_of_becoming_excluded_negatives() {
    let snapshots: Vec<InputState> = vec![
        json!({"rage":10,"block_value":10}),
        json!({"rage":0}),
        json!({"rage":10,"block_value":0}),
    ]
    .into_iter()
    .map(|value| serde_json::from_value(value).unwrap())
    .collect();
    let rows: Vec<_> = snapshots
        .iter()
        .enumerate()
        .map(|(index, state)| Sample {
            index,
            positive: index == 0,
            same_skill_outside_combo: index == 1,
            state: Some(state),
            last_skill: None,
        })
        .collect();
    let known = score(condition("rage=10"), &rows, "血怒", 1).unwrap();
    let missing = score(condition("energy=10"), &rows, "血怒", 1).unwrap();
    assert_eq!(missing.candidate.same_skill_outside_combo_unknown, 1);
    assert_eq!(missing.candidate.same_skill_outside_combo_fp, 0);
    assert!(known.candidate.f1 < missing.candidate.f1);
    assert_eq!(compare_scored(&known, &missing), std::cmp::Ordering::Less);
}

#[test]
fn absent_outside_same_skill_samples_are_explicitly_unavailable() {
    let result = analyze(&request(
        vec![cast("血怒", state(50, 120)), cast("盾击", state(0, 0))],
        0,
        0,
        0,
    ))
    .unwrap();
    assert!(result.same_skill_outside_combo_indices.is_empty());
    assert!(result
        .candidates
        .iter()
        .all(|item| item.same_skill_outside_combo_f1.is_none()));
    assert!(result
        .limitations
        .iter()
        .any(|text| text.contains("缺少该技能在匹配组合之外")));
}

#[test]
fn whole_program_jointly_covers_repeated_steps_and_retains_same_skill_outside_evidence() {
    let timeline = vec![
        cast("盾击", state(0, 0)),
        cast("血怒", state(10, 0)),
        cast("盾击", state(0, 0)),
        cast("绝刀", state(20, 0)),
        cast("盾击", state(0, 0)),
        cast("血怒", state(10, 0)),
        cast("盾击", state(0, 0)),
        cast("绝刀", state(20, 0)),
        cast("盾击", state(30, 0)),
    ];
    let req = request(timeline, 0, 2, usize::MAX);
    let result = analyze_program(&req).unwrap();
    assert_eq!(result.steps.len(), 3);
    assert_eq!(result.steps[0].same_skill_outside_combo_indices, vec![8]);
    assert_eq!(result.steps[2].same_skill_outside_combo_indices, vec![8]);
    assert!(!result.programs.is_empty());
    assert!(result.programs.len() <= MAX_PROGRAMS);
    let best = &result.programs[0];
    assert_eq!((best.matched_occurrences, best.total_occurrences), (2, 2));
    assert_eq!(
        (best.priority_conflicts, best.unknowns, best.outside_matches),
        (0, 0, 0)
    );
    // Joint ranking can reuse one rule for both identical-skill steps. Merely
    // concatenating the independently best step candidates has three lines.
    assert_eq!(best.lines.len(), 2);
    let independent = result
        .steps
        .iter()
        .map(|step| step.candidates[0].macro_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(best.macro_text, independent);
    for program in &result.programs {
        assert_eq!(program.chars, program.macro_text.encode_utf16().count());
        let parsed = crate::macro_parser::parse_macro_text(&program.macro_text).unwrap();
        for line in parsed.pages.iter().flat_map(|page| &page.lines) {
            if let Some(condition) = &line.condition {
                assert_readable_tenth_buff_tree(condition);
            }
        }
    }
    assert_program_text_matches_metrics(&req, &result);
}

/// Re-parse the delivered macro and check it directly against snapshots. This
/// deliberately does not read candidate indices or the joint beam's decisions.
fn assert_program_text_matches_metrics(request: &AssistRequest, result: &ProgramResult) {
    let active: Vec<_> = request
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect();
    let combo: BTreeSet<_> = result.steps[0]
        .occurrences
        .iter()
        .flat_map(|occurrence| occurrence.start_active_index..=occurrence.end_active_index)
        .collect();
    for program in &result.programs {
        let parsed = crate::macro_parser::parse_macro_text(&program.macro_text).unwrap();
        let lines: Vec<_> = parsed.pages.iter().flat_map(|page| &page.lines).collect();
        let mut correct = BTreeSet::new();
        let mut conflicts = 0;
        let mut unknowns = 0;
        let mut outside = 0;
        for (index, event) in active.iter().enumerate() {
            let row = Sample {
                index,
                positive: combo.contains(&index),
                same_skill_outside_combo: false,
                state: event.state_before.as_ref(),
                last_skill: index
                    .checked_sub(1)
                    .map(|previous| base_name(&active[previous].name)),
            };
            let truth: Vec<_> = lines
                .iter()
                .map(|line| {
                    line.condition
                        .as_ref()
                        .map_or(Truth::True, |condition| eval(condition, &row))
                })
                .collect();
            if combo.contains(&index) {
                if let Some((position, first)) = truth
                    .iter()
                    .enumerate()
                    .find(|(_, value)| **value != Truth::False)
                {
                    if *first == Truth::Unknown {
                        unknowns += 1;
                    } else if lines[position].action.skill_name() == event_name(event) {
                        correct.insert(index);
                    } else {
                        conflicts += 1;
                    }
                }
            } else if lines.iter().zip(&truth).any(|(line, value)| {
                line.action.skill_name() == event_name(event) && *value == Truth::True
            }) {
                outside += 1;
            }
        }
        let matched = result.steps[0]
            .occurrences
            .iter()
            .filter(|occurrence| {
                (occurrence.start_active_index..=occurrence.end_active_index)
                    .all(|index| correct.contains(&index))
            })
            .count();
        assert_eq!(
            (
                program.matched_occurrences,
                program.priority_conflicts,
                program.unknowns,
                program.outside_matches
            ),
            (matched, conflicts, unknowns, outside),
            "{}",
            program.macro_text
        );
    }
}

#[test]
fn whole_program_report_matches_emitted_priority_with_missing_fields_and_outside_skills() {
    let req = request(
        vec![
            cast("盾击", json!({"rage":0})),
            cast("血怒", state(20, 0)),
            cast("绝刀", state(20, 0)),
            cast("盾击", state(10, 0)),
            cast("血怒", json!({"rage":20})),
            cast("绝刀", json!({})),
            cast("盾击", json!({})),
        ],
        0,
        1,
        0,
    );
    let result = analyze_program(&req).unwrap();
    assert_program_text_matches_metrics(&req, &result);
}

#[test]
fn whole_program_overlapping_occurrences_do_not_duplicate_conflict_samples() {
    let result = analyze_program(&request(vec![cast("盾击", state(0, 0)); 4], 0, 2, 0)).unwrap();
    let best = &result.programs[0];
    assert_eq!(best.total_occurrences, 2);
    assert_eq!(best.matched_occurrences, 2);
    assert_eq!(best.lines.len(), 1);
    assert_eq!(best.priority_conflicts, 0);
    assert_eq!(best.outside_matches, 0);
}

#[test]
fn whole_program_unknown_priority_is_not_counted_as_matching_or_conflicting() {
    let mut beam = ProgramBeam {
        lines: vec![0, 1],
        decisions: vec![
            ProgramDecision {
                skill: Some(0),
                unknown: false,
            },
            ProgramDecision {
                skill: Some(1),
                unknown: true,
            },
            ProgramDecision {
                skill: Some(0),
                unknown: false,
            },
        ],
        outside: vec![Truth::False, Truth::False, Truth::True],
        matched: 0,
        correct: 0,
        conflicts: 0,
        unknowns: 0,
        outside_matches: 0,
        outside_unknowns: 0,
        chars: 20,
    };
    let occurrences = vec![Occurrence {
        start_active_index: 0,
        end_active_index: 1,
        step_active_index: 0,
        cast_time: None,
        snapshot_time: None,
    }];
    score_program(
        &mut beam,
        &[0, 1, 0],
        &[true, true, false],
        &BTreeSet::from([0, 1]),
        &occurrences,
    );
    assert_eq!(
        (beam.matched, beam.correct, beam.conflicts, beam.unknowns),
        (0, 1, 0, 1)
    );
    assert_eq!(beam.outside_matches, 1);
    beam.decisions[1] = ProgramDecision {
        skill: Some(0),
        unknown: false,
    };
    score_program(
        &mut beam,
        &[0, 1, 0],
        &[true, true, false],
        &BTreeSet::from([0, 1]),
        &occurrences,
    );
    assert_eq!((beam.matched, beam.conflicts, beam.unknowns), (0, 1, 0));
}

#[test]
fn whole_program_preserves_selection_and_event_budgets() {
    assert!(analyze_program(&request(vec![cast("盾击", state(0, 0)); 33], 0, 32, 0)).is_err());
    assert!(analyze_program(&request(vec![cast("盾击", state(0, 0)); 2049], 0, 0, 0)).is_err());
    assert!(analyze_program(&request(vec![], 0, 0, 0)).is_err());
    assert!(analyze_program(&request(vec![cast("盾击", state(0, 0))], 1, 0, 0)).is_err());
}
