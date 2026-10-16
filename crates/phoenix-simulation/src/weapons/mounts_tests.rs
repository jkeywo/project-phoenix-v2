use super::*;

#[test]
fn aiming_uses_only_the_same_permitted_current_measurement_and_conceal_wins() {
    use crate::core::messages::{SensorRadarBlackboard, SystemBlackboard};
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState, BroadTracks};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = authored.ship_config.as_ref().unwrap();
    let profile = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("sensor-radar".into());
    let owner = SystemId("passive-receiver".into());
    let profile = &profile.0[&instrument].profile;
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                source: "private-actor".into(),
                selectable: true,
                observation: None,
                position: [40.0, 10.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let public = state.projection(profile, 0, 0.5).unwrap();
    let measured = public[0].position;
    let contact = public[0].contact_id.clone();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        instrument.clone(),
        SystemBlackboard::SensorRadar(SensorRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    let tracks = BroadTracks(BTreeMap::from([(instrument, state)]));
    assert_eq!(
        current_point(config, &boards, &tracks, &owner, &contact, "observer", None),
        Some(measured)
    );
    assert!(measured_point(config, &boards, &tracks, &owner, "private-actor").is_none());
    assert!(measured_point(
        config,
        &boards,
        &tracks,
        &SystemId("unowned".into()),
        &contact
    )
    .is_none());
    let mut overrides = crate::gm::operations::contact::ContactOverrides::default();
    crate::gm::operations::contact::set(
        &mut overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    assert!(current_point(
        config,
        &boards,
        &tracks,
        &owner,
        &contact,
        "observer",
        Some(&overrides)
    )
    .is_none());
    // A different observer's conceal cannot revoke this Console's picture.
    assert_eq!(
        current_point(
            config,
            &boards,
            &tracks,
            &owner,
            &contact,
            "another",
            Some(&overrides)
        ),
        Some(measured)
    );
    let SystemBlackboard::SensorRadar(board) =
        boards.0.get_mut(&SystemId("sensor-radar".into())).unwrap()
    else {
        unreachable!()
    };
    let records = board.tracks.as_mut().unwrap();
    records.push(records[0].clone());
    assert_eq!(
        measured_point(config, &boards, &tracks, &owner, &contact),
        Some(measured)
    );
    let SystemBlackboard::SensorRadar(board) =
        boards.0.get_mut(&SystemId("sensor-radar".into())).unwrap()
    else {
        unreachable!()
    };
    board.tracks.as_mut().unwrap()[1].position[0] += 1.0;
    assert!(
        measured_point(config, &boards, &tracks, &owner, &contact).is_none(),
        "conflicting current copies cannot pick an arbitrary firing point"
    );
}
fn profile() -> (
    SystemId,
    MountProfiles,
    ShipMounts,
    crate::ship::power::ShipPowerSystem,
) {
    let entity = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let owner = SystemId("phaser-fore".into());
    let profiles = MountProfiles(BTreeMap::from([(
        owner.clone(),
        TurretConfig {
            facing_deg: 0.0,
            traverse_arc_deg: 270.0,
            traverse_deg_per_sec: 90.0,
            aim_tolerance_deg: 1.0,
        },
    )]));
    let states = ShipMounts::new(&profiles);
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        entity.energy.unwrap(),
        entity.ship_config.unwrap(),
    )
    .unwrap();
    (
        owner,
        profiles,
        states,
        crate::ship::power::ShipPowerSystem(power),
    )
}

#[test]
fn actual_local_reserve_caps_motion_replay_does_not_pay_and_unavailable_work_freezes() {
    let (owner, profiles, mut states, mut power) = profile();
    let rate = power
        .0
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == owner)
        .unwrap()
        .continuous_cost_per_sec;
    assert!(rate > 0.0);
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = rate * 0.1;
    advance_system(
        &profiles,
        &mut states,
        &owner,
        0,
        1.0,
        true,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert!(states.0[&owner].offset_deg > 0.0 && states.0[&owner].offset_deg < 90.0);
    let paid = power.0.capture_continuation();
    let emitted = power.0.emitted_work(&owner);
    let before = states.clone();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        0,
        100.0,
        true,
        Some(-90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(states, before);
    assert_eq!(power.0.capture_continuation(), paid);
    assert_eq!(power.0.emitted_work(&owner), emitted);
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&owner)
        .unwrap()
        .energy = 0.0;
    advance_system(
        &profiles,
        &mut states,
        &owner,
        1,
        1.0,
        true,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(states.0[&owner].offset_deg, before.0[&owner].offset_deg);
    let paid = power.0.capture_continuation();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        2,
        1.0,
        false,
        Some(90.0),
        1.0,
        &mut power,
    )
    .unwrap();
    advance_system(
        &profiles,
        &mut states,
        &owner,
        3,
        1.0,
        true,
        Some(90.0),
        0.0,
        &mut power,
    )
    .unwrap();
    assert_eq!(power.0.capture_continuation(), paid);
    assert_eq!(states.0[&owner].offset_deg, before.0[&owner].offset_deg);
}

#[test]
fn static_geometry_remains_equivalent_and_authored_missing_state_cannot_fire() {
    let (owner, profiles, states, _) = profile();
    let mount = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 270.0,
        range: 100.0,
    };
    for target in [[0.0, -40.0], [40.0, 0.0], [0.0, 110.0]] {
        assert_eq!(
            geometry(None, None, &owner, &mount, target, [0.0, 0.0], 0.0),
            mount.geometry(target[0], target[1], 0.0, 0.0, 0.0)
        );
    }
    assert!(
        !geometry(
            Some(&profiles),
            None,
            &owner,
            &mount,
            [0.0, -40.0],
            [0.0, 0.0],
            0.0
        )
        .in_arc
    );
    assert!(
        geometry(
            Some(&profiles),
            Some(&states),
            &owner,
            &mount,
            [0.0, -40.0],
            [0.0, 0.0],
            0.0
        )
        .in_arc
    );
}

