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
    /// Torpedoes remaining in the magazine — the rounds still available to
    /// RELOAD with, reported for the benefit of callers and policies that care
    /// about resupply.
    ///
    /// It deliberately does NOT gate firing. A round sitting in a tube was drawn
    /// from the magazine when its load *started* (`TorpedoSystem::start_load`
    /// decrements there), so the magazine counts what is left to load NEXT, not
    /// what can be fired NOW — and a hull whose last rounds are in its tubes has
    /// a magazine of zero and a battery ready to launch. See
    /// [`auto_fire_torpedo`].
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
///
/// # Why the magazine is not one of them
///
/// A round in a tube has *already been paid for*: `TorpedoSystem::start_load`
/// (and the auto-load block in `TorpedoSystem::tick`) decrements
/// `torpedoes_remaining` when a load STARTS, so the magazine counts the rounds
/// left to reload with and says nothing about the rounds a tube is holding. An
/// `input.magazine == 0` conjunct here therefore refused to fire a fully loaded
/// battery whenever the hold happened to be empty — and on any hull whose
/// magazine divides evenly into its salvo, that is the state it ends in.
///
/// The Harrow cruiser is the deterministic case: 8 rounds, a 4-round battery.
/// Load 4 (magazine 4), fire, reload the last 4 (magazine 0) — and from that
/// moment a full battery could never launch again, while `tubes_full` read
/// permanently true. The helm doctrine's salvo-spent resume conjoins
/// `fact(tubes_full) < 1`, so it could never fire either: the hull sat bow-on
/// with a loaded battery it would not shoot, bounded only by the target
/// recovering a shield — precisely the dependency that bound exists to remove.
///
/// Nothing downstream re-imposes the gate, and deliberately so:
/// `handle_fire_torpedo` gates on the tube's and magazine's *online* state, and
/// `TorpedoSystem::launch` on `loaded_count`. Emptiness stops the next LOAD
/// (`start_load` and `claim_magazine_round` both refuse at zero), which is where
/// running dry belongs. This is shared mechanics: it applies to a player's ship
/// exactly as it does to an NPC's (AGENTS.md #6).
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
    if !input.target_locked || input.target_facing_shields > 0 {
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
