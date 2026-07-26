use bevy::prelude::*;

// Vertical thrust (the AI-only fourth axis, not one of the migration's
// dependency-modelled operators) still emits directly through the shared
// arbiter. The player-facing per-axis operators route through their own
// single-owner seams (`helm_ai_emit` / `helm_lateral_emit`, issue #745) so each
// operator's command-admission dependency is a per-entity observed code edge.
use crate::command_admission::ai_emit::emit_ai_command;
#[cfg(test)]
use crate::ship::components::LastHelmInput;
use crate::ship::components::{
    BoostConfigResource, HelmWaypointClearance, ImpulseConfigResource, PendingArcBearingRequest,
    ShipSystemControlSources,
};
#[cfg(test)]
use crate::ship::helm::{
    ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput, VerticalThrustInput,
};
use crate::ship::helm_ai_emit::emit_helm_ai_command;
use crate::ship::helm_lateral_emit::emit_helm_lateral_command;
use crate::ship_state::ShipPhysics;
use crate::simulation::{ShipBoost, ShipImpulse};

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
            crate::system_registry::vertical_thrust_system_id(),
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
fn apply_arc_bearing_request(
    steering: &mut f32,
    pending: Option<&mut PendingArcBearingRequest>,
    world_view: &crate::ai::WorldView,
    physics: &ShipPhysics,
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

/// Per-ship inline stateless **Engines** AI policy (issue #779).
///
/// Attached to every ship at spawn: from the ship's `[helm_console.engines_ai]`
/// block when authored, otherwise the canonical
/// [`crate::entities::config::default_engines_ai_config`] policy. Read by
/// [`ai_helm_thrust`], which resolves its `longitudinal` channel over a per-tick
/// fact snapshot to decide *whether* to actuate the planner's desired travel —
/// the DECISION now flows through a data-authored policy verb instead of an
/// unconditional hardcoded branch. The continuous thrust magnitude still comes
/// from the shared `DesiredMotion` planner fact (issue #741).
#[derive(Component, Clone, Debug, Default)]
pub struct HelmEnginesAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship inline stateless **Steering** AI policy (issue #779). Mirror of
/// [`HelmEnginesAiPolicy`] for the `yaw` channel: from
/// `[helm_console.steering_ai]` when authored, else
/// [`crate::entities::config::default_steering_ai_config`]. Read by
/// [`ai_helm_steering`] to decide whether to actuate the planner's desired
/// facing.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmSteeringAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship inline stateless **Lateral Thrust** AI policy (issue #780). From
/// `[helm_console.lateral_ai]` when authored, else
/// [`crate::entities::config::default_lateral_ai_config`]. Read by
/// [`ai_helm_lateral_thrust`] to decide whether to actuate the dodge this tick;
/// the continuous magnitude still comes from the shared hazard surface.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmLateralAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship inline stateless **Vertical Thrust** AI policy (issue #780). From
/// `[helm_console.vertical_ai]` when authored, else
/// [`crate::entities::config::default_vertical_ai_config`]. Read by
/// [`ai_helm_vertical_thrust`] to decide whether to actuate the climb/return this
/// tick; the authored `VerticalMovementMode` still gates the magnitude host-side.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmVerticalAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship inline stateless **Impulse** AI policy (issue #780). From
/// `[helm_console.impulse_ai]` when authored, else
/// [`crate::entities::config::default_impulse_ai_config`]. Read by
/// [`ai_helm_impulse`] to decide whether the impulse manoeuvre is permitted this
/// tick; the host still applies doctrine `use_impulse` and `decide_impulse`
/// geometry.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmImpulseAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship inline stateless **Boost** AI policy (issue #780). From
/// `[helm_console.boost_ai]` when authored, else the idle
/// [`crate::entities::config::default_boost_ai_config`] (no AI boost by default).
/// Read by [`ai_helm_boost`] to decide whether to engage boost this tick, emitted
/// through the same admitted `SetBoost` seam a human uses.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmBoostAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship runtime state for a STATEFUL Boost policy (issue #882) — the
/// minimal host that proves the optional stateful path end to end.
///
/// Boost was chosen as the demonstrator because it is the smallest credible
/// stateful axis in the game: its shipped default policy is *idle*, so nothing
/// that ships today changes behaviour, and its host already resolves exactly
/// one channel from an already-seeded fact snapshot. (The destroyer doctrine
/// this spine exists for is issue #883, deliberately not built here.)
///
/// ## Why this is a separate component
///
/// [`HelmBoostAiPolicy`] is immutable authored data; taking it `&mut` to tick
/// a state machine would dirty Bevy change-detection on the policy every tick.
/// So the runtime state is its own sibling component.
///
/// ## Why it is per-fine-system, not per-ship
///
/// This component belongs to the Boost fine system ALONE, and there is
/// deliberately no `ShipAiState`. That is the structural answer to AC3: the
/// `memory(...)` / `state_time` bag handed to an evaluation is seeded from
/// THIS component, so no sibling fine system's policy can observe it and no
/// ship-wide state machine can form by accretion.
///
/// Inserted/removed alongside `AiHighFidelity` by `lod_ai_ships`, so a demoted
/// ship drops its policy state and a re-promoted one starts from `initial`
/// (AC5).
#[derive(Component, Clone, Debug, Default)]
pub struct HelmBoostAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// Per-ship runtime state for a STATEFUL Engines policy (issue #883).
///
/// The Engines twin of [`HelmBoostAiPolicyState`], and separate from it for the
/// same structural reason: private memory belongs to ONE fine system. The
/// destroyer's Engines, Steering and Boost each run their own copy of the
/// fly-through machine over the same host-seeded facts, so they reach the same
/// leg on the same tick *independently* — there is no ship-wide pass state that
/// one of them owns and the others read.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmEnginesAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// Per-ship runtime state for a STATEFUL Steering policy (issue #883). The
/// Steering twin of [`HelmEnginesAiPolicyState`]; this is the one whose current
/// state decides which leg [`HelmPassSurface`] publishes, because the yaw
/// channel is the axis that carries the two different facing verbs.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmSteeringAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

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
/// every frame rate because both systems run on the shared `ai_helm_tick_ready`
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
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
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

/// The fact name the shared hazard surface is seeded under by
/// [`seed_helm_actuator_facts`].
pub(crate) const HAZARD_URGENCY_FACT: &str = "hazard_urgency";

// ── Target-relative travel facts (issue #883, AC5) ───────────────────────────
//
// #779 shipped the two TRAVEL axes with `AiFacts::new()` — an empty snapshot, so
// every `fact(...)` guard on `longitudinal`/`yaw` validated and then never
// fired. #780 closed that hole for the four SECONDARY axes by seeding hazard and
// availability facts; these constants and `seed_helm_travel_facts` close it for
// the travel axes, which is what a doctrine reasoning about a moving target
// needs. All of it is computed HOST-side from the frame's merged view and
// `ShipPhysics`, so `policy.rs` stays Bevy-free (AGENTS.md #10).

/// Planar distance to the ship's current target, world units.
pub(crate) const RANGE_TO_TARGET_FACT: &str = "range_to_target";
/// Rate at which the range is shrinking, world units/s. Positive closing,
/// negative opening — the sign flip IS closest approach.
pub(crate) const CLOSING_RATE_FACT: &str = "closing_rate";
/// Signed bearing to the target, radians, starboard positive.
pub(crate) const BEARING_TO_TARGET_FACT: &str = "bearing_to_target";
/// `1.0` when the ship has a target its own helm view can actually see.
pub(crate) const TARGET_VALID_FACT: &str = "target_valid";
/// Current forward speed as a fraction of the hull's authored `max_speed`.
pub(crate) const SPEED_FRACTION_FACT: &str = "speed_fraction";
/// How far the range has re-opened above the minimum this policy state has seen:
/// `range_to_target - memory(min_range_seen)`.
///
/// DERIVED per fine system, because it folds a world reading against that
/// system's OWN private memory. The predicate grammar compares one atom to a
/// literal or a `param(...)` — deliberately, so guards stay a flat readable
/// table — so the subtraction is the host's job, exactly as the continuous
/// thrust magnitude is. The policy still owns the decision: it compares this
/// against its authored `closest_approach_hysteresis`.
pub(crate) const RANGE_ABOVE_MIN_SEEN_FACT: &str = "range_above_min_seen";

// ── Shield-recovery facts (issue #788) ───────────────────────────────────────
//
// SCOPE, and it is narrower than the facts above: these three are seeded by
// `seed_recovery_facts`, which only `ai_policy_state_tick` calls. They are
// therefore available to TRANSITION guards and not to a state's continuous
// RULE guards, which the per-axis actuator hosts resolve from their own
// snapshot.
//
// That is deliberate — `safe_distance_held` is the verdict of a bounded history
// window that must be folded exactly once per shared tick, and four hosts each
// folding it would advance it four times as fast — but it is also a sharp edge
// of precisely the #779 shape: a RULE guard authored on one of these names
// parses, validates, and then reads absent for ever. Author them in
// transitions. The shipped destroyer doctrine does; every recovery RULE it
// authors is unconditional.

/// This ship's OWN total shield health as a fraction of capacity, `[0, 1]`.
///
/// Transition-scope only — see the note above.
///
/// New plumbing: the shield fraction was computed host-side for BROADCAST only
/// (`server_app`'s entity-health delta), so no ship could reason about the state
/// of its own shields. Seeded from the shared, pure
/// [`crate::shield::ShieldSystem::fraction`] — the same function the player
/// ship's shields go through, because a shield does not care who owns the hull
/// (AGENTS.md #6).
///
/// Deliberately ABSENT (not zero) for a hull with no shield system at all, so a
/// `fact(shield_fraction) <= …` guard reads false rather than firing
/// permanently on a ship that has no shields to recover.
pub(crate) const SHIELD_FRACTION_FACT: &str = "shield_fraction";
/// The TARGET's longest usable direct-fire range, world units — the threat
/// radius a standoff ring is derived from. Sourced from
/// [`crate::ai::AiWorldEntity::direct_fire_range`], i.e. from that ship's own
/// online blaster and phaser banks. `0.0` for an unarmed or fully-disarmed
/// target.
pub(crate) const TARGET_DIRECT_FIRE_RANGE_FACT: &str = "target_direct_fire_range";
/// The derived safe-ring radius: [`TARGET_DIRECT_FIRE_RANGE_FACT`] plus this
/// hull's authored [`SAFE_RANGE_MARGIN_PARAM`].
///
/// DERIVED host-side for the same reason [`RANGE_ABOVE_MIN_SEEN_FACT`] is: the
/// predicate grammar compares one atom to a literal or a `param(...)`, so a sum
/// of a fact and a param is the host's job. Seeded only when the hull authors
/// the margin — a hull with no recovery doctrine gets no ring.
pub(crate) const SAFE_RANGE_FACT: &str = "safe_range";
/// `1.0` when the ship has HELD at least the safe ring across the whole
/// authored history window (or has no live target at all), `0.0` otherwise.
///
/// The bounded-window half of the re-entry gate. Reduced host-side from
/// [`HelmRecoveryHistory`] to a single reading, because a policy predicate reads
/// scalars and because the window's meaning (full-or-not, tolerance band) is
/// measurement detail the doctrine should not have to restate.
pub(crate) const SAFE_DISTANCE_HELD_FACT: &str = "safe_distance_held";

// ── Pressed-detection facts (issue #789) ─────────────────────────────────────
//
// Same scope and the same sharp edge as the four above: seeded by
// `seed_pressed_facts`, which only `ai_policy_state_tick` calls, so they are
// available to TRANSITION guards and NOT to a state's continuous rule guards.
// `separation_progress` is the verdict of a bounded history window that must be
// folded exactly once per shared tick — four per-axis actuator hosts each
// folding it would advance it four times as fast, so the window would mean a
// quarter of the span the designer authored. The shipped destroyer doctrine
// authors both names in transitions only.

/// How far this ship's separation from its target has NET changed across the
/// authored `pressed_window_ticks` history window, world units — positive when
/// the gap is opening.
///
/// Transition-scope only — see the note above.
///
/// This is the "progress" half of AC1, and it is deliberately a different
/// measurement from [`SAFE_DISTANCE_HELD_FACT`] rather than a re-reading of it.
/// `safe_distance_held` asks whether every sample stayed past a line, which a
/// ship pinned at a *constant* 40 units answers "no" to just as flatly as one
/// being steadily run down — but those are the same answer to the wrong
/// question. A destroyer that cannot escape is one whose separation is not
/// GROWING, and only the two ends of a window can say that.
///
/// Deliberately ABSENT (not zero) until the window is full, and absent for a
/// hull that does not author the complete pressed parameter set, so a
/// `fact(separation_progress) < …` guard reads false rather than firing
/// permanently on a ship whose window has just been cleared — the reading a
/// zero would give, and the worst possible moment to act on it.
pub(crate) const SEPARATION_PROGRESS_FACT: &str = "separation_progress";
/// `1.0` when this ship is inside its target's EFFECTIVE threat range — that
/// is, inside [`TARGET_DIRECT_FIRE_RANGE_FACT`] — and `0.0` otherwise.
///
/// Transition-scope only — see the note above.
///
/// Derived host-side for the same reason [`SAFE_RANGE_FACT`] is: the predicate
/// grammar compares one atom to a literal or a `param(...)`, so a comparison of
/// two FACTS (this ship's range against that ship's reach) is the host's job.
///
/// An unarmed or fully-disarmed target has a reach of `0.0`, so this reads
/// `0.0` at every range against one — which is the correct reading, not an edge
/// case: a ship that cannot shoot cannot be pressing anybody.
pub(crate) const INSIDE_THREAT_RANGE_FACT: &str = "inside_threat_range";

/// Private-memory slot: the smallest range seen since this policy state was
/// entered (issue #883). A running MINIMUM, folded every gated tick by the host —
/// the exact mirror of [`PEAK_HAZARD_MEMORY`]'s running maximum, and the reason
/// #882 built host-written memory in the first place: no single-tick fact and no
/// authored constant can express it.
pub(crate) const MIN_RANGE_SEEN_MEMORY: &str = "min_range_seen";
/// Private-memory slot: the ship's heading (radians) at the instant this
/// policy's last transition committed (issue #883).
///
/// This is what makes "commit to the current outward heading" mean the HEADING
/// rather than the steering command. Written by the host on every commit, read
/// by the host when the authored `hold_committed_heading` verb wins the yaw
/// channel. There is no authored write verb, by #882's design.
pub(crate) const ESCAPE_HEADING_MEMORY: &str = "escape_heading_rad";
/// Private-memory slot: which TARGET [`MIN_RANGE_SEEN_MEMORY`] was accumulated
/// against (issue #883).
///
/// The running minimum is scoped to the state, but a state can outlive a target:
/// swap mid-`inbound` to a target that is further away and an unscoped fold
/// would keep the dead target's minimum, so `range_above_min_seen` would jump
/// straight past the authored hysteresis and fire a SPURIOUS closest approach on
/// a ship that has not begun its run. Storing the identity alongside the minimum
/// lets [`tick_policy_machine`] restart the fold on a target change.
///
/// Host-written and host-read only; no authored guard reads it (memory is `f64`,
/// so it holds [`target_identity_fingerprint`]'s value rather than a uuid).
pub(crate) const MIN_RANGE_TARGET_MEMORY: &str = "min_range_target";

/// A stable numeric fingerprint of a target uuid, for [`MIN_RANGE_TARGET_MEMORY`].
///
/// Private memory is a `f64` bag, so the identity is carried as the uuid's low
/// 48 bits — exactly representable in an `f64` mantissa, so the comparison is
/// never approximate. Two distinct targets colliding needs a 1-in-2^48 match on
/// randomly generated uuids, and the only consequence would be the pre-fix
/// behaviour for that one pair.
fn target_identity_fingerprint(uuid: uuid::Uuid) -> f64 {
    (uuid.as_u128() as u64 & 0x0000_FFFF_FFFF_FFFF) as f64
}

/// Authored Engines `param` naming the inbound throttle fraction (issue #883).
pub(crate) const APPROACH_SPEED_PARAM: &str = "approach_speed";
/// Authored Engines `param` naming the escape-leg throttle fraction.
pub(crate) const ESCAPE_SPEED_PARAM: &str = "escape_speed";
/// Authored Steering `param` naming the tracking deadband, radians.
pub(crate) const TRACKING_DEADBAND_PARAM: &str = "tracking_deadband_rad";
/// Authored Steering `param` naming the tracking saturation angle, radians.
pub(crate) const TRACKING_FULL_STEER_PARAM: &str = "tracking_full_steer_rad";

// ── Authored shield-recovery manoeuvre params (issue #788) ───────────────────
//
// All six are read off the STEERING policy, the axis that owns the recovery
// legs (its yaw verb is what tells the host which leg is being flown). There is
// no default for any of them anywhere in Rust: a hull that omits one publishes
// `recover = false` and flies ordinary doctrine travel rather than orbiting at
// an invented radius (AGENTS.md #11).

/// World units added to the target's own direct-fire reach to get the safe ring.
pub(crate) const SAFE_RANGE_MARGIN_PARAM: &str = "safe_range_margin";
/// Throttle fraction flown while orbiting the ring.
pub(crate) const ORBIT_SPEED_PARAM: &str = "orbit_speed";
/// Radians of heading offset per unit of *fractional* radial error — how hard
/// the orbit spirals back onto the ring.
pub(crate) const ORBIT_SPIRAL_GAIN_PARAM: &str = "orbit_spiral_gain";
/// World units inside the ring that still count as "at safe distance" when
/// folding the history window. Absorbs the orbit's own overshoot so a spiral
/// that is converging correctly is not read as a breach.
pub(crate) const SAFE_RING_TOLERANCE_PARAM: &str = "safe_ring_tolerance";
/// Length of the bounded distance history, in shared AI ticks. The "maintained"
/// in "maintained safe distance" — one good sample is not a maintained distance.
pub(crate) const SAFE_DISTANCE_WINDOW_TICKS_PARAM: &str = "safe_distance_window_ticks";
/// Throttle fraction flown on the re-entry pivot. `0.0` cuts thrust for the turn.
pub(crate) const REENGAGE_SPEED_PARAM: &str = "reengage_speed";

/// Every scalar the shield-recovery arm needs, gated as ONE unit by
/// [`recovery_params_authored`].
///
/// All six, not merely the four [`build_pass_surface`] reads for itself:
/// `safe_ring_tolerance` and `safe_distance_window_ticks` are consumed by
/// [`seed_recovery_facts`] instead, and a hull that omits either can never
/// satisfy `fact(safe_distance_held)` — so admitting the arm without them would
/// orbit for ever rather than decline.
pub(crate) const RECOVERY_PARAMS: &[&str] = &[
    SAFE_RANGE_MARGIN_PARAM,
    ORBIT_SPEED_PARAM,
    ORBIT_SPIRAL_GAIN_PARAM,
    SAFE_RING_TOLERANCE_PARAM,
    SAFE_DISTANCE_WINDOW_TICKS_PARAM,
    REENGAGE_SPEED_PARAM,
];

/// Does this Steering policy author the complete recovery scalar set?
///
/// The one place the six-name question is asked, because two callers need the
/// same answer and must not drift apart. [`build_pass_surface`] asks it to
/// decide whether to publish `recover`/`reengage` at all; [`seed_pressed_facts`]
/// asks it because the pressed pivot is FLOWN as the re-engage leg, so a hull
/// that fails this check cannot fly the pressed arm either — see that function.
fn recovery_params_authored(params: &crate::world::flags::AiParams) -> bool {
    RECOVERY_PARAMS
        .iter()
        .all(|name| params.get(name).is_some())
}

// ── Authored pressed-doctrine params (issue #789) ────────────────────────────
//
// The four scalars the pressed short-pass arm is flown by. Like the recovery
// six they are read off the STEERING policy, they have no default anywhere in
// Rust, and the host gates on ALL of them together: see [`PRESSED_PARAMS`].

/// Length of the separation-PROGRESS history, in shared AI ticks — the span the
/// "am I actually getting away" question is asked over.
///
/// Its own parameter rather than a reuse of
/// [`SAFE_DISTANCE_WINDOW_TICKS_PARAM`]: see [`HelmRecoveryHistory::separation`].
pub(crate) const PRESSED_WINDOW_TICKS_PARAM: &str = "pressed_window_ticks";
/// World units of separation the ship must have GAINED across that window for
/// its escape to count as working. Below it, inside the target's own reach, the
/// escape has failed and the ship is pressed.
pub(crate) const PRESSED_MIN_PROGRESS_PARAM: &str = "pressed_min_progress";
/// How long (seconds) the boosted, thrust-cut pivot runs before the short pass
/// begins.
pub(crate) const PRESSED_PIVOT_SECS_PARAM: &str = "pressed_pivot_secs";
/// The SHORT pass's own closest-approach hysteresis: how far the range must
/// re-open above the minimum seen before the pressed pass breaks off. Authored
/// separately from — and smaller than — `closest_approach_hysteresis`, which is
/// what makes a pressed pass a shorter pass rather than a re-run of the
/// ordinary one.
pub(crate) const PRESSED_HYSTERESIS_PARAM: &str = "pressed_closest_approach_hysteresis";

/// The pressed arm's OWN four scalars, gated as one unit by
/// [`seed_pressed_facts`] — which requires [`RECOVERY_PARAMS`] on top of these,
/// because the pressed pivot is flown as the re-engage leg.
///
/// All four, not merely the one the host reads for itself. #788's review caught
/// the mirror of this: a gate that required four of six params admitted a
/// partially-authored hull into an arm it could never fly out of. The same trap
/// is here — a hull authoring the window but not the progress threshold would
/// have the host folding a measurement no guard can use, and one authoring the
/// thresholds but not the window would fold a zero-length window and read
/// "never pressed" for ever with nothing failing. Declining the whole arm on any
/// one missing name leaves the hull flying the ordinary recovery doctrine, which
/// is a behaviour a designer can actually see.
pub(crate) const PRESSED_PARAMS: &[&str] = &[
    PRESSED_WINDOW_TICKS_PARAM,
    PRESSED_MIN_PROGRESS_PARAM,
    PRESSED_PIVOT_SECS_PARAM,
    PRESSED_HYSTERESIS_PARAM,
];

// ── Authored combat-orbit params (issue #790) ────────────────────────────────
//
// The broadside orbit's own three scalars, read off the STEERING policy for the
// same reason the recovery six are: Steering's yaw verb is what tells the host
// which leg is being flown. There is no default for any of them anywhere in
// Rust, and the host gates on ALL THREE together — see [`COMBAT_ORBIT_PARAMS`].

/// The fighting radius the orbit holds, world units.
///
/// AUTHORED, and deliberately not routed through [`SAFE_RANGE_FACT`] /
/// [`seed_recovery_facts`]. Those derive a ring from the TARGET's direct-fire
/// reach plus a margin, which is the right question for a shield-recovery
/// standoff and the wrong one for a fighting range: this hull wants the enemy
/// inside its OWN weapon envelope, a number that belongs to the hull and is
/// knowable when the file is written.
pub(crate) const COMBAT_ORBIT_RANGE_PARAM: &str = "combat_orbit_range";
/// Throttle fraction flown on the combat ring. Non-zero by construction — a
/// broadside orbit that stops is a station-keeper, not an orbit.
pub(crate) const COMBAT_ORBIT_SPEED_PARAM: &str = "combat_orbit_speed";
/// Radians of heading offset per unit of *fractional* radial error — how hard
/// the orbit spirals back onto the ring from inside or outside it.
pub(crate) const COMBAT_ORBIT_SPIRAL_GAIN_PARAM: &str = "combat_orbit_spiral_gain";

/// Every scalar the combat-orbit arm needs, gated as ONE unit by
/// [`combat_orbit_params_authored`].
///
/// All three, for the reason #788's and #789's reviews both landed on: a gate
/// that requires only some of the params an arm needs admits a
/// partially-authored hull into an arm it half-flies. A hull authoring the range
/// but not the throttle would orbit at zero speed (a parked ship inside a
/// hostile's guns); one authoring the throttle but not the range would fly a
/// tangent of a ring of radius zero, which is a spiral straight into the target.
/// Declining the whole arm leaves it flying ordinary doctrine travel, which is a
/// behaviour a designer can actually see.
pub(crate) const COMBAT_ORBIT_PARAMS: &[&str] = &[
    COMBAT_ORBIT_RANGE_PARAM,
    COMBAT_ORBIT_SPEED_PARAM,
    COMBAT_ORBIT_SPIRAL_GAIN_PARAM,
];

/// Does this Steering policy author the complete combat-orbit scalar set?
///
/// The sibling of [`recovery_params_authored`], and separate from it on purpose:
/// a hull may fly a combat orbit with no shield-recovery doctrine at all, and
/// vice versa.
fn combat_orbit_params_authored(params: &crate::world::flags::AiParams) -> bool {
    COMBAT_ORBIT_PARAMS
        .iter()
        .all(|name| params.get(name).is_some())
}

// ── Authored torpedo-opportunity params (issue #791) ─────────────────────────
//
// The bow-on hold's single scalar, read off the STEERING policy for the same
// reason every other leg's are: Steering's yaw verb is what tells the host which
// leg is being flown. There is no default for it anywhere in Rust, and the host
// gates the arm on it — see [`TORPEDO_BEARING_PARAMS`].

/// Throttle fraction flown while holding the bow on a torpedo opportunity.
///
/// AUTHORED, and deliberately its own name rather than a reuse of
/// [`REENGAGE_SPEED_PARAM`]. The value a hull wants here is very often `0.0`
/// (cut thrust, stop swinging the beam, let the tube line up), and `0.0` is
/// exactly the value that cannot be distinguished from "unauthored" unless the
/// gate asks for the name. A hull omitting it declines the whole arm and flies
/// its ordinary leg instead of coasting to a halt in front of an enemy.
pub(crate) const TORPEDO_BEARING_SPEED_PARAM: &str = "torpedo_bearing_speed";

/// Every scalar the torpedo-opportunity arm needs, gated as ONE unit by
/// [`torpedo_bearing_params_authored`].
///
/// A one-element set today, and expressed as a set anyway: the shape is what
/// #788's and #789's reviews both landed on — the gate is over the *arm's whole
/// requirement*, so adding a second scalar later cannot leave a half-gated arm
/// behind. Everything else the phase needs (which shield is down, which arc the
/// tubes cover, whether a salvo is still in flight) is a host reading, not an
/// authored constant.
pub(crate) const TORPEDO_BEARING_PARAMS: &[&str] = &[TORPEDO_BEARING_SPEED_PARAM];

/// Does this Steering policy author the complete torpedo-bearing scalar set?
///
/// The sibling of [`recovery_params_authored`] and
/// [`combat_orbit_params_authored`], and separate from both on purpose: a hull
/// may fly a torpedo opportunity out of a combat orbit, out of a fly-through
/// pass, or out of nothing at all.
fn torpedo_bearing_params_authored(params: &crate::world::flags::AiParams) -> bool {
    TORPEDO_BEARING_PARAMS
        .iter()
        .all(|name| params.get(name).is_some())
}

// ── Authored artillery-position params (issue #792) ──────────────────────────
//
// The artillery platform's own three scalars, read off the STEERING policy for
// the same reason every other leg's are: Steering's yaw verb is what tells the
// host which leg is being flown. There is no default for any of them anywhere in
// Rust, and the host gates the arm on ALL THREE together — see
// [`ARTILLERY_PARAMS`].

/// The outer edge of the artillery envelope, world units. Beyond it the doctrine
/// stops holding and starts repositioning.
///
/// AUTHORED, and deliberately not derived from the bank's own `range`. The two
/// are related but they are not the same statement: `range` is where a bolt
/// stops existing, and this is where a designer decided the gun line is no longer
/// worth holding. Deriving one from the other would silently re-tune the
/// manoeuvre every time a weapon was rebalanced.
pub(crate) const MAX_ARTILLERY_RANGE_PARAM: &str = "max_artillery_range";
/// The inner edge: repositioning stops here and the firing position is taken.
/// Authored BELOW [`MAX_ARTILLERY_RANGE_PARAM`], and the gap between the two IS
/// the hysteresis — one threshold would have the hull chattering between closing
/// and holding every time the target drifted across it.
pub(crate) const ARTILLERY_HOLD_RANGE_PARAM: &str = "artillery_hold_range";
/// Throttle fraction flown while the firing position is held.
///
/// Its own name rather than a reuse of [`TORPEDO_BEARING_SPEED_PARAM`] or
/// [`REENGAGE_SPEED_PARAM`] for the reason those two are distinct from each
/// other: the value a hull wants here is very often `0.0`, and `0.0` is exactly
/// the value that cannot be distinguished from "unauthored" unless the gate asks
/// for the NAME.
pub(crate) const ARTILLERY_HOLD_SPEED_PARAM: &str = "artillery_hold_speed";

/// Every scalar the artillery-position arm needs, gated as ONE unit by
/// [`artillery_params_authored`].
///
/// All three, for the reason #788's, #789's and #790's reviews all landed on: a
/// gate that requires only some of the params an arm needs admits a
/// partially-authored hull into an arm it half-flies. A hull authoring the hold
/// throttle but neither range would hold station wherever it happened to be; one
/// authoring the ranges but not the throttle would take the firing position at an
/// invented zero and never be told it had. Declining the whole arm leaves the
/// hull flying ordinary doctrine travel, which is a behaviour a designer can
/// actually see.
///
/// Note the two range params are ALSO structurally required, because the
/// doctrine's own transition guards reference them by name and content
/// validation rejects an undeclared `param(...)` at load. That is a second lock
/// on the same door rather than a reason to leave this one open: the gate is over
/// the arm's whole requirement, so a future hull that reads a range host-side
/// without guarding on it cannot leave a half-gated arm behind.
pub(crate) const ARTILLERY_PARAMS: &[&str] = &[
    MAX_ARTILLERY_RANGE_PARAM,
    ARTILLERY_HOLD_RANGE_PARAM,
    ARTILLERY_HOLD_SPEED_PARAM,
];

/// Does this Steering policy author the complete artillery scalar set?
///
/// The sibling of [`recovery_params_authored`], [`combat_orbit_params_authored`]
/// and [`torpedo_bearing_params_authored`], and separate from all three on
/// purpose: an artillery platform has no shield-recovery doctrine, no ring and no
/// torpedo tubes.
fn artillery_params_authored(params: &crate::world::flags::AiParams) -> bool {
    ARTILLERY_PARAMS
        .iter()
        .all(|name| params.get(name).is_some())
}

/// The lead speed the artillery hold predicts with: the flight speed of the
/// hull's longest-reaching blaster bank (issue #792).
///
/// A HOST reading of the ship's own armament rather than an authored duplicate of
/// it, for the same reason [`SAFE_RANGE_FACT`] is derived rather than authored: a
/// second copy of a weapon's flight speed is a number that can silently disagree
/// with the weapon. The artillery piece is by construction the hull's
/// longest-reaching direct-fire bolt — that is what makes the standoff a standoff
/// — so "longest range" is the selector, and a hull with no blaster bank at all
/// reads `0.0`, which [`crate::ai::plan_artillery_position`] degrades to aiming at
/// the target's live position rather than at an invented intercept.
fn artillery_lead_speed(banks: &[crate::blaster::BlasterSystem]) -> f32 {
    banks
        .iter()
        .max_by(|a, b| a.config.range.total_cmp(&b.config.range))
        .map(|bank| bank.config.projectile_speed)
        .unwrap_or(0.0)
}

// ── Torpedo-opportunity facts (issue #791) ───────────────────────────────────
//
// SCOPE, and it is the same narrow one the recovery and pressed facts have:
// these two are seeded by `seed_torpedo_opportunity_facts`, which only
// `ai_policy_state_tick` calls. They are therefore available to TRANSITION
// guards and NOT to a state's continuous RULE guards, which the per-axis
// actuator hosts resolve from their own snapshot. Author them in transitions;
// the shipped cruiser doctrine does, and every rule it authors is unconditional.

/// `1.0` when the ONE shield arc of the current target that faces this ship is
/// down — offline, or absent because the target carries no shield system at all
/// — and `0.0` when it is online and blocking.
///
/// Transition-scope only — see the note above.
///
/// Resolved through the SAME path damage takes and the same one
/// `ai_torpedo_auto_fire` gates on: the target's live `Transform` + its own
/// `ShipShields`, through [`crate::shield::attacker_bearing_relative`] and then
/// the target's own `facing_index_for_bearing`. That resolver is
/// priority-tiered, so a hull that authors overlapping arcs routes the AI's
/// belief and the eventual hit to the same arc. Deriving the arc any other way
/// would let the manoeuvre commit to an opportunity the shot cannot take.
///
/// Deliberately ABSENT (not zero) when the helm has no target at all, so a
/// `fact(target_facing_shield_down) > 0` guard reads false rather than firing on
/// nothing. It reads `0.0` — "no opportunity" — when the target is live but
/// cannot be resolved to an entity carrying a transform (an asteroid, say):
/// unknowable is treated as closed, so the guard that OPENS the phase reads
/// false and the phase is never entered on a target nothing is known about.
///
/// ## This fact is not, and cannot be, a phase bound
///
/// Note carefully what the paragraph above does NOT say. Unknowable-is-closed
/// keeps the phase from opening; it does nothing to end one already open,
/// because the case that traps a hull is the opposite one. A target that
/// RESOLVES but carries no `[shields]` at all — a station, a probe, a hull
/// authored without the block — reads `1.0` here, correctly and permanently:
/// there is genuinely no arc in the way and there never will be. A doctrine
/// whose only way back out of the bow hold were "this fact went to zero" would
/// hold its bow on such a target until one of them died. That is why the shipped
/// cruiser's resume guards do not rest on this fact alone — see
/// [`TUBES_FULL_FACT`], which bounds the phase on the hull's OWN armament and so
/// cannot depend on the target ever raising a shield.
pub(crate) const TARGET_FACING_SHIELD_DOWN_FACT: &str = "target_facing_shield_down";
/// How many of this ship's OWN torpedo rounds are still UNRESOLVED — airborne,
/// or already committed to a burst and waiting on its timer.
///
/// Transition-scope only — see the note above.
///
/// Read off the live [`crate::weapons_plugin::TorpedoSystemResource`] component,
/// NOT off `SystemBlackboard::TorpedoMagazine`: the blackboard is published in
/// `SimSet::Publish`, one whole tick after this system runs in `SimSet::Physics`,
/// so a doctrine gating on it would see a salvo it launched a tick after it
/// launched it — and, worse, would read "no salvo" on the launch tick itself.
/// This is the identical trap `ai_shield_focus` calls out for `ShipShields` vs
/// `ShieldsBlackboard`.
///
/// "Every projectile has hit, missed, or expired" covers the airborne half
/// exactly: `tick_torpedo_lifecycle` removes a round from `in_flight` on
/// detonation and on expiry alike, so there is one reading rather than three.
/// `ai_policy_state_tick` is ordered after both the launcher and the lifecycle
/// (see `ship_plugin`), so the count is this tick's settled one.
///
/// Always seeded, including as `0.0` for a hull with no torpedo system at all —
/// a ship cannot be held bow-on by a salvo it can never have fired.
///
/// ## Why `in_flight` alone is not the count
///
/// A burst launch puts its FIRST round in `in_flight` immediately and schedules
/// the rest as a [`crate::torpedo::TubeBurstState`], whose `pending` rounds are
/// not in `in_flight` until their timer elapses. `in_flight.len()` on its own
/// therefore under-reports a salvo mid-burst, and a doctrine reading `< 1`
/// releases the hull in the gap between the last airborne round resolving and
/// the next pending one launching. So this fact is `in_flight.len()` PLUS the
/// pending rounds of every live burst state.
///
/// That gap is not theoretical, and the arithmetic that once said it was is
/// worth recording as the mistake it was. The reasoning ran: `volley_max = 2`
/// and `burst_interval_secs = 0.35`, so the two rounds of a tube's burst are
/// 0.35 s apart, while a round at `speed = 45` needs ~0.9 s to cross the
/// 42-unit combat ring — the first round cannot resolve before the second is
/// airborne. It assumes the round has to fly the AUTHORED ring radius. It does
/// not: the cruiser enters the phase with thrust cut and the target closing, and
/// an instrumented `combat_test` run measured the first two rounds of a salvo
/// launching at t=172.10 and both resolving by t=172.33 — 0.23 s, well inside
/// the burst interval. `in_flight` hit zero with `pending` still at 2, the
/// salvo-spent guard fired, and the back half of the salvo launched in `orbit`
/// with the bow already swinging away: `|bearing| = 0.230` rad and `in_arc = 0`,
/// i.e. rounds thrown outside the tubes' 24-degree cone. Counting the pending
/// rounds here holds the hull bow-on instead — the same run measured the second
/// pair away at `|bearing| = 0.163` rad, `in_arc = 1`.
///
/// The lesson generalises past this hull: flight TIME is a function of the
/// closing geometry, not of the ring the doctrine authors, so no arrangement of
/// `speed`, `lifespan` and `burst_interval_secs` licenses reading only the
/// airborne half. A round that has been committed to is a round the hull owes
/// the manoeuvre, whether or not it has left the tube yet.
pub(crate) const TORPEDOES_IN_FLIGHT_FACT: &str = "torpedoes_in_flight";

/// `1.0` when this ship could still get every tube to `volley_max` — i.e. when
/// a WHOLE SALVO is still a reachable state — and `0.0` when it is not.
///
/// Transition-scope only — see the note above.
///
/// The slower half of the pair it forms with [`TUBES_FULL_FACT`], and it is a
/// STAY reading rather than an entry one. `tubes_full` is "the salvo is ready
/// this instant"; this is "the salvo is still a reachable state at all" — no
/// tube and not the magazine has been shot out, and there are enough rounds left
/// to top every tube up. A hull that has just fired fails `tubes_full` for the
/// whole of its 18 s reload and yet passes this the entire time, which is
/// exactly the distinction: the first says whether to break a firing geometry
/// NOW, the second says whether this hull is still in the torpedo business.
///
/// It is a phase BOUND as well as an entry conjunct, and for a case
/// [`TUBES_FULL_FACT`] cannot reach: a tube shot out mid-phase keeps the rounds
/// already loaded into it, so the loaded-count reading stays true for ever while
/// the launcher declines every shot. Against a target with no arc to raise that
/// traps the hull bow-on until something dies. Reachability is the reading that
/// notices, so the shipped cruiser conjoins it on an EXIT as well as on entry.
///
/// Which is why the shipped cruiser conjoins BOTH on entry and neither alone.
/// `tubes_full` on its own would let a hull with a destroyed tube open the phase
/// on a magazine-full coincidence; this on its own is what issue #791's first
/// round shipped, and it opens the phase throughout every reload window — 94% of
/// the resulting bow-on time was spent at a moment the launcher could not have
/// fired whatever the target's shield did. Together they read "a whole salvo is
/// loaded, and the battery that fired it is still intact".
///
/// Three things have to hold, and each is a reason the salvo is unreachable
/// rather than merely not-yet-reached:
///
/// * the hull HAS tubes. A tubeless hull reads `0.0`, not the vacuous truth an
///   `all`-over-nothing would give it;
/// * every tube and the magazine are ONLINE — not Disabled, not Destroyed.
///   Loading and firing both gate on the fine system, so one dead tube makes a
///   ship-wide `tubes_full` permanently false. Read as "the system is not
///   offline" (`accept_human_input || operate_ai`), the same reading
///   `handle_fire_torpedo` gates a launch on, so this stays a statement about
///   the hull and not about who is crewing it (AGENTS.md #6);
/// * the magazine holds at least
///   [`crate::torpedo::TorpedoSystem::salvo_shortfall`] rounds — the ones still
///   needed to top every tube up, over and above those already claimed for an
///   in-progress load.
///
/// Always seeded, including `0.0` for a hull with no torpedo system at all, for
/// the same reason [`TORPEDOES_IN_FLIGHT_FACT`] is: a doctrine that asks must
/// get an answer, and "no tubes" is a definite one.
pub(crate) const TUBES_FILLABLE_FACT: &str = "tubes_fillable";

/// `1.0` when EVERY tube on this ship is at its `volley_max` right now — a whole
/// salvo loaded and ready to leave — and `0.0` otherwise.
///
/// Transition-scope only — see the note above.
///
/// The launcher's question, seeded helm-side so a MANOEUVRE can ask it too. It
/// is deliberately the identical reading `ai_torpedo_auto_fire` computes for the
/// `torpedo_launch` channel's fact of the same name
/// (`tubes.iter().all(|t| t.loaded_count >= t.volley_max)`), because the two
/// halves of a salvo doctrine have to agree: the helm gives up a firing geometry
/// to create the shot, and the launcher takes it. If the helm asked a weaker
/// question than the launcher, it would spend the geometry on windows the
/// launcher was always going to decline.
///
/// That is precisely what happened when the entry guard asked
/// [`TUBES_FILLABLE_FACT`] alone. Reachability stays true through the initial
/// load-up and through the whole 18 s reload after every salvo
/// (`load_time = 9.0` × `volley_max = 2`), so the cruiser broke its ring on
/// every arc collapse in those windows with nothing loadable inside them.
/// Measured over a 400 sim-second `combat_test` run: 506 ticks bow-on against
/// 431 orbiting, and only 29 of the 506 — 5.7% — with the tubes actually full.
///
/// It is ALSO what bounds the phase, and that second job is why it is a fact
/// rather than a detail of the entry guard. A hull that has fired fails this for
/// its whole reload, so a resume guard conjoining it is guaranteed to fire once
/// the salvo resolves — no matter what the target's shields do, and in
/// particular for a resolvable target with no `[shields]` block at all, whose
/// [`TARGET_FACING_SHIELD_DOWN_FACT`] is permanently `1.0`.
///
/// What it does NOT bound is a battery that stops working with the rounds still
/// in it. This reads the ROUNDS: destroying a tube leaves its `loaded_count`
/// untouched, so a hull that loses a tube mid-phase still reads `1.0` here for
/// ever while `handle_fire_torpedo` declines every launch. That case is
/// [`TUBES_FILLABLE_FACT`]'s, and it is why the shipped cruiser carries a
/// reachability resume beside this one rather than only a reachability entry
/// guard.
///
/// A hull with no tubes reads `0.0`, not the vacuous truth `all` over an empty
/// battery would give it — the same treatment [`TUBES_FILLABLE_FACT`] gets, and
/// for the same reason.
pub(crate) const TUBES_FULL_FACT: &str = "tubes_full";

/// Private-memory slot: which way round the current orbit runs, `+1.0` or
/// `-1.0` (issues #788, #790).
///
/// Host-written on the tick an ORBITING state is entered — the shield-recovery
/// standoff (#788) or the combat broadside ring (#790) — from
/// [`crate::composite_rng::signed_choice`] over a (world, ship, system,
/// transition, occurrence) key, so the choice is reproducible for a given seed
/// and yet is not the same every time, and two ships entering an orbit on the
/// same tick do not both break the same way. Read back by the host when it
/// builds [`HelmPassSurface`]; no authored guard reads it.
///
/// ONE slot for both orbit legs, deliberately: a ship circles one way at a time,
/// and the two legs are mutually exclusive (a state resolves exactly one yaw
/// verb). What differs between them is the RADIUS, and that has its own field.
pub(crate) const ORBIT_DIRECTION_MEMORY: &str = "orbit_direction";
/// Private-memory slot: how many times this machine has entered an orbiting
/// state since its last reset (issues #788, #790).
///
/// The OCCURRENCE field of the orbit-direction seed key, and another
/// host-written counter in the `memory(min_range_seen)` /
/// `memory(peak_hazard_urgency)` family: the host owns the quantity, the policy
/// would own any decision made from it. It is what stops a ship that orbits
/// twice against the same target from picking the same direction both times.
pub(crate) const ORBIT_OCCURRENCES_MEMORY: &str = "orbit_occurrences";

/// The composite-seed SYSTEM key for the Steering fine system (issue #788).
///
/// A stable string rather than the `SystemId`, so the derived value cannot move
/// when an unrelated registry detail changes. It is part of the reproducibility
/// contract in exactly the way `SimStream::name` is.
pub(crate) const STEERING_SEED_SYSTEM_NAME: &str = "helm-steering";

/// Private-memory slot: how many times this machine has entered a state that
/// engages boost, since its last reset (issue #882).
///
/// Written by [`ai_policy_state_tick`] — the HOST — and read by authored
/// guards as `memory(engagements)`. Host-writes / policy-reads is the same
/// split #779 and #780 use for continuous magnitudes: the host owns the
/// quantity, the policy owns the decision made from it. There is deliberately
/// no authored *write* verb; a policy cannot mutate its own memory.
pub(crate) const ENGAGEMENTS_MEMORY: &str = "engagements";

/// Private-memory slot: the highest hazard urgency this ship has seen since
/// the policy state last reset (issue #882), read as
/// `memory(peak_hazard_urgency)`.
///
/// A running aggregate over ticks — the shape issue #883's closest-approach
/// detector needs (`min_range_seen`, `prev_closing_rate`) — and the reason
/// memory is not just a second name for `param`: no authored constant and no
/// single-tick fact can express it.
pub(crate) const PEAK_HAZARD_MEMORY: &str = "peak_hazard_urgency";

/// The tick-derived clock the policy state machines measure `state_time`
/// against (issue #882, AC4).
///
/// Advanced by exactly one increment of the authored AI-helm tick period each
/// time [`ai_policy_state_tick`] runs — and that system runs under
/// `run_if(ai_helm_tick_ready)`, the shared fixed-rate latch. It is therefore
/// derived from the shared AI tick cadence and NOT from `Time::delta`: a 144 Hz
/// host and a 60 Hz host advance policy state time identically, which is the
/// whole point of the #803 latch and of PRD #620's determinism goal. Issue #784
/// retired the last per-frame AI timer; nothing here reintroduces one.
#[derive(Resource, Default)]
pub(crate) struct AiPolicyTickClock(pub(crate) f64);

/// The mutable per-ship policy runtime [`ai_policy_state_tick`] owns, bundled as
/// one `QueryData` (issue #788).
///
/// Bundled because Bevy's query tuples cap out and that system already carries
/// most of a ship's helm configuration, but also because these five components
/// are one thing: the runtime state of a ship's helm policy machines plus the
/// two surfaces derived from them. Nothing else writes any of them, so there is
/// exactly one writer for the whole bundle.
#[derive(bevy::ecs::query::QueryData)]
#[query_data(mutable)]
pub(crate) struct HelmPolicyRuntime {
    engines: &'static mut HelmEnginesAiPolicyState,
    steering: &'static mut HelmSteeringAiPolicyState,
    boost: &'static mut HelmBoostAiPolicyState,
    pass: &'static mut HelmPassSurface,
    recovery: &'static mut HelmRecoveryHistory,
}

/// Advance every stateful fine-system policy's state machine, ONCE per shared
/// AI tick, and COMMIT the entered state before any output resolves this tick
/// (issue #882).
///
/// Ordering (declared in `ship_plugin.rs`): `.after(helm_motion_planner)` so
/// the hazard surface a transition guard reads is this tick's, and `.before`
/// the per-axis actuator systems so the state they resolve their continuous
/// outputs in is the state committed here — AC2's "the resulting state supplies
/// continuous outputs immediately in the same tick". Runs under the same
/// `run_if(ai_helm_tick_ready)` latch as those systems.
///
/// AC2's other half — at most ONE transition per eligible tick — is not
/// enforced here at all: [`crate::ai::policy::AiPolicy::resolve_transition`]
/// returns an `Option`, so this host has no way to chain two.
///
/// AC5 reset: a ship whose Boost system is not AI-operated, or whose boost
/// capability is absent/disabled, is reset to `initial` every tick it stays
/// that way. So the tick AI *gains* control — and the tick an unavailable
/// system *recovers* — begins from the initial state with authored memory,
/// never resuming a stale mid-manoeuvre state.
///
/// ## This host is also the WRITER of this fine system's private memory
///
/// There is no authored write verb and there never will be: a policy READS
/// `memory(name)`, the host WRITES it. That is the same split #779/#780 use for
/// continuous magnitudes (the planner owns the number, the policy owns the
/// decision), and it is what makes memory more than a second spelling of
/// `param` — the values are retained across ticks and only
/// [`crate::ai::policy::AiPolicyRuntimeState::reset`] puts them back to their
/// authored declarations. Two slots are written here, both named by the host,
/// neither knowable from a single tick's facts:
///
/// * [`PEAK_HAZARD_MEMORY`] — a running maximum, folded every gated tick. This
///   is the shape issue #883's closest-approach detector needs.
/// * [`ENGAGEMENTS_MEMORY`] — incremented when a committed transition enters a
///   state whose OWN rules engage boost. The host asks the policy what the
///   entered state does on this system's channel, so the counter needs no
///   knowledge of authored state names.
///
/// Issue #883 adds the two travel axes and two more host-written slots, folded
/// for EVERY machine by [`tick_policy_machine`]:
///
/// * [`MIN_RANGE_SEEN_MEMORY`] — a running MINIMUM of `range_to_target`, scoped
///   to the current state (the host resets it on every commit). Closest approach
///   is then "the range has re-opened past the authored hysteresis", which one
///   tick of retention is exactly enough to know and no single-tick fact can say.
/// * [`ESCAPE_HEADING_MEMORY`] — the ship's yaw at the instant a transition
///   commits, so the state that was just entered can fly a heading frozen at the
///   merge rather than a heading that keeps being re-solved.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_policy_state_tick(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    // The run's master seed — the WORLD field of the orbit-direction composite
    // key (issue #788). `Option` for the same reason every other simulation
    // system takes it optionally: a bare `Res` fails Bevy parameter validation
    // in every bare-`App` fixture in this crate. Absent resolves to seed 0,
    // which is still deterministic.
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    mut clock: ResMut<AiPolicyTickClock>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            // Optional for the same reason the per-axis hosts take them
            // optionally: a bare-`App` fixture may attach only the policy it is
            // testing. A ship missing one falls back to that axis's canonical
            // default, which is stateless, so its machine tick returns
            // immediately. Requiring all three here would instead make the whole
            // QUERY fail to match and silently skip the ship — the same class of
            // silent skip #883 added the `resolve_helm_channel` guard for.
            Option<&HelmEnginesAiPolicy>,
            Option<&HelmSteeringAiPolicy>,
            Option<&HelmBoostAiPolicy>,
            // This ship's OWN shields (issue #788). Read-only here — `tick_shields`
            // is the single writer — so this adds no ordering question, only a
            // reading that may be one tick old.
            Option<&crate::ship::shields::ShipShields>,
            // This ship's OWN tubes and rounds in flight (issue #791). Read-only,
            // and unlike the shields above this one DOES carry an ordering
            // question — `handle_fire_torpedo` appends to `in_flight` and
            // `tick_torpedo_lifecycle` removes from it, both in `SimSet::Physics`
            // — so `ship_plugin` pins this system after both of them.
            Option<&crate::weapons_plugin::TorpedoSystemResource>,
            // This ship's OWN blaster banks (issue #792), read for their authored
            // `range`/`projectile_speed` alone. Bank CONFIG never changes at
            // runtime, so unlike the tubes above this carries no ordering
            // question — no system in the schedule writes the field this reads.
            Option<&crate::weapons_plugin::BlasterSystemResource>,
            // The SHIP field of the orbit-direction composite key.
            Option<&crate::entity_spawner::EntityUuid>,
            HelmPolicyRuntime,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
    // Every entity a target uuid could name, for the facing-shield reading
    // (issue #791). The same shape `ai_torpedo_auto_fire` resolves its own
    // striking arc through, and read-only, so it conflicts with nothing the
    // ship query above mutates.
    targets: Query<(
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::ship::shields::ShipShields>,
        Option<&ShipPhysics>,
    )>,
) {
    // One authored tick period per run — the shared cadence, never Time::delta.
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_helm_tick_hz)
        .unwrap_or_else(|| crate::entity_config::GlobalConfig::default().ai_helm_tick_hz);
    if hz > 0.0 {
        clock.0 += 1.0 / hz as f64;
    }
    let now = clock.0;

    // Canonical (stateless) fallbacks for a ship missing an attached policy;
    // built once per tick, mirroring the per-axis hosts.
    let default_engines = crate::entities::config::default_engines_ai_config()
        .to_policy()
        .unwrap_or_default();
    let default_steering = crate::entities::config::default_steering_ai_config()
        .to_policy()
        .unwrap_or_default();
    let default_boost = crate::entities::config::default_boost_ai_config()
        .to_policy()
        .unwrap_or_default();

    let world_seed = sim_rng.as_deref().map(|r| r.seed()).unwrap_or(0);

    for (
        entity,
        sources,
        physics,
        physics_cfg,
        boost_cfg,
        impulse_cfg,
        engines_policy,
        steering_policy,
        boost_policy,
        shields,
        torpedoes,
        blasters,
        entity_uuid,
        mut runtime,
    ) in ships.iter_mut()
    {
        let engines_policy = engines_policy.map(|p| &p.0).unwrap_or(&default_engines);
        let steering_policy = steering_policy.map(|p| &p.0).unwrap_or(&default_steering);
        let boost_policy = boost_policy.map(|p| &p.0).unwrap_or(&default_boost);

        // One fact snapshot per ship per tick, shared by all three machines —
        // they must reason about the SAME world or they would reach different
        // legs. Private memory is what stays per-system (AC3): the derived
        // memory fact is folded in separately, inside each machine's own tick.
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            impulse_cfg.is_some(),
            boost_cfg.map(|c| c.enabled).unwrap_or(false),
            physics.y,
        );
        // The identity of the target the geometry above was seeded from. The
        // running range minimum is scoped to it, so a mid-state target switch
        // restarts the fold rather than inheriting a stranger's minimum.
        let travel_target = seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        // Shield-recovery readings (issue #788): own shield fraction, the derived
        // safe ring, and the bounded distance history's verdict. Read off the
        // STEERING policy's params — the axis that owns the recovery legs — so
        // all three machines see one consistent ring.
        seed_recovery_facts(
            &mut facts,
            &steering_policy.params,
            shields.map(|s| s.0.fraction()),
            &mut runtime.recovery,
            travel_target,
        );
        // Pressed readings (issue #789): the SECOND bounded window's separation
        // trend and the "inside the target's own reach" comparison. Folded here,
        // once per shared tick, after the target scope above has been settled.
        seed_pressed_facts(&mut facts, &steering_policy.params, &mut runtime.recovery);
        // Torpedo-opportunity readings (issue #791): whether the ONE shield arc
        // of the target that faces us is down, how many of our own rounds are
        // still in the air, and whether a whole salvo is still reachable at all.
        // All three are pure world readings with no authored threshold, so they
        // are seeded for every hull; a hull whose doctrine never asks simply
        // never reads them.
        seed_torpedo_opportunity_facts(
            &mut facts,
            travel_target,
            physics,
            &targets,
            torpedoes.map(|t| &t.0),
            sources,
        );

        // ── Engines ──────────────────────────────────────────────────────────
        tick_policy_machine(
            engines_policy,
            &mut runtime.engines.0,
            sources
                .0
                .policy_for(&crate::system_registry::helm_thrust_system_id())
                .operate_ai,
            &facts,
            travel_target,
            now,
            physics.yaw,
            |_| {},
        );

        // ── Steering ─────────────────────────────────────────────────────────
        let steering_entered = tick_policy_machine(
            steering_policy,
            &mut runtime.steering.0,
            sources
                .0
                .policy_for(&crate::system_registry::helm_steering_system_id())
                .operate_ai,
            &facts,
            travel_target,
            now,
            physics.yaw,
            |_| {},
        );
        if let Some(entered) = steering_entered {
            // Entering an ORBITING state draws that orbit's circulation direction
            // (issues #788, #790). The host asks the policy what the state it
            // just entered does on this system's own channel, exactly as the
            // boost engagement counter below does, so this needs no knowledge of
            // authored state names.
            //
            // BOTH orbit verbs draw, and they must: the shield-recovery standoff
            // and the combat broadside ring each need a definite side to circle
            // on, and gating the draw on one of them would leave the other
            // reading whatever the hull happened to declare — a constant, which
            // is precisely what a seeded choice exists to avoid.
            let orbits = matches!(
                resolve_helm_channel(
                    steering_policy,
                    Some(&runtime.steering.0),
                    crate::entities::config::HELM_YAW_CHANNEL,
                    &facts,
                    now,
                ),
                Some(
                    &crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit
                        | &crate::ai::policy::AiPolicyVerb::HoldCombatOrbit
                )
            );
            if orbits {
                let occurrence = runtime
                    .steering
                    .0
                    .memory
                    .get(ORBIT_OCCURRENCES_MEMORY)
                    .unwrap_or(0.0)
                    + 1.0;
                runtime
                    .steering
                    .0
                    .memory
                    .set(ORBIT_OCCURRENCES_MEMORY, occurrence);
                // Deterministic from (world, ship, system, transition,
                // occurrence) and from nothing else — no `Time`, no frame count,
                // no OS entropy. Two runs of the same seeded scenario break the
                // same way; two ships breaking off on the same tick do not.
                let key = crate::composite_rng::CompositeKey {
                    world: world_seed,
                    ship: entity_uuid
                        .and_then(|u| uuid::Uuid::parse_str(&u.0).ok())
                        .map(|u| u.as_u128() as u64)
                        .unwrap_or_else(|| entity.to_bits()),
                    system: crate::composite_rng::key_from_name(STEERING_SEED_SYSTEM_NAME),
                    transition: crate::composite_rng::key_from_name(&entered),
                    occurrence: occurrence as u64,
                };
                runtime.steering.0.memory.set(
                    ORBIT_DIRECTION_MEMORY,
                    crate::composite_rng::signed_choice(&key),
                );
            }
        }

        // ── Boost ────────────────────────────────────────────────────────────
        // Availability is part of this axis's AC5 reset gate: an absent or
        // feature-disabled boost holds the machine at `initial`.
        let boost_operable = sources
            .0
            .policy_for(&crate::system_registry::helm_boost_system_id())
            .operate_ai
            && boost_cfg.map(|c| c.enabled).unwrap_or(false);
        let entered = tick_policy_machine(
            boost_policy,
            &mut runtime.boost.0,
            boost_operable,
            &facts,
            travel_target,
            now,
            physics.yaw,
            // Boost's own extra host-written slot (issue #882): a running
            // maximum of the hazard faced since the last reset.
            |memory| {
                let urgency = facts
                    .get(HAZARD_URGENCY_FACT)
                    .unwrap_or(0.0)
                    .max(memory.get(PEAK_HAZARD_MEMORY).unwrap_or(0.0));
                memory.set(PEAK_HAZARD_MEMORY, urgency);
            },
        );
        if entered.is_some() {
            // Count the entries into a boost-ENGAGING state. The host asks the
            // policy what the state it just entered does on this system's own
            // channel, so the counter needs no knowledge of authored state
            // names: any content whose entered state engages boost increments
            // it. This survives the transition that produced it and every
            // later tick, and only `AiPolicyRuntimeState::reset` clears it back
            // to the authored declaration — which is the property that makes
            // `memory(...)` different from `param(...)`.
            let engages = resolve_helm_channel(
                boost_policy,
                Some(&runtime.boost.0),
                crate::entities::config::HELM_BOOST_CHANNEL,
                &facts,
                now,
            ) == Some(&crate::ai::policy::AiPolicyVerb::EngageBoost);
            if engages {
                let n = runtime
                    .boost
                    .0
                    .memory
                    .get(ENGAGEMENTS_MEMORY)
                    .unwrap_or(0.0);
                runtime.boost.0.memory.set(ENGAGEMENTS_MEMORY, n + 1.0);
            }
        }

        // ── Publish the derived fly-through pass surface (issues #883, #788) ──
        let surface = build_pass_surface(
            engines_policy,
            steering_policy,
            &runtime.steering.0,
            sources,
            &facts,
            now,
            // The artillery lead speed (issue #792): a reading of this hull's own
            // longest-reaching bolt, resolved here so the pure planner never has
            // to know what a blaster bank is.
            blasters.map(|b| artillery_lead_speed(&b.0)).unwrap_or(0.0),
        );
        *runtime.pass = surface;
    }
}

