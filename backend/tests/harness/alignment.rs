use super::*;
use crate::agent::AgentRuntime;
use serde_json::{json, Value};
use std::sync::LazyLock;

fn simulate_fixture() -> crate::SimulateResponse {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    let context = runtime.context();
    crate::simulate_core(
        &scenario.simulation,
        context.skills,
        context.game_version,
        context.mount,
        context.constants,
        context.recipes,
        context.team_buffs,
        context.formations,
    )
}

// Start with an actual simulator event so synthetic cases retain the real shape.
static SAMPLE: LazyLock<CastEvent> = LazyLock::new(|| {
    simulate_fixture()
        .timeline
        .into_iter()
        .find(is_active)
        .expect("fixture must cast a skill")
});

fn event(name: &str, cast_time: f64) -> CastEvent {
    let mut event = SAMPLE.clone();
    event.name = name.into();
    event.skill_id = 1;
    event.cast_time = cast_time;
    event.triggered = false;
    event.channel_ticks = None;
    event.state_before = None;
    event
}

fn fixture_event(value: &Value) -> CastEvent {
    let mut event = event(
        value["name"].as_str().unwrap(),
        value["cast_time"].as_f64().unwrap(),
    );
    event.skill_id = value["skill_id"].as_u64().unwrap_or(1) as u32;
    event.triggered = value["triggered"].as_bool().unwrap_or(false);
    event.channel_ticks = value["channel_ticks"].as_u64().map(|n| n as u32);
    if let Some(state) = value.get("state_before") {
        let mut before = SAMPLE
            .state_before
            .clone()
            .expect("full fixture needs snapshots");
        before.rage = state["rage"].as_i64().unwrap() as i32;
        before.berserk_value = state["berserk_value"].as_i64().map(|n| n as i32);
        before.max_berserk_value = state["max_berserk_value"].as_i64().map(|n| n as i32);
        before.block_value = state["block_value"].as_i64().map(|n| n as i32);
        event.state_before = Some(before);
    }
    event
}

fn paired_indices(result: &Alignment) -> Vec<[usize; 2]> {
    result
        .rows
        .iter()
        .filter_map(|row| Some([row.reference_index?, row.actual_index?]))
        .collect()
}

#[test]
fn shared_frontend_fixtures_have_the_same_alignment_meaning() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("alignment-cases.json")).unwrap();
    for fixture in fixtures {
        let reference: Vec<_> = fixture["reference"]
            .as_array()
            .unwrap()
            .iter()
            .map(fixture_event)
            .collect();
        let actual: Vec<_> = fixture["actual"]
            .as_array()
            .unwrap()
            .iter()
            .map(fixture_event)
            .collect();
        let result = align(
            &reference,
            &actual,
            fixture["window"].as_f64().unwrap(),
            fixture["time_tolerance"]
                .as_f64()
                .unwrap_or(DEFAULT_TIME_TOLERANCE),
        )
        .unwrap();
        let mut observed = json!({
            "pairs": paired_indices(&result), "missing": result.summary.missing,
            "extra": result.summary.extra, "changed": result.summary.changed,
            "first_difference": result.summary.first_difference, "time_error": result.summary.time_error,
        });
        if fixture["expected"].get("resource_fields").is_some() {
            observed["resource_fields"] = json!(result
                .rows
                .iter()
                .flat_map(|row| row.resource_diffs.iter().map(|diff| &diff.field))
                .collect::<Vec<_>>());
        }
        let mut expected = fixture["expected"].clone();
        expected["time_error"] = json!(expected["time_error"].as_f64().unwrap());
        assert_eq!(observed, expected, "fixture {}", fixture["name"]);
    }
}

#[test]
fn actual_simulator_timeline_aligns_and_detects_a_changed_cast() {
    let response = simulate_fixture();
    assert!(
        response
            .timeline
            .iter()
            .filter(|event| is_active(event))
            .count()
            >= 2
    );
    let window = response.fight_time + 1.0;
    let same = align(
        &response.timeline,
        &response.timeline,
        window,
        DEFAULT_TIME_TOLERANCE,
    )
    .unwrap();
    assert_eq!(same.summary, AlignmentSummary::default());
    let mut actual = response.timeline.clone();
    let index = actual.iter().position(is_active).unwrap();
    actual[index].state_before.as_mut().unwrap().rage += 1;
    actual[index].cast_time += 0.125;
    let changed = align(&response.timeline, &actual, window, DEFAULT_TIME_TOLERANCE).unwrap();
    assert_eq!(changed.summary.changed, 1);
    assert_eq!(changed.summary.time_error, 0.125);
    let row = &changed.rows[changed.summary.first_difference.unwrap()];
    assert_eq!(row.reference_index, Some(index));
    assert_eq!(row.actual_index, Some(index));
    assert_eq!(row.resource_diffs[0].field, "rage");
}

