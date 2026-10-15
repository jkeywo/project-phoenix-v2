use super::*;
use phoenix_model::weapon_operation::BeamOperation;

#[test]
fn sustained_capture_json_apply_retains_operation_debt_and_visual_owner_without_relight() {
    let mut beam = ActiveBeam::default();
    beam.start("aft", "known-target", 0.05, 3.0);
    let slot = beam.bank_slot_mut("aft").unwrap();
    slot.operation = BeamOperation::Sustained;
    slot.damage_accumulator = 0.625;
    slot.strike_damage_bonus = 0.25;
    let saved = capture::weapon_state(Some(&beam), None, None, None, None, None);
    assert_eq!(saved.beams[0].5, BeamOperation::Sustained);
    let decoded: WeaponState =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut world = World::new();
    let entity = world
        .spawn((ActiveBeam::default(), PhaserCooldown::default()))
        .id();
    world.register_component::<crate::entities::spawner::EntityUuid>();
    world.register_component::<crate::ship::state::ShipPhysics>();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::ship::state::ShipRedAlert>();
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntityUuid(
            "sustained-owner".into(),
        ));
    let row = EntityState {
        weapons: Some(decoded.clone()),
        ..Default::default()
    };
    preflight::validate_physical_attacks(&world, &row).unwrap();
    application::apply_weapons(&mut world.entity_mut(entity), &decoded);
    let restored = world.get::<ActiveBeam>(entity).unwrap();
    // Presentation renders live_banks directly; the restored slot is already
    // visible without a fresh BeamStarted event or reset of fractional damage.
    assert_eq!(restored.live_banks().count(), 1);
    assert_eq!(
        restored.live_banks().next().unwrap().1,
        beam.live_banks().next().unwrap().1
    );
    assert_eq!(
        capture::weapon_state(Some(restored), None, None, None, None, None),
        saved
    );
    let sustained_digest = crate::sim_digest::world_digest(&world);
    world
        .get_mut::<ActiveBeam>(entity)
        .unwrap()
        .bank_slot_mut("aft")
        .unwrap()
        .operation = BeamOperation::Fixed;
    assert_ne!(
        crate::sim_digest::world_digest(&world),
        sustained_digest,
        "equal finite durations with different continuation operation must fold differently"
    );
    let mut missing = serde_json::to_value(&saved).unwrap();
    missing["beams"][0].as_array_mut().unwrap().pop();
    assert!(
        serde_json::from_value::<WeaponState>(missing).is_err(),
        "active operation cannot silently default on resume"
    );
}
#[test]
fn malformed_active_frontiers_refuse_before_replacing_existing_live_slots() {
    let mut world = World::new();
    let mut beam = ActiveBeam::default();
    beam.start("fore", "existing", 1.0, 2.0);
    let entity = world.spawn(beam.clone()).id();
    let before = capture::weapon_state(Some(&beam), None, None, None, None, None);
    for variant in 0..4 {
        let mut invalid = before.clone();
        match variant {
            0 => invalid.beams[0].2 = f32::NAN,
            1 => invalid.beams[0].3 = 1.0,
            2 => invalid.beams[0].2 = 0.0,
            _ => invalid.beams.push(invalid.beams[0].clone()),
        }
        let row = EntityState {
            weapons: Some(invalid),
            ..Default::default()
        };
        assert!(preflight::validate_physical_attacks(&world, &row).is_err());
        assert_eq!(
            capture::weapon_state(
                world.get::<ActiveBeam>(entity),
                None,
                None,
                None,
                None,
                None
            ),
            before
        );
    }
}
