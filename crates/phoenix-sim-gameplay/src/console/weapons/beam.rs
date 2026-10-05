use crate::core::messages::PhaserBank;
use bevy::prelude::*;

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own weapons target.
#[derive(Component, Default, Clone, Debug)]
pub struct TacticalRadarSelection(pub Option<String>);

/// Per-ship resolved Tactical target selector (issue #777).
///
/// The Tactical mirror of [`crate::ship::sensors::SensorsTargetSelector`]:
/// holds the ship's data-driven [`crate::ai::selector::TargetSelector`], decoded
/// from the authored `[weapons_console.selector]` block, plus the authored ship
/// `power_rating`, which `ai_target_selection` exposes to the selector's
/// expressions as `self_fact(power_rating)`.
///
/// Attached at spawn on every ship (NPC and player) alongside
/// `SensorsTargetSelector`. Since #885b stage 5d there is no Rust-side
/// synthesised default behind it: a ship without the component ranks nothing and
/// `ai_target_selection` skips it. The selector RANKS candidates; it never
/// writes authoritative state — the host applies the chosen UUID to
/// `TacticalRadarSelection` directly, keeping Tactical the sole writer (AC4).
#[derive(Component, Clone, Debug)]
pub struct TacticalTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
    /// Explicit Tactical-radar idle declaration (issue #781, AC6). When `true`
    /// the radar takes NO AI target selection: `ai_target_selection` clears any
    /// stale lock and skips the ship even when a tactical fine system is
    /// AI-operated. Seeded from `[weapons_console] selector_idle`. This is the
    /// explicit AI-or-idle opt-out that distinguishes "the radar deliberately
    /// makes no AI selection" from "no selector authored → default selector".
    pub idle: bool,
}

/// UUID of the last entity that attacked this ship. Written by the unified
/// `tick_beams` in the Damage phase on the targeted ship's entity;
/// consumed by that ship's `ai_target_selection` as a fallback target.
/// `None` when no recent attacker is known.
///
/// Per-ship `Component` — every ship (player + NPC) tracks its own attacker.
///
/// # Change detection is load-bearing (issue #702)
///
/// This is the single "who last attacked me" surface, and its change detection
/// is the rising-edge latch that fires `AiEntityAttacked` — which in turn drives
/// `on_entity_attacked` scenario triggers. Every writer must therefore
/// **compare before writing** (`set_if_neq`), or sustained fire from one shooter
/// re-fires the trigger every tick the beam is live. `PartialEq` exists for
/// exactly that reason; do not replace it with a blind assignment.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct LastShipAttacker(pub Option<String>);

/// One live beam: what a single phaser bank is currently burning at.
///
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
/// The slot's mere existence in [`ActiveBeam`] means "this bank is firing" —
/// there is no `Option<String>` target, because a beam with no target is not a
/// beam.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActiveBeamSlot {
    pub strike_damage_bonus: f32,
    pub target_uuid: String,
    pub remaining_secs: f32,
    pub damage_accumulator: f32,
    /// The cooldown this cycle has already BOUGHT, in seconds (issue #929).
    ///
    /// Written when the bank lights and read by every site that ends the beam,
    /// instead of each of them re-deriving `cooldown_secs` from the bank config.
    /// That is what makes `cycle_jitter` LINKED: one draw produces the firing
    /// duration and the cooldown together, here, and there is no later moment at
    /// which the two could disagree or a second draw could be taken.
    ///
    /// It also makes the jitter resume-safe for free. Both numbers are ordinary
    /// slot state on a component the snapshot already captures, so a run
    /// restored mid-cycle continues the cycle it was in — same remaining burn,
    /// same cooldown waiting behind it — rather than redrawing. A slot restored
    /// from a pre-#929 payload reads `0.0` here, which every end site treats as
    /// "no jittered cooldown was recorded" and falls back to the authored value.
    pub pending_cooldown_secs: f32,
}

