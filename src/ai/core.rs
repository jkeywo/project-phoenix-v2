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
use crate::simmath;
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
/// Authored shape of the avoidance severity ramp (issue #968): the exponent
/// [`hazard_threat_fraction`] raises "share of the authored `avoidance_buffer`
/// already spent" to. `1.0` is a straight line; the shipped `2.0` is gentle
/// where there is still room and steep where there is not.
///
/// This is a designer knob, not geometry. Both ends of the ramp are pinned by
/// the model itself — `0.0` at a full buffer's clearance and `1.0` at contact,
/// at every obstacle size — and the exponent only decides how a hull spends the
/// distance in between. That trade is a gameplay one: a flatter ramp reacts
/// earlier and pulls hulls off their firing solutions in a close fight, a
/// steeper one leaves the dodge later. See the table in
/// [`hazard_threat_fraction`] for what the shipped value costs at each size.
/// Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::hazard_threat_exponent`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const HAZARD_THREAT_EXPONENT: f32 = 2.0;
/// Authored ceiling (radians) on how far a DEAD-RECKONED hull holds its heading
/// off its route bearing to clear an obstacle (issue #968). A quarter turn.
///
/// The ceiling itself is geometric: at 90° off the line to an obstacle the ship
/// is flying the tangent, less than that is still closing on it, and more is
/// flying back the way it came — which is how a hull ends up oscillating in
/// front of a rock instead of getting past it. What is NOT geometry, and is why
/// this is a knob rather than a bare `const` in the mover, is the ramp UP to it:
/// the deviation applied is `threat × ceiling`, a smooth proportional bend, and
/// not the true tangent angle `asin((r_self + r_hazard) / d)` for the distance
/// in hand. At threat 0.25 a hull holds 22.5° off its route, which is nothing
/// like that tangent — it is simply "a quarter of the way to the hardest turn I
/// am willing to make". How eagerly a hull leaves its route is a designer's
/// call, so a hull may author its own ceiling.
///
/// Note what this replaced. The low-LOD avoidance used to step toward a desired
/// heading at `max_yaw_rate * dt`, so the hull's own authored turn rate bounded
/// the manoeuvre. It no longer does: this path SETS a heading rather than
/// turning toward one (see `ai::server::low_lod_avoid_yaw` for why the old form
/// could never deviate by more than one tick's turn), so a battleship and a
/// courier now deviate identically unless they author different values here.
/// `max_yaw_rate` still bounds the Destroy-target turn in the same mover, and
/// every high-fidelity hull's steering.
///
/// Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::low_lod_avoidance_deviation_rad`].
pub const LOW_LOD_AVOIDANCE_DEVIATION_RAD: f32 = PI / 2.0;
/// Speed fraction [0, 1] used for the Channel-3 Navigation→Helm handoff
/// (`NavigationWaypoint`) fallthrough when the entity has no `[behaviour]`
/// section to author one. Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::nav_handoff_speed`], whose serde
/// default reads this constant so the two cannot drift apart.
pub const NAV_HANDOFF_SPEED: f32 = 0.6;
/// Distance (world units) within which a docking intent switches from normal
/// objective approach to the close-quarters [`docking_close_manoeuvre`].
/// Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::docking_engage_distance`].
pub const DOCKING_ENGAGE_DISTANCE: f32 = 40.0;
/// Speed fraction `[0, 1]` capping the low-speed reverse / lateral translation
/// of a docking close manoeuvre. Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::docking_approach_speed`].
pub const DOCKING_APPROACH_SPEED: f32 = 0.3;
const AVOIDANCE_MIN_SPEED: f32 = 0.25;
/// Authored size-ignore ratio default: a ship ignores a **mobile** hazard whose
/// `size_rating` is below `self_size_rating * ratio`. Static terrain is never
/// ignored at any ratio (issue #958). `0.0` disables the rule outright (every
/// dangerous hazard is assessed regardless of size), which is the
/// backward-compatible default and what every shipped hull uses today — no
/// entity TOML authors this field, so the rule is currently inert in shipped
/// content. Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::hazard_ignore_size_ratio`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const HAZARD_IGNORE_SIZE_RATIO: f32 = 0.0;
/// Authored lateral-thrust hazard sensitivity default: the multiplier a fine
/// lateral-thrust actuator applies to the shared hazard assessment's starboard
/// (local `+X`) repulsion component before clamping to `[-1, 1]`. `1.0` passes
/// the boids-style repulsion through unweighted. Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::lateral_hazard_sensitivity`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const LATERAL_HAZARD_SENSITIVITY: f32 = 1.0;
/// Authored vertical-thrust hazard sensitivity default (issue #744): the
/// multiplier the vertical-thrust actuator applies to the shared assessment's
/// moving-hazard threat before clamping to `[0, 1]`. `1.0` passes it through
/// unweighted. Parse-time default only — see
/// [`crate::entities::config::BehaviourConfig::vertical_hazard_sensitivity`], whose
/// serde default reads this constant so the two cannot drift apart.
pub const VERTICAL_HAZARD_SENSITIVITY: f32 = 1.0;
/// Authored maximum vertical offset (world units) a `Bounded` craft may climb
/// away from its cruise plane while dodging (issue #744). Parse-time default
/// only — see [`crate::entities::config::HelmCapabilityConfig::max_vertical_offset`].
pub const MAX_VERTICAL_OFFSET: f32 = 30.0;
/// Authored gradual return-to-cruise gain for `Bounded` craft (issue #744):
/// once avoidance urgency falls, the vertical actuator commands a descent of
/// `-y * VERTICAL_RETURN_RATE` (clamped) so the ship eases back to its cruise
/// plane rather than snapping. Parse-time default only — see
/// [`crate::entities::config::HelmCapabilityConfig::vertical_return_rate`].
pub const VERTICAL_RETURN_RATE: f32 = 0.05;
/// Authored hazard-urgency threshold at or above which an imminent collision may
/// TEMPORARILY override the ship's desired facing to point along the escape
/// direction (issue #780, AC4). Below it, ordinary avoidance only bends travel
/// and never touches facing. `1.0` here means "off by default" (only a
/// full-urgency, effectively-unavoidable collision qualifies) — a hull opts into
/// an earlier facing bail-out by authoring a lower
/// [`crate::entities::config::BehaviourConfig::imminent_collision_facing_threshold`].
/// Parse-time default only; the override is stateless and evaporates the tick
/// urgency drops back under the threshold.
///
/// Since issue #968 the `1.0` default is REACHABLE rather than merely nominal,
/// and that is the point. Urgency used to be `1 - centre_distance /
/// avoidance_radius`, which only reaches 1 when two hulls occupy the same point
/// — so "off by default" meant "off, full stop", and a ship that had driven
/// inside a rock had no behaviour that would turn it back out. Urgency is now
/// the share of the authored buffer the SURFACE clearance has spent
/// ([`hazard_threat_fraction`]), so it reads 1.0 exactly when the hulls are
/// touching or overlapping. The default therefore now means what its wording
/// always claimed: bail out on a collision that is no longer avoidable, and
/// never before.
///
/// Reachable AT SPEED, not merely when stationary: [`assess_hazards`] measures
/// each hazard from both the projected and the current position and keeps the
/// worse reading, so a hull moving fast enough to project its look-ahead point
/// clean past a rock still reads the rock it is inside. Without that half the
/// claim above holds only for a ship at rest — the projection at a destroyer's
/// authored cruise is 40.5 units, more than twice the whole avoidance radius
/// for a `huge` rock.
pub const IMMINENT_COLLISION_FACING_THRESHOLD: f32 = 1.0;
/// Proportional deceleration factor for approach: thrust begins ramping down
/// when distance is within this multiple of the target stop-distance.
/// At 1.5× the stop threshold the ship starts slowing; at the threshold it
/// reaches zero thrust, preventing overshoot oscillation near targets.
pub const APPROACH_DECEL_FACTOR: f32 = 1.5;

