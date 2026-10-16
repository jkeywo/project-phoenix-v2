//! Adapt permitted findings into the existing pure Helm inputs. No scene query
//! or authoritative observation state belongs in this temporary projection.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct MeasuredHelm {
    pub view: crate::ai::WorldView,
    pub routes: BTreeMap<uuid::Uuid, uuid::Uuid>,
    pub velocities: BTreeMap<uuid::Uuid, [f32; 2]>,
    known: Vec<(String, Vec<String>, uuid::Uuid)>,
    current: BTreeSet<uuid::Uuid>,
}

/// A controller-local key from the PUBLIC handle, never the actual actor UUID.
fn contact_key(handle: &str) -> uuid::Uuid {
    let high = vellum_digest::fnv1a(format!("helm-measured-a:{handle}").as_bytes());
    let low = vellum_digest::fnv1a(format!("helm-measured-b:{handle}").as_bytes());
    let mut bytes = ((u128::from(high) << 64) | u128::from(low)).to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

impl MeasuredHelm {
    pub fn selection(&self, reference: Option<&str>) -> Option<uuid::Uuid> {
        let reference = reference?;
        // A frozen selection may carry its private actuator reference. It only
        // routes to a current selectable contact from the permitted picture.
        uuid::Uuid::parse_str(reference)
            .ok()
            .and_then(|id| self.routes.get(&id).copied())
            .or_else(|| {
                self.known
                    .iter()
                    .find(|(handle, _, _)| handle == reference)
                    .map(|(_, _, key)| *key)
                    .filter(|key| self.current.contains(key))
            })
    }

    pub fn named_target(&self, reference: &str) -> Option<uuid::Uuid> {
        let mut candidates = self.known.iter().filter(|(handle, names, _)| {
            handle == reference || names.iter().any(|name| name == reference)
        });
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first.2)
    }

    pub fn destroy_target(
        &self,
        scored: &[crate::core::messages::ScoredObjective],
        shared: &[uuid::Uuid],
        registry: &crate::ai::faction::FactionRegistry,
    ) -> Option<uuid::Uuid> {
        use crate::core::messages::{AiDirective, SystemAffinity};
        let objective = scored.iter().find(|objective| {
            objective.score > 0.0
                && objective.relevance.contains(&SystemAffinity::Helm)
                && matches!(objective.directive, AiDirective::Destroy { .. })
        })?;
        let AiDirective::Destroy { target } = &objective.directive else {
            return None;
        };
        if target.is_empty() {
            helm_destroy_target(scored, &self.view, shared, registry)
        } else {
            self.named_target(target)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    boards: &impl HelmObservations,
    tracks: Option<&crate::science::broad::BroadTracks>,
    controls: Option<&crate::science::broad::SensingControls>,
    physics: &ShipPhysics,
    observer: Option<&crate::entities::spawner::EntityUuid>,
    faction: Option<&crate::entities::spawner::FactionComponent>,
    collider: Option<&crate::entities::spawner::ColliderSection>,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    chart: Option<&crate::lobby::server::WorldResource>,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
) -> MeasuredHelm {
    let mut result = MeasuredHelm {
        view: crate::ai::WorldView {
            entity_pos: [physics.x, physics.y, physics.z],
            entity_yaw: physics.yaw,
            self_faction: faction.map(|faction| faction.0),
            self_radius: collider.map_or(0.0, |collider| collider.0.radius),
            self_size_rating: collider.map_or(0.0, |collider| collider.0.radius),
            anchors: anchors.clone(),
            ..Default::default()
        },
        routes: BTreeMap::new(),
        velocities: BTreeMap::new(),
        known: Vec::new(),
        current: BTreeSet::new(),
    };
    // Without the observer key, an active observer-specific permission table
    // cannot be applied. Missing scope must not become a legacy world view.
    if runtime.is_some() && observer.is_none() {
        return result;
    }
    let mut publications: Vec<_> = boards.iter().collect();
    publications.sort_by_key(|(owner, _)| *owner);
    let preferred = controls.and_then(|controls| controls.0.combat_lock.as_ref());
    let mut routes =
        BTreeMap::<uuid::Uuid, Vec<(crate::core::messages::SystemId, uuid::Uuid)>>::new();
    let mut records = BTreeMap::<
        uuid::Uuid,
        Vec<(
            &crate::core::messages::SystemId,
            &phoenix_model::sensing_tracks::SensorTrackSnapshot,
        )>,
    >::new();
    for (owner, board) in &publications {
        for track in crate::science::contact_routing::observations(board).unwrap_or_default() {
            records
                .entry(contact_key(&track.contact_id))
                .or_default()
                .push((owner, track));
        }
    }
    for (key, copies) in records {
        let track = copies[0].1;
        // Explicit feeds may carry the SAME held record through several boards.
        // Collapse exact copies; conflicting records/handle collisions confer no
        // pose or control preference merely because one source sorts first.
        if copies.iter().any(|(_, other)| *other != track)
            || !track.position.into_iter().all(f32::is_finite)
            || !track.position_error.is_finite()
            || track.position_error < 0.0
        {
            continue;
        }
        let actuals: BTreeSet<_> = copies
            .iter()
            .filter_map(|(owner, _)| {
                tracks
                    .and_then(|tracks| tracks.0.get(*owner))
                    .and_then(|state| state.resolve_current(&track.contact_id))
                    .and_then(|actual| uuid::Uuid::parse_str(actual).ok())
            })
            .collect();
        if observer.is_some_and(|observer| {
            actuals.iter().any(|actual| {
                actual.to_string() == observer.0
                    || runtime.is_some_and(|runtime| {
                        crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &observer.0,
                            &actual.to_string(),
                        ) == crate::gm::operations::contact::ContactMode::Conceal
                    })
            })
        }) {
            continue;
        }
        let names = track.identity.as_ref().map_or_else(Vec::new, |identity| {
            let mut names = identity.references.clone();
            if !identity.name.is_empty() {
                names.push(identity.name.clone());
            }
            names
        });
        result.known.push((track.contact_id.clone(), names, key));
        if let Some(velocity) = track
            .velocity
            .filter(|velocity| velocity.iter().all(|value| value.is_finite()))
        {
            result.velocities.insert(key, velocity);
        }
        result.view.entities.push(crate::ai::AiWorldEntity {
            uuid: key,
            name: track
                .identity
                .as_ref()
                .map(|identity| identity.name.clone()),
            position: [track.position[0], 0.0, track.position[1]],
            faction: track.identity.as_ref().and_then(|identity| {
                identity
                    .faction
                    .as_deref()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok())
            }),
            // A current uncertainty envelope is conservative avoidance geometry.
            // Lost history stays available for navigation/knowledge, but growing
            // uncertainty is not an expanding solid obstacle that can force the
            // ship away from a separate current contact indefinitely.
            radius: track.position_error,
            dangerous: !track.predicted,
            // Unknown physical size cannot satisfy an ignore-smaller rule;
            // do not invent an authored mobile-hull profile from a return.
            movable: false,
            ..Default::default()
        });
        if track.selectable && !track.predicted {
            result.current.insert(key);
            if actuals.len() == 1 {
                let actual = *actuals.first().unwrap();
                for (owner, _) in &copies {
                    routes
                        .entry(actual)
                        .or_default()
                        .push(((*owner).clone(), key));
                }
            }
        }
    }
    for (actual, candidates) in routes {
        let selected = preferred
            .and_then(|preferred| candidates.iter().find(|(owner, _)| owner == preferred))
            .or_else(|| {
                candidates
                    .iter()
                    .all(|(_, key)| *key == candidates[0].1)
                    .then(|| &candidates[0])
            });
        if let Some((_, key)) = selected {
            result.routes.insert(actual, *key);
        }
    }
    // Explicit chart permission supplies public geometry only. Conceal remains
    // observer-specific; neither a chart nor an observation grants live actor facts.
    if let Some(observer) = observer {
        for entity in chart.into_iter().flat_map(|world| &world.0.entities) {
            if entity.uuid == observer.0
                || !entity
                    .tags
                    .iter()
                    .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
                || runtime.is_some_and(|runtime| {
                    crate::gm::operations::contact::mode(
                        &runtime.contact_overrides,
                        &observer.0,
                        &entity.uuid,
                    ) == crate::gm::operations::contact::ContactMode::Conceal
                })
            {
                continue;
            }
            let Some(position) = entity
                .position
                .filter(|position| position.iter().all(|value| value.is_finite()))
            else {
                continue;
            };
            let key = contact_key(&format!("chart:{}", entity.uuid));
            result.known.push((
                entity.uuid.clone(),
                entity.name.clone().into_iter().collect(),
                key,
            ));
            result.view.entities.push(crate::ai::AiWorldEntity {
                uuid: key,
                name: entity.name.clone(),
                position,
                radius: entity
                    .radius
                    .filter(|radius| radius.is_finite() && *radius >= 0.0)
                    .unwrap_or(0.0),
                ..Default::default()
            });
        }
    }
    result.view.entities.sort_by_key(|entity| entity.uuid);
    result
}

#[cfg(test)]
#[path = "surfaces_measured_tests.rs"]
mod tests;
