//! The decision-input surfaces the helm AI reads (issue #702) — a Bevy
//! adapter, not a pure module: [`build_helm_ai_surfaces_frame`] is the system
//! that runs once per shared tick, folding `WorldSnapshot`, the console-owned
//! goal surfaces ([`HelmAiSurfaces`] — waypoint, clearance, cursors) and each
//! ship's viewscreen blackboard into the [`HelmAiSurfacesFrame`] resource
//! every per-axis host then reads.
//!
//! Also owns the doctrine pass surface ([`HelmPassSurface`],
//! [`build_pass_surface`]) the policy-state machine advances, and the
//! Weapons→Helm arc-bearing override ([`apply_arc_bearing_request`]).
//!
//! Invariant: every surface here is something a human operator could equally
//! drive — the Helm AI owns none of it and keeps no private copy, so folding
//! happens exactly once per tick rather than once per host.

use super::*;

/// The console-owned surfaces the AI Helm derives its goals from (issue #702).
///
/// Every one of these is a shared, authoritative surface that a human operator
/// could equally drive — that symmetry is the point. The Helm reads them; it
/// owns none of them, and keeps no private copy of any of them:
///
/// | Surface | Owner | Answers |
/// |---|---|---|
/// | [`TacticalRadarSelection`] | Tactical (human `SetTarget` / `ai_target_selection`) | who to pursue |
/// | [`NavigationWaypoint`] + [`HelmWaypointClearance`] | Navigation (+ the Channel-3 lag) | where to travel |
/// | [`ObjectiveCursors`] | `advance_objective_cursors` | where on the route |
///
/// All `Option` because minimal test spawns omit them; a missing surface means
/// "no goal from that console", never a fabricated default.
///
/// Bundled as one `QueryData` because all three per-axis helm systems need the
/// identical set, and because their per-system queries are close to Bevy's
/// tuple cap.
///
/// [`NavigationWaypoint`]: crate::console::navigation::NavigationWaypoint
/// [`ObjectiveCursors`]: crate::ai::server::ObjectiveCursors
///
/// The Combat Lock (who to pursue) is no longer read from a targeting component
/// here — it comes from this ship's frozen viewscreen blackboard
/// (`ViewscreenBlackboard::combat_lock`, issue #829), read in
/// `build_helm_ai_surfaces_frame`.
#[derive(bevy::ecs::query::QueryData)]
pub struct HelmAiSurfaces {
    waypoint: Option<&'static crate::console::navigation::NavigationWaypoint>,
    clearance: Option<&'static HelmWaypointClearance>,
    cursors: Option<&'static crate::ai::server::ObjectiveCursors>,
}

/// The read-only entity query the helm AI falls back to when `WorldSnapshot`
/// is absent (tests that don't register `AiPlugin`).
pub(crate) type HelmAiFallbackQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static crate::entities::spawner::EntityUuid,
        &'static Transform,
        Option<&'static crate::entities::spawner::EntityName>,
        Option<&'static crate::entities::spawner::FactionComponent>,
        Option<&'static crate::entities::spawner::EntitySystemHull>,
        Option<&'static crate::entities::spawner::ColliderSection>,
    ),
>;

/// Snapshot every world entity for avoidance / target resolution.
///
/// Uses `WorldSnapshot` when available (production); falls back to a direct
/// ECS query for tests that don't register `AiPlugin`.
pub(crate) fn helm_ai_snapshot_entities(
    world_snapshot: Option<&crate::ai::server::WorldSnapshot>,
    runtime_ref: Option<&crate::world::server::WorldContentRuntime>,
    entity_fallback_q: &HelmAiFallbackQuery,
) -> Vec<crate::ai::AiWorldEntity> {
    if let Some(ws) = world_snapshot {
        return ws.entities.clone();
    }
    entity_fallback_q
        .iter()
        .map(|(uuid, transform, name, faction, hull, collider)| {
            let runtime_name = runtime_ref.and_then(|rt| {
                rt.name_to_uuid
                    .iter()
                    .find_map(|(n, mapped)| (mapped == &uuid.0).then(|| n.clone()))
            });
            let hull_fraction = hull.and_then(|h| {
                let max = h.0.total_max();
                (max > 0.0).then(|| h.0.total_current() / max)
            });
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::parse_str(&uuid.0).unwrap_or_default(),
                name: runtime_name.or_else(|| name.map(|n| n.0.clone())),
                position: [
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ],
                faction: faction.map(|f| f.0),
                hull_fraction,
                yaw: Some(-transform.rotation.to_euler(EulerRot::YXZ).0),
                radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                // Mobility is the authored `[collider] movable` fact, matching
                // `ai::server::build_world_snapshot` (issue #958) so the
                // fallback picture and the production one classify terrain the
                // same way. Dangerous, with size rating tracking the collision
                // radius (issue #743).
                movable: collider.map(|c| c.0.movable).unwrap_or(false),
                dangerous: true,
                size_rating: collider.map(|c| c.0.radius).unwrap_or(0.0),
                ..Default::default()
            }
        })
        .collect()
}

