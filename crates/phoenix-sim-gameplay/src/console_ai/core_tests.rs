use super::*;

// ── FrequencyHintState helpers ────────────────────────────────────────

fn hint_input(target: Option<&str>, frequency: f32, dt: f32, delay: f32) -> FrequencyHintInput {
    FrequencyHintInput {
        locked_target: target.map(str::to_owned),
        correct_frequency: frequency,
        dt,
        delay_secs: delay,
    }
}

// ── tick_frequency_hint ───────────────────────────────────────────────

#[test]
fn no_target_returns_none_and_resets_state() {
    let mut state = FrequencyHintState::default();
    let out = tick_frequency_hint(&mut state, &hint_input(None, 0.5, 1.0, 3.0));
    assert_eq!(out, FrequencyHintOutput::None);
    assert!(state.current_target.is_none());
}

#[test]
fn under_delay_returns_none() {
    let mut state = FrequencyHintState::default();
    // 1s tick with 3s delay → no hint
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 1.0, 3.0));
    assert_eq!(out, FrequencyHintOutput::None);
    assert!(!state.hint_sent);
}

#[test]
fn at_delay_fires_hint_with_correct_frequency() {
    let mut state = FrequencyHintState::default();
    // 3s tick with 3s delay → fires
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.75, 3.0, 3.0));
    assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.75 });
    assert!(state.hint_sent);
}

#[test]
fn above_delay_fires_hint() {
    let mut state = FrequencyHintState::default();
    // Two ticks: 2s + 2s > 3s → fires on second tick
    tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.0, 3.0));
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.0, 3.0));
    assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.5 });
}

#[test]
fn hint_fires_only_once_per_target_lock() {
    let mut state = FrequencyHintState::default();
    // Fire hint
    tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
    // Second tick same target → no additional hint
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
    assert_eq!(out, FrequencyHintOutput::None);
}

#[test]
fn target_change_resets_timer() {
    let mut state = FrequencyHintState::default();
    // Almost at delay with t1
    tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.9, 3.0));
    assert!(!state.hint_sent);
    // Switch to t2 → timer resets
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.5, 2.9, 3.0));
    assert_eq!(out, FrequencyHintOutput::None);
    assert_eq!(state.current_target.as_deref(), Some("t2"));
    // Only 2.9s elapsed for t2 → still no hint
    assert!(!state.hint_sent);
}

#[test]
fn target_change_then_delay_fires_hint_for_new_target() {
    let mut state = FrequencyHintState::default();
    // Nearly full for t1
    tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 2.9, 3.0));
    // Switch targets → timer resets, accumulate enough for t2
    tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.0, 3.0));
    tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.0, 3.0));
    let out = tick_frequency_hint(&mut state, &hint_input(Some("t2"), 0.9, 1.5, 3.0));
    // elapsed = 3.5 >= 3.0 → hint
    assert_eq!(out, FrequencyHintOutput::Hint { frequency: 0.9 });
}

#[test]
fn clearing_target_resets_hint_sent_flag() {
    let mut state = FrequencyHintState::default();
    // Fire hint for t1
    tick_frequency_hint(&mut state, &hint_input(Some("t1"), 0.5, 5.0, 3.0));
    assert!(state.hint_sent);
    // Clear target → reset
    tick_frequency_hint(&mut state, &hint_input(None, 0.5, 1.0, 3.0));
    assert!(!state.hint_sent);
    assert!(state.current_target.is_none());
}

// ── tick_auto_match_frequency ─────────────────────────────────────────

fn match_input(
    target: Option<&str>,
    frequency: f32,
    dt: f32,
    delay: f32,
    trigger_active: bool,
) -> FrequencyMatchInput {
    FrequencyMatchInput {
        locked_target: target.map(str::to_owned),
        target_frequency: frequency,
        dt,
        delay_secs: delay,
        trigger_active,
    }
}

#[test]
fn auto_match_trigger_inactive_returns_none_and_resets() {
    let mut state = FrequencyMatchState {
        current_target: Some("t1".into()),
        elapsed_secs: 10.0,
        match_sent: false,
    };
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
    assert_eq!(out, FrequencyMatchOutput::None);
    assert!(state.current_target.is_none());
    assert_eq!(state.elapsed_secs, 0.0);
}