/// Active phaser beam state, tracked independently **per bank** (issue #790).
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own beam state.
///
/// After issue #846, phaser fire commands arrive as admitted `ControlSystem`
/// payloads rather than through `PhaserIntents` — the `#[require]` was deleted
/// alongside the intents component.
///
/// ## Why per-bank, and why it is not a cruiser feature
///
/// Until issue #790 this was ONE slot per ship: a single `(target, bank)` pair,
/// with `handle_fire_phaser` refusing any fire while it was occupied and
/// `ai_phaser_auto_fire` picking exactly one bank per tick. That made
/// overlapping fire arcs unrepresentable — a hull whose fore and aft banks each
/// sweep 270° has both bearing on anything abeam, and could still only light
/// one of them.
///
/// The shape is [`PhaserCooldown`]'s, deliberately: per-bank state that already
/// worked keyed by the same `PhaserBankConfig.id` used everywhere else. Nothing
/// here branches on who owns the hull (AGENTS.md #6) — how many banks bear is
/// decided entirely by the arcs a hull authors, and the two gates are separate
/// fields: `handle_fire_phaser` reads `fire_arc_deg`, `ai_phaser_auto_fire` reads
/// `auto_arc_deg`. The player's `alliance_cruiser` authored 270 and 180
/// respectively until issue #929 and so double-broadsided on the MANUAL path
/// only; it now authors 270 on both and double-broadsides on either.
/// `ship_harrow_cruiser` has always authored 270 on both. `alliance_battleship`
/// is the remaining split case, with abutting 180-degree auto arcs.
///
/// ## Why `BTreeMap` and not `HashMap`
///
/// [`PhaserCooldown`] can use a `HashMap` because nothing ever *iterates* it in
/// an order-sensitive way — it is a lookup plus an order-free decay. This map is
/// iterated to build the per-tick shooter snapshots that drive damage
/// application, and damage draws from the shared seeded RNG stream. `HashMap`'s
/// iteration order is randomised per process, so it would make two runs of the
/// same seeded scenario diverge. A `BTreeMap` orders by bank id, which is
/// authored content and therefore stable.
#[derive(Component, Default, Clone, Debug)]
pub struct ActiveBeam {
    per_bank: std::collections::BTreeMap<PhaserBank, ActiveBeamSlot>,
}

impl ActiveBeam {
    /// Is ANY bank burning? The ship-level "phasers are firing" question — HUD
    /// state, power drain gating, observability.
    pub fn is_firing(&self) -> bool {
        !self.per_bank.is_empty()
    }

    /// Is this specific bank burning? The gate every firing path uses now: one
    /// bank's live beam must never block another's.
    pub fn is_bank_firing(&self, bank: &str) -> bool {
        self.per_bank.contains_key(bank)
    }

    /// What this bank is burning at, if anything.
    pub fn bank_target(&self, bank: &str) -> Option<&str> {
        self.per_bank.get(bank).map(|s| s.target_uuid.as_str())
    }

    /// The lowest-keyed live bank's target — for the handful of single-value
    /// observability surfaces (test helpers, the legacy no-banks path) that ask
    /// "what is this ship shooting at" rather than "what is each bank doing".
    /// Deterministic because the map is ordered.
    pub fn any_target(&self) -> Option<&str> {
        self.per_bank
            .values()
            .next()
            .map(|s| s.target_uuid.as_str())
    }

    /// The lowest-keyed live bank's id. Same caveat as [`Self::any_target`].
    pub fn any_bank(&self) -> Option<&str> {
        self.per_bank.keys().next().map(|b| b.as_str())
    }

    /// Every live `(bank, slot)` in authored-id order.
    pub fn live_banks(&self) -> impl Iterator<Item = (&PhaserBank, &ActiveBeamSlot)> {
        self.per_bank.iter()
    }

    /// How much longer this bank's beam burns, seconds. `0.0` when it is not
    /// firing — the same shape [`PhaserCooldown::bank_remaining_secs`] uses for
    /// its own per-bank map.
    pub fn bank_remaining_secs(&self, bank: &str) -> f32 {
        self.per_bank
            .get(bank)
            .map(|s| s.remaining_secs)
            .unwrap_or(0.0)
    }

    /// Light `bank` at `target_uuid` for `duration_secs`, with
    /// `pending_cooldown_secs` the rest this cycle has bought. Replaces any beam
    /// already on that bank (and only that bank).
    ///
    /// The two durations arrive together because they are one decision — see
    /// [`ActiveBeamSlot::pending_cooldown_secs`].
    pub fn start(
        &mut self,
        bank: impl Into<PhaserBank>,
        target_uuid: impl Into<String>,
        duration_secs: f32,
        pending_cooldown_secs: f32,
    ) {
        self.per_bank.insert(
            bank.into(),
            ActiveBeamSlot {
                target_uuid: target_uuid.into(),
                remaining_secs: duration_secs,
                damage_accumulator: 0.0,
                pending_cooldown_secs,
                strike_damage_bonus: 0.0,
            },
        );
    }

    /// The cooldown `bank`'s current cycle bought, or `None` when it is not
    /// burning or the slot predates issue #929's paired draw.
    pub fn bank_pending_cooldown(&self, bank: &str) -> Option<f32> {
        self.per_bank
            .get(bank)
            .map(|s| s.pending_cooldown_secs)
            .filter(|secs| *secs > 0.0)
    }

