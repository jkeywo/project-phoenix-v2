use super::*;
use crate::core::messages::{HelmBlackboard, SensorRadarBlackboard};
use phoenix_model::{
    console::{ConsoleFeed, ConsoleObservationKind},
    sensing_tracks::TrackStage,
};

pub(crate) fn config() -> ShipConfig {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .ship_config
    .unwrap();
    for console in &mut config.consoles {
        console.feeds.clear();
    }
    config
}

pub(crate) fn track(contact: &str) -> SensorTrackSnapshot {
    SensorTrackSnapshot {
        elevation: None,
        references: Vec::new(),
        samples: Vec::new(),
        contact_id: contact.into(),
        observation: None,
        selectable: true,
        observed_tick: 10,
        position: [11.0, 22.0],
        velocity: None,
        position_error: 7.0,
        stage: TrackStage::Return,
        identity: None,
        identity_tick: None,
        classification: None,
        classification_tick: None,
        assessment: None,
        assessment_tick: None,
        predicted: false,
    }
}

pub(crate) fn science_board(track: SensorTrackSnapshot) -> ShipSystemBlackboards {
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::sensor_radar_system_id(),
        SystemBlackboard::SensorRadar(SensorRadarBlackboard {
            tracks: Some(vec![track]),
            ..Default::default()
        }),
    );
    boards
}

pub(crate) fn grant_to_viewscreen(config: &mut ShipConfig) {
    let console = config
        .console_for_system(&crate::ship::system_registry::viewscreen_system_id())
        .unwrap()
        .id
        .clone();
    config
        .consoles
        .iter_mut()
        .find(|item| item.id == console)
        .unwrap()
        .feeds
        .push(ConsoleFeed {
            source: crate::ship::system_registry::sensor_radar_system_id(),
            kind: ConsoleObservationKind::SensorRadar,
        });
}

#[test]
fn active_overlay_uses_requesting_console_and_only_explicit_cross_feeds() {
    let mut config = config();
    let mut boards = science_board(track("science-observation"));
    let helm = crate::ship::system_registry::helm_radar_system_id();
    boards.0.insert(
        helm.clone(),
        SystemBlackboard::Helm(HelmBlackboard {
            movement: None,
            tracks: Some(vec![track("helm-observation")]),
            ..Default::default()
        }),
    );
    // Co-hosting is not an observation binding.
    let same_seat = config.consoles[0].home_station.clone();
    for console in &mut config.consoles {
        console.home_station = same_seat.clone();
    }
    let owner = crate::ship::system_registry::sensors_system_id();
    let mut view = ShipViewMode::default();
    view.request_view_mode_from(owner.clone(), ViewMode::SensorsRadar);
    let held = contacts(&config, &boards, &view);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].contact_id, "science-observation");
    let console = config.console_for_system(&owner).unwrap().id.clone();
    config
        .consoles
        .iter_mut()
        .find(|item| item.id == console)
        .unwrap()
        .feeds
        .push(ConsoleFeed {
            source: helm,
            kind: ConsoleObservationKind::Helm,
        });
    assert_eq!(contacts(&config, &boards, &view).len(), 2);
    view.request_view_mode_from(SystemId("missing-owner".into()), ViewMode::Radar);
    assert!(contacts(&config, &boards, &view).is_empty());
}

#[test]
fn camera_has_no_new_personal_map_and_reads_only_viewscreen_feeds() {
    let mut config = config();
    let boards = science_board(track("science-observation"));
    let view = ShipViewMode::default();
    assert!(contacts(&config, &boards, &view).is_empty());
    grant_to_viewscreen(&mut config);
    assert_eq!(
        contacts(&config, &boards, &view)[0].contact_id,
        "science-observation"
    );
}

#[test]
fn measured_radar_rows_cannot_disclose_live_actor_identity_size_heading_or_health() {
    let mut measured = track("observed-alias");
    measured.predicted = true;
    let style = ContactProxyConfig::default();
    let row = radar_rows(&[measured.clone()], &style).remove(0);
    assert_eq!(row.uuid, "observed-alias");
    assert_eq!(row.position, Some([11.0, 0.0, 22.0]));
    assert_eq!(row.radar_size, Some(style.minimum_radius * 2.0));
    assert_eq!(row.radius, Some(measured.position_error * 2.0));
    assert!(row.name.is_none() && row.id.is_none() && row.yaw.is_none());
    assert!(row.hull_fraction.is_none() && row.shield_fraction.is_none());
    measured.identity = Some(phoenix_model::sensing_tracks::TrackIdentity {
        name: "Recorded name".into(),
        faction: None,
        references: Vec::new(),
    });
    let row = radar_rows(&[measured], &style).remove(0);
    assert_eq!(row.name.as_deref(), Some("Recorded name"));
    assert!(row.yaw.is_none());
}

