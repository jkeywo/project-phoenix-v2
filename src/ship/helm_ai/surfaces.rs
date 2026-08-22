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
/// `process_coordination_lag` then latches the generation into
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

/// One ship's helm decision surface for this AI tick. See the module note
/// above for what may (inputs) and may not (decisions) live here.
#[derive(Debug, Clone, Default)]
pub(crate) struct HelmAiShipFrame {
    /// This ship's scored objectives from its viewscreen blackboard.
    pub(crate) scored: Vec<crate::core::messages::ScoredObjective>,
    /// `has_helm_objective(&scored)`, precomputed once.
    pub(crate) has_objective: bool,
    /// Radar-gated world view of what the Helm itself can see — no
    /// shared-target merge. Consumed by the lateral-thrust dodge and the
    /// impulse target resolution, exactly the view those axes built for
    /// themselves before #824.
    pub(crate) visible_view: crate::ai::WorldView,
    /// `visible_view` plus explicit console selections
    /// (`helm_shared_target_view`): Tactical's lock, Sensors' science target,
    /// the anchored waypoint source. Consumed by the thrust and steering
    /// decisions.
    pub(crate) merged_view: crate::ai::WorldView,
    /// Helm's resolved target for an active Destroy directive
    /// (`helm_destroy_target` over the merged view).
    pub(crate) destroy_target: Option<uuid::Uuid>,
    /// This ship's Tactical lock as a UUID (`helm_weapons_target`).
    pub(crate) weapons_target: Option<uuid::Uuid>,
    /// The Navigation waypoint this ship is cleared to fly
    /// (`cleared_nav_waypoint`), if any.
    pub(crate) nav_waypoint: Option<[f32; 2]>,
    /// The entity that cleared waypoint is ANCHORED to, when it is anchored to
    /// one (`cleared_nav_waypoint_anchor`) — i.e. *what* the destination is,
    /// not merely where it currently sits. `None` for a `Free` waypoint, which
    /// names a position and nothing else.
    ///
    /// Carried alongside the position because the planner has to tell a
    /// destination order that CONFLICTS with an authored manoeuvre from one
    /// that names the very ship that manoeuvre is attacking — see
    /// `pass_under_navigation_orders` in [`crate::ship::helm_planner`].
    pub(crate) nav_waypoint_anchor: Option<uuid::Uuid>,
    /// `ShipPhysics.forward_speed` at frame-build time.
    pub(crate) forward_speed: f32,
    /// This ship's exposure to hostile weapon arcs (issue #874), reduced ONCE
    /// per shared AI tick by [`crate::ai::hostile_arc_exposure`] over the
    /// merged view. Seeded as three facts by [`seed_hostile_arc_facts`].
    ///
    /// Folded here rather than in each actuator host for the reason the frame
    /// exists at all: the seven hosts listed in the SCOPE note below reducing it
    /// independently is seven chances for the axes to disagree about the same
    /// tick's geometry.
    pub(crate) hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure,
    /// This ship's own red-alert state (issue #875), read once per shared tick
    /// off `ShipRedAlert` and seeded by [`seed_helm_actuator_facts`] as
    /// [`POSTURE_FACT`].
    ///
    /// Folded here for the reason [`Self::hostile_arc_exposure`] is: the seven
    /// policy hosts must agree about the same tick's posture, and a component
    /// read repeated in each of them is seven chances not to.
    ///
    /// `false` for a ship with no `ShipRedAlert` component at all, which is the
    /// same reading as "stood down" — a hull that cannot raise an alert is never
    /// at one.
    pub(crate) red_alert: bool,
}

/// The per-tick helm decision surface, keyed by ship entity. Rebuilt in
/// place (cleared + refilled) by `build_helm_ai_surfaces_frame` on every
/// shared AI-helm sim tick; consumed read-only by the four per-axis systems.
#[derive(Resource, Default)]
pub(crate) struct HelmAiSurfacesFrame {
    /// World anchors, captured once per tick rather than cloned per ship.
    pub(crate) anchors: std::collections::HashMap<String, [f32; 3]>,
    /// Per-ship frames for every `AiHighFidelity` ship with at least one
    /// AI-operated helm axis.
    pub(crate) ships: std::collections::HashMap<Entity, HelmAiShipFrame>,
}

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

