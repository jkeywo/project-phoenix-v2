use super::*;
use crate::core::messages::{
    AdmittedCommand, AdmittedCommands, SystemControlPayload as Payload, TacticalRadarBlackboard,
};
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState};
use std::collections::BTreeMap;

fn fixture() -> (App, Entity, String) {
    fixture_from("assets/entities/alliance_cruiser.toml")
}

fn missile_reading(app: &mut App, ship: Entity, height: Option<f32>, velocity: [f32; 2]) {
    let mut boards = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(ship)
        .unwrap();
    let SystemBlackboard::TacticalRadar(board) = boards
        .0
        .get_mut(&SystemId("tactical-radar".into()))
        .unwrap()
    else {
        unreachable!()
    };
    let reading = &mut board.tracks.as_mut().unwrap()[0];
    reading.classification = Some("torpedo".into());
    reading.velocity = Some(velocity);
    reading.elevation = height.map(|height| phoenix_model::sensing_tracks::ObservedElevation {
        height,
        error: 0.5,
        velocity: Some(0.0),
    });
}

#[test]
fn defensive_aim_requires_permitted_height_uses_full_range_and_obeys_explicit_hold() {
    use bevy::ecs::system::SystemState;
    let (mut app, ship, contact) = fixture_from("assets/entities/broadside_cruiser.toml");
    let owner = SystemId("blaster-side".into());
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    missile_reading(&mut app, ship, Some(100.0), [-1.0, -1.0]);
    let mount = crate::weapons::mount::MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 360.0,
        range: 50.0,
    };
    let read = |world: &mut World| {
        let mut state = SystemState::<WeaponAimInputs>::new(world);
        let inputs = state.get(world);
        let aim = inputs.resolve_allowed(
            ship,
            &owner,
            Some(&[phoenix_model::tags::EntityTag::Missile]),
            |_| panic!("defensive aim read hidden position"),
        )?;
        let geometry = inputs.geometry(ship, &owner, &mount, &aim, &Default::default());
        Some((aim.point_3d(0.0), geometry.in_range, geometry.range))
    };
    let (point, in_range, range) = read(app.world_mut()).unwrap();
    assert_eq!(point.unwrap()[1], 100.0);
    assert!(!in_range && range > 100.0);
    missile_reading(&mut app, ship, None, [-1.0, -1.0]);
    assert!(read(app.world_mut()).is_none());
    missile_reading(&mut app, ship, Some(10.0), [-1.0, -1.0]);
    assert!(read(app.world_mut()).unwrap().1);
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget { uuid: None },
    );
    assert!(read(app.world_mut()).is_none());
}