/// Read this entity's scored objectives out of its viewscreen blackboard.
pub(crate) fn helm_ai_scored_objectives(
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Vec<crate::core::messages::ScoredObjective> {
    match blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => {
            bb.scored_objectives.clone()
        }
        _ => vec![],
    }
}

/// True when any scored objective is live and Helm-relevant. When false the
/// helm AI has nothing to pursue and zeroes its intent.
pub(crate) fn has_helm_objective(scored: &[crate::core::messages::ScoredObjective]) -> bool {
    scored.iter().any(|o| {
        o.score > 0.0
            && o.relevance
                .contains(&crate::core::messages::SystemAffinity::Helm)
    })
}

/// This ship's damage-scaled helm radar range (issue #674).
///
/// Prefers the live value from the ship's own Helm blackboard entry — which
/// `publish_helm_blackboard` publishes per-entity since #824, so NPCs get the
/// live damage-scaled value too. The static-config fallback remains for ships
/// whose entry has not been published yet (low-LOD ships, and any ship before
/// its first publish); `helm_ai_radar_range_prefers_the_npc_blackboard_entry`
/// pins both sides.
pub(crate) fn helm_ai_radar_range(
    blackboards: &crate::server_app::ShipSystemBlackboards,
    helm_section: Option<&crate::entities::spawner::HelmConsoleSection>,
    ship_client_config: Option<&crate::lobby::server::ShipClientConfigResource>,
    is_local: bool,
) -> f32 {
    let from_blackboard = match blackboards
        .0
        .get(&crate::ship::system_registry::helm_station_key())
    {
        Some(crate::core::messages::SystemBlackboard::Helm(bb)) if bb.radar_range > 0.0 => {
            Some(bb.radar_range)
        }
        _ => None,
    };
    from_blackboard.unwrap_or_else(|| {
        if is_local {
            ship_client_config
                .map(|c| c.0.helm_radar_range)
                .unwrap_or(0.0)
        } else {
            helm_section
                .map(|hc| hc.0.effective_radar_range())
                .unwrap_or(0.0)
        }
    })
}

/// Build the `WorldView` the helm AI reasons over: every snapshot entity
/// except self, gated by this ship's damage-scaled radar range.
#[allow(clippy::too_many_arguments)]
pub(crate) fn helm_ai_world_view(
    physics: &ShipPhysics,
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
    faction: Option<&crate::entities::spawner::FactionComponent>,
    collider: Option<&crate::entities::spawner::ColliderSection>,
    helm_section: Option<&crate::entities::spawner::HelmConsoleSection>,
    blackboards: &crate::server_app::ShipSystemBlackboards,
    ship_client_config: Option<&crate::lobby::server::ShipClientConfigResource>,
    is_local: bool,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    snapshot_entities: &[crate::ai::AiWorldEntity],
) -> crate::ai::WorldView {
    let self_uuid_str = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
    let self_filtered: Vec<crate::ai::AiWorldEntity> = snapshot_entities
        .iter()
        .filter(|e| e.uuid.to_string() != self_uuid_str)
        .cloned()
        .collect();

    let radar_range = helm_ai_radar_range(blackboards, helm_section, ship_client_config, is_local);
    let entity_pos = [physics.x, 0.0, physics.z];
    let entities = crate::ai::visible_entities(entity_pos, radar_range, &self_filtered);

    crate::ai::WorldView {
        entity_pos,
        entity_yaw: physics.yaw,
        anchors: anchors.clone(),
        entities,
        self_faction: faction.map(|f| f.0),
        self_radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
        // Size rating drives the authored ignore-smaller rule (issue #743);
        // populated from the collision radius, the same measure used for
        // published hazard `size_rating`.
        self_size_rating: collider.map(|c| c.0.radius).unwrap_or(0.0),
        ..crate::ai::WorldView::default()
    }
}

