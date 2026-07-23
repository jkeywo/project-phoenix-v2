/// Pure AI module — no Bevy imports.
///
/// Contains navigation utilities (`steer_toward`, `avoidance_steering`),
/// per-system operate functions (`operate_helm`), the shared
/// [`assess_hazards`] collision surface (issue #743), and the `CaptainAi`
/// helper. The operate functions are pure: issue #702
/// deleted the `AiMemory` private-reasoning state they used to mutate, so all
/// per-ship AI state now lives in ECS components.
///
/// The old FSM (`AiState`/`TransitionConfig`/`Blackboard`/`tick()`) was
/// dissolved in issue #572; motor behaviour now lives in the operate functions
/// that read the scored objective pool from the viewscreen aggregator.
use std::f32::consts::PI;
use uuid::Uuid;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Arrival radius in world units — closer than this counts as "reached waypoint".
pub const WAYPOINT_ARRIVAL_RADIUS: f32 = 20.0;
/// Angular deadband for `steer_toward`: within this angle, steering = 0.
pub const PATROL_DEADBAND_RAD: f32 = 0.05;
/// Angular error at which steering saturates to ±1.
pub const PATROL_FULL_STEER_RAD: f32 = PI / 4.0;
/// Extra clearance (world units) added on top of radii for collision avoidance.
pub const AVOIDANCE_BUFFER: f32 = 5.0;
/// Look-ahead horizon (seconds) for predictive collision avoidance.
pub const AVOIDANCE_LOOK_AHEAD_SECS: f32 = 3.0;
/// Speed fraction [0, 1] used for the Channel-3 Navigation→Helm handoff
/// (`NavigationWaypoint`) fallthrough when the entity has no `[behaviour]`
/// section to author one. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::nav_handoff_speed`], whose serde
/// default reads this constant so the two cannot drift apart.
pub const NAV_HANDOFF_SPEED: f32 = 0.6;
/// Distance (world units) within which a docking intent switches from normal
/// objective approach to the close-quarters [`docking_close_manoeuvre`].
/// Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::docking_engage_distance`].
pub const DOCKING_ENGAGE_DISTANCE: f32 = 40.0;
/// Speed fraction `[0, 1]` capping the low-speed reverse / lateral translation
/// of a docking close manoeuvre. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::docking_approach_speed`].
pub const DOCKING_APPROACH_SPEED: f32 = 0.3;
const AVOIDANCE_MIN_SPEED: f32 = 0.25;
/// Authored size-ignore ratio default: a ship ignores a hazard whose
/// `size_rating` is below `self_size_rating * ratio`. `0.0` disables the rule
/// (every dangerous hazard is assessed regardless of size), which is the
/// backward-compatible default. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::hazard_ignore_size_ratio`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const HAZARD_IGNORE_SIZE_RATIO: f32 = 0.0;
/// Authored lateral-thrust hazard sensitivity default: the multiplier a fine
/// lateral-thrust actuator applies to the shared hazard assessment's starboard
/// (local `+X`) repulsion component before clamping to `[-1, 1]`. `1.0` passes
/// the boids-style repulsion through unweighted. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::lateral_hazard_sensitivity`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const LATERAL_HAZARD_SENSITIVITY: f32 = 1.0;
/// Authored vertical-thrust hazard sensitivity default (issue #744): the
/// multiplier the vertical-thrust actuator applies to the shared assessment's
/// moving-hazard threat before clamping to `[0, 1]`. `1.0` passes it through
/// unweighted. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::vertical_hazard_sensitivity`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const VERTICAL_HAZARD_SENSITIVITY: f32 = 1.0;
/// Authored maximum vertical offset (world units) a `Bounded` craft may climb
/// away from its cruise plane while dodging (issue #744). Parse-time default
/// only — see [`crate::entity_config::HelmCapabilityConfig::max_vertical_offset`].
pub const MAX_VERTICAL_OFFSET: f32 = 30.0;
/// Authored gradual return-to-cruise gain for `Bounded` craft (issue #744):
/// once avoidance urgency falls, the vertical actuator commands a descent of
/// `-y * VERTICAL_RETURN_RATE` (clamped) so the ship eases back to its cruise
/// plane rather than snapping. Parse-time default only — see
/// [`crate::entity_config::HelmCapabilityConfig::vertical_return_rate`].
pub const VERTICAL_RETURN_RATE: f32 = 0.05;
/// Proportional deceleration factor for approach: thrust begins ramping down
/// when distance is within this multiple of the target stop-distance.
/// At 1.5× the stop threshold the ship starts slowing; at the threshold it
/// reaches zero thrust, preventing overshoot oscillation near targets.
pub const APPROACH_DECEL_FACTOR: f32 = 1.5;

/// Angular tolerance for engaging impulse: the target bearing must be within
/// this many radians of dead-ahead to qualify.
pub const IMPULSE_ANGLE_TOLERANCE_RAD: f32 = 0.08;

// ── Impulse AI decision ───────────────────────────────────────────────────────

/// Outcome of a single `decide_impulse` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpulseDecision {
    /// Engage impulse (start charging).
    Engage,
    /// Cancel impulse (return to idle).
    Cancel,
    /// No change this tick.
    NoChange,
}

/// All inputs required by `decide_impulse`.
#[derive(Debug, Clone, Copy)]
pub struct ImpulseDecisionInput {
    /// Ship position [x, z].
    pub pos: [f32; 2],
    /// Ship yaw in radians.
    pub yaw: f32,
    /// Target position [x, y, z].
    pub target_pos: [f32; 3],
    /// Current impulse phase.
    pub phase: crate::impulse::ImpulsePhase,
    /// Minimum distance to engage impulse.
    pub engage_distance: f32,
    /// Distance below which impulse is cancelled.
    pub cancel_distance: f32,
    /// Angular tolerance in radians for "directly ahead".
    pub angle_tolerance: f32,
}

/// Decide whether to engage, cancel, or leave the impulse drive unchanged.
///
/// Rules:
/// - If the target is within `cancel_distance` and the impulse is not already
///   Idle, return `Cancel`.
/// - If the target is at least `engage_distance` away AND the target bearing
///   is within `angle_tolerance` radians of dead-ahead AND the impulse phase
///   is `Idle`, return `Engage`.
/// - Otherwise return `NoChange`.
pub fn decide_impulse(input: &ImpulseDecisionInput) -> ImpulseDecision {
    let dx = input.target_pos[0] - input.pos[0];
    let dz = input.target_pos[2] - input.pos[1];
    let dist = (dx * dx + dz * dz).sqrt();

    // Cancel when close enough.
    if dist <= input.cancel_distance && input.phase != crate::impulse::ImpulsePhase::Idle {
        return ImpulseDecision::Cancel;
    }

    // Only engage from Idle.
    if input.phase != crate::impulse::ImpulsePhase::Idle {
        return ImpulseDecision::NoChange;
    }

    // Must be far enough to make impulse worthwhile.
    if dist < input.engage_distance {
        return ImpulseDecision::NoChange;
    }

    // Check if target is directly ahead.
    let fwd_x = input.yaw.sin();
    let fwd_z = -input.yaw.cos();
    let dir_x = dx / dist;
    let dir_z = dz / dist;
    let cross = fwd_x * dir_z - fwd_z * dir_x;
    let dot = fwd_x * dir_x + fwd_z * dir_z;
    let angle = cross.atan2(dot);

    if angle.abs() <= input.angle_tolerance {
        ImpulseDecision::Engage
    } else {
        ImpulseDecision::NoChange
    }
}

// ── WorldView ─────────────────────────────────────────────────────────────────

/// A visible entity in the AI's world view.
#[derive(Debug, Clone)]
pub struct AiWorldEntity {
    /// Stable UUID of the entity.
    pub uuid: Uuid,
    /// Authored scenario name or display name, when available.
    pub name: Option<String>,
    /// World-space position [x, y, z].
    pub position: [f32; 3],
    /// Faction UUID, if any.
    pub faction: Option<Uuid>,
    /// Four-quadrant shield state (from the entity broadcast), if the entity has shields.
    pub shields: Option<Vec<crate::messages::ShieldFacingStatus>>,
    /// Hull integrity fraction [0, 1], if known.
    pub hull_fraction: Option<f32>,
    /// Yaw in radians (Y-up, forward = -Z at yaw 0), if known.
    pub yaw: Option<f32>,
    /// Physical radius of the entity (world units) used for collision avoidance.
    pub radius: f32,
    /// Current forward speed of the entity (world units/s) used for predictive avoidance.
    pub forward_speed: f32,
    /// Hazard fact: whether this entity can move under its own power (a ship)
    /// versus being a static obstacle (an asteroid). Published so fine helm
    /// systems can apply their own policy — e.g. a bounded vertical thruster
    /// dodging only moving hazards while engines still brake for static ones
    /// (issue #743).
    pub movable: bool,
    /// Hazard fact: whether this entity is a collision hazard worth avoiding at
    /// all. `false` entities are skipped by [`assess_hazards`]. All physical
    /// obstacles (ships and asteroids) are dangerous today; the fact exists so
    /// non-hazard entities can be published without being dodged (issue #743).
    pub dangerous: bool,
    /// Hazard fact: this entity's authored size rating, used by the
    /// ignore-smaller rule ([`assess_hazards`] skips a hazard whose rating is
    /// below the assessing ship's own, scaled by an authored ratio). Populated
    /// from the collision radius today (issue #743).
    pub size_rating: f32,
}

/// Hand-written so a bare `AiWorldEntity { ..Default::default() }` is a
/// *dangerous* obstacle: collision avoidance is the default posture, so a test
/// or caller that omits `dangerous` still gets an entity the hazard assessment
/// reasons about. A derived `Default` would zero `dangerous` to `false` and
/// silently drop every such entity from the avoidance surface.
impl Default for AiWorldEntity {
    fn default() -> Self {
        Self {
            uuid: Uuid::default(),
            name: None,
            position: [0.0; 3],
            faction: None,
            shields: None,
            hull_fraction: None,
            yaw: None,
            radius: 0.0,
            forward_speed: 0.0,
            movable: false,
            dangerous: true,
            size_rating: 0.0,
        }
    }
}

/// A read-only snapshot of world state visible to the AI.
#[derive(Debug, Clone, Default)]
pub struct WorldView {
    /// Current simulation time in seconds.
    pub sim_time: f64,
    /// Current entity position [x, y, z].
    pub entity_pos: [f32; 3],
    /// Current entity yaw in radians (Y-up, forward = -Z at yaw 0).
    pub entity_yaw: f32,
    /// Named map anchors: name → [x, y, z].
    pub anchors: std::collections::HashMap<String, [f32; 3]>,
    /// All other entities currently visible to this AI.
    pub entities: Vec<AiWorldEntity>,
    /// UUID of an entity that attacked this entity during this tick, if any.
    pub attacker_this_tick: Option<Uuid>,
    /// Faction of this AI entity itself (used to detect enemies).
    pub self_faction: Option<Uuid>,
    /// Effective beam/weapons range of this AI entity (world units), if it has weapons.
    pub entity_weapons_range: Option<f32>,
    /// `true` when the AI entity's phasers are ready to fire this tick.
    pub entity_phaser_ready: bool,
    /// Name of the first ready torpedo tube, if any. `None` = no tubes loaded.
    pub torpedo_tube_ready: Option<String>,
    /// Hull integrity fraction [0, 1] of the AI entity itself.
    pub self_hull_fraction: Option<f32>,
    /// Set to `true` when the current scenario is being unloaded.
    pub scenario_unloaded: bool,
    /// Physical radius of this AI entity (world units), used for collision avoidance.
    pub self_radius: f32,
    /// This AI entity's own size rating, compared against each hazard's
    /// `size_rating` by the authored ignore-smaller rule (issue #743).
    /// Populated from the ship's collision radius today.
    pub self_size_rating: f32,
}

// ── steer_toward ─────────────────────────────────────────────────────────────