/// Call the pure `crate::ai::plan_helm_travel` (renamed from `operate_helm`,
/// issue #745) with this ship's TOML-authored behaviour tuning, returning
/// `(thrust, steering)`.
///
/// The shared `helm_motion_planner` is this function's sole caller; it
/// publishes the resulting decision to the shared motion plan, and the
/// per-axis systems decode only their own axis from that plan (see the
/// module note on `ai_helm_thrust`). Every tunable it passes down — arrival
/// radius, avoidance buffer, avoidance look-ahead, nav-handoff speed — comes
/// from the entity's `[behaviour]` TOML section. The `crate::ai::*` constants
/// below appear only as `unwrap_or` fallbacks for an entity that has no
/// `[behaviour]` section at all; every one of them is the same value the
/// matching serde `default =` fn supplies, so an entity that omits the field
/// and an entity that omits the whole section behave identically.
///
/// Takes everything by shared reference: `plan_helm_travel` has been pure since
/// #702, so calling this twice in a tick (once per axis) is safe by
/// construction rather than by scheduling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn helm_ai_decision(
    world_view: &crate::ai::WorldView,
    scored: &[crate::core::messages::ScoredObjective],
    behaviour_section: Option<&crate::entities::spawner::BehaviourSection>,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai::server::ObjectiveCursors>,
    weapons_target: Option<uuid::Uuid>,
    destroy_target: Option<uuid::Uuid>,
    nav_waypoint: Option<[f32; 2]>,
    forward_speed: f32,
) -> (f32, f32) {
    const NO_CURSORS: &[crate::ai::patrol_cursor::PatrolCursor] = &[];
    crate::ai::plan_helm_travel(
        world_view,
        scored,
        behaviour_section
            .map(|b| b.0.doctrine.as_slice())
            .unwrap_or(&[]),
        anchors,
        cursors.map(|c| c.0.as_slice()).unwrap_or(NO_CURSORS),
        destroy_target.or(weapons_target),
        nav_waypoint,
        // Authored per entity template in TOML (`[behaviour]
        // waypoint_arrival_radius`), same as the cursor evaluator reads —
        // the helm's turn-at-waypoint decision must not disagree with the
        // arrival that fires the scenario trigger.
        behaviour_section
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS),
        behaviour_section
            .map(|b| b.0.avoidance_buffer)
            .unwrap_or(crate::ai::AVOIDANCE_BUFFER),
        behaviour_section
            .map(|b| b.0.avoidance_look_ahead_secs)
            .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS),
        behaviour_section
            .map(|b| b.0.hazard_threat_exponent)
            .unwrap_or(crate::ai::HAZARD_THREAT_EXPONENT),
        forward_speed,
        behaviour_section
            .map(|b| b.0.nav_handoff_speed)
            .unwrap_or(crate::ai::NAV_HANDOFF_SPEED),
    )
}

