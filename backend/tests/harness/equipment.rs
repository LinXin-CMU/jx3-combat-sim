use super::*;

fn runtime() -> AgentRuntime {
    AgentRuntime::fixture().with_equipment_fixture()
}

fn empty_slot() -> equip::SlotConfig {
    equip::SlotConfig {
        equip_id: 0,
        strength: 0,
        embedding: vec![],
        enhance_id: 0,
        enchant_id: 0,
    }
}

fn fixture(runtime: &AgentRuntime) -> EquipmentRequest {
    let raw: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut simulation: SimulateRequest =
        serde_json::from_value(raw["simulation"].clone()).unwrap();
    simulation.sequence = vec!["__macro__".into(); 60];
    simulation.macro_text = Some("/cast 盾击\n/cast 盾刀".into());
    simulation.macro_duration = Some(10.0);
    simulation.talents.clear();
    simulation.recipes.clear();
    let mut slots = HashMap::new();
    for position in POSITIONS {
        let mut items = runtime
            .equipment_items()
            .filter(|item| {
                item.sub_type == equip::pos_to_subtype(position)
                    && matches_mount(runtime.mount(), item)
                    && item.require_level <= runtime.context().constants.level
                    && !item.magic_type.contains("(PVP)")
                    && !item.magic_type.contains("(PVX)")
            })
            .collect::<Vec<_>>();
        items.sort_by_key(|item| (item.level, item.id));
        let mut slot = empty_slot();
        slot.equip_id = items.first().expect("real catalog has each position").id;
        slots.insert(position.to_owned(), slot);
    }
    EquipmentRequest {
        simulation,
        version: runtime.game_version(),
        mount: runtime.mount(),
        equipment: EquipmentSnapshot {
            slots,
            stone_id: 0,
            source_label: "test real catalog".into(),
        },
        locked_slots: POSITIONS
            .iter()
            .filter(|p| **p != "HAT")
            .map(|p| p.to_string())
            .collect(),
        candidate_source: CandidateSource::ProvidedIds,
        candidate_ids: BTreeMap::new(),
        min_item_level: None,
        max_item_level: None,
        allowed_sources: vec![],
        haste_min: None,
        haste_max: None,
        max_candidates_per_slot: 12,
        max_simulations: 24,
        wall_time_ms: 60_000,
        max_rounds: 3,
        duration_seconds: 10.0,
    }
}

fn add_candidates(
    request: &mut EquipmentRequest,
    runtime: &AgentRuntime,
    position: &str,
    count: usize,
) {
    let mut items = runtime
        .equipment_items()
        .filter(|i| {
            eligible(request, runtime, position, i)
                && i.id != request.equipment.slots[position].equip_id
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.level.cmp(&a.level).then(a.id.cmp(&b.id)));
    let ids = items
        .into_iter()
        .take(count)
        .map(|i| i.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), count, "real candidate pool must be available");
    request.candidate_ids.insert(position.into(), ids);
}

fn snapshot(request: &EquipmentRequest) -> ScenarioSnapshotV1 {
    ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone()).unwrap()
}
fn execute(request: &EquipmentRequest, runtime: &AgentRuntime) -> Value {
    run(
        request,
        runtime,
        &snapshot(request),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}
fn replay(runtime: &AgentRuntime, simulation: &SimulateRequest) -> crate::SimulateResponse {
    let c = runtime.context();
    crate::simulate_core(
        simulation,
        c.skills,
        c.game_version,
        c.mount,
        c.constants,
        c.recipes,
        c.team_buffs,
        c.formations,
    )
}

#[test]
fn actual_catalog_search_has_full_baseline_and_replayable_complete_candidate() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 2);
    // Deliberately stale UI attributes cannot contaminate the equipment baseline.
    request.simulation.attributes.as_mut().unwrap().base_attack = 999_999.0;
    let original = serde_json::to_value(&request).unwrap();
    let result = execute(&request, &runtime);
    assert_eq!(result["task"], "optimize_equipment");
    assert_eq!(result["simulations"], 3);
    assert_eq!(
        result["baseline"]["equipment"]["slots"]
            .as_object()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        result["baseline"]["equipment"],
        serde_json::to_value(&request.equipment).unwrap()
    );
    assert!(
        result["baseline"]["simulation"]["attributes"]["base_attack"]
            .as_f64()
            .unwrap()
            < 999_999.0
    );
    let best: SimulateRequest =
        serde_json::from_value(result["best"]["simulation"].clone()).unwrap();
    let checked = replay(&runtime, &best);
    assert_eq!(
        checked.fingerprint.to_string(),
        result["best"]["fingerprint"].as_str().unwrap()
    );
    assert_eq!(checked.dps, result["best"]["dps"].as_f64().unwrap());
    assert!(result["best"]["constraints_passed"].as_bool().unwrap());
    for position in &request.locked_slots {
        assert_eq!(
            result["baseline"]["equipment"]["slots"][position],
            result["best"]["equipment"]["slots"][position]
        );
    }
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        original,
        "operator is read-only"
    );
    for diff in result["slot_diff"].as_array().unwrap() {
        assert!(diff["after"]["price"].is_null());
    }
}

