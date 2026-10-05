use super::*;
use crate::{GameVersion, Mount};

fn fixture() -> (Checkpoint, AgentRuntime) {
    let data: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut sim: SimulateRequest = serde_json::from_value(data["simulation"].clone()).unwrap();
    sim.sequence = vec!["盾刀".into(); 6];
    sim.macro_text = None;
    sim.macro_duration = None;
    sim.network_delay = 0;
    sim.talents.clear();
    sim.recipes.clear();
    sim.equipment.clear();
    let request = RunRequest {
        goal: "研究宏与循环".into(),
        provider_profile: "offline".into(),
        simulation: sim,
        version: GameVersion::AnYingQianJi,
        mount: Mount::FenShanJin,
        equipment: None,
        constraints: RunConstraints {
            duration_seconds: 10.0,
            allowed_skills: vec!["盾刀".into()],
            ..Default::default()
        },
        budget: RunBudget::default(),
    };
    let scenario = request.scenario().unwrap();
    let state = Checkpoint {
        schema_version: RUNTIME_VERSION.into(),
        run_id: "experiment-tools-test".into(),
        sequence: 0,
        status: "running".into(),
        phase: "test".into(),
        message: String::new(),
        request,
        scenario,
        runtime_hash: "fixture".into(),
        experiment_hash: "fixture".into(),
        model: "offline".into(),
        usage: RunUsage::default(),
        events: vec![],
        artifacts: vec![],
        attempts: Default::default(),
        result: None,
        persistence_error: false,
        reserved_simulations: 0,
    };
    (state, AgentRuntime::fixture())
}
fn experiment(value: Value) -> Experiment {
    let mut value = value;
    value["hypothesis"] = json!("此候选应通过完整场景回放，或产生可观察的反例。");
    serde_json::from_value(value).unwrap()
}
fn execute_test(state: &Checkpoint, runtime: &AgentRuntime, args: Value) -> ExperimentOutput {
    execute(
        state,
        runtime,
        &experiment(args),
        &AtomicBool::new(false),
        60_000,
        |_| {},
    )
    .unwrap()
}
fn add_artifact(state: &mut Checkpoint, output: ExperimentOutput, id: &str, kind: &str) {
    state.artifacts.push(Artifact {
        id: id.into(),
        parent_id: None,
        kind: kind.into(),
        scenario_hash: artifact_hash(&output.simulation, &state.request),
        request_hash: "test".into(),
        summary: "test".into(),
        result: output.result,
        simulation: output.simulation,
        equipment: output.equipment,
    });
}

#[test]
fn arbitrary_macro_replays_full_scene_and_exposes_failure_states() {
    let (mut state, runtime) = fixture();
    state.request.simulation.dunya_reset_seed = 88;
    state.scenario = state.request.scenario().unwrap();
    let output = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","macro_text":"/cast rage>100 盾刀"}),
    );
    assert_eq!(output.simulations, 2);
    assert_eq!(output.simulation.dunya_reset_seed, 88);
    assert_eq!(output.result["best"]["macro_text"], "/cast [rage>100] 盾刀");
    assert_eq!(output.result["best"]["verified"], false);
    assert_eq!(output.result["best"]["reproduced"], false);
    assert!(
        output.result["best"]["alignment"]["summary"]["missing"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(output.result["reference_timeline"][0]["state_before"].is_object());
    assert!(
        output.result["macro_trace"]["decisions"]
            .as_array()
            .unwrap()
            .len()
            > 0
    );
    add_artifact(&mut state, output, "evidence-1", "evaluate");
    let observed = inspect(
        &state,
        &runtime,
        &json!({"section":"timeline","artifact_id":"evidence-1","query":"reference","limit":2}),
    )
    .unwrap();
    assert_eq!(observed["timeline"].as_array().unwrap().len(), 2);
    assert!(observed["timeline"][0]["state_before"].is_object());
}

#[test]
fn evaluate_cannot_bypass_skill_pool_or_game_page_limit() {
    let (state, runtime) = fixture();
    for text in [
        "/cast 盾猛".to_owned(),
        format!("//{}\n/cast 盾刀", "x".repeat(128)),
    ] {
        let output = execute_test(
            &state,
            &runtime,
            json!({"kind":"evaluate","macro_text":text}),
        );
        assert_eq!(output.simulations, 2);
        assert_eq!(output.result["best"]["constraints_passed"], false);
        assert_eq!(output.result["best"]["verified"], false);
        assert_eq!(output.result["best"]["reproduced"], false);
    }
}

#[test]
fn exact_macro_has_replayable_fingerprint_without_fake_same_duration_gain() {
    let (state, runtime) = fixture();
    let output = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","macro_text":"/cast 盾刀"}),
    );
    assert_eq!(output.result["best"]["verified"], true);
    assert_eq!(output.result["best"]["reproduced"], true);
    assert_eq!(output.result["dps_comparable"], false);
    assert!(output.result["improvement_pct"].is_null());
    let replay = run_simulation(&runtime, &output.simulation);
    assert_eq!(
        output.result["best"]["fingerprint"],
        replay.fingerprint.to_string()
    );
    assert!(
        output.result["window_seconds"].as_f64().unwrap()
            > output.result["baseline"]["metrics"]["fight_time"]
                .as_f64()
                .unwrap()
    );
}

