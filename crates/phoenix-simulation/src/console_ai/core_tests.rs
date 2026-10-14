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
fn shipped_shield_tuning() -> crate::entities::config::ShieldsAiConfigToml {
    crate::entities::include_resolve::load_entity_config("assets/entities/alliance_cruiser.toml")
        .unwrap()
        .shields_console
        .unwrap()
        .ai
        .unwrap()
}

#[test]
fn shipped_observation_tuning_matches_the_typed_defaults() {
    let authored = shipped_shield_tuning();
    let typed = crate::ship::shields::ShieldsAiConfigResource::default();
    assert_eq!(authored.damage_window_secs, typed.damage_window_secs);
    assert_eq!(
        authored.min_damage_window_secs,
        typed.min_damage_window_secs
    );
    assert_eq!(authored.damage_pct_threshold, typed.damage_pct_threshold);
    assert_eq!(
        authored.health_ratio_threshold,
        typed.health_ratio_threshold
    );
}

#[test]
fn authored_observation_windows_preserve_the_concentrated_damage_ranking() {
    let authored = shipped_shield_tuning();

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
        damage_window_ticks: authored.damage_window_secs.ceil() as u64,
        min_damage_window_ticks: authored.min_damage_window_secs.ceil() as u64,
        damage_pct_threshold: authored.damage_pct_threshold,
        health_ratio_threshold: authored.health_ratio_threshold,
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