/// Explicit console selections are shared intent, not new Helm detections.
/// Add only those live entities to the Helm view, so Helm may act on a target
/// selected by Tactical, Sensors, or Navigation without gaining broad
/// out-of-range awareness.
pub(crate) fn helm_shared_target_view(
    mut world_view: crate::ai::WorldView,
    snapshot_entities: &[crate::ai::AiWorldEntity],
    blackboards: &crate::server_app::ShipSystemBlackboards,
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
) -> (crate::ai::WorldView, Vec<uuid::Uuid>) {
    let mut ids = Vec::new();
    let mut push = |id: Option<String>| {
        if let Some(id) = id.and_then(|id| uuid::Uuid::parse_str(&id).ok()) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    };
    // Combat Lock + Science Target come from the frozen viewscreen blackboard
    // (issue #829, spec §3): cross-system target reads must not reach the
    // tactical / sensor radar's live selection synchronously.
    if let Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) = blackboards
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        push(bb.combat_lock.clone());
        push(bb.science_target.clone());
    }
    if let Some(crate::console::navigation::WaypointMode::Anchored { source_uuid, .. }) =
        waypoint.and_then(|w| w.mode())
    {
        push(Some(source_uuid.clone()));
    }
    for id in &ids {
        if !world_view.entities.iter().any(|e| e.uuid == *id) {
            if let Some(entity) = snapshot_entities.iter().find(|e| e.uuid == *id) {
                world_view.entities.push(entity.clone());
            }
        }
    }
    (world_view, ids)
}