#[test]
fn channel_ticks_skill_ids_and_variants_are_separately_detected() {
    let reference = event("盾舞", 0.0);
    let mut actual = reference.clone();
    actual.skill_id += 1;
    assert_eq!(
        align(&[reference.clone()], &[actual], 1.0, 0.0)
            .unwrap()
            .summary
            .changed,
        1
    );
    let mut before = reference.clone();
    let mut after = reference.clone();
    before.channel_ticks = Some(2);
    after.channel_ticks = Some(3);
    assert_eq!(
        align(&[before], &[after.clone()], 1.0, 0.0)
            .unwrap()
            .summary
            .changed,
        1
    );
    // An unrecorded tick count is not proof of a changed count.
    assert_eq!(
        align(&[reference.clone()], &[after], 1.0, 0.0)
            .unwrap()
            .summary
            .changed,
        0
    );
    let mut after = reference.clone();
    after.name = "盾舞·2级".into();
    assert_eq!(
        align(&[reference], &[after], 1.0, 0.0)
            .unwrap()
            .summary
            .changed,
        1
    );
}

#[test]
fn invalid_windows_and_tolerances_are_rejected_before_allocation() {
    for window in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(align(&[], &[], window, DEFAULT_TIME_TOLERANCE).is_err());
    }
    for tolerance in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(align(&[], &[], 1.0, tolerance).is_err());
    }
}

#[test]
fn window_excludes_nonfinite_and_boundary_times_without_renumbering() {
    let reference = vec![
        event("盾刀", f64::NAN),
        event("盾刀", f64::INFINITY),
        event("盾刀", -1.0),
        event("盾刀", 0.0),
        event("盾刀", 2.0),
    ];
    let result = align(&reference, &[event("盾刀", 0.0)], 2.0, 0.0).unwrap();
    assert_eq!(paired_indices(&result), vec![[3, 0]]);
    assert_eq!(result.summary, AlignmentSummary::default());
}

#[test]
fn active_event_limit_is_enforced_on_each_side_without_truncation() {
    let maximum = vec![event("盾刀", 0.0); MAX_ACTIVE_EVENTS];
    let mut overflow = maximum.clone();
    overflow.push(event("盾刀", 0.0));
    assert!(align(&overflow, &[], 1.0, 0.0)
        .unwrap_err()
        .contains("MACRO_ALIGNMENT_LIMIT"));
    assert!(align(&[], &overflow, 1.0, 0.0)
        .unwrap_err()
        .contains("MACRO_ALIGNMENT_LIMIT"));
    let mut passive = event("破·盾刀", 0.0);
    passive.triggered = true;
    let mut accepted = maximum.clone();
    accepted.extend(vec![passive; MAX_ACTIVE_EVENTS]);
    accepted.push(event("盾刀", 1.0));
    let result = align(&accepted, &maximum[..MAX_ACTIVE_EVENTS - 1], 1.0, 0.0).unwrap();
    assert_eq!(result.rows.len(), MAX_ACTIVE_EVENTS);
    assert_eq!(result.summary.missing, 1);
    assert_eq!(result.summary.extra, 0);
    assert_eq!(result.summary.changed, 0);
}

#[test]
fn explicit_zero_tolerance_and_missing_snapshot_remain_meaningful() {
    let mut before = event("盾刀", 0.0);
    before.state_before = SAMPLE.state_before.clone();
    let after = event("盾刀", DEFAULT_TIME_TOLERANCE);
    let tolerant = align(
        &[before.clone()],
        &[after.clone()],
        1.0,
        DEFAULT_TIME_TOLERANCE,
    )
    .unwrap();
    assert_eq!(tolerant.summary.changed, 0);
    assert!(tolerant.rows[0].resource_diffs.is_empty());
    let strict = align(&[before], &[after], 1.0, 0.0).unwrap();
    assert_eq!(strict.summary.changed, 1);
    assert_eq!(strict.rows[0].time_delta, Some(DEFAULT_TIME_TOLERANCE));
}