/// Planar clearance between two entity surfaces.
///
/// Combat movement and direct-fire reach are authored in visible hull-to-hull
/// units, not centre-to-centre units.  Keeping that conversion here prevents a
/// large starbase from silently consuming a weapon's useful range.
pub fn surface_distance_xz(
    a_position: [f32; 3],
    a_radius: f32,
    b_position: [f32; 3],
    b_radius: f32,
) -> f32 {
    let dx = b_position[0] - a_position[0];
    let dz = b_position[2] - a_position[2];
    (dx * dx + dz * dz).sqrt() - a_radius.max(0.0) - b_radius.max(0.0)
}

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
    pub phase: crate::ship::impulse::ImpulsePhase,
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
    if dist <= input.cancel_distance && input.phase != crate::ship::impulse::ImpulsePhase::Idle {
        return ImpulseDecision::Cancel;
    }

    // Only engage from Idle.
    if input.phase != crate::ship::impulse::ImpulsePhase::Idle {
        return ImpulseDecision::NoChange;
    }

    // Must be far enough to make impulse worthwhile.
    if dist < input.engage_distance {
        return ImpulseDecision::NoChange;
    }

    // Check if target is directly ahead.
    let fwd_x = simmath::sin(input.yaw);
    let fwd_z = -simmath::cos(input.yaw);
    let dir_x = dx / dist;
    let dir_z = dz / dist;
    let cross = fwd_x * dir_z - fwd_z * dir_x;
    let dot = fwd_x * dir_x + fwd_z * dir_z;
    let angle = simmath::atan2(cross, dot);

    if angle.abs() <= input.angle_tolerance {
        ImpulseDecision::Engage
    } else {
        ImpulseDecision::NoChange
    }
}

