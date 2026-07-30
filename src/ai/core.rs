/// Pure AI module — no Bevy imports.
///
/// Contains navigation utilities (`steer_toward`, `avoidance_steering`),
/// per-system operate functions (`operate_helm`), and the shared
/// [`assess_hazards`] collision surface (issue #743). The operate functions are pure: issue #702
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
/// Authored hazard-urgency threshold at or above which an imminent collision may
/// TEMPORARILY override the ship's desired facing to point along the escape
/// direction (issue #780, AC4). Below it, ordinary avoidance only bends travel
/// and never touches facing. `1.0` here means "off by default" (only a
/// full-urgency, effectively-unavoidable collision qualifies) — a hull opts into
/// an earlier facing bail-out by authoring a lower
/// [`crate::entity_config::BehaviourConfig::imminent_collision_facing_threshold`].
/// Parse-time default only; the override is stateless and evaporates the tick
/// urgency drops back under the threshold.
pub const IMMINENT_COLLISION_FACING_THRESHOLD: f32 = 1.0;
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
    /// Threat fact: how far this entity can put **direct fire** — the longest
    /// effective range across its usable, online phaser and blaster banks
    /// (issue #788). Homing weapons are deliberately excluded; see
    /// `console::weapons::longest_usable_direct_fire_range`.
    ///
    /// `0.0` means "no reach known": an unarmed entity, an entity whose banks
    /// are all offline, or a snapshot source that does not carry weapon
    /// configuration (the helm's fallback ECS query). A doctrine standing off at
    /// "their reach plus a margin" therefore falls back to the margin alone
    /// rather than to an invented distance.
    pub direct_fire_range: f32,
    /// Threat fact: this entity's ONLINE direct-fire arcs as **world-bearing**
    /// sectors (issue #874), produced once per snapshot rebuild by
    /// `ai::server::entity_weapon_arc_sectors`.
    ///
    /// Never scan-gated: a weapon bank's arc is a property of the hull's
    /// authored configuration, so it is known for every hostile whether or not
    /// anyone has run a sensor sweep on it.
    ///
    /// This is the ONE representation both consumers read — the helm AI's
    /// exposure fact reduction and the local ship's helm-radar overlay payload —
    /// so what a human helm is shown and what a backfilled helm policy reasons
    /// about are the same sectors by construction, not by coincidence.
    ///
    /// Empty for an unarmed entity, an entity whose banks are all offline, an
    /// asteroid, or a snapshot source that carries no weapon configuration.
    pub weapon_arcs: Vec<crate::weapons::arc_geometry::WeaponArcSector>,
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
            direct_fire_range: 0.0,
            weapon_arcs: Vec::new(),
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

/// Read one authored doctrine entry as the [`AiDirective`] the runtime will
/// actually fly.
///
/// This is the **single** statement of which `directive_*` field each kind
/// reads: `Patrol` takes the plural `directive_anchors`, `Reach`/`Retreat` the
/// singular `directive_anchor`, and everything else takes neither. Public since
/// issue #888 so the world-composition validator can ask the same question
/// rather than re-deriving the field table a third time — a second copy is
/// exactly how the Requiem courier ended up authoring `directive_anchors` on a
/// `Reach` and resolving to nothing.
///
/// [`AiDirective`]: crate::messages::AiDirective
pub fn parse_doctrine_directive(
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

// ── Target-relative motion + the fly-through attack pass (issue #883) ────────
//
// `helm_destroy` above is a **brake-and-orbit station-keeper**: it aims at an
// offset approach point, ramps thrust down through a decel zone, and parks at
// `stop_dist` re-facing the target so the forward phaser arc bears. That is the
// exact opposite of a fly-through attack run, so #883 does NOT reuse it and
// does NOT bolt a mode flag onto it. The pass gets its own pure decision arm
// below — one that never brakes, never offsets the approach point, and whose
// two legs differ only in *which direction they steer toward*.
//
// Both legs fold the shared `avoidance_steering` contribution in before the
// final clamp, exactly as `helm_destroy` does (the `(base_steer + avoidance)
// .clamp(-1, 1)` shape). That is what lets hazard avoidance BEND the escape
// without any part of it touching the policy's pass state (AC3): avoidance is a
// steering force here, never a state input.

/// The target-relative motion readings a fly-through pass reasons about
/// (issue #883), all measured in the XZ plane like the rest of the helm maths.
///
/// [`AiWorldEntity`] carries no velocity vector, so the relative velocity used
/// for [`Self::closing_rate`] is *reconstructed* from each party's yaw and
/// forward speed — the same reconstruction `avoidance_steering` performs for its
/// projected positions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetRelativeMotion {
    /// Planar distance from the observer to the target (world units).
    pub range: f32,
    /// Rate at which `range` is shrinking (world units/s). **Positive while
    /// closing, negative while opening** — so closest approach is the instant
    /// this crosses from `+` to `-`.
    pub closing_rate: f32,
    /// Signed angle (radians) from the observer's forward vector to the range
    /// vector; positive is starboard, matching [`steer_toward`]'s convention.
    pub bearing_rad: f32,
}

/// Compute [`TargetRelativeMotion`] for one observer/target pair (issue #883).
///
/// `target_yaw` is `None` for an entity whose heading is unknown (an asteroid,
/// or a snapshot entry with no yaw): its velocity then contributes nothing, so
/// the closing rate degrades to the observer's own approach speed rather than
/// silently inventing a heading.
///
/// A degenerate (near-zero) range yields all-zero readings rather than a NaN
/// bearing — the no-panic contract the fact seeding depends on.
pub fn target_relative_motion(
    self_pos: [f32; 3],
    self_yaw: f32,
    self_speed: f32,
    target_pos: [f32; 3],
    target_yaw: Option<f32>,
    target_speed: f32,
) -> TargetRelativeMotion {
    let dx = target_pos[0] - self_pos[0];
    let dz = target_pos[2] - self_pos[2];
    let range = (dx * dx + dz * dz).sqrt();
    if range <= f32::EPSILON {
        return TargetRelativeMotion::default();
    }
    let (ux, uz) = (dx / range, dz / range);

    // Velocities reconstructed from (yaw, forward_speed) — see the struct note.
    let self_vx = self_yaw.sin() * self_speed;
    let self_vz = -self_yaw.cos() * self_speed;
    let (tgt_vx, tgt_vz) = match target_yaw {
        Some(y) => (y.sin() * target_speed, -y.cos() * target_speed),
        None => (0.0, 0.0),
    };

    // d(range)/dt = relative_velocity · unit_range_vector; closing is its
    // negation so "closing" reads positive.
    let closing_rate = -((tgt_vx - self_vx) * ux + (tgt_vz - self_vz) * uz);

    let fwd_x = self_yaw.sin();
    let fwd_z = -self_yaw.cos();
    let cross = fwd_x * uz - fwd_z * ux;
    let dot = fwd_x * ux + fwd_z * uz;

    TargetRelativeMotion {
        range,
        closing_rate,
        bearing_rad: cross.atan2(dot),
    }
}

