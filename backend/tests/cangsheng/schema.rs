// Included inside the existing test module to reuse its fixtures.
#[test]
fn cangsheng_test_snapshot_has_an_independent_identity_and_only_fenshan() {
    let snapshot = ScenarioSnapshotV1::capture(
        GameVersion::CangShengZhuShiTest,
        Mount::FenShanJin,
        request(),
    )
    .unwrap();
    assert_eq!(snapshot.game_version, "2026_10_cangsheng_zhushi_test");
    snapshot.verify_hash().unwrap();
    for version in [
        GameVersion::AnYingQianJi,
        GameVersion::AnYingQianJiTest,
        GameVersion::ShanHaiYuanLiu,
    ] {
        let other = ScenarioSnapshotV1::capture(version, Mount::FenShanJin, request()).unwrap();
        assert_ne!(snapshot.scenario_hash, other.scenario_hash);
    }
    assert_eq!(
        ScenarioSnapshotV1::capture(GameVersion::CangShengZhuShiTest, Mount::TieGuYi, request())
            .unwrap_err(),
        ScenarioError::InvalidField("mount")
    );
}
