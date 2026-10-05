use super::*;
use std::f32::consts::PI;

// ── ShieldFacing ─────────────────────────────────────────────────────────

#[test]
fn facing_starts_at_full_hp_and_online() {
    let f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
    assert_eq!(f.hp, 100);
    assert!(f.is_online());
}

#[test]
fn damage_reduces_hp() {
    let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
    let passthrough = f.apply_damage(30);
    assert_eq!(f.hp, 70);
    assert_eq!(passthrough, 0);
}

#[test]
fn damage_that_depletes_facing_sends_overflow_to_hull() {
    let mut f = ShieldFacing::new("Fore", 50, 5.0, 10.0);
    let passthrough = f.apply_damage(70); // 70 > 50 max_hp
    assert_eq!(f.hp, 0);
    assert_eq!(passthrough, 20);
    assert!(!f.is_online());
}

#[test]
fn exact_depletion_leaves_no_passthrough_but_goes_offline() {
    let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
    let passthrough = f.apply_damage(100);
    assert_eq!(passthrough, 0);
    assert!(!f.is_online());
}

#[test]
fn offline_facing_passes_all_damage_to_hull() {
    let mut f = ShieldFacing::new("Fore", 100, 5.0, 10.0);
    f.apply_damage(100); // deplete → offline
    let passthrough = f.apply_damage(40);
    assert_eq!(passthrough, 40);
}

#[test]
fn offline_timer_counts_down_via_tick() {
    let mut f = ShieldFacing::new("Fore", 100, 0.0, 10.0);
    f.apply_damage(100); // offline for 10s
    f.tick(4.0);
    assert!(!f.is_online());
    assert!((f.offline_remaining - 6.0).abs() < 1e-4);
}

/// Issue #788 (AC1). A collapsed facing waits out the authored no-damage
/// delay and then comes back **empty**, climbing at its authored
/// `regen_per_sec`. It used to snap straight to `max_hp` the instant the
/// timer expired.
///
/// This is a deliberate behaviour change, and the assertion below is the
/// whole reason a "wait until shields are back to 75%" doctrine can exist at
/// all: under the old snap there was no instant at which a recovering shield
/// was partially recovered, so any fractional threshold was either already
/// met or unreachable.
#[test]
fn facing_comes_back_online_empty_and_regenerates_at_its_authored_rate() {
    let mut f = ShieldFacing::new("Fore", 100, 10.0, 10.0);
    f.apply_damage(100); // collapse → offline for 10s

    // The delay is a delay: nothing regenerates during it.
    f.tick(9.0);
    assert!(!f.is_online());
    assert_eq!(f.hp, 0);

    // The tick the delay expires, the facing is online again — and empty.
    f.tick(1.0);
    assert!(f.is_online(), "the authored offline duration has elapsed");
    assert_eq!(
        f.hp, 0,
        "a recovered facing restarts from zero, not from max_hp"
    );

    // From there it climbs at the authored rate, and only at that rate.
    f.tick(3.0); // 10 hp/s × 3 s
    assert_eq!(f.hp, 30);
    f.tick(4.5); // +45
    assert_eq!(f.hp, 75);
    // ...all the way back to full, where it caps.
    f.tick(10.0);
    assert_eq!(f.hp, 100);
}

/// Issue #788 (AC1/AC8, interrupted regeneration): a hit while the facing is
/// still ramping knocks it back to zero and collapses it again for a FRESH
/// no-damage delay, with the surplus passing to the hull.
#[test]
fn a_hit_during_the_regen_ramp_collapses_the_facing_again() {
    let mut f = ShieldFacing::new("Fore", 100, 10.0, 10.0);
    f.apply_damage(100); // collapse
    f.tick(10.0); // back online, empty
    f.tick(3.0); // ramped to 30
    assert_eq!(f.hp, 30);

    let passthrough = f.apply_damage(50);
    assert_eq!(f.hp, 0, "the ramp is knocked back to zero");
    assert_eq!(passthrough, 20, "the surplus reaches the hull");
    assert!(!f.is_online(), "and the facing collapses again");
    assert!(
        (f.offline_remaining - 10.0).abs() < 1e-4,
        "the no-damage delay restarts in full, got {}",
        f.offline_remaining
    );

    // The restarted delay is real: it holds the facing down for its full
    // duration rather than resuming the interrupted ramp.
    f.tick(9.0);
    assert!(!f.is_online());
    assert_eq!(f.hp, 0);
}

/// A facing sitting at 0 HP while ONLINE (the first instant of its ramp) is
/// a state that could not occur before #788. A zero-damage hit on it must
/// not re-arm the offline timer — that would freeze the ramp for ever on any
/// ship being grazed by rounded-to-zero damage.
#[test]
fn a_zero_damage_hit_on_an_empty_online_facing_does_not_re_collapse_it() {
    let mut f = ShieldFacing::new("Fore", 100, 10.0, 10.0);
    f.apply_damage(100);
    f.tick(10.0);
    assert!(f.is_online());
    assert_eq!(f.hp, 0);

    assert_eq!(f.apply_damage(0), 0);
    assert!(f.is_online(), "a no-op hit must not collapse the facing");
    f.tick(1.0);
    assert_eq!(f.hp, 10, "the ramp continues uninterrupted");
}