/// Advance ONE fine system's policy machine by one shared AI tick (issue #883,
/// generalised from #882's Boost-only body).
///
/// Returns the state entered when a transition committed this tick.
///
/// Everything a *fly-through* needs beyond #882 is the two host-written slots
/// folded here, and both are deliberately generic rather than doctrine-specific:
///
/// * [`MIN_RANGE_SEEN_MEMORY`] is a running minimum **scoped to the current
///   state AND to the current target's identity**. The host resets it to the
///   current range on every commit, and restarts the fold whenever `target`
///   differs from the one it was accumulated against
///   ([`MIN_RANGE_TARGET_MEMORY`]). The state scoping is what lets a machine
///   cycle through repeated attack runs without the host knowing a single
///   authored state name; the identity scoping is what stops a mid-state target
///   switch — to a further ship, say — synthesising a `range_above_min_seen`
///   spike out of the previous target's minimum and firing a closest approach
///   the ship never flew.
/// * [`ESCAPE_HEADING_MEMORY`] is written on every commit, so any state can be
///   authored to fly the heading captured at the transition that entered it.
///
/// A stateless policy — every hull that ships today — returns immediately
/// without touching memory, state, or the transition scan.
#[allow(clippy::too_many_arguments)]
fn tick_policy_machine<F>(
    policy: &crate::ai::policy::AiPolicy,
    state: &mut crate::ai::policy::AiPolicyRuntimeState,
    operable: bool,
    facts: &crate::world::flags::AiFacts,
    // The target this tick's `range_to_target` was seeded from, as returned by
    // `seed_helm_travel_facts`.
    target: Option<uuid::Uuid>,
    now: f64,
    yaw: f32,
    fold_extra_memory: F,
) -> Option<String>
where
    F: FnOnce(&mut crate::world::flags::AiPolicyMemory),
{
    // Stateless policies never enter any of this.
    policy.machine()?;
    // AC5: not AI-operated, or the system is unavailable → hold at initial, so
    // the tick AI *gains* control begins from the authored initial state rather
    // than resuming a stale mid-manoeuvre one.
    if !operable {
        *state = crate::ai::policy::AiPolicyRuntimeState::reset(policy, now);
        return None;
    }
    // A state component that was never initialised (or whose authored machine
    // changed) starts at `initial`.
    if policy
        .machine()
        .and_then(|m| m.state(&state.current))
        .is_none()
    {
        *state = crate::ai::policy::AiPolicyRuntimeState::reset(policy, now);
    }

    fold_extra_memory(&mut state.memory);

    // Running minimum of the range, scoped to the state AND to the target's
    // identity (see the doc comment). A target switch restarts the fold at this
    // tick's range: carrying the previous target's minimum forward would let a
    // swap to a further ship read as a huge `range_above_min_seen` and fire a
    // closest approach that never happened.
    if let Some(range) = facts.get(RANGE_TO_TARGET_FACT) {
        if facts.get(TARGET_VALID_FACT).unwrap_or(0.0) > 0.0 {
            let fingerprint = target.map(target_identity_fingerprint);
            let same_target = fingerprint == state.memory.get(MIN_RANGE_TARGET_MEMORY);
            let folded = if same_target {
                state
                    .memory
                    .get(MIN_RANGE_SEEN_MEMORY)
                    .map_or(range, |min| min.min(range))
            } else {
                range
            };
            state.memory.set(MIN_RANGE_SEEN_MEMORY, folded);
            if let Some(fingerprint) = fingerprint {
                state.memory.set(MIN_RANGE_TARGET_MEMORY, fingerprint);
            }
        }
    }

    // The private bag is seeded from THIS fine system's own state component and
    // nothing else (AC3) — including the memory-derived fact.
    let mut facts_with_memory = facts.clone();
    seed_memory_derived_facts(&mut facts_with_memory, &state.memory);
    let memory = state.memory_at(now);
    let to = policy
        .resolve_transition(&state.current, &facts_with_memory, &memory, &[])?
        .to
        .clone();
    state.enter(&to, now);

    // Commit-time host writes. The heading is captured from THIS tick's yaw, so
    // "the current outward heading" means the heading at the merge instant.
    state.memory.set(ESCAPE_HEADING_MEMORY, yaw as f64);
    // Re-scope the running minimum to the state just entered.
    if let Some(range) = facts.get(RANGE_TO_TARGET_FACT) {
        state.memory.set(MIN_RANGE_SEEN_MEMORY, range);
    }
    Some(to)
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
fn build_pass_surface(
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
) -> HelmPassSurface {
    let travel_axes_ai = sources
        .0
        .policy_for(&crate::system_registry::helm_thrust_system_id())
        .operate_ai
        && sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
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

/// Resolve a helm fine-system policy's single mode channel to a bare "actuate
/// this tick?" boolean (issue #779).
///
/// The policy is a pure fact→verb(mode) map: it returns the channel's mode verb
/// when a guard fires and `None` ("hold") otherwise. This collapses that to the
/// host's decision — emit the planner-decoded scalar, or emit nothing — without
/// letting the continuous magnitude leak into the policy. `expected` is the mode
/// verb this channel is allowed to carry (validated at load), so a mismatched
/// verb resolves to "hold" defensively rather than actuating on a wrong axis.
fn helm_policy_actuates(
    policy: &crate::ai::policy::AiPolicy,
    channel: &str,
    facts: &crate::world::flags::AiFacts,
    expected: &crate::ai::policy::AiPolicyVerb,
) -> bool {
    policy.resolve_channel(channel, facts, &[]) == Some(expected)
}

/// Seed the per-tick policy fact snapshot for a helm actuator host (issue
/// #780). This is THE piece that resolves the #779 empty-facts sharp edge: a
/// host that passes an empty `AiFacts` leaves every `fact(...)` guard validating
/// at load and then never firing, so each host seeds hazard and
/// capability/availability facts here so authored guards (AC5/AC6) actually
/// evaluate. #883 closed the last gap by routing the two travel-axis hosts
/// through this seeder as well, so no helm host resolves against an empty
/// snapshot. Facts are read from the shared `HazardAssessment` the planner
/// already published — no re-scan (AC2) — and from host-side capability, keeping
/// `policy.rs` Bevy-free (AGENTS.md #10).
fn seed_helm_actuator_facts(
    hazard: Option<&crate::ship::helm_planner::HazardAssessment>,
    impulse_available: bool,
    boost_available: bool,
    vertical_offset: f32,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    let (urgency, moving_threat) = hazard
        .map(|h| (h.urgency, h.moving_hazard_threat))
        .unwrap_or((0.0, 0.0));
    facts.set(HAZARD_URGENCY_FACT, urgency as f64);
    facts.set("moving_hazard_threat", moving_threat as f64);
    facts.set("hazard_present", if urgency > 0.0 { 1.0 } else { 0.0 });
    facts.set(
        "impulse_available",
        if impulse_available { 1.0 } else { 0.0 },
    );
    facts.set("boost_available", if boost_available { 1.0 } else { 0.0 });
    facts.set("vertical_offset", vertical_offset as f64);
    facts
}

/// Seed the TARGET-RELATIVE travel facts (issue #883, AC5) — the sibling of
/// [`seed_helm_actuator_facts`] that closes the #779 empty-facts gap for the two
/// travel axes.
///
/// The target is the same one the helm already pursues (`destroy_target` falling
/// back to the Weapons combat lock), resolved against the frame's MERGED view —
/// the same surface `helm_ai_decision` steers by, so a guard can never fire on a
/// target the travel solution cannot see. An unresolvable target seeds
/// `target_valid = 0` and no geometry at all, so a `fact(range_to_target) < …`
/// guard reads absent (false) rather than a stale or invented number.
///
/// [`crate::ai::AiWorldEntity`] has no velocity field, so the closing rate's
/// relative velocity is reconstructed from `(yaw, forward_speed)` for BOTH
/// parties inside the pure [`crate::ai::target_relative_motion`]; nothing about
/// that geometry lives here.
///
/// Returns the uuid of the target the geometry was actually seeded from, or
/// `None` when there was none to resolve. That is the identity
/// [`tick_policy_machine`] scopes its running range minimum to, and returning it
/// from here is what guarantees the two can never disagree about *which* target
/// this tick's `range_to_target` belongs to.
fn seed_helm_travel_facts(
    facts: &mut crate::world::flags::AiFacts,
    frame_ship: Option<&HelmAiShipFrame>,
    physics: &ShipPhysics,
    max_speed: f32,
) -> Option<uuid::Uuid> {
    // Always seeded, so an authored guard distinguishes "no target" from
    // "target at range 0" without either reading as absent.
    facts.set(TARGET_VALID_FACT, 0.0);
    if max_speed > 0.0 {
        facts.set(
            SPEED_FRACTION_FACT,
            (physics.forward_speed / max_speed) as f64,
        );
    }

    let sf = frame_ship?;
    let uuid = sf.destroy_target.or(sf.weapons_target)?;
    let target = sf.merged_view.entities.iter().find(|e| e.uuid == uuid)?;

    let motion = crate::ai::target_relative_motion(
        [physics.x, physics.y, physics.z],
        physics.yaw,
        physics.forward_speed,
        target.position,
        target.yaw,
        target.forward_speed,
    );
    facts.set(TARGET_VALID_FACT, 1.0);
    facts.set(RANGE_TO_TARGET_FACT, motion.range as f64);
    facts.set(CLOSING_RATE_FACT, motion.closing_rate as f64);
    facts.set(BEARING_TO_TARGET_FACT, motion.bearing_rad as f64);
    // How far the ship being fought can shoot (issue #788). Published on the
    // snapshot entity by `build_world_snapshot`, so it is a reading of the
    // TARGET's own online banks rather than a guess about its hull class.
    facts.set(
        TARGET_DIRECT_FIRE_RANGE_FACT,
        target.direct_fire_range as f64,
    );
    Some(uuid)
}

/// Fold the SHIELD-RECOVERY readings into the shared fact snapshot and advance
/// the bounded distance history (issue #788).
///
/// Called only from [`ai_policy_state_tick`], which is where the transitions
/// that read these facts are resolved. The per-axis actuator hosts deliberately
/// do NOT seed them: their job is resolving a *rule* inside an already-committed
/// state, and every recovery rule the doctrine authors is unconditional
/// (`when = "true"`), so nothing they resolve reads one. Seeding them there too
/// would mean folding the history four times a tick.
///
/// Returns the derived safe-ring radius when the hull authors a margin.
///
/// The window's capacity is re-applied every call because the authored value
/// lives on the policy, which the component (a plain `default()` at spawn)
/// cannot see; `BoundedHistory::set_capacity` is a no-op when unchanged, so this
/// cannot reset the window.
fn seed_recovery_facts(
    facts: &mut crate::world::flags::AiFacts,
    params: &crate::world::flags::AiParams,
    shield_fraction: Option<f32>,
    history: &mut HelmRecoveryHistory,
    target: Option<uuid::Uuid>,
) -> Option<f32> {
    // Absent (not zero) for a hull with no shield system — see the constant.
    if let Some(fraction) = shield_fraction {
        facts.set(SHIELD_FRACTION_FACT, fraction as f64);
    }

    let safe_range = params
        .get(SAFE_RANGE_MARGIN_PARAM)
        .map(|margin| facts.get(TARGET_DIRECT_FIRE_RANGE_FACT).unwrap_or(0.0) + margin);
    if let Some(range) = safe_range {
        facts.set(SAFE_RANGE_FACT, range);
    }

    if let Some(ticks) = params.get(SAFE_DISTANCE_WINDOW_TICKS_PARAM) {
        history.ranges.set_capacity(ticks.max(0.0).round() as usize);
    }
    // A target switch invalidates the history outright: distance held against a
    // ship that is no longer the threat says nothing about the one that is —
    // and neither does distance opened from it, so BOTH windows go (issue #789).
    if history.target != target {
        history.ranges.clear();
        history.separation.clear();
        history.target = target;
    }

    let target_valid = facts.get(TARGET_VALID_FACT).unwrap_or(0.0) > 0.0;
    let held = if !target_valid {
        // Nothing visible is shooting: the ship is trivially at a safe
        // distance from a threat it cannot see. Answering `false` here would
        // trap a destroyer whose target died mid-recovery in an orbit around
        // nothing, for ever.
        history.ranges.clear();
        true
    } else {
        match (
            facts.get(RANGE_TO_TARGET_FACT),
            safe_range,
            params.get(SAFE_RING_TOLERANCE_PARAM),
        ) {
            (Some(range), Some(safe), Some(tolerance)) => {
                history.ranges.push(range);
                history.ranges.all_at_least(safe - tolerance)
            }
            // A hull that authors no recovery params never holds — and never
            // authors a state that asks.
            _ => false,
        }
    };
    facts.set(SAFE_DISTANCE_HELD_FACT, if held { 1.0 } else { 0.0 });

    safe_range.map(|r| r as f32)
}

/// Fold the PRESSED readings into the shared fact snapshot and advance the
/// separation-progress history (issue #789).
///
/// Called from [`ai_policy_state_tick`] alone, immediately after
/// [`seed_recovery_facts`] — which owns the component's target scope and has
/// already cleared both windows if the target changed. Folding a second window
/// from the per-axis actuator hosts as well would advance it four times per
/// shared tick, so the authored `pressed_window_ticks` would silently mean a
/// quarter of the span it says.
///
/// Two facts, and the split between them is deliberate. The host derives the
/// *measurements* — a comparison of two facts, and a trend across a bounded
/// window, neither of which the predicate grammar can express — and the
/// doctrine still owns every *decision*: how much progress counts as escaping is
/// `param(pressed_min_progress)` in the hull's own TOML, not a number here
/// (AGENTS.md #11).
fn seed_pressed_facts(
    facts: &mut crate::world::flags::AiFacts,
    params: &crate::world::flags::AiParams,
    history: &mut HelmRecoveryHistory,
) {
    let target_valid = facts.get(TARGET_VALID_FACT).unwrap_or(0.0) > 0.0;
    let range = facts.get(RANGE_TO_TARGET_FACT);

    // "Effective player threat range" is the TARGET's own longest usable
    // direct-fire reach — the same reading the standoff ring is derived from, so
    // the two halves of the doctrine cannot disagree about how far the ship
    // being fought can shoot. Always seeded, so a guard distinguishes "outside
    // the threat" from "no reading" without either being absent.
    let inside = target_valid
        && range
            .map(|r| r <= facts.get(TARGET_DIRECT_FIRE_RANGE_FACT).unwrap_or(0.0))
            .unwrap_or(false);
    facts.set(INSIDE_THREAT_RANGE_FACT, if inside { 1.0 } else { 0.0 });

    // Decline rather than invent, on ALL TEN names together — the four in
    // [`PRESSED_PARAMS`] and the six in [`RECOVERY_PARAMS`]. A declining hull
    // keeps a zero-capacity window (no retention, no memory cost) and seeds no
    // progress fact at all, so every pressed guard reads false and the ordinary
    // recovery doctrine runs.
    //
    // The recovery six are load-bearing HERE, one level up from where they are
    // obviously needed, and that is the whole reason this gate is not just
    // `PRESSED_PARAMS`. The pressed pivot is flown as `FlyThroughLeg::Reengage`,
    // which the planner only reaches when `HelmPassSurface::reengage` is true,
    // and `build_pass_surface` only sets that when all six are authored. A hull
    // admitted into the pressed arm without them would enter `pressed_pivot` and
    // fall through to the INBOUND leg instead — a boosted, full-approach-throttle,
    // hard-turning run straight at the enemy, which is strictly worse than the
    // doctrine travel it would fly by declining. Nothing in content validation
    // ties the `pivot_to_reengage` verb to those scalars, so this is the check.
    if !recovery_params_authored(params)
        || !PRESSED_PARAMS.iter().all(|name| params.get(name).is_some())
    {
        history.separation.set_capacity(0);
        return;
    }
    // Re-applied every call for the same reason the recovery window's is: the
    // authored value lives on the policy, which the `default()` component at
    // spawn cannot see, and `set_capacity` is a no-op when unchanged.
    history.separation.set_capacity(
        params
            .get(PRESSED_WINDOW_TICKS_PARAM)
            .unwrap_or(0.0)
            .max(0.0)
            .round() as usize,
    );

    match (target_valid, range) {
        (true, Some(range)) => {
            history.separation.push(range);
            // Absent until the window is full: a partly-filled window measures a
            // shorter span than the authored one, so its progress reads low for
            // no reason but youth — and "low progress" is the pressed reading.
            if let Some(progress) = history.separation.net_change() {
                facts.set(SEPARATION_PROGRESS_FACT, progress);
            }
        }
        // Nothing visible to be escaping FROM. The window is emptied rather than
        // frozen, so a target that reappears is measured from scratch instead of
        // against a gap it never had, and the fact stays absent — a ship with no
        // target is not pressed by it.
        _ => history.separation.clear(),
    }
}

/// Fold the TORPEDO-OPPORTUNITY readings into the shared fact snapshot
/// (issue #791).
///
/// Called from [`ai_policy_state_tick`] alone, like the recovery and pressed
/// seeders, so all four facts are TRANSITION-scope (see their constants).
///
/// Four readings, and none carries an authored threshold — the doctrine owns
/// every decision made from them, in the hull's own TOML:
///
/// * [`TARGET_FACING_SHIELD_DOWN_FACT`] resolves the ONE arc of the target that
///   faces this ship through the target's OWN
///   [`crate::shield::ShieldSystem::facing_index_for_bearing`] — the same
///   priority-tiered resolver `apply_damage` routes a hit through, and the same
///   one `ai_torpedo_auto_fire` gates its launch on. Going through a parallel
///   view of the target's arcs would let the manoeuvre commit to an opportunity
///   the shot cannot take, which is a bug that shows up as a cruiser holding its
///   bow on a healthy shield for ever.
/// * [`TORPEDOES_IN_FLIGHT_FACT`] is the LIVE component reading, not the
///   blackboard's, and it counts the rounds a burst still owes alongside the
///   airborne ones — see the constant.
/// * [`TUBES_FULL_FACT`] is the READY-NOW reading — a whole salvo loaded —
///   computed exactly as `ai_torpedo_auto_fire` computes the launch channel's
///   fact of the same name, so the manoeuvre that spends a firing geometry and
///   the launcher that uses it ask one question and not two.
/// * [`TUBES_FILLABLE_FACT`] is the slower REACHABILITY reading beside it — see
///   the constant, and [`torpedo_tubes_fillable`] for how it is resolved.
fn seed_torpedo_opportunity_facts(
    facts: &mut crate::world::flags::AiFacts,
    target: Option<uuid::Uuid>,
    physics: &ShipPhysics,
    targets: &Query<(
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::ship::shields::ShipShields>,
        Option<&ShipPhysics>,
    )>,
    torpedoes: Option<&crate::torpedo::TorpedoSystem>,
    sources: &ShipSystemControlSources,
) {
    // A hull with no torpedo system reads zero rather than absent: it can never
    // be held bow-on by a salvo it could not have fired.
    //
    // Airborne rounds PLUS the rounds a live burst still owes. A burst launch
    // puts only its first round in `in_flight` and leaves the rest pending on a
    // timer, so the airborne count alone dips to zero mid-salvo and releases the
    // hull between rounds — see the constant for the measured trace.
    facts.set(
        TORPEDOES_IN_FLIGHT_FACT,
        torpedoes
            .map(|t| {
                t.in_flight.len() as u32 + t.burst_states.iter().map(|b| b.pending).sum::<u32>()
            })
            .unwrap_or(0) as f64,
    );
    // Likewise zero rather than absent for a hull that has no tubes to fill.
    facts.set(
        TUBES_FILLABLE_FACT,
        if torpedo_tubes_fillable(torpedoes, sources) {
            1.0
        } else {
            0.0
        },
    );
    // ...and the launcher's own question, asked helm-side. Same treatment again:
    // a hull with no tubes reads a definite zero rather than `all`'s vacuous
    // truth over an empty battery.
    facts.set(
        TUBES_FULL_FACT,
        if torpedo_tubes_full(torpedoes) {
            1.0
        } else {
            0.0
        },
    );

    // Absent (not zero) with no target at all — see the constant. The guard that
    // opens the phase conjoins `target_valid` anyway, but an absent reading is
    // what makes a doctrine that forgets to say so still safe.
    let Some(target) = target else {
        return;
    };
    let wanted = target.to_string();
    let resolved = targets.iter().find(|(uuid, _, _, _)| uuid.0 == wanted).map(
        |(_, transform, shields, target_physics)| {
            let Some(shields) = shields else {
                // No shield system at all: nothing is in the way, which is
                // exactly the reading `ai_torpedo_auto_fire` takes from the same
                // case (it reports 0 HP on the striking arc).
                //
                // Note what this reading is permanently: a station, a probe or
                // any hull authored without `[shields]` reads `1.0` here for as
                // long as it lives, because there is genuinely no arc to come
                // back. A doctrine may therefore OPEN a phase on this fact but
                // must never rely on it alone to CLOSE one — see the constant,
                // and `TUBES_FULL_FACT` for the bound that does not depend on
                // the target.
                return true;
            };
            // Arcs are authored relative to the TARGET's own facing, so the
            // bearing is taken in the target's frame.
            let incoming = crate::shield::attacker_bearing_relative(
                physics.x,
                physics.z,
                transform.translation.x,
                transform.translation.z,
                target_physics.map(|p| p.yaw).unwrap_or(0.0),
            );
            let facing = &shields.0.facings[shields.0.facing_index_for_bearing(incoming)];
            !facing.is_online()
        },
    );
    // A live target this ship cannot resolve to a transform (an asteroid, say)
    // reads "no opportunity" rather than absent: unknowable is treated as
    // closed, so the phase is never opened on a target nothing is known about.
    facts.set(
        TARGET_FACING_SHIELD_DOWN_FACT,
        if resolved.unwrap_or(false) { 1.0 } else { 0.0 },
    );
}

/// Resolve [`TUBES_FULL_FACT`]: is EVERY tube at `volley_max` right now?
///
/// One expression, and it is deliberately the SAME one `ai_torpedo_auto_fire`
/// evaluates to seed the launch channel's `tubes_full`. Two spellings of "the
/// salvo is ready" that could drift apart is the whole failure this fact exists
/// to close: the helm must not break a broadside orbit for a window the launcher
/// is going to decline.
///
/// Unlike [`torpedo_tubes_fillable`] this asks NOTHING about the fine systems'
/// control policy. Being loaded is a fact about the rounds in the tubes, not
/// about who is crewing them or whether the console is Disabled — a shot-out
/// tube that still has rounds in it reads full here, and the doctrine conjoins
/// `tubes_fillable` beside this precisely to catch that case.
fn torpedo_tubes_full(torpedoes: Option<&crate::torpedo::TorpedoSystem>) -> bool {
    // A hull with no tubes reads false, not `all`'s vacuous true.
    torpedoes
        .filter(|sys| !sys.tubes.is_empty())
        .is_some_and(|sys| {
            sys.tubes
                .iter()
                .all(|tube| tube.loaded_count >= tube.volley_max)
        })
}

/// Resolve [`TUBES_FILLABLE_FACT`]: can this ship still bring EVERY tube to
/// `volley_max`?
///
/// See the constant for why a manoeuvre asks this rather than `tubes_full`. The
/// three clauses below are the three ways the answer is permanently no.
///
/// The online test is `accept_human_input || operate_ai` — "this fine system is
/// not Disabled/Destroyed" — rather than `operate_ai` alone. `handle_fire_torpedo`
/// gates a launch on exactly that pair, so the manoeuvre and the shot agree; and
/// a hull-capability reading must not turn on who happens to be crewing the
/// tube, which is what an `operate_ai`-only test would make it (AGENTS.md #6).
fn torpedo_tubes_fillable(
    torpedoes: Option<&crate::torpedo::TorpedoSystem>,
    sources: &ShipSystemControlSources,
) -> bool {
    // No torpedo system, or a system with no tubes: nothing to fill. Ruled out
    // here rather than left to `all`, which is vacuously true over no tubes.
    let Some(sys) = torpedoes.filter(|s| !s.tubes.is_empty()) else {
        return false;
    };

    let online = |id: &crate::messages::SystemId| {
        let policy = if crate::console::weapons::shared::system_is_registered(sources, id) {
            sources.0.policy_for(id)
        } else {
            // An unregistered fine system falls back to the default-source
            // policy, matching `handle_fire_torpedo` and `ai_torpedo_load`
            // (issue #801 — no coarse fallback).
            crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            )
        };
        policy.accept_human_input || policy.operate_ai
    };

    // The magazine is the shared bottleneck every tube draws from: offline, and
    // no tube tops up again.
    if !online(&crate::system_registry::torpedo_magazine_system_id()) {
        return false;
    }
    // One dead tube is enough. `tubes_full` is an ALL-tubes reading, so a tube
    // that can never load makes it permanently false however healthy the rest
    // of the battery is.
    let every_tube_online = sys.tubes.iter().all(|tube| {
        crate::system_registry::torpedo_tube_system_id(&tube.id).is_some_and(|id| online(&id))
    });
    if !every_tube_online {
        return false;
    }

    // And finally the rounds: enough left in the magazine to cover what the
    // tubes are still short of.
    sys.torpedoes_remaining >= sys.salvo_shortfall()
}

/// Fold this fine system's OWN private memory into the shared fact snapshot
/// (issue #883).
///
/// [`RANGE_ABOVE_MIN_SEEN_FACT`] is the only derived fact, and it is derived
/// per-system on purpose: `min_range_seen` is private memory, so two siblings
/// looking at the same world can legitimately hold different minima and must not
/// see each other's. Seeded only when both halves are present, so an
/// unfolded/undeclared minimum leaves the guard reading absent (false).
fn seed_memory_derived_facts(
    facts: &mut crate::world::flags::AiFacts,
    memory: &crate::world::flags::AiPolicyMemory,
) {
    if let (Some(range), Some(min)) = (
        facts.get(RANGE_TO_TARGET_FACT),
        memory.get(MIN_RANGE_SEEN_MEMORY),
    ) {
        facts.set(RANGE_ABOVE_MIN_SEEN_FACT, range - min);
    }
}

/// Resolve one helm fine system's single mode channel, on whichever of the two
/// policy paths the authored content chose (issues #779, #882, #883).
///
/// A stateless policy (`machine: None` — every hull that ships today) takes the
/// frozen `resolve_channel` path. A policy that opted into the #882 machine
/// resolves the SAME channel inside the state `ai_policy_state_tick` committed
/// earlier this tick, so an entered state's outputs are live immediately.
///
/// ## The loud middle case
///
/// `(Some(machine), None)` — content declares a machine but the ship carries no
/// runtime-state component — silently fell back to the stateless path before
/// #883. That is precisely the failure mode of #882's blocking bug (a per-ship
/// AI component reaching one spawn path and not the other), and it had now
/// recurred three times. The `debug_assert!` makes a fourth recurrence stop the
/// test suite instead of quietly degrading a doctrine to its stateless shadow.
/// Release builds still degrade rather than panic — a live scenario should not
/// die over it — but they can no longer do so unnoticed in development.
fn resolve_helm_channel<'a>(
    policy: &'a crate::ai::policy::AiPolicy,
    state: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    channel: &str,
    facts: &crate::world::flags::AiFacts,
    now_secs: f64,
) -> Option<&'a crate::ai::policy::AiPolicyVerb> {
    match (policy.machine(), state) {
        (Some(_), Some(st)) => {
            let mut facts = facts.clone();
            seed_memory_derived_facts(&mut facts, &st.memory);
            policy.resolve_channel_in_state(
                &st.current,
                channel,
                &facts,
                &st.memory_at(now_secs),
                &[],
            )
        }
        (Some(_), None) => {
            debug_assert!(
                false,
                "fine system channel '{channel}' has a STATEFUL authored policy but the ship \
                 carries no policy-state component: the machine cannot run and this would \
                 silently degrade to the stateless path. Every per-ship AI component must be \
                 declared in ai_high_fidelity_components() (src/ai/server.rs), never inserted \
                 by hand on one spawn path"
            );
            policy.resolve_channel(channel, facts, &[])
        }
        (None, _) => policy.resolve_channel(channel, facts, &[]),
    }
}