#[test]
fn auto_match_no_target_returns_none_and_resets() {
    let mut state = FrequencyMatchState::default();
    let out = tick_auto_match_frequency(&mut state, &match_input(None, 0.5, 1.0, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::None);
    assert!(state.current_target.is_none());
}

#[test]
fn auto_match_under_delay_returns_none() {
    let mut state = FrequencyMatchState::default();
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::None);
    assert!(!state.match_sent);
}

#[test]
fn auto_match_at_delay_fires_with_correct_frequency() {
    let mut state = FrequencyMatchState::default();
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.75, 3.0, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.75 });
    assert!(state.match_sent);
}

#[test]
fn auto_match_above_delay_fires() {
    let mut state = FrequencyMatchState::default();
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.5 });
}

#[test]
fn auto_match_fires_only_once_per_target() {
    let mut state = FrequencyMatchState::default();
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::None);
}

#[test]
fn auto_match_target_change_resets_timer() {
    let mut state = FrequencyMatchState::default();
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.5, 2.9, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::None);
    assert_eq!(state.current_target.as_deref(), Some("t2"));
    assert!(!state.match_sent);
}

#[test]
fn auto_match_target_change_then_delay_fires_for_new_target() {
    let mut state = FrequencyMatchState::default();
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
    // Switch target — timer resets to 0, accumulate enough for t2
    tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.0, 3.0, true));
    tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.0, 3.0, true));
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.5, 3.0, true));
    assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.9 });
}

#[test]
fn auto_match_trigger_flip_to_inactive_resets_state() {
    let mut state = FrequencyMatchState::default();
    // Nearly at delay
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
    // Trigger turns off (e.g. either console goes Full)
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
    assert_eq!(out, FrequencyMatchOutput::None);
    assert!(
        state.current_target.is_none(),
        "state must reset when trigger deactivates"
    );
    assert_eq!(state.elapsed_secs, 0.0);
}

#[test]
fn auto_match_no_auto_revert_after_match_sent_trigger_ends() {
    let mut state = FrequencyMatchState::default();
    // Fire the match
    tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
    assert!(state.match_sent);
    // Trigger ends — state resets but we are NOT emitting a revert
    let out = tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
    // Must NOT emit a Match (which would be a revert) and must just return None
    assert_eq!(
        out,
        FrequencyMatchOutput::None,
        "frequency must persist at last value — no auto-revert when trigger ends"
    );
}

fn tube(id: &str, loaded: bool, in_arc: bool) -> TubeSummary {
    TubeSummary {
        id: id.to_string(),
        loaded,
        in_arc,
    }
}

fn all_ready_input() -> TorpedoAiInput {
    TorpedoAiInput {
        target_locked: true,
        target_facing_shields: 0,
        tubes: vec![
            tube("fore_port", true, true),
            tube("fore_starboard", true, true),
            tube("aft", true, true),
        ],
        magazine: 10,
    }
}

// ── NON-condition: facing-arc shields (issue #956) ─────────────────────

/// **The gate that left this function.**
///
/// A healthy striking arc, a collapsed one, an overkilled one and an
/// unshielded target all yield the SAME candidate list here, because
/// `auto_fire_torpedo` no longer asks: "phasers strip the shields,
/// torpedoes finish the hull" is now the tube's own authored
/// `torpedo_launch` guard, resolved by `ai_torpedo_auto_fire` over a
/// snapshot seeded from this very number.
///
/// This is a NON-condition assertion in the same spirit as
/// [`an_empty_magazine_does_not_stop_a_loaded_tube_firing`] below, and it
/// is deliberately anti-vacuous: every row is compared against the same
/// non-empty baseline, so a function that had quietly stopped returning
/// candidates for some other reason cannot pass.
///
/// The gate itself did not weaken, and the proof moved WITH it, onto the
/// shipped content it now lives in:
///
/// * `entities::authored_ai_pins::torpedo_launch_shield_gate_truth_table`
///   resolves the fleet-baseline tube policy over a healthy arc and a
///   downed one and shows it holding and firing respectively. That is the
///   whole of what a POLICY can be shown to do here: it reads one
///   already-resolved scalar, so "which arc" is not a question it can be
///   asked.
/// * `console::weapons::server_tests::
///   ai_torpedo_auto_fire_holds_fire_while_target_shields_are_up` drives the
///   whole host end to end against a real shielded target, and its sibling
///   `ai_torpedo_auto_fire_gates_on_the_arc_the_torpedo_would_strike` is
///   where the PER-ARC half is proved — a healthy rear arc holding the shot
///   while the front one is collapsed, and the reverse — because only the
///   host resolves the arc the round would meet.
#[test]
fn the_striking_arcs_shields_are_no_longer_a_condition_here() {
    let baseline = auto_fire_torpedo(&all_ready_input());
    assert!(
        !baseline.is_empty(),
        "precondition: the fixture must produce candidates at all"
    );
    for hp in [50, 120, 0, -5] {
        let mut input = all_ready_input();
        input.target_facing_shields = hp;
        assert_eq!(
            auto_fire_torpedo(&input),
            baseline,
            "target_facing_shields = {hp}: the pure candidate filter reports \
                 loaded, in-arc tubes whatever the striking arc is doing. The \
                 shields-down decision belongs to the tube's authored \
                 `torpedo_launch` predicate (issue #956), not to this function — \
                 a Rust gate here sits upstream of every policy and can only ever \
                 be narrowed by one, never opened or retuned."
        );
    }
}