#[test]
fn validation_distinguishes_same_candidate_replay_from_changed_environment() {
    let (mut state, runtime) = fixture();
    let parent = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","macro_text":"/cast 盾刀"}),
    );
    add_artifact(&mut state, parent, "evidence-1", "evaluate");
    let repeated = execute_test(
        &state,
        &runtime,
        json!({"kind":"validate","parent_id":"evidence-1"}),
    );
    assert_eq!(
        repeated.result["validation"]["scope"],
        "same_scenario_replay"
    );
    assert_eq!(repeated.result["validation"]["independent"], false);
    assert_eq!(
        repeated.result["validation"]["fingerprint_matches_parent"],
        true
    );
    let independent = execute_test(
        &state,
        &runtime,
        json!({"kind":"validate","parent_id":"evidence-1","network_delay":100}),
    );
    assert_eq!(
        independent.result["validation"]["scope"],
        "held_out_delay_or_seed"
    );
    assert_eq!(independent.result["validation"]["independent"], true);
    assert_eq!(
        independent.simulation.network_delay, 0,
        "Applying a validation artifact must retain the original environment"
    );
    assert_ne!(
        independent.result["validation"]["application_scenario_hash"],
        independent.result["validation"]["evaluation_scenario_hash"]
    );
    assert_eq!(
        independent.result["baseline"]["simulation"]["network_delay"],
        100
    );
    assert_eq!(
        independent.result["best"]["simulation"]["network_delay"],
        100
    );
    let forbidden = execute_test(
        &state,
        &runtime,
        json!({"kind":"validate","parent_id":"evidence-1","macro_text":"/cast [rage>100] 盾刀","network_delay":100}),
    );
    assert_eq!(forbidden.simulations, 0);
    assert_eq!(forbidden.result["ok"], false);
    assert!(forbidden.result["error"]
        .as_str()
        .unwrap()
        .contains("evaluate"));
}

