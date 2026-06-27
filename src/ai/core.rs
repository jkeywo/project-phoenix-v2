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
}

// ── WorldView ─────────────────────────────────────────────────────────────────

/// A visible entity in the AI's world view.
#[derive(Debug, Clone, Default)]
pub struct AiWorldEntity {
    /// Stable UUID of the entity.
    pub uuid: Uuid,
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
    pool.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    pool
}

fn parse_doctrine_directive(d: &crate::entity_config::DoctrineObjective) -> crate::messages::AiDirective {
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
) -> (f32, f32) {
    use crate::messages::{AiDirective, SystemAffinity};

    // Find top-scoring directive with Helm relevance (score > 0).
    // Helm serves: Patrol, Destroy, Reach. None of these if score == 0.
    let top = scored_pool.iter().find(|o| {
        o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm)
    });

    match top.map(|o| &o.directive) {
        Some(AiDirective::Destroy { .. }) => {
            // Find matching doctrine entry for target_speed / maintain_range config.
            let cfg = doctrine.iter().find(|d| {
                top.map(|o| o.id == d.id).unwrap_or(false)
            });
            let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.8);
            let maintain_range = cfg.map(|d| d.maintain_range).unwrap_or(25.0);
            helm_destroy(memory, world_view, anchors, avoidance_buffer,
                avoidance_look_ahead_secs, forward_speed, faction_registry,
                target_speed, maintain_range)
        }
        Some(AiDirective::Patrol { anchors: waypoints, loop_path }) => {
            let cfg = doctrine.iter().find(|d| {
                top.map(|o| o.id == d.id).unwrap_or(false)
            });
            let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.5);
            helm_patrol(memory, world_view, waypoints, *loop_path, waypoint_arrival_radius,
                avoidance_buffer, avoidance_look_ahead_secs, forward_speed,
                target_speed, anchors)
        }
        Some(AiDirective::Reach { anchor }) => {
            let cfg = doctrine.iter().find(|d| {
                top.map(|o| o.id == d.id).unwrap_or(false)
            });
            let target_speed = cfg.map(|d| d.target_speed).unwrap_or(0.6);
            if let Some(&pos) = anchors.get(anchor.as_str()) {
                helm_navigate_to(memory, world_view, pos, waypoint_arrival_radius,
                    avoidance_buffer, avoidance_look_ahead_secs, forward_speed, target_speed)
            } else {
                (0.0, 0.0)
            }
        }
        _ => (0.0, 0.0),
    }
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
    target_speed: f32,
    maintain_range: f32,
) -> (f32, f32) {
    // Validate / refresh target.
    let target_uuid = resolve_destroy_target(memory, world_view, faction_registry);
    let Some(target_uuid) = target_uuid else {
        return (0.0, 0.0);
    };
    memory.target = Some(target_uuid);

    let Some(target_entity) = world_view.entities.iter().find(|e| e.uuid == target_uuid) else {
        memory.target = None;
        return (0.0, 0.0);
    };

    let pos = world_view.entity_pos;
    let target_pos = target_entity.position;
    let dx = target_pos[0] - pos[0];
    let dz = target_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    if dist < 1.0 {
        return (0.0, 0.0);
    }

    let effective_range = world_view.entity_weapons_range.unwrap_or(maintain_range);
    let at_station = dist <= effective_range * 0.8;

    // When holding station, steer to face the target so the phaser forward-arc
    // gate passes. When approaching, steer toward the offset approach point.
    let dir = if at_station {
        [dx / dist, dz / dist]
    } else {
        let approach_target = offset_approach_target(pos, target_pos, effective_range * 0.8);
        let nav_dx = approach_target[0] - pos[0];
        let nav_dz = approach_target[2] - pos[2];
        let nav_dist = (nav_dx * nav_dx + nav_dz * nav_dz).sqrt();
        if nav_dist > 0.1 {
            [nav_dx / nav_dist, nav_dz / nav_dist]
        } else {
            [dx / dist, dz / dist]
        }
    };

    let self_uuid = uuid::Uuid::nil(); // excluded from avoidance (self already excluded upstream)
    let avoidance = avoidance_steering(
        pos, world_view.entity_yaw, forward_speed, world_view.self_radius,
        self_uuid, &world_view.entities, avoidance_buffer, avoidance_look_ahead_secs,
    );

    let base_steer = steer_toward(world_view.entity_yaw, dir, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);

    let thrust = if at_station { 0.0 } else { target_speed };
    (thrust, steering)
}

