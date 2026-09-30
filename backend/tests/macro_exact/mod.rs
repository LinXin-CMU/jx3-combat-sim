use super::*;

fn scene() -> Request {
    serde_json::from_str(include_str!("../fixtures/exact_macro_short.json")).unwrap()
}

#[test]
fn active_cast_acceptance_keeps_state_diagnostics_and_rejects_wrong_actions() {
    let mut emitter = ScriptEmitter::new();
    emitter.emit("盾回", 13051, 10.0);
    let mut expected = emitter.events.pop().unwrap();
    expected.triggered = false;
    expected.state_after = Some(snapshot_event_state(&Player::new(0, vec![], vec![])));
    let mut actual = expected.clone();
    actual.cast_time += 0.0065;
    actual.state_after.as_mut().unwrap().rage += 1;
    let comparison = compare_with_policy(&[expected.clone()], &[actual.clone()], 0.125, Acceptance::SkillsAndTime);
    assert_eq!(comparison["reproduced"], true);
    assert_eq!(comparison["state_reproduced"], false);
    assert_eq!(comparison["state_prefix"], 0);
    assert_eq!(compare_with_policy(&[expected.clone()], &[actual.clone()], 0.125, Acceptance::SkillsAndState)["reproduced"], false);
    actual.name = "盾飞".into();
    assert_eq!(compare_with_policy(&[expected], &[actual], 0.125, Acceptance::SkillsAndTime)["reproduced"], false);
}

#[test]
fn search_alphabet_canonicalizes_only_scenes_without_channels() {
    let mut request = scene();
    request.acceptance = Acceptance::SkillsAndTime;
    let result = run(request).unwrap();
    assert!(result["actions"].as_array().unwrap().iter().all(|a| a["fcast"] == false));
    let mut request = scene();
    request.acceptance = Acceptance::SkillsAndTime;
    request.simulation.sequence = vec!["盾舞".into()];
    request.simulation.timing_offsets.clear();
    request.simulation.channel_ticks.clear();
    let result = run(request).unwrap();
    // Channel alphabets must retain interruption commands even when this
    // manual channel fixture reports a separate teacher-path mismatch.
    assert!(result["actions"].as_array().unwrap().iter().any(|a| a["fcast"] == true));
}

#[test]
fn compact_session_preserves_truth_and_invalidates_changed_scene() {
    let mut cache = None;
    let full = run_cached(scene(), &mut cache).unwrap();
    let mut request = scene();
    request.compact_result = true;
    let compact = run_cached(request, &mut cache).unwrap();
    assert_eq!(compact["timings_ms"]["scene_cache_hit"], true);
    assert_eq!(compact["comparison"], full["comparison"]);
    assert_eq!(compact["actual_fingerprint"], full["actual_fingerprint"]);
    for (a,b) in full["rows"].as_array().unwrap().iter().zip(compact["rows"].as_array().unwrap()) {
        let hex = b["truth_hex"].as_str().unwrap();
        for (i,truth) in a["truth"].as_array().unwrap().iter().enumerate() {
            let byte = u8::from_str_radix(&hex[i/8*2..i/8*2+2],16).unwrap();
            assert_eq!(truth.as_bool().unwrap(), byte & (1 << (i%8)) != 0);
        }
        assert!(b.get("truth").is_none());
        assert_eq!(a["allowed"], b["allowed"]);
    }
    let mut changed = scene();
    changed.simulation.network_delay += 1;
    let changed = run_cached(changed, &mut cache).unwrap();
    assert_eq!(changed["timings_ms"]["scene_cache_hit"], false);
}

#[test]
fn early_failure_stops_only_candidate_and_preserves_first_difference() {
    let mut cache = None;
    let mut request = scene();
    request.candidate = Some("/cast 血怒\n/cast 盾刀".into());
    let full = run_cached(request, &mut cache).unwrap();
    let mut request = scene();
    request.candidate = Some("/cast 血怒\n/cast 盾刀".into());
    request.stop_on_divergence = true;
    let fast = run_cached(request, &mut cache).unwrap();
    assert_eq!(fast["status"], "ok");
    assert_eq!(fast["comparison"]["completed_full_replay"], false);
    assert_eq!(fast["comparison"]["reproduced"], false);
    assert_eq!(fast["comparison"]["first_difference"], full["comparison"]["first_difference"]);
    assert!(fast["actual"].as_array().unwrap().len() < full["actual"].as_array().unwrap().len());
    assert_eq!(run_cached(scene(), &mut cache).unwrap()["comparison"]["reproduced"], true);
}