// ── WorldView ─────────────────────────────────────────────────────────────────

/// A visible entity in the AI's world view.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    pub shields: Option<Vec<crate::core::messages::ShieldFacingStatus>>,
    /// Hull integrity fraction [0, 1], if known.
    pub hull_fraction: Option<f32>,
    /// Yaw in radians (Y-up, forward = -Z at yaw 0), if known.
    pub yaw: Option<f32>,
    /// Physical radius of the entity (world units) used for collision avoidance.
    pub radius: f32,
    /// Current forward speed of the entity (world units/s) used for predictive avoidance.
    pub forward_speed: f32,
    /// Hazard fact: whether this entity can move under its own power (a ship)
    /// versus being static terrain (an asteroid, a station, a planet). Published
    /// so fine helm systems can apply their own policy — e.g. a bounded vertical
    /// thruster dodging only moving hazards while engines still brake for static
    /// ones (issue #743).
    ///
    /// Authored per template as `[collider] movable` and copied here by the
    /// world-snapshot builders (issue #958); it is never inferred from which ECS
    /// query the entity arrived on. [`assess_hazards`] additionally keys the
    /// ignore-smaller rule off it: only a mobile contact can be dropped for
    /// being small.
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
    let fwd_x = simmath::sin(yaw);
    let fwd_z = -simmath::cos(yaw);

    let cross = fwd_x * target_dir[1] - fwd_z * target_dir[0];
    let dot = fwd_x * target_dir[0] + fwd_z * target_dir[1];
    let angle = simmath::atan2(cross, dot);

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

