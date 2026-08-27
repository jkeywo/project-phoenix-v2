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

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_entity_inspector_string(json.clone());

    capture.0 = Some(json);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(name: &str, x: f32, z: f32) -> InspectorEntityInput {
        InspectorEntityInput {
            name: name.to_string(),
            tags: vec![],
            x,
            z,
            faction: None,
            hull_current: None,
            hull_max: None,
            comms_range: None,
            ai_target: None,
        }
    }

    #[test]
    fn no_player_still_lists_entities_versioned() {
        let payload = project_inspector(None, vec![ent("scout", 3.0, 4.0)]);
        assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
        assert!(payload.player.is_none());
        assert_eq!(payload.entities.len(), 1);
        // Distance from the origin fallback: sqrt(3^2 + 4^2) = 5.
        assert!((payload.entities[0].distance - 5.0).abs() < 1e-4);
    }

    #[test]
    fn distance_is_measured_from_the_player_and_sorts_the_list() {
        let player = InspectorPlayerInput {
            x: 10.0,
            z: 0.0,
            hull: vec![InspectorHullEntry {
                system: "core".into(),
                current: 50.0,
                max: 100.0,
            }],
            shields: vec![InspectorShieldFacing {
                label: "Fore".into(),
                hp: 20,
                max_hp: 40,
                offline: false,
                focused: true,
            }],
        };
        // "far" is 20u away, "near" is 5u away — expect near first after sorting.
        let payload = project_inspector(
            Some(player),
            vec![ent("far", 30.0, 0.0), ent("near", 15.0, 0.0)],
        );
        let p = payload.player.expect("player present");
        assert_eq!(p.hull.len(), 1);
        assert_eq!(p.shields[0].label, "Fore");
        assert!(p.shields[0].focused);
        assert_eq!(payload.entities.len(), 2);
        assert_eq!(payload.entities[0].name, "near");
        assert!((payload.entities[0].distance - 5.0).abs() < 1e-4);
        assert_eq!(payload.entities[1].name, "far");
        assert!((payload.entities[1].distance - 20.0).abs() < 1e-4);
    }

    #[test]
    fn comms_range_derives_hailable_and_in_range() {
        let mut in_range = ent("friendly", 3.0, 0.0);
        in_range.comms_range = Some(10.0); // 3u away, within 10u range
        let mut out_of_range = ent("distant", 50.0, 0.0);
        out_of_range.comms_range = Some(10.0); // 50u away, outside range
        let payload = project_inspector(None, vec![in_range, out_of_range]);
        // Sorted by distance: friendly (3u) first.
        let friendly = &payload.entities[0];
        assert_eq!(friendly.name, "friendly");
        assert_eq!(friendly.comms_hailable, Some(true));
        assert_eq!(friendly.comms_in_range, Some(true));
        let distant = &payload.entities[1];
        assert_eq!(distant.comms_hailable, Some(true));
        assert_eq!(distant.comms_in_range, Some(false));
    }

    #[test]
    fn no_comms_component_leaves_comms_fields_absent() {
        let payload = project_inspector(None, vec![ent("rock-adjacent", 1.0, 1.0)]);
        let e = &payload.entities[0];
        assert_eq!(e.comms_hailable, None);
        assert_eq!(e.comms_in_range, None);
        assert_eq!(e.comms_range, None);
    }
}