/// The whole-ship reading a recovery doctrine gates on: it traverses the
/// interval as the arcs ramp, rather than jumping between 0 and 1.
#[test]
fn system_fraction_tracks_the_recovery_ramp() {
    let mut s = ShieldSystem::new(&ShieldConfig {
        num_facings: 1,
        max_hp: 40,
        regen_per_sec: 10.0,
        offline_duration: 4.0,
    });
    assert!((s.fraction() - 1.0).abs() < 1e-6);

    s.apply_damage(40, 0.0); // collapse the single arc
    assert_eq!(s.fraction(), 0.0);

    s.tick(4.0); // no-damage delay expires: online, empty
    assert_eq!(s.fraction(), 0.0);
    s.tick(2.0); // +20 of 40
    assert!((s.fraction() - 0.5).abs() < 1e-6, "got {}", s.fraction());
    s.tick(1.0); // +10 → 30 of 40
    assert!((s.fraction() - 0.75).abs() < 1e-6, "got {}", s.fraction());
    s.tick(10.0);
    assert!((s.fraction() - 1.0).abs() < 1e-6);
}

#[test]
fn system_fraction_of_a_shieldless_hull_is_zero() {
    let s = ShieldSystem::new(&ShieldConfig {
        num_facings: 1,
        max_hp: 0,
        ..Default::default()
    });
    assert_eq!(s.fraction(), 0.0);
}

#[test]
fn regen_increases_hp_while_online() {
    let mut f = ShieldFacing::new("Fore", 100, 10.0, 10.0);
    f.apply_damage(40); // hp = 60
    f.tick(2.0); // +20 → hp = 80
    assert_eq!(f.hp, 80);
}

#[test]
fn regen_does_not_exceed_max_hp() {
    let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
    f.apply_damage(10); // hp = 90
    f.tick(10.0); // +500 → capped at 100
    assert_eq!(f.hp, 100);
}

#[test]
fn no_regen_while_offline() {
    let mut f = ShieldFacing::new("Fore", 100, 50.0, 10.0);
    f.apply_damage(100); // offline
    f.tick(1.0); // timer ticks, no regen
    assert_eq!(f.hp, 0);
}

// ── ShieldSystem facing index ────────────────────────────────────────────

#[test]
fn four_facings_default_layout() {
    let s = ShieldSystem::default(); // 4 facings
                                     // Forward (bearing 0) → facing 0 (Fore)
    assert_eq!(s.facing_index_for_bearing(0.0), 0);
    // 90° left (port, bearing -PI/2) → facing 1 (Port)
    assert_eq!(s.facing_index_for_bearing(-PI / 2.0), 1);
    // Directly aft (PI or -PI) → facing 2 (Aft)
    assert_eq!(s.facing_index_for_bearing(PI), 2);
    // 90° right (starboard, bearing +PI/2) → facing 3 (Starboard)
    assert_eq!(s.facing_index_for_bearing(PI / 2.0), 3);
}

#[test]
fn two_facings_fore_aft_layout() {
    let config = ShieldConfig {
        num_facings: 2,
        ..Default::default()
    };
    let s = ShieldSystem::new(&config);
    assert_eq!(s.facing_index_for_bearing(0.0), 0); // fore
    assert_eq!(s.facing_index_for_bearing(PI), 1); // aft
                                                   // 45° to the left: still in the fore hemisphere
    assert_eq!(s.facing_index_for_bearing(-PI / 4.0), 0);
}

// ── ShieldSystem damage routing ──────────────────────────────────────────

#[test]
fn damage_routed_to_correct_facing() {
    let mut s = ShieldSystem::default();
    s.apply_damage(20, 0.0); // hits fore
    assert_eq!(s.facings[0].hp, 80);
    assert_eq!(s.facings[1].hp, 100); // port untouched
}

#[test]
fn damage_passthrough_when_facing_depleted() {
    let config = ShieldConfig {
        max_hp: 50,
        ..Default::default()
    };
    let mut s = ShieldSystem::new(&config);
    let passthrough = s.apply_damage(60, 0.0); // fore only has 50
    assert_eq!(passthrough, 10);
}

// ── ShieldSystem tick ────────────────────────────────────────────────────

#[test]
fn tick_regenerates_all_facings() {
    let mut s = ShieldSystem::default(); // regen 2/s
    s.apply_damage(20, 0.0); // fore: 80
    s.apply_damage(10, PI / 2.0); // starboard: 90
    s.tick(2.0); // +4 fore → 84, +4 starboard → 94
    assert_eq!(s.facings[0].hp, 84);
    assert_eq!(s.facings[3].hp, 94);
}

