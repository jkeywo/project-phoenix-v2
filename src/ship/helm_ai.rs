use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
#[cfg(test)]
use crate::ship::components::LastHelmInput;
use crate::ship::components::{
    HelmWaypointClearance, ImpulseConfigResource, PendingArcBearingRequest,
    ShipSystemControlSources,
};
#[cfg(test)]
use crate::ship::helm::{ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput};
use crate::ship_state::ShipPhysics;
use crate::simulation::ShipImpulse;

/// The shared fixed-rate AI-helm sim tick (issue #803). One repeating timer
/// gates **all four** per-axis AI helm systems (`ai_helm_thrust`,
/// `ai_helm_steering`, `ai_helm_lateral_thrust`, `ai_helm_impulse`) so the
/// AI's helm decision cadence is decoupled from the frame rate. Production
/// time is rAF-driven — `bridge.rs` installs `WinitSettings` with
/// `UpdateMode::Continuous` for both focused and unfocused — so `Update` runs
/// at the host's display refresh, ~16.7 ms at 60 Hz and ~6.9 ms at 144 Hz.
/// Without this gate the helm AI would recompute once per rendered frame and
/// a 144 Hz host would steer on ~4x fresher data than a 60 Hz one — precisely
/// the nondeterminism PRD #620 (P2P deterministic lockstep) exists to remove.
/// (`WorldSnapshot` itself is rebuilt every frame — see
/// `ai::server::build_world_snapshot` — so the ticks this gate skips would
/// have seen genuinely fresh data, not a recomputation of an identical
/// result.)
///
/// The rate is TOML-authored: `[global] ai_helm_tick_hz` in the world TOML
/// (`GlobalConfig::ai_helm_tick_hz`, serde default 30 Hz — the old
/// `AiLateralThrustTimer` period). The resource is created at plugin build,
/// before any `WorldConfig` exists, so `tick_ai_helm_timer` reconciles the
/// period against the loaded world config on each frame (a cheap
/// duration-equality check that only writes when the authored rate differs).
#[derive(Resource)]
pub(crate) struct AiHelmTickTimer(pub(crate) Timer);

impl Default for AiHelmTickTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            1.0 / crate::entity_config::GlobalConfig::default().ai_helm_tick_hz,
            TimerMode::Repeating,
        ))
    }
}

/// Boolean latch set each frame by `tick_ai_helm_timer` (issue #803).
/// `run_if` conditions must use read-only params, so the timer is advanced by
/// a dedicated system that writes this flag, which the condition then reads —
/// the same shape as `ai::server::AiSnapshotReady`. Initialises to `true` so
/// the very first update always runs the helm AI (before the timer has had a
/// chance to fire).
#[derive(Resource)]
pub(crate) struct AiHelmTickReady(pub(crate) bool);

/// Advance the `AiHelmTickTimer` and set `AiHelmTickReady`. Registered
/// `.after` all four per-axis AI helm systems so the flag is consumed before
/// it is re-armed for the next frame. Only leaves `true` when the timer
/// fires; on frames where it doesn't the flag is explicitly cleared so the
/// gated systems skip their work.
///
/// Also reconciles the timer period against the TOML-authored
/// `[global] ai_helm_tick_hz` once `WorldConfig` exists — the timer resource
/// is created at plugin build, before the world TOML has been parsed.
pub(crate) fn tick_ai_helm_timer(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut timer: ResMut<AiHelmTickTimer>,
    mut ready: ResMut<AiHelmTickReady>,
) {
    if let Some(wc) = world_config.as_deref() {
        let hz = wc.global.ai_helm_tick_hz;
        if hz > 0.0 {
            let configured = std::time::Duration::from_secs_f32(1.0 / hz);
            if timer.0.duration() != configured {
                timer.0.set_duration(configured);
            }
        }
    }
    ready.0 = timer.0.tick(time.delta()).just_finished();
}

/// Read-only run condition for the four per-axis AI helm systems: fires only
/// on shared sim-tick frames (issue #803).
pub(crate) fn ai_helm_tick_ready(ready: Res<AiHelmTickReady>) -> bool {
    ready.0
}

/// True when the AI helm is flying this ship: both stick axes
/// (`helm-thrust` AND `helm-steering`) are AI-operated. The coarse `helm`
/// system this used to gate on was deleted by #801; per Rule 6 the answer
/// derives from the per-axis declarations, never a coarse fallback.
pub(crate) fn helm_axes_operate_ai(sources: &ShipSystemControlSources) -> bool {
    sources
        .0
        .policy_for(&crate::system_registry::helm_thrust_system_id())
        .operate_ai
        && sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai
}

// ── Shared helm-AI decision inputs (issue #701) ───────────────────────────────
//
// The per-axis `ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust`
// / `ai_helm_impulse` all need the same three inputs: the world entity list,
// the entity's scored objectives, and a `WorldView`. These helpers are the
// single implementation of each, so the per-axis systems cannot silently
// drift from the monolith they replace in #704.

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
/// [`NavigationWaypoint`]: crate::navigation_plugin::NavigationWaypoint
/// [`ObjectiveCursors`]: crate::ai_plugin::ObjectiveCursors
///
/// The Combat Lock (who to pursue) is no longer read from a targeting component
/// here — it comes from this ship's frozen viewscreen blackboard
/// (`ViewscreenBlackboard::combat_lock`, issue #829), read in
/// `build_helm_ai_surfaces_frame`.
#[derive(bevy::ecs::query::QueryData)]
pub struct HelmAiSurfaces {
    waypoint: Option<&'static crate::navigation_plugin::NavigationWaypoint>,
    clearance: Option<&'static HelmWaypointClearance>,
    cursors: Option<&'static crate::ai_plugin::ObjectiveCursors>,
}

/// The read-only entity query the helm AI falls back to when `WorldSnapshot`
/// is absent (tests that don't register `AiPlugin`).
type HelmAiFallbackQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static crate::entity_spawner::EntityUuid,
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
fn helm_ai_snapshot_entities(
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
                yaw: Some(transform.rotation.to_euler(EulerRot::YXZ).0),
                radius: collider.map(|c| c.0.radius).unwrap_or(0.0),
                // Ships are movable, dangerous collision hazards; size rating
                // tracks the collision radius (issue #743).
                movable: true,
                dangerous: true,
                size_rating: collider.map(|c| c.0.radius).unwrap_or(0.0),
                ..Default::default()
            }
        })
        .collect()
}

/// Read this entity's scored objectives out of its viewscreen blackboard.
fn helm_ai_scored_objectives(
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Vec<crate::messages::ScoredObjective> {
    match blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
        _ => vec![],
    }
}

/// True when any scored objective is live and Helm-relevant. When false the
/// helm AI has nothing to pursue and zeroes its intent.
fn has_helm_objective(scored: &[crate::messages::ScoredObjective]) -> bool {
    scored
        .iter()
        .any(|o| o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm))
}

