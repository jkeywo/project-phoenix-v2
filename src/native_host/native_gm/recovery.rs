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
#[path = "recovery_tests.rs"]
mod tests;
