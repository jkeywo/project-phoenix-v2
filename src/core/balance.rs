//! Balance telemetry — structured facts about what the simulation did to whom.
//!
//! `OutboundMessage` already gives a headless run the player-facing wire
//! traffic, but that stream is deliberately player-shaped: `DamageTaken` only
//! fires for the `LocalShip`, and nothing on the wire says who pulled the
//! trigger. Balance work needs the other view — every hit, every shot, every
//! knockout, on every ship, attributed to an attacker — so it gets its own
//! message rather than a wider `ServerMessage`.
//!
//! Two rules keep this honest:
//!
//! 1. **Emitted unconditionally.** The chokepoints write a [`BalanceEvent`]
//!    next to the state mutation, outside any `is_local` gate and in every
//!    build. A tracer that only fires for the player ship would report exactly
//!    the half of a fight that is already visible.
//! 2. **Aggregation is pure.** [`aggregate_ledgers`] turns a stamped event log
//!    into per-ship ledgers with no ECS access at all, so the reporting logic
//!    is unit-testable without booting an app.

use bevy::prelude::*;
use std::collections::BTreeMap;

/// Weapon-kind labels for damage that does not come from a configured bank.
/// Bank-sourced damage (beam, blaster, torpedo) uses the bank/tube id from
/// TOML instead, so these are the only fixed ones.
pub const WEAPON_KIND_COLLISION: &str = "collision";
pub const WEAPON_KIND_REGION: &str = "region";

/// Fired-weapon family labels for [`BalanceEvent::WeaponFired`]. The `weapon`
/// field on that variant names the specific bank/tube; `kind` groups it into
/// one of these families so a reader can split shots by weapon type without
/// re-deriving the family from the id.
pub const FIRED_KIND_BEAM: &str = "beam";
pub const FIRED_KIND_TORPEDO: &str = "torpedo";
pub const FIRED_KIND_BLASTER: &str = "blaster";

/// What kind of thing took a hit.
///
/// Mining an asteroid is a real event worth seeing in the timeline, but it is
/// not combat: folding it into the per-ship ledgers would credit a shooter
/// with `damage_dealt` for shooting a rock. The discriminator lets emission
/// stay unconditional while aggregation stays ship-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VictimKind {
    /// A ship, station, or any other entity with an `EntityUuid`.
    Ship,
    /// An asteroid — telemetry only, never counted in the ledgers.
    Asteroid,
}

impl VictimKind {
    /// Lowercase label for the ndjson timeline.
    pub fn as_str(self) -> &'static str {
        match self {
            VictimKind::Ship => "ship",
            VictimKind::Asteroid => "asteroid",
        }
    }
}

/// Which side prevailed when a run reaches `GamePhase::GameOver`.
///
/// The engine's only structural end signal is `final_phase`
/// (`InProgress`/`GameOver`) plus the free-form `GameOverReason` string, and
/// that string is per-world — in `combat_test` it is even a strings.csv key, so
/// no substring reliably tells a victory from a defeat. Rather than string-match
/// (fragile and per-world), scenario authors *declare* the outcome on the
/// `game_over` trigger action, and the built-in player-death sites latch
/// [`Outcome::Defeat`]. The classifier reads this flag instead of guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Victory,
    Defeat,
}

impl Outcome {
    /// Parse the author-facing `outcome = "..."` field (case-insensitive).
    /// `Err` on any other value so a typo fails the world parse loudly rather
    /// than silently defaulting to the wrong side.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "victory" => Ok(Outcome::Victory),
            "defeat" => Ok(Outcome::Defeat),
            other => Err(format!(
                "unknown game_over outcome '{other}' (expected 'victory' or 'defeat')"
            )),
        }
    }

    /// Lowercase label — matches the `RunOutcome` spelling so the report reads
    /// consistently.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Victory => "victory",
            Outcome::Defeat => "defeat",
        }
    }
}

/// A structured fact about the simulation, for balance analysis.
///
/// Each variant is emitted from its own chokepoint, unconditionally (all ships,
/// all builds, outside every `is_local` gate). The timestamped facts a ledger
/// needs (deaths, knockouts) are stamped at collection into a
/// [`StampedBalanceEvent`]; the variant itself carries only what the chokepoint
/// knows.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum BalanceEvent {
    /// Damage landed on a ship (or asteroid) somewhere in the world.
    DamageApplied {
        /// UUID of whoever dealt it. `None` for environmental damage —
        /// collisions and region damage zones have no shooter.
        attacker: Option<String>,
        /// UUID of what took it.
        victim: String,
        /// Whether `victim` is a ship or an asteroid. Only ship victims reach
        /// the ledgers; see [`aggregate_ledgers`].
        victim_kind: VictimKind,
        /// Which weapon delivered it: a bank/tube id for configured weapons,
        /// otherwise one of the `WEAPON_KIND_*` labels.
        weapon: String,
        /// Damage offered to the target, before shields and before the hull
        /// pool clamps it.
        amount: f32,
        /// The portion the shields ate.
        shield_absorbed: f32,
        /// The portion that actually came off the hull.
        hull_damage: f32,
        /// Which ship system took the hit. Always `None` for now — no
        /// chokepoint can name the system that `apply_hull_damage` picked;
        /// attribution arrives with the tier-crossing events.
        system_hit: Option<String>,
    },
    /// A shot left a ship — a beam opened, a torpedo launched, a blaster fired.
    /// Distinct from `DamageApplied` (a shot *landing*): a ledger's
    /// `shots_fired` counts these, its `by_weapon` counts landings.
    WeaponFired {
        /// UUID of the ship that fired. `None` for a shooter with no identity.
        shooter: Option<String>,
        /// Bank/tube id the shot came from.
        weapon: String,
        /// Weapon family — one of the `FIRED_KIND_*` labels.
        kind: String,
    },
    /// A shield facing dropped from online to offline under fire. Emitted once,
    /// on the online→offline edge, at the weapon chokepoint that broke it.
    ShieldArcCollapsed {
        /// UUID of the ship whose facing collapsed.
        ship: String,
        /// Stable arc id (`"fore"`, `"aft"`, …) of the facing.
        arc_id: String,
    },
    /// A ship system crossed a damage tier (either direction). A crossing to
    /// `Disabled`/`Destroyed` is a knockout the ledger timestamps.
    SystemTierCrossed {
        /// UUID of the ship the system belongs to.
        ship: String,
        /// System id that crossed.
        system_id: String,
        /// Tier before the crossing (`Debug`-formatted `DamageTier`).
        from_tier: String,
        /// Tier after the crossing.
        to_tier: String,
    },
    /// Every weapon system on a ship is now non-operational — the ship can no
    /// longer attack. Reported, not terminal: the run continues.
    Disarmed {
        /// UUID of the disarmed ship.
        ship: String,
    },
    /// A ship (or station) was destroyed. Emitted exactly once per death, at
    /// the kill site, carrying the killer credit the `AiEntityDestroyed` path
    /// throws away.
    EntityDestroyed {
        /// UUID of the destroyed entity.
        victim: String,
        /// UUID of whoever landed the kill, when a shooter was in scope.
        /// `None` for environmental deaths (collision, region).
        killer: Option<String>,
    },
    /// A ship's red-alert state was toggled.
    RedAlertChanged {
        /// UUID of the ship.
        ship: String,
        /// The new state after the toggle.
        on: bool,
    },
    /// A mission objective transitioned to `Completed`. Ship-agnostic: the
    /// objective manager is shared.
    ObjectiveCompleted {
        /// Stable objective id that completed.
        objective_id: String,
    },
    /// The global game phase changed (`Lobby` → `InProgress` → `GameOver`, …).
    PhaseChanged {
        /// Phase before the transition (`Debug`-formatted `GamePhase`).
        from: String,
        /// Phase after the transition.
        to: String,
    },
    /// A ship's repair teams restored hull HP this tick. `hp` is the positive
    /// delta of total hull current across the team tick.
    RepairApplied {
        /// UUID of the repairing ship.
        ship: String,
        /// Hull HP restored this tick (always > 0 when emitted).
        hp: f32,
    },
    /// A ship's committed doctrine movement phase changed (issue #915) — the
    /// Engines policy machine's current state, which is the authored
    /// `engines_ai.state` id (`"acquire"`, `"attack_run"`, `"escape"`, …).
    /// Emitted once per observed change, including the initial phase on the
    /// first AI tick, so the report can fold per-ship time-in-phase.
    DoctrinePhaseChanged {
        /// UUID of the ship.
        ship: String,
        /// The authored state id just committed.
        phase: String,
    },
}