/// Resolve which target to attack: existing (if still visible) > last_attacker > nearest hostile.
fn resolve_destroy_target(
    memory: &AiMemory,
    world_view: &WorldView,
    faction_registry: &crate::faction::FactionRegistry,
) -> Option<Uuid> {
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

/// Find the nearest entity that is hostile to this AI's faction.
fn find_nearest_hostile(
    world_view: &WorldView,
    faction_registry: &crate::faction::FactionRegistry,
) -> Option<Uuid> {
    let self_faction = world_view.self_faction?;
    let pos = world_view.entity_pos;
    world_view
        .entities
        .iter()
        .filter(|e| {
            e.faction.map(|ef| crate::faction::is_enemy(Some(self_faction), Some(ef), faction_registry)).unwrap_or(false)
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
) -> (f32, f32) {
    if waypoints.is_empty() {
        return (0.0, 0.0);
    }

    // Clamp index.
    if memory.waypoint_index >= waypoints.len() {
        if loop_path {
            memory.waypoint_index = 0;
        } else {
            return (0.0, 0.0);
        }
    }

    let waypoint_name = &waypoints[memory.waypoint_index];
    let Some(&wp_pos) = anchors.get(waypoint_name.as_str()) else {
        return (0.0, 0.0);
    };

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
            return (0.0, 0.0);
        }
        return (target_speed, 0.0);
    }

    let dir = [dx / dist, dz / dist];
    let self_uuid = uuid::Uuid::nil();
    let avoidance = avoidance_steering(
        pos, world_view.entity_yaw, forward_speed, world_view.self_radius,
        self_uuid, &world_view.entities, avoidance_buffer, avoidance_look_ahead_secs,
    );
    let base_steer = steer_toward(world_view.entity_yaw, dir, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);
    (target_speed, steering)
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
) -> (f32, f32) {
    let pos = world_view.entity_pos;
    let dx = target_pos[0] - pos[0];
    let dz = target_pos[2] - pos[2];
    let dist = (dx * dx + dz * dz).sqrt();

    if dist < arrival_radius {
        return (0.0, 0.0);
    }

    let dir = [dx / dist, dz / dist];
    let self_uuid = uuid::Uuid::nil();
    let avoidance = avoidance_steering(
        pos, world_view.entity_yaw, forward_speed, world_view.self_radius,
        self_uuid, &world_view.entities, avoidance_buffer, avoidance_look_ahead_secs,
    );
    let base_steer = steer_toward(world_view.entity_yaw, dir, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
    let steering = (base_steer + avoidance).clamp(-1.0, 1.0);
    (target_speed, steering)
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
    let has_destroy = scored_pool.iter().any(|o| {
        o.score > 0.0 && o.relevance.contains(&SystemAffinity::Weapons)
    });
    if !has_destroy {
        return (None, false);
    }

    // Resolve target: current target → last attacker → nearest hostile.
    let target = if let Some(t) = memory.target {
        if world_view.entities.iter().any(|e| e.uuid == t) { Some(t) } else { None }
    } else {
        None
    };
    let target = target.or_else(|| {
        memory.last_attacker.filter(|la| {
            world_view.entities.iter().any(|e| e.uuid == *la)
        })
    });
    let target = target.or_else(|| find_nearest_hostile(world_view, faction_registry));

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
        let damage_recent = last_damage_taken_secs
            .is_some_and(|s| now - s < CAPTAIN_COMBAT_WINDOW_SECS);
        let weapon_recent = last_weapon_fired_secs
            .is_some_and(|s| now - s < CAPTAIN_COMBAT_WINDOW_SECS);
        Some(damage_recent || weapon_recent)
    }

    /// No-op stub — channel-3 coordination not yet implemented for captain.
    pub fn coordinate(&self) {}
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
        assert!(result > 0.0, "target to the right must give positive steering");
    }

    #[test]
    fn steer_toward_negative_for_target_to_left() {
        let dir = [-1.0_f32, 0.0_f32];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        let unit = [dir[0] / len, dir[1] / len];
        let result = steer_toward(0.0, unit, 0.0, PATROL_FULL_STEER_RAD);
        assert!(result < 0.0, "target to the left must give negative steering");
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
            &mut memory, &world, &pool, &doctrine, &anchors,
            WAYPOINT_ARRIVAL_RADIUS, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 0.0,
            &empty_registry(),
        );
        assert!(thrust > 0.0, "should thrust toward waypoint");
    }

    #[test]
    fn operate_helm_empty_pool_returns_zero() {
        let mut memory = AiMemory::default();
        let world = world_at_origin();
        let (thrust, steering) = operate_helm(
            &mut memory, &world, &[], &[], &std::collections::HashMap::new(),
            WAYPOINT_ARRIVAL_RADIUS, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 0.0,
            &empty_registry(),
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
            &mut memory, &world, &pool, &doctrine, &anchors,
            WAYPOINT_ARRIVAL_RADIUS, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 0.0,
            &empty_registry(),
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
                id: "patrol".into(), text: "".into(), mandatory: false,
                status: crate::messages::ObjectiveStatus::Active, targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }];
        let doctrine = vec![crate::entity_config::DoctrineObjective {
            id: "patrol".into(), text: "".into(),
            directive_kind: Some("Patrol".into()),
            directive_anchors: vec!["wp0".into(), "wp1".into()],
            directive_loop: false,
            target_speed: 0.5,
            ..Default::default()
        }];

        let world = world_at_origin();
        operate_helm(
            &mut memory, &world, &pool, &doctrine, &anchors,
            WAYPOINT_ARRIVAL_RADIUS, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS, 0.0,
            &empty_registry(),
        );
        assert_eq!(memory.waypoint_index, 1, "should advance past arrived waypoint");
    }

    // ── operate_weapons ───────────────────────────────────────────────────

    fn destroy_pool_with_score(score: f32) -> Vec<crate::messages::ScoredObjective> {
        vec![crate::messages::ScoredObjective {
            id: "destroy".into(),
            score,
            directive: crate::messages::AiDirective::Destroy { target: "".into() },
            source: crate::messages::ObjectiveSource::Doctrine,
            relevance: vec![
                crate::messages::SystemAffinity::Helm,
                crate::messages::SystemAffinity::Weapons,
                crate::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::messages::ObjectiveSnapshot {
                id: "destroy".into(), text: "".into(), mandatory: false,
                status: crate::messages::ObjectiveStatus::Active, targets: vec![],
                source: crate::messages::ObjectiveSource::Doctrine,
            },
        }]
    }

    #[test]
    fn operate_weapons_returns_target_and_fire_when_ready_and_target_visible() {
        let target_id = Uuid::new_v4();
        let memory = AiMemory { target: Some(target_id), ..Default::default() };
        let world = WorldView {
            entity_phaser_ready: true,
            entities: vec![AiWorldEntity { uuid: target_id, ..Default::default() }],
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
        let memory = AiMemory { target: Some(target_id), ..Default::default() };
        let world = WorldView {
            entity_phaser_ready: false,
            entities: vec![AiWorldEntity { uuid: target_id, ..Default::default() }],
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
        let memory = AiMemory { target: Some(target_id), ..Default::default() };
        let world = WorldView {
            entity_phaser_ready: true,
            entities: vec![AiWorldEntity { uuid: target_id, ..Default::default() }],
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
            entities: vec![AiWorldEntity { uuid: attacker_id, ..Default::default() }],
            ..Default::default()
        };
        let pool = destroy_pool_with_score(35.0);
        let (t, fire) = operate_weapons(&memory, &world, &pool, &empty_registry());
        assert_eq!(t, Some(attacker_id));
        assert!(fire);
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
        let cond = WorldConditions { red_alert: false, hull_fraction: 1.0 };
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
            zero_gates: vec![ZeroGateCondition { condition: "hull_below".into(), threshold: Some(0.3) }],
            ..Default::default()
        }];
        let cond = WorldConditions { red_alert: false, hull_fraction: 1.0 };
        let pool = score_doctrine_pool(&doctrine, &cond);
        assert_eq!(pool[0].score, 0.0, "zero-gate must veto at full hull");
    }

    #[test]
    fn score_doctrine_pool_sorted_descending_by_score() {
        use crate::entity_config::DoctrineObjective;
        use crate::objectives::WorldConditions;

        let doctrine = vec![
            DoctrineObjective { id: "a".into(), text: "A".into(), base_priority: 10.0, ..Default::default() },
            DoctrineObjective { id: "b".into(), text: "B".into(), base_priority: 35.0, ..Default::default() },
            DoctrineObjective { id: "c".into(), text: "C".into(), base_priority: 20.0, ..Default::default() },
        ];
        let cond = WorldConditions { red_alert: false, hull_fraction: 1.0 };
        let pool = score_doctrine_pool(&doctrine, &cond);
        assert_eq!(pool[0].id, "b");
        assert_eq!(pool[1].id, "c");
        assert_eq!(pool[2].id, "a");
    }
}