/// The `shields` power group's `ModifierSlot::ShieldRegen` scales every
/// facing's authored rate (issue #952). ×1.0 is the authored rate exactly,
/// so a hull whose reactor never moves the group is unchanged.
#[test]
fn regen_scale_multiplies_every_facings_authored_rate() {
    for (scale, expected_gain) in [(1.0f32, 4), (2.0, 8), (0.5, 2)] {
        let mut s = ShieldSystem::default(); // regen 2/s
        s.apply_damage(20, 0.0); // fore: 80
        s.tick_with_regen_scale(2.0, scale);
        assert_eq!(
            s.facings[0].hp,
            80 + expected_gain,
            "regen scale x{scale} over 2 s at 2 HP/s"
        );
    }
}

/// A negative or NaN slot value must not DRAIN a shield. `ShipModifiers`
/// cannot produce one (`rebuild_cache` folds a negative sum as
/// `1/(1+|sum|)`, always positive), but nothing in this pure module knows
/// that, and a regen that ran backwards would be indistinguishable from
/// being shot at.
#[test]
fn a_negative_regen_scale_does_not_drain_the_shield() {
    let mut s = ShieldSystem::default();
    s.apply_damage(20, 0.0);
    s.tick_with_regen_scale(2.0, -5.0);
    assert_eq!(s.facings[0].hp, 80);
}

// ── ShieldSystem snapshot ────────────────────────────────────────────────

#[test]
fn snapshot_returns_all_four_facings() {
    let s = ShieldSystem::default();
    let snaps = s.snapshot();
    assert_eq!(snaps.len(), 4);
    // Default facings carry `strings.csv` ids now (issue #977); the client
    // resolves them through `localiseTree`.
    assert_eq!(snaps[0].label, "shield.facing.fore");
    assert_eq!(snaps[1].label, "shield.facing.port");
    assert_eq!(snaps[2].label, "shield.facing.aft");
    assert_eq!(snaps[3].label, "shield.facing.starboard");
}

#[test]
fn snapshot_reflects_current_hp_and_online_status() {
    let mut s = ShieldSystem::default();
    s.apply_damage(100, 0.0); // deplete fore
    let snaps = s.snapshot();
    assert_eq!(snaps[0].hp, 0);
    assert!(!snaps[0].online);
    assert_eq!(snaps[1].hp, 100);
    assert!(snaps[1].online);
}

// ── configurable arcs ────────────────────────────────────────────────────

#[test]
fn single_facing_absorbs_all_bearings() {
    let config = ShieldConfig {
        num_facings: 1,
        ..Default::default()
    };
    let mut s = ShieldSystem::new(&config);
    s.apply_damage(10, 0.0);
    s.apply_damage(10, PI);
    s.apply_damage(10, PI / 2.0);
    assert_eq!(s.facings[0].hp, 70);
}

#[test]
fn custom_config_max_hp_and_regen() {
    let config = ShieldConfig {
        num_facings: 2,
        max_hp: 200,
        regen_per_sec: 20.0,
        offline_duration: 5.0,
    };
    let s = ShieldSystem::new(&config);
    assert_eq!(s.facings.len(), 2);
    assert_eq!(s.facings[0].max_hp, 200);
    assert_eq!(s.facings[0].hp, 200);
}

// ── attacker_bearing_relative ────────────────────────────────────────────

#[test]
fn attacker_directly_ahead_gives_zero_bearing() {
    // Ship at origin, yaw = 0, attacker in front (negative Z)
    let b = attacker_bearing_relative(0.0, -10.0, 0.0, 0.0, 0.0);
    assert!(b.abs() < 1e-4, "expected ~0, got {b}");
}

#[test]
fn attacker_directly_aft_gives_pi_bearing() {
    let b = attacker_bearing_relative(0.0, 10.0, 0.0, 0.0, 0.0);
    assert!((b.abs() - PI).abs() < 1e-4, "expected ~±π, got {b}");
}

#[test]
fn attacker_to_starboard_gives_positive_bearing() {
    // Starboard is to the right; with yaw=0 forward=-Z, right = +X
    let b = attacker_bearing_relative(10.0, 0.0, 0.0, 0.0, 0.0);
    assert!((b - PI / 2.0).abs() < 1e-4, "expected ~+π/2, got {b}");
}

#[test]
fn attacker_to_port_gives_negative_bearing() {
    let b = attacker_bearing_relative(-10.0, 0.0, 0.0, 0.0, 0.0);
    assert!((b + PI / 2.0).abs() < 1e-4, "expected ~-π/2, got {b}");
}

#[test]
fn bearing_accounts_for_ship_yaw() {
    // Ship rotated 90° clockwise (yaw = +π/2).
    // Attacker is in the world's +X direction; relative to the ship that
    // should now be directly ahead.
    let b = attacker_bearing_relative(10.0, 0.0, 0.0, 0.0, PI / 2.0);
    assert!(b.abs() < 1e-4, "expected ~0, got {b}");
}

#[test]
fn bearing_routes_to_fore_facing() {
    let s = ShieldSystem::default(); // 4 facings
                                     // Attacker straight ahead → Fore (index 0)
    let b = attacker_bearing_relative(0.0, -10.0, 0.0, 0.0, 0.0);
    assert_eq!(s.facing_index_for_bearing(b), 0);
}

