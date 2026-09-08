//! Prelaunch escape when the saved GM screen is the only monitor available.
//! The explicit request is a host layout action, never a simulation GM verb.

use super::{NativeGmLifecycle, NativeGmSurface};
use crate::core::messages::GamePhase;
use crate::gm_action::NativeGmAuthority;
use crate::native_host::bridge_display::BridgeLayoutResource;
use crate::native_host::host_lobby::{HostLobbyBridgeResource, HostLobbyRecord};
use bevy::prelude::*;

pub fn host_lobby_unavailable(
    phase: &GamePhase,
    enabled: bool,
    layout: Option<&BridgeLayoutResource>,
) -> bool {
    *phase == GamePhase::Lobby
        && enabled
        && layout.is_some_and(|layout| {
            layout.layout.game_master_monitor().is_some()
                && !layout
                    .monitors
                    .iter()
                    .any(|monitor| &monitor.identity == layout.layout.viewscreen())
        })
}

/// Recheck the actual host state before returning the original layout action to
/// its own bridge. The normal host-lobby reader rechecks role mutability too.
pub(crate) fn request(world: &World) -> bool {
    let Some(phase) = world.get_resource::<State<GamePhase>>() else {
        return false;
    };
    let enabled = world
        .get_resource::<NativeGmLifecycle>()
        .is_some_and(|gm| gm.enabled);
    if !host_lobby_unavailable(
        phase.get(),
        enabled,
        world.get_resource::<BridgeLayoutResource>(),
    ) || !world
        .get_resource::<NativeGmAuthority>()
        .is_some_and(|gm| gm.connected)
        || !world
            .get_resource::<NativeGmSurface>()
            .is_some_and(|gm| gm.bridge.live() && !gm.bridge.failed())
    {
        return false;
    }
    world
        .get_resource::<HostLobbyBridgeResource>()
        .is_some_and(|bridge| {
            bridge
                .0
                .submit_record(&HostLobbyRecord::SetGameMaster { monitor: None })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::bridge_layout::{BridgeLayout, LayoutAction};
    use crate::native_host::bridge_profile::{identify, RawMonitor};
    use crate::native_host::host_lobby::HostLobbyBridge;
    use crate::native_host::panes::PaneId;

    fn fixture() -> World {
        let monitors = identify(&[
            RawMonitor {
                name: Some("Viewscreen".into()),
                physical_width: 1920,
                physical_height: 1080,
                position_x: 0,
                position_y: 0,
                scale_factor: 1.0,
                primary: true,
            },
            RawMonitor {
                name: Some("GM".into()),
                physical_width: 1920,
                physical_height: 1080,
                position_x: 1920,
                position_y: 0,
                scale_factor: 1.0,
                primary: false,
            },
        ]);
        let layout = BridgeLayout::new(
            monitors.iter().map(|m| m.identity.clone()),
            Vec::new(),
            &monitors[0].identity,
        )
        .unwrap()
        .apply(&LayoutAction::SetGameMaster {
            monitor: Some(monitors[1].identity.clone()),
        })
        .unwrap();
        let remaining = vec![monitors[1].clone()];
        let layout = layout.reconcile(&remaining, Vec::new()).0;
        let mut world = World::new();
        world.insert_resource(State::new(GamePhase::Lobby));
        world.insert_resource(BridgeLayoutResource {
            layout,
            monitors: remaining,
            notices: Vec::new(),
        });
        world.insert_resource(NativeGmLifecycle {
            enabled: true,
            ..Default::default()
        });
        world.insert_resource(NativeGmAuthority {
            connected: true,
            screen_pause: false,
        });
        let surface = NativeGmSurface {
            bridge: Default::default(),
            url: "/gm".into(),
        };
        surface.bridge.activate(PaneId(1));
        surface.bridge.mark_live();
        world.insert_resource(surface);
        world.insert_resource(HostLobbyBridgeResource(HostLobbyBridge::new()));
        world
    }

    #[test]
    fn prelaunch_recovery_queues_the_existing_off_action_without_editing_layout() {
        let world = fixture();
        let before = world.resource::<BridgeLayoutResource>().layout.clone();
        assert!(request(&world));
        assert_eq!(world.resource::<BridgeLayoutResource>().layout, before);
        let records = world.resource::<HostLobbyBridgeResource>().0.take_records();
        assert_eq!(records.len(), 1);
        assert_eq!(
            crate::core::codec::decode_host_lobby_record(&records[0]).unwrap(),
            HostLobbyRecord::SetGameMaster { monitor: None }
        );
        let next = before
            .apply(&LayoutAction::SetGameMaster { monitor: None })
            .unwrap();
        assert!(next.game_master_monitor().is_none());
        assert_eq!(
            next.viewscreen(),
            &world.resource::<BridgeLayoutResource>().monitors[0].identity
        );
    }

    #[test]
    fn host_rechecks_phase_surface_role_and_actual_viewscreen_availability() {
        for phase in [GamePhase::Loading, GamePhase::InProgress] {
            let mut world = fixture();
            world.insert_resource(State::new(phase));
            assert!(!request(&world));
            assert!(world
                .resource::<HostLobbyBridgeResource>()
                .0
                .take_records()
                .is_empty());
        }
        let mut world = fixture();
        world.resource_mut::<NativeGmLifecycle>().enabled = false;
        assert!(!request(&world));
        world.resource_mut::<NativeGmLifecycle>().enabled = true;
        world.resource::<NativeGmSurface>().bridge.fault();
        assert!(!request(&world));
        let mut world = fixture();
        let mut layout = world.resource_mut::<BridgeLayoutResource>();
        let mut monitor = layout.monitors[0].clone();
        monitor.identity = layout.layout.viewscreen().clone();
        layout.monitors.push(monitor);
        assert!(!request(&world));
        assert!(world
            .resource::<HostLobbyBridgeResource>()
            .0
            .take_records()
            .is_empty());
    }
}