impl BalanceEvent {
    /// How many variants this enum has.
    ///
    /// Hand-maintained, and deliberately so: its only job is to fail the
    /// timeline-coverage test when a variant is added, forcing whoever adds one
    /// to say whether it is a story beat or per-tick bookkeeping. A derived
    /// count would track the enum silently and guard nothing.
    pub const VARIANT_COUNT: usize = 11;

    /// Whether this event belongs in the ndjson *timeline stream*.
    ///
    /// The timeline is the story of a fight — hits, shots, collapses,
    /// knockouts, deaths, phase changes. Every variant qualifies except
    /// [`BalanceEvent::RepairApplied`], which repair teams emit *per tick per
    /// ship* for as long as anything is damaged: a 250s `combat_test` run
    /// produced 6,761 of them against 1,688 of everything else, i.e. 80% of
    /// the timeline was one ship trickling hull back. That is a *rate*, not a
    /// story beat, and nobody reads it a line at a time.
    ///
    /// # Why filter the stream rather than the emission
    ///
    /// `repair_hp` in the per-ship ledger is a real metric and has to stay
    /// exact, and the only honest way to total a per-tick delta is to see
    /// every tick. So the events keep flowing to
    /// [`aggregate_damage`] unchanged and only the *display* stream is
    /// filtered — the alternative (coalescing at the emitter into repair
    /// episodes) would have to reconstruct the total anyway, and would lose
    /// the tail of any episode still running when the run ended.
    ///
    /// Repair remains visible in the report: `damage_by_ship.*.repair_hp`.
    pub fn in_timeline_stream(&self) -> bool {
        !matches!(self, BalanceEvent::RepairApplied { .. })
    }

    /// Encode as a JSON object. Hand-rolled rather than serde because
    /// `serde_json` is confined to `codec.rs`.
    pub fn to_json(&self) -> String {
        match self {
            BalanceEvent::DamageApplied {
                attacker,
                victim,
                victim_kind,
                weapon,
                amount,
                shield_absorbed,
                hull_damage,
                system_hit,
            } => format!(
                "{{\"event\":\"DamageApplied\",\"attacker\":{},\"victim\":{:?},\"victim_kind\":{:?},\"weapon\":{:?},\"amount\":{:.3},\"shield_absorbed\":{:.3},\"hull_damage\":{:.3},\"system_hit\":{}}}",
                opt_string(attacker),
                victim,
                victim_kind.as_str(),
                weapon,
                amount,
                shield_absorbed,
                hull_damage,
                opt_string(system_hit),
            ),
            BalanceEvent::WeaponFired {
                shooter,
                weapon,
                kind,
            } => format!(
                "{{\"event\":\"WeaponFired\",\"shooter\":{},\"weapon\":{:?},\"kind\":{:?}}}",
                opt_string(shooter),
                weapon,
                kind,
            ),
            BalanceEvent::ShieldArcCollapsed { ship, arc_id } => format!(
                "{{\"event\":\"ShieldArcCollapsed\",\"ship\":{:?},\"arc_id\":{:?}}}",
                ship, arc_id,
            ),
            BalanceEvent::SystemTierCrossed {
                ship,
                system_id,
                from_tier,
                to_tier,
            } => format!(
                "{{\"event\":\"SystemTierCrossed\",\"ship\":{:?},\"system_id\":{:?},\"from_tier\":{:?},\"to_tier\":{:?}}}",
                ship, system_id, from_tier, to_tier,
            ),
            BalanceEvent::Disarmed { ship } => {
                format!("{{\"event\":\"Disarmed\",\"ship\":{ship:?}}}")
            }
            BalanceEvent::EntityDestroyed { victim, killer } => format!(
                "{{\"event\":\"EntityDestroyed\",\"victim\":{:?},\"killer\":{}}}",
                victim,
                opt_string(killer),
            ),
            BalanceEvent::RedAlertChanged { ship, on } => format!(
                "{{\"event\":\"RedAlertChanged\",\"ship\":{ship:?},\"on\":{on}}}"
            ),
            BalanceEvent::ObjectiveCompleted { objective_id } => format!(
                "{{\"event\":\"ObjectiveCompleted\",\"objective_id\":{objective_id:?}}}"
            ),
            BalanceEvent::PhaseChanged { from, to } => {
                format!("{{\"event\":\"PhaseChanged\",\"from\":{from:?},\"to\":{to:?}}}")
            }
            BalanceEvent::RepairApplied { ship, hp } => {
                format!("{{\"event\":\"RepairApplied\",\"ship\":{ship:?},\"hp\":{hp:.3}}}")
            }
            BalanceEvent::DoctrinePhaseChanged { ship, phase } => format!(
                "{{\"event\":\"DoctrinePhaseChanged\",\"ship\":{ship:?},\"phase\":{phase:?}}}"
            ),
        }
    }
}

/// A [`BalanceEvent`] with the tick and sim-time it landed on.
///
/// Lives here rather than in the headless report so [`aggregate_ledgers`] — the
/// pure fold — can consume the stamped log directly and still name a death's
/// tick / a knockout's sim-time.
#[derive(Debug, Clone, PartialEq)]
pub struct StampedBalanceEvent {
    pub tick: u64,
    pub sim_t: f64,
    pub event: BalanceEvent,
}

/// `Some("x")` → `"x"`, `None` → `null`. Uses `{:?}` on `&str` for escaping,
/// the same trick the run report uses for its string fields.
fn opt_string(v: &Option<String>) -> String {
    match v {
        Some(s) => format!("{s:?}"),
        None => "null".to_string(),
    }
}

/// A single system knockout: the system that dropped out and when.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemKnockout {
    /// System id that was knocked out.
    pub system_id: String,
    /// Tier it crossed to (`"Disabled"` or `"Destroyed"`).
    pub tier: String,
    /// Sim tick of the knockout.
    pub tick: u64,
    /// Sim time (seconds) of the knockout.
    pub sim_t: f64,
}