// ── Condition: target lock ─────────────────────────────────────────────

#[test]
fn no_target_lock_no_fire() {
    let mut input = all_ready_input();
    input.target_locked = false;
    let result = auto_fire_torpedo(&input);
    assert!(
        result.is_empty(),
        "should not fire when no target is locked"
    );
}

// ── Non-condition: the magazine ────────────────────────────────────────

/// An empty magazine does NOT stop a loaded tube firing, and it must not:
/// the rounds in the tubes were drawn from the magazine when their load
/// started, so "nothing left to reload with" and "nothing left to shoot" are
/// different states and only the second should hold fire.
///
/// This test used to assert the opposite. The gate it asserted made a hull
/// whose magazine divides evenly into its battery permanently unable to fire
/// its last, fully-loaded salvo — the Harrow cruiser (8 rounds, 4-round
/// battery) reaches that state on its second reload every single run, and
/// then held a bow-on torpedo phase open around a battery it refused to
/// shoot.
///
/// The comparison against a stocked magazine is the anti-vacuity half: the
/// two calls differ in the magazine and nothing else, so an implementation
/// that had quietly stopped firing for some other reason cannot pass.
#[test]
fn an_empty_magazine_does_not_stop_a_loaded_tube_firing() {
    let stocked = auto_fire_torpedo(&all_ready_input());
    assert!(
        !stocked.is_empty(),
        "precondition: the fixture must fire at all"
    );

    let mut input = all_ready_input();
    input.magazine = 0;
    assert_eq!(
        auto_fire_torpedo(&input),
        stocked,
        "rounds already in the tubes are already paid for — an empty magazine \
             stops the next LOAD, not the shot that is standing ready"
    );
}

/// ...and the magazine is not a back door either: emptying it while the
/// tubes are empty still fires nothing, because the tubes are what is asked
/// about.
#[test]
fn an_empty_magazine_with_empty_tubes_still_fires_nothing() {
    let mut input = all_ready_input();
    input.magazine = 0;
    for t in &mut input.tubes {
        t.loaded = false;
    }
    assert!(
        auto_fire_torpedo(&input).is_empty(),
        "no rounds anywhere is still no shot"
    );
}

// ── Condition: loaded ──────────────────────────────────────────────────

#[test]
fn unloaded_tube_not_in_result() {
    let mut input = all_ready_input();
    // Only fore-port is unloaded
    input.tubes[0].loaded = false;
    let result = auto_fire_torpedo(&input);
    assert!(
        !result.contains(&"fore_port".to_string()),
        "unloaded ForePort should not appear in result"
    );
}

#[test]
fn all_tubes_unloaded_no_fire() {
    let mut input = all_ready_input();
    for t in &mut input.tubes {
        t.loaded = false;
    }
    let result = auto_fire_torpedo(&input);
    assert!(result.is_empty(), "should not fire when all tubes unloaded");
}

// ── Condition: arc ─────────────────────────────────────────────────────

