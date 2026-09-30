use super::*;

fn request() -> Start {
    let scene: Value =
        serde_json::from_str(include_str!("../fixtures/exact_macro_short.json")).unwrap();
    serde_json::from_value(scene).unwrap()
}

#[test]
fn http_requires_complete_manual_scene_without_a_wall_time_limit() {
    assert!(request().validate().is_ok());
    assert!(request().compress);
    let mut r = request();
    r.simulation.attributes = None;
    assert!(r.validate().is_err());
    let mut r = request();
    r.simulation.sequence.push("__macro__".into());
    assert!(r.validate().is_err());
    let mut r = request();
    r.horizon = f64::NAN;
    assert!(r.validate().is_err());
}

#[test]
fn jobs_are_isolated_and_completed_jobs_do_not_block_admission() {
    let manager = Manager::default();
    let other = Manager::default();
    let job = Arc::new(Job {
        source: Value::Null,
        id: "test".into(),
        view: Mutex::new(json!({"status":"running"})),
        cancel: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        done: AtomicBool::new(false),
        started: Instant::now(),
        clock: Mutex::new(RunClock::new()),
        revision: AtomicU64::new(1),
        wake: Notify::new(),
    });
    *manager.current.lock().unwrap() = Some(job.clone());
    assert!(manager.active());
    assert!(!other.active());
    assert!(manager.get("different").is_none());
    job.done.store(true, Ordering::Release);
    assert!(!manager.active());
    assert_eq!(job.snapshot()["done"], true);
    // Lightweight polling excludes growing histories and unchanged macro bodies.
    *job.view.lock().unwrap() = json!({"id":"test","best":{"macro":"/cast 盾刀"},"progress":[{"iteration":10}],"target":[1,2,3]});
    let first = job.snapshot_options(&SnapshotQuery {
        compact: true,
        revision: None,
        ..Default::default()
    });
    assert_eq!(first["best"]["macro"], "/cast 盾刀");
    assert!(first.get("progress").is_none() && first.get("target").is_none());
    let default_view = job.snapshot();
    assert!(default_view.get("progress").is_none() && default_view.get("target").is_none());
    assert!(default_view.get("download_ready").is_none());
    let unchanged = job.snapshot_options(&SnapshotQuery {
        compact: true,
        revision: Some(1),
        ..Default::default()
    });
    assert!(unchanged.get("best").is_none());
    *job.view.lock().unwrap() = json!({"id":"test","best":{"macro":"/cast 盾刀"},
        "candidate":{"macro":"/cast 血怒","comparison":null}});
    let first = job.snapshot();
    job.view.lock().unwrap()["stage"] = "compression".into();
    job.view.lock().unwrap()["compression"] = json!({"initial_chars":200,"best_chars":150,"trial_count":12});
    let mut query = SnapshotQuery {
        job_id: Some("test".into()), revision: Some(1),
        best_macro: first["best"]["macro_revision"].as_str().map(str::to_owned),
        candidate_macro: first["candidate"]["macro_revision"].as_str().map(str::to_owned),
        ..Default::default()
    };
    job.revision.store(2, Ordering::Release);
    job.view.lock().unwrap()["candidate"]["comparison"] = json!({"state_prefix":12});
    let progress = job.snapshot_options(&query);
    assert_eq!(progress["stage"], "compression");
    assert_eq!(progress["compression"]["best_chars"], 150);
    assert!(progress["best"].get("macro").is_none());
    assert!(progress["candidate"].get("macro").is_none());
    assert_eq!(progress["candidate"]["comparison"]["state_prefix"], 12);
    job.view.lock().unwrap()["candidate"]["macro"] = "/cast 盾击".into();
    let changed = job.snapshot_options(&query);
    assert!(changed["best"].get("macro").is_none());
    assert_eq!(changed["candidate"]["macro"], "/cast 盾击");
    query.revision = Some(2);
    assert_eq!(job.snapshot_options(&query)["candidate"]["macro"], "/cast 盾击");
    query.job_id = Some("other-job".into());
    assert_eq!(job.snapshot_options(&query)["best"]["macro"], "/cast 盾刀");
    job.paused.store(true, Ordering::Release);
    job.clock.lock().unwrap().set_running(false);
    let frozen = job.snapshot()["elapsed_ms"].clone();
    assert_eq!(job.snapshot()["elapsed_ms"], frozen);
    job.clock.lock().unwrap().set_running(true);
    assert!(job.snapshot()["elapsed_ms"].as_f64().unwrap() >= frozen.as_f64().unwrap());
}