/// Apply the Weapons→Helm arc-bearing request (issues #677, #767) to `steering`.
///
/// Biases steering to face the requested target so the emitting weapon
/// family's firing arc can bear on it, without disturbing the thrust/range-
/// holding decision `operate_helm` already made.
///
/// The request carries the emitting family's usable ONLINE emitter arcs
/// (facing/arc/effective-range) in `pending.arcs` (issue #767 — was a
/// hard-coded phaser-only `auto_arc` scan before). Cleared — so the bias never
/// outlives the situation that created it, and stays consistent with the
/// emitter that raised it — when ANY of:
///   - the requested entity is no longer visible (destroyed / out of radar);
///   - the target is beyond the range of EVERY carried arc (no yaw helps —
///     this is the AC4 "target leaves range" clear);
///   - the target is already inside some carried arc AND that arc reaches it
///     (the family can fire — the same in-range-and-in-arc geometry the emitter
///     uses to decide whether to ask at all).
///
/// Only while the target is in reach of some arc but no arc bears does it
/// steer.
///
/// ## The leg's consent (issue #918)
///
/// `leg_yields` is the authored answer of the doctrine leg the helm is flying
/// this tick — [`crate::ai::policy::AiPolicy::leg_yields_to_arc_requests`],
/// resolved off the ship's OWN steering machine and nothing else. `false`
/// DECLINES the request: the facing the planner solved stands, and the bow-on
/// tracking solution below is never written.
///
/// Everything else about the request's life is unchanged by a decline —
/// satisfaction and expiry are still evaluated, so a declined request clears
/// when its GEOMETRY stops meaning anything (the target leaves visibility,
/// leaves the range of every carried arc, or a carried arc already bears)
/// rather than sitting stale until the hull happens to change leg. Only the
/// steering write is gated, because only the steering write is the thing a
/// committed heading cannot survive.
///
/// That geometry check reads `pending.arcs` — the emitting family's usable
/// arc set AS OF THE TICK THE REQUEST WAS RAISED — and nothing here re-derives
/// it. If the family that raised the request goes unusable afterwards (a
/// torpedo tube drains its load, a bank is knocked offline), this function
/// still does not notice on its own: `pending.arcs` is a snapshot, not a live
/// read of the family. That hole predates #918, which only widened the window
/// it was visible in — a declining leg let a stale request stand for as long
/// as the leg kept declining, instead of being satisfied by an early turn
/// onto it.
///
/// **Issue #932 closes the hole, but not here.** `tick_weapons_arc_request`
/// (`src/console/weapons/mod.rs`) already recomputes family usability every
/// tick to decide whether to keep asking; it is the one place that can notice
/// the family it last asked FOR has gone unusable, and now does: when the
/// family named by its own debounce state (`WeaponsArcRequestState`) drops out
/// of the qualifying set, it enqueues a channel-3 `ArcBearingWithdraw` for
/// that family over the same bus the original request travelled.
/// `process_coordination_lag` consumes it unconditionally — clearing
/// `PendingArcBearingRequest` is expiry, not a steering decision, so
/// `leg_yields_to_arc_requests` plays no part in it, exactly as satisfaction
/// and the geometry clears above are never gated on it either. A withdrawn
/// request cannot be honoured by a later leg that yields, because there is
/// nothing left in `pending.target` for that leg to read.
///
/// A decline is not a refusal to the *requester*, and nothing about a decline
/// makes the emitter re-raise the request either — `tick_weapons_arc_request`
/// is DEBOUNCED (`src/console/weapons/mod.rs:651-657`): it re-fires only on a
/// change to the family, target, or usable-arc set, never on every tick the
/// same miss persists. What carries the request across ticks is that nothing
/// on a decline clears `PendingArcBearingRequest`: the request the last
/// debounced emission set simply stands, unconsumed, until the hull enters a
/// leg that yields — which then honours it on the very first tick it is
/// current, PROVIDED the emitting family is still usable; #932's withdrawal is
/// what keeps that proviso true.
pub(crate) fn apply_arc_bearing_request(
    steering: &mut f32,
    pending: Option<&mut PendingArcBearingRequest>,
    world_view: &crate::ai::WorldView,
    physics: &ShipPhysics,
    leg_yields: bool,
) {
    let Some(pending) = pending else { return };
    let Some(bearing_uuid) = pending.target else {
        return;
    };
    match world_view.entities.iter().find(|e| e.uuid == bearing_uuid) {
        Some(target_entity) => {
            // Per carried emitter arc, resolve the shared target geometry with
            // the family's own arc + effective range. `in_reach` = the target
            // is within some emitter's range; `can_bear` = some emitter has it
            // in BOTH range and arc (i.e. that emitter could now fire).
            let mut in_reach = false;
            let mut can_bear = false;
            for a in &pending.arcs {
                let g = crate::weapons::phaser::target_geometry(
                    target_entity.position[0],
                    target_entity.position[2],
                    physics.x,
                    physics.z,
                    physics.yaw,
                    a.range,
                    a.facing_deg,
                    a.arc_deg,
                );
                in_reach |= g.in_range;
                can_bear |= g.in_range && g.in_arc;
            }
            // Satisfied (clear) when nothing is in reach — no bearing helps —
            // or when the family can already bear on the target.
            let satisfied = !in_reach || can_bear;

            if satisfied {
                pending.target = None;
            } else if leg_yields {
                let dx = target_entity.position[0] - world_view.entity_pos[0];
                let dz = target_entity.position[2] - world_view.entity_pos[2];
                let dist = (dx * dx + dz * dz).sqrt();
                if dist > 1.0 {
                    *steering = crate::ai::steer_toward(
                        physics.yaw,
                        [dx / dist, dz / dist],
                        crate::ai::PATROL_DEADBAND_RAD,
                        crate::ai::PATROL_FULL_STEER_RAD,
                    );
                }
            }
        }
        None => pending.target = None,
    }
}

/// Resolve the target position from the highest-scored Helm objective.
///
/// Owned by `ai_helm_impulse`, its sole caller since #704. It was
/// `operate_helm_ai`'s helper, shared with `ai_helm_impulse` when #703 extracted
/// that system; deleting the monolith left the helper with one caller rather
/// than none, so it stays a free function here (beside the other shared helm-AI
/// input helpers) rather than being inlined — `ai_helm_impulse` is not its only
/// *conceivable* caller, and the top-objective selection it performs has to stay
/// consistent with the `top_obj` filter at that call site — see
/// `ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective`, which pins
/// the two agreeing.
///
/// Reads exactly the surfaces `operate_helm` reads (issue #702) — the ship's
/// `TacticalRadarSelection` for `Destroy`, the objective's cursor for `Patrol`, the named
/// anchor for `Reach`/`Retreat`. Two answers to "where is the Helm going?" must
/// not diverge, or the ship charges its impulse drive at a point it is not
/// steering toward.
pub(crate) fn resolve_helm_target_position(
    scored: &[crate::core::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai::server::ObjectiveCursors>,
    weapons_target: Option<uuid::Uuid>,
) -> Option<[f32; 3]> {
    use crate::core::messages::{AiDirective, SystemAffinity};
    let top = scored
        .iter()
        .find(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm))?;
    match &top.directive {
        // `Reach` and `Retreat` are the same shape: fly to a named anchor.
        // An unknown or empty anchor resolves to nowhere, exactly as the
        // matching arms of `operate_helm` do.
        AiDirective::Reach { anchor } | AiDirective::Retreat { anchor } => {
            anchors.get(anchor.as_str()).copied()
        }
        // The directive's own `target` is Tactical's input, not the Helm's.
        // `helm_destroy` pursues the `TacticalRadarSelection` that `ai_target_selection`
        // resolved from it, so this must read the same lock or the impulse
        // could aim at the authored target while the helm closes on whoever
        // Tactical actually locked.
        AiDirective::Destroy { .. } => {
            let uuid = weapons_target?;
            world_view
                .entities
                .iter()
                .find(|e| e.uuid == uuid)
                .map(|e| e.position)
        }
        AiDirective::Patrol {
            anchors: waypoints,
            loop_path,
        } => {
            let index = cursors
                .and_then(|c| {
                    c.0.iter()
                        .find(|cursor| cursor.objective_id == top.id)
                        .map(|cursor| cursor.index())
                })
                .unwrap_or(0);
            crate::ai::patrol_cursor::cursor_target(index, waypoints, *loop_path, anchors)
        }
        _ => None,
    }
}

