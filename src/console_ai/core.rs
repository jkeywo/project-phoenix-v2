//! Pure AI decision functions for console automation.
//!
//! This module is Bevy-free and platform-agnostic. It contains decision
//! functions that replicate the actions a human player would take on a
//! console set to "Low" complexity.
//!
//! The Bevy orchestrator lives in `console_ai_plugin`.

use crate::shield::ShieldFacingSnapshot;
use crate::ship::shields::DamageRecord;
use crate::torpedo::TorpedoTubeId;

// ── Frequency hint state ───────────────────────────────────────────────────

/// Persistent timer state for the frequency-hint AI.
///
/// The hint fires once per target lock after `delay_secs` seconds.
/// Reset whenever the locked target changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrequencyHintState {
    /// UUID of the target for which the timer is currently running.
    /// `None` means no timer is running.
    pub current_target: Option<String>,
    /// Accumulated elapsed time while the current target has been locked, in
    /// seconds. Resets to 0.0 on target change.
    pub elapsed_secs: f32,
    /// Set to `true` once the hint has fired for the current target. Prevents
    /// repeated hints on the same lock.
    pub hint_sent: bool,
}

/// All inputs required by `tick_frequency_hint`.
#[derive(Clone, Debug)]
pub struct FrequencyHintInput {
    /// UUID of the currently locked target. `None` = no lock.
    pub locked_target: Option<String>,
    /// The recommended phaser frequency to hint (0.0–1.0).
    pub correct_frequency: f32,
    /// Seconds elapsed this frame (delta time).
    pub dt: f32,
    /// Configured delay before the hint fires (from TOML / server config).
    pub delay_secs: f32,
}

/// Outcome of a single `tick_frequency_hint` call.
#[derive(Clone, Debug, PartialEq)]
pub enum FrequencyHintOutput {
    /// No action this tick.
    None,
    /// Emit a `FrequencyHint` with this frequency to the Tactical console.
    Hint { frequency: f32 },
}

/// Advance the frequency-hint timer by one tick.
///
/// Rules:
/// - If there is no locked target, reset the timer and return `None`.
/// - If the locked target changes, reset the timer to 0 and return `None`.
/// - If the timer has not yet reached `delay_secs`, accumulate `dt` and
///   return `None`.
/// - At the first tick where `elapsed_secs >= delay_secs` (and the hint has
///   not already been sent for this target), emit `Hint { frequency }` and
///   mark the hint as sent.
/// - On subsequent ticks with the same target (hint already sent), return
///   `None`.
pub fn tick_frequency_hint(
    state: &mut FrequencyHintState,
    input: &FrequencyHintInput,
) -> FrequencyHintOutput {
    match &input.locked_target {
        None => {
            // No target — clear state.
            *state = FrequencyHintState::default();
            FrequencyHintOutput::None
        }
        Some(uuid) => {
            // Target changed → reset timer.
            if state.current_target.as_deref() != Some(uuid.as_str()) {
                *state = FrequencyHintState {
                    current_target: Some(uuid.clone()),
                    elapsed_secs: 0.0,
                    hint_sent: false,
                };
            }

            if state.hint_sent {
                return FrequencyHintOutput::None;
            }

            state.elapsed_secs += input.dt;
            if state.elapsed_secs >= input.delay_secs {
                state.hint_sent = true;
                FrequencyHintOutput::Hint {
                    frequency: input.correct_frequency,
                }
            } else {
                FrequencyHintOutput::None
            }
        }
    }
}

// ── Input types ────────────────────────────────────────────────────────────

/// State of a single torpedo tube as seen by the AI.
#[derive(Clone, Debug, PartialEq)]
pub struct TubeSummary {
    pub id: TorpedoTubeId,
    /// Tube is loaded and ready to fire.
    pub loaded: bool,
    /// Target bearing (radians from ship forward) is within this tube's arc.
    pub in_arc: bool,
}

/// All inputs required by `auto_fire_torpedo`.
#[derive(Clone, Debug)]
pub struct TorpedoAiInput {
    /// Whether the player has locked a target.
    pub target_locked: bool,
    /// HP of the **one** shield arc a torpedo arriving from this ship would
    /// strike — not the target's shield pool. Fire only when this is ≤ 0.
    ///
    /// The caller resolves the arc from the attack bearing (see
    /// `ShieldSystem::facing_index_for_bearing`) and reports 0 when that arc
    /// is offline (an offline arc passes damage through to the hull, so it is
    /// not blocking the shot) or when the target has no shield arcs at all
    /// (asteroids, debris — always torpedo-eligible).
    pub target_facing_shields: i32,
    /// Tubes considered by the AI, in priority order.
    pub tubes: Vec<TubeSummary>,
    /// Torpedoes remaining in the magazine.
    pub magazine: u32,
}

// ── Decision function ──────────────────────────────────────────────────────

/// Decide which torpedo tubes to fire this tick.
///
/// Auto-fire conditions (ALL must hold):
/// - A target is locked
/// - The shield arc the torpedo would *strike* is down (`target_facing_shields ≤ 0`)
/// - The tube is loaded
/// - The tube is in arc
/// - The magazine has torpedoes remaining
///
/// # Why the shield gate is per-arc, not ship-wide
///
/// The doctrine is "phasers strip the shields, torpedoes finish the hull", and
/// on a single-arc hull those are the same test. On a four-arc hull they are
/// not: summing every arc lets three healthy REAR arcs veto a shot into a
/// collapsed FRONT arc while the attacker is sitting dead ahead — exactly where
/// the hull is exposed and where the torpedo would land. Since every arc regens
/// independently and goes offline for only a few seconds, a multi-arc ship
/// essentially never has all arcs down at once, so the summed gate meant AI
/// crews on four-arc hulls never launched a torpedo at all. The gate therefore
/// asks about the *one* arc the shot would hit.
///
/// Returns a list of `TorpedoTubeId` values to fire in deterministic priority
/// order `[ForePort, ForeStarboard, Aft]`.  Each tube that passes all
/// conditions appears at most once in the result.
pub fn auto_fire_torpedo(input: &TorpedoAiInput) -> Vec<TorpedoTubeId> {
    if !input.target_locked || input.target_facing_shields > 0 || input.magazine == 0 {
        return vec![];
    }
    input
        .tubes
        .iter()
        .filter(|t| t.loaded && t.in_arc)
        .map(|t| t.id.clone())
        .collect()
}

// ── Tube loading ───────────────────────────────────────────────────────────

/// One tube's loading state as seen by the AI *loader* (as opposed to
/// [`TubeSummary`], which is what the AI *gunner* sees).
#[derive(Clone, Debug, PartialEq)]
pub struct TubeLoadSummary {
    pub id: TorpedoTubeId,
    /// The tube's current volley target — what it is loading toward now.
    pub target_count: u32,
    /// The volley target this tube's TOML says an AI crew keeps it at.
    pub ai_target_count: u32,
    /// Whether the tube's own fine system is under AI control this tick.
    pub operates_ai: bool,
}

