use super::*;
use crate::entities::config::PhaserBankConfig;

fn bank(id: &str, facing_deg: f32, fire_arc_deg: f32, auto_arc_deg: f32) -> PhaserBankConfig {
    PhaserBankConfig {
        id: id.to_string(),
        facing_deg,
        fire_arc_deg,
        auto_arc_deg,
        beam_range: 0.0,
        shield_pierce: None,
        marker: None,
        ..Default::default()
    }
}

fn default_system() -> PhaserSystem {
    // Replicates the legacy port/starboard pair at 270° / 240°.
    let banks = vec![
        bank("port", -90.0, 270.0, 240.0),
        bank("starboard", 90.0, 270.0, 240.0),
    ];
    PhaserSystem::from_configs(&banks, 3.0, 40.0)
}

// ── Cooldown ──────────────────────────────────────────────────────────

#[test]
fn banks_are_ready_initially() {
    let sys = default_system();
    assert!(sys.bank("port").unwrap().is_ready());
    assert!(sys.bank("starboard").unwrap().is_ready());
}

#[test]
fn firing_sets_cooldown() {
    let mut sys = default_system();
    assert!(sys.fire_manual("port"));
    assert!(!sys.bank("port").unwrap().is_ready());
}

#[test]
fn tick_reduces_cooldown() {
    let mut sys = default_system();
    sys.fire_manual("port");
    sys.tick(1.0);
    assert!(!sys.bank("port").unwrap().is_ready());
}

#[test]
fn tick_to_zero_makes_bank_ready() {
    let mut sys = default_system();
    sys.fire_manual("port");
    sys.tick(sys.cooldown_secs);
    assert!(sys.bank("port").unwrap().is_ready());
}

#[test]
fn tick_does_not_go_negative() {
    let mut sys = default_system();
    sys.fire_manual("port");
    sys.tick(100.0);
    assert_eq!(sys.bank("port").unwrap().cooldown_remaining, 0.0);
}

#[test]
fn fire_while_on_cooldown_returns_false() {
    let mut sys = default_system();
    sys.fire_manual("port");
    assert!(!sys.fire_manual("port"));
}

#[test]
fn banks_are_independent() {
    let mut sys = default_system();
    sys.fire_manual("port");
    assert!(!sys.bank("port").unwrap().is_ready());
    assert!(sys.bank("starboard").unwrap().is_ready());
}

#[test]
fn unknown_bank_id_returns_none() {
    let sys = default_system();
    assert!(sys.bank("dorsal").is_none());
    assert!(!sys.is_in_fire_arc("dorsal", 0.0, -10.0, 0.0, 0.0, 0.0));
}

// ── Fire arc (270° default) ───────────────────────────────────────────