/// What one ship did and had done to it over a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DamageLedger {
    /// Whatever `EntityName` held, when the world knew one — *not* resolved
    /// display text. For a TOML-defined entity this is a strings.csv key
    /// (`entity.alliance_cruiser.name`); for a scenario-spawned NPC it is the
    /// literal name the trigger assigned (`wave_1b`). Left unresolved on
    /// purpose: this report is a dev artifact, and threading the localisation
    /// table into it would buy nothing a human reading a uuid-keyed table
    /// cannot already work out.
    pub name_id: Option<String>,
    /// Damage this ship landed on others.
    pub damage_dealt: f32,
    /// Damage landed on this ship.
    pub damage_taken: f32,
    /// Landed damage this ship dealt, split by the weapon that delivered it.
    pub by_weapon: BTreeMap<String, f32>,
    /// Landed damage this ship dealt, split by the victim it hit.
    pub by_pair: BTreeMap<String, f32>,
    /// Of `damage_taken`, the portion the shields ate.
    pub shield_absorbed: f32,
    /// Of `damage_taken`, the portion that came off the hull.
    pub hull_taken: f32,
    /// Shots this ship fired, split by weapon. From `WeaponFired`, so it counts
    /// shots that missed as well as ones that landed.
    pub shots_fired: BTreeMap<String, u64>,
    /// How many kills this ship was credited with (`EntityDestroyed.killer`).
    pub kills: u64,
    /// When this ship died, if it did: `(tick, sim_t)`.
    pub death: Option<(u64, f64)>,
    /// Every system knockout this ship suffered, in event order.
    pub system_knockouts: Vec<SystemKnockout>,
    /// Total hull HP this ship's repair teams restored over the run.
    pub repair_hp: f32,
    /// Sim-seconds this ship spent in each committed doctrine movement phase
    /// (the Engines machine's authored state ids), folded from
    /// [`BalanceEvent::DoctrinePhaseChanged`] (issue #915). The open interval at
    /// run end is closed at the ship's death time when it died, otherwise at
    /// the run's final sim time. Empty for a hull with a stateless policy.
    pub phase_seconds: BTreeMap<String, f64>,
}

fn add_f32(map: &mut BTreeMap<String, f32>, key: &str, amount: f32) {
    *map.entry(key.to_string()).or_default() += amount;
}

/// Fold the non-timestamped facts of a bare event log into ledgers.
///
/// Handles everything a bare [`BalanceEvent`] carries: damage totals and their
/// by-weapon / by-pair / shield-vs-hull splits, shots fired, and kill credit.
/// Timestamped facts (deaths, knockouts) need the tick/sim-time that only lives
/// on [`StampedBalanceEvent`], so they are filled by [`aggregate_ledgers`],
/// which wraps this.
///
/// Landed damage is `shield_absorbed + hull_damage`, not the offered `amount`:
/// a shot into an overkilled hull pool should not read as more effective than
/// one that connected. Asteroid victims are dropped from both sides — the map
/// is per-*ship* combat effectiveness, and mining a rock is not combat.
pub fn aggregate_damage<'a>(
    events: impl IntoIterator<Item = &'a BalanceEvent>,
    names: &BTreeMap<String, String>,
) -> BTreeMap<String, DamageLedger> {
    let mut ledgers: BTreeMap<String, DamageLedger> = BTreeMap::new();
    for event in events {
        match event {
            BalanceEvent::DamageApplied {
                attacker,
                victim,
                victim_kind,
                weapon,
                shield_absorbed,
                hull_damage,
                ..
            } => {
                if *victim_kind != VictimKind::Ship {
                    continue;
                }
                let landed = shield_absorbed + hull_damage;
                if let Some(attacker) = attacker {
                    let l = ledgers.entry(attacker.clone()).or_default();
                    l.damage_dealt += landed;
                    add_f32(&mut l.by_weapon, weapon, landed);
                    add_f32(&mut l.by_pair, victim, landed);
                }
                let v = ledgers.entry(victim.clone()).or_default();
                v.damage_taken += landed;
                v.shield_absorbed += shield_absorbed;
                v.hull_taken += hull_damage;
            }
            BalanceEvent::WeaponFired {
                shooter, weapon, ..
            } => {
                if let Some(shooter) = shooter {
                    *ledgers
                        .entry(shooter.clone())
                        .or_default()
                        .shots_fired
                        .entry(weapon.clone())
                        .or_default() += 1;
                }
            }
            BalanceEvent::EntityDestroyed { killer, .. } => {
                if let Some(killer) = killer {
                    ledgers.entry(killer.clone()).or_default().kills += 1;
                }
            }
            BalanceEvent::RepairApplied { ship, hp } => {
                ledgers.entry(ship.clone()).or_default().repair_hp += hp;
            }
            // Timeline-only or timestamp-dependent variants: no bare-fold
            // contribution. Deaths, knockouts and phase occupancy are folded in
            // `aggregate_ledgers` where the stamp is available.
            BalanceEvent::ShieldArcCollapsed { .. }
            | BalanceEvent::SystemTierCrossed { .. }
            | BalanceEvent::Disarmed { .. }
            | BalanceEvent::RedAlertChanged { .. }
            | BalanceEvent::ObjectiveCompleted { .. }
            | BalanceEvent::PhaseChanged { .. }
            | BalanceEvent::DoctrinePhaseChanged { .. } => {}
        }
    }
    for (uuid, ledger) in ledgers.iter_mut() {
        ledger.name_id = names.get(uuid).cloned();
    }
    ledgers
}

/// Fold a *stamped* balance-event log into per-ship ledgers.
///
/// The full aggregation: [`aggregate_damage`] over the bare events for the
/// untimed facts, then a stamped pass for the facts that need a clock — a
/// ship's death timestamp, each system knockout, and doctrine phase occupancy.
/// Pure by design: no ECS, no resources, no time beyond what the stamps carry.
///
/// `final_sim_t` is the run's final sim time, used to close each ship's open
/// phase interval: a ship that died has its last phase closed at its death
/// stamp instead, so a corpse never accrues occupancy.
pub fn aggregate_ledgers(
    events: &[StampedBalanceEvent],
    names: &BTreeMap<String, String>,
    final_sim_t: f64,
) -> BTreeMap<String, DamageLedger> {
    let mut ledgers = aggregate_damage(events.iter().map(|s| &s.event), names);
    // Per-ship open phase interval: (phase, entered-at sim_t).
    let mut open_phase: BTreeMap<String, (String, f64)> = BTreeMap::new();
    for stamped in events {
        match &stamped.event {
            BalanceEvent::EntityDestroyed { victim, .. } => {
                // First death wins — a ship dies once. Later hits on a corpse
                // (rare, but the log is append-only) must not move the stamp.
                let l = ledgers.entry(victim.clone()).or_default();
                if l.death.is_none() {
                    l.death = Some((stamped.tick, stamped.sim_t));
                }
            }
            BalanceEvent::SystemTierCrossed {
                ship,
                system_id,
                to_tier,
                ..
            } if to_tier == "Disabled" || to_tier == "Destroyed" => {
                ledgers
                    .entry(ship.clone())
                    .or_default()
                    .system_knockouts
                    .push(SystemKnockout {
                        system_id: system_id.clone(),
                        tier: to_tier.clone(),
                        tick: stamped.tick,
                        sim_t: stamped.sim_t,
                    });
            }
            BalanceEvent::DoctrinePhaseChanged { ship, phase } => {
                let ledger = ledgers.entry(ship.clone()).or_default();
                if let Some((prev, since)) = open_phase.remove(ship) {
                    *ledger.phase_seconds.entry(prev).or_default() +=
                        (stamped.sim_t - since).max(0.0);
                }
                open_phase.insert(ship.clone(), (phase.clone(), stamped.sim_t));
            }
            _ => {}
        }
    }
    // Close every still-open phase interval: at the ship's death when it died
    // (the machine stops with the ship), otherwise at the end of the run.
    for (uuid, (phase, since)) in open_phase {
        let ledger = ledgers.entry(uuid).or_default();
        let end = ledger.death.map(|(_, t)| t).unwrap_or(final_sim_t);
        *ledger.phase_seconds.entry(phase).or_default() += (end - since).max(0.0);
    }
    // Names may have arrived only via a stamped-only variant (a ship that only
    // ever died or was knocked out), so re-attach after the stamped pass.
    for (uuid, ledger) in ledgers.iter_mut() {
        if ledger.name_id.is_none() {
            ledger.name_id = names.get(uuid).cloned();
        }
    }
    ledgers
}