/// Per-ship BOUNDED history of the range to the current travel target
/// (issue #788), and the identity that history was accumulated against.
///
/// ## Why a window rather than another memory slot
///
/// Private memory is a bag of `f64`s, so it can carry running aggregates
/// (`min_range_seen`) but not a *window*. "Has this ship HELD its safe distance"
/// is not a running aggregate: a running minimum never recovers once one bad
/// sample folds into it, so a destroyer that dipped inside the ring once at the
/// start of its recovery could never satisfy a re-entry gate built on one. The
/// answer needs the last N readings and nothing older, which is exactly
/// [`crate::bounded_history::BoundedHistory`].
///
/// ## Why the bound is the point
///
/// A `Vec` that only grows is a leak in a scenario that runs for an hour, and a
/// growing window silently redefines "recently" as the run goes on. The
/// capacity is authored (`safe_distance_window_ticks`) and re-applied every
/// tick, so memory is constant and the window always means the same span of
/// shared AI ticks.
///
/// ## Why it is host-side rather than policy memory
///
/// It is a derived measurement surface, in the same family as
/// [`HelmPassSurface`]: the host folds it, the host reduces it to the single
/// `fact(safe_distance_held)` reading, and the policy makes the decision. Being
/// per-SHIP rather than per-fine-system is correct here for the same reason the
/// fact snapshot is shared: it is a reading of the world, not a private belief,
/// and all three machines must agree about it or they would reach different
/// legs.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmRecoveryHistory {
    /// Recent range readings, oldest first. Capacity is authored.
    pub ranges: crate::bounded_history::BoundedHistory,
    /// A SECOND window over the same reading, with its own authored capacity
    /// (issue #789) — the one the pressed detector measures separation
    /// *progress* across.
    ///
    /// ## Why not reuse [`Self::ranges`]
    ///
    /// The two windows ask different questions of the same measurement and want
    /// independent lengths. `ranges` answers a LEVEL question — "has every
    /// sample stayed past the safe ring" — and its authored capacity
    /// (`safe_distance_window_ticks`) is tuned for "how long counts as a
    /// maintained standoff before re-entry is allowed". `separation` answers a
    /// TREND question — "is this ship actually getting further away" — and its
    /// capacity (`pressed_window_ticks`) is tuned for "how long a failing escape
    /// takes to be obvious". Sharing one window would silently couple the two
    /// tunings: lengthening the re-entry standoff would make the destroyer
    /// slower to notice it is pinned, which is not a trade a designer asked for.
    ///
    /// Same target scope as `ranges`, same fold site, same bound — it is the
    /// [`crate::bounded_history::BoundedHistory`] *type* being reused, not the
    /// instance.
    pub separation: crate::bounded_history::BoundedHistory,
    /// Which target the readings belong to. A target switch clears BOTH windows:
    /// distances held against a ship that is no longer the threat say nothing
    /// about the one that is, and neither does distance opened from it.
    pub target: Option<uuid::Uuid>,
}

