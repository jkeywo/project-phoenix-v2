use crate::core::messages::{AiDirective, ScoredObjective, SystemAffinity};
pub use phoenix_sim_contracts::directive;
pub use phoenix_sim_contracts::objective_utility::*;
/// When something last LANDED a hit on this ship: the more recent of hull
/// damage taken and hostile fire an arc absorbed. The pure fold both `attacked`
/// publish sites reduce a ship's combat activity through before calling
/// [`attacked_recently`], so neither can pick a different set of readings.
///
/// Hull damage alone is not enough. `RecentCombatActivity::last_damage_taken`
/// is written only when the hull TOTAL actually drops
/// (`ship::combat_activity::update_combat_activity`), so fire a shield eats
/// never reaches it — it lands in `last_hostile_fire_taken` instead.
/// `assets/entities/station_axiom.toml` shoots 5 dps in 4 s bursts with no
/// `shield_pierce` at a Harrow's single 90 hp arc regenerating 2/s: the burst
/// (20 dmg) barely outpaces the regen over the same cycle (16 dmg over 8 s),
/// netting only ~4 hp/cycle, so shield-absorbed fire dominates the opening of
/// an engagement and hull damage does not register until sustained pressure
/// collapses the arc (roughly three minutes of continuous fire). Reading
/// damage alone would leave that Harrow flying its raid while the station
/// shot at it for most of a short engagement, which is the behaviour the old
/// per-beam `LastShipAttacker` signal did get right.
///
/// `last_weapon_fired` is deliberately NOT folded in: firing your own guns is
/// not being attacked, and folding it would make a `not_attacked` gate veto
/// itself the moment the hull opened fire and hold the veto for as long as it
/// kept firing. (The captain's `secs_since_combat` red-alert fact DOES fold all
/// three — it asks "is this ship in a fight", which is a different question.)
pub fn last_landed_hit_secs(
    last_damage_taken_secs: Option<f32>,
    last_hostile_fire_taken_secs: Option<f32>,
) -> Option<f32> {
    match (last_damage_taken_secs, last_hostile_fire_taken_secs) {
        (Some(damage), Some(fire)) => Some(damage.max(fire)),
        (Some(damage), None) => Some(damage),
        (None, fire) => fire,
    }
}

/// Whether a hit landed recently enough that the ship still counts as under
/// attack (issue #1010). Take `last_hit_secs` from [`last_landed_hit_secs`] —
/// a hit that connected, shields or hull.
///
/// `attacked` used to read the `LastShipAttacker` latch, which is set on the
/// first beam that connects and cleared only when the ship dies or when its red
/// alert stands down (`server_app::clear_last_attacker_on_red_alert_off`).
/// Every Harrow hull DOES author a captain stand-down —
/// `combat_window_secs = 10.0` in `ship_harrow_cruiser.toml` — so the latch is
/// releasable in principle. What it is not is releasable DURING a fight: the
/// captain's `secs_since_combat` fact folds the hull's OWN weapon fire in
/// alongside damage and hostile fire, so a Harrow returning fire keeps resetting
/// its own stand-down clock, red alert never drops, and the latch never clears.
/// (Hulls that author an alert-on-hostile rule hold the alert up on mere contact
/// as well — `alliance_courier.toml`'s priority-5 rule; a Harrow authors none.)
/// So with a player ship loitering nearby, `combat_test.toml`'s
/// `not_attacked`-gated `assault-starbase` stayed retired for as long as the
/// loitering lasted — the raid the scenario is named for never resumed, which
/// is what the playtest saw.
///
/// Recency decays instead: the gate closes on the landed hit and reopens once
/// `window_secs` of simulation time pass with no further one. Both times are
/// `Time::elapsed_secs()` read inside `FixedUpdate` — SIM seconds off the fixed
/// clock, never a wall clock (AGENTS.md #7) — and `window_secs` is authored as
/// `[global] attacked_memory_secs`.
///
/// The two windows are separate on purpose. The per-hull captain
/// `combat_window_secs` governs ALERT POSTURE; the global `attacked_memory_secs`
/// governs this doctrine gate directly, which is what decouples resuming a raid
/// from the red-alert/`LastShipAttacker` chain that could not release while the
/// shooting continued.
///
/// A ship nothing has hit (`None`) is not under attack. A non-positive window
/// degenerates to "never under attack", which is the honest reading of a
/// designer authoring a zero-length memory.
pub fn attacked_recently(last_hit_secs: Option<f32>, now_secs: f32, window_secs: f32) -> bool {
    match last_hit_secs {
        Some(last) => now_secs - last < window_secs,
        None => false,
    }
}