#[test]
fn tube_not_in_arc_not_in_result() {
    let mut input = all_ready_input();
    // Only fore-starboard is out of arc
    input.tubes[1].in_arc = false;
    let result = auto_fire_torpedo(&input);
    assert!(
        !result.contains(&"fore_starboard".to_string()),
        "out-of-arc ForeStarboard should not appear in result"
    );
}

#[test]
fn no_tube_in_arc_no_fire() {
    let mut input = all_ready_input();
    for t in &mut input.tubes {
        t.in_arc = false;
    }
    let result = auto_fire_torpedo(&input);
    assert!(result.is_empty(), "should not fire when no tube in arc");
}

// ── Tube priority ──────────────────────────────────────────────────────

#[test]
fn priority_order_is_fore_port_then_fore_starboard_then_aft() {
    let result = auto_fire_torpedo(&all_ready_input());
    assert_eq!(
        result,
        vec![
            "fore_port".to_string(),
            "fore_starboard".to_string(),
            "aft".to_string()
        ],
        "tubes must appear in deterministic priority order"
    );
}

#[test]
fn only_aft_ready_returns_just_aft() {
    let mut input = all_ready_input();
    input.tubes[0].loaded = false; // ForePort unloaded
    input.tubes[1].in_arc = false; // ForeStarboard out of arc
    let result = auto_fire_torpedo(&input);
    assert_eq!(result, vec!["aft".to_string()]);
}

#[test]
fn fore_port_and_aft_ready_returns_in_order() {
    let mut input = all_ready_input();
    input.tubes[1].loaded = false; // ForeStarboard unloaded
    let result = auto_fire_torpedo(&input);
    assert_eq!(result, vec!["fore_port".to_string(), "aft".to_string()]);
}

// ── Shields AI ────────────────────────────────────────────────────────

use crate::weapons::shield::ShieldFacingSnapshot;

fn make_snap(label: &str, hp: i32, max_hp: i32, focused: bool) -> ShieldFacingSnapshot {
    ShieldFacingSnapshot {
        id: label.to_ascii_lowercase(),
        label: label.into(),
        hp,
        max_hp,
        online: hp > 0,
        offline_remaining: 0.0,
        is_focused: focused,
        center_deg: 0.0,
        width_deg: 90.0,
        priority: 1,
    }
}

fn empty_history(len: usize) -> Vec<Vec<DamageRecord>> {
    vec![Vec::new(); len]
}

fn make_input(
    facings: Vec<ShieldFacingSnapshot>,
    shields_is_low: bool,
    damage_history: Vec<Vec<DamageRecord>>,
    current_tick: u64,
) -> ShieldFocusAiInput {
    ShieldFocusAiInput {
        facings,
        shields_is_low,
        damage_history,
        damage_window_ticks: 4,
        min_damage_window_ticks: 1,
        damage_pct_threshold: 50.0,
        health_ratio_threshold: 50.0,
        current_tick,
    }
}

#[test]
fn shield_ai_not_low_returns_none() {
    let input = make_input(
        vec![make_snap("Fore", 50, 100, false)],
        false,
        empty_history(1),
        0,
    );
    assert_eq!(tick_shield_focus_ai(&input), ShieldFocusAiOutput::None);
}

#[test]
fn shield_ai_single_arc_returns_none() {
    let input = make_input(
        vec![make_snap("All", 50, 100, false)],
        true,
        empty_history(1),
        0,
    );
    assert_eq!(tick_shield_focus_ai(&input), ShieldFocusAiOutput::None);
}

#[test]
fn shield_ai_damage_concentration_focuses_arc() {
    // Arc at index 1 (Port) takes 80% of damage over the AUTHORED window
    // → should be focused even though every arc's health is equal.
    // Concentration is measured over records whose age is strictly less
    // than max(damage_window_ticks, min_damage_window_ticks) = 4. The
    // damage sits at tick 1 when current_tick=4 (age 3) — inside the
    // authored window but OUTSIDE the old last-minimum-window slice. With balanced health
    // (all arcs 90/100) the health-imbalance fallback cannot fire, so a
    // Focus here proves the authored window governs concentration.
    let mut history = empty_history(4);
    // Damage to Port at tick 1 (inside the authored four-tick window).
    history[1].push(DamageRecord {
        recorded_tick: 1,
        amount: 80,
    });
    // Scattered damage to other arcs at the same time.
    history[0].push(DamageRecord {
        recorded_tick: 1,
        amount: 10,
    });
    history[2].push(DamageRecord {
        recorded_tick: 1,
        amount: 10,
    });
    let facings = vec![
        make_snap("Fore", 90, 100, false),
        make_snap("Port", 90, 100, false),
        make_snap("Aft", 90, 100, false),
        make_snap("Starboard", 90, 100, false),
    ];
    let input = make_input(facings, true, history, 4);
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::Focus { facing_index: 1 }
    );
}

