//! The active screen reads one Console's observations, never the scene registry.
use crate::{
    core::messages::{EntitySnapshot, SystemBlackboard, SystemId, ViewMode},
    server_app::ShipSystemBlackboards,
    ship::{config::ShipConfig, state::ShipViewMode},
    world::contact_render_config::ContactProxyConfig,
};
use phoenix_model::sensing_tracks::SensorTrackSnapshot;
use std::collections::BTreeMap;

/// The accepted topology survives actor destruction; losing the live Console
/// must never reactivate the legacy unrestricted presentation path.
pub(crate) fn restricted(
    accepted: Option<&crate::lobby::server::ShipClientConfigResource>,
    live_sensing: bool,
) -> bool {
    live_sensing
        || accepted.is_some_and(|accepted| {
            accepted.0.system_kinds.values().any(|kind| {
                use crate::ship::system_registry::*;
                matches!(
                    kind.as_str(),
                    HELM_RADAR_KIND
                        | TACTICAL_RADAR_KIND
                        | SENSOR_RADAR_KIND
                        | SENSORS_KIND
                        | PASSIVE_RECEIVER_KIND
                )
            })
        })
}

pub(crate) fn concealed(
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    observer: &str,
    source: &str,
) -> bool {
    runtime
        .and_then(|runtime| runtime.contact_overrides.get(observer))
        .and_then(|rows| rows.get(source))
        == Some(&crate::gm_contact::ContactMode::Conceal)
}

fn diameter(radius: f32) -> f32 {
    (f64::from(radius) * 2.0).min(f64::from(f32::MAX)) as f32
}

/// Camera modes have no personal radar: their scope is the existing Viewscreen
/// Console. An overlay explicitly shares its requesting System's Console.
fn active_owner(config: &ShipConfig, view: &ShipViewMode) -> Option<SystemId> {
    let owner = match view.view_mode {
        ViewMode::Camera(_) | ViewMode::Cinematic | ViewMode::Comms => {
            crate::ship::system_registry::viewscreen_system_id()
        }
        _ => {
            let resolution = view.viewscreen.resolved();
            if resolution.mode != view.view_mode {
                return None;
            }
            resolution.owner
        }
    };
    config.console_for_system(&owner).map(|_| owner)
}

pub(crate) fn contacts(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    view: &ShipViewMode,
) -> Vec<SensorTrackSnapshot> {
    let Some(owner) = active_owner(config, view) else {
        return Vec::new();
    };
    let permitted = config.observations_for_system(&owner, &boards.0);
    let mut contacts = BTreeMap::new();
    for board in permitted.values() {
        let tracks = match board {
            SystemBlackboard::Helm(board) => board.tracks.as_deref(),
            SystemBlackboard::TacticalRadar(board) => board.tracks.as_deref(),
            SystemBlackboard::SensorRadar(board) => board.tracks.as_deref(),
            _ => None,
        };
        for track in tracks.into_iter().flatten() {
            // A corrupted presentation input must not inject invalid geometry.
            // Live authoritative acquisition is validated by the sensing owner.
            if track.position.iter().all(|value| value.is_finite())
                && track.position_error.is_finite()
                && track.position_error >= 0.0
            {
                contacts
                    .entry(track.contact_id.clone())
                    .or_insert_with(|| track.clone());
            }
        }
    }
    contacts.into_values().collect()
}

pub(crate) fn radar_rows(
    contacts: &[SensorTrackSnapshot],
    style: &ContactProxyConfig,
) -> Vec<EntitySnapshot> {
    contacts
        .iter()
        .map(|track| EntitySnapshot {
            uuid: track.contact_id.clone(),
            name: track
                .identity
                .as_ref()
                .map(|identity| identity.name.clone()),
            position: Some([track.position[0], 0.0, track.position[1]]),
            // The producing scanner already applied its detection/tag rules. The
            // generic tag keeps an unidentified return visible without inventing
            // a ship/station classification or copying private actor tags.
            tags: vec![crate::gm_contact::BASIC_RADAR_TAG.into()],
            radar_icon: Some(style.radar_icon.clone()),
            radar_size: Some(diameter(style.minimum_radius)),
            colour: Some(style.colour),
            shape: (track.position_error > 0.0).then(|| "sphere".into()),
            // The existing GUI region adapter consumes a world-space diameter.
            radius: (track.position_error > 0.0).then_some(diameter(track.position_error)),
            region_colour: (track.position_error > 0.0).then_some(style.colour),
            ..Default::default()
        })
        .collect()
}

/// Charts retain only explicitly authored chart permission, never live actors'
/// health, targeting details or dynamic equipment state.
pub(crate) fn chart_rows(
    entities: &[EntitySnapshot],
    visible: impl Fn(&str) -> bool,
) -> Vec<EntitySnapshot> {
    entities
        .iter()
        .filter(|entity| {
            visible(&entity.uuid)
                && entity
                    .tags
                    .iter()
                    .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
        })
        .map(|entity| EntitySnapshot {
            uuid: entity.uuid.clone(),
            name: entity.name.clone(),
            position: entity.position,
            tags: entity.tags.clone(),
            shape: entity.shape.clone(),
            radius: entity.radius,
            colour: entity.colour,
            yaw: entity.yaw,
            inner_radius: entity.inner_radius,
            radar_size: entity.radar_size,
            half_extents: entity.half_extents,
            radar_icon: entity.radar_icon.clone(),
            region_colour: entity.region_colour,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
#[path = "measured_contacts_tests.rs"]
pub(super) mod tests;
