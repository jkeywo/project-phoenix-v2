//! Entity-inspector surface on the structured debug pipeline (issue #1150,
//! PRD #1144).
//!
//! The migration of the legacy `debug_overlay::update_entity_inspector` text
//! block onto the #1145 pipeline. It carries the player ship's position,
//! per-system hull and per-arc shields, plus every non-asteroid world entity's
//! name, tags, position, distance, faction, hull, comms hailability and Tactical
//! lock. [`project_inspector`] is the pure, Bevy-free core — it derives each
//! entity's distance from the player, its comms in-range flag, and sorts the
//! entities by distance (then name) so the JSON is deterministic — and the
//! publish system gathers the queries into it.
//!
//! # Determinism
//!
//! The publish system reads presentation/authoritative components and the
//! faction registry and writes only the presentation [`EntityInspectorCapture`]
//! and the WASM bridge, so it cannot move the #894 digest (proven by
//! `tests/debug_overlays.rs`).

use bevy::prelude::*;

use crate::debug::payload::{
    EntityInspectorPayload, InspectorEntity, InspectorHullEntry, InspectorPlayer,
    InspectorShieldFacing, DEBUG_SCHEMA_VERSION,
};

impl crate::debug::catalogue::DebugSurfaceState
    for crate::debug_overlay::DebugEntityInspectorEnabled
{
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the entity-inspector Debug Surface.
pub const DEBUG_INSPECTOR_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<
        crate::debug_overlay::DebugEntityInspectorEnabled,
    >(crate::core::debug_surface::DebugSurface::Inspector);

/// The latest entity-inspector JSON, when capture is enabled (issue #1150).
///
/// The target-agnostic sink, mirroring `debug::StationActivityCapture`. `None`
/// until the first publish; never folded into the digest.
#[derive(Resource, Default, Debug)]
pub struct EntityInspectorCapture(pub Option<String>);

/// The player ship's already-extracted inspector data, the Bevy-free input to
/// [`project_inspector`]. Its `x`/`z` also anchor every entity's distance.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorPlayerInput {
    pub x: f32,
    pub z: f32,
    pub hull: Vec<InspectorHullEntry>,
    pub shields: Vec<InspectorShieldFacing>,
}

/// One world entity's already-extracted inspector data, the Bevy-free input to
/// [`project_inspector`]. Distance and the comms in-range flag are DERIVED by the
/// projection from the player position, not carried here.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorEntityInput {
    pub name: String,
    pub tags: Vec<String>,
    pub x: f32,
    pub z: f32,
    pub faction: Option<String>,
    pub hull_current: Option<f32>,
    pub hull_max: Option<f32>,
    /// `Some(range)` when the entity has a comms range (and so is hailable).
    pub comms_range: Option<f32>,
    /// `Some(target-or-"none")` when the entity carries a Tactical selection.
    pub ai_target: Option<String>,
}

/// Project the player block and extracted entities into the wire payload.
///
/// Derives each entity's planar distance from the player (from `(0, 0)` when
/// there is no player) and its comms in-range flag, then sorts the entities by
/// distance and name — a total order, so two hosts folding the same world
/// serialise byte-identical JSON.
pub fn project_inspector(
    player: Option<InspectorPlayerInput>,
    entities: Vec<InspectorEntityInput>,
) -> EntityInspectorPayload {
    let (px, pz) = player.as_ref().map(|p| (p.x, p.z)).unwrap_or((0.0, 0.0));

    let mut out: Vec<InspectorEntity> = entities
        .into_iter()
        .map(|e| {
            let dx = e.x - px;
            let dz = e.z - pz;
            let distance = (dx * dx + dz * dz).sqrt();
            let comms_hailable = e.comms_range.map(|_| true);
            let comms_in_range = e.comms_range.map(|r| distance <= r);
            InspectorEntity {
                name: e.name,
                tags: e.tags,
                x: e.x,
                z: e.z,
                distance,
                faction: e.faction,
                hull_current: e.hull_current,
                hull_max: e.hull_max,
                comms_hailable,
                comms_in_range,
                comms_range: e.comms_range,
                ai_target: e.ai_target,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| a.name.cmp(&b.name))
    });

    EntityInspectorPayload {
        schema_version: DEBUG_SCHEMA_VERSION,
        player: player.map(|p| InspectorPlayer {
            x: p.x,
            z: p.z,
            hull: p.hull,
            shields: p.shields,
        }),
        entities: out,
    }
}

