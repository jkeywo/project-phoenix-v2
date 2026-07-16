/// Pure AI module — no Bevy imports.
///
/// Contains navigation utilities (`steer_toward`, `avoidance_steering`),
/// per-system operate functions (`operate_helm`, `operate_weapons`),
/// the `AiMemory` private-reasoning state, and the `CaptainAi` helper.
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
/// (nav_goal) fallthrough when the entity has no `[behaviour]` section to
/// author one. Parse-time default only — see
/// [`crate::entity_config::BehaviourConfig::nav_handoff_speed`], whose serde
/// default reads this constant so the two cannot drift apart.
pub const NAV_HANDOFF_SPEED: f32 = 0.6;
const AVOIDANCE_MIN_SPEED: f32 = 0.25;
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

// ── AiMemory ──────────────────────────────────────────────────────────────────

/// Private per-entity reasoning state that persists across ticks.
///
/// Replaces the old 5-slot `Blackboard`; contains only state that cannot be
/// derived from the published blackboards or world registry (chosen target,
/// last attacker, home position, waypoint cursor). Combat timers live on the
/// viewscreen blackboard (issue #572).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiMemory {
    /// UUID of the entity currently selected as the attack/pursuit target.
    pub target: Option<Uuid>,
    /// UUID of the last entity to damage this AI entity.
    pub last_attacker: Option<Uuid>,
    /// World-space spawn position — used as the fallback retreat anchor.
    pub home_position: [f32; 3],
    /// Current index into the active patrol waypoint list.
    pub waypoint_index: usize,
    /// Channel-3 Navigation-to-Helm handoff steer target (issue #681).
    /// Set when Navigation AI sends `NavigateTo`; consumed by `operate_helm`
    /// as a fallthrough when no local Helm-relevant objective resolves.
    /// Cleared on arrival (`WAYPOINT_ARRIVAL_RADIUS`) or when a local
    /// objective (Destroy / Patrol / Reach) resolves to a non-None result.
    pub nav_goal: Option<[f32; 2]>,
}

// ── WorldView ─────────────────────────────────────────────────────────────────

/// A visible entity in the AI's world view.
#[derive(Debug, Clone, Default)]
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

// ── operate_helm ─────────────────────────────────────────────────────────────

/// Per-system operate function for the Helm.
///
/// Reads the scored objective pool, selects the top-scoring directive the Helm
/// can serve (Patrol, Destroy, Reach), and returns `(thrust, steering)`.
///
/// Mutates `memory` to track the current waypoint index and selected target.
pub fn operate_helm(
    memory: &mut AiMemory,
    world_view: &WorldView,
    scored_pool: &[crate::messages::ScoredObjective],
    doctrine: &[crate::entity_config::DoctrineObjective],
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    waypoint_arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    faction_registry: &crate::faction::FactionRegistry,
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
            AiDirective::Destroy { target } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.8);
                let maintain_range = cfg.map(|d| d.maintain_range).unwrap_or(25.0);
                helm_destroy(
                    memory,
                    world_view,
                    anchors,
                    avoidance_buffer,
                    avoidance_look_ahead_secs,
                    forward_speed,
                    faction_registry,
                    Some(target.as_str()),
                    target_speed,
                    maintain_range,
                )
            }
            AiDirective::Patrol {
                anchors: waypoints,
                loop_path,
            } => {
                let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.5);
                helm_patrol(
                    memory,
                    world_view,
                    waypoints,
                    *loop_path,
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
                        memory,
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
                // Resolve the retreat anchor by name, falling back to the ship's
                // home/spawn position when the anchor is empty or unknown. The
                // synthetic hull-triggered Retreat carries an empty anchor
                // (see `aggregate_doctrine_blackboards`), so this fallback is
                // what makes it steer back toward spawn. Retreat therefore
                // always resolves, so it returns `Some(..)` rather than falling
                // through to lower-priority directives.
                let pos = anchors
                    .get(anchor.as_str())
                    .copied()
                    .unwrap_or(memory.home_position);
                helm_navigate_to(
                    memory,
                    world_view,
                    pos,
                    waypoint_arrival_radius,
                    avoidance_buffer,
                    avoidance_look_ahead_secs,
                    forward_speed,
                    target_speed,
                )
            }
            _ => None,
        };
        if let Some(result) = result {
            // Local objective resolved — clear the nav_goal handoff so the
            // ship doesn't blend two conflicting targets (issue #681).
            memory.nav_goal = None;
            return result;
        }
    }

    // Fall through to Channel-3 Navigation handoff (issue #681).
    // When no Helm-relevant objective resolved and Navigation has given us a
    // long-range steer target, navigate toward it. This lets a Navigation AI
    // guide a short-range Helm toward an objective the Helm cannot yet see.
    if let Some([nx, nz]) = memory.nav_goal {
        if let Some(result) = helm_navigate_to(
            memory,
            world_view,
            [nx, 0.0, nz],
            waypoint_arrival_radius,
            avoidance_buffer,
            avoidance_look_ahead_secs,
            forward_speed,
            nav_handoff_speed,
        ) {
            if result == (0.0, 0.0) {
                memory.nav_goal = None; // arrived
            }
            return result;
        }
        memory.nav_goal = None; // stale / invalid target
    }

    (0.0, 0.0)
}