/// Render a `BTreeMap<String, f32>` as a JSON object body. Stable order.
fn f32_map_to_json(map: &BTreeMap<String, f32>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k:?}: {v:.3}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a `BTreeMap<String, f64>` as a JSON object body. Stable order.
fn f64_map_to_json(map: &BTreeMap<String, f64>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k:?}: {v:.3}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a `BTreeMap<String, u64>` as a JSON object body. Stable order.
fn u64_map_to_json(map: &BTreeMap<String, u64>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k:?}: {v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn knockouts_to_json(knockouts: &[SystemKnockout]) -> String {
    knockouts
        .iter()
        .map(|k| {
            format!(
                "{{\"system_id\": {:?}, \"tier\": {:?}, \"tick\": {}, \"sim_t\": {:.4}}}",
                k.system_id, k.tier, k.tick, k.sim_t
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the ledger map as the body of a JSON object, keyed by uuid.
///
/// `BTreeMap` ordering makes this byte-identical across runs, which is what
/// lets a report be diffed between builds.
pub fn ledgers_to_json(ledgers: &BTreeMap<String, DamageLedger>) -> String {
    ledgers
        .iter()
        .map(|(uuid, l)| {
            let death = match l.death {
                Some((tick, sim_t)) => format!("[{tick}, {sim_t:.4}]"),
                None => "null".to_string(),
            };
            format!(
                "{:?}: {{\"name_id\": {}, \"damage_dealt\": {:.3}, \"damage_taken\": {:.3}, \
                 \"by_weapon\": {{{}}}, \"by_pair\": {{{}}}, \"shield_absorbed\": {:.3}, \
                 \"hull_taken\": {:.3}, \"shots_fired\": {{{}}}, \"kills\": {}, \"death\": {}, \
                 \"system_knockouts\": [{}], \"repair_hp\": {:.3}, \"phase_seconds\": {{{}}}}}",
                uuid,
                opt_string(&l.name_id),
                l.damage_dealt,
                l.damage_taken,
                f32_map_to_json(&l.by_weapon),
                f32_map_to_json(&l.by_pair),
                l.shield_absorbed,
                l.hull_taken,
                u64_map_to_json(&l.shots_fired),
                l.kills,
                death,
                knockouts_to_json(&l.system_knockouts),
                l.repair_hp,
                f64_map_to_json(&l.phase_seconds),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ── Run outcome classification (issue #843) ────────────────────────────────

/// Length of the closing window used to tell a live `timeout` from a stalemate
/// `draw`. A *reporting* heuristic, not a gameplay value: it never feeds the
/// simulation, only the exit classification, so it lives as a const here rather
/// than in world TOML. Damage still landing within this many sim-seconds of the
/// tick budget running out means both sides were still fighting (timeout); a
/// silent window means mutual ineffectiveness (draw).
pub const CLOSING_WINDOW_SECS: f64 = 15.0;

/// Landed-damage-rate floor (HP/sec, summed across both sides) above which a
/// budget-exhausted run is a live `timeout` rather than a stalemate `draw`.
/// Also a reporting heuristic — a hair above zero so a single stray tick of
/// chip damage in the closing window does not read as an active fight.
pub const CLOSING_ACTIVE_RATE: f32 = 0.01;

/// How a finished run is classified for the exit report.
///
/// Victory/defeat come from the scenario game-over path (a declared
/// [`Outcome`], or the player-death latch); draw/timeout come from the tick
/// budget exhausting with combatants still present. The terminal conditions
/// stay annihilation-or-budget-exhaustion — combat-ineffectiveness is
/// *reported* (draw vs timeout), never adjudicated by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Victory,
    Defeat,
    Draw,
    Timeout,
}

impl RunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RunOutcome::Victory => "victory",
            RunOutcome::Defeat => "defeat",
            RunOutcome::Draw => "draw",
            RunOutcome::Timeout => "timeout",
        }
    }
}

/// Landed-damage rate (HP/sec) per attacker over the closing window.
///
/// Filters `DamageApplied` to ship victims stamped within `window_secs` of
/// `final_sim_t`, sums the *landed* portion (`shield_absorbed + hull_damage` —
/// the same convention [`aggregate_damage`] uses so a shot into an overkilled
/// hull does not inflate the rate), and divides by the window. Keyed by
/// attacker uuid; environmental damage (no attacker) is dropped because it
/// belongs to no side. Pure: consumes only the stamped log.
pub fn closing_damage_rates(
    events: &[StampedBalanceEvent],
    final_sim_t: f64,
    window_secs: f64,
) -> BTreeMap<String, f32> {
    let mut rates: BTreeMap<String, f32> = BTreeMap::new();
    if window_secs <= 0.0 {
        return rates;
    }
    let cutoff = final_sim_t - window_secs;
    for stamped in events {
        if stamped.sim_t < cutoff {
            continue;
        }
        if let BalanceEvent::DamageApplied {
            attacker: Some(attacker),
            victim_kind: VictimKind::Ship,
            shield_absorbed,
            hull_damage,
            ..
        } = &stamped.event
        {
            *rates.entry(attacker.clone()).or_default() += shield_absorbed + hull_damage;
        }
    }
    for v in rates.values_mut() {
        *v /= window_secs as f32;
    }
    rates
}

/// One side's balance margins at the end of a run.
///
/// A "side" is a faction grouping relative to the player's ship — see the
/// headless report's `build_report`. These fields are the balance signal a
/// draw/timeout carries: how much hull each side had left, how much damage
/// flowed each way over the whole run, and how hard each side was still hitting
/// in the closing window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SideMargins {
    /// Hull HP still on this side's *surviving* ships.
    pub remaining_hull: f32,
    /// Max hull HP of this side's surviving ships (the fraction denominator).
    pub remaining_hull_max: f32,
    /// `remaining_hull / remaining_hull_max`, or 0.0 with no surviving ships.
    pub remaining_hull_fraction: f32,
    /// Landed damage this side dealt to others over the whole run.
    pub damage_dealt: f32,
    /// Landed damage this side took over the whole run.
    pub damage_taken: f32,
    /// Landed-damage rate (HP/sec) this side dealt in the closing window.
    pub closing_damage_rate: f32,
}

impl SideMargins {
    /// Build from summed hull and damage totals, deriving the fraction.
    pub fn new(
        remaining_hull: f32,
        remaining_hull_max: f32,
        damage_dealt: f32,
        damage_taken: f32,
        closing_damage_rate: f32,
    ) -> Self {
        let remaining_hull_fraction = if remaining_hull_max > 0.0 {
            remaining_hull / remaining_hull_max
        } else {
            0.0
        };
        SideMargins {
            remaining_hull,
            remaining_hull_max,
            remaining_hull_fraction,
            damage_dealt,
            damage_taken,
            closing_damage_rate,
        }
    }

    /// Serialise as a JSON object (with braces), stable field order.
    fn to_json(&self) -> String {
        format!(
            "{{\"remaining_hull\": {:.3}, \"remaining_hull_max\": {:.3}, \
             \"remaining_hull_fraction\": {:.4}, \"damage_dealt\": {:.3}, \
             \"damage_taken\": {:.3}, \"closing_damage_rate\": {:.4}}}",
            self.remaining_hull,
            self.remaining_hull_max,
            self.remaining_hull_fraction,
            self.damage_dealt,
            self.damage_taken,
            self.closing_damage_rate,
        )
    }
}

/// The classified outcome plus the per-side margins that carry the balance
/// signal. Every run gets one (AC1); draw/timeout runs lean on the margins.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeReport {
    pub outcome: RunOutcome,
    pub player: SideMargins,
    pub enemy: SideMargins,
}

impl OutcomeReport {
    /// Serialise as `"outcome": ..., "sides": {...}` (object body, no outer
    /// braces) so the report can inline it between its other fields.
    pub fn to_json(&self) -> String {
        format!(
            "\"outcome\": {:?},\n  \"sides\": {{\"player\": {}, \"enemy\": {}}}",
            self.outcome.as_str(),
            self.player.to_json(),
            self.enemy.to_json(),
        )
    }
}

/// Classify a finished run. PURE — no ECS, no clock beyond the stamps already
/// folded into the margins — so every outcome is unit-testable (AC3).
///
/// Precedence:
/// 1. Reached `GamePhase::GameOver` → victory or defeat from `outcome_flag`.
///    A scenario `game_over` with **no** declared outcome defaults to
///    **victory**: the scenario ran to a scripted end-state, and the built-in
///    player-death path is the separately-latched [`Outcome::Defeat`]. So an
///    undeclared scripted end is the ship surviving to the finish, not losing.
/// 2. Budget exhausted (still `InProgress`) → draw vs timeout from the closing
///    window: damage still landing means both sides were fighting (timeout); a
///    silent window means mutual ineffectiveness (draw). Both carry the same
///    margin payload.
pub fn classify(
    final_phase_is_game_over: bool,
    outcome_flag: Option<Outcome>,
    player: SideMargins,
    enemy: SideMargins,
) -> OutcomeReport {
    let outcome = if final_phase_is_game_over {
        match outcome_flag {
            Some(Outcome::Defeat) => RunOutcome::Defeat,
            // Declared victory, or an undeclared scripted end (default victory).
            Some(Outcome::Victory) | None => RunOutcome::Victory,
        }
    } else {
        let closing = player.closing_damage_rate + enemy.closing_damage_rate;
        if closing > CLOSING_ACTIVE_RATE {
            RunOutcome::Timeout
        } else {
            RunOutcome::Draw
        }
    };
    OutcomeReport {
        outcome,
        player,
        enemy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(attacker: Option<&str>, victim: &str, shield: f32, hull: f32) -> BalanceEvent {
        BalanceEvent::DamageApplied {
            attacker: attacker.map(|s| s.to_string()),
            victim: victim.to_string(),
            victim_kind: VictimKind::Ship,
            weapon: "fore_phaser".into(),
            amount: shield + hull,
            shield_absorbed: shield,
            hull_damage: hull,
            system_hit: None,
        }
    }

    fn stamp(tick: u64, sim_t: f64, event: BalanceEvent) -> StampedBalanceEvent {
        StampedBalanceEvent { tick, sim_t, event }
    }

    #[test]
    fn aggregate_splits_dealt_and_taken_per_ship() {
        let events = vec![
            hit(Some("player"), "raider", 6.0, 4.0),
            hit(Some("player"), "raider", 0.0, 5.0),
            hit(Some("raider"), "player", 3.0, 0.0),
        ];
        let ledgers = aggregate_damage(&events, &BTreeMap::new());

        assert_eq!(ledgers["player"].damage_dealt, 15.0);
        assert_eq!(ledgers["player"].damage_taken, 3.0);
        assert_eq!(ledgers["raider"].damage_dealt, 3.0);
        assert_eq!(ledgers["raider"].damage_taken, 15.0);
    }

    #[test]
    fn environmental_damage_has_no_attacker_but_still_charges_the_victim() {
        let events = vec![BalanceEvent::DamageApplied {
            attacker: None,
            victim: "player".into(),
            victim_kind: VictimKind::Ship,
            weapon: WEAPON_KIND_REGION.into(),
            amount: 8.0,
            shield_absorbed: 2.0,
            hull_damage: 6.0,
            system_hit: None,
        }];
        let ledgers = aggregate_damage(&events, &BTreeMap::new());

        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers["player"].damage_taken, 8.0);
        assert_eq!(ledgers["player"].damage_dealt, 0.0);
    }

    /// `damage_by_ship` is a combat-effectiveness table. A shooter chewing
    /// through an asteroid field must not read as a shooter winning a fight,
    /// and the rock must not get a ledger row of its own.
    #[test]
    fn asteroid_victims_are_excluded_from_both_sides_of_the_ledger() {
        let mining = BalanceEvent::DamageApplied {
            attacker: Some("player".into()),
            victim: "asteroid-7".into(),
            victim_kind: VictimKind::Asteroid,
            weapon: "fore_phaser".into(),
            amount: 30.0,
            shield_absorbed: 0.0,
            hull_damage: 30.0,
            system_hit: None,
        };
        let events = vec![mining, hit(Some("player"), "raider", 1.0, 2.0)];
        let ledgers = aggregate_damage(&events, &BTreeMap::new());

        assert!(
            !ledgers.contains_key("asteroid-7"),
            "an asteroid must not get a ledger row"
        );
        assert_eq!(
            ledgers["player"].damage_dealt, 3.0,
            "only the hit on the raider counts as damage dealt"
        );
        assert_eq!(ledgers["raider"].damage_taken, 3.0);
    }

    /// A run that only ever shot rocks produced no combat at all.
    #[test]
    fn asteroid_only_log_yields_an_empty_ledger_map() {
        let events = vec![BalanceEvent::DamageApplied {
            attacker: Some("player".into()),
            victim: "asteroid-1".into(),
            victim_kind: VictimKind::Asteroid,
            weapon: "fore_tube".into(),
            amount: 12.0,
            shield_absorbed: 0.0,
            hull_damage: 12.0,
            system_hit: None,
        }];

        assert!(aggregate_damage(&events, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn names_are_attached_when_known_and_null_otherwise() {
        let events = vec![hit(Some("player"), "raider", 0.0, 1.0)];
        // A TOML-defined ship carries a strings.csv key here, not display
        // text — the ledger stores whatever `EntityName` held, verbatim.
        let names: BTreeMap<String, String> = [(
            "player".to_string(),
            "entity.alliance_cruiser.name".to_string(),
        )]
        .into_iter()
        .collect();
        let ledgers = aggregate_damage(&events, &names);

        assert_eq!(
            ledgers["player"].name_id.as_deref(),
            Some("entity.alliance_cruiser.name")
        );
        assert_eq!(ledgers["raider"].name_id, None);
    }

    // ── New split fields ──────────────────────────────────────────────────────

    /// `by_weapon` credits each weapon with the damage it landed; `by_pair`
    /// credits each victim.
    #[test]
    fn by_weapon_and_by_pair_split_the_dealt_total() {
        let events = vec![
            BalanceEvent::DamageApplied {
                attacker: Some("player".into()),
                victim: "raider".into(),
                victim_kind: VictimKind::Ship,
                weapon: "fore_phaser".into(),
                amount: 10.0,
                shield_absorbed: 4.0,
                hull_damage: 6.0,
                system_hit: None,
            },
            BalanceEvent::DamageApplied {
                attacker: Some("player".into()),
                victim: "scout".into(),
                victim_kind: VictimKind::Ship,
                weapon: "fore_tube".into(),
                amount: 5.0,
                shield_absorbed: 0.0,
                hull_damage: 5.0,
                system_hit: None,
            },
        ];
        let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
        assert_eq!(l.damage_dealt, 15.0);
        assert_eq!(l.by_weapon["fore_phaser"], 10.0);
        assert_eq!(l.by_weapon["fore_tube"], 5.0);
        assert_eq!(l.by_pair["raider"], 10.0);
        assert_eq!(l.by_pair["scout"], 5.0);
    }

    /// The shield/hull split of `damage_taken` reconciles to the total.
    #[test]
    fn shield_and_hull_taken_split_the_taken_total() {
        let events = vec![
            hit(Some("raider"), "player", 6.0, 4.0),
            hit(Some("raider"), "player", 0.0, 5.0),
        ];
        let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
        assert_eq!(l.damage_taken, 15.0);
        assert_eq!(l.shield_absorbed, 6.0);
        assert_eq!(l.hull_taken, 9.0);
        assert_eq!(l.shield_absorbed + l.hull_taken, l.damage_taken);
    }

    #[test]
    fn shots_fired_counts_weapon_fired_per_weapon_even_without_a_hit() {
        let events = vec![
            BalanceEvent::WeaponFired {
                shooter: Some("player".into()),
                weapon: "port".into(),
                kind: FIRED_KIND_BEAM.into(),
            },
            BalanceEvent::WeaponFired {
                shooter: Some("player".into()),
                weapon: "port".into(),
                kind: FIRED_KIND_BEAM.into(),
            },
            BalanceEvent::WeaponFired {
                shooter: Some("player".into()),
                weapon: "fore_tube".into(),
                kind: FIRED_KIND_TORPEDO.into(),
            },
        ];
        let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
        assert_eq!(l.shots_fired["port"], 2);
        assert_eq!(l.shots_fired["fore_tube"], 1);
        // A ship that only fired (never landed) still gets a row.
        assert_eq!(l.damage_dealt, 0.0);
    }

    #[test]
    fn kills_credit_the_killer_and_death_stamps_the_victim() {
        let events = vec![
            stamp(5, 0.5, hit(Some("player"), "raider", 0.0, 3.0)),
            stamp(
                9,
                0.9,
                BalanceEvent::EntityDestroyed {
                    victim: "raider".into(),
                    killer: Some("player".into()),
                },
            ),
            // A later hit on the corpse must not move the death stamp.
            stamp(
                12,
                1.2,
                BalanceEvent::EntityDestroyed {
                    victim: "raider".into(),
                    killer: Some("player".into()),
                },
            ),
        ];
        let ledgers = aggregate_ledgers(&events, &BTreeMap::new(), 2.0);
        assert_eq!(ledgers["player"].kills, 2);
        assert_eq!(ledgers["raider"].death, Some((9, 0.9)));
    }

    #[test]
    fn knockouts_record_disabling_crossings_with_timestamps() {
        let events = vec![
            // A crossing to Damaged is not a knockout.
            stamp(
                3,
                0.3,
                BalanceEvent::SystemTierCrossed {
                    ship: "raider".into(),
                    system_id: "phaser-fore".into(),
                    from_tier: "Operational".into(),
                    to_tier: "Damaged".into(),
                },
            ),
            stamp(
                7,
                0.7,
                BalanceEvent::SystemTierCrossed {
                    ship: "raider".into(),
                    system_id: "phaser-fore".into(),
                    from_tier: "Damaged".into(),
                    to_tier: "Disabled".into(),
                },
            ),
            stamp(
                8,
                0.8,
                BalanceEvent::SystemTierCrossed {
                    ship: "raider".into(),
                    system_id: "helm".into(),
                    from_tier: "Operational".into(),
                    to_tier: "Destroyed".into(),
                },
            ),
        ];
        let l = &aggregate_ledgers(&events, &BTreeMap::new(), 1.0)["raider"];
        assert_eq!(
            l.system_knockouts.len(),
            2,
            "only disabling crossings count"
        );
        assert_eq!(l.system_knockouts[0].system_id, "phaser-fore");
        assert_eq!(l.system_knockouts[0].tier, "Disabled");
        assert_eq!(l.system_knockouts[0].tick, 7);
        assert_eq!(l.system_knockouts[1].system_id, "helm");
        assert_eq!(l.system_knockouts[1].sim_t, 0.8);
    }

    fn phase(ship: &str, phase: &str) -> BalanceEvent {
        BalanceEvent::DoctrinePhaseChanged {
            ship: ship.to_string(),
            phase: phase.to_string(),
        }
    }

    /// Occupancy is the time between consecutive phase changes, per ship, with
    /// the open interval at the end closed at the run's final sim time.
    #[test]
    fn phase_occupancy_attributes_time_between_changes_and_closes_at_run_end() {
        let events = vec![
            stamp(1, 0.0, phase("player", "acquire")),
            stamp(50, 5.0, phase("player", "attack_run")),
            // Re-entering a phase accumulates onto the same key.
            stamp(120, 12.0, phase("player", "acquire")),
            // A second ship's machine is folded independently.
            stamp(2, 1.0, phase("raider", "acquire")),
        ];
        let ledgers = aggregate_ledgers(&events, &BTreeMap::new(), 20.0);
        let p = &ledgers["player"].phase_seconds;
        assert!((p["acquire"] - (5.0 + 8.0)).abs() < 1e-9, "got {p:?}");
        assert!((p["attack_run"] - 7.0).abs() < 1e-9, "got {p:?}");
        assert!((ledgers["raider"].phase_seconds["acquire"] - 19.0).abs() < 1e-9);
    }

    /// A dead ship's machine stops with the ship: its open phase closes at the
    /// death stamp, never at the end of the run.
    #[test]
    fn phase_occupancy_closes_at_the_ships_death_not_run_end() {
        let events = vec![
            stamp(1, 0.0, phase("raider", "acquire")),
            stamp(30, 3.0, phase("raider", "escape")),
            stamp(
                90,
                9.0,
                BalanceEvent::EntityDestroyed {
                    victim: "raider".into(),
                    killer: Some("player".into()),
                },
            ),
        ];
        let l = &aggregate_ledgers(&events, &BTreeMap::new(), 60.0)["raider"];
        assert!((l.phase_seconds["acquire"] - 3.0).abs() < 1e-9);
        assert!(
            (l.phase_seconds["escape"] - 6.0).abs() < 1e-9,
            "the corpse must not accrue occupancy: got {:?}",
            l.phase_seconds
        );
    }

    /// The ledger JSON carries `phase_seconds` as a parseable object.
    #[test]
    fn ledger_json_carries_phase_seconds() {
        let events = vec![
            stamp(1, 0.0, phase("player", "acquire")),
            stamp(60, 6.0, phase("player", "attack_run")),
        ];
        let json = format!(
            "{{{}}}",
            ledgers_to_json(&aggregate_ledgers(&events, &BTreeMap::new(), 10.0))
        );
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("ledgers are not valid JSON: {e}\n{json}"));
        assert_eq!(parsed["player"]["phase_seconds"]["acquire"], 6.0);
        assert_eq!(parsed["player"]["phase_seconds"]["attack_run"], 4.0);
    }

    #[test]
    fn repair_applied_accumulates_hull_restored() {
        let events = vec![
            BalanceEvent::RepairApplied {
                ship: "player".into(),
                hp: 2.5,
            },
            BalanceEvent::RepairApplied {
                ship: "player".into(),
                hp: 1.5,
            },
        ];
        let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
        assert_eq!(l.repair_hp, 4.0);
    }

    /// The ndjson timeline drops per-tick repair deltas (they were ~80% of a
    /// combat run's lines) — but the ledger must still total every one of
    /// them, so filtering the *stream* may never touch the fold.
    #[test]
    fn repair_is_kept_out_of_the_timeline_but_still_totalled() {
        let events = vec![
            BalanceEvent::RepairApplied {
                ship: "player".into(),
                hp: 2.0,
            },
            hit(Some("raider"), "player", 1.0, 3.0),
            BalanceEvent::RepairApplied {
                ship: "player".into(),
                hp: 1.0,
            },
        ];
        let streamed: Vec<&BalanceEvent> =
            events.iter().filter(|e| e.in_timeline_stream()).collect();
        assert_eq!(streamed.len(), 1, "only the hit is a timeline beat");
        assert!(matches!(streamed[0], BalanceEvent::DamageApplied { .. },));
        let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
        assert_eq!(l.repair_hp, 3.0, "the fold still sees every repair tick");
    }

    /// Everything other than the per-tick repair delta is a story beat and
    /// belongs in the timeline. Written as an exhaustive list so a new variant
    /// forces a deliberate decision rather than silently defaulting in.
    #[test]
    fn every_other_variant_stays_in_the_timeline() {
        let cases = vec![
            hit(Some("a"), "b", 1.0, 1.0),
            BalanceEvent::WeaponFired {
                shooter: Some("a".into()),
                weapon: "port".into(),
                kind: FIRED_KIND_BEAM.into(),
            },
            BalanceEvent::ShieldArcCollapsed {
                ship: "b".into(),
                arc_id: "fore".into(),
            },
            BalanceEvent::SystemTierCrossed {
                ship: "b".into(),
                system_id: "helm".into(),
                from_tier: "Operational".into(),
                to_tier: "Disabled".into(),
            },
            BalanceEvent::Disarmed { ship: "b".into() },
            BalanceEvent::EntityDestroyed {
                victim: "b".into(),
                killer: Some("a".into()),
            },
            BalanceEvent::RedAlertChanged {
                ship: "a".into(),
                on: true,
            },
            BalanceEvent::ObjectiveCompleted {
                objective_id: "reach_beacon".into(),
            },
            BalanceEvent::PhaseChanged {
                from: "Lobby".into(),
                to: "InProgress".into(),
            },
            BalanceEvent::DoctrinePhaseChanged {
                ship: "a".into(),
                phase: "attack_run".into(),
            },
        ];
        // Anti-vacuity: the list above is one per variant *except*
        // `RepairApplied`, which is the only intentional exclusion. Pinning the
        // count is what makes "a new variant forces a deliberate decision" true
        // rather than aspirational — add a variant and this fails until you have
        // said, here, which side of the timeline it belongs on.
        assert_eq!(
            cases.len(),
            BalanceEvent::VARIANT_COUNT - 1,
            "every BalanceEvent variant but RepairApplied must be covered here"
        );
        for event in cases {
            assert!(event.in_timeline_stream(), "{event:?} left the timeline");
        }
    }

    #[test]
    fn ledger_json_is_parseable_and_ordered_by_uuid() {
        let events = vec![
            stamp(1, 0.1, hit(Some("zulu"), "alpha", 1.0, 2.0)),
            stamp(2, 0.2, hit(Some("alpha"), "zulu", 0.5, 0.0)),
            stamp(
                3,
                0.3,
                BalanceEvent::WeaponFired {
                    shooter: Some("alpha".into()),
                    weapon: "port".into(),
                    kind: FIRED_KIND_BEAM.into(),
                },
            ),
            stamp(
                4,
                0.4,
                BalanceEvent::EntityDestroyed {
                    victim: "zulu".into(),
                    killer: Some("alpha".into()),
                },
            ),
        ];
        let names: BTreeMap<String, String> = [("alpha".to_string(), "Ironveil".to_string())]
            .into_iter()
            .collect();
        let json = format!(
            "{{{}}}",
            ledgers_to_json(&aggregate_ledgers(&events, &names, 0.4))
        );
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("ledgers are not valid JSON: {e}\n{json}"));

        assert_eq!(parsed["alpha"]["name_id"], "Ironveil");
        assert_eq!(parsed["alpha"]["damage_taken"], 3.0);
        assert_eq!(parsed["alpha"]["damage_dealt"], 0.5);
        assert_eq!(parsed["alpha"]["by_weapon"]["fore_phaser"], 0.5);
        assert_eq!(parsed["alpha"]["by_pair"]["zulu"], 0.5);
        assert_eq!(parsed["alpha"]["shots_fired"]["port"], 1);
        assert_eq!(parsed["alpha"]["kills"], 1);
        assert!(parsed["zulu"]["name_id"].is_null());
        assert_eq!(parsed["zulu"]["death"][0], 4);
        assert!(json.find("\"alpha\"").unwrap() < json.find("\"zulu\"").unwrap());
    }

    #[test]
    fn damage_event_json_carries_the_split_and_nulls_the_unknowns() {
        let event = BalanceEvent::DamageApplied {
            attacker: None,
            victim: "player".into(),
            victim_kind: VictimKind::Ship,
            weapon: WEAPON_KIND_COLLISION.into(),
            amount: 12.0,
            shield_absorbed: 4.0,
            hull_damage: 8.0,
            system_hit: None,
        };
        let parsed: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(parsed["event"], "DamageApplied");
        assert_eq!(parsed["victim_kind"], "ship");
        assert!(parsed["attacker"].is_null());
        assert!(parsed["system_hit"].is_null());
        assert_eq!(parsed["victim"], "player");
        assert_eq!(parsed["weapon"], "collision");
        assert_eq!(parsed["shield_absorbed"], 4.0);
        assert_eq!(parsed["hull_damage"], 8.0);
    }

    /// Every new variant round-trips through `to_json` as parseable JSON that
    /// names the variant and carries its fields.
    #[test]
    fn new_variants_encode_parseable_json() {
        let cases = vec![
            BalanceEvent::WeaponFired {
                shooter: Some("player".into()),
                weapon: "port".into(),
                kind: FIRED_KIND_BEAM.into(),
            },
            BalanceEvent::ShieldArcCollapsed {
                ship: "player".into(),
                arc_id: "fore".into(),
            },
            BalanceEvent::SystemTierCrossed {
                ship: "raider".into(),
                system_id: "helm".into(),
                from_tier: "Operational".into(),
                to_tier: "Disabled".into(),
            },
            BalanceEvent::Disarmed {
                ship: "raider".into(),
            },
            BalanceEvent::EntityDestroyed {
                victim: "raider".into(),
                killer: Some("player".into()),
            },
            BalanceEvent::RedAlertChanged {
                ship: "player".into(),
                on: true,
            },
            BalanceEvent::ObjectiveCompleted {
                objective_id: "reach_beacon".into(),
            },
            BalanceEvent::PhaseChanged {
                from: "Lobby".into(),
                to: "InProgress".into(),
            },
            BalanceEvent::RepairApplied {
                ship: "player".into(),
                hp: 3.5,
            },
            BalanceEvent::DoctrinePhaseChanged {
                ship: "player".into(),
                phase: "acquire".into(),
            },
        ];
        for event in cases {
            let json = event.to_json();
            let parsed: serde_json::Value = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("variant JSON is not parseable: {e}\n{json}"));
            assert!(parsed["event"].is_string(), "event tag missing in {json}");
        }
    }

    #[test]
    fn weapon_fired_and_destroyed_json_shape() {
        let fired = BalanceEvent::WeaponFired {
            shooter: None,
            weapon: "fore_tube".into(),
            kind: FIRED_KIND_TORPEDO.into(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&fired.to_json()).unwrap();
        assert_eq!(parsed["event"], "WeaponFired");
        assert!(parsed["shooter"].is_null());
        assert_eq!(parsed["weapon"], "fore_tube");
        assert_eq!(parsed["kind"], "torpedo");

        let killed = BalanceEvent::EntityDestroyed {
            victim: "raider".into(),
            killer: Some("player".into()),
        };
        let parsed: serde_json::Value = serde_json::from_str(&killed.to_json()).unwrap();
        assert_eq!(parsed["killer"], "player");
        assert_eq!(parsed["victim"], "raider");
    }

    // ── Run outcome classification (issue #843) ────────────────────────────

    #[test]
    fn outcome_parse_is_case_insensitive_and_rejects_junk() {
        assert_eq!(Outcome::parse("victory"), Ok(Outcome::Victory));
        assert_eq!(Outcome::parse("  Victory "), Ok(Outcome::Victory));
        assert_eq!(Outcome::parse("DEFEAT"), Ok(Outcome::Defeat));
        assert!(Outcome::parse("draw").is_err());
        assert!(Outcome::parse("").is_err());
    }

    fn margins(hull: f32, hull_max: f32, dealt: f32, taken: f32, closing: f32) -> SideMargins {
        SideMargins::new(hull, hull_max, dealt, taken, closing)
    }

    /// A run that reached GameOver with a declared victory flag is a victory.
    #[test]
    fn classify_victory_from_game_over_flag() {
        let report = classify(
            true,
            Some(Outcome::Victory),
            margins(80.0, 100.0, 200.0, 40.0, 0.0),
            margins(0.0, 0.0, 40.0, 200.0, 0.0),
        );
        assert_eq!(report.outcome, RunOutcome::Victory);
        // A scripted end with no declared outcome also reads as victory.
        assert_eq!(
            classify(true, None, SideMargins::default(), SideMargins::default()).outcome,
            RunOutcome::Victory
        );
    }

    /// The built-in player-death latch (Defeat) wins over the default.
    #[test]
    fn classify_defeat_from_game_over_flag() {
        let report = classify(
            true,
            Some(Outcome::Defeat),
            margins(0.0, 100.0, 30.0, 220.0, 0.0),
            margins(60.0, 100.0, 220.0, 30.0, 0.0),
        );
        assert_eq!(report.outcome, RunOutcome::Defeat);
    }

    /// Budget exhausted with damage still landing in the closing window is a
    /// timeout, not a draw.
    #[test]
    fn classify_timeout_when_closing_window_is_live() {
        let report = classify(
            false,
            None,
            margins(55.0, 100.0, 120.0, 90.0, 3.5),
            margins(40.0, 100.0, 90.0, 120.0, 2.0),
        );
        assert_eq!(report.outcome, RunOutcome::Timeout);
        // Margins survive onto the report for AC2.
        assert_eq!(report.player.remaining_hull_fraction, 0.55);
        assert_eq!(report.enemy.closing_damage_rate, 2.0);
    }

    /// Budget exhausted with a dead closing window (mutual ineffectiveness) is
    /// a draw.
    #[test]
    fn classify_draw_when_closing_window_is_silent() {
        let report = classify(
            false,
            None,
            margins(70.0, 100.0, 10.0, 8.0, 0.0),
            margins(65.0, 100.0, 8.0, 10.0, 0.0),
        );
        assert_eq!(report.outcome, RunOutcome::Draw);
        assert_eq!(report.player.damage_dealt, 10.0);
        assert_eq!(report.enemy.damage_taken, 10.0);
    }

    /// The outcome flag only matters once GameOver is reached: an unresolved
    /// run ignores a stray flag and still classifies on the closing window.
    #[test]
    fn classify_ignores_outcome_flag_before_game_over() {
        let live = classify(
            false,
            Some(Outcome::Victory),
            margins(50.0, 100.0, 0.0, 0.0, 5.0),
            margins(50.0, 100.0, 0.0, 0.0, 0.0),
        );
        assert_eq!(live.outcome, RunOutcome::Timeout);
    }

    #[test]
    fn closing_rates_only_count_landed_damage_inside_the_window() {
        let events = vec![
            // Well before the window — ignored.
            stamp(10, 5.0, hit(Some("player"), "raider", 4.0, 6.0)),
            // Inside the 15 s window ending at t=60 (cutoff 45).
            stamp(100, 50.0, hit(Some("player"), "raider", 0.0, 10.0)),
            stamp(110, 55.0, hit(Some("raider"), "player", 5.0, 5.0)),
            // Environmental (no attacker) — belongs to no side, dropped.
            stamp(
                115,
                58.0,
                BalanceEvent::DamageApplied {
                    attacker: None,
                    victim: "player".into(),
                    victim_kind: VictimKind::Ship,
                    weapon: WEAPON_KIND_REGION.into(),
                    amount: 30.0,
                    shield_absorbed: 0.0,
                    hull_damage: 30.0,
                    system_hit: None,
                },
            ),
        ];
        let rates = closing_damage_rates(&events, 60.0, CLOSING_WINDOW_SECS);
        // player: 10 landed / 15 s window.
        assert!((rates["player"] - 10.0 / 15.0).abs() < 1e-4);
        // raider: 10 landed / 15 s window.
        assert!((rates["raider"] - 10.0 / 15.0).abs() < 1e-4);
        assert_eq!(rates.len(), 2, "environmental damage gets no side row");
    }

    #[test]
    fn closing_rates_are_empty_for_a_nonpositive_window() {
        let events = vec![stamp(1, 59.0, hit(Some("player"), "raider", 0.0, 10.0))];
        assert!(closing_damage_rates(&events, 60.0, 0.0).is_empty());
    }

    #[test]
    fn outcome_report_json_carries_outcome_and_both_sides() {
        let report = classify(
            false,
            None,
            margins(55.0, 100.0, 120.0, 90.0, 3.5),
            margins(40.0, 80.0, 90.0, 120.0, 2.0),
        );
        let json = format!("{{{}}}", report.to_json());
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("outcome report is not valid JSON: {e}\n{json}"));
        assert_eq!(parsed["outcome"], "timeout");
        assert_eq!(parsed["sides"]["player"]["remaining_hull"], 55.0);
        assert_eq!(parsed["sides"]["player"]["closing_damage_rate"], 3.5);
        assert_eq!(parsed["sides"]["enemy"]["remaining_hull_max"], 80.0);
        assert_eq!(parsed["sides"]["enemy"]["damage_dealt"], 90.0);
    }
}
