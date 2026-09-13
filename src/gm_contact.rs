//! Persistent observing-player-ship / target contact overrides (issue #1309).
//! Reveal changes contact availability only. It grants no classification or scan.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactMode {
    Reveal,
    Conceal,
    #[default]
    Normal,
}

pub type ContactOverrides = BTreeMap<String, BTreeMap<String, ContactMode>>;

/// One deliberately reported identity, resolved from authored content at the
/// admitted apply tick. It never changes the real entity or its capabilities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReportedClassification {
    pub palette: String,
    pub label: String,
}

pub type ContactClassifications = BTreeMap<String, BTreeMap<String, ReportedClassification>>;

/// An absolute classification edit, independent of the detection override.
pub fn set_classification(
    classifications: &mut ContactClassifications,
    observer: &str,
    target: &str,
    requested: Option<ReportedClassification>,
) -> bool {
    if classifications
        .get(observer)
        .and_then(|rows| rows.get(target))
        == requested.as_ref()
    {
        return false;
    }
    if let Some(value) = requested {
        classifications
            .entry(observer.into())
            .or_default()
            .insert(target.into(), value);
    } else if let Some(rows) = classifications.get_mut(observer) {
        rows.remove(target);
        if rows.is_empty() {
            classifications.remove(observer);
        }
    }
    true
}

/// The observing Sensors surface gets only its reported labels, never the GM's
/// palette selection or another observer's interpretation.
pub fn classification_labels(
    classifications: &ContactClassifications,
    observer: &str,
) -> BTreeMap<String, String> {
    classifications
        .get(observer)
        .into_iter()
        .flat_map(|rows| rows.iter())
        .map(|(target, value)| (target.clone(), value.label.clone()))
        .collect()
}

/// Apply only to contacts already admitted by detection. Misclassification is
/// not Reveal: changing interpretation must not create a previously absent blip.
pub fn classify_viewscreen_contacts(
    entities: &mut [crate::core::messages::EntitySnapshot],
    classifications: &ContactClassifications,
    observer: &str,
) {
    let Some(rows) = classifications.get(observer) else {
        return;
    };
    for entity in entities {
        if let Some(value) = rows.get(&entity.uuid) {
            entity.name = Some(value.label.clone());
        }
    }
}

/// Presentation-only sentinel, never an authored entity class or protected fact.
pub const BASIC_RADAR_TAG: &str = "__gm_basic_contact";
pub const BASIC_RADAR_ICON: &str = "__gm_basic_contact";

/// The active Sensors viewscreen's ordinary contact projection, with Reveal as a floor.
/// Only added basic points are clamped to the edge; ordinary snapshots are retained exactly.
pub fn viewscreen_contacts(
    entities: &[crate::core::messages::EntitySnapshot],
    overrides: &BTreeMap<String, ContactMode>,
    x: f32,
    z: f32,
    range: f32,
    shows: &std::collections::HashSet<String>,
) -> Vec<crate::core::messages::EntitySnapshot> {
    entities
        .iter()
        .filter_map(|entity| {
            let mode = overrides.get(&entity.uuid).copied().unwrap_or_default();
            if mode == ContactMode::Conceal {
                return None;
            }
            let dx = entity.x() - x;
            let dz = entity.z() - z;
            let distance = crate::simmath::hypot(dx, dz);
            let ordinary = distance <= range
                && entity.tags.iter().any(|tag| shows.contains(tag))
                && (entity.radar_icon.is_some() || entity.region_colour.is_some())
                && (!entity.tags.iter().any(|tag| tag == "objective_marker")
                    || entity.objective_target);
            if mode != ContactMode::Reveal || ordinary {
                return Some(entity.clone());
            }
            let scale = if distance > range {
                0.96 * range.max(0.0) / distance
            } else {
                1.0
            };
            Some(crate::core::messages::EntitySnapshot {
                uuid: entity.uuid.clone(),
                position: Some([x + dx * scale, 0.0, z + dz * scale]),
                name: Some("console.sensors.basic_contact".into()),
                tags: vec![BASIC_RADAR_TAG.into()],
                radar_icon: Some(BASIC_RADAR_ICON.into()),
                radar_size: Some(4.0),
                objective_target: entity.objective_target,
                ..Default::default()
            })
        })
        .collect()
}

pub fn mode(overrides: &ContactOverrides, observer: &str, target: &str) -> ContactMode {
    overrides
        .get(observer)
        .and_then(|rows| rows.get(target))
        .copied()
        .unwrap_or_default()
}

/// Returns whether the absolute request changed authoritative state.
pub fn set(
    overrides: &mut ContactOverrides,
    observer: &str,
    target: &str,
    requested: ContactMode,
) -> bool {
    if mode(overrides, observer, target) == requested {
        return false;
    }
    if requested == ContactMode::Normal {
        if let Some(rows) = overrides.get_mut(observer) {
            rows.remove(target);
            if rows.is_empty() {
                overrides.remove(observer);
            }
        }
    } else {
        overrides
            .entry(observer.into())
            .or_default()
            .insert(target.into(), requested);
    }
    true
}

/// Every disappearance route is covered, including scripted unload and combat.
pub fn prune(
    mut content: Option<bevy::prelude::ResMut<crate::world::server::WorldContentRuntime>>,
    identities: bevy::prelude::Query<(
        &crate::entities::spawner::EntityUuid,
        bevy::prelude::Has<crate::lockstep::FleetSlotOf>,
    )>,
) {
    let Some(content) = content.as_deref_mut() else {
        return;
    };
    let live: BTreeMap<&str, bool> = identities
        .iter()
        .map(|(id, fleet)| (id.0.as_str(), fleet))
        .collect();
    content.contact_overrides.retain(|observer, rows| {
        if live.get(observer.as_str()) != Some(&true) {
            return false;
        }
        rows.retain(|target, _| live.contains_key(target.as_str()));
        !rows.is_empty()
    });
    content.contact_classifications.retain(|observer, rows| {
        if live.get(observer.as_str()) != Some(&true) {
            return false;
        }
        rows.retain(|target, _| live.contains_key(target.as_str()));
        !rows.is_empty()
    });
}