#[test]
fn shield_ai_damage_below_threshold_does_not_focus() {
    // Damage is spread evenly, no arc reaches 50% threshold.
    let mut history = empty_history(4);
    // Each arc gets 25 damage → no arc has ≥ 50% of total (100)
    for arc in &mut history {
        arc.push(DamageRecord {
            recorded_tick: 3,
            amount: 25,
        });
    }
    let facings = vec![
        make_snap("Fore", 75, 100, false),
        make_snap("Port", 75, 100, false),
        make_snap("Aft", 75, 100, false),
        make_snap("Starboard", 75, 100, false),
    ];
    let input = make_input(facings, true, history, 4);
    // Falls through to health check: worst normalized = 0.75,
    // second_worst = 0.75, ratio = 1.0, not < 0.5 → ClearFocus
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::ClearFocus
    );
}

#[test]
fn shield_ai_health_imbalance_focuses_weakest() {
    // No damage in window. Port (idx 1) at 30/100 = 0.3, others at 0.8+.
    // health_ratio_threshold=50%, so need lowest < 0.5 * second_lowest.
    // worst=0.3, second=0.8, 0.5*0.8=0.4, 0.3<0.4 → focus idx 1.
    let facings = vec![
        make_snap("Fore", 80, 100, false),
        make_snap("Port", 30, 100, false),
        make_snap("Aft", 80, 100, false),
        make_snap("Starboard", 80, 100, false),
    ];
    let input = make_input(facings, true, empty_history(4), 5);
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::Focus { facing_index: 1 }
    );
}

#[test]
fn shield_ai_health_imbalance_no_op_if_already_focused() {
    // Same health imbalance but the worst arc is already focused.
    let facings = vec![
        make_snap("Fore", 80, 100, false),
        make_snap("Port", 30, 100, true),
        make_snap("Aft", 80, 100, false),
        make_snap("Starboard", 80, 100, false),
    ];
    let input = make_input(facings, true, empty_history(4), 5);
    assert_eq!(tick_shield_focus_ai(&input), ShieldFocusAiOutput::None);
}

#[test]
fn shield_ai_no_damage_and_balanced_health_clears() {
    // All arcs at full HP, no damage → clear focus.
    let facings = vec![
        make_snap("Fore", 100, 100, false),
        make_snap("Port", 100, 100, false),
        make_snap("Aft", 100, 100, false),
        make_snap("Starboard", 100, 100, false),
    ];
    let input = make_input(facings, true, empty_history(4), 5);
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::ClearFocus
    );
}

#[test]
fn shield_ai_damage_outside_active_window_ignored() {
    // Damage on Port (idx 1) at tick 2 is exactly the authored window old
    // when current_tick=6: window = max(4, 1) = 4, so strict `age < window`
    // excludes age 4 and the decision must fall through to health.
    // Port is kept healthy (90/100) and Aft (idx 2) is the weak arc, so if
    // the expired hit were (wrongly) counted the result would be Focus{1};
    // because it is ignored, health imbalance focuses Aft instead.
    let mut history = empty_history(4);
    history[1].push(DamageRecord {
        recorded_tick: 2, // exact end boundary: age 4 is expired
        amount: 80,
    });
    let facings = vec![
        make_snap("Fore", 90, 100, false),
        make_snap("Port", 90, 100, false),
        make_snap("Aft", 20, 100, false),
        make_snap("Starboard", 90, 100, false),
    ];
    let input = make_input(facings, true, history, 6);
    // No damage in active window → health check.
    // worst normalized = 0.2 (Aft), second = 0.9, 0.5*0.9=0.45, 0.2<0.45 → focus Aft
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::Focus { facing_index: 2 }
    );
}

