use super::*;
use crate::console::weapons::assignments::ShipWeaponTargets;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    fixture_from("assets/entities/alliance_cruiser.toml")
}
fn fixture_from(path: &str) -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(path).unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "assigned-ship".into(),
        None,
    );
    world.flush();
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySpawnOrigin(
            crate::world::spawn_origin::SpawnOrigin {
                template_path: path.into(),
                name: "assigned-ship".into(),
                ..Default::default()
            },
        ));
    let mut targets = world.get_mut::<ShipWeaponTargets>(entity).unwrap();
    let owner = SystemId("phaser-fore".into());
    targets.0.set_group(
        owner.clone(),
        "main-beams".into(),
        Some("retained-public-contact".into()),
    );
    targets.0.set_individual(owner, None);
    let snapshot = capture(&world);
    assert!(
        snapshot.entities[0].weapons.is_some(),
        "the actual machinery row must be captured"
    );
    (world, entity, snapshot)
}
#[test]
fn actual_spawn_capture_json_apply_preserve_hold_and_lost_group_without_target_truth() {
    let (mut world, entity, saved) = fixture();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &saved).unwrap();
    super::super::preflight::validate_entity_resources(&world, entity, &saved.entities[0]).unwrap();
    super::super::validate_entity_repair_jobs(&world, entity, &saved.entities[0]).unwrap();
    let owner = SystemId("phaser-fore".into());
    world
        .get_mut::<ShipWeaponTargets>(entity)
        .unwrap()
        .0
        .follow_primary(&owner);
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &saved.entities[0],
        saved.tick,
    );
    assert_eq!(
        capture(&world).entities[0].weapon_targets,
        saved.entities[0].weapon_targets
    );
    let mut targets = world.get_mut::<ShipWeaponTargets>(entity).unwrap();
    assert_eq!(targets.0.target_for(&owner, Some("primary")), None);
    targets.0.follow_group(&owner);
    assert_eq!(
        targets.0.target_for(&owner, Some("primary")),
        Some("retained-public-contact")
    );
}
#[test]
fn forged_ownership_groups_contacts_and_missing_owner_refuse_before_any_write() {
    for case in 0..5 {
        let (mut world, _, mut saved) = fixture();
        let before = capture(&world);
        if case == 4 {
            saved.entities[0].weapon_targets = None;
        } else {
            let targets = saved.entities[0].weapon_targets.as_mut().unwrap();
            match case {
                0 => {
                    targets.set_individual(SystemId("power".into()), None);
                }
                1 => {
                    targets
                        .0
                        .get_mut(&SystemId("phaser-fore".into()))
                        .unwrap()
                        .group
                        .as_mut()
                        .unwrap()
                        .group = "missiles".into();
                }
                2 => {
                    targets.set_individual(
                        SystemId("phaser-fore".into()),
                        Some("bad\ncontact".into()),
                    );
                }
                _ => {
                    targets
                        .0
                        .insert(SystemId("phaser-aft".into()), Default::default());
                }
            }
        }
        let report = restore(&mut world, &saved);
        assert!(!report.is_complete(), "case {case}");
        assert_eq!(capture(&world), before, "case {case} mutated state");
    }
}

#[test]
fn automatic_only_rows_round_trip_and_bad_fallbacks_refuse_before_writes() {
    let (mut world, entity, _) = fixture_from("assets/entities/broadside_cruiser.toml");
    let owner = SystemId("blaster-side".into());
    world
        .get_mut::<ShipWeaponTargets>(entity)
        .unwrap()
        .0
        .set_automatic(owner.clone(), Some("retained-opaque-contact".into()));
    let saved = capture(&world);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &saved).unwrap();
    world
        .get_mut::<ShipWeaponTargets>(entity)
        .unwrap()
        .0
        .set_automatic(owner.clone(), None);
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &saved.entities[0],
        saved.tick,
    );
    assert_eq!(
        capture(&world).entities[0].weapon_targets,
        saved.entities[0].weapon_targets
    );
    let before = capture(&world);
    for contact in ["", "bad\ncontact"] {
        let mut forged = saved.clone();
        forged.entities[0]
            .weapon_targets
            .as_mut()
            .unwrap()
            .set_automatic(owner.clone(), Some(contact.into()));
        assert!(!restore(&mut world, &forged).is_complete());
        assert_eq!(capture(&world), before);
    }
    let mut forged = saved;
    forged.entities[0]
        .weapon_targets
        .as_mut()
        .unwrap()
        .set_automatic(
            SystemId("phaser-fore".into()),
            Some("otherwise-valid-handle".into()),
        );
    assert!(!restore(&mut world, &forged).is_complete());
    assert_eq!(capture(&world), before);
}