#[test]
fn defensive_choice_uses_own_relative_motion_and_rejects_lost_height_or_concealed_contacts() {
    use bevy::ecs::system::SystemState;
    let (mut app, ship, contact) = fixture_from("assets/entities/broadside_cruiser.toml");
    let owner = SystemId("blaster-side".into());
    let mount = crate::weapons::mount::MountConfig {
        facing_deg: 0.0,
        fire_arc_deg: 360.0,
        range: 50.0,
    };
    missile_reading(&mut app, ship, Some(0.0), [0.0, 0.0]);
    let read = |world: &mut World, physics: &crate::ship::state::ShipPhysics| {
        let mut state = SystemState::<WeaponAimInputs>::new(world);
        state
            .get(world)
            .defensive_contact(ship, &owner, &mount, physics)
    };
    assert!(read(app.world_mut(), &Default::default()).is_none());
    let mut own = crate::ship::state::ShipPhysics::default();
    own.lateral_speed = 2.0;
    own.forward_speed = -3.0;
    assert_eq!(read(app.world_mut(), &own), Some(contact.clone()));
    own.lateral_speed = -2.0;
    own.forward_speed = 3.0;
    assert!(read(app.world_mut(), &own).is_none());
    missile_reading(&mut app, ship, None, [-1.0, -1.0]);
    assert!(read(app.world_mut(), &Default::default()).is_none());
    missile_reading(&mut app, ship, Some(0.0), [-1.0, -1.0]);
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    assert!(read(app.world_mut(), &Default::default()).is_none());
}
fn fixture_from(path: &str) -> (App, Entity, String) {
    let authored = crate::entities::include_resolve::load_entity_config(path).unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let config = authored.ship_config.unwrap();
    let sensing = authored.sensing.unwrap();
    let profile = sensing
        .broad
        .iter()
        .find(|profile| profile.system.0 == "tactical-radar")
        .unwrap();
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "private-actor".into(),
                selectable: true,
                observation: None,
                position: [20.0, 30.0],
                elevation: None,
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let public = state.projection(profile, 0, 0.5).unwrap();
    let contact = public[0].contact_id.clone();
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        SystemId("tactical-radar".into()),
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    let mut app = App::new();
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    app.insert_resource(crate::sim_tick::SimTick(0));
    app.insert_resource(Time::<Fixed>::default());
    app.add_systems(Update, handle_assignments);
    let ship = app
        .world_mut()
        .spawn((
            crate::ship_plugin::ShipConfigComponent(config),
            crate::entities::spawner::WeaponsConsoleSection(authored.weapons_console.unwrap()),
            crate::entities::spawner::EntityUuid("observer".into()),
            boards,
            BroadTracks(BTreeMap::from([(SystemId("tactical-radar".into()), state)])),
            ShipWeaponTargets::default(),
            AdmittedCommands::default(),
            profiles,
        ))
        .id();
    (app, ship, contact)
}
fn command(app: &mut App, ship: Entity, owner: &str, payload: Payload) {
    app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap().0 = vec![AdmittedCommand {
        target: SystemId(owner.into()),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
    app.update();
}
#[test]
fn actual_member_commands_preserve_override_and_do_not_command_other_members() {
    let (mut app, ship, contact) = fixture();
    let fore = SystemId("phaser-fore".into());
    let aft = SystemId("phaser-aft".into());
    command(
        &mut app,
        ship,
        &fore.0,
        Payload::SetWeaponTarget { uuid: None },
    );
    command(
        &mut app,
        ship,
        &fore.0,
        Payload::SetWeaponGroupTarget {
            group: "main-beams".into(),
            uuid: Some(contact.clone()),
        },
    );
    let saved = &app.world().get::<ShipWeaponTargets>(ship).unwrap().0;
    assert_eq!(saved.target_for(&fore, Some("primary")), None);
    assert!(!saved.0.contains_key(&aft));
    command(&mut app, ship, &fore.0, Payload::FollowWeaponGroup);
    assert_eq!(
        app.world()
            .get::<ShipWeaponTargets>(ship)
            .unwrap()
            .0
            .target_for(&fore, None),
        Some(contact.as_str())
    );
    command(&mut app, ship, &fore.0, Payload::FollowPrimaryTarget);
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
}
#[test]
fn guessed_private_contacts_nonmembers_and_current_conceal_cannot_mutate_intent() {
    let (mut app, ship, contact) = fixture();
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetWeaponTarget {
            uuid: Some("private-actor".into()),
        },
    );
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetWeaponGroupTarget {
            group: "missiles".into(),
            uuid: Some(contact.clone()),
        },
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetWeaponTarget {
            uuid: Some(contact),
        },
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetWeaponTarget { uuid: None },
    );
    assert_eq!(
        app.world()
            .get::<ShipWeaponTargets>(ship)
            .unwrap()
            .0
             .0
            .len(),
        1
    );
}
#[test]
fn losing_a_selected_publication_withholds_aim_without_forgetting_own_assignment() {
    let (mut app, ship, contact) = fixture();
    let owner = SystemId("phaser-fore".into());
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    let saved = app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
        .clone();
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .clear();
    let world = app.world();
    assert!(effective_weapon_contact(
        &world
            .get::<crate::ship_plugin::ShipConfigComponent>(ship)
            .unwrap()
            .0,
        &owner,
        world.get::<ShipSystemBlackboards>(ship).unwrap(),
        world.get::<BroadTracks>(ship),
        None,
        world.get::<ShipWeaponTargets>(ship)
    )
    .is_none());
    assert_eq!(world.get::<ShipWeaponTargets>(ship).unwrap().0, saved);
}

#[test]
fn active_cycle_keeps_its_known_route_after_selection_changes_but_not_after_feed_loss() {
    use bevy::ecs::system::SystemState;
    let (mut app, ship, contact) = fixture();
    let owner = SystemId("phaser-fore".into());
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget {
            uuid: Some(contact),
        },
    );
    // Changing the next-shot order does not revoke a running cycle's target.
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget { uuid: None },
    );
    let read = |world: &mut World, route: &str| {
        let mut state = SystemState::<WeaponAimInputs>::new(world);
        state
            .get(world)
            .active(ship, &owner, route, |_| {
                panic!("active measured aim read private geometry")
            })
            .map(|aim| (aim.route, aim.position, aim.measured))
    };
    assert_eq!(
        read(app.world_mut(), "private-actor"),
        Some(("private-actor".into(), [20.0, 30.0], true))
    );
    assert!(read(app.world_mut(), "never-observed-actor").is_none());
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .clear();
    assert!(read(app.world_mut(), "private-actor").is_none());
}

