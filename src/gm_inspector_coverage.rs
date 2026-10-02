//! Deterministic coverage inventory for the five Live Inspector domains.
//!
//! Domain adapters remain the owners of schema discovery and readings. This
//! module checks their published descriptor tables as one contract: a reading
//! key must have the same canonical grammar as its descriptor, every field has
//! a Live mutability classification, and a named action must name an existing
//! panel owner. Reading maps are deliberately not traversed, so an unknown
//! runtime extension remains visible only to the adapter that understands it
//! and cannot silently become a generic Live control.

use crate::gm_entity_inspector::EntityInspectorProjection;
use crate::gm_presentation_inspector::PresentationInspectorProjection;
use crate::gm_region_inspector::RegionInspectorProjection;
use crate::gm_ship_inspector::ShipInspectorProjection;
use crate::gm_world_inspector::WorldInspectorProjection;
use crate::inspector::{FieldDescriptor, LiveMutability};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveInspectorDomain {
    Entity,
    World,
    Ship,
    Region,
    Presentation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveInspectorCoverageEntry {
    pub domain: LiveInspectorDomain,
    /// Canonical authored/derived schema path from the shared descriptor.
    pub descriptor: String,
    /// Canonical key grammar used by the domain's runtime reading map.
    pub runtime: String,
    pub mutability: LiveMutability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_panel: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveInspectorCoverageError {
    MissingDomain(LiveInspectorDomain),
    EmptyPath(LiveInspectorDomain),
    DescriptorRuntimeMismatch {
        domain: LiveInspectorDomain,
        descriptor: String,
        runtime: String,
    },
    ConflictingEntry {
        domain: LiveInspectorDomain,
        runtime: String,
    },
    MissingActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
    },
    UnexpectedActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
        action_panel: String,
    },
    InvalidActionOwner {
        domain: LiveInspectorDomain,
        runtime: String,
        action_panel: String,
    },
}

pub struct LiveInspectorCoverageSources<'a> {
    pub entity: &'a EntityInspectorProjection,
    pub world: &'a WorldInspectorProjection,
    pub ship: &'a ShipInspectorProjection,
    pub region: &'a RegionInspectorProjection,
    pub presentation: &'a PresentationInspectorProjection,
}

/// Replace instance keys (`station[helm]`, `event[first-contact]`) with their
/// schema grammar (`station[]`, `event[]`). Brackets are data, not structure;
/// malformed unmatched brackets are left intact and will fail the descriptor
/// comparison rather than being guessed into a valid path.
pub fn canonical_runtime_path(path: &str) -> String {
    let mut canonical = String::with_capacity(path.len());
    let mut chars = path.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch != '[' {
            canonical.push(ch);
            continue;
        }
        if let Some(end) = path[start + 1..].find(']') {
            canonical.push_str("[]");
            let end = start + 1 + end;
            while chars.peek().is_some_and(|(index, _)| *index <= end) {
                chars.next();
            }
        } else {
            canonical.push_str(&path[start..]);
            break;
        }
    }
    canonical
}

fn allowed_action_owners(domain: LiveInspectorDomain) -> &'static [&'static str] {
    match domain {
        LiveInspectorDomain::Entity => &["npc"],
        LiveInspectorDomain::World => &["mission", "objective", "session"],
        LiveInspectorDomain::Ship => &["effect", "station", "system"],
        LiveInspectorDomain::Region => &[],
        LiveInspectorDomain::Presentation => &["presentation"],
    }
}

fn add_entry(
    entries: &mut BTreeMap<(LiveInspectorDomain, String), LiveInspectorCoverageEntry>,
    domain: LiveInspectorDomain,
    runtime: &str,
    descriptor: &FieldDescriptor,
    action_panel: Option<&str>,
) -> Result<(), LiveInspectorCoverageError> {
    if runtime.is_empty() || descriptor.origin.schema_path.is_empty() {
        return Err(LiveInspectorCoverageError::EmptyPath(domain));
    }
    let runtime = canonical_runtime_path(runtime);
    let descriptor_path = canonical_runtime_path(&descriptor.origin.schema_path);
    if runtime != descriptor_path {
        return Err(LiveInspectorCoverageError::DescriptorRuntimeMismatch {
            domain,
            descriptor: descriptor_path,
            runtime,
        });
    }
    match (descriptor.live_mutability, action_panel) {
        (LiveMutability::NamedAction, None) => {
            return Err(LiveInspectorCoverageError::MissingActionOwner { domain, runtime });
        }
        (LiveMutability::NamedAction, Some(owner))
            if !allowed_action_owners(domain).contains(&owner) =>
        {
            return Err(LiveInspectorCoverageError::InvalidActionOwner {
                domain,
                runtime,
                action_panel: owner.into(),
            });
        }
        (LiveMutability::NamedAction, Some(_)) => {}
        (_, Some(owner)) => {
            return Err(LiveInspectorCoverageError::UnexpectedActionOwner {
                domain,
                runtime,
                action_panel: owner.into(),
            });
        }
        (_, None) => {}
    }
    let entry = LiveInspectorCoverageEntry {
        domain,
        descriptor: descriptor_path,
        runtime: runtime.clone(),
        mutability: descriptor.live_mutability,
        action_panel: action_panel.map(str::to_owned),
    };
    match entries.entry((domain, runtime.clone())) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(entry);
        }
        std::collections::btree_map::Entry::Occupied(slot) if slot.get() == &entry => {}
        std::collections::btree_map::Entry::Occupied(_) => {
            return Err(LiveInspectorCoverageError::ConflictingEntry { domain, runtime });
        }
    }
    Ok(())
}

/// Build the sorted, de-duplicated active-schema inventory.
///
/// A domain with no descriptors is an error. This prevents a projection
/// wiring regression from producing a plausible four-domain manifest.
pub fn inventory(
    sources: LiveInspectorCoverageSources<'_>,
) -> Result<Vec<LiveInspectorCoverageEntry>, LiveInspectorCoverageError> {
    let mut entries = BTreeMap::new();
    let mut domains = BTreeSet::new();
    for field in &sources.entity.fields {
        domains.insert(LiveInspectorDomain::Entity);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Entity,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.world.fields {
        domains.insert(LiveInspectorDomain::World);
        add_entry(
            &mut entries,
            LiveInspectorDomain::World,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.ship.fields {
        domains.insert(LiveInspectorDomain::Ship);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Ship,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for field in &sources.region.fields {
        domains.insert(LiveInspectorDomain::Region);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Region,
            &field.id,
            &field.descriptor,
            None,
        )?;
    }
    for field in &sources.presentation.fields {
        domains.insert(LiveInspectorDomain::Presentation);
        add_entry(
            &mut entries,
            LiveInspectorDomain::Presentation,
            &field.id,
            &field.descriptor,
            field.action_panel.as_deref(),
        )?;
    }
    for domain in [
        LiveInspectorDomain::Entity,
        LiveInspectorDomain::World,
        LiveInspectorDomain::Ship,
        LiveInspectorDomain::Region,
        LiveInspectorDomain::Presentation,
    ] {
        if !domains.contains(&domain) {
            return Err(LiveInspectorCoverageError::MissingDomain(domain));
        }
    }
    Ok(entries.into_values().collect())
}

#[cfg(test)]
#[path = "gm_inspector_coverage_tests.rs"]
mod tests;
