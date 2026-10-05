use super::*;

/// The planner encodes a forward Reach into a forward desired velocity and,
/// for an anchor off the starboard bow, a starboard desired facing — with
/// facing carried as a distinct field from velocity (issue #741).
#[test]
fn encodes_forward_travel_and_independent_facing() {
    // Reuse the pure codec directly: a positive throttle -> negative local-Z
    // velocity; a positive (starboard) steering -> +X facing.
    let motion = DesiredMotion {
        desired_velocity_local: Vec3::from_array(crate::ai::encode_local_velocity(0.6, 0.0)),
        desired_facing_local: Vec3::from_array(crate::ai::encode_local_facing(0.5)),
    };
    assert!(
        motion.desired_velocity_local.z < 0.0,
        "forward throttle must be a negative local-Z velocity"
    );
    assert!(
        motion.desired_facing_local.x > 0.0,
        "a starboard turn must point the desired facing to +X"
    );
    // Facing and travel are genuinely separate axes of the contract.
    assert_ne!(
        motion.desired_facing_local, motion.desired_velocity_local,
        "facing must be represented separately from travel"
    );
}

// ── A captain's navigation order outranks the pass (issue #875 AC5) ──────

fn active_pass() -> crate::ship::helm_ai::HelmPassSurface {
    crate::ship::helm_ai::HelmPassSurface {
        active: true,
        pass_legs: true,
        escape: true,
        approach_speed: 0.85,
        escape_speed: 1.0,
        recover: true,
        reengage: true,
        combat_orbit: true,
        torpedo_bearing: true,
        artillery_hold: true,
        ..Default::default()
    }
}

fn some_uuid(byte: u8) -> Option<uuid::Uuid> {
    Some(uuid::Uuid::from_bytes([byte; 16]))
}

/// **PRD #774 stories 10/11 on a hull flying an authored manoeuvre.** A
/// cleared waypoint to somewhere ELSE stands the WHOLE surface down, so the
/// planner falls to ordinary doctrine travel — the only arm that reads the
/// waypoint.
///
/// Every leg is asserted, not just `active`. The fallback `pass_legs` arm is
/// the one that actually traps a redirected ship: it fires in any state that
/// resolved no other leg verb, so a hull resting in its defensive leg would
/// keep closing on a target while its captain's order went unflown.
#[test]
fn a_cleared_waypoint_stands_the_whole_pass_surface_down() {
    let stood_down = pass_under_navigation_orders(
        active_pass(),
        Some([120.0, -45.0]),
        some_uuid(0x11),
        some_uuid(0x22),
    );
    assert!(!stood_down.active, "the surface is inactive under orders");
    assert!(
            !stood_down.pass_legs,
            "the FALLBACK leg above all: it fires in any state that resolved no              other verb, so leaving it set would keep a redirected hull closing"
        );
    for (leg, set) in [
        ("escape", stood_down.escape),
        ("recover", stood_down.recover),
        ("reengage", stood_down.reengage),
        ("combat_orbit", stood_down.combat_orbit),
        ("torpedo_bearing", stood_down.torpedo_bearing),
        ("artillery_hold", stood_down.artillery_hold),
    ] {
        assert!(!set, "{leg} must not survive a navigation order");
    }
}

/// The other half, and the one that makes the assertion above mean
/// something: with NO cleared waypoint the surface is passed through
/// untouched, so an unredirected hull flies its doctrine exactly as before.
#[test]
fn an_uncleared_helm_flies_its_authored_pass_untouched() {
    let pass = active_pass();
    assert_eq!(
            pass_under_navigation_orders(pass, None, None, some_uuid(0x22)),
            pass,
            "no navigation order ⇒ the authored manoeuvre is untouched. A              precedence that stood the doctrine down unconditionally would pass              the test above and delete every class doctrine in the fleet."
        );
}

/// **The ship's own Navigation is not a redirection.** A waypoint anchored
/// to the very entity the pass is attacking names the same destination the
/// manoeuvre is already flying at, so the doctrine survives.
///
/// This is not a hypothetical: `operate_navigation_ai` waypoints the top
/// Helm-relevant `Destroy` target, so on any hull that declares a navigation
/// system — every Alliance hull — this is the STEADY state of a ship
/// prosecuting a named target. A stand-down on the mere presence of a
/// clearance had a ship's own backfilled Navigation delete its own
/// backfilled Helm's class doctrine on every such mission, permanently:
/// `NavigationWaypoint::set` is idempotent for an anchored target that
/// merely moves, so the clearance latches once and never lifts.
#[test]
fn a_waypoint_onto_the_passs_own_target_leaves_the_doctrine_flying() {
    let pass = active_pass();
    assert_eq!(
        pass_under_navigation_orders(pass, Some([120.0, -45.0]), some_uuid(0x33), some_uuid(0x33),),
        pass,
        "a destination that IS the pass target is not a conflicting order"
    );
}

/// Two `None`s are not a match. A `Free` waypoint names a place and anchors
/// to nobody, and a hull with no resolved target has nothing to compare — so
/// a bare `cleared_anchor == pass_target` would read those two absences as
/// agreement and hand a tap-to-place order straight back to the doctrine.
#[test]
fn a_free_waypoint_still_redirects_a_hull_with_no_target() {
    assert_eq!(
        pass_under_navigation_orders(active_pass(), Some([120.0, -45.0]), None, None),
        crate::ship::helm_ai::HelmPassSurface::default(),
        "an anchorless destination is always a redirection"
    );
}
