use super::*;
use crate::ship::shields::DamageRecord;
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
fn default_focus_policy_params_equal_the_typed_knobs() {
    // Baseline preservation: the canonical default policy seeds its four
    // `param`s from the retained typed `default_shields_ai_*()` values, so a
    // ship that omits `[shields_console.ai_policy]` feeds the kernel exactly
    // the windows/thresholds it always did.
    let policy = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus")
        .to_policy()
        .unwrap();
    let typed = crate::ship::shields::ShieldsAiConfigResource::default();
    assert_eq!(
        policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_DAMAGE_WINDOW_PARAM),
        Some(typed.damage_window_secs as f64)
    );
    assert_eq!(
        policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM),
        Some(typed.min_damage_window_secs as f64)
    );
    assert_eq!(
        policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_DAMAGE_PCT_PARAM),
        Some(typed.damage_pct_threshold as f64)
    );
    assert_eq!(
        policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM),
        Some(typed.health_ratio_threshold as f64)
    );
}

#[test]
fn params_sourced_windows_produce_the_same_kernel_decision_as_typed_knobs() {
    // The kernel is unchanged: feeding it windows/thresholds read from the
    // default policy `param` map yields the identical decision to feeding the
    // typed defaults directly, over a concentrated-damage scenario.
    let policy = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus")
        .to_policy()
        .unwrap();
    let p = |name: &str| policy.params.get(name).unwrap() as f32;

    let mut history = empty_history(4);
    history[1].push(DamageRecord {
        recorded_tick: 1,
        amount: 80,
    });
    history[0].push(DamageRecord {
        recorded_tick: 1,
        amount: 10,
    });
    let facings = vec![
        make_snap("Fore", 90, 100, false),
        make_snap("Port", 90, 100, false),
        make_snap("Aft", 90, 100, false),
        make_snap("Starboard", 90, 100, false),
    ];

    let params_input = ShieldFocusAiInput {
        facings: facings.clone(),
        shields_is_low: true,
        damage_history: history.clone(),
        damage_window_ticks: p(crate::entities::config::SHIELD_FOCUS_DAMAGE_WINDOW_PARAM).ceil()
            as u64,
        min_damage_window_ticks: p(crate::entities::config::SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM)
            .ceil() as u64,
        damage_pct_threshold: p(crate::entities::config::SHIELD_FOCUS_DAMAGE_PCT_PARAM),
        health_ratio_threshold: p(crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM),
        current_tick: 4,
    };
    let typed_input = make_input(facings, true, history, 4);

    assert_eq!(
        tick_shield_focus_ai(&params_input),
        tick_shield_focus_ai(&typed_input)
    );
    assert_eq!(
        tick_shield_focus_ai(&params_input),
        ShieldFocusAiOutput::Focus { facing_index: 1 }
    );
}