/// How threatening one projected hazard is, `[0, 1]`, from the share of the
/// hull's authored `avoidance_buffer` that the SURFACE-TO-SURFACE clearance has
/// used up, raised to the hull's authored `hazard_threat_exponent`. `0.0` = a
/// full buffer's worth of clear space still ahead; `1.0` = the two surfaces are
/// touching or already overlapping. Both ends are size-invariant, which is the
/// point; the shape in between is explained at the exponent below.
///
/// # Why clearance and not centre distance (issue #968)
///
/// This used to be `1 - dist / (self_radius + hazard_radius + buffer)`, i.e. the
/// projected CENTRE separation as a fraction of the whole avoidance radius. That
/// makes the response depend on how big the obstacle is rather than on how close
/// the hull is to hitting it, and it gets *weaker* as the obstacle gets *bigger*:
/// with the shipped 5-unit buffer and a 1.2-radius destroyer, the moment of
/// contact scored
///
/// | rock radius | old threat at contact | clearance-based |
/// |-------------|-----------------------|-----------------|
/// | 2           | 0.61                  | 1.00            |
/// | 4           | 0.49                  | 1.00            |
/// | 12 (`huge`) | 0.27                  | 1.00            |
///
/// so the `huge` class added in issue #947 was pushed away with less than half
/// the force of the rocks the avoidance was originally tuned against — the
/// "separation response is too weak for a large obstacle" this issue reports.
/// Ships ground their way through radius-12 rocks, ending up as much as 6.5
/// units INSIDE the collider.
///
/// Measuring the clearance instead makes the response size-invariant: a hull one
/// buffer-width off a rock's skin reacts the same whether the rock is a pebble
/// or a mountain, and a hull actually touching one always reacts at full
/// strength. The authored `avoidance_buffer` is the whole ramp, which is what a
/// designer tuning that field already believes it to be.
///
/// # What this DOES re-scale for an existing hull
///
/// The ends of the ramp are unchanged, but a hull that authored a THRESHOLD
/// against the old curve now crosses it somewhere else, and the four Alliance
/// hulls all author `imminent_collision_facing_threshold = 0.6`. That trigger
/// used to be "projected centre separation below 40% of the avoidance radius";
/// it is now "surface clearance at or under
/// `buffer × (1 − √threshold)`" — 1.13 units at the shipped 5-unit buffer,
/// whatever the obstacle. Against a `huge` rock that is the fix (the old form
/// did not fire until the hull was already 5.9 units INSIDE it). Against a
/// `small` rock it fires roughly fourteen times earlier — 1.13 units of
/// clearance instead of 0.08 — and the override snaps `desired_facing_local`
/// off the gunnery solution, so it is a real behaviour change on the small end
/// and not only a repair on the large one. Squaring does not damp it, because a
/// threshold crossing is not a proportional response. The crossing point is
/// pinned in
/// `imminent_collision_facing_threshold_now_crosses_at_a_fixed_surface_clearance`.
///
/// `avoidance_buffer <= 0.0` means a hull that authored no standoff at all:
/// there is no ramp, so the answer is a step — full threat while the surfaces
/// overlap and none once they do not. Both in-tree callers range-gate before
/// calling, and for them that is the same answer the `1.0` this used to return
/// unconditionally gave (their gate reduces to "overlapping" when the buffer is
/// zero); stating it as a function of the distance rather than ignoring the
/// distance is what makes this total for any future caller.
pub fn hazard_threat_fraction(
    projected_distance: f32,
    self_radius: f32,
    hazard_radius: f32,
    avoidance_buffer: f32,
    hazard_threat_exponent: f32,
) -> f32 {
    let clearance = projected_distance - self_radius - hazard_radius;
    if avoidance_buffer <= 0.0 {
        return if clearance <= 0.0 { 1.0 } else { 0.0 };
    }
    let spent = (1.0 - clearance / avoidance_buffer).clamp(0.0, 1.0);
    // The authored exponent shapes the ramp between those two fixed ends. At the
    // shipped default of 2 it is gentle where there is still room and steep where
    // there is not. A LINEAR ramp (exponent 1) fixes the size bias but reacts to
    // a hazard half a buffer away twice as hard as the old model did to ANY of
    // them, and that is not a free change: the shared surface feeds the lateral
    // thruster, which does not exclude a ship's own gunnery target the way the
    // steering legs do, so a hull in a close-range fight strafed off its firing
    // solution and the `combat_test` demo stopped chaining its waves.
    //
    // Squaring is a compromise, not a restoration, and it is only near-neutral
    // for one band of sizes. Old and new cross at
    //
    //     spent = buffer / (self_radius + hazard_radius + buffer)
    //
    // and BELOW that crossing (i.e. over the far end of the ramp, where there is
    // still clearance) the squared response is the WEAKER of the two. For a
    // 1.2-radius destroyer at the shipped 5-unit buffer:
    //
    // | hazard    | crossing | old at spent 0.5 | squared | verdict at mid-ramp |
    // |-----------|----------|------------------|---------|---------------------|
    // | r=2       | 0.61     | 0.30             | 0.25    | weaker              |
    // | r=4       | 0.49     | 0.25             | 0.25    | ~neutral            |
    // | r=12      | 0.27     | 0.14             | 0.25    | stronger            |
    // | ship-ship | 0.68     | 0.34             | 0.25    | weaker              |
    //
    // So "about as strong as before at mid-range" is true for the radius-4 rocks
    // the avoidance was tuned against and false either side of them: against a
    // small rock at spent 0.2 the new response is 0.04 against the old 0.12,
    // three times weaker. With a wide authored buffer the crossing goes to ~1
    // and the new curve is weaker over essentially the whole ramp — which is why
    // `helm_ai`'s 60-unit-buffer climb test needed its observation window
    // re-baselined from 60 ticks to 150. What is NOT a compromise, and is the
    // whole point of the issue, is the contact end: 1.0 at every size, for every
    // exponent.
    //
    // Through `simmath` rather than `f32::powf`, like every other transcendental
    // in the simulation: the authored exponent makes this a real `powf` call
    // rather than a multiply, and a host-libm `powf` is not bit-identical across
    // targets (issue #908).
    simmath::powf(spent, hazard_threat_exponent)
}