/// Proportional steering toward a direction, with deadband and saturation.
///
/// - `yaw`: current entity yaw in radians (forward = `(sin(yaw), -cos(yaw))` in XZ).
/// - `target_dir`: 2-element [dx, dz] unit vector pointing at target.
/// - `deadband_rad`: angular error below which steering = 0.
/// - `full_steer_rad`: angular error at which steering saturates to ±1.
///
/// Returns a steering value in [-1, 1].
pub fn steer_toward(yaw: f32, target_dir: [f32; 2], deadband_rad: f32, full_steer_rad: f32) -> f32 {
    let fwd_x = yaw.sin();
    let fwd_z = -yaw.cos();

    let cross = fwd_x * target_dir[1] - fwd_z * target_dir[0];
    let dot = fwd_x * target_dir[0] + fwd_z * target_dir[1];
    let angle = cross.atan2(dot);

    if angle.abs() < deadband_rad {
        return 0.0;
    }

    (angle / full_steer_rad).clamp(-1.0, 1.0)
}

// ── should_emit ───────────────────────────────────────────────────────────────

/// Returns `true` if the change from `last` to `current` is significant enough
/// to warrant emitting a new input message.
pub fn should_emit(last: f32, current: f32, epsilon: f32) -> bool {
    (current - last).abs() > epsilon
}

// ── visible_entities ──────────────────────────────────────────────────────────

/// Filter `all` down to the entities within `range` of `center`, measured as
/// XZ-plane distance (ignores the Y/vertical component, matching the rest of
/// the AI's ground-plane steering math).
///
/// `range <= 0.0` or a non-finite `range` (NaN/infinite) means "unlimited" —
/// used for rangeless systems that have no radar gating at all. Entities
/// exactly at the boundary (`distance == range`) are included.
pub fn visible_entities(center: [f32; 3], range: f32, all: &[AiWorldEntity]) -> Vec<AiWorldEntity> {
    if range <= 0.0 || !range.is_finite() {
        return all.to_vec();
    }

    all.iter()
        .filter(|e| {
            let dx = e.position[0] - center[0];
            let dz = e.position[2] - center[2];
            let dist = (dx * dx + dz * dz).sqrt();
            dist <= range
        })
        .cloned()
        .collect()
}

// ── Collision avoidance helpers ───────────────────────────────────────────────

fn offset_approach_target(self_pos: [f32; 3], target_pos: [f32; 3], min_dist: f32) -> [f32; 3] {
    let dx = self_pos[0] - target_pos[0];
    let dz = self_pos[2] - target_pos[2];
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1.0 {
        return target_pos;
    }
    let inv_dist = 1.0 / dist;
    let ux = dx * inv_dist;
    let uz = dz * inv_dist;
    [
        target_pos[0] + ux * min_dist,
        target_pos[1],
        target_pos[2] + uz * min_dist,
    ]
}

fn avoidance_steering(
    self_pos: [f32; 3],
    self_yaw: f32,
    self_speed: f32,
    self_radius: f32,
    excluded_uuid: Uuid,
    world_entities: &[AiWorldEntity],
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
) -> f32 {
    if self_speed.abs() < AVOIDANCE_MIN_SPEED {
        return 0.0;
    }

    let fwd_x = self_yaw.sin();
    let fwd_z = -self_yaw.cos();
    let proj_self_x = self_pos[0] + fwd_x * self_speed * avoidance_look_ahead_secs;
    let proj_self_z = self_pos[2] + fwd_z * self_speed * avoidance_look_ahead_secs;

    let mut total_avoidance: f32 = 0.0;

    for entity in world_entities {
        if entity.uuid == excluded_uuid {
            continue;
        }
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;

        let (ent_proj_x, ent_proj_z) = if let Some(ent_yaw) = entity.yaw {
            let ent_fwd_x = ent_yaw.sin();
            let ent_fwd_z = -ent_yaw.cos();
            (
                entity.position[0] + ent_fwd_x * entity.forward_speed * avoidance_look_ahead_secs,
                entity.position[2] + ent_fwd_z * entity.forward_speed * avoidance_look_ahead_secs,
            )
        } else {
            (entity.position[0], entity.position[2])
        };

        let ddx = proj_self_x - ent_proj_x;
        let ddz = proj_self_z - ent_proj_z;
        let proj_dist = (ddx * ddx + ddz * ddz).sqrt();

        if proj_dist < avoidance_radius {
            let threat_fraction = 1.0 - (proj_dist / avoidance_radius);
            let to_x = ent_proj_x - proj_self_x;
            let to_z = ent_proj_z - proj_self_z;
            let cross = fwd_x * to_z - fwd_z * to_x;
            let sign = if cross >= 0.0 { -1.0_f32 } else { 1.0_f32 };
            total_avoidance += sign * threat_fraction;
        }
    }

    total_avoidance.clamp(-1.0, 1.0)
}

// ── Doctrine scoring ──────────────────────────────────────────────────────────

/// Convert a slice of `DoctrineObjective`s into a scored pool using the given
/// world conditions. The pool is sorted descending by score (highest first).
pub fn score_doctrine_pool(
    doctrine: &[crate::entity_config::DoctrineObjective],
    conditions: &crate::objectives::WorldConditions,
) -> Vec<crate::messages::ScoredObjective> {
    let mut pool: Vec<crate::messages::ScoredObjective> = doctrine
        .iter()
        .map(|d| {
            let utility = crate::objectives::UtilityConfig {
                base_priority: d.base_priority,
                modifiers: d.modifiers.clone(),
                zero_gates: d.zero_gates.clone(),
            };
            let score = utility.score(d.mandatory, conditions);
            let directive = parse_doctrine_directive(d);
            let relevance = crate::objectives::directive_relevance(&directive);
            crate::messages::ScoredObjective {
                id: d.id.clone(),
                score,
                directive,
                source: crate::messages::ObjectiveSource::Doctrine,
                relevance,
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: d.id.clone(),
                    text: d.text.clone(),
                    mandatory: d.mandatory,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Doctrine,
                },
            }
        })
        .collect();
    pool.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pool
}

fn parse_doctrine_directive(
    d: &crate::entity_config::DoctrineObjective,
) -> crate::messages::AiDirective {
    match d.directive_kind.as_deref() {
        Some("Patrol") => crate::messages::AiDirective::Patrol {
            anchors: d.directive_anchors.clone(),
            loop_path: d.directive_loop,
        },
        Some("Destroy") => crate::messages::AiDirective::Destroy {
            target: d.directive_target.clone().unwrap_or_default(),
        },
        Some("Reach") => crate::messages::AiDirective::Reach {
            anchor: d.directive_anchor.clone().unwrap_or_default(),
        },
        Some("Hail") => crate::messages::AiDirective::Hail {
            target: d.directive_hail_target.clone().unwrap_or_default(),
        },
        Some("Retreat") => crate::messages::AiDirective::Retreat {
            anchor: d.directive_anchor.clone().unwrap_or_default(),
        },
        _ => crate::messages::AiDirective::None,
    }
}

// ── plan_helm_travel ──────────────────────────────────────────────────────────

/// Shared pure Helm travel doctrine, consumed by the motion planner.
///
/// Renamed from `operate_helm` in issue #745: the legacy per-axis operators no
/// longer each call it — the shared `helm_motion_planner` (via `helm_ai_decision`)
/// is now its sole caller, folding the doctrine decision into the desired-motion
/// contract the per-axis systems decode. The `operate_helm` symbol is retired so
/// the helm-shared-motion-planner-rollout migration's removal condition holds.
///
/// Reads the scored objective pool, selects the top-scoring directive the Helm
/// can serve (Destroy, Patrol, Reach, Retreat), and returns `(thrust, steering)`.
///
/// # Pure (issue #702)
///
/// A pure function of its arguments: it owns no state and mutates nothing.
/// Every goal it serves is read from a surface some other console owns —
/// Tactical's `weapons_target`, Navigation's `nav_waypoint`, the objective's
/// own `cursors`. **Objectives own goals; the Helm only derives controls from
/// them.**
///
/// That is what dissolved the #701 "first per-axis system to run owns the
/// commit" rule. The rule existed solely because this function mutated a shared
/// `AiMemory`, which made calling it twice in a tick (once per axis) unsafe —
/// two commits double-advanced the patrol waypoint, zero commits froze it.
/// Purity removes the hazard rather than guarding it: `ai_helm_thrust` and
/// `ai_helm_steering` can both call this, in any order, any number of times,
/// and each keeps its own axis from an identical answer.
///
/// Arguments that are *not* pure inputs must not be added back. In particular
/// this function does not take a `FactionRegistry`: it has nobody to be hostile
/// to, because it no longer acquires targets (see `helm_destroy`).
///
/// - `cursors` — this ship's `ObjectiveCursors`, read-only. `cursor_target()`
///   resolves the active waypoint; `advance_objective_cursors`
///   (`SimSet::Modifiers`) is the sole writer.
/// - `weapons_target` — the ship's **Combat Lock**, read from the frozen
///   `ViewscreenBlackboard::combat_lock` (the aggregator lifts it from the
///   tactical radar's own selection, #829) — i.e. whoever Tactical, human or AI,
///   has locked. Helm never reaches the radar's live selection directly.
/// - `nav_waypoint` — Navigation's waypoint, `Some` only once the caller has
///   confirmed the Channel-3 clearance matches its generation.
#[allow(clippy::too_many_arguments)]
pub fn plan_helm_travel(
    world_view: &WorldView,
    scored_pool: &[crate::messages::ScoredObjective],
    doctrine: &[crate::entity_config::DoctrineObjective],
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: &[crate::ai::patrol_cursor::PatrolCursor],
    weapons_target: Option<Uuid>,
    nav_waypoint: Option<[f32; 2]>,
    waypoint_arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    nav_handoff_speed: f32,
) -> (f32, f32) {
    use crate::messages::{AiDirective, SystemAffinity};

    // Iterate directives in descending score order. A directive that cannot
    // resolve (e.g. Destroy with an unknown target, or Reach with an unknown
    // anchor) returns `None`; in that case we fall through to the next
    // lower-priority directive rather than leaving the ship idle.
    // `Some((0, 0))` is intentional resolved idleness, such as holding station
    // at weapons range, and must not fall through to Patrol.
    // This ensures a Patrol objective acts as a default when the higher-scored
    // Destroy target has not yet appeared in the world snapshot (as happens at
    // the start of combat_test.toml where wave objectives are added on the
    // same tick as the entities spawn).
    for objective in scored_pool
        .iter()
        .filter(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm))
    {
        let cfg = doctrine.iter().find(|d| d.id == objective.id);
        let result = match &objective.directive {
            AiDirective::Destroy { .. } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.8);
                let maintain_range = cfg.map(|d| d.maintain_range).unwrap_or(25.0);
                // The directive's own `target` is deliberately ignored here: it
                // is Tactical's input, not the Helm's. `ai_target_selection`
                // resolves it (tier 1) and publishes the result as
                // `TacticalRadarSelection`, which is what we pursue — see `helm_destroy`.
                helm_destroy(
                    world_view,
                    avoidance_buffer,
                    avoidance_look_ahead_secs,
                    forward_speed,
                    weapons_target,
                    target_speed,
                    maintain_range,
                )
                // Tactical has locked nothing this Helm can see — the target is
                // over the horizon, or (as with a factionless starbase) never
                // auto-acquired at all. That is exactly the case Navigation's
                // waypoint exists to cover, so close on it *here*, at this
                // objective's priority, rather than falling through.
                //
                // Without this the Helm dropped to the next directive down and
                // a raider ordered to assault the starbase flew its patrol
                // route instead: Patrol resolved and returned before the
                // waypoint fallback at the end of this function was ever
                // reached. Navigation is only consulted when it has actually
                // issued a cleared waypoint, so a Destroy that resolves neither
                // a target nor a waypoint still yields to Patrol.
                .or_else(|| {
                    nav_waypoint.and_then(|[nx, nz]| {
                        helm_navigate_to(
                            world_view,
                            [nx, 0.0, nz],
                            waypoint_arrival_radius,
                            avoidance_buffer,
                            avoidance_look_ahead_secs,
                            forward_speed,
                            target_speed,
                        )
                    })
                })
            }
            AiDirective::Patrol {
                anchors: waypoints,
                loop_path,
            } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.5);
                // The cursor for *this* objective. A ship with no cursor entry
                // yet (the evaluator inserts it at the end of the tick) steers
                // toward the first waypoint in the meantime — the same
                // treatment `simulate_low_lod_ships` gives the gap.
                let index = cursors
                    .iter()
                    .find(|c| c.objective_id == objective.id)
                    .map(|c| c.index())
                    .unwrap_or(0);
                helm_patrol(
                    world_view,
                    waypoints,
                    *loop_path,
                    index,
                    waypoint_arrival_radius,
                    avoidance_buffer,
                    avoidance_look_ahead_secs,
                    forward_speed,
                    target_speed,
                    anchors,
                )
            }
            AiDirective::Reach { anchor } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.6);
                anchors.get(anchor.as_str()).and_then(|&pos| {
                    helm_navigate_to(
                        world_view,
                        pos,
                        waypoint_arrival_radius,
                        avoidance_buffer,
                        avoidance_look_ahead_secs,
                        forward_speed,
                        target_speed,
                    )
                })
            }
            AiDirective::Retreat { anchor } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.6);
                // Retreat resolves its anchor by name and nothing else. An
                // empty or unknown anchor returns `None` and falls through to
                // the next directive, exactly as `Reach` does — a Retreat that
                // names nowhere is a Retreat to nowhere.
                //
                // This arm used to fall back to `AiMemory.home_position` for the
                // benefit of a *synthetic* hull-triggered Retreat that
                // `aggregate_doctrine_blackboards` injected with an empty
                // anchor. #702 deleted that injector: it was broken three ways
                // (home_position was never seeded in production, so every
                // shipped ship "retreated" to world origin; its 0..1 score could
                // never outrank doctrine's tens; the anchor was empty only
                // because the anchors map was out of scope). Retreat is now
                // ordinary authored doctrine — `directive_kind = "Retreat"` with
                // a real `directive_anchor`, a real `base_priority`, and a
                // `hull_below` zero-gate — which a designer can tune per hull in
                // TOML and which scores on the right scale.
                anchors.get(anchor.as_str()).and_then(|&pos| {
                    helm_navigate_to(
                        world_view,
                        pos,
                        waypoint_arrival_radius,
                        avoidance_buffer,
                        avoidance_look_ahead_secs,
                        forward_speed,
                        target_speed,
                    )
                })
            }
            _ => None,
        };
        if let Some(result) = result {
            return result;
        }
    }

    // Fall through to the Channel-3 Navigation handoff (issues #681, #702).
    // When no Helm-relevant objective resolved and Navigation has set a
    // waypoint, travel to it. This lets a Navigation AI guide a short-range
    // Helm toward an objective the Helm cannot yet see.
    //
    // `nav_waypoint` is `Some` only when the caller has confirmed this ship's
    // `HelmWaypointClearance` matches the waypoint's `generation` — that is
    // where the Channel-3 lag lives now. There is nothing to clear on arrival
    // or staleness: the waypoint belongs to Navigation, and a Helm that
    // resolved a local objective simply never consults it.
    if let Some([nx, nz]) = nav_waypoint {
        if let Some(result) = helm_navigate_to(
            world_view,
            [nx, 0.0, nz],
            waypoint_arrival_radius,
            avoidance_buffer,
            avoidance_look_ahead_secs,
            forward_speed,
            nav_handoff_speed,
        ) {
            return result;
        }
    }

    (0.0, 0.0)
}