    /// Extinguish `bank`, returning the slot it was burning (if any).
    pub fn end_bank(&mut self, bank: &str) -> Option<ActiveBeamSlot> {
        self.per_bank.remove(bank)
    }

    /// Complete an ordinary cycle, serving its recorded cooldown. Zero in a
    /// legacy snapshot falls back to the authored cooldown supplied by the host.
    /// Relight cleanup deliberately continues to use `end_bank` directly.
    pub fn complete_bank(
        &mut self,
        bank: &str,
        cooldown: &mut PhaserCooldown,
        fallback_secs: f32,
    ) -> Option<ActiveBeamSlot> {
        let slot = self.end_bank(bank)?;
        let served = if slot.pending_cooldown_secs > 0.0 {
            slot.pending_cooldown_secs
        } else {
            fallback_secs
        };
        cooldown.start_bank(bank, served);
        Some(slot)
    }

    /// Mutable access to one bank's live slot, for the per-tick damage and
    /// lifetime folds.
    pub fn bank_slot_mut(&mut self, bank: &str) -> Option<&mut ActiveBeamSlot> {
        self.per_bank.get_mut(bank)
    }

    /// Replace every live beam wholesale (issue #862's snapshot restore).
    ///
    /// Deliberately not expressible as a sequence of [`Self::start`] calls:
    /// `start` zeroes `damage_accumulator`, which is the fractional damage
    /// carried between ticks, so a restore built out of `start` would put every
    /// live beam back mid-burn but with its sub-tick debt forgiven — a small,
    /// silent, per-beam divergence one tick after a restore whose digest
    /// matched. Restoring is not firing, and it does not go through the firing
    /// door.
    pub fn restore_live_banks(
        &mut self,
        banks: impl IntoIterator<Item = (PhaserBank, ActiveBeamSlot)>,
    ) {
        self.per_bank = banks.into_iter().collect();
    }
}

/// Post-beam cooldown, tracked independently per phaser bank.
/// The weapons console rejects a fire request for a specific bank while
/// that bank's cooldown is active; other banks remain unaffected.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own cooldowns.
#[derive(Component, Default, Clone, Debug)]
pub struct PhaserCooldown {
    pub per_bank: std::collections::HashMap<String, f32>,
}

impl PhaserCooldown {
    pub fn is_bank_active(&self, bank: &str) -> bool {
        self.per_bank.get(bank).copied().unwrap_or(0.0) > 0.0
    }

    pub fn bank_remaining_secs(&self, bank: &str) -> f32 {
        self.per_bank.get(bank).copied().unwrap_or(0.0)
    }

    pub fn start_bank(&mut self, bank: &str, cooldown_secs: f32) {
        self.per_bank.insert(bank.to_string(), cooldown_secs);
    }

    pub fn start_bank_with_cooldown(&mut self, bank: &str, secs: f32) {
        self.per_bank.insert(bank.to_string(), secs);
    }

    pub fn tick(&mut self, dt: f32) {
        for v in self.per_bank.values_mut() {
            *v = (*v - dt).max(0.0);
        }
    }