/// This ship's damage-scaled helm radar range (issue #674).
///
/// Prefers the live value from the ship's own Helm blackboard entry — which
/// `publish_helm_blackboard` publishes per-entity since #824, so NPCs get the
/// live damage-scaled value too. The static-config fallback remains for ships
/// whose entry has not been published yet (low-LOD ships, and any ship before
/// its first publish); `helm_ai_radar_range_prefers_the_npc_blackboard_entry`
/// pins both sides.
fn helm_ai_radar_range(
    blackboards: &crate::server_app::ShipSystemBlackboards,
    helm_section: Option<&crate::entities::spawner::HelmConsoleSection>,
    ship_client_config: Option<&crate::lobby::server::ShipClientConfigResource>,
    is_local: bool,
) -> f32 {
    let from_blackboard = match blackboards
        .0
        .get(&crate::system_registry::helm_station_key())
    {
        Some(crate::messages::SystemBlackboard::Helm(bb)) if bb.radar_range > 0.0 => {
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
fn helm_ai_world_view(
    physics: &ShipPhysics,
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
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
fn helm_shared_target_view(
    mut world_view: crate::ai::WorldView,
    snapshot_entities: &[crate::ai::AiWorldEntity],
    blackboards: &crate::server_app::ShipSystemBlackboards,
    waypoint: Option<&crate::navigation_plugin::NavigationWaypoint>,
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
    if let Some(crate::messages::SystemBlackboard::Viewscreen(bb)) = blackboards
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        push(bb.combat_lock.clone());
        push(bb.science_target.clone());
    }
    if let Some(crate::navigation_plugin::WaypointMode::Anchored { source_uuid, .. }) =
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
fn helm_destroy_target(
    scored: &[crate::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    shared: &[uuid::Uuid],
    registry: &crate::faction::FactionRegistry,
) -> Option<uuid::Uuid> {
    use crate::messages::{AiDirective, SystemAffinity};
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
            .is_some_and(|e| crate::faction::is_enemy(world_view.self_faction, e.faction, registry))
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
fn cleared_nav_waypoint(
    waypoint: Option<&crate::navigation_plugin::NavigationWaypoint>,
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

/// This ship's Combat Lock as a UUID, for the Helm to pursue (issue #702/#829).
///
/// The lock is a `String` because it may name an asteroid as well as an entity;
/// the Helm only pursues things with a canonical UUID, and an unparseable id
/// names nobody. Sourced from the frozen viewscreen `combat_lock` (spec §3).
fn helm_weapons_target(combat_lock: Option<&str>) -> Option<uuid::Uuid> {
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
    pub(crate) scored: Vec<crate::messages::ScoredObjective>,
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
    /// `ShipPhysics.forward_speed` at frame-build time.
    pub(crate) forward_speed: f32,
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
/// per-axis systems, under the same `run_if(ai_helm_tick_ready)` gate — so
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
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::entities::spawner::ColliderSection>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Has<crate::server_app::LocalShip>,
            HelmAiSurfaces,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
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
    let default_registry = crate::faction::FactionRegistry::default();
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
    ) in ships.iter()
    {
        // Build only for ships some helm axis is actually flying: the frame
        // is a decision surface, and a fully human-held helm makes none.
        let any_axis_ai = [
            crate::system_registry::helm_thrust_system_id(),
            crate::system_registry::helm_steering_system_id(),
            crate::system_registry::lateral_thrust_system_id(),
            crate::system_registry::helm_impulse_system_id(),
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

        // Combat Lock from the frozen viewscreen (issue #829).
        let combat_lock = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
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
                forward_speed: physics.forward_speed,
            },
        );
    }
}

// The per-axis helm AI's private `emit_helm_ai_command` (issue #824 — the
// first of the seven identical copies) is gone: every AI operator now emits
// through `command_admission::ai_emit::emit_ai_command` (issue #738).

/// Call the pure `crate::ai::operate_helm` with this ship's TOML-authored
/// behaviour tuning, returning `(thrust, steering)`.
///
/// Both per-axis systems call this and keep only their own axis (see the
/// module note on `ai_helm_thrust`). Every tunable it passes down — arrival
/// radius, avoidance buffer, avoidance look-ahead, nav-handoff speed — comes
/// from the entity's `[behaviour]` TOML section. The `crate::ai::*` constants
/// below appear only as `unwrap_or` fallbacks for an entity that has no
/// `[behaviour]` section at all; every one of them is the same value the
/// matching serde `default =` fn supplies, so an entity that omits the field
/// and an entity that omits the whole section behave identically.
///
/// Takes everything by shared reference: `operate_helm` has been pure since
/// #702, so calling this twice in a tick (once per axis) is safe by
/// construction rather than by scheduling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn helm_ai_decision(
    world_view: &crate::ai::WorldView,
    scored: &[crate::messages::ScoredObjective],
    behaviour_section: Option<&crate::entities::spawner::BehaviourSection>,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai_plugin::ObjectiveCursors>,
    weapons_target: Option<uuid::Uuid>,
    destroy_target: Option<uuid::Uuid>,
    nav_waypoint: Option<[f32; 2]>,
    forward_speed: f32,
) -> (f32, f32) {
    const NO_CURSORS: &[crate::ai::patrol_cursor::PatrolCursor] = &[];
    crate::ai::operate_helm(
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
        forward_speed,
        behaviour_section
            .map(|b| b.0.nav_handoff_speed)
            .unwrap_or(crate::ai::NAV_HANDOFF_SPEED),
    )
}

/// Apply the Weapons→Helm arc-bearing request (issue #677) to `steering`.
///
/// Biases steering to face the requested target so the phaser firing arc can
/// bear on it, without disturbing the thrust/range-holding decision
/// `operate_helm` already made. Cleared once the requested entity is no
/// longer visible (destroyed or out of radar range), OR once the ship's
/// current facing already brings some bank's arc onto the target — the same
/// `in_arc` check Weapons uses to decide whether to ask at all — so the bias
/// never persists after the request has been satisfied or outlives the
/// situation that created it.
fn apply_arc_bearing_request(
    steering: &mut f32,
    pending: Option<&mut PendingArcBearingRequest>,
    world_view: &crate::ai::WorldView,
    physics: &ShipPhysics,
    combat_config: Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
) {
    let Some(pending) = pending else { return };
    let Some(bearing_uuid) = pending.0 else {
        return;
    };
    match world_view.entities.iter().find(|e| e.uuid == bearing_uuid) {
        Some(target_entity) => {
            let arc_satisfied = combat_config.is_some_and(|cfg| {
                cfg.0.banks.iter().any(|b| {
                    let (rx, ry) = crate::weapons::phaser::ship_local(
                        target_entity.position[0],
                        target_entity.position[2],
                        physics.x,
                        physics.z,
                        physics.yaw,
                    );
                    crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.auto_arc_deg)
                })
            });

            if arc_satisfied {
                pending.0 = None;
            } else {
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
        None => pending.0 = None,
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
fn resolve_helm_target_position(
    scored: &[crate::messages::ScoredObjective],
    world_view: &crate::ai::WorldView,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: Option<&crate::ai_plugin::ObjectiveCursors>,
    weapons_target: Option<uuid::Uuid>,
) -> Option<[f32; 3]> {
    use crate::messages::{AiDirective, SystemAffinity};
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

/// Mark Reach objectives complete once any ship arrives within its
/// TOML-authored `[behaviour] waypoint_arrival_radius` of the objective's
/// anchor (falling back to `WAYPOINT_ARRIVAL_RADIUS` for ships without a
/// behaviour section).
///
/// Runs in `Broadcast` (after `PublishAggregate` so `scored_objectives` is
/// fresh) and only counts ships whose helm system is AI-controlled.
/// Iterates every ship (player + NPC) so any ship pursuing a shared
/// world Reach objective can complete it. The `ObjectiveManagerRes` is a
/// single world-level resource, so multiple ships arriving at the same
/// anchor complete the shared objective once (idempotent complete()).
pub(crate) fn detect_reached_objective_completion(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    objectives: Option<ResMut<crate::world::server::ObjectiveManagerRes>>,
    ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entities::spawner::BehaviourSection>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let Some(mut objectives) = objectives else {
        return;
    };
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    for (sources, physics, blackboards, behaviour_section) in ships.iter() {
        if !helm_axes_operate_ai(sources) {
            continue;
        }

        let arrival_radius = behaviour_section
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS);

        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
            _ => continue,
        };

        for obj in &scored {
            if obj.score <= 0.0 {
                continue;
            }
            let crate::messages::AiDirective::Reach { anchor } = &obj.directive else {
                continue;
            };
            let Some(&target) = anchors.get(anchor.as_str()) else {
                continue;
            };
            let dx = target[0] - physics.x;
            let dz = target[2] - physics.z;
            if (dx * dx + dz * dz).sqrt() < arrival_radius {
                // Guard the tracer on the actual transition so repeated arrivals
                // at a shared anchor (idempotent complete) emit once (issue #841).
                if objectives.0.complete(&obj.snapshot.id) {
                    if let Some(ref mut msgs) = balance_events {
                        msgs.write(crate::balance::BalanceEvent::ObjectiveCompleted {
                            objective_id: obj.snapshot.id.clone(),
                        });
                    }
                }
            }
        }
    }
}

// ── Per-axis helm AI (issues #701, #703, #824) ────────────────────────────────
//
// `ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust` and
// `ai_helm_impulse` are the per-axis helm AI: one decides the throttle, one
// the yaw, one the dodge, one the impulse drive. Each gates on its own axis
// alone:
//
//     if !<own axis>.operate_ai { continue; }
//
// They are the successors to the `operate_helm_ai` monolith (deleted in #704,
// after #800/#703 declared every axis on every shipped hull and removed the
// coarse half of each gate).
//
// **Since #824 no per-axis system writes an intent component.** Each one
// emits its decision as an admitted `SystemControlPayload` — `SetThrust`,
// `SetSteering`, `LateralThrustInput`, `StartImpulseCharge`/`CancelImpulse` —
// into its own ship's per-entity `AdmittedCommands`, through the same
// `validate_and_admit` seam every network command passes (admission symmetry,
// `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §2). The write into
// `AdmittedCommands` is direct and same-tick — deliberately NOT a round-trip
// through the `InboundMessage` queue, which would add a one-tick lag and move
// every NPC trajectory. `process_helm_inputs` then applies the admitted
// payloads to the intent components later in the same tick, for AI and human
// commands alike, with no branching on source downstream of admission.
//
// **Each axis has exactly one decider, and the applier is shared:**
//
//   SetThrust            ← `ai_helm_thrust`         iff T
//   SetSteering          ← `ai_helm_steering`       iff S
//   LateralThrustInput   ← `ai_helm_lateral_thrust` iff L
//   Start/CancelImpulse  ← `ai_helm_impulse`        iff I
//
// (T/S/L/I = the helm-thrust / helm-steering / helm-lateral-thrust /
// helm-impulse `operate_ai` policies.) One decider per axis means Bevy's
// arbitrary intra-set ordering cannot decide the outcome (the #697 failure
// mode) because there is nothing to decide between; the shared applier
// (`process_helm_inputs`) applies whatever admission let through.
//
// **The coarse `helm` policy C is no longer an input to any of this.** It gated
// the monolith and nothing else; with the monolith gone, no helm-AI system reads
// it. That is a load-bearing absence, not an accident: `C` is exactly the
// coarse-fallback channel #800 spent an issue proving dormant, and re-admitting
// it would resurrect the failure mode where an axis is silently driven by
// something other than its own declaration.
// `helm_writers_are_invariant_under_coarse_policy` pins the whole outcome
// invariant under C over every (C, T, S, L, I) combination;
// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` pins it
// end-to-end through a ticking app;
// `shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone` pins it on a
// real hull's control sources.
//
// The corollary is that an axis a hull does not declare is an axis no AI drives.
// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
// undeclared axis resolves to "human-held" and its system stands down; before
// #704 the monolith quietly covered that case. All nine shipped hulls therefore
// declare all four axes — see `shipped_hull_config_drives_the_per_axis_helm_systems`
// and `shipped_hull_config_drives_ai_helm_lateral_thrust`, which pin the
// declarations themselves against the real TOMLs. Adding a hull means declaring
// four axes, not one.
//
// **The decision surface is assembled once, by `build_helm_ai_surfaces_frame`**
// (issue #824 — see the `HelmAiSurfacesFrame` note above). The owner's ruling
// recorded here through #823 said each per-axis system should call the pure
// `operate_helm` itself and keep only its own output, duplicating the
// `WorldView` build per ship per tick, because a shared cached `HelmDecision`
// would re-create the mini-monolith this split exists to remove. #824 keeps
// the load-bearing half of that ruling and retires the duplication: there is
// still **no shared decision** — the frame carries only derived, read-only
// decision *inputs*, rebuilt every AI tick, and each axis still calls its own
// pure decision function (`operate_helm` per axis is pure and cheap; the
// expensive part was always the view build). The identical-inputs invariant
// the old shape left unenforced — both `operate_helm` callers must see the
// same view or the axes disagree — is now true by construction, and
// `all_four_axes_observe_the_same_frame` pins it.
//
// **No shared mutable state** (issue #702). `operate_helm` is a pure function:
// it reads the frame (built from `TacticalRadarSelection`, `NavigationWaypoint` +
// `HelmWaypointClearance`, `ObjectiveCursors`, the scored pool) and returns
// `(thrust, steering)`. The axis systems consume the frame via `Res<_>` —
// immutable by construction — so "did some axis mutate the surface between
// systems?" is not a question anyone has to answer.
//
// **`LastHelmInput` has one writer now.** The per-axis systems no longer
// mirror their fields; `process_helm_inputs` mirrors every applied helm
// payload into the LocalShip's `LastHelmInput` as it applies the intent. The
// pair readers in `SimSet::Physics` (`publish_joystick_to_engines`,
// `operate_helm_engine_ai`, `tick_boost`) are ordered
// `.after(process_helm_inputs)`, so a torn pair — this tick's AI throttle
// beside last tick's stale human steering — cannot be observed;
// `helm_ai_last_input_pair_is_not_torn` pins the result.

/// Per-axis helm AI: throttle. Decides the throttle for ships whose
/// helm-thrust system is AI-operated and emits it as an admitted `SetThrust`
/// into the ship's own `AdmittedCommands` (issues #800, #704, #824) —
/// `process_helm_inputs` applies it to `ThrustInput` later this tick.
///
/// `AiHighFidelity`-scoped: the frame is only built for ships carrying that
/// marker, and the intent components the admitted command lands on only
/// exist there (`lod_ai_ships` inserts/removes them with the marker).
///
/// Consumes the shared `HelmAiSurfacesFrame` (built this tick, see the
/// module note) and keeps only its own axis of the pure `operate_helm`
/// decision.
pub(crate) fn ai_helm_thrust(
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (entity, sources, entity_uuid, ship_config, mut admitted) in ships.iter_mut() {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_thrust_system_id())
            .operate_ai
        {
            continue;
        }
        // Consume the shared desired-motion contract published by the motion
        // planner this tick (issue #741): decode our own axis (forward
        // throttle) from the ship's 3D desired velocity rather than
        // re-deriving the decision here. No plan entry (no AI helm axis / no
        // frame) means nothing to actuate.
        let Some(sp) = plan.ships.get(&entity) else {
            continue;
        };
        let thrust =
            crate::ai::decode_thrust_from_velocity(sp.motion.desired_velocity_local.to_array());

        emit_ai_command(
            entity_uuid,
            crate::system_registry::helm_thrust_system_id(),
            crate::messages::SystemControlPayload::SetThrust { value: thrust },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

/// Per-axis helm AI: steering. Decides the yaw for ships whose helm-steering
/// system is AI-operated and emits it as an admitted `SetSteering` into the
/// ship's own `AdmittedCommands` (issues #800, #704, #824); it owns the
/// arc-bearing step outright.
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, which resolves the top-scored Helm-relevant
/// directive. That includes the **Retreat consumer** (issue #688): when
/// `AiDirective::Retreat` is the top-scored directive, `operate_helm`'s Retreat
/// arm resolves its named anchor and steers toward it.
/// `ai_helm_steering_retreats_toward_anchor` pins that behaviour through this
/// system, and `ai_helm_steering_retreat_with_unknown_anchor_falls_through`
/// pins the other side of it.
pub(crate) fn ai_helm_steering(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&mut PendingArcBearingRequest>,
            Option<&crate::weapons_plugin::PhaserCombatConfigResource>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        physics,
        entity_uuid,
        ship_config,
        mut pending_bearing,
        combat_config_opt,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai
        {
            continue;
        }
        // Base steering comes from the shared desired-motion contract published
        // by the motion planner this tick (issue #741): decode the yaw intent
        // from the ship's 3D desired facing. Arc-bearing (issue #677) is a
        // facing override this axis still owns, applied on top and resolved
        // against the frame's merged view.
        let (Some(sp), Some(sf)) = (plan.ships.get(&entity), frame.ships.get(&entity)) else {
            continue;
        };

        let mut steering =
            crate::ai::decode_steering_from_facing(sp.motion.desired_facing_local.to_array());

        // ── Weapons->Helm arc-bearing request (issue #677) ───────────────
        // Gated on a live objective, matching the pre-#741 shape: with nothing
        // to pursue the ship holds its facing rather than turning to bear.
        if sf.has_objective {
            apply_arc_bearing_request(
                &mut steering,
                pending_bearing.as_deref_mut(),
                &sf.merged_view,
                physics,
                combat_config_opt,
            );
        }

        emit_ai_command(
            entity_uuid,
            crate::system_registry::helm_steering_system_id(),
            crate::messages::SystemControlPayload::SetSteering { value: steering },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

/// Per-axis helm AI: impulse drive. Decides engage/cancel for ships whose
/// helm-impulse system is AI-operated and emits it as an admitted
/// `StartImpulseCharge`/`CancelImpulse` into the ship's own
/// `AdmittedCommands` (issues #703, #704, #824); `process_helm_inputs`
/// applies it to `ImpulseCommand` later this tick, before
/// `apply_helm_commands` consumes the transition.
///
/// **Reads the shared helm surfaces; mutates none of them.** It resolves where
/// the Helm is going via `resolve_helm_target_position`, over the frame's
/// radar-gated `visible_view` — deliberately NOT the merged view, preserving
/// the pre-#824 shape where the impulse decision never saw an out-of-radar
/// shared target — so the drive charges toward a point the Helm can actually
/// see.
///
/// Emits only on an `Engage`/`Cancel` decision, never on `NoChange`:
/// `apply_helm_commands` transitions on `ImpulseCommand` change detection, so
/// an unconditional emission would re-issue `start_charge`/`cancel_charge`
/// every tick.
pub(crate) fn ai_helm_impulse(
    frame: Res<HelmAiSurfacesFrame>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::ai_plugin::ObjectiveCursors>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        physics,
        impulse_comp,
        impulse_cfg,
        behaviour_section,
        entity_uuid,
        ship_config,
        cursors,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_impulse_system_id())
            .operate_ai
        {
            continue;
        }

        // No drive or no per-hull drive config → nothing to command. Matches
        // the monolith, which guards the same pair.
        let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) else {
            continue;
        };

        let Some(sf) = frame.ships.get(&entity) else {
            continue;
        };
        // No Helm objective → emit nothing. The monolith's no-objective
        // branch `continue`s before its impulse block for exactly the same
        // reason: an in-progress charge is not something a lull in objectives
        // should cancel. (Behaviourally a redundant early-out — the
        // top-objective filters below reach the same `continue` — kept
        // because it short-circuits the target resolution and keeps the shape
        // legible against the monolith it replaces.)
        if !sf.has_objective {
            continue;
        }

        // Resolve where the Helm is going, from the same surfaces `operate_helm`
        // reads, over the radar-gated visible view (see the doc comment).
        let Some(target_pos) = resolve_helm_target_position(
            &sf.scored,
            &sf.visible_view,
            &frame.anchors,
            cursors,
            sf.weapons_target,
        ) else {
            continue;
        };

        // Whether the AI may engage impulse at all while pursuing this
        // objective is TOML-authored per doctrine entry
        // (`[[behaviour.doctrine]] use_impulse`); an objective with no matching
        // doctrine entry never engages.
        let top_obj = sf.scored.iter().find(|o| {
            o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
        });
        let use_impulse = top_obj
            .and_then(|obj| {
                behaviour_section.and_then(|b| b.0.doctrine.iter().find(|d| d.id == obj.id))
            })
            .map(|d| d.effective_use_impulse())
            .unwrap_or(false);
        if !use_impulse {
            continue;
        }

        let decision = crate::ai::decide_impulse(&crate::ai::ImpulseDecisionInput {
            pos: [physics.x, physics.z],
            yaw: physics.yaw,
            target_pos,
            phase: impulse.0.phase,
            engage_distance: cfg.engage_distance,
            cancel_distance: cfg.cancel_distance,
            angle_tolerance: crate::ai::IMPULSE_ANGLE_TOLERANCE_RAD,
        });
        let payload = match decision {
            crate::ai::ImpulseDecision::Engage => {
                crate::messages::SystemControlPayload::StartImpulseCharge
            }
            crate::ai::ImpulseDecision::Cancel => {
                crate::messages::SystemControlPayload::CancelImpulse
            }
            crate::ai::ImpulseDecision::NoChange => continue,
        };
        emit_ai_command(
            entity_uuid,
            crate::system_registry::helm_impulse_system_id(),
            payload,
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

// ── Per-axis helm AI: lateral thrust (issues #697, #703, #824) ────────────────
//
// Born in #697 as `operate_lateral_thrust_ai`, a partial-automation system
// gated `L && !C`; #703 collapsed the gate to `L` alone and closed three
// behaviour divergences against the monolith (radar gating, snapshot
// fallback, no-objective zeroing); #704 deleted the monolith, leaving `L`
// the whole story. #824 moved the transport: the dodge is now emitted as an
// admitted `LateralThrustInput` command rather than a direct
// `LateralThrustInput` component write — see the per-axis module note above.
//
// The ~30 Hz cadence predates the split (it was the private
// `AiLateralThrustTimer` until #803) and is load-bearing: production `Update`
// is rAF-driven, so without the shared `run_if(ai_helm_tick_ready)` gate the
// dodge cadence would follow the host's display refresh rate — precisely the
// nondeterminism PRD #620 (P2P deterministic lockstep) exists to remove.
// A skipped frame runs none of the four axis systems, so an axis simply
// holds its last applied intent through the gap and `integrate_ship_physics`
// keeps integrating it.
// `*_runs_on_the_shared_sim_tick_not_per_frame` pins the cadence for each of
// the four systems.

/// Per-axis helm AI: lateral thrust. Decides the dodge for ships whose
/// helm-lateral-thrust system is AI-operated and emits it as an admitted
/// `LateralThrustInput` into the ship's own `AdmittedCommands` (issues #703,
/// #704, #824).
///
/// Since #743 the dodge is no longer re-derived here: it reads the shared
/// hazard assessment the planner published in `HelmMotionPlan` (the ship-level
/// `assess_hazards` surface built from the hull's authored avoidance tuning)
/// and weights its starboard repulsion by this hull's authored
/// `lateral_hazard_sensitivity`. Docking translation still overrides it (issue
/// #742), and the emit → admit → apply arbiter path is unchanged.
pub(crate) fn ai_helm_lateral_thrust(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            // Optional, so it does not filter the iteration set: a ship without
            // a `[behaviour]` section still runs AI lateral thrust, on the
            // `crate::ai::*` fallbacks that match the serde defaults.
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (entity, sources, behaviour_section, entity_uuid, ship_config, mut admitted) in
        ships.iter_mut()
    {
        // Gate on our own system alone (issue #703) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::lateral_thrust_system_id())
            .operate_ai
        {
            continue;
        }
        let Some(sf) = frame.ships.get(&entity) else {
            continue;
        };

        // The ship's plan for this tick: both the docking translation (issue
        // #742) and the shared hazard surface (issue #743) are read off it, so
        // the human and AI paths stay symmetric downstream (the planner is the
        // single writer of both).
        let ship_plan = plan.ships.get(&entity);

        // A docking close manoeuvre (issue #742), when the planner engaged one
        // this tick, owns the lateral axis: its controlled translation is the
        // sanctioned use of lateral thrust, distinct from the avoidance dodge
        // below and from the facing-only arc-bearing request. Read straight off
        // the shared desired-motion contract's `x`.
        let docking_lateral = ship_plan
            .filter(|sp| sp.docking_active)
            .map(|sp| sp.motion.desired_velocity_local.x);

        let lateral = if let Some(docking_lateral) = docking_lateral {
            docking_lateral
        } else if !sf.has_objective {
            // No objectives → zero the dodge rather than latch the last one,
            // matching what the monolith did for the axis.
            0.0
        } else {
            // Horizontal collision avoidance now flows from the shared hazard
            // assessment (issue #743): the planner's `assess_hazards` publishes a
            // ship-local repulsion, and this actuator responds through its own
            // authored `lateral_hazard_sensitivity` rather than re-deriving the
            // projected-collision geometry in a separate helper. The dodge and
            // the yaw agree because both read the one hazard surface the planner
            // built from the hull's authored avoidance tuning.
            // `lateral_thrust_ai_honours_toml_authored_avoidance_buffer` /
            // `..._look_ahead` pin the buffer/look-ahead reaching that surface;
            // `lateral_thrust_ai_responds_to_shared_hazard_surface` pins the
            // sensitivity weighting.
            let sensitivity = behaviour_section
                .map(|b| b.0.lateral_hazard_sensitivity)
                .unwrap_or(crate::ai::LATERAL_HAZARD_SENSITIVITY);
            ship_plan
                .map(|sp| (sp.hazard.hazard_forces.x * sensitivity).clamp(-1.0, 1.0))
                .unwrap_or(0.0)
        };

        emit_ai_command(
            entity_uuid,
            crate::system_registry::lateral_thrust_system_id(),
            crate::messages::SystemControlPayload::LateralThrustInput { lateral },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_source::{ControlSource, ControlSourceResolver};
    use crate::messages::{ClientMessage, InterSystemPayload, InterSystemQueue};
    use crate::ship::components::ShipConfigComponent;
    use crate::ship::components::HELM_AI_MAX_DT_SECS;
    use crate::ship::test_support::*;
    use crate::ship_physics::ShipPhysicsConfig;
    use crate::simulation::Ship;

    /// Lock this ship's Tactical surface onto `uuid` (issue #702).
    ///
    /// The helm pursues `TacticalRadarSelection`; it no longer resolves a `Destroy`
    /// directive's authored name itself. In production `ai_target_selection`
    /// does that resolution (tier 1) and publishes the result here, so a test
    /// that poses a Destroy objective and expects pursuit must supply the lock
    /// that system would have written.
    fn set_ship_weapons_target(app: &mut App, uuid: &str) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut target = entity
            .get_mut::<crate::weapons_plugin::TacticalRadarSelection>()
            .expect("ship must carry TacticalRadarSelection");
        target.0 = Some(uuid.to_string());
    }

    /// Give this ship a Navigation waypoint *and* the Channel-3 clearance to
    /// fly it, as `operate_navigation_ai` → `process_coordination_lag` would
    /// once the order came due (issue #702). Returns the waypoint's generation.
    use crate::navigation_plugin::WaypointMode;

    fn set_cleared_nav_waypoint(app: &mut App, x: f32, z: f32) -> u64 {
        let ship = find_ship_entity(app);
        let generation = {
            let mut entity = app.world_mut().entity_mut(ship);
            let mut waypoint = entity
                .get_mut::<crate::navigation_plugin::NavigationWaypoint>()
                .expect("ship must carry NavigationWaypoint");
            waypoint.set(WaypointMode::Free { x, z });
            waypoint.generation()
        };
        let mut entity = app.world_mut().entity_mut(ship);
        let mut clearance = entity
            .get_mut::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance");
        clearance.0 = Some(generation);
        generation
    }

    // ── #575: player ship AI helm navigation ──────────────────────────────────

    // ── Per-axis helm AI (issue #701) ──────────────────────────────────────

    fn get_impulse_command(app: &mut App) -> crate::impulse::ImpulsePhase {
        app.world_mut()
            .query_filtered::<&ImpulseCommand, With<Ship>>()
            .single(app.world())
            .expect("expected Ship with ImpulseCommand")
            .0
    }

    fn set_impulse_command(app: &mut App, phase: crate::impulse::ImpulsePhase) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ImpulseCommand>()
            .expect("expected ImpulseCommand")
            .0 = phase;
    }

    /// A `[behaviour]` section whose one doctrine entry matches `objective_id`
    /// and permits impulse.
    ///
    /// `use_impulse` is authored explicitly rather than left to
    /// `effective_use_impulse`'s directive-kind default, so these tests pin the
    /// impulse *system* and not that default (which says `false` for Patrol —
    /// the directive some of them use).
    ///
    /// `target_speed`/`maintain_range` are restated because `DoctrineObjective`
    /// derives `Default`, which zeroes them rather than applying their serde
    /// `default =` values; a zero `target_speed` would silently pin the helm's
    /// throttle at 0 alongside whatever the test meant to measure.
    fn impulse_doctrine(objective_id: &str) -> crate::entity_config::BehaviourConfig {
        crate::entity_config::BehaviourConfig {
            doctrine: vec![crate::entity_config::DoctrineObjective {
                id: objective_id.into(),
                use_impulse: Some(true),
                target_speed: 0.8,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// A ship set up for `ai_helm_impulse`: a per-hull impulse config (the
    /// system no-ops without one) and helm-impulse on AI. The coarse helm is
    /// left Human — which until #704 was what kept `operate_helm_ai` from being
    /// the writer of the `ImpulseCommand` these tests measure, and now simply
    /// isolates the axis. The shipped-hull test below exercises the everything-AI
    /// case.
    fn impulse_ai_app(objective: crate::messages::ScoredObjective) -> App {
        let mut app = test_app();
        let objective_id = objective.id.clone();
        set_ship_blackboard_objectives(&mut app, vec![objective]);
        set_behaviour_section(&mut app, impulse_doctrine(&objective_id));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource::default());
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_impulse_system_id(),
            ControlSource::Ai,
        );
        app
    }

    /// Build a `ControlSourceResolver` from a shipped hull's TOML the way the
    /// game does when nobody is driving: parse the file, then set every
    /// *declared* system to `ControlSource::Ai`. That is literally what the NPC
    /// spawn path (`crate::entities::spawner`) does, and what the `Backfill`
    /// rating does to a player hull whose station goes unmanned — the two hull
    /// families reach the same end state, so the same helper serves both.
    ///
    /// Nothing is hand-set, so the resolver reflects exactly what the hull
    /// declares — which is the point of the tests that use it.
    fn resolver_from_shipped_hull(toml_str: &str) -> ControlSourceResolver {
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("shipped hull TOML must parse");
        let ship_config = config
            .ship_config
            .expect("shipped hull must declare [[system]] blocks");
        let mut resolver = ControlSourceResolver::new();
        for system in &ship_config.systems {
            resolver.set(system.id.clone(), ControlSource::Ai);
        }
        resolver
    }

    /// #704's precondition, pinned against every hull the game ships.
    ///
    /// The delete is only behaviour-preserving if every hull declares every axis.
    /// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
    /// *undeclared* axis resolves to "human-held" and its per-axis system stands
    /// down — and until #704 the `operate_helm_ai` monolith silently covered that
    /// case, because it stood down only from axes that were declared *and* AI.
    /// Undeclare an axis after the delete and nothing writes that component at
    /// all: the ship loses the behaviour, quietly, with every test still green.
    ///
    /// That is not hypothetical. When #704 went to delete the monolith, five NPC
    /// hulls declared neither `helm-impulse` nor `helm-lateral-thrust`, and
    /// `alliance_battleship` declared no `helm-lateral-thrust` — so the monolith
    /// was still driving impulse and the avoidance dodge on their behalf, and
    /// deleting it would have removed both. #704 declares them; this test is what
    /// stops the gap re-opening, and it is deliberately a *table over every hull*
    /// rather than one hull per axis, because the previous shipped-hull tests
    /// (`shipped_hull_config_drives_the_per_axis_helm_systems` on `pirate_raider`,
    /// `shipped_hull_config_drives_ai_helm_lateral_thrust` on `alliance_cruiser`)
    /// each pinned one hull and one axis pair, which is exactly how six hulls
    /// drifted without anything going red.
    ///
    /// Reads the shipped TOMLs through the same resolver the game builds, so it
    /// fails on the declaration a hull is actually missing rather than on a
    /// hand-built fixture's idea of one.
    #[test]
    fn every_shipped_hull_declares_every_helm_axis() {
        let hulls: [(&str, &str); 9] = [
            (
                "alliance_battleship",
                include_str!("../../assets/entities/alliance_battleship.toml"),
            ),
            (
                "alliance_courier",
                include_str!("../../assets/entities/alliance_courier.toml"),
            ),
            (
                "alliance_cruiser",
                include_str!("../../assets/entities/alliance_cruiser.toml"),
            ),
            (
                "alliance_destroyer",
                include_str!("../../assets/entities/alliance_destroyer.toml"),
            ),
            (
                "pirate_raider",
                include_str!("../../assets/entities/pirate_raider.toml"),
            ),
            (
                "pirate_raider_reinforcement",
                include_str!("../../assets/entities/pirate_raider_reinforcement.toml"),
            ),
            (
                "ship_harrow_patrol",
                include_str!("../../assets/entities/ship_harrow_patrol.toml"),
            ),
            (
                "ship_harrow_warhawk",
                include_str!("../../assets/entities/ship_harrow_warhawk.toml"),
            ),
            (
                "ship_requiem_courier",
                include_str!("../../assets/entities/ship_requiem_courier.toml"),
            ),
        ];

        let axes: [(&str, crate::messages::SystemId); 4] = [
            (
                "helm-thrust",
                crate::system_registry::helm_thrust_system_id(),
            ),
            (
                "helm-steering",
                crate::system_registry::helm_steering_system_id(),
            ),
            (
                "helm-impulse",
                crate::system_registry::helm_impulse_system_id(),
            ),
            (
                "helm-lateral-thrust",
                crate::system_registry::lateral_thrust_system_id(),
            ),
        ];

        for (hull, toml_str) in hulls {
            let resolver = resolver_from_shipped_hull(toml_str);

            // Sanity (#801): the coarse `helm` system is deleted from every
            // shipped hull — a TOML that still declared it would fail parse
            // (the kind is unregistered), but pin the resolver view too.
            assert!(
                !resolver
                    .policy_for(&crate::messages::SystemId(
                        crate::system_registry::HELM_STATION_ID.to_string()
                    ))
                    .operate_ai,
                "{hull} must NOT declare a coarse `helm` system (#801)"
            );

            for (axis_name, axis_id) in &axes {
                assert!(
                    resolver.policy_for(axis_id).operate_ai,
                    "{hull} does not declare `{axis_name}`. Since #704 deleted \
                     operate_helm_ai there is no coarse fallback: an undeclared axis \
                     resolves to ControlSource::Human, its per-axis system stands down, \
                     and nothing writes that intent component at all — the hull silently \
                     loses the behaviour. Declare it in the hull TOML with the same owner \
                     as the coarse `helm`"
                );
            }
        }
    }

    /// Install a resolver verbatim on every ship, replacing whatever the test
    /// harness set up.
    fn install_control_sources(app: &mut App, resolver: &ControlSourceResolver) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0 = resolver.clone();
        }
    }

    /// AC5 (issue #800), and the coverage gap that let the dormancy ship.
    ///
    /// Every other per-axis test hand-builds its control sources, so all of
    /// them passed while `helm-thrust` / `helm-steering` were declared in
    /// **zero** shipped TOMLs — their policy defaulted to Human, the per-axis
    /// systems never fired in shipped content, and `operate_helm_ai` quietly did
    /// all the work. This test refuses to hand-build: the sources come from a
    /// real shipped hull.
    ///
    /// That the *per-axis* systems produced the intent needed proving while the
    /// monolith was alive, and the proof was its stand-down: this hull declares
    /// every axis and the NPC spawn path backfills each to AI, so
    /// `operate_helm_ai` skipped both writes. Since #704 deleted it the point is
    /// simply structural — a non-zero intent has no other possible writer.
    #[test]
    fn shipped_hull_config_drives_the_per_axis_helm_systems() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../../assets/entities/pirate_raider.toml"));

        // The declaration itself — what #800 adds, and what was missing.
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_thrust_system_id())
                .operate_ai,
            "the shipped hull must declare helm-thrust, or ai_helm_thrust is dormant \
             in shipped content"
        );
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_steering_system_id())
                .operate_ai,
            "the shipped hull must declare helm-steering, or ai_helm_steering is dormant \
             in shipped content"
        );
        // #801: the shipped hull no longer declares a coarse helm at all —
        // the per-axis declarations above are the whole story.

        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "ai_helm_thrust must drive a shipped hull's throttle toward a Reach anchor \
             (since #704 it is the thrust axis's only AI writer)"
        );
        assert!(
            get_steering_input(&mut app).abs() > 0.0,
            "ai_helm_steering must drive a shipped hull's yaw toward a Reach anchor \
             (since #704 it is the steering axis's only AI writer)"
        );
    }

    /// Ported in #704 from `shipped_hull_per_axis_intent_matches_the_coarse_path`,
    /// which pinned the #800 migration on a real hull: the per-axis path had to
    /// reproduce the monolith's intent exactly, so `run(&shipped)` had to equal
    /// `run(&pre_800)`. `pre_800` *is* the monolith path, so the delete removes
    /// the right-hand side of that equality outright.
    ///
    /// Kept, with both terms retained and the question changed from "do these
    /// agree?" to "which of these still drives the ship?". That is the honest
    /// successor: the old test's whole point was that the two paths were
    /// interchangeable on shipped content, and #704's point is that only one of
    /// them exists. Same hull, same resolver, same objective, same measurement.
    ///
    /// The `pre_800` arm is what makes this more than a restatement of
    /// `shipped_hull_config_drives_the_per_axis_helm_systems`: it pins that the
    /// hull's *declarations* are load-bearing. Strip `helm-thrust`/`helm-steering`
    /// back out of `pirate_raider.toml` and the shipped arm keeps passing on a
    /// coarse fallback if one ever returns — this arm would not.
    #[test]
    fn shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone() {
        let anchor = "station-alpha";

        let run = |resolver: &ControlSourceResolver| {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            install_control_sources(&mut app, resolver);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        let shipped =
            resolver_from_shipped_hull(include_str!("../../assets/entities/pirate_raider.toml"));

        // The same hull as it behaved before #800: coarse helm on AI, the two
        // axes undeclared and therefore Human by default.
        let mut pre_800 = shipped.clone();
        pre_800.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Human,
        );
        pre_800.set(
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Human,
        );

        let shipped_intent = run(&shipped);
        assert!(
            shipped_intent.0 > 0.0 && shipped_intent.1.abs() > 0.0,
            "a shipped hull's declared per-axis systems must drive it toward a Reach \
             anchor (got {shipped_intent:?})"
        );
        assert_eq!(
            run(&pre_800),
            (0.0, 0.0),
            "with helm-thrust/helm-steering undeclared the hull's coarse helm is on AI \
             and nothing else is — the shape operate_helm_ai used to serve. Since #704 \
             deleted it that ship must not move: the axis declarations, not the coarse \
             system, are what fly it"
        );
    }

    /// AC3 on the shipped-hull shape: the Weapons->Helm arc-bearing bias (#677)
    /// must survive the move to the per-axis path. Before #800 the bias reached
    /// shipped hulls via `operate_helm_ai`; now `ai_helm_steering` owns steering
    /// there and has to fold it in instead. Nothing else pins that on a real
    /// hull's control sources.
    ///
    /// Note this does *not* pin the monolith's arc-bearing stand-down: both
    /// systems compute the same bias from the same inputs, so calling it twice
    /// is currently unobservable. See the comment at that call site.
    #[test]
    fn shipped_hull_helm_ai_folds_pending_arc_bearing_request_into_steering() {
        let mut app = test_app();
        // Destroy target directly ahead and far away, so the baseline pursuit
        // steering (before any arc-bearing bias) is ~0.
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);

        // A separate hostile well off to starboard is the arc-bearing request
        // target — distinct from the Destroy pursuit target, so any steering
        // bias can only be attributed to the pending request.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(bearing_uuid)));

        // Shipped-hull sources: coarse + both axes on AI.
        let resolver =
            resolver_from_shipped_hull(include_str!("../../assets/entities/pirate_raider.toml"));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.steering.abs() > 0.01,
            "ai_helm_steering owns steering on a shipped hull, so it must be the one to \
             fold in the pending arc-bearing request; operate_helm_ai must not consume \
             the request out from under it. got {last:?}"
        );
    }

    /// AC1: `ai_helm_thrust` writes `ThrustInput` when its own system is
    /// AI-operated and the coarse helm is not.
    #[test]
    fn ai_helm_thrust_writes_thrust_intent() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "ai_helm_thrust must throttle up toward a Reach anchor"
        );
    }

    /// AC1: the fine gate is real — helm-thrust left Human means no AI write,
    /// even with a live Helm objective on the blackboard.
    #[test]
    fn ai_helm_thrust_does_not_write_when_its_system_is_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        // Coarse helm human, helm-thrust left at its Human default.
        set_helm_control_source(&mut app, ControlSource::Human);

        tick(&mut app);

        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "helm-thrust under human control must not be written by ai_helm_thrust"
        );
    }

    /// AC2: `ai_helm_steering` writes `SteeringInput`, steering toward the
    /// selected waypoint. The anchor sits to the right of a ship at the origin
    /// facing yaw 0, so steering must be positive.
    #[test]
    fn ai_helm_steering_writes_steering_intent_toward_waypoint() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "ai_helm_steering must steer toward an anchor off the starboard bow"
        );
    }

    /// AC2: the fine gate is real for the steering axis too.
    #[test]
    fn ai_helm_steering_does_not_write_when_its_system_is_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Human);

        tick(&mut app);

        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "helm-steering under human control must not be written by ai_helm_steering"
        );
    }

    /// The axes are genuinely independent: automating only the throttle must
    /// leave steering alone, which is the whole point of the per-axis split.
    #[test]
    fn per_axis_gates_are_independent() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Human);
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "throttle axis is AI-operated → must be written"
        );
        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "steering axis is still human → must be untouched"
        );
        // The third assertion here used to be a `nav_goal` probe for "did the
        // AiMemory mutation get committed?" — the #701 commit rule's half of
        // this test. #702 made `operate_helm` pure, so there is no commit to
        // observe and no half-dead-AI failure mode to guard: a system that runs
        // computes its axis from the shared surfaces and writes it, full stop.
    }

    /// Regression (issue #701 review, finding 1): `ai_helm_thrust` and
    /// `ai_helm_steering` write one `LastHelmInput` field each, and
    /// `publish_joystick_to_engines` reads both as a pair. Unless it is ordered
    /// after *both* writers it can interleave between them and publish this
    /// tick's AI throttle next to the stale human steering still sitting in
    /// `LastHelmInput` — a torn pair that lands in `HelmEngineBlackboard`, i.e.
    /// on the player's engine gauge. Which half tears is decided by Bevy's
    /// arbitrary intra-set order, so this pins the published pair against the
    /// stale value rather than against a lucky schedule.
    #[test]
    fn helm_ai_last_input_pair_is_not_torn() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        // Off the starboard bow → the AI wants positive thrust AND positive
        // steering, so both differ in sign from the stale human stick below.
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        // Stale human stick, hard astern and hard to port, left over from
        // before the axes were handed to the AI.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: -0.9,
                steering: -0.9,
                lateral: 0.0,
            },
        );

        tick(&mut app);

        let ai_thrust = get_thrust_input(&mut app);
        let ai_steering = get_steering_input(&mut app);
        assert!(
            ai_thrust > 0.0 && ai_steering > 0.0,
            "precondition: the AI must actually want to move, else there is no \
             stale value to tear against; got thrust={ai_thrust} steering={ai_steering}"
        );

        let queue = app.world().resource::<InterSystemQueue>();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
        assert!(
            !msgs.is_empty(),
            "expected a JoystickState message for helm-engine-port"
        );

        for msg in &msgs {
            let InterSystemPayload::JoystickState { thrust, steering } = &msg.payload else {
                panic!("expected JoystickState payload");
            };
            assert_eq!(
                (*thrust, *steering),
                (ai_thrust, ai_steering),
                "published joystick pair must be the AI's whole decision. A \
                 mismatch on one axis only means the pair tore: \
                 publish_joystick_to_engines interleaved between ai_helm_thrust \
                 and ai_helm_steering and picked up the stale human -0.9"
            );
        }
    }

    /// AC3 (Retreat consumer): with a Retreat directive top-scored, steering
    /// must resolve the named anchor and steer toward it. `operate_helm`'s
    /// Retreat arm is what does the work — this pins that `ai_helm_steering`
    /// actually routes it through to `SteeringInput`.
    #[test]
    fn ai_helm_steering_retreats_toward_anchor() {
        let mut app = test_app();
        let anchor = "rally-point";
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 90.0)]);
        // Rally point off the starboard bow → positive steering.
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "Retreat must steer toward the named rally anchor"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "Retreat must also throttle up to actually leave"
        );
    }

    /// AC3 (Retreat consumer, unresolvable case): a Retreat naming an anchor
    /// the world does not declare resolves to nowhere and leaves the ship idle.
    ///
    /// This asserted the opposite until #702: an *empty*-anchor Retreat — which
    /// is what `aggregate_doctrine_blackboards` synthesised below a
    /// `[behaviour] retreat_hull_threshold` — used to fall back to the ship's
    /// `AiMemory.home_position`. Both the injector and `home_position` are gone.
    /// The fallback only ever looked like a safety net: `home_position` was
    /// never seeded in production, so "retreat home" meant "fly to world
    /// origin" on every shipped ship. Retreat is authored doctrine with a real
    /// anchor now (see `assets/entities/pirate_raider.toml`), and an anchor that
    /// resolves to nothing steers nowhere — see
    /// `ai_helm_steering_retreats_toward_anchor` for the resolvable case.
    #[test]
    fn ai_helm_steering_retreat_with_unknown_anchor_does_not_steer() {
        let mut app = test_app();
        // No anchors in the world config → the Retreat cannot resolve, and
        // there is no lower-priority objective to fall through to.
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective("", 90.0)]);
        app.insert_resource(crate::world::config::WorldConfig::default());
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "a Retreat that names nowhere must not steer; the old home_position \
             fallback made this a flight to world origin"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "and must not throttle up either"
        );
    }

    /// A top-scored Retreat wins over a lower-scored Helm objective pointing
    /// the other way, so the ship actually breaks off rather than pressing on.
    ///
    /// The pool is listed descending by score because that is the contract
    /// every producer honours (`score_doctrine_pool` and
    /// `ObjectiveManager::scored_pool_with_boost` both sort before publishing)
    /// and what `operate_helm` consumes — it takes the first Helm-relevant
    /// entry rather than scanning for the maximum.
    #[test]
    fn ai_helm_steering_retreat_outranks_lower_priority_objective() {
        let mut app = test_app();
        let mut cfg = crate::world::config::WorldConfig::default();
        // Rally to starboard, patrol waypoint to port.
        cfg.anchors.insert("rally".into(), [100.0, 0.0, 0.0]);
        cfg.anchors.insert("wp".into(), [-100.0, 0.0, 0.0]);
        app.insert_resource(cfg);
        set_ship_blackboard_objectives(
            &mut app,
            vec![
                retreat_scored_objective("rally", 90.0),
                patrol_scored_objective(vec!["wp"], 10.0),
            ],
        );
        set_per_axis_helm_ai(&mut app);

        tick(&mut app);

        assert!(
            get_steering_input(&mut app) > 0.0,
            "top-scored Retreat must win over the lower-scored patrol waypoint"
        );
    }

    /// AC4: both per-axis systems are `AiHighFidelity`-scoped. A demoted ship
    /// (marker removed) must not be driven by them.
    #[test]
    fn per_axis_helm_ai_is_scoped_to_high_fidelity() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        // Demote: drop the marker, keep the intent components so a write would
        // still be observable if the scoping were missing.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick(&mut app);

        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "ai_helm_thrust must not touch a ship without AiHighFidelity"
        );
        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "ai_helm_steering must not touch a ship without AiHighFidelity"
        );
    }

    // ── Per-axis helm AI: impulse (issue #703) ─────────────────────────────

    /// AC1: `ai_helm_impulse` writes `ImpulseCommand`, gating on helm-impulse
    /// alone. The anchor is dead ahead down -Z at 500 units — past the
    /// 200-unit `engage_distance` and inside the angle tolerance — so the
    /// decision is `Engage`.
    #[test]
    fn ai_helm_impulse_engages_toward_a_distant_target_ahead() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "ai_helm_impulse must command a charge toward a distant anchor dead ahead"
        );
    }

    /// AC1: the gate is real. Identical geometry to the test above, but
    /// helm-impulse is left at its Human default — and the coarse helm is
    /// Human too, so nothing may command the drive.
    #[test]
    fn ai_helm_impulse_does_not_write_when_its_system_is_human() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_impulse_system_id(),
            ControlSource::Human,
        );

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "helm-impulse under human control must not be commanded by ai_helm_impulse"
        );
    }

    /// AC1, the deactivate half: inside `cancel_distance` with a charge already
    /// running, `ai_helm_impulse` must stand the drive down. The command starts
    /// at `Charging`, so `Idle` here is an observed write and not the default.
    #[test]
    fn ai_helm_impulse_cancels_when_the_target_is_close() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        // 20 units out — inside the 40-unit `cancel_distance`.
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
        // `decide_impulse` only cancels from a non-Idle phase.
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must cancel the charge once the target is inside \
             cancel_distance; still Charging means it never wrote"
        );
    }

    /// AC3: `ai_helm_impulse` is `AiHighFidelity`-scoped. The demoted ship keeps
    /// its `ImpulseCommand` here only so a stray write would be observable.
    #[test]
    fn ai_helm_impulse_is_scoped_to_high_fidelity() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must not touch a ship without AiHighFidelity"
        );
    }

    /// A live Helm objective is a precondition: `operate_helm_ai`'s
    /// no-objective branch `continue`d before its impulse block rather than
    /// cancelling, and `ai_helm_impulse` inherited that when #703 extracted it —
    /// a behaviour it now carries alone, the monolith having been deleted in
    /// #704. A lull in objectives is not a reason to drop an in-progress
    /// charge.
    ///
    /// Pins the *behaviour*, not any one line. `ai_helm_impulse` enforces it
    /// three times over — the `has_helm_objective` early-out,
    /// `resolve_helm_target_position`'s top-objective filter, and the `top_obj`
    /// lookup behind `use_impulse` — each carrying the same `score > 0.0 &&
    /// Helm`-relevant predicate. Deleting any one or two of them leaves this
    /// green; only losing all three turns it red. That is a statement about the
    /// implementation being belt-and-braces, not about the test being weak: the
    /// behaviour it asserts (a dead objective must not cancel a live charge) is
    /// the thing that matters, and it is unreachable by any single regression.
    #[test]
    fn ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        // Inside cancel_distance with a charge running: the one geometry where
        // a system that ignored the objective gate would visibly cancel.
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);
        // Same objective, scored dead: `has_helm_objective` requires score > 0.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 0.0)]);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "with no live Helm objective ai_helm_impulse must leave ImpulseCommand \
             untouched, as the monolith does"
        );
    }

    /// `use_impulse` is TOML-authored per doctrine entry (AGENTS.md rule 11):
    /// an objective whose doctrine forbids impulse must not engage it, however
    /// inviting the geometry.
    #[test]
    fn ai_helm_impulse_honours_toml_authored_use_impulse() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        // The same doctrine entry `impulse_ai_app` installs, with the one
        // authored field flipped.
        set_behaviour_section(
            &mut app,
            crate::entity_config::BehaviourConfig {
                doctrine: vec![crate::entity_config::DoctrineObjective {
                    id: "reach-station-alpha".into(),
                    use_impulse: Some(false),
                    target_speed: 0.8,
                    maintain_range: 25.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "[[behaviour.doctrine]] use_impulse = false must veto the engage that \
             ai_helm_impulse_engages_toward_a_distant_target_ahead proves is otherwise \
             reachable from this geometry"
        );
    }

    /// `ai_helm_impulse` must resolve its target from the *same* waypoint the
    /// rest of the helm AI is steering at this tick — one leg further on than the
    /// tick started, because `advance_objective_cursors` (`SimSet::Modifiers`)
    /// runs before this system and has already advanced the cursor off the
    /// waypoint underfoot.
    ///
    /// The name is historical, and so is the failure it guards: this system used
    /// to reach that leg by *replaying* the helm decision on a scratch clone of
    /// `AiMemory`, which only matched the committer's view while the memory was
    /// still pre-commit — hence `.before(operate_helm_ai)`. #702 deleted
    /// `AiMemory` and with it the clone, the replay and the commit; the cursor is
    /// now a read-only surface that cannot move underneath this system at all
    /// (see the registration comment on `ai_helm_impulse`). What is left to pin
    /// is the answer, not the mechanism that reached it.
    ///
    /// The patrol makes the leg observable. wp-a and wp-b both sit on the ship,
    /// so the cursor advances off wp-a during `Modifiers`; wp-c is 500 units dead
    /// ahead.
    ///
    ///   correct → cursor 1 → target wp-b underfoot → inside cancel_distance
    ///             with a charge running → **Cancel**
    ///   broken  → a leg out of step → target wp-c at 500 → far → NoChange →
    ///             command stays `Charging`
    ///
    /// So the correct answer is also the one that performs a write, which keeps
    /// a do-nothing regression from passing this too.
    #[test]
    fn ai_helm_impulse_reads_pre_commit_memory() {
        let mut app = impulse_ai_app(patrol_scored_objective(vec!["wp-a", "wp-b", "wp-c"], 20.0));
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.anchors.insert("wp-a".into(), [0.0, 0.0, 0.0]);
        cfg.anchors.insert("wp-b".into(), [0.0, 0.0, 0.0]);
        cfg.anchors.insert("wp-c".into(), [0.0, 0.0, -500.0]);
        app.insert_resource(cfg);
        set_behaviour_section(&mut app, impulse_doctrine("obj-defend"));
        // Coarse helm on AI, as this test has always run it. (This was once
        // load-bearing: it put `operate_helm_ai` in the tick as the committer
        // this system had to run ahead of. There is no committer now.)
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut state = crate::impulse::ImpulseState::new();
        state.start_charge();
        set_ship_impulse(&mut app, state);
        set_impulse_command(&mut app, crate::impulse::ImpulsePhase::Charging);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "ai_helm_impulse must resolve its target from this tick's advance (wp-b, \
             underfoot) — still Charging means it replayed the decision on memory \
             operate_helm_ai had already committed and skipped a leg to wp-c"
        );
    }

    /// The coverage gap #800 was caught by, applied to impulse: every test above
    /// hand-builds its control sources, so all of them would pass with
    /// `helm-impulse` declared in zero TOMLs. This one refuses to hand-build.
    ///
    /// `alliance_cruiser` declares the coarse helm *and* helm-impulse, so with
    /// the station unmanned the monolith stands down from the impulse decision
    /// and a `Charging` command has nowhere else to come from.
    #[test]
    fn shipped_hull_config_drives_ai_helm_impulse() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../../assets/entities/alliance_cruiser.toml"));
        assert!(
            resolver
                .policy_for(&crate::system_registry::helm_impulse_system_id())
                .operate_ai,
            "the shipped hull must declare helm-impulse, or ai_helm_impulse is dormant \
             in shipped content"
        );
        // #801: the shipped hull no longer declares a coarse helm at all.

        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        install_control_sources(&mut app, &resolver);

        tick(&mut app);

        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Charging,
            "ai_helm_impulse must drive a shipped hull's impulse decision \
             (operate_helm_ai stands down from it here)"
        );
    }

    // ── Per-axis helm AI: lateral thrust (issue #703) ──────────────────────

    /// An obstacle the default avoidance tuning ignores and an authored 60-unit
    /// `avoidance_buffer` treats as a threat (radius 0 + 1 + 60 = 61 > 40), on a
    /// stationary ship so the look-ahead cannot also move. Any nonzero lateral
    /// is this obstacle and nothing else.
    fn lateral_dodge_app() -> App {
        let mut app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0);
        app
    }

    /// AC2, the gate collapse itself. `ai_helm_lateral_thrust`'s old `L && !C`
    /// gate stood the system down whenever the coarse helm was AI, because the
    /// monolith owned `LateralThrustInput` outright in that case. Since #703 the
    /// monolith stands down instead — so if the `!C` half had been left in
    /// place, this configuration would leave the dodge with **no writer at all**
    /// rather than two.
    ///
    /// That asymmetry is what this test exploits: a nonzero lateral proves the
    /// half came off. (It cannot distinguish one writer from two — both compute
    /// the identical dodge from identical inputs — which is what
    /// `helm_writers_are_invariant_under_coarse_policy` is for.)
    #[test]
    fn ai_helm_lateral_thrust_dodges_when_the_coarse_helm_is_also_ai() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick_twice(&mut app);

        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "with helm-lateral-thrust on AI the dodge must be written whatever the \
             coarse helm is doing; zero means the collapsed gate stood the system \
             down and the monolith had already stood down too"
        );
    }

    /// AC3, and a behaviour change #697 declined to make:
    /// `ai_helm_lateral_thrust` is now `AiHighFidelity`-scoped like its two
    /// siblings. The coarse helm stays Human, so the monolith (also scoped)
    /// cannot cover for it.
    #[test]
    fn ai_helm_lateral_thrust_is_scoped_to_high_fidelity() {
        let mut app = lateral_dodge_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>();

        tick_twice(&mut app);

        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "ai_helm_lateral_thrust must not touch a ship without AiHighFidelity"
        );
    }

    /// The monolith zeroes `LateralThrustInput` when no Helm objective is live,
    /// so the shared integrator decelerates the dodge off through the normal
    /// physics curve. #697 `continue`d instead, latching the last dodge forever.
    /// That divergence had to close before the monolith could stand down.
    #[test]
    fn ai_helm_lateral_thrust_zeroes_the_dodge_without_a_helm_objective() {
        let mut app = lateral_dodge_app();
        tick_twice(&mut app);
        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "precondition: the obstacle produces a dodge while an objective is live"
        );

        // Objectives go quiet; the obstacle does not move.
        set_ship_blackboard_objectives(&mut app, vec![]);
        tick(&mut app);

        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "no live Helm objective must zero the dodge, not latch the last one"
        );
    }

    fn set_lateral_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<LateralThrustInput>()
            .expect("ship must carry LateralThrustInput")
            .0 = value;
    }

    /// A sentinel the helm-AI maths can never produce — intents are
    /// normalised to [-1, 1] — so a frame that leaves the sentinel standing
    /// is a frame the probed system did not run on.
    const CADENCE_SENTINEL: f32 = 123.456;

    /// Drive `app` at 10 ms per frame — under the 33.3 ms shared sim-tick
    /// period, i.e. what a 60 Hz rAF-driven host actually does — and count
    /// the frames on which the probed system ran. `arm` re-stamps the probe
    /// before each frame; `ran_this_frame` reads it back after.
    ///
    /// The shared AI-helm sim tick (issue #803) is a real fixed-rate
    /// throttle, not a formality. Production `Update` is rAF-driven:
    /// `server/bridge.rs` installs `WinitSettings` with
    /// `UpdateMode::Continuous` for both focused and unfocused, so a 60 Hz
    /// host frames at ~16.7 ms — under the period — and the helm AI must
    /// recompute on only *some* frames. Without the gate the AI's decision
    /// cadence would follow the host's display refresh rate (a 144 Hz host
    /// deciding on ~4x fresher data than a 60 Hz one), which is exactly the
    /// nondeterminism PRD #620's lockstep has to eliminate. Until #803 only
    /// the lateral axis was throttled (by the private `AiLateralThrustTimer`);
    /// all four per-axis systems now share one cadence, and there is one of
    /// these tests per system.
    fn count_sim_tick_runs(
        app: &mut App,
        mut arm: impl FnMut(&mut App),
        mut ran_this_frame: impl FnMut(&mut App) -> bool,
    ) -> (usize, usize) {
        const FRAME_MS: u64 = 10;
        const TICKS: usize = 12;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(FRAME_MS),
        ));
        let mut ran = 0usize;
        for _ in 0..TICKS {
            arm(app);
            tick(app);
            if ran_this_frame(app) {
                ran += 1;
            }
        }
        (ran, TICKS)
    }

    /// Shared assertions for the four cadence tests. `ran > 0` guards the
    /// probe itself; `ran <= ticks / 2` is the throttle. Over 12 frames x
    /// 10 ms the 33.3 ms timer fires ~3 times, plus the first frame's
    /// `AiHelmTickReady`-initialises-`true` free run (mirroring
    /// `AiSnapshotReady`); `ticks / 2` leaves generous margin while still
    /// failing loudly if the gate goes away.
    fn assert_shared_sim_tick_cadence(system: &str, (ran, ticks): (usize, usize)) {
        assert!(
            ran > 0,
            "precondition: {ticks} frames x 10 ms spans several 33.3 ms periods, so \
             {system} must run at least once — 0 runs means the probe is broken and \
             this test proves nothing about cadence"
        );
        assert!(
            ran <= ticks / 2,
            "the shared AI-helm sim tick must throttle {system}: at 10 ms/frame — \
             under the 33.3 ms period, i.e. what a 60 Hz rAF-driven host actually \
             does — it ran on {ran} of {ticks} frames. Running every frame means the \
             run_if(ai_helm_tick_ready) gate is gone and the decision cadence \
             follows display refresh rate again (PRD #620)"
        );
    }

    fn set_thrust_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ThrustInput>()
            .expect("ship must carry ThrustInput")
            .0 = value;
    }

    fn set_steering_intent(app: &mut App, value: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<SteeringInput>()
            .expect("ship must carry SteeringInput")
            .0 = value;
    }

    /// Since #824 `process_helm_inputs` writes `LateralThrustInput` only when
    /// an admitted `LateralThrustInput` command exists for the ship — and the
    /// only emitter here is `ai_helm_lateral_thrust` itself — so a frame that
    /// clears the sentinel is a frame the AI decided on. (`HelmInputTimer`
    /// and the coarse-AI stand-down this comment used to describe are gone;
    /// the coarse-AI setup is kept purely as the historical fixture shape.)
    #[test]
    fn ai_helm_lateral_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_lateral_intent(app, CADENCE_SENTINEL),
            |app| lateral_intent(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_lateral_thrust", counts);
    }

    /// AC (issue #803): `ai_helm_thrust` used to run once per rendered frame;
    /// it must now run on the shared sim tick. `set_per_axis_helm_ai` puts the
    /// thrust axis on AI, so `process_helm_inputs` skips the axis and this
    /// system is `ThrustInput`'s sole writer — the sentinel can only be
    /// cleared by it.
    #[test]
    fn ai_helm_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_thrust_intent(app, CADENCE_SENTINEL),
            |app| get_thrust_input(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_thrust", counts);
    }

    /// AC (issue #803): `ai_helm_steering` on the shared sim tick — same
    /// isolation argument as the thrust test.
    #[test]
    fn ai_helm_steering_runs_on_the_shared_sim_tick_not_per_frame() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        let counts = count_sim_tick_runs(
            &mut app,
            |app| set_steering_intent(app, CADENCE_SENTINEL),
            |app| get_steering_input(app) != CADENCE_SENTINEL,
        );
        assert_shared_sim_tick_cadence("ai_helm_steering", counts);
    }

    /// AC (issue #803): `ai_helm_impulse` on the shared sim tick.
    /// `ImpulseCommand` is an enum, so the probe is a reset-and-observe
    /// rather than a sentinel: each frame re-arms the drive to `Idle` (both
    /// the command and the `ShipImpulse` phase, so `decide_impulse` sees the
    /// same Engage-able geometry every time — the anchor 500 units dead
    /// ahead, past `engage_distance`); a frame that ends `Charging` is a
    /// frame the system ran on.
    #[test]
    fn ai_helm_impulse_runs_on_the_shared_sim_tick_not_per_frame() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

        let counts = count_sim_tick_runs(
            &mut app,
            |app| {
                set_ship_impulse(app, crate::impulse::ImpulseState::new());
                set_impulse_command(app, crate::impulse::ImpulsePhase::Idle);
            },
            |app| get_impulse_command(app) == crate::impulse::ImpulsePhase::Charging,
        );
        assert_shared_sim_tick_cadence("ai_helm_impulse", counts);
    }

    /// The shared sim-tick rate is TOML-authored (`[global] ai_helm_tick_hz`),
    /// not hardcoded: `tick_ai_helm_timer` must reconcile the timer period
    /// against a loaded `WorldConfig` that authors a different rate. At an
    /// authored 100 Hz the 10 ms frames land exactly on the period, so the
    /// lateral dodge recomputes every frame — where the default 30 Hz gate
    /// (asserted by the cadence tests above) allows at most half.
    #[test]
    fn ai_helm_tick_rate_is_reconfigured_from_world_config() {
        let mut app = lateral_dodge_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.ai_helm_tick_hz = 100.0;
        // `lateral_dodge_app` leaves no WorldConfig installed; the dodge only
        // needs the snapshot obstacle, so the empty-anchor config is inert
        // apart from the authored tick rate.
        app.insert_resource(cfg);

        let (ran, ticks) = count_sim_tick_runs(
            &mut app,
            |app| set_lateral_intent(app, CADENCE_SENTINEL),
            |app| lateral_intent(app) != CADENCE_SENTINEL,
        );
        assert!(
            ran > ticks / 2,
            "with [global] ai_helm_tick_hz = 100 the 10 ms period fires every frame, \
             so the dodge must recompute on (nearly) all of them — {ran} of {ticks} \
             means tick_ai_helm_timer never applied the TOML-authored rate"
        );
    }

    /// The #800 coverage gap, applied to lateral thrust. `alliance_cruiser`
    /// declares the coarse helm *and* helm-lateral-thrust, so an unmanned Helm
    /// puts both on AI — the exact combination the old `!C` half made
    /// unreachable, and the one every hand-built test above misses.
    #[test]
    fn shipped_hull_config_drives_ai_helm_lateral_thrust() {
        let resolver =
            resolver_from_shipped_hull(include_str!("../../assets/entities/alliance_cruiser.toml"));
        assert!(
            resolver
                .policy_for(&crate::system_registry::lateral_thrust_system_id())
                .operate_ai,
            "the shipped hull must declare helm-lateral-thrust, or ai_helm_lateral_thrust \
             is dormant in shipped content"
        );
        // #801: the shipped hull no longer declares a coarse helm at all.

        let mut app = lateral_dodge_app();
        install_control_sources(&mut app, &resolver);

        tick_twice(&mut app);

        assert!(
            lateral_intent(&mut app).abs() > 0.0,
            "ai_helm_lateral_thrust must drive a shipped hull's dodge (since #704 it is \
             the lateral axis's only AI writer)"
        );
    }

    /// Pins the per-axis gate algebra: **the coarse helm policy `C` is not an
    /// input to any intent writer.** Each writer is a function of its own axis
    /// alone — this test sweeps C across all three control sources for every
    /// fixed (T,S,L,I) and demands the whole outcome (every component's
    /// writer) be invariant under it. It also pins the coverage half: each
    /// component is written exactly when its own axis is AI.
    ///
    /// This is a **model** test: it states the gate algebra against the policy
    /// resolver, it does not run the systems. A coarse fallback re-introduced
    /// into `ai_helm_thrust` leaves this test green; what catches that is
    /// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` and
    /// its siblings, which exercise the real systems. Read this test as the
    /// specification and those as the enforcement.
    #[test]
    fn helm_writers_are_invariant_under_coarse_policy() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // #801: "helm" is not a system; seeding it (the C dimension) must
        // have no influence on any writer — which is what this test proves.
        let coarse = crate::messages::SystemId(crate::system_registry::HELM_STATION_ID.to_string());
        let thrust = crate::system_registry::helm_thrust_system_id();
        let steering = crate::system_registry::helm_steering_system_id();
        let lateral = crate::system_registry::lateral_thrust_system_id();
        let impulse = crate::system_registry::helm_impulse_system_id();

        let all = [
            ControlSource::Human,
            ControlSource::Ai,
            ControlSource::Offline,
        ];

        // Every writer decision for one ship in one tick: which system writes
        // each intent component.
        #[derive(Debug, PartialEq, Eq)]
        struct HelmWriters {
            thrust: bool,
            steering: bool,
            lateral: bool,
            impulse: bool,
        }

        let mut saw_all_four_running = false;

        for t in all {
            for s in all {
                for l in all {
                    for i in all {
                        // Sweep the coarse source innermost so that, for one
                        // fixed (T,S,L,I), we can compare the outcome across all
                        // three coarse sources and demand they agree.
                        let mut outcome_per_coarse = Vec::new();

                        for c in all {
                            let mut r = ControlSourceResolver::new();
                            r.set(coarse.clone(), c);
                            r.set(thrust.clone(), t);
                            r.set(steering.clone(), s);
                            r.set(lateral.clone(), l);
                            r.set(impulse.clone(), i);

                            // The gate each system actually applies: its own
                            // system alone (#800 for thrust/steering, #703 for
                            // lateral/impulse). No system reads the coarse
                            // policy — the one that did is gone.
                            let tt = r.policy_for(&thrust).operate_ai;
                            let ss = r.policy_for(&steering).operate_ai;
                            let ll = r.policy_for(&lateral).operate_ai;
                            let ii = r.policy_for(&impulse).operate_ai;

                            let writers = HelmWriters {
                                thrust: tt,
                                steering: ss,
                                lateral: ll,
                                impulse: ii,
                            };

                            // Each component is written exactly when its own
                            // axis is AI — never otherwise (no coarse fallback),
                            // never dropped when it is (no lost writer).
                            for (name, own_axis_is_ai, written) in [
                                ("ThrustInput", tt, writers.thrust),
                                ("SteeringInput", ss, writers.steering),
                                ("LateralThrustInput", ll, writers.lateral),
                                ("ImpulseCommand", ii, writers.impulse),
                            ] {
                                assert_eq!(
                                    written, own_axis_is_ai,
                                    "{name} must be written exactly when its own axis is \
                                     AI-operated: coarse={c:?} thrust={t:?} steering={s:?} \
                                     lateral={l:?} impulse={i:?}"
                                );
                            }

                            if tt && ss && ll && ii {
                                saw_all_four_running = true;
                            }

                            outcome_per_coarse.push(writers);
                        }

                        // The #704 invariant: nothing above depended on `c`.
                        for (idx, other) in outcome_per_coarse.iter().enumerate().skip(1) {
                            assert_eq!(
                                &outcome_per_coarse[0], other,
                                "the coarse helm policy must not influence any helm-AI \
                                 writer — #704 deleted the only system that read it. \
                                 Differed between coarse={:?} and coarse={:?} at \
                                 thrust={t:?} steering={s:?} lateral={l:?} impulse={i:?}",
                                all[0], all[idx]
                            );
                        }
                    }
                }
            }
        }

        // The shipped-hull shape (every axis declared, station backfilled to AI)
        // must be inside the space this test covers — that combination was
        // unreachable under the old per-ship gates and is the whole point of
        // #800, #703 and #704.
        assert!(
            saw_all_four_running,
            "the shipped-hull all-AI combination must be covered"
        );
    }

    /// Ported in #704 from `coarse_helm_ai_result_is_unchanged_by_per_axis_systems`,
    /// which pinned #800's stand-down: with the coarse helm on AI the monolith
    /// owned the write and the per-axis systems stood down, so turning the fine
    /// systems on changed nothing and the two runs were bit-identical.
    ///
    /// Both terms of that equality were the monolith's output, so the delete
    /// removes the property rather than moving it. Kept — same fixture, same two
    /// runs, same measurement — with the assertion inverted, because inverting it
    /// is precisely what #704 does: the coarse system no longer writes anything,
    /// so the two runs must now *differ*, and the difference is the whole delete.
    /// Equality here would now mean either a surviving coarse fallback (both
    /// non-zero) or a dead per-axis path (both zero); the old test could not tell
    /// you about either, and this one fails on both.
    ///
    /// This had an end-to-end companion, `coarse_helm_alone_commits_no_memory`,
    /// pinning that the coarse system wrote no `AiMemory` while this one pins
    /// that it writes no intent. #702 deleted `AiMemory`, so the companion had
    /// nothing left to observe and went with it; "writes no intent" is now the
    /// whole of the property.
    #[test]
    fn coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do() {
        let anchor = "station-alpha";

        let coarse_only = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_coarse_helm_only_ai(&mut app);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        let coarse_plus_fine = {
            let mut app = test_app();
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            tick(&mut app);
            (get_thrust_input(&mut app), get_steering_input(&mut app))
        };

        assert_eq!(
            coarse_only,
            (0.0, 0.0),
            "the coarse helm system has no AI behaviour of its own since #704 deleted \
             operate_helm_ai; on its own it must leave the intent components untouched \
             (non-zero = a coarse fallback has come back)"
        );
        assert!(
            coarse_plus_fine.0 > 0.0 && coarse_plus_fine.1.abs() > 0.0,
            "declaring the axes is what drives the ship now: the per-axis systems must \
             produce the intent the monolith used to (got {coarse_plus_fine:?})"
        );
    }

    fn set_ship_blackboard_objectives(
        app: &mut App,
        objectives: Vec<crate::messages::ScoredObjective>,
    ) {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        let vb = ViewscreenBlackboard {
            scored_objectives: objectives,
            ..Default::default()
        };
        let entry = (
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vb),
        );
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<Ship>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipSystemBlackboards");
        bbs.0.insert(entry.0, entry.1);
    }

    fn world_config_with_anchor(anchor: &str, pos: [f32; 3]) -> crate::world::config::WorldConfig {
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.anchors.insert(anchor.into(), pos);
        cfg
    }

    #[test]
    fn helm_ai_navigates_toward_reach_objective() {
        let mut app = test_app();
        // Place anchor 100 units ahead (positive X) — ship starts at origin.
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Reach anchor; got {last:?}"
        );
    }

    /// AC (issue #741): a ship pursues a test destination through the shared
    /// motion path — the planner publishes a 3D desired-motion contract, the
    /// per-axis AI decode it into admitted actuator input, and the shared
    /// integrator moves the ship — while facing is carried and actuated
    /// separately from travel.
    ///
    /// The anchor sits off the starboard bow, so the ship must simultaneously
    /// throttle up (travel) and yaw toward it (facing). The two are distinct
    /// fields of the published `DesiredMotion`, and the integrator turns yaw
    /// separately from forward travel.
    #[test]
    fn helm_motion_planner_drives_ship_to_destination_with_independent_facing() {
        use crate::ship::helm_planner::HelmMotionPlan;

        let mut app = test_app();
        let anchor = "test-destination";
        // Off the starboard bow of a ship at the origin facing -Z: +X is to the
        // right, so travel wants forward and facing wants to turn to starboard.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, -50.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        // The planner published a 3D desired-motion contract for the ship.
        let ship = find_ship_entity(&mut app);
        let plan = app.world().resource::<HelmMotionPlan>();
        let sp = plan
            .ships
            .get(&ship)
            .copied()
            .expect("planner must publish a desired-motion plan for the AI-helmed ship");
        assert!(
            sp.motion.desired_velocity_local.z < 0.0,
            "desired velocity must be forward (local -Z); got {:?}",
            sp.motion.desired_velocity_local
        );
        assert!(
            sp.motion.desired_facing_local.x > 0.0,
            "desired facing must point to starboard toward the destination; got {:?}",
            sp.motion.desired_facing_local
        );
        // Facing is a separate field from travel — the whole point of the split.
        assert_ne!(
            sp.motion.desired_facing_local, sp.motion.desired_velocity_local,
            "facing must be represented separately from travel"
        );

        // The shared path actuated it: the ship travels and turns.
        for _ in 0..6 {
            tick(&mut app);
        }
        let physics = get_ship_physics(&mut app);
        assert!(
            physics.forward_speed > 0.0,
            "the ship must move forward through the shared actuator path; got {physics:?}"
        );
        assert!(
            physics.yaw > 0.0,
            "the ship must yaw toward the starboard destination, integrated separately \
             from its forward travel; got {physics:?}"
        );
    }

    #[test]
    fn helm_ai_navigates_toward_retreat_objective() {
        let mut app = test_app();
        // Place anchor 100 units ahead (positive X) — ship starts at origin.
        let anchor = "rally-point";
        set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Retreat anchor; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_patrols_from_viewscreen_objective() {
        let mut app = test_app();
        let anchor = "starbase_patrol_east";
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec![anchor], 20.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must apply positive thrust toward Patrol anchor; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_pursues_named_destroy_objective() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        let target_uuid_str = target_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid_str);
        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "AI helm must pursue named Destroy objective target; got {last:?}"
        );
    }

    // ── #674: helm radar gating ─────────────────────────────────────────────

    #[test]
    fn helm_ai_ignores_hostile_beyond_radar_range() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        // Hostile is 100 units away.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        // Radar range (10.0) is far shorter than the hostile's distance (100.0).
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 10.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "hostile beyond helm radar range must not be perceived; pursuit should fall through to idle, got {last:?}"
        );
    }

    #[test]
    fn helm_ai_pursues_hostile_within_radar_range() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        // Hostile is 100 units away.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        let target_uuid_str = target_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
        app.insert_resource(runtime);
        // Radar range (500.0) comfortably covers the hostile's distance (100.0).
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 500.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid_str);
        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "hostile within helm radar range must still be pursued as before; got {last:?}"
        );
    }

    // ── #677: Weapons->Helm arc-bearing request ──────────────────────────────

    #[test]
    fn helm_ai_folds_pending_arc_bearing_request_into_steering() {
        let mut app = test_app();
        // Destroy target directly ahead and far away, so the baseline
        // pursuit steering (before any arc-bearing bias) is ~0.
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        let destroy_uuid_str = destroy_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // A separate hostile well off to starboard is the Weapons arc-bearing
        // request target — distinct from the Destroy pursuit target, so any
        // steering bias can only be attributed to the pending request.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(bearing_uuid)));

        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &destroy_uuid_str);
        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust > 0.0,
            "pending arc-bearing request must not disturb thrust/range-holding; got {last:?}"
        );
        assert!(
            last.steering.abs() > 0.01,
            "pending arc-bearing request must bias steering toward the requested bearing; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_clears_arc_bearing_request_once_facing_already_satisfies_the_arc() {
        let mut app = test_app();
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Bearing contact is directly ahead of the ship's starting facing
        // (yaw=0, forward=-Z) — i.e. the ship is already oriented such that a
        // wide-arc fore bank already bears on it.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(0.0, 0.0, -200.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            PendingArcBearingRequest(Some(bearing_uuid)),
            crate::weapons_plugin::PhaserCombatConfigResource(
                crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 30.0,
                        auto_arc_deg: 30.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                },
            ),
        ));

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.0, None,
            "a request must clear once the ship's own facing already brings a bank's arc onto the target, \
             not persist indefinitely after being satisfied"
        );
    }

    #[test]
    fn helm_ai_clears_arc_bearing_request_when_target_not_visible() {
        let mut app = test_app();
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Pending bearing references an entity that was never spawned — it
        // cannot be visible in the world view.
        let stale_uuid = uuid::Uuid::new_v4();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(stale_uuid)));

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.0, None,
            "a pending request for a no-longer-visible target must be cleared, not stuck forever"
        );
    }

    /// AC2 (issue #742): an arc-bearing request is *facing-only*. It biases
    /// steering to bring a bank onto the target, but must never leak into the
    /// travel axes — no reverse, no lateral drift. The distinction is what keeps
    /// arc-bearing separate from the docking intent, which alone may translate.
    #[test]
    fn arc_bearing_request_never_commands_reverse_or_lateral() {
        use crate::ship::helm_planner::HelmMotionPlan;

        let mut app = test_app();
        // Destroy target directly ahead and far away → baseline steering ~0 and
        // steady forward throttle, so any change is attributable to the request.
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -1000.0),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        let destroy_uuid_str = destroy_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);

        // A hostile well off to starboard is the arc-bearing target.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest(Some(bearing_uuid)));

        set_ship_weapons_target(&mut app, &destroy_uuid_str);
        tick(&mut app);

        // Facing did move (the request was folded into steering)...
        let last = get_last_helm_input(&mut app);
        assert!(
            last.steering.abs() > 0.01,
            "arc-bearing must bias steering toward the requested bearing; got {last:?}"
        );
        // ...but the travel axes are untouched: forward throttle held, no
        // reverse, and crucially no lateral drift.
        assert!(
            last.thrust > 0.0,
            "arc-bearing must not command reverse; thrust must stay forward; got {last:?}"
        );
        assert_eq!(
            last.lateral, 0.0,
            "arc-bearing is facing-only: it must never command lateral thrust; got {last:?}"
        );

        // The shared desired-motion contract confirms it at the source: the
        // planner never marked docking active and never wrote a lateral (`x`)
        // component — arc-bearing lives entirely in the facing field.
        let sp = *app
            .world()
            .resource::<HelmMotionPlan>()
            .ships
            .get(&ship)
            .expect("planner must publish a plan for the AI-helmed ship");
        assert!(
            !sp.docking_active,
            "an arc-bearing request must not engage the docking manoeuvre"
        );
        assert_eq!(
            sp.motion.desired_velocity_local.x, 0.0,
            "arc-bearing must leave the lateral travel component at zero; got {:?}",
            sp.motion.desired_velocity_local
        );
        assert!(
            sp.motion.desired_velocity_local.z < 0.0,
            "arc-bearing must leave forward travel forward (local -Z), not reverse; got {:?}",
            sp.motion.desired_velocity_local
        );
    }

    // ── #742: distinct docking motion intent ─────────────────────────────────

    /// Build an AI-helmed ship with a Destroy objective on a far-ahead target
    /// (so baseline travel is steady forward) plus a spawned dock contact within
    /// radar range at `dock_pos`. Returns the app and the dock's UUID so the
    /// caller can set (or mis-set) the `DockingMotionIntent`.
    fn docking_app(dock_pos: [f32; 3]) -> (App, uuid::Uuid) {
        let mut app = test_app();
        let destroy_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(destroy_uuid.clone()),
            crate::entities::spawner::EntityName("Harrow Destroyer".into()),
            Transform::from_xyz(0.0, 0.0, -4000.0),
        ));
        let dock_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(dock_uuid.to_string()),
            crate::entities::spawner::EntityName("Axiom Station Dock".into()),
            Transform::from_xyz(dock_pos[0], dock_pos[1], dock_pos[2]),
        ));
        let mut runtime = crate::world::server::WorldContentRuntime::default();
        let destroy_uuid_str = destroy_uuid.clone();
        runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
        app.insert_resource(runtime);
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::messages::ShipClientConfig {
                helm_radar_range: 5000.0,
                ..Default::default()
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);
        set_ship_weapons_target(&mut app, &destroy_uuid_str);
        (app, dock_uuid)
    }

    /// AC3 (issue #742): an active docking intent, once the dock is within
    /// engage distance, drives a controlled *translation* — reverse and lateral
    /// — through the shared motion path. These are exactly the motions
    /// arc-bearing (facing-only) must never command, proving the two intents are
    /// distinct.
    #[test]
    fn docking_intent_commands_controlled_reverse_and_lateral() {
        use crate::ship::helm_planner::HelmMotionPlan;

        // Dock 20 units astern (+Z) and to starboard (+X) of a ship at the
        // origin facing -Z — well inside the default 40-unit engage distance.
        let (mut app, dock_uuid) = docking_app([10.0, 0.0, 20.0]);
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_plugin::DockingMotionIntent(Some(dock_uuid)));

        tick(&mut app);

        let sp = *app
            .world()
            .resource::<HelmMotionPlan>()
            .ships
            .get(&ship)
            .expect("planner must publish a plan for the AI-helmed ship");
        assert!(
            sp.docking_active,
            "a dock within engage distance must engage the docking manoeuvre"
        );
        assert!(
            sp.motion.desired_velocity_local.z > 0.0,
            "an astern dock must command controlled reverse (local +Z); got {:?}",
            sp.motion.desired_velocity_local
        );
        assert!(
            sp.motion.desired_velocity_local.x > 0.0,
            "a starboard dock must command starboard lateral translation; got {:?}",
            sp.motion.desired_velocity_local
        );

        // The shared actuator path carried it: reverse thrust and lateral thrust
        // both landed on the ship's admitted inputs.
        let last = get_last_helm_input(&mut app);
        assert!(
            last.thrust < 0.0,
            "docking reverse must reach the thrust actuator as negative thrust; got {last:?}"
        );
        assert!(
            last.lateral.abs() > 0.0,
            "docking lateral must reach the lateral-thrust actuator; got {last:?}"
        );
    }

    /// AC4 (issue #742): a docking intent expires the instant its dock target is
    /// no longer visible — the planner clears it rather than leaving the ship
    /// manoeuvring toward a ghost. Mirrors arc-bearing's target-not-visible
    /// clear.
    #[test]
    fn docking_intent_expires_when_target_not_visible() {
        use crate::ship::helm_planner::HelmMotionPlan;

        // Dock exists but the intent names a UUID that was never spawned.
        let (mut app, _dock_uuid) = docking_app([10.0, 0.0, 20.0]);
        let ghost = uuid::Uuid::new_v4();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_plugin::DockingMotionIntent(Some(ghost)));

        tick(&mut app);

        let intent = app
            .world()
            .get::<crate::ship_plugin::DockingMotionIntent>(ship)
            .expect("ship must carry DockingMotionIntent");
        assert_eq!(
            intent.0, None,
            "a docking intent for a no-longer-visible dock must be cleared, not stuck forever"
        );
        let sp = *app
            .world()
            .resource::<HelmMotionPlan>()
            .ships
            .get(&ship)
            .expect("planner must publish a plan");
        assert!(
            !sp.docking_active,
            "an expired docking intent must not leave the manoeuvre engaged"
        );
    }

    /// AC1 (issue #742): the Helm AI consumes the authoritative Navigation
    /// waypoint through the shared motion path — the planner's DesiredMotion
    /// steers toward it — regardless of whether a human officer or the
    /// Navigation AI wrote it. Both sources converge on the same
    /// `NavigationWaypoint` + `HelmWaypointClearance` latch
    /// (`human_set_nav_waypoint_eventually_clears_and_the_ai_helm_flies_it`
    /// pins the human wire path; `operate_navigation_ai` emits the identical
    /// admitted command), so asserting the planner consumes that latch covers
    /// both origins.
    #[test]
    fn cleared_nav_waypoint_reaches_the_motion_planner_regardless_of_source() {
        use crate::ship::helm_planner::HelmMotionPlan;

        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        // A Helm-relevant objective that cannot resolve, so the only thing left
        // to fly is the Navigation waypoint reaching the planner.
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );
        // Waypoint dead ahead (local -Z) with the clearance latched — the shared
        // state both the human and AI Navigation sources write.
        set_cleared_nav_waypoint(&mut app, 0.0, -900.0);

        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        let sp = *app
            .world()
            .resource::<HelmMotionPlan>()
            .ships
            .get(&ship)
            .expect("planner must publish a plan for the AI-helmed ship");
        assert!(
            sp.motion.desired_velocity_local.z < 0.0,
            "the planner must turn the cleared nav waypoint into forward desired travel; got {:?}",
            sp.motion.desired_velocity_local
        );
    }

    #[test]
    fn helm_ai_does_nothing_when_helm_human() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        // helm stays Human (default)

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "helm AI must not overwrite LastHelmInput when helm is human; got {last:?}"
        );
    }

    #[test]
    fn helm_ai_stays_zero_when_destroy_target_missing() {
        let mut app = test_app();
        // Blackboard has a Destroy directive, but no live entity resolves to it.
        use crate::messages::{
            AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
            SystemAffinity,
        };
        set_ship_blackboard_objectives(
            &mut app,
            vec![ScoredObjective {
                id: "destroy-pirates".into(),
                score: 5.0,
                directive: AiDirective::Destroy {
                    target: "pirate".into(),
                },
                source: ObjectiveSource::Mission,
                relevance: vec![SystemAffinity::Helm],
                snapshot: ObjectiveSnapshot {
                    id: "destroy-pirates".into(),
                    text: "Destroy pirates".into(),
                    mandatory: true,
                    status: ObjectiveStatus::Active,
                    targets: vec![],
                    source: ObjectiveSource::Mission,
                },
            }],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick(&mut app);

        // operate_helm_ai: unresolved Destroy target → zero thrust remains.
        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last,
            LastHelmInput::default(),
            "missing Destroy target means Backfill zero should remain; got {last:?}"
        );
    }

    #[test]
    fn detect_reach_completion_marks_objective_complete() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-alpha";
        // Anchor at origin — ship also starts at origin, so distance == 0.
        // detect_reached_objective_completion reads from ShipSystemBlackboards component.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-alpha",
            "Dock at Alpha",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-alpha");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Completed)
                .unwrap_or(false),
            "Reach objective should be completed when ship is within arrival radius"
        );
    }

    /// Drives the REAL emission seam — `detect_reached_objective_completion`
    /// running in `test_app` — rather than constructing the variant by hand, so
    /// a regression of the `if objectives.0.complete(...)` guard at the site
    /// (issue #841) fails a test. The pure JSON round-trip test constructs the
    /// variant literally and would stay green even if this wiring were deleted;
    /// this is the only guard on the `ObjectiveCompleted` emission itself.
    ///
    /// Two ticks share one cursor: arrival emits exactly one
    /// `ObjectiveCompleted` for the right id, and the second tick — where
    /// `complete()` no longer transitions — emits nothing, pinning the
    /// idempotency guard (deleting `if complete()` would double-emit here).
    #[test]
    fn detect_reach_completion_emits_objective_completed_once() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;
        use bevy::ecs::message::Messages;

        let mut app = test_app();
        // Register the balance sink the emission site writes to. `init_resource`
        // (not `add_message`) so no per-frame double-buffer swap can drop the
        // first-tick message before the second-tick idempotency read.
        app.init_resource::<Messages<crate::balance::BalanceEvent>>();

        let anchor = "dock-alpha";
        // Anchor at origin — the ship also starts at origin (distance == 0).
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-alpha",
            "Dock at Alpha",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        let mut cursor = app
            .world()
            .resource::<Messages<crate::balance::BalanceEvent>>()
            .get_cursor();

        tick(&mut app);

        let messages = app
            .world()
            .resource::<Messages<crate::balance::BalanceEvent>>();
        let first: Vec<&crate::balance::BalanceEvent> = cursor.read(messages).collect();
        assert_eq!(
            first.len(),
            1,
            "arrival must emit exactly one balance event, got {first:?}"
        );
        match first[0] {
            crate::balance::BalanceEvent::ObjectiveCompleted { objective_id } => {
                assert_eq!(objective_id, "reach-dock-alpha");
            }
            other => panic!("expected ObjectiveCompleted, got {other:?}"),
        }

        tick(&mut app);

        let messages = app
            .world()
            .resource::<Messages<crate::balance::BalanceEvent>>();
        let second: Vec<&crate::balance::BalanceEvent> = cursor.read(messages).collect();
        assert!(
            second.is_empty(),
            "re-completing an already-Completed objective must not emit again; got {second:?}"
        );
    }

    // ── Channel-3 Navigation→Helm clearance (issue #702) ──────────────────
    //
    // `cleared_nav_waypoint` is where the Channel-3 lag lives on the read side:
    // the Helm follows the ship's `NavigationWaypoint` only while its
    // `HelmWaypointClearance` names that waypoint's `generation`. These pin the
    // gate itself — deleting the comparison must not be a silent no-op.

    /// The happy path: clearance matches the waypoint's generation, so the Helm
    /// is cleared to fly it.
    #[test]
    fn cleared_nav_waypoint_returns_the_waypoint_when_the_clearance_matches() {
        let waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });
        let clearance = HelmWaypointClearance(Some(waypoint.generation()));

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
            Some([5.0, -7.0])
        );
    }

    /// The lag itself: Navigation has set a *new* waypoint, but the `NavigateTo`
    /// carrying its generation is still in the coordination queue. The Helm must
    /// not fly it yet — it has been given the waypoint but not the order.
    ///
    /// This is why the clearance is a generation rather than a bool: a bool
    /// ("Navigation has spoken") would go true once and wave every subsequent
    /// waypoint straight through, so only the first order would ever be delayed.
    #[test]
    fn cleared_nav_waypoint_withholds_a_waypoint_newer_than_the_clearance() {
        let mut waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });
        // The Helm was cleared for this one, and is flying it.
        let clearance = HelmWaypointClearance(Some(waypoint.generation()));
        assert!(cleared_nav_waypoint(Some(&waypoint), Some(&clearance)).is_some());

        // Navigation now re-tasks the ship. The order has not arrived yet.
        waypoint.set(WaypointMode::Free { x: 900.0, z: 900.0 });

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
            None,
            "a re-tasked waypoint must re-incur the Channel-3 lag; without this \
             every waypoint after the first would be followed instantly"
        );

        // …and once `process_coordination_lag` latches the new generation, it is.
        let caught_up = HelmWaypointClearance(Some(waypoint.generation()));
        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&caught_up)),
            Some([900.0, 900.0])
        );
    }

    /// A ship never cleared for anything follows nothing.
    #[test]
    fn cleared_nav_waypoint_is_none_without_a_clearance() {
        let waypoint = crate::navigation_plugin::NavigationWaypoint::new(WaypointMode::Free {
            x: 5.0,
            z: -7.0,
        });

        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), Some(&HelmWaypointClearance(None))),
            None,
            "never cleared = never followed"
        );
        assert_eq!(
            cleared_nav_waypoint(Some(&waypoint), None),
            None,
            "a ship with no clearance component at all is never cleared"
        );
        assert_eq!(
            cleared_nav_waypoint(None, Some(&HelmWaypointClearance(Some(1)))),
            None,
            "a clearance with no waypoint names nowhere"
        );
    }

    /// Through the real system: an uncleared waypoint does not move the ship,
    /// and the same waypoint does once the clearance lands.
    ///
    /// The unit tests above pin `cleared_nav_waypoint`; this pins that
    /// `ai_helm_thrust` actually consults it rather than reading the waypoint
    /// directly and skipping the lag.
    #[test]
    fn ai_helm_flies_the_nav_waypoint_only_once_cleared() {
        fn app_with_waypoint(clear_it: bool) -> App {
            let mut app = test_app();
            set_helm_control_source(&mut app, ControlSource::Ai);
            // A Helm-relevant objective that cannot resolve, so the only thing
            // left to fly is the Navigation waypoint.
            set_ship_blackboard_objectives(
                &mut app,
                vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
            );
            if clear_it {
                set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
            } else {
                // Waypoint set, order not yet delivered.
                let ship = find_ship_entity(&mut app);
                let mut entity = app.world_mut().entity_mut(ship);
                let mut waypoint = entity
                    .get_mut::<crate::navigation_plugin::NavigationWaypoint>()
                    .expect("ship must carry NavigationWaypoint");
                waypoint.set(WaypointMode::Free { x: 0.0, z: -900.0 });
            }
            tick(&mut app);
            app
        }

        assert_eq!(
            get_thrust_input(&mut app_with_waypoint(false)),
            0.0,
            "the waypoint is set but the Channel-3 order has not been delivered, \
             so the AI helm must not fly it yet"
        );
        assert!(
            get_thrust_input(&mut app_with_waypoint(true)) > 0.0,
            "once process_coordination_lag latches the clearance, the same \
             waypoint must be flown"
        );
    }

    /// Rule-6 symmetry, end to end over the wire: a *human* navigation
    /// officer's admitted `SetNavigationWaypoint` reaches an AI Helm exactly
    /// as an AI-set waypoint does — the same `NavigateTo` clearance, the same
    /// Channel-3 delivery lag, the same `HelmWaypointClearance` latch — and
    /// the AI Helm then flies it.
    ///
    /// Before the fix only `operate_navigation_ai` enqueued the clearance, so
    /// a human-set waypoint sat on the shared `NavigationWaypoint` forever
    /// unfollowed: `cleared_nav_waypoint` withholds any generation the
    /// clearance has not latched, and nothing ever latched one.
    #[test]
    fn human_set_nav_waypoint_eventually_clears_and_the_ai_helm_flies_it() {
        let mut app = test_app();
        // The waypoint write path lives in NavigationPlugin
        // (`handle_navigation_waypoint`); its blackboard publisher needs the
        // client-config resource.
        app.add_plugins(crate::navigation_plugin::NavigationPlugin)
            .init_resource::<crate::lobby::server::ShipClientConfigResource>();

        // A human captain + navigation officer, game started; the Helm
        // station is unmanned and on AI.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push(
            &mut app,
            "navigation",
            ClientMessage::SetReady { ready: true },
        );
        tick(&mut app);

        set_helm_control_source(&mut app, ControlSource::Ai);
        // A Helm-relevant objective that cannot resolve, so the only thing
        // left to fly is the Navigation waypoint (same shape as
        // `ai_helm_flies_the_nav_waypoint_only_once_cleared`).
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );

        // The human sets the waypoint over the wire.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: crate::messages::SystemControlPayload::SetNavigationWaypoint {
                    x: 0.0,
                    z: -900.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        let generation = app
            .world()
            .entity(ship)
            .get::<crate::navigation_plugin::NavigationWaypoint>()
            .expect("ship must carry NavigationWaypoint")
            .generation();
        assert!(
            app.world()
                .entity(ship)
                .get::<crate::navigation_plugin::NavigationWaypoint>()
                .and_then(|w| w.snapshot())
                .is_some(),
            "the admitted SetNavigationWaypoint must set the shared waypoint"
        );
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "the NavigateTo order must still be serving its Channel-3 lag"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "the AI Helm must not fly a waypoint before the clearance lands"
        );

        // Serve the Channel-3 delivery lag (authored per hull; each tick
        // advances the manual clock by 200 ms), plus slack for the tick that
        // enqueues and the tick that delivers.
        let lag_secs = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            q.single(app.world())
                .expect("ship config")
                .0
                .coordination_lag_secs
        };
        let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
        for _ in 0..ticks {
            tick(&mut app);
        }

        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            Some(generation),
            "the human-set waypoint's NavigateTo must latch its generation \
             into the AI Helm's clearance once the lag is served"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "once cleared, the AI Helm must fly the human-set waypoint — \
             rule-6 symmetry with the AI-set path"
        );
    }

    /// Waypoint clearance survives a helm control flip: a waypoint set while
    /// the helm is HUMAN-manned delivers as suppressed/popup (no latch); when
    /// the helm later flips to AI (disconnect → Backfill), the shared issuer
    /// re-issues the `NavigateTo` on the Human→AI edge, the order serves the
    /// normal Channel-3 lag, latches, and the AI helm flies the existing
    /// waypoint — no human re-set required, and no instant latch.
    #[test]
    fn waypoint_set_while_helm_human_is_flown_once_helm_flips_to_ai() {
        let mut app = test_app();
        app.add_plugins(crate::navigation_plugin::NavigationPlugin)
            .init_resource::<crate::lobby::server::ShipClientConfigResource>();

        // A human captain + navigation officer, game started. The helm axes
        // stay on their default Human control for now.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push(
            &mut app,
            "navigation",
            ClientMessage::SetReady { ready: true },
        );
        tick(&mut app);

        // A Helm-relevant objective that cannot resolve, so once the helm is
        // AI the only thing left to fly is the Navigation waypoint.
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );

        // The human sets the waypoint over the wire while the helm is human.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: crate::messages::SystemControlPayload::SetNavigationWaypoint {
                    x: 0.0,
                    z: -900.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        let generation = app
            .world()
            .entity(ship)
            .get::<crate::navigation_plugin::NavigationWaypoint>()
            .expect("ship must carry NavigationWaypoint")
            .generation();

        // Serve well past the delivery lag with the helm still human: the
        // order routes to the human helm (suppress — human sender, human
        // target) and must NOT latch a clearance.
        let lag_secs = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            q.single(app.world())
                .expect("ship config")
                .0
                .coordination_lag_secs
        };
        let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
        for _ in 0..ticks {
            tick(&mut app);
        }
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "an order delivered to a human helm must not latch a clearance"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "no AI helm, no flight — nothing should be driving the thrust axis"
        );

        // The helm flips to AI (the disconnect → Backfill shape).
        set_helm_control_source(&mut app, ControlSource::Ai);

        // The clearance must not latch instantly — the re-issued order still
        // serves the authored Channel-3 delivery lag.
        tick(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            None,
            "the re-issued NavigateTo must serve the delivery lag, not latch instantly"
        );

        // Serve the lag (authored per hull), plus slack for the tick that
        // enqueues and the tick that delivers.
        for _ in 0..ticks {
            tick(&mut app);
        }
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("ship must carry HelmWaypointClearance")
                .0,
            Some(generation),
            "after the helm flips to AI, the re-issued NavigateTo must latch \
             the existing waypoint's generation once the lag is served"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "the AI helm must fly the waypoint that was set while the helm \
             was human — clearance survives the control flip"
        );
    }

    /// Regression (issue #696 review, finding 2): `[behaviour]
    /// waypoint_arrival_radius` is authored per entity template in TOML and
    /// read by the cursor evaluator at every LOD. The high-LOD helm's own
    /// turn-at-waypoint decision must agree with it rather than hardcoding
    /// `WAYPOINT_ARRIVAL_RADIUS` — otherwise a designer's widened radius is
    /// honoured for triggers but ignored for steering.
    ///
    /// Probed through the helm's *steering* rather than through a waypoint
    /// index (issue #702). The helm no longer keeps an index of its own to
    /// look at: `advance_objective_cursors` owns every cursor, and `helm_patrol`
    /// only reads. What the radius still decides here — and all this test ever
    /// really cared about — is the helm's own arrival branch: short of the
    /// radius it turns toward the waypoint; inside it, it flies straight
    /// through. That is directly observable.
    #[test]
    fn high_lod_helm_honours_toml_authored_waypoint_arrival_radius() {
        fn patrol_app(arrival_radius: Option<f32>) -> App {
            let mut app = test_app();
            // wp0 sits 100 units to starboard — inside a 150 radius, outside
            // the default 20.
            let mut cfg = crate::world::config::WorldConfig::default();
            cfg.anchors.insert("wp0".into(), [100.0, 0.0, 0.0]);
            cfg.anchors.insert("wp1".into(), [900.0, 0.0, 0.0]);
            set_ship_blackboard_objectives(
                &mut app,
                vec![patrol_scored_objective(vec!["wp0", "wp1"], 20.0)],
            );
            app.insert_resource(cfg);
            set_helm_control_source(&mut app, ControlSource::Ai);
            if let Some(radius) = arrival_radius {
                let ship = find_ship_entity(&mut app);
                app.world_mut().entity_mut(ship).insert(
                    crate::entities::spawner::BehaviourSection(
                        crate::entity_config::BehaviourConfig {
                            waypoint_arrival_radius: radius,
                            ..Default::default()
                        },
                    ),
                );
            }
            tick(&mut app);
            app
        }

        assert!(
            get_steering_input(&mut patrol_app(None)) > 0.0,
            "with the default arrival radius the helm is still 100 units short of \
             wp0, so it must turn toward it (wp0 is to starboard)"
        );
        assert_eq!(
            get_steering_input(&mut patrol_app(Some(150.0))),
            0.0,
            "a TOML-widened arrival radius must put the high-LOD helm *inside* \
             wp0, so it flies straight through — the same radius, and the same \
             call, the cursor evaluator makes. A hardcoded WAYPOINT_ARRIVAL_RADIUS \
             would still be turning."
        );
    }

    // ── TOML-authored avoidance tuning (AGENTS.md rule 11) ────────────────
    //
    // `[behaviour] avoidance_buffer` / `avoidance_look_ahead_secs` are
    // declared with serde defaults, so a designer can author them per entity
    // template. Two sites feed them to the pure AI: `helm_ai_decision`
    // (steering/thrust) and the per-axis `ai_helm_lateral_thrust` (lateral
    // dodge). Each test below pins one of the tuning
    // fields by choosing a geometry that the constant and the authored value
    // disagree about, so reverting a site to `crate::ai::AVOIDANCE_*` turns
    // the assertion red.

    /// Seeds a `WorldSnapshot` holding a single stationary obstacle, so the
    /// avoidance maths has exactly one threat to reason about and the
    /// assertions below can attribute any lateral dodge to it alone.
    fn snapshot_with_obstacle(app: &mut App, position: [f32; 3], radius: f32) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("rock".into()),
                position,
                faction: None,
                shields: None,
                hull_fraction: None,
                // `None` yaw keeps the obstacle un-projected, so
                // `avoidance_look_ahead_secs` only moves *our* projected
                // position — one variable, not two.
                yaw: None,
                radius,
                forward_speed: 0.0,
                // A static rock: not movable, but a dangerous collision hazard;
                // size rating tracks its radius (issue #743).
                movable: false,
                dangerous: true,
                size_rating: radius,
            }],
        });
    }

    fn set_behaviour_section(app: &mut App, behaviour: crate::entity_config::BehaviourConfig) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entities::spawner::BehaviourSection(behaviour));
    }

    fn lateral_intent(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&LateralThrustInput>()
            .single(app.world())
            .expect("ship must carry LateralThrustInput")
            .0
    }

    /// `ai_helm_lateral_thrust` under the "Simplified" partial-automation
    /// rating: lateral thrust AI-operated, the helm proper still human. Since
    /// #703 the coarse helm's state no longer gates the system, but these tests
    /// keep it human so the monolith cannot be the writer of the dodge they
    /// measure.
    fn lateral_thrust_ai_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Human);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::lateral_thrust_system_id(),
                    ControlSource::Ai,
                );
            }
        }
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        app
    }

    /// A wider TOML `avoidance_buffer` must widen the dodge radius of the
    /// standalone lateral-thrust AI. The obstacle sits 40 units off the bow:
    /// outside the default 5-unit buffer (radius 0+1+5 = 6), inside an
    /// authored 60 (radius 0+1+60 = 61).
    #[test]
    fn lateral_thrust_ai_honours_toml_authored_avoidance_buffer() {
        // Stationary ship, so `avoidance_look_ahead_secs` scales a zero
        // velocity and cannot influence the result — isolating the buffer.
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = lateral_thrust_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        // Two ticks: belt-and-braces against the shared AI-helm sim tick
        // (#803) — the first update runs on the ready latch's initial `true`,
        // and the second's 200 ms delta fires the timer outright.
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer a 40-unit-distant obstacle is not a threat"
        );

        let mut authored_app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 60-unit avoidance_buffer must bring the same obstacle \
             inside the dodge radius; got no lateral thrust, so the system is still \
             reading crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// A longer TOML `avoidance_look_ahead_secs` must project the ship further
    /// forward before testing for a threat. At 10 u/s the default 3 s horizon
    /// stops 70 units short of the obstacle (well outside the 6-unit dodge
    /// radius); an authored 10 s lands the projection right on top of it.
    #[test]
    fn lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead() {
        // Forward at yaw 0 is -Z, so the obstacle sits 100 units down -Z with
        // a 2-unit lateral offset to give the dodge a defined sign.
        let obstacle = [2.0, 0.0, -100.0];

        fn moving_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
            let mut app = lateral_thrust_ai_app(behaviour);
            let mut physics = get_ship_physics(&mut app);
            physics.forward_speed = 10.0;
            physics.yaw = 0.0;
            set_ship_physics(&mut app, physics);
            app
        }

        let mut default_app = moving_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        // See the buffer test: two ticks, because the timer skips the first.
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "the default 3 s horizon projects only 30 units ahead — the obstacle at \
             100 is not yet a threat"
        );

        let mut authored_app = moving_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 10 s look-ahead projects 100 units ahead, onto the \
             obstacle; got no lateral thrust, so the system is still reading \
             crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// Seeds a `WorldSnapshot` holding a single *moving* obstacle: a ship with
    /// its own `yaw`/`forward_speed`, so the predictive projection folds the
    /// obstacle's motion into the collision test (issue #743). `movable` is set
    /// so the published fact matches a real moving hull.
    fn snapshot_with_moving_obstacle(
        app: &mut App,
        position: [f32; 3],
        radius: f32,
        yaw: f32,
        forward_speed: f32,
    ) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("raider".into()),
                position,
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: Some(yaw),
                radius,
                forward_speed,
                movable: true,
                dangerous: true,
                size_rating: radius,
            }],
        });
    }

    /// Give the subject ship a collision radius, so its `self_size_rating` is
    /// nonzero and the authored ignore-smaller rule has a size to compare a
    /// hazard against (issue #743). Without a collider the test ship rates 0 and
    /// the rule can never fire.
    fn set_ship_collider(app: &mut App, radius: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entities::spawner::ColliderSection(
                crate::entity_config::ColliderConfig {
                    shape: crate::entity_config::ColliderShape::Ball,
                    radius,
                    length: 0.0,
                },
            ));
    }

    /// A static hazard on the starboard bow must push the lateral dodge to port
    /// (negative): the shared hazard assessment's repulsion points away from the
    /// obstacle, and the actuator follows it (issue #743). The obstacle sits
    /// inside an authored 60-unit buffer.
    #[test]
    fn lateral_thrust_ai_dodges_static_hazard() {
        // Starboard bow (+X), dead-ahead-ish down -Z. Stationary ship, so the
        // obstacle's own (absent) motion cannot confound the sign.
        let obstacle = [4.0, 0.0, -40.0];
        let mut app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut app, obstacle, 1.0);
        tick_twice(&mut app);
        assert!(
            lateral_intent(&mut app) < 0.0,
            "a starboard-bow hazard must dodge to port (negative lateral); got {}",
            lateral_intent(&mut app)
        );
    }

    /// A moving hazard is handled through the same shared surface: an obstacle
    /// that is out of range when static becomes a threat once its own forward
    /// motion is projected into the collision test, and the lateral dodge fires
    /// (issue #743).
    #[test]
    fn lateral_thrust_ai_dodges_moving_hazard() {
        // Stationary self at origin (its projection is fixed), obstacle 50 units
        // ahead on the starboard bow. Static, it is far outside the default
        // ~7-unit dodge radius; closing at 16 u/s (yaw = PI faces +Z, back
        // toward us) its 3 s projection lands ~2 units ahead — a real threat.
        let obstacle = [2.0, 0.0, -50.0];

        let mut static_app = lateral_thrust_ai_app(None);
        snapshot_with_obstacle(&mut static_app, obstacle, 1.0);
        tick_twice(&mut static_app);
        assert_eq!(
            lateral_intent(&mut static_app),
            0.0,
            "a static obstacle 50 units off is outside the default dodge radius"
        );

        let mut moving_app = lateral_thrust_ai_app(None);
        snapshot_with_moving_obstacle(&mut moving_app, obstacle, 1.0, std::f32::consts::PI, 16.0);
        tick_twice(&mut moving_app);
        assert!(
            lateral_intent(&mut moving_app) < 0.0,
            "the obstacle's own motion must bring it into collision and dodge to \
             port; got {}",
            lateral_intent(&mut moving_app)
        );
    }

    /// The authored `lateral_hazard_sensitivity` gates the response to the
    /// shared hazard surface: an obstacle that dodges at the default sensitivity
    /// produces no lateral thrust when the hull authors sensitivity 0, and a
    /// wider authored sensitivity does not zero it (issue #743). This pins that
    /// the actuator reads the shared surface scaled by its own authored weight.
    #[test]
    fn lateral_thrust_ai_responds_to_shared_hazard_surface() {
        let obstacle = [4.0, 0.0, -40.0];

        // Default sensitivity (1.0): the in-range obstacle dodges.
        let mut responsive = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut responsive, obstacle, 1.0);
        tick_twice(&mut responsive);
        assert!(
            lateral_intent(&mut responsive).abs() > 0.0,
            "the shared hazard force must drive a dodge at the default sensitivity"
        );

        // Sensitivity 0: the same shared hazard force is weighted to nothing.
        let mut muted = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            lateral_hazard_sensitivity: 0.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut muted, obstacle, 1.0);
        tick_twice(&mut muted);
        assert_eq!(
            lateral_intent(&mut muted),
            0.0,
            "an authored zero sensitivity must mute the response to the shared \
             hazard surface"
        );
    }

    /// The authored ignore-smaller rule reaches the lateral dodge: a large ship
    /// ignores a hazard below its own size rating entirely, so the same obstacle
    /// that would otherwise dodge produces zero lateral thrust (issue #743).
    #[test]
    fn lateral_thrust_ai_ignores_hazard_smaller_than_self() {
        // Obstacle inside an authored 60-unit buffer so it *is* a threat when
        // the ignore rule is off. Self rates size 10 (collider radius); the
        // obstacle rates 1.
        let obstacle = [4.0, 0.0, -40.0];

        let mut dodges = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        set_ship_collider(&mut dodges, 10.0);
        snapshot_with_obstacle(&mut dodges, obstacle, 1.0);
        tick_twice(&mut dodges);
        assert!(
            lateral_intent(&mut dodges).abs() > 0.0,
            "with the ignore rule off, the in-range obstacle must dodge"
        );

        let mut ignores = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            // Ignore any hazard whose size rating is below self's (10 × 1.0 = 10);
            // the obstacle rates 1, so it is skipped.
            hazard_ignore_size_ratio: 1.0,
            ..Default::default()
        }));
        set_ship_collider(&mut ignores, 10.0);
        snapshot_with_obstacle(&mut ignores, obstacle, 1.0);
        tick_twice(&mut ignores);
        assert_eq!(
            lateral_intent(&mut ignores),
            0.0,
            "a hazard smaller than self must be ignored under the authored rule"
        );
    }

    /// Drives the `helm_ai_decision` → `operate_helm` → `avoidance_steering`
    /// path: a Reach anchor dead ahead down -Z, so the base steer sits in the
    /// deadband at zero and any nonzero `SteeringInput` is avoidance and
    /// nothing else. `avoidance_steering` ignores ships slower than
    /// `AVOIDANCE_MIN_SPEED`, hence the explicit forward speed.
    fn helm_ai_steering_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        app.insert_resource(world_config_with_anchor("far-ahead", [0.0, 0.0, -900.0]));
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("far-ahead", 8.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        app
    }

    fn steering_intent(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&SteeringInput>()
            .single(app.world())
            .expect("ship must carry SteeringInput")
            .0
    }

    /// `helm_ai_decision` feeds `avoidance_buffer` to `operate_helm`, where it
    /// widens the radius `avoidance_steering` treats as a threat.
    #[test]
    fn helm_ai_decision_honours_toml_authored_avoidance_buffer() {
        // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
        // ~10.8 units away: outside the default 6-unit dodge radius
        // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = helm_ai_steering_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            steering_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer the obstacle is no threat and the anchor \
             is dead ahead, so steering stays in the deadband"
        );

        let mut authored_app = helm_ai_steering_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            steering_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 60-unit avoidance_buffer must make the helm steer around \
             the obstacle; got no steering, so helm_ai_decision is still passing \
             crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// `helm_ai_decision` feeds `avoidance_look_ahead_secs` to `operate_helm`,
    /// where it sets how far forward `avoidance_steering` projects the ship
    /// before testing for a threat.
    #[test]
    fn helm_ai_decision_honours_toml_authored_avoidance_look_ahead() {
        // At 10 u/s the default 3 s horizon projects 30 units ahead, leaving
        // the obstacle ~70 units off; a 10 s horizon projects 100 units, right
        // onto it.
        let obstacle = [2.0, 0.0, -100.0];

        let mut default_app = helm_ai_steering_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick(&mut default_app);
        assert_eq!(
            steering_intent(&mut default_app),
            0.0,
            "the default 3 s horizon does not reach the obstacle at 100 units"
        );

        let mut authored_app = helm_ai_steering_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick(&mut authored_app);
        assert!(
            steering_intent(&mut authored_app).abs() > 0.0,
            "a TOML-authored 10 s look-ahead must bring the obstacle into the helm's \
             projected path; got no steering, so helm_ai_decision is still passing \
             crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// Drives the full-AI helm's dodge — every helm axis on AI, the shape an
    /// unmanned Helm station or an NPC hull comes up in.
    ///
    /// Until #704 the subject here was `operate_helm_ai`, which derived the
    /// dodge itself; it now comes from `ai_helm_lateral_thrust` like every other
    /// lateral write, reading the shared hazard surface (issue #743). The
    /// fixture still earns its place next to `lateral_thrust_ai_app`: that one
    /// pins the same tunables under the *Simplified* rating (coarse helm human,
    /// lateral automated — what the cruiser and destroyer ship), this one under
    /// a fully-AI helm. Same system, the two gate shapes real content deploys.
    ///
    /// Forward speed is not optional scaffolding — the shared `assess_hazards`
    /// projects the ship by `forward_speed * avoidance_look_ahead_secs`, so a
    /// stationary ship collapses that projection onto its own position and makes
    /// the look-ahead term unobservable no matter what value is passed.
    fn helm_ai_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        let mut cfg = crate::world::config::WorldConfig::default();
        // Waypoint far down -Z keeps the helm driving straight ahead, so
        // the lateral axis reflects avoidance alone.
        cfg.anchors.insert("wp0".into(), [0.0, 0.0, -900.0]);
        app.insert_resource(cfg);
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        app
    }

    /// The same two tunables reach the pure AI a second way: the full-AI helm's
    /// dodge. The dodge and the steering must agree about clearance, so this
    /// site must read the same TOML the steering does.
    ///
    /// Ported in #704, then rewired in #743: the dodge is now the shared
    /// hazard surface read by `ai_helm_lateral_thrust`. Faithful because the
    /// property under test is unchanged — a TOML-authored `avoidance_buffer`
    /// must reach the shared `assess_hazards` on a fully-AI helm rather than the
    /// `crate::ai::AVOIDANCE_BUFFER` constant — and it is asserted on the same
    /// hull, obstacle and geometry as before. What the delete changed is only
    /// *which* system performs the write, and hence the tick count: the
    /// monolith's call was unthrottled, whereas `ai_helm_lateral_thrust` is
    /// gated by the deliberate shared AI-helm sim tick (~30 Hz by default,
    /// issue #803). Hence `tick_twice`, matching `lateral_thrust_ai_honours_*`.
    #[test]
    fn full_ai_helm_honours_toml_authored_avoidance_buffer() {
        // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
        // ~10.8 units away: outside the default 6-unit dodge radius
        // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
        let obstacle = [4.0, 0.0, -40.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "with the default 5-unit buffer the obstacle sits ~10.8 units off the \
             projected path and is not a threat"
        );

        let mut authored_app = helm_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "the full-AI helm must pass the TOML-authored avoidance_buffer to \
             the shared assess_hazards, not crate::ai::AVOIDANCE_BUFFER"
        );
    }

    /// The full-AI helm must pass the TOML-authored `avoidance_look_ahead_secs`
    /// to the shared `assess_hazards`, which uses it to project the ship forward
    /// before testing for a threat. Mirrors
    /// `lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead`, but with
    /// every helm axis on AI rather than the Simplified rating's lateral-only.
    ///
    /// Ported in #704, rewired in #743 to the shared hazard surface — same
    /// property, same geometry, hence `tick_twice` for the shared AI-helm sim
    /// tick. See that test's note.
    #[test]
    fn full_ai_helm_honours_toml_authored_avoidance_look_ahead() {
        // Forward at yaw 0 is -Z. At 10 u/s the default 3 s horizon projects
        // only 30 units ahead, leaving the obstacle ~70 units off; an authored
        // 10 s projects 100 units, landing 2 units from it — inside the default
        // 6-unit dodge radius (0 + 1 + 5), so the buffer is held constant and
        // the look-ahead is the only variable.
        let obstacle = [2.0, 0.0, -100.0];

        let mut default_app = helm_ai_app(None);
        snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
        tick_twice(&mut default_app);
        assert_eq!(
            lateral_intent(&mut default_app),
            0.0,
            "the default 3 s horizon projects only 30 units ahead — the obstacle at \
             100 is not yet a threat"
        );

        let mut authored_app = helm_ai_app(Some(crate::entity_config::BehaviourConfig {
            avoidance_look_ahead_secs: 10.0,
            ..Default::default()
        }));
        snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
        tick_twice(&mut authored_app);
        assert!(
            lateral_intent(&mut authored_app).abs() > 0.0,
            "the full-AI helm must pass the TOML-authored avoidance_look_ahead_secs to the \
             shared assess_hazards, not crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
        );
    }

    /// `nav_handoff_speed` is the throttle the helm adopts for a Channel-3
    /// Navigation→Helm handoff. It is authored in `[behaviour]`, and the
    /// `crate::ai::NAV_HANDOFF_SPEED` fallback exists only for an entity with
    /// no `[behaviour]` section at all.
    #[test]
    fn helm_ai_honours_toml_authored_nav_handoff_speed() {
        fn nav_goal_app(behaviour: Option<crate::entity_config::BehaviourConfig>) -> App {
            let mut app = test_app();
            set_helm_control_source(&mut app, ControlSource::Ai);
            // A Helm-relevant objective must exist (an empty pool makes
            // `operate_helm_ai` zero the intent and skip the decision
            // entirely), but it must not *resolve* — a Reach whose anchor is
            // absent from the WorldConfig yields `None`, so `operate_helm`
            // falls through to the Navigation waypoint handoff, the only path
            // that reads `nav_handoff_speed`.
            set_ship_blackboard_objectives(
                &mut app,
                vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
            );
            if let Some(behaviour) = behaviour {
                set_behaviour_section(&mut app, behaviour);
            }
            // Post-#702 the handoff is the ship's own `NavigationWaypoint`,
            // gated by a matching `HelmWaypointClearance`, rather than a
            // private `AiMemory.nav_goal` copy. Dead ahead and far away, so the
            // helm throttles up at exactly `nav_handoff_speed`.
            set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
            tick(&mut app);
            app
        }

        fn thrust(app: &mut App) -> f32 {
            app.world_mut()
                .query::<&ThrustInput>()
                .single(app.world())
                .expect("ship must carry ThrustInput")
                .0
        }

        assert!(
            (thrust(&mut nav_goal_app(None)) - crate::ai::NAV_HANDOFF_SPEED).abs() < 1e-6,
            "a ship with no [behaviour] section must fall back to NAV_HANDOFF_SPEED"
        );
        assert!(
            (thrust(&mut nav_goal_app(Some(
                crate::entity_config::BehaviourConfig {
                    nav_handoff_speed: 0.25,
                    ..Default::default()
                }
            ))) - 0.25)
                .abs()
                < 1e-6,
            "a TOML-authored nav_handoff_speed must be the throttle the helm adopts, \
             not crate::ai::NAV_HANDOFF_SPEED"
        );
    }

    /// Regression (issue #696 review, finding 2): Reach completion is the
    /// other site that judged arrival against the hardcoded constant.
    #[test]
    fn detect_reach_completion_honours_toml_authored_arrival_radius() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        fn reach_app(arrival_radius: Option<f32>) -> App {
            let mut app = test_app();
            let anchor = "dock-mid";
            // 100 units out: inside a 150 radius, outside the default 20.
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
            app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
            set_helm_control_source(&mut app, ControlSource::Ai);
            if let Some(radius) = arrival_radius {
                let ship = find_ship_entity(&mut app);
                app.world_mut().entity_mut(ship).insert(
                    crate::entities::spawner::BehaviourSection(
                        crate::entity_config::BehaviourConfig {
                            waypoint_arrival_radius: radius,
                            ..Default::default()
                        },
                    ),
                );
            }
            let mut mgr = ObjectiveManager::new();
            mgr.add_full(
                "reach-dock-mid",
                "Dock at Mid",
                true,
                vec![],
                AiDirective::Reach {
                    anchor: anchor.into(),
                },
                UtilityConfig::default(),
                ObjectiveSource::Mission,
            );
            app.insert_resource(ObjectiveManagerRes(mgr));
            tick(&mut app);
            app
        }

        fn status(app: &App) -> Option<crate::messages::ObjectiveStatus> {
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach-dock-mid")
                .map(|o| o.status)
        }

        assert_eq!(
            status(&reach_app(None)),
            Some(crate::messages::ObjectiveStatus::Active),
            "the default arrival radius must not count 100 units away as reached"
        );
        assert_eq!(
            status(&reach_app(Some(150.0))),
            Some(crate::messages::ObjectiveStatus::Completed),
            "a TOML-widened arrival radius must complete the Reach objective"
        );
    }

    #[test]
    fn detect_reach_completion_does_not_complete_when_far() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-far";
        // Anchor 500 units away — ship starts at origin.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [500.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-far",
            "Dock at Far",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-far");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Active)
                .unwrap_or(false),
            "Reach objective must remain Active when ship is far from the anchor"
        );
    }

    #[test]
    fn detect_reach_completion_does_not_complete_when_helm_human() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig};
        use crate::world::server::ObjectiveManagerRes;

        let mut app = test_app();
        let anchor = "dock-beta";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
        // helm stays Human — completion system must not fire

        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-beta",
            "Dock at Beta",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));

        tick(&mut app);

        let res = app.world().resource::<ObjectiveManagerRes>();
        let obj = res
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-beta");
        assert!(
            obj.map(|o| o.status == crate::messages::ObjectiveStatus::Active)
                .unwrap_or(false),
            "Reach completion must not fire when helm is human-controlled"
        );
    }

    // ── E5 smoke tests (#553) ─────────────────────────────────────────────────

    // (a) Pirate raider — verifies that an NPC ship with both stick axes on
    // Ai control satisfies `helm_axes_operate_ai`, the gate every per-ship
    // "is the AI flying this" consumer reads since #801.
    #[test]
    fn pirate_raider_ai_helm_policy_routes_through_npc_path() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        for system_id in [
            crate::system_registry::helm_thrust_system_id(),
            crate::system_registry::helm_steering_system_id(),
        ] {
            resolver.set(system_id, ControlSource::Ai);
        }
        let sources = ShipSystemControlSources(resolver);
        assert!(
            helm_axes_operate_ai(&sources),
            "NPC raider helm axes must route through the AI helm path"
        );
        assert!(
            !sources
                .0
                .policy_for(&crate::system_registry::helm_thrust_system_id())
                .accept_human_input,
            "NPC raider must not accept human helm input"
        );
    }

    // (b) All-Backfill player ship — verifies that when the player ship has
    // both stick axes on Ai control but no BehaviourSection (no
    // behaviour tree), `helm_axes_operate_ai` still returns true. A single
    // AI axis is NOT enough — the predicate answers "is the AI flying this
    // ship", which needs both.
    #[test]
    fn all_backfill_helm_policy_gates_operate_ai() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut resolver = ControlSourceResolver::new();
        resolver.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        let sources = ShipSystemControlSources(resolver);
        assert!(
            !helm_axes_operate_ai(&sources),
            "one AI axis alone must not satisfy the whole-helm AI predicate"
        );

        let mut resolver = ControlSourceResolver::new();
        resolver.set(
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );
        resolver.set(
            crate::system_registry::helm_steering_system_id(),
            ControlSource::Ai,
        );
        let sources = ShipSystemControlSources(resolver);
        assert!(
            helm_axes_operate_ai(&sources),
            "Backfill player helm (both axes AI) must satisfy the AI-helm gate"
        );
    }

    // (c) Player ship Backfill runs full operate_helm (avoidance + doctrine).
    // Verifies that the player ship on Backfill goes through the same
    // `operate_helm` decision (via the per-axis AI systems) as NPC ships — not
    // a Reach-only stub — satisfying issue #587 AC.
    #[test]
    fn backfill_runs_full_operate_helm_with_objectives() {
        let mut app = test_app();
        // Give the ship a Destroy objective (non-Reach) pointing at an entity.
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.clone()),
            crate::entities::spawner::EntityName("enemy_fighter".into()),
            Transform::from_xyz(80.0, 0.0, 0.0),
        ));
        set_ship_blackboard_objectives(
            &mut app,
            vec![destroy_scored_objective("enemy_fighter", 60.0)],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);
        // Tactical's lock: what the helm pursues (issue #702).
        set_ship_weapons_target(&mut app, &target_uuid);

        tick(&mut app);

        let last = get_last_helm_input(&mut app);
        // The Destroy directive targets an entity at (80, 0). Full operate_helm
        // should produce non-zero thrust to pursue it.
        assert!(
            last.thrust > 0.0 || last.steering.abs() > 0.0,
            "player ship Backfill must run full operate_helm (non-Reach); \
             got thrust={}, steering={}",
            last.thrust,
            last.steering
        );
    }

    #[test]
    fn backfill_helm_ai_caps_long_frame_yaw_step() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid),
            crate::entities::spawner::EntityName("enemy_fighter".into()),
            Transform::from_xyz(80.0, 0.0, 0.0),
        ));
        set_ship_blackboard_objectives(
            &mut app,
            vec![destroy_scored_objective("enemy_fighter", 60.0)],
        );
        set_helm_control_source(&mut app, ControlSource::Ai);

        let before = get_ship_physics(&mut app);
        tick(&mut app);
        let after = get_ship_physics(&mut app);

        let max_step = ShipPhysicsConfig::new().max_yaw_rate * HELM_AI_MAX_DT_SECS;
        let yaw_delta = (after.yaw - before.yaw).abs();
        assert!(
            yaw_delta <= max_step + 0.0001,
            "AI helm must not consume a long frame as one oversized yaw step; \
             yaw_delta={yaw_delta}, max_step={max_step}"
        );
    }

    // ── The shared decision-surface frame (issue #824) ─────────────────────

    /// Probe scratch for `all_four_axes_observe_the_same_frame`: snapshots of
    /// the frame taken immediately after it is built and again after every
    /// per-axis system has run.
    #[derive(Resource, Default)]
    struct FrameProbe {
        before: Option<String>,
        after: Option<String>,
    }

    fn probe_frame_before(frame: Res<HelmAiSurfacesFrame>, mut probe: ResMut<FrameProbe>) {
        probe.before = Some(format!("{:?}|{:?}", frame.anchors, frame.ships));
    }

    fn probe_frame_after(frame: Res<HelmAiSurfacesFrame>, mut probe: ResMut<FrameProbe>) {
        probe.after = Some(format!("{:?}|{:?}", frame.anchors, frame.ships));
    }

    /// AC (issue #824): all four per-axis systems observe the *same* frame —
    /// the identical-inputs invariant is true by construction. The axis
    /// systems take `Res<HelmAiSurfacesFrame>` (immutable), so the compiler
    /// already forbids them mutating it; this pins the runtime half — nothing
    /// else rebuilds or edits the frame between the builder and the last
    /// axis system within a tick.
    #[test]
    fn all_four_axes_observe_the_same_frame() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);

        app.init_resource::<FrameProbe>();
        app.add_systems(
            Update,
            (
                probe_frame_before
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(build_helm_ai_surfaces_frame)
                    .before(ai_helm_thrust)
                    .before(ai_helm_steering)
                    .before(ai_helm_lateral_thrust)
                    .before(ai_helm_impulse)
                    .run_if(ai_helm_tick_ready),
                probe_frame_after
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(ai_helm_thrust)
                    .after(ai_helm_steering)
                    .after(ai_helm_lateral_thrust)
                    .after(ai_helm_impulse)
                    .before(tick_ai_helm_timer)
                    .run_if(ai_helm_tick_ready),
            ),
        );

        tick(&mut app);

        let probe = app.world().resource::<FrameProbe>();
        let before = probe
            .before
            .as_ref()
            .expect("probe must run on the first (always-ready) AI tick");
        let after = probe
            .after
            .as_ref()
            .expect("probe must run after the four axis systems");
        assert!(
            before.contains("station-alpha") && before.contains("HelmAiShipFrame"),
            "precondition: the frame must actually carry a ship entry and the anchor, \
             else this equality is vacuous; got {before}"
        );
        assert_eq!(
            before, after,
            "the frame every axis observes must be identical before the first \
             and after the last per-axis system — a difference means something \
             mutated the shared decision surface mid-tick"
        );
    }

    /// AC (issue #824, work item 1): with per-entity Helm publishing, an NPC
    /// ship's `helm_ai_radar_range` reads the live (damage-scaled) value from
    /// its own Helm blackboard entry instead of the static
    /// `HelmConsoleSection` fallback — which remains in place for ships whose
    /// entry has not been published (low-LOD / missing-entry).
    #[test]
    fn helm_ai_radar_range_prefers_the_npc_blackboard_entry() {
        let helm_config = crate::entity_config::EntityConfig::from_toml(
            "[helm_console]\nmax_speed = 30.0\n\n[helm_console.radar]\nrange = 800.0\nshows = [\"ship\"]\n",
        )
        .unwrap()
        .helm_console
        .unwrap();
        let helm_section = crate::entities::spawner::HelmConsoleSection(helm_config);

        // With a published Helm entry (as per-entity publish now provides for
        // NPCs): the live, damage-scaled value wins.
        let mut bbs = crate::server_app::ShipSystemBlackboards::default();
        bbs.0.insert(
            crate::system_registry::helm_station_key(),
            crate::messages::SystemBlackboard::Helm(crate::messages::HelmBlackboard {
                radar_range: 400.0,
                ..Default::default()
            }),
        );
        assert_eq!(
            helm_ai_radar_range(&bbs, Some(&helm_section), None, false),
            400.0,
            "an NPC with a published Helm entry must read the live radar_range"
        );

        // Without an entry (low-LOD ship / pre-first-publish): the static
        // config fallback is preserved.
        let empty_bbs = crate::server_app::ShipSystemBlackboards::default();
        assert_eq!(
            helm_ai_radar_range(&empty_bbs, Some(&helm_section), None, false),
            800.0,
            "a ship with no Helm entry must fall back to its authored radar range"
        );
    }
}
