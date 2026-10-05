use super::*;

fn insert(manager: &JobManager) -> Result<Arc<JobRecord>, &'static str> {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    let request = serde_json::from_value(json!({"simulation":scenario.simulation,
        "version":"AnYingQianJi","mount":"FenShanJin"}))
    .unwrap();
    manager.insert(
        request,
        scenario,
        "runtime".into(),
        "experiment".into(),
        json!({}),
    )
}

#[test]
fn cancelled_work_keeps_lease_until_it_finishes_and_keeps_result() {
    let manager = JobManager::new();
    let first = insert(&manager).unwrap();
    assert!(insert(&manager).is_err());
    assert!(first.request_cancel());
    assert!(first.request_cancel());
    assert!(manager.active());
    assert!(insert(&manager).is_err());
    first.finish(Ok(
        json!({"stop_reason":"cancelled","simulations":3,"best":{"macro_text":"/cast 盾击"}}),
    ));
    let snapshot = first.snapshot();
    assert!(!snapshot.running);
    assert_eq!(snapshot.status, "cancelled");
    assert_eq!(snapshot.simulations, 3);
    assert_eq!(snapshot.result.unwrap()["best"]["macro_text"], "/cast 盾击");
    assert!(!first.request_cancel());
    assert!(insert(&manager).is_ok());
}

#[test]
fn job_history_is_bounded_and_late_subscribers_see_terminal_state() {
    let manager = JobManager::new();
    let first = insert(&manager).unwrap();
    let old_id = first.id.clone();
    first.finish(Ok(json!({"stop_reason":"time_budget","simulations":2})));
    assert_eq!(first.subscribe().borrow().status, "budget_exhausted");
    for _ in 0..RETAINED_JOBS {
        let record = insert(&manager).unwrap();
        record.finish(Err("没有可用目标轴。".into()));
        assert_eq!(record.snapshot().status, "failed");
    }
    assert_eq!(manager.list().len(), RETAINED_JOBS);
    assert!(manager.get(&old_id).is_none());
    assert!(manager
        .list()
        .iter()
        .all(|r| r.result.is_none() && r.progress.is_none()));
}