#[test]
fn unfinished_channel_metadata_alone_does_not_trigger_early_stop() {
    let mut emitter = ScriptEmitter::new();
    emitter.emit("盾舞", 13048, 0.0);
    let mut expected = emitter.events.pop().unwrap();
    expected.triggered = false;
    expected.channel_ticks = Some(1);
    let mut actual = expected.clone();
    actual.channel_ticks = Some(4);
    let mut probe = Probe::new(false, vec![expected], vec![], &["rage=0".into()], 0.125).unwrap();
    probe.stop_on_divergence = true;
    probe.finish(&CastOutcome { events: vec![actual], cast_success: true }, "盾舞", false);
    assert!(!probe.should_stop());
}

#[test]
fn fast_atom_observation_keeps_native_truth_and_last_skill_semantics() {
    let mut player = Player::new(0, vec![], vec![]);
    player.add_buff_with_stacks(BUFF_XUE_NU, 1, 0);
    let parsed = macro_parser::parse_macro_text(
        "/cast [bufftime:血怒>100.0] 盾刀\n/cast [bufftime:援戈<100.0] 盾刀\n/cast [rage<0&rage=0|last_skill=盾回] 盾刀\n/cast [last_skill=盾回] 盾刀"
    ).unwrap();
    let map = HashMap::new();
    let ids = HashMap::new();
    let fast = evaluate_condition_truths(&parsed.pages[0], &player, &map, &ids, Some("盾回".into()));
    let (_, last, debug) = macro_eval::evaluate_phase1(&parsed.pages[0], &player, &map, &ids, Some("盾回".into()), true);
    assert_eq!(fast, vec![true, false, false, true]);
    assert_eq!(fast, debug.iter().map(|r| r.passed).collect::<Vec<_>>());
    assert_eq!(last.as_deref(), Some("盾回"));
}

#[test]
fn tolerance_accepts_timer_drift_but_not_resources_or_accumulation() {
    let mut emitter = ScriptEmitter::new();
    emitter.emit("盾回", 13051, 10.0);
    let mut expected = emitter.events.pop().unwrap();
    expected.triggered = false;
    expected.state_after = Some(snapshot_event_state(&Player::new(0, vec![], vec![])));
    let mut actual = expected.clone();
    actual.cast_time += 0.002;
    actual.state_after.as_mut().unwrap().time += 0.002;
    assert!(same_cast_with_tolerance(&expected, &actual, 0.0625));
    assert!(same_state_with_tolerance(&expected, &actual, 0.0625));
    assert!(!same_cast(&expected, &actual));
    actual.state_after.as_mut().unwrap().rage += 1;
    assert!(!same_state_with_tolerance(&expected, &actual, 0.0625));
    let reference: Vec<_> = (0..3).map(|i| { let mut e=expected.clone(); e.cast_time+=i as f64; e }).collect();
    let candidate: Vec<_> = reference.iter().enumerate().map(|(i,e)| { let mut e=e.clone(); e.cast_time+=i as f64*0.05; e }).collect();
    let comparison = compare_with_tolerance(&reference, &candidate, 0.0625);
    assert_eq!(comparison["exact_prefix"], 2);
    assert_eq!(comparison["reproduced"], false);
}

#[test]
fn tolerant_labels_never_advance_the_reference_teacher() {
    let mut req = scene();
    req.time_tolerance_seconds = 0.0625;
    req.simulation.network_delay = 23;
    let result = run(req).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["comparison"]["strict_reproduced"], true);
    assert_eq!(result["comparison"]["max_time_error_on_order_prefix"], 0.0);
}