/// The host's per-tick, read-only publication of a ship's fly-through pass
/// (issue #883), written by [`ai_policy_state_tick`] and consumed by
/// `helm_motion_planner`.
///
/// ## Why it exists, and why it is not a state machine
///
/// The escape leg has to be expressed as a DESIRED FACING fed through the shared
/// motion planner rather than as a raw steering override, or the #780 hazard
/// contribution would no longer compose onto it (AC3). The planner is therefore
/// the thing that must know which leg is being flown — but the planner may not
/// know authored state NAMES, and it must not re-resolve three policies itself.
///
/// So this component carries the *derived* answer: whether the pass is running,
/// which leg, the frozen heading, and the authored manoeuvre scalars, all
/// resolved once by the host that already holds the policies and their state.
/// It decides nothing; it is the motion-plan surface's sibling, in the same way
/// `HelmMotionPlan` is a derived publication rather than an authority. Every
/// field traces to authored ship data or to host-written private memory.
///
/// ## The one-tick offset
///
/// `ai_policy_state_tick` runs `.after(helm_motion_planner)` (its boost guards
/// read this tick's hazard surface — issue #882), so the planner consumes the
/// surface published on the PREVIOUS AI tick. At the authored AI-helm cadence
/// that delays the heading freeze by one tick and nothing else: the frozen value
/// itself is captured at the transition instant, and it is deterministic at
/// every frame rate because both systems run on the shared `ai_tick_ready`
/// latch (AGENTS.md #7).
///
/// The measured cost on the Harrow is one AI tick of that hull's yaw rate —
/// 0.55 rad/s at 30 Hz, so 1.05 deg, or 0.74 world units of lateral offset —
/// against a 6-unit `closest_approach_hysteresis` and a 260-unit `commit_range`,
/// so it can neither delay nor mis-fire the transition.
///
/// What happens to the residual AFTERWARDS depends on the hull, and the Harrow
/// sits on both sides of the line depending on whether its drive is lit. 1.05
/// deg is 0.018 rad, just *inside* its authored `tracking_deadband_rad` of 0.02,
/// so an unboosted escape's `steer_toward` reads the residual as zero error and
/// commands no correction: the leg flies with a standing offset from the frozen
/// heading rather than converging onto it, which is within the hull's own
/// authored tolerance for "on heading" by definition — the deadband IS that
/// tolerance. While boost is engaged, though, `apply_ship_physics` multiplies
/// the yaw rate by the authored `steering_multiplier` (1.8 since issue #789, for
/// the pressed pivot's sake), so one tick is 1.89 deg — *outside* the deadband —
/// and the planner steers the residual back out instead. Both are correct; the
/// second is simply the case of a hull whose deadband is tighter than its
/// one-tick yaw rate, and it converges rather than drifting.
#[derive(
    Component, Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct HelmPassSurface {
    /// The hull authors at least ONE complete host-flown leg set — the
    /// fly-through pass ([`Self::pass_legs`]), the shield-recovery standoff, or
    /// the combat broadside orbit — AND both travel axes are AI-operated.
    /// `false` means the planner uses the ordinary `plan_helm_travel` arm and
    /// nothing changes.
    ///
    /// Widened by issue #790. Before it, `active` was exactly "the fly-through
    /// pass is authored", because that was the only host-flown leg set there
    /// was; the recovery legs rode along on top of it because the destroyer
    /// happens to author both. A hull that flies a combat orbit and nothing else
    /// has no `approach_speed` and no `escape_speed` to author, and gating its
    /// orbit on them would have made it fly ordinary doctrine travel for ever —
    /// silently, since every other assertion about it would still hold.
    pub active: bool,
    /// The hull authors the fly-through pass's own two throttles
    /// ([`Self::approach_speed`] / [`Self::escape_speed`]), so the planner may
    /// select the INBOUND and RE-ENGAGE legs (issue #790).
    ///
    /// Separate from [`Self::active`] because those two legs are the planner's
    /// *fallback* when no other leg is selected, and a fallback flown on an
    /// unauthored throttle is a ship coasting at zero into a fight. A hull
    /// without them falls back to ordinary doctrine travel instead.
    pub pass_legs: bool,
    /// `true` once the Steering machine's current state answers the `yaw`
    /// channel with `hold_committed_heading` AND [`Self::pass_legs`] holds: the
    /// facing solution is closed and the ship is flying
    /// [`Self::escape_heading_rad`].
    pub escape: bool,
    /// The heading (radians) the host froze at the last committed transition,
    /// read out of the Steering system's own `memory(escape_heading_rad)`.
    pub escape_heading_rad: f32,
    /// Authored throttle fraction for the inbound leg (Engines `param`).
    pub approach_speed: f32,
    /// Authored throttle fraction for the escape leg (Engines `param`).
    pub escape_speed: f32,
    /// Authored steering deadband, radians (Steering `param`).
    pub tracking_deadband_rad: f32,
    /// Authored steering saturation angle, radians (Steering `param`).
    pub tracking_full_steer_rad: f32,
    // ── The shield-recovery standoff (issue #788) ────────────────────────────
    //
    // Three more legs' worth of derived answer, resolved the same way the
    // escape leg is: off the AUTHORED yaw verb, never off a state name. A hull
    // that authors no recovery states leaves every field below at its default
    // and the planner never selects the arm.
    /// `true` once the Steering machine's current state answers the `yaw`
    /// channel with `hold_recovery_orbit` AND the hull authors the full
    /// recovery parameter set: the ship is spiralling onto / holding the safe
    /// ring around [`Self::safe_range`].
    pub recover: bool,
    /// `true` once that channel answers with `pivot_to_reengage`: recovery is
    /// over, the ship is turning back onto the target at
    /// [`Self::reengage_speed`] to begin another pass.
    pub reengage: bool,
    /// The radius to hold, world units: the TARGET's own longest usable
    /// direct-fire range plus this hull's authored `safe_range_margin`.
    ///
    /// Derived per tick from a threat fact about the ship being fought, not
    /// authored as a constant — a destroyer standing off a blaster boat and one
    /// standing off a beam cruiser are not standing off at the same distance.
    pub safe_range: f32,
    /// Which way round the ring: `+1.0` or `-1.0`, read out of the Steering
    /// system's own host-written `memory(orbit_direction)`, drawn once per
    /// recovery from a seeded composite key.
    pub orbit_direction: f32,
    /// Authored throttle fraction flown on the ring (Steering `param`).
    pub orbit_speed: f32,
    /// Authored spiral gain: radians of heading offset per unit of fractional
    /// radial error (Steering `param`).
    pub orbit_spiral_gain: f32,
    /// Authored throttle fraction flown on the re-entry pivot (Steering
    /// `param`). `0.0` cuts thrust for the turn.
    pub reengage_speed: f32,
    // ── The combat broadside orbit (issue #790) ──────────────────────────────
    //
    // A FIFTH leg, resolved exactly like the four above: off the authored yaw
    // verb, never off a state name. It shares [`Self::orbit_direction`] — a ship
    // circles one way at a time, and the slot is written by the same host draw —
    // but nothing else, because a fighting ring and a standoff ring are
    // different numbers chosen for opposite reasons.
    /// `true` once the Steering machine's current state answers the `yaw`
    /// channel with `hold_combat_orbit` AND the hull authors the full
    /// combat-orbit parameter set: the ship is circling its target at
    /// [`Self::combat_orbit_range`] with its broadsides bearing.
    pub combat_orbit: bool,
    /// The fighting radius to hold, world units — an AUTHORED Steering `param`,
    /// not a derivation from the target's reach.
    ///
    /// It is the hull's own weapon envelope expressed as a distance, so it
    /// cannot come from the enemy: a cruiser whose broadsides reach 50 units
    /// wants to be at 50 units whether it is fighting a knife-fighter or a
    /// long-range missile boat. [`Self::safe_range`] is the opposite question and
    /// keeps its own field.
    pub combat_orbit_range: f32,
    /// Authored throttle fraction flown on the combat ring (Steering `param`).
    pub combat_orbit_speed: f32,
    /// Authored spiral gain for the combat ring: radians of heading offset per
    /// unit of *fractional* radial error (Steering `param`).
    pub combat_orbit_spiral_gain: f32,
    // ── The torpedo-opportunity bow hold (issue #791) ────────────────────────
    //
    // A SIXTH leg and a FOURTH independent leg set, resolved exactly like the
    // five above: off the authored yaw verb, never off a state name. It shares
    // nothing with the orbit legs — a bow-on hold has no ring, no radius and no
    // circulation — which is why it carries only its own throttle.
    /// `true` once the Steering machine's current state answers the `yaw`
    /// channel with `hold_torpedo_bearing` AND the hull authors the
    /// torpedo-bearing parameter set: the ship is tracking the target's live
    /// position bow-on while a fixed forward tube lines up.
    pub torpedo_bearing: bool,
    /// Authored throttle fraction flown on the bow-on hold (Steering `param`).
    /// `0.0` cuts thrust for the phase.
    ///
    /// Its own field rather than a reuse of [`Self::reengage_speed`], for the
    /// reason the verbs are distinct: `reengage_speed` is one of the six
    /// shield-recovery scalars and the host declines all six together, so a hull
    /// with no standoff doctrine would have to invent five unrelated numbers to
    /// borrow this one (AGENTS.md #11).
    pub torpedo_bearing_speed: f32,
    // ── The artillery firing position (issue #792) ───────────────────────────
    //
    // A SEVENTH leg and a FIFTH independent leg set, resolved exactly like the
    // six above: off the authored yaw verb, never off a state name. It shares
    // nothing with the ring legs (a held position has no radius and no
    // circulation) and nothing with the bow hold (which tracks the target's live
    // position with no lead), so it carries its own throttle and its own lead
    // speed.
    /// `true` once the Steering machine's current state answers the `yaw`
    /// channel with `hold_artillery_position` AND the hull authors the full
    /// artillery parameter set: the ship is holding translational station and
    /// pivoting its bow onto a predicted intercept.
    pub artillery_hold: bool,
    /// Authored throttle fraction flown while the firing position is held
    /// (Steering `param`). `0.0` holds translational station.
    pub artillery_hold_speed: f32,
    /// The lead speed the intercept is solved at, world units/s: the flight
    /// speed of the hull's longest-reaching blaster bank.
    ///
    /// DERIVED host-side from this ship's OWN armament, not authored — the same
    /// posture [`Self::safe_range`] takes toward the target's reach, and for the
    /// same reason: a second authored copy of a weapon's flight speed is a number
    /// that can silently disagree with the weapon. `0.0` when the hull carries no
    /// blaster bank, which the planner degrades to aiming at the live position.
    pub artillery_lead_speed: f32,
}