#[test]
fn fitted_observations_without_their_console_scope_never_fall_back_to_scene_truth() {
    use bevy::ecs::system::SystemState;
    let (mut app, ship, contact) = fixture();
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship_plugin::ShipConfigComponent>();
    let mut state = SystemState::<WeaponAimInputs>::new(app.world_mut());
    let inputs = state.get(app.world());
    let owner = SystemId("phaser-fore".into());
    assert!(inputs
        .resolve_reference(ship, &owner, &contact, |_| panic!(
            "missing scope read private geometry"
        ))
        .is_none());
    assert!(inputs
        .active(ship, &owner, "private-actor", |_| panic!(
            "active cycle read private geometry"
        ))
        .is_none());
}

#[test]
fn acquired_ray_cover_is_not_live_shield_truth_and_requires_fresh_compatible_evidence() {
    use bevy::ecs::system::SystemState;
    let (mut app, ship, contact) = fixture();
    let owner = SystemId("phaser-fore".into());
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget {
            uuid: Some(contact),
        },
    );
    let read = |world: &mut World, own: [f32; 2]| {
        let mut state = SystemState::<WeaponAimInputs>::new(world);
        let inputs = state.get(world);
        let aim = inputs
            .resolve(ship, &owner, |_| {
                panic!("fitted aim read private physical geometry")
            })
            .unwrap();
        inputs.observed_shield_cover(ship, &owner, &aim, own)
    };
    assert_eq!(read(app.world_mut(), [0.0, 0.0]), None);
    {
        let mut boards = app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(ship)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        let record = &mut board.tracks.as_mut().unwrap()[0];
        record.assessment_tick = Some(0);
        record.assessment = Some(phoenix_model::sensing_tracks::TrackAssessment {
            protection: Some(phoenix_model::sensing_tracks::ObservedShieldCover {
                incoming_sector: phoenix_sim_gameplay::science::tracks::shield_observation_sector(
                    [0.0, 0.0],
                    record.position,
                ),
                blocked: false,
            }),
            alert: None,
            disabled: None,
            weapons_restrained: None,
        });
    }
    assert_eq!(read(app.world_mut(), [0.0, 0.0]), Some(0));
    {
        let mut boards = app
            .world_mut()
            .get_mut::<ShipSystemBlackboards>(ship)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        let record = &mut board.tracks.as_mut().unwrap()[0];
        // One actual sampled ray, with the profile's ordinary two-unit error.
        record.position = [1.4142135, 46.414215];
        record.position_error = 2.0;
        record
            .assessment
            .as_mut()
            .unwrap()
            .protection
            .as_mut()
            .unwrap()
            .incoming_sector = 0;
    }
    assert_eq!(read(app.world_mut(), [0.0, 0.0]), Some(0));
    assert_eq!(read(app.world_mut(), [100.0, 0.0]), None);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1_000;
    assert_eq!(read(app.world_mut(), [0.0, 0.0]), None);
}

#[test]
fn automatic_command_uses_the_same_current_permitted_contact_boundary_and_clear_is_not_hold() {
    let (mut app, ship, contact) = fixture_from("assets/entities/broadside_cruiser.toml");
    let owner = SystemId("blaster-side".into());
    for supplied in ["private-actor", "unobserved-public-handle"] {
        command(
            &mut app,
            ship,
            &owner.0,
            Payload::SetAutomaticWeaponTarget {
                uuid: Some(supplied.into()),
            },
        );
        assert!(app
            .world()
            .get::<ShipWeaponTargets>(ship)
            .unwrap()
            .0
             .0
            .is_empty());
    }
    // Unsupported ownership is tested while this same handle is observable.
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    assert_eq!(
        app.world().get::<ShipWeaponTargets>(ship).unwrap().0 .0[&owner]
            .automatic
            .as_deref(),
        Some(contact.as_str())
    );
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetWeaponTarget { uuid: None },
    );
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    assert_eq!(
        app.world()
            .get::<ShipWeaponTargets>(ship)
            .unwrap()
            .0
            .target_for(&owner, Some("eligible-primary")),
        None
    );
    let before = app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
        .clone();
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    assert_eq!(
        app.world().get::<ShipWeaponTargets>(ship).unwrap().0,
        before
    );
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget { uuid: None },
    );
    let kept = &app.world().get::<ShipWeaponTargets>(ship).unwrap().0 .0[&owner];
    assert!(kept.automatic.is_none() && kept.has_explicit_order());
    command(&mut app, ship, &owner.0, Payload::FollowPrimaryTarget);
    command(
        &mut app,
        ship,
        "phaser-fore",
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact.clone()),
        },
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "private-actor",
        crate::gm::operations::contact::ContactMode::Reveal,
    );
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .clear();
    command(
        &mut app,
        ship,
        &owner.0,
        Payload::SetAutomaticWeaponTarget {
            uuid: Some(contact),
        },
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(ship)
        .unwrap()
        .0
         .0
        .is_empty());
}