#[test]
fn bearing_routes_to_aft_facing() {
    let s = ShieldSystem::default();
    // Attacker straight behind → Aft (index 2)
    let b = attacker_bearing_relative(0.0, 10.0, 0.0, 0.0, 0.0);
    assert_eq!(s.facing_index_for_bearing(b), 2);
}

// ── Focus mechanics ────────────────────────────────────────────────────

#[test]
fn default_focused_facing_is_none() {
    let s = ShieldSystem::default();
    assert!(s.focused_facing.is_none());
    for f in &s.facings {
        assert!(!f.is_focused);
    }
}

#[test]
fn set_focused_facing_toggles_the_focused_flag() {
    let mut s = ShieldSystem::default();
    s.set_focused_facing(Some(0)); // Focus Fore
    assert_eq!(s.focused_facing, Some(0));
    assert!(s.facings[0].is_focused);
    assert!(!s.facings[1].is_focused);
    assert!(!s.facings[2].is_focused);
    assert!(!s.facings[3].is_focused);
}

#[test]
fn set_focused_facing_none_clears_focus() {
    let mut s = ShieldSystem::default();
    s.set_focused_facing(Some(0));
    assert!(s.facings[0].is_focused);
    s.set_focused_facing(None);
    assert!(s.focused_facing.is_none());
    for f in &s.facings {
        assert!(!f.is_focused);
    }
}

#[test]
fn focused_facing_gets_bonus_max_hp_and_regen() {
    let mut s = ShieldSystem::default();
    assert_eq!(s.facings[0].max_hp, 100);
    s.set_focused_facing(Some(0));
    // Default focus config: bonus_max_hp=50, bonus_regen=5.0
    assert_eq!(s.facings[0].max_hp, 150);
    assert!((s.facings[0].regen_per_sec - 7.0).abs() < 1e-4);
}

#[test]
fn non_focused_facings_get_penalty_max_hp_and_regen() {
    let mut s = ShieldSystem::default();
    s.set_focused_facing(Some(0)); // Focus Fore
                                   // Default: penalty_max_hp=25, penalty_regen=1.0
    assert_eq!(s.facings[1].max_hp, 75); // Port
    assert_eq!(s.facings[2].max_hp, 75); // Aft
    assert_eq!(s.facings[3].max_hp, 75); // Starboard
    assert!((s.facings[1].regen_per_sec - 1.0).abs() < 1e-4);
}

#[test]
fn clearing_focus_restores_base_max_hp_and_regen_for_all() {
    let mut s = ShieldSystem::default();
    s.set_focused_facing(Some(0));
    assert_eq!(s.facings[0].max_hp, 150);
    assert_eq!(s.facings[1].max_hp, 75);
    // Simulate the focused facing having regen'd above base max_hp.
    s.facings[0].hp = 130;
    s.set_focused_facing(None);
    for f in &s.facings {
        assert_eq!(f.max_hp, 100);
        assert!((f.regen_per_sec - 2.0).abs() < 1e-4);
    }
    // HP is NOT snapped immediately — it decays gradually via tick().
    assert_eq!(
        s.facings[0].hp, 130,
        "HP should persist above max after clear"
    );
    s.tick(0.5); // decay_rate=10/s * 0.5s = 5 HP decay
    assert_eq!(s.facings[0].hp, 125);
    s.tick(3.0); // 125 - min(10*3, 125-100) = 100
    assert_eq!(s.facings[0].hp, 100);
    // Once at max, regen applies normally on subsequent ticks.
    s.tick(0.5); // regen 2/s → 100 + 1.0 = 101.0 → capped to 100
    assert_eq!(s.facings[0].hp, 100);
}

#[test]
fn non_focused_facing_decays_when_above_reduced_max() {
    let mut s = ShieldSystem::default();
    // Damage fore (facing 0) so HP drops, then focus Port (facing 1).
    // Port becomes focused at 150 max_hp. Fore becomes non-focused at 75 max_hp.
    s.facings[1].hp = 130; // Port HP above base 100 (will become 75 effective max)
    s.facings[3].hp = 120; // Starboard HP above base 100
    s.set_focused_facing(Some(1)); // Focus Port

    // After recalculate_focus, facings 0,2,3 have effective max=75 with HP above that.
    // Port (focused) has effective max=150 with HP=130 (no decay).
    s.tick(0.5); // decay_rate=10/s * 0.5s = 5 HP decay

    assert!(!s.facings[0].is_focused);
    assert!(s.facings[1].is_focused);
    // Facing 0 (Fore): base was 100, but focus recalculate doesn't clamp HP.
    // After recalculate max_hp=75, HP=100 (above max). Decays at 10/s for 0.5s = 5.
    assert_eq!(s.facings[0].hp, 95);
    // Facing 3 (Starboard): HP was 120, max becomes 75. Decays 10/s for 0.5s = 5.
    assert_eq!(s.facings[3].hp, 115);
    // Facing 1 (Port, focused): normal tick (base 2.0 + bonus 5.0 = 7.0/s). 130 + 7.0*0.5 ≈ +3 → 133.
    assert_eq!(s.facings[1].hp, 133);
}

