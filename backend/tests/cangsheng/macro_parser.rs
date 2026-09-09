// Included inside the existing test module to reuse its fixtures.
#[test]
fn berserk_resource_aliases_and_rendering_round_trip() {
    for keyword in ["sun", "berserk", "baonu"] {
        for operator in [">", "<", "=", ">=", "<=", "~="] {
            let condition = format!("{keyword}{operator}120");
            for text in [
                format!("/cast [{condition}] 阵云结晦"),
                format!("/cast {condition} 阵云结晦"),
            ] {
                let config = parse_macro_text(&text).unwrap();
                let rendered = render_macro_text(&config);
                let reparsed = parse_macro_text(&rendered).unwrap();
                let line = &reparsed.pages[0].lines[0];
                assert_eq!(line.action.skill_name(), "阵云结晦");
                assert_eq!(
                    line.condition.as_ref().unwrap().display_string(),
                    format!("sun{operator}120")
                );
                assert!(matches!(
                    line.condition,
                    Some(MacroCondition::Berserk(_, 120))
                ));
            }
        }
    }
    assert!(parse_macro_text("/cast [baonu>=abc] 阵云结晦").is_err());
}

#[test]
fn energy_comparisons_round_trip_with_and_without_brackets() {
    for (operator, expected) in [
        (">", CmpOp::Gt),
        ("<", CmpOp::Lt),
        ("=", CmpOp::Eq),
        (">=", CmpOp::GtEq),
        ("<=", CmpOp::LtEq),
        ("~=", CmpOp::Neq),
    ] {
        for command in ["/cast", "/fcast"] {
            for text in [
                format!("{command} [energy{operator}100] 阵云结晦"),
                format!("{command} energy{operator}100 阵云结晦"),
            ] {
                let config = parse_macro_text(&text).unwrap();
                let rendered = render_macro_text(&config);
                let reparsed = parse_macro_text(&rendered).unwrap();
                let line = &reparsed.pages[0].lines[0];
                assert_eq!(line.action.skill_name(), "阵云结晦");
                assert_eq!(line.action.is_fcast(), command == "/fcast");
                let cond = line.condition.as_ref().unwrap();
                assert_eq!(cond.display_string(), format!("energy{operator}100"));
                assert!(matches!(cond, MacroCondition::Energy(op, 100) if *op == expected));
                assert_eq!(cond.clone().semantic_string(), cond.semantic_string());
            }
        }
    }
    for invalid in [
        "energy",
        "energy>=",
        "energy>=abc",
        "energy>=1.5",
        "energy:100",
    ] {
        assert!(parse_macro_text(&format!("/cast [{invalid}] 阵云结晦")).is_err());
    }
}

#[test]
fn energy_optimizer_keeps_resource_types_and_fixed_comparisons_separate() {
    use crate::optimizer::{analyze, render, rule_pool};
    let text = "/cast [energy=120&berserk>=50&rage>20&energy>=100&skill_energy:血怒>1] 阵云结晦";
    let analyzed = analyze::extract_tunables(text).unwrap();
    assert_eq!(
        analyzed
            .params
            .iter()
            .map(|p| (p.key.as_str(), p.visit_idx))
            .collect::<Vec<_>>(),
        vec![("rage", 1), ("energy", 2), ("skill_energy:血怒", 3)]
    );
    assert_eq!(analyzed.params[1].leaf_kind, analyze::LeafKind::Energy);
    assert!(analyzed.params[1].suggested_max > 100.0);
    let mut config = parse_macro_text(text).unwrap();
    analyze::apply_values(&mut config, &analyzed.params, &[44.0, 180.0, 2.0]);
    let expected = "energy=120&sun>=50&rage>44&energy>=180&skill_energy:血怒>2";
    assert_eq!(
        config.pages[0].lines[0]
            .condition
            .as_ref()
            .unwrap()
            .display_string(),
        expected
    );

    let pool = rule_pool::build_pool(text, &[], false).unwrap();
    let rule = pool.rules.get("S1").unwrap();
    assert_eq!(rule.tunables.len(), 3);
    assert_eq!(rule.tunables[1].key, "energy");
    let rendered = render::render_individual(
        &pool,
        &[("S1".to_string(), true)],
        &[],
        &[
            ("S1".to_string(), 1),
            ("S1".to_string(), 2),
            ("S1".to_string(), 3),
        ],
        &[44.0, 180.0, 2.0],
    );
    assert_eq!(
        rendered.config.pages[0].lines[0]
            .condition
            .as_ref()
            .unwrap()
            .display_string(),
        expected
    );
    let high = analyze::extract_tunables("/cast [energy>=190] 盾刀").unwrap();
    assert_eq!(high.params[0].suggested_max, 200.0);
}

#[test]
fn energy_pruning_and_reordering_preserve_resource_semantics() {
    let text = "/cast [energy>=150&rage>20] 盾刀\n/cast [berserk>=50] 阵云结晦";
    let swapped = crate::macro_prune::list_swap_candidates(text).unwrap();
    let config = parse_macro_text(&swapped[0].after_macro).unwrap();
    assert_eq!(
        config.pages[0].lines[1]
            .condition
            .as_ref()
            .unwrap()
            .display_string(),
        "energy>=150&rage>20"
    );
    let tightened = crate::macro_prune::list_tighten_candidates(text).unwrap();
    let energy_variants = tightened
        .iter()
        .filter(|c| c.original == "energy>=150")
        .collect::<Vec<_>>();
    assert_eq!(energy_variants.len(), 2);
    for candidate in energy_variants {
        assert!(matches!(
            candidate.tightened.as_str(),
            "energy>=155" | "energy>=160"
        ));
        assert!(parse_macro_text(&candidate.after_macro).is_ok());
        assert!(candidate.after_macro.contains("sun>=50"));
    }
}