#[test]
fn sensor_field_of_view_uses_the_single_actual_bearing_instead_of_weapon_tolerance() {
    let (owner, profiles, mut states, _) = profile();
    let mount = MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 30.0,
        range: 100.0,
    };
    states
        .0
        .get_mut(&owner)
        .unwrap()
        .advance(&profiles.0[&owner], 0, 1.0, true, Some(90.0), 1.0)
        .unwrap();
    let rotated = sensor_mount(Some(&profiles), Some(&states), &owner, &mount).unwrap();
    assert_eq!(rotated.facing_deg, 90.0);
    assert_eq!(rotated.fire_arc_deg, 30.0);
    assert!(rotated.geometry(40.0, 0.0, 0.0, 0.0, 0.0).in_arc);
    assert!(!rotated.geometry(0.0, -40.0, 0.0, 0.0, 0.0).in_arc);
    assert!(sensor_mount(Some(&profiles), None, &owner, &mount).is_none());
}

#[test]
fn actual_tick_preserves_bearing_and_power_during_refit_and_vacancy_then_resumes() {
    use crate::core::messages::{SystemBlackboard, TacticalRadarBlackboard, ViewscreenBlackboard};
    use crate::resources::fitting_server::{
        CompiledShipFitting, RefitStage, RefitWork, ShipFitting,
    };
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTracks};
    use crate::ship::power::ShipPowerSystem;
    use bevy::ecs::system::RunSystemOnce;

    let mut authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let owner = SystemId("phaser-fore".into());
    // The shipped bank has a fixed arc. This refit fixture explicitly equips
    // a traversing mount while retaining the real bank's fitting and power owner.
    let bank = authored
        .ship_config
        .as_mut()
        .unwrap()
        .systems
        .iter_mut()
        .find(|system| system.id == owner)
        .unwrap();
    bank.components.extend([
        crate::ship::composition::SystemComponentConfig::PowerInput {
            id: "test-mount-power".into(),
        },
        crate::ship::composition::SystemComponentConfig::Turret {
            id: "test-mount".into(),
            power_input: "test-mount-power".into(),
            config: TurretConfig {
                facing_deg: 0.0,
                traverse_arc_deg: 180.0,
                traverse_deg_per_sec: 30.0,
                aim_tolerance_deg: 1.0,
            },
        },
    ]);
    let instrument = SystemId("tactical-radar".into());
    let broad = BroadProfile::from_config(&authored).unwrap().unwrap();
    let mut tracks = BroadTracks::new(&broad);
    let state = tracks.0.get_mut(&instrument).unwrap();
    state
        .advance(
            &broad.0[&instrument].profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                elevation: None,
                source: "held-target".into(),
                selectable: true,
                observation: None,
                position: [40.0, 10.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let measured = state
        .projection(&broad.0[&instrument].profile, 0, 0.5)
        .unwrap();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
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
            combat_lock: Some("held-target".into()),
            ..Default::default()
        }),
    );
    let mut world = World::new();
    world.insert_resource(Time::<Fixed>::from_seconds(0.1));
    world.insert_resource(crate::sim_tick::SimTick(0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &authored,
        Vec3::ZERO,
        "observer".into(),
        None,
    );
    world.flush();
    world.entity_mut(entity).insert((
        boards,
        tracks,
        crate::core::messages::AdmittedCommands::default(),
    ));
    world
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    let pose = *world.get::<Transform>(entity).unwrap();
    let initial_power = world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    world.run_system_once(tick).unwrap();
    let bearing = world.get::<ShipMounts>(entity).unwrap().0[&owner].offset_deg;
    assert!(
        bearing > 0.0,
        "positive control: an installed bank really traverses"
    );
    let paid = world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    assert_ne!(paid, initial_power);

    // Set up the physical equipment owner's normal stage frontiers. The mount
    // reader neither advances this job nor moves its actual assemblies.
    let profile = world.get::<CompiledShipFitting>(entity).unwrap().clone();
    let slot = profile
        .0
        .config()
        .slots
        .iter()
        .find(|slot| slot.system == owner)
        .unwrap();
    let mut fitting = world.get::<ShipFitting>(entity).unwrap().clone();
    let spare = fitting.equipment.assemblies.values().find(|item| {
        matches!(&item.location, crate::resources::fitting::EquipmentLocation::Stored { locker }
            if profile.0.locker(locker).unwrap().compartment == profile.0.compartment(&slot.id).unwrap())
            && profile.0.compatible(&slot.id, &item.definition).is_ok()
    }).unwrap().clone();
    let crate::resources::fitting::EquipmentLocation::Stored { locker } = spare.location else {
        unreachable!()
    };
    let job = world
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap()
        .0
        .claim_refit(0, 1, owner.clone(), slot.id.clone())
        .unwrap();
    fitting
        .equipment
        .stored_to_work(&profile.0, &spare.id, job.0, &slot.id)
        .unwrap();
    fitting.work.insert(
        slot.id.clone(),
        RefitWork {
            request: 1,
            slot: slot.id.clone(),
            item: spare.id.clone(),
            removed_item: fitting
                .equipment
                .installed(&slot.id)
                .map(|item| item.id.clone()),
            locker: locker.clone(),
            source_locker: locker.clone(),
            job,
            stage: RefitStage::Removing,
            progress: 0.0,
            reserved: false,
            waiting: String::new(),
        },
    );
    let check_frozen = |world: &mut World, now| {
        world.resource_mut::<crate::sim_tick::SimTick>().0 = now;
        world.run_system_once(tick).unwrap();
        assert_eq!(
            world.get::<ShipMounts>(entity).unwrap().0[&owner].offset_deg,
            bearing
        );
        assert_eq!(
            world
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .capture_continuation(),
            paid
        );
        assert_eq!(world.get::<Transform>(entity).unwrap(), &pose);
    };
    world.entity_mut(entity).insert(fitting.clone());
    check_frozen(&mut world, 1);
    let condition = world
        .get::<crate::ship::compartment_damage::ShipDamage>(entity)
        .unwrap()
        .state
        .components[&owner][&slot.operation_component]
        .clone();
    let old = fitting
        .equipment
        .remove_installed(&profile.0, &slot.id, job.0, condition)
        .unwrap();
    fitting
        .equipment
        .return_to_locker(&profile.0, &old, job.0, &locker)
        .unwrap();
    fitting.work.get_mut(&slot.id).unwrap().stage = RefitStage::Installing;
    world.entity_mut(entity).insert(fitting.clone());
    check_frozen(&mut world, 2);
    fitting.work.clear();
    world.entity_mut(entity).insert(fitting.clone());
    check_frozen(&mut world, 3);
    let condition = fitting
        .equipment
        .install_from_work(&profile.0, &slot.id, &spare.id, job.0)
        .unwrap();
    world
        .get_mut::<crate::ship::compartment_damage::ShipDamage>(entity)
        .unwrap()
        .state
        .components
        .get_mut(&owner)
        .unwrap()
        .insert(slot.operation_component.clone(), condition);
    world.entity_mut(entity).insert(fitting);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 4;
    world.run_system_once(tick).unwrap();
    assert!(world.get::<ShipMounts>(entity).unwrap().0[&owner].offset_deg > bearing);
    assert_ne!(
        world
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .capture_continuation(),
        paid
    );
    assert_eq!(world.get::<Transform>(entity).unwrap(), &pose);
}