#[test]
fn all_frozen_environment_fields_survive_full_equipment_recalculation() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    request.simulation.network_delay = 87;
    request.simulation.initial_rage = Some(40);
    request.simulation.dunya_reset_seed = 4294967000;
    request.simulation.boss_attack_interval = Some(2.0);
    request.simulation.hanjia_expectation = Some(true);
    request.simulation.pre_releases = vec![crate::PreReleaseSpec {
        skill: "血怒".into(),
        time_before: 1.0,
    }];
    request.simulation.pauses = vec![(3.0, 0.5)];
    // Select actual versioned records, not invented buff/formation identifiers.
    let context = runtime.context();
    if let Some(recipe) = context.recipes.iter().find(|r| !r.hidden) {
        request.simulation.recipes = vec![recipe.id];
    }
    if let Some(talent) = context.talents.first() {
        request.simulation.talents = vec![talent.id];
    }
    if let Some(buff) = context.team_buffs.first() {
        request.simulation.team_buffs = vec![crate::TeamBuffSelection {
            key: buff.key.clone(),
            enabled: true,
            stacks: 1,
            first_release: 0.0,
            period: 0.0,
            duration: 3.0,
            release_times: Some(vec![0.0, 5.0]),
        }];
    }
    if let Some(formation) = context
        .formations
        .iter()
        .find(|f| f.applicable_when == "other")
    {
        request.simulation.formation = Some(crate::FormationSelection {
            key: formation.key.clone(),
        });
    }
    let result = execute(&request, &runtime);
    let mut before = serde_json::to_value(&request.simulation).unwrap();
    let mut after = result["best"]["simulation"].clone();
    for key in [
        "attributes",
        "equipment",
        "haste_level",
        "sequence",
        "macro_duration",
        "lite",
        "lite_keep_timeline",
    ] {
        before.as_object_mut().unwrap().remove(key);
        after.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(before, after);
    let slots: EquipmentSnapshot =
        serde_json::from_value(result["best"]["equipment"].clone()).unwrap();
    let calc =
        runtime.calculate_equipment(&slots.slots, slots.stone_id, &request.simulation.talents);
    assert_eq!(
        serde_json::to_value(attributes(&calc.raw)).unwrap(),
        result["best"]["simulation"]["attributes"]
    );
}

#[test]
fn small_two_slot_pool_is_exhaustively_scored_against_engine_oracle() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    request.locked_slots.retain(|p| p != "SHOES");
    add_candidates(&mut request, &runtime, "HAT", 2);
    add_candidates(&mut request, &runtime, "SHOES", 2);
    let scenario = snapshot(&request);
    let result = execute(&request, &runtime);
    assert_eq!(result["candidate_combinations"], 9);
    assert_eq!(result["simulations"], 9);
    assert_eq!(result["pool_search_complete"], true);
    let mut hats = request.candidate_ids["HAT"].clone();
    hats.push(request.equipment.slots["HAT"].equip_id);
    let mut shoes = request.candidate_ids["SHOES"].clone();
    shoes.push(request.equipment.slots["SHOES"].equip_id);
    let mut oracle = 0.0_f64;
    for hat in hats {
        for shoe in &shoes {
            let mut config = request.equipment.clone();
            config.slots.get_mut("HAT").unwrap().equip_id = hat;
            config.slots.get_mut("SHOES").unwrap().equip_id = *shoe;
            let (sim, _) = build_simulation(&request, &runtime, &scenario, &config).unwrap();
            oracle = oracle.max(replay(&runtime, &sim).dps);
        }
    }
    assert_eq!(result["best"]["dps"].as_f64().unwrap(), oracle);
}

