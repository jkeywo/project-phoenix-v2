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
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
    /// A mission objective actually entered one of its lifecycle states.
    ///
    /// Unlike [`ObjectiveCompleted`](Self::ObjectiveCompleted), which remains
    /// for balance-report compatibility, this carries the complete lifecycle
    /// and the objective's authored targets for presentation projections.
    ObjectiveChanged {
        /// Stable authored objective id.
        objective_id: String,
        /// New state after the successful transition.
        status: crate::core::messages::ObjectiveStatus,
        /// Authored target names/UUIDs. Consumers resolve names through the
        /// world's existing public identity table rather than inventing ids.
        targets: Vec<String>,
    },
    /// One actual trigger firing at the shared evaluator seam.
    TriggerFired {
        /// Authored trigger id, or the stable `script_path::fn_name` fallback
        /// for an anonymous registration. Never a mutable vector index.
        trigger_id: String,
        /// Content-relative script path that registered the handler.
        origin: String,
        /// Stable UUID involved in the condition when one was resolved.
        entity: Option<String>,
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
    pub const VARIANT_COUNT: usize = 13;

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
            BalanceEvent::ObjectiveChanged {
                objective_id,
                status,
                targets,
            } => {
                let status = match status {
                    crate::core::messages::ObjectiveStatus::Active => "active",
                    crate::core::messages::ObjectiveStatus::Completed => "completed",
                    crate::core::messages::ObjectiveStatus::Failed => "failed",
                };
                format!(
                    "{{\"event\":\"ObjectiveChanged\",\"objective_id\":{objective_id:?},\"status\":{status:?},\"targets\":{targets:?}}}"
                )
            }
            BalanceEvent::TriggerFired {
                trigger_id,
                origin,
                entity,
            } => format!(
                "{{\"event\":\"TriggerFired\",\"trigger_id\":{trigger_id:?},\"origin\":{origin:?},\"entity\":{}}}",
                opt_string(entity),
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
            | BalanceEvent::ObjectiveChanged { .. }
            | BalanceEvent::TriggerFired { .. }
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
    /// The ending carried a structured post-mission report (issue #1344): the
    /// scenario wrote at least one [`crate::core::report::ReportRow`] before it
    /// ended, so what the run came away with is the ROWS and not a single word.
    ///
    /// It outranks victory and defeat rather than sitting beside them, because
    /// the two answer different questions and only one of them can be the
    /// headline. A Falling Skyway run that pulls Lyra clear and then loses the
    /// skyway to the Lark is a declared `defeat` AND a report holding a saved
    /// row; framing it as DEFEAT throws the row away, which is the exact
    /// failure the report exists to fix. The declared [`Outcome`] is not lost —
    /// it is still latched on `GameOverReason`, still folded into the digest,
    /// and still on the wire — it simply stops being the frame.
    Reported,
}

impl RunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            RunOutcome::Victory => "victory",
            RunOutcome::Defeat => "defeat",
            RunOutcome::Draw => "draw",
            RunOutcome::Timeout => "timeout",
            RunOutcome::Reported => "reported",
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
    pub fn to_json(&self) -> String {
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

pub use phoenix_sim_contracts::outcome::Outcome;
