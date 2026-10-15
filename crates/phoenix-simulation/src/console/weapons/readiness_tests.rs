use super::*;
use crate::core::messages::{
    SystemBlackboard, SystemId, TacticalRadarBlackboard, ViewscreenBlackboard,
};
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTracks};
use crate::server_app::ShipSystemBlackboards;
use crate::ship_plugin::ShipConfigComponent;
use bevy::ecs::system::RunSystemOnce;
use std::collections::BTreeMap;

fn fixture() -> (World, Entity, String) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("tactical-radar".into());
    let mut tracks = BroadTracks::new(&profile);
    let state = tracks.0.get_mut(&instrument).unwrap();
    // This read-only publisher fixture starts with one paid instrument's held
    // measurement. No publisher is permitted to query the hidden actor pose.
    state
        .advance(
            &profile.0[&instrument].profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                observation: None,
                source: "private-target".into(),
                selectable: true,
                position: [10.0, 0.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let measured = state
        .projection(&profile.0[&instrument].profile, 0, 0.5)
        .unwrap();
    let alias = measured[0].contact_id.clone();
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        instrument,
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(measured),
            ..Default::default()
        }),
    );
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            combat_lock: Some("private-target".into()),
            ..Default::default()
        }),
    );
    let config = authored.ship_config.as_ref().unwrap();
    let owner = SystemId("phaser-fore".into());
    let mounts = crate::weapons::mounts::MountProfiles(BTreeMap::from([(
        owner.clone(),
        phoenix_model::turret::TurretConfig {
            facing_deg: 0.0,
            traverse_arc_deg: 270.0,
            traverse_deg_per_sec: 45.0,
            aim_tolerance_deg: 2.0,
        },
    )]));
    let states = crate::weapons::mounts::ShipMounts::new(&mounts);
    let mut targets = super::super::super::assignments::ShipWeaponTargets::default();
    targets.0.set_individual(owner, Some(alias.clone()));
    let mut world = World::new();
    world.init_resource::<CurrentPhaserMode>();
    world.init_resource::<crate::lobby::server::ShipClientConfigResource>();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &authored,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    world.flush();
    world.entity_mut(entity).insert((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        ShipConfigComponent(config.clone()),
        boards,
        tracks,
        mounts,
        states,
        targets,
    ));
    // An actual actor at a completely different live point cannot improve aim.
    world.spawn((
        crate::entities::spawner::EntityUuid("private-target".into()),
        Transform::from_xyz(900.0, 0.0, 900.0),
    ));
    (world, entity, alias)
}
fn published(
    world: &mut World,
    entity: Entity,
) -> (
    crate::core::messages::WeaponsBlackboard,
    crate::core::messages::PhaserBankBlackboard,
    LastWeaponsUpdate,
) {
    world
        .run_system_once(publish_weapons_core_blackboard)
        .unwrap();
    world
        .run_system_once(publish_phaser_bank_blackboards)
        .unwrap();
    let boards = world.get::<ShipSystemBlackboards>(entity).unwrap();
    let SystemBlackboard::Weapons(core) =
        &boards.0[&crate::ship::system_registry::tactical_station_key()]
    else {
        panic!("Weapons")
    };
    let SystemBlackboard::PhaserBank(fine) = &boards.0[&SystemId("phaser-fore".into())] else {
        panic!("Bank")
    };
    let (core, fine) = (core.clone(), fine.clone());
    (core, fine, compute_current_weapons_update(world))
}

fn aim_at_held_measurement(world: &mut World, entity: Entity, owner: &SystemId) {
    let boards = world.get::<ShipSystemBlackboards>(entity).unwrap();
    let SystemBlackboard::TacticalRadar(board) = &boards.0[&SystemId("tactical-radar".into())]
    else {
        unreachable!()
    };
    let point = board.tracks.as_ref().unwrap()[0].position;
    let physics = world
        .get::<ShipPhysics>(entity)
        .copied()
        .unwrap_or_default();
    let (x, forward) =
        crate::weapons::mount::ship_local(point[0], point[1], physics.x, physics.z, physics.yaw);
    let bearing = crate::simmath::atan2(x, forward).to_degrees();
    let facing = world
        .get::<crate::weapons::mounts::MountProfiles>(entity)
        .unwrap()
        .0[owner]
        .facing_deg;
    world
        .get_mut::<crate::weapons::mounts::ShipMounts>(entity)
        .unwrap()
        .0
        .get_mut(owner)
        .unwrap()
        .offset_deg = f64::from(bearing - facing);
}
#[test]
fn ordinary_core_fine_and_reconnect_readiness_agree_with_measured_point_and_real_bearing() {
    let (mut world, entity, _) = fixture();
    let owner = SystemId("phaser-fore".into());
    let expected_range = {
        let boards = world.get::<ShipSystemBlackboards>(entity).unwrap();
        let SystemBlackboard::TacticalRadar(board) = &boards.0[&SystemId("tactical-radar".into())]
        else {
            unreachable!()
        };
        let point = board.tracks.as_ref().unwrap()[0].position;
        point[0].hypot(point[1])
    };
    assert!(
        expected_range < 100.0,
        "measured point differs from hidden actual pose"
    );
    let (core, fine, reconnect) = published(&mut world, entity);
    let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
    assert_eq!(bank.readiness.target_range, Some(expected_range));
    assert!(!bank.fire_ready && !fine.fire_ready);
    assert_eq!(
        bank.readiness,
        reconnect
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .readiness
    );
    aim_at_held_measurement(&mut world, entity, &owner);
    let (core, fine, reconnect) = published(&mut world, entity);
    let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
    assert!(bank.fire_ready && fine.fire_ready);
    assert_eq!(bank.readiness.target_range, Some(expected_range));
    assert_eq!(
        bank.readiness,
        reconnect
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .readiness
    );
}
#[test]
fn explicit_hold_lost_feed_and_conceal_never_borrow_primary_or_live_world_geometry() {
    for mode in 0..3 {
        let (mut world, entity, _) = fixture();
        let owner = SystemId("phaser-fore".into());
        aim_at_held_measurement(&mut world, entity, &owner);
        assert!(published(&mut world, entity).1.fire_ready);
        match mode {
            0 => world
                .get_mut::<super::super::super::assignments::ShipWeaponTargets>(entity)
                .unwrap()
                .0
                .set_individual(owner, None),
            1 => world
                .get_mut::<BroadTracks>(entity)
                .unwrap()
                .0
                .get_mut(&SystemId("tactical-radar".into()))
                .unwrap()
                .current
                .clear(),
            _ => {
                world.init_resource::<crate::world::server::WorldContentRuntime>();
                crate::gm::operations::contact::set(
                    &mut world
                        .resource_mut::<crate::world::server::WorldContentRuntime>()
                        .contact_overrides,
                    "observer",
                    "private-target",
                    crate::gm::operations::contact::ContactMode::Conceal,
                );
            }
        }
        let (core, fine, reconnect) = published(&mut world, entity);
        let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
        assert_eq!(bank.readiness.target_range, None, "mode {mode}");
        assert!(!fine.fire_ready);
        assert_eq!(
            bank.readiness,
            reconnect
                .banks
                .iter()
                .find(|bank| bank.id == "fore")
                .unwrap()
                .readiness
        );
    }
}

