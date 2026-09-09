// Included inside the existing test module to reuse its fixtures.
#[test]
fn cangsheng_test_knowledge_never_falls_back_to_current_or_old_test_seasons() {
    let fixture = Fixture::create();
    let context = KnowledgeVersionContext::from_game_version(GameVersion::CangShengZhuShiTest);
    let index = KnowledgeIndex::load(&fixture.root).unwrap();
    assert_eq!(context.current_season, "苍生铸世测试服（2026）");
    let missing = index
        .search(
            &context,
            query(KnowledgeVersionScope::CurrentOnly, "盾飞循环"),
        )
        .unwrap();
    assert!(missing.results.is_empty());
    assert!(matches!(
        index.search(
            &context,
            query(KnowledgeVersionScope::CurrentOnly, "暗影千机盾飞循环")
        ),
        Err(KnowledgeIndexError::VersionConflict { .. })
    ));

    let manifest_path = fixture.root.join(MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entries"]
        .as_array_mut()
        .unwrap()
        .push(fixture_entry(
            &fixture.root,
            "cangsheng_test.md",
            "苍生铸世测试服分山劲技改",
            context.current_season,
            "130级",
            "external_mirror",
            "full",
            "# 盾飞循环\n苍生铸世测试服分山劲盾飞循环使用独立暴怒值。",
        ));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let index = KnowledgeIndex::load(&fixture.root).unwrap();
    let response = index
        .search(
            &context,
            query(KnowledgeVersionScope::CurrentOnly, "盾飞循环"),
        )
        .unwrap();
    assert!(!response.results.is_empty());
    assert!(response
        .results
        .iter()
        .all(|result| result.season == context.current_season));
    let current = index
        .search(
            &KnowledgeVersionContext::from_game_version(GameVersion::AnYingQianJi),
            query(KnowledgeVersionScope::CurrentOnly, "盾飞循环"),
        )
        .unwrap();
    assert!(!current.results.is_empty());
    assert!(current
        .results
        .iter()
        .all(|result| result.season == "暗影千机（2026）"));
}