#[test]
fn consumer_payment_matches_clone_based_mount_transaction() {
    for case in 0..7 {
        let (owner, profiles, mut states, mut power) = profile();
        let rate = power
            .0
            .config()
            .consumers
            .iter()
            .find(|consumer| consumer.system == owner)
            .unwrap()
            .continuous_cost_per_sec;
        match case {
            1 => {
                power
                    .0
                    .network_mut()
                    .consumers
                    .get_mut(&owner)
                    .unwrap()
                    .energy = rate * 0.1
            }
            2 => {
                power
                    .0
                    .network_mut()
                    .consumers
                    .get_mut(&owner)
                    .unwrap()
                    .energy = 0.0
            }
            3 => {
                power
                    .0
                    .network_mut()
                    .consumers
                    .get_mut(&owner)
                    .unwrap()
                    .thermal_stopped = true
            }
            4 => {
                power.0.network_mut().consumers.remove(&owner);
            }
            _ => {}
        }
        let mut reference_states = states.clone();
        let mut reference_power = power.clone();
        let desired = if case == 5 {
            Some(f32::NAN)
        } else {
            Some(90.0)
        };
        let factor = if case == 6 { 0.0 } else { 1.0 };
        for tick in [0, 0, 1] {
            let actual = advance_system(
                &profiles,
                &mut states,
                &owner,
                tick,
                1.0,
                true,
                desired,
                factor,
                &mut power,
            );
            let expected = payment_reference::advance_system_reference(
                &profiles,
                &mut reference_states,
                &owner,
                tick,
                1.0,
                true,
                desired,
                factor,
                &mut reference_power,
            );
            assert_eq!(actual, expected, "case {case}");
            assert_eq!(states, reference_states);
            assert_eq!(power.0, reference_power.0);
            assert_eq!(
                serde_json::to_string(&power.0.capture_continuation()).unwrap(),
                serde_json::to_string(&reference_power.0.capture_continuation()).unwrap()
            );
            assert_eq!(
                power.0.emitted_work(&owner).to_bits(),
                reference_power.0.emitted_work(&owner).to_bits()
            );
        }
    }
}