#[test]
fn compile_accepts_model_supplied_axis_without_user_preset_switch() {
    let (mut state, runtime) = fixture();
    state.request.simulation = macro_request(&state.request.simulation, "/cast 盾刀", 10.0);
    state.scenario = state.request.scenario().unwrap();
    let output = execute_test(
        &state,
        &runtime,
        json!({"kind":"compile_macro","sequence":["盾刀","盾刀","盾刀"],"macro_text":"/cast 盾刀","max_simulations":2}),
    );
    assert_eq!(output.simulations, 2);
    assert_eq!(
        output.result["best"]["reproduced"], true,
        "{}",
        output.result
    );
    assert_eq!(
        output.result["baseline"]["simulation"]["sequence"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn cancellation_and_compile_failure_return_exact_consumed_counts() {
    let (state, runtime) = fixture();
    let cancel = AtomicBool::new(true);
    let args = experiment(json!({"kind":"evaluate","macro_text":"/cast 盾刀"}));
    let zero = execute(&state, &runtime, &args, &cancel, 60_000, |_| {}).unwrap();
    assert_eq!(zero.simulations, 0);
    cancel.store(false, Ordering::Relaxed);
    let one = execute(&state, &runtime, &args, &cancel, 60_000, |p| {
        if p["simulations"] == 1 {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert_eq!(one.simulations, 1);
    assert_eq!(one.result["stop_reason"], "cancelled");
    assert!(one.result["baseline"].is_object());
    let mut invalid = state;
    invalid.request.constraints.allowed_skills.clear();
    let failed = execute_test(
        &invalid,
        &runtime,
        json!({"kind":"compile_macro","sequence":["绝刀"],"max_simulations":4}),
    );
    assert_eq!(failed.simulations, 1, "{}", failed.result);
    assert_eq!(failed.result["ok"], false);
}

fn with_equipment() -> (
    Checkpoint,
    AgentRuntime,
    super::super::equipment::EquipmentSnapshot,
    u32,
) {
    let (mut state, _) = fixture();
    let runtime = AgentRuntime::fixture().with_equipment_fixture();
    let make_slot = |id| crate::equip::SlotConfig {
        equip_id: id,
        strength: 0,
        embedding: vec![],
        enhance_id: 0,
        enchant_id: 0,
    };
    let choose = |position: &str| {
        runtime
            .equipment_items()
            .filter(|i| {
                i.sub_type == crate::equip::pos_to_subtype(position)
                    && i.require_level <= runtime.context().constants.level
                    && matches!(
                        (i.belong_school.as_str(), i.magic_kind.as_str()),
                        ("苍云", "外功") | ("通用", "身法") | ("精简", "外功")
                    )
                    && !i.magic_type.contains("(PVP)")
                    && !i.magic_type.contains("(PVX)")
            })
            .min_by_key(|i| (i.level, i.id))
            .unwrap()
            .id
    };
    let slots = super::super::equipment::POSITIONS
        .iter()
        .map(|p| {
            (
                p.to_string(),
                make_slot(if *p == "PRIMARY_WEAPON" { choose(p) } else { 0 }),
            )
        })
        .collect();
    let original = super::super::equipment::EquipmentSnapshot {
        slots,
        stone_id: 0,
        source_label: "real catalog fixture".into(),
    };
    state.request.equipment = Some(original.clone());
    let hat = choose("HAT");
    (state, runtime, original, hat)
}

#[test]
fn direct_equipment_candidate_rebuilds_real_baseline_and_preserves_axis() {
    let (state, runtime, mut candidate, hat) = with_equipment();
    candidate.slots.get_mut("HAT").unwrap().equip_id = hat;
    let output = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","equipment":candidate}),
    );
    assert_eq!(output.simulations, 2, "{}", output.result);
    assert_eq!(
        output.simulation.sequence,
        state.scenario.simulation.sequence
    );
    assert_eq!(output.simulation.equipment["HAT"], hat);
    assert_ne!(
        output.result["baseline"]["simulation"]["attributes"],
        json!(state.scenario.simulation.attributes)
    );
    assert!(output.result["baseline"]["simulation"]["equipment"]
        .get("HAT")
        .is_none());
    assert_eq!(output.result["slot_diff"][0]["position"], "HAT");
    assert_eq!(output.result["best"]["verified"], true);
}

#[test]
fn equipment_lock_anchor_cannot_be_rebased_through_parent_candidate() {
    let (mut state, runtime, mut candidate, hat) = with_equipment();
    candidate.slots.get_mut("HAT").unwrap().equip_id = hat;
    state.request.constraints.locked_slots = vec!["HAT".into()];
    let rejected = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","equipment":candidate}),
    );
    assert_eq!(rejected.simulations, 0);
    assert_eq!(rejected.result["ok"], false);
    state.artifacts.push(Artifact {
        id: "evidence-prior".into(),
        parent_id: None,
        kind: "evaluate".into(),
        scenario_hash: state.scenario.scenario_hash.clone(),
        request_hash: "fixture".into(),
        summary: "constraint failed observation".into(),
        result: json!({"best":{"verified":false}}),
        simulation: state.scenario.simulation.clone(),
        equipment: Some(candidate),
    });
    let inherited = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","parent_id":"evidence-prior"}),
    );
    assert_eq!(inherited.simulations, 0);
    assert_eq!(inherited.result["ok"], false);
}

#[test]
fn direct_equipment_cannot_bypass_source_or_explicit_id_constraints() {
    let (mut state, runtime, mut candidate, hat) = with_equipment();
    candidate.slots.get_mut("HAT").unwrap().equip_id = hat;
    state.request.constraints.allowed_sources = vec!["not-an-allowed-catalog-source".into()];
    let denied = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","equipment":candidate}),
    );
    assert_eq!(denied.simulations, 0);
    assert_eq!(denied.result["ok"], false);
    state.request.constraints.allowed_sources.clear();
    state.request.constraints.candidate_source = "provided_ids".into();
    state
        .request
        .constraints
        .candidate_ids
        .insert("HAT".into(), vec![hat + 1]);
    let denied = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","equipment":candidate}),
    );
    assert_eq!(denied.simulations, 0);
    assert_eq!(denied.result["ok"], false);
}

#[test]
fn catalog_inspection_returns_filtered_position_aware_candidates() {
    let (mut state, runtime, _, _) = with_equipment();
    state.request.constraints.locked_slots = super::super::equipment::POSITIONS
        .iter()
        .filter(|p| **p != "HAT")
        .map(|s| s.to_string())
        .collect();
    let result = inspect(
        &state,
        &runtime,
        &json!({"section":"equipment_catalog","query":"HAT","limit":3}),
    )
    .unwrap();
    let items = result["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items
        .iter()
        .all(|i| i["position"] == "HAT" && i["item"]["id"].as_u64().unwrap() > 0));
}

#[test]
fn experiment_schema_describes_complete_equipment_and_rejection_is_visible_in_digest() {
    let tools = definitions();
    let experiment = tools.iter().find(|t| t.name == "experiment").unwrap();
    assert_eq!(
        experiment.parameters["properties"]["equipment"]["properties"]["slots"]["required"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    let (mut state, runtime) = fixture();
    let rejected = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","macro_text":"invalid"}),
    );
    assert_eq!(rejected.simulations, 0);
    add_artifact(&mut state, rejected, "rejected", "evaluate");
    assert_eq!(compact_artifact(&state.artifacts[0])["ok"], false);
    assert!(compact_artifact(&state.artifacts[0])["error"].is_string());
}

#[test]
fn macro_language_examples_are_accepted_by_current_parser() {
    let (state, runtime) = fixture();
    let grammar = inspect(&state, &runtime, &json!({"section":"macro_language"})).unwrap();
    for example in grammar["examples"].as_array().unwrap() {
        crate::macro_parser::parse_macro_text(example.as_str().unwrap()).unwrap();
    }
    for page in grammar["pages"].as_array().unwrap() {
        crate::macro_parser::parse_macro_text(&format!("{}\n/cast 盾刀", page.as_str().unwrap()))
            .unwrap();
    }
}

#[test]
fn validating_a_partial_compile_keeps_its_target_instead_of_self_comparing() {
    let (mut state, runtime) = fixture();
    let partial = execute_test(
        &state,
        &runtime,
        json!({"kind":"compile_macro","macro_text":"/cast [rage<10] 盾刀","max_simulations":2}),
    );
    assert_eq!(partial.result["best"]["verified"], true);
    assert_eq!(partial.result["best"]["reproduced"], false);
    let reference = partial.result["baseline"]["simulation"].clone();
    add_artifact(&mut state, partial, "partial-compile", "compile_macro");
    let checked = execute_test(
        &state,
        &runtime,
        json!({"kind":"validate","parent_id":"partial-compile"}),
    );
    assert_eq!(checked.result["baseline"]["simulation"], reference);
    assert_eq!(checked.result["best"]["reproduced"], false);
    assert_eq!(checked.result["validation"]["reproduced"], false);
    assert_eq!(
        checked.result["validation"]["fingerprint_matches_parent"],
        true
    );
    assert_eq!(
        checked.result["comparison_scope"],
        "parent_experiment_baseline"
    );
}

#[test]
fn manual_policy_validation_compares_complete_actions_across_haste_durations() {
    let (mut state, runtime) = fixture();
    let baseline = state.scenario.simulation.clone();
    let mut candidate = baseline.clone();
    candidate.haste_level = 50_000;
    candidate.attributes.as_mut().unwrap().haste_level = 50_000.0;
    let replay = run_simulation(&runtime, &candidate);
    state.artifacts.push(Artifact {id:"manual-haste-candidate".into(),parent_id:None,kind:"optimize_equipment".into(),
        scenario_hash:artifact_hash(&candidate,&state.request),request_hash:"fixture".into(),summary:"Measured manual policy with higher haste".into(),
        result:json!({"baseline":{"simulation":baseline},"best":{"verified":true,"fingerprint":replay.fingerprint.to_string()}}),
        simulation:candidate,equipment:None});
    let checked = execute_test(
        &state,
        &runtime,
        json!({"kind":"validate","parent_id":"manual-haste-candidate","network_delay":50}),
    );
    assert_eq!(checked.result["validation"]["policy_comparable"], true);
    assert_eq!(checked.result["validation"]["same_time_comparable"], false);
    assert_eq!(
        checked.result["validation"]["dps_comparison_scope"],
        "same_complete_manual_policy"
    );
    assert_eq!(checked.result["validation"]["improved"], true);
    assert_eq!(
        checked.result["best"]["simulation"]["sequence"],
        checked.result["baseline"]["simulation"]["sequence"]
    );
    assert_ne!(
        checked.result["best"]["metrics"]["fight_time"],
        checked.result["baseline"]["metrics"]["fight_time"]
    );
}

#[test]
fn compact_equipment_metrics_are_available_to_the_model() {
    let digest = candidate_digest(
        &json!({"dps":1234.5,"total_damage":12345.0,"fight_time":10.0,
        "skipped_count":0,"verified":true,"policy":{"scope":"fixed_action_sequence"}}),
    );
    assert_eq!(digest["metrics"]["dps"], 1234.5);
    assert_eq!(digest["metrics"]["fight_time"], 10.0);
    assert_eq!(digest["policy"]["scope"], "fixed_action_sequence");
}

#[test]
fn validation_digest_retains_outcomes_without_full_replay_inputs() {
    let (mut state, _) = fixture();
    let repeated = vec!["__macro__"; 600];
    let candidate = json!({"macro_text":"/cast 盾刀","metrics":{"dps":1234.5,"fight_time":120.0},
        "verified":true,"fingerprint":"12345678901234567890","simulation":{"sequence":repeated},
        "pages":[{"chars":8}],"trajectory":[{"state_before":{"rage":50}}]});
    let validation = json!({"status":"completed","scope":"one_held_out_delay","independent":true,
        "improved":true,"dps_comparable":true,"application_scenario_hash":"application-hash",
        "evaluation_scenario_hash":"evaluation-hash","application_fingerprint":"original-fingerprint",
        "fingerprint_matches_parent":null,"network_delay":100,"seed":3,
        "baseline":candidate,"best":candidate,"macro_trace":{"decisions":repeated}});
    state.artifacts.push(Artifact {
        id: "evidence-1".into(),
        parent_id: None,
        kind: "search_rotation".into(),
        scenario_hash: state.scenario.scenario_hash.clone(),
        request_hash: "fixture".into(),
        summary: "fixture".into(),
        result: json!({"validation":validation}),
        simulation: state.scenario.simulation.clone(),
        equipment: None,
    });
    let summary = compact_artifact(&state.artifacts[0]);
    let encoded = summary.to_string();
    assert!(!encoded.contains("__macro__"));
    assert!(!encoded.contains("\"simulation\""));
    assert!(!encoded.contains("\"pages\""));
    assert!(!encoded.contains("\"trajectory\""));
    assert!(
        encoded.len() < 2500,
        "semantic validation projection unexpectedly large: {}",
        encoded.len()
    );
    let compact = &summary["validation"];
    for key in [
        "status",
        "scope",
        "independent",
        "improved",
        "dps_comparable",
        "application_scenario_hash",
        "evaluation_scenario_hash",
        "application_fingerprint",
        "fingerprint_matches_parent",
        "network_delay",
        "seed",
    ] {
        assert_eq!(
            compact[key], validation[key],
            "lost validation metadata {key}"
        );
    }
    assert_eq!(compact["best"]["metrics"]["dps"], 1234.5);
    assert_eq!(compact["baseline"]["fingerprint"], "12345678901234567890");
}

#[test]
fn evidence_inspection_pages_old_branches_in_stable_creation_order() {
    let (mut state, runtime) = fixture();
    for id in 1..=14 {
        state.artifacts.push(Artifact {
            id: format!("evidence-{id}"),
            parent_id: None,
            kind: "evaluate".into(),
            scenario_hash: state.scenario.scenario_hash.clone(),
            request_hash: "fixture".into(),
            summary: "fixture".into(),
            result: json!({"simulations":0}),
            simulation: state.scenario.simulation.clone(),
            equipment: None,
        });
    }
    let first = inspect(
        &state,
        &runtime,
        &json!({"section":"evidence","offset":8,"limit":4}),
    )
    .unwrap();
    let again = inspect(
        &state,
        &runtime,
        &json!({"section":"evidence","offset":8,"limit":4}),
    )
    .unwrap();
    assert_eq!(first, again);
    assert_eq!(first["total"], 14);
    assert_eq!(first["offset"], 8);
    assert_eq!(first["limit"], 4);
    assert_eq!(first["artifacts"][0]["id"], "evidence-9");
    assert_eq!(first["artifacts"][3]["id"], "evidence-12");
    let empty = inspect(&state, &runtime, &json!({"section":"evidence","offset":20})).unwrap();
    assert!(empty["artifacts"].as_array().unwrap().is_empty());
}

#[test]
fn skill_inspection_defaults_to_paginated_catalog_and_exact_details_are_opt_in() {
    let (state, runtime) = fixture();
    let catalog = inspect(&state, &runtime, &json!({"section":"skills","limit":3})).unwrap();
    assert_eq!(catalog["view"], "compact_catalog");
    assert_eq!(catalog["skills"].as_array().unwrap().len(), 3);
    assert!(catalog["total"].as_u64().unwrap() > 3);
    assert_eq!(catalog["next_offset"], 3);
    assert!(catalog["talents"].as_array().unwrap().is_empty());
    for item in catalog["skills"].as_array().unwrap() {
        let context = runtime.context();
        let skill = context
            .skills
            .iter()
            .find(|s| json!(s.skill_id) == item["skill_id"])
            .unwrap();
        assert_eq!(item["name"], skill.name);
        assert_eq!(item["passive"], skill.passive);
        assert_eq!(item["stance"], json!(skill.stance));
        assert_eq!(item["rage_cost"], skill.rage_cost);
        assert_eq!(item["rage_gain"], skill.rage_gain);
        assert_eq!(item["max_charges"], skill.max_charges);
        assert_eq!(item["charge_cd"], skill.charge_cd);
        if !skill.cooldowns.is_empty() {
            assert_eq!(item["cooldowns"], json!(skill.cooldowns));
        }
        assert!(item.get("description").is_none());
        assert!(item.get("attack_coeff").is_none());
        assert!(item.get("icon").is_none());
        let detail = inspect(
            &state,
            &runtime,
            &json!({"section":"skills","query":skill.skill_id.to_string()}),
        )
        .unwrap();
        assert_eq!(detail["skills"][0], serde_json::to_value(skill).unwrap());
        assert_eq!(detail["view"], "matched_definitions");
    }
    let next = inspect(
        &state,
        &runtime,
        &json!({"section":"skills","offset":3,"limit":3}),
    )
    .unwrap();
    assert_ne!(
        next["skills"][0]["skill_id"],
        catalog["skills"][0]["skill_id"]
    );
    let filtered = inspect(&state, &runtime, &json!({"section":"skills","query":"盾"})).unwrap();
    assert_eq!(filtered["view"], "compact_catalog");
    assert!(filtered["skills"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s.get("description").is_none()));
}

#[test]
fn compact_skill_catalog_keeps_talent_gate_and_exact_talent_details() {
    let (mut state, runtime) = fixture();
    let context = runtime.context();
    let gated = context
        .skills
        .iter()
        .find(|s| s.requires_talent.is_some())
        .unwrap();
    let query = json!({"section":"skills","query":gated.skill_id.to_string()});
    let unavailable = inspect(&state, &runtime, &query).unwrap();
    assert_eq!(unavailable["total"], 0);
    let talent = gated.requires_talent.unwrap();
    state.request.simulation.talents.push(talent);
    let available = inspect(&state, &runtime, &query).unwrap();
    assert_eq!(available["skills"][0]["requires_talent"], talent);
    if let Some(talent) = context.talents.iter().find(|t| t.id == talent) {
        let selected = inspect(&state, &runtime, &json!({"section":"skills"})).unwrap();
        assert_eq!(selected["talents"][0]["id"], talent.id);
        assert!(selected["talents"][0].get("desc").is_none());
        let detailed = inspect(
            &state,
            &runtime,
            &json!({"section":"skills","query":talent.id.to_string()}),
        )
        .unwrap();
        assert_eq!(
            detailed["talents"][0],
            serde_json::to_value(talent).unwrap()
        );
    }
}

#[test]
fn artifact_inspection_separates_summary_replay_inputs_diagnosis_and_timeline() {
    let (mut state, runtime, equipment, _) = with_equipment();
    let output = execute_test(
        &state,
        &runtime,
        json!({"kind":"evaluate","macro_text":"/cast [rage>100] 盾刀"}),
    );
    add_artifact(&mut state, output, "layered-evidence", "evaluate");
    state.artifacts[0].equipment = Some(equipment.clone());
    let original = serde_json::to_value(&state.artifacts[0]).unwrap();
    let read = |query: &str| {
        inspect(
            &state,
            &runtime,
            &json!({"section":"artifact","artifact_id":"layered-evidence","query":query}),
        )
        .unwrap()
    };
    let summary = read("");
    assert_eq!(
        summary["best"]["verified"],
        original["result"]["best"]["verified"]
    );
    assert_eq!(
        summary["best"]["reproduced"],
        original["result"]["best"]["reproduced"]
    );
    assert_eq!(
        summary["best"]["metrics"],
        original["result"]["best"]["metrics"]
    );
    assert_eq!(
        summary["dps_comparable"],
        original["result"]["dps_comparable"]
    );
    assert_eq!(
        summary["improvement_pct"],
        original["result"]["improvement_pct"]
    );
    for omitted in [
        "result",
        "simulation",
        "equipment",
        "timeline",
        "reference_timeline",
        "macro_trace",
        "diagnosis",
    ] {
        assert!(summary.get(omitted).is_none(), "default leaked {omitted}");
    }
    assert_eq!(read("simulation")["simulation"], original["simulation"]);
    assert_eq!(read("simulation")["scope"], "application_scene");
    assert_eq!(
        read("baseline_simulation")["simulation"],
        original["result"]["baseline"]["simulation"]
    );
    assert_eq!(
        read("evaluation_simulation")["simulation"],
        original["result"]["best"]["simulation"]
    );
    assert_eq!(read("equipment")["equipment"], json!(equipment));
    assert_eq!(
        read("diagnosis")["diagnosis"],
        original["result"]["best"]["diagnosis"]
    );
    assert_eq!(
        read("diagnosis")["first_difference"],
        original["result"]["best"]["first_difference"]
    );
    assert!(inspect(
        &state,
        &runtime,
        &json!({"section":"artifact","artifact_id":"layered-evidence","query":"everything"})
    )
    .is_err());
    let offset = summary["best"]["first_difference"]["reference_offset"]
        .as_u64()
        .unwrap();
    let timeline = inspect(&state, &runtime, &json!({"section":"timeline","artifact_id":"layered-evidence","query":"reference","offset":offset,"limit":1})).unwrap();
    assert_eq!(
        timeline["timeline"][0]["index"],
        summary["best"]["first_difference"]["reference_index"]
    );
    assert!(timeline.get("diagnosis").is_none());
    assert!(timeline.get("alignment").is_none());
    assert_eq!(
        serde_json::to_value(&state.artifacts[0]).unwrap(),
        original,
        "inspection must not mutate verification or replay evidence"
    );
}

#[test]
fn first_difference_digest_bounds_snapshot_catalogs_but_preserves_action_and_resources() {
    let enormous = vec![
        json!({"description":"large unrelated state","icon":"icon","name":"other skill"});
        2048
    ];
    let difference = json!({"kind":"changed","reference_index":8,"actual_index":9,"reference_offset":3,"actual_offset":4,
        "time_delta":0.125,"resource_diffs":[{"field":"rage","reference":40,"actual":50}],
        "reference":{"name":"月照连营·雾海·2","skill_id":90011,"cast_time":1.0,"channel_ticks":2,"rage_after":20,"rage_delta":-20,
            "state_before":{"time":1.0,"stance":"blade","rage":40,"block_value":7,"berserk_value":20,"max_berserk_value":100,
                "buffs":enormous,"skill_states":enormous,"skill_cds":enormous,"runtime_stats":enormous}},
        "actual":{"name":"月照连营·雾海·3","skill_id":90011,"cast_time":1.125,"channel_ticks":3,"rage_after":30,"rage_delta":-20,
            "state_before":{"time":1.125,"stance":"blade","rage":50,"block_value":7,"berserk_value":20,"max_berserk_value":100,
                "buffs":enormous,"skill_states":enormous}}});
    let digest = candidate_digest(
        &json!({"verified":true,"reproduced":false,"first_difference":difference}),
    );
    let compact = &digest["first_difference"];
    for key in [
        "kind",
        "reference_index",
        "actual_index",
        "reference_offset",
        "actual_offset",
        "time_delta",
        "resource_diffs",
    ] {
        assert_eq!(compact[key], difference[key]);
    }
    for side in ["reference", "actual"] {
        for key in [
            "name",
            "skill_id",
            "cast_time",
            "channel_ticks",
            "rage_after",
            "rage_delta",
        ] {
            assert_eq!(compact[side][key], difference[side][key]);
        }
        for key in [
            "time",
            "stance",
            "rage",
            "block_value",
            "berserk_value",
            "max_berserk_value",
        ] {
            assert_eq!(
                compact[side]["state_before"][key],
                difference[side]["state_before"][key]
            );
        }
    }
    let encoded = digest.to_string();
    assert!(
        encoded.len() < 1800,
        "unbounded first difference: {} bytes",
        encoded.len()
    );
    for omitted in [
        "large unrelated state",
        "skill_states",
        "skill_cds",
        "runtime_stats",
        "buffs",
    ] {
        assert!(!encoded.contains(omitted));
    }
    assert_eq!(digest["verified"], true);
    assert_eq!(digest["reproduced"], false);
}