/// Derive this tick's [`HelmPassSurface`] from the two travel-axis policies and
/// the Steering machine's committed state (issue #883).
///
/// The leg is read off the AUTHORED yaw verb — `hold_committed_heading` means
/// escape — so this host never learns an authored state name and a designer can
/// call the states anything. Each leg SET goes live only when the hull authors
/// every scalar that set needs, and the surface only goes `active` when at least
/// one of them does AND both travel axes are AI-operated: a partial authoring
/// falls back to ordinary doctrine travel rather than flying a leg with invented
/// numbers (AGENTS.md #11 — there is no default for any of these values anywhere
/// in Rust).
///
/// ## Four independent leg sets (issues #790, #791)
///
/// * the FLY-THROUGH pass (`approach_speed` + `escape_speed`) → `pass_legs`,
/// * the SHIELD-RECOVERY standoff ([`RECOVERY_PARAMS`]) → `recover`/`reengage`,
/// * the COMBAT broadside orbit ([`COMBAT_ORBIT_PARAMS`]) → `combat_orbit`,
/// * the TORPEDO-OPPORTUNITY bow hold ([`TORPEDO_BEARING_PARAMS`]) →
///   `torpedo_bearing`,
/// * the ARTILLERY firing position ([`ARTILLERY_PARAMS`]) → `artillery_hold`
///   (issue #792).
///
/// They are gated separately because a hull may author any one of them without
/// the others: the destroyer authors the first two and none of the rest, the
/// cruiser the third and fourth, the battleship the fifth alone. Only the two
/// steering-response scalars are common to all five — every pure planner arm
/// takes them — so those alone are required unconditionally.
pub(crate) fn build_pass_surface(
    engines_policy: &crate::ai::policy::AiPolicy,
    steering_policy: &crate::ai::policy::AiPolicy,
    steering_state: &crate::ai::policy::AiPolicyRuntimeState,
    sources: &ShipSystemControlSources,
    facts: &crate::world::flags::AiFacts,
    now: f64,
    // The lead speed the artillery hold predicts with, read host-side off this
    // hull's own armament (issue #792). Not an authored param, so it is handed
    // in rather than looked up here — see [`HelmPassSurface::artillery_lead_speed`].
    artillery_lead_speed: f32,
    // The scenario world-flag chain (issue #891 stage 2).
    flags: &[&crate::world::flags::FlagStore],
) -> HelmPassSurface {
    let travel_axes_ai = sources
        .0
        .policy_for(&crate::ship::system_registry::helm_thrust_system_id())
        .operate_ai
        && sources
            .0
            .policy_for(&crate::ship::system_registry::helm_steering_system_id())
            .operate_ai;
    let authored = steering_policy.machine().is_some() && engines_policy.machine().is_some();
    // The two steering-response scalars every pure planner arm takes. Required
    // unconditionally, because no leg can be flown without them.
    let (Some(tracking_deadband_rad), Some(tracking_full_steer_rad)) = (
        steering_policy.params.get(TRACKING_DEADBAND_PARAM),
        steering_policy.params.get(TRACKING_FULL_STEER_PARAM),
    ) else {
        return HelmPassSurface::default();
    };
    if !authored || !travel_axes_ai {
        return HelmPassSurface::default();
    }

    let yaw_verb = resolve_helm_channel(
        steering_policy,
        Some(steering_state),
        crate::entities::config::HELM_YAW_CHANNEL,
        facts,
        now,
        flags,
    );

    // ── The fly-through pass legs (issue #883) ───────────────────────────────
    //
    // Both throttles or neither: the inbound/re-engage legs are the planner's
    // FALLBACK arm, so admitting them on a missing throttle would fly a run-in
    // at zero thrust rather than declining.
    let (pass_legs, approach_speed, escape_speed) = match (
        engines_policy.params.get(APPROACH_SPEED_PARAM),
        engines_policy.params.get(ESCAPE_SPEED_PARAM),
    ) {
        (Some(approach), Some(escape)) => (true, approach as f32, escape as f32),
        _ => (false, 0.0, 0.0),
    };
    let escape =
        pass_legs && yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::HoldCommittedHeading);

    // ── The shield-recovery legs (issue #788) ────────────────────────────────
    //
    // Same rule as the escape leg: the leg is the authored yaw verb, and every
    // scalar it needs is an authored `param` with no Rust default. A hull that
    // authors the verb but omits a param publishes `recover = false` and flies
    // ordinary doctrine travel — the same "decline rather than invent" posture
    // the four params above take for the pass as a whole.
    // All SIX are required, not just the three this surface reads itself — see
    // [`RECOVERY_PARAMS`], which is also what `seed_pressed_facts` gates on.
    let (recover, reengage, orbit_speed, orbit_spiral_gain, reengage_speed) = match (
        recovery_params_authored(&steering_policy.params),
        steering_policy.params.get(ORBIT_SPEED_PARAM),
        steering_policy.params.get(ORBIT_SPIRAL_GAIN_PARAM),
        steering_policy.params.get(REENGAGE_SPEED_PARAM),
    ) {
        (true, Some(orbit_speed), Some(gain), Some(reengage_speed)) => (
            yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit),
            yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::PivotToReengage),
            orbit_speed as f32,
            gain as f32,
            reengage_speed as f32,
        ),
        _ => (false, false, 0.0, 0.0, 0.0),
    };

    // ── The combat broadside orbit (issue #790) ──────────────────────────────
    //
    // Gated independently of the recovery six: a hull may fight a ring without
    // authoring any shield-recovery doctrine at all, and the destroyer authors
    // the recovery set without ever flying a combat ring. All THREE of its own
    // params are required together — see [`COMBAT_ORBIT_PARAMS`].
    let (combat_orbit, combat_orbit_range, combat_orbit_speed, combat_orbit_spiral_gain) = match (
        combat_orbit_params_authored(&steering_policy.params),
        steering_policy.params.get(COMBAT_ORBIT_RANGE_PARAM),
        steering_policy.params.get(COMBAT_ORBIT_SPEED_PARAM),
        steering_policy.params.get(COMBAT_ORBIT_SPIRAL_GAIN_PARAM),
    ) {
        (true, Some(range), Some(speed), Some(gain)) => (
            yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::HoldCombatOrbit),
            range as f32,
            speed as f32,
            gain as f32,
        ),
        _ => (false, 0.0, 0.0, 0.0),
    };

    // ── The torpedo-opportunity bow hold (issue #791) ────────────────────────
    //
    // Gated independently of both ring sets, and of the pass throttles: the
    // opportunity is a thing a hull does *while* flying some other doctrine, and
    // which doctrine that is is none of this arm's business. Its own param is
    // required — see [`TORPEDO_BEARING_PARAMS`] — because the value a hull most
    // often wants is `0.0`, which is indistinguishable from "unauthored" unless
    // the gate asks for the name.
    let (torpedo_bearing, torpedo_bearing_speed) = match (
        torpedo_bearing_params_authored(&steering_policy.params),
        steering_policy.params.get(TORPEDO_BEARING_SPEED_PARAM),
    ) {
        (true, Some(speed)) => (
            yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::HoldTorpedoBearing),
            speed as f32,
        ),
        _ => (false, 0.0),
    };

    // ── The artillery firing position (issue #792) ───────────────────────────
    //
    // Gated independently of every set above: an artillery platform authors no
    // shield-recovery doctrine, no ring and no torpedo tubes, and none of those
    // has any business being a precondition for holding a gun line. All THREE of
    // its own params are required together — see [`ARTILLERY_PARAMS`] — because
    // the throttle a hull most often wants here is `0.0`, which is
    // indistinguishable from "unauthored" unless the gate asks for the name.
    let (artillery_hold, artillery_hold_speed) = match (
        artillery_params_authored(&steering_policy.params),
        steering_policy.params.get(ARTILLERY_HOLD_SPEED_PARAM),
    ) {
        (true, Some(speed)) => (
            yaw_verb == Some(&crate::ai::policy::AiPolicyVerb::HoldArtilleryPosition),
            speed as f32,
        ),
        _ => (false, 0.0),
    };

    // At least ONE leg set has to be fully authored, or there is nothing for the
    // planner to fly and the hull is better served by ordinary doctrine travel.
    // `recover`/`reengage`/`combat_orbit`/`torpedo_bearing`/`artillery_hold` are
    // per-tick verb readings, so the gate is over the PARAM sets rather than over
    // this tick's booleans — a recovery-capable hull must publish `active` while
    // it is still inbound.
    let recovery_legs = recovery_params_authored(&steering_policy.params);
    let combat_orbit_legs = combat_orbit_params_authored(&steering_policy.params);
    let torpedo_bearing_legs = torpedo_bearing_params_authored(&steering_policy.params);
    let artillery_legs = artillery_params_authored(&steering_policy.params);
    if !pass_legs
        && !recovery_legs
        && !combat_orbit_legs
        && !torpedo_bearing_legs
        && !artillery_legs
    {
        return HelmPassSurface::default();
    }

    HelmPassSurface {
        active: true,
        pass_legs,
        escape,
        escape_heading_rad: steering_state
            .memory
            .get(ESCAPE_HEADING_MEMORY)
            .unwrap_or(0.0) as f32,
        approach_speed,
        escape_speed,
        tracking_deadband_rad: tracking_deadband_rad as f32,
        tracking_full_steer_rad: tracking_full_steer_rad as f32,
        recover,
        reengage,
        // The ring the host derived this tick (target reach + authored margin).
        // Zero when there is no target to derive one from, which is exactly when
        // the planner has no orbit centre either.
        safe_range: facts.get(SAFE_RANGE_FACT).unwrap_or(0.0) as f32,
        // `1.0` when the slot has not been written yet. A direction has to be
        // one of two, so this is a structural fallback rather than a gameplay
        // value — and it is unreachable in practice: the slot is written on the
        // tick the recovery state is entered, before any leg reads it.
        orbit_direction: steering_state
            .memory
            .get(ORBIT_DIRECTION_MEMORY)
            .unwrap_or(1.0) as f32,
        orbit_speed,
        orbit_spiral_gain,
        reengage_speed,
        combat_orbit,
        combat_orbit_range,
        combat_orbit_speed,
        combat_orbit_spiral_gain,
        torpedo_bearing,
        torpedo_bearing_speed,
        artillery_hold,
        artillery_hold_speed,
        // Published unconditionally, like `safe_range`: it is a reading of the
        // hull's own armament rather than part of the artillery gate, and a hull
        // with no blaster bank publishes `0.0` — which is exactly when the
        // planner has no flight time to lead by either.
        artillery_lead_speed,
    }
}