#[test]
fn non_focused_facing_stops_decaying_when_at_or_below_reduced_max() {
    let mut s = ShieldSystem::default();
    // Fore (facing 0) at 80 HP, gets reduced max=75 when another arc focused.
    s.facings[0].hp = 80;
    s.set_focused_facing(Some(1)); // Focus Port
                                   // Fore max=75, HP=80 → 80 - 10*0.5 = 75 → should decay to exactly 75
    s.tick(0.5);
    assert_eq!(s.facings[0].hp, 75);
    // Next tick: HP=75 ≤ max=75 → no more decay
    s.tick(0.5);
    assert_eq!(s.facings[0].hp, 75);
}

#[test]
fn snapshot_includes_is_focused() {
    let mut s = ShieldSystem::default();
    s.set_focused_facing(Some(0));
    let snaps = s.snapshot();
    assert!(snaps[0].is_focused);
    assert!(!snaps[1].is_focused);
    assert!(!snaps[2].is_focused);
    assert!(!snaps[3].is_focused);
}

#[test]
fn focused_facing_not_subject_to_decay_rate() {
    let mut s = ShieldSystem::default();
    // Focus Fore so effective max becomes 150. HP above base can only
    // be reduced by the normal regen cap, not by focus decay.
    s.set_focused_facing(Some(0));
    s.facings[0].hp = 200;
    s.tick(0.5);
    // Normal regen tick caps at max_hp=150: (200 + 7.0*0.5 → +3).min(150) = 150
    assert_eq!(s.facings[0].hp, 150);
    // The decay code (which only targets non-focused facings) did not run,
    // confirming the focused arc does not get focus-decayed.
}

/// End-to-end TOML-driven wiring check: build the runtime `ShieldSystem`
/// the same way `spawn_game_start_entities` does (parse alliance_battleship.toml
/// → ShieldsBaseConfig::to_runtime → ShieldSystem::new) and assert the
/// facings reflect the TOML. Changing `max_hp = 140` to `max_hp = 999`
/// in `[shields_console.base]` would fail this test.
#[test]
fn shield_system_reflects_battleship_toml_shields_console_base_block() {
    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document `spawn_game_start_entities` reads.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_battleship.toml",
    )
    .expect("alliance_battleship.toml must compose and parse");
    let base = config
        .shields_console
        .expect("alliance_battleship must declare [shields_console]")
        .base
        .expect("alliance_battleship must declare [shields_console.base]");
    let shield_config = base.to_runtime();
    let system = ShieldSystem::new(&shield_config);
    // Ship-wide `[shields_console.base]` no longer carries `num_facings`
    // post-#514; the historical shield-config path defaults to 4 facings
    // via `ShieldsBaseConfig::default().num_facings`. Assert facings
    // reflect the shipped ship-wide HP/regen values.
    assert_eq!(system.facings.len(), shield_config.num_facings);
    for f in &system.facings {
        assert_eq!(f.max_hp, base.max_hp, "facing max_hp must match TOML");
        assert_eq!(f.hp, base.max_hp, "facing starts full");
        assert_eq!(f.regen_per_sec, base.regen_per_sec, "regen must match TOML");
        assert_eq!(
            f.offline_duration, base.offline_duration,
            "offline_duration must match TOML"
        );
    }
}

// ── from_arcs / variable-width arc tests (issue #514) ─────────────────────

fn ship_wide() -> ShieldConfig {
    ShieldConfig {
        num_facings: 4, // ignored by from_arcs
        max_hp: 100,
        regen_per_sec: 2.0,
        offline_duration: 10.0,
    }
}

#[test]
fn from_arcs_builds_one_facing_per_input() {
    let arcs = vec![
        ArcRuntimeConfig {
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 90.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
        },
        ArcRuntimeConfig {
            id: "port".into(),
            label: "Port".into(),
            center_deg: 270.0,
            width_deg: 90.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
        },
    ];
    let s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    assert_eq!(s.facings.len(), 2);
    assert_eq!(s.facings[0].id, "fore");
    assert_eq!(s.facings[0].label, "Fore");
    assert_eq!(s.facings[0].center_deg, 0.0);
    assert_eq!(s.facings[0].width_deg, 90.0);
    assert_eq!(s.facings[1].id, "port");
}

#[test]
fn from_arcs_per_arc_overrides_take_precedence() {
    let arcs = vec![ArcRuntimeConfig {
        id: "fore".into(),
        label: "Fore".into(),
        center_deg: 0.0,
        width_deg: 90.0,
        max_hp: Some(50),
        regen_per_sec: Some(0.5),
        offline_duration: Some(3.0),
        priority: 1,
    }];
    let s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    assert_eq!(s.facings[0].max_hp, 50);
    assert_eq!(s.facings[0].regen_per_sec, 0.5);
    assert_eq!(s.facings[0].offline_duration, 3.0);
}