/// Project the entity inspector to JSON when capture is enabled (flag-gated).
///
/// The player block is present when a LocalShip carries shields (the browser
/// host); a headless run has no LocalShip, so `player` is `None` and only the
/// world entities are listed.
///
/// The `DebugEntityInspectorEnabled` flag is taken as `Option<Res<..>>` and the
/// projection short-circuits when it is absent or off — see
/// `publish_modifier_debug` for why gating inside the system beats a `run_if` on
/// a possibly-absent flag resource. Read-only w.r.t. every folded resource; see
/// the module docs.
#[allow(clippy::type_complexity)]
pub fn publish_entity_inspector_debug(
    enabled: Option<Res<crate::debug_overlay::DebugEntityInspectorEnabled>>,
    entities: Query<
        (
            &Transform,
            &crate::entities::spawner::EntityName,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::comms::component::CommsRange>,
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &crate::entities::spawner::EntityTagsSection,
        ),
        bevy::ecs::query::Without<crate::server_app::Asteroid>,
    >,
    ship_physics_q: Query<&crate::ship::state::ShipPhysics, With<crate::server_app::LocalShip>>,
    player_hull_q: Query<
        &crate::entities::spawner::EntitySystemHull,
        With<crate::server_app::LocalShip>,
    >,
    ship_shields_q: Query<&crate::server_app::ShipShields, With<crate::server_app::LocalShip>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    mut capture: ResMut<EntityInspectorCapture>,
) {
    if !enabled.map(|f| f.0).unwrap_or(false) {
        return;
    }

    // The player block, present only when a LocalShip carries shields — the same
    // precondition the legacy overlay used to render anything at all.
    let player = ship_shields_q.iter().next().map(|shields| {
        let phys = ship_physics_q.iter().next().copied().unwrap_or_default();
        let hull = player_hull_q
            .iter()
            .next()
            .map(|h| {
                h.0.entries()
                    .map(|(sid, cur, max)| InspectorHullEntry {
                        system: sid.0.clone(),
                        current: cur,
                        max,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let shield_facings = shields
            .0
            .facings
            .iter()
            .map(|f| InspectorShieldFacing {
                label: f.label.clone(),
                hp: f.hp,
                max_hp: f.max_hp,
                offline: f.offline_remaining > 0.0,
                focused: f.is_focused,
            })
            .collect();
        InspectorPlayerInput {
            x: phys.x,
            z: phys.z,
            hull,
            shields: shield_facings,
        }
    });

    let entity_inputs: Vec<InspectorEntityInput> = entities
        .iter()
        .map(
            |(transform, name, hull, faction_comp, comms_range, ai, tags)| {
                let p = transform.translation;
                let (hull_current, hull_max) = match hull {
                    Some(h) => (Some(h.0.total_current()), Some(h.0.total_max())),
                    None => (None, None),
                };
                // Present whenever the entity has a faction — "<unknown>" when the
                // registry has no name for it, matching the legacy overlay.
                let faction = faction_comp.map(|fc| {
                    faction_registry
                        .as_ref()
                        .and_then(|r| r.0.get(&fc.0).map(|f| f.name.clone()))
                        .unwrap_or_else(|| "<unknown>".to_string())
                });
                InspectorEntityInput {
                    name: name.0.clone(),
                    tags: tags.0.clone(),
                    x: p.x,
                    z: p.z,
                    faction,
                    hull_current,
                    hull_max,
                    comms_range: comms_range.map(|r| r.0),
                    ai_target: ai.map(|t| t.0.clone().unwrap_or_else(|| "none".to_string())),
                }
            },
        )
        .collect();

    let payload = project_inspector(player, entity_inputs);
    let json = crate::core::codec::encode_entity_inspector(&payload);

    capture.0 = Some(json);
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod tests;
