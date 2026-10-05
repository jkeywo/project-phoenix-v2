use super::*;
use crate::server_app::LocalShip;
use crate::ship::state::ShipViewMode;
use bevy::ecs::system::RunSystemOnce;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_systems(Update, toggle_viewscreen_radar_widgets);
    app.world_mut().spawn((LocalShip, ShipViewMode::default()));
    app.world_mut()
        .spawn((RadarContainerMode::Helm, Visibility::Hidden));
    app
}

fn helm_container_visibility(app: &mut App) -> Visibility {
    let mut q = app
        .world_mut()
        .query::<(&RadarContainerMode, &Visibility)>();
    q.iter(app.world())
        .find(|(m, _)| **m == RadarContainerMode::Helm)
        .map(|(_, v)| *v)
        .unwrap()
}

/// Regression test: a SetView request arriving mid-game (e.g. Helm's ON
/// SCREEN button) must flip the matching radar container's visibility even
/// though the `GamePhase` state itself hasn't changed since the single
/// Lobby→InProgress transition. Before this fix, `toggle_viewscreen_radar_widgets`
/// only re-ran on `state.is_changed()`, so the viewscreen never actually
/// switched to radar no matter how many times a console requested it.
#[test]
fn radar_container_becomes_visible_on_mid_game_view_mode_change() {
    let mut app = test_app();

    // Lobby → InProgress transition, still on the default Camera(Fore) view.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    assert_eq!(helm_container_visibility(&mut app), Visibility::Hidden);

    // A later, unrelated frame with no state change and no view-mode change
    // must leave the container exactly as it was.
    app.update();
    assert_eq!(helm_container_visibility(&mut app), Visibility::Hidden);

    // Mid-game SetView request (what handle_set_view does when Helm clicks
    // ON SCREEN): mutate ShipViewMode directly, GamePhase does NOT change.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipViewMode, With<LocalShip>>();
        q.single_mut(app.world_mut()).unwrap().view_mode = ViewMode::Radar;
    }
    app.update();

    assert_eq!(helm_container_visibility(&mut app), Visibility::Visible);
}

#[test]
fn backdrop_shape_matches_viewscreen_mode_family() {
    assert_eq!(
        backdrop_for_mode(RadarContainerMode::Helm),
        RadarBackdropMode::CircularWidget
    );
    assert_eq!(
        backdrop_for_mode(RadarContainerMode::Science),
        RadarBackdropMode::CircularWidget
    );
    assert_eq!(
        backdrop_for_mode(RadarContainerMode::SystemChart),
        RadarBackdropMode::FullScreen
    );
    assert_eq!(
        backdrop_for_mode(RadarContainerMode::Nav),
        RadarBackdropMode::FullScreen
    );
}

#[test]
fn viewscreen_radar_uses_the_selected_hull_config_path() {
    let selected =
        crate::lobby::SelectedShipResource("assets/entities/alliance_battleship.toml".into());
    assert_eq!(
        viewscreen_ship_config_path(Some(&selected)),
        "assets/entities/alliance_battleship.toml"
    );
    assert_eq!(
        viewscreen_ship_config_path(None),
        "assets/entities/alliance_cruiser.toml"
    );
}

#[test]
fn viewscreen_objective_markers_follow_recipient_scope_for_seeded_and_spawned_metadata() {
    use crate::entities::spawner::{
        EntityId, EntityTagsSection, EntityUuid, RadarAppearanceSection,
    };
    use crate::server_app::{
        LastBroadcastEntityHealth, LastBroadcastEntityPositions, SimOutbox, TrackedEntities,
    };
    let mut app = App::new();
    app.init_resource::<WorldResource>()
        .init_resource::<TrackedEntities>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<SimOutbox>()
        .init_resource::<crate::world::server::ObjectiveManagerRes>();
    let mut view_mode = ShipViewMode::default();
    view_mode.view_mode = ViewMode::NavigationChart;
    let viewer = app
        .world_mut()
        .spawn((LocalShip, EntityUuid("ship-b".into()), view_mode))
        .id();
    app.world_mut().spawn((
        ConsoleRadar::ViewscreenNav,
        RadarBlipMap::default(),
        crate::gui::radar::GenericRadarWidget {
            range: 100.0,
            orientation: OrientationMode::WorldFixed,
            filter: RadarFilter(["objective_marker".into()].into_iter().collect()),
            clip_mode: RadarClipMode::None,
            face_fraction: 1.0,
        },
    ));
    {
        let manager = &mut app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>()
            .0;
        manager.add(
            "private",
            "objective.test",
            true,
            vec!["seeded".into(), "spawned".into()],
        );
        manager.set_recipients("private", vec!["ship-a".into()]);
    }
    for id in ["seeded", "spawned"] {
        app.world_mut().spawn((
            EntityUuid(format!("uuid-{id}")),
            EntityId(id.into()),
            EntityTagsSection(vec!["objective_marker".into()]),
            Transform::from_xyz(20.0, 0.0, 10.0),
            RadarAppearanceSection(crate::entities::config::RadarAppearanceConfig {
                icon: Some("waypoint".into()),
                region_colour: Some(vec![0.1, 0.2, 0.3]),
                colour: None,
                size: None,
            }),
        ));
        app.world_mut()
            .run_system_once(crate::server_app::reconcile_runtime_entities)
            .unwrap();
        let metadata = app.world().resource::<WorldResource>().0.clone();
        assert!(metadata
            .entities
            .iter()
            .all(|entity| entity.objective_target));
        // Ship B receives those exact seeded / EntitySpawned / reconnect
        // metadata rows, but its view must not inherit ship A's annotation.
        app.world_mut()
            .run_system_once(sync_server_radar_bridge)
            .unwrap();
        let mut query = app.world_mut().query::<(
            &crate::gui::radar::RadarEntityUuid,
            &crate::gui::radar::RadarAppearance,
        )>();
        assert!(query
            .iter(app.world())
            .all(|(_, appearance)| !appearance.objective_target));
        assert_eq!(app.world().resource::<WorldResource>().0, metadata);
    }
    app.world_mut().get_mut::<EntityUuid>(viewer).unwrap().0 = "ship-a".into();
    app.world_mut()
        .run_system_once(sync_server_radar_bridge)
        .unwrap();
    let mut query = app.world_mut().query::<(
        &crate::gui::radar::RadarEntityUuid,
        &crate::gui::radar::RadarAppearance,
    )>();
    assert_eq!(
        query
            .iter(app.world())
            .filter(|(_, appearance)| appearance.objective_target)
            .count(),
        2
    );
    app.world_mut()
        .resource_mut::<crate::world::server::ObjectiveManagerRes>()
        .0
        .complete("private");
    app.world_mut()
        .run_system_once(sync_server_radar_bridge)
        .unwrap();
    assert!(query
        .iter(app.world())
        .all(|(_, appearance)| !appearance.objective_target));
    assert!(
        app.world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .all(|entity| entity.objective_target),
        "presentation must not overwrite even stale shared metadata"
    );
}
