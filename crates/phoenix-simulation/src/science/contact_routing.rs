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

pub(crate) fn measured_machinery(
    tracks: Option<&BroadTracks>,
    controls: Option<&super::broad::SensingControls>,
) -> bool {
    tracks.is_some() || controls.is_some()
}

/// The no-fallback boundary also applies to focused-only fitted hulls. A
/// missing/empty permitted Broad picture never turns a guessed UUID into input.
pub(crate) fn resolve_machinery(
    config: Option<&ShipConfig>,
    boards: Option<&ShipSystemBlackboards>,
    tracks: Option<&BroadTracks>,
    controls: Option<&super::broad::SensingControls>,
    consumer: &SystemId,
    presentation: &str,
) -> Option<String> {
    if !measured_machinery(tracks, controls) {
        return (!presentation.is_empty()).then(|| presentation.to_owned());
    }
    resolve(config?, boards?, Some(tracks?), consumer, presentation)
}

pub(crate) fn machinery_alias(
    config: Option<&ShipConfig>,
    boards: Option<&ShipSystemBlackboards>,
    tracks: Option<&BroadTracks>,
    controls: Option<&super::broad::SensingControls>,
    consumer: &SystemId,
    source: &str,
) -> Option<String> {
    if !measured_machinery(tracks, controls) {
        return (!source.is_empty()).then(|| source.to_owned());
    }
    alias_for_source(config?, boards?, Some(tracks?), consumer, source)
}

/// A mission's named request is not detection. Resolve it only through one
/// unambiguous current, selectable record the consuming Console already holds.
pub(crate) fn request_contact(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: &BroadTracks,
    consumer: &SystemId,
    request: &str,
) -> Option<(String, String)> {
    if let Some(source) = resolve(config, boards, Some(tracks), consumer, request) {
        return Some((request.to_owned(), source));
    }
    let view = config.observations_for_system(consumer, &boards.0);
    let mut matches = std::collections::BTreeMap::new();
    for owner in tracks.0.keys() {
        let Some(public) = view.get(owner).and_then(observations) else {
            continue;
        };
        for record in public {
            if record.contact_id != request
                && !record.identity.as_ref().is_some_and(|identity| {
                    identity.name == request
                        || identity.references.iter().any(|name| name == request)
                })
            {
                continue;
            }
            if let Some(source) =
                resolve(config, boards, Some(tracks), consumer, &record.contact_id)
            {
                matches.insert(record.contact_id.clone(), source);
            }
        }
    }
    (matches.len() == 1).then(|| matches.into_iter().next().unwrap())
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

/// Current public handles which this exact consuming Console can independently
/// route to an already-authorized physical source. This is actuator correlation,
/// not evidence fusion: the original observations and their times stay intact.
pub(crate) fn aliases_for_source(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: Option<&BroadTracks>,
    consumer: &SystemId,
    source: &str,
) -> std::collections::BTreeSet<String> {
    let Some(tracks) = tracks else {
        return (!source.is_empty())
            .then(|| source.to_owned())
            .into_iter()
            .collect();
    };
    let permitted = config.observations_for_system(consumer, &boards.0);
    tracks
        .0
        .keys()
        .filter_map(|owner| {
            permitted
                .get(&publication_source(config, owner))
                .and_then(observations)
        })
        .flatten()
        .filter(|record| {
            resolve(config, boards, Some(tracks), consumer, &record.contact_id).as_deref()
                == Some(source)
        })
        .map(|record| record.contact_id.clone())
        .collect()
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
    let find = |owner: &SystemId,
                state: &phoenix_sim_gameplay::science::tracks::BroadTrackState| {
        let mut candidates = observations(permitted.get(&publication_source(config, owner))?)?
            .iter()
            .filter(|track| {
                track.selectable
                    && !track.predicted
                    && state.resolve_current(&track.contact_id) == Some(source)
            });
        let first = candidates.next()?;
        candidates
            .next()
            .is_none()
            .then(|| first.contact_id.clone())
    };
    // Prefer the consuming radar's own permitted public picture. Fusion may
    // retain several private samples but publishes one primary contact handle.
    tracks
        .0
        .get(consumer)
        .and_then(|state| find(consumer, state))
        .or_else(|| {
            tracks
                .0
                .iter()
                .find_map(|(owner, state)| find(owner, state))
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