/// Helm execute: pursue the target Tactical has selected.
///
/// # The Helm does not acquire (issue #702)
///
/// `weapons_target` is the ship's `TacticalRadarSelection` — the one authoritative lock,
/// written by whoever operates Tactical: a human via `SetTarget`, or
/// `ai_target_selection`. The Helm reads that selection and closes on it; it
/// never picks a target of its own.
///
/// This deleted a real bug rather than merely moving code. The Helm used to
/// acquire independently (`resolve_destroy_target`: explicit → current →
/// last_attacker → nearest hostile) against its *own* radar horizon, while
/// `ai_target_selection` ran the identical four tiers against Tactical's — 187.5
/// vs 75 on alliance hulls. Two selectors over two horizons could disagree, so a
/// ship would close on A while shooting B. One selector, one surface, no
/// divergence.
///
/// Returns `None` when the target is not in `world_view.entities`, i.e. Tactical
/// has locked something this ship's *helm* radar cannot see — the caller then
/// falls through to a lower-priority directive rather than flying blind at a
/// bearing it cannot confirm.
fn helm_destroy(
    world_view: &WorldView,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    weapons_target: Option<Uuid>,
    target_speed: f32,
    maintain_range: f32,
) -> Option<(f32, f32)> {
    let target_uuid = weapons_target?;

    let target_entity = world_view.entities.iter().find(|e| e.uuid == target_uuid)?;

    let pos = world_view.entity_pos;
    let target_pos = target_entity.position;
    let dx = target_pos[0] - pos[0];
    let dz = target_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    if dist < 1.0 {
        return Some((0.0, 0.0));
    }

    let effective_range = world_view.entity_weapons_range.unwrap_or(maintain_range);
    let stop_dist = effective_range * 0.8;
    let at_station = dist <= stop_dist;

    // When holding station, steer to face the target so the phaser forward-arc
    // gate passes. When approaching, steer toward the offset approach point.
    let dir = if at_station {
        [dx / dist, dz / dist]
    } else {
        let approach_target = offset_approach_target(pos, target_pos, stop_dist);
        let nav_dx = approach_target[0] - pos[0];
        let nav_dz = approach_target[2] - pos[2];
        let nav_dist = (nav_dx * nav_dx + nav_dz * nav_dz).sqrt();
        if nav_dist > 0.1 {
            [nav_dx / nav_dist, nav_dz / nav_dist]
        } else {
            [dx / dist, dz / dist]
        }
    };

    let avoidance = avoidance_steering(
        pos,
        world_view.entity_yaw,
        forward_speed,
        world_view.self_radius,
        target_uuid,
        &world_view.entities,
        avoidance_buffer,
        avoidance_look_ahead_secs,
    );

    let base_steer = steer_toward(
        world_view.entity_yaw,
        dir,
        PATROL_DEADBAND_RAD,
        PATROL_FULL_STEER_RAD,
    );
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);

    // Proportional approach: ramp thrust down as the ship enters the decel
    // zone (APPROACH_DECEL_FACTOR × stop_dist) so it arrives at stop_dist
    // with near-zero speed, preventing the overshoot oscillation that causes
    // juddery movement near the target.
    let thrust = if at_station {
        0.0
    } else {
        let decel_start = stop_dist * APPROACH_DECEL_FACTOR;
        if dist < decel_start {
            let t = (dist - stop_dist) / (decel_start - stop_dist);
            target_speed * t.clamp(0.0, 1.0)
        } else {
            target_speed
        }
    };
    Some((thrust, steering))
}

/// Resolve an authored objective target against the AI world view.
///
/// The target may be the entity UUID string itself or the scenario/display name
/// carried on `AiWorldEntity::name`.
pub fn resolve_objective_target(target: &str, world_view: &WorldView) -> Option<Uuid> {
    if target.is_empty() {
        return None;
    }
    world_view
        .entities
        .iter()
        .find(|e| e.uuid.to_string() == target || e.name.as_deref() == Some(target))
        .map(|e| e.uuid)
}

/// Find the nearest entity that is hostile to this AI's faction.
///
/// The single definition of "who is the enemy, and which one is closest". Its
/// one caller is the weapons path — `ai_target_selection`'s nearest-hostile
/// tier (issue #703).
///
/// The helm path was the second caller until #702 deleted its acquisition
/// outright: the Helm now reads the `TacticalRadarSelection` that `ai_target_selection`
/// writes instead of running its own tiers over its own radar horizon. That
/// closed the divergence this function's "both must agree" note used to warn
/// about — there is now one selector rather than two obliged to agree. Nothing
/// on the helm path may grow a hostile scan back; that is the bug, not the fix.
///
/// Distance is measured in the XZ plane (see [`dist_sq`]), matching the
/// range checks the caller gates on.
pub fn find_nearest_hostile(
    world_view: &WorldView,
    faction_registry: &crate::faction::FactionRegistry,
) -> Option<Uuid> {
    let self_faction = world_view.self_faction?;
    let pos = world_view.entity_pos;
    world_view
        .entities
        .iter()
        .filter(|e| {
            e.faction
                .map(|ef| crate::faction::is_enemy(Some(self_faction), Some(ef), faction_registry))
                .unwrap_or(false)
        })
        .min_by(|a, b| {
            let da = dist_sq(pos, a.position);
            let db = dist_sq(pos, b.position);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|e| e.uuid)
}

fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    dx * dx + dz * dz
}

/// Helm execute: steer toward the waypoint this objective's cursor names.
///
/// # Read-only (issue #702)
///
/// `index` comes from the ship's `ObjectiveCursors` entry for this objective and
/// is never written here. `advance_objective_cursors` (`SimSet::Modifiers`) is
/// the sole writer of every cursor, for every ship, at every LOD — which is what
/// makes it impossible to advance a cursor twice in one tick. This function used
/// to keep its own `AiMemory.waypoint_index`, a *second* high-LOD cursor that
/// duplicated the low-LOD `ObjectiveCursors` one; the two are now one surface.
///
/// Advancement consequently lands one tick later than it did. That is benign:
/// the arrival tick already returns zero steering (below), so the tick before
/// the cursor moves is a tick the ship flies straight — exactly what it did
/// when this function advanced the index itself and returned the same
/// `(target_speed, 0.0)`.
#[allow(clippy::too_many_arguments)]
fn helm_patrol(
    world_view: &WorldView,
    waypoints: &[String],
    loop_path: bool,
    index: usize,
    waypoint_arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    target_speed: f32,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
) -> Option<(f32, f32)> {
    // No route at all — not a Patrol this Helm can serve. Fall through.
    if waypoints.is_empty() {
        return None;
    }

    // Terminal stop: a non-looping route walked past its final waypoint. This
    // is resolved idleness — hold station rather than falling through to a
    // lower-priority directive.
    if index >= waypoints.len() && !loop_path {
        return Some((0.0, 0.0));
    }

    // `cursor_target` owns index resolution (wraparound for looping routes) and
    // anchor lookup. `None` here means the current waypoint's anchor is unknown
    // — fall through; the cursor evaluator skips past it this same tick.
    let wp_pos = crate::ai::patrol_cursor::cursor_target(index, waypoints, loop_path, anchors)?;

    let pos = world_view.entity_pos;
    let dx = wp_pos[0] - pos[0];
    let dz = wp_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    // Arrived. Fly straight through; the cursor advances in `Modifiers` later
    // this tick, and the next tick steers toward the new waypoint.
    if dist < waypoint_arrival_radius {
        return Some((target_speed, 0.0));
    }

    let dir = [dx / dist, dz / dist];
    let self_uuid = uuid::Uuid::nil();
    let avoidance = avoidance_steering(
        pos,
        world_view.entity_yaw,
        forward_speed,
        world_view.self_radius,
        self_uuid,
        &world_view.entities,
        avoidance_buffer,
        avoidance_look_ahead_secs,
    );
    let base_steer = steer_toward(
        world_view.entity_yaw,
        dir,
        PATROL_DEADBAND_RAD,
        PATROL_FULL_STEER_RAD,
    );
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);
    Some((target_speed, steering))
}

/// Helm execute: navigate to a fixed position (Reach / Retreat / the Channel-3
/// Navigation handoff).
fn helm_navigate_to(
    world_view: &WorldView,
    target_pos: [f32; 3],
    arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    target_speed: f32,
) -> Option<(f32, f32)> {
    let pos = world_view.entity_pos;
    let dx = target_pos[0] - pos[0];
    let dz = target_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    if dist < arrival_radius {
        return Some((0.0, 0.0));
    }

    let dir = [dx / dist, dz / dist];
    let self_uuid = uuid::Uuid::nil();
    let avoidance = avoidance_steering(
        pos,
        world_view.entity_yaw,
        forward_speed,
        world_view.self_radius,
        self_uuid,
        &world_view.entities,
        avoidance_buffer,
        avoidance_look_ahead_secs,
    );
    let base_steer = steer_toward(
        world_view.entity_yaw,
        dir,
        PATROL_DEADBAND_RAD,
        PATROL_FULL_STEER_RAD,
    );
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);

    // Proportional approach: ramp thrust down within APPROACH_DECEL_FACTOR ×
    // arrival_radius so the ship doesn't overshoot and oscillate on arrival.
    let decel_start = arrival_radius * APPROACH_DECEL_FACTOR;
    let thrust = if dist < decel_start {
        let t = (dist - arrival_radius) / (decel_start - arrival_radius);
        target_speed * t.clamp(0.0, 1.0)
    } else {
        target_speed
    };
    Some((thrust, steering))
}