/// Choose Helm's target for the active Destroy directive. Named objectives keep
/// their authored target; untargeted combat directives prefer explicit console
/// selections, then acquire the nearest hostile visible to Helm itself.
pub(crate) fn helm_destroy_target(
    scored: &[crate::core::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    shared: &[uuid::Uuid],
    registry: &crate::ai::faction::FactionRegistry,
) -> Option<uuid::Uuid> {
    use crate::core::messages::{AiDirective, SystemAffinity};
    let objective = scored.iter().find(|o| {
        o.score > 0.0
            && o.relevance.contains(&SystemAffinity::Helm)
            && matches!(o.directive, AiDirective::Destroy { .. })
    })?;
    let AiDirective::Destroy { target } = &objective.directive else {
        return None;
    };
    if !target.is_empty() {
        return crate::ai::resolve_objective_target(target, world_view);
    }
    let hostile = |id: &uuid::Uuid| {
        world_view
            .entities
            .iter()
            .find(|e| e.uuid == *id)
            .is_some_and(|e| {
                crate::ai::faction::is_enemy(world_view.self_faction, e.faction, registry)
            })
    };
    shared
        .iter()
        .find(|id| hostile(id))
        .copied()
        .or_else(|| crate::ai::find_nearest_hostile(world_view, registry))
}

/// The Navigation waypoint this ship's AI Helm is currently *cleared* to follow
/// (issue #702), or `None` if there is none or the clearance has not caught up.
///
/// This is the whole of the Channel-3 Navigation-to-Helm lag on the read side.
/// Navigation — `operate_navigation_ai` or a human's admitted
/// `SetNavigationWaypoint` alike (AGENTS.md rule 6) — sets `NavigationWaypoint`
/// and enqueues a `NavigateTo`
/// carrying its `generation`; that message serves the delivery lag in the queue;
/// the generic router emits a typed delivery and Helm's
/// `receive_helm_coordination` latches the generation into
/// `HelmWaypointClearance`. Until the latch matches, the Helm has been given the
/// waypoint but not yet the order, so this returns `None` — every *new* waypoint
/// re-incurs the lag, not merely the first.
///
/// `None` during the lag does not mean "carry on as before": the waypoint is
/// overwritten in place and the old position is not kept anywhere, so the Helm
/// cannot resume the previous bearing. It falls back to its own local
/// objectives, or idles if it has none, until the clearance catches up.
///
/// A ship missing either component (bare test spawns) is never cleared, which is
/// the same safe default: it falls back to its own local objectives.
pub(crate) fn cleared_nav_waypoint(
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
    clearance: Option<&HelmWaypointClearance>,
) -> Option<[f32; 2]> {
    let waypoint = waypoint?;
    let cleared_generation = clearance?.0?;
    if cleared_generation != waypoint.generation() {
        return None;
    }
    let snapshot = waypoint.snapshot()?;
    Some([snapshot.x, snapshot.z])
}

/// *What* the cleared waypoint names, when it names an entity rather than a
/// place: the `Anchored` waypoint's `source_uuid` (issue #875).
///
/// The clearance gate is [`cleared_nav_waypoint`]'s, called for exactly that
/// rather than restated — a second copy of the generation comparison could drift
/// and would then answer "the helm is cleared to X" on a tick the position half
/// said the helm was cleared to nothing.
///
/// `None` for a `Free` waypoint: a tap-to-place destination is a position and
/// names no entity at all, so there is nothing for a consumer to compare a
/// target against. That is the conservative answer — see
/// `pass_under_navigation_orders`, whose only use of this is to recognise a
/// waypoint that names the ship it is already attacking.
pub(crate) fn cleared_nav_waypoint_anchor(
    waypoint: Option<&crate::console::navigation::NavigationWaypoint>,
    clearance: Option<&HelmWaypointClearance>,
) -> Option<uuid::Uuid> {
    cleared_nav_waypoint(waypoint, clearance)?;
    match waypoint?.mode()? {
        crate::console::navigation::WaypointMode::Anchored { source_uuid, .. } => {
            uuid::Uuid::parse_str(source_uuid).ok()
        }
        crate::console::navigation::WaypointMode::Free { .. } => None,
    }
}

/// This ship's Combat Lock as a UUID, for the Helm to pursue (issue #702/#829).
///
/// The lock is a `String` because it may name an asteroid as well as an entity;
/// the Helm only pursues things with a canonical UUID, and an unparseable id
/// names nobody. Sourced from the frozen viewscreen `combat_lock` (spec §3).
pub(crate) fn helm_weapons_target(combat_lock: Option<&str>) -> Option<uuid::Uuid> {
    combat_lock.and_then(|t| uuid::Uuid::parse_str(t).ok())
}

// ── The helm decision-surface frame (issue #824) ─────────────────────────────
//
// `HelmAiSurfacesFrame` is the single helm decision seam named by issue #824
// and `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §2. It is a
// **derived, read-only** structure rebuilt from scratch on every shared
// AI-helm sim tick by `build_helm_ai_surfaces_frame`, which runs
// `.after(AiTickLabel)` and `.before` all four per-axis systems. The per-axis
// systems consume it via `Res<_>` (immutable by construction), each still
// making its own pure per-axis decision (`operate_helm` / `decide_impulse`, or
// reading the shared hazard surface for lateral thrust, issue #743) — so
// per-axis decision ownership is preserved and the seam never becomes a coarse
// helm controller (#801 constraint).
//
// Why this does not violate the module's recorded owner ruling ("no shared
// cached `HelmDecision`", see the per-axis module note below): the frame
// carries decision *inputs* — the merged world view, the scored-objective
// slice, the resolved destroy target, the cleared nav waypoint — never a
// decision. No axis's output is stored anywhere another axis could read it,
// and nothing persists across ticks (the map is rebuilt wholesale each AI
// tick). What it removes is the 3-4× duplicated `WorldView` rebuild the old
// per-axis systems each performed, and with it the *unenforced*
// identical-inputs invariant: all four axes now observe the same frame
// because there is only one frame.

/// Assemble the helm decision surface once per shared AI-helm sim tick
/// (issue #824). Runs `.after(AiTickLabel)` (the scored objectives and
/// `WorldSnapshot` it reads are written there) and `.before` all four
/// per-axis systems, under the same `run_if(ai_tick_ready)` gate — so
/// whenever an axis system runs, the frame it reads was built this tick.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_helm_ai_surfaces_frame(
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ship_client_config: Option<Res<crate::lobby::server::ShipClientConfigResource>>,
    entity_fallback_q: HelmAiFallbackQuery,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Has<crate::server_app::LocalShip>,
            HelmAiSurfaces,
            // Issue #875: the ship's own alert state, folded onto the frame so
            // every helm policy host seeds the same `posture` this tick.
            Option<&crate::ship::state::ShipRedAlert>,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
    mut frame: ResMut<HelmAiSurfacesFrame>,
) {
    frame.anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();
    frame.ships.clear();

    let snapshot_entities = helm_ai_snapshot_entities(
        world_snapshot.as_deref(),
        runtime.as_deref(),
        &entity_fallback_q,
    );
    let default_registry = crate::ai::faction::FactionRegistry::default();
    let registry = faction_registry
        .as_deref()
        .map(|r| &r.0)
        .unwrap_or(&default_registry);

    for (
        entity,
        sources,
        physics,
        blackboards,
        entity_uuid,
        faction,
        collider,
        helm_section,
        is_local,
        surfaces,
        ship_red_alert,
    ) in ships.iter()
    {
        let red_alert = ship_red_alert.is_some_and(|r| r.0);
        // Build only for ships some helm axis is actually flying: the frame
        // is a decision surface, and a fully human-held helm makes none.
        let any_axis_ai = [
            crate::ship::system_registry::helm_thrust_system_id(),
            crate::ship::system_registry::helm_steering_system_id(),
            crate::ship::system_registry::lateral_thrust_system_id(),
            crate::ship::system_registry::vertical_thrust_system_id(),
            crate::ship::system_registry::helm_impulse_system_id(),
        ]
        .iter()
        .any(|id| sources.0.policy_for(id).operate_ai);
        if !any_axis_ai {
            continue;
        }

        let scored = helm_ai_scored_objectives(blackboards);
        let has_objective = has_helm_objective(&scored);
        if !has_objective {
            // The axis systems still need the entry (to zero their axis /
            // stand impulse down correctly), but none of them reads a view
            // without a live objective — skip the expensive build.
            frame.ships.insert(
                entity,
                HelmAiShipFrame {
                    scored,
                    has_objective,
                    forward_speed: physics.forward_speed,
                    // Seeded on the objective-less path too: posture is a
                    // reading of the ship's own bridge, not of what it has been
                    // ordered to do, and a doctrine that holds a defensive line
                    // with no objective at all still has to know which line.
                    red_alert,
                    ..Default::default()
                },
            );
            continue;
        }

        let visible_view = helm_ai_world_view(
            physics,
            entity_uuid,
            faction,
            collider,
            helm_section,
            blackboards,
            ship_client_config.as_deref(),
            is_local,
            &frame.anchors,
            &snapshot_entities,
        );
        let (merged_view, shared_targets) = helm_shared_target_view(
            visible_view.clone(),
            &snapshot_entities,
            blackboards,
            surfaces.waypoint,
        );
        let destroy_target = helm_destroy_target(&scored, &merged_view, &shared_targets, registry);
        // Issue #874: reduce the hostiles' published arc sectors against this
        // ship's own position, once, here.
        let hostile_arc_exposure = crate::ai::hostile_arc_exposure(&merged_view, registry);

        // Combat Lock from the frozen viewscreen (issue #829).
        let combat_lock = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };

        frame.ships.insert(
            entity,
            HelmAiShipFrame {
                scored,
                has_objective,
                visible_view,
                merged_view,
                destroy_target,
                weapons_target: helm_weapons_target(combat_lock.as_deref()),
                nav_waypoint: cleared_nav_waypoint(surfaces.waypoint, surfaces.clearance),
                nav_waypoint_anchor: cleared_nav_waypoint_anchor(
                    surfaces.waypoint,
                    surfaces.clearance,
                ),
                forward_speed: physics.forward_speed,
                hostile_arc_exposure,
                red_alert,
            },
        );
    }
}

// The per-axis helm AI's private `emit_helm_ai_command` (issue #824 — the
// first of the seven identical copies) is gone: the travel axes now emit
// through the AI host spine's `AiHostEnv::emitter()` (issue #1211, which
// deleted the last per-axis pass-through shim), which itself wraps the shared
// `command_admission::ai_emit::emit_ai_command` seam (issue #738).

pub use phoenix_sim_gameplay::ship::helm_ai::surfaces::*;