#[test]
fn constraints_reject_invalid_ids_sources_incomplete_builds_and_version_mismatch() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    let mut wrong = request.clone();
    wrong.candidate_ids.insert("HAT".into(), vec![u32::MAX]);
    assert!(run(
        &wrong,
        &runtime,
        &snapshot(&wrong),
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    wrong = request.clone();
    wrong.allowed_sources = vec!["nonexistent-source".into()];
    assert!(run(
        &wrong,
        &runtime,
        &snapshot(&wrong),
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    wrong = request.clone();
    wrong.equipment.slots.remove("SHOES");
    assert!(wrong.validate().is_err());
    wrong = request.clone();
    wrong.version = GameVersion::ShanHaiYuanLiu;
    assert!(run(
        &wrong,
        &runtime,
        &snapshot(&wrong),
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    wrong = request.clone();
    wrong.equipment.stone_id = u32::MAX;
    assert!(run(
        &wrong,
        &runtime,
        &snapshot(&wrong),
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    wrong = request.clone();
    wrong.equipment.slots.get_mut("HAT").unwrap().enhance_id = u32::MAX;
    assert!(run(
        &wrong,
        &runtime,
        &snapshot(&wrong),
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    let mut unknown = serde_json::to_value(&request).unwrap();
    unknown["gold_budget"] = json!(100);
    assert!(
        serde_json::from_value::<EquipmentRequest>(unknown).is_err(),
        "unknown prices are never free"
    );
}

#[test]
fn budget_and_cancellation_charge_real_replays_and_preserve_only_tested_results() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 4);
    request.max_simulations = 2;
    let result = execute(&request, &runtime);
    assert_eq!(result["simulations"], 2);
    assert_eq!(result["stop_reason"], "budget_exhausted");
    assert_eq!(result["best"]["verified"], true);
    let flag = AtomicBool::new(true);
    let stopped = run(&request, &runtime, &snapshot(&request), &flag, |_| {}).unwrap();
    assert_eq!(stopped["simulations"], 0);
    assert!(stopped["baseline"].is_null());
    assert!(stopped["best"].is_null());
    flag.store(false, Ordering::Relaxed);
    let stopped = run(&request, &runtime, &snapshot(&request), &flag, |_| {
        flag.store(true, Ordering::Relaxed)
    })
    .unwrap();
    assert_eq!(stopped["simulations"], 1);
    assert_eq!(stopped["stop_reason"], "cancelled");
}

#[test]
fn infeasible_haste_constraint_does_not_publish_an_invalid_best() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    request.haste_min = Some(10_000_000);
    let result = execute(&request, &runtime);
    assert_eq!(result["baseline"]["constraints_passed"], false);
    assert!(result["best"].is_null());
    assert_eq!(result["simulations"], 1);
    assert_eq!(result["skipped_constraints"], 1);
}

#[test]
fn ties_and_catalog_iteration_are_repeatable_and_current_build_is_not_a_partial_baseline() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 2);
    let one = execute(&request, &runtime);
    let two = execute(&request, &runtime);
    assert_eq!(one["best"]["equipment_hash"], two["best"]["equipment_hash"]);
    assert_eq!(one["best"]["fingerprint"], two["best"]["fingerprint"]);
    assert_eq!(one["history"], two["history"]);
    assert_eq!(
        one["baseline"]["simulation"]["equipment"]
            .as_object()
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn hybrid_or_empty_rotation_is_refused_instead_of_silently_reinterpreted() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    request.simulation.sequence = vec!["盾刀".into(), "__macro__".into()];
    assert!(request.validate().is_err());
    request.simulation.sequence.clear();
    request.simulation.macro_text = None;
    assert!(request.validate().is_err());
}

#[test]
fn manual_axis_search_preserves_the_complete_policy_and_replays_every_result() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 2);
    request.simulation.sequence = vec!["盾刀".into(); 20];
    request.simulation.macro_text = None;
    request.simulation.macro_duration = None;
    request.simulation.network_delay = 87;
    request.simulation.initial_rage = Some(40);
    request.simulation.channel_ticks.insert("3".into(), 1);
    request.simulation.timing_offsets.insert("4".into(), 0.5);
    request.simulation.qijin_buffs.insert("5".into(), 0);
    request.simulation.pre_releases = vec![crate::PreReleaseSpec {
        skill: "血怒".into(),
        time_before: 1.0,
    }];
    request.simulation.pauses = vec![(3.0, 0.5)];
    let result = execute(&request, &runtime);
    assert_eq!(result["policy_scope"], "fixed_action_sequence");
    assert_eq!(result["baseline_policy"], result["candidate_policy"]);
    assert_eq!(result["baseline_policy"]["action_count"], 20);
    assert!(result["baseline_policy"]["macro_start_window_seconds"].is_null());
    assert_eq!(result["simulations"], 3);
    assert_eq!(result["pool_search_complete"], true);
    for label in ["baseline", "best"] {
        let simulation: SimulateRequest =
            serde_json::from_value(result[label]["simulation"].clone()).unwrap();
        let mut original = serde_json::to_value(&request.simulation).unwrap();
        let mut installed = serde_json::to_value(&simulation).unwrap();
        for field in [
            "attributes",
            "equipment",
            "haste_level",
            "lite",
            "lite_keep_timeline",
        ] {
            original.as_object_mut().unwrap().remove(field);
            installed.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(
            original, installed,
            "only complete equipment values may change"
        );
        let replayed = replay(&runtime, &simulation);
        assert_eq!(
            result[label]["fingerprint"],
            replayed.fingerprint.to_string()
        );
        assert_eq!(result[label]["dps"], replayed.dps);
        assert!(
            replayed.fight_time > request.duration_seconds,
            "manual axis is neither truncated nor padded to the macro window"
        );
    }
}

#[test]
fn manual_axis_output_does_not_change_when_only_macro_window_budget_changes() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    request.simulation.sequence = vec!["盾刀".into(); 5];
    request.simulation.macro_text = None;
    request.simulation.macro_duration = None;
    let first = execute(&request, &runtime);
    request.duration_seconds = 600.0;
    let second = execute(&request, &runtime);
    for label in ["baseline", "best"] {
        for field in [
            "fingerprint",
            "scenario_hash",
            "dps",
            "total_damage",
            "fight_time",
            "simulation",
            "policy",
        ] {
            assert_eq!(
                first[label][field], second[label][field],
                "manual policy field {field} must not depend on macro window"
            );
        }
    }
    assert_eq!(first["baseline_policy"], second["baseline_policy"]);
}

#[test]
fn skipped_manual_actions_are_observations_and_cannot_be_published_as_verified_best() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 1);
    request.simulation.sequence = vec!["盾刀".into(), "不存在的动作".into()];
    request.simulation.macro_text = None;
    request.simulation.macro_duration = None;
    // One instantaneous cast at t=0 has no positive fight duration. That must
    // stay a failed baseline, distinct from a positive-duration partial replay.
    let zero_duration = execute(&request, &runtime);
    assert_eq!(zero_duration["stop_reason"], "baseline_failed");
    assert_eq!(zero_duration["simulations"], 1);
    assert!(zero_duration["baseline"].is_null());
    assert!(zero_duration["best"].is_null());
    request.simulation.sequence = vec!["盾刀".into(), "盾刀".into(), "不存在的动作".into()];
    let result = execute(&request, &runtime);
    assert!(result["baseline"]["active_casts"].as_u64().unwrap() > 0);
    assert!(result["baseline"]["skipped_count"].as_u64().unwrap() > 0);
    assert_eq!(result["baseline"]["verified"], false);
    assert!(result["best"].is_null());
    assert_eq!(result["simulations"], 2);
    assert_eq!(result["unverified_candidates"], 2);
    assert_eq!(result["pool_search_complete"], false);
    assert!(result["history"]
        .as_array()
        .unwrap()
        .iter()
        .all(|h| h["accepted"] == false));

    request.simulation.sequence = vec!["__macro__".into(); 60];
    request.simulation.macro_text = Some("/cast [rage>100] 盾刀".into());
    request.simulation.macro_duration = Some(10.0);
    let blocked = execute(&request, &runtime);
    assert_eq!(blocked["baseline"]["active_casts"], 0);
    assert_eq!(blocked["baseline"]["verified"], false);
    assert!(blocked["best"].is_null());
    assert_eq!(blocked["pool_search_complete"], false);
}

#[test]
fn direct_model_build_admission_preserves_locks_and_recalculates_any_legal_catalog_candidate() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 2);
    let alternative = request.candidate_ids["HAT"][1];
    request.candidate_source = CandidateSource::Catalog;
    request.candidate_ids.clear();
    request.max_candidates_per_slot = 1;
    let mut proposal = request.equipment.clone();
    proposal.slots.get_mut("HAT").unwrap().equip_id = alternative;
    let sim = evaluate_snapshot(&request, &runtime, &snapshot(&request), &proposal).unwrap();
    assert_eq!(sim.equipment["HAT"], alternative);
    let expected = runtime.calculate_equipment(&proposal.slots, proposal.stone_id, &sim.talents);
    assert_eq!(
        sim.attributes.unwrap().base_attack,
        expected.raw.base_attack
    );
    proposal.slots.get_mut("SHOES").unwrap().strength = 1;
    assert!(evaluate_snapshot(&request, &runtime, &snapshot(&request), &proposal).is_err());
    proposal = request.equipment.clone();
    proposal.slots.remove("HAT");
    assert!(evaluate_snapshot(&request, &runtime, &snapshot(&request), &proposal).is_err());
}

