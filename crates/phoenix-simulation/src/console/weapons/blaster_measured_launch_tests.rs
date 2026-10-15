use super::*;
use crate::core::messages::{
    AdmittedCommand, AdmittedCommands, SystemBlackboard, SystemId, TacticalRadarBlackboard,
};
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTracks};
use crate::server_app::ShipSystemBlackboards;
use bevy::ecs::system::RunSystemOnce;

fn fixture(height: Option<f32>) -> (World, Entity, String) {
    let authored =
        crate::entities::include_resolve::load_entity_config("assets/entities/pdc_cruiser.toml")
            .unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("tactical-radar".into());
    let profile = &profiles.0[&instrument].profile;
    let mut tracks = BroadTracks::new(&profiles);
    let state = tracks.0.get_mut(&instrument).unwrap();
    // Actual configured acquisition work, rather than a raw actor target.
    for tick in 0..20 {
        state
            .advance(
                profile,
                tick,
                0.5,
                1.0,
                &[BroadMeasurement {
                    source: "incoming-physical-round".into(),
                    selectable: true,
                    observation: None,
                    position: [0.0, -15.0],
                    elevation: height.map(|height| {
                        phoenix_model::sensing_tracks::ObservedElevation {
                            height,
                            error: profile.position_error,
                            velocity: None,
                        }
                    }),
                    support: 1.0,
                    identity: None,
                    classification: Some("torpedo".into()),
                    assessment: None,
                }],
            )
            .unwrap();
    }
    let public = state.projection(profile, 19, 0.5).unwrap();
    assert_eq!(public[0].classification.as_deref(), Some("torpedo"));
    let alias = public[0].contact_id.clone();
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        instrument,
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    let mut world = World::new();
    world.init_resource::<SimOutbox>();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    world.insert_resource(time);
    let ship = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &authored,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    world.flush();
    world.entity_mut(ship).insert((
        crate::server_app::Ship,
        boards,
        tracks,
        AdmittedCommands::default(),
    ));
    let owner = SystemId("blaster-side".into());
    world
        .get_mut::<super::super::assignments::ShipWeaponTargets>(ship)
        .unwrap()
        .0
        .set_individual(owner.clone(), Some(alias.clone()));
    world
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(owner, crate::ship::control_source::ControlSource::Human);
    // Scene Y is deliberately incompatible with the acquired reading. The
    // launcher may not borrow it even when it has the correct private route.
    world.spawn((
        crate::entities::spawner::EntityUuid("incoming-physical-round".into()),
        Transform::from_xyz(1000.0, -900.0, 1000.0),
    ));
    (world, ship, alias)
}

fn start(world: &mut World, ship: Entity) {
    world.get_mut::<AdmittedCommands>(ship).unwrap().0 = vec![AdmittedCommand {
        target: SystemId("blaster-side".into()),
        payload: SystemControlPayload::ChargeBlasterStart,
        response_token: None,
        feedback_correlation: None,
    }];
    world.run_system_once(handle_fire_blaster).unwrap();
    world.get_mut::<AdmittedCommands>(ship).unwrap().0.clear();
}

#[test]
fn actual_manual_launcher_uses_measured_height_real_rig_and_horizontal_recoil() {
    let (mut world, ship, _) = fixture(Some(10.0));
    let owner = SystemId("blaster-side".into());
    let initial = world
        .get::<crate::infrastructure::InfrastructureCondition>(ship)
        .unwrap()
        .0
        .capacity_stock("side-kinetic-rounds")
        .unwrap()
        .level;
    let energy = world
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .consumers[&owner]
        .energy;
    start(&mut world, ship);
    world.run_system_once(tick_blaster_system).unwrap();
    let banks = world.get::<BlasterSystemResource>(ship).unwrap();
    let bank = banks
        .0
        .iter()
        .find(|bank| bank.config.id == "side")
        .unwrap();
    assert_eq!(bank.in_flight.len(), 2);
    let mut expected = [0.0_f32; 2];
    for bolt in &bank.in_flight {
        assert!(
            bolt.pitch > 0.0,
            "aim uses acquired positive elevation, not hidden negative Y"
        );
        assert!(
            bolt.y.abs() < 2.0,
            "launch starts at the actual rig, not the target"
        );
        let [vx, vz] = crate::weapons::operation::recoil_delta(
            bolt.heading,
            bank.config.recoil_impulse * simmath::cos(bolt.pitch),
        )
        .unwrap();
        expected[0] -= vz;
        expected[1] += vx;
    }
    let physics = world.get::<ShipPhysics>(ship).unwrap();
    assert_eq!(physics.forward_speed, expected[0]);
    assert_eq!(physics.lateral_speed, expected[1]);
    assert_eq!(
        world
            .get::<crate::infrastructure::InfrastructureCondition>(ship)
            .unwrap()
            .0
            .capacity_stock("side-kinetic-rounds")
            .unwrap()
            .level,
        initial - 2
    );
    assert!(
        world
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .network()
            .consumers[&owner]
            .energy
            < energy
    );
}

#[test]
fn missing_height_full_3d_range_hold_and_offline_cannot_buy_or_emit_a_round() {
    for mode in 0..4 {
        let (mut world, ship, _) = fixture(if mode == 0 {
            None
        } else if mode == 1 {
            Some(1000.0)
        } else {
            Some(10.0)
        });
        let owner = SystemId("blaster-side".into());
        if mode == 2 {
            world
                .get_mut::<super::super::assignments::ShipWeaponTargets>(ship)
                .unwrap()
                .0
                .set_individual(owner.clone(), None);
        }
        start(&mut world, ship);
        if mode == 3 {
            world
                .get_mut::<ShipSystemControlSources>(ship)
                .unwrap()
                .0
                .set(owner, crate::ship::control_source::ControlSource::Offline);
        }
        let inventory = world
            .get::<crate::infrastructure::InfrastructureCondition>(ship)
            .unwrap()
            .clone();
        let power = world
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .capture_continuation();
        world.run_system_once(tick_blaster_system).unwrap();
        assert!(
            world
                .get::<BlasterSystemResource>(ship)
                .unwrap()
                .0
                .iter()
                .all(|bank| bank.in_flight.is_empty()),
            "mode {mode}"
        );
        assert_eq!(
            *world
                .get::<crate::infrastructure::InfrastructureCondition>(ship)
                .unwrap(),
            inventory
        );
        assert_eq!(
            world
                .get::<crate::ship::power::ShipPowerSystem>(ship)
                .unwrap()
                .0
                .capture_continuation(),
            power
        );
    }
}
