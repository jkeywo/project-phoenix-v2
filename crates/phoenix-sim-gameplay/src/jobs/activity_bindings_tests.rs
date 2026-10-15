use super::*;

#[test]
fn immutable_goal_team_and_actual_ordinary_visit_share_the_same_support_boundary() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let ship = config.ship_config.as_ref().unwrap();
    let definition = DamageDefinition::compile(
        config.interior.as_ref().unwrap(),
        config.damage.as_ref().unwrap(),
        ship,
    )
    .unwrap();
    let owner = crate::ship::system_registry::repair_system_id();
    let target = RepairWorkTarget::Component {
        system: SystemId("power-reactor".into()),
        component: "operation".into(),
    };
    let exact = RepairTarget::Work(target.clone());
    let supported = |team, request: &RepairTarget, target: &RepairWorkTarget, goal| {
        repair_supported(
            RepairBinding {
                owner: &owner,
                team,
                request,
                target,
                goal,
            },
            ship,
            &definition,
            &owner,
            2,
        )
    };
    assert!(supported(0, &exact, &target, RepairGoal::Full));
    assert!(supported(0, &exact, &target, RepairGoal::Patch));
    assert!(!supported(2, &exact, &target, RepairGoal::Full));
    assert!(
        !repair_supported(
            RepairBinding {
                owner: &owner,
                team: 256,
                request: &exact,
                target: &target,
                goal: RepairGoal::Full
            },
            ship,
            &definition,
            &owner,
            257,
        ),
        "ordinary dispatch team indices are u8 even when a malformed roster is larger"
    );
    assert!(!supported(0, &exact, &target, RepairGoal::Structure));
    let station = RepairTarget::Station(phoenix_model::messages::StationId("engineering".into()));
    let selected = resolve_repair_visit(&station, ship, &definition).unwrap();
    let actual_room = &definition.interior.placements[&selected];
    let local = definition
        .components
        .keys()
        .find(|system| definition.interior.placements.get(*system) == Some(actual_room))
        .unwrap();
    let component = definition.components[local].keys().next().unwrap();
    let in_visit = RepairWorkTarget::Component {
        system: local.clone(),
        component: component.clone(),
    };
    assert!(supported(0, &station, &in_visit, RepairGoal::Full));
    assert_ne!(
        definition.interior.placements[&SystemId("power-reactor".into())],
        *actual_room
    );
    assert!(!supported(0, &station, &target, RepairGoal::Full));
    assert!(!supported(
        0,
        &RepairTarget::External,
        &target,
        RepairGoal::Full
    ));
}

#[test]
fn delivery_support_uses_actual_approved_route_and_batch_bound_without_stock_polling() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&config)
        .unwrap()
        .unwrap();
    let owner = &profile.handling().system;
    assert!(delivery_supported(
        &profile,
        owner,
        "hold-ammunition",
        "magazine",
        "torpedoes",
        2
    ));
    for (source, destination, commodity, amount) in [
        ("magazine", "hold-ammunition", "torpedoes", 2),
        ("hold-ammunition", "magazine", "parts", 2),
        ("hold-ammunition", "missing", "torpedoes", 2),
        ("hold-ammunition", "magazine", "torpedoes", 0),
        (
            "hold-ammunition",
            "magazine",
            "torpedoes",
            profile.handling().max_batch + 1,
        ),
    ] {
        assert!(!delivery_supported(
            &profile,
            owner,
            source,
            destination,
            commodity,
            amount
        ));
    }
    assert!(!delivery_supported(
        &profile,
        &SystemId("foreign".into()),
        "hold-ammunition",
        "magazine",
        "torpedoes",
        2
    ));
}