#[test]
fn active_flag_reports_actual_burn_separately_from_post_cycle_rest() {
    let (mut world, entity, _) = fixture();
    world
        .get_mut::<ActiveBeam>(entity)
        .unwrap()
        .start("fore", "private-target", 3.0, 2.0);
    let (core, _, reconnect) = published(&mut world, entity);
    let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
    assert!(bank.firing && bank.on_cooldown);
    assert!(
        reconnect
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .firing
    );
    world
        .get_mut::<ActiveBeam>(entity)
        .unwrap()
        .end_bank("fore");
    world
        .get_mut::<PhaserCooldown>(entity)
        .unwrap()
        .start_bank_with_cooldown("fore", 2.0);
    let (core, _, reconnect) = published(&mut world, entity);
    let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
    assert!(!bank.firing && bank.on_cooldown);
    assert!(
        !reconnect
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .firing
    );
}

#[test]
fn actual_core_fine_and_reconnect_target_filters_use_only_held_classification() {
    use phoenix_model::tags::EntityTag;
    let (mut world, entity, _) = fixture();
    let owner = SystemId("phaser-fore".into());
    aim_at_held_measurement(&mut world, entity, &owner);
    assert!(published(&mut world, entity).1.fire_ready);
    // Unknown held classification cannot satisfy an explicit tag, regardless
    // of the real actor's private definition/position elsewhere in the scene.
    world
        .get_mut::<PhaserCombatConfigResource>(entity)
        .unwrap()
        .0
        .banks
        .iter_mut()
        .find(|bank| bank.id == "fore")
        .unwrap()
        .targets = Some(vec![EntityTag::Missile]);
    for classification in [None, Some("ship"), Some("missile")] {
        let mut boards = world.get_mut::<ShipSystemBlackboards>(entity).unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!();
        };
        board.tracks.as_mut().unwrap()[0].classification = classification.map(str::to_owned);
        drop(boards);
        let (core, fine, reconnect) = published(&mut world, entity);
        let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
        assert_eq!(fine.fire_ready, classification == Some("missile"));
        assert_eq!(bank.fire_ready, fine.fire_ready);
        assert_eq!(
            bank.readiness,
            reconnect
                .banks
                .iter()
                .find(|bank| bank.id == "fore")
                .unwrap()
                .readiness
        );
    }
    world
        .get_mut::<PhaserCombatConfigResource>(entity)
        .unwrap()
        .0
        .banks
        .iter_mut()
        .find(|bank| bank.id == "fore")
        .unwrap()
        .targets = Some(vec![]);
    assert!(
        !published(&mut world, entity).1.fire_ready,
        "empty explicit policy holds even on a measured missile"
    );
}
#[test]
fn omitted_policy_does_not_turn_a_nonordinary_route_into_ready_geometry() {
    let (mut world, entity, _) = fixture();
    let owner = SystemId("phaser-fore".into());
    aim_at_held_measurement(&mut world, entity, &owner);
    assert!(published(&mut world, entity).1.fire_ready);
    let physical = world
        .query::<(Entity, &crate::entities::spawner::EntityUuid)>()
        .iter(&world)
        .find_map(|(entity, id)| (id.0 == "private-target").then_some(entity))
        .unwrap();
    world
        .entity_mut(physical)
        .remove::<crate::entities::spawner::EntityUuid>();
    let (core, fine, reconnect) = published(&mut world, entity);
    assert!(!fine.fire_ready);
    let bank = core.banks.iter().find(|bank| bank.id == "fore").unwrap();
    assert_eq!(bank.readiness.target_range, None);
    assert_eq!(
        bank.readiness,
        reconnect
            .banks
            .iter()
            .find(|bank| bank.id == "fore")
            .unwrap()
            .readiness
    );
}