#[test]
fn chart_permission_does_not_copy_hidden_mobile_actors_or_live_status() {
    let rows = chart_rows(
        &[
            EntitySnapshot {
                uuid: "hidden-ship".into(),
                tags: vec!["ship".into()],
                ..Default::default()
            },
            EntitySnapshot {
                uuid: "known-moon".into(),
                tags: vec![crate::console_delivery::CHARTED_TAG.into()],
                position: Some([1.0, 0.0, 3.0]),
                hull_fraction: Some(0.2),
                target_description: Some("private machinery".into()),
                ..Default::default()
            },
        ],
        |_| true,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uuid, "known-moon");
    assert!(rows[0].hull_fraction.is_none() && rows[0].target_description.is_none());
}

#[test]
fn actual_radar_bridge_reconciles_measured_aliases_and_an_honestly_empty_picture() {
    use crate::gui::radar::{
        BlipWorldPose, ConsoleRadar, GenericRadarWidget, OrientationMode, RadarBlipMap,
        RadarClipMode, RadarFilter,
    };
    use bevy::{ecs::system::RunSystemOnce, prelude::*};
    let mut app = App::new();
    app.init_resource::<crate::lobby::WorldResource>();
    app.world_mut()
        .resource_mut::<crate::lobby::WorldResource>()
        .0
        .entities = vec![EntitySnapshot {
        uuid: "private-live-actor".into(),
        position: Some([900.0, 40.0, 800.0]),
        name: Some("Private actual name".into()),
        radar_icon: Some("ship".into()),
        tags: vec!["ship".into()],
        ..Default::default()
    }];
    let mut view = ShipViewMode::default();
    view.request_view_mode_from(
        crate::ship::system_registry::sensors_system_id(),
        ViewMode::SensorsRadar,
    );
    let observer = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            crate::entities::spawner::EntityUuid("observer".into()),
            crate::science::broad::SensingControls(Default::default()),
            crate::ship_plugin::ShipConfigComponent(config()),
            science_board(track("observed-alias")),
            crate::ship::state::ShipPhysics::default(),
            view,
        ))
        .id();
    let widget = app
        .world_mut()
        .spawn((
            ConsoleRadar::ViewscreenScience,
            RadarBlipMap::default(),
            GenericRadarWidget {
                range: 100.0,
                orientation: OrientationMode::WorldFixed,
                filter: RadarFilter([crate::gm_contact::BASIC_RADAR_TAG.into()].into()),
                clip_mode: RadarClipMode::Circle,
                face_fraction: 1.0,
            },
        ))
        .id();
    app.world_mut()
        .run_system_once(crate::server::radar::sync_server_radar_bridge)
        .unwrap();
    let map = app.world().get::<RadarBlipMap>(widget).unwrap();
    assert_eq!(map.blips.len(), 1);
    let pose = app
        .world()
        .get::<BlipWorldPose>(map.blips["observed-alias"])
        .unwrap();
    assert_eq!((pose.x, pose.z), (11.0, 22.0));
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(observer)
        .unwrap()
        .0
        .clear();
    app.world_mut()
        .run_system_once(crate::server::radar::sync_server_radar_bridge)
        .unwrap();
    assert!(app
        .world()
        .get::<RadarBlipMap>(widget)
        .unwrap()
        .blips
        .is_empty());

    // Loss of the accepted observing actor cannot reactivate live world data
    // or leave the previously shared measured picture mounted.
    let mut accepted = crate::lobby::server::ShipClientConfigResource::default();
    accepted.0.system_kinds.insert(
        "sensor-radar".into(),
        crate::ship::system_registry::SENSOR_RADAR_KIND.into(),
    );
    app.insert_resource(accepted);
    *app.world_mut()
        .get_mut::<ShipSystemBlackboards>(observer)
        .unwrap() = science_board(track("observed-alias"));
    app.world_mut()
        .run_system_once(crate::server::radar::sync_server_radar_bridge)
        .unwrap();
    assert_eq!(
        app.world().get::<RadarBlipMap>(widget).unwrap().blips.len(),
        1
    );
    app.world_mut().entity_mut(observer).despawn();
    app.world_mut()
        .run_system_once(crate::server::radar::sync_server_radar_bridge)
        .unwrap();
    let map = app.world().get::<RadarBlipMap>(widget).unwrap();
    assert!(map.blips.is_empty() && map.center.is_none());
}

#[test]
fn concealed_chart_permission_is_withheld_before_building_radar_rows() {
    let entity = EntitySnapshot {
        uuid: "charted-but-concealed".into(),
        tags: vec![crate::console_delivery::CHARTED_TAG.into()],
        ..Default::default()
    };
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm_contact::set(
        &mut runtime.contact_overrides,
        "observer",
        &entity.uuid,
        crate::gm_contact::ContactMode::Conceal,
    );
    let rows = chart_rows(&[entity], |source| {
        !concealed(Some(&runtime), "observer", source)
    });
    assert!(rows.is_empty());
}

#[test]
fn maximum_finite_observation_and_authored_sizes_keep_finite_radar_geometry() {
    let mut measured = track("large-error");
    measured.position_error = f32::MAX;
    let style = ContactProxyConfig {
        minimum_radius: f32::MAX,
        ..Default::default()
    };
    let row = radar_rows(&[measured], &style).remove(0);
    assert_eq!(row.radar_size, Some(f32::MAX));
    assert_eq!(row.radius, Some(f32::MAX));
}