/// Decide which tubes the AI should re-order a volley target for.
///
/// Returns `(tube_id, count)` pairs for every AI-operated tube whose current
/// `target_count` differs from its configured `ai_target_count` — the caller
/// turns each into one `SetTorpedoVolleyTarget` command. Tubes already sitting
/// at their configured count are skipped so the AI does not re-issue an
/// identical order every tick.
pub fn torpedo_load_orders(tubes: &[TubeLoadSummary]) -> Vec<(TorpedoTubeId, u32)> {
    tubes
        .iter()
        .filter(|t| t.operates_ai && t.target_count != t.ai_target_count)
        .map(|t| (t.id.clone(), t.ai_target_count))
        .collect()
}

// ── Frequency auto-match state ────────────────────────────────────────────

/// Persistent timer state for the frequency auto-match AI.
///
/// When both Tactical and Science are Low (or Science is unmanned), and a
/// target is locked, the AI waits `delay_secs` then synthesises a
/// `SetPhaserFrequency` to match the target's shield frequency.
///
/// Resets whenever the locked target changes or the trigger condition ends.
/// The frequency persists at its last set value when the trigger ends — no
/// auto-revert.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrequencyMatchState {
    /// UUID of the target for which the timer is currently running.
    /// `None` means no timer is active.
    pub current_target: Option<String>,
    /// Accumulated elapsed time while the current target has been locked, in
    /// seconds. Resets to 0.0 on target change.
    pub elapsed_secs: f32,
    /// Set to `true` once the `SetPhaserFrequency` has been synthesised for the
    /// current target. Prevents repeated matches on the same lock.
    pub match_sent: bool,
}

/// All inputs required by `tick_auto_match_frequency`.
#[derive(Clone, Debug)]
pub struct FrequencyMatchInput {
    /// UUID of the currently locked target. `None` = no lock.
    pub locked_target: Option<String>,
    /// The target's shield frequency to match (0.0–1.0).
    pub target_frequency: f32,
    /// Seconds elapsed this frame (delta time).
    pub dt: f32,
    /// Configured delay before the match fires (from TOML / server config).
    pub delay_secs: f32,
    /// Whether the trigger condition is active (both Tactical and Science Low,
    /// or Science unmanned). When `false`, reset state and return `None`.
    pub trigger_active: bool,
}

/// Outcome of a single `tick_auto_match_frequency` call.
#[derive(Clone, Debug, PartialEq)]
pub enum FrequencyMatchOutput {
    /// No action this tick.
    None,
    /// Synthesise `SetPhaserFrequency` with this frequency.
    Match { frequency: f32 },
}

/// Advance the auto-match timer by one tick.
///
/// Rules:
/// - If `trigger_active` is `false`, reset state and return `None`.
/// - If there is no locked target, reset the timer and return `None`.
/// - If the locked target changes, reset the timer to 0 and return `None`.
/// - If the timer has not yet reached `delay_secs`, accumulate `dt` and
///   return `None`.
/// - At the first tick where `elapsed_secs >= delay_secs` (and the match has
///   not already been sent for this target), emit `Match { frequency }` and
///   mark the match as sent.
/// - On subsequent ticks with the same target (match already sent), return
///   `None`.
pub fn tick_auto_match_frequency(
    state: &mut FrequencyMatchState,
    input: &FrequencyMatchInput,
) -> FrequencyMatchOutput {
    if !input.trigger_active {
        *state = FrequencyMatchState::default();
        return FrequencyMatchOutput::None;
    }

    match &input.locked_target {
        None => {
            *state = FrequencyMatchState::default();
            FrequencyMatchOutput::None
        }
        Some(uuid) => {
            // Target changed → reset timer.
            if state.current_target.as_deref() != Some(uuid.as_str()) {
                *state = FrequencyMatchState {
                    current_target: Some(uuid.clone()),
                    elapsed_secs: 0.0,
                    match_sent: false,
                };
            }

            if state.match_sent {
                return FrequencyMatchOutput::None;
            }

            state.elapsed_secs += input.dt;
            if state.elapsed_secs >= input.delay_secs {
                state.match_sent = true;
                FrequencyMatchOutput::Match {
                    frequency: input.target_frequency,
                }
            } else {
                FrequencyMatchOutput::None
            }
        }
    }
}

// ── Power AI ───────────────────────────────────────────────────────────────

/// Engage state machine used by both Power Low AI rules.
///
/// Each rule independently tracks whether a battery overflow point is
/// currently engaged. Engagement is gated by a sustained-condition timer;
/// disengagement is immediate on battery/condition drop.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum EngageState {
    /// No engagement active.
    #[default]
    Idle,
    /// Condition has been sustained for `elapsed_secs` so far.
    Counting { elapsed_secs: f32 },
    /// Overflow point is currently engaged.
    Engaged,
    /// Just disengaged; waiting for battery to reach `battery_recharge_pct`
    /// before counting can restart.
    WaitingForRecharge,
}

/// All inputs required by `tick_power_movement_rule`.
#[derive(Clone, Debug)]
pub struct PowerMovementInput {
    /// Current forward thrust input (0.0–1.0). Comes from latest `HelmInput`.
    pub thrust: f32,
    /// Minimum thrust to count as "driving" (from TOML `thrust_threshold`).
    pub thrust_threshold: f32,
    /// Seconds sustained thrust must persist before engaging (from TOML).
    pub engage_delay_secs: f32,
    /// Battery % must be ≥ this to allow engaging (from TOML).
    pub battery_engage_min_pct: f32,
    /// Battery % must reach this to allow re-engaging after a disengage (from TOML).
    pub battery_recharge_pct: f32,
    /// Current battery percentage (0.0–100.0).
    pub battery_pct: f32,
    /// Seconds elapsed this frame.
    pub dt: f32,
    /// Whether the Power console is currently at "Low" complexity. When
    /// `false`, any pending engage is cancelled and `Idle` is returned.
    pub power_is_low: bool,
}

/// Outcome of a single `tick_power_movement_rule` call.
#[derive(Clone, Debug, PartialEq)]
pub enum PowerEngageOutput {
    /// No change — +1 overflow was NOT engaged this tick.
    NoChange,
    /// Engage the overflow point: add +1 to the target console.
    Engage,
    /// Disengage the overflow point: remove +1 from the target console.
    Disengage,
}