/// Regression: `recalculate_focus` must derive each facing's effective
/// `max_hp` / `regen_per_sec` from **that facing's own** baseline (set at
/// construction from the per-arc override), not from the ship-wide
/// `self.base_max_hp` / `self.base_regen_per_sec`. Previously the focus
/// recalculation clobbered per-arc overrides with the ship-wide default
/// the moment focus changed, silently overwriting designer-authored
/// per-arc HP tuning.
#[test]
fn from_arcs_per_arc_overrides_preserved_across_focus_recalc() {
    // Two arcs with widely different per-arc max_hp / regen overrides.
    // Ship-wide default is 100 / 2.0 — deliberately different from both
    // arcs so any accidental fall-back to ship-wide values is visible.
    let arcs = vec![
        ArcRuntimeConfig {
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 180.0,
            max_hp: Some(200),
            regen_per_sec: Some(4.0),
            offline_duration: None,
            priority: 1,
        },
        ArcRuntimeConfig {
            id: "aft".into(),
            label: "Aft".into(),
            center_deg: 180.0,
            width_deg: 180.0,
            max_hp: Some(50),
            regen_per_sec: Some(1.0),
            offline_duration: None,
            priority: 1,
        },
    ];
    let mut s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    // Default focus config: bonus_max_hp=50, bonus_regen=5.0,
    //                       penalty_max_hp=25, penalty_regen=1.0
    let fc = s.focus_config.clone();

    // ── (1) Initial state — no focus, per-arc baselines apply.
    assert_eq!(s.facings[0].max_hp, 200, "fore max_hp = per-arc override");
    assert_eq!(s.facings[1].max_hp, 50, "aft max_hp = per-arc override");
    assert!((s.facings[0].regen_per_sec - 4.0).abs() < 1e-4);
    assert!((s.facings[1].regen_per_sec - 1.0).abs() < 1e-4);
    // Per-facing baselines survived construction.
    assert_eq!(s.facings[0].base_max_hp, 200);
    assert_eq!(s.facings[1].base_max_hp, 50);

    // ── (2) Focus fore, tick, verify fore=base+bonus, aft=base-penalty.
    s.set_focused_facing(Some(0));
    s.tick(0.1);
    assert_eq!(
        s.facings[0].max_hp,
        200 + fc.bonus_max_hp,
        "focused fore = own base + bonus (NOT ship-wide + bonus)"
    );
    assert_eq!(
        s.facings[1].max_hp,
        (50 - fc.penalty_max_hp).max(0),
        "non-focused aft = own base - penalty (NOT ship-wide - penalty)"
    );
    assert!((s.facings[0].regen_per_sec - (4.0 + fc.bonus_regen)).abs() < 1e-4);
    assert!((s.facings[1].regen_per_sec - (1.0 - fc.penalty_regen).max(0.0)).abs() < 1e-4);
    assert!(s.facings[0].is_focused);
    assert!(!s.facings[1].is_focused);

    // ── (3) Switch focus to aft, verify roles swap on per-arc baselines.
    s.set_focused_facing(Some(1));
    assert_eq!(
        s.facings[1].max_hp,
        50 + fc.bonus_max_hp,
        "focused aft = own base + bonus"
    );
    assert_eq!(
        s.facings[0].max_hp,
        (200 - fc.penalty_max_hp).max(0),
        "non-focused fore = own base - penalty (fore's 200 baseline preserved)"
    );
    assert!(!s.facings[0].is_focused);
    assert!(s.facings[1].is_focused);

    // ── (4) Clear focus, verify both facings restore to their own per-arc
    //       baselines exactly (fore→200, aft→50; NOT both→100 ship-wide).
    s.set_focused_facing(None);
    assert_eq!(s.facings[0].max_hp, 200, "fore restored to own baseline");
    assert_eq!(s.facings[1].max_hp, 50, "aft restored to own baseline");
    assert!((s.facings[0].regen_per_sec - 4.0).abs() < 1e-4);
    assert!((s.facings[1].regen_per_sec - 1.0).abs() < 1e-4);
    assert!(!s.facings[0].is_focused);
    assert!(!s.facings[1].is_focused);
}

#[test]
fn from_arcs_ship_wide_defaults_fill_in_when_arc_omits_field() {
    let arcs = vec![ArcRuntimeConfig {
        id: "all".into(),
        label: "All".into(),
        center_deg: 0.0,
        width_deg: 360.0,
        max_hp: None,
        regen_per_sec: None,
        offline_duration: None,
        priority: 1,
    }];
    let s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    assert_eq!(s.facings[0].max_hp, 100);
    assert_eq!(s.facings[0].regen_per_sec, 2.0);
    assert_eq!(s.facings[0].offline_duration, 10.0);
}

