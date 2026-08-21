use super::*;

/// The fact name the shared hazard surface is seeded under by
/// [`seed_helm_actuator_facts`].
pub(crate) const HAZARD_URGENCY_FACT: &str = "hazard_urgency";

// ── The movement POSTURE fact (issue #875) ───────────────────────────────────
//
// SCOPE: seeded by [`seed_helm_actuator_facts`], which is the ONE seeder every
// helm policy host calls — `ai_policy_state_tick` and all six per-axis actuator
// hosts. So unlike the recovery facts below, `posture` is available to both
// transition guards and a state's continuous rule guards, on every axis. A class
// movement doctrine has to be able to read it in both places: the transition is
// what changes leg, and the rule is what the leg does while it is held.
//
// Derived HELM-SIDE from `ShipRedAlert`, folded once per shared tick onto
// [`HelmAiShipFrame::red_alert`] rather than re-read by each host, for the same
// reason `hostile_arc_exposure` is: seven hosts reading the ship's alert
// independently is seven chances for the axes to disagree about the same tick.
//
// It is deliberately NOT the same name as `red_alert`. The weapons' fire gate
// (#872) reads `fact(red_alert)` on the WEAPON hosts and means "the captain has
// authorised fire"; this means "which movement doctrine is licensed", and the
// two are only coincidentally driven by the same switch today. A hull that later
// wants to press without being weapons-free, or to hold a defensive line while
// the guns are hot, retunes one without disturbing the other.

/// Which movement posture the ship's own alert state licenses this tick.
///
/// An ORDINAL, not a boolean, so the ladder has room to grow: `0.0` is
/// DEFENSIVE — the captain has not called red alert, and the class doctrine's
/// aggressive half is not licensed — and `1.0` is PRESSED, at red alert. A
/// doctrine compares it against its own authored threshold
/// (`fact(posture) >= param(press_posture)`), never against a literal, so a
/// future intermediate rung does not have to touch any guard that already reads
/// it.
///
/// ALWAYS seeded, on every one of the seven hosts, and never conditionally: an
/// absent fact makes every comparison against it false, which reads exactly like
/// "defensive" and would hide the difference between a stood-down ship and a
/// posture that was never wired up. That is the same rule
/// [`seed_hostile_arc_facts`] states, and it is the reason the seeding lives in
/// [`seed_helm_actuator_facts`] — the one call every host already makes — rather
/// than beside the frame lookups, which several hosts make conditionally.
pub(crate) const POSTURE_FACT: &str = "posture";
/// [`POSTURE_FACT`]'s defensive rung: the captain has not called red alert.
pub(crate) const POSTURE_DEFENSIVE: f64 = 0.0;
/// [`POSTURE_FACT`]'s pressed rung: this ship is at red alert.
pub(crate) const POSTURE_PRESSED: f64 = 1.0;

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

// ── Hostile weapon-arc facts (issue #874) ────────────────────────────────────
//
// SCOPE, precisely: `seed_hostile_arc_facts` is the one seeder, and it is
// reached from ALL SEVEN policy hosts — `ai_policy_state_tick` (via
// `seed_helm_travel_facts`), the two travel-axis hosts `ai_helm_thrust` and
// `ai_helm_steering` and the boost host `ai_helm_boost` (likewise), and the
// three remaining actuator hosts `ai_helm_impulse`, `ai_helm_lateral_thrust`
// and `ai_helm_vertical_thrust`, which call it directly alongside
// `seed_helm_actuator_facts`. So unlike the recovery/pressed facts below, these
// are available to BOTH rule guards and transition guards, on every axis.
//
// Lateral is the one that had to be closed by hand: it is the literal dodge
// axis, so a #877 doctrine reaching for `hostile_arc_exposure` reaches for it
// there first, and a fact that validated at load and then read absent for ever
// is exactly the #779 trap.
//
// Seeding on every host is safe precisely because these fold no history: each
// is a stateless reduction of this tick's geometry, taken from the ONE
// `HelmAiShipFrame` that `build_helm_ai_surfaces_frame` folded before any host
// ran. A host reading it seven times reads the same number seven times rather
// than advancing a window seven times — unlike `safe_distance_held` below,
// which is why that one stays transition-only.
//
// The reduction itself is `crate::ai::hostile_arc_exposure` over the merged
// view, whose input is the sector list `ai::server::entity_weapon_arc_sectors`
// published on each hostile's snapshot entry. The helm-radar overlay renders
// that same sector list, so the two cannot diverge (AC4).