#[test]
fn delayed_auxiliary_cast_keeps_tick_and_expiry_in_time_order() {
    let mut req = scene();
    req.version = GameVersion::AnYingQianJi;
    req.simulation.sequence = vec!["盾击", "盾击", "盾击", "盾击", "业火麟光", "盾飞", "血怒"]
        .into_iter().map(str::to_owned).collect();
    req.simulation.talents = vec![13090, 36058, 34912, 36205, 21281, 30769, 22897, 14838, 37239];
    req.simulation.haste_level = 28816;
    req.simulation.timing_offsets.insert("6".into(), 1.25);
    let result = run(req).unwrap();
    assert_eq!(result["status"], "ok", "{}", result["probe_failure"]);
    assert_eq!(result["comparison"]["state_prefix"], 7);
    let weak = result["target"][6]["state_after"]["target_buffs"].as_array().unwrap()
        .iter().find(|b| b["buff_id"] == BUFF_XU_RUO).unwrap();
    assert!((weak["remaining"].as_f64().unwrap() - 24.75).abs() < EPS);
}

#[test]
fn exact_oracle_retains_off_gcd_actions_and_uses_real_combo_resolution() {
    let result = run(scene()).unwrap();
    assert_eq!(result["status"], "ok", "{}", result["skipped"]);
    assert_eq!(
        result["comparison"]["reproduced"], true,
        "{}",
        result["comparison"]
    );
    let names: Vec<_> = result["target"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"血怒"));
    assert!(names.contains(&"月照连营"));
    assert_eq!(names.len(), 23);
    assert!(!result["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|atom| atom.as_str().unwrap().contains("step")));
}

#[test]
fn early_cast_is_a_wait_counterexample_and_diverged_suffix_is_not_labeled() {
    let mut request = scene();
    request.candidate = Some("/cast 血怒\n/cast 盾刀".into());
    let result = run(request).unwrap();
    assert_eq!(result["comparison"]["exact_prefix"], 0);
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["rows"][0]["cursor"], 0);
}

#[test]
fn unknown_skills_cannot_be_silently_dropped_from_target() {
    let mut request = scene();
    request.simulation.sequence.push("not-a-skill".into());
    assert_eq!(run(request).unwrap()["status"], "invalid_target");
}

#[test]
fn empty_macro_waits_are_observed_and_late_states_are_not_labeled() {
    let mut request = scene();
    request.candidate = Some("/cast [rage<0] 盾刀".into());
    let result = run(request).unwrap();
    assert_eq!(result["comparison"]["actual_count"], 0);
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert!(!result["rows"][0]["allowed"].as_array().unwrap().is_empty());
    assert_eq!(result["probe_failure"]["kind"], "missed_decision_time");
    let last = result["rows"].as_array().unwrap().last().unwrap();
    assert!(last["wait_next_time"].as_f64().unwrap() > last["decision_latest"].as_f64().unwrap());
    assert!(last["time"].as_f64().unwrap() <= last["decision_latest"].as_f64().unwrap());
}

#[test]
fn failed_transition_removes_only_executed_action_from_aligned_row() {
    let player = Player::new(0, vec![], vec![]);
    let mut emitter = ScriptEmitter::new();
    emitter.emit("盾回", 13051, 10.0);
    let mut expected = emitter.events.pop().unwrap();
    expected.triggered = false;
    expected.state_after = Some(snapshot_event_state(&player));
    let mut actual = expected.clone();
    actual.state_after.as_mut().unwrap().rage += 1;
    let mut probe = Probe::new(false, vec![expected], vec![
        Action { name: "盾回".into(), fcast: false },
        Action { name: "盾回".into(), fcast: true },
    ], &["rage=0".into()], 0.125).unwrap();
    probe.rows.push(Row {
        time: 10.0, cursor: 0, last_skill: None, state: json!({}), truth: vec![true],
        executable: vec![true, true], outcomes: vec![Some((13051, 10.0)); 2],
        allowed: vec![0, 1], wait_allowed: true, decision_latest: Some(10.125),
        wait_next_time: None, wake_atoms: vec![], rejected_actions: vec![],
    });
    // Rejection is action-specific: fcast can interrupt a channel whereas cast
    // cannot. It would be unsound to blacklist both from the same observation.
    probe.finish(&CastOutcome { events: vec![actual], cast_success: true }, "盾回", true);
    assert_eq!(probe.rows[0].allowed, vec![0]);
    assert_eq!(probe.rows[0].rejected_actions, vec![1]);
    assert_eq!(probe.rows[0].cursor, 0);
    assert_eq!(probe.failure.as_ref().unwrap()["kind"], "state_mismatch");
}