/// Per-axis helm AI: throttle. Decides the throttle for ships whose
/// helm-thrust system is AI-operated and emits it as an admitted `SetThrust`
/// into the ship's own `AdmittedCommands` (issues #800, #704, #824) —
/// `process_helm_inputs` applies it to `ThrustInput` later this tick.
///
/// `AiHighFidelity`-scoped: the frame is only built for ships carrying that
/// marker, and the intent components the admitted command lands on only
/// exist there (`lod_ai_ships` inserts/removes them with the marker).
///
/// Decodes only its own axis from the shared motion plan (built this tick by
/// `helm_motion_planner` from the pure `plan_helm_travel` decision, see the
/// module note).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_thrust(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    clock: Res<AiPolicyTickClock>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            // Availability of the two optional drives, seeded honestly into the
            // fact snapshot below (see the `seed_helm_actuator_facts` call).
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&HelmEnginesAiPolicy>,
            Option<&HelmEnginesAiPolicyState>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    // Canonical fallback for any ship missing an attached policy component (bare
    // `App` unit fixtures). Real ships always carry one, authored or
    // synthesised, attached at spawn. Built once per tick, not per ship —
    // mirrors `operate_captain_ai`.
    let default_policy = crate::entities::config::default_engines_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        physics,
        physics_cfg,
        entity_uuid,
        ship_config,
        boost_cfg,
        impulse_cfg,
        engines_policy,
        engines_state,
        mut admitted,
    ) in ships.iter_mut()
    {
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
        // Resolve the data-authored #779 Engines policy's `longitudinal` mode
        // verb to decide WHETHER to actuate this tick. The stateless policy is a
        // pure fact→mode map; the continuous magnitude below still comes from
        // the planner fact, so no geometry lives in the policy (AGENTS.md #11).
        // A "hold" resolution (no rule fires / explicit idle) emits nothing and
        // the throttle coasts on its last input.
        //
        // Issue #883 (AC5) closes the #779 empty-facts gap on this axis: the
        // snapshot below is really seeded — hazard/availability from the shared
        // surfaces, target-relative motion from the frame's merged view — so a
        // `fact(...)` guard on `longitudinal` evaluates against the world
        // instead of validating and never firing.
        //
        // The availability pair is seeded from the ship's OWN config resources,
        // exactly as `ai_policy_state_tick` and `ai_helm_boost` seed it. Passing
        // a hardcoded `false` here would have been the #779 trap one fact
        // narrower: a guard on `fact(boost_available)` would validate at load
        // and then read 0 for ever, which is silently wrong in the same way an
        // absent fact is.
        let policy = engines_policy.map(|p| &p.0).unwrap_or(&default_policy);
        let mut facts = seed_helm_actuator_facts(
            Some(&sp.hazard),
            impulse_cfg.is_some(),
            boost_cfg.map(|c| c.enabled).unwrap_or(false),
            physics.y,
        );
        seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        if resolve_helm_channel(
            policy,
            engines_state.map(|s| &s.0),
            crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
            &facts,
            clock.0,
        ) != Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel)
        {
            continue;
        }
        let thrust =
            crate::ai::decode_thrust_from_velocity(sp.motion.desired_velocity_local.to_array());

        emit_helm_ai_command(
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_steering(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    clock: Res<AiPolicyTickClock>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            // Availability of the two optional drives — see `ai_helm_thrust`.
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&HelmSteeringAiPolicy>,
            Option<&HelmSteeringAiPolicyState>,
            Option<&mut PendingArcBearingRequest>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    // Canonical fallback for ships missing an attached policy (bare-`App`
    // fixtures); built once per tick — mirrors `ai_helm_thrust`.
    let default_policy = crate::entities::config::default_steering_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        physics,
        physics_cfg,
        entity_uuid,
        ship_config,
        boost_cfg,
        impulse_cfg,
        steering_policy,
        steering_state,
        mut pending_bearing,
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

        // Resolve the data-authored #779 Steering policy's `yaw` mode verb to
        // decide WHETHER to actuate this tick (see `ai_helm_thrust` for the
        // mode-verb rationale). "Hold" emits nothing and yaw coasts on its last
        // input — including any pending arc-bearing this axis owns.
        //
        // Issue #883 gives this channel a SECOND mode verb and (AC5) a really
        // seeded fact snapshot. `hold_committed_heading` actuates exactly like
        // `actuate_desired_facing` here — both emit the planner's decoded yaw —
        // because the difference between them was already resolved upstream:
        // the planner solved the facing against the FROZEN heading rather than
        // against the moving target. That is deliberate. Overriding
        // `SteeringInput` here instead would bypass the planner, and #780's
        // hazard contribution would stop composing onto the escape (AC3).
        // The availability pair is seeded honestly from the ship's own config
        // resources — see the note in `ai_helm_thrust`.
        let policy = steering_policy.map(|p| &p.0).unwrap_or(&default_policy);
        let mut facts = seed_helm_actuator_facts(
            Some(&sp.hazard),
            impulse_cfg.is_some(),
            boost_cfg.map(|c| c.enabled).unwrap_or(false),
            physics.y,
        );
        seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        let actuates = matches!(
            resolve_helm_channel(
                policy,
                steering_state.map(|s| &s.0),
                crate::entities::config::HELM_YAW_CHANNEL,
                &facts,
                clock.0,
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing)
                | Some(&crate::ai::policy::AiPolicyVerb::HoldCommittedHeading)
                // Issue #788's two recovery mode verbs actuate here identically
                // too, and for the identical reason: the difference between them
                // was already resolved upstream, by the planner solving the
                // facing against a ring tangent or against the target rather
                // than against a frozen heading. Overriding `SteeringInput` here
                // would bypass the planner and stop hazard avoidance composing
                // onto the orbit.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit)
                | Some(&crate::ai::policy::AiPolicyVerb::PivotToReengage)
                // ...and issue #790's combat broadside orbit, for the third
                // time and the same reason: the planner already solved the
                // facing against the fighting ring's tangent, so this axis's
                // only job is to emit it.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldCombatOrbit)
                // ...and issue #791's torpedo-opportunity bow hold, for the
                // fourth time and the same reason: the planner already solved
                // the facing against the target's live position, so this axis's
                // only job is to emit it — and emitting it through the planner
                // is what keeps hazard avoidance composing onto the hold.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldTorpedoBearing)
                // ...and issue #792's artillery firing position, for the fifth
                // time and the same reason: the planner already solved the
                // facing against the PREDICTED intercept, so this axis's only
                // job is to emit it — and emitting it through the planner is
                // what keeps hazard avoidance composing onto the hold.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldArtilleryPosition)
        );
        if !actuates {
            continue;
        }

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
            );
        }

        emit_helm_ai_command(
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_impulse(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&HelmImpulseAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::ai_plugin::ObjectiveCursors>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    // Canonical fallback for ships missing an attached policy (bare-`App`
    // fixtures); built once per tick — mirrors `ai_helm_thrust`.
    let default_policy = crate::entities::config::default_impulse_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        physics,
        impulse_comp,
        impulse_cfg,
        boost_cfg,
        behaviour_section,
        impulse_policy,
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
        // the monolith, which guards the same pair. Availability (AC6): the
        // presence of `ImpulseConfigResource` is the impulse capability — no
        // config, no emit.
        let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) else {
            continue;
        };

        // Authored manoeuvre policy gate (issue #780, AC6): seed the hazard +
        // availability facts and resolve the `impulse` channel. Its default
        // (unconditional permit) preserves the pre-#780 baseline exactly — the
        // engage/cancel decision is still made below from doctrine + geometry —
        // while an authored guard may hold impulse. A "hold" resolution emits
        // nothing.
        let boost_available = boost_cfg.map(|c| c.enabled).unwrap_or(false);
        let facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            true,
            boost_available,
            physics.y,
        );
        let policy = impulse_policy.map(|p| &p.0).unwrap_or(&default_policy);
        if !helm_policy_actuates(
            policy,
            crate::entities::config::HELM_IMPULSE_CHANNEL,
            &facts,
            &crate::ai::policy::AiPolicyVerb::EngageImpulse,
        ) {
            continue;
        }

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
        emit_helm_ai_command(
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
            Option<&HelmLateralAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    // Canonical fallback for ships missing an attached policy; built once per
    // tick — mirrors `ai_helm_thrust`.
    let default_policy = crate::entities::config::default_lateral_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        behaviour_section,
        lateral_policy,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
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
        // the shared desired-motion contract's `x`. This is an UNCONDITIONAL
        // sanctioned override (issue #780): it precedes the policy gate so a
        // docking hull always translates onto its berth.
        let docking_lateral = ship_plan
            .filter(|sp| sp.docking_active)
            .map(|sp| sp.motion.desired_velocity_local.x);

        // Authored actuation-policy gate (issue #780, AC1/AC3): outside a docking
        // manoeuvre, the DECISION to actuate the dodge flows through
        // HelmLateralAiPolicy over a fact snapshot seeded from the shared hazard
        // surface — never a doctrine swap (AC3), only a gate on the dodge. Its
        // default (unconditional actuate) reproduces the pre-#780 always-on
        // avoidance; a "hold" resolution emits nothing and lateral coasts.
        if docking_lateral.is_none() {
            let facts = seed_helm_actuator_facts(ship_plan.map(|sp| &sp.hazard), false, false, 0.0);
            let policy = lateral_policy.map(|p| &p.0).unwrap_or(&default_policy);
            if !helm_policy_actuates(
                policy,
                crate::entities::config::HELM_LATERAL_CHANNEL,
                &facts,
                &crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
            ) {
                continue;
            }
        }

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

        emit_helm_lateral_command(
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

/// Per-axis helm AI: vertical thrust (issue #744). Decides the up/down axis for
/// ships whose `helm-vertical-thrust` system is AI-operated and emits it as an
/// admitted `VerticalThrustInput` into the ship's own `AdmittedCommands`,
/// through the same `emit_ai_command` arbiter as the other per-axis operators.
///
/// AI-only: the vertical axis has no player-facing control, so this operator is
/// the sole decider of it. Its behaviour is gated on the hull's authored
/// [`VerticalMovementMode`](crate::entity_config::VerticalMovementMode):
///
/// - **Planar** — never commands vertical motion (the axis stays at cruise).
/// - **Bounded** — climbs to dodge *moving* hazards up to the authored
///   `max_vertical_offset`, then eases back toward the cruise plane (`y = 0`) at
///   the authored `vertical_return_rate` once the moving-hazard threat falls
///   (the return is the hysteresis: it only engages when avoidance does not).
/// - **Full3D** — the same avoidance climb without the offset ceiling and with
///   no auto-return, exposing the full vertical degree of freedom.
///
/// The dodge responds to the shared hazard assessment's `moving_hazard_threat`
/// (the planner pre-filters the contribution list to movable hazards, issue
/// #744) weighted by the hull's authored `vertical_hazard_sensitivity` — so a
/// static obstacle, however close, never drives a vertical dodge.
pub(crate) fn ai_helm_vertical_thrust(
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::entities::spawner::HelmCapabilitySection>,
            Option<&HelmVerticalAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    use crate::entity_config::VerticalMovementMode;
    // Canonical fallback for ships missing an attached policy; built once per
    // tick — mirrors `ai_helm_thrust`.
    let default_policy = crate::entities::config::default_vertical_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        physics,
        behaviour_section,
        capability,
        vertical_policy,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800), like every per-axis operator.
        if !sources
            .0
            .policy_for(&crate::system_registry::vertical_thrust_system_id())
            .operate_ai
        {
            continue;
        }

        // Authored actuation-policy gate (issue #780, AC1/AC5): the DECISION to
        // actuate the vertical axis flows through HelmVerticalAiPolicy over a
        // fact snapshot seeded from the shared moving-hazard threat and the
        // ship's current vertical offset (for return-to-cruise guards). Its
        // default (unconditional actuate) preserves the pre-#780 behaviour; the
        // authored `VerticalMovementMode` still gates the magnitude below, so a
        // Planar hull takes no Y component regardless of the verb. A "hold"
        // resolution emits nothing.
        let facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            false,
            false,
            physics.y,
        );
        let policy = vertical_policy.map(|p| &p.0).unwrap_or(&default_policy);
        if !helm_policy_actuates(
            policy,
            crate::entities::config::HELM_VERTICAL_CHANNEL,
            &facts,
            &crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        ) {
            continue;
        }

        let mode = capability
            .map(|c| c.0.vertical_movement_mode)
            .unwrap_or_default();

        let vertical = match mode {
            // A planar hull has no vertical axis — hold the cruise plane.
            VerticalMovementMode::Planar => 0.0,
            VerticalMovementMode::Bounded | VerticalMovementMode::Full3D => {
                let sensitivity = behaviour_section
                    .map(|b| b.0.vertical_hazard_sensitivity)
                    .unwrap_or(crate::ai::VERTICAL_HAZARD_SENSITIVITY);
                let moving_threat = plan
                    .ships
                    .get(&entity)
                    .map(|sp| sp.hazard.moving_hazard_threat)
                    .unwrap_or(0.0);
                // Climb to dodge; the initial policy only ever climbs (positive)
                // away from moving hazards sharing the cruise plane.
                let climb = (moving_threat * sensitivity).clamp(0.0, 1.0);

                if climb > f32::EPSILON {
                    match mode {
                        // Bounded: respect the authored ceiling — stop climbing
                        // once at/above the max offset from cruise.
                        VerticalMovementMode::Bounded => {
                            let max_offset = capability
                                .map(|c| c.0.max_vertical_offset)
                                .unwrap_or(crate::ai::MAX_VERTICAL_OFFSET);
                            if physics.y >= max_offset {
                                0.0
                            } else {
                                climb
                            }
                        }
                        // Full3D: unbounded vertical DOF, no ceiling.
                        _ => climb,
                    }
                } else {
                    // No moving hazard: Bounded eases back to the cruise plane;
                    // Full3D holds its altitude (no auto-return).
                    match mode {
                        VerticalMovementMode::Bounded => {
                            let return_rate = capability
                                .map(|c| c.0.vertical_return_rate)
                                .unwrap_or(crate::ai::VERTICAL_RETURN_RATE);
                            (-physics.y * return_rate).clamp(-1.0, 1.0)
                        }
                        _ => 0.0,
                    }
                }
            }
        };

        emit_ai_command(
            entity_uuid,
            crate::system_registry::vertical_thrust_system_id(),
            crate::messages::SystemControlPayload::VerticalThrustInput { vertical },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

/// Per-axis helm AI: boost drive (issue #780). Decides engage/release for ships
/// whose `helm-boost` system is AI-operated and emits it as an admitted
/// `SetBoost { active }` into the ship's own `AdmittedCommands` — the SAME seam
/// a human `SetBoost`/`ToggleBoost` passes through (`process_helm_inputs`,
/// which since issue #881 applies boost for EVERY ship, not just the
/// `LocalShip`), preserving human/AI symmetry (AGENTS.md #6).
///
/// Modelled on [`ai_helm_impulse`]: discrete and on-change. Availability (AC6) is
/// the presence of an *enabled* [`BoostConfigResource`] — no config, or a
/// feature-disabled one, and the system stands down without emitting. The
/// DECISION flows through [`HelmBoostAiPolicy`] resolving the `boost` channel to
/// the `engage_boost` mode verb over a fact snapshot seeded from the shared
/// hazard surface: fires ⇒ boost on, holds ⇒ boost off. The canonical default is
/// idle, so a ship that authors no `[helm_console.boost_ai]` never AI-boosts —
/// the pre-#780 baseline. It emits only when the desired state differs from the
/// current `ShipBoost`, so it does not re-issue `SetBoost` every tick.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_boost(
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&ShipBoost>,
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&HelmBoostAiPolicy>,
            Option<&HelmBoostAiPolicyState>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
    clock: Res<AiPolicyTickClock>,
) {
    // Canonical fallback (idle) for ships missing an attached policy; built once
    // per tick — mirrors `ai_helm_thrust`.
    let default_policy = crate::entities::config::default_boost_ai_config()
        .to_policy()
        .unwrap_or_default();
    for (
        entity,
        sources,
        physics,
        physics_cfg,
        boost_comp,
        boost_cfg,
        impulse_cfg,
        boost_policy,
        boost_state,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_boost_system_id())
            .operate_ai
        {
            continue;
        }

        // Availability (AC6): the feature must be present AND enabled. No
        // BoostConfigResource, or one with the feature disabled, means no boost
        // capability — emit nothing (mirrors the shared applier's
        // `enabled`-guard in `process_helm_inputs`).
        let (Some(boost), Some(cfg)) = (boost_comp, boost_cfg) else {
            continue;
        };
        if !cfg.enabled {
            continue;
        }

        // Authored manoeuvre policy (issue #780, AC6): resolve the `boost`
        // channel over a fact snapshot seeded from the shared hazard surface and
        // availability. Fires ⇒ engage; holds ⇒ release.
        // Issue #883 also seeds the target-relative travel facts here, so an
        // authored escape-leg boost rule can guard on the pass geometry (range,
        // closing rate, speed fraction) and not just on hazard/state time.
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            impulse_cfg.is_some(),
            true,
            physics.y,
        );
        seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        let policy = boost_policy.map(|p| &p.0).unwrap_or(&default_policy);
        // Stateless (the shipped shape) resolves exactly as it always has.
        // A policy that opted into the #882 machine instead resolves the SAME
        // channel inside its current state — committed earlier this tick by
        // `ai_policy_state_tick`, so the outputs are the new state's outputs
        // immediately (AC2). The shared helper also carries the #883
        // silent-degradation guard for the "machine declared, state component
        // missing" case that used to fall through unnoticed.
        let desired_active = resolve_helm_channel(
            policy,
            boost_state.map(|s| &s.0),
            crate::entities::config::HELM_BOOST_CHANNEL,
            &facts,
            clock.0,
        ) == Some(&crate::ai::policy::AiPolicyVerb::EngageBoost);

        // On-change only: `SetBoost` sets the desired active state, and the
        // shared integrator applies the transition; re-issuing an unchanged state
        // every tick is redundant. Mirrors `ai_helm_impulse`'s NoChange skip.
        if desired_active == boost.0.is_active() {
            continue;
        }

        emit_ai_command(
            entity_uuid,
            crate::system_registry::helm_boost_system_id(),
            crate::messages::SystemControlPayload::SetBoost {
                active: desired_active,
            },
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

        let axes: [(&str, crate::messages::SystemId); 5] = [
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
            ("helm-boost", crate::system_registry::helm_boost_system_id()),
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
            .insert(PendingArcBearingRequest {
                target: Some(bearing_uuid),
                // Fore bank, narrow arc, ample range: the far-starboard target is
                // in reach but well out of arc, so steering must bias to bear.
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 5000.0,
                }],
            });

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

    // ── #779: data-authored Engines/Steering policy spine ────────────────────

    /// Attach an authored Engines policy to the ship (overriding the spawn
    /// default the hosts fall back to).
    fn attach_engines_policy(app: &mut App, cfg: crate::entity_config::FineSystemAiConfigToml) {
        let ship = find_ship_entity(app);
        let policy = cfg.to_policy().expect("engines policy resolves");
        app.world_mut()
            .entity_mut(ship)
            .insert(HelmEnginesAiPolicy(policy));
    }

    /// Attach an authored Steering policy to the ship.
    fn attach_steering_policy(app: &mut App, cfg: crate::entity_config::FineSystemAiConfigToml) {
        let ship = find_ship_entity(app);
        let policy = cfg.to_policy().expect("steering policy resolves");
        app.world_mut()
            .entity_mut(ship)
            .insert(HelmSteeringAiPolicy(policy));
    }

    /// AC1/AC3/AC4: with the canonical default Engines *and* Steering policies
    /// explicitly attached — the same policies spawn synthesises — a Reach
    /// objective produces both actuator inputs and drives the ship toward its
    /// destination. The DECISION to actuate now flows through the resolved mode
    /// verb; the continuous magnitude still comes from the planner.
    #[test]
    fn authored_default_policy_actuates_travel_toward_reach_anchor() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        // Anchor off the starboard bow so both axes must engage.
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        attach_engines_policy(&mut app, crate::entity_config::default_engines_ai_config());
        attach_steering_policy(&mut app, crate::entity_config::default_steering_ai_config());

        tick(&mut app);

        assert!(
            get_thrust_input(&mut app) > 0.0,
            "the authored Engines policy resolving `actuate_desired_travel` must emit forward SetThrust"
        );
        assert!(
            get_steering_input(&mut app) > 0.0,
            "the authored Steering policy resolving `actuate_desired_facing` must emit SetSteering toward the starboard anchor"
        );

        // AC4: the ship actually closes on its destination — several ticks of
        // forward travel build speed and move it downrange through the shared
        // actuator path (not a coarse direct write).
        let start = get_ship_physics(&mut app);
        for _ in 0..30 {
            tick(&mut app);
        }
        let end = get_ship_physics(&mut app);
        assert!(
            end.forward_speed > 0.0,
            "the ship must build forward speed under the authored policy; got {end:?}"
        );
        let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
        assert!(
            moved > 0.0,
            "the ship must make positional progress toward its Reach destination; \
             start=({},{}) end=({},{})",
            start.x,
            start.z,
            end.x,
            end.z
        );
    }

    /// AC1: the policy is a real gate, not decoration. An Engines policy whose
    /// only rule never fires (`when = false`) resolves to "hold" on the
    /// `longitudinal` channel, so `ai_helm_thrust` emits nothing even though the
    /// Reach objective and the planner both want forward travel — while an
    /// unchanged default Steering policy still turns the ship. This is the seam
    /// #794 will exploit to retire the hardcoded planner branch.
    #[test]
    fn engines_policy_that_never_fires_holds_thrust_but_not_steering() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);

        let hold = crate::entity_config::FineSystemAiConfigToml {
            idle: false,
            param: Default::default(),
            rule: vec![crate::entity_config::FineSystemAiRuleToml {
                priority: 0,
                channel: crate::entity_config::HELM_LONGITUDINAL_CHANNEL.into(),
                when: "false".into(),
                verb: crate::entity_config::HELM_ACTUATE_DESIRED_TRAVEL_VERB.into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        attach_engines_policy(&mut app, hold);

        tick(&mut app);

        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "an Engines policy whose guard never fires must hold thrust: no SetThrust emitted"
        );
        assert!(
            get_steering_input(&mut app) > 0.0,
            "Steering is independently authored and still actuates — the two systems are separable"
        );
    }

    /// AC1 mirror on the yaw axis: an explicit-idle Steering policy holds the
    /// facing while the default Engines policy still throttles.
    #[test]
    fn idle_steering_policy_holds_yaw_but_not_thrust() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        attach_steering_policy(
            &mut app,
            crate::entity_config::FineSystemAiConfigToml {
                idle: true,
                ..Default::default()
            },
        );

        tick(&mut app);

        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "an idle Steering policy resolves no verb → yaw holds, no SetSteering emitted"
        );
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "the default Engines policy still actuates travel"
        );
    }

    /// AC6: human takeover preserves input authority, and Backfill reacquisition
    /// restores AI actuation without any lifecycle carry-over (the policy is
    /// stateless, so reacquisition is a clean resolve, not a resumed machine).
    /// Under the same authored default policy throughout, the emit tracks the
    /// per-axis control source tick to tick.
    #[test]
    fn human_takeover_and_backfill_reacquisition_track_input_authority() {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        attach_engines_policy(&mut app, crate::entity_config::default_engines_ai_config());
        attach_steering_policy(&mut app, crate::entity_config::default_steering_ai_config());

        // Backfill: both axes AI → the policy actuates.
        set_per_axis_helm_ai(&mut app);
        tick(&mut app);
        assert!(
            get_thrust_input(&mut app) > 0.0 && get_steering_input(&mut app) > 0.0,
            "AI-operated axes under the authored policy must actuate"
        );

        // Human takeover: both axes handed back to a human. The AI hosts must
        // not write the intent — input authority is the human's.
        set_helm_control_source(&mut app, ControlSource::Human);
        // The intent components retain their last value; zero them so a stale
        // read cannot masquerade as a fresh AI write, then confirm the AI leaves
        // them at zero.
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            crate::ship::helm::ThrustInput::default(),
            crate::ship::helm::SteeringInput::default(),
        ));
        tick(&mut app);
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "under human takeover the AI Engines host must not write thrust"
        );
        assert_eq!(
            get_steering_input(&mut app),
            0.0,
            "under human takeover the AI Steering host must not write yaw"
        );

        // Backfill reacquisition: hand the axes back to AI. The stateless policy
        // resolves cleanly and actuation resumes the same tick — no reset needed.
        set_per_axis_helm_ai(&mut app);
        tick(&mut app);
        assert!(
            get_thrust_input(&mut app) > 0.0 && get_steering_input(&mut app) > 0.0,
            "reacquired AI axes must re-actuate immediately under the same stateless policy"
        );
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

    /// Put `uuid` in the frozen viewscreen's Combat Lock (issue #829), leaving
    /// the blackboard's scored objectives alone.
    ///
    /// In production `ai_target_selection` publishes this lock for any ship
    /// pursuing a Destroy directive, and `ai_helm_impulse` is the one helm axis
    /// that resolves its target through it rather than through the directive's
    /// own name — so a fixture that poses a Destroy objective without a lock has
    /// an impulse system that can never resolve a target and therefore never
    /// acts. Must be called AFTER `set_ship_blackboard_objectives`, which
    /// replaces the whole viewscreen blackboard.
    fn set_ship_combat_lock(app: &mut App, uuid: uuid::Uuid) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<Ship>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipSystemBlackboards");
        match bbs
            .0
            .get_mut(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => {
                bb.combat_lock = Some(uuid.to_string());
            }
            _ => panic!("set the viewscreen blackboard's objectives before its combat lock"),
        }
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
            .insert(PendingArcBearingRequest {
                target: Some(bearing_uuid),
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 5000.0,
                }],
            });

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
        // The carried family arc (issue #767): a fore bank, narrow arc, range
        // that reaches the target directly ahead — so the ship's own facing
        // already brings it into arc AND range, i.e. the family can fire.
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest {
                target: Some(bearing_uuid),
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 500.0,
                }],
            });

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.target, None,
            "a request must clear once the ship's own facing already brings the carried family's arc \
             onto the target, not persist indefinitely after being satisfied"
        );
    }

    /// AC4 (issue #767): a request clears when the target leaves the range of
    /// every carried emitter arc — no yaw can help, so the bias must not
    /// persist steering the ship at an unreachable contact.
    #[test]
    fn helm_ai_clears_arc_bearing_request_once_target_leaves_range() {
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

        // Bearing contact well off to starboard AND far beyond the carried
        // arc's range (range 50, target ~200 away) — out of reach entirely.
        let bearing_uuid = uuid::Uuid::new_v4();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(bearing_uuid.to_string()),
            crate::entities::spawner::EntityName("Bearing Contact".into()),
            Transform::from_xyz(200.0, 0.0, -1.0),
        ));
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest {
                target: Some(bearing_uuid),
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 50.0,
                }],
            });

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.target, None,
            "a request must clear once the target is beyond every carried arc's range — no bearing helps"
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
            .insert(PendingArcBearingRequest {
                target: Some(stale_uuid),
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 5000.0,
                }],
            });

        tick(&mut app);

        let pending = app
            .world()
            .get::<PendingArcBearingRequest>(ship)
            .expect("ship must carry PendingArcBearingRequest");
        assert_eq!(
            pending.target, None,
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
            .insert(PendingArcBearingRequest {
                target: Some(bearing_uuid),
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 30.0,
                    range: 5000.0,
                }],
            });

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
                direct_fire_range: 0.0,
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
                direct_fire_range: 0.0,
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

    // ── Vertical thrust AI (issue #744) ──────────────────────────────────

    fn capability_with_mode(
        mode: crate::entity_config::VerticalMovementMode,
        max_vertical_offset: f32,
    ) -> crate::entity_config::HelmCapabilityConfig {
        crate::entity_config::HelmCapabilityConfig {
            vertical_movement_mode: mode,
            max_vertical_offset,
            ..Default::default()
        }
    }

    /// Build an app whose ship runs AI vertical thrust under the given
    /// capability. The helm proper stays human so only the vertical-thrust
    /// operator can ever write the vertical axis (issue #744).
    fn vertical_thrust_ai_app(
        capability: crate::entity_config::HelmCapabilityConfig,
        behaviour: Option<crate::entity_config::BehaviourConfig>,
    ) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Human);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::vertical_thrust_system_id(),
                    ControlSource::Ai,
                );
            }
        }
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entities::spawner::HelmCapabilitySection(capability));
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        app
    }

    fn vertical_intent(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&VerticalThrustInput>()
            .single(app.world())
            .expect("ship must carry VerticalThrustInput")
            .0
    }

    /// The initial vertical policy filters to *moving* hazards: an in-range
    /// static obstacle (which the planar actuators would still dodge) drives no
    /// vertical thrust, while an in-range moving hazard makes the ship climb.
    /// Both sit inside the same authored 60-unit buffer, so the only difference
    /// is the `movable` fact (issue #744).
    #[test]
    fn vertical_thrust_ai_responds_to_moving_hazard_not_static() {
        let obstacle = [4.0, 0.0, -40.0];
        let behaviour = crate::entity_config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        };

        // Static hazard, in range: no vertical response.
        let mut static_app = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(behaviour.clone()),
        );
        snapshot_with_obstacle(&mut static_app, obstacle, 1.0);
        tick_twice(&mut static_app);
        assert_eq!(
            vertical_intent(&mut static_app),
            0.0,
            "an in-range STATIC hazard must not drive vertical thrust"
        );

        // Moving hazard, same spot and range: the ship climbs to dodge.
        let mut moving_app = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(behaviour),
        );
        snapshot_with_moving_obstacle(&mut moving_app, obstacle, 1.0, 0.0, 0.0);
        tick_twice(&mut moving_app);
        assert!(
            vertical_intent(&mut moving_app) > 0.0,
            "an in-range MOVING hazard must drive a climb; got {}",
            vertical_intent(&mut moving_app)
        );
    }

    /// The authored `vertical_hazard_sensitivity` gates the response: sensitivity
    /// 0 mutes the climb the default weight produces (issue #744).
    #[test]
    fn vertical_thrust_ai_honours_authored_sensitivity() {
        let obstacle = [4.0, 0.0, -40.0];

        let mut muted = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                vertical_hazard_sensitivity: 0.0,
                ..Default::default()
            }),
        );
        snapshot_with_moving_obstacle(&mut muted, obstacle, 1.0, 0.0, 0.0);
        tick_twice(&mut muted);
        assert_eq!(
            vertical_intent(&mut muted),
            0.0,
            "authored zero vertical sensitivity must mute the climb"
        );
    }

    /// The three movement modes produce demonstrably divergent authoritative Y
    /// motion under the same persistent moving hazard (issue #744): Planar holds
    /// the cruise plane, Bounded climbs but is capped at its authored offset,
    /// and Full3D keeps climbing past that cap.
    #[test]
    fn vertical_movement_modes_diverge_under_a_moving_hazard() {
        use crate::entity_config::VerticalMovementMode;
        let obstacle = [4.0, 0.0, -40.0];
        const BOUNDED_OFFSET: f32 = 2.0;

        fn final_y(mode: VerticalMovementMode, obstacle: [f32; 3], offset: f32) -> f32 {
            let mut app = vertical_thrust_ai_app(
                capability_with_mode(mode, offset),
                Some(crate::entity_config::BehaviourConfig {
                    avoidance_buffer: 60.0,
                    ..Default::default()
                }),
            );
            // A persistent, planar moving hazard: assess_hazards is planar, so it
            // stays a threat no matter how high the ship climbs.
            snapshot_with_moving_obstacle(&mut app, obstacle, 1.0, 0.0, 0.0);
            for _ in 0..60 {
                tick(&mut app);
            }
            get_ship_physics(&mut app).y
        }

        let planar_y = final_y(VerticalMovementMode::Planar, obstacle, BOUNDED_OFFSET);
        let bounded_y = final_y(VerticalMovementMode::Bounded, obstacle, BOUNDED_OFFSET);
        let full3d_y = final_y(VerticalMovementMode::Full3D, obstacle, BOUNDED_OFFSET);

        assert!(
            planar_y.abs() < 0.01,
            "Planar hull must never leave the cruise plane, got y={planar_y}"
        );
        assert!(
            bounded_y > 0.5,
            "Bounded hull must climb to dodge, got y={bounded_y}"
        );
        assert!(
            bounded_y <= BOUNDED_OFFSET + 5.0,
            "Bounded hull must respect its authored max offset ({BOUNDED_OFFSET}), got y={bounded_y}"
        );
        assert!(
            full3d_y > bounded_y + 3.0,
            "Full3D hull must climb well past the bounded cap; bounded={bounded_y} full3d={full3d_y}"
        );
    }

    /// Bounded avoidance returns gradually to the cruise plane once the moving
    /// hazard is gone (issue #744): the ship climbs while threatened, then eases
    /// back toward y = 0 when the threat clears.
    #[test]
    fn bounded_vertical_returns_to_cruise_after_hazard_clears() {
        let obstacle = [4.0, 0.0, -40.0];
        let mut app = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        );
        snapshot_with_moving_obstacle(&mut app, obstacle, 1.0, 0.0, 0.0);
        for _ in 0..40 {
            tick(&mut app);
        }
        let climbed = get_ship_physics(&mut app).y;
        assert!(
            climbed > 1.0,
            "the ship must have climbed while threatened, got y={climbed}"
        );

        // Threat clears: the world is now empty.
        app.insert_resource(crate::ai::server::WorldSnapshot { entities: vec![] });
        for _ in 0..120 {
            tick(&mut app);
        }
        let returned = get_ship_physics(&mut app).y;
        assert!(
            returned < climbed - 0.5,
            "the ship must ease back toward cruise after the hazard clears; \
             climbed={climbed} returned={returned}"
        );
        assert!(
            returned < 1.0,
            "the ship must return close to the cruise plane, got y={returned}"
        );
    }

    // ── Secondary-actuator policy gate + fact seeding (issue #780) ───────────

    fn set_vertical_ai_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(HelmVerticalAiPolicy(policy));
    }

    fn set_lateral_ai_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(HelmLateralAiPolicy(policy));
    }

    /// A vertical policy that actuates only while the seeded `moving_hazard_threat`
    /// fact exceeds an authored threshold — a `fact(...)`-referencing guard.
    fn threat_gated_vertical_policy(threshold: f64) -> crate::ai::policy::AiPolicy {
        let mut params = crate::world::flags::AiParams::new();
        params.set("threshold", threshold);
        crate::ai::policy::AiPolicy {
            params,
            rules: vec![crate::ai::policy::AiPolicyRule {
                priority: 10,
                channel: crate::entities::config::HELM_VERTICAL_CHANNEL.into(),
                when: crate::world::flags::parse_predicate(
                    "fact(moving_hazard_threat) > param(threshold)",
                )
                .unwrap(),
                verb: crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
            }],
            idle: false,
            machine: None,
        }
    }

    /// THE #779 empty-facts sharp edge, resolved (issue #780). A vertical policy
    /// whose guard references the seeded `moving_hazard_threat` fact must actually
    /// FIRE — impossible before #780 because the helm hosts passed an empty
    /// `AiFacts`. With no moving hazard the guard is false and the axis holds at
    /// cruise; introduce a moving hazard and the same guard fires and the ship
    /// climbs. Proves the host now seeds real hazard facts.
    #[test]
    fn vertical_fact_guard_fires_only_once_hazard_fact_is_seeded() {
        // Guard needs threat > 0.1. No hazard → fact seeds 0.0 → hold at cruise.
        let mut calm = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        );
        set_vertical_ai_policy(&mut calm, threat_gated_vertical_policy(0.1));
        app_empty_snapshot(&mut calm);
        tick_twice(&mut calm);
        assert_eq!(
            vertical_intent(&mut calm),
            0.0,
            "with no hazard the seeded moving_hazard_threat is 0, the guard is \
             false, and the vertical axis holds — the pre-#780 empty facts would \
             have made this guard un-fireable at all"
        );

        // Same policy, now a moving hazard seeds a nonzero threat → guard fires.
        let mut threatened = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        );
        set_vertical_ai_policy(&mut threatened, threat_gated_vertical_policy(0.1));
        snapshot_with_moving_obstacle(&mut threatened, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
        tick_twice(&mut threatened);
        assert!(
            vertical_intent(&mut threatened) > 0.0,
            "a seeded moving_hazard_threat above the authored threshold must fire \
             the guard and climb; got {}",
            vertical_intent(&mut threatened)
        );
    }

    /// AC1/AC7 typed output + an authored idle/hold: a vertical policy that never
    /// fires holds the axis, proving the actuator emits a TYPED VerticalThrustInput
    /// only when its own channel resolves — not unconditionally.
    #[test]
    fn vertical_actuator_holds_under_never_firing_policy() {
        let mut app = vertical_thrust_ai_app(
            capability_with_mode(crate::entity_config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        );
        // Threshold above 1.0 can never be crossed by a [0,1] threat → never fires.
        set_vertical_ai_policy(&mut app, threat_gated_vertical_policy(2.0));
        snapshot_with_moving_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
        tick_twice(&mut app);
        assert_eq!(
            vertical_intent(&mut app),
            0.0,
            "a policy that never fires must hold the vertical axis despite a live \
             moving hazard the default policy would climb for"
        );
    }

    /// AC3: ordinary avoidance BENDS travel without swapping the doctrine. A
    /// lateral policy that never fires suppresses the dodge, proving the dodge
    /// flows through the actuator gate — while the same tick's engine/steering
    /// doctrine (a forward Reach) is untouched.
    #[test]
    fn lateral_actuator_holds_under_never_firing_policy() {
        let mut app = lateral_dodge_app();
        // A policy on the lateral channel that never fires.
        let never = crate::ai::policy::AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: vec![crate::ai::policy::AiPolicyRule {
                priority: 10,
                channel: crate::entities::config::HELM_LATERAL_CHANNEL.into(),
                when: crate::world::flags::parse_predicate("fact(hazard_urgency) > 9.0").unwrap(),
                verb: crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
            }],
            idle: false,
            machine: None,
        };
        set_lateral_ai_policy(&mut app, never);
        tick_twice(&mut app);
        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "a never-firing lateral policy must hold the dodge even with a hazard \
             in range the default policy would dodge"
        );
    }

    fn app_empty_snapshot(app: &mut App) {
        app.insert_resource(crate::ai::server::WorldSnapshot { entities: vec![] });
    }

    // ── Boost AI operator (issue #780) ───────────────────────────────────────

    fn boost_command(app: &mut App) -> bool {
        app.world_mut()
            .query::<&crate::ship::helm::BoostCommand>()
            .single(app.world())
            .expect("ship must carry BoostCommand")
            .0
    }

    /// Build an app whose ship runs AI boost. Boost feature enabled, helm-boost
    /// on AI, an objective + a moving hazard so the plan carries urgency.
    fn boost_ai_app(policy: Option<crate::ai::policy::AiPolicy>) -> App {
        let mut app = test_app();
        // Full helm on AI: this puts a travel axis on AI so the shared frame +
        // hazard plan are built (the frame is gated on any of
        // thrust/steering/lateral/vertical/impulse being AI, not boost), and it
        // puts helm-boost on AI so the boost operator runs.
        set_helm_control_source(&mut app, ControlSource::Ai);
        set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
        set_behaviour_section(
            &mut app,
            crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            },
        );
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship::components::BoostConfigResource {
                enabled: true,
                ..Default::default()
            });
        if let Some(policy) = policy {
            app.world_mut()
                .entity_mut(ship)
                .insert(HelmBoostAiPolicy(policy));
        }
        snapshot_with_moving_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
        app
    }

    /// A boost policy that engages while the seeded hazard-urgency fact is above a
    /// threshold and boost is available.
    fn hazard_boost_policy() -> crate::ai::policy::AiPolicy {
        crate::ai::policy::AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: vec![crate::ai::policy::AiPolicyRule {
                priority: 10,
                channel: crate::entities::config::HELM_BOOST_CHANNEL.into(),
                when: crate::world::flags::parse_predicate(
                    "fact(hazard_urgency) > 0.0 and fact(boost_available) > 0",
                )
                .unwrap(),
                verb: crate::ai::policy::AiPolicyVerb::EngageBoost,
            }],
            idle: false,
            machine: None,
        }
    }

    /// AC1/AC6: `ai_helm_boost` emits a typed `SetBoost` through the same admitted
    /// seam a human uses, engaging boost when its authored policy fires and the
    /// feature is available.
    #[test]
    fn ai_helm_boost_engages_under_authored_hazard_policy() {
        let mut app = boost_ai_app(Some(hazard_boost_policy()));
        tick_twice(&mut app);
        assert!(
            boost_command(&mut app),
            "an authored boost policy firing on the seeded hazard fact must engage \
             boost through the admitted SetBoost seam"
        );
    }

    /// AC6 availability/capability filtering: with the boost feature absent, the
    /// operator stands down and boost never engages, however urgent the hazard —
    /// even under the same policy that engages it when available.
    #[test]
    fn ai_helm_boost_stands_down_without_boost_config() {
        let mut app = boost_ai_app(Some(hazard_boost_policy()));
        // Strip the boost capability.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ship::components::BoostConfigResource>();
        tick_twice(&mut app);
        assert!(
            !boost_command(&mut app),
            "no BoostConfigResource means no boost capability — the operator must \
             emit nothing"
        );
    }

    /// Baseline preservation (issue #780): the canonical default boost policy is
    /// idle, so a ship that authors no `[helm_console.boost_ai]` never AI-boosts,
    /// exactly as before #780 — even with the feature enabled and a live hazard.
    #[test]
    fn ai_helm_boost_default_idle_never_engages() {
        // No policy component → the host falls back to the idle default.
        let mut app = boost_ai_app(None);
        tick_twice(&mut app);
        assert!(
            !boost_command(&mut app),
            "the default idle boost policy must never engage boost (the pre-#780 \
             baseline: no AI boost)"
        );
    }

    // ── AC8 demo host: the minimal stateful Boost policy (issue #882) ────────

    /// The AC8 demonstrator, authored as TOML and decoded through the real
    /// schema path so the test exercises content authoring, not a hand-built
    /// typed value. Two states on the existing `boost` channel: `cruise` holds
    /// boost and leaves for `surge` when the seeded hazard-urgency fact crosses
    /// the AUTHORED `surge_urgency` param; `surge` engages boost unconditionally
    /// and returns to `cruise` once `state_time` reaches the AUTHORED
    /// `surge_dwell_secs`. Every threshold is an authored param (AGENTS.md #11)
    /// — there is not a gameplay number in the Rust.
    ///
    /// It also READS the host-written private memory: `cruise` only surges
    /// while `memory(engagements)` is under the authored `max_engagements` cap.
    /// That closes the #882 loop — the host writes the slot on entering a
    /// boost-engaging state, the authored guard reads it back on a later tick —
    /// and it is why `memory(...)` is not just a second spelling of `param`.
    fn stateful_boost_policy() -> crate::ai::policy::AiPolicy {
        // A re-engagement cap far above anything these tests drive, so the
        // demonstrator behaves exactly as it did before the memory read was
        // authored; `stateful_boost_policy_capped` exercises the cap itself.
        stateful_boost_policy_with("3.0", "99.0")
    }

    /// The demonstrator with its authored dwell and re-engagement cap supplied,
    /// so a test can drive the memory read without a second copy of the TOML.
    fn stateful_boost_policy_with(
        surge_dwell_secs: &str,
        max_engagements: &str,
    ) -> crate::ai::policy::AiPolicy {
        let src = format!(
            r#"
initial_state = "cruise"

[param]
surge_urgency = 0.0
surge_dwell_secs = {surge_dwell_secs}
max_engagements = {max_engagements}

[memory]
engagements = 0.0
peak_hazard_urgency = 0.0

[[state]]
id = "cruise"

[[state.transition]]
priority = 10
to = "surge"
when = "fact(hazard_urgency) > param(surge_urgency) and fact(boost_available) > 0 and memory(engagements) < param(max_engagements)"

[[state]]
id = "surge"

[[state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"

[[state.transition]]
priority = 0
to = "cruise"
when = "state_time >= param(surge_dwell_secs)"
"#
        );
        let cfg: crate::entities::config::FineSystemAiConfigToml =
            toml::from_str(&src).expect("the authored stateful boost policy parses");
        assert!(
            crate::entities::config::validate_fine_system_ai_policy(
                &cfg,
                &[crate::entities::config::HELM_BOOST_CHANNEL],
                &[crate::entities::config::HELM_ENGAGE_BOOST_VERB],
            )
            .is_ok(),
            "the demo policy must pass real content validation"
        );
        cfg.to_policy().expect("decodes to a typed machine")
    }

    fn boost_policy_state(app: &mut App) -> crate::ai::policy::AiPolicyRuntimeState {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<HelmBoostAiPolicyState>()
            .expect("ship must carry HelmBoostAiPolicyState")
            .0
            .clone()
    }

    /// AC8 (+ AC1, AC2): the minimal stateful host end to end. The machine
    /// starts in the authored initial state `cruise`, which holds boost. On the
    /// tick its transition guard fires, `ai_policy_state_tick` commits `surge`
    /// BEFORE `ai_helm_boost` resolves — so the entered state's continuous rule
    /// engages boost through the same admitted `SetBoost` seam a human uses,
    /// in that very tick.
    #[test]
    fn stateful_boost_policy_transitions_and_engages_in_the_same_tick() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        // Before any tick: nothing committed, nothing engaged.
        assert!(!boost_command(&mut app));

        tick_twice(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "surge",
            "the hazard guard must carry the machine out of `cruise`"
        );
        assert!(
            boost_command(&mut app),
            "the entered state's continuous rule must engage boost through the \
             admitted SetBoost seam in the same tick the transition committed"
        );
    }

    /// AC2 (one transition per tick) at the host: `surge` can only return to
    /// `cruise` after the authored dwell, so a machine cannot walk two edges in
    /// one tick — the state is `surge`, never back at `cruise`, immediately
    /// after the first transition.
    #[test]
    fn stateful_boost_policy_fires_at_most_one_transition_per_tick() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        tick_twice(&mut app);
        assert_eq!(boost_policy_state(&mut app).current, "surge");
        // Several more ticks inside the authored dwell keep it there: the AI
        // tick cadence is 30 Hz and the dwell is 3 s, so ~90 ticks would be
        // needed. One tick can only ever advance one edge.
        tick_twice(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "surge",
            "no second edge may be walked while the authored dwell holds"
        );
    }

    /// AC4: state time is derived from the shared AI tick cadence, not from
    /// `Time::delta`. The clock advances by exactly one authored tick period
    /// per gated run, so `entered_at_secs` and the state clock are reproducible
    /// regardless of frame rate.
    #[test]
    fn stateful_policy_state_time_advances_on_the_shared_ai_tick() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        tick_twice(&mut app);
        let before = app.world().resource::<AiPolicyTickClock>().0;
        tick_twice(&mut app);
        let after = app.world().resource::<AiPolicyTickClock>().0;
        assert!(
            after > before,
            "the tick-derived policy clock must advance on gated ticks"
        );
        let period = 1.0 / crate::entity_config::GlobalConfig::default().ai_helm_tick_hz as f64;
        let advanced = after - before;
        assert!(
            (advanced % period).abs() < 1e-9 || ((advanced % period) - period).abs() < 1e-9,
            "the clock must advance in whole authored tick periods, got {advanced}"
        );
    }

    /// AC5: policy state resets when the system is unavailable, and again when
    /// AI regains control — a recovered system never resumes a stale
    /// mid-manoeuvre state.
    #[test]
    fn stateful_policy_state_resets_when_the_system_is_unavailable_and_on_recovery() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        tick_twice(&mut app);
        assert_eq!(boost_policy_state(&mut app).current, "surge");

        // Boost becomes unavailable (the capability is stripped): the machine
        // is put back to the authored initial state rather than left in
        // `surge`, and boost stands down.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ship::components::BoostConfigResource>();
        tick_twice(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "cruise",
            "an unavailable system must reset its policy state"
        );

        // The system recovers: it restarts from `cruise` (proved above) and
        // re-earns `surge` through its guard rather than resuming it.
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship::components::BoostConfigResource {
                enabled: true,
                ..Default::default()
            });
        tick_twice(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "surge",
            "on recovery the machine re-enters `surge` via its guard, from initial"
        );
    }

    /// AC5 (the other half): a system NOT operated by AI holds at the authored
    /// initial state, so the tick AI gains control begins from `initial`.
    #[test]
    fn stateful_policy_state_holds_at_initial_while_ai_does_not_operate_the_system() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        set_helm_control_source(&mut app, ControlSource::Human);
        tick_twice(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "cruise",
            "a human-operated system's policy state stays at initial"
        );
        assert!(!boost_command(&mut app));
    }

    /// AC7 at the HOST: the stateless boost path is untouched. The same host,
    /// given a #775-shaped policy, still resolves through `resolve_channel` and
    /// never touches the state component — which stays at its default.
    #[test]
    fn stateless_boost_policy_never_enters_the_state_machine_path() {
        let mut app = boost_ai_app(Some(hazard_boost_policy()));
        tick_twice(&mut app);
        assert!(
            boost_command(&mut app),
            "the stateless hazard policy engages exactly as it did before #882"
        );
        assert_eq!(
            boost_policy_state(&mut app).current,
            "",
            "a stateless policy must leave the state component untouched"
        );
    }

    // ── Host-written private memory (issue #882) ─────────────────────────────

    fn boost_memory(app: &mut App, slot: &str) -> Option<f64> {
        boost_policy_state(app).memory.get(slot)
    }

    /// Empty the world snapshot, so the shared plan carries no hazard and the
    /// live `hazard_urgency` reading falls back to zero.
    fn clear_snapshot(app: &mut App) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: Vec::new(),
        });
    }

    /// Tick until the machine reaches `want`, so a test does not depend on
    /// exactly which gated tick the shared plan first carries hazard.
    fn tick_until_state(app: &mut App, want: &str) {
        for _ in 0..8 {
            if boost_policy_state(app).current == want {
                return;
            }
            tick(app);
        }
        panic!(
            "machine never reached `{want}` (stuck in `{}`)",
            boost_policy_state(app).current
        );
    }

    /// Finding 1's guard, at the host: the PRODUCTION player ship is a
    /// `LocalShip`, and a `LocalShip` carrying a stateful boost policy must
    /// actually transition. It only can if it carries `HelmBoostAiPolicyState`
    /// — which it now takes from `ai_high_fidelity_components`, the one
    /// definition both `lod_ai_ships` and `server_app::spawn_game_start_entities`
    /// insert. Before that, the player ship silently had no state component,
    /// `ai_policy_state_tick`'s non-optional query skipped it, and the host fell
    /// through to the stateless arm with an empty top-level rule list: boost
    /// never engaged, with no warning.
    #[test]
    fn local_ship_with_a_stateful_boost_policy_transitions_and_engages() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        let ship = find_ship_entity(&mut app);
        assert!(
            app.world()
                .entity(ship)
                .get::<crate::simulation::LocalShip>()
                .is_some(),
            "this is the player-ship spawn shape, not an NPC's"
        );
        assert!(
            app.world()
                .entity(ship)
                .get::<HelmBoostAiPolicyState>()
                .is_some(),
            "the LocalShip must carry the per-fine-system policy state component"
        );
        tick_twice(&mut app);
        assert_eq!(boost_policy_state(&mut app).current, "surge");
        assert!(boost_command(&mut app));
    }

    /// The writer exists and its value PERSISTS across ticks.
    ///
    /// `peak_hazard_urgency` is a running maximum the host folds every tick. It
    /// is not authored (so it cannot be a `param`) and it is not a reading of
    /// this tick (so it cannot be a `fact`): retention is the whole content of
    /// the slot. Later ticks whose hazard is lower must not lower it.
    #[test]
    fn host_written_memory_persists_across_ticks() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        tick_until_state(&mut app, "surge");
        let peak = boost_memory(&mut app, PEAK_HAZARD_MEMORY)
            .expect("the host must have written the peak-hazard slot");
        assert!(
            peak > 0.0,
            "the seeded moving hazard must have been recorded, got {peak}"
        );

        // Clear the hazard: this tick's reading is 0, but the retained maximum
        // is not a reading of this tick.
        clear_snapshot(&mut app);
        tick_twice(&mut app);
        tick_twice(&mut app);
        assert_eq!(
            boost_memory(&mut app, PEAK_HAZARD_MEMORY),
            Some(peak),
            "a retained maximum must survive ticks whose live reading is lower"
        );
    }

    /// The written value SURVIVES a transition: `engagements` is incremented on
    /// entering the boost-engaging `surge` state and is still there after the
    /// machine walks the edge back to `cruise`. State time restarts on entry;
    /// private memory deliberately does not.
    #[test]
    fn host_written_memory_survives_a_transition() {
        // Zero dwell so `surge` returns to `cruise` as soon as it is re-eligible,
        // and a cap high enough that the memory read never blocks the re-entry.
        let mut app = boost_ai_app(Some(stateful_boost_policy_with("0.0", "99.0")));
        tick_until_state(&mut app, "surge");
        assert_eq!(
            boost_memory(&mut app, ENGAGEMENTS_MEMORY),
            Some(1.0),
            "entering a boost-engaging state must increment the host-written slot"
        );

        // Walk the edge back to `cruise`. State time restarts; memory does not.
        tick_until_state(&mut app, "cruise");
        let state = boost_policy_state(&mut app);
        assert_eq!(
            state.memory.get(ENGAGEMENTS_MEMORY),
            Some(1.0),
            "private memory must survive the transition that follows it"
        );
    }

    /// The POLICY reads what the host wrote. With the authored re-engagement cap
    /// at one, the machine surges once, returns on the zero dwell, and can never
    /// surge again — because `cruise`'s guard reads `memory(engagements)`. If
    /// the slot were frozen at its declared 0.0 (i.e. behaviourally a `param`),
    /// the machine would surge again immediately.
    #[test]
    fn authored_guard_reads_the_host_written_memory() {
        let mut app = boost_ai_app(Some(stateful_boost_policy_with("0.0", "1.0")));
        tick_until_state(&mut app, "surge");
        assert_eq!(boost_memory(&mut app, ENGAGEMENTS_MEMORY), Some(1.0));

        // Back to cruise on the zero dwell...
        tick_until_state(&mut app, "cruise");
        // ...and the cap now holds it there, with the hazard still live.
        for _ in 0..10 {
            tick(&mut app);
            assert_eq!(
                boost_policy_state(&mut app).current,
                "cruise",
                "the authored cap must be read from host-written memory"
            );
        }
    }

    /// The reset CLEARS it. An unavailable system is reset to the authored
    /// initial state AND the authored memory, so a recovered system never
    /// resumes a stale count (AC5).
    #[test]
    fn host_written_memory_is_cleared_by_the_reset() {
        let mut app = boost_ai_app(Some(stateful_boost_policy()));
        tick_until_state(&mut app, "surge");
        assert_eq!(boost_memory(&mut app, ENGAGEMENTS_MEMORY), Some(1.0));
        assert!(boost_memory(&mut app, PEAK_HAZARD_MEMORY).unwrap_or(0.0) > 0.0);

        // Strip the capability → AC5 reset.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ship::components::BoostConfigResource>();
        tick_twice(&mut app);
        assert_eq!(boost_policy_state(&mut app).current, "cruise");
        assert_eq!(
            boost_memory(&mut app, ENGAGEMENTS_MEMORY),
            Some(0.0),
            "reset must restore the AUTHORED memory, not keep the drifted count"
        );
        assert_eq!(
            boost_memory(&mut app, PEAK_HAZARD_MEMORY),
            Some(0.0),
            "every host-written slot goes back to its authored declaration"
        );
    }

    /// `ai_helm_boost` runs on the shared AI-helm sim tick like its four siblings
    /// (issue #780 + #803), not once per rendered frame.
    #[test]
    fn ai_helm_boost_runs_on_the_shared_sim_tick_not_per_frame() {
        // A boost policy keyed off a sentinel-independent fact so it toggles
        // deterministically: engage while the seeded hazard is present. The probe
        // measures BoostCommand transitions, which only the operator can drive.
        let mut app = boost_ai_app(Some(hazard_boost_policy()));
        let counts = count_sim_tick_runs(
            &mut app,
            // Arm: force the applied BoostCommand back off so a run is observable
            // as a re-engage. (The operator re-emits only on change.)
            |app| {
                let ship = find_ship_entity(app);
                let mut entity = app.world_mut().entity_mut(ship);
                entity
                    .get_mut::<crate::ship::helm::BoostCommand>()
                    .unwrap()
                    .0 = false;
                *entity.get_mut::<ShipBoost>().unwrap() = ShipBoost::default();
            },
            boost_command,
        );
        assert_shared_sim_tick_cadence("ai_helm_boost", counts);
    }

    // ── Avoidance bends travel; only imminent collision overrides facing ─────

    fn plan_desired_facing(app: &mut App, ship: Entity) -> Vec3 {
        app.world()
            .resource::<crate::ship::helm_planner::HelmMotionPlan>()
            .ships
            .get(&ship)
            .map(|sp| sp.motion.desired_facing_local)
            .unwrap_or_default()
    }

    /// An app steering toward a forward Reach with a moving hazard on the
    /// starboard bow. `imminent_collision_facing_threshold` is authored so the
    /// same hazard is either ordinary avoidance (default 1.0 — off) or an
    /// imminent-collision facing override (low threshold).
    fn avoidance_facing_app(imminent_threshold: f32) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        app.insert_resource(world_config_with_anchor("far-ahead", [0.0, 0.0, -900.0]));
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("far-ahead", 8.0)]);
        set_behaviour_section(
            &mut app,
            crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                imminent_collision_facing_threshold: imminent_threshold,
                ..Default::default()
            },
        );
        // Stationary: below `AVOIDANCE_MIN_SPEED`, so the ordinary steering
        // doctrine (which includes avoidance_steering) does NOT turn facing —
        // isolating the imminent-collision facing override as the only thing that
        // can move desired facing off the forward objective heading.
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 0.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        // Close hazard on the starboard bow → high urgency, port-ward repulsion.
        snapshot_with_moving_obstacle(&mut app, [3.0, 0.0, -10.0], 1.0, 0.0, 0.0);
        app
    }

    /// AC4: only an imminent collision may temporarily override desired facing.
    /// With the default (off) threshold the same in-range hazard leaves facing on
    /// the forward objective heading (≈ -Z, x ≈ 0); with a low authored threshold
    /// the imminent hazard overrides it toward the escape heading (a nonzero
    /// local-X facing away from the starboard threat). The ship is stationary so
    /// the ordinary avoidance-steering doctrine cannot itself turn facing —
    /// proving the override, not doctrine, is what moves it.
    #[test]
    fn facing_overridden_only_when_collision_imminent() {
        let mut ordinary = avoidance_facing_app(1.0);
        let ship_o = find_ship_entity(&mut ordinary);
        tick_twice(&mut ordinary);
        let ordinary_facing = plan_desired_facing(&mut ordinary, ship_o);
        assert!(
            ordinary_facing.x.abs() < 0.1 && ordinary_facing.z < 0.0,
            "ordinary avoidance must leave facing on the forward objective heading \
             (the doctrine never touches facing on hazards below the imminent \
             threshold), got {ordinary_facing:?}"
        );

        let mut imminent = avoidance_facing_app(0.01);
        let ship_i = find_ship_entity(&mut imminent);
        tick_twice(&mut imminent);
        let imminent_facing = plan_desired_facing(&mut imminent, ship_i);
        assert!(
            imminent_facing.x.abs() > 0.2,
            "an imminent collision must temporarily override facing toward the \
             escape heading (nonzero local-X), got {imminent_facing:?}"
        );
    }

    /// AC3: ordinary avoidance BENDS travel without changing the active doctrine.
    /// A forward Reach's throttle doctrine is identical with and without a hazard
    /// in range — the avoidance response shows up in the lateral dodge, not in a
    /// swapped travel decision.
    #[test]
    fn avoidance_bends_travel_without_changing_doctrine() {
        fn forward_throttle_and_dodge(with_hazard: bool) -> (f32, f32) {
            let mut app = lateral_thrust_ai_app(Some(crate::entity_config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }));
            // A forward objective so the engine doctrine commands forward travel.
            set_helm_control_source(&mut app, ControlSource::Ai);
            app.insert_resource(world_config_with_anchor("ahead", [0.0, 0.0, -900.0]));
            set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("ahead", 8.0)]);
            let mut physics = get_ship_physics(&mut app);
            physics.forward_speed = 10.0;
            physics.yaw = 0.0;
            set_ship_physics(&mut app, physics);
            if with_hazard {
                snapshot_with_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0);
            }
            tick_twice(&mut app);
            let ship = find_ship_entity(&mut app);
            let thrust = app
                .world()
                .resource::<crate::ship::helm_planner::HelmMotionPlan>()
                .ships
                .get(&ship)
                .map(|sp| {
                    crate::ai::decode_thrust_from_velocity(
                        sp.motion.desired_velocity_local.to_array(),
                    )
                })
                .unwrap_or(0.0);
            (thrust, lateral_intent(&mut app))
        }

        let (clear_thrust, clear_dodge) = forward_throttle_and_dodge(false);
        let (hazard_thrust, hazard_dodge) = forward_throttle_and_dodge(true);

        assert!(
            (clear_thrust - hazard_thrust).abs() < 1e-4,
            "the travel doctrine (forward throttle) must be UNCHANGED by avoidance; \
             clear={clear_thrust} hazard={hazard_thrust}"
        );
        assert_eq!(
            clear_dodge, 0.0,
            "no hazard means no dodge — precondition for the bend below"
        );
        assert!(
            hazard_dodge.abs() > 0.0,
            "avoidance must BEND travel via the lateral dodge, got {hazard_dodge}"
        );
    }

    /// AC6 capability filtering for impulse: with no `ImpulseConfigResource` the
    /// impulse operator stands down and never charges, even with an engaging
    /// objective geometry that otherwise would.
    #[test]
    fn ai_helm_impulse_stands_down_without_impulse_config() {
        let anchor = "station-alpha";
        let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
        app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
        // Strip the impulse capability the fixture installed.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<ImpulseConfigResource>();
        tick(&mut app);
        assert_eq!(
            get_impulse_command(&mut app),
            crate::impulse::ImpulsePhase::Idle,
            "no ImpulseConfigResource means no impulse capability — no charge"
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

    // ── The Harrow Destroyer fly-through attack pass (issue #883) ────────────
    //
    // These drive the SHIPPED hull's authored policies through a real ticking
    // app, so they fail on the content as well as on the code. Every assertion
    // below is about something observable — an admitted actuator input, the
    // ship's boost state, the committed policy state — never about an internal
    // computation.
    //
    // Positions are set directly rather than flown, because flying a 200-unit
    // approach at 200 ms per tick would take dozens of ticks and pin nothing
    // extra: the interesting events are the merge and the tick after it, and
    // setting the pose reaches them exactly and deterministically.

    const BOGEY: &str = "bogey";

    fn destroyer_hull() -> crate::entity_config::EntityConfig {
        crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_destroyer.toml"
        ))
        .expect("the shipped destroyer hull must parse")
    }

    /// Put a single named target into the world snapshot at `pos`, heading
    /// `yaw` at `speed`. The heading and speed matter: the closing rate is
    /// reconstructed from them, so a target with its own velocity is a
    /// genuinely different problem from a stationary one.
    fn set_bogey(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3], yaw: f32, speed: f32) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: pos,
                yaw: Some(yaw),
                forward_speed: speed,
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            }],
        });
    }

    /// A ship carrying the shipped destroyer's three authored policy machines,
    /// its physics envelope, and an enabled boost drive — the same components
    /// `entities::spawner` would attach — hunting a single named bogey.
    fn fly_through_app(bogey_pos: [f32; 3]) -> (App, uuid::Uuid) {
        fly_through_app_omitting(bogey_pos, &[])
    }

    /// As [`fly_through_app`], but with the named STEERING `param`s stripped
    /// from the hull before its policy is built — the partially-authored hull
    /// AGENTS.md #11 says must decline rather than invent.
    ///
    /// Each name must actually be present to begin with, so this cannot quietly
    /// pass by "removing" a param the hull renamed out from under it.
    fn fly_through_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
        let mut app = test_app();
        let cfg = destroyer_hull();
        let mut hc = cfg
            .helm_console
            .clone()
            .expect("hull declares [helm_console]");
        for name in omit {
            hc.steering_ai
                .as_mut()
                .expect("hull declares [helm_console.steering_ai]")
                .param
                .remove(*name)
                .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
        }
        let boost = hc
            .boost
            .clone()
            .expect("hull declares [helm_console.boost]");
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            HelmEnginesAiPolicy(hc.engines_ai.as_ref().unwrap().to_policy().unwrap()),
            HelmSteeringAiPolicy(hc.steering_ai.as_ref().unwrap().to_policy().unwrap()),
            HelmBoostAiPolicy(hc.boost_ai.as_ref().unwrap().to_policy().unwrap()),
            crate::ship_plugin::ShipPhysicsConfigResource(crate::ship_physics::ShipPhysicsConfig {
                max_speed: hc.max_speed,
                max_reverse_speed: hc.max_reverse_speed,
                acceleration: hc.acceleration,
                deceleration: hc.deceleration,
                max_yaw_rate: hc.max_yaw_rate,
                ..crate::ship_physics::ShipPhysicsConfig::new()
            }),
            BoostConfigResource {
                enabled: true,
                multiplier: boost.multiplier,
                steering_multiplier: boost.steering_multiplier,
                active_duration: boost.active_duration,
                recharge_duration: boost.recharge_duration,
            },
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective(BOGEY, 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);
        let uuid = uuid::Uuid::new_v4();
        set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
        (app, uuid)
    }

    fn steering_state(app: &mut App) -> String {
        app.world_mut()
            .query::<&HelmSteeringAiPolicyState>()
            .single(app.world())
            .expect("ship carries HelmSteeringAiPolicyState")
            .0
            .current
            .clone()
    }

    fn engines_state(app: &mut App) -> String {
        app.world_mut()
            .query::<&HelmEnginesAiPolicyState>()
            .single(app.world())
            .expect("ship carries HelmEnginesAiPolicyState")
            .0
            .current
            .clone()
    }

    fn pass_surface(app: &mut App) -> HelmPassSurface {
        *app.world_mut()
            .query::<&HelmPassSurface>()
            .single(app.world())
            .expect("ship carries HelmPassSurface")
    }

    fn boost_is_active(app: &mut App) -> bool {
        app.world_mut()
            .query::<&ShipBoost>()
            .single(app.world())
            .expect("ship carries ShipBoost")
            .0
            .is_active()
    }

    /// Fly the ship to a pose directly. Used to place it at the merge and past
    /// it — see the module note above.
    fn place_ship(app: &mut App, x: f32, z: f32, yaw: f32, speed: f32) {
        set_ship_physics(
            app,
            ShipPhysics {
                x,
                z,
                yaw,
                forward_speed: speed,
                ..Default::default()
            },
        );
    }

    /// AC5, stated as behaviour rather than as a fact dump: the machine's
    /// `acquire → inbound` guard reads `fact(range_to_target)`, so the axis only
    /// commits to a run once the target is inside the authored `commit_range`.
    ///
    /// Before #883 the two travel axes were handed `AiFacts::new()` — an EMPTY
    /// snapshot — so this guard would have validated at load and then been false
    /// for ever, and the machine could never have left its initial state. That
    /// this test can distinguish "far" from "near" at all is the proof the
    /// travel axes now see seeded facts.
    #[test]
    fn travel_axis_facts_are_seeded_so_the_commit_range_guard_actually_gates() {
        // Authored `commit_range` is 260; put the bogey well beyond it.
        let (mut app, uuid) = fly_through_app([0.0, 0.0, -900.0]);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "acquire",
            "a target beyond commit_range must not start a run"
        );
        assert_eq!(engines_state(&mut app), "acquire");

        // Bring it inside the authored commit range.
        set_bogey(&mut app, uuid, [0.0, 0.0, -150.0], 0.0, 0.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "inbound",
            "inside commit_range the machine must commit to the run — if this reads \
             `acquire` the travel axis is seeing empty facts again"
        );
        assert_eq!(
            engines_state(&mut app),
            "inbound",
            "Engines runs its OWN copy of the machine and must reach the same leg \
             from the same facts, not by reading Steering's state"
        );
    }

    /// AC1: the inbound leg is flown at the authored approach throttle, flat,
    /// and Steering tracks the target continuously.
    ///
    /// The throttle assertion is the one that separates this from
    /// `helm_destroy`: at 60 units from a target whose `maintain_range` is 40,
    /// the shared Destroy arm would be deep in its decel ramp and commanding a
    /// small fraction of `target_speed`. The pass commands its full authored
    /// approach fraction.
    #[test]
    fn inbound_leg_holds_the_authored_approach_speed_and_tracks_the_target() {
        let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
        // Two ticks: the first publishes the pass surface, the second is the
        // first planner pass that consumes it (see `HelmPassSurface`).
        tick_twice(&mut app);
        tick(&mut app);

        let pass = pass_surface(&mut app);
        assert!(pass.active, "the destroyer must be flying an authored pass");
        assert!(!pass.escape, "still inbound");
        assert!(
            (get_thrust_input(&mut app) - pass.approach_speed).abs() < 1e-3,
            "inbound throttle must be the flat authored approach fraction ({}), got {}",
            pass.approach_speed,
            get_thrust_input(&mut app)
        );

        // Dead ahead: nothing to correct.
        place_ship(&mut app, 0.0, 0.0, 0.0, 15.0);
        set_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 0.0, 0.0);
        tick(&mut app);
        tick(&mut app);
        assert!(
            get_steering_input(&mut app).abs() < 0.05,
            "a target dead ahead needs no turn, got {}",
            get_steering_input(&mut app)
        );

        // The target MOVES off the starboard bow: the facing solution is
        // re-derived, so the ship turns after it.
        place_ship(&mut app, 0.0, 0.0, 0.0, 15.0);
        set_bogey(&mut app, uuid, [180.0, 0.0, -100.0], 0.0, 0.0);
        tick(&mut app);
        assert!(
            get_steering_input(&mut app) > 0.0,
            "Steering must keep tracking a MOVING target while inbound, got {}",
            get_steering_input(&mut app)
        );
    }

    /// Drive one full merge: approach, pass through the closest point, and open
    /// out again. Leaves the app in the `escape` leg for the tests below.
    fn run_to_escape() -> (App, uuid::Uuid) {
        let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
        // Commit to the run.
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");

        // THE MERGE: the ship is right on top of the bogey, so the host folds a
        // small `min_range_seen` into every axis's private memory. Still
        // closing, so nothing transitions yet.
        place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "inbound",
            "at the merge itself the range is still shrinking: not yet closest approach"
        );

        // PAST IT: the ship has flown through and the range is opening again,
        // well past the authored hysteresis.
        place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
        tick(&mut app);
        (app, uuid)
    }

    /// AC2: closest approach ends target tracking and commits Engines, Steering
    /// and Boost to the heading held at the merge.
    ///
    /// All three axes must arrive together — and they do it by each running
    /// their own machine over the same seeded facts, which is why the assertion
    /// is over both travel states and the published leg rather than over one
    /// shared flag.
    #[test]
    fn closest_approach_commits_every_axis_to_the_outward_heading() {
        let (mut app, _uuid) = run_to_escape();

        assert_eq!(
            steering_state(&mut app),
            "escape",
            "the closing rate went negative and the range opened past the authored \
             hysteresis: that is closest approach"
        );
        assert_eq!(
            engines_state(&mut app),
            "escape",
            "Engines must reach the escape leg independently, from the same facts"
        );

        let pass = pass_surface(&mut app);
        assert!(pass.escape, "the published leg must be the escape");
        assert!(
            pass.escape_heading_rad.abs() < 1e-3,
            "the frozen heading must be the yaw held AT the merge (0), got {}",
            pass.escape_heading_rad
        );
    }

    /// AC2's fly-through half, and the reason `hold_committed_heading` exists as
    /// a verb rather than as a bare "hold".
    ///
    /// Once committed, the target is swung hard onto the beam. A tracking axis
    /// would haul the ship round after it — straight back into the ship it just
    /// passed. The committed axis flies its frozen heading and ignores it.
    #[test]
    fn the_escape_leg_ignores_the_target_and_flies_the_frozen_heading() {
        let (mut app, uuid) = run_to_escape();
        let frozen = pass_surface(&mut app).escape_heading_rad;

        // Hard to starboard — a bearing that would saturate a tracking solution.
        set_bogey(&mut app, uuid, [400.0, 0.0, -260.0], 0.0, 0.0);
        // Put the hull exactly ON the frozen heading first, so the only thing
        // that can command yaw this tick is the target. Left to fly, the boosted
        // escape carries up to one tick of yaw rate as a residual (see the
        // one-tick-offset note on `HelmPassSurface`), and this test would be
        // measuring that convergence rather than the commitment it is about.
        place_ship(&mut app, 0.0, -260.0, frozen, 20.0);
        tick(&mut app);

        assert_eq!(
            steering_state(&mut app),
            "escape",
            "nothing the target does may end the escape leg"
        );
        // Asserted as a SIGN rather than as a magnitude, because the pin above
        // makes the expected value exactly zero and an `abs() < tolerance` band
        // around zero would pass for any regression small enough to be quiet.
        // The bogey is hard to STARBOARD, so a leg that tracked it would command
        // a saturated POSITIVE yaw — which this catches, while still admitting
        // the free-flight case where the hull is converging back onto the frozen
        // heading from the other side.
        assert!(
            get_steering_input(&mut app) <= 0.0,
            "the escape must fly the FROZEN heading, not turn back onto the target \
             off the starboard beam; got steering {}",
            get_steering_input(&mut app)
        );
        assert!(
            (pass_surface(&mut app).escape_heading_rad - frozen).abs() < 1e-3,
            "the frozen heading must not be re-derived while the leg runs"
        );
        // ...and it is still driving away under power.
        assert!(
            get_thrust_input(&mut app) > 0.0,
            "the escape leg never brakes"
        );
    }

    /// The escape leg outlives its target — which in combat is the ORDINARY
    /// case, because the pass is what kills the target.
    ///
    /// `plan_fly_through_pass` never reads `target_pos` on the escape leg, and
    /// neither escape state carries a `target_valid < 1` transition (only
    /// `inbound` does), so a destroyer that has committed must keep flying the
    /// frozen heading at the authored escape throttle with nothing in the world
    /// at all. Gating the escape on a resolvable target instead dropped it back
    /// to ordinary doctrine travel, which — with no objective geometry left —
    /// commands zero thrust and zero yaw: the hull brakes to a standstill in the
    /// middle of its escape while the Boost machine, independent of the planner,
    /// keeps the drive lit for the remaining dwell.
    #[test]
    fn the_escape_leg_survives_its_target_dying() {
        let (mut app, _uuid) = run_to_escape();
        let escape_speed = pass_surface(&mut app).escape_speed;
        let frozen = pass_surface(&mut app).escape_heading_rad;

        // The target is destroyed: nothing left in the world to resolve, so the
        // frame has no destroy target and no merged-view entity for it.
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: Vec::new(),
        });
        // Put the hull off the frozen heading by well more than the authored
        // deadband, so "still solving against the frozen heading" is observable
        // as a real correction rather than as a deadbanded zero.
        place_ship(&mut app, 0.0, -260.0, frozen + 0.3, 20.0);
        tick_twice(&mut app);

        assert_eq!(
            steering_state(&mut app),
            "escape",
            "only the authored dwell ends the escape — a dead target must not"
        );
        assert!(
            pass_surface(&mut app).active && pass_surface(&mut app).escape,
            "the published surface must still be an active escape leg"
        );
        assert!(
            (get_thrust_input(&mut app) - escape_speed).abs() < 1e-3,
            "the escape must still be flown at the authored escape throttle ({escape_speed}), \
             got {} — a target-gated escape brakes the destroyer to a standstill",
            get_thrust_input(&mut app)
        );
        assert!(
            get_steering_input(&mut app).abs() > 0.05,
            "the escape must still SOLVE against the frozen heading with the hull \
             0.3 rad off it, got steering {}",
            get_steering_input(&mut app)
        );
        assert!(
            (pass_surface(&mut app).escape_heading_rad - frozen).abs() < 1e-3,
            "and the frozen heading itself is untouched by the target's death"
        );
    }

    /// AC8's boost-out: the authored escape rule lights the drive through the
    /// same admitted `SetBoost` a human sends, and only on the escape leg.
    #[test]
    fn the_escape_leg_boosts_out_and_the_approach_does_not() {
        let (mut app, _uuid) = fly_through_app([0.0, 0.0, -200.0]);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");
        assert!(
            !boost_is_active(&mut app),
            "the approach is flown at normal speed: boost stays cold"
        );

        let (mut app, _uuid) = run_to_escape();
        tick(&mut app);
        assert_eq!(steering_state(&mut app), "escape");
        assert!(
            boost_is_active(&mut app),
            "the escape leg must engage boost through the shared admitted SetBoost path"
        );
    }

    /// AC3: shared hazard avoidance BENDS the escape without changing any pass
    /// state.
    ///
    /// This is why the frozen heading is expressed as a desired FACING through
    /// the motion planner rather than as a raw `SteeringInput` override: the
    /// #780 hazard contribution is folded into the same pure arm, so it still
    /// composes. A raw override would fly the destroyer straight through the
    /// rock.
    #[test]
    fn a_hazard_bends_the_escape_without_changing_the_pass_state() {
        let (mut app, uuid) = run_to_escape();
        let before_state = steering_state(&mut app);
        let before_heading = pass_surface(&mut app).escape_heading_rad;

        // Clear escape first: nothing to avoid, so no yaw. On the frozen heading
        // exactly, so the baseline is not reading the boosted escape's own
        // one-tick convergence residual (see `HelmPassSurface`).
        set_bogey(&mut app, uuid, [0.0, 0.0, -100.0], 0.0, 0.0);
        place_ship(&mut app, 0.0, -260.0, before_heading, 20.0);
        tick(&mut app);
        // A sign, not a band around zero: the pin above makes the expected value
        // exactly zero, so `abs() < tolerance` would wave through any quiet
        // regression. The bogey is dead ASTERN of the frozen heading, so a leg
        // that tracked it would command a saturated positive yaw to haul the
        // ship round; the negative half stays in scope for the free-flight case.
        assert!(
            get_steering_input(&mut app) <= 0.0,
            "baseline: an unobstructed escape commands no yaw toward the target, got {}",
            get_steering_input(&mut app)
        );

        // Drop a rock onto the projected escape path, keeping the bogey where
        // it was so the only new thing in the world is the hazard.
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![
                crate::ai::AiWorldEntity {
                    uuid,
                    name: Some(BOGEY.into()),
                    position: [0.0, 0.0, -100.0],
                    yaw: Some(0.0),
                    radius: 3.0,
                    size_rating: 3.0,
                    movable: true,
                    dangerous: true,
                    ..Default::default()
                },
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::new_v4(),
                    name: Some("rock".into()),
                    position: [4.0, 0.0, -320.0],
                    yaw: None,
                    radius: 8.0,
                    size_rating: 8.0,
                    movable: false,
                    dangerous: true,
                    ..Default::default()
                },
            ],
        });
        tick(&mut app);

        assert!(
            get_steering_input(&mut app).abs() > 0.0,
            "a hazard on the escape path must bend the escape"
        );
        assert_eq!(
            steering_state(&mut app),
            before_state,
            "avoidance is a steering force, NOT an input to the pass state machine"
        );
        assert!(
            pass_surface(&mut app).escape,
            "and the published leg is unchanged"
        );
        assert!(
            (pass_surface(&mut app).escape_heading_rad - before_heading).abs() < 1e-3,
            "bending the escape must not rewrite the committed heading"
        );
    }

    /// The running range minimum is scoped to the TARGET as well as to the
    /// state, so a mid-`inbound` target switch cannot fire a closest approach
    /// the destroyer never flew.
    ///
    /// The machine calls closest approach from `range_above_min_seen`, i.e.
    /// `range_to_target - memory(min_range_seen)`. Fold that across a target
    /// swap and the minimum belongs to a different ship: pick up a new contact
    /// further out and the subtraction reads as a huge re-opening, every
    /// conjunct of the authored guard passes at once, and the destroyer commits
    /// to an escape from a target it has not even closed on yet.
    ///
    /// The two halves below differ ONLY in whether the bogey keeps its uuid, so
    /// this pins the identity scoping specifically and not merely "no commit".
    #[test]
    fn a_mid_inbound_target_switch_does_not_fire_a_spurious_closest_approach() {
        // Positive control: the SAME target, first close and then far astern.
        // That is a genuine fly-through, and it must still commit.
        let (mut app, uuid) = fly_through_app([0.0, 0.0, -50.0]);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");
        set_bogey(&mut app, uuid, [0.0, 0.0, 200.0], 0.0, 0.0);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "escape",
            "the same target, now 200 units astern of a ship still driving away \
             from it, IS a closest approach"
        );

        // The real case: a DIFFERENT ship inherits the bogey role mid-run, and
        // it happens to be further out than the minimum the previous target
        // accumulated. Nothing about the destroyer's own run has changed.
        let (mut app, _uuid) = fly_through_app([0.0, 0.0, -50.0]);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");
        set_bogey(&mut app, uuid::Uuid::new_v4(), [0.0, 0.0, 200.0], 0.0, 0.0);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "inbound",
            "a target SWITCH restarts the range fold: the old target's minimum must \
             not synthesise a range_above_min_seen spike and commit the escape"
        );
        assert_eq!(
            engines_state(&mut app),
            "inbound",
            "Engines runs its own copy of the machine over its own private memory \
             and must be scoped the same way"
        );
        assert!(
            !pass_surface(&mut app).escape,
            "and no escape leg is published"
        );
    }

    /// A stateless policy for one travel channel whose single rule is gated on
    /// `fact(boost_available)` and nothing else — so whether the axis actuates
    /// at all is a direct readout of what the host seeded that fact to.
    fn boost_availability_gated_policy(
        channel: &str,
        verb: crate::ai::policy::AiPolicyVerb,
    ) -> crate::ai::policy::AiPolicy {
        crate::ai::policy::AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: vec![crate::ai::policy::AiPolicyRule {
                priority: 10,
                channel: channel.into(),
                when: crate::world::flags::parse_predicate("fact(boost_available) > 0").unwrap(),
                verb,
            }],
            idle: false,
            machine: None,
        }
    }

    /// A ship chasing a Reach anchor off the starboard bow, with the travel-axis
    /// policy `policy` attached and boost capability present or absent.
    fn availability_fact_app(policy: impl Bundle, boost_enabled: Option<bool>) -> App {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert(policy);
        if let Some(enabled) = boost_enabled {
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ship::components::BoostConfigResource {
                    enabled,
                    ..Default::default()
                });
        }
        app
    }

    /// The #779 empty-facts trap, one fact narrower: `ai_helm_thrust` and
    /// `ai_helm_steering` used to pass a HARDCODED `false` for both availability
    /// facts, so a travel-axis guard on `fact(boost_available)` validated at load
    /// and then read 0 for ever — silently wrong in exactly the way an absent
    /// fact is. They now seed it from the ship's own `BoostConfigResource`, as
    /// `ai_policy_state_tick` and `ai_helm_boost` already did.
    #[test]
    fn a_travel_axis_guard_reads_the_real_boost_availability() {
        // ── Engines ─────────────────────────────────────────────────────────
        let mut available = availability_fact_app(
            HelmEnginesAiPolicy(boost_availability_gated_policy(
                crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
            )),
            Some(true),
        );
        tick(&mut available);
        assert!(
            get_thrust_input(&mut available) > 0.0,
            "an Engines guard on fact(boost_available) must fire on a ship that HAS \
             an enabled boost drive; got thrust {}",
            get_thrust_input(&mut available)
        );

        let mut unavailable = availability_fact_app(
            HelmEnginesAiPolicy(boost_availability_gated_policy(
                crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
            )),
            None,
        );
        tick(&mut unavailable);
        assert_eq!(
            get_thrust_input(&mut unavailable),
            0.0,
            "and must NOT fire on a ship with no boost drive — the fact is a real \
             reading of capability, not a constant"
        );

        // ── Steering ────────────────────────────────────────────────────────
        let mut available = availability_fact_app(
            HelmSteeringAiPolicy(boost_availability_gated_policy(
                crate::entities::config::HELM_YAW_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
            )),
            Some(true),
        );
        tick(&mut available);
        assert!(
            get_steering_input(&mut available).abs() > 0.0,
            "a Steering guard on fact(boost_available) must fire on a ship that HAS \
             an enabled boost drive; got steering {}",
            get_steering_input(&mut available)
        );

        let mut unavailable = availability_fact_app(
            HelmSteeringAiPolicy(boost_availability_gated_policy(
                crate::entities::config::HELM_YAW_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
            )),
            Some(false),
        );
        tick(&mut unavailable);
        assert_eq!(
            get_steering_input(&mut unavailable),
            0.0,
            "a feature-DISABLED boost drive reads unavailable too, exactly as it does \
             on the boost axis itself"
        );
    }

    /// AC5's reset, on the new axes: a travel axis that is not AI-operated holds
    /// its machine at the authored initial state, so the tick AI gains control
    /// never resumes a stale mid-pass leg.
    #[test]
    fn a_human_held_travel_axis_holds_its_machine_at_initial() {
        let (mut app, _uuid) = run_to_escape();
        assert_eq!(steering_state(&mut app), "escape");

        set_helm_control_source(&mut app, ControlSource::Human);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "acquire",
            "a human-flown axis resets to the authored initial state"
        );
        assert_eq!(engines_state(&mut app), "acquire");
        assert!(
            !pass_surface(&mut app).active,
            "and the planner stops being offered a pass at all"
        );
    }

    // ── The shield-recovery standoff orbit (issue #788) ──────────────────────
    //
    // Same posture as the fly-through tests above: the SHIPPED hull's authored
    // policies driven through a real ticking app, asserting only on observable
    // things — admitted actuator inputs, the ship's boost state, the committed
    // policy state, the published pass surface.
    //
    // Two things are held constant across the long dwells below (the escape is
    // an authored 7 seconds, which is 210 shared AI ticks): the ship's pose and
    // its shield fraction. Both would otherwise drift — the hull keeps flying,
    // and `tick_shields` keeps regenerating — and a test that let them drift
    // would be measuring the drift rather than the doctrine.

    /// How far the bogey below can shoot. Not the authored margin and not a
    /// round number, so a safe ring computed as "reach + margin" is
    /// distinguishable from one that quietly used only one of them.
    const BOGEY_DIRECT_FIRE_REACH: f32 = 90.0;

    /// The destroyer's authored `safe_range_margin`, restated so the expected
    /// ring below is arithmetic on named values rather than a magic constant.
    fn authored_steering_param(name: &str) -> f32 {
        let cfg = destroyer_hull();
        cfg.helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .and_then(|ai| ai.param.get(name).copied())
            .unwrap_or_else(|| panic!("the shipped hull must author `{name}`"))
    }

    /// As [`authored_steering_param`], for the BOOST axis's own param table —
    /// the machine that owns when the drive is lit, and so the one that authors
    /// `boost_min_speed_fraction`.
    fn authored_boost_param(name: &str) -> f32 {
        let cfg = destroyer_hull();
        cfg.helm_console
            .as_ref()
            .and_then(|hc| hc.boost_ai.as_ref())
            .and_then(|ai| ai.param.get(name).copied())
            .unwrap_or_else(|| panic!("the shipped hull's boost axis must author `{name}`"))
    }

    /// The hull's authored `max_speed`, so a test can turn an authored speed
    /// FRACTION into the world-units speed `place_ship` takes.
    fn authored_max_speed() -> f32 {
        destroyer_hull()
            .helm_console
            .as_ref()
            .expect("hull declares [helm_console]")
            .max_speed
    }

    /// A bogey that can shoot back — the reach a standoff ring is derived from.
    fn set_armed_bogey(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3], reach: f32) {
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: pos,
                yaw: Some(0.0),
                forward_speed: 0.0,
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                direct_fire_range: reach,
                ..Default::default()
            }],
        });
    }

    /// Force this ship's shields to `fraction` of capacity, online.
    ///
    /// Written through the real `ShipShields` component rather than through a
    /// fact, so the guard under test reads the same surface production reads.
    fn set_shield_fraction(app: &mut App, fraction: f32) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut shields = entity
            .get_mut::<crate::ship::shields::ShipShields>()
            .expect("the test ship carries ShipShields");
        for facing in &mut shields.0.facings {
            facing.hp = (facing.max_hp as f32 * fraction).round() as i32;
            facing.offline_remaining = 0.0;
        }
    }

    fn shield_fraction(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&crate::ship::shields::ShipShields>()
            .single(app.world())
            .expect("ship carries ShipShields")
            .0
            .fraction()
    }

    /// Advance the shared AI-policy clock by roughly `secs`, pinning the ship's
    /// pose and shield fraction every tick.
    ///
    /// `+3` covers the clock's own quantisation and the one-tick offset between
    /// publishing the pass surface and the planner consuming it.
    fn hold_and_tick(app: &mut App, secs: f32, pose: (f32, f32, f32, f32), shields: f32) {
        let ticks = (secs * 30.0).ceil() as usize + 3;
        for _ in 0..ticks {
            place_ship(app, pose.0, pose.1, pose.2, pose.3);
            set_shield_fraction(app, shields);
            tick(app);
        }
    }

    /// Where the destroyer sits out the escape dwell in [`run_to_recovery`],
    /// with the bogey at `z = -200`: 120 units astern of it.
    ///
    /// The distance is pinned between two lines, and the fixture is only a
    /// RECOVERY fixture while it stays between them:
    ///
    /// * beyond the bogey's own [`BOGEY_DIRECT_FIRE_REACH`] (90), so
    ///   `inside_threat_range` reads false and the escape counts as having
    ///   worked. Inside it — where this fixture sat before issue #789 — a
    ///   stationary destroyer is by definition an escape that gained nothing
    ///   under the enemy's guns, so the machine correctly takes the PRESSED
    ///   branch instead and every assertion below is about the wrong doctrine;
    /// * inside the safe ring by more than `safe_ring_tolerance`
    ///   (`90 + 120 - 25 = 185`), so the distance history is full of breaches
    ///   when recovery begins and re-entry is not already satisfied.
    const RECOVERY_DWELL_Z: f32 = -320.0;

    /// Fly a complete pass against an ARMED bogey and sit out the authored
    /// escape dwell with the shields collapsed, leaving the machine in
    /// `recover` and the ship well inside the safe ring.
    fn run_to_recovery() -> (App, uuid::Uuid) {
        run_to_recovery_omitting(&[])
    }

    /// Fly one complete merge against an ARMED bogey of the given `reach` and
    /// stop the instant the escape commits.
    ///
    /// The shared front half of every dwell fixture below: what separates a
    /// recovery from a pressed run is only how the destroyer spends the escape
    /// dwell that starts here, so sharing the approach keeps that the *only*
    /// difference between them.
    fn run_to_armed_escape(omit: &[&str], reach: f32) -> (App, uuid::Uuid) {
        let (mut app, uuid) = fly_through_app_omitting([0.0, 0.0, -200.0], omit);
        set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], reach);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");
        place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
        tick(&mut app);
        place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(steering_state(&mut app), "escape");
        (app, uuid)
    }

    /// As [`run_to_recovery`], but flying a hull whose STEERING policy is
    /// missing the named recovery `param`s.
    fn run_to_recovery_omitting(omit: &[&str]) -> (App, uuid::Uuid) {
        let (mut app, uuid) = run_to_armed_escape(omit, BOGEY_DIRECT_FIRE_REACH);
        // Sit out the dwell with the shields gone, parked at `RECOVERY_DWELL_Z`
        // — see that constant for why the distance matters in two directions at
        // once.
        hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
        (app, uuid)
    }

    /// The "decline rather than invent" gate covers all SIX recovery scalars,
    /// not only the four the pass surface reads for itself.
    ///
    /// `safe_ring_tolerance` and `safe_distance_window_ticks` are consumed by
    /// `seed_recovery_facts` instead, and a hull that omits either can never
    /// satisfy `fact(safe_distance_held)`: without the window the history keeps
    /// its `Default` capacity of zero, so `is_full` — and therefore
    /// `all_at_least` — is false for ever; without the tolerance the fold falls
    /// through to its `_ => false` arm. The AC6 re-entry conjunct then cannot be
    /// met and the hull flies the standoff ring indefinitely, which is a
    /// strictly WORSE failure than the documented one. So either name missing,
    /// on its own, must decline the whole arm.
    ///
    /// The shipped hull reaches `recover` and publishes `recover = true` at this
    /// exact point (asserted by the tests below), so nothing here passes for
    /// want of getting that far.
    #[test]
    fn a_hull_omitting_either_history_scalar_declines_the_recovery_arm() {
        for omitted in [SAFE_RING_TOLERANCE_PARAM, SAFE_DISTANCE_WINDOW_TICKS_PARAM] {
            let (mut app, _uuid) = run_to_recovery_omitting(&[omitted]);

            // The MACHINE still enters the authored recovery state: the guard
            // that takes it there reads shields, not these scalars. What must
            // not happen is the HOST flying an orbit it can never fly out of.
            assert_eq!(
                steering_state(&mut app),
                "recover",
                "omitting `{omitted}` must not change which state is entered"
            );
            let pass = pass_surface(&mut app);
            assert!(
                !pass.recover,
                "omitting `{omitted}` must decline the recovery arm outright; \
                 publishing `recover` without it orbits for ever, because \
                 `safe_distance_held` can never be satisfied"
            );
            assert!(
                !pass.reengage,
                "the whole arm declines together, not half of it"
            );

            // And it stays declined. The shipped hull is holding its ring
            // through this dwell, so a run that keeps ticking must not quietly
            // start orbiting a few ticks later.
            hold_and_tick(&mut app, 3.0, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
            let pass = pass_surface(&mut app);
            assert!(
                !pass.recover && !pass.reengage,
                "omitting `{omitted}` must keep declining the arm, not orbit"
            );
        }
    }

    /// AC1's consequence at the doctrine level, and the anti-trap for an
    /// unseeded fact: `fact(shield_fraction)` is genuinely read, so the escape
    /// hands off to recovery ONLY when the pass actually cost the destroyer its
    /// shields.
    ///
    /// The negative control is the load-bearing half. A guard on a fact nobody
    /// seeds parses fine and reads false for ever — which here would look like a
    /// destroyer that simply never recovers, with nothing failing. The two runs
    /// below differ in exactly one thing: the shield fraction.
    #[test]
    fn the_escape_hands_off_to_recovery_only_when_the_shields_collapsed() {
        let (mut app, _uuid) = run_to_recovery();
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "shields at zero when the dwell expired: the destroyer must break off"
        );
        assert_eq!(
            engines_state(&mut app),
            "recover",
            "Engines runs its own copy of the machine and must reach the same leg \
             from the same facts, not by reading Steering's state"
        );

        // Negative control: identical run, healthy shields.
        let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
        set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
        tick(&mut app);
        place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(steering_state(&mut app), "escape");
        hold_and_tick(&mut app, 7.2, (0.0, -260.0, 0.0, 20.0), 1.0);
        // It re-acquires and — parked 60 units from the bogey, well inside the
        // authored `commit_range` — commits to the next run immediately. Which
        // of the two pass states it lands in is a detail; that it is back on the
        // pass cycle rather than orbiting is the point.
        assert!(
            ["acquire", "inbound"].contains(&steering_state(&mut app).as_str()),
            "with its shields intact the destroyer turns straight back in — if this \
             reads `recover` the shield guard is not reading the ship's shields; got {}",
            steering_state(&mut app)
        );
    }

    /// AC2: the safe ring is the TARGET's own longest usable direct-fire range
    /// plus this hull's authored margin — not an authored distance, and not a
    /// property of the destroyer at all.
    ///
    /// Asserted by changing only the bogey's reach and watching the published
    /// ring move with it, which no constant could do.
    #[test]
    fn the_safe_ring_derives_from_the_targets_direct_fire_reach_plus_the_margin() {
        let margin = authored_steering_param(SAFE_RANGE_MARGIN_PARAM);
        let (mut app, uuid) = run_to_recovery();

        let pass = pass_surface(&mut app);
        assert!(pass.recover, "the published leg must be the recovery orbit");
        assert!(
            (pass.safe_range - (BOGEY_DIRECT_FIRE_REACH + margin)).abs() < 1e-3,
            "the ring must be the target's reach ({BOGEY_DIRECT_FIRE_REACH}) plus the \
             authored margin ({margin}), got {}",
            pass.safe_range
        );

        // A longer-ranged opponent pushes the ring out by exactly the change in
        // ITS reach. Nothing about the destroyer changed.
        set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 400.0);
        hold_and_tick(&mut app, 0.2, (0.0, -260.0, 0.0, 20.0), 0.0);
        assert!(
            (pass_surface(&mut app).safe_range - (400.0 + margin)).abs() < 1e-3,
            "the ring must follow the target's reach, got {}",
            pass_surface(&mut app).safe_range
        );

        // ...and an unarmed target collapses it to the margin alone, rather than
        // to an invented distance.
        set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 0.0);
        hold_and_tick(&mut app, 0.2, (0.0, -260.0, 0.0, 20.0), 0.0);
        assert!((pass_surface(&mut app).safe_range - margin).abs() < 1e-3);
    }

    /// AC3 at the host: the recovery leg is flown at the authored ORBIT
    /// throttle, under power and turning — not stopped, and not simply pointed
    /// away.
    ///
    /// The throttle assertion is what separates the orbit from the two
    /// alternatives the issue rules out: a station-keeper would be braking
    /// toward zero at the ring, and a retreat would be flying the escape
    /// throttle straight down the outward bearing with no turn at all.
    #[test]
    fn the_recovery_leg_is_flown_as_a_powered_turning_orbit() {
        let (mut app, _uuid) = run_to_recovery();
        let orbit_speed = authored_steering_param(ORBIT_SPEED_PARAM);
        let pass = pass_surface(&mut app);
        assert!(pass.recover);

        assert!(
            (get_thrust_input(&mut app) - orbit_speed).abs() < 1e-3,
            "the ring is flown at the authored orbit throttle ({orbit_speed}), got {}",
            get_thrust_input(&mut app)
        );
        assert!(
            get_steering_input(&mut app).abs() > 0.0,
            "an orbit turns; a retreat does not"
        );
        assert!(
            pass.orbit_direction == 1.0 || pass.orbit_direction == -1.0,
            "the circulation direction must be a definite choice, got {}",
            pass.orbit_direction
        );
    }

    /// AC4: the circulation direction is drawn from a
    /// (world, ship, system, transition, occurrence) key, so it reproduces
    /// exactly for a given seed — and is not simply a constant.
    #[test]
    fn the_orbit_direction_is_deterministic_from_the_seed_without_being_constant() {
        fn direction_for(seed: u64, ship: uuid::Uuid) -> f32 {
            let (mut app, bogey) = fly_through_app([0.0, 0.0, -200.0]);
            app.insert_resource(crate::sim_rng::SimRng::new(
                seed,
                crate::sim_rng::SeedSource::Cli,
            ));
            let entity = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(entity)
                .insert(crate::entity_spawner::EntityUuid(ship.to_string()));
            set_armed_bogey(&mut app, bogey, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
            place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
            tick_twice(&mut app);
            place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
            tick(&mut app);
            place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
            tick(&mut app);
            hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
            assert_eq!(steering_state(&mut app), "recover");
            pass_surface(&mut app).orbit_direction
        }

        let ship = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        // Reproducible: same world seed, same ship, same answer. This is the
        // property a replayed `--seed` run depends on.
        assert_eq!(direction_for(4242, ship), direction_for(4242, ship));

        // Not a constant: over a handful of seeds both directions occur. A
        // hardcoded `+1` would pass every other assertion in this file.
        let directions: Vec<f32> = [1, 2, 3, 4, 5, 6, 7, 8]
            .into_iter()
            .map(|seed| direction_for(seed, ship))
            .collect();
        assert!(
            directions.contains(&1.0) && directions.contains(&-1.0),
            "the direction must genuinely vary with the seed, got {directions:?}"
        );
    }

    /// AC6: re-entry needs BOTH the authored shield fraction and a MAINTAINED
    /// safe distance, and takes neither alone.
    ///
    /// Three runs from the same starting point, differing only in which half is
    /// satisfied. The "distance" half is what the bounded history window buys:
    /// the ship is at range in the third run for long enough to fill it.
    #[test]
    fn re_entry_takes_neither_half_of_the_gate_alone() {
        let reentry_fraction = authored_steering_param("reentry_shield_fraction");
        let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;

        // (a) Shields fully restored, but parked well INSIDE the ring.
        let (mut app, _uuid) = run_to_recovery();
        hold_and_tick(&mut app, window_secs, (0.0, -260.0, 0.0, 20.0), 1.0);
        assert!(
            shield_fraction(&mut app) >= reentry_fraction,
            "precondition: the shields really are back"
        );
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "full shields inside the enemy's reach is not a recovery: the destroyer \
             must keep its ring"
        );

        // (b) Out beyond the ring for the whole window, but the shields are
        // still short of the authored fraction.
        let (mut app, _uuid) = run_to_recovery();
        hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 0.5);
        assert!(
            shield_fraction(&mut app) < reentry_fraction,
            "precondition: the shields are still short"
        );
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "a maintained standoff with half its shields is not a recovery either"
        );

        // (c) Both: out beyond the ring for the whole window AND shields back.
        let (mut app, _uuid) = run_to_recovery();
        hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 1.0);
        assert_eq!(
            steering_state(&mut app),
            "reenter",
            "both halves satisfied: the destroyer turns back in"
        );
        assert_eq!(
            engines_state(&mut app),
            "reenter",
            "and every axis agrees, from the same facts"
        );
    }

    /// AC5/AC6, the "maintained" in maintained safe distance: ONE tick at range
    /// does not open the gate. The window is authored and bounded, so the
    /// destroyer must actually hold the ring for its full span.
    #[test]
    fn one_tick_at_range_is_not_a_maintained_safe_distance() {
        let (mut app, _uuid) = run_to_recovery();
        // Shields back, and a single tick out beyond the ring.
        hold_and_tick(&mut app, 0.05, (0.0, -700.0, 0.0, 20.0), 1.0);
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "a couple of ticks at range is not a held distance — if this re-enters, \
             the history window is not being consulted"
        );
    }

    /// AC5: the history is BOUNDED. Ticking for many multiples of the authored
    /// window must never let it grow past its capacity — the property that keeps
    /// a scenario running for an hour from accumulating a per-ship buffer of
    /// every range it ever saw.
    #[test]
    fn the_distance_history_never_grows_past_its_authored_bound() {
        let (mut app, _uuid) = run_to_recovery();
        let capacity = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) as usize;
        hold_and_tick(&mut app, 12.0, (0.0, -700.0, 0.0, 20.0), 0.0);
        let history = app
            .world_mut()
            .query::<&HelmRecoveryHistory>()
            .single(app.world())
            .expect("ship carries HelmRecoveryHistory")
            .clone();
        assert_eq!(
            history.ranges.capacity(),
            capacity,
            "the window's capacity is the authored value"
        );
        assert!(
            history.ranges.len() <= capacity,
            "the window must stay bounded: {} samples in a window of {capacity}",
            history.ranges.len()
        );
        assert!(history.ranges.is_full());
    }

    /// AC1/AC8, interrupted regeneration seen from the doctrine: shields that
    /// are knocked back down mid-recovery keep the destroyer on its ring. The
    /// gate is a level, not an edge, so a ship that briefly touched the
    /// threshold and was then hit again does not get to re-enter.
    #[test]
    fn regeneration_interrupted_mid_recovery_keeps_the_destroyer_on_its_ring() {
        let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;
        let (mut app, _uuid) = run_to_recovery();

        // Out at range with the shields climbing, but knocked back to zero
        // before they ever reach the authored fraction.
        hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 0.6);
        assert_eq!(steering_state(&mut app), "recover");
        hold_and_tick(&mut app, 0.5, (0.0, -700.0, 0.0, 20.0), 0.0);
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "a shield ramp that was interrupted has not recovered"
        );

        // Let it actually finish this time.
        hold_and_tick(&mut app, 0.5, (0.0, -700.0, 0.0, 20.0), 1.0);
        assert_eq!(
            steering_state(&mut app),
            "reenter",
            "and once it does finish, with the distance still held, re-entry follows"
        );
    }

    /// AC7: normal re-entry cuts thrust, pivots onto the target WITHOUT boost,
    /// and then begins another normal-speed pass.
    #[test]
    fn re_entry_cuts_thrust_pivots_cold_and_starts_another_normal_speed_pass() {
        let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;
        let (mut app, uuid) = run_to_recovery();
        hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 1.0);
        assert_eq!(steering_state(&mut app), "reenter");

        // Put the bogey hard off the beam so a real pivot is demanded and a
        // "hold the last steering command" fallback would be visible as zero.
        set_armed_bogey(
            &mut app,
            uuid,
            [500.0, 0.0, -700.0],
            BOGEY_DIRECT_FIRE_REACH,
        );
        place_ship(&mut app, 0.0, -700.0, 0.0, 20.0);
        set_shield_fraction(&mut app, 1.0);
        tick(&mut app);
        tick(&mut app);

        assert!(
            pass_surface(&mut app).reengage,
            "the published leg must be the re-entry pivot"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            0.0,
            "the pivot cuts thrust — that is what the authored reengage_speed of 0 means"
        );
        assert!(
            get_steering_input(&mut app) > 0.0,
            "and it turns onto the target off the starboard beam, got {}",
            get_steering_input(&mut app)
        );
        assert!(
            !boost_is_active(&mut app),
            "the pivot is flown COLD: no recovery state authors a boost rule"
        );

        // ...and the pivot's authored dwell hands off to an ordinary
        // normal-speed pass, not to another escape.
        let pivot_secs = authored_steering_param("reenter_pivot_secs");
        for _ in 0..((pivot_secs * 30.0).ceil() as usize + 3) {
            place_ship(&mut app, 0.0, -700.0, 0.0, 20.0);
            set_shield_fraction(&mut app, 1.0);
            tick(&mut app);
        }
        assert_eq!(
            steering_state(&mut app),
            "acquire",
            "the pivot ends in the ordinary approach state, so the next run is a \
             normal-speed pass"
        );
        assert!(
            !boost_is_active(&mut app),
            "an approach never boosts: acquire authors no boost rule"
        );
    }

    // ── The pressed short-pass fallback (issue #789) ─────────────────────────
    //
    // Same posture again: the SHIPPED hull's authored policies driven through a
    // real ticking app, asserting only on the committed policy state, the
    // published pass surface, the admitted actuator inputs, and the ship's boost
    // state.
    //
    // The whole section turns on ONE distinction the fixtures have to keep
    // honest, so it is worth stating plainly. A destroyer whose escape WORKED
    // recovers; one whose escape FAILED presses. "Failed" is two things at once
    // — it gained no ground AND it is still inside the guns — so every negative
    // control below changes exactly one of those and holds the other, plus the
    // shields, plus the dwell, constant.

    /// Where the destroyer sits out the escape dwell to read as PRESSED: 60
    /// units astern of the bogey, i.e. well INSIDE its
    /// [`BOGEY_DIRECT_FIRE_REACH`] of 90, and stationary — an escape that ran
    /// its full authored dwell and finished no further from the guns than it
    /// started.
    ///
    /// The mirror of [`RECOVERY_DWELL_Z`], and the two differ in one thing only.
    const PRESSED_DWELL_Z: f32 = -260.0;

    /// A bogey that outranges the whole test arena.
    ///
    /// Exists so a control can open real distance and STILL be inside the
    /// target's reach when the dwell ends. Against the ordinary 90-unit bogey
    /// those two are physically inseparable — gaining ground from inside 90
    /// units takes you outside them — so isolating the progress conjunct on its
    /// own needs guns long enough that leaving them is not the same event as
    /// getting away.
    const LONG_REACH: f32 = 400.0;

    /// As [`run_to_pressed`], but flying a hull whose STEERING policy is missing
    /// the named `param`s.
    fn run_to_pressed_omitting(omit: &[&str]) -> (App, uuid::Uuid) {
        let (mut app, uuid) = run_to_armed_escape(omit, BOGEY_DIRECT_FIRE_REACH);
        hold_and_tick(&mut app, 7.2, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
        (app, uuid)
    }

    /// Fly a complete pass against an armed bogey and sit out the escape dwell
    /// pinned inside its reach with the shields gone, leaving the machine in
    /// `pressed_pivot`.
    fn run_to_pressed() -> (App, uuid::Uuid) {
        run_to_pressed_omitting(&[])
    }

    /// Sit out `secs` of shared AI ticks while the ship OPENS the range from a
    /// bogey at the origin end by `per_tick` world units every tick, holding its
    /// shields at `shields`.
    ///
    /// The counterpart of [`hold_and_tick`]: that one pins a pose so a test is
    /// not measuring drift, this one moves it deliberately so a test can measure
    /// a TREND. Both pin the shields for the same reason.
    fn withdraw_and_tick(app: &mut App, secs: f32, start_z: f32, per_tick: f32, shields: f32) {
        let ticks = (secs * 30.0).ceil() as usize + 3;
        for i in 0..ticks {
            place_ship(app, 0.0, start_z - per_tick * i as f32, 0.0, 20.0);
            set_shield_fraction(app, shields);
            tick(app);
        }
    }

    /// This ship's planar distance from `pos`, so a control can assert its own
    /// geometric precondition instead of asserting it in a comment.
    fn range_from(app: &mut App, pos: [f32; 3]) -> f32 {
        let physics = *app
            .world_mut()
            .query_filtered::<&ShipPhysics, With<Ship>>()
            .single(app.world())
            .expect("ship carries ShipPhysics");
        let (dx, dz) = (physics.x - pos[0], physics.z - pos[2]);
        (dx * dx + dz * dz).sqrt()
    }

    /// AC1, and the anti-trap for the two new facts: pressed detection is a
    /// comparison of authored minimum separation PROGRESS across an authored
    /// history window, taken only while inside the target's own effective threat
    /// range.
    ///
    /// Both conjuncts get their own matched control, because either one alone
    /// would be a fact nobody seeded reading false for ever — which would look
    /// exactly like a destroyer that simply never presses, with nothing failing.
    ///
    /// (a) and (b) differ in ONE thing: whether the destroyer moved. Same bogey,
    /// same reach, same shield collapse, same dwell, and (b) ends its dwell
    /// STILL inside those guns — asserted, not assumed — so the threat-range
    /// conjunct is identical in both and only the progress reading differs.
    /// (c) then holds the progress at zero and moves the threat line instead.
    #[test]
    fn only_an_escape_that_gains_no_ground_under_the_guns_presses_the_destroyer() {
        // (a) Pinned inside the reach for the whole dwell: the escape failed.
        let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);
        hold_and_tick(&mut app, 7.2, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
        assert_eq!(
            steering_state(&mut app),
            "pressed_pivot",
            "an escape that ran its full dwell and gained nothing, inside the target's \
             own reach, must abandon recovery"
        );
        assert_eq!(
            engines_state(&mut app),
            "pressed_pivot",
            "Engines runs its own copy of the machine and must reach the same leg from \
             the same facts, not by reading Steering's state"
        );

        // (b) The one-variable control: the same run, opening ground steadily
        // across the window the detector measures, and still under those guns
        // when the dwell expires.
        //
        // The per-tick step is DERIVED from the two authored scalars it has to
        // beat rather than hand-picked against today's values: enough ground per
        // tick that a full `pressed_window_ticks` window nets twice
        // `pressed_min_progress`. Retuning either param moves this with it
        // instead of quietly turning the control into a second copy of (a).
        let per_tick = 2.0 * authored_steering_param(PRESSED_MIN_PROGRESS_PARAM)
            / authored_steering_param(PRESSED_WINDOW_TICKS_PARAM);
        let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);
        withdraw_and_tick(&mut app, 7.2, -205.0, per_tick, 0.0);
        assert!(
            range_from(&mut app, [0.0, 0.0, -200.0]) < LONG_REACH,
            "precondition: this control must still be INSIDE the target's reach, or it \
             is testing the threat conjunct instead of the progress one — got {}",
            range_from(&mut app, [0.0, 0.0, -200.0])
        );
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "an escape that kept opening real distance WORKED: the destroyer takes the \
             ordinary recovery doctrine even though it is still in range"
        );

        // (c) The other conjunct, alone: standing still again, but out beyond a
        // reach that can no longer touch it. Distance that does not matter is
        // not distance worth measuring.
        let (mut app, _uuid) = run_to_recovery();
        assert!(
            range_from(&mut app, [0.0, 0.0, -200.0]) > BOGEY_DIRECT_FIRE_REACH,
            "precondition: this control must be OUTSIDE the target's reach"
        );
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "a destroyer sitting still beyond the enemy's guns is not pinned — if this \
             reads `pressed_pivot`, the threat-range conjunct is not being read"
        );
    }

    /// AC2: taking a hit — even one that collapses the shields outright — does
    /// not by itself press the destroyer.
    ///
    /// Structurally it cannot: there is no hit or damage EVENT fact anywhere in
    /// this codebase, only `shield_fraction` as a level, so a detector built on
    /// separation alone has nothing to fire on. This is the explicit negative
    /// control for that, in the shape the recovery hand-off test uses — one run,
    /// one fact moved, and the damage arriving as a discrete event partway
    /// through an escape that is otherwise going perfectly well.
    ///
    /// The state is sampled on EVERY tick rather than only at the end, because
    /// "never pressed" is the claim; a run that dipped into the pressed loop and
    /// came back out would satisfy an end-state assertion and still be wrong.
    #[test]
    fn a_shield_collapse_alone_never_presses_the_destroyer() {
        let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);

        // The first half of the escape goes perfectly: shields up, ground being
        // opened steadily.
        let mut visited: Vec<String> = Vec::new();
        let mut z = -205.0_f32;
        for tick_index in 0..222 {
            // THE HIT, once, at a single instant: full shields to none.
            let shields = if tick_index < 90 { 1.0 } else { 0.0 };
            place_ship(&mut app, 0.0, z, 0.0, 20.0);
            set_shield_fraction(&mut app, shields);
            tick(&mut app);
            z -= 1.5;
            visited.push(steering_state(&mut app));
        }

        assert!(
            !visited.iter().any(|s| s.starts_with("pressed")),
            "a destroyer that is opening ground the whole time must never press, \
             whatever its shields do; it visited {:?}",
            visited.iter().collect::<std::collections::BTreeSet<_>>()
        );
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "the collapse still costs it the pass — it breaks off to the ordinary \
             standoff — but that is the shield gate doing its job, not the pressed one"
        );
    }

    /// AC4: the pressed pivot is a STATIONARY turn flown with the drive lit, and
    /// the drive is cancelled before the normal-speed pass that follows.
    ///
    /// The cancel is the load-bearing half and it is authored as an absence —
    /// `pressed_pass` declares no boost rule at all, so the channel holds and
    /// `ai_helm_boost`'s on-change release fires. An absence is exactly the kind
    /// of content that gets helpfully filled in, so it is asserted here through
    /// the ship's real boost state as well as pinned in the hull's parse tests.
    #[test]
    fn the_pressed_pivot_boosts_a_stationary_turn_and_the_short_pass_does_not() {
        let (mut app, _uuid) = run_to_pressed();
        assert_eq!(steering_state(&mut app), "pressed_pivot");

        let pass = pass_surface(&mut app);
        assert!(
            pass.reengage,
            "the pivot is published as the re-engage leg, so the host pairs it with the \
             authored reengage_speed"
        );
        assert!(!pass.recover, "and emphatically not as a standoff orbit");
        assert_eq!(
            get_thrust_input(&mut app),
            authored_steering_param(REENGAGE_SPEED_PARAM),
            "a STATIONARY pivot: the throttle is the authored re-engage fraction"
        );
        assert!(
            boost_is_active(&mut app),
            "the pivot is flown with the drive lit — that is what buys the extra yaw \
             rate, and it is the one place outside the escape that boosts"
        );

        // The authored pivot dwell expires into the short pass...
        let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
        hold_and_tick(
            &mut app,
            pivot_secs + 0.2,
            (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
            0.0,
        );
        assert_eq!(
            steering_state(&mut app),
            "pressed_pass",
            "the pivot ends on its own authored dwell"
        );
        assert!(
            !boost_is_active(&mut app),
            "...and the drive is CANCELLED for it: the short pass is a normal-speed \
             pass, and it is recharging for the escape at the end of it"
        );
    }

    /// AC5/AC7: the short pass runs in at NORMAL speed, tracks the target, and
    /// breaks off into a straight boost-out escape on its own shorter authored
    /// hysteresis.
    ///
    /// The break-off is the half that makes the pass *short*, and it is asserted
    /// against a matched control rather than against a number: the same
    /// re-opening — chosen to sit between the two authored hysteresis values —
    /// commits the pressed pass and does NOT commit an ordinary inbound leg.
    #[test]
    fn the_short_pass_runs_in_at_normal_speed_and_breaks_off_sooner() {
        let pressed_hysteresis = authored_steering_param(PRESSED_HYSTERESIS_PARAM);
        let ordinary_hysteresis = authored_steering_param("closest_approach_hysteresis");
        // Between the two, so it can only be read one way.
        let reopen_by = (pressed_hysteresis + ordinary_hysteresis) / 2.0;
        assert!(
            pressed_hysteresis < reopen_by && reopen_by < ordinary_hysteresis,
            "the hull must author a SHORTER pressed hysteresis for this pair to mean \
             anything: {pressed_hysteresis} vs {ordinary_hysteresis}"
        );

        let (mut app, uuid) = run_to_pressed();
        let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
        hold_and_tick(
            &mut app,
            pivot_secs + 0.2,
            (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
            0.0,
        );
        assert_eq!(steering_state(&mut app), "pressed_pass");

        // ── The motion ──────────────────────────────────────────────────────
        // Abeam the bogey, so a real turn onto it is demanded and a "hold the
        // last steering command" fallback would show up as zero. The range is
        // unchanged, so nothing here can trip the break-off below.
        let pass = pass_surface(&mut app);
        place_ship(&mut app, 60.0, -200.0, 0.0, 20.0);
        tick(&mut app);
        tick(&mut app);
        assert!(
            pass.active && !pass.escape && !pass.recover && !pass.reengage,
            "the short pass is published as an ordinary inbound leg — that is what \
             makes it a normal-speed attack pass and not a fifth kind of manoeuvre"
        );
        assert!(
            (get_thrust_input(&mut app) - pass.approach_speed).abs() < 1e-3,
            "the short pass runs in at the authored APPROACH throttle ({}), not the \
             escape throttle, got {}",
            pass.approach_speed,
            get_thrust_input(&mut app)
        );
        assert!(
            get_steering_input(&mut app) < 0.0,
            "and it tracks the target, which is off the port beam, got {}",
            get_steering_input(&mut app)
        );
        assert!(
            !boost_is_active(&mut app),
            "still cold: a pass is not an escape"
        );

        // ── The break-off ───────────────────────────────────────────────────
        // Back astern of the bogey and driving away from it, so the closing rate
        // is negative, then let the range re-open by `reopen_by`.
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "pressed_pass",
            "an opening rate alone is not a closest approach on either doctrine"
        );
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "escape",
            "{reopen_by} units of re-opening is past the SHORT pass's authored \
             hysteresis of {pressed_hysteresis}: the jab is over and the destroyer \
             commits to another boost-out"
        );
        assert!(
            pass_surface(&mut app).escape,
            "and the published leg is the escape, flown from a heading frozen at this \
             merge"
        );

        // ── The boost-out itself ────────────────────────────────────────────
        // "Commits to another boost-out" is a claim about the DRIVE, not only
        // about the state, so it is asserted through the ship's real boost
        // state — and asserted as the authored behaviour rather than as
        // "instantly". The escape's own rule carries
        // `fact(speed_fraction) >= param(boost_min_speed_fraction)`, so a jab
        // that broke off before the hull had rebuilt speed relights LATE, not
        // never. Both sides of that authored line are checked, because only the
        // pair distinguishes "waiting for speed" from "never lights at all".
        let min_fraction = authored_boost_param("boost_min_speed_fraction");
        let min_speed = min_fraction * authored_max_speed();
        place_ship(
            &mut app,
            0.0,
            PRESSED_DWELL_Z - reopen_by,
            0.0,
            min_speed * 0.5,
        );
        tick(&mut app);
        assert!(
            !boost_is_active(&mut app),
            "under the authored {min_fraction} speed fraction the escape holds the \
             drive: boost is an escape accelerant, not a launch assist"
        );
        place_ship(
            &mut app,
            0.0,
            PRESSED_DWELL_Z - reopen_by,
            0.0,
            min_speed + 1.0,
        );
        tick(&mut app);
        assert!(
            boost_is_active(&mut app),
            "once past the authored fraction the escape out of a pressed pass lights \
             the drive like any other escape — the jab ends in a real attempt to leave"
        );
        assert_eq!(
            steering_state(&mut app),
            "escape",
            "and nothing about the relight ends the escape leg"
        );

        // The matched control: the identical re-opening on an ORDINARY inbound
        // leg is short of ITS authored hysteresis and commits nothing.
        let (mut app, _uuid) = fly_through_app([0.0, 0.0, -200.0]);
        set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "inbound");
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "inbound",
            "the same {reopen_by} units is short of the ordinary {ordinary_hysteresis}-unit \
             hysteresis — if this commits too, the pressed pass is not actually shorter"
        );
    }

    /// AC3: while pressed, the destroyer waits for neither of the things the
    /// recovery doctrine waits for.
    ///
    /// Both of recovery's re-entry conjuncts are handed to it mid-loop —
    /// shields fully restored — and it neither switches to the standoff ring nor
    /// jumps to the re-entry pivot. It finishes its own authored pivot dwell and
    /// makes its pass, because being pinned is not a thing that gets better by
    /// waiting.
    #[test]
    fn pressed_behaviour_waits_on_neither_the_shield_threshold_nor_the_ring() {
        let (mut app, _uuid) = run_to_pressed();
        let reentry_fraction = authored_steering_param("reentry_shield_fraction");

        // Hand it the shield half of the recovery gate outright.
        let mut visited: Vec<String> = Vec::new();
        let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
        for _ in 0..((pivot_secs * 30.0).ceil() as usize + 3) {
            place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
            set_shield_fraction(&mut app, 1.0);
            tick(&mut app);
            visited.push(steering_state(&mut app));
            assert!(
                !pass_surface(&mut app).recover,
                "a pressed destroyer never publishes the standoff orbit"
            );
        }
        assert!(
            shield_fraction(&mut app) >= reentry_fraction,
            "precondition: the shields really are back past the re-entry threshold"
        );
        assert!(
            !visited.iter().any(|s| s == "recover" || s == "reenter"),
            "restoring the shields must not pull a pressed destroyer into the recovery \
             doctrine mid-loop; it visited {visited:?}"
        );
        assert_eq!(
            steering_state(&mut app),
            "pressed_pass",
            "it finishes the pivot it started and makes its pass"
        );
    }

    /// AC6: the pressed loop is a response, not a mode. The moment one of its
    /// escapes actually opens ground, the destroyer is back on the ordinary
    /// recovery doctrine.
    ///
    /// This is the round trip end to end — recovery abandoned, pivot, short
    /// pass, escape, recovery resumed — so it also pins that the pressed states
    /// hand back to the SAME `escape` state the ordinary pass uses rather than
    /// to a private copy of it.
    #[test]
    fn a_successful_escape_out_of_the_pressed_loop_resumes_the_recovery_doctrine() {
        let reopen_by = authored_steering_param(PRESSED_HYSTERESIS_PARAM) + 1.0;
        let (mut app, _uuid) = run_to_pressed();
        assert_eq!(steering_state(&mut app), "pressed_pivot");

        // Pivot → short pass → break-off into another escape attempt.
        let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
        hold_and_tick(
            &mut app,
            pivot_secs + 0.2,
            (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
            0.0,
        );
        assert_eq!(steering_state(&mut app), "pressed_pass");
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
        tick(&mut app);
        assert_eq!(steering_state(&mut app), "escape");

        // THIS escape works: it ends the dwell out beyond the target's reach,
        // with the shields still gone.
        hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "an escape that succeeded hands off to the ordinary standoff orbit, however \
             many failed ones came before it"
        );
        assert_eq!(
            engines_state(&mut app),
            "recover",
            "and every axis comes back together, from the same facts"
        );
        assert!(
            pass_surface(&mut app).recover,
            "the published leg is the orbit again"
        );
    }

    /// "Decline rather than invent", on all four pressed scalars.
    ///
    /// Each is genuinely load-bearing on its own and each fails differently if
    /// admitted alone: without `pressed_window_ticks` the progress window keeps
    /// its `Default` capacity of zero and can never report a trend; without
    /// `pressed_min_progress` there is no line to compare that trend against;
    /// without `pressed_pivot_secs` the pivot never ends; without
    /// `pressed_closest_approach_hysteresis` the short pass never breaks off. A
    /// hull admitted into the arm on three of the four would stall inside it —
    /// strictly worse than never entering — so the host gates on all four
    /// together and the hull flies its ordinary recovery doctrine instead.
    ///
    /// The shipped hull presses at this exact point (asserted above), so nothing
    /// here passes for want of getting that far.
    #[test]
    fn a_hull_omitting_any_pressed_scalar_declines_the_whole_pressed_arm() {
        for omitted in PRESSED_PARAMS {
            let (mut app, _uuid) = run_to_pressed_omitting(&[omitted]);
            assert_eq!(
                steering_state(&mut app),
                "recover",
                "omitting `{omitted}` must decline the pressed arm outright and leave the \
                 hull on the ordinary recovery doctrine"
            );
            assert_eq!(
                engines_state(&mut app),
                "recover",
                "omitting `{omitted}` declines it on EVERY axis — the host's gate is over \
                 the one shared fact snapshot, so the three machines cannot disagree"
            );

            // And it stays declined: a run that keeps ticking must not start
            // pressing a few ticks later.
            hold_and_tick(&mut app, 3.0, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
            assert!(
                !steering_state(&mut app).starts_with("pressed"),
                "omitting `{omitted}` must keep declining the arm"
            );
        }
    }

    /// ...and on all six RECOVERY scalars too, which is the same trap one level
    /// up.
    ///
    /// The pressed pivot is not a fifth kind of manoeuvre: it is flown as
    /// `FlyThroughLeg::Reengage`, and the planner only flies that leg when
    /// `HelmPassSurface::reengage` is published, which `build_pass_surface` only
    /// does when the whole recovery six are authored. A hull admitted into the
    /// pressed arm on the four pressed scalars alone would therefore enter
    /// `pressed_pivot` and have the planner fall through to the INBOUND leg —
    /// boosted, at full approach throttle, turning hard, straight at the ship
    /// that has it pinned. That is strictly worse than the doctrine travel such
    /// a hull flew before the pressed arm existed, so the arm declines outright.
    ///
    /// Nothing in content validation ties the `pivot_to_reengage` verb to those
    /// scalars, so the host's gate is the only thing that can catch it — and
    /// this is the test that holds the gate in place.
    #[test]
    fn a_hull_omitting_any_recovery_scalar_declines_the_pressed_arm_too() {
        for omitted in RECOVERY_PARAMS {
            let (mut app, _uuid) = run_to_pressed_omitting(&[omitted]);
            assert_eq!(
                steering_state(&mut app),
                "recover",
                "omitting `{omitted}` must decline the pressed arm outright — a hull that \
                 cannot publish the re-engage leg cannot fly the pressed pivot"
            );
            assert_eq!(
                engines_state(&mut app),
                "recover",
                "omitting `{omitted}` declines it on EVERY axis, from the one shared fact \
                 snapshot"
            );

            // And it stays declined, for the same reason the pressed-scalar
            // case does: a run that keeps ticking must not start pressing a few
            // ticks later.
            hold_and_tick(&mut app, 3.0, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
            assert!(
                !steering_state(&mut app).starts_with("pressed"),
                "omitting `{omitted}` must keep declining the arm"
            );
        }
    }

    // ── The Harrow Cruiser broadside orbit (issue #790) ──────────────────────
    //
    // Same posture as the destroyer block above: these drive the SHIPPED hull's
    // authored policies through a real ticking app, so they fail on the content
    // as well as on the code, and every assertion is about something observable
    // — an admitted actuator input, the published pass surface, the committed
    // policy state, or the ship's own flown range.
    //
    // Unlike the destroyer's tests, the orbit ones deliberately let the ship
    // FLY rather than pinning its pose each tick. A spiral is a claim about how
    // the range changes over time, and pinning the pose would make that claim
    // untestable.

    fn cruiser_hull() -> crate::entity_config::EntityConfig {
        crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_cruiser.toml"
        ))
        .expect("the shipped cruiser hull must parse")
    }

    /// The cruiser's authored Steering `param`s, so expectations below are
    /// arithmetic on named values rather than magic numbers.
    fn cruiser_steering_param(name: &str) -> f32 {
        cruiser_hull()
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .and_then(|ai| ai.param.get(name).copied())
            .unwrap_or_else(|| panic!("the shipped cruiser must author `{name}`"))
    }

    /// A ship carrying the shipped cruiser's two authored policy machines and
    /// its physics envelope — the same components `entities::spawner` would
    /// attach — hunting a single named bogey.
    ///
    /// The cruiser authors no boost drive and no boost doctrine, so nothing
    /// boost-shaped is inserted here either: the fixture mirrors the hull.
    fn broadside_app(bogey_pos: [f32; 3]) -> (App, uuid::Uuid) {
        broadside_app_omitting(bogey_pos, &[])
    }

    /// As [`broadside_app`], but with the named STEERING `param`s stripped from
    /// the hull before its policy is built — the partially-authored hull
    /// AGENTS.md #11 says must decline rather than invent.
    ///
    /// Each name must actually be present to begin with, so this cannot quietly
    /// pass by "removing" a param the hull renamed out from under it.
    fn broadside_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
        let mut app = test_app();
        let cfg = cruiser_hull();
        let mut hc = cfg
            .helm_console
            .clone()
            .expect("hull declares [helm_console]");
        for name in omit {
            hc.steering_ai
                .as_mut()
                .expect("hull declares [helm_console.steering_ai]")
                .param
                .remove(*name)
                .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
        }
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            HelmEnginesAiPolicy(hc.engines_ai.as_ref().unwrap().to_policy().unwrap()),
            HelmSteeringAiPolicy(hc.steering_ai.as_ref().unwrap().to_policy().unwrap()),
            crate::ship_plugin::ShipPhysicsConfigResource(crate::ship_physics::ShipPhysicsConfig {
                max_speed: hc.max_speed,
                max_reverse_speed: hc.max_reverse_speed,
                acceleration: hc.acceleration,
                deceleration: hc.deceleration,
                max_yaw_rate: hc.max_yaw_rate,
                ..crate::ship_physics::ShipPhysicsConfig::new()
            }),
        ));
        set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective(BOGEY, 80.0)]);
        set_helm_control_source(&mut app, ControlSource::Ai);
        let uuid = uuid::Uuid::new_v4();
        set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
        (app, uuid)
    }

    /// The bogey sits at the origin for every orbit fixture, so a "range to the
    /// target" reading is just the ship's own distance from the origin.
    const ORBIT_BOGEY: [f32; 3] = [0.0, 0.0, 0.0];

    fn ship_pose(app: &mut App) -> ShipPhysics {
        *app.world_mut()
            .query_filtered::<&ShipPhysics, With<Ship>>()
            .single(app.world())
            .expect("ship carries ShipPhysics")
    }

    /// Planar distance from the ship to the bogey at [`ORBIT_BOGEY`].
    fn range_to_bogey(app: &mut App) -> f32 {
        let p = ship_pose(app);
        (p.x * p.x + p.z * p.z).sqrt()
    }

    /// Shortest signed angular step from `previous` to `now`, radians. Summing
    /// these is what turns a wrapping bearing into a swept angle.
    fn wrapped_delta(now: f32, previous: f32) -> f32 {
        let mut d = now - previous;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        d
    }

    /// The ship's bearing around the bogey, radians. Its RATE of change is the
    /// circulation: a positive drift is one way round the ring, a negative one
    /// the other, and that is what "clockwise or anticlockwise" means
    /// observably.
    fn bearing_around_bogey(app: &mut App) -> f32 {
        let p = ship_pose(app);
        p.x.atan2(p.z)
    }

    /// Put the cruiser into its orbit state, flying, at `start_range` from the
    /// bogey.
    ///
    /// The ship starts abeam of its own heading (placed down `-Z`, facing `+X`)
    /// so it begins with a tangential component rather than head-on, which is
    /// the pose the approach leg would have delivered it in anyway.
    fn run_to_orbit_omitting(start_range: f32, omit: &[&str]) -> (App, uuid::Uuid) {
        let (mut app, uuid) = broadside_app_omitting(ORBIT_BOGEY, omit);
        let speed = cruiser_hull().helm_console.as_ref().unwrap().max_speed
            * cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM);
        place_ship(
            &mut app,
            0.0,
            -start_range,
            std::f32::consts::FRAC_PI_2,
            speed,
        );
        // Two ticks: the first publishes the pass surface, the second is the
        // first planner pass that consumes it (see `HelmPassSurface`).
        tick_twice(&mut app);
        (app, uuid)
    }

    fn run_to_orbit(start_range: f32) -> (App, uuid::Uuid) {
        run_to_orbit_omitting(start_range, &[])
    }

    /// Let the ship actually fly for roughly `secs` of SIMULATED flight,
    /// touching nothing.
    ///
    /// The physics step is capped at [`HELM_AI_MAX_DT_SECS`] (1/30 s) regardless
    /// of how long the fixture's `Time` says a frame took, so flown seconds are
    /// counted in physics steps rather than in the fixture's 200 ms frames.
    /// Getting this wrong makes a convergence test read as a failure to converge.
    fn fly_for(app: &mut App, secs: f32) {
        for _ in 0..((secs / HELM_AI_MAX_DT_SECS).ceil() as usize) {
            tick(app);
        }
    }

    /// Flown seconds needed for the orbit to reach its steady radius from any of
    /// the fixtures below — measured, not guessed: the worst case starts inside
    /// the ring facing the wrong way round and takes about 28 s to settle.
    const ORBIT_SETTLE_SECS: f32 = 40.0;

    /// Sum the ship's swept bearing around the bogey over `secs` of flight. The
    /// SIGN is the circulation and the magnitude is how far round it got.
    fn swept_bearing(app: &mut App, secs: f32) -> f32 {
        let mut previous = bearing_around_bogey(app);
        let mut swept = 0.0_f32;
        for _ in 0..((secs / HELM_AI_MAX_DT_SECS).ceil() as usize) {
            tick(app);
            let now = bearing_around_bogey(app);
            swept += wrapped_delta(now, previous);
            previous = now;
        }
        swept
    }

    /// AC1's precondition and AC2's first half: the cruiser commits to the
    /// orbit and the host publishes the combat-orbit leg with the hull's OWN
    /// authored ring, throttle and gain.
    ///
    /// The `engage_range` half is the anti-trap for an unseeded fact: before the
    /// travel axes were seeded, a `fact(range_to_target)` guard validated at
    /// load and read false for ever. That this test can distinguish "far" from
    /// "near" is the proof the guard actually gates.
    #[test]
    fn the_cruiser_commits_to_the_orbit_inside_its_authored_engage_range() {
        let engage = cruiser_steering_param("engage_range");

        // Well outside the authored engage range: still closing, not circling.
        let (mut app, _uuid) = broadside_app([0.0, 0.0, -(engage * 3.0)]);
        place_ship(&mut app, 0.0, engage * 3.0, 0.0, 0.0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "acquire",
            "a target beyond engage_range must not start an orbit"
        );
        assert!(
            !pass_surface(&mut app).combat_orbit,
            "and the host must not publish the orbit leg for a ship still closing"
        );

        // Inside it.
        let (mut app, _uuid) = run_to_orbit(engage * 0.5);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "inside engage_range the machine must commit to the ring — if this reads \
             `acquire` the travel axis is seeing empty facts"
        );
        assert_eq!(
            engines_state(&mut app),
            "orbit",
            "Engines runs its OWN copy of the machine and must reach the same leg from \
             the same facts, not by reading Steering's state"
        );

        let pass = pass_surface(&mut app);
        assert!(pass.active, "the cruiser must be flying an authored leg");
        assert!(pass.combat_orbit, "and that leg is the combat orbit");
        assert!(
            !pass.recover && !pass.reengage && !pass.escape,
            "the combat orbit is its own leg — it must not masquerade as the \
             shield-recovery standoff, whose ring is derived from the TARGET's reach"
        );
        assert_eq!(
            pass.combat_orbit_range,
            cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM),
            "the ring is the hull's OWN authored fighting radius"
        );
        assert_eq!(
            pass.combat_orbit_speed,
            cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM)
        );
        assert_eq!(
            pass.combat_orbit_spiral_gain,
            cruiser_steering_param(COMBAT_ORBIT_SPIRAL_GAIN_PARAM)
        );
        assert_eq!(
            pass.safe_range, 0.0,
            "the shield-recovery ring is untouched: this hull authors no recovery \
             doctrine, so `safe_range` must stay at its default rather than being \
             quietly repurposed"
        );
    }

    /// AC2, the continuous half: the ring is flown UNDER POWER and TURNING, at
    /// the authored orbit throttle.
    ///
    /// The throttle assertion is what separates an orbit from the two things it
    /// is not: a station-keeper would be braking toward zero at the ring, and a
    /// retreat would be running the outward bearing with no turn at all.
    #[test]
    fn the_orbit_is_flown_as_a_powered_continuous_turn() {
        let orbit_speed = cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM);
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
        let (mut app, _uuid) = run_to_orbit(ring);
        tick(&mut app);

        assert!(
            (get_thrust_input(&mut app) - orbit_speed).abs() < 1e-3,
            "the ring is flown at the authored orbit throttle ({orbit_speed}), got {}",
            get_thrust_input(&mut app)
        );
        let pass = pass_surface(&mut app);
        assert!(
            pass.orbit_direction == 1.0 || pass.orbit_direction == -1.0,
            "the circulation direction must be a definite choice, got {}",
            pass.orbit_direction
        );

        // Continuous TANGENTIAL movement: the bearing around the target keeps
        // advancing, always the same way. This is the observable form of
        // "continuous tangential movement" — a ship that stopped, or that flew
        // straight at or away from the target, would sweep no bearing at all.
        //
        // Measured after the orbit has settled, because the fixture drops the
        // cruiser onto the ring already flying, and the direction it is dealt
        // may be the opposite of the one it happens to be pointing: the first
        // half-turn is the ship getting onto its chosen circulation, not the
        // circulation itself.
        fly_for(&mut app, ORBIT_SETTLE_SECS);
        assert_eq!(steering_state(&mut app), "orbit");
        let direction = pass_surface(&mut app).orbit_direction;

        let range_before = range_to_bogey(&mut app);
        let swept = swept_bearing(&mut app, 8.0);
        assert!(
            swept * direction > 0.0,
            "the cruiser must circle the way it was dealt (direction {direction}),              but it swept {swept} rad"
        );
        assert!(
            swept.abs() > 0.5,
            "eight seconds on the ring must sweep real bearing, got {swept} rad"
        );
        // ...and it swept that bearing while HOLDING the ring rather than by
        // running past the target: tangential, not radial.
        let range_after = range_to_bogey(&mut app);
        for (label, range) in [("before", range_before), ("after", range_after)] {
            assert!(
                (range - ring).abs() < ring * 0.25,
                "the settled orbit must stay near the authored ring ({ring}); {label}                  the sweep it was at {range}"
            );
        }
    }

    /// AC2, the spiral half: the cruiser MAINTAINS the authored range from
    /// either side of it.
    ///
    /// Two runs, identical but for which side of the ring the cruiser starts on.
    /// Both must converge toward the ring, which is what distinguishes a spiral
    /// correction from a bare tangent (which would hold whatever radius it
    /// started at for ever) and from a retreat or a charge.
    #[test]
    fn the_orbit_spirals_onto_the_authored_ring_from_inside_and_outside() {
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);

        for (label, start) in [("inside", ring * 0.4), ("outside", ring * 2.5)] {
            let (mut app, _uuid) = run_to_orbit(start);
            assert_eq!(steering_state(&mut app), "orbit");
            let before = range_to_bogey(&mut app);
            let error_before = (before - ring).abs();

            fly_for(&mut app, ORBIT_SETTLE_SECS);

            let after = range_to_bogey(&mut app);
            let error_after = (after - ring).abs();
            assert!(
                error_after < error_before,
                "starting {label} the ring ({before} vs {ring}), the spiral must close \
                 the radial error, but it went from {error_before} to {error_after}"
            );
            assert_eq!(
                steering_state(&mut app),
                "orbit",
                "and it corrects INSIDE the orbit state — the spiral is the leg, not a \
                 separate manoeuvre the hull has to enter"
            );
        }
    }

    /// AC1: the circulation direction is drawn from a
    /// (world, ship, system, transition, occurrence) key, so it reproduces
    /// exactly for a given seed — and is not simply a constant.
    ///
    /// The negative half is the load-bearing one: the hull DECLARES
    /// `orbit_direction = 1.0` in its authored memory, so a host that never drew
    /// would publish `+1.0` every time and pass every other assertion in this
    /// file.
    #[test]
    fn the_combat_orbit_direction_is_deterministic_from_the_seed_without_being_constant() {
        fn direction_for(seed: u64, ship: uuid::Uuid) -> f32 {
            let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
            let (mut app, _bogey) = broadside_app(ORBIT_BOGEY);
            app.insert_resource(crate::sim_rng::SimRng::new(
                seed,
                crate::sim_rng::SeedSource::Cli,
            ));
            let entity = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(entity)
                .insert(crate::entity_spawner::EntityUuid(ship.to_string()));
            place_ship(&mut app, 0.0, -ring, std::f32::consts::FRAC_PI_2, 9.0);
            tick_twice(&mut app);
            assert_eq!(steering_state(&mut app), "orbit");
            pass_surface(&mut app).orbit_direction
        }

        let ship = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        // Reproducible: same world seed, same ship, same answer. This is the
        // property a replayed `--seed` run depends on.
        assert_eq!(direction_for(4242, ship), direction_for(4242, ship));

        // Not a constant: over a handful of seeds both directions occur.
        let directions: Vec<f32> = [1, 2, 3, 4, 5, 6, 7, 8]
            .into_iter()
            .map(|seed| direction_for(seed, ship))
            .collect();
        assert!(
            directions.contains(&1.0) && directions.contains(&-1.0),
            "the direction must genuinely vary with the seed, got {directions:?}"
        );
    }

    /// AC3: a hazard detour BENDS the orbit and never exits it, and the same
    /// circulation resumes once the hazard is clear.
    ///
    /// Three things are asserted and each has its own failure mode:
    ///
    /// * the steering command genuinely changes while the obstacle is there —
    ///   without this the rest of the test would pass on a hazard the ship never
    ///   noticed;
    /// * the committed policy state and the drawn direction are untouched — a
    ///   transition guarded on urgency would exit the orbit, and RE-entering it
    ///   would re-draw the direction, so flying past debris would randomise
    ///   which way the cruiser circles;
    /// * the bearing keeps advancing the same way afterwards — the resume.
    #[test]
    fn a_hazard_detour_bends_the_orbit_without_changing_its_direction() {
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
        let (mut app, bogey) = run_to_orbit(ring);
        // Settle onto the ring first: a cruiser still hauling itself round onto
        // its chosen circulation is commanding saturated steering, and a
        // saturated command cannot be observed to bend.
        fly_for(&mut app, ORBIT_SETTLE_SECS);
        assert_eq!(steering_state(&mut app), "orbit");
        let direction = pass_surface(&mut app).orbit_direction;
        let clean_steering = get_steering_input(&mut app);
        assert!(
            clean_steering.abs() < 1.0,
            "precondition: the settled orbit must have steering authority to spare,              or a detour could not show up in the command at all (got {clean_steering})"
        );

        // Drop an obstacle right on the ship's projected path. Deliberately NOT
        // the target — the orbit's own centre is excluded from the avoidance
        // scan, because circling a thing you are also fleeing is incoherent.
        let pose = ship_pose(&mut app);
        let hazard = uuid::Uuid::from_u128(0x0b57ac1e);
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![
                crate::ai::AiWorldEntity {
                    uuid: bogey,
                    name: Some(BOGEY.into()),
                    position: ORBIT_BOGEY,
                    yaw: Some(0.0),
                    forward_speed: 0.0,
                    radius: 3.0,
                    size_rating: 3.0,
                    movable: true,
                    dangerous: true,
                    ..Default::default()
                },
                crate::ai::AiWorldEntity {
                    uuid: hazard,
                    name: Some("rock".into()),
                    position: [
                        pose.x + pose.yaw.sin() * 12.0,
                        0.0,
                        pose.z - pose.yaw.cos() * 12.0,
                    ],
                    yaw: None,
                    forward_speed: 0.0,
                    radius: 25.0,
                    size_rating: 25.0,
                    movable: false,
                    dangerous: false,
                    ..Default::default()
                },
            ],
        });
        tick(&mut app);
        tick(&mut app);

        assert!(
            (get_steering_input(&mut app) - clean_steering).abs() > 1e-3,
            "precondition: the obstacle must actually bend the steering solution \
             (clean {clean_steering}, detour {})",
            get_steering_input(&mut app)
        );
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "a detour must not exit the orbit — nothing in this doctrine is guarded on \
             a hazard reading"
        );
        assert_eq!(engines_state(&mut app), "orbit");
        let during = pass_surface(&mut app);
        assert!(during.combat_orbit, "the published leg is still the orbit");
        assert_eq!(
            during.orbit_direction, direction,
            "and the circulation direction survives the detour untouched"
        );

        // Clear the hazard: the cruiser resumes the SAME circulation.
        set_bogey(&mut app, bogey, ORBIT_BOGEY, 0.0, 0.0);
        tick(&mut app);
        tick(&mut app);
        let resumed = pass_surface(&mut app);
        assert!(resumed.combat_orbit);
        assert_eq!(
            resumed.orbit_direction, direction,
            "clearing the hazard must not re-draw the direction either"
        );
        assert_eq!(steering_state(&mut app), "orbit");

        let mut previous = bearing_around_bogey(&mut app);
        let mut swept = 0.0_f32;
        for _ in 0..30 {
            tick(&mut app);
            let now = bearing_around_bogey(&mut app);
            swept += wrapped_delta(now, previous);
            previous = now;
        }
        assert!(
            swept * direction > 0.0,
            "after the detour the cruiser must go on circling the way it chose \
             (direction {direction}), but it swept {swept} rad"
        );
    }

    /// "Decline rather than invent", on all three combat-orbit scalars.
    ///
    /// Each fails differently if admitted alone, and each failure is worse than
    /// not flying the arm: without `combat_orbit_range` the host would solve a
    /// tangent of a ring of radius zero, which is a spiral straight into the
    /// target; without `combat_orbit_speed` the cruiser would sit at zero
    /// throttle inside a hostile's guns; without `combat_orbit_spiral_gain` it
    /// would fly the bare tangent and hold whatever radius it happened to arrive
    /// at, for ever. So the host gates on all three together and the hull falls
    /// back to ordinary doctrine travel — a behaviour a designer can see.
    ///
    /// The shipped hull orbits at this exact point (asserted above), so nothing
    /// here passes for want of getting that far.
    #[test]
    fn a_hull_omitting_any_combat_orbit_scalar_declines_the_whole_arm() {
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
        for omitted in COMBAT_ORBIT_PARAMS {
            let (mut app, _uuid) = run_to_orbit_omitting(ring, &[omitted]);
            // The machine still enters the state — the verb parses and resolves
            // with or without the params. What must not happen is the HOST
            // flying a ring it has no numbers for.
            assert_eq!(
                steering_state(&mut app),
                "orbit",
                "omitting `{omitted}` must not change which state is entered"
            );
            let pass = pass_surface(&mut app);
            assert!(
                !pass.combat_orbit,
                "omitting `{omitted}` must decline the combat-orbit arm outright"
            );
            assert_eq!(
                pass.combat_orbit_range, 0.0,
                "the whole arm declines together, not part of it"
            );

            // And it stays declined: a run that keeps flying must not quietly
            // start orbiting a few ticks later.
            fly_for(&mut app, 3.0);
            assert!(
                !pass_surface(&mut app).combat_orbit,
                "omitting `{omitted}` must keep declining the arm"
            );
        }
    }

    // ── The cruiser's shield-opportunity torpedo phase (issue #791) ──────────
    //
    // The orbit fixtures above only ever needed the bogey as a row in the
    // `WorldSnapshot`, because ring geometry is solved from the merged view. The
    // torpedo phase is different in two ways, and both drive the fixtures below:
    //
    // * the arc of the TARGET that faces this ship is resolved through that
    //   target's own `Transform` + `ShipShields`, through the same
    //   `attacker_bearing_relative` → `facing_index_for_bearing` pair damage
    //   takes — so the bogey has to be a real entity, not a snapshot row;
    // * "the salvo has resolved" is `TorpedoSystem::in_flight` being empty on
    //   this ship's OWN component, so the cruiser has to carry one.
    //
    // The result is a fixture that composes the helm and weapons conventions,
    // which is what AC7 spans.

    /// Give the cruiser the shipped hull's real torpedo system — the same
    /// `from_configs` call `entities::spawner` makes — so tube capacity, the
    /// magazine and `in_flight` are the hull's own and not a fixture's idea of
    /// them.
    fn attach_cruiser_torpedoes(app: &mut App) {
        let cfg = cruiser_hull();
        let torpedoes = cfg
            .torpedoes
            .as_ref()
            .expect("the shipped cruiser must carry a torpedo magazine");
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::weapons_plugin::TorpedoSystemResource(
                crate::torpedo::TorpedoSystem::from_configs(
                    &torpedoes.tubes,
                    torpedoes.to_runtime(),
                ),
            ));
    }

    /// Spawn the bogey as a real ECS entity carrying shields, alongside the
    /// snapshot row `set_bogey` already wrote.
    ///
    /// Deliberately carries NO `Ship`/`LocalShip` marker and no `ShipPhysics`:
    /// every helper above resolves the cruiser through a `With<Ship>` filter, and
    /// a second marked ship would make them ambiguous. A target with no physics
    /// reads yaw 0, which is exactly what `ai_torpedo_auto_fire` does for the
    /// same case.
    fn spawn_bogey_entity(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3]) -> Entity {
        app.world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(uuid.to_string()),
                Transform::from_xyz(pos[0], pos[1], pos[2]),
                crate::simulation::ShipShields(crate::shield::ShieldSystem::default(), 0.5),
            ))
            .id()
    }

    /// Which of the bogey's arcs currently faces the cruiser, resolved the way
    /// the host resolves it: the bearing of the attacker in the TARGET's frame,
    /// through the target's own priority-tiered router.
    ///
    /// The tests below flip arcs BY THIS INDEX rather than by a hardcoded one, so
    /// they keep meaning "the arc that faces us" if the shield layout is ever
    /// re-authored — and so the negative case ("some other arc is down") can be
    /// expressed at all.
    fn bogey_facing_index(app: &mut App, bogey: Entity) -> usize {
        let pose = ship_pose(app);
        let transform = *app
            .world()
            .get::<Transform>(bogey)
            .expect("bogey carries a Transform");
        let shields = app
            .world()
            .get::<crate::simulation::ShipShields>(bogey)
            .expect("bogey carries ShipShields");
        let incoming = crate::shield::attacker_bearing_relative(
            pose.x,
            pose.z,
            transform.translation.x,
            transform.translation.z,
            0.0,
        );
        shields.0.facing_index_for_bearing(incoming)
    }

    /// Knock one of the bogey's arcs offline (or bring it back).
    fn set_bogey_arc_online(app: &mut App, bogey: Entity, index: usize, online: bool) {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::simulation::ShipShields>(bogey)
            .expect("bogey carries ShipShields");
        shields.0.facings[index].offline_remaining = if online { 0.0 } else { 30.0 };
    }

    /// How many rounds the cruiser has in the air, written directly. Standing in
    /// for `handle_fire_torpedo` (which is not in this fixture's schedule): what
    /// the doctrine reads is the count, and writing it is the smallest thing that
    /// makes the salvo half of AC4 observable.
    fn set_torpedoes_in_flight(app: &mut App, n: usize) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        torpedoes.0.in_flight.clear();
        for i in 0..n {
            torpedoes.0.in_flight.push(crate::torpedo::Torpedo {
                uuid: format!("salvo-{i}"),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                heading: 0.0,
                pitch: 0.0,
                lifespan_remaining: 5.0,
                target_uuid: None,
                source_uuid: None,
                tube_id: "bow_port".into(),
                shield_pierce: 0.0,
            });
        }
    }

    /// Bring every tube to its authored `volley_max` and take the rounds out of
    /// the magazine, exactly as a completed load cycle would.
    ///
    /// Standing in for the loader, which is not in this fixture's schedule, and
    /// it has to stand in because the entry guard asks `tubes_full`: a cruiser
    /// fresh off `from_configs` has empty tubes and a 9-second-per-round load
    /// time, so without this no fixture below would ever reach the phase at all.
    /// That is the doctrine working — a reloading cruiser keeps circling — but it
    /// makes "loaded" a precondition every opportunity fixture has to establish
    /// rather than assume.
    fn fill_the_tubes(app: &mut App) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        let mut drawn = 0;
        for tube in &mut torpedoes.0.tubes {
            drawn += tube.volley_max.saturating_sub(tube.loaded_count);
            tube.loaded_count = tube.volley_max;
            tube.load_state = crate::torpedo::TubeLoadState::Unloaded;
        }
        torpedoes.0.torpedoes_remaining = torpedoes.0.torpedoes_remaining.saturating_sub(drawn);
    }

    /// Empty every tube without touching the magazine — the tube half of the
    /// state a cruiser is in the instant after it launches.
    fn empty_the_tubes(app: &mut App) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        for tube in &mut torpedoes.0.tubes {
            tube.loaded_count = 0;
            tube.load_state = crate::torpedo::TubeLoadState::Unloaded;
        }
    }

    /// The cruiser settled on its fighting ring, with a live shielded bogey
    /// entity, its own tubes, and a salvo loaded in them. Returns the app, the
    /// bogey's uuid and the bogey's entity.
    fn opportunity_app_omitting(omit: &[&str]) -> (App, uuid::Uuid, Entity) {
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
        let (mut app, uuid) = run_to_orbit_omitting(ring, omit);
        attach_cruiser_torpedoes(&mut app);
        fill_the_tubes(&mut app);
        let bogey = spawn_bogey_entity(&mut app, uuid, ORBIT_BOGEY);
        tick_twice(&mut app);
        (app, uuid, bogey)
    }

    fn opportunity_app() -> (App, uuid::Uuid, Entity) {
        opportunity_app_omitting(&[])
    }

    /// Signed bearing from the cruiser's own bow to the bogey, radians. Zero is
    /// bow-on; the sign is which side the target sits.
    fn bearing_to_bogey(app: &mut App) -> f32 {
        let p = ship_pose(app);
        crate::ai::target_relative_motion(
            [p.x, p.y, p.z],
            p.yaw,
            p.forward_speed,
            ORBIT_BOGEY,
            Some(0.0),
            0.0,
        )
        .bearing_rad
    }

    /// AC1: a down target-facing shield arc breaks the orbit into a bow-on hold,
    /// and the hold cuts thrust.
    ///
    /// The negative half comes first and is the anti-trap for an unseeded fact:
    /// a `fact(target_facing_shield_down)` name that were never seeded would
    /// parse, validate, and read false for ever, so the cruiser would circle
    /// exactly as it does today and every #790 assertion would still pass. That
    /// this fixture can distinguish "arc up" from "arc down" is the proof.
    #[test]
    fn a_downed_target_arc_breaks_the_orbit_into_a_bow_on_hold_with_thrust_cut() {
        let (mut app, _uuid, bogey) = opportunity_app();

        // Healthy shields: the cruiser keeps circling.
        assert_eq!(steering_state(&mut app), "orbit");
        assert!(pass_surface(&mut app).combat_orbit);
        assert!(
            !pass_surface(&mut app).torpedo_bearing,
            "a target behind a healthy arc offers no opportunity"
        );

        // Knock the arc that actually faces us offline.
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);

        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "a down facing arc must break the orbit — if this reads `orbit` the \
             shield fact is not reaching the transition guard"
        );
        assert_eq!(
            engines_state(&mut app),
            "torpedo_run",
            "Engines runs its OWN copy of the machine and must reach the phase from \
             the same shared facts, not by reading Steering's state"
        );

        let pass = pass_surface(&mut app);
        assert!(pass.active);
        assert!(pass.torpedo_bearing, "the published leg is the bow hold");
        assert!(
            !pass.combat_orbit && !pass.recover && !pass.reengage && !pass.escape,
            "the bow hold is its own leg — it must not masquerade as the ring it \
             just left, nor as the recovery pivot whose geometry it shares"
        );
        assert_eq!(
            pass.torpedo_bearing_speed,
            cruiser_steering_param(TORPEDO_BEARING_SPEED_PARAM),
            "the hold flies the hull's OWN authored throttle"
        );

        // The thrust axis actually cuts, and the ship actually slows.
        let entry_speed = ship_pose(&mut app).forward_speed;
        assert!(
            entry_speed > 0.0,
            "precondition: the cruiser must have been moving, or 'cuts thrust' is \
             unobservable"
        );
        fly_for(&mut app, 2.0);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        assert_eq!(
            get_thrust_input(&mut app),
            cruiser_steering_param(TORPEDO_BEARING_SPEED_PARAM),
            "the commanded throttle is the authored bow-hold fraction"
        );
        assert!(
            ship_pose(&mut app).forward_speed < entry_speed,
            "and the hull is genuinely slowing: {} vs {entry_speed}",
            ship_pose(&mut app).forward_speed
        );

        // ...and it swings its bow onto the target rather than holding the beam
        // aspect the ring left it in. The hull's authored `max_yaw_rate` is 0.30
        // rad/s and the orbit hands the phase a target roughly abeam, so this is
        // a manoeuvre that takes several seconds by construction.
        let entry_bearing = bearing_to_bogey(&mut app).abs();
        fly_for(&mut app, 8.0);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        let held_bearing = bearing_to_bogey(&mut app).abs();
        assert!(
            held_bearing < entry_bearing && held_bearing < 0.2,
            "the bow must come onto the target ({entry_bearing} → {held_bearing} rad)"
        );
    }

    /// AC1's other half: the arc that matters is the one that FACES this ship,
    /// resolved through the target's own router.
    ///
    /// A cruiser that opened the phase on "any arc is down" would break its orbit
    /// for a hole in the far side of the enemy and then hold its bow on a healthy
    /// shield until something else moved. This is the assertion that the belief
    /// and the shot agree about which arc is in the way.
    #[test]
    fn only_the_arc_that_faces_the_cruiser_opens_the_opportunity() {
        let (mut app, _uuid, bogey) = opportunity_app();
        let facing = bogey_facing_index(&mut app, bogey);

        // Knock out every OTHER arc.
        let arcs = app
            .world()
            .get::<crate::simulation::ShipShields>(bogey)
            .expect("bogey carries ShipShields")
            .0
            .facings
            .len();
        assert!(arcs > 1, "precondition: the bogey must have several arcs");
        for index in 0..arcs {
            if index != facing {
                set_bogey_arc_online(&mut app, bogey, index, false);
            }
        }
        fly_for(&mut app, 1.0);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "arcs on the far side of the target are no opportunity at all"
        );
        assert!(!pass_surface(&mut app).torpedo_bearing);

        // Now the one that does face us.
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "torpedo_run");
    }

    /// Leave the cruiser in the state it is in after its last salvo: nothing in
    /// the tubes and nothing left in the magazine to reload them with.
    fn spend_the_magazine(app: &mut App) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        torpedoes.0.torpedoes_remaining = 0;
        for tube in &mut torpedoes.0.tubes {
            tube.loaded_count = 0;
            tube.load_state = crate::torpedo::TubeLoadState::Unloaded;
        }
    }

    /// Put `rounds` back in the magazine — the positive control for the two
    /// tests below, so neither can pass by simply never reaching the phase.
    fn restock_the_magazine(app: &mut App, rounds: u32) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        torpedoes.0.torpedoes_remaining = rounds;
    }

    /// Mark one of the cruiser's fine systems damage-offline, the way
    /// `sync_console_damage_tiers` does when a console crosses into
    /// Disabled/Destroyed.
    fn knock_system_offline(app: &mut App, system_id: &str) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut sources = entity
            .get_mut::<ShipSystemControlSources>()
            .expect("the cruiser carries control sources");
        sources
            .0
            .set_offline(crate::messages::SystemId(system_id.into()), true);
    }

    /// The doctrine-quality guard #790's premise depends on: a cruiser that
    /// CANNOT launch does not give up its broadside orbit to try.
    ///
    /// The end of the line for this hull's armament: the state it is left in
    /// after its last salvo, tubes empty and no rounds left to refill them with.
    /// The magazine is 8 rounds against a 4-round salvo, so the cruiser gets two
    /// torpedo runs and then never another, and from that point on every arc it
    /// collapses is somebody else's opportunity.
    ///
    /// With no armament conjunct on the entry guard at all the cruiser could not
    /// see that: from the third arc collapse onward it broke the ring, cut thrust
    /// and held its nose on the enemy for the rest of the fight. Measured over a
    /// 180 s headless `combat_test` run, 287 ticks in `torpedo_run` against 87 in
    /// `orbit` — the interruption had become the hull's normal combat mode, which
    /// is precisely what #790 says it must not be.
    ///
    /// The restock at the end is the anti-vacuity half: the same arc, the same
    /// target, the same tick cadence, and the only things that changed are the
    /// rounds.
    #[test]
    fn a_spent_magazine_keeps_the_cruiser_in_its_orbit() {
        let (mut app, _uuid, bogey) = opportunity_app();
        spend_the_magazine(&mut app);

        // The opportunity opens, and it is a real one — the arc that faces us is
        // down. The only thing missing is anything to shoot through it with.
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        fly_for(&mut app, 2.0);

        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "a cruiser with an empty magazine must keep circling: breaking the ring \
             costs it the broadside and buys a salvo it cannot load"
        );
        assert_eq!(
            engines_state(&mut app),
            "orbit",
            "the travel axis reads the same fact and must reach the same conclusion"
        );
        let pass = pass_surface(&mut app);
        assert!(pass.combat_orbit, "the ring is still the manoeuvre");
        assert!(
            !pass.torpedo_bearing,
            "and the bow hold was never published"
        );

        // Anti-vacuity: rounds back and a salvo loaded from them, same
        // everything else, phase opens. Both halves are needed because both
        // halves of "can shoot into it" are guarded — restocking alone leaves
        // the hull in its reload window, which is the case the test below owns.
        restock_the_magazine(&mut app, 8);
        fill_the_tubes(&mut app);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "with the magazine restocked the SAME opportunity must open — otherwise \
             this test is only asserting that the phase never runs"
        );
    }

    /// The case that is `tubes_fillable`'s alone, and the reason it stays
    /// conjoined beside `tubes_full` rather than being replaced by it: a tube
    /// that has been shot out.
    ///
    /// The fixture arrives here with a full salvo loaded, so `tubes_full` reads
    /// TRUE throughout — being loaded is a fact about the rounds in the tubes and
    /// says nothing about whether the tube can still fire them. Only
    /// `tubes_fillable` looks at the fine system, and `handle_fire_torpedo` gates
    /// a launch on exactly that, so a cruiser without this conjunct would break
    /// its orbit and hold its nose out for a salvo the launcher will decline.
    ///
    /// One dead tube is enough: the salvo doctrine this hull authors is
    /// all-or-nothing, so the launch gate is an ALL-tubes reading however healthy
    /// the other tube is.
    #[test]
    fn a_shot_out_tube_keeps_the_cruiser_in_its_orbit() {
        let (mut app, _uuid, bogey) = opportunity_app();
        knock_system_offline(&mut app, "torpedo-tube-bow-port");

        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        fly_for(&mut app, 2.0);

        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "one dead tube makes a full salvo unreachable, so the opportunity is \
             not one"
        );
        assert!(!pass_surface(&mut app).torpedo_bearing);
    }

    /// The RELEASE half of the case above, which the entry guard cannot cover
    /// and the salvo-spent resume structurally cannot see.
    ///
    /// `tubes_full` reads the rounds, not the tubes. Destroying a tube leaves
    /// `loaded_count` exactly where it was, so a hull that loses a tube AFTER
    /// the phase opens still reads `tubes_full` true, has nothing in the air,
    /// and — against a target that carries no arc to raise — has every exit
    /// shut. It sits bow-on, thrust cut, for a salvo `handle_fire_torpedo` will
    /// decline, which is the same trap the salvo-spent bound was added to close,
    /// reopened by a lucky hit.
    ///
    /// The bound that catches it is reachability, which the entry guard already
    /// asks and which now sits on an exit as well. The target is deliberately
    /// shieldless so the window-closed resume is unable to fire at all: whatever
    /// releases the hull here can only be the hull's own armament.
    #[test]
    fn a_tube_shot_out_mid_phase_releases_the_cruiser_back_to_its_orbit() {
        let (mut app, _uuid, bogey) = opportunity_app();
        app.world_mut()
            .entity_mut(bogey)
            .remove::<crate::simulation::ShipShields>();
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "precondition: an unshielded target opens the phase"
        );

        // Battery intact and loaded: the phase holds, and it holds indefinitely
        // — this target will never close the window for it.
        fly_for(&mut app, 3.0);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        assert!(
            tubes_are_full(&mut app),
            "precondition: a full battery is what makes the salvo-spent resume \
             unable to end this phase"
        );

        // A hit takes a tube out while rounds are still in the air.
        knock_system_offline(&mut app, "torpedo-tube-bow-port");
        set_torpedoes_in_flight(&mut app, 2);
        fly_for(&mut app, 2.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "the battery-lost resume takes the same in-flight conjunct as the \
             other two: a hull does not turn away from rounds it has committed, \
             whatever has happened to the tube that fired them"
        );

        // The rounds resolve. Nothing is committed, the battery can never be
        // filled again, and there is no arc to come back.
        set_torpedoes_in_flight(&mut app, 0);
        fly_for(&mut app, 2.0);
        assert!(
            tubes_are_full(&mut app),
            "the rounds are still sitting in the dead tube — which is precisely \
             why `tubes_full` cannot be the reading that ends this phase"
        );
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "a cruiser whose battery has been shot out must go back to raking \
             with the beams it still has — with a full battery, nothing in the \
             air and a target with no arc to raise, reachability is the ONLY \
             thing left that can release the hull"
        );
        assert_eq!(
            engines_state(&mut app),
            "orbit",
            "the travel axis is bounded by the same reading, not by Steering's state"
        );
        let pass = pass_surface(&mut app);
        assert!(
            pass.combat_orbit && !pass.torpedo_bearing,
            "and the published leg is the ring again, with thrust restored"
        );

        // ...and it stays out: the entry guard asks reachability too, so a dead
        // tube keeps the phase shut rather than chattering the hull in and out.
        fly_for(&mut app, 4.0);
        assert_eq!(steering_state(&mut app), "orbit");
    }

    /// The dominant-state bug, and the reason the entry guard asks the
    /// LAUNCHER's question rather than the reachability one.
    ///
    /// This is the reload window: tubes empty, magazine full, every fine system
    /// healthy. `tubes_fillable` is TRUE throughout it — a whole salvo is
    /// perfectly reachable, in 18 seconds — and it is true throughout the
    /// initial load-up too, so a cruiser guarded on reachability alone breaks
    /// its ring for arc collapses it cannot possibly shoot into. Measured over a
    /// 400 sim-second `combat_test` run before `tubes_full` was conjoined: 506
    /// bow-on ticks against 431 orbiting, and only 29 of the 506 — 5.7% — with
    /// the tubes actually full. The "brief interruption" was the majority of
    /// resolved manoeuvre time and almost none of it could have ended in a shot.
    ///
    /// The restock/spend pair at the end is the anti-vacuity half, and it is
    /// deliberately the OTHER axis of the same question: same arc, same target,
    /// same cadence, magazine untouched, and the only thing that changes is
    /// whether the rounds are in the tubes.
    #[test]
    fn a_cruiser_mid_reload_keeps_its_orbit_however_full_the_magazine_is() {
        let (mut app, _uuid, bogey) = opportunity_app();
        // The state the hull is in for 18 seconds after every salvo: nothing in
        // the tubes, plenty in the magazine.
        empty_the_tubes(&mut app);
        restock_the_magazine(&mut app, 8);

        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        fly_for(&mut app, 3.0);

        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "a reloading cruiser must keep circling: the tubes cannot fill inside \
             the window, so breaking the ring buys a shot that will not be taken"
        );
        assert_eq!(
            engines_state(&mut app),
            "orbit",
            "the travel axis reads the same facts and must reach the same conclusion"
        );
        let pass = pass_surface(&mut app);
        assert!(pass.combat_orbit, "the ring is still the manoeuvre");
        assert!(
            !pass.torpedo_bearing,
            "and the bow hold was never published"
        );

        // Anti-vacuity: load the salvo and nothing else. The magazine was
        // already full, so `tubes_fillable` did not change — only `tubes_full`
        // did, which is the whole claim.
        fill_the_tubes(&mut app);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "with the salvo loaded the SAME opportunity must open — otherwise this \
             test is only asserting that the phase never runs"
        );
    }

    /// AC3: a shield that recovers before anything has launched aborts the
    /// opportunity, and the cruiser resumes its orbit.
    #[test]
    fn a_recovered_shield_aborts_the_opportunity_before_launch() {
        let (mut app, _uuid, bogey) = opportunity_app();
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        assert_eq!(
            torpedoes_in_flight(&mut app),
            0,
            "precondition: this is the BEFORE-launch abort"
        );

        set_bogey_arc_online(&mut app, bogey, facing, true);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "with nothing in the air, a recovered arc ends the opportunity"
        );
        assert_eq!(engines_state(&mut app), "orbit");
        let pass = pass_surface(&mut app);
        assert!(
            pass.combat_orbit && !pass.torpedo_bearing,
            "and the published leg goes back to the ring"
        );
    }

    /// The cruiser's own count of rounds in the air, read off the live component
    /// the doctrine reads.
    fn torpedoes_in_flight(app: &mut App) -> usize {
        app.world_mut()
            .query::<&crate::weapons_plugin::TorpedoSystemResource>()
            .single(app.world())
            .expect("the cruiser carries a torpedo system")
            .0
            .in_flight
            .len()
    }

    /// AC4: once a salvo is away the cruiser stays bow-on until every round has
    /// hit, missed or expired — even after the shield it was shooting at comes
    /// back.
    ///
    /// The recovered-shield half is the load-bearing one. A shield regenerates
    /// while the rounds are flying essentially every time, so an exit guarded on
    /// the shield alone would turn the cruiser away mid-salvo in almost every
    /// real engagement — and the abort test above would still pass.
    #[test]
    fn a_salvo_in_flight_holds_the_cruiser_bow_on_after_the_shield_recovers() {
        let (mut app, _uuid, bogey) = opportunity_app();
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "torpedo_run");

        // A salvo is away, and the arc recovers behind it.
        set_torpedoes_in_flight(&mut app, 2);
        set_bogey_arc_online(&mut app, bogey, facing, true);
        fly_for(&mut app, 2.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "a recovered shield must NOT release the hull while its own rounds are \
             still in the air"
        );
        assert!(pass_surface(&mut app).torpedo_bearing);

        // One round resolves; one is still flying. Still committed.
        set_torpedoes_in_flight(&mut app, 1);
        fly_for(&mut app, 1.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "the commitment is to the whole salvo, not to its first round"
        );

        // The last one hits, misses or expires — `in_flight` is empty either way.
        set_torpedoes_in_flight(&mut app, 0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "with the salvo resolved and the arc back, the phase is over"
        );
        assert!(pass_surface(&mut app).combat_orbit);
    }

    /// Fire a REAL salvo out of the cruiser's own tubes, through the same
    /// `TorpedoSystem::launch` call `handle_fire_torpedo` makes.
    ///
    /// Deliberately not [`set_torpedoes_in_flight`], which writes `in_flight`
    /// directly and so stages a salvo that is fully airborne from its first tick.
    /// A real burst launch is not: round 0 of each tube goes immediately and the
    /// rest are left as a [`crate::torpedo::TubeBurstState`] waiting on
    /// `burst_interval_secs`. Returns `(airborne, owed)`.
    fn launch_a_real_salvo(app: &mut App) -> (usize, u32) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        let ids: Vec<String> = torpedoes.0.tubes.iter().map(|t| t.id.clone()).collect();
        for id in &ids {
            let result =
                torpedoes
                    .0
                    .launch(id, format!("salvo-{id}"), 0.0, 0.0, 0.0, 0.0, None, None);
            assert!(
                matches!(result, crate::torpedo::LaunchResult::Launched { .. }),
                "precondition: tube '{id}' must have a volley loaded to fire, got {result:?}"
            );
        }
        (
            torpedoes.0.in_flight.len(),
            torpedoes.0.burst_states.iter().map(|b| b.pending).sum(),
        )
    }

    /// Rounds this ship has COMMITTED to a burst but not yet put in the air.
    fn torpedoes_pending(app: &mut App) -> u32 {
        let ship = find_ship_entity(app);
        app.world()
            .get::<crate::weapons_plugin::TorpedoSystemResource>(ship)
            .expect("the cruiser carries a torpedo system")
            .0
            .burst_states
            .iter()
            .map(|b| b.pending)
            .sum()
    }

    /// Whether every tube is at its `volley_max` — the reading the doctrine's
    /// `tubes_full` fact takes, read here so a fixture can assert on the state
    /// that arms the salvo-spent resume.
    fn tubes_are_full(app: &mut App) -> bool {
        let ship = find_ship_entity(app);
        let torpedoes = app
            .world()
            .get::<crate::weapons_plugin::TorpedoSystemResource>(ship)
            .expect("the cruiser carries a torpedo system");
        !torpedoes.0.tubes.is_empty()
            && torpedoes
                .0
                .tubes
                .iter()
                .all(|t| t.loaded_count >= t.volley_max)
    }

    /// Advance the cruiser's own torpedo system by `dt` through the real
    /// `TorpedoSystem::tick`, so a burst pays its owed rounds out by production
    /// code rather than by fixture fiat. This fixture's schedule carries no
    /// weapons plugin, so nothing else moves the burst timer.
    fn tick_the_torpedoes(app: &mut App, dt: f32) {
        let ship = find_ship_entity(app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut torpedoes = entity
            .get_mut::<crate::weapons_plugin::TorpedoSystemResource>()
            .expect("the cruiser carries a torpedo system");
        let mut n = 0_u32;
        torpedoes
            .0
            .tick(dt, &std::collections::HashMap::new(), &mut || {
                n += 1;
                format!("burst-{n}")
            });
    }

    /// AC4's other half, and the one the test above structurally cannot see.
    ///
    /// That fixture writes `in_flight` directly, so its salvo is airborne all at
    /// once and the count it asserts on can only fall. A real salvo can fall to
    /// zero and come back: a burst launch puts one round per tube in the air and
    /// leaves the rest pending on `burst_interval_secs` (0.35 s), and the
    /// airborne rounds can resolve inside that gap. They do — the cruiser enters
    /// the phase with thrust cut and the target closing, and an instrumented
    /// `combat_test` run measured a salvo's first pair away at t=172.10 and both
    /// resolved by t=172.33.
    ///
    /// What made that a bug rather than a curiosity is what else is true on the
    /// launch tick: firing empties the tubes, so `tubes_full` is already false
    /// and the salvo-spent resume is armed with `torpedoes_in_flight` the only
    /// conjunct still holding it shut. Counting the airborne half alone released
    /// the hull mid-salvo, and the owed rounds then left the tubes in `orbit`
    /// with the bow swinging away — measured at `|bearing| = 0.230` rad and
    /// `in_arc = 0`, i.e. thrown outside the tubes' own 24-degree cone. Counting
    /// the owed rounds holds the bow on and puts them away in arc.
    #[test]
    fn a_pending_burst_holds_the_cruiser_bow_on_between_its_own_rounds() {
        let (mut app, _uuid, bogey) = opportunity_app();
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "torpedo_run");

        // A real salvo, through the launcher's own call.
        let (airborne, owed) = launch_a_real_salvo(&mut app);
        assert_eq!(airborne, 2, "one round per tube leaves immediately");
        assert!(
            owed > 0,
            "precondition: the hull must OWE rounds, or there is no burst to be \
             released in the middle of"
        );
        assert!(
            !tubes_are_full(&mut app),
            "precondition: firing empties the tubes, so the salvo-spent resume is \
             already armed and `torpedoes_in_flight` is the only conjunct holding \
             the phase"
        );

        // The airborne pair resolves before the burst timer elapses — the
        // measured case, and the one that used to release the hull.
        set_torpedoes_in_flight(&mut app, 0);
        assert_eq!(
            torpedoes_pending(&mut app),
            owed,
            "precondition: nothing is in the air and the burst still owes every \
             round it was scheduled with"
        );
        fly_for(&mut app, 1.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "a hull that has committed rounds to a burst must hold its bow on \
             until they have actually left the tubes — releasing here fires the \
             back half of its own salvo out of arc"
        );
        assert_eq!(
            engines_state(&mut app),
            "torpedo_run",
            "the travel axis reads the same fact and must reach the same conclusion"
        );
        assert!(pass_surface(&mut app).torpedo_bearing);

        // The timer elapses and the owed rounds actually leave the tubes.
        tick_the_torpedoes(&mut app, 0.4);
        assert_eq!(torpedoes_pending(&mut app), 0, "the burst has paid out");
        assert_eq!(
            torpedoes_in_flight(&mut app),
            owed as usize,
            "...and every owed round is now airborne"
        );
        fly_for(&mut app, 1.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "still committed — the same rounds, now counted on the other side of \
             the ledger"
        );

        // ...and those resolve too. Anti-vacuity: nothing owed and nothing
        // airborne must end the phase, or this test only asserts it never ends.
        set_torpedoes_in_flight(&mut app, 0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "with the WHOLE salvo resolved the phase is over"
        );
        assert!(pass_surface(&mut app).combat_orbit);
    }

    /// The phase is BOUNDED against a target that can never close the window.
    ///
    /// `target_facing_shield_down` reads `1.0` for a target that resolves but
    /// carries no `[shields]` at all — a station, a probe, any hull authored
    /// without the block — and it reads it for as long as that target lives,
    /// correctly: there is genuinely no arc in the way and there never will be.
    /// (Asteroids are a different case and a safe one: they resolve to no
    /// transform-carrying row here, so the fact reads `0.0` and the phase is
    /// never entered.)
    ///
    /// So the window-closed exit can never fire against such a target, and with
    /// `target_valid` the only other way out the cruiser would hold its bow on a
    /// station, thrust cut, until one of them died. The bound is the salvo-spent
    /// exit, drawn on the hull's own armament: it fires whatever the target does.
    #[test]
    fn a_shieldless_target_does_not_trap_the_cruiser_bow_on() {
        let (mut app, _uuid, bogey) = opportunity_app();

        // Turn the bogey into a resolvable target with NO shield system, leaving
        // everything else — position, uuid, the snapshot row — exactly as the
        // orbit fixture set it. Only the bogey's component goes: this is a
        // statement about what the TARGET carries, not about the cruiser.
        app.world_mut()
            .entity_mut(bogey)
            .remove::<crate::simulation::ShipShields>();
        tick_twice(&mut app);

        // The opportunity opens, and it opens permanently: nothing about this
        // target will ever make `target_facing_shield_down` read zero.
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "an unshielded target is a real opportunity — every arc is 'down'"
        );
        fly_for(&mut app, 5.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "and with a salvo loaded and nothing in the air the hull stays committed"
        );

        // The salvo goes. Emptying the tubes is what `handle_fire_torpedo` does
        // to them, and this fixture's schedule does not run it.
        empty_the_tubes(&mut app);
        set_torpedoes_in_flight(&mut app, 2);
        fly_for(&mut app, 1.0);
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "rounds in the air still hold the hull, exactly as against a shielded \
             target"
        );

        // ...and the rounds resolve. There is still no shield to come back, so
        // this release can only be the armament bound.
        set_torpedoes_in_flight(&mut app, 0);
        fly_for(&mut app, 2.0);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "the cruiser must go back to raking a shieldless target once its salvo \
             is spent — with the window-closed exit unable to fire against a target \
             that has no arc to raise, this is the ONLY bound on the phase, and \
             without it the hull holds its nose on a station for the rest of the run"
        );
        assert_eq!(
            engines_state(&mut app),
            "orbit",
            "the travel axis is bounded by the same reading, not by Steering's state"
        );
        let pass = pass_surface(&mut app);
        assert!(
            pass.combat_orbit && !pass.torpedo_bearing,
            "and the published leg is the ring again, with thrust restored"
        );

        // It stays out: the reload is 9 seconds a round, so the phase cannot
        // immediately re-open and chatter the ship between the two states.
        fly_for(&mut app, 4.0);
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "and it stays on the ring while it reloads rather than chattering"
        );
    }

    /// AC1/AC4, the tracking half: the bow hold follows a target that KEEPS
    /// MOVING, because the facing solution is re-derived from its live position
    /// every tick.
    ///
    /// This is the property that separates `hold_torpedo_bearing` from the
    /// frozen-heading escape leg, and it is the whole reason a fixed forward
    /// tube can be aimed at all.
    #[test]
    fn the_bow_hold_tracks_a_moving_target() {
        let (mut app, uuid, bogey) = opportunity_app();
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        // Settle onto the bow-on solution first, so what follows is the hold
        // reacting to the target rather than the hull still swinging round.
        fly_for(&mut app, 6.0);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        let settled = bearing_to_bogey(&mut app).abs();
        assert!(
            settled < 0.2,
            "precondition: the hull must have come bow-on first, got {settled} rad"
        );

        // Jink the target well off the current bow line, on BOTH sides in turn:
        // each move must command a turn back toward it, and the sign must follow
        // the target rather than being a fixed bias.
        let pose = ship_pose(&mut app);
        for side in [1.0_f32, -1.0] {
            let offset = [
                pose.x + side * 60.0 + pose.yaw.sin() * 60.0,
                0.0,
                pose.z - pose.yaw.cos() * 60.0,
            ];
            set_bogey(&mut app, uuid, offset, 0.0, 0.0);
            app.world_mut()
                .entity_mut(bogey)
                .insert(Transform::from_xyz(offset[0], offset[1], offset[2]));
            // Moving the bogey moves which of ITS arcs faces the cruiser, so the
            // arc knocked offline before the loop is no longer the one the guard
            // reads. Re-derive and re-knock, or the opportunity closes the moment
            // the target jinks: `target_facing_shield_down` goes to 0, the phase
            // aborts back to `orbit`, and both assertions below are then
            // satisfied by `hold_combat_orbit` — a bow-hold test that never
            // exercises the bow hold. (Measured: without this the `side = -1`
            // iteration ran entirely in `orbit`.)
            let moved_facing = bogey_facing_index(&mut app, bogey);
            set_bogey_arc_online(&mut app, bogey, moved_facing, false);
            tick_twice(&mut app);

            // The tripwire for exactly that: everything below is about the HOLD,
            // so the hold has to still be the resolved state.
            assert_eq!(
                steering_state(&mut app),
                "torpedo_run",
                "the bow hold must survive the target jinking — a fallback to \
                 `orbit` would satisfy the tracking assertions below off the \
                 wrong verb"
            );

            let bearing = crate::ai::target_relative_motion(
                {
                    let p = ship_pose(&mut app);
                    [p.x, p.y, p.z]
                },
                ship_pose(&mut app).yaw,
                0.0,
                offset,
                Some(0.0),
                0.0,
            )
            .bearing_rad;
            let steering = get_steering_input(&mut app);
            assert!(
                steering * bearing > 0.0,
                "the hold must turn TOWARD the target's live position (bearing \
                 {bearing} rad, commanded steering {steering})"
            );
            // ...and following it actually closes the angle.
            let before = bearing.abs();
            fly_for(&mut app, 4.0);
            let after = crate::ai::target_relative_motion(
                {
                    let p = ship_pose(&mut app);
                    [p.x, p.y, p.z]
                },
                ship_pose(&mut app).yaw,
                0.0,
                offset,
                Some(0.0),
                0.0,
            )
            .bearing_rad
            .abs();
            assert!(
                after < before,
                "tracking must close the angle on a moved target ({before} → {after})"
            );
        }
    }

    /// AC6, and the reason the phase is a genuinely distinct STATE rather than a
    /// flag on the orbit: coming back re-enters `orbit`, and entering an orbiting
    /// state is what makes the host re-draw the circulation direction from the
    /// seeded key.
    ///
    /// A flag layered on the existing state would leave `hold_combat_orbit`
    /// resolved throughout, so nothing would ever be re-entered and the cruiser
    /// would circle the same way for the whole engagement — which is precisely
    /// what a seeded per-entry choice exists to avoid.
    ///
    /// Asserted over a spread of seeds rather than one: a re-draw legitimately
    /// lands on the same side about half the time, so "it changed for THIS seed"
    /// is not the property. "It can change at all" is.
    #[test]
    fn resuming_the_orbit_after_a_torpedo_run_redraws_the_circulation() {
        fn round_trip(seed: u64) -> (f32, f32) {
            let (mut app, _uuid, bogey) = opportunity_app();
            app.insert_resource(crate::sim_rng::SimRng::new(
                seed,
                crate::sim_rng::SeedSource::Cli,
            ));
            let entity = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(entity)
                .insert(crate::entity_spawner::EntityUuid(
                    uuid::Uuid::from_u128(0x9111_2222_3333_4444_5555_6666_7777_8888).to_string(),
                ));
            // Re-enter the ring once under this seed so the "before" reading is
            // a real draw rather than the hull's authored declaration.
            let facing = bogey_facing_index(&mut app, bogey);
            set_bogey_arc_online(&mut app, bogey, facing, false);
            tick_twice(&mut app);
            set_bogey_arc_online(&mut app, bogey, facing, true);
            tick_twice(&mut app);
            assert_eq!(steering_state(&mut app), "orbit");
            let first = pass_surface(&mut app).orbit_direction;

            // ...and again.
            let facing = bogey_facing_index(&mut app, bogey);
            set_bogey_arc_online(&mut app, bogey, facing, false);
            tick_twice(&mut app);
            assert_eq!(steering_state(&mut app), "torpedo_run");
            set_bogey_arc_online(&mut app, bogey, facing, true);
            tick_twice(&mut app);
            assert_eq!(steering_state(&mut app), "orbit");
            (first, pass_surface(&mut app).orbit_direction)
        }

        // Reproducible for a given seed — the property a replayed run depends on.
        assert_eq!(round_trip(31), round_trip(31));

        // ...and genuinely re-drawn: over a spread of seeds, at least one round
        // trip comes back circling the other way. A flag on the orbit state, or
        // a host that only drew on the FIRST entry, could never produce this.
        let flipped = (1_u64..=12).any(|seed| {
            let (before, after) = round_trip(seed);
            before != after
        });
        assert!(
            flipped,
            "resuming the orbit after a torpedo run must re-draw the circulation \
             direction, but it came back the same way for every seed tried"
        );
    }

    /// "Decline rather than invent", on the bow hold's own scalar.
    ///
    /// `torpedo_bearing_speed` is authored `0.0` on this hull, which is exactly
    /// the value an unauthored param would be mistaken for — so the gate is over
    /// the NAME. A hull that omits it must fly its ordinary leg rather than
    /// coasting to a halt in front of an enemy on a number nobody chose.
    ///
    /// The shipped hull holds its bow at this exact point (asserted above), so
    /// nothing here passes for want of getting that far.
    #[test]
    fn a_hull_omitting_the_bow_hold_throttle_declines_the_whole_arm() {
        for omitted in TORPEDO_BEARING_PARAMS {
            let (mut app, _uuid, bogey) = opportunity_app_omitting(&[omitted]);
            let facing = bogey_facing_index(&mut app, bogey);
            set_bogey_arc_online(&mut app, bogey, facing, false);
            tick_twice(&mut app);

            // The machine still enters the state — the verb parses and resolves
            // with or without the param. What must not happen is the HOST flying
            // a leg it has no throttle for.
            assert_eq!(
                steering_state(&mut app),
                "torpedo_run",
                "omitting `{omitted}` must not change which state is entered"
            );
            let pass = pass_surface(&mut app);
            assert!(
                !pass.torpedo_bearing,
                "omitting `{omitted}` must decline the bow-hold arm outright"
            );
            assert_eq!(
                pass.torpedo_bearing_speed, 0.0,
                "the whole arm declines together, not part of it"
            );

            // And it stays declined.
            fly_for(&mut app, 3.0);
            assert!(
                !pass_surface(&mut app).torpedo_bearing,
                "omitting `{omitted}` must keep declining the arm"
            );
        }
    }

    // ── The Harrow Battleship artillery position (issue #792) ────────────────
    //
    // Same posture as the cruiser block above: these drive the SHIPPED hull's
    // authored policies through a real ticking app, so they fail on the content
    // as well as on the code, and every assertion is about something observable
    // — an admitted actuator input, the published pass surface, the committed
    // policy state, or the ship's own flown range.
    //
    // The ships here are allowed to FLY rather than being posed each tick,
    // because every claim below is a claim about what a position does over time:
    // "holds station" is only observable as a range that stops changing, and
    // "pivots onto a lead" is only observable as a bearing that converges on one.

    fn warhawk_hull() -> crate::entity_config::EntityConfig {
        crate::entity_config::EntityConfig::from_toml(include_str!(
            "../../assets/entities/ship_harrow_warhawk.toml"
        ))
        .expect("the shipped battleship hull must parse")
    }

    /// The battleship's authored Steering `param`s, so expectations below are
    /// arithmetic on named values rather than magic numbers.
    fn warhawk_steering_param(name: &str) -> f32 {
        warhawk_hull()
            .helm_console
            .as_ref()
            .and_then(|hc| hc.steering_ai.as_ref())
            .and_then(|ai| ai.param.get(name).copied())
            .unwrap_or_else(|| panic!("the shipped battleship must author `{name}`"))
    }

    /// The bolt whose flight speed the artillery hold leads by — the hull's
    /// longest-reaching blaster bank, resolved exactly as the host resolves it.
    fn warhawk_artillery_bank() -> crate::entity_config::BlasterBankConfig {
        let cfg = warhawk_hull();
        let wc = cfg
            .weapons_console
            .as_ref()
            .expect("the battleship declares [weapons_console]");
        wc.blaster_banks
            .iter()
            .max_by(|a, b| a.range.total_cmp(&b.range))
            .expect("the battleship carries an artillery bank")
            .clone()
    }

    /// A ship carrying the shipped battleship's two authored policy machines, its
    /// physics envelope, and its artillery bank — the same components
    /// `entities::spawner` would attach — hunting a single named bogey, with the
    /// named STEERING `param`s optionally stripped from the hull before its
    /// policy is built (the partially-authored hull AGENTS.md #11 says must
    /// decline rather than invent).
    ///
    /// The bank is attached because the LEAD SPEED is a host reading of it. A
    /// fixture without one would publish a zero lead speed, the predictive
    /// solution would silently degrade to "aim at where it is", and the aim test
    /// below would pass by measuring the wrong thing.
    ///
    /// The battleship authors no boost drive and no boost doctrine, so nothing
    /// boost-shaped is inserted here either: the fixture mirrors the hull.
    ///
    /// The IMPULSE drive, by contrast, is attached — and it is attached because
    /// leaving it out was how #792's blocking defect hid. `entities::spawner`
    /// gives an `ImpulseConfigResource` to every hull that declares a
    /// `[helm_console]`, and the impulse autopilot in `integrate_ship_physics`
    /// hard-overrides commanded throttle with `thrust = 1.0`. A fixture that
    /// omitted the drive measured this doctrine in a world without the one
    /// component capable of discarding it, so "holds station" could pass here
    /// while the shipped hull sailed straight through its own gun line. The three
    /// pieces below are the spawner's, verbatim in shape: the per-hull drive
    /// config off `[helm_console]`, the authored `[helm_console.impulse_ai]`
    /// policy (falling back to the canonical unconditional permit exactly as the
    /// spawner does, so a hull that stopped authoring one is measured on the
    /// default it would really get), and a `BehaviourSection` — `ai_helm_impulse`
    /// reads `use_impulse` off the doctrine entry matching the top objective, so
    /// without one the drive is unreachable and the fixture is back to lying.
    ///
    /// That doctrine entry is deliberately in the SCENARIO shape rather than the
    /// hull's own: a bare Destroy with no `use_impulse`, which is what
    /// `assets/worlds/duel.toml` and `combat_test.toml`'s wave 8 hand this hull
    /// when they replace its doctrine list wholesale, and which
    /// `effective_use_impulse()` resolves to TRUE. It is the permissive case, so
    /// anything that holds here holds for the hull's own doctrine too.
    ///
    /// Each omitted name must actually be present to begin with, so this cannot
    /// quietly pass by "removing" a param the hull renamed out from under it.
    fn artillery_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
        let mut app = test_app();
        let cfg = warhawk_hull();
        let mut hc = cfg
            .helm_console
            .clone()
            .expect("hull declares [helm_console]");
        for name in omit {
            hc.steering_ai
                .as_mut()
                .expect("hull declares [helm_console.steering_ai]")
                .param
                .remove(*name)
                .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
        }
        let banks: Vec<crate::blaster::BlasterSystem> = cfg
            .weapons_console
            .as_ref()
            .expect("hull declares [weapons_console]")
            .blaster_banks
            .iter()
            .map(|b| crate::blaster::BlasterSystem::new(b.to_runtime()))
            .collect();
        let ship = find_ship_entity(&mut app);
        app.world_mut().entity_mut(ship).insert((
            HelmEnginesAiPolicy(hc.engines_ai.as_ref().unwrap().to_policy().unwrap()),
            HelmSteeringAiPolicy(hc.steering_ai.as_ref().unwrap().to_policy().unwrap()),
            crate::ship_plugin::ShipPhysicsConfigResource(crate::ship_physics::ShipPhysicsConfig {
                max_speed: hc.max_speed,
                max_reverse_speed: hc.max_reverse_speed,
                acceleration: hc.acceleration,
                deceleration: hc.deceleration,
                max_yaw_rate: hc.max_yaw_rate,
                ..crate::ship_physics::ShipPhysicsConfig::new()
            }),
            crate::weapons_plugin::BlasterSystemResource(banks),
            // The impulse drive, exactly as `entities::spawner` builds it — see
            // the doc comment for why its absence was load-bearing.
            HelmImpulseAiPolicy(match hc.impulse_ai.as_ref() {
                Some(ai) => ai.to_policy().expect("authored impulse policy decodes"),
                None => crate::entities::config::default_impulse_ai_config()
                    .to_policy()
                    .expect("the canonical impulse policy decodes"),
            }),
            ImpulseConfigResource {
                charge_duration: hc.impulse_charge_duration,
                speed_multiplier: hc.impulse_speed_multiplier,
                acceleration_multiplier: hc.impulse_acceleration_multiplier,
                engage_distance: hc.impulse_engage_distance,
                cancel_distance: hc.impulse_cancel_distance,
                steering_multiplier: cfg
                    .helm_capability
                    .as_ref()
                    .map(|cap| cap.impulse.steering_multiplier)
                    .unwrap_or(crate::impulse::IMPULSE_STEERING_MULTIPLIER_DEFAULT),
            },
        ));
        let objective = destroy_scored_objective(BOGEY, 80.0);
        // The scenario-shaped doctrine entry the drive's `use_impulse` gate reads
        // — id-matched to the objective above, because that is how the two meet in
        // production. `use_impulse` is left unauthored on purpose (see the doc
        // comment): that is the permissive default every shipped scenario hands
        // this hull.
        set_behaviour_section(
            &mut app,
            crate::entity_config::BehaviourConfig {
                doctrine: vec![crate::entity_config::DoctrineObjective {
                    id: objective.id.clone(),
                    directive_kind: Some("Destroy".into()),
                    base_priority: 80.0,
                    target_speed: 0.9,
                    maintain_range: 25.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        set_ship_blackboard_objectives(&mut app, vec![objective]);
        set_helm_control_source(&mut app, ControlSource::Ai);
        let uuid = uuid::Uuid::new_v4();
        set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
        // The Tactical lock `ai_target_selection` would have published. The helm's
        // travel axes resolve their target from the Destroy directive's own name
        // and so never needed it, but `ai_helm_impulse` resolves through the lock
        // alone — the last of the three things whose absence made this fixture a
        // world the impulse drive could not act in.
        set_ship_combat_lock(&mut app, uuid);
        (app, uuid)
    }

    /// Put the battleship at `start_range` from the bogey at the origin, pointed
    /// straight at it and coasting inbound at its doctrine throttle — the pose
    /// the approach would have delivered it in.
    fn run_to_artillery_omitting(start_range: f32, omit: &[&str]) -> (App, uuid::Uuid) {
        let (mut app, uuid) = artillery_app_omitting(ORBIT_BOGEY, omit);
        let speed = warhawk_hull().helm_console.as_ref().unwrap().max_speed;
        // Down `+Z` from the bogey at the origin, facing `-Z` — which is straight
        // at it, since ship forward is `(sin yaw, -cos yaw)`.
        place_ship(&mut app, 0.0, start_range, 0.0, speed);
        // Two ticks: the first publishes the pass surface, the second is the
        // first planner pass that consumes it (see `HelmPassSurface`).
        tick_twice(&mut app);
        (app, uuid)
    }

    fn run_to_artillery(start_range: f32) -> (App, uuid::Uuid) {
        run_to_artillery_omitting(start_range, &[])
    }

    /// AC1/AC2: the range band is TWO thresholds, and the gap between them is
    /// hysteresis rather than slack.
    ///
    /// Four readings, and each one is a different claim:
    ///
    /// * beyond the outer edge the hull is repositioning, not holding;
    /// * crossing the outer edge INWARD does not stop it — the run-in continues
    ///   through the band, which is the half a single threshold cannot express;
    /// * reaching the inner edge stops it;
    /// * and once holding, drifting back out past the inner edge does NOT restart
    ///   it. Only clearing the OUTER edge does.
    ///
    /// The first reading is also the anti-trap for an unseeded fact: before the
    /// travel axes were seeded a `fact(range_to_target)` guard validated at load
    /// and read false for ever. That this test can distinguish "far" from "near"
    /// at all is the proof the guard actually gates.
    #[test]
    fn the_artillery_band_is_two_thresholds_with_hysteresis_between_them() {
        let max = warhawk_steering_param(MAX_ARTILLERY_RANGE_PARAM);
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        assert!(
            hold < max,
            "precondition: the band must have a gap, or every reading below is \
             the same reading"
        );

        // Beyond the outer edge: closing.
        let (mut app, uuid) = run_to_artillery(max * 1.5);
        assert_eq!(
            steering_state(&mut app),
            "reposition",
            "a target beyond the artillery envelope must be closed on — if this \
             reads `acquire` the travel axis is seeing empty facts"
        );
        assert_eq!(
            engines_state(&mut app),
            "reposition",
            "Engines runs its OWN copy of the machine and must reach the same leg \
             from the same facts, not by reading Steering's state"
        );
        assert!(
            !pass_surface(&mut app).artillery_hold,
            "and the host must not publish the hold leg for a ship still closing"
        );

        // INSIDE the outer edge but outside the inner one: still closing. This is
        // the reading that fails if the two thresholds are collapsed into one.
        let between = (max + hold) * 0.5;
        set_ship_physics(
            &mut app,
            ShipPhysics {
                z: between,
                ..Default::default()
            },
        );
        set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "reposition",
            "inside `max_artillery_range` but outside `artillery_hold_range` the \
             run-in must continue: the ENTRY threshold is the inner one"
        );

        // The inner edge stops it.
        let (mut app, uuid) = run_to_artillery(hold * 0.99);
        assert_eq!(
            steering_state(&mut app),
            "hold",
            "reaching the inner edge must take up the firing position"
        );
        assert_eq!(engines_state(&mut app), "hold");

        // ...and drifting back out past the INNER edge does not restart it.
        let between = (max + hold) * 0.5;
        set_ship_physics(
            &mut app,
            ShipPhysics {
                z: between,
                ..Default::default()
            },
        );
        set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "hold",
            "the EXIT threshold is the outer one — a hull that left the moment it \
             drifted past the entry threshold would chatter across the band"
        );

        // Only clearing the outer edge does.
        set_ship_physics(
            &mut app,
            ShipPhysics {
                z: max * 1.2,
                ..Default::default()
            },
        );
        set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
        tick_twice(&mut app);
        assert_eq!(
            steering_state(&mut app),
            "reposition",
            "beyond `max_artillery_range` the hull must start repositioning again"
        );
        assert_eq!(engines_state(&mut app), "reposition");
    }

    /// AC3, the translational half: inside the band the hull commands the authored
    /// hold throttle, actually comes to a stop, and STAYS at the range it stopped
    /// at.
    #[test]
    fn the_firing_position_holds_station_rather_than_travelling() {
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        let (mut app, _uuid) = run_to_artillery(hold * 0.99);
        assert_eq!(steering_state(&mut app), "hold");

        let pass = pass_surface(&mut app);
        assert!(pass.active, "the battleship must be flying an authored leg");
        assert!(pass.artillery_hold, "and that leg is the artillery hold");
        assert!(
            !pass.combat_orbit
                && !pass.recover
                && !pass.reengage
                && !pass.escape
                && !pass.torpedo_bearing,
            "the artillery hold is its own leg — it must not masquerade as a ring \
             it never flies nor as the bow hold, which leads by nothing"
        );
        assert_eq!(
            pass.artillery_hold_speed,
            warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
            "the hold flies the hull's OWN authored throttle"
        );

        let entry_speed = ship_pose(&mut app).forward_speed;
        assert!(
            entry_speed > 0.0,
            "precondition: the battleship must have been moving, or 'holds \
             station' is unobservable"
        );
        // One more tick before reading the throttle: the surface asserted above
        // was published at the END of this tick, and the planner consumes the
        // PREVIOUS tick's surface (see `HelmPassSurface`'s one-tick offset).
        tick(&mut app);
        assert_eq!(
            get_thrust_input(&mut app),
            warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
            "the commanded throttle is the authored hold fraction"
        );

        // It genuinely stops, and then genuinely stays.
        fly_for(&mut app, 12.0);
        assert_eq!(steering_state(&mut app), "hold");
        let settled = ship_pose(&mut app).forward_speed;
        assert!(
            settled.abs() < 0.1,
            "the hull must come to rest in its firing position, got {settled}"
        );
        let settled_range = range_to_bogey(&mut app);
        fly_for(&mut app, 12.0);
        assert_eq!(steering_state(&mut app), "hold");
        assert!(
            (range_to_bogey(&mut app) - settled_range).abs() < 0.5,
            "and the range must stop changing: {} vs {settled_range}",
            range_to_bogey(&mut app)
        );
    }

    /// AC2/AC3 against the drive that used to discard them: a run-in that starts
    /// OUTSIDE the artillery envelope must end inside the band.
    ///
    /// Every other test in this block poses the hull at or near its holding
    /// radius and measures what it does from there. That skips the one geometry
    /// where the impulse drive engages — the autopilot only lights up beyond
    /// `impulse_engage_distance` (200 by parse default) with the bow on the
    /// target, which is precisely the pose an artillery run-in arrives in. From
    /// there it holds `thrust = 1.0` until the target is inside
    /// `impulse_cancel_distance` (40), overriding the `SetThrust{0.0}` the hold
    /// commands the whole way down. The hull entered `hold` at 180, said stop,
    /// and did not stop.
    ///
    /// So this flies the approach rather than posing it, and asserts the
    /// stopping point — which is where the defect is legible as a number: 180 if
    /// the doctrine is flown, ~40 if the drive is flying the hull instead. The
    /// idle phase is asserted alongside it because it names the CAUSE rather than
    /// the symptom, and would still fail if a future change re-permitted the
    /// channel while some other brake happened to stop the ship in the band.
    #[test]
    fn the_run_in_from_outside_the_envelope_stops_inside_the_band() {
        let max = warhawk_steering_param(MAX_ARTILLERY_RANGE_PARAM);
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);

        // Well beyond the envelope, bow on the target, at cruise — the pose the
        // impulse autopilot engages from.
        let (mut app, _uuid) = run_to_artillery(max * 1.5);
        assert_eq!(
            steering_state(&mut app),
            "reposition",
            "precondition: a target beyond the envelope must start a run-in"
        );

        // Long enough to cover the run-in (120 units at 9 units/s) several times
        // over, so this measures where the hull SETTLES rather than where it
        // happened to be. The drive is sampled every tick rather than read at the
        // end, because it CANCELS itself on arrival — a final reading of `Idle`
        // is what both the healthy hull and the broken one show.
        let mut drive_ever_engaged = None;
        for _ in 0..((60.0 / HELM_AI_MAX_DT_SECS).ceil() as usize) {
            tick(&mut app);
            let phase = get_ship_impulse(&mut app).phase;
            if phase != crate::impulse::ImpulsePhase::Idle && drive_ever_engaged.is_none() {
                drive_ever_engaged = Some((phase, range_to_bogey(&mut app)));
            }
        }

        assert_eq!(
            steering_state(&mut app),
            "hold",
            "the run-in must end in the firing position"
        );
        assert_eq!(
            drive_ever_engaged, None,
            "the battleship must never engage its impulse drive: the autopilot \
             replaces commanded throttle with full thrust, so an engaged drive \
             discards the hold's `SetThrust{{0.0}}` for as long as it runs"
        );
        let settled = ship_pose(&mut app).forward_speed;
        assert!(
            settled.abs() < 0.1,
            "and it must actually be stopped, got {settled}"
        );
        let range = range_to_bogey(&mut app);
        assert!(
            (range - hold).abs() < hold * 0.1,
            "the hull must come to rest at its authored holding radius ({hold}); \
             got {range}. A reading near the drive's `impulse_cancel_distance` is \
             the autopilot having flown the hull through its own gun line"
        );
    }

    /// AC5: the battleship holds rather than retreating when the player closes.
    ///
    /// Stated as the property that would break if a `maintain_range`-style
    /// standoff crept into the doctrine: the target is walked from the inner edge
    /// of the band all the way to point-blank, and at every step the hull must
    /// still be in `hold`, must still command the authored throttle, and must
    /// never command REVERSE.
    #[test]
    fn the_battleship_holds_rather_than_retreating_when_the_target_closes() {
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        let (mut app, uuid) = run_to_artillery(hold * 0.99);
        assert_eq!(steering_state(&mut app), "hold");
        fly_for(&mut app, 12.0);
        let station = range_to_bogey(&mut app);

        // Walk the bogey in. Each step is a real approach, not a teleport to the
        // end: a doctrine that only backed off below some inner limit would slip
        // through a single point-blank reading.
        for fraction in [0.75_f32, 0.5, 0.25, 0.05] {
            let pose = ship_pose(&mut app);
            let closer = [pose.x * (1.0 - fraction), 0.0, pose.z * (1.0 - fraction)];
            set_bogey(&mut app, uuid, closer, 0.0, 0.0);
            fly_for(&mut app, 2.0);

            assert_eq!(
                steering_state(&mut app),
                "hold",
                "a closing target must not push the battleship out of its firing \
                 position (target at {fraction} of the station range)"
            );
            assert_eq!(
                get_thrust_input(&mut app),
                warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
                "and the commanded throttle must stay the authored hold fraction"
            );
            assert!(
                get_thrust_input(&mut app) >= 0.0,
                "a battleship that answered a charge with reverse thrust would be \
                 kiting, which is the manoeuvre this hull deliberately does not fly"
            );
        }

        // The hull has not moved off its station through any of that.
        assert!(
            (range_to_bogey(&mut app) - station).abs() < station,
            "sanity: the hull's own position must not have run away"
        );
        let pose = ship_pose(&mut app);
        assert!(
            pose.forward_speed.abs() < 0.1,
            "and it is still stationary, got {}",
            pose.forward_speed
        );
    }

    /// AC3's facing half, and the whole reason this leg is not
    /// `hold_torpedo_bearing`: the bow goes onto the PREDICTED INTERCEPT, not
    /// onto the target.
    ///
    /// The two are only distinguishable against a target with real crossing
    /// velocity, so the bogey is given one — and the expected lead is derived
    /// from the SAME `predict_intercept_heading` the bolt is launched on, at the
    /// authored bank speed, so this asserts agreement between the aim and the
    /// ballistics rather than agreement with a number written here.
    ///
    /// The control is the assertion that carries it: the settled bow bearing to
    /// where the target IS must be non-zero and must sit on the side the target
    /// is travelling towards. A leg that merely tracked would settle at zero.
    #[test]
    fn the_firing_position_pivots_onto_a_predicted_intercept_not_onto_the_target() {
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        let bank = warhawk_artillery_bank();
        // Crossing square across the line of sight, fast enough that the lead is
        // a real angle rather than float noise.
        let crossing_speed = 24.0_f32;
        let crossing_yaw = std::f32::consts::FRAC_PI_2; // heading +X

        let (mut app, uuid) = run_to_artillery(hold * 0.99);
        assert_eq!(steering_state(&mut app), "hold");
        set_bogey(&mut app, uuid, ORBIT_BOGEY, crossing_yaw, crossing_speed);
        // Let the hull settle onto the solution. The bogey's snapshot position is
        // held still deliberately: a target that both moved and was led would mix
        // "did the bow follow it" into a test about "did the bow lead it".
        fly_for(&mut app, 25.0);
        assert_eq!(
            steering_state(&mut app),
            "hold",
            "everything below is about the HOLD, so the hold must still be the \
             resolved state"
        );

        let pose = ship_pose(&mut app);
        // The heading the gun itself would fire on, from this pose.
        let expected = crate::weapons::blaster::predict_intercept_heading(
            pose.x,
            pose.z,
            ORBIT_BOGEY[0],
            ORBIT_BOGEY[2],
            crossing_yaw.sin() * crossing_speed,
            -crossing_yaw.cos() * crossing_speed,
            bank.projectile_speed,
            pose.yaw,
            0.0,
        );
        let lead_error = crate::ai::target_relative_motion(
            [pose.x, pose.y, pose.z],
            pose.yaw,
            0.0,
            [
                pose.x + expected.sin() * 100.0,
                0.0,
                pose.z - expected.cos() * 100.0,
            ],
            Some(0.0),
            0.0,
        )
        .bearing_rad;
        assert!(
            lead_error.abs() < warhawk_steering_param(TRACKING_DEADBAND_PARAM) * 2.0,
            "the bow must settle on the heading the gun fires ({expected} rad); \
             residual bearing error {lead_error} rad"
        );

        // ...and that heading is NOT the bearing to the target. This is the
        // control: a leg that tracked the live position would leave this at zero.
        let live_error = bearing_to_bogey(&mut app);
        assert!(
            live_error.abs() > warhawk_steering_param(TRACKING_DEADBAND_PARAM) * 3.0,
            "a predictive solution must be OFF the target's live bearing against a \
             crossing target — got {live_error} rad, which is what a plain \
             tracking leg would produce"
        );
        assert!(
            live_error < 0.0,
            "and the lead must fall on the side the target is travelling TOWARDS: \
             the bogey runs +X, so the aim point is to STARBOARD of it and the \
             target's own live bearing therefore sits to port of the bow. A \
             positive reading would be a lead aimed behind a crossing target; got \
             {live_error}"
        );
    }

    /// "Decline rather than invent", over the artillery arm's whole requirement.
    ///
    /// All THREE params, one at a time. The throttle is the one an authored value
    /// cannot distinguish from an omission — this hull authors `0.0` — and the two
    /// ranges are here because a gate over only part of an arm's requirement is
    /// the exact mistake #788's and #790's reviews each caught once.
    ///
    /// The shipped hull holds its position at this exact point (asserted above),
    /// so nothing here passes for want of getting that far.
    #[test]
    fn a_hull_omitting_an_artillery_param_declines_the_whole_arm() {
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        for omitted in ARTILLERY_PARAMS {
            let (mut app, _uuid) = run_to_artillery_omitting(hold * 0.99, &[omitted]);

            // Omitting the THROTTLE changes nothing about the machine — the verb
            // parses and resolves either way, and the state is reached exactly as
            // the shipped hull reaches it. That is what makes this the sharp case:
            // the leg is selected, the host simply refuses to fly it. (The two
            // range params are different: their names appear in the machine's own
            // guards, so removing one strands the machine in `acquire`. It is
            // rejected outright at content load — see
            // `harrow_warhawk_cannot_drop_a_guard_referenced_artillery_range` —
            // and this loop covers what the host does if one ever reached it.)
            if *omitted == ARTILLERY_HOLD_SPEED_PARAM {
                assert_eq!(
                    steering_state(&mut app),
                    "hold",
                    "omitting `{omitted}` must not change which state is entered"
                );
            }
            let pass = pass_surface(&mut app);
            assert!(
                !pass.artillery_hold,
                "omitting `{omitted}` must decline the artillery arm outright"
            );
            assert_eq!(
                pass.artillery_hold_speed, 0.0,
                "the whole arm declines together, not part of it"
            );

            // And it stays declined.
            fly_for(&mut app, 3.0);
            assert!(
                !pass_surface(&mut app).artillery_hold,
                "omitting `{omitted}` must keep declining the arm"
            );
        }
    }

    /// AC6: hazard avoidance may RELOCATE the firing position, and may not turn
    /// the battleship into something that orbits or kites.
    ///
    /// The relocation is measured on the LATERAL axis, and it has to be: a hull
    /// that has come to rest projects no forward path, so `avoidance_steering` —
    /// which is the layer that bends a *travelling* leg — is zero by construction
    /// for a ship holding station (`avoidance_steering_is_zero_when_stationary`
    /// pins that directly). `ai_helm_lateral_thrust` is the layer that actually
    /// nudges a stopped hull sideways off its held point, it runs as its own fine
    /// system, and it never touches Engines or Steering — which is exactly what
    /// makes the detour "limited". The pure planner's own additive fold is pinned
    /// where it is testable, on the pure function
    /// (`artillery_position_folds_avoidance_onto_the_intercept_facing`).
    ///
    /// The other half is the absence: the machine must never leave `hold` for any
    /// of it. A detour that became a state would be a manoeuvre with an exit to
    /// get stuck in, and re-entering the hold afterwards would be a second
    /// commitment nobody authored.
    #[test]
    fn a_hazard_relocates_the_firing_position_without_ending_the_hold() {
        let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
        let (mut app, uuid) = run_to_artillery(hold * 0.99);
        fly_for(&mut app, 12.0);
        assert_eq!(steering_state(&mut app), "hold");
        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "precondition: an unobstructed gun line commands no dodge, or the \
             reading below is not the hazard's doing"
        );
        let station = range_to_bogey(&mut app);

        // Drop a large, dangerous obstacle right alongside the hull, off to one
        // side so the repulsion is a genuine lateral push rather than a head-on
        // one. The bogey is republished with it: the snapshot is the whole world.
        let pose = ship_pose(&mut app);
        let hazard = uuid::Uuid::new_v4();
        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![
                crate::ai::AiWorldEntity {
                    uuid,
                    name: Some(BOGEY.into()),
                    position: ORBIT_BOGEY,
                    yaw: Some(0.0),
                    forward_speed: 0.0,
                    radius: 3.0,
                    size_rating: 3.0,
                    movable: true,
                    dangerous: true,
                    ..Default::default()
                },
                crate::ai::AiWorldEntity {
                    uuid: hazard,
                    name: Some("rock".into()),
                    position: [pose.x + 5.0, 0.0, pose.z - 5.0],
                    yaw: None,
                    forward_speed: 0.0,
                    radius: 9.0,
                    size_rating: 9.0,
                    movable: false,
                    dangerous: true,
                    ..Default::default()
                },
            ],
        });
        tick_twice(&mut app);

        assert_eq!(
            steering_state(&mut app),
            "hold",
            "a hazard must NOT be a leg: the doctrine authors no hazard-guarded \
             transition, so the detour stays a stateless bend"
        );
        assert!(
            pass_surface(&mut app).artillery_hold,
            "and the published leg is still the artillery hold"
        );
        assert!(
            lateral_intent(&mut app) < 0.0,
            "the hull must be pushed sideways AWAY from the obstacle (it sits to \
             starboard, so the dodge is to port); got {}",
            lateral_intent(&mut app)
        );
        assert_eq!(
            get_thrust_input(&mut app),
            warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
            "and the dodge must not become a translation the ENGINES fly: the \
             hold's throttle is untouched by hazards"
        );

        // Clearing the hazard evaporates the detour: no state was entered, so
        // there is none to leave, and the gun line is where it was.
        set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
        fly_for(&mut app, 3.0);
        assert_eq!(steering_state(&mut app), "hold");
        assert!(pass_surface(&mut app).artillery_hold);
        assert_eq!(
            lateral_intent(&mut app),
            0.0,
            "the dodge must evaporate with the hazard rather than persisting as \
             state"
        );
        assert!(
            (range_to_bogey(&mut app) - station).abs() < 5.0,
            "and the firing position is RELOCATED, not abandoned: {} vs {station}",
            range_to_bogey(&mut app)
        );
    }
}
