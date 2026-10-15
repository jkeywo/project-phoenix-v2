//! Correlation of held dossier findings with retained contact observations.
//! This port has no world entities, current condition or private routing keys.
use super::tracks::BroadTrack;
use phoenix_model::messages::DossierBlackboard;
use phoenix_model::sensing_references::{AttachedSensorReference, KnownReferenceMetadata};
use std::collections::BTreeMap;

/// Unique marked findings from a Console's permitted dossier publication.
/// Ambiguous/reused IDs are withheld rather than choosing a convenient source.
pub fn held_references(dossiers: &DossierBlackboard) -> BTreeMap<String, AttachedSensorReference> {
    let mut held = BTreeMap::new();
    for subject in &dossiers.subjects {
        for evidence in &subject.evidence {
            let Some(reference) = evidence
                .reference
                .as_ref()
                .filter(|value| value.validate().is_ok())
            else {
                continue;
            };
            let candidate = AttachedSensorReference {
                reference: reference.clone(),
                text: evidence.text.clone(),
                provenance: evidence.provenance.clone(),
                gathered_at_tick: evidence.gathered_at_tick,
                attached_at_tick: evidence.gathered_at_tick,
                source_name: subject.name.clone(),
            };
            held.entry(reference.id.clone())
                .and_modify(|prior: &mut Option<AttachedSensorReference>| {
                    if prior.as_ref() != Some(&candidate) {
                        *prior = None;
                    }
                })
                .or_insert(Some(candidate));
        }
    }
    held.into_iter()
        .filter_map(|(id, value)| value.map(|value| (id, value)))
        .collect()
}

pub fn attach(
    track: &mut BroadTrack,
    reference: &KnownReferenceMetadata,
    tick: u64,
) -> Result<(), String> {
    reference.validate()?;
    if tick < track.observed_tick {
        return Err("reference attachment predates the observed contact".into());
    }
    // Duplicate requests never renew the original attachment time.
    track
        .attached_references
        .entry(reference.id.clone())
        .or_insert(tick);
    Ok(())
}

/// Full-candidate preflight supplies the known marked IDs, not live actors.
pub fn validate_attachments(
    state: &super::tracks::BroadTrackState,
    known: &BTreeMap<String, u64>,
    tick: u64,
) -> Result<(), String> {
    if state.tracks.values().any(|track| {
        track.attached_references.iter().any(|(id, attached)| {
            known.get(id).is_none_or(|gathered| attached < gathered) || *attached > tick
        })
    }) {
        return Err("sensor attachment has no held reference or has a future clock".into());
    }
    Ok(())
}

/// Ordinary matches stay responsive. An explicitly unusual finding is shown
/// only after deliberate attachment; it never changes observed identity.
pub fn project(track: &BroadTrack, dossiers: &DossierBlackboard) -> Vec<AttachedSensorReference> {
    held_references(dossiers)
        .into_values()
        .filter_map(|mut reference| {
            let deliberate = track
                .attached_references
                .get(&reference.reference.id)
                .copied();
            let routine = !reference.reference.unusual
                && track.identity.as_ref().is_some_and(|identity| {
                    !reference.source_name.is_empty() && identity.name == reference.source_name
                });
            reference.attached_at_tick = deliberate.or_else(|| {
                routine.then_some(
                    track.observed_tick.max(reference.gathered_at_tick).max(
                        track
                            .observation
                            .as_ref()
                            .and_then(|metadata| metadata.received_tick)
                            .unwrap_or(0),
                    ),
                )
            })?;
            Some(reference)
        })
        .collect()
}

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;