#[test]
fn equipment_diff_reports_refining_and_gem_changes_even_when_item_id_stays_the_same() {
    let runtime = runtime();
    let request = fixture(&runtime);
    let mut candidate = request.equipment.clone();
    candidate.slots.get_mut("HAT").unwrap().strength = 1;
    candidate.slots.get_mut("HAT").unwrap().embedding = vec![6, 6, 6];
    let diff = equipment_diff(&runtime, &request.equipment, &candidate);
    assert_eq!(diff.as_array().unwrap().len(), 1);
    assert_eq!(diff[0]["position"], "HAT");
    assert_eq!(diff[0]["before"]["id"], diff[0]["after"]["id"]);
    assert_eq!(diff[0]["after"]["config"]["embedding"], json!([6, 6, 6]));
}

#[test]
fn post_simulation_failures_retain_spent_budget_and_verified_evidence() {
    let runtime = runtime();
    let mut request = fixture(&runtime);
    add_candidates(&mut request, &runtime, "HAT", 2);
    let mut calls = 0_u32;
    let mut observed = 0_u64;
    let result = run_with_evaluator(
        &request,
        &runtime,
        &snapshot(&request),
        &AtomicBool::new(false),
        |event| observed = observed.max(event["simulations"].as_u64().unwrap()),
        |request, runtime, scenario, config| {
            calls += 1;
            let candidate = evaluate(request, runtime, scenario, config)?;
            if calls == 2 {
                Err("injected rejection after real replay".into())
            } else {
                Ok(candidate)
            }
        },
    )
    .unwrap();
    assert_eq!(calls, 3);
    assert_eq!(result["simulations"], calls);
    assert_eq!(observed, u64::from(calls));
    assert_eq!(result["failed_candidates"], 1);
    assert_eq!(result["pool_search_complete"], false);
    assert_eq!(result["baseline"]["verified"], true);
    assert_eq!(result["best"]["verified"], true);
    assert_eq!(result["history"][0]["verified"], false);

    let result = run_with_evaluator(
        &request,
        &runtime,
        &snapshot(&request),
        &AtomicBool::new(false),
        |_| {},
        |request, runtime, scenario, config| {
            evaluate(request, runtime, scenario, config)?;
            Err("injected baseline rejection after real replay".into())
        },
    )
    .unwrap();
    assert_eq!(result["simulations"], 1);
    assert_eq!(result["failed_candidates"], 1);
    assert_eq!(result["stop_reason"], "baseline_failed");
    assert!(result["baseline"].is_null());
    assert!(result["best"].is_null());
}