#[test]
fn shield_ai_damage_in_future_window_ignored() {
    // Damage on Port (idx 1) at tick 5 is in the future relative to
    // current_tick=4 and must be ignored.
    // Port is kept healthy and Aft (idx 2) is the weak arc, so the ignored
    // future hit cannot mask the health-imbalance fallback focusing Aft.
    let mut history = empty_history(4);
    history[1].push(DamageRecord {
        recorded_tick: 5,
        amount: 80,
    });
    let facings = vec![
        make_snap("Fore", 90, 100, false),
        make_snap("Port", 90, 100, false),
        make_snap("Aft", 20, 100, false),
        make_snap("Starboard", 90, 100, false),
    ];
    let input = make_input(facings, true, history, 4);
    // Future damage ignored → health check: worst=0.2 (Aft), second=0.9 → focus Aft
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::Focus { facing_index: 2 }
    );
}

#[test]
fn shield_ai_concentration_window_floored_at_min_damage_window() {
    // A misconfigured one-tick authored window below the two-tick reaction
    // minimum must NOT shrink the concentration window below the floor.
    // Port's tick-3 hit has age 1 at current_tick=4, so the two-tick floor
    // includes it. Without the floor, strict age < 1 would exclude it and,
    // with balanced health, the decision would clear instead.
    let mut history = empty_history(4);
    history[1].push(DamageRecord {
        recorded_tick: 3,
        amount: 80,
    });
    history[0].push(DamageRecord {
        recorded_tick: 3,
        amount: 10,
    });
    history[2].push(DamageRecord {
        recorded_tick: 3,
        amount: 10,
    });
    let facings = vec![
        make_snap("Fore", 90, 100, false),
        make_snap("Port", 90, 100, false),
        make_snap("Aft", 90, 100, false),
        make_snap("Starboard", 90, 100, false),
    ];
    let input = ShieldFocusAiInput {
        facings,
        shields_is_low: true,
        damage_history: history,
        damage_window_ticks: 1,
        min_damage_window_ticks: 2,
        damage_pct_threshold: 50.0,
        health_ratio_threshold: 50.0,
        current_tick: 4,
    };
    assert_eq!(
        tick_shield_focus_ai(&input),
        ShieldFocusAiOutput::Focus { facing_index: 1 }
    );
}

// ── #783 policy-param equivalence + seeded facts ─────────────────────────

#[test]
fn seed_shields_focus_facts_exposes_bounded_per_arc_damage() {
    // AC1: per-arc recent-damage facts computed from the pruned window only.
    // Port (idx 1) took a concentrated hit inside the window; a stale hit on
    // Fore at tick 1 has age 4 at current_tick=5 and must NOT count: the
    // end boundary is strict (bounded — no unbounded accumulation).
    let mut history = empty_history(4);
    history[1].push(DamageRecord {
        recorded_tick: 3,
        amount: 60,
    });
    history[0].push(DamageRecord {
        recorded_tick: 3,
        amount: 20,
    });
    history[0].push(DamageRecord {
        recorded_tick: 1, // exact end boundary: age 4 is expired
        amount: 999,
    });
    let facings = vec![
        make_snap("Fore", 80, 100, false),
        make_snap("Port", 40, 100, false),
        make_snap("Aft", 100, 100, false),
        make_snap("Starboard", 100, 100, false),
    ];
    // window = max(4, 1) = 4 ticks; only ages 0 through 3 are recent.
    let facts = seed_shields_focus_facts(&facings, &history, 4, 1, 5);

    assert_eq!(facts.get("recent_damage_port"), Some(60.0));
    assert_eq!(
        facts.get("recent_damage_fore"),
        Some(20.0),
        "the age-equals-window hit on Fore must be excluded from the bounded window"
    );
    assert_eq!(facts.get("recent_damage_aft"), Some(0.0));
    assert_eq!(facts.get("recent_damage_total"), Some(80.0));
    // Concentration: Port 60 of 80 = 75%.
    assert_eq!(facts.get("recent_damage_pct_max"), Some(75.0));
    assert_eq!(facts.get("recent_damage_fraction_max"), Some(0.75));
    // Health-imbalance fallback signal: lowest 0.4 (Port) / second 0.8 (Fore).
    let ratio = facts.get("health_fraction_min_ratio").unwrap();
    assert!(
        (ratio - 0.5).abs() < 1e-6,
        "min/second health ratio should be 0.5"
    );
}