/// Which leg of a fly-through attack pass the ship is flying (issue #883).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyThroughLeg {
    /// Closing on the target, steering is re-solved against the target's CURRENT
    /// position every tick (it is a moving target).
    Inbound,
    /// Past closest approach: the heading is FROZEN. Nothing about the target
    /// enters the steering solution any more — that is what "commits to the
    /// current outward heading" means, and it is why the escape cannot be
    /// dragged back around by a target that keeps moving.
    Escape,
    /// Recovery is over: the ship is turning back onto the target to begin
    /// another pass (issue #788). Steering tracks the target exactly as
    /// [`FlyThroughLeg::Inbound`] does — this IS the pivot — but the throttle is
    /// the authored re-engage fraction rather than the approach fraction, so a
    /// hull can author `0.0` and pivot on cut thrust before the run starts.
    Reengage,
    /// A torpedo opportunity is open: hold the BOW on the target while a fixed
    /// forward tube lines up (issue #791). Steering tracks the target's live
    /// position exactly as [`FlyThroughLeg::Inbound`] and
    /// [`FlyThroughLeg::Reengage`] do — a target that keeps manoeuvring keeps
    /// being followed, which is the whole point of a bow-on hold — and the
    /// throttle is the authored `torpedo_bearing_speed`.
    ///
    /// A third tracking leg rather than a flag on `Reengage` because the two
    /// take their throttle from different authored scalars, and the host gates
    /// them on different authored sets: `Reengage` needs the six shield-recovery
    /// params, this needs one of its own.
    TorpedoBearing,
}

/// Inputs to [`plan_fly_through_pass`] (issue #883).
///
/// Every gameplay scalar here (`approach_speed`, `escape_speed`, the two
/// steering-response angles, the avoidance tuning) arrives from authored ship
/// data — there is no default and no fallback constant in this module, so
/// AGENTS.md #11 holds by construction: omit the authoring and the host never
/// selects this arm at all.
pub struct FlyThroughPassInput<'a> {
    pub leg: FlyThroughLeg,
    pub self_pos: [f32; 3],
    pub self_yaw: f32,
    pub self_speed: f32,
    pub self_radius: f32,
    /// The target's current world position. Read only on the [`FlyThroughLeg::Inbound`] leg.
    pub target_pos: [f32; 3],
    /// The target's uuid, excluded from the avoidance scan so the ship does not
    /// treat the ship it is attacking as an obstacle to swerve around — the same
    /// exclusion `helm_destroy` makes.
    pub target_uuid: Uuid,
    /// The heading (radians) frozen at the closest-approach transition. Read
    /// only on the [`FlyThroughLeg::Escape`] leg.
    pub escape_heading_rad: f32,
    /// Throttle fraction flown while inbound.
    pub approach_speed: f32,
    /// Throttle fraction flown on the escape leg.
    pub escape_speed: f32,
    /// Throttle fraction flown on the [`FlyThroughLeg::Reengage`] pivot
    /// (issue #788). Read only on that leg.
    pub reengage_speed: f32,
    /// Throttle fraction flown on the [`FlyThroughLeg::TorpedoBearing`] hold
    /// (issue #791). Read only on that leg. `0.0` cuts thrust for the bow-on
    /// tracking phase — the authored value a hull that wants to stop swinging
    /// its beam and line a fixed tube up gives it.
    pub torpedo_bearing_speed: f32,
    /// Angular deadband below which the tracking solution commands no yaw.
    pub tracking_deadband_rad: f32,
    /// Angular error at which the tracking solution saturates to ±1.
    pub tracking_full_steer_rad: f32,
    pub entities: &'a [AiWorldEntity],
    pub avoidance_buffer: f32,
    pub avoidance_look_ahead_secs: f32,
}

/// The fly-through attack pass: `(thrust, steering)` for one tick (issue #883).
///
/// **No braking.** Thrust is the leg's authored throttle fraction, flat, for the
/// whole leg. A pass that decelerated near the target would be an orbit, which
/// is what `helm_destroy` already does and what this deliberately is not.
///
/// **Inbound tracks; escape does not.** The inbound leg re-derives the steering
/// direction from the target's position every call, so a moving target is
/// followed continuously (AC1). The escape leg derives it from
/// `escape_heading_rad` alone (AC2) — the target could vanish or reverse and the
/// escape would not notice.
///
/// **Avoidance bends both legs** (AC3): the shared repulsion steering is summed
/// onto the leg's own steering and the result clamped, so a hazard curves the
/// escape without the leg — or the caller's pass state — changing at all.
pub fn plan_fly_through_pass(input: &FlyThroughPassInput) -> (f32, f32) {
    let (dir, thrust) = match input.leg {
        FlyThroughLeg::Inbound | FlyThroughLeg::Reengage | FlyThroughLeg::TorpedoBearing => {
            let dx = input.target_pos[0] - input.self_pos[0];
            let dz = input.target_pos[2] - input.self_pos[2];
            let dist = (dx * dx + dz * dz).sqrt();
            let dir = if dist > f32::EPSILON {
                [dx / dist, dz / dist]
            } else {
                // On top of the target: hold the current heading rather than
                // dividing by ~0. The pass is over in any meaningful sense.
                [input.self_yaw.sin(), -input.self_yaw.cos()]
            };
            // Three tracking legs, three authored throttles. The geometry above
            // is shared precisely because "point at where the target IS now" is
            // one question; what a hull does with its engines while it does that
            // is the doctrine's, and each leg names its own scalar.
            let thrust = match input.leg {
                FlyThroughLeg::Reengage => input.reengage_speed,
                FlyThroughLeg::TorpedoBearing => input.torpedo_bearing_speed,
                _ => input.approach_speed,
            };
            (dir, thrust)
        }
        FlyThroughLeg::Escape => (
            [
                input.escape_heading_rad.sin(),
                -input.escape_heading_rad.cos(),
            ],
            input.escape_speed,
        ),
    };

    let base_steer = steer_toward(
        input.self_yaw,
        dir,
        input.tracking_deadband_rad,
        input.tracking_full_steer_rad,
    );
    let avoidance = avoidance_steering(
        input.self_pos,
        input.self_yaw,
        input.self_speed,
        input.self_radius,
        input.target_uuid,
        input.entities,
        input.avoidance_buffer,
        input.avoidance_look_ahead_secs,
    );
    (
        thrust.clamp(-1.0, 1.0),
        (base_steer + avoidance).clamp(-1.0, 1.0),
    )
}

/// Inputs to [`plan_recovery_orbit`] (issue #788; second caller added by #790).
///
/// Every gameplay scalar arrives from authored ship data or from host-written
/// private memory; this module holds no default for any of them (AGENTS.md #11).
///
/// The name is historical — this is the input to *ring* geometry, not to the
/// shield-recovery doctrine specifically. Two doctrines fill it in, with
/// opposite intents and different radii: the shield-recovery standoff (the
/// radius derived from the TARGET's reach, held to stay out of trouble) and the
/// cruiser's combat broadside orbit (an authored radius from the shooter's OWN
/// weapon envelope, held to stay in it). See `ship::helm_planner`'s `fly_orbit`,
/// which is the single call site both route through.
pub struct RecoveryOrbitInput<'a> {
    pub self_pos: [f32; 3],
    pub self_yaw: f32,
    pub self_speed: f32,
    pub self_radius: f32,
    /// The centre of the ring: the ship being stood off from.
    pub target_pos: [f32; 3],
    /// Excluded from the avoidance scan — the ship is deliberately circling it,
    /// so treating it as an obstacle would fight the orbit.
    pub target_uuid: Uuid,
    /// The radius of the ring to hold, world units. Whose radius it is depends
    /// on the calling doctrine and the geometry does not care: the standoff leg
    /// passes a host-derived "target's direct-fire reach + this hull's authored
    /// margin"; the combat orbit passes the hull's own authored
    /// `combat_orbit_range`. The name is historical (issue #788 had only the
    /// first caller).
    pub safe_range: f32,
    /// Which way round: `+1.0` clockwise (starboard-hand), `-1.0` the other
    /// way. Chosen once per recovery from a seeded composite key, so it is
    /// deterministic without being predictable.
    pub orbit_direction: f32,
    /// How hard a radial error bends the tangential course, in radians of
    /// heading offset per unit of *fractional* range error. The spiral gain.
    pub spiral_gain: f32,
    /// Throttle fraction flown on the ring.
    pub orbit_speed: f32,
    pub tracking_deadband_rad: f32,
    pub tracking_full_steer_rad: f32,
    pub entities: &'a [AiWorldEntity],
    pub avoidance_buffer: f32,
    pub avoidance_look_ahead_secs: f32,
}

