//! Runtime identity must survive process-local hash seeds without hiding data changes.
use super::AgentRuntime;
use std::collections::HashSet;
use std::sync::Arc;

#[test]
fn independently_loaded_real_catalogs_have_the_same_runtime_identity() {
    let first = AgentRuntime::fixture().with_equipment_fixture();
    let second = AgentRuntime::fixture().with_equipment_fixture();
    assert!(!first.equipment_data().unwrap().items.is_empty());
    assert_eq!(
        first.equipment_identity().unwrap(),
        second.equipment_identity().unwrap()
    );
    assert_eq!(
        crate::harness::run_http::identity(&first).unwrap(),
        crate::harness::run_http::identity(&second).unwrap()
    );
}

#[test]
fn equipment_identity_normalizes_sets_but_keeps_their_contents_and_numeric_data() {
    let mut runtime = AgentRuntime::fixture().with_equipment_fixture();
    let key = *runtime
        .equipment_data()
        .unwrap()
        .items
        .keys()
        .min()
        .unwrap();
    let tags: HashSet<String> = (0..32).map(|n| format!("identity-fixture-{n}")).collect();
    Arc::get_mut(runtime.equip_data.as_mut().unwrap())
        .unwrap()
        .items
        .get_mut(&key)
        .unwrap()
        .attr_tags = tags.clone();
    let expected = runtime.equipment_identity().unwrap();
    let serialized_order = serde_json::to_string(&tags).unwrap();
    let mut different_order = None;
    for _ in 0..64 {
        let rebuilt: HashSet<String> = tags.iter().cloned().collect();
        if serde_json::to_string(&rebuilt).unwrap() != serialized_order {
            different_order = Some(rebuilt);
            break;
        }
    }
    let rebuilt = different_order
        .expect("independent random hash seed must exercise another serialization order");
    Arc::get_mut(runtime.equip_data.as_mut().unwrap())
        .unwrap()
        .items
        .get_mut(&key)
        .unwrap()
        .attr_tags = rebuilt;
    assert_eq!(expected, runtime.equipment_identity().unwrap());

    Arc::get_mut(runtime.equip_data.as_mut().unwrap())
        .unwrap()
        .items
        .get_mut(&key)
        .unwrap()
        .level += 1;
    let changed_numeric = runtime.equipment_identity().unwrap();
    assert_ne!(
        expected, changed_numeric,
        "numeric equipment changes must still invalidate evidence"
    );
    Arc::get_mut(runtime.equip_data.as_mut().unwrap())
        .unwrap()
        .items
        .get_mut(&key)
        .unwrap()
        .attr_tags
        .insert("new-constraint-tag".into());
    assert_ne!(
        changed_numeric,
        runtime.equipment_identity().unwrap(),
        "constraint metadata changes must still invalidate evidence"
    );
}