/// Verifies `facing_index_for_bearing` routes correctly for arcs of
/// non-uniform width. Uses a 3-arc layout: 180° fore, 90° port, 90°
/// starboard.
#[test]
fn shield_arc_facing_bearing_math_variable_widths() {
    use std::f32::consts::PI;
    let arcs = vec![
        // Wide fore arc — half the circle.
        ArcRuntimeConfig {
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 180.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
        },
        // Narrow port + starboard.
        ArcRuntimeConfig {
            id: "port".into(),
            label: "Port".into(),
            center_deg: 270.0,
            width_deg: 90.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
        },
        ArcRuntimeConfig {
            id: "starboard".into(),
            label: "Starboard".into(),
            center_deg: 90.0,
            width_deg: 90.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
        },
    ];
    let s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    // Forward (bearing 0) → Fore
    assert_eq!(s.facing_index_for_bearing(0.0), 0);
    // Slightly right of forward (bearing +π/4) → still inside 180° Fore arc.
    assert_eq!(s.facing_index_for_bearing(PI / 4.0), 0);
    // Directly starboard (+π/2 = 90°) → Starboard arc (center 90, half 45)
    // Bearing +π/2 = 90 is at the edge → routes to fore or starboard
    // depending on rounding. Test at 91° to be safely inside starboard.
    assert_eq!(s.facing_index_for_bearing(91.0f32.to_radians()), 2);
    // Directly aft (bearing π = 180°) → neither fore nor port nor
    // starboard: 180 is on the boundary of fore. Bearing 179 lands in
    // fore, bearing 181 wraps to -179 which is also fore. Fore's arc
    // wraps around aft when width=180 → covers -90..90 which
    // *excludes* 180. However 180 falls in fore's `delta.abs() <= 90`
    // test with delta = 180 → 180 wraps to -180 → |−180| = 180 > 90.
    // In that case the algorithm returns the legacy fallback.
    // Skip the exact-180 assertion — check nearby.
    // Bearing -π/2 (port) → Port arc
    assert_eq!(s.facing_index_for_bearing(-PI / 2.0), 1);
}

#[test]
fn from_arcs_snapshot_carries_id_and_geometry() {
    let arcs = vec![ArcRuntimeConfig {
        id: "custom".into(),
        label: "Custom".into(),
        center_deg: 45.0,
        width_deg: 60.0,
        max_hp: None,
        regen_per_sec: None,
        offline_duration: None,
        priority: 1,
    }];
    let s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    let snap = &s.snapshot()[0];
    assert_eq!(snap.id, "custom");
    assert_eq!(snap.label, "Custom");
    assert!((snap.center_deg - 45.0).abs() < 1e-6);
    assert!((snap.width_deg - 60.0).abs() < 1e-6);
}

// ── Priority routing ─────────────────────────────────────────────────────

/// Helper: build two overlapping arcs covering the full 360° — a "priority
/// shield" (priority 2, centre 0°, width 360°) and a "standard shield"
/// (priority 1, same geometry). When the priority shield is online, all
/// hits route to it. When it goes offline, hits fall through to the
/// lower-priority arc.
fn two_priority_arcs(priority_arc_offline: bool) -> ShieldSystem {
    let wide = ship_wide();
    let mut s = ShieldSystem::from_arcs(
        &[
            ArcRuntimeConfig {
                id: "hi".into(),
                label: "High".into(),
                center_deg: 0.0,
                width_deg: 360.0,
                max_hp: None,
                regen_per_sec: None,
                offline_duration: Some(999.0),
                priority: 2,
            },
            ArcRuntimeConfig {
                id: "lo".into(),
                label: "Low".into(),
                center_deg: 0.0,
                width_deg: 360.0,
                max_hp: None,
                regen_per_sec: None,
                offline_duration: Some(999.0),
                priority: 1,
            },
        ],
        &wide,
    );
    if priority_arc_offline {
        // Deplete the high-priority arc to put it offline.
        s.facings[0].apply_damage(9999);
    }
    s
}

#[test]
fn priority_arc_online_absorbs_hit_first() {
    let mut s = two_priority_arcs(false);
    let before_hi = s.facings[0].hp;
    let before_lo = s.facings[1].hp;
    s.apply_damage(10, 0.0);
    assert_eq!(
        s.facings[0].hp,
        before_hi - 10,
        "high-priority arc should absorb"
    );
    assert_eq!(
        s.facings[1].hp, before_lo,
        "low-priority arc should be untouched"
    );
}

#[test]
fn priority_arc_offline_falls_through_to_lower_priority() {
    let mut s = two_priority_arcs(true);
    assert!(
        !s.facings[0].is_online(),
        "sanity: high-priority arc is offline"
    );
    let before_lo = s.facings[1].hp;
    s.apply_damage(10, 0.0);
    assert_eq!(
        s.facings[1].hp,
        before_lo - 10,
        "low-priority arc should absorb when high is offline"
    );
}

#[test]
fn snapshot_carries_priority() {
    let wide = ship_wide();
    let s = ShieldSystem::from_arcs(
        &[ArcRuntimeConfig {
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 360.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 5,
        }],
        &wide,
    );
    let snap = &s.snapshot()[0];
    assert_eq!(snap.priority, 5);
}

// ── Damage multiplier (focus reduction/increase) ───────────────────────────