/// Helm execute: pursue/attack using `memory.target` (or discover a new hostile).
fn helm_destroy(
    memory: &mut AiMemory,
    world_view: &WorldView,
    _anchors: &std::collections::HashMap<String, [f32; 3]>,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    faction_registry: &crate::faction::FactionRegistry,
    directive_target: Option<&str>,
    target_speed: f32,
    maintain_range: f32,
) -> Option<(f32, f32)> {
    // Validate / refresh target.
    let target_uuid =
        resolve_destroy_target(memory, world_view, faction_registry, directive_target)?;
    memory.target = Some(target_uuid);

    let Some(target_entity) = world_view.entities.iter().find(|e| e.uuid == target_uuid) else {
        memory.target = None;
        return None;
    };

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

/// Resolve which target to attack: existing (if still visible) > last_attacker > nearest hostile.
fn resolve_destroy_target(
    memory: &AiMemory,
    world_view: &WorldView,
    faction_registry: &crate::faction::FactionRegistry,
    directive_target: Option<&str>,
) -> Option<Uuid> {
    if let Some(target) = directive_target.filter(|t| !t.is_empty()) {
        return resolve_objective_target(target, world_view);
    }

    // Prefer current target if still in world view.
    if let Some(t) = memory.target {
        if world_view.entities.iter().any(|e| e.uuid == t) {
            return Some(t);
        }
    }
    // Fall back to last attacker if visible.
    if let Some(la) = memory.last_attacker {
        if world_view.entities.iter().any(|e| e.uuid == la) {
            return Some(la);
        }
    }
    // Scan for nearest hostile.
    find_nearest_hostile(world_view, faction_registry)
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
/// The single definition of "who is the enemy, and which one is closest",
/// shared by the helm path (`resolve_destroy_target`) and the weapons path
/// (`ai_target_selection`'s nearest-hostile tier, issue #703). Both must agree
/// — a helm that closes on one ship while weapons locks another is a bug — so
/// neither may grow a second hostile scan.
///
/// Distance is measured in the XZ plane (see [`dist_sq`]), matching the
/// range checks both callers gate on.
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

/// Helm execute: patrol waypoints in order, advancing when each is reached.
fn helm_patrol(
    memory: &mut AiMemory,
    world_view: &WorldView,
    waypoints: &[String],
    loop_path: bool,
    waypoint_arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
    target_speed: f32,
    anchors: &std::collections::HashMap<String, [f32; 3]>,
) -> Option<(f32, f32)> {
    if waypoints.is_empty() {
        return None;
    }

    // Clamp index.
    if memory.waypoint_index >= waypoints.len() {
        if loop_path {
            memory.waypoint_index = 0;
        } else {
            return Some((0.0, 0.0));
        }
    }

    let waypoint_name = &waypoints[memory.waypoint_index];
    let &wp_pos = anchors.get(waypoint_name.as_str())?;

    let pos = world_view.entity_pos;
    let dx = wp_pos[0] - pos[0];
    let dz = wp_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    // Arrived — advance waypoint.
    if dist < waypoint_arrival_radius {
        if memory.waypoint_index + 1 < waypoints.len() {
            memory.waypoint_index += 1;
        } else if loop_path {
            memory.waypoint_index = 0;
        } else {
            return Some((0.0, 0.0));
        }
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

/// Helm execute: navigate to a fixed position (for Reach directives).
fn helm_navigate_to(
    _memory: &mut AiMemory,
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

// ── operate_weapons ───────────────────────────────────────────────────────────

/// Per-system operate function for Weapons.
///
/// Returns `(target_uuid, should_fire)` — the caller emits `SetTarget` and
/// `FirePhaser` InboundMessages with the AI token as needed.
///
/// Selects the top-scoring Destroy directive (score > 0, Weapons relevance)
/// and targets whoever `memory.target` points to (or falls back to
/// `memory.last_attacker` when no current target is set).
pub fn operate_weapons(
    memory: &AiMemory,
    world_view: &WorldView,
    scored_pool: &[crate::messages::ScoredObjective],
    faction_registry: &crate::faction::FactionRegistry,
) -> (Option<Uuid>, bool) {
    use crate::messages::SystemAffinity;

    // Find top Destroy directive with Weapons relevance and positive score.
    let top_destroy = scored_pool
        .iter()
        .find(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Weapons));
    let Some(top_destroy) = top_destroy else {
        return (None, false);
    };
    let directive_target = match &top_destroy.directive {
        crate::messages::AiDirective::Destroy { target } => Some(target.as_str()),
        _ => return (None, false),
    };

    // Resolve target: explicit directive target, or current target →
    // last attacker → nearest hostile for standing "destroy hostiles" doctrine.
    let target = resolve_destroy_target(memory, world_view, faction_registry, directive_target);

    let Some(t) = target else {
        return (None, false);
    };

    // Only fire when phaser is ready.
    if !world_view.entity_phaser_ready {
        return (Some(t), false);
    }

    (Some(t), true)
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

// ── operate_lateral_thrust ────────────────────────────────────────────────────

/// Per-system operate function for the Helm Lateral Thrust system.
///
/// Returns a lateral input value in `[-1.0, 1.0]` for obstacle avoidance.
/// A positive value pushes the ship to starboard; a negative value pushes to port.
/// The AI checks all visible entities in `world_view` for potential collisions
/// and applies lateral thrust to dodge the nearest threat.
///
/// Returns `0.0` when no avoidance is needed or no suitable objective is active.
pub fn operate_lateral_thrust(
    world_view: &WorldView,
    scored_pool: &[crate::messages::ScoredObjective],
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    forward_speed: f32,
) -> f32 {
    use crate::messages::SystemAffinity;

    // Only dodge when a helm-relevant objective is active.
    let has_helm_objective = scored_pool
        .iter()
        .any(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm));
    if !has_helm_objective {
        return 0.0;
    }

    if world_view.entities.is_empty() {
        return 0.0;
    }

    let self_pos = world_view.entity_pos;
    let self_yaw = world_view.entity_yaw;
    let self_radius = world_view.self_radius;

    // Find the nearest threat within avoidance range.
    // A "threat" is any entity with a nonzero radius that could collide.
    let fwd_x = self_yaw.sin();
    let fwd_z = -self_yaw.cos();

    let mut best_threat = 0.0_f32;
    let mut best_sign = 0.0_f32;

    for entity in &world_view.entities {
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;

        // Project both entities forward.
        let proj_self_x = self_pos[0] + fwd_x * forward_speed * avoidance_look_ahead_secs;
        let proj_self_z = self_pos[2] + fwd_z * forward_speed * avoidance_look_ahead_secs;

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

        if proj_dist < avoidance_radius && proj_dist > 0.01 {
            let threat_fraction = 1.0 - (proj_dist / avoidance_radius);
            let to_x = ent_proj_x - proj_self_x;
            let to_z = ent_proj_z - proj_self_z;
            let cross = fwd_x * to_z - fwd_z * to_x;

            // Cross product sign: positive = threat is to the left → dodge right (+).
            let sign = if cross >= 0.0 { 1.0 } else { -1.0 };

            if threat_fraction > best_threat {
                best_threat = threat_fraction;
                best_sign = sign;
            }
        }
    }

    best_sign * best_threat
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── AiMemory ──────────────────────────────────────────────────────────

    #[test]
    fn ai_memory_default_has_no_target() {
        let m = AiMemory::default();
        assert!(m.target.is_none());
    }

    #[test]
    fn ai_memory_default_has_no_last_attacker() {
        let m = AiMemory::default();
        assert!(m.last_attacker.is_none());
    }

    #[test]
    fn ai_memory_default_home_is_origin() {
        let m = AiMemory::default();
        assert_eq!(m.home_position, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn ai_memory_default_waypoint_index_is_zero() {
        let m = AiMemory::default();
        assert_eq!(m.waypoint_index, 0);
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

    fn empty_registry() -> crate::faction::FactionRegistry {
        crate::faction::FactionRegistry::new()
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
        let mut memory = AiMemory::default();
        let world = world_at_origin();
        let pool = patrol_pool();
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha();

        let (thrust, _steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(thrust > 0.0, "should thrust toward waypoint");
    }

    #[test]
    fn operate_helm_empty_pool_returns_zero() {
        let mut memory = AiMemory::default();
        let world = world_at_origin();
        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &[],
            &[],
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert_eq!(thrust, 0.0);
        assert_eq!(steering, 0.0);
    }

    #[test]
    fn operate_helm_zeroed_pool_returns_zero() {
        let mut memory = AiMemory::default();
        let world = world_at_origin();
        let mut pool = patrol_pool();
        pool[0].score = 0.0; // zero-gated
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha();

        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert_eq!(thrust, 0.0, "zero-gated pool must produce no thrust");
        assert_eq!(steering, 0.0);
    }

    #[test]
    fn operate_helm_advances_waypoint_on_arrival() {
        let mut memory = AiMemory::default();
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("wp0".into(), [0.0, 0.0, 0.0]); // at origin = already arrived
        anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);

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

        let world = world_at_origin();
        operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert_eq!(
            memory.waypoint_index, 1,
            "should advance past arrived waypoint"
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
        let mut memory = AiMemory::default();
        let world = world_at_origin(); // entities list is empty → wave_1 not found
        let anchors = anchors_with_alpha();
        let pool = destroy_then_patrol_pool(&anchors);

        let (thrust, _steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &[],
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust > 0.0,
            "should fall through to Patrol and produce thrust when Destroy target is unresolvable"
        );
    }

    #[test]
    fn operate_helm_uses_destroy_when_target_exists_not_patrol() {
        // Regression guard: when the Destroy target IS in the world snapshot,
        // the ship must pursue that target and NOT fall through to Patrol.
        let target_uuid = Uuid::new_v4();
        let mut memory = AiMemory::default();
        let mut anchors = anchors_with_alpha();
        // Place the patrol anchor directly ahead so patrol would also produce
        // positive thrust — this confirms Destroy wins by checking the memory
        // target, not just thrust direction.
        anchors.insert("alpha".into(), [100.0, 0.0, 0.0]);

        // Target is far away in a different direction (to the side) so the
        // Destroy path steers differently from the patrol path.
        let world = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![crate::ai::AiWorldEntity {
                uuid: target_uuid,
                name: Some("wave_1".into()),
                position: [0.0, 0.0, -200.0], // behind the ship
                ..Default::default()
            }],
            ..Default::default()
        };

        let pool = vec![
            crate::messages::ScoredObjective {
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
        ];

        operate_helm(
            &mut memory,
            &world,
            &pool,
            &[],
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );

        // The Destroy path sets memory.target to the resolved UUID.
        assert_eq!(
            memory.target,
            Some(target_uuid),
            "memory.target must be set by Destroy path, not left None by Patrol"
        );
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
        let mut memory = AiMemory::default();
        let world = world_at_origin();
        let mut anchors = std::collections::HashMap::new();
        anchors.insert("rally".to_string(), [100.0, 0.0, 0.0]);
        let pool = retreat_pool("rally", 50.0);

        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &[],
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(thrust > 0.0, "Retreat should thrust toward the anchor");
        assert!(
            steering > 0.0,
            "Retreat anchor to the right must give positive steering"
        );
    }

    #[test]
    fn operate_helm_retreat_falls_back_to_home_position_when_anchor_empty() {
        // Regression: the synthetic hull-triggered Retreat carries an empty
        // anchor (see aggregate_doctrine_blackboards). With no matching anchor
        // in the map, operate_helm must fall back to memory.home_position and
        // still steer toward it (never falling through to idle). home_position
        // is at [100, 0, 0] — to the right → positive steering.
        let mut memory = AiMemory {
            home_position: [100.0, 0.0, 0.0],
            ..Default::default()
        };
        let world = world_at_origin();
        let anchors = std::collections::HashMap::new(); // empty → no match
        let pool = retreat_pool("", 50.0);

        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &[],
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust > 0.0,
            "Retreat with empty anchor should thrust toward home_position"
        );
        assert!(
            steering > 0.0,
            "home_position to the right must give positive steering"
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
        let mut memory = AiMemory::default();
        let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
        let (thrust, _) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
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
        let mut memory = AiMemory::default();
        let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
        let (thrust, _) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
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
        let mut memory = AiMemory::default();
        let (pool, doctrine) = destroy_pool_for("enemy", 0.8, maintain_range);
        let (thrust, _) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
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
        let mut memory = AiMemory::default();
        let (pool, doctrine) = destroy_pool_for("enemy", 0.8, 25.0);

        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
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
        let mut memory = AiMemory::default();
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

        let (thrust, steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );

        assert_eq!(thrust, 0.0);
        assert_eq!(
            steering, 0.0,
            "resolved Destroy should hold station instead of falling through to Patrol"
        );
    }

    // ── operate_weapons ───────────────────────────────────────────────────

    fn destroy_pool_with_score(score: f32) -> Vec<crate::messages::ScoredObjective> {
        destroy_pool_with_target(score, "")
    }

    fn destroy_pool_with_target(score: f32, target: &str) -> Vec<crate::messages::ScoredObjective> {
        vec![crate::messages::ScoredObjective {
            id: "destroy".into(),
            score,
            directive: crate::messages::AiDirective::Destroy {
                target: target.into(),
            },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![
                crate::messages::SystemAffinity::Helm,
                crate::messages::SystemAffinity::Weapons,
                crate::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "destroy".into(),
                text: "".into(),
                mandatory: false,
                status: crate::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    #[test]
    fn operate_weapons_returns_target_and_fire_when_ready_and_target_visible() {
        let target_id = Uuid::new_v4();
        let memory = AiMemory {
            target: Some(target_id),
            ..Default::default()
        };
        let world = WorldView {
            entity_phaser_ready: true,
            entities: vec![AiWorldEntity {
                uuid: target_id,
                ..Default::default()
            }],
            ..Default::default()
        };
        let pool = destroy_pool_with_score(35.0);
        let (t, fire) = operate_weapons(&memory, &world, &pool, &empty_registry());
        assert_eq!(t, Some(target_id));
        assert!(fire);
    }

    #[test]
    fn operate_weapons_no_fire_when_phaser_not_ready() {
        let target_id = Uuid::new_v4();
        let memory = AiMemory {
            target: Some(target_id),
            ..Default::default()
        };
        let world = WorldView {
            entity_phaser_ready: false,
            entities: vec![AiWorldEntity {
                uuid: target_id,
                ..Default::default()
            }],
            ..Default::default()
        };
        let pool = destroy_pool_with_score(35.0);
        let (t, fire) = operate_weapons(&memory, &world, &pool, &empty_registry());
        assert_eq!(t, Some(target_id), "should still select target");
        assert!(!fire, "must not fire when phaser not ready");
    }

    #[test]
    fn operate_weapons_zero_gated_destroy_returns_no_fire() {
        let target_id = Uuid::new_v4();
        let memory = AiMemory {
            target: Some(target_id),
            ..Default::default()
        };
        let world = WorldView {
            entity_phaser_ready: true,
            entities: vec![AiWorldEntity {
                uuid: target_id,
                ..Default::default()
            }],
            ..Default::default()
        };
        let pool = destroy_pool_with_score(0.0); // zero-gated
        let (_t, fire) = operate_weapons(&memory, &world, &pool, &empty_registry());
        assert!(!fire, "zero-gated destroy must not fire");
    }

    #[test]
    fn operate_weapons_falls_back_to_last_attacker_when_no_target() {
        let attacker_id = Uuid::new_v4();
        let memory = AiMemory {
            target: None,
            last_attacker: Some(attacker_id),
            ..Default::default()
        };
        let world = WorldView {
            entity_phaser_ready: true,
            entities: vec![AiWorldEntity {
                uuid: attacker_id,
                ..Default::default()
            }],
            ..Default::default()
        };
        let pool = destroy_pool_with_score(35.0);
        let (t, fire) = operate_weapons(&memory, &world, &pool, &empty_registry());
        assert_eq!(t, Some(attacker_id));
        assert!(fire);
    }

    #[test]
    fn operate_weapons_prefers_named_destroy_target_over_nearest_hostile() {
        let named_id = Uuid::new_v4();
        let nearer_hostile = Uuid::new_v4();
        let hostile_faction = Uuid::new_v4();
        let self_faction = Uuid::new_v4();
        let mut registry = crate::faction::FactionRegistry::new();
        registry.add_enemy(self_faction, hostile_faction);
        let memory = AiMemory::default();
        let world = WorldView {
            entity_phaser_ready: true,
            self_faction: Some(self_faction),
            entities: vec![
                AiWorldEntity {
                    uuid: nearer_hostile,
                    faction: Some(hostile_faction),
                    position: [1.0, 0.0, 0.0],
                    ..Default::default()
                },
                AiWorldEntity {
                    uuid: named_id,
                    name: Some("wave_1".into()),
                    faction: Some(hostile_faction),
                    position: [100.0, 0.0, 0.0],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let pool = destroy_pool_with_target(35.0, "wave_1");

        let (target, fire) = operate_weapons(&memory, &world, &pool, &registry);

        assert_eq!(target, Some(named_id));
        assert!(fire);
    }

    #[test]
    fn operate_weapons_ignores_missing_named_destroy_target() {
        let hostile_id = Uuid::new_v4();
        let hostile_faction = Uuid::new_v4();
        let self_faction = Uuid::new_v4();
        let mut registry = crate::faction::FactionRegistry::new();
        registry.add_enemy(self_faction, hostile_faction);
        let memory = AiMemory::default();
        let world = WorldView {
            entity_phaser_ready: true,
            self_faction: Some(self_faction),
            entities: vec![AiWorldEntity {
                uuid: hostile_id,
                name: Some("other_wave".into()),
                faction: Some(hostile_faction),
                ..Default::default()
            }],
            ..Default::default()
        };
        let pool = destroy_pool_with_target(35.0, "wave_1");

        let (target, fire) = operate_weapons(&memory, &world, &pool, &registry);

        assert_eq!(target, None);
        assert!(!fire);
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

    // ── nav_goal (issue #681) ──────────────────────────────────────────────

    /// When no Helm-relevant objective resolves, nav_goal fallthrough should
    /// steer toward the stored nav_goal position.
    #[test]
    fn operate_helm_falls_through_to_nav_goal_when_no_objective() {
        let mut memory = AiMemory {
            nav_goal: Some([100.0, 0.0]),
            ..Default::default()
        };
        let world = world_at_origin(); // entities list empty, anchors empty
        let pool: Vec<crate::messages::ScoredObjective> = vec![];
        let doctrine = vec![];

        let (thrust, _steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust > 0.0,
            "nav_goal fallthrough must produce positive thrust"
        );
    }

    /// When nav_goal target is reached (within WAYPOINT_ARRIVAL_RADIUS),
    /// operate_helm should return zero thrust and clear nav_goal.
    #[test]
    fn operate_helm_clears_nav_goal_on_arrival() {
        let mut memory = AiMemory {
            nav_goal: Some([0.0, -1.0]), // just one unit away (within arrival radius)
            ..Default::default()
        };
        // Ship at origin, target at [0, 0, -1] => dist = 1 < 20 => arrived
        let world = world_at_origin();
        let pool = vec![];
        let doctrine = vec![];

        let (thrust, _steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert_eq!(thrust, 0.0, "arrived at nav_goal must produce zero thrust");
        assert!(
            memory.nav_goal.is_none(),
            "nav_goal must be cleared on arrival"
        );
    }

    /// When a local objective resolves (Patrol with valid anchor), nav_goal
    /// must be cleared so the ship doesn't blend two conflicting targets.
    #[test]
    fn operate_helm_clears_nav_goal_when_local_objective_resolves() {
        let mut memory = AiMemory {
            nav_goal: Some([-999.0, -999.0]), // far away (not nav_goal land)
            ..Default::default()
        };
        let world = world_at_origin();
        let pool = patrol_pool(); // Patrol toward "alpha"
        let doctrine = patrol_doctrine();
        let anchors = anchors_with_alpha(); // alpha at [100, 0, 0]

        let (thrust, _) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &doctrine,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust > 0.0,
            "patrol must produce thrust even when nav_goal is set"
        );
        assert!(
            memory.nav_goal.is_none(),
            "nav_goal must be cleared when a local objective resolves"
        );
    }

    /// End-to-end: when no local Helm objective resolves and Navigation has
    /// set a nav_goal, operate_helm navigates toward it. Once a visible hostile
    /// appears with a matching Destroy objective, the helm transitions to
    /// destroy behaviour (AC #5).
    #[test]
    fn operate_helm_transitions_from_nav_goal_to_destroy() {
        let target_uuid = Uuid::new_v4();
        let mut memory = AiMemory {
            nav_goal: Some([200.0, 0.0]),
            ..Default::default()
        };

        // Phase 1: empty pool, only nav_goal → navigate toward nav_goal
        let world_empty = world_at_origin();
        let empty_pool: Vec<crate::messages::ScoredObjective> = vec![];

        let (thrust1, _) = operate_helm(
            &mut memory,
            &world_empty,
            &empty_pool,
            &[],
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust1 > 0.0,
            "nav_goal fallthrough must produce thrust toward steer target"
        );
        assert!(
            memory.nav_goal.is_some(),
            "nav_goal must persist when target not yet reached"
        );

        // Phase 2: hostile entity appears in range with a Destroy objective
        // → helm switches to destroy, clearing nav_goal.
        let world_with_hostile = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 2.0,
            entities: vec![AiWorldEntity {
                uuid: target_uuid,
                name: Some("enemy-hostile".into()),
                position: [100.0, 0.0, 0.0],
                ..Default::default()
            }],
            ..Default::default()
        };

        let destroy_pool: Vec<crate::messages::ScoredObjective> =
            vec![crate::messages::ScoredObjective {
                id: "destroy-hostile".into(),
                score: 90.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: "enemy-hostile".into(),
                },
                source: crate::messages::ObjectiveSource::Doctrine,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-hostile".into(),
                    text: "Destroy hostile".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["enemy-hostile".into()],
                    source: crate::messages::ObjectiveSource::Doctrine,
                },
            }];
        let destroy_doctrine = vec![crate::entity_config::DoctrineObjective {
            id: "destroy-hostile".into(),
            text: "Destroy hostile".into(),
            directive_kind: Some("Destroy".into()),
            base_priority: 90.0,
            target_speed: 0.9,
            maintain_range: 25.0,
            ..Default::default()
        }];

        let (thrust2, _) = operate_helm(
            &mut memory,
            &world_with_hostile,
            &destroy_pool,
            &destroy_doctrine,
            &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );

        assert!(
            thrust2 > 0.0,
            "resolved Destroy must produce thrust toward hostile"
        );
        assert!(
            memory.nav_goal.is_none(),
            "nav_goal must be cleared when local Destroy objective resolves"
        );
        assert_eq!(
            memory.target,
            Some(target_uuid),
            "memory.target must be set to the hostile entity"
        );
    }

    /// When a local objective exists but fails to resolve (e.g. Destroy with
    /// target not in world view), operate_helm should fall through to nav_goal.
    #[test]
    fn operate_helm_falls_through_unresolvable_destroy_to_nav_goal() {
        let mut memory = AiMemory {
            nav_goal: Some([100.0, 0.0]),
            ..Default::default()
        };
        let world = world_at_origin(); // no entities -> Destroy unresolvable
        let pool = destroy_then_patrol_pool(&std::collections::HashMap::new());
        // destroy_then_patrol_pool has Destroy (score 90, unresolvable), Patrol (score 30, resolvable)
        // but we pass empty anchors, so Patrol also unresolvable => fall through to nav_goal

        let (thrust, _steering) = operate_helm(
            &mut memory,
            &world,
            &pool,
            &[],
            &std::collections::HashMap::new(), // no anchors -> Patrol also unresolvable
            WAYPOINT_ARRIVAL_RADIUS,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            &empty_registry(),
            0.6,
        );
        assert!(
            thrust > 0.0,
            "must fall through to nav_goal when no objective resolves"
        );
    }
}