/// Advance the movement-rule AI state by one tick.
///
/// Rules:
/// - If `power_is_low` is false, reset state to `Idle` and return `NoChange`.
/// - While `Idle`: if thrust ≥ threshold AND battery ≥ engage_min_pct, start
///   counting; otherwise stay `Idle`.
/// - While `Counting`: if condition drops, reset to `Idle`; otherwise
///   accumulate `dt`. When `elapsed >= engage_delay_secs`, transition to
///   `Engaged` and return `Engage`.
/// - While `Engaged`: if battery < engage_min_pct, transition to
///   `WaitingForRecharge` and return `Disengage`. Otherwise `NoChange`.
/// - While `WaitingForRecharge`: if battery ≥ battery_recharge_pct, reset to
///   `Idle`. Still `NoChange` (re-engagement starts from `Idle` next tick).
pub fn tick_power_movement_rule(
    state: &mut EngageState,
    input: &PowerMovementInput,
) -> PowerEngageOutput {
    if !input.power_is_low {
        *state = EngageState::Idle;
        return PowerEngageOutput::NoChange;
    }

    let condition_met =
        input.thrust >= input.thrust_threshold && input.battery_pct >= input.battery_engage_min_pct;

    match state {
        EngageState::Idle => {
            if condition_met {
                *state = EngageState::Counting {
                    elapsed_secs: input.dt,
                };
                // Check if we already crossed the threshold in this tick.
                if input.dt >= input.engage_delay_secs {
                    *state = EngageState::Engaged;
                    return PowerEngageOutput::Engage;
                }
            }
            PowerEngageOutput::NoChange
        }
        EngageState::Counting { elapsed_secs } => {
            if !condition_met {
                *state = EngageState::Idle;
                return PowerEngageOutput::NoChange;
            }
            *elapsed_secs += input.dt;
            if *elapsed_secs >= input.engage_delay_secs {
                *state = EngageState::Engaged;
                PowerEngageOutput::Engage
            } else {
                PowerEngageOutput::NoChange
            }
        }
        EngageState::Engaged => {
            if input.battery_pct < input.battery_engage_min_pct {
                *state = EngageState::WaitingForRecharge;
                PowerEngageOutput::Disengage
            } else {
                PowerEngageOutput::NoChange
            }
        }
        EngageState::WaitingForRecharge => {
            if input.battery_pct >= input.battery_recharge_pct {
                *state = EngageState::Idle;
            }
            PowerEngageOutput::NoChange
        }
    }
}

/// All inputs required by `tick_power_red_alert_rule`.
#[derive(Clone, Debug)]
pub struct PowerRedAlertInput {
    /// Whether red alert is currently active.
    pub red_alert: bool,
    /// Seconds red alert must persist before engaging (from TOML).
    pub engage_delay_secs: f32,
    /// Battery % must be ≥ this to allow engaging.
    pub battery_engage_min_pct: f32,
    /// Battery % must reach this to allow re-engaging after a disengage.
    pub battery_recharge_pct: f32,
    /// Current battery percentage (0.0–100.0).
    pub battery_pct: f32,
    /// Seconds elapsed this frame.
    pub dt: f32,
    /// Whether the Power console is currently at "Low" complexity.
    pub power_is_low: bool,
}

/// Advance the red-alert-rule AI state by one tick.
///
/// Symmetric to `tick_power_movement_rule`, but the condition is
/// `red_alert == true AND battery ≥ engage_min_pct`.
pub fn tick_power_red_alert_rule(
    state: &mut EngageState,
    input: &PowerRedAlertInput,
) -> PowerEngageOutput {
    let movement_input = PowerMovementInput {
        thrust: if input.red_alert { 1.0 } else { 0.0 },
        thrust_threshold: 0.5,
        engage_delay_secs: input.engage_delay_secs,
        battery_engage_min_pct: input.battery_engage_min_pct,
        battery_recharge_pct: input.battery_recharge_pct,
        battery_pct: input.battery_pct,
        dt: input.dt,
        power_is_low: input.power_is_low,
    };
    tick_power_movement_rule(state, &movement_input)
}

// ── Data-authored power rules (issue #762) ──────────────────────────────────
//
// Generalises the two hardcoded rule blocks (movement→helm, red-alert→weapons)
// that used to live in `console_ai::server::ai_power_allocation` into a single
// per-rule evaluator. Each authored rule targets one power group, declares its
// own battery floor (`min_battery_reserve`), and reuses the exact
// `EngageState` / `tick_power_movement_rule` hysteresis so the timer/battery
// semantics are unchanged — only the source of the trigger and thresholds
// moves from hardcoded fields to TOML.

/// What condition arms an authored power rule.
///
/// This is the only place the rule vocabulary is hardcoded — the *set* of
/// rules, their target groups, and their floors are all authored in TOML.
/// Adding a new trigger kind is an intentional code change (a new sensor the
/// AI can read), not a balance tweak.
#[derive(Clone, Debug, PartialEq)]
pub enum PowerRuleTrigger {
    /// Armed while forward thrust ≥ `threshold` (0.0–1.0).
    Thrust { threshold: f32 },
    /// Armed while the ship is at red alert.
    RedAlert,
}

/// A single data-authored power-allocation rule (issue #762).
///
/// Each rule nudges one power group's allocation by `nudge` when its trigger
/// has been sustained for `engage_delay_secs` **and** the battery is at or
/// above `min_battery_reserve` — the per-rule floor below which the rule can
/// never engage (AC2). Disengagement is immediate on battery/condition drop,
/// with re-arming gated on `battery_recharge_pct`, exactly as the legacy
/// movement/red-alert rules behaved.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerAiRule {
    /// Target power group id string (e.g. "helm", "weapons", "ops").
    pub group: String,
    /// Condition that arms this rule.
    pub trigger: PowerRuleTrigger,
    /// Battery floor (0.0–100.0). The rule cannot engage while battery is
    /// below this, and disengages if battery falls below it while engaged.
    pub min_battery_reserve: f32,
    /// Battery % that must be reached to re-arm after a disengage.
    pub battery_recharge_pct: f32,
    /// Seconds the trigger must persist before engaging.
    pub engage_delay_secs: f32,
    /// Allocation delta applied to `group` on engage (and removed on
    /// disengage). Typically `1`.
    pub nudge: i16,
}

impl PowerAiRule {
    /// The two canonical rules the hardcoded blocks used to implement, now
    /// expressed as data. Used as the parse-time back-compat default for a
    /// ship whose `[power.ai]` block carries the flat legacy fields (or no
    /// `[[power.ai.rule]]` array at all).
    pub fn legacy_defaults() -> Vec<PowerAiRule> {
        vec![
            PowerAiRule {
                group: crate::modifiers::power_system::HELM_POWER_GROUP.to_string(),
                trigger: PowerRuleTrigger::Thrust { threshold: 0.7 },
                min_battery_reserve: 50.0,
                battery_recharge_pct: 100.0,
                engage_delay_secs: 3.0,
                nudge: 1,
            },
            PowerAiRule {
                group: crate::modifiers::power_system::WEAPONS_POWER_GROUP.to_string(),
                trigger: PowerRuleTrigger::RedAlert,
                min_battery_reserve: 10.0,
                battery_recharge_pct: 100.0,
                engage_delay_secs: 3.0,
                nudge: 1,
            },
        ]
    }
}

/// Per-tick sensor readings shared by every rule on a ship.
#[derive(Clone, Debug)]
pub struct PowerRuleInput {
    /// Current forward thrust (0.0–1.0), from the latest `HelmInput`.
    pub thrust: f32,
    /// Whether red alert is active.
    pub red_alert: bool,
    /// Current battery percentage (0.0–100.0).
    pub battery_pct: f32,
    /// Seconds elapsed this frame.
    pub dt: f32,
    /// When `false`, any pending engage is cancelled and `NoChange` returned
    /// (mirrors the retired Low/Full complexity gate). The Bevy caller passes
    /// `true` while the reactor is AI-operated.
    pub enabled: bool,
}

