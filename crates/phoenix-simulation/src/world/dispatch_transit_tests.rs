use super::*;

#[test]
fn runtime_gravity_template_or_computed_override_refuses_before_identity_or_effects() {
    for computed in [false, true] {
        let mut fixture = Fixture::new().with_destroyer();
        if !computed {
            let mut body = destroyer_template();
            body.arrival_gravity = Some(phoenix_model::transit::ArrivalGravityConfig {
                exclusion_radius: 8.0,
            });
            fixture.loader = fixture.loader.with_template(DESTROYER_TEMPLATE, body);
        }
        let mut context = fixture.ctx();
        context.uuid_source =
            &|| panic!("unsupported gravity must refuse before consuming actual identity");
        let mut action = spawn(None, Some([100.0, 0.0, 30.0]), vec!["wave"]);
        if computed {
            let TriggerAction::SpawnEntity { overrides, .. } = &mut action else {
                unreachable!()
            };
            *overrides = Some(toml::from_str("[arrival_gravity]\nexclusion_radius=8.0").unwrap());
        }
        let result = dispatch_spawn_entity(&action, &context);
        assert!(result.commands.is_empty());
        assert!(result.new_events.is_empty());
        assert!(result.name_to_uuid_inserts.is_empty());
        assert!(result.entity_group_inserts.is_empty());
        assert_eq!(result.override_failures.len(), 1);
        assert!(result.override_failures[0].contains("base World static rows"));
    }
}

#[test]
fn computed_layer_drive_refuses_unsupported_context_before_uuid_or_effects() {
    let ship = crate::entities::include_resolve::load_entity_config(
        "assets/entities/sector_jump_cruiser.toml",
    )
    .unwrap();
    let mut fixture = Fixture::new();
    fixture.loader = fixture.loader.with_template(DESTROYER_TEMPLATE, ship);
    fixture.origin_layer = Some("assets/worlds/outer.toml".into());
    fixture
        .layers
        .insert("assets/worlds/outer.toml".into(), layer(None));
    let mut context = fixture.ctx();
    context.uuid_source = &|| panic!("unsupported drive context must not mint");
    let action = spawn(None, Some([100.0, 0.0, 30.0]), vec!["wave"]);
    let result = dispatch_spawn_entity(&action, &context);
    assert!(
        result.commands.is_empty()
            && result.name_to_uuid_inserts.is_empty()
            && result.entity_group_inserts.is_empty()
            && result.new_events.is_empty()
    );
    assert_eq!(result.override_failures.len(), 1);
    assert!(result.override_failures[0].contains("Transit source context"));
    fixture
        .layers
        .get_mut("assets/worlds/outer.toml")
        .unwrap()
        .transit_origin_supported = true;
    let accepted = dispatch_spawn_entity(&action, &fixture.ctx());
    assert!(
        accepted.override_failures.is_empty(),
        "{:?}",
        accepted.override_failures
    );
    assert_eq!(accepted.commands.len(), 1);
    assert_eq!(accepted.name_to_uuid_inserts.len(), 1);
}
