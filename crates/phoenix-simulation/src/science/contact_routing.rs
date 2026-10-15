//! Resolve a permitted measured contact at the existing machinery boundary.
//! Physical associations stay on the observing ship; Console hosting grants no
//! access to another Console's observations or to unobserved world identities.
use super::broad::BroadTracks;
use crate::core::messages::{SystemBlackboard, SystemId};
use crate::server_app::ShipSystemBlackboards;
use crate::ship::config::ShipConfig;
use phoenix_model::sensing_tracks::SensorTrackSnapshot;

pub fn publication_source(_config: &ShipConfig, owner: &SystemId) -> SystemId {
    owner.clone()
}

pub fn observations(board: &SystemBlackboard) -> Option<&[SensorTrackSnapshot]> {
    match board {
        SystemBlackboard::SensorRadar(board) => board.tracks.as_deref(),
        SystemBlackboard::TacticalRadar(board) => board.tracks.as_deref(),
        SystemBlackboard::Helm(board) => board.tracks.as_deref(),
        _ => None,
    }
}

/// Lift an observing radar's already-published selection into the existing
/// private machinery reference. This is derived routing, never command
/// admission or evidence of identity. Outgoing publications project it back
/// through the producing Console's permitted contact view.
pub(crate) fn machinery_control_selection(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: &BroadTracks,
    owner: Option<SystemId>,
    selected: Option<&str>,
) -> Option<String> {
    let selected = selected?;
    alias_for_source(config, boards, Some(tracks), &owner?, selected).map(|_| selected.to_owned())
}

pub fn resolve(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: Option<&BroadTracks>,
    consumer: &SystemId,
    presentation: &str,
) -> Option<String> {
    let Some(tracks) = tracks else {
        return (!presentation.is_empty()).then(|| presentation.to_owned());
    };
    let permitted = config.observations_for_system(consumer, &boards.0);
    tracks.0.iter().find_map(|(source, state)| {
        let public = observations(permitted.get(&publication_source(config, source))?)?
            .iter()
            .find(|track| track.contact_id == presentation)?;
        if public.predicted || !public.selectable {
            return None;
        }
        state.resolve_current(presentation).map(str::to_owned)
    })
}

/// Produce an ordinary command argument from an already-held private selection.
/// This uses the same permitted current observations as human target admission.
pub fn alias_for_source(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: Option<&BroadTracks>,
    consumer: &SystemId,
    source: &str,
) -> Option<String> {
    let Some(tracks) = tracks else {
        return (!source.is_empty()).then(|| source.to_owned());
    };
    let permitted = config.observations_for_system(consumer, &boards.0);
    tracks.0.iter().find_map(|(owner, state)| {
        let contact = state.contact_for_source(source)?;
        observations(permitted.get(&publication_source(config, owner))?)?
            .iter()
            .any(|track| track.contact_id == contact && track.selectable && !track.predicted)
            .then(|| contact.to_owned())
    })
}

/// A machinery selection conveys no additional identifying knowledge.
pub(crate) fn identity_for_source(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: &BroadTracks,
    consumer: &SystemId,
    source: &str,
) -> Option<String> {
    let alias = alias_for_source(config, boards, Some(tracks), consumer, source)?;
    let view = config.observations_for_system(consumer, &boards.0);
    boards
        .0
        .keys()
        .filter_map(|owner| view.get(owner))
        .filter_map(observations)
        .flatten()
        .find(|track| track.contact_id == alias && track.selectable && !track.predicted)?
        .identity
        .as_ref()
        .map(|identity| identity.name.clone())
}

#[cfg(test)]
#[path = "contact_routing_tests.rs"]
mod tests;
