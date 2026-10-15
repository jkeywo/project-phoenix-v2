use super::*;
use crate::console::weapons::BlasterSystemResource;
use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};

fn emitted() -> BlasterSystemResource {
    let mut bank = BlasterSystem::new(BlasterBankConfig {
        id: "pdc".into(),
        volley_count: 1,
        projectile_speed: 10.0,
        range: 100.0,
        collision_radius: 0.1,
        ..Default::default()
    });
    bank.request_fire();
    assert_eq!(
        bank.tick_with_origins_3d(
            0.1,
            0.1,
            &[Some([0.0, 5.0, 0.0])],
            0.0,
            [0.0, 15.0, -10.0],
            [0.0; 3],
            "launcher",
            &mut |_| true,
            &mut || "emitted-bolt".into(),
        )
        .len(),
        1
    );
    bank.advance_flight_only(0.25);
    bank.finish_collision_interval();
    BlasterSystemResource(vec![bank])
}

fn saved(banks: &BlasterSystemResource) -> WeaponState {
    capture::weapon_state(None, None, None, None, Some(banks), None)
}

#[test]
fn settled_xyz_json_continuation_uses_frozen_flight_and_never_replays_damage() {
    let mut original = emitted();
    let snapshot = saved(&original);
    let decoded: WeaponState =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut world = World::new();
    let mut changed = BlasterSystem::new(BlasterBankConfig {
        id: "pdc".into(),
        projectile_speed: 999.0,
        damage: 999,
        ..Default::default()
    });
    changed.config.volley_count = 0;
    let owner = world.spawn(BlasterSystemResource(vec![changed])).id();
    preflight::validate_physical_attacks(
        &world,
        &EntityState {
            weapons: Some(decoded.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    application::apply_weapons(&mut world.entity_mut(owner), &decoded);
    let bodies = [("already-passed".into(), [0.0, 5.5, -0.5], 0.1)];
    assert!(world.get::<BlasterSystemResource>(owner).unwrap().0[0]
        .find_hits_3d(&bodies)
        .is_empty());
    // Supported captures follow Damage. Its previous segment is scratch: a
    // paused consumer has nothing to repeat, and Physics derives the next one.
    for _ in 0..360 {
        original.0[0].advance_flight_only(0.01);
        original.0[0].finish_collision_interval();
        let mut restored = world.get_mut::<BlasterSystemResource>(owner).unwrap();
        restored.0[0].advance_flight_only(0.01);
        restored.0[0].finish_collision_interval();
        assert_eq!(saved(&original), saved(&restored));
    }
    let mut missing = serde_json::to_value(&decoded).unwrap();
    missing["blasters"][0]["in_flight"][0]
        .as_object_mut()
        .unwrap()
        .remove("y");
    assert!(serde_json::from_value::<WeaponState>(missing).is_err());
}

#[test]
fn malformed_emitted_xyz_frontiers_refuse_without_replacing_live_flight() {
    let mut world = World::new();
    let banks = emitted();
    let before = saved(&banks);
    let owner = world.spawn(banks).id();
    for case in 0..8 {
        let mut invalid = before.clone();
        let bolt = &mut invalid.blasters[0].in_flight[0];
        match case {
            0 => bolt.y = f32::INFINITY,
            1 => bolt.pitch = std::f32::consts::PI,
            2 => bolt.speed = 0.0,
            3 => bolt.lifespan_remaining = 0.0,
            4 => bolt.collision_radius = -1.0,
            5 => bolt.damage = -1,
            6 => bolt.shield_pierce = 1.1,
            _ => bolt.impact.penetration = f32::NAN,
        }
        assert!(
            preflight::validate_physical_attacks(
                &world,
                &EntityState {
                    weapons: Some(invalid),
                    ..Default::default()
                }
            )
            .is_err(),
            "case {case}"
        );
        assert_eq!(
            saved(world.get::<BlasterSystemResource>(owner).unwrap()),
            before
        );
    }
}
