use super::*;
use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState};
use phoenix_model::messages::SensorRadarBlackboard;
use std::collections::BTreeMap;

fn rig() -> (
    ShipConfig,
    ShipSystemBlackboards,
    BroadTracks,
    SystemId,
    String,
) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = authored.ship_config.clone().unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let owner = crate::ship::system_registry::sensor_radar_system_id();
    let profile = &profiles.0[&owner].profile;
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "private-physical-identity".into(),
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
        owner.clone(),
        SystemBlackboard::SensorRadar(SensorRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    (
        config,
        boards,
        BroadTracks(BTreeMap::from([(owner.clone(), state)])),
        owner,
        contact,
    )
}

#[test]
fn only_current_permitted_observations_can_route_to_physical_identity() {
    let (config, boards, tracks, owner, contact) = rig();
    assert_eq!(
        resolve(&config, &boards, Some(&tracks), &owner, &contact).as_deref(),
        Some("private-physical-identity")
    );
    assert!(resolve(
        &config,
        &boards,
        Some(&tracks),
        &owner,
        "private-physical-identity"
    )
    .is_none());
    assert!(resolve(
        &config,
        &boards,
        Some(&tracks),
        &SystemId("unowned".into()),
        &contact
    )
    .is_none());
    assert_eq!(
        alias_for_source(
            &config,
            &boards,
            Some(&tracks),
            &owner,
            "private-physical-identity"
        ),
        Some(contact)
    );
}

#[test]
fn selected_anonymous_return_does_not_supply_a_machinery_target_name() {
    let (config, mut boards, tracks, owner, _) = rig();
    assert!(identity_for_source(
        &config,
        &boards,
        &tracks,
        &owner,
        "private-physical-identity"
    )
    .is_none());
    let Some(SystemBlackboard::SensorRadar(board)) = boards.0.get_mut(&owner) else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].identity =
        Some(phoenix_model::sensing_tracks::TrackIdentity {
            name: "Observed identity".into(),
            faction: None,
            references: Vec::new(),
        });
    assert_eq!(
        identity_for_source(
            &config,
            &boards,
            &tracks,
            &owner,
            "private-physical-identity"
        )
        .as_deref(),
        Some("Observed identity")
    );
}

#[test]
fn public_selection_permission_and_private_current_association_are_both_required() {
    let (config, mut boards, mut tracks, owner, contact) = rig();
    let Some(SystemBlackboard::SensorRadar(board)) = boards.0.get_mut(&owner) else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].selectable = false;
    assert!(resolve(&config, &boards, Some(&tracks), &owner, &contact).is_none());
    let Some(SystemBlackboard::SensorRadar(board)) = boards.0.get_mut(&owner) else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].selectable = true;
    board.tracks.as_mut().unwrap()[0].predicted = true;
    assert!(resolve(&config, &boards, Some(&tracks), &owner, &contact).is_none());
    let Some(SystemBlackboard::SensorRadar(board)) = boards.0.get_mut(&owner) else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].predicted = false;
    tracks.0.get_mut(&owner).unwrap().current.clear();
    assert!(resolve(&config, &boards, Some(&tracks), &owner, &contact).is_none());
}

#[test]
fn sharing_a_host_station_does_not_grant_an_unbound_console_contact() {
    let (mut config, boards, tracks, owner, contact) = rig();
    let console = config.console_for_system(&owner).unwrap().id.clone();
    let other = config
        .consoles
        .iter()
        .find(|c| c.id != console)
        .unwrap()
        .id
        .clone();
    let consumer = config
        .systems
        .iter_mut()
        .find(|system| system.console.as_ref() == Some(&other))
        .unwrap();
    let consumer = consumer.id.clone();
    config
        .consoles
        .iter_mut()
        .find(|c| c.id == other)
        .unwrap()
        .feeds
        .clear();
    assert!(resolve(&config, &boards, Some(&tracks), &consumer, &contact).is_none());
    assert!(resolve(
        &config,
        &boards,
        Some(&BroadTracks(BTreeMap::new())),
        &owner,
        "private-physical-identity"
    )
    .is_none());
}
