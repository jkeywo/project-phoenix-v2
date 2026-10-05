//! Pure AI decision functions for console automation.
//!
//! This module is Bevy-free and platform-agnostic. It contains decision
//! functions that replicate the actions a human player would take on a
//! console set to "Low" complexity.
//!
//! The Bevy orchestrator lives in `console_ai_plugin`.

use crate::ship::shields::DamageRecord;
use crate::weapons::shield::ShieldFacingSnapshot;
use crate::weapons::torpedo::TorpedoTubeId;

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
    /// strike — not the target's shield pool.
    ///
    /// Reported for the benefit of the AUTHORED launch guard, never read here:
    /// since issue #956 the "fire only when this is ≤ 0" decision belongs to the
    /// tube's own `torpedo_launch` predicate, which the host resolves over a
    /// snapshot seeded from this value. See [`auto_fire_torpedo`].
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

/// Decide which torpedo tubes are *candidates* to fire this tick.
///
/// Candidate conditions (ALL must hold):
/// - A target is locked
/// - The tube is loaded
/// - The tube is in arc
///
/// # Why the striking arc's shields are NOT one of them (issue #956)
///
/// They used to be: this function opened with
/// `if !target_locked || target_facing_shields > 0 { return vec![] }`, and that
/// second clause is the entire "phasers strip the shields, torpedoes finish the
/// hull" doctrine, written in Rust, unconditional, and UPSTREAM of every
/// authored policy. A tube's `torpedo_launch` predicate is resolved after this
/// returns, so it could only ever narrow the gate further (AND); no doctrine
/// could authorise a torpedo while the striking arc was up, and none could drop
/// the gate for a hull that wanted to spend rounds differently.
///
/// The gate itself has not gone anywhere — it moved into the content that was
/// always the right home for it. Armed tubes author
/// `fact(target_facing_shields) <= 0` in their own `torpedo_launch` guard, which
/// is the same text the Harrow tubes already carried, over the same
/// host-resolved per-arc HP reading (`seed_torpedo_tube_launch_facts`). What
/// changed is that it is now a threshold a designer can retune — or a doctrine
/// can decline — rather than a constant compiled into the decision.
///
/// **Every armed tube in the fleet does currently author `<= 0`, and that is a
/// fact about CONTENT rather than an invariant this function may assume
/// (issue #929).** It has already been false once. #929's first pass had
/// `alliance_cruiser.toml`'s three tubes compare against a
/// `param(max_striking_shield_hp)` authored past any arc reading, switching the
/// conjunct off, because at `beam_damage_per_sec = 4` that hull could not reach
/// the fleet reading against a self-focusing warship — and its helm's
/// `torpedo_run` bow hold exits only on a round having LEFT a tube, so an
/// unreachable launch gate turned an authored leg into a sink. #929's second pass
/// restored the fleet text there and paid for it on the guns instead (32 dmg/s a
/// bank since the third pass, across two that both bear), which it could do
/// because a round no longer bypasses a raised arc: the gate is now worth ten
/// times the round it delays. Either way the point for a
/// reader HERE is the narrow one — which tubes gate on the striking arc, and
/// against what threshold, is per-hull content, and this function reads none of
/// it. `authored_ai_pins::a_bow_hold_a_hull_can_reach_and_a_launcher_that_can_answer_it`
/// is the census that keeps the leg/launcher pairing honest, and
/// `torpedo_launch_shield_gate_truth_table` enumerates who authors what.
///
/// `target_facing_shields` stays on [`TorpedoAiInput`] because the host seeds
/// the fact from it; this function simply no longer reads it.
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
/// # Why the shield reading the authored gate uses is per-arc, not ship-wide
///
/// The doctrine is "phasers strip the shields, torpedoes finish the hull", and
/// on a single-arc hull those are the same test. On a four-arc hull they are
/// not: summing every arc lets three healthy REAR arcs veto a shot into a
/// collapsed FRONT arc while the attacker is sitting dead ahead — exactly where
/// the hull is exposed and where the torpedo would land. Since every arc regens
/// independently and goes offline for only a few seconds, a multi-arc ship
/// essentially never has all arcs down at once, so the summed gate meant AI
/// crews on four-arc hulls never launched a torpedo at all. The reading the
/// authored guard compares against therefore asks about the *one* arc the shot
/// would hit — `ai_torpedo_auto_fire` resolves it through the target's own arc
/// router before seeding it.
///
/// Returns a list of `TorpedoTubeId` values in deterministic priority order
/// `[ForePort, ForeStarboard, Aft]`. Each tube that passes all conditions
/// appears at most once in the result.
pub fn auto_fire_torpedo(input: &TorpedoAiInput) -> Vec<TorpedoTubeId> {
    if !input.target_locked {
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

impl FrequencyMatchState {
    /// Read the exact delayed-match continuation for snapshot projection.
    pub fn continuation(&self) -> (Option<&str>, f32, bool) {
        (
            self.current_target.as_deref(),
            self.elapsed_secs,
            self.match_sent,
        )
    }

    /// Rebuild the delayed-match continuation after the snapshot layer has
    /// resolved its target identity and validated its scalar fields.
    pub fn from_continuation(
        current_target: Option<String>,
        elapsed_secs: f32,
        match_sent: bool,
    ) -> Self {
        Self {
            current_target,
            elapsed_secs,
            match_sent,
        }
    }
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
    /// Records outside the effective tick window should be pruned by the caller.
    pub damage_history: Vec<Vec<DamageRecord>>,
    /// Authored damage-history window converted to whole simulation ticks.
    pub damage_window_ticks: u64,
    /// Authored minimum reaction window converted to whole simulation ticks.
    pub min_damage_window_ticks: u64,
    /// Percentage threshold (0.0–100.0): if an arc receives this fraction of
    /// total damage in the active window, focus it.
    pub damage_pct_threshold: f32,
    /// Percentage threshold (0.0–100.0): if the lowest-arc normalized health
    /// is below this fraction of the second-lowest, focus the weakest arc.
    pub health_ratio_threshold: f32,
    /// Current authoritative logical simulation tick.
    pub current_tick: u64,
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
///    authored recent-damage window, where a record is recent iff
///    `current_tick - recorded_tick < window_ticks` and `window_ticks` is
///    `max(damage_window_ticks, min_damage_window_ticks)`. The caller prunes
///    using that identical strict boundary. If any arc
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
    // Prune is done by the caller (ai_shield_focus prunes before building input).
    // Concentration is measured over the AUTHORED recent-damage window
    // (`damage_window_ticks`), floored at `min_damage_window_ticks` so a
    // misconfigured window can never fall below the reaction minimum. The
    // authored window — not a fixed last-`min_damage_window_ticks` slice — is
    // what "recent concentrated damage over authored windows" (issue #747)
    // means.
    let window_ticks = input.damage_window_ticks.max(input.min_damage_window_ticks);

    let mut damage_per_arc: Vec<i32> = vec![0; n];
    let mut total_window_damage: i32 = 0;

    for (idx, records) in input.damage_history.iter().enumerate() {
        if idx >= n {
            break;
        }
        for record in records {
            if crate::ship::shields::damage_record_is_recent(
                record.recorded_tick,
                input.current_tick,
                window_ticks,
            ) {
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
/// is computed from the ALREADY-PRUNED window the caller built (records whose
/// tick age reaches the effective window are gone before this runs) — "bounded" means we read
/// only that window and add NO new unbounded accumulator. The window matches the
/// kernel's concentration window exactly (`max(damage_window_ticks,
/// min_damage_window_ticks)`), so the facts describe the same slice the retained
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
    facings: &[crate::weapons::shield::ShieldFacingSnapshot],
    damage_history: &[Vec<DamageRecord>],
    damage_window_ticks: u64,
    min_damage_window_ticks: u64,
    current_tick: u64,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();

    // Same window the kernel measures concentration over.
    let window_ticks = damage_window_ticks.max(min_damage_window_ticks);

    let mut total: i32 = 0;
    let mut max_arc: i32 = 0;
    for (idx, facing) in facings.iter().enumerate() {
        let arc_sum: i32 = damage_history
            .get(idx)
            .map(|records| {
                records
                    .iter()
                    .filter(|record| {
                        crate::ship::shields::damage_record_is_recent(
                            record.recorded_tick,
                            current_tick,
                            window_ticks,
                        )
                    })
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

    facts.set_fact(fid::RECENT_DAMAGE_TOTAL, total as f64);
    let fraction_max = if total > 0 {
        max_arc as f64 / total as f64
    } else {
        0.0
    };
    facts.set_fact(fid::RECENT_DAMAGE_FRACTION_MAX, fraction_max);
    facts.set_fact(fid::RECENT_DAMAGE_PCT_MAX, fraction_max * 100.0);

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
        facts.set_fact(fid::HEALTH_FRACTION_MIN_RATIO, ratio as f64);
        facts.set_fact(fid::HEALTH_RATIO_PCT, (ratio * 100.0) as f64);
    }

    facts
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