#[test]
fn target_ahead_in_arc_for_both_banks() {
    let sys = default_system();
    assert!(sys.is_in_fire_arc("port", 0.0, -20.0, 0.0, 0.0, 0.0));
    assert!(sys.is_in_fire_arc("starboard", 0.0, -20.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_directly_to_port_in_fire_arc_for_port_bank() {
    let sys = default_system();
    assert!(sys.is_in_fire_arc("port", -20.0, 0.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_directly_to_starboard_in_fire_arc_for_starboard_bank() {
    let sys = default_system();
    assert!(sys.is_in_fire_arc("starboard", 20.0, 0.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_directly_to_starboard_outside_port_fire_arc() {
    let sys = default_system();
    // Bearing = +π/2; port facing = −π/2; |Δ| = π > 135° → out.
    assert!(!sys.is_in_fire_arc("port", 20.0, 0.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_directly_to_port_outside_starboard_fire_arc() {
    let sys = default_system();
    assert!(!sys.is_in_fire_arc("starboard", -20.0, 0.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_aft_in_fire_arc_for_both_banks() {
    let sys = default_system();
    // Aft = (0, +20): bearing = atan2(0, -20) = π. |π − ±π/2| = π/2 ≤ 135°.
    assert!(sys.is_in_fire_arc("port", 0.0, 20.0, 0.0, 0.0, 0.0));
    assert!(sys.is_in_fire_arc("starboard", 0.0, 20.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_out_of_range_not_in_fire_arc() {
    let sys = default_system();
    assert!(!sys.is_in_fire_arc("port", 0.0, -100.0, 0.0, 0.0, 0.0));
}

// ── Auto-fire arc ─────────────────────────────────────────────────────

#[test]
fn target_slightly_starboard_of_forward_in_auto_arc_for_both() {
    let sys = default_system();
    assert!(sys.is_in_auto_arc("port", 5.0, -20.0, 0.0, 0.0, 0.0));
    assert!(sys.is_in_auto_arc("starboard", 5.0, -20.0, 0.0, 0.0, 0.0));
}

#[test]
fn target_directly_to_starboard_outside_port_auto_arc() {
    let sys = default_system();
    // Port auto arc = 240°, half = 120°. Bearing to (20,0) = +π/2 = 90°.
    // |90 − (−90)| = 180° > 120° → out.
    assert!(!sys.is_in_auto_arc("port", 20.0, 0.0, 0.0, 0.0, 0.0));
}

// ── Mode and auto-fire ────────────────────────────────────────────────

#[test]
fn default_mode_is_auto() {
    let sys = default_system();
    assert_eq!(sys.mode, PhaserMode::Auto);
}

#[test]
fn set_mode_changes_mode() {
    let mut sys = default_system();
    sys.set_mode(PhaserMode::Manual);
    assert_eq!(sys.mode, PhaserMode::Manual);
}

#[test]
fn auto_fire_fires_banks_in_arc() {
    let mut sys = default_system();
    let fired = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
    assert!(fired.contains(&"port".to_string()));
    assert!(fired.contains(&"starboard".to_string()));
}

#[test]
fn auto_fire_does_nothing_in_manual_mode() {
    let mut sys = default_system();
    sys.set_mode(PhaserMode::Manual);
    let fired = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
    assert!(fired.is_empty());
}

#[test]
fn auto_fire_skips_bank_on_cooldown() {
    let mut sys = default_system();
    sys.fire_manual("port"); // port on cooldown
    let fired = sys.auto_fire(0.0, -20.0, 0.0, 0.0, 0.0);
    assert!(!fired.contains(&"port".to_string()));
    assert!(fired.contains(&"starboard".to_string()));
}

#[test]
fn auto_fire_out_of_range_does_not_fire() {
    let mut sys = default_system();
    let fired = sys.auto_fire(0.0, -100.0, 0.0, 0.0, 0.0);
    assert!(fired.is_empty());
}

// ── Per-bank beam_range fallback ──────────────────────────────────────

#[test]
fn bank_with_zero_beam_range_uses_fallback() {
    let banks = vec![bank("port", -90.0, 270.0, 240.0)];
    let sys = PhaserSystem::from_configs(&banks, 3.0, 50.0);
    assert_eq!(sys.bank("port").unwrap().beam_range, 50.0);
}

#[test]
fn bank_with_explicit_beam_range_overrides_fallback() {
    let mut b = bank("port", -90.0, 270.0, 240.0);
    b.beam_range = 25.0;
    let sys = PhaserSystem::from_configs(&[b], 3.0, 50.0);
    assert_eq!(sys.bank("port").unwrap().beam_range, 25.0);
}

// ── Production fore/aft 270° geometry at non-zero yaw ─────────────────
//
// These tests use the same `ship_local` + `in_arc` helpers the runtime
// gate uses (see `console::weapons::handle_fire_phaser`). They
// exercise the *actual* player ship config: fore facing 0°, aft facing
// 180°, each with a 270° arc — so the blind cone is the 90° wedge
// directly opposite each bank's facing.
//
// We sweep over a range of ship yaws to catch any rotation-frame bugs.

fn fwd_xz(yaw: f32) -> (f32, f32) {
    // Matches `src/ship/physics.rs`: forward = (sin yaw, -cos yaw).
    (simmath::sin(yaw), -simmath::cos(yaw))
}

#[test]
fn fore_bank_270_rejects_target_directly_aft_at_any_yaw() {
    for &yaw in &[
        0.0_f32,
        0.5,
        1.0,
        std::f32::consts::FRAC_PI_2,
        2.0,
        PI,
        -1.0,
        -2.5,
    ] {
        let (fwd_x, fwd_z) = fwd_xz(yaw);
        // Place target 20 units directly behind the ship in world space.
        let tx = -fwd_x * 20.0;
        let tz = -fwd_z * 20.0;
        let (rx, ry) = ship_local(tx, tz, 0.0, 0.0, yaw);
        assert!(
                !in_arc(rx, ry, 0.0, 270.0),
                "fore bank (facing 0°, 270° arc) must reject directly-aft target at yaw={yaw}: rx={rx}, ry={ry}"
            );
    }
}

#[test]
fn aft_bank_270_rejects_target_directly_ahead_at_any_yaw() {
    for &yaw in &[
        0.0_f32,
        0.5,
        1.0,
        std::f32::consts::FRAC_PI_2,
        2.0,
        PI,
        -1.0,
        -2.5,
    ] {
        let (fwd_x, fwd_z) = fwd_xz(yaw);
        // Place target 20 units directly ahead of the ship in world space.
        let tx = fwd_x * 20.0;
        let tz = fwd_z * 20.0;
        let (rx, ry) = ship_local(tx, tz, 0.0, 0.0, yaw);
        assert!(
                !in_arc(rx, ry, 180.0, 270.0),
                "aft bank (facing 180°, 270° arc) must reject directly-ahead target at yaw={yaw}: rx={rx}, ry={ry}"
            );
    }
}

#[test]
fn both_banks_accept_target_abeam_at_any_yaw() {
    for &yaw in &[
        0.0_f32,
        0.5,
        1.0,
        std::f32::consts::FRAC_PI_2,
        2.0,
        PI,
        -1.0,
        -2.5,
    ] {
        // Right (starboard) vector: (cos yaw, sin yaw) per beam_render.rs.
        let right_x = simmath::cos(yaw);
        let right_z = simmath::sin(yaw);
        // Place target 20 units directly to starboard.
        let tx = right_x * 20.0;
        let tz = right_z * 20.0;
        let (rx, ry) = ship_local(tx, tz, 0.0, 0.0, yaw);
        assert!(
            in_arc(rx, ry, 0.0, 270.0),
            "fore bank must accept abeam-starboard target at yaw={yaw}: rx={rx}, ry={ry}"
        );
        assert!(
            in_arc(rx, ry, 180.0, 270.0),
            "aft bank must accept abeam-starboard target at yaw={yaw}: rx={rx}, ry={ry}"
        );
    }
}
