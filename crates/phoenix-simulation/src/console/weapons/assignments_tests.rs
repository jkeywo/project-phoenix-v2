use super::*;
use crate::core::messages::{
    AdmittedCommand, AdmittedCommands, SystemControlPayload as Payload, TacticalRadarBlackboard,
};
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState};
use std::collections::BTreeMap;

fn fixture() -> (App, Entity, String) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
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
    assert_eq!(read(app.world_mut(), [100.0, 0.0]), None);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1_000;
    assert_eq!(read(app.world_mut(), [0.0, 0.0]), None);
}
