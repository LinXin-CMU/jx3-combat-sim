use super::*;

#[test]
fn berserk_survives_model_projection_and_compaction() {
    let mut result = serde_json::json!({
        "berserk": {"modeled":true,"cap":120,"overflow_total":30},
        "match_index": [{"event_number":1,"rage_before":85,"berserk_before":100,"berserk_after":0,"max_berserk_value":120}],
        "windows": [{"matched":{"event_number":1,"state_before":{"rage":85,"berserk_value":100,"max_berserk_value":120},"state_after":{"rage":85,"berserk_value":0}},"context":[]}]
    });
    project_tool_result("inspect_timeline_events", &mut result);
    let compact = compact_result_facts(Some(&result));
    assert_eq!(compact["berserk"]["modeled"], true);
    assert_eq!(compact["match_index"][0]["berserk_before"], 100);
    assert_eq!(compact["windows"][0]["matched"]["berserk_before"], 100);
    assert_eq!(compact["windows"][0]["matched"]["berserk_after"], 0);
}

#[test]
fn resource_paging_cursor_preserves_events_under_transport_budget() {
    let events = (0..8).map(|index| serde_json::json!({"transaction_number":index+1,"time_seconds":index,"source":"不归每秒回复","before":120,"after":120,"cap":120,"requested_delta":2,"applied_delta":0,"overflow":2,"event_number":null})).collect::<Vec<_>>();
    let result = serde_json::json!({"selector":"berserk_overflow","berserk":{"modeled":true,"overflow_total":16},"berserk_events":events,"start_match":0,"next_start_match":null,"total_matches":8});
    let facts = compact_result_facts(Some(&result));
    assert_eq!(facts["berserk_events"].as_array().unwrap().len(), 8);
    let item = serde_json::json!({"evidence_id":"test","tool_name":"inspect_timeline_events","result":facts});
    let compact = shrink_model_evidence_item(&item, 1200);
    let count = compact["result"]["berserk_events"]
        .as_array()
        .unwrap()
        .len();
    assert!(count > 0 && count < 8);
    assert_eq!(compact["result"]["next_start_match"], count as u64);
    assert_eq!(compact["result"]["resource_events_truncated"], true);
    assert!(model_json_bytes(&compact) <= 1200);
}