#[test]
fn failed_wait_records_real_threshold_alternative_without_forcing_cast() {
    let mut player = Player::new(0, vec![], vec![]);
    player.add_buff_with_stacks(BUFF_XUE_NU, 1, 160); // expires at ten seconds
    let mut emitter = ScriptEmitter::new();
    emitter.emit("盾回", 13051, 5.0);
    let mut event = emitter.events.pop().unwrap();
    event.is_main = false;
    let mut probe = Probe::new(false, vec![event], vec![],
        &["bufftime:血怒<5.0".into(), "bufftime:血怒>1.0".into()], 0.125).unwrap();
    player.current_time = 4.9;
    probe.rows.push(Row {
        time: 4.9, cursor: 0, last_skill: None, state: json!({}), truth: vec![false, true],
        executable: vec![], outcomes: vec![], allowed: vec![], wait_allowed: true,
        decision_latest: Some(5.125), wait_next_time: None, wake_atoms: vec![], rejected_actions: vec![],
    });
    assert_eq!(probe.next_time(&player, 9.0, false, 0.0), 9.0);
    assert_eq!(probe.rows[0].wake_atoms, vec![0]);
    assert!(probe.rows[0].wait_allowed);
    assert_eq!(probe.rows[0].wait_next_time, Some(9.0));
}

#[test]
fn synthesized_macro_matches_actions_times_states_and_terminal_wait() {
    let text = include_str!("../fixtures/exact_macro_short_verified.txt");
    let mut request = scene();
    request.candidate = Some(text.into());
    let result = run(request).unwrap();
    assert_eq!(
        result["comparison"]["reproduced"], true,
        "{}",
        result["comparison"]
    );
    assert_eq!(result["comparison"]["state_prefix"], 23);
    assert!(result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["cursor"] == 23 && r["time"].as_f64().unwrap() > 24.0));

    // Matching all target casts is insufficient: keep the extra terminal cast.
    let mut request = scene();
    request.simulation.sequence.pop();
    request.candidate = Some(text.into());
    let result = run(request).unwrap();
    assert_eq!(result["comparison"]["exact_prefix"], 22);
    assert_eq!(result["comparison"]["reproduced"], false);
    assert!(result["comparison"]["actual_count"].as_u64().unwrap() > 22);
}

#[test]
fn nonzero_delay_is_part_of_target_and_execution_semantics() {
    let mut request = scene();
    request.simulation.network_delay = 50;
    let result = run(request).unwrap();
    assert_eq!(result["status"], "ok", "{}", result["comparison"]);
    assert_eq!(result["comparison"]["state_prefix"], 23);
    for atom in result["atoms"].as_array().unwrap() {
        if let Some((_, decimals)) = atom.as_str().unwrap().rsplit_once('.') {
            assert_eq!(decimals.len(), 1, "threshold must be quantized before evaluation: {atom}");
        }
    }
}

#[test]
fn one_decimal_verified_macro_retains_exact_timing() {
    let mut request = scene();
    request.candidate = Some(include_str!("../fixtures/exact_macro_short_verified.txt").replace(".500000", ".5").replace(".000000", ".0"));
    assert_eq!(run(request).unwrap()["comparison"]["reproduced"], true);
}

#[test]
fn single_page_is_not_a_reimplementation_of_page_or_condition_rules() {
    let mut request = scene();
    // Equal precedence, right associativity: FALSE & (FALSE | TRUE) is false.
    // The first unfiltered page shadows the later matching shield page.
    request.candidate = Some(
        "#page\n/cast [rage<0&rage<0|rage=0] 血怒\n/cast 盾猛\n#page shield\n/cast 血怒".into(),
    );
    let result = run(request).unwrap();
    assert_eq!(result["actual"][0]["name"], "盾猛");
}