#[test]
fn default_damage_multiplier_is_one() {
    let s = ShieldSystem::default();
    for f in &s.facings {
        assert!(
            (f.damage_multiplier - 1.0).abs() < 1e-6,
            "default multiplier must be 1.0"
        );
    }
}

#[test]
fn focused_arc_gets_configured_damage_multiplier() {
    let mut s = ShieldSystem::default();
    s.focus_config.focused_damage_multiplier = 0.7;
    s.set_focused_facing(Some(0));
    assert!(
        (s.facings[0].damage_multiplier - 0.7).abs() < 1e-6,
        "focused arc should get damage reduction"
    );
}

#[test]
fn non_focused_arcs_get_unfocused_damage_multiplier() {
    let mut s = ShieldSystem::default();
    s.focus_config.unfocused_damage_multiplier = 1.25;
    s.set_focused_facing(Some(0));
    for i in 1..s.facings.len() {
        assert!(
            (s.facings[i].damage_multiplier - 1.25).abs() < 1e-6,
            "non-focused arc should get damage increase"
        );
    }
}

#[test]
fn clearing_focus_resets_damage_multiplier_to_one() {
    let mut s = ShieldSystem::default();
    s.focus_config.focused_damage_multiplier = 0.7;
    s.set_focused_facing(Some(0));
    s.set_focused_facing(None);
    for f in &s.facings {
        assert!(
            (f.damage_multiplier - 1.0).abs() < 1e-6,
            "clearing focus resets multiplier to 1.0"
        );
    }
}

#[test]
fn apply_damage_scales_by_damage_multiplier() {
    let mut s = ShieldSystem::default();
    // Focus arc 0 with 30% reduction.
    s.focus_config.focused_damage_multiplier = 0.7;
    s.set_focused_facing(Some(0));
    // Damage 100 → effective 70 on 100 HP shield → no leak.
    let leak = s.apply_damage(100, 0.0); // bearing 0 = fore
    assert_eq!(leak, 0, "all damage absorbed");
    assert_eq!(s.facings[0].hp, 30, "100 dmg * 0.7 = 70 taken");
}

#[test]
fn apply_damage_non_focused_gets_increase_multiplier() {
    let mut s = ShieldSystem::default();
    // Focus arc 0 (fore), so arc 1 (port) gets unfocused multiplier.
    s.focus_config.unfocused_damage_multiplier = 1.5;
    s.set_focused_facing(Some(0));
    // Damage 100 to port (bearing -90°) → effective 150 on 100 HP shield.
    let port_bearing = -std::f32::consts::FRAC_PI_2;
    let leak = s.apply_damage(100, port_bearing);
    // Shield takes 150 dmg, has 100 HP → overflow of 50.
    assert_eq!(leak, 50, "overflow passthrough = 150 - 100");
    assert_eq!(s.facings[1].hp, 0, "port facing depleted");
}

#[test]
fn apply_damage_multiplier_with_no_focus_is_unity() {
    let mut s = ShieldSystem::default();
    let leak = s.apply_damage(50, 0.0);
    assert_eq!(leak, 0);
    assert_eq!(
        s.facings[0].hp, 50,
        "no multiplier change: 50 dmg → 50 taken"
    );
}

#[test]
fn from_arcs_per_arc_overrides_preserve_damage_multiplier_across_focus() {
    let arcs = vec![
        ArcRuntimeConfig {
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 180.0,
            max_hp: Some(200),
            regen_per_sec: Some(4.0),
            offline_duration: None,
            priority: 1,
        },
        ArcRuntimeConfig {
            id: "aft".into(),
            label: "Aft".into(),
            center_deg: 180.0,
            width_deg: 180.0,
            max_hp: Some(50),
            regen_per_sec: Some(1.0),
            offline_duration: None,
            priority: 1,
        },
    ];
    let mut s = ShieldSystem::from_arcs(&arcs, &ship_wide());
    s.focus_config.focused_damage_multiplier = 0.5;
    s.focus_config.unfocused_damage_multiplier = 2.0;

    // No focus: all at 1.0.
    assert!((s.facings[0].damage_multiplier - 1.0).abs() < 1e-6);
    assert!((s.facings[1].damage_multiplier - 1.0).abs() < 1e-6);

    // Focus fore → fore gets 0.5, aft gets 2.0.
    s.set_focused_facing(Some(0));
    assert!((s.facings[0].damage_multiplier - 0.5).abs() < 1e-6);
    assert!((s.facings[1].damage_multiplier - 2.0).abs() < 1e-6);

    // Switch focus to aft → aft gets 0.5, fore gets 2.0.
    s.set_focused_facing(Some(1));
    assert!((s.facings[0].damage_multiplier - 2.0).abs() < 1e-6);
    assert!((s.facings[1].damage_multiplier - 0.5).abs() < 1e-6);

    // Clear focus → all back to 1.0.
    s.set_focused_facing(None);
    assert!((s.facings[0].damage_multiplier - 1.0).abs() < 1e-6);
    assert!((s.facings[1].damage_multiplier - 1.0).abs() < 1e-6);
}