/// Signed steering `[-1, 1]` that turns a hull AROUND a projected obstacle
/// rather than back off it: negative is to port, positive to starboard, and the
/// magnitude is the summed [`hazard_threat_fraction`] of everything in play.
///
/// Public since issue #968 so the dead-reckoned low-LOD mover
/// (`ai::server::low_lod_avoid_yaw`) can share it. That path used to take its
/// heading from [`assess_hazards`]' repulsion VECTOR, which points radially away
/// from the obstacle — so a ship whose route ran through a rock turned to face
/// straight back out, cleared the buffer, snapped onto its route bearing again
/// and drove straight back in. Radial repulsion is the right thing for a
/// THRUSTER, which can push sideways while the hull points where it likes; it is
/// the wrong thing for a heading. A signed turn is what actually gets a ship
/// past something.
pub fn avoidance_steering(
    self_pos: [f32; 3],
    self_yaw: f32,
    self_speed: f32,
    self_radius: f32,
    excluded_uuid: Uuid,
    world_entities: &[AiWorldEntity],
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    hazard_threat_exponent: f32,
) -> f32 {
    if self_speed.abs() < AVOIDANCE_MIN_SPEED {
        return 0.0;
    }

    let fwd_x = simmath::sin(self_yaw);
    let fwd_z = -simmath::cos(self_yaw);
    let proj_self_x = self_pos[0] + fwd_x * self_speed * avoidance_look_ahead_secs;
    let proj_self_z = self_pos[2] + fwd_z * self_speed * avoidance_look_ahead_secs;

    let mut total_avoidance: f32 = 0.0;

    for entity in world_entities {
        if entity.uuid == excluded_uuid {
            continue;
        }
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;

        let (ent_proj_x, ent_proj_z) = if let Some(ent_yaw) = entity.yaw {
            let ent_fwd_x = simmath::sin(ent_yaw);
            let ent_fwd_z = -simmath::cos(ent_yaw);
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
            let threat_fraction = hazard_threat_fraction(
                proj_dist,
                self_radius,
                entity.radius,
                avoidance_buffer,
                hazard_threat_exponent,
            );
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
    doctrine: &[crate::entities::config::DoctrineObjective],
    conditions: &crate::objectives::WorldConditions,
) -> Vec<crate::core::messages::ScoredObjective> {
    let mut pool: Vec<crate::core::messages::ScoredObjective> = doctrine
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
            crate::core::messages::ScoredObjective {
                id: d.id.clone(),
                score,
                directive,
                source: crate::core::messages::ObjectiveSource::Doctrine,
                relevance,
                snapshot: crate::core::messages::ObjectiveSnapshot {
                    id: d.id.clone(),
                    text: d.text.clone(),
                    // Doctrine text is authored on the hull and names no runtime
                    // figure — there is no tick at which a standing objective
                    // acquires one.
                    text_params: Default::default(),
                    mandatory: d.mandatory,
                    status: crate::core::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::core::messages::ObjectiveSource::Doctrine,
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
/// [`AiDirective`]: crate::core::messages::AiDirective
pub fn parse_doctrine_directive(
    d: &crate::entities::config::DoctrineObjective,
) -> crate::core::messages::AiDirective {
    match d.directive_kind.as_deref() {
        Some("Patrol") => crate::core::messages::AiDirective::Patrol {
            anchors: d.directive_anchors.clone(),
            loop_path: d.directive_loop,
        },
        Some("Destroy") => crate::core::messages::AiDirective::Destroy {
            target: d.directive_target.clone().unwrap_or_default(),
        },
        Some("Reach") => crate::core::messages::AiDirective::Reach {
            anchor: d.directive_anchor.clone().unwrap_or_default(),
        },
        Some("Hail") => crate::core::messages::AiDirective::Hail {
            target: d.directive_hail_target.clone().unwrap_or_default(),
        },
        Some("Retreat") => crate::core::messages::AiDirective::Retreat {
            anchor: d.directive_anchor.clone().unwrap_or_default(),
        },
        Some("Dock") => crate::core::messages::AiDirective::Dock {
            target: d.directive_dock_target.clone().unwrap_or_default(),
        },
        // The issue-#1162 operate verbs, all reading the shared
        // `directive_operate_target`.
        Some("Tow") => crate::core::messages::AiDirective::Tow {
            target: d.directive_operate_target.clone().unwrap_or_default(),
        },
        Some("Stabilise") => crate::core::messages::AiDirective::Stabilise {
            target: d.directive_operate_target.clone().unwrap_or_default(),
        },
        Some("Escort") => crate::core::messages::AiDirective::Escort {
            target: d.directive_operate_target.clone().unwrap_or_default(),
        },
        Some("Transfer") => crate::core::messages::AiDirective::Transfer {
            target: d.directive_operate_target.clone().unwrap_or_default(),
        },
        Some("FieldRepair") => crate::core::messages::AiDirective::FieldRepair {
            target: d.directive_operate_target.clone().unwrap_or_default(),
        },
        _ => crate::core::messages::AiDirective::None,
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
    scored_pool: &[crate::core::messages::ScoredObjective],
    doctrine: &[crate::entities::config::DoctrineObjective],
    anchors: &std::collections::HashMap<String, [f32; 3]>,
    cursors: &[crate::ai::patrol_cursor::PatrolCursor],
    weapons_target: Option<Uuid>,
    nav_waypoint: Option<[f32; 2]>,
    waypoint_arrival_radius: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    hazard_threat_exponent: f32,
    forward_speed: f32,
    nav_handoff_speed: f32,
) -> (f32, f32) {
    use crate::core::messages::{AiDirective, SystemAffinity};

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
                    hazard_threat_exponent,
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
                            hazard_threat_exponent,
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
                    hazard_threat_exponent,
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
                        hazard_threat_exponent,
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
                        hazard_threat_exponent,
                        forward_speed,
                        target_speed,
                    )
                })
            }
            // Dock (issue #1028) resolves NOTHING here, on purpose. It is a
            // destination directive, and its destination reaches the helm the
            // way a destination the helm cannot see for itself always has:
            // `operate_navigation_ai` turns the objective into this ship's own
            // anchored waypoint, and the fall-through below flies it at
            // `nav_handoff_speed`. Resolving it a second time here would be the
            // second steering implementation the slice exists to avoid — and
            // would fly a stale position, since the anchored waypoint is the
            // thing that tracks a structure as it moves.
            crate::core::messages::AiDirective::Dock { .. } => None,
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
            hazard_threat_exponent,
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
    hazard_threat_exponent: f32,
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
    let stop_surface_distance = effective_range * 0.8;
    // Moving ships already negotiate their shared combat envelope through the
    // doctrine/radar loop.  Static targets need their visible radius folded
    // into approach, otherwise a starbase's centre is treated as its hull.
    let target_radius = (!target_entity.movable)
        .then_some(target_entity.radius)
        .unwrap_or(0.0);
    let surface_distance = surface_distance_xz(
        pos,
        // Doctrine ranges are measured from the ship's navigation origin.
        // Only the entity being approached expands that origin into its
        // visible surface; charging the self radius here would silently shift
        // every existing hold envelope.
        0.0,
        target_pos,
        target_radius,
    );
    let at_station = surface_distance <= stop_surface_distance;
    // `offset_approach_target` still works in centre coordinates, so convert
    // the authored surface clearance back once at its boundary.
    let stop_dist = stop_surface_distance + target_radius;

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
        hazard_threat_exponent,
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
        let decel_start = stop_surface_distance * APPROACH_DECEL_FACTOR;
        if surface_distance < decel_start {
            let t =
                (surface_distance - stop_surface_distance) / (decel_start - stop_surface_distance);
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
    let self_vx = simmath::sin(self_yaw) * self_speed;
    let self_vz = -simmath::cos(self_yaw) * self_speed;
    let (tgt_vx, tgt_vz) = match target_yaw {
        Some(y) => (
            simmath::sin(y) * target_speed,
            -simmath::cos(y) * target_speed,
        ),
        None => (0.0, 0.0),
    };

    // d(range)/dt = relative_velocity · unit_range_vector; closing is its
    // negation so "closing" reads positive.
    let closing_rate = -((tgt_vx - self_vx) * ux + (tgt_vz - self_vz) * uz);

    let fwd_x = simmath::sin(self_yaw);
    let fwd_z = -simmath::cos(self_yaw);
    let cross = fwd_x * uz - fwd_z * ux;
    let dot = fwd_x * ux + fwd_z * uz;

    TargetRelativeMotion {
        range,
        closing_rate,
        bearing_rad: simmath::atan2(cross, dot),
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
    pub hazard_threat_exponent: f32,
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
                [simmath::sin(input.self_yaw), -simmath::cos(input.self_yaw)]
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
                simmath::sin(input.escape_heading_rad),
                -simmath::cos(input.escape_heading_rad),
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
        input.hazard_threat_exponent,
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
    pub hazard_threat_exponent: f32,
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
        [simmath::sin(input.self_yaw), -simmath::cos(input.self_yaw)]
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
        let (s, c) = simmath::sin_cos(correction);
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
        input.hazard_threat_exponent,
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
    pub hazard_threat_exponent: f32,
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
        [simmath::sin(input.self_yaw), -simmath::cos(input.self_yaw)]
    } else {
        // Velocities reconstructed from (yaw, forward_speed) — the snapshot
        // carries no velocity field. Same recipe as `target_relative_motion`.
        let (tvx, tvz) = match input.target_yaw {
            Some(y) => (
                simmath::sin(y) * input.target_speed,
                -simmath::cos(y) * input.target_speed,
            ),
            None => (0.0, 0.0),
        };
        // The heading straight at where the target is NOW, in the project's
        // `atan2(dx, -dz)` convention. Handed in as the shooter yaw with a zero
        // bank facing so that `predict_intercept_heading`'s own fallback — which
        // is `shooter_yaw + facing_deg` — resolves to exactly this, rather than
        // to whichever way the hull happened to be pointing.
        let live = simmath::atan2(dx, -dz);
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
        [simmath::sin(heading), -simmath::cos(heading)]
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
        input.hazard_threat_exponent,
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
    faction_registry: &crate::ai::faction::FactionRegistry,
) -> Option<Uuid> {
    let self_faction = world_view.self_faction?;
    let pos = world_view.entity_pos;
    world_view
        .entities
        .iter()
        .filter(|e| {
            e.faction
                .map(|ef| {
                    crate::ai::faction::is_enemy(Some(self_faction), Some(ef), faction_registry)
                })
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
    faction_registry: &crate::ai::faction::FactionRegistry,
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
            .map(|ef| crate::ai::faction::is_enemy(self_faction, Some(ef), faction_registry))
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
    hazard_threat_exponent: f32,
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
        hazard_threat_exponent,
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
    hazard_threat_exponent: f32,
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
        hazard_threat_exponent,
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
    [simmath::sin(theta), 0.0, -simmath::cos(theta)]
}

/// Recover the yaw-steering intent (`[-1, 1]`) from a ship-local desired facing.
/// Inverse of [`encode_local_facing`]; sign-preserving and exact up to floating
/// point for the representable `[-1, 1]` range.
pub fn decode_steering_from_facing(facing_local: [f32; 3]) -> f32 {
    (simmath::atan2(facing_local[0], -facing_local[2]) / PATROL_FULL_STEER_RAD).clamp(-1.0, 1.0)
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
    let cos_y = simmath::cos(ship_yaw);
    let sin_y = simmath::sin(ship_yaw);
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
/// - the ignore-smaller rule skips a **`movable`** hazard whose `size_rating` is
///   below `self_size_rating * hazard_ignore_size_ratio` — a ratio of `0.0`
///   disables the rule (every dangerous hazard is assessed).
///
/// The ignore-smaller rule is deliberately mobile-only (issue #958). A big ship
/// may ignore a small ship, which can manoeuvre out of its way; static terrain —
/// an asteroid, a station, a planet — cannot, so it is avoided at any size. That
/// split is a doctrine invariant rather than a knob, but which side of it an
/// entity falls on is authored: `[collider] movable` in the entity's TOML
/// becomes [`AiWorldEntity::movable`], and the threshold itself stays authored
/// on the observer as `hazard_ignore_size_ratio`.
///
/// WHICH hazards register is a function of the avoidance radius
/// (`self_radius + hazard_radius + avoidance_buffer`); HOW HARD each one pushes
/// is [`hazard_threat_fraction`], which measures the surface-to-surface
/// clearance against the authored buffer so the response is the same at contact
/// whatever the obstacle's size (issue #968). `urgency` therefore reads as "how
/// much of my authored standoff is gone", and reaches `1.0` exactly when a hull
/// is touching or inside something — which is also what makes the authored
/// `imminent_collision_facing_threshold` reachable at its `1.0` default.
///
/// Each hazard is measured TWICE — once from the `forward_speed`-projected
/// position and once from where the ship actually is — and the worse reading
/// wins, with the repulsion taken from whichever won. The look-ahead exists to
/// find hazards EARLY; it must never be able to argue a hull out of one it is
/// already overlapping, which is exactly what a 40-unit projection past a
/// 12-radius rock used to do. See the note at the comparison itself.
///
/// Pure: no ECS, no Bevy. The planner converts the local force array to the
/// engine's vector type.
pub fn assess_hazards(
    world_view: &WorldView,
    forward_speed: f32,
    avoidance_buffer: f32,
    avoidance_look_ahead_secs: f32,
    hazard_ignore_size_ratio: f32,
    hazard_threat_exponent: f32,
) -> HazardAssessmentRaw {
    let self_pos = world_view.entity_pos;
    let self_yaw = world_view.entity_yaw;
    let self_radius = world_view.self_radius;
    let self_size_rating = world_view.self_size_rating;

    let fwd_x = simmath::sin(self_yaw);
    let fwd_z = -simmath::cos(self_yaw);
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
        // Issue #958: the ignore-smaller rule is a MOBILE-CONTACT rule. A
        // battleship may sweep past a courier because the courier can get out of
        // the way; an asteroid, a station or a planet cannot, so static terrain
        // is assessed regardless of how small it rates. Which side of that split
        // an entity falls on is the authored `[collider] movable` fact carried
        // in on `AiWorldEntity`, not a category test on the object here.
        if entity.movable
            && hazard_ignore_size_ratio > 0.0
            && entity.size_rating < self_size_rating * hazard_ignore_size_ratio
        {
            continue;
        }
        let avoidance_radius = self_radius + entity.radius + avoidance_buffer;
        let (ent_proj_x, ent_proj_z) = if let Some(ent_yaw) = entity.yaw {
            (
                entity.position[0]
                    + simmath::sin(ent_yaw) * entity.forward_speed * avoidance_look_ahead_secs,
                entity.position[2]
                    + (-simmath::cos(ent_yaw)) * entity.forward_speed * avoidance_look_ahead_secs,
            )
        } else {
            (entity.position[0], entity.position[2])
        };

        let proj_ddx = proj_self_x - ent_proj_x;
        let proj_ddz = proj_self_z - ent_proj_z;
        let proj_dist = (proj_ddx * proj_ddx + proj_ddz * proj_ddz).sqrt();

        // ── The look-ahead never subtracts from the here-and-now (issue #968) ──
        // The projection is a PREDICTION, and a prediction can point clean past
        // something the hull is standing in. At `combat_test`'s authored cruise
        // (a destroyer's `target_speed = 0.9` of 15 u/s) the 3-second projection
        // lands 40.5 units ahead, while the whole avoidance radius for a `huge`
        // rock is 18.2 — so every centre distance under 22.3 units, INCLUDING
        // the entire interior of the rock, fell outside the projected picture
        // and a buried hull read 0.0 urgency. That is the exact case the
        // reachable `imminent_collision_facing_threshold` exists to answer, so
        // reading it as "nothing there" is the worst possible moment to be
        // wrong.
        //
        // Both readings are taken and the WORSE one wins. A look-ahead may only
        // ever discover a hazard earlier than the current geometry does; it may
        // never talk the hull out of one it is already up against. The repulsion
        // is taken from whichever reading won, so a hull inside a collider
        // pushes radially out of it rather than out of a point 40 units the far
        // side of it.
        let here_ddx = self_pos[0] - entity.position[0];
        let here_ddz = self_pos[2] - entity.position[2];
        let here_dist = (here_ddx * here_ddx + here_ddz * here_ddz).sqrt();

        let in_range = |d: f32| d < avoidance_radius && d > 0.01;
        if !in_range(proj_dist) && !in_range(here_dist) {
            continue;
        }
        // Severity is the share of the AUTHORED BUFFER the surface-to-surface
        // clearance has used up, not the share of the whole avoidance radius
        // the centre separation has (issue #968) — see
        // [`hazard_threat_fraction`] for why the old form under-reacted to
        // exactly the biggest obstacles.
        let threat_of = |d: f32| {
            if in_range(d) {
                hazard_threat_fraction(
                    d,
                    self_radius,
                    entity.radius,
                    avoidance_buffer,
                    hazard_threat_exponent,
                )
            } else {
                0.0
            }
        };
        let projected_threat = threat_of(proj_dist);
        let here_threat = threat_of(here_dist);
        let (threat_fraction, ddx, ddz, dist) = if here_threat > projected_threat {
            (here_threat, here_ddx, here_ddz, here_dist)
        } else {
            (projected_threat, proj_ddx, proj_ddz, proj_dist)
        };
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

    HazardAssessmentRaw {
        forces_local: force,
        urgency: urgency.clamp(0.0, 1.0),
        primary,
        contributions,
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "core_tests.rs"]
mod tests;
