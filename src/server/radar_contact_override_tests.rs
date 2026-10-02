use super::*;
use crate::gui::radar::{BlipLabel, BlipWorldPose, GenericRadarWidget, RadarAppearance};
use bevy::ecs::system::RunSystemOnce;
#[test]
fn sensors_viewscreen_bridge_preserves_normal_and_reconciles_conceal_and_basic_reveal() {
    let mut app = App::new();
    app.init_resource::<WorldResource>()
        .init_resource::<crate::world::server::WorldContentRuntime>();
    app.world_mut().spawn((
        crate::server_app::LocalShip,
        crate::entities::spawner::EntityUuid("observer".into()),
        ShipPhysics::default(),
        {
            let mut mode = crate::ship::state::ShipViewMode::default();
            mode.view_mode = ViewMode::SensorsRadar;
            mode
        },
    ));
    let widget = app
        .world_mut()
        .spawn((
            ConsoleRadar::ViewscreenScience,
            RadarBlipMap::default(),
            GenericRadarWidget {
                range: 100.0,
                orientation: OrientationMode::WorldFixed,
                filter: RadarFilter(
                    ["ship".into(), crate::gm_contact::BASIC_RADAR_TAG.into()]
                        .into_iter()
                        .collect(),
                ),
                clip_mode: RadarClipMode::Circle,
                face_fraction: 1.0,
            },
        ))
        .id();
    app.world_mut().resource_mut::<WorldResource>().0.entities =
        vec![crate::core::messages::EntitySnapshot {
            uuid: "target".into(),
            name: Some("protected name".into()),
            tags: vec!["ship".into()],
            position: Some([200.0, 0.0, 0.0]),
            radar_icon: Some("ship".into()),
            hull_fraction: Some(0.2),
            ..Default::default()
        }];
    for mode in [
        crate::gm_contact::ContactMode::Reveal,
        crate::gm_contact::ContactMode::Conceal,
        crate::gm_contact::ContactMode::Normal,
    ] {
        crate::gm_contact::set(
            &mut app
                .world_mut()
                .resource_mut::<crate::world::server::WorldContentRuntime>()
                .contact_overrides,
            "observer",
            "target",
            mode,
        );
        app.world_mut()
            .run_system_once(sync_server_radar_bridge)
            .unwrap();
        let map = app.world().get::<RadarBlipMap>(widget).unwrap();
        if mode == crate::gm_contact::ContactMode::Conceal {
            assert!(!map.blips.contains_key("target"));
            continue;
        }
        let source = map.blips["target"];
        let appearance = app.world().get::<RadarAppearance>(source).unwrap();
        let pose = app.world().get::<BlipWorldPose>(source).unwrap();
        if mode == crate::gm_contact::ContactMode::Reveal {
            assert_eq!(
                appearance.icon.as_deref(),
                Some(crate::gm_contact::BASIC_RADAR_ICON)
            );
            assert!(pose.x.abs() < 100.0);
            assert_eq!(
                app.world().get::<BlipLabel>(source).unwrap().0.as_deref(),
                Some("console.sensors.basic_contact")
            );
            assert!(crate::gui::radar::project_radar_entity(
                pose.x,
                pose.z,
                0.0,
                0.0,
                0.0,
                100.0,
                2.0,
                &OrientationMode::WorldFixed
            )
            .is_some());
        } else {
            assert_eq!(appearance.icon.as_deref(), Some("ship"));
            assert_eq!(pose.x, 200.0);
        }
    }
}