// ── CaptainAi ────────────────────────────────────────────────────────────────

/// Pure AI controller for the Captain console.
///
/// Reads combat timers from the viewscreen blackboard (issue #572) instead of
/// the `RecentCombatActivity` resource; no private AI copy of timing state.
#[derive(Debug, Clone, Default)]
pub struct CaptainAi;

/// How many seconds of recent activity count as "in combat".
const CAPTAIN_COMBAT_WINDOW_SECS: f32 = 10.0;

impl CaptainAi {
    /// Returns `Some(true)` when the ship should be in red alert (damage taken
    /// or weapon fired within the last 10 seconds), `Some(false)` otherwise.
    ///
    /// `now` is the current simulation elapsed time in seconds. The
    /// `last_damage_taken_secs` and `last_weapon_fired_secs` values are absolute
    /// elapsed-second timestamps (read from the viewscreen blackboard).
    pub fn operate(
        &self,
        now: f32,
        last_damage_taken_secs: Option<f32>,
        last_weapon_fired_secs: Option<f32>,
    ) -> Option<bool> {
        let damage_recent =
            last_damage_taken_secs.is_some_and(|s| now - s < CAPTAIN_COMBAT_WINDOW_SECS);
        let weapon_recent =
            last_weapon_fired_secs.is_some_and(|s| now - s < CAPTAIN_COMBAT_WINDOW_SECS);
        Some(damage_recent || weapon_recent)
    }

    /// No-op stub — channel-3 coordination not yet implemented for captain.
    pub fn coordinate(&self) {}
}

// ── Shared desired-motion contract (issue #741) ───────────────────────────────
//
// The shared motion planner (`helm_motion_planner`, `src/ship/helm_planner.rs`)
// turns a ship's objective travel decision into a 3D desired-motion contract —
// a desired velocity and a desired *facing*, kept separate so orientation can
// diverge from travel (arc-bearing, docking) — plus a hazard assessment. These
// pure helpers are the lossless codec between the per-axis actuator scalars a
// human or the fine helm AI would set (`thrust`/`steering`, each `[-1, 1]`) and
// that ship-local 3D contract. Keeping the contract 3D now (a `[f32; 3]` local
// vector) avoids baking planar assumptions in before bounded / full-3D craft
// arrive; the vertical axis stays 0 while physics is planar.

/// Encode a normalized forward-thrust intent (`[-1, 1]`) as a ship-local
/// desired-velocity vector. Local forward is `-Z`; `vertical` is the local `+Y`
/// (up) component, always 0 in the planar tracer but carried so bounded /
/// full-3D craft can fill it later.
pub fn encode_local_velocity(thrust: f32, vertical: f32) -> [f32; 3] {
    [0.0, vertical, -thrust.clamp(-1.0, 1.0)]
}

/// Recover the forward-thrust intent (`[-1, 1]`) from a ship-local desired
/// velocity. Exact inverse of [`encode_local_velocity`]'s forward axis.
pub fn decode_thrust_from_velocity(velocity_local: [f32; 3]) -> f32 {
    (-velocity_local[2]).clamp(-1.0, 1.0)
}

/// Encode a yaw-steering intent (`[-1, 1]`) as a ship-local desired-facing unit
/// vector. Steering is proportional to yaw error via [`PATROL_FULL_STEER_RAD`],
/// so the desired facing is that error rotated off local forward (`-Z`) toward
/// starboard (`+X`).
pub fn encode_local_facing(steering: f32) -> [f32; 3] {
    let theta = steering.clamp(-1.0, 1.0) * PATROL_FULL_STEER_RAD;
    [theta.sin(), 0.0, -theta.cos()]
}

/// Recover the yaw-steering intent (`[-1, 1]`) from a ship-local desired facing.
/// Inverse of [`encode_local_facing`]; sign-preserving and exact up to floating
/// point for the representable `[-1, 1]` range.
pub fn decode_steering_from_facing(facing_local: [f32; 3]) -> f32 {
    (facing_local[0].atan2(-facing_local[2]) / PATROL_FULL_STEER_RAD).clamp(-1.0, 1.0)
}

/// Close-quarters docking manoeuvre (issue #742).
///
/// Given the ship's world pose and a dock target's world position, returns a
/// ship-local *translation* velocity intent `[starboard, aft]` — both in
/// `[-1, 1]` — that slides the hull straight onto the dock. This is the
/// sanctioned home for the two motions arc-bearing must never command:
/// `starboard != 0` is lateral translation, and `aft > 0` is controlled
/// reverse (the dock is behind the current facing). Facing is left untouched:
/// docking translates, it does not turn, so a ship can back into a berth
/// without spinning to point at it.
///
/// Returns `None` when the dock is farther than `engage_distance` (the ship is
/// still in normal objective approach, not yet at close-manoeuvre range) or
/// coincident with the ship (nothing to translate toward). `approach_speed`
/// caps the intent magnitude so close manoeuvres stay low-speed; both tunables
/// are authored per hull in `[behaviour]` TOML.
///
/// The ship-local transform matches [`crate::weapons::phaser::ship_local`]:
/// `starboard = dx·cos + dz·sin`, forward (`+` = ahead) `= dx·sin − dz·cos`,
/// with forward travel along local `-Z` so an ahead dock yields `aft < 0`.
pub fn docking_close_manoeuvre(
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    dock_x: f32,
    dock_z: f32,
    engage_distance: f32,
    approach_speed: f32,
) -> Option<[f32; 2]> {
    let dx = dock_x - ship_x;
    let dz = dock_z - ship_z;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist > engage_distance || dist < 1e-3 {
        return None;
    }
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    let starboard = dx * cos_y + dz * sin_y;
    let forward = dx * sin_y - dz * cos_y;
    let inv = 1.0 / dist;
    let speed = approach_speed.clamp(0.0, 1.0);
    let lateral = (starboard * inv * speed).clamp(-1.0, 1.0);
    // Forward component points to the dock; a dock behind the hull (forward < 0)
    // becomes positive `aft`, i.e. controlled reverse.
    let aft = (-forward * inv * speed).clamp(-1.0, 1.0);
    Some([lateral, aft])
}

/// One hazard's contribution to a [`HazardAssessmentRaw`]: the entity's
/// published facts alongside the ship-local repulsion force it added and the
/// threat fraction it registered (issue #743).
///
/// Recorded per contributing hazard so a fine actuator — or a test — can see
/// *which* hazards drove the aggregate force and reason about them by fact
/// (movable / dangerous / size) rather than re-deriving the geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct HazardContribution {
    /// The contributing hazard's UUID.
    pub uuid: Uuid,
    /// Whether the hazard can move under its own power (a ship) versus a static
    /// obstacle (an asteroid).
    pub movable: bool,
    /// Whether the hazard is a collision danger. Always `true` here — a
    /// non-dangerous entity never contributes — but carried so consumers read
    /// facts, not categories.
    pub dangerous: bool,
    /// The hazard's authored size rating.
    pub size_rating: f32,
    /// This hazard's ship-local repulsion contribution (`x` = starboard,
    /// `y` = up, `z` = aft).
    pub force_local: [f32; 3],
    /// This hazard's threat fraction `[0, 1]` (`1 - dist / avoidance_radius`).
    pub threat_fraction: f32,
}

/// A ship-level hazard assessment: a repulsion force (ship-local), the peak
/// avoidance urgency `[0, 1]`, the identity of the strongest threat, and the
/// per-hazard force contributions that produced them.
///
/// Computed centrally for the whole ship rather than re-derived inside each fine
/// helm operator (issue #741). It is a *published* boids-style contribution —
/// consumers may weight it by their own sensitivity — not a direct actuator
/// order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HazardAssessmentRaw {
    /// Aggregate repulsion in the ship's local frame (`x` = starboard,
    /// `y` = up, `z` = aft). Points away from projected collisions.
    pub forces_local: [f32; 3],
    /// Peak threat fraction across all hazards, `[0, 1]`.
    pub urgency: f32,
    /// The strongest threat's UUID, if any.
    pub primary: Option<Uuid>,
    /// Per-hazard force contributions, one per hazard that registered a threat
    /// (issue #743). Exposes each contributor's movable / dangerous / size-rating
    /// facts alongside the force it added.
    pub contributions: Vec<HazardContribution>,
}

