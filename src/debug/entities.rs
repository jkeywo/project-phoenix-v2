//! Entity-behavior surface on the structured debug pipeline (issue #1150,
//! PRD #1144).
//!
//! The migration of the legacy `debug_overlay::write_entity_debug_state` text
//! table onto the #1145 pipeline. Every AI-driven entity (one carrying a
//! `BehaviourSection`) contributes a row: its name, world position, and current
//! Tactical lock (the ship's authoritative `TacticalRadarSelection`, issue
//! #702). [`project_entity_behavior`] is the pure, Bevy-free core — it sorts the
//! rows by name so the JSON is deterministic regardless of ECS iteration order,
//! a property the legacy text table never had — and the publish system gathers
//! the query into it.
//!
//! # Determinism
//!
//! The publish system reads `Transform`, `EntityName`, `BehaviourSection` and
//! `TacticalRadarSelection` and writes only the presentation
//! [`EntityBehaviorCapture`] and the WASM bridge, so it cannot move the #894
//! digest (proven by `tests/debug_overlays.rs`).

use bevy::prelude::*;

use crate::debug::payload::{EntityBehaviorEntry, EntityBehaviorPayload, DEBUG_SCHEMA_VERSION};

impl crate::debug::catalogue::DebugSurfaceState for crate::debug_overlay::DebugEntitiesEnabled {
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the entity-behaviour Debug Surface.
pub const DEBUG_ENTITIES_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<
        crate::debug_overlay::DebugEntitiesEnabled,
    >(crate::core::debug_surface::DebugSurface::Entities);

/// The latest entity-behavior JSON, when capture is enabled (issue #1150).
///
/// The target-agnostic sink, mirroring `debug::StationActivityCapture`. `None`
/// until the first publish; never folded into the digest.
#[derive(Resource, Default, Debug)]
pub struct EntityBehaviorCapture(pub Option<String>);

/// One AI-driven entity's already-extracted debug row, the Bevy-free input to
/// [`project_entity_behavior`].
#[derive(Clone, Debug, PartialEq)]
pub struct EntityBehaviorInput {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub target: String,
}

/// Project the extracted rows into the wire payload, sorted for determinism.
///
/// Sorted by name, then by position bit-pattern and target as a total-order
/// tiebreak, so two hosts folding the same entities serialise byte-identical
/// JSON — the ordering property payload convention 4 asks for and the legacy
/// text table (which numbered rows in raw ECS order) lacked.
pub fn project_entity_behavior(mut inputs: Vec<EntityBehaviorInput>) -> EntityBehaviorPayload {
    inputs.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.x.to_bits().cmp(&b.x.to_bits()))
            .then_with(|| a.y.to_bits().cmp(&b.y.to_bits()))
            .then_with(|| a.z.to_bits().cmp(&b.z.to_bits()))
            .then_with(|| a.target.cmp(&b.target))
    });
    EntityBehaviorPayload {
        schema_version: DEBUG_SCHEMA_VERSION,
        entries: inputs
            .into_iter()
            .map(|i| EntityBehaviorEntry {
                name: i.name,
                x: i.x,
                y: i.y,
                z: i.z,
                target: i.target,
            })
            .collect(),
    }
}

/// Project AI-entity behavior to JSON when capture is enabled (flag-gated).
///
/// The `DebugEntitiesEnabled` flag is taken as `Option<Res<..>>` and the
/// projection short-circuits when it is absent or off — see
/// `publish_modifier_debug` for why gating inside the system beats a `run_if` on
/// a possibly-absent flag resource. Read-only w.r.t. every folded resource; see
/// the module docs.
pub fn publish_entity_behavior_debug(
    enabled: Option<Res<crate::debug_overlay::DebugEntitiesEnabled>>,
    entities: Query<(
        &crate::entities::spawner::BehaviourSection,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
        Option<&crate::console::weapons::TacticalRadarSelection>,
    )>,
    mut capture: ResMut<EntityBehaviorCapture>,
) {
    if !enabled.map(|f| f.0).unwrap_or(false) {
        return;
    }

    let inputs: Vec<EntityBehaviorInput> = entities
        .iter()
        .map(|(_ai, transform, name, memory)| {
            let p = transform.translation;
            EntityBehaviorInput {
                name: name
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|| "<unnamed>".to_string()),
                x: p.x,
                y: p.y,
                z: p.z,
                // The ship's authoritative Tactical lock (issue #702), or "none".
                target: memory
                    .and_then(|t| t.0.clone())
                    .unwrap_or_else(|| "none".to_string()),
            }
        })
        .collect();

    let payload = project_entity_behavior(inputs);
    let json = crate::core::codec::encode_entity_behavior(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_entity_debug_string(json.clone());

    capture.0 = Some(json);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, x: f32, target: &str) -> EntityBehaviorInput {
        EntityBehaviorInput {
            name: name.to_string(),
            x,
            y: 0.0,
            z: 0.0,
            target: target.to_string(),
        }
    }

    #[test]
    fn empty_projects_to_an_empty_versioned_payload() {
        let payload = project_entity_behavior(vec![]);
        assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
        assert!(payload.entries.is_empty());
    }

    #[test]
    fn rows_are_sorted_by_name_and_carry_position_and_target() {
        let payload = project_entity_behavior(vec![
            input("Zephyr", 10.0, "player"),
            input("Aurora", -5.0, "none"),
        ]);
        assert_eq!(payload.entries.len(), 2);
        // Sorted by name: Aurora before Zephyr, regardless of input order.
        assert_eq!(payload.entries[0].name, "Aurora");
        assert!((payload.entries[0].x + 5.0).abs() < f32::EPSILON);
        assert_eq!(payload.entries[0].target, "none");
        assert_eq!(payload.entries[1].name, "Zephyr");
        assert_eq!(payload.entries[1].target, "player");
    }
}
