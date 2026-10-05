use super::*;

fn scan_body(source: &str) -> RecipientScriptReferences {
    scan(&[vellum_script::ScriptSource {
        path: "mission.rhai".into(),
        source: source.into(),
    }])
}

#[test]
fn references_are_source_located_and_share_instance_declarations() {
    let refs = scan_body(
        r#"
            fn run(ctx) {
                ctx.effects.add_objective(#{ id: "escort", instance_id: "pair", all_player_ships: true });
                ctx.effects.open_comms(#{ from: "control", node_fn: "hail",
                    recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }],
                    recipient_ship_slots: ["typo"] });
            }
        "#,
    );
    let catalog = RecipientCatalog {
        objective_instances: refs.declarations.clone(),
        ..Default::default()
    };
    let findings = refs.validate(&catalog);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].source.file, "mission.rhai");
    assert_eq!(findings[0].source.line, Some(4));
    assert!(findings[0].message.contains("typo"));
}

#[test]
fn comments_strings_and_computed_values_do_not_become_literal_references() {
    let refs = scan_body(
        r#"
            // ctx.effects.open_comms(#{ recipient_ship_slots: ["bad"] });
            let note = "ctx.effects.addressed(#{ recipient_factions: [\"bad\"] })";
            fn run(ctx) { ctx.effects.open_comms(#{ recipient_ship_slots: choose_slots(),
                recipient_factions: ["Alliance"] }); }
        "#,
    );
    assert_eq!(refs.selections.len(), 1);
    let catalog = RecipientCatalog {
        factions: ["Alliance".to_string()].into(),
        ..Default::default()
    };
    assert!(refs.validate(&catalog).is_empty());
}

#[test]
fn malformed_literal_selection_is_a_save_error() {
    for fields in [
        "all_player_ships: 1",
        "recipient_ship_slots: false",
        "recipient_factions: [\"\"]",
        "recipient_objective_instances: [#{ objective_id: \"x\" }]",
    ] {
        let refs = scan_body(&format!(
            "fn run(ctx) {{ ctx.effects.addressed(#{{ {fields} }}); }}"
        ));
        let errors = refs.validate(&RecipientCatalog::default());
        assert_eq!(errors.len(), 1, "{fields}");
        assert_eq!(errors[0].source.line, Some(1));
    }
}

#[test]
fn string_escape_decoding_matches_runtime() {
    let refs = scan_body(
        r#"fn f(ctx) { ctx.effects.addressed(#{ recipient_ship_slots: ["le\u0061d"] }); }"#,
    );
    let catalog = RecipientCatalog {
        ship_slots: ["lead".to_string()].into(),
        ..Default::default()
    };
    assert_eq!(refs.selections.len(), 1);
    assert!(refs.validate(&catalog).is_empty());
}
#[test]
fn literal_instance_ties_fail_save_for_slots_factions_and_all_players() {
    for selector in [
        r#"recipient_ship_slots: ["lead"]"#,
        r#"recipient_factions: ["Alliance"]"#,
        "all_player_ships: true",
    ] {
        let refs = scan_body(&format!(
            r#"
                fn run(ctx) {{
                    ctx.effects.add_objective(#{{ id: "hold", instance_id: "one", {selector} }});
                    ctx.effects.add_objective(#{{ id: "hold", instance_id: "two", {selector} }});
                }}"#
        ));
        let catalog = RecipientCatalog {
            ship_slots: ["lead".into()].into(),
            factions: ["Alliance".into()].into(),
            ..Default::default()
        };
        let findings = refs.validate(&catalog);
        assert_eq!(findings.len(), 1, "{selector}");
        assert_eq!(findings[0].category, "ambiguous-objective-instances");
        assert_eq!(findings[0].source.file, "mission.rhai");
        assert_eq!(findings[0].source.line, Some(3));
        for label in ["hold", "one", "two", "lead"] {
            assert!(findings[0].message.contains(label));
        }
    }
}

#[test]
fn static_precedence_matches_runtime_and_computed_selectors_defer_to_runtime() {
    let body = r#"
            fn run(ctx) {
                ctx.effects.add_objective(#{ id: "hold", instance_id: "one", recipient_factions: ["Alliance"] });
                ctx.effects.add_objective(#{ id: "hold", instance_id: "two", recipient_factions: ["Alliance"] });
                ctx.effects.add_objective(#{ id: "hold", instance_id: "lead", recipient_ship_slots: ["lead"] });
            }"#;
    let catalog = RecipientCatalog {
        ship_slots: ["lead".into()].into(),
        factions: ["Alliance".into()].into(),
        ..Default::default()
    };
    assert!(
        scan_body(body).validate(&catalog).is_empty(),
        "explicit slot shields the lower faction tie"
    );
    let computed = body.replace(r#"["lead"]"#, "chosen_slots()");
    assert!(
        scan_body(&computed).validate(&catalog).is_empty(),
        "computed higher selectors cannot cause a false static finding"
    );
    let mut more_slots = catalog.clone();
    more_slots.ship_slots.insert("wing".into());
    assert_eq!(
        scan_body(body).validate(&more_slots).len(),
        1,
        "an unshielded slot exposes the faction tie"
    );
}
#[test]
fn computed_declarations_cannot_hide_explicit_ties_or_invent_lower_ties() {
    let catalog = RecipientCatalog {
        ship_slots: ["lead".into()].into(),
        factions: ["Alliance".into()].into(),
        ..Default::default()
    };
    let explicit = r#"fn run(ctx) {
            ctx.effects.add_objective(#{id: "hold", instance_id: "one", recipient_ship_slots: ["lead"]});
            ctx.effects.add_objective(#{id: "hold", instance_id: "two", recipient_ship_slots: ["lead"]});
            ctx.effects.add_objective(#{id: "hold", instance_id: "computed", recipient_ship_slots: choose_slots()});
        }"#;
    assert_eq!(scan_body(explicit).validate(&catalog).len(), 1);
    let opaque_explicit = explicit.replace(
        r#"#{id: "hold", instance_id: "computed", recipient_ship_slots: choose_slots()}"#,
        "chosen_objective()",
    );
    assert_eq!(scan_body(&opaque_explicit).validate(&catalog).len(), 1);
    let lower = r#"fn run(ctx) {
            ctx.effects.add_objective(#{id: "hold", instance_id: "one", recipient_factions: ["Alliance"]});
            ctx.effects.add_objective(#{id: "hold", instance_id: "two", recipient_factions: ["Alliance"]});
            ctx.effects.add_objective(#{id: "hold", instance_id: choose_id(), recipient_ship_slots: ["lead"]});
        }"#;
    assert!(scan_body(lower).validate(&catalog).is_empty());
    assert!(scan_body(&lower.replace(
        r#"id: "hold", instance_id: choose_id()"#,
        r#"id: choose_id(), instance_id: "lead""#
    ))
    .validate(&catalog)
    .is_empty());
    let opaque_lower = lower.replace(
        r#"#{id: "hold", instance_id: choose_id(), recipient_ship_slots: ["lead"]}"#,
        "chosen_objective()",
    );
    assert!(scan_body(&opaque_lower).validate(&catalog).is_empty());
}