/// The top-scored ACTIVE directive relevant to `affinity` whose kind `wanted`
/// accepts, from a viewscreen scored-objective pool (issue #1162).
///
/// The pool is already sorted descending by score (see `scored_pool`), so the
/// first match is the top one. Pure and Bevy-free (AGENTS.md rule 10), so the
/// four backfill operate hosts — tractor, umbilical, dock and external repair —
/// share ONE selection rule and it can be unit-tested without an `App`. A
/// zero-score objective is skipped exactly as the other AI-facing consumers skip
/// it, so a boosted or condition-changed directive re-activates without the pool
/// being republished.
pub fn top_operate_directive(
    scored: &[ScoredObjective],
    affinity: SystemAffinity,
    wanted: impl Fn(&AiDirective) -> bool,
) -> Option<&AiDirective> {
    scored.iter().find_map(|o| {
        (o.score > 0.0 && o.relevance.contains(&affinity) && wanted(&o.directive))
            .then_some(&o.directive)
    })
}

/// The target a tractor operate directive (`Tow`/`Stabilise`/`Escort`) names, or
/// `None` for any other directive (issue #1162). The tractor host's `wanted`
/// predicate, factored out so the host and its tests read one rule.
pub fn tractor_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Tow { target }
        | AiDirective::Stabilise { target }
        | AiDirective::Escort { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target a `Rescue` directive names, or `None` for any other directive
/// (issue #1348). The transporter host's own-kind test, factored out so the host
/// and its tests read one rule. The tractor-versus-rescue precedence is settled
/// through [`engineering_seat_operate_target`] (the shared seat), not here.
pub fn rescue_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Rescue { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target of any directive that occupies the single Engineering backfill
/// seat — a tractor verb (`Tow`/`Stabilise`/`Escort`) or the transporter's
/// `Rescue` — or `None` for anything else (issue #1348).
///
/// The tractor and the rescue transporter are distinct systems but share one
/// Engineering seat: one pair of hands. Both backfill hosts pass THIS predicate
/// to [`top_operate_directive`], so they rank the tractor and rescue orders
/// against the ONE scored pool and the seat resolves to a single winner. Each
/// host then keeps only its own kind of that winner (via
/// [`tractor_directive_target`] / [`rescue_directive_target`]): when a tractor
/// obligation outscores a rescue the transporter host sees `None` and stands
/// down, and when a rescue outscores the tractor the tractor host stands down —
/// so a higher-scored life-saving stabilisation defers the rescue exactly as the
/// acceptance criterion requires, and neither ever runs while the other holds the
/// seat.
pub fn engineering_seat_operate_target(directive: &AiDirective) -> Option<&str> {
    tractor_directive_target(directive).or_else(|| rescue_directive_target(directive))
}

/// The target a `Transfer` directive names, or `None` (issue #1162). Shared by
/// the Helm dock host and the Engineering umbilical host — the two seats of the
/// resupply chain.
pub fn transfer_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Transfer { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target a `FieldRepair` directive names, or `None` (issue #1162). The
/// external-repair dispatch host's predicate.
pub fn field_repair_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::FieldRepair { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The target a `Secure` directive names, or `None` (issue #1346). The Security
/// backfill host's predicate: a target it names has its authored Security work
/// promoted to `urgent_objective` in the host's ranking.
pub fn secure_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Secure { target } => Some(target.as_str()),
        _ => None,
    }
}

/// The civilian and route named by an `Order` directive, or `None` for any
/// other directive (issue #1141). Kept pure so authoring, selection and the
/// Navigation host share the same payload projection.
pub fn order_directive(directive: &AiDirective) -> Option<(&str, &str)> {
    match directive {
        AiDirective::Order { target, route } => Some((target.as_str(), route.as_str())),
        _ => None,
    }
}

/// The target a `Scan` directive names, or `None` for another directive
/// (issue #1139). Shared by the Sensors host and its pure selection tests.
pub fn scan_directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Scan { target } => Some(target.as_str()),
        _ => None,
    }
}