/// How many hostile weapon arcs currently bear on this ship — in arc AND within
/// that bank's reach — summed across every hostile in the merged view.
///
/// `0.0` when clear. Always seeded, so a `fact(hostile_arc_exposure) > 0` guard
/// distinguishes "nothing bears on me" from "no reading" without either being
/// absent.
///
/// NOT scan-gated: arcs are authored hull configuration, so this reads for any
/// hostile the helm can see at all, with no Sensors sweep in the way.
pub(crate) const HOSTILE_ARC_EXPOSURE_FACT: &str = "hostile_arc_exposure";
/// Signed bearing change in DEGREES, about the nearest hostile that is bearing
/// on this ship, that would clear every one of that hostile's covering arcs by
/// the shorter way round. Positive means "further round toward the hostile's
/// starboard side".
///
/// `0.0` when nothing bears. A dodging policy needs a direction as well as a
/// gate — with the count alone it can only thrash — and the magnitude is what
/// lets a doctrine author "break contact" as a bounded manoeuvre rather than an
/// unconditional one.
///
/// Also `0.0` when [`HOSTILE_ARC_INESCAPABLE_FACT`] reads `1.0`: an all-round
/// bank has no exit bearing, so there is no honest magnitude to report. Gate on
/// the flag before acting on this.
pub(crate) const HOSTILE_ARC_ESCAPE_DEG_FACT: &str = "hostile_arc_escape_deg";
/// `1.0` when at least one arc bearing on this ship spans a full turn, so no
/// amount of turning leaves it; `0.0` otherwise.
///
/// This is what keeps "nothing bears on me" and "I cannot turn out of this"
/// apart — both of which read `hostile_arc_escape_deg == 0`. A doctrine that
/// wants to break contact from an all-round hull has to open the range rather
/// than come about, and this is the fact that tells it so.
pub(crate) const HOSTILE_ARC_INESCAPABLE_FACT: &str = "hostile_arc_inescapable";

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
pub(crate) fn target_identity_fingerprint(uuid: uuid::Uuid) -> f64 {
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
pub(crate) fn recovery_params_authored(params: &crate::world::flags::AiParams) -> bool {
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
pub(crate) fn combat_orbit_params_authored(params: &crate::world::flags::AiParams) -> bool {
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
pub(crate) fn torpedo_bearing_params_authored(params: &crate::world::flags::AiParams) -> bool {
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
pub(crate) fn artillery_params_authored(params: &crate::world::flags::AiParams) -> bool {
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
pub(crate) fn artillery_lead_speed(banks: &[crate::blaster::BlasterSystem]) -> f32 {
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
///
/// `red_alert` seeds [`POSTURE_FACT`] (issue #875). It is threaded through THIS
/// function rather than seeded beside each host's frame lookup precisely because
/// every host calls this one unconditionally, which is what makes the fact
/// unconditional too — see the constant.
pub(crate) fn seed_helm_actuator_facts(
    hazard: Option<&crate::ship::helm_planner::HazardAssessment>,
    impulse_available: bool,
    boost_available: bool,
    vertical_offset: f32,
    red_alert: bool,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    let (urgency, moving_threat) = hazard
        .map(|h| (h.urgency, h.moving_hazard_threat))
        .unwrap_or((0.0, 0.0));
    facts.set(HAZARD_URGENCY_FACT, urgency as f64);
    facts.set(
        POSTURE_FACT,
        if red_alert {
            POSTURE_PRESSED
        } else {
            POSTURE_DEFENSIVE
        },
    );
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
/// Seed the three hostile weapon-arc facts (issue #874) from this tick's helm
/// frame.
///
/// Split out of [`seed_helm_travel_facts`] so the three actuator hosts that do
/// NOT seed travel geometry — impulse, lateral and vertical — can seed these
/// anyway. Lateral is the reason: it is the dodge axis, so it is the first place
/// a #877 movement doctrine will author `fact(hostile_arc_exposure)`, and before
/// this split the guard would have validated at load and then read absent for
/// ever (the #779 shape).
///
/// Being in someone's guns is not conditional on having picked them as a target,
/// so this is deliberately independent of target resolution: a dodging policy
/// that only reacted to its own target would fly happily through a third ship's
/// broadside.
///
/// All three names are ALWAYS seeded once a frame exists, so a guard reads
/// "clear" rather than "absent" — an absent fact makes every comparison false
/// and hides the difference between clear and never-wired-up.
///
/// Folds no history: every value is a stateless read of the one `ArcExposure`
/// `build_helm_ai_surfaces_frame` reduced before any host ran this tick, so
/// calling this from seven hosts reads the same numbers seven times.
/// This tick's posture reading for one ship (issue #875), as
/// [`seed_helm_actuator_facts`] wants it.
///
/// A ship with no frame entry has no AI-operated helm axis at all, so nothing
/// resolves a movement policy for it this tick; `false` — defensive — is the
/// honest reading rather than a guess, and the fact is still SET, which is the
/// property the constant insists on.
pub(crate) fn frame_red_alert(frame_ship: Option<&HelmAiShipFrame>) -> bool {
    frame_ship.is_some_and(|sf| sf.red_alert)
}

pub(crate) fn seed_hostile_arc_facts(
    facts: &mut crate::world::flags::AiFacts,
    frame_ship: Option<&HelmAiShipFrame>,
) {
    let Some(sf) = frame_ship else {
        return;
    };
    let exposure = &sf.hostile_arc_exposure;
    facts.set(HOSTILE_ARC_EXPOSURE_FACT, exposure.covering_count as f64);
    facts.set(
        HOSTILE_ARC_ESCAPE_DEG_FACT,
        exposure.escape_offset_deg as f64,
    );
    facts.set(
        HOSTILE_ARC_INESCAPABLE_FACT,
        if exposure.inescapable { 1.0 } else { 0.0 },
    );
}

pub(crate) fn seed_helm_travel_facts(
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

    // Seeded BEFORE the target resolution below and independently of it — see
    // `seed_hostile_arc_facts`.
    seed_hostile_arc_facts(facts, Some(sf));

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
pub(crate) fn seed_recovery_facts(
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
pub(crate) fn seed_pressed_facts(
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
pub(crate) fn seed_torpedo_opportunity_facts(
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
pub(crate) fn torpedo_tubes_full(torpedoes: Option<&crate::torpedo::TorpedoSystem>) -> bool {
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
pub(crate) fn torpedo_tubes_fillable(
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
pub(crate) fn seed_memory_derived_facts(
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