/// Advance one authored rule's `EngageState` by a tick.
///
/// Reuses [`tick_power_movement_rule`] verbatim: the rule's trigger is reduced
/// to a boolean "armed" signal, fed in as saturated thrust past a fixed 0.5
/// threshold, and the rule's own `min_battery_reserve` is passed as the
/// `battery_engage_min_pct` floor. This keeps a single hysteresis engine for
/// every rule while giving each rule an independent battery floor (AC2).
pub fn tick_power_rule(
    state: &mut EngageState,
    rule: &PowerAiRule,
    input: &PowerRuleInput,
) -> PowerEngageOutput {
    let armed = match &rule.trigger {
        PowerRuleTrigger::Thrust { threshold } => input.thrust >= *threshold,
        PowerRuleTrigger::RedAlert => input.red_alert,
    };
    let movement_input = PowerMovementInput {
        thrust: if armed { 1.0 } else { 0.0 },
        thrust_threshold: 0.5,
        engage_delay_secs: rule.engage_delay_secs,
        battery_engage_min_pct: rule.min_battery_reserve,
        battery_recharge_pct: rule.battery_recharge_pct,
        battery_pct: input.battery_pct,
        dt: input.dt,
        power_is_low: input.enabled,
    };
    tick_power_movement_rule(state, &movement_input)
}

// ── Shields AI ────────────────────────────────────────────────────────────

/// Input for the shield focus AI decision function.
#[derive(Clone, Debug)]
pub struct ShieldFocusAiInput {
    /// Current shield facing snapshots.
    pub facings: Vec<ShieldFacingSnapshot>,
    /// Whether the Shields console is at Low complexity.
    /// When false, no AI action is taken.
    pub shields_is_low: bool,
    /// Per-arc damage history, indexed by facing index.
    /// Records older than `damage_window_secs` should be pruned by the caller.
    pub damage_history: Vec<Vec<DamageRecord>>,
    /// Maximum time window (seconds) for damage tracking.
    pub damage_window_secs: f32,
    /// Minimum time window (seconds) before the AI reacts to damage concentration.
    pub min_damage_window_secs: f32,
    /// Percentage threshold (0.0–100.0): if an arc receives this fraction of
    /// total damage in the active window, focus it.
    pub damage_pct_threshold: f32,
    /// Percentage threshold (0.0–100.0): if the lowest-arc normalized health
    /// is below this fraction of the second-lowest, focus the weakest arc.
    pub health_ratio_threshold: f32,
    /// Current absolute time in seconds (used to compute the active window).
    pub current_time_secs: f32,
}

/// Outcome of a single `tick_shield_focus_ai` call.
#[derive(Clone, Debug, PartialEq)]
pub enum ShieldFocusAiOutput {
    /// No focus change this tick.
    None,
    /// Focus the given facing (by index).
    Focus { facing_index: usize },
    /// Clear the current focus.
    ClearFocus,
}

/// Decide which shield facing to focus based on current shield state.
///
/// Rules (evaluated in order):
/// 1. If `shields_is_low` is false or there are fewer than 2 facings, return
///    `None` (no AI involvement; single-arc ships have nothing to focus).
/// 2. Damage concentration check — sum recorded damage per arc over the
///    authored recent-damage window `[current_time - window, current_time]`,
///    where `window = max(damage_window_secs, min_damage_window_secs)` (the
///    authored `damage_window_secs`, floored at `min_damage_window_secs` so a
///    misconfigured window can never shrink below the reaction minimum). The
///    caller prunes records older than `damage_window_secs` first. If any arc
///    accounts for `damage_pct_threshold` % or more of total window damage,
///    focus it.
/// 3. Health imbalance check — if no arc met the damage threshold, compare
///    normalized health fractions (hp/max_hp). Sort ascending; if the lowest
///    is below `(health_ratio_threshold/100) × second_lowest`, focus it.
/// 4. Otherwise return `ClearFocus`.
pub fn tick_shield_focus_ai(input: &ShieldFocusAiInput) -> ShieldFocusAiOutput {
    if !input.shields_is_low || input.facings.len() < 2 {
        return ShieldFocusAiOutput::None;
    }

    let n = input.facings.len();

    // ── 1. Damage concentration check ────────────────────────────────────────
    // Prune is done by the caller (operate_shields_ai prunes before building input).
    // Concentration is measured over the AUTHORED recent-damage window
    // (`damage_window_secs`), floored at `min_damage_window_secs` so a
    // misconfigured window can never fall below the reaction minimum. The
    // authored window — not a fixed last-`min_damage_window_secs` slice — is
    // what "recent concentrated damage over authored windows" (issue #747)
    // means.
    let window = input.damage_window_secs.max(input.min_damage_window_secs);
    let effective_start = input.current_time_secs - window;

    let mut damage_per_arc: Vec<i32> = vec![0; n];
    let mut total_window_damage: i32 = 0;

    for (idx, records) in input.damage_history.iter().enumerate() {
        if idx >= n {
            break;
        }
        for record in records {
            if record.timestamp >= effective_start && record.timestamp <= input.current_time_secs {
                damage_per_arc[idx] += record.amount;
                total_window_damage += record.amount;
            }
        }
    }

    if total_window_damage > 0 {
        let threshold = input.damage_pct_threshold / 100.0;
        for (idx, &dmg) in damage_per_arc.iter().enumerate() {
            let fraction = dmg as f32 / total_window_damage as f32;
            if fraction >= threshold {
                // Don't re-focus the already-focused arc
                if !input.facings[idx].is_focused {
                    return ShieldFocusAiOutput::Focus { facing_index: idx };
                }
                return ShieldFocusAiOutput::None;
            }
        }
    }

    // ── 2. Health imbalance check ────────────────────────────────────────────
    #[derive(Clone)]
    struct HealthEntry {
        index: usize,
        normalized: f32,
    }

    let mut healths: Vec<HealthEntry> = input
        .facings
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let normalized = if f.max_hp > 0 {
                f.hp as f32 / f.max_hp as f32
            } else {
                0.0
            };
            HealthEntry {
                index: i,
                normalized,
            }
        })
        .collect();

    healths.sort_by(|a, b| a.normalized.partial_cmp(&b.normalized).unwrap());

    if healths.len() >= 2 {
        let lowest = &healths[0];
        let second_lowest = &healths[1];

        let ratio_threshold = input.health_ratio_threshold / 100.0;
        if lowest.normalized < ratio_threshold * second_lowest.normalized {
            if !input.facings[lowest.index].is_focused {
                return ShieldFocusAiOutput::Focus {
                    facing_index: lowest.index,
                };
            }
            return ShieldFocusAiOutput::None;
        }
    }

    ShieldFocusAiOutput::ClearFocus
}