/// Assess collision hazards for one ship over its visible world view, using the
/// same forward-projection model as [`avoidance_steering`]. Returns a ship-local
/// repulsion vector, the peak urgency, the primary hazard, and the per-hazard
/// force contributions.
///
/// Two authored policies filter the hazard picture (issue #743), applied to the
/// published facts rather than hard-coded object categories:
/// - a non-`dangerous` entity is never a hazard and is skipped;
/// - the ignore-smaller rule skips a hazard whose `size_rating` is below
///   `self_size_rating * hazard_ignore_size_ratio` — a ratio of `0.0` disables
///   the rule (every dangerous hazard is assessed).
///
/// Pure: no ECS, no Bevy. The planner converts the local force array to the
/// engine's vector type.
pub fn assess_hazards(
    world_view: &WorldView,
    forward_speed: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    hazard_ignore_size_ratio: f32,
) -> HazardAssessmentRaw {
    let self_pos = world_view.entity_pos;
    let self_yaw = world_view.entity_yaw;
    let self_radius = world_view.self_radius;
    let self_size_rating = world_view.self_size_rating;

    let fwd_x = self_yaw.sin();
    let fwd_z = -self_yaw.cos();
    // Starboard (local +X): forward rotated +90° in the XZ plane.
    let stbd_x = -fwd_z;
    let stbd_z = fwd_x;

    let proj_self_x = self_pos[0] + fwd_x * forward_speed * avoidance_look_ahead_secs;
    let proj_self_z = self_pos[2] + fwd_z * forward_speed * avoidance_look_ahead_secs;

    let mut force = [0.0_f32; 3];
    let mut urgency = 0.0_f32;
    let mut primary = None;
    let mut contributions: Vec<HazardContribution> = Vec::new();

    for entity in &world_view.entities {
        // Fact-driven policy, not category branches (issue #743): skip anything
        // published as not-dangerous, and apply the authored ignore-smaller rule
        // against the entity's size rating.
        if !entity.dangerous {
            continue;
        }
        if hazard_ignore_size_ratio > 0.0
            && entity.size_rating < self_size_rating * hazard_ignore_size_ratio
        {
            continue;
        }
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;
        let (ent_proj_x, ent_proj_z) = if let Some(ent_yaw) = entity.yaw {
            (
                entity.position[0]
                    + ent_yaw.sin() * entity.forward_speed * avoidance_look_ahead_secs,
                entity.position[2]
                    + (-ent_yaw.cos()) * entity.forward_speed * avoidance_look_ahead_secs,
            )
        } else {
            (entity.position[0], entity.position[2])
        };

        let ddx = proj_self_x - ent_proj_x;
        let ddz = proj_self_z - ent_proj_z;
        let dist = (ddx * ddx + ddz * ddz).sqrt();
        if dist < avoidance_radius && dist > 0.01 {
            let threat_fraction = 1.0 - (dist / avoidance_radius);
            // World-space repulsion: away from the threat, scaled by severity.
            let inv = threat_fraction / dist;
            let rx = ddx * inv;
            let rz = ddz * inv;
            // Rotate into the ship-local frame (x = starboard, z = aft).
            let contribution = [rx * stbd_x + rz * stbd_z, 0.0, -(rx * fwd_x + rz * fwd_z)];
            force[0] += contribution[0];
            force[2] += contribution[2];
            contributions.push(HazardContribution {
                uuid: entity.uuid,
                movable: entity.movable,
                dangerous: entity.dangerous,
                size_rating: entity.size_rating,
                force_local: contribution,
                threat_fraction,
            });
            if threat_fraction > urgency {
                urgency = threat_fraction;
                primary = Some(entity.uuid);
            }
        }
    }

    HazardAssessmentRaw {
        forces_local: force,
        urgency: urgency.clamp(0.0, 1.0),
        primary,
        contributions,
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// No `ObjectiveCursors` entries — the right input for every test that is
    /// not about patrol routes. `helm_patrol` treats a missing cursor as index
    /// 0, so this is "the ship is at the start of any route it has".
    const NO_CURSORS: &[crate::ai::patrol_cursor::PatrolCursor] = &[];

    // ── steer_toward ──────────────────────────────────────────────────────

    #[test]
    fn steer_toward_returns_zero_within_deadband() {
        // Forward yaw = 0 → forward direction = (0, -1) in XZ.
        // Target slightly ahead → nearly zero error, within deadband.
        let result = steer_toward(0.0, [0.0, -1.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn steer_toward_positive_for_target_to_right() {
        // Yaw = 0, forward = (0, -1). Target at (1, 0) = to the right.
        let dir = [1.0_f32, 0.0_f32];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        let unit = [dir[0] / len, dir[1] / len];
        let result = steer_toward(0.0, unit, 0.0, PATROL_FULL_STEER_RAD);
        assert!(
            result > 0.0,
            "target to the right must give positive steering"
        );
    }

    #[test]
    fn steer_toward_negative_for_target_to_left() {
        let dir = [-1.0_f32, 0.0_f32];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        let unit = [dir[0] / len, dir[1] / len];
        let result = steer_toward(0.0, unit, 0.0, PATROL_FULL_STEER_RAD);
        assert!(
            result < 0.0,
            "target to the left must give negative steering"
        );
    }

    #[test]
    fn steer_toward_saturates_at_one() {
        // Target directly to the right (90°) saturates.
        let result = steer_toward(0.0, [1.0, 0.0], 0.0, PATROL_FULL_STEER_RAD);
        assert!(result >= 1.0 || result <= -1.0 || result.abs() <= 1.0);
        assert!(result.abs() <= 1.0, "steering must be clamped to [-1, 1]");
    }

    // ── should_emit ───────────────────────────────────────────────────────

    #[test]
    fn should_emit_returns_false_when_within_epsilon() {
        assert!(!should_emit(0.5, 0.5 + 0.001, 0.01));
    }

    #[test]
    fn should_emit_returns_true_when_outside_epsilon() {
        assert!(should_emit(0.0, 0.5, 0.01));
    }

    #[test]
    fn should_emit_returns_false_when_equal() {
        assert!(!should_emit(0.3, 0.3, 0.0));
    }

    // ── visible_entities ──────────────────────────────────────────────────

    #[test]
    fn visible_entities_includes_in_range_entity() {
        let near = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [10.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], 20.0, std::slice::from_ref(&near));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uuid, near.uuid);
    }

    #[test]
    fn visible_entities_excludes_out_of_range_entity() {
        let far = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [100.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[far]);
        assert!(result.is_empty());
    }

    #[test]
    fn visible_entities_includes_entity_exactly_at_boundary() {
        let boundary = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [20.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[boundary]);
        assert_eq!(result.len(), 1, "entity exactly at range must be included");
    }

    #[test]
    fn visible_entities_ignores_y_component() {
        // Same XZ position, wildly different Y — should still be in range.
        let above = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [5.0, 500.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[above]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn visible_entities_unlimited_when_range_zero() {
        let far = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [10_000.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], 0.0, &[far]);
        assert_eq!(result.len(), 1, "range <= 0 must mean unlimited");
    }

    #[test]
    fn visible_entities_unlimited_when_range_negative() {
        let far = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [10_000.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], -5.0, &[far]);
        assert_eq!(result.len(), 1, "negative range must mean unlimited");
    }

    #[test]
    fn visible_entities_unlimited_when_range_nan() {
        let far = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [10_000.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], f32::NAN, &[far]);
        assert_eq!(result.len(), 1, "NaN range must mean unlimited");
    }

    #[test]
    fn visible_entities_unlimited_when_range_infinite() {
        let far = AiWorldEntity {
            uuid: Uuid::from_u128(1),
            position: [10_000.0, 0.0, 0.0],
            ..Default::default()
        };
        let result = visible_entities([0.0, 0.0, 0.0], f32::INFINITY, &[far]);
        assert_eq!(result.len(), 1, "infinite range must mean unlimited");
    }

    #[test]
    fn avoidance_steering_is_zero_when_stationary() {
        let obstacle = AiWorldEntity {
            uuid: Uuid::from_u128(2),
            position: [0.0, 0.0, -2.0],
            radius: 20.0,
            ..Default::default()
        };

        let steering = avoidance_steering(
            [0.0, 0.0, 0.0],
            0.0,
            0.0,
            2.0,
            Uuid::nil(),
            &[obstacle],
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
        );

        assert_eq!(
            steering, 0.0,
            "stationary ships should not yaw away from nearby bodies"
        );
    }

    // ── operate_helm patrol ───────────────────────────────────────────────

    fn patrol_pool() -> Vec<crate::messages::ScoredObjective> {
        vec![crate::messages::ScoredObjective {
            id: "patrol".into(),
            score: 20.0,
            directive: crate::messages::AiDirective::Patrol {
                anchors: vec!["alpha".into()],
                loop_path: true,
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "patrol".into(),
                text: "Patrol".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    fn patrol_doctrine() -> Vec<crate::entity_config::DoctrineObjective> {
        vec![crate::entity_config::DoctrineObjective {
            id: "patrol".into(),
            text: "Patrol".into(),
            directive_kind: Some("Patrol".into()),
            directive_anchors: vec!["alpha".into()],
            directive_loop: true,
            base_priority: 20.0,
            target_speed: 0.5,
            ..Default::default()
        }]
    }

    /// A two-waypoint, non-looping Patrol over `wp0` then `wp1`, with matching
    /// doctrine (`target_speed` 0.5). The caller supplies the anchor positions,
    /// so the same route can be posed as "arrived", "en route" or "terminal".
    fn two_waypoint_patrol() -> (
        Vec<crate::messages::ScoredObjective>,
        Vec<crate::entity_config::DoctrineObjective>,
    ) {
        let pool = vec![crate::messages::ScoredObjective {
            id: "patrol".into(),
            score: 20.0,
            directive: crate::messages::AiDirective::Patrol {
                anchors: vec!["wp0".into(), "wp1".into()],
                loop_path: false,
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "patrol".into(),
                text: "".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }];
        let doctrine = vec![crate::entity_config::DoctrineObjective {
            id: "patrol".into(),
            text: "".into(),
            directive_kind: Some("Patrol".into()),
            directive_anchors: vec!["wp0".into(), "wp1".into()],
            directive_loop: false,
            target_speed: 0.5,
            ..Default::default()
        }];
        (pool, doctrine)
    }

    /// A single Reach objective naming `anchor`, scored `score`.
    fn reach_pool(anchor: &str, score: f32) -> Vec<crate::messages::ScoredObjective> {
        vec![crate::messages::ScoredObjective {
            id: "reach".into(),
            score,
            directive: crate::messages::AiDirective::Reach {
                anchor: anchor.into(),
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "reach".into(),
                text: "".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    fn anchors_with_alpha() -> std::collections::HashMap<String, [f32; 3]> {
        let mut m = std::collections::HashMap::new();
        m.insert("alpha".into(), [100.0, 0.0, 0.0]);
        m
    }

    fn world_at_origin() -> WorldView {
        WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn operate_helm_patrol_generates_nonzero_steering_toward_waypoint() {
        let world = world_at_origin();
        let pool = patrol_pool();
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha();

        let (thrust, _steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(thrust > 0.0, "should thrust toward waypoint");
    }

    #[test]
    fn operate_helm_empty_pool_returns_zero() {
        let world = world_at_origin();
        let (thrust, steering) = plan_helm_travel(
            &world,
            &[],
            &[],
            &std::collections::HashMap::new(),
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(thrust, 0.0);
        assert_eq!(steering, 0.0);
    }

    #[test]
    fn operate_helm_zeroed_pool_returns_zero() {
        let world = world_at_origin();
        let mut pool = patrol_pool();
        pool[0].score = 0.0; // zero-gated
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(thrust, 0.0, "zero-gated pool must produce no thrust");
        assert_eq!(steering, 0.0);
    }

    /// `helm_patrol` steers toward the waypoint the *cursor* names, not
    /// blindly toward the route's first anchor (issue #702).
    ///
    /// This is the core of the `waypoint_index` migration. `operate_helm` used
    /// to own a private `AiMemory.waypoint_index` and advance it itself, giving
    /// the high-LOD helm a second cursor that could disagree with the
    /// `ObjectiveCursors` the low-LOD path and the scenario triggers used.
    /// There is now one cursor and the helm reads it.
    #[test]
    fn operate_helm_patrol_steers_to_the_cursors_waypoint() {
        let mut anchors = std::collections::HashMap::new();
        // wp0 is dead ahead (negative Z at yaw 0); wp1 is to starboard.
        anchors.insert("wp0".into(), [0.0, 0.0, -100.0]);
        anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
        let (pool, doctrine) = two_waypoint_patrol();
        let world = world_at_origin();

        // A cursor sitting on index 1 must produce a turn to starboard.
        let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol");
        crate::ai::patrol_cursor::advance_cursor(
            &mut cursor,
            &["wp0".to_string(), "wp1".to_string()],
            false,
            [0.0, 0.0, -100.0], // sitting on wp0 → the cursor advances to wp1
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
        );
        assert_eq!(cursor.index(), 1, "precondition: cursor must be on wp1");

        let (_thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            std::slice::from_ref(&cursor),
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            steering > 0.0,
            "the helm must steer toward the cursor's waypoint (wp1, to starboard),              not the route's first anchor (wp0, dead ahead, which would steer ~0);              got steering={steering}"
        );
    }

    /// On the arrival tick the helm flies straight through rather than
    /// advancing anything itself — `advance_objective_cursors`
    /// (`SimSet::Modifiers`) owns advancement, and lands it later the same
    /// tick. This is why moving the cursor out of `operate_helm` is benign:
    /// the tick the cursor moves was already a zero-steering tick.
    #[test]
    fn operate_helm_patrol_flies_straight_through_on_arrival() {
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("wp0".into(), [0.0, 0.0, 0.0]); // at origin = arrived
        anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
        let (pool, doctrine) = two_waypoint_patrol();
        let world = world_at_origin();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(
            (thrust, steering),
            (0.5, 0.0),
            "arrival tick must hold course at the doctrine's target_speed"
        );
    }

    /// A non-looping route walked past its final waypoint is resolved
    /// idleness — hold station rather than falling through to a lower-priority
    /// directive.
    #[test]
    fn operate_helm_patrol_terminal_stop_holds_station() {
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("wp0".into(), [0.0, 0.0, -100.0]);
        anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
        let (mut pool, doctrine) = two_waypoint_patrol();
        // Add a resolvable lower-priority Reach; a terminal Patrol must not
        // fall through to it.
        pool.extend(reach_pool("wp1", 1.0));
        let world = world_at_origin();

        let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol");
        // Walk the cursor off the end of the non-looping route.
        for pos in [[0.0, 0.0, -100.0], [100.0, 0.0, 0.0]] {
            crate::ai::patrol_cursor::advance_cursor(
                &mut cursor,
                &["wp0".to_string(), "wp1".to_string()],
                false,
                pos,
                &anchors,
                WAYPOINT_ARRIVAL_RADIUS,
            );
        }
        assert!(cursor.index() >= 2, "precondition: cursor must be terminal");

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            std::slice::from_ref(&cursor),
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(
            (thrust, steering),
            (0.0, 0.0),
            "a finished non-looping patrol holds station; it must not fall              through to the lower-priority Reach"
        );
    }

    // ── operate_helm fallback ─────────────────────────────────────────────

    /// Build a scored pool with Destroy (high score, unresolvable target) first,
    /// then Patrol (lower score, resolvable anchor) second.
    fn destroy_then_patrol_pool(
        anchors: &std::collections::HashMap<String, [f32; 3]>,
    ) -> Vec<crate::messages::ScoredObjective> {
        let _ = anchors; // anchors used externally; pool just carries names
        vec![
            crate::messages::ScoredObjective {
                id: "destroy-wave-1".into(),
                score: 90.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: "wave_1".into(), // entity not in world_view → unresolvable
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![
                    crate::messages::SystemAffinity::Helm,
                    crate::messages::SystemAffinity::Weapons,
                    crate::messages::SystemAffinity::Captain,
                ],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-wave-1".into(),
                    text: "Destroy wave 1".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["wave_1".into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            },
            crate::messages::ScoredObjective {
                id: "patrol-base".into(),
                score: 30.0,
                directive: crate::messages::AiDirective::Patrol {
                    anchors: vec!["alpha".into()],
                    loop_path: true,
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "patrol-base".into(),
                    text: "Patrol".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            },
        ]
    }

    #[test]
    fn operate_helm_falls_through_unresolvable_destroy_to_patrol() {
        // Regression: when the top Destroy directive has no valid target in the
        // world snapshot (entity not yet spawned / not in WorldSnapshot),
        // operate_helm must fall through to the next lower-priority directive
        // (Patrol) rather than leaving the ship idle.  Matches the
        // combat_test.toml scenario where wave objectives are added on the same
        // tick as the entities spawn, before the WorldSnapshot is rebuilt.
        let world = world_at_origin(); // entities list is empty → wave_1 not found
        let anchors = anchors_with_alpha();
        let pool = destroy_then_patrol_pool(&anchors);

        let (thrust, _steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            thrust > 0.0,
            "should fall through to Patrol and produce thrust when Destroy target is unresolvable"
        );
    }

    /// The Helm pursues the ship's `TacticalRadarSelection` and does not fall through to
    /// Patrol (issue #702).
    ///
    /// This is the `target` migration in one test. `operate_helm` used to
    /// resolve the Destroy directive's authored name itself, via a private
    /// four-tier `resolve_destroy_target` (explicit → current → last_attacker →
    /// nearest) over its own radar horizon — the same four tiers
    /// `ai_target_selection` runs over Tactical's. Two selectors, two horizons,
    /// so a ship could close on one ship while shooting another. Now Tactical
    /// selects and the Helm reads the selection.
    ///
    /// Geometry: the Destroy target sits dead ahead and the Patrol anchor to
    /// starboard, so "which directive won" is legible from the steering alone —
    /// ~0 means Destroy, positive means it fell through to Patrol.
    #[test]
    fn operate_helm_destroy_pursues_the_weapons_target() {
        let (world, pool, anchors, target_uuid) = destroy_vs_patrol_scene();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            Some(target_uuid),
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert!(thrust > 0.0, "should close on the target");
        assert!(
            steering.abs() < PATROL_DEADBAND_RAD,
            "the helm must pursue the TacticalRadarSelection (dead ahead → ~0 steering),              not fall through to Patrol (to starboard → positive steering);              got steering={steering}"
        );
    }

    /// The converse, and the reason the Helm may not acquire on its own: with
    /// Tactical holding no lock, a Destroy directive resolves to nobody and
    /// falls through — *even though* a perfectly good hostile is sitting in the
    /// Helm's world view. A Helm that scanned for its own target would pursue
    /// it and diverge from what Tactical is shooting.
    #[test]
    fn operate_helm_destroy_without_a_weapons_target_falls_through() {
        let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None, // Tactical has locked nothing
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert!(
            thrust > 0.0,
            "should fall through to Patrol and keep flying"
        );
        assert!(
            steering > 0.0,
            "with no Tactical lock the Destroy directive resolves to nobody and              must fall through to Patrol (to starboard → positive steering); a              ~0 steering means the helm acquired the visible hostile itself,              which is the divergence #702 removed; got steering={steering}"
        );
    }

    /// The starbase-assault bug. Tactical holds no lock — the target is over the
    /// horizon, or factionless and never auto-acquired — but Navigation *has*
    /// cleared a waypoint to it. The Destroy directive must consume that
    /// waypoint at its own priority, not fall through to the lower-scored
    /// Patrol. Previously Patrol resolved and returned first, so a raider
    /// ordered to assault the starbase flew its patrol circuit instead and the
    /// waypoint fallback at the end of `operate_helm` was never reached.
    #[test]
    fn operate_helm_destroy_without_a_weapons_target_flies_the_nav_waypoint() {
        let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

        // Patrol anchor alpha is at [100, 0, 0] — to starboard. Put Navigation's
        // waypoint to *port* so the two are unambiguously distinguishable.
        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None,                // Tactical has locked nothing
            Some([-100.0, 0.0]), // but Navigation has cleared a waypoint
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert!(thrust > 0.0, "should be under way toward the waypoint");
        assert!(
            steering < 0.0,
            "must steer to port toward Navigation's waypoint; positive steering              means it fell through to the starboard Patrol anchor, which is the              bug — got steering={steering}"
        );
    }

    /// The fallback is conditional, not a takeover: a Destroy that resolves
    /// neither a target nor a waypoint still yields to Patrol.
    #[test]
    fn operate_helm_destroy_still_yields_to_patrol_without_a_nav_waypoint() {
        let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

        let (_thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None,
            None, // no lock and no waypoint
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert!(
            steering > 0.0,
            "with nothing to close on, Destroy must still fall through to the              starboard Patrol anchor; got steering={steering}"
        );
    }

    /// A Tactical lock the Helm's own radar cannot see is not pursuable: the
    /// directive falls through rather than flying at a bearing it cannot
    /// confirm. (`world_view.entities` is already radar-filtered by the caller.)
    #[test]
    fn operate_helm_destroy_ignores_a_target_outside_its_world_view() {
        let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

        let (_thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            Some(Uuid::new_v4()), // locked, but not in the world view
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            steering > 0.0,
            "an invisible target must fall through to Patrol, not steer at a              bearing the helm cannot confirm; got steering={steering}"
        );
    }

    /// A scene where Destroy and Patrol are told apart by steering alone:
    /// the Destroy target is dead ahead (yaw 0 → forward is -Z), the Patrol
    /// anchor `alpha` is to starboard. Destroy outscores Patrol.
    ///
    /// Returns `(world, pool, anchors, target_uuid)`.
    #[allow(clippy::type_complexity)]
    fn destroy_vs_patrol_scene() -> (
        WorldView,
        Vec<crate::messages::ScoredObjective>,
        std::collections::HashMap<String, [f32; 3]>,
        Uuid,
    ) {
        let target_uuid = Uuid::new_v4();
        let anchors = anchors_with_alpha(); // alpha = [100, 0, 0], to starboard
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![crate::ai::AiWorldEntity {
                uuid: target_uuid,
                name: Some("wave_1".into()),
                position: [0.0, 0.0, -200.0], // dead ahead
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut pool = vec![crate::messages::ScoredObjective {
            id: "destroy-wave-1".into(),
            score: 90.0,
            directive: crate::messages::AiDirective::Destroy {
                target: "wave_1".into(),
            },
            source: crate::messages::ObjectiveSource::Mission,
            relevance: vec![
                crate::messages::SystemAffinity::Helm,
                crate::messages::SystemAffinity::Weapons,
                crate::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "destroy-wave-1".into(),
                text: "Destroy".into(),
                mandatory: true,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Mission,
            },
        }];
        pool.extend(patrol_pool()); // score 20 — below Destroy
        (world, pool, anchors, target_uuid)
    }

    // ── operate_helm Retreat ──────────────────────────────────────────────

    /// Build a single-objective Retreat scored pool naming `anchor`.
    fn retreat_pool(anchor: &str, score: f32) -> Vec<crate::messages::ScoredObjective> {
        vec![crate::messages::ScoredObjective {
            id: "retreat".into(),
            score,
            directive: crate::messages::AiDirective::Retreat {
                anchor: anchor.into(),
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "retreat".into(),
                text: "Retreat".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    #[test]
    fn operate_helm_retreat_steers_toward_valid_anchor() {
        // A Retreat directive with a known anchor name must steer toward that
        // anchor, mirroring the Reach directive. Anchor "rally" is at
        // [100, 0, 0] — to the right of a ship at origin facing yaw 0
        // (forward = (0, -1)), so steering must be positive (see
        // steer_toward_positive_for_target_to_right).
        let world = world_at_origin();
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("rally".to_string(), [100.0, 0.0, 0.0]);
        let pool = retreat_pool("rally", 50.0);

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(thrust > 0.0, "Retreat should thrust toward the anchor");
        assert!(
            steering > 0.0,
            "Retreat anchor to the right must give positive steering"
        );
    }

    /// The other side of `operate_helm_retreat_steers_toward_valid_anchor`: a
    /// Retreat naming an anchor the world does not declare resolves to nowhere
    /// and falls through to the next directive, exactly as `Reach` does.
    ///
    /// This test used to assert the opposite — that an empty anchor fell back to
    /// `AiMemory.home_position` — because a *synthetic* hull-triggered Retreat
    /// injected by `aggregate_doctrine_blackboards` always carried an empty
    /// anchor and needed somewhere to go. #702 deleted that injector and the
    /// `home_position` it leaned on. The old fallback was never the safety net
    /// it looked like: `home_position` was never seeded in production, so it was
    /// world origin, and "retreat" meant "fly to [0,0,0]" for every shipped
    /// ship. Falling through to Patrol is both the honest answer and the useful
    /// one. Retreat is now authored doctrine with a real anchor.
    #[test]
    fn operate_helm_retreat_with_unknown_anchor_falls_through() {
        let world = world_at_origin();
        // "alpha" is known and Patrol wants it; the Retreat anchor is not.
        let anchors = anchors_with_alpha();
        let mut pool = retreat_pool("nowhere-in-particular", 50.0);
        pool.extend(patrol_pool());
        let doctrine = patrol_doctrine();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        // Patrol's "alpha" is at [100, 0, 0] — to the right of a ship at origin
        // at yaw 0, so a positive steering means we are flying the *Patrol*, not
        // idling and not retreating to a phantom origin.
        assert!(
            thrust > 0.0 && steering > 0.0,
            "an unresolvable Retreat must fall through to the next directive              (here Patrol toward `alpha`), not resolve to a fabricated position;              got thrust={thrust}, steering={steering}"
        );
    }

    /// And with nothing to fall through *to*, an unresolvable Retreat is idle —
    /// not a flight to world origin.
    #[test]
    fn operate_helm_lone_unresolvable_retreat_is_idle() {
        let world = world_at_origin();
        let anchors = std::collections::HashMap::new();
        let pool = retreat_pool("", 50.0);

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &anchors,
            NO_CURSORS,
            None,
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(
            (thrust, steering),
            (0.0, 0.0),
            "a Retreat that names nowhere is a Retreat to nowhere"
        );
    }

    // ── helm_destroy proportional approach ────────────────────────────────

    /// Build a minimal Destroy scored pool that targets `uuid` with
    /// `target_speed` and `maintain_range` taken from matching doctrine.
    fn destroy_pool_for(
        target_name: &str,
        target_speed: f32,
        maintain_range: f32,
    ) -> (
        Vec<crate::messages::ScoredObjective>,
        Vec<crate::entity_config::DoctrineObjective>,
    ) {
        let pool = vec![crate::messages::ScoredObjective {
            id: "destroy-target".into(),
            score: 50.0,
            directive: crate::messages::AiDirective::Destroy {
                target: target_name.into(),
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![
                crate::messages::SystemAffinity::Helm,
                crate::messages::SystemAffinity::Weapons,
            ],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "destroy-target".into(),
                text: "".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![target_name.into()],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }];
        let doctrine = vec![crate::entity_config::DoctrineObjective {
            id: "destroy-target".into(),
            text: "".into(),
            directive_kind: Some("Destroy".into()),
            target_speed,
            maintain_range,
            ..Default::default()
        }];
        (pool, doctrine)
    }

    #[test]
    fn helm_destroy_full_thrust_far_from_target() {
        // Ship is far beyond the decel zone — should emit full target_speed thrust.
        let target_uuid = Uuid::new_v4();
        let target_speed = 0.8_f32;
        let maintain_range = 25.0_f32;
        // stop_dist = 25 * 0.8 = 20; decel_start = 20 * 1.5 = 30; place at 100
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy".into()),
                position: [0.0, 0.0, -100.0],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
        let (thrust, _) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            NO_CURSORS,
            Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            (thrust - target_speed).abs() < 1e-4,
            "beyond decel zone: expected full thrust {target_speed}, got {thrust}"
        );
    }

    #[test]
    fn helm_destroy_reduced_thrust_inside_decel_zone() {
        // Ship is halfway between decel_start and stop_dist — thrust should be
        // roughly half of target_speed (proportional ramp).
        let target_uuid = Uuid::new_v4();
        let target_speed = 0.8_f32;
        let maintain_range = 25.0_f32;
        // stop_dist = 20, decel_start = 30; midpoint = 25
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy".into()),
                position: [0.0, 0.0, -25.0], // dist = 25
                ..Default::default()
            }],
            ..Default::default()
        };
        let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
        let (thrust, _) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            NO_CURSORS,
            Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        // At dist=25, t = (25-20)/(30-20) = 0.5 → expected thrust = 0.4
        let expected = target_speed * 0.5;
        assert!(
            (thrust - expected).abs() < 0.01,
            "inside decel zone (midpoint): expected ~{expected}, got {thrust}"
        );
    }

    #[test]
    fn helm_destroy_zero_thrust_at_station() {
        // Ship is inside stop_dist — thrust must be exactly 0.
        let target_uuid = Uuid::new_v4();
        let maintain_range = 25.0_f32;
        // stop_dist = 20; place at 10 (inside)
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy".into()),
                position: [0.0, 0.0, -10.0], // dist = 10
                ..Default::default()
            }],
            ..Default::default()
        };
        let (pool, doctrine) = destroy_pool_for("enemy", 0.8, maintain_range);
        let (thrust, _) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            NO_CURSORS,
            Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(thrust, 0.0, "inside stop_dist: thrust must be 0");
    }

    #[test]
    fn helm_destroy_holding_station_does_not_avoid_destroy_target() {
        // While holding weapons range, the active Destroy target is the thing
        // the ship intentionally faces. Treating that same target as an
        // avoidance obstacle makes a stationary AI ship yaw away from a nearby
        // enemy even when already lined up.
        let target_uuid = Uuid::new_v4();
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy".into()),
                position: [0.0, 0.0, -10.0],
                radius: 20.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let (pool, doctrine) = destroy_pool_for("enemy", 0.8, 25.0);

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            NO_CURSORS,
            Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert_eq!(thrust, 0.0, "inside stop_dist: thrust must be 0");
        assert_eq!(
            steering, 0.0,
            "active destroy target must not push avoidance steering while holding station"
        );
    }

    #[test]
    fn helm_destroy_holding_station_does_not_fall_through_to_patrol() {
        let target_uuid = Uuid::new_v4();
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy".into()),
                position: [0.0, 0.0, -10.0],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (mut pool, doctrine) = destroy_pool_for("enemy", 0.8, 25.0);
        pool.push(crate::messages::ScoredObjective {
            id: "patrol-base".into(),
            score: 10.0,
            directive: crate::messages::AiDirective::Patrol {
                anchors: vec!["alpha".into()],
                loop_path: true,
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![crate::messages::SystemAffinity::Helm],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "patrol-base".into(),
                text: "Patrol".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        });
        let anchors = anchors_with_alpha();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            Some(target_uuid), // Tactical's lock - what the helm pursues (#702)
            None,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );

        assert_eq!(thrust, 0.0);
        assert_eq!(
            steering, 0.0,
            "resolved Destroy should hold station instead of falling through to Patrol"
        );
    }

    // ── CaptainAi ─────────────────────────────────────────────────────────

    #[test]
    fn captain_ai_returns_true_when_damage_within_window() {
        let ai = CaptainAi;
        // now=10, damage at ts=5 → delta=5s < 10s → true
        assert_eq!(ai.operate(10.0, Some(5.0), None), Some(true));
    }

    #[test]
    fn captain_ai_returns_true_when_weapon_fired_within_window() {
        let ai = CaptainAi;
        // now=10, weapon at ts=7 → delta=3s < 10s → true
        assert_eq!(ai.operate(10.0, None, Some(7.0)), Some(true));
    }

    #[test]
    fn captain_ai_returns_false_when_no_activity() {
        let ai = CaptainAi;
        assert_eq!(ai.operate(10.0, None, None), Some(false));
    }

    #[test]
    fn captain_ai_returns_false_when_activity_older_than_window() {
        let ai = CaptainAi;
        // now=20, damage at ts=5 → delta=15s > 10s → false
        // weapon at ts=8 → delta=12s > 10s → false
        assert_eq!(ai.operate(20.0, Some(5.0), Some(8.0)), Some(false));
    }

    #[test]
    fn captain_ai_returns_true_at_window_boundary() {
        let ai = CaptainAi;
        // now=10, damage at ts=0.1 → delta=9.9s < 10s → true
        assert_eq!(ai.operate(10.0, Some(0.1), None), Some(true));
    }

    // ── score_doctrine_pool ───────────────────────────────────────────────

    #[test]
    fn score_doctrine_pool_patrol_always_scores() {
        use crate::entity_config::DoctrineObjective;
        use crate::objectives::WorldConditions;

        let doctrine = vec![DoctrineObjective {
            id: "patrol".into(),
            text: "Patrol".into(),
            directive_kind: Some("Patrol".into()),
            base_priority: 20.0,
            ..Default::default()
        }];
        let cond = WorldConditions {
            red_alert: false,
            hull_fraction: 1.0,
            attacked: false,
        };
        let pool = score_doctrine_pool(&doctrine, &cond);
        assert_eq!(pool.len(), 1);
        assert!((pool[0].score - 20.0).abs() < 1e-5);
    }

    #[test]
    fn score_doctrine_pool_zero_gate_vetoes_destroy() {
        use crate::entity_config::DoctrineObjective;
        use crate::objectives::{WorldConditions, ZeroGateCondition};

        // Zero gate: hull must be below 0.3 (but hull = 1.0 → gate fails → score 0)
        let doctrine = vec![DoctrineObjective {
            id: "flee".into(),
            text: "Flee".into(),
            directive_kind: Some("Reach".into()),
            base_priority: 50.0,
            zero_gates: vec![ZeroGateCondition {
                condition: "hull_below".into(),
                threshold: Some(0.3),
            }],
            ..Default::default()
        }];
        let cond = WorldConditions {
            red_alert: false,
            hull_fraction: 1.0,
            attacked: false,
        };
        let pool = score_doctrine_pool(&doctrine, &cond);
        assert_eq!(pool[0].score, 0.0, "zero-gate must veto at full hull");
    }

    #[test]
    fn score_doctrine_pool_sorted_descending_by_score() {
        use crate::entity_config::DoctrineObjective;
        use crate::objectives::WorldConditions;

        let doctrine = vec![
            DoctrineObjective {
                id: "a".into(),
                text: "A".into(),
                base_priority: 10.0,
                ..Default::default()
            },
            DoctrineObjective {
                id: "b".into(),
                text: "B".into(),
                base_priority: 35.0,
                ..Default::default()
            },
            DoctrineObjective {
                id: "c".into(),
                text: "C".into(),
                base_priority: 20.0,
                ..Default::default()
            },
        ];
        let cond = WorldConditions {
            red_alert: false,
            hull_fraction: 1.0,
            attacked: false,
        };
        let pool = score_doctrine_pool(&doctrine, &cond);
        assert_eq!(pool[0].id, "b");
        assert_eq!(pool[1].id, "c");
        assert_eq!(pool[2].id, "a");
    }

    // ── decide_impulse ────────────────────────────────────────────────────

    fn impulse_input(
        pos: [f32; 2],
        yaw: f32,
        target_pos: [f32; 3],
        phase: crate::impulse::ImpulsePhase,
        engage_dist: f32,
        cancel_dist: f32,
    ) -> ImpulseDecisionInput {
        ImpulseDecisionInput {
            pos,
            yaw,
            target_pos,
            phase,
            engage_distance: engage_dist,
            cancel_distance: cancel_dist,
            angle_tolerance: IMPULSE_ANGLE_TOLERANCE_RAD,
        }
    }

    #[test]
    fn impulse_decide_engage_when_ahead_and_far() {
        // Ship at (0,0) facing -Z (yaw=0), target at (0, 0, -300)
        let input = impulse_input(
            [0.0, 0.0],
            0.0,                // yaw = 0 → facing -Z
            [0.0, 0.0, -300.0], // target 300 units ahead
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
    }

    #[test]
    fn impulse_decide_engage_when_ahead_at_exact_threshold() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -200.0],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
    }

    #[test]
    fn impulse_decide_no_engage_when_too_close() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -150.0],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    #[test]
    fn impulse_decide_no_engage_when_target_not_ahead() {
        // Target at 90 degrees to the right
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [300.0, 0.0, 0.0],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    #[test]
    fn impulse_decide_cancel_when_close_during_charging() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -20.0],
            crate::impulse::ImpulsePhase::Charging,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
    }

    #[test]
    fn impulse_decide_cancel_when_close_during_active() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -20.0],
            crate::impulse::ImpulsePhase::Active,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
    }

    #[test]
    fn impulse_decide_noop_when_idle_and_not_ahead() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [300.0, 0.0, -100.0],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    #[test]
    fn impulse_decide_noop_when_active_and_ahead() {
        // Already active, target is ahead and far — no change needed
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -500.0],
            crate::impulse::ImpulsePhase::Active,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    #[test]
    fn impulse_decide_engage_at_angle_tolerance_boundary() {
        // Target at the edge of the tolerance cone
        let angle = IMPULSE_ANGLE_TOLERANCE_RAD; // 0.08 rad
        let target_x = 300.0 * angle.sin();
        let target_z = -300.0 * angle.cos();
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [target_x, 0.0, target_z],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
    }

    #[test]
    fn impulse_decide_noop_past_angle_tolerance() {
        let angle = IMPULSE_ANGLE_TOLERANCE_RAD + 0.01; // just past boundary
        let target_x = 300.0 * angle.sin();
        let target_z = -300.0 * angle.cos();
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [target_x, 0.0, target_z],
            crate::impulse::ImpulsePhase::Idle,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    #[test]
    fn impulse_decide_cancel_at_cancel_distance_boundary() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -40.0],
            crate::impulse::ImpulsePhase::Active,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
    }

    #[test]
    fn impulse_decide_noop_barely_above_cancel_distance() {
        let input = impulse_input(
            [0.0, 0.0],
            0.0,
            [0.0, 0.0, -41.0],
            crate::impulse::ImpulsePhase::Active,
            200.0,
            40.0,
        );
        assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
    }

    // ── Navigation waypoint handoff (issues #681, #702) ────────────────────
    //
    // `nav_waypoint` is the position of the ship's `NavigationWaypoint`,
    // supplied by the caller only once the Channel-3 clearance matches its
    // generation. It replaced `AiMemory.nav_goal`, a private copy of the same
    // position laundered through the coordination message.
    //
    // These tests lost their "…and clears nav_goal" halves, because there is no
    // longer anything to clear: the waypoint belongs to Navigation, and a Helm
    // that resolves a local objective simply never consults it. The
    // clearing-on-arrival and clearing-on-resolve rules existed only to stop the
    // private copy drifting out of sync with the real waypoint.

    /// With no Helm-relevant objective, the Helm travels to the cleared
    /// Navigation waypoint.
    #[test]
    fn operate_helm_falls_through_to_nav_waypoint_when_no_objective() {
        let world = world_at_origin(); // entities list empty, anchors empty
        let pool: Vec<crate::messages::ScoredObjective> = vec![];

        let (thrust, _steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &std::collections::HashMap::new(),
            NO_CURSORS,
            None,
            Some([100.0, 0.0]),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            thrust > 0.0,
            "the nav-waypoint fallthrough must produce positive thrust"
        );
    }

    /// An *uncleared* waypoint is not followed. The caller passes `None` until
    /// `HelmWaypointClearance` matches the waypoint's generation, so this is
    /// where the Channel-3 lag is visible from `operate_helm`'s side: the Helm
    /// has been given a waypoint but not yet the order to fly it.
    #[test]
    fn operate_helm_ignores_an_uncleared_nav_waypoint() {
        let world = world_at_origin();
        let pool: Vec<crate::messages::ScoredObjective> = vec![];

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &std::collections::HashMap::new(),
            NO_CURSORS,
            None,
            None, // clearance has not caught up with the waypoint's generation
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(
            (thrust, steering),
            (0.0, 0.0),
            "an uncleared waypoint must not be followed - that lag is the whole \
             job of the Channel-3 handoff"
        );
    }

    /// Arriving at the waypoint stops the ship. It does *not* clear the
    /// waypoint: the waypoint is Navigation's, and the Helm holds station on it
    /// rather than reaching into another console's state.
    #[test]
    fn operate_helm_holds_station_on_reaching_the_nav_waypoint() {
        // Ship at origin, waypoint at [0, -1] => dist = 1 < 20 => arrived.
        let world = world_at_origin();
        let pool = vec![];

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &std::collections::HashMap::new(),
            NO_CURSORS,
            None,
            Some([0.0, -1.0]),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert_eq!(
            (thrust, steering),
            (0.0, 0.0),
            "arrived at the nav waypoint must produce zero thrust"
        );
    }

    /// A resolvable local objective outranks the nav waypoint - the ship must
    /// fly the objective, not blend the two bearings.
    #[test]
    fn operate_helm_prefers_a_local_objective_over_the_nav_waypoint() {
        let world = world_at_origin();
        let pool = patrol_pool(); // Patrol toward "alpha" at [100, 0, 0] (starboard)
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha();

        let (thrust, steering) = plan_helm_travel(
            &world,
            &pool,
            &doctrine,
            &anchors,
            NO_CURSORS,
            None,
            Some([0.0, -999.0]), // cleared, and nowhere near `alpha`
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            thrust > 0.0 && steering > 0.0,
            "Patrol resolves, so the helm must fly it (toward `alpha`, to \
             starboard = positive steering) and ignore the nav waypoint; \
             got thrust={thrust}, steering={steering}"
        );
    }

    /// The fallthrough is per-tick and stateless: an objective that *cannot*
    /// resolve yields to the nav waypoint. This is the case the handoff exists
    /// for - a Navigation AI steering a short-range Helm toward an objective the
    /// Helm cannot see yet.
    #[test]
    fn operate_helm_falls_through_unresolvable_objectives_to_the_nav_waypoint() {
        let world = world_at_origin(); // no entities -> Destroy unresolvable
                                       // Destroy (90, unresolvable) then Patrol (30); with empty anchors the
                                       // Patrol cannot resolve either, so both fall through.
        let pool = destroy_then_patrol_pool(&std::collections::HashMap::new());

        let (thrust, _steering) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &std::collections::HashMap::new(),
            NO_CURSORS,
            None,
            Some([100.0, 0.0]),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            thrust > 0.0,
            "must fall through to the nav waypoint when no objective resolves"
        );
    }

    /// End-to-end: the Helm flies the nav waypoint while it has nothing better
    /// to do, then switches to Destroy the moment Tactical locks a target the
    /// Helm can see.
    #[test]
    fn operate_helm_transitions_from_nav_waypoint_to_destroy() {
        let (world, pool, _anchors, target_uuid) = destroy_vs_patrol_scene();
        let no_anchors = std::collections::HashMap::new();

        // Phase 1: no objective resolves (no anchors for the Patrol, no lock for
        // the Destroy) -> fly the waypoint, which sits to starboard.
        let (thrust1, steering1) = plan_helm_travel(
            &world,
            &[],
            &[],
            &no_anchors,
            NO_CURSORS,
            None,
            Some([200.0, 0.0]),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(
            thrust1 > 0.0 && steering1 > 0.0,
            "phase 1: must fly toward the nav waypoint (to starboard)"
        );

        // Phase 2: Tactical locks the hostile, which is dead ahead -> Destroy
        // resolves and outranks the waypoint.
        let (thrust2, steering2) = plan_helm_travel(
            &world,
            &pool,
            &[],
            &no_anchors,
            NO_CURSORS,
            Some(target_uuid),
            Some([200.0, 0.0]),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            0.6,
        );
        assert!(thrust2 > 0.0, "phase 2: must close on the hostile");
        assert!(
            steering2.abs() < PATROL_DEADBAND_RAD,
            "phase 2: a resolved Destroy must win over the nav waypoint - the \
             target is dead ahead (~0 steering), the waypoint to starboard; \
             got steering={steering2}"
        );
    }

    // ── Desired-motion codec (issue #741) ─────────────────────────────────

    #[test]
    fn thrust_velocity_codec_round_trips() {
        for thrust in [-1.0, -0.37, 0.0, 0.42, 1.0] {
            let v = encode_local_velocity(thrust, 0.0);
            // Forward is local -Z, no lateral/vertical component.
            assert_eq!(v[0], 0.0);
            assert!((decode_thrust_from_velocity(v) - thrust).abs() < 1e-6);
        }
        // Forward thrust yields a negative Z (local forward) component.
        assert!(encode_local_velocity(0.8, 0.0)[2] < 0.0);
    }

    #[test]
    fn steering_facing_codec_round_trips_and_preserves_sign() {
        for steering in [-1.0, -0.5, 0.0, 0.25, 1.0] {
            let f = encode_local_facing(steering);
            // Unit-length facing direction.
            assert!(((f[0] * f[0] + f[2] * f[2]).sqrt() - 1.0).abs() < 1e-6);
            let decoded = decode_steering_from_facing(f);
            assert!(
                (decoded - steering).abs() < 1e-6,
                "steering {steering} round-tripped to {decoded}"
            );
        }
        // Zero steering faces exactly local forward (-Z); decode is exactly 0.
        assert_eq!(decode_steering_from_facing(encode_local_facing(0.0)), 0.0);
        // Starboard steering points the facing to +X.
        assert!(encode_local_facing(0.5)[0] > 0.0);
    }

    // ── Docking close manoeuvre (issue #742) ──────────────────────────────

    #[test]
    fn docking_reverses_for_a_dock_directly_astern() {
        // Ship at origin facing -Z (forward). A dock at +Z is dead astern.
        let m = docking_close_manoeuvre(0.0, 0.0, 0.0, 0.0, 10.0, 40.0, 0.3)
            .expect("dock inside engage distance must yield a close manoeuvre");
        assert!(
            m[1] > 0.0,
            "an astern dock must command controlled reverse (aft > 0); got {m:?}"
        );
        assert!(
            m[0].abs() < 1e-6,
            "a dock straight astern needs no lateral translation; got {m:?}"
        );
        assert!(
            m[1] <= 0.3 + 1e-6,
            "reverse must be capped by approach_speed"
        );
    }

    #[test]
    fn docking_translates_laterally_for_a_dock_abeam() {
        // Ship at origin facing -Z; a dock at +X is off the starboard beam.
        let m = docking_close_manoeuvre(0.0, 0.0, 0.0, 10.0, 0.0, 40.0, 0.3)
            .expect("dock inside engage distance must yield a close manoeuvre");
        assert!(
            m[0] > 0.0,
            "a starboard-beam dock must command starboard lateral translation; got {m:?}"
        );
        assert!(
            m[1].abs() < 1e-6,
            "a dock straight abeam needs no fore/aft translation; got {m:?}"
        );
    }

    #[test]
    fn docking_holds_off_beyond_engage_distance() {
        // Dock 100 units away, engage distance 40 — still normal approach.
        assert_eq!(
            docking_close_manoeuvre(0.0, 0.0, 0.0, 0.0, 100.0, 40.0, 0.3),
            None,
            "a dock beyond engage distance must not trigger a close manoeuvre"
        );
    }

    #[test]
    fn assess_hazards_flags_a_projected_collision_ahead() {
        // Ship at origin facing -Z, moving forward; obstacle dead ahead.
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(9),
                position: [0.0, 0.0, -10.0],
                radius: 5.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        // Speed 3 over the 3 s look-ahead projects the ship to z=-9, right up
        // against the obstacle at z=-10 (projected distance 1 < radius 12).
        let hz = assess_hazards(
            &view,
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert!(
            hz.urgency > 0.0,
            "an imminent head-on must register urgency"
        );
        assert_eq!(hz.primary, Some(Uuid::from_u128(9)));
        // Repulsion pushes aft (local +Z) to brake off the obstacle ahead.
        assert!(
            hz.forces_local[2] > 0.0,
            "expected an aft-pushing repulsion, got {:?}",
            hz.forces_local
        );
        // The contributing hazard is exposed with its published facts and the
        // force it added (issue #743).
        assert_eq!(hz.contributions.len(), 1);
        let c = &hz.contributions[0];
        assert_eq!(c.uuid, Uuid::from_u128(9));
        assert_eq!(c.size_rating, 0.0);
        assert!(
            c.dangerous,
            "a contributing hazard is dangerous by definition"
        );
        assert!(
            c.force_local[2] > 0.0,
            "the contribution's own force must push aft, got {:?}",
            c.force_local
        );
        assert!((c.threat_fraction - hz.urgency).abs() < 1e-6);
    }

    #[test]
    fn assess_hazards_is_quiet_with_no_entities() {
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            self_radius: 2.0,
            ..Default::default()
        };
        let hz = assess_hazards(
            &view,
            10.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert_eq!(hz, HazardAssessmentRaw::default());
    }

    #[test]
    fn assess_hazards_skips_non_dangerous_entities() {
        // A non-dangerous entity dead ahead must not register as a hazard: the
        // published `dangerous` fact, not the geometry, decides (issue #743).
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            self_size_rating: 2.0,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(9),
                position: [0.0, 0.0, -10.0],
                radius: 5.0,
                dangerous: false,
                ..Default::default()
            }],
            ..Default::default()
        };
        let hz = assess_hazards(
            &view,
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert_eq!(
            hz,
            HazardAssessmentRaw::default(),
            "a non-dangerous entity must contribute no force"
        );
    }

    #[test]
    fn assess_hazards_ignores_hazards_smaller_than_self_when_authored() {
        // Large self (size_rating 10) versus a small obstacle (size_rating 1)
        // dead ahead. With the ignore rule authored on (ratio 1.0), a hazard
        // strictly smaller than self is skipped entirely — "large ships do not
        // avoid smaller ships at all" (issue #743).
        let small_obstacle = AiWorldEntity {
            uuid: Uuid::from_u128(9),
            position: [0.0, 0.0, -10.0],
            radius: 1.0,
            size_rating: 1.0,
            dangerous: true,
            ..Default::default()
        };
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            self_size_rating: 10.0,
            entities: vec![small_obstacle.clone()],
            ..Default::default()
        };

        // Rule off (ratio 0.0, the default): the small obstacle is a hazard.
        let assessed = assess_hazards(&view, 3.0, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 0.0);
        assert!(
            assessed.urgency > 0.0,
            "with the ignore rule off, even a small obstacle is a hazard"
        );

        // Rule on (ratio 1.0): the smaller hazard is ignored → no force at all.
        let ignored = assess_hazards(&view, 3.0, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 1.0);
        assert_eq!(
            ignored,
            HazardAssessmentRaw::default(),
            "an authored ignore-smaller rule must skip a hazard below self's size rating"
        );

        // A same-or-larger hazard is still assessed under the same ratio.
        let big_view = WorldView {
            entities: vec![AiWorldEntity {
                size_rating: 10.0,
                ..small_obstacle
            }],
            ..view
        };
        let big = assess_hazards(
            &big_view,
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            1.0,
        );
        assert!(
            big.urgency > 0.0,
            "a hazard at or above self's size rating is never ignored"
        );
    }
}