    /// Every bank with time still on it, in bank-id order.
    ///
    /// Sorted rather than raw `HashMap` order because the one reader is issue
    /// #862's snapshot capture, and a payload must no more inherit a map's
    /// iteration order than the digest may.
    pub fn active_banks_sorted(&self) -> Vec<(String, f32)> {
        let mut rows: Vec<(String, f32)> = self
            .per_bank
            .iter()
            .filter(|(_, secs)| **secs > 0.0)
            .map(|(bank, secs)| (bank.clone(), *secs))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Replace every bank's remaining cooldown wholesale (snapshot restore).
    pub fn restore_banks(&mut self, banks: impl IntoIterator<Item = (String, f32)>) {
        self.per_bank = banks.into_iter().collect();
    }
}

/// Current phaser firing mode (Auto or Manual), set by the Weapons console.
#[derive(Resource)]
pub struct CurrentPhaserMode(pub crate::core::messages::PhaserMode);

impl Default for CurrentPhaserMode {
    fn default() -> Self {
        Self(crate::core::messages::PhaserMode::Manual)
    }
}

/// Bevy resource holding the player-ship phaser combat tuning
/// (beam duration, beam cooldown, beam damage per second, phaser range).
///
/// Seeded with `PhaserCombatConfig::default()` (the historical
/// hardcoded values) by `WeaponsPlugin::build`, and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block. Read by `handle_fire_phaser`, `tick_beams`, and the
/// `weapons_update_broadcaster` to drive player phaser behaviour.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Default, Clone)]
pub struct PhaserCombatConfigResource(pub crate::entities::config::PhaserCombatConfig);

/// Per-ship map of each phaser bank's inline stateless open-fire policy
/// (issue #781), keyed by the same bank id used everywhere else
/// (`PhaserBankConfig.id`). Built at spawn from each bank's authored `ai` block,
/// falling back to the canonical
/// [`crate::entities::config::default_phaser_bank_ai_config`] (unconditional
/// fire) so a bank without an authored policy keeps auto-firing exactly as
/// before (AC1 baseline preservation).
///
/// Read by [`ai_phaser_auto_fire`]: for each candidate bank the host seeds a
/// per-bank readiness fact snapshot ([`seed_phaser_bank_facts`]) and resolves the
/// bank's policy on the `phaser_fire` channel; only a bank whose policy fires is
/// selected. A bank with no entry falls back to the default policy, so the map
/// being absent (bare-`App` fixtures) means "every bank fires unconditionally".
#[derive(Component, Default, Clone, Debug)]
pub struct PhaserBankAiPolicies(
    pub  std::collections::HashMap<
        crate::entities::config::PhaserBankId,
        crate::ai::policy::AiPolicy,
    >,
);

/// Seed the per-tick policy fact snapshot for one phaser bank's open-fire
/// decision (issue #781), modelled on
/// [`crate::ship::helm_ai::seed_helm_actuator_facts`]. This is THE piece that
/// closes the #779 empty-facts sharp edge for weapon banks: without seeding, a
/// `fact(...)` guard validates but never fires. The host has already resolved the
/// bank's live readiness (target lock, cooldown, range, arc, frequency) before
/// calling this, so the policy evaluates over the real per-bank state while
/// `policy.rs` stays Bevy-free (AGENTS.md #10).
/// `red_alert` is the SHIP-WIDE reading added by issue #872: this ship's own
/// [`crate::ship::state::ShipRedAlert`], the same per-entity state the captain's
/// `SetRedAlert` command writes for human and AI captains alike. It is seeded
/// unconditionally (a ship with no component reads `0.0`, not absent) so an
/// authored guard can read it in BOTH directions rather than only failing
/// closed. Nothing in this file tests it — the fire gate is an authored
/// predicate on the bank, never a Rust rule.
///
/// Since issue #1041 the reading is the bank's whole firing
/// [`WeaponsAlertPosture`] rather than the bare alert: a bank that cannot shoot
/// — its authored power group switched off (issue #1396), or the captain's hold
/// called (retiring in #1398) — seeds a value below every authorable
/// `min_alert_to_fire` floor, and a live one seeds exactly the `1.0`/`0.0` this
/// line always did. See that type for why restraint rides the existing fact
/// instead of adding a second one.
#[allow(clippy::too_many_arguments)]
pub fn seed_phaser_bank_facts(
    target_valid: bool,
    on_cooldown: bool,
    cooldown_remaining: f32,
    in_range: bool,
    in_arc: bool,
    frequency: f32,
    posture: super::WeaponsAlertPosture,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::TARGET_VALID, if target_valid { 1.0 } else { 0.0 });
    facts.set_fact(fid::ON_COOLDOWN, if on_cooldown { 1.0 } else { 0.0 });
    facts.set_fact(fid::COOLDOWN_REMAINING, cooldown_remaining as f64);
    facts.set_fact(fid::IN_RANGE, if in_range { 1.0 } else { 0.0 });
    facts.set_fact(fid::IN_ARC, if in_arc { 1.0 } else { 0.0 });
    facts.set_fact(fid::FREQUENCY, frequency as f64);
    facts.set_fact(fid::RED_ALERT, posture.alert_fact_value());
    facts
}

/// Resolve a phaser bank's policy to a bare "open fire this tick?" boolean
/// (issue #781), the weapon-bank twin of `helm_policy_actuates`. The policy is a
/// pure fact→verb map: it returns `FirePhaser` when a guard fires and `None`
/// ("hold") otherwise. A mismatched verb resolves to "hold" defensively.
pub fn phaser_bank_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(crate::entities::config::PHASER_FIRE_CHANNEL, facts, flags)
        == Some(&crate::ai::policy::AiPolicyVerb::FirePhaser)
}

#[derive(Event, Clone, Debug)]
pub struct BeamStartedEvent {
    pub bank: PhaserBank,
    pub target_uuid: String,
    /// The ship entity that fired the beam. Used by the observer to set the
    /// `WeaponFiredThisTick` component on the correct ship.
    pub source_entity: Entity,
}

#[derive(Event, Clone, Debug)]
pub struct BeamEndedEvent {
    pub bank: PhaserBank,
    pub target_uuid: String,
    /// The ship entity that fired the beam.
    pub source_entity: Entity,
}

#[cfg(test)]
#[path = "beam_tests.rs"]
mod tests;
