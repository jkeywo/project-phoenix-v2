//! Authored Helm AI tuning vocabulary.
/// The margin, in degrees, at which the ring stops caring about speed and starts
/// caring about staying pointed. When [`OWN_BANK_ARC_MARGIN_DEG_FACT`] falls to
/// or below this, the ring is flown at [`ARC_KEEP_SPEED_PARAM`] instead of
/// [`COMBAT_ORBIT_SPEED_PARAM`].
pub const ARC_KEEP_MARGIN_DEG_PARAM: &str = "arc_keep_margin_deg";

/// The throttle fraction flown while the target is near an arc edge.
///
/// Lower than `combat_orbit_speed` is the whole point, and the mechanism it
/// leans on is already in the physics: `[helm_console] low_speed_turn_boost`
/// gives `max_yaw_rate * (1 + boost * (1 - speed_fraction))`, so backing off the
/// throttle buys turn authority. Slowing to hold the target in arc is therefore
/// not a new capability, it is the doctrine finally spending one the hull has
/// always had.
///
/// Validated `> 0.0` at load. Zero does not mean "slow", it means STOP, and a
/// parked hull inside a hostile's guns is the exact hazard [`COMBAT_ORBIT_PARAMS`]
/// was given its all-or-nothing gate to prevent.
pub const ARC_KEEP_SPEED_PARAM: &str = "arc_keep_speed";

/// Both arc-keeping scalars, gated and validated as ONE unit.
pub const ARC_KEEP_PARAMS: &[&str] = &[ARC_KEEP_MARGIN_DEG_PARAM, ARC_KEEP_SPEED_PARAM];

/// At or below this many HP on [`OWN_FACING_SHIELD_HP_FACT`], the ring reverses
/// so the OTHER broadside faces the enemy while the hurt side recovers.
///
/// Owner's priority order: shield protection beats arc-keeping. A flip costs arc
/// dwell — the guns swap sides and the target crosses the blind wedge to get
/// there — and that cost is accepted deliberately.
pub const WEAK_SHIELD_FLIP_HP_PARAM: &str = "weak_shield_flip_hp";

/// The HP the arc that TRIPPED the latch has to climb back to before the ring
/// flips back.
///
/// Strictly at or above [`WEAK_SHIELD_FLIP_HP_PARAM`], validated at load rather
/// than clamped at read: a restore floor below the flip floor is not a deadband,
/// and quietly substituting the value the author "appears to have meant" is how a
/// hull ends up flying a doctrine nobody wrote down.
pub const WEAK_SHIELD_RESTORE_HP_PARAM: &str = "weak_shield_restore_hp";

/// The minimum time, in seconds, a flip is held before the restore reading is
/// even consulted.
///
/// The second half of the limit-cycle fix, and it guards a case arc identity
/// alone does not: an arc can cross the restore threshold for a few ticks from
/// regeneration or from a focus change while the swing that the flip ordered is
/// still in progress. Unflipping there costs the whole manoeuvre and buys
/// nothing. Authored rather than derived because how long a broadside needs to
/// be worth presenting is a hull's business, not this module's.
pub const WEAK_SHIELD_FLIP_DWELL_SECS_PARAM: &str = "weak_shield_flip_dwell_secs";

/// All three flip scalars, gated and validated as ONE unit.
pub const WEAK_SHIELD_FLIP_PARAMS: &[&str] = &[
    WEAK_SHIELD_FLIP_HP_PARAM,
    WEAK_SHIELD_RESTORE_HP_PARAM,
    WEAK_SHIELD_FLIP_DWELL_SECS_PARAM,
];