/// Hold a ring around a target: `(thrust, steering)` for one tick (issue #788,
/// generalised by #790).
///
/// **Shared geometry, parameterised by a radius.** Two doctrines fly it and the
/// solution below knows about neither: the shield-recovery standoff circles at a
/// radius derived from the target's own reach to stay OUT of its envelope, and
/// the cruiser's combat broadside orbit circles at its own authored
/// `combat_orbit_range` to keep its beams ON. Only the radius
/// ([`RecoveryOrbitInput::safe_range`]), `orbit_speed` and `spiral_gain` differ;
/// the intent is the caller's and never appears here. The name is historical —
/// #788 had one caller and named the function after it.
///
/// **It spirals; it does not stop and it does not simply run.** The commanded
/// heading is the *tangent* of the ring — perpendicular to the bearing back to
/// the target, on the authored side — rotated toward or away from the target in
/// proportion to how wrong the current radius is:
///
/// * inside the ring (`range < safe_range`) the tangent is rotated *outward*,
///   so the ship spirals out while still travelling around;
/// * outside it, the tangent is rotated *inward*, so it spirals back in rather
///   than fleeing to infinity;
/// * on it, the ship flies the pure tangent and holds the ring.
///
/// The error is *fractional* (`(range - safe_range) / safe_range`) so the same
/// authored gain behaves identically for a ring of 80 units and one of 800 —
/// the alternative would make `spiral_gain` a value a designer has to re-tune
/// every time a weapon's range changes, and it is what lets the two doctrines
/// share a gain scale at all. The rotation is clamped to a quarter turn, which
/// is the point at which the "tangent bent toward the target" becomes "straight
/// at the target": beyond it the correction would start spiralling the wrong way
/// round.
///
/// Throttle is the flat authored orbit fraction — the ship keeps its energy up
/// while it circles, because a stationary ship inside a hostile's reach is
/// neither recovering nor fighting, it is a target.
///
/// **Avoidance bends the orbit** exactly as it bends the pass legs: the shared
/// repulsion steering is summed on and the result clamped.
pub fn plan_recovery_orbit(input: &RecoveryOrbitInput) -> (f32, f32) {
    let dx = input.target_pos[0] - input.self_pos[0];
    let dz = input.target_pos[2] - input.self_pos[2];
    let range = (dx * dx + dz * dz).sqrt();

    // Sitting exactly on the target (or an un-authored ring) leaves no tangent
    // to fly: hold the current heading rather than dividing by ~0.
    let dir = if range <= f32::EPSILON || input.safe_range <= f32::EPSILON {
        [input.self_yaw.sin(), -input.self_yaw.cos()]
    } else {
        let inward = [dx / range, dz / range];
        // Tangent of the ring, on the authored side. `sign` is the sense of the
        // circulation; ±1 is all `orbit_direction` ever carries.
        let sign = if input.orbit_direction >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let tangent = [-inward[1] * sign, inward[0] * sign];

        // Fractional radial error, positive when too far out.
        let error = (range - input.safe_range) / input.safe_range;
        // Rotate the tangent toward the target when outside the ring, away from
        // it when inside. Clamped to a quarter turn — see the doc comment.
        let correction = (error * input.spiral_gain)
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
        let (s, c) = correction.sin_cos();
        // Blend tangent toward `inward` by `correction`: at 0 it is the pure
        // tangent, at ±π/2 it is straight at (or straight away from) the target.
        [
            tangent[0] * c + inward[0] * s,
            tangent[1] * c + inward[1] * s,
        ]
    };

    let base_steer = steer_toward(
        input.self_yaw,
        dir,
        input.tracking_deadband_rad,
        input.tracking_full_steer_rad,
    );
    let avoidance = avoidance_steering(
        input.self_pos,
        input.self_yaw,
        input.self_speed,
        input.self_radius,
        input.target_uuid,
        input.entities,
        input.avoidance_buffer,
        input.avoidance_look_ahead_secs,
    );
    (
        input.orbit_speed.clamp(-1.0, 1.0),
        (base_steer + avoidance).clamp(-1.0, 1.0),
    )
}

/// Inputs to [`plan_artillery_position`] (issue #792).
///
/// Every gameplay scalar arrives from authored ship data — the hold throttle
/// from the Steering policy's own `param` map, the lead speed from the hull's
/// artillery bank — and this module holds no default for either (AGENTS.md #11).
pub struct ArtilleryPositionInput<'a> {
    pub self_pos: [f32; 3],
    pub self_yaw: f32,
    pub self_speed: f32,
    pub self_radius: f32,
    /// The target's current world position — the point the lead is measured
    /// FROM, never the point the bow is put on.
    pub target_pos: [f32; 3],
    /// The target's heading, or `None` for an entity whose heading is unknown
    /// (an asteroid, or a snapshot entry with no yaw). [`AiWorldEntity`] carries
    /// no velocity field, so the target's velocity is reconstructed from
    /// `(yaw, forward_speed)` exactly as [`target_relative_motion`] reconstructs
    /// it. An unknown heading contributes no velocity, so the solution degrades
    /// to "aim at where it is" rather than inventing a course.
    pub target_yaw: Option<f32>,
    /// The target's forward speed, world units/s.
    pub target_speed: f32,
    /// Excluded from the avoidance scan — the ship is deliberately holding
    /// station on it, so treating it as an obstacle would fight the hold.
    pub target_uuid: Uuid,
    /// Authored throttle fraction flown while the firing position is held.
    /// `0.0` is the value an artillery platform wants: a gun line that keeps
    /// closing is not a gun line.
    pub hold_speed: f32,
    /// The lead speed: the flight speed of the bolt whose intercept is being
    /// solved, read host-side off the hull's OWN artillery bank. `0.0` (no
    /// artillery aboard, or an unresolvable bank) degrades the solution to the
    /// target's live position rather than inventing a flight time.
    pub projectile_speed: f32,
    pub tracking_deadband_rad: f32,
    pub tracking_full_steer_rad: f32,
    pub entities: &'a [AiWorldEntity],
    pub avoidance_buffer: f32,
    pub avoidance_look_ahead_secs: f32,
}