/// Seed the per-tick policy fact snapshot for the Shields focus decision (issue
/// #783), modelled on [`crate::console::weapons::beam::seed_phaser_bank_facts`].
///
/// This is THE piece that exposes BOUNDED RECENT INCOMING-DAMAGE facts by shield
/// arc (AC1) and closes the #779 empty-facts sharp edge for the Shields policy:
/// without seeding, a `fact(...)` guard validates but never fires. Every reading
/// is computed from the ALREADY-PRUNED window the caller built (records older
/// than `damage_window_secs` are gone before this runs) — "bounded" means we read
/// only that window and add NO new unbounded accumulator. The window matches the
/// kernel's concentration window exactly (`max(damage_window_secs,
/// min_damage_window_secs)`), so the facts describe the same slice the retained
/// argmax ranks over.
///
/// Facts emitted:
///   - `recent_damage_<arc-id>` — per-arc damage summed over the window (AC1).
///   - `recent_damage_total` — total window damage across all arcs.
///   - `recent_damage_fraction_max` / `recent_damage_pct_max` — the most
///     concentrated arc's share of the total (0–1 and 0–100). The concentration
///     signal the authored damage rule gates on (AC2).
///   - `health_fraction_min_ratio` / `health_ratio_pct` — the lowest arc's
///     normalized health as a ratio of the second-lowest (0–1 and 0–100). The
///     shield-health imbalance signal used only as the authored FALLBACK (AC3).
///
/// Pure and Bevy-free (AGENTS.md rule #10): the host resolves the live per-arc
/// state before calling this, so the policy evaluates over real readings while
/// `policy.rs` stays free of ECS types.
pub fn seed_shields_focus_facts(
    facings: &[crate::shield::ShieldFacingSnapshot],
    damage_history: &[Vec<DamageRecord>],
    damage_window_secs: f32,
    min_damage_window_secs: f32,
    current_time_secs: f32,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();

    // Same window the kernel measures concentration over.
    let window = damage_window_secs.max(min_damage_window_secs);
    let effective_start = current_time_secs - window;

    let mut total: i32 = 0;
    let mut max_arc: i32 = 0;
    for (idx, facing) in facings.iter().enumerate() {
        let arc_sum: i32 = damage_history
            .get(idx)
            .map(|records| {
                records
                    .iter()
                    .filter(|r| r.timestamp >= effective_start && r.timestamp <= current_time_secs)
                    .map(|r| r.amount)
                    .sum()
            })
            .unwrap_or(0);
        // Per-arc bounded recent-damage fact, keyed by the stable arc id (for a
        // canonical 4-arc ship: `recent_damage_fore/port/aft/starboard`).
        if !facing.id.is_empty() {
            facts.set(&format!("recent_damage_{}", facing.id), arc_sum as f64);
        }
        total += arc_sum;
        max_arc = max_arc.max(arc_sum);
    }

    facts.set("recent_damage_total", total as f64);
    let fraction_max = if total > 0 {
        max_arc as f64 / total as f64
    } else {
        0.0
    };
    facts.set("recent_damage_fraction_max", fraction_max);
    facts.set("recent_damage_pct_max", fraction_max * 100.0);

    // Health-imbalance fallback signal: lowest normalized health as a ratio of
    // the second-lowest (the same comparison the kernel's fallback branch makes).
    let mut normalized: Vec<f32> = facings
        .iter()
        .map(|f| {
            if f.max_hp > 0 {
                f.hp as f32 / f.max_hp as f32
            } else {
                0.0
            }
        })
        .collect();
    normalized.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if normalized.len() >= 2 {
        let lowest = normalized[0];
        let second = normalized[1];
        let ratio = if second > 0.0 { lowest / second } else { 1.0 };
        facts.set("health_fraction_min_ratio", ratio as f64);
        facts.set("health_ratio_pct", (ratio * 100.0) as f64);
    }

    facts
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
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
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
        assert!(!state.match_sent);
    }

    #[test]
    fn auto_match_at_delay_fires_with_correct_frequency() {
        let mut state = FrequencyMatchState::default();
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.75, 3.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.75 });
        assert!(state.match_sent);
    }

    #[test]
    fn auto_match_above_delay_fires() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.5 });
    }

    #[test]
    fn auto_match_fires_only_once_per_target() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 5.0, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::None);
    }

    #[test]
    fn auto_match_target_change_resets_timer() {
        let mut state = FrequencyMatchState::default();
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.5, 2.9, 3.0, true));
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
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t2"), 0.9, 1.5, 3.0, true));
        assert_eq!(out, FrequencyMatchOutput::Match { frequency: 0.9 });
    }

    #[test]
    fn auto_match_trigger_flip_to_inactive_resets_state() {
        let mut state = FrequencyMatchState::default();
        // Nearly at delay
        tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 2.9, 3.0, true));
        // Trigger turns off (e.g. either console goes Full)
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
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
        let out =
            tick_auto_match_frequency(&mut state, &match_input(Some("t1"), 0.5, 1.0, 3.0, false));
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

    // ── Condition: facing-arc shields ──────────────────────────────────────

    #[test]
    fn facing_arc_up_no_fire() {
        let mut input = all_ready_input();
        input.target_facing_shields = 50;
        let result = auto_fire_torpedo(&input);
        assert!(
            result.is_empty(),
            "should not fire while the arc the torpedo would strike is up"
        );
    }

    #[test]
    fn facing_arc_depleted_fires() {
        let mut input = all_ready_input();
        input.target_facing_shields = 0;
        let result = auto_fire_torpedo(&input);
        assert!(
            !result.is_empty(),
            "should fire when the facing arc is depleted"
        );
    }

    #[test]
    fn facing_arc_overkilled_fires() {
        let mut input = all_ready_input();
        input.target_facing_shields = -5;
        let result = auto_fire_torpedo(&input);
        assert!(
            !result.is_empty(),
            "should fire when the facing arc is past zero"
        );
    }

    /// A target with no shield arcs at all (asteroid, unshielded NPC) has
    /// nothing blocking the shot — the caller reports 0 and the AI fires.
    #[test]
    fn target_with_no_arcs_fires() {
        let mut input = all_ready_input();
        input.target_facing_shields = 0;
        let result = auto_fire_torpedo(&input);
        assert!(
            !result.is_empty(),
            "an unshielded target must stay torpedo-eligible"
        );
    }

    /// The case that motivated the per-arc gate: a four-arc hull whose FRONT
    /// arc has collapsed while all three others are healthy. Attacking from
    /// ahead the shot is on; from astern the healthy rear arc still blocks it.
    /// The caller does the bearing→arc resolution, so the pure fn sees only
    /// the resolved arc's HP — these two calls are the same target, one tick
    /// apart, differing solely in where the attacker sits.
    #[test]
    fn collapsed_front_arc_fires_only_from_the_front() {
        let mut from_ahead = all_ready_input();
        from_ahead.target_facing_shields = 0; // front arc, collapsed
        assert!(
            !auto_fire_torpedo(&from_ahead).is_empty(),
            "healthy rear arcs must not veto a shot into a collapsed front arc"
        );

        let mut from_astern = all_ready_input();
        from_astern.target_facing_shields = 120; // rear arc, healthy
        assert!(
            auto_fire_torpedo(&from_astern).is_empty(),
            "a collapsed front arc must not licence a shot into a healthy rear arc"
        );
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

    // ── Condition: magazine ────────────────────────────────────────────────

    #[test]
    fn empty_magazine_no_fire() {
        let mut input = all_ready_input();
        input.magazine = 0;
        let result = auto_fire_torpedo(&input);
        assert!(result.is_empty(), "should not fire when magazine is empty");
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

    // ── Power movement rule helpers ───────────────────────────────────────

    fn movement_input(thrust: f32, battery_pct: f32, dt: f32) -> PowerMovementInput {
        PowerMovementInput {
            thrust,
            thrust_threshold: 0.7,
            engage_delay_secs: 3.0,
            battery_engage_min_pct: 50.0,
            battery_recharge_pct: 100.0,
            battery_pct,
            dt,
            power_is_low: true,
        }
    }

    // ── tick_power_movement_rule ──────────────────────────────────────────

    #[test]
    fn thrust_below_threshold_no_engage() {
        let mut state = EngageState::Idle;
        let out = tick_power_movement_rule(&mut state, &movement_input(0.5, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle);
    }

    #[test]
    fn thrust_at_threshold_with_battery_starts_counting() {
        let mut state = EngageState::Idle;
        let out = tick_power_movement_rule(&mut state, &movement_input(0.7, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert!(matches!(state, EngageState::Counting { .. }));
    }

    #[test]
    fn sustained_thrust_and_battery_engages_after_delay() {
        let mut state = EngageState::Idle;
        // 2 ticks of 1.5s = 3.0s total ≥ engage_delay_secs(3.0)
        tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.5));
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.5));
        assert_eq!(
            out,
            PowerEngageOutput::Engage,
            "should engage after delay elapsed"
        );
        assert_eq!(state, EngageState::Engaged);
    }

    #[test]
    fn thrust_drop_before_engage_resets_timer() {
        let mut state = EngageState::Idle;
        // Start counting
        tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.0));
        assert!(matches!(state, EngageState::Counting { .. }));
        // Thrust drops below threshold
        let out = tick_power_movement_rule(&mut state, &movement_input(0.3, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(
            state,
            EngageState::Idle,
            "timer should reset when thrust drops"
        );
    }

    #[test]
    fn battery_dip_below_min_during_engage_disengages() {
        let mut state = EngageState::Engaged;
        // Battery drops below 50%
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 40.0, 1.0));
        assert_eq!(out, PowerEngageOutput::Disengage);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn waiting_for_recharge_no_re_engage_until_full() {
        let mut state = EngageState::WaitingForRecharge;
        // Battery at 80% — below recharge_pct (100%)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn recharge_to_full_transitions_to_idle_allowing_re_engage() {
        let mut state = EngageState::WaitingForRecharge;
        // Battery reaches 100% (recharge_pct)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.8, 100.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(
            state,
            EngageState::Idle,
            "should return to Idle when battery recharged"
        );
    }

    #[test]
    fn switching_to_full_complexity_cancels_pending_engage() {
        let mut state = EngageState::Counting { elapsed_secs: 2.9 };
        let mut input = movement_input(0.8, 80.0, 0.5);
        input.power_is_low = false; // switched to Full
        let out = tick_power_movement_rule(&mut state, &input);
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(
            state,
            EngageState::Idle,
            "pending engage cancelled when switching to Full"
        );
    }

    #[test]
    fn switching_to_full_while_engaged_disengages_immediately() {
        // The plugin must disengage before cancelling. This tests that
        // the state machine goes to Idle when power_is_low is false,
        // and the plugin is responsible for synthesising the Disengage action
        // when the console goes Full while engaged.
        let mut state = EngageState::Engaged;
        let mut input = movement_input(0.8, 80.0, 1.0);
        input.power_is_low = false;
        let out = tick_power_movement_rule(&mut state, &input);
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "state resets to Idle on Full");
    }

    #[test]
    fn low_battery_prevents_counting() {
        let mut state = EngageState::Idle;
        // Thrust high but battery below minimum (50%)
        let out = tick_power_movement_rule(&mut state, &movement_input(0.9, 40.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(
            state,
            EngageState::Idle,
            "should not count when battery too low"
        );
    }

    // ── tick_power_red_alert_rule ─────────────────────────────────────────

    fn red_alert_input(red_alert: bool, battery_pct: f32, dt: f32) -> PowerRedAlertInput {
        PowerRedAlertInput {
            red_alert,
            engage_delay_secs: 3.0,
            battery_engage_min_pct: 10.0,
            battery_recharge_pct: 100.0,
            battery_pct,
            dt,
            power_is_low: true,
        }
    }

    #[test]
    fn no_red_alert_no_weapons_engage() {
        let mut state = EngageState::Idle;
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(false, 80.0, 1.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle);
    }

    #[test]
    fn sustained_red_alert_and_battery_engages_weapons() {
        let mut state = EngageState::Idle;
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 50.0, 1.5));
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(true, 50.0, 1.5));
        assert_eq!(
            out,
            PowerEngageOutput::Engage,
            "should engage after delay under red alert"
        );
    }

    #[test]
    fn red_alert_battery_dip_disengages_weapons() {
        let mut state = EngageState::Engaged;
        // Battery drops below 10% (min for red alert rule)
        let out = tick_power_red_alert_rule(&mut state, &red_alert_input(true, 5.0, 1.0));
        assert_eq!(out, PowerEngageOutput::Disengage);
        assert_eq!(state, EngageState::WaitingForRecharge);
    }

    #[test]
    fn red_alert_re_engage_gated_on_recharge_pct() {
        let mut state = EngageState::WaitingForRecharge;
        // At 80%, recharge_pct=100% → still waiting
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 80.0, 1.0));
        assert_eq!(state, EngageState::WaitingForRecharge);
        // At 100%, transitions to Idle
        tick_power_red_alert_rule(&mut state, &red_alert_input(true, 100.0, 1.0));
        assert_eq!(state, EngageState::Idle);
    }

    // ── Both rules stacking ───────────────────────────────────────────────

    #[test]
    fn both_rules_can_engage_simultaneously() {
        let mut helm_state = EngageState::Idle;
        let mut weapons_state = EngageState::Idle;

        // Sustained thrust + red alert + full battery → both engage after delay
        for _ in 0..3 {
            tick_power_movement_rule(&mut helm_state, &movement_input(0.9, 80.0, 1.0));
            tick_power_red_alert_rule(&mut weapons_state, &red_alert_input(true, 80.0, 1.0));
        }

        // At 3s elapsed both should be Engaged now
        assert_eq!(
            helm_state,
            EngageState::Engaged,
            "movement rule should be Engaged"
        );
        assert_eq!(
            weapons_state,
            EngageState::Engaged,
            "red alert rule should be Engaged"
        );
    }

    // ── Data-authored rule evaluator (issue #762) ─────────────────────────

    fn thrust_rule(group: &str, min_reserve: f32) -> PowerAiRule {
        PowerAiRule {
            group: group.to_string(),
            trigger: PowerRuleTrigger::Thrust { threshold: 0.7 },
            min_battery_reserve: min_reserve,
            battery_recharge_pct: 100.0,
            engage_delay_secs: 3.0,
            nudge: 1,
        }
    }

    fn red_alert_rule(group: &str, min_reserve: f32) -> PowerAiRule {
        PowerAiRule {
            group: group.to_string(),
            trigger: PowerRuleTrigger::RedAlert,
            min_battery_reserve: min_reserve,
            battery_recharge_pct: 100.0,
            engage_delay_secs: 3.0,
            nudge: 1,
        }
    }

    fn rule_input(thrust: f32, red_alert: bool, battery_pct: f32, dt: f32) -> PowerRuleInput {
        PowerRuleInput {
            thrust,
            red_alert,
            battery_pct,
            dt,
            enabled: true,
        }
    }

    #[test]
    fn rule_engages_just_above_its_configured_floor() {
        // AC2 reserve boundary: a rule with a 50% floor engages at 50.0.
        let rule = thrust_rule("helm", 50.0);
        let mut state = EngageState::Idle;
        // 4s ≥ 3s delay in a single tick, battery exactly at the floor.
        let out = tick_power_rule(&mut state, &rule, &rule_input(0.9, false, 50.0, 4.0));
        assert_eq!(out, PowerEngageOutput::Engage);
        assert_eq!(state, EngageState::Engaged);
    }

    #[test]
    fn rule_cannot_engage_just_below_its_configured_floor() {
        // AC2 reserve boundary: the SAME rule cannot even start counting at
        // 49.9%, one tenth of a percent under its own floor.
        let rule = thrust_rule("helm", 50.0);
        let mut state = EngageState::Idle;
        let out = tick_power_rule(&mut state, &rule, &rule_input(0.9, false, 49.9, 4.0));
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle, "must not count below the floor");
    }

    #[test]
    fn per_rule_floor_is_independent_across_rules() {
        // Two rules on the same trigger but DIFFERENT floors: at 30% battery a
        // 10%-floor rule engages while a 50%-floor rule stays idle. Proves the
        // floor is per-rule, not a shared system-wide threshold.
        let low_floor = red_alert_rule("weapons", 10.0);
        let high_floor = red_alert_rule("shields", 50.0);
        let mut low_state = EngageState::Idle;
        let mut high_state = EngageState::Idle;

        let low_out = tick_power_rule(
            &mut low_state,
            &low_floor,
            &rule_input(0.0, true, 30.0, 4.0),
        );
        let high_out = tick_power_rule(
            &mut high_state,
            &high_floor,
            &rule_input(0.0, true, 30.0, 4.0),
        );

        assert_eq!(
            low_out,
            PowerEngageOutput::Engage,
            "10% floor engages at 30%"
        );
        assert_eq!(
            high_out,
            PowerEngageOutput::NoChange,
            "50% floor cannot engage at 30%"
        );
    }

    #[test]
    fn conflicting_rules_on_same_group_both_evaluate_independently() {
        // Two rules competing for the SAME group ("weapons"): one armed by
        // thrust, one by red alert. Each keeps its own EngageState, so their
        // engage/disengage decisions do not clobber one another. Here thrust is
        // high but battery (40%) is below the thrust rule's 50% floor, while
        // red alert is active and battery is above the red-alert rule's 10%
        // floor — so only the red-alert rule engages.
        let thrust = thrust_rule("weapons", 50.0);
        let red = red_alert_rule("weapons", 10.0);
        let mut thrust_state = EngageState::Idle;
        let mut red_state = EngageState::Idle;

        let input = rule_input(0.9, true, 40.0, 4.0);
        let thrust_out = tick_power_rule(&mut thrust_state, &thrust, &input);
        let red_out = tick_power_rule(&mut red_state, &red, &input);

        assert_eq!(
            thrust_out,
            PowerEngageOutput::NoChange,
            "thrust rule gated out by its own 50% floor at 40% battery"
        );
        assert_eq!(
            red_out,
            PowerEngageOutput::Engage,
            "red-alert rule engages on the same group under its 10% floor"
        );
    }

    #[test]
    fn disabled_input_cancels_pending_engage() {
        // Preserves the human-control yield: when the reactor is not AI-operated
        // the caller passes enabled=false and any pending engage resets.
        let rule = thrust_rule("helm", 50.0);
        let mut state = EngageState::Counting { elapsed_secs: 2.9 };
        let mut input = rule_input(0.9, false, 80.0, 1.0);
        input.enabled = false;
        let out = tick_power_rule(&mut state, &rule, &input);
        assert_eq!(out, PowerEngageOutput::NoChange);
        assert_eq!(state, EngageState::Idle);
    }

    // ── Shields AI ────────────────────────────────────────────────────────

    use crate::shield::ShieldFacingSnapshot;

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
        current_time_secs: f32,
    ) -> ShieldFocusAiInput {
        ShieldFocusAiInput {
            facings,
            shields_is_low,
            damage_history,
            damage_window_secs: 4.0,
            min_damage_window_secs: 1.0,
            damage_pct_threshold: 50.0,
            health_ratio_threshold: 50.0,
            current_time_secs,
        }
    }

    #[test]
    fn shield_ai_not_low_returns_none() {
        let input = make_input(
            vec![make_snap("Fore", 50, 100, false)],
            false,
            empty_history(1),
            0.0,
        );
        assert_eq!(tick_shield_focus_ai(&input), ShieldFocusAiOutput::None);
    }

    #[test]
    fn shield_ai_single_arc_returns_none() {
        let input = make_input(
            vec![make_snap("All", 50, 100, false)],
            true,
            empty_history(1),
            0.0,
        );
        assert_eq!(tick_shield_focus_ai(&input), ShieldFocusAiOutput::None);
    }

    #[test]
    fn shield_ai_damage_concentration_focuses_arc() {
        // Arc at index 1 (Port) takes 80% of damage over the AUTHORED window
        // → should be focused even though every arc's health is equal.
        // Concentration is measured over [current_time - window, current_time]
        // where window = max(damage_window_secs, min_damage_window_secs) = 4.0,
        // i.e. [0.0, 4.0]. The damage sits at t=1.0 — inside the authored 4s
        // window but OUTSIDE the old last-`min_damage_window_secs` slice
        // ([3.0, 4.0]) that the pre-#747 code measured. With balanced health
        // (all arcs 90/100) the health-imbalance fallback cannot fire, so a
        // Focus here proves the authored window governs concentration.
        let mut history = empty_history(4);
        // Damage to Port at t=1.0s (inside the authored 4s window).
        history[1].push(DamageRecord {
            timestamp: 1.0,
            amount: 80,
        });
        // Scattered damage to other arcs at the same time.
        history[0].push(DamageRecord {
            timestamp: 1.0,
            amount: 10,
        });
        history[2].push(DamageRecord {
            timestamp: 1.0,
            amount: 10,
        });
        let facings = vec![
            make_snap("Fore", 90, 100, false),
            make_snap("Port", 90, 100, false),
            make_snap("Aft", 90, 100, false),
            make_snap("Starboard", 90, 100, false),
        ];
        let input = make_input(facings, true, history, 4.0);
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
                timestamp: 3.5,
                amount: 25,
            });
        }
        let facings = vec![
            make_snap("Fore", 75, 100, false),
            make_snap("Port", 75, 100, false),
            make_snap("Aft", 75, 100, false),
            make_snap("Starboard", 75, 100, false),
        ];
        let input = make_input(facings, true, history, 4.0);
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
        let input = make_input(facings, true, empty_history(4), 5.0);
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
        let input = make_input(facings, true, empty_history(4), 5.0);
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
        let input = make_input(facings, true, empty_history(4), 5.0);
        assert_eq!(
            tick_shield_focus_ai(&input),
            ShieldFocusAiOutput::ClearFocus
        );
    }

    #[test]
    fn shield_ai_damage_outside_active_window_ignored() {
        // Damage on Port (idx 1) at t=1.0s is older than the authored window
        // when current_time=6.0: window = max(4.0, 1.0) = 4.0, so the active
        // window is [2.0, 6.0] and t=1.0 falls outside it → the concentration
        // branch sees nothing and the decision must fall through to health.
        // Port is kept healthy (90/100) and Aft (idx 2) is the weak arc, so if
        // the expired hit were (wrongly) counted the result would be Focus{1};
        // because it is ignored, health imbalance focuses Aft instead.
        let mut history = empty_history(4);
        history[1].push(DamageRecord {
            timestamp: 1.0, // older than the 2.0s window start
            amount: 80,
        });
        let facings = vec![
            make_snap("Fore", 90, 100, false),
            make_snap("Port", 90, 100, false),
            make_snap("Aft", 20, 100, false),
            make_snap("Starboard", 90, 100, false),
        ];
        let input = make_input(facings, true, history, 6.0);
        // No damage in active window → health check.
        // worst normalized = 0.2 (Aft), second = 0.9, 0.5*0.9=0.45, 0.2<0.45 → focus Aft
        assert_eq!(
            tick_shield_focus_ai(&input),
            ShieldFocusAiOutput::Focus { facing_index: 2 }
        );
    }

    #[test]
    fn shield_ai_damage_in_future_window_ignored() {
        // Damage on Port (idx 1) at t=5.0s is in the future relative to
        // current_time=4.0 (active window [0.0, 4.0]) and must be ignored.
        // Port is kept healthy and Aft (idx 2) is the weak arc, so the ignored
        // future hit cannot mask the health-imbalance fallback focusing Aft.
        let mut history = empty_history(4);
        history[1].push(DamageRecord {
            timestamp: 5.0,
            amount: 80,
        });
        let facings = vec![
            make_snap("Fore", 90, 100, false),
            make_snap("Port", 90, 100, false),
            make_snap("Aft", 20, 100, false),
            make_snap("Starboard", 90, 100, false),
        ];
        let input = make_input(facings, true, history, 4.0);
        // Future damage ignored → health check: worst=0.2 (Aft), second=0.9 → focus Aft
        assert_eq!(
            tick_shield_focus_ai(&input),
            ShieldFocusAiOutput::Focus { facing_index: 2 }
        );
    }

    #[test]
    fn shield_ai_concentration_window_floored_at_min_damage_window() {
        // A misconfigured authored window (damage_window_secs=0.5) below the
        // reaction minimum (min_damage_window_secs=2.0) must NOT shrink the
        // concentration window below the floor. window = max(0.5, 2.0) = 2.0,
        // so the active window is [2.0, 4.0] and Port's hit at t=2.5 is
        // counted → Focus{1}. Without the floor (window=0.5 → [3.5, 4.0]) the
        // hit would be excluded and, with balanced health, the decision would
        // clear instead.
        let mut history = empty_history(4);
        history[1].push(DamageRecord {
            timestamp: 2.5,
            amount: 80,
        });
        history[0].push(DamageRecord {
            timestamp: 2.5,
            amount: 10,
        });
        history[2].push(DamageRecord {
            timestamp: 2.5,
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
            damage_window_secs: 0.5,
            min_damage_window_secs: 2.0,
            damage_pct_threshold: 50.0,
            health_ratio_threshold: 50.0,
            current_time_secs: 4.0,
        };
        assert_eq!(
            tick_shield_focus_ai(&input),
            ShieldFocusAiOutput::Focus { facing_index: 1 }
        );
    }

    // ── #783 policy-param equivalence + seeded facts ─────────────────────────

    #[test]
    fn default_focus_policy_params_equal_the_typed_knobs() {
        // Baseline preservation: the canonical default policy seeds its four
        // `param`s from the retained typed `default_shields_ai_*()` values, so a
        // ship that omits `[shields_console.ai_policy]` feeds the kernel exactly
        // the windows/thresholds it always did.
        let policy = crate::entities::config::default_shields_focus_ai_config()
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
        let policy = crate::entities::config::default_shields_focus_ai_config()
            .to_policy()
            .unwrap();
        let p = |name: &str| policy.params.get(name).unwrap() as f32;

        let mut history = empty_history(4);
        history[1].push(DamageRecord {
            timestamp: 1.0,
            amount: 80,
        });
        history[0].push(DamageRecord {
            timestamp: 1.0,
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
            damage_window_secs: p(crate::entities::config::SHIELD_FOCUS_DAMAGE_WINDOW_PARAM),
            min_damage_window_secs: p(
                crate::entities::config::SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM,
            ),
            damage_pct_threshold: p(crate::entities::config::SHIELD_FOCUS_DAMAGE_PCT_PARAM),
            health_ratio_threshold: p(crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM),
            current_time_secs: 4.0,
        };
        let typed_input = make_input(facings, true, history, 4.0);

        assert_eq!(
            tick_shield_focus_ai(&params_input),
            tick_shield_focus_ai(&typed_input)
        );
        assert_eq!(
            tick_shield_focus_ai(&params_input),
            ShieldFocusAiOutput::Focus { facing_index: 1 }
        );
    }

    #[test]
    fn seed_shields_focus_facts_exposes_bounded_per_arc_damage() {
        // AC1: per-arc recent-damage facts computed from the pruned window only.
        // Port (idx 1) took a concentrated hit inside the window; a stale hit on
        // Fore at t=0.0 falls OUTSIDE [current-window, current] and must NOT
        // count (bounded — no unbounded accumulation).
        let mut history = empty_history(4);
        history[1].push(DamageRecord {
            timestamp: 3.0,
            amount: 60,
        });
        history[0].push(DamageRecord {
            timestamp: 3.0,
            amount: 20,
        });
        history[0].push(DamageRecord {
            timestamp: 0.0, // stale: outside the [1.0, 5.0] window
            amount: 999,
        });
        let facings = vec![
            make_snap("Fore", 80, 100, false),
            make_snap("Port", 40, 100, false),
            make_snap("Aft", 100, 100, false),
            make_snap("Starboard", 100, 100, false),
        ];
        // window = max(4.0, 1.0) = 4.0 → [1.0, 5.0].
        let facts = seed_shields_focus_facts(&facings, &history, 4.0, 1.0, 5.0);

        assert_eq!(facts.get("recent_damage_port"), Some(60.0));
        assert_eq!(
            facts.get("recent_damage_fore"),
            Some(20.0),
            "the stale t=0.0 hit on Fore must be excluded from the bounded window"
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
}