/// Hold the artillery firing position: `(thrust, steering)` for one tick
/// (issue #792).
///
/// **It holds; it does not orbit and it does not kite.** Thrust is the flat
/// authored [`ArtilleryPositionInput::hold_speed`] — nothing here reads the
/// range, so a target that closes cannot push the hull backwards and a target
/// that opens cannot pull it forwards. Whether the hull should be here at all is
/// the doctrine's question, answered by the authored range band in its own
/// transition guards; this function only flies the position once it is held.
///
/// **The facing is a PREDICTIVE intercept, not a bearing.** The target's
/// velocity is reconstructed from `(yaw, forward_speed)` and fed through the
/// SAME [`crate::weapons::blaster::predict_intercept_heading`] the bolt itself is
/// launched on, at the same lead speed — so the bow ends up on the heading the
/// gun will actually fire, and the AI's aim cannot drift from its own ballistics.
/// That function solves the closed-form intercept
/// ([`crate::weapons::blaster::solve_intercept_time`]) rather than estimating a
/// flight time, so the bow leads a crossing target by the full solved angle
/// instead of the systematically short first-order one — the gun and the bow
/// improve together, because they are the same call.
/// The fallback that function takes when it cannot lead at all (no flight speed,
/// or a predicted point sitting on the shooter) is passed as "the heading
/// straight at the target right now", which is the honest degradation rather
/// than a frozen heading. When an intercept simply does not exist — a target
/// outrunning the bolt — the solver's own first-order degradation applies, and
/// the bow still points ahead of the runner rather than at it.
///
/// **Avoidance bends the hold** exactly as it bends every other leg: the shared
/// repulsion steering is summed onto the solved facing and the result clamped.
/// That is the whole of this leg's hazard handling by design — it composes,
/// temporarily, and evaporates when the hazard clears, so a detour can never
/// become a state the hull has to get back out of.
pub fn plan_artillery_position(input: &ArtilleryPositionInput) -> (f32, f32) {
    let dx = input.target_pos[0] - input.self_pos[0];
    let dz = input.target_pos[2] - input.self_pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    let dir = if dist <= f32::EPSILON {
        // Sitting on the target: hold the current heading rather than solving a
        // bearing from a zero-length vector.
        [input.self_yaw.sin(), -input.self_yaw.cos()]
    } else {
        // Velocities reconstructed from (yaw, forward_speed) — the snapshot
        // carries no velocity field. Same recipe as `target_relative_motion`.
        let (tvx, tvz) = match input.target_yaw {
            Some(y) => (y.sin() * input.target_speed, -y.cos() * input.target_speed),
            None => (0.0, 0.0),
        };
        // The heading straight at where the target is NOW, in the project's
        // `atan2(dx, -dz)` convention. Handed in as the shooter yaw with a zero
        // bank facing so that `predict_intercept_heading`'s own fallback — which
        // is `shooter_yaw + facing_deg` — resolves to exactly this, rather than
        // to whichever way the hull happened to be pointing.
        let live = dx.atan2(-dz);
        let heading = crate::weapons::blaster::predict_intercept_heading(
            input.self_pos[0],
            input.self_pos[2],
            input.target_pos[0],
            input.target_pos[2],
            tvx,
            tvz,
            input.projectile_speed,
            live,
            0.0,
        );
        [heading.sin(), -heading.cos()]
    };

    let base_steer = steer_toward(
        input.self_yaw,
        dir,
        input.tracking_deadband_rad,
        input.tracking_full_steer_rad,
    );
    let avoidance = avoidance_steering(
        input.self_pos,
        input.self_yaw,
        input.self_speed,
        input.self_radius,
        input.target_uuid,
        input.entities,
        input.avoidance_buffer,
        input.avoidance_look_ahead_secs,
    );
    (
        input.hold_speed.clamp(-1.0, 1.0),
        (base_steer + avoidance).clamp(-1.0, 1.0),
    )
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

/// Reduce every hostile's published weapon-arc sectors against this ship's own
/// position (issue #874).
///
/// Pure and Bevy-free: consumes the same [`WorldView`] the helm already steers
/// by, so a guard can never fire on a contact the helm cannot see.
///
/// **No scan gate.** Arcs come from authored hull configuration, published on
/// [`AiWorldEntity::weapon_arcs`] by the world-snapshot build, so they are known
/// for every hostile in view whether or not Sensors has swept it. That is the
/// point of the fact: dodging a gun should not require identifying it first.
///
/// ## The reduction, and why it is this one
///
/// `AiFacts` values are `f64` scalars, so the sector list cannot itself be a
/// fact. Two readings come out:
///
/// - `covering_count` — summed across ALL hostiles, because a movement policy
///   being borne on by two ships is in more trouble than one borne on by one,
///   and a bare "am I exposed" boolean throws that away for nothing.
/// - `escape_offset_deg` — taken from the NEAREST hostile that has at least one
///   arc bearing, because a single number cannot escape two ships at once and
///   the nearest gun is the urgent one. `0.0` when nothing bears.
/// - `inescapable` — set when ANY hostile in view is bearing with an all-round
///   bank, which suppresses the escape magnitude for the same reason the
///   per-ship reduction does: there is no turn out of it. See the
///   `arc_geometry` module note.
pub fn hostile_arc_exposure(
    world_view: &WorldView,
    faction_registry: &crate::faction::FactionRegistry,
) -> crate::weapons::arc_geometry::ArcExposure {
    let mut total_covering = 0u32;
    let mut any_inescapable = false;
    let mut nearest: Option<(f32, f32)> = None; // (dist_sq, escape_offset_deg)
    let self_faction = world_view.self_faction;
    let pos = world_view.entity_pos;
    for e in &world_view.entities {
        if e.weapon_arcs.is_empty() {
            continue;
        }
        let hostile = e
            .faction
            .map(|ef| crate::faction::is_enemy(self_faction, Some(ef), faction_registry))
            .unwrap_or(false);
        if !hostile {
            continue;
        }
        let exposure = crate::weapons::arc_geometry::arc_exposure(
            &e.weapon_arcs,
            e.position[0],
            e.position[2],
            pos[0],
            pos[2],
        );
        if exposure.covering_count == 0 {
            continue;
        }
        total_covering += exposure.covering_count;
        any_inescapable |= exposure.inescapable;
        let d = dist_sq(pos, e.position);
        if nearest.map(|(nd, _)| d < nd).unwrap_or(true) {
            nearest = Some((d, exposure.escape_offset_deg));
        }
    }
    crate::weapons::arc_geometry::ArcExposure {
        covering_count: total_covering,
        // Suppressed when ANY hostile in view has an all-round bank bearing:
        // a turn that clears the nearest ship's finite arcs does not clear an
        // all-round one, so reporting the magnitude would be the same lie the
        // per-ship reduction refuses to tell.
        escape_offset_deg: if any_inescapable {
            0.0
        } else {
            nearest.map(|(_, o)| o).unwrap_or(0.0)
        },
        inescapable: any_inescapable,
    }
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
            // Vertical (local +Y = up) repulsion, issue #780: the surface is now
            // genuinely 3D, but only ELIGIBLE MOVING hazards contribute a vertical
            // component (AC5) — static obstacles stay a purely planar concern for
            // the lateral/engine actuators. When the hazard is off the ship's
            // cruise plane the climb follows the actual vertical separation; when
            // both share the plane (`dy ≈ 0`, the common case today) the initial
            // policy climbs UP by convention to clear a co-planar moving threat.
            // The magnitude matches the planar contribution's severity so a
            // bounded/full-3D hull's authored sensitivity weights all axes alike.
            let vertical_contribution = if entity.movable {
                let dy = self_pos[1] - entity.position[1];
                if dy.abs() > 0.01 {
                    dy.signum() * threat_fraction
                } else {
                    threat_fraction
                }
            } else {
                0.0
            };
            // Rotate the horizontal repulsion into the ship-local frame
            // (x = starboard, z = aft); the vertical component is already ship-up.
            let contribution = [
                rx * stbd_x + rz * stbd_z,
                vertical_contribution,
                -(rx * fwd_x + rz * fwd_z),
            ];
            force[0] += contribution[0];
            force[1] += contribution[1];
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

    // ── hostile_arc_exposure: the all-round case (issue #874) ─────────────

    /// A hostile carrying an all-round bank (`fire_arc_deg = 360`, which the
    /// Harrow Lancer authors twice) must reach the fact reduction as
    /// INESCAPABLE with no escape magnitude — even when a nearer hostile's
    /// finite arcs offer a real one, because turning out of those does not turn
    /// out of the all-round one.
    #[test]
    fn an_all_round_hostile_suppresses_the_escape_magnitude_across_the_view() {
        let hostile_faction = uuid::Uuid::new_v4();
        let own_faction = uuid::Uuid::new_v4();
        let mut registry = crate::faction::FactionRegistry::new();
        registry.insert(crate::faction::FactionConfig {
            uuid: own_faction,
            name: "Own".into(),
            enemies: vec![hostile_faction],
        });

        let armed = |z: f32, half: f32| AiWorldEntity {
            uuid: uuid::Uuid::new_v4(),
            position: [0.0, 0.0, z],
            faction: Some(hostile_faction),
            // Bearing 0 from either hostile astern of us points straight at us.
            weapon_arcs: vec![crate::weapons::arc_geometry::WeaponArcSector {
                bearing_deg: 0.0,
                half_angle_deg: half,
                range: 500.0,
            }],
            ..Default::default()
        };

        // Nearest hostile is the narrow one, so it owns `escape_offset_deg` —
        // and the far all-round hull must still veto it.
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            self_faction: Some(own_faction),
            entities: vec![armed(50.0, 30.0), armed(200.0, 180.0)],
            ..Default::default()
        };
        let e = hostile_arc_exposure(&view, &registry);
        assert_eq!(e.covering_count, 2, "{e:?}");
        assert!(e.inescapable, "{e:?}");
        assert_eq!(e.escape_offset_deg, 0.0, "{e:?}");

        // Drop the all-round hull and the nearest one's real exit comes back.
        let escapable = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            self_faction: Some(own_faction),
            entities: vec![armed(50.0, 30.0)],
            ..Default::default()
        };
        let e = hostile_arc_exposure(&escapable, &registry);
        assert_eq!(e.covering_count, 1, "{e:?}");
        assert!(!e.inescapable, "{e:?}");
        assert!(e.escape_offset_deg.abs() > 0.0, "{e:?}");
    }

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
        // Issue #780: the surface is 3D, but a STATIC hazard contributes no
        // vertical component — the vertical axis is a moving-hazard concern (AC5).
        assert_eq!(
            hz.forces_local[1], 0.0,
            "a static hazard must not push the vertical axis, got {:?}",
            hz.forces_local
        );
        assert_eq!(c.force_local[1], 0.0);
    }

    /// Issue #780 (AC2/AC5): `assess_hazards` is genuinely 3D — an ELIGIBLE
    /// MOVING hazard populates a vertical (local +Y) force so a bounded/full-3D
    /// hull can climb to clear it, while an identically-placed STATIC hazard
    /// leaves the vertical axis untouched. Both register a horizontal threat, so
    /// the only difference is the `movable` fact.
    #[test]
    fn assess_hazards_computes_vertical_force_for_moving_only() {
        // A co-planar obstacle dead ahead that projects into collision.
        let make = |movable: bool| WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(7),
                position: [0.0, 0.0, -10.0],
                radius: 5.0,
                movable,
                dangerous: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let moving = assess_hazards(
            &make(true),
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert!(
            moving.forces_local[1] > 0.0,
            "a co-planar MOVING hazard must produce an upward (climb) vertical \
             force, got {:?}",
            moving.forces_local
        );
        assert!(moving.contributions[0].force_local[1] > 0.0);

        let static_hz = assess_hazards(
            &make(false),
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert_eq!(
            static_hz.forces_local[1], 0.0,
            "a STATIC hazard must leave the vertical axis at zero, got {:?}",
            static_hz.forces_local
        );
        // Both still registered a horizontal (aft) repulsion, proving the vertical
        // difference is the `movable` fact and not just a missed collision.
        assert!(moving.forces_local[2] > 0.0 && static_hz.forces_local[2] > 0.0);
    }

    /// Issue #780: an off-plane moving hazard drives the climb along the ACTUAL
    /// vertical separation — a hazard below the ship pushes it up, one above
    /// pushes it down.
    #[test]
    fn assess_hazards_follows_vertical_separation_sign() {
        let make = |hazard_y: f32| WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(8),
                position: [0.0, hazard_y, -10.0],
                radius: 5.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        // Hazard below (y = -5): dy = self(0) - (-5) = +5 → climb up (+).
        let below = assess_hazards(
            &make(-5.0),
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert!(below.forces_local[1] > 0.0, "hazard below must push up");
        // Hazard above (y = +5): dy = -5 → descend (-).
        let above = assess_hazards(
            &make(5.0),
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            HAZARD_IGNORE_SIZE_RATIO,
        );
        assert!(above.forces_local[1] < 0.0, "hazard above must push down");
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

    // ── Target-relative motion + fly-through pass (issue #883) ───────────────

    /// The closing rate is the signed rate of change of range: positive while
    /// the gap shrinks, negative once it opens. Closest approach is exactly that
    /// sign flip, which is what the destroyer doctrine's transition guard reads.
    #[test]
    fn closing_rate_is_positive_closing_and_negative_opening() {
        // Ship at the origin at yaw 0 (forward = -Z), target 100 units ahead.
        let closing =
            target_relative_motion([0.0, 0.0, 0.0], 0.0, 10.0, [0.0, 0.0, -100.0], None, 0.0);
        assert!((closing.range - 100.0).abs() < 1e-3);
        assert!(
            (closing.closing_rate - 10.0).abs() < 1e-3,
            "flying straight at a stationary target closes at our own speed, got {}",
            closing.closing_rate
        );
        assert!(
            closing.bearing_rad.abs() < 1e-3,
            "dead ahead is zero bearing"
        );

        // Same geometry, but the target is now ASTERN (we already flew past).
        let opening =
            target_relative_motion([0.0, 0.0, 0.0], 0.0, 10.0, [0.0, 0.0, 100.0], None, 0.0);
        assert!(
            opening.closing_rate < 0.0,
            "a target astern of a forward-moving ship must be opening, got {}",
            opening.closing_rate
        );
        assert!(
            (opening.bearing_rad.abs() - std::f32::consts::PI).abs() < 1e-3,
            "a target dead astern bears +/-pi"
        );
    }

    /// `AiWorldEntity` carries no velocity, so the target's contribution to the
    /// closing rate is reconstructed from its yaw + forward speed. A target
    /// running away faster than we chase must read as OPENING even though we are
    /// pointed straight at it — the case a range-only detector gets wrong.
    #[test]
    fn closing_rate_reconstructs_the_targets_own_velocity() {
        // Both at yaw 0 (heading -Z); the target ahead of us and faster.
        let m = target_relative_motion(
            [0.0, 0.0, 0.0],
            0.0,
            5.0,
            [0.0, 0.0, -50.0],
            Some(0.0),
            20.0,
        );
        assert!(
            m.closing_rate < 0.0,
            "a target outrunning us is opening the range, got {}",
            m.closing_rate
        );
        // Target running head-on toward us (yaw pi => heading +Z).
        let head_on = target_relative_motion(
            [0.0, 0.0, 0.0],
            0.0,
            5.0,
            [0.0, 0.0, -50.0],
            Some(std::f32::consts::PI),
            20.0,
        );
        assert!(
            (head_on.closing_rate - 25.0).abs() < 1e-2,
            "head-on closure is the sum of both speeds, got {}",
            head_on.closing_rate
        );
    }

    /// A target off the starboard bow bears positive; off the port bow,
    /// negative — the same sign convention [`steer_toward`] uses, so a policy
    /// guard on `bearing_to_target` and the steering solution cannot disagree
    /// about which way "right" is.
    #[test]
    fn bearing_to_target_is_signed_starboard_positive() {
        let starboard =
            target_relative_motion([0.0, 0.0, 0.0], 0.0, 0.0, [50.0, 0.0, -50.0], None, 0.0);
        assert!(starboard.bearing_rad > 0.0);
        let port =
            target_relative_motion([0.0, 0.0, 0.0], 0.0, 0.0, [-50.0, 0.0, -50.0], None, 0.0);
        assert!(port.bearing_rad < 0.0);
    }

    /// Degenerate (co-located) geometry yields zeroes, never NaN — the seeding
    /// path has no way to poison a policy guard with a NaN comparison.
    #[test]
    fn target_relative_motion_is_degenerate_safe() {
        let m =
            target_relative_motion([7.0, 0.0, -3.0], 1.2, 9.0, [7.0, 0.0, -3.0], Some(0.4), 4.0);
        assert_eq!(m, TargetRelativeMotion::default());
    }

    fn pass_input(leg: FlyThroughLeg, entities: &[AiWorldEntity]) -> FlyThroughPassInput<'_> {
        FlyThroughPassInput {
            leg,
            self_pos: [0.0, 0.0, 0.0],
            self_yaw: 0.0,
            self_speed: 10.0,
            self_radius: 1.0,
            target_pos: [60.0, 0.0, -60.0],
            target_uuid: Uuid::nil(),
            escape_heading_rad: 0.0,
            approach_speed: 0.85,
            escape_speed: 1.0,
            reengage_speed: 0.0,
            // Deliberately different from `reengage_speed` so a leg that took
            // the wrong scalar would be visible rather than accidentally right.
            torpedo_bearing_speed: 0.25,
            tracking_deadband_rad: 0.03,
            tracking_full_steer_rad: 0.6,
            entities,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        }
    }

    /// The inbound leg flies its authored approach throttle FLAT and steers at
    /// the target's current position. Contrast `helm_destroy`, which would be
    /// ramping thrust toward zero at the near range — the pass never brakes.
    #[test]
    fn inbound_leg_tracks_the_target_without_braking() {
        let none: [AiWorldEntity; 0] = [];
        let far = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
        let mut near_input = pass_input(FlyThroughLeg::Inbound, &none);
        near_input.target_pos = [3.0, 0.0, -3.0];
        let near = plan_fly_through_pass(&near_input);
        assert_eq!(
            far.0, near.0,
            "throttle must not fall off as the target gets closer: a fly-through \
             pass does not decelerate into the merge"
        );
        assert!(
            (far.0 - 0.85).abs() < 1e-6,
            "throttle is the authored approach fraction"
        );
        assert!(
            far.1 > 0.0,
            "a target off the starboard bow must command a starboard turn"
        );
    }

    /// The escape leg ignores the target completely: moving it to the opposite
    /// side of the ship changes nothing, because the heading is frozen. This is
    /// the observable difference between "hold the command" and "hold the
    /// heading".
    #[test]
    fn escape_leg_flies_the_frozen_heading_and_ignores_the_target() {
        let none: [AiWorldEntity; 0] = [];
        let mut a = pass_input(FlyThroughLeg::Escape, &none);
        a.target_pos = [500.0, 0.0, 0.0];
        let mut b = pass_input(FlyThroughLeg::Escape, &none);
        b.target_pos = [-500.0, 0.0, 0.0];
        assert_eq!(
            plan_fly_through_pass(&a),
            plan_fly_through_pass(&b),
            "the escape solution must not depend on the target at all"
        );
        // Already on the frozen heading (yaw 0 == heading 0) -> no yaw demanded.
        assert_eq!(plan_fly_through_pass(&a).1, 0.0);
        assert!((plan_fly_through_pass(&a).0 - 1.0).abs() < 1e-6);

        // A frozen heading off to starboard turns the ship onto it and holds.
        let mut turned = pass_input(FlyThroughLeg::Escape, &none);
        turned.escape_heading_rad = 1.0;
        assert!(plan_fly_through_pass(&turned).1 > 0.0);
    }

    /// AC3 at the pure layer: a hazard beside the escape path bends the escape
    /// steering while the leg — the caller's pass state — is untouched. The arm
    /// takes the leg as an INPUT and returns only actuator scalars, so avoidance
    /// has no channel through which it could change the pass at all.
    #[test]
    fn hazard_bends_the_escape_without_changing_the_leg() {
        let none: [AiWorldEntity; 0] = [];
        let clear = plan_fly_through_pass(&pass_input(FlyThroughLeg::Escape, &none));
        assert_eq!(clear.1, 0.0, "nothing to avoid: dead-ahead escape, no yaw");

        // A rock just off the projected escape path (10 u/s * 3 s look-ahead
        // puts our projection at z = -30).
        let rock = [AiWorldEntity {
            uuid: Uuid::new_v4(),
            position: [3.0, 0.0, -30.0],
            radius: 2.0,
            size_rating: 2.0,
            ..Default::default()
        }];
        let bent = plan_fly_through_pass(&pass_input(FlyThroughLeg::Escape, &rock));
        assert!(
            bent.1.abs() > 0.0,
            "a hazard on the escape path must bend the escape steering"
        );
        assert_eq!(
            bent.0, clear.0,
            "avoidance bends the heading, it does not change the leg's throttle"
        );
    }

    /// Issue #788, AC7: the re-entry pivot tracks the target exactly as the
    /// inbound leg does, but flies the authored re-engage throttle. With the
    /// destroyer's authored `0.0` that is a cut-thrust turn — the observable
    /// difference between "turning to start a pass" and "running the pass".
    #[test]
    fn reengage_leg_tracks_the_target_on_the_authored_reengage_throttle() {
        let none: [AiWorldEntity; 0] = [];
        let inbound = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
        let pivot = plan_fly_through_pass(&pass_input(FlyThroughLeg::Reengage, &none));
        assert_eq!(
            pivot.1, inbound.1,
            "the pivot IS the tracking solution: same steering as the inbound leg"
        );
        assert_eq!(pivot.0, 0.0, "and it cuts thrust to make the turn");

        // The throttle is the authored scalar, not a hardcoded zero.
        let mut powered = pass_input(FlyThroughLeg::Reengage, &none);
        powered.reengage_speed = 0.4;
        assert!((plan_fly_through_pass(&powered).0 - 0.4).abs() < 1e-6);
    }

    /// Issue #791: the torpedo-opportunity hold tracks the target's LIVE
    /// position (so a manoeuvring target keeps being followed onto the bow) and
    /// flies its OWN authored throttle — not the re-engage one, and not the
    /// approach one.
    ///
    /// The throttle half is the load-bearing assertion. The fixture authors
    /// `reengage_speed = 0.0` and `torpedo_bearing_speed = 0.25` precisely so a
    /// leg that quietly took the wrong scalar would look like a cut-thrust turn
    /// and pass every other check here.
    #[test]
    fn torpedo_bearing_leg_tracks_the_live_target_on_its_own_authored_throttle() {
        let none: [AiWorldEntity; 0] = [];
        let inbound = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
        let hold = plan_fly_through_pass(&pass_input(FlyThroughLeg::TorpedoBearing, &none));
        assert_eq!(
            hold.1, inbound.1,
            "the hold IS a tracking solution: same steering as the inbound leg"
        );
        assert!(
            (hold.0 - 0.25).abs() < 1e-6,
            "the throttle is `torpedo_bearing_speed`, not `reengage_speed` (0.0) \
             and not `approach_speed` (0.85), got {}",
            hold.0
        );

        // A target that MOVES to the other side flips the commanded turn: the
        // solution is re-derived from the live position every call, which is
        // what separates this leg from the frozen-heading escape.
        let mut port = pass_input(FlyThroughLeg::TorpedoBearing, &none);
        port.target_pos = [-60.0, 0.0, -60.0];
        assert!(
            plan_fly_through_pass(&port).1 < 0.0 && hold.1 > 0.0,
            "moving the target across the bow must reverse the commanded turn"
        );

        // An authored cut-thrust hold is a real authored value, not a default.
        let mut cut = pass_input(FlyThroughLeg::TorpedoBearing, &none);
        cut.torpedo_bearing_speed = 0.0;
        assert_eq!(plan_fly_through_pass(&cut).0, 0.0);
    }

    // ── The artillery firing position (issue #792) ───────────────────────────

    /// A battleship at the origin facing `-Z`, holding station on a target 180
    /// units dead ahead. `hold_speed` is deliberately NON-zero here so the tests
    /// below pin "the authored throttle" rather than accidentally agreeing with a
    /// hardcoded stop.
    fn artillery_input(entities: &[AiWorldEntity]) -> ArtilleryPositionInput<'_> {
        ArtilleryPositionInput {
            self_pos: [0.0, 0.0, 0.0],
            self_yaw: 0.0,
            self_speed: 12.0,
            self_radius: 3.0,
            target_pos: [0.0, 0.0, -180.0],
            target_yaw: None,
            target_speed: 0.0,
            target_uuid: Uuid::from_u128(9),
            hold_speed: 0.15,
            projectile_speed: 35.0,
            tracking_deadband_rad: 0.03,
            tracking_full_steer_rad: 0.6,
            entities,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        }
    }

    /// AC3/AC4: the hold flies its OWN authored throttle, and the facing is a
    /// PREDICTIVE intercept rather than a bearing to the target.
    #[test]
    fn artillery_position_leads_a_crossing_target_on_its_authored_throttle() {
        let none: [AiWorldEntity; 0] = [];

        // A stationary target is the degenerate case: lead nothing, and with the
        // bow already on it command no turn.
        let still = plan_artillery_position(&artillery_input(&none));
        assert!(
            (still.0 - 0.15).abs() < 1e-6,
            "the throttle is the authored `hold_speed`, got {}",
            still.0
        );
        assert_eq!(
            still.1, 0.0,
            "a stationary target dead ahead needs no correction"
        );

        // Crossing square across the line of sight, at a named heading and speed.
        let crossing = |yaw: f32, bolt: f32| {
            let mut input = artillery_input(&none);
            input.target_yaw = Some(yaw);
            input.target_speed = 24.0;
            input.projectile_speed = bolt;
            plan_artillery_position(&input)
        };

        // To starboard (+X): the aim point moves ahead of it, so the commanded
        // turn is to starboard.
        let led = crossing(std::f32::consts::FRAC_PI_2, 35.0);
        assert!(
            led.1 > 0.0,
            "the bow must turn toward where the target is GOING, got {}",
            led.1
        );

        // ...and the other way, so the sign follows the target rather than being
        // a fixed bias.
        assert!(crossing(-std::f32::consts::FRAC_PI_2, 35.0).1 < 0.0);

        // The lead is the FLIGHT TIME's doing: a bolt that arrives instantly has
        // nothing to lead by, and the same crossing target then commands no turn
        // at all. This is the assertion that would fail if the leg quietly
        // tracked the live position and got its sign right by luck.
        assert!(
            crossing(std::f32::consts::FRAC_PI_2, 100_000.0).1.abs() < 1e-3,
            "a bolt with no flight time has nothing to lead by"
        );

        // A target whose heading is unknown contributes no velocity: the solution
        // degrades to "aim at where it is" rather than inventing a course.
        let mut unknown = artillery_input(&none);
        unknown.target_speed = 24.0;
        assert_eq!(plan_artillery_position(&unknown).1, 0.0);

        // ...as does an unresolvable lead speed, which is what a hull carrying no
        // artillery bank at all publishes.
        assert_eq!(
            crossing(std::f32::consts::FRAC_PI_2, 0.0).1,
            0.0,
            "no flight speed must fall back to the target's live bearing, not to \
             whichever way the hull happened to be pointing"
        );
    }

    /// AC6, the additive half: a hazard BENDS the intercept facing and changes
    /// nothing else. The thrust is untouched, so avoidance can never turn the
    /// hold into a translation, and the bend is a sum rather than a substitution
    /// — the leg still knows where its target is.
    #[test]
    fn artillery_position_folds_avoidance_onto_the_intercept_facing() {
        let none: [AiWorldEntity; 0] = [];
        let clean = plan_artillery_position(&artillery_input(&none));

        let obstacle = [AiWorldEntity {
            uuid: Uuid::from_u128(77),
            // On the hull's projected path, off the starboard bow.
            position: [6.0, 0.0, -34.0],
            radius: 8.0,
            size_rating: 8.0,
            dangerous: true,
            ..Default::default()
        }];
        let bent = plan_artillery_position(&artillery_input(&obstacle));

        assert!(
            bent.1 < clean.1,
            "an obstacle off the starboard bow must push the facing to port \
             ({} vs {})",
            bent.1,
            clean.1
        );
        assert_eq!(
            bent.0, clean.0,
            "and it must not touch the throttle: a hold that accelerated around a \
             rock would be flying, not holding"
        );

        // The target itself is excluded from the scan — a hull deliberately
        // holding station on a ship must not treat it as something to swerve
        // around.
        let target_as_obstacle = [AiWorldEntity {
            uuid: Uuid::from_u128(9),
            position: [0.0, 0.0, -180.0],
            radius: 400.0,
            size_rating: 400.0,
            dangerous: true,
            ..Default::default()
        }];
        assert_eq!(
            plan_artillery_position(&artillery_input(&target_as_obstacle)).1,
            clean.1,
            "the target must be excluded from the avoidance scan"
        );
    }

    // ── The shield-recovery standoff orbit (issue #788) ──────────────────────

    fn orbit_input(entities: &[AiWorldEntity]) -> RecoveryOrbitInput<'_> {
        RecoveryOrbitInput {
            self_pos: [0.0, 0.0, 0.0],
            self_yaw: 0.0,
            self_speed: 10.0,
            self_radius: 1.0,
            // Target dead ahead at 200; the ring below sits at 200 too, so the
            // default fixture starts exactly ON the ring.
            target_pos: [0.0, 0.0, -200.0],
            target_uuid: Uuid::nil(),
            safe_range: 200.0,
            orbit_direction: 1.0,
            spiral_gain: 1.2,
            orbit_speed: 0.7,
            tracking_deadband_rad: 0.02,
            tracking_full_steer_rad: 0.5,
            entities,
            avoidance_buffer: AVOIDANCE_BUFFER,
            avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        }
    }

    /// The heading the orbit commands, in world radians, recovered from the
    /// steering it demands. Only meaningful for the fixture's yaw of 0 and a
    /// non-saturated turn, which is why the tests below check the SIGN of the
    /// steering rather than reconstructing angles.
    fn orbit_steer(input: &RecoveryOrbitInput) -> f32 {
        plan_recovery_orbit(input).1
    }

    /// AC3, the core claim: on the ring the ship flies the TANGENT — it neither
    /// closes nor opens. With the target dead ahead and the ring at the current
    /// range, a tangential course is a hard turn, not "carry on" and not "stop".
    #[test]
    fn on_the_ring_the_orbit_flies_the_tangent() {
        let none: [AiWorldEntity; 0] = [];
        let input = orbit_input(&none);
        let (thrust, steering) = plan_recovery_orbit(&input);
        assert!(
            (thrust - 0.7).abs() < 1e-6,
            "the ring is flown at the authored orbit throttle, not coasted"
        );
        assert!(
            steering.abs() > 0.9,
            "a target dead ahead means the tangent is 90 degrees off the bow: \
             the orbit must command a hard turn, got {steering}"
        );
    }

    /// AC3's "spirals rather than stopping or retreating indefinitely".
    ///
    /// The ship is pointed ALONG the pure tangent, so the steering the orbit
    /// demands is exactly the spiral correction and nothing else: zero means
    /// "hold the ring", and the sign says which way the correction bends. With
    /// the target dead ahead of the ring's centre and a starboard-hand orbit, a
    /// positive demand turns further off the target (opening the range) and a
    /// negative one turns back onto it (closing).
    #[test]
    fn the_orbit_spirals_outward_when_inside_and_inward_when_outside() {
        let none: [AiWorldEntity; 0] = [];
        // Facing +X: for a target dead astern-of-ring at -Z, that is the pure
        // starboard-hand tangent.
        let tangent_yaw = std::f32::consts::FRAC_PI_2;

        let mut on_ring = orbit_input(&none);
        on_ring.self_yaw = tangent_yaw;
        assert_eq!(
            plan_recovery_orbit(&on_ring).1,
            0.0,
            "already on the ring and already on the tangent: no correction at all"
        );

        let mut inside = orbit_input(&none);
        inside.self_yaw = tangent_yaw;
        inside.target_pos = [0.0, 0.0, -60.0]; // range 60 vs a 200 ring
        let inside_steer = plan_recovery_orbit(&inside).1;

        let mut outside = orbit_input(&none);
        outside.self_yaw = tangent_yaw;
        outside.target_pos = [0.0, 0.0, -600.0]; // range 600 vs a 200 ring
        let outside_steer = plan_recovery_orbit(&outside).1;

        assert!(
            inside_steer > 0.0,
            "inside the ring the orbit must bend AWAY from the target and work \
             its way out, got {inside_steer}"
        );
        assert!(
            outside_steer < 0.0,
            "outside the ring it must bend BACK toward the target rather than \
             running away indefinitely, got {outside_steer}"
        );
        // And it never stops: the throttle is the same on the ring and off it.
        assert_eq!(
            plan_recovery_orbit(&inside).0,
            plan_recovery_orbit(&on_ring).0
        );
        assert_eq!(
            plan_recovery_orbit(&outside).0,
            plan_recovery_orbit(&on_ring).0
        );
    }

    /// The gain is fractional, so the same authored value produces the same
    /// correction for a small ring and a large one. Without this a designer
    /// would have to re-tune `orbit_spiral_gain` every time a weapon's range
    /// changed, which is exactly the coupling the safe ring exists to avoid.
    #[test]
    fn the_spiral_correction_is_scale_free() {
        let none: [AiWorldEntity; 0] = [];
        let mut small = orbit_input(&none);
        small.safe_range = 80.0;
        small.target_pos = [0.0, 0.0, -40.0]; // 50% of the ring
        let mut large = orbit_input(&none);
        large.safe_range = 800.0;
        large.target_pos = [0.0, 0.0, -400.0]; // also 50% of the ring
        assert!(
            (orbit_steer(&small) - orbit_steer(&large)).abs() < 1e-5,
            "the same FRACTIONAL error must produce the same correction: {} vs {}",
            orbit_steer(&small),
            orbit_steer(&large)
        );
    }

    /// The circulation direction is an input, and reversing it reverses the turn
    /// — which is what makes a seeded ±1 a meaningful choice rather than
    /// decoration.
    #[test]
    fn reversing_the_orbit_direction_reverses_the_turn() {
        let none: [AiWorldEntity; 0] = [];
        let mut cw = orbit_input(&none);
        cw.orbit_direction = 1.0;
        let mut ccw = orbit_input(&none);
        ccw.orbit_direction = -1.0;
        let (a, b) = (orbit_steer(&cw), orbit_steer(&ccw));
        assert!(
            a * b < 0.0,
            "the two directions must turn opposite ways, got {a} and {b}"
        );
    }

    /// AC3 again, at the pure layer: a hazard bends the orbit the same way it
    /// bends the escape, and the throttle is untouched.
    #[test]
    fn hazard_bends_the_orbit_without_changing_its_throttle() {
        let none: [AiWorldEntity; 0] = [];
        // Put the ship well inside the ring so the commanded course is nearly
        // straight ahead and a rock ahead of it is genuinely in the way.
        let mut clear_input = orbit_input(&none);
        clear_input.target_pos = [200.0, 0.0, 0.0];
        clear_input.safe_range = 200.0;
        let clear = plan_recovery_orbit(&clear_input);

        let rock = [AiWorldEntity {
            uuid: Uuid::new_v4(),
            position: [3.0, 0.0, -30.0],
            radius: 2.0,
            size_rating: 2.0,
            ..Default::default()
        }];
        let mut bent_input = orbit_input(&rock);
        bent_input.target_pos = [200.0, 0.0, 0.0];
        bent_input.safe_range = 200.0;
        let bent = plan_recovery_orbit(&bent_input);

        assert_ne!(
            bent.1, clear.1,
            "a hazard on the orbit path must bend the orbit steering"
        );
        assert_eq!(
            bent.0, clear.0,
            "avoidance bends the heading, never the leg's throttle"
        );
    }

    /// Degenerate geometry must not produce NaN steering: sitting on top of the
    /// target, and an un-derivable ring, both hold the current heading.
    #[test]
    fn degenerate_orbit_geometry_holds_the_current_heading() {
        let none: [AiWorldEntity; 0] = [];
        let mut on_top = orbit_input(&none);
        on_top.target_pos = on_top.self_pos;
        let (thrust, steering) = plan_recovery_orbit(&on_top);
        assert!(steering.is_finite() && thrust.is_finite());
        assert_eq!(
            steering, 0.0,
            "already on the held heading: no turn demanded"
        );

        let mut no_ring = orbit_input(&none);
        no_ring.safe_range = 0.0;
        let (_, steering) = plan_recovery_orbit(&no_ring);
        assert!(steering.is_finite());
        assert_eq!(steering, 0.0);
    }
}
