//! Run telemetry.
//!
//! The message stream is tapped at `OutboundMessage` and encoded with the same
//! [`JsonCodec`] the browser bridge uses, so what a headless run reports is the
//! real wire protocol rather than a parallel view of internal state. A test
//! asserting on this is asserting on something a player would actually receive.

use bevy::prelude::*;
use std::collections::BTreeMap;

use crate::core::balance::{
    aggregate_ledgers, classify, closing_damage_rates, ledgers_to_json, BalanceEvent, DamageLedger,
    OutcomeReport, SideMargins, StampedBalanceEvent, CLOSING_WINDOW_SECS,
};
use crate::core::codec::{JsonCodec, MessageCodec};
use crate::core::messages::{GamePhase, ServerMessage, ServerMessageDiscriminants};
use crate::debug::payload::StationActivityPayload;
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid, FactionComponent};
use crate::lobby::OutboundMessage;
use crate::server_app::{GameOverReason, LocalShip, Ship};
use crate::ship::damage::DamageTier;
use crate::ship::state::ShipPhysics;
use crate::sim_rng::SeedSource;

use super::args::{HeadlessArgs, ReportFormat};

/// The telemetry accumulator, re-exported from its portable home.
///
/// The type moved to `crate::core::telemetry` (issue #904) so that
/// `crate::sim_digest` — which folds this resource's collision attribution —
/// can compile for `wasm32`, where the `headless` module does not exist. The
/// collector systems below and the report builder that reads them stay here;
/// only the plain-field struct moved. Every existing
/// `headless::report::RunTelemetry` path still resolves through this
/// re-export.
pub use crate::core::telemetry::RunTelemetry;

/// `ServerMessage`'s variant name, for counting.
///
/// Taken from the `strum` discriminant rather than by scraping the encoded
/// JSON: `ServerMessage` is internally tagged (`#[serde(tag = "type")]`), so
/// the variant is a *value* inside the object, not the key, and any
/// key-scraping approach just reports `"type"` for everything.
fn variant_name(msg: &ServerMessage) -> String {
    format!("{:?}", ServerMessageDiscriminants::from(msg))
}

/// Records every outbound message. Runs in `Last` so it sees the whole tick's
/// traffic regardless of which `SimSet` produced it.
///
/// Stamped with `Res<SimTick>` (issue #895 re-review), not a per-`update()`
/// counter: this system runs once per FRAME, and a frame can run zero or more
/// logical sim ticks (2 at `--hz 30` against the shipped `sim_tick_hz = 60`),
/// so a frame counter silently folded that frame's worth of ticks into one
/// stamp. `SimTick` is the real counter, and this system running once per
/// frame rather than once per tick means every message this frame's `Last`
/// pass sees still shares the one `SimTick` value current at that point (the
/// most recently completed tick) — imprecise across a multi-tick frame, but
/// no longer a meaningless frame index.
pub fn collect_outbound(
    mut telemetry: ResMut<RunTelemetry>,
    mut reader: MessageReader<OutboundMessage>,
    time: Res<Time>,
    sim_tick: Res<crate::sim_tick::SimTick>,
) {
    let codec = JsonCodec;
    let tick = sim_tick.0;
    let sim_t = time.elapsed_secs_f64();
    for out in reader.read() {
        *telemetry
            .message_counts
            .entry(variant_name(&out.msg))
            .or_insert(0) += 1;
        if telemetry.capture_stream {
            let Ok(encoded) = codec.encode_server(&out.msg) else {
                continue;
            };
            telemetry.stream.push(format!(
                "{{\"tick\":{tick},\"sim_t\":{sim_t:.4},\"msg\":{encoded}}}"
            ));
        }
    }
}

/// The ship uuids a balance event names, so their `EntityName` can be
/// snapshotted before the ship despawns. Ship-agnostic variants (phase,
/// objective) name none.
fn referenced_ship_uuids(event: &BalanceEvent) -> Vec<&String> {
    let mut ids = Vec::new();
    match event {
        BalanceEvent::DamageApplied {
            attacker, victim, ..
        } => {
            ids.extend(attacker.iter());
            ids.push(victim);
        }
        BalanceEvent::WeaponFired { shooter, .. } => ids.extend(shooter.iter()),
        BalanceEvent::ShieldArcCollapsed { ship, .. }
        | BalanceEvent::SystemTierCrossed { ship, .. }
        | BalanceEvent::Disarmed { ship }
        | BalanceEvent::RedAlertChanged { ship, .. }
        | BalanceEvent::RepairApplied { ship, .. }
        | BalanceEvent::DoctrinePhaseChanged { ship, .. } => ids.push(ship),
        BalanceEvent::EntityDestroyed { victim, killer } => {
            ids.push(victim);
            ids.extend(killer.iter());
        }
        BalanceEvent::ObjectiveCompleted { .. } | BalanceEvent::PhaseChanged { .. } => {}
    }
    ids
}

/// Records every balance event, alongside the `EntityName` of any ship it
/// names. Runs in `Last` for the same reason as [`collect_outbound`]: the
/// chokepoints are spread across several `SimSet`s. Stamped with
/// `Res<SimTick>` for the same reason too — see that function's doc.
pub fn collect_balance_events(
    mut telemetry: ResMut<RunTelemetry>,
    mut reader: MessageReader<BalanceEvent>,
    time: Res<Time>,
    sim_tick: Res<crate::sim_tick::SimTick>,
    named_q: Query<(&EntityUuid, &EntityName)>,
    faction_q: Query<(&EntityUuid, &crate::entities::spawner::FactionComponent)>,
) {
    let tick = sim_tick.0;
    let sim_t = time.elapsed_secs_f64();
    for event in reader.read() {
        // Snapshot the `EntityName` and faction of every ship any variant
        // names, while the ship is still alive — a destroyed NPC is gone before
        // the report is built. Every variant contributes whichever uuids it
        // carries.
        for uuid in referenced_ship_uuids(event) {
            if !telemetry.entity_names.contains_key(uuid) {
                if let Some((_, name)) = named_q.iter().find(|(u, _)| &u.0 == uuid) {
                    telemetry.entity_names.insert(uuid.clone(), name.0.clone());
                }
            }
            if !telemetry.entity_factions.contains_key(uuid) {
                if let Some((_, fac)) = faction_q.iter().find(|(u, _)| &u.0 == uuid) {
                    telemetry
                        .entity_factions
                        .insert(uuid.clone(), fac.0.to_string());
                }
            }
        }
        // The ledger sees every event; the ndjson timeline sees only the ones
        // that read as story beats (`BalanceEvent::in_timeline_stream`). This
        // is the only difference between the two, and it exists so per-tick
        // repair deltas stop drowning the timeline while `repair_hp` stays
        // exact.
        if telemetry.capture_stream && event.in_timeline_stream() {
            let encoded = event.to_json();
            telemetry.stream.push(format!(
                "{{\"tick\":{tick},\"sim_t\":{sim_t:.4},\"balance\":{encoded}}}"
            ));
        }
        telemetry.balance_events.push(StampedBalanceEvent {
            tick,
            sim_t,
            event: event.clone(),
        });
    }
}

/// Final state of the player ship.
#[derive(Debug, Clone, Default)]
pub struct ShipSummary {
    pub name: Option<String>,
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub forward_speed: f32,
    pub hull_current: f32,
    pub hull_max: f32,
    /// Systems not at `Operational`, as `system_id -> tier`.
    pub damaged_systems: BTreeMap<String, String>,
}

/// The exit summary.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub ticks: u64,
    pub sim_seconds: f64,
    /// The master RNG seed the run actually used. Always present, seeded or
    /// not, so any run can be replayed with `--seed`.
    pub seed: u64,
    /// Where `seed` came from: `"cli"`, `"world"`, or `"random"`.
    pub seed_source: String,
    pub wall_seconds: f64,
    pub ticks_per_second: f64,
    pub final_phase: String,
    pub game_over_reason: Option<String>,
    pub entity_count: usize,
    pub ship: Option<ShipSummary>,
    pub message_counts: BTreeMap<String, u64>,
    /// Per-ship damage ledgers, keyed by uuid. Built from the balance-event
    /// log rather than from the world, so ships destroyed mid-run still
    /// appear.
    pub damage_by_ship: BTreeMap<String, DamageLedger>,
    /// The classified run outcome (victory | defeat | draw | timeout) plus the
    /// per-side margins (#843). Present in *every* report (AC1); draw/timeout
    /// lean on the margins (AC2).
    pub outcome_report: OutcomeReport,
    /// The AI doctrine-pool debug surface as JSON (issue #1149, PRD #1144): each
    /// AI ship's scored-objective pool with every candidate's score, chosen
    /// directive and resolved target. A one-shot read-only projection off the
    /// final world (see [`build_report`]), so a seeded sweep captures *why* the AI
    /// went the way it did. Empty string when no projection was run (the test
    /// constructors); rendered as `null` then.
    pub ai_doctrine: String,
    /// Always-on per-station admitted-command activity (PRD #1144, issue
    /// #1147): the bounded, per-bucket, per-control-source series the
    /// station-activity tracker held at run end. Present in *every* report with
    /// no debug flag — `build_report` reads the tracker's read-only projection
    /// directly, not the flag-gated capture, so the report is the always-on
    /// native/headless output path for the payload the browser host publishes to
    /// its dock. Empty (`buckets: []`) for a run that admitted no station
    /// commands.
    pub station_activity: StationActivityPayload,
}

impl RunReport {
    /// Whether this run should be treated as a failure under
    /// `--fail-on-game-over`.
    pub fn ended_in_game_over(&self) -> bool {
        self.final_phase == format!("{:?}", GamePhase::GameOver)
    }

    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n");
        s.push_str(&format!("  \"ticks\": {},\n", self.ticks));
        s.push_str(&format!("  \"sim_seconds\": {:.4},\n", self.sim_seconds));
        s.push_str(&format!("  \"seed\": {},\n", self.seed));
        s.push_str(&format!("  \"seed_source\": {:?},\n", self.seed_source));
        s.push_str(&format!("  \"wall_seconds\": {:.4},\n", self.wall_seconds));
        s.push_str(&format!(
            "  \"ticks_per_second\": {:.1},\n",
            self.ticks_per_second
        ));
        s.push_str(&format!(
            "  \"speedup_vs_realtime\": {:.1},\n",
            if self.wall_seconds > 0.0 {
                self.sim_seconds / self.wall_seconds
            } else {
                0.0
            }
        ));
        s.push_str(&format!("  \"final_phase\": \"{}\",\n", self.final_phase));
        s.push_str(&format!(
            "  \"game_over_reason\": {},\n",
            match &self.game_over_reason {
                Some(r) => format!("{:?}", r),
                None => "null".to_string(),
            }
        ));
        s.push_str(&format!("  \"entity_count\": {},\n", self.entity_count));
        match &self.ship {
            None => s.push_str("  \"ship\": null,\n"),
            Some(ship) => {
                s.push_str("  \"ship\": {\n");
                s.push_str(&format!(
                    "    \"name\": {},\n",
                    match &ship.name {
                        Some(n) => format!("{:?}", n),
                        None => "null".to_string(),
                    }
                ));
                s.push_str(&format!(
                    "    \"position\": [{:.3}, {:.3}],\n    \"yaw\": {:.4},\n    \"forward_speed\": {:.3},\n",
                    ship.x, ship.z, ship.yaw, ship.forward_speed
                ));
                s.push_str(&format!(
                    "    \"hull\": [{:.1}, {:.1}],\n",
                    ship.hull_current, ship.hull_max
                ));
                s.push_str(&format!(
                    "    \"damaged_systems\": {{{}}}\n",
                    ship.damaged_systems
                        .iter()
                        .map(|(k, v)| format!("{:?}: {:?}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                s.push_str("  },\n");
            }
        }
        s.push_str(&format!(
            "  \"message_counts\": {{{}}},\n",
            self.message_counts
                .iter()
                .map(|(k, v)| format!("{:?}: {}", k, v))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        s.push_str(&format!(
            "  \"damage_by_ship\": {{{}}},\n",
            ledgers_to_json(&self.damage_by_ship)
        ));
        // The AI doctrine-pool surface (issue #1149). Already a JSON object
        // (`codec::encode_ai_doctrine`), so it slots in verbatim; an empty string
        // (a test constructor that ran no projection) renders as `null` so the
        // report is always valid JSON.
        let ai_doctrine = if self.ai_doctrine.is_empty() {
            "null"
        } else {
            self.ai_doctrine.as_str()
        };
        s.push_str(&format!("  \"ai_doctrine\": {ai_doctrine},\n"));
        // The always-on station-activity series (issue #1147). Encoded through
        // the one `serde_json` seam (AGENTS.md Key Constraint 1) rather than
        // hand-rolled here — the payload is a plain serde struct, and reusing
        // `encode_station_activity` keeps the report's JSON byte-identical with
        // what the dock chart parses.
        s.push_str(&format!(
            "  \"station_activity\": {},\n",
            crate::core::codec::encode_station_activity(&self.station_activity)
        ));
        // `OutcomeReport::to_json` emits `"outcome": ..., "sides": {...}` as a
        // body, so it slots straight in as the final two report fields.
        s.push_str(&format!("  {}\n", self.outcome_report.to_json()));
        s.push('}');
        s
    }
}

/// The `wall_seconds` a report should carry, given where its seed came from.
///
/// `wall_seconds` — and the two fields derived from it, `ticks_per_second` and
/// `speedup_vs_realtime` — are measured off the host clock, so two replays of
/// the same run differed on exactly those three lines and nothing else. That
/// defeats the point of a replay: a designer A/B-ing a tuning change wants
/// `diff a.json b.json` to be empty unless the *simulation* moved. So a
/// reproducible run drops the measurement and reports all three as 0.
///
/// **Only a `--seed` run counts as reproducible**, because only `--seed`
/// implies `--deterministic` and pins the single-threaded scheduler. A run
/// seeded from the world TOML draws the same numbers from the same streams but
/// still executes on the default thread pool, so system order — and with it the
/// report — can drift run to run. Handing that run a tidy empty diff would
/// overclaim a reproducibility it does not have. Zeroed timings mean exactly
/// "this run is replayable"; everything else keeps the real figures, which is
/// also where perf numbers belong.
///
/// Zeroed rather than omitted so the JSON schema is identical either way —
/// `scripts/balance-runs.mjs` reads none of the three today, and a consumer
/// that starts to should not have to branch on seededness.
fn reported_wall_seconds(seed_source: &str, wall_seconds: f64) -> f64 {
    if seed_source == SeedSource::Cli.as_str() {
        0.0
    } else {
        wall_seconds
    }
}

/// Read the finished world and produce the summary.
pub fn build_report(app: &mut App, args: &HeadlessArgs, wall_seconds: f64) -> RunReport {
    let telemetry = app.world().resource::<RunTelemetry>();
    let message_counts = telemetry.message_counts.clone();
    let final_sim_t = app.world().resource::<Time>().elapsed_secs_f64();
    // Pure fold: the ledgers come from the stamped event log and the names
    // captured alongside it, never from the world. Stamped so the fold can
    // name each death's tick, each knockout's sim-time, and close each ship's
    // open doctrine-phase interval at the end of the run.
    let damage_by_ship = aggregate_ledgers(
        &telemetry.balance_events,
        &telemetry.entity_names,
        final_sim_t,
    );

    // The logical sim-tick count (issue #895 re-review): this used to be
    // `RunTelemetry`'s own per-`update()` frame counter, but a headless run's
    // `--hz` frame rate and the world's `[global] sim_tick_hz` are
    // independent — at `--hz 30` against the shipped `sim_tick_hz = 60` every
    // frame runs TWO fixed steps, so a frame counter silently reported half
    // the number of ticks the simulation actually ran. `SimTick` is the real
    // counter every other tick-keyed artifact (command stamps, the AI
    // cadence) already keys on.
    let ticks = app.world().resource::<crate::sim_tick::SimTick>().0;

    // Read off the resource rather than off `args`: `args.seed` is only the
    // CLI tier of the precedence chain, and the report has to name the seed
    // that was actually used whichever tier supplied it.
    let (seed, seed_source) = app
        .world()
        .get_resource::<crate::sim_rng::SimRng>()
        .map(|r| (r.seed(), r.source().as_str().to_string()))
        .unwrap_or((0, "absent".to_string()));

    let wall_seconds = reported_wall_seconds(&seed_source, wall_seconds);

    let sim_seconds = final_sim_t;
    let final_phase = format!("{:?}", app.world().resource::<State<GamePhase>>().get());
    let is_game_over = final_phase == format!("{:?}", GamePhase::GameOver);
    let game_over_res = app.world().resource::<GameOverReason>();
    let game_over_reason = game_over_res.0.clone();
    // The declared/latched outcome (#843): `Some(Defeat)` at a player-death
    // site, whatever a scenario declared, or `None` for an undeclared scripted
    // end (the classifier defaults that to victory).
    let outcome_flag = game_over_res.1;
    let entity_count = app.world().entities().len() as usize;

    // Closing-window landed-damage rates per attacker uuid, and a snapshot of
    // each combatant's faction — both read off telemetry before any `world_mut`
    // query below re-borrows the world. `entity_factions` covers ships that
    // died mid-run (gone from the ECS); surviving ships are read authoritatively
    // from the world just below.
    let closing_rates =
        closing_damage_rates(&telemetry.balance_events, sim_seconds, CLOSING_WINDOW_SECS);
    let entity_factions = telemetry.entity_factions.clone();

    let mut ship_q = app.world_mut().query_filtered::<(
        &ShipPhysics,
        Option<&EntityName>,
        Option<&EntitySystemHull>,
    ), With<LocalShip>>();
    let ship = ship_q.single(app.world()).ok().map(|(phys, name, hull)| {
        let mut summary = ShipSummary {
            name: name.map(|n| n.0.clone()),
            x: phys.x,
            z: phys.z,
            yaw: phys.yaw,
            forward_speed: phys.forward_speed,
            ..Default::default()
        };
        if let Some(hull) = hull {
            summary.hull_current = hull.0.total_current();
            summary.hull_max = hull.0.total_max();
            for (sid, _entry) in hull.0.iter() {
                let tier = hull.0.tier_for(sid);
                if tier != DamageTier::Operational {
                    summary
                        .damaged_systems
                        .insert(sid.0.clone(), format!("{tier:?}"));
                }
            }
        }
        summary
    });

    let outcome_report = build_outcome_report(
        app,
        is_game_over,
        outcome_flag,
        &damage_by_ship,
        &closing_rates,
        &entity_factions,
    );

    // The AI doctrine-pool surface (issue #1149): a one-shot read-only projection
    // off the finished world, so every headless report carries *why* the AI went
    // the way it did — independent of the live debug flag, which drives the dock
    // and the determinism guard rather than the report.
    let ai_doctrine = collect_ai_doctrine_json(app, ticks);
    // Always-on station activity (issue #1147): the tracker's read-only
    // projection, read straight off the resource rather than through the
    // flag-gated `StationActivityCapture`, so every report carries the
    // per-station busyness split whether or not the debug surface was rendered
    // this run. `unwrap_or_default` covers a bare fixture app that never added
    // `DebugPlugin` — production always has the tracker.
    let station_activity = app
        .world()
        .get_resource::<crate::debug::StationActivityTracker>()
        .map(|tracker| tracker.report())
        .unwrap_or_default();

    RunReport {
        ticks,
        sim_seconds,
        seed,
        seed_source,
        wall_seconds,
        ticks_per_second: if wall_seconds > 0.0 {
            ticks as f64 / wall_seconds
        } else {
            0.0
        },
        final_phase,
        game_over_reason,
        entity_count,
        ship,
        message_counts,
        damage_by_ship,
        outcome_report,
        ai_doctrine,
        station_activity,
    }
    .tap_stream(app, args)
}

/// Project the AI doctrine pool off the finished world into the report's JSON
/// (issue #1149).
///
/// Read-only, and it runs after the sim has stopped, so it cannot perturb the
/// run: it reads each `BehaviourSection` ship's viewscreen scored-objective pool
/// — the same set `debug::ai_state::publish_ai_doctrine` covers live — and folds
/// it through the shared `collect_ai_doctrine` projector so the report and the
/// dock speak the identical schema.
fn collect_ai_doctrine_json(app: &mut App, tick: u64) -> String {
    use crate::server_app::ShipSystemBlackboards;

    let mut q = app.world_mut().query_filtered::<(
        &ShipSystemBlackboards,
        Option<&EntityName>,
        Option<&EntityUuid>,
    ), With<crate::entities::spawner::BehaviourSection>>();
    let ships: Vec<(
        String,
        Option<String>,
        Vec<crate::core::messages::ScoredObjective>,
    )> = q
        .iter(app.world())
        .map(|(blackboards, name, uuid)| {
            (
                name.map(|n| n.0.clone())
                    .unwrap_or_else(|| "<unnamed>".to_string()),
                uuid.map(|u| u.0.clone()),
                crate::debug::ai_state::ship_scored_pool(blackboards),
            )
        })
        .collect();
    let payload = crate::debug::ai_state::collect_ai_doctrine(tick, ships);
    crate::core::codec::encode_ai_doctrine(&payload)
}

/// Bucket ships by side relative to the `LocalShip`, sum the margins, and hand
/// them to the pure [`classify`].
///
/// "Sides" are faction groupings: the **player** side is every ship the
/// `LocalShip`'s faction does *not* consider an enemy (the LocalShip itself,
/// allies, and factionless neutrals); the **enemy** side is every ship it does
/// consider an enemy (`is_enemy` via the `FactionRegistry`). Surviving ships
/// give the remaining-hull margins straight from the ECS; damage totals and
/// closing rates come from the ledgers/telemetry (which include ships that have
/// already despawned), mapped to a side via each uuid's captured faction.
fn build_outcome_report(
    app: &mut App,
    is_game_over: bool,
    outcome_flag: Option<crate::core::balance::Outcome>,
    damage_by_ship: &BTreeMap<String, DamageLedger>,
    closing_rates: &BTreeMap<String, f32>,
    entity_factions: &BTreeMap<String, String>,
) -> OutcomeReport {
    use uuid::Uuid;

    // The LocalShip's own faction anchors every side decision.
    let local_faction: Option<Uuid> = {
        let mut lq = app
            .world_mut()
            .query_filtered::<&FactionComponent, With<LocalShip>>();
        lq.iter(app.world()).next().map(|f| f.0)
    };

    // Surviving ships: (uuid, hull_current, hull_max, faction). Collected into
    // an owned Vec so the world borrow is released before the registry read.
    let mut ship_q = app
        .world_mut()
        .query_filtered::<(&EntityUuid, &EntitySystemHull, Option<&FactionComponent>), With<Ship>>(
        );
    let surviving: Vec<(String, f32, f32, Option<Uuid>)> = ship_q
        .iter(app.world())
        .map(|(uuid, hull, fac)| {
            (
                uuid.0.clone(),
                hull.0.total_current(),
                hull.0.total_max(),
                fac.map(|f| f.0),
            )
        })
        .collect();

    // Faction of every uuid the margins touch: surviving ships are
    // authoritative; dead ships fall back to the telemetry snapshot.
    let mut faction_of: BTreeMap<String, Option<Uuid>> = BTreeMap::new();
    for (uuid, _, _, fac) in &surviving {
        faction_of.insert(uuid.clone(), *fac);
    }
    for (uuid, fac_str) in entity_factions {
        faction_of
            .entry(uuid.clone())
            .or_insert_with(|| Uuid::parse_str(fac_str).ok());
    }

    // `true` when the LocalShip's faction considers `fac` an enemy. Everything
    // else (allies, neutrals, the LocalShip itself) is the player side.
    let registry = app
        .world()
        .get_resource::<crate::entities::config_cache::FactionRegistryResource>();
    let is_enemy_side = |fac: Option<Uuid>| -> bool {
        registry
            .map(|reg| crate::ai::faction::is_enemy(local_faction, fac, &reg.0))
            .unwrap_or(false)
    };

    // Remaining hull, per side, from the surviving ships.
    let (mut p_hull, mut p_hull_max, mut e_hull, mut e_hull_max) = (0.0, 0.0, 0.0, 0.0);
    for (_, cur, max, fac) in &surviving {
        if is_enemy_side(*fac) {
            e_hull += cur;
            e_hull_max += max;
        } else {
            p_hull += cur;
            p_hull_max += max;
        }
    }

    // Cumulative damage each way, from the ledgers (dead ships included).
    let (mut p_dealt, mut p_taken, mut e_dealt, mut e_taken) = (0.0, 0.0, 0.0, 0.0);
    for (uuid, ledger) in damage_by_ship {
        let fac = faction_of.get(uuid).copied().flatten();
        if is_enemy_side(fac) {
            e_dealt += ledger.damage_dealt;
            e_taken += ledger.damage_taken;
        } else {
            p_dealt += ledger.damage_dealt;
            p_taken += ledger.damage_taken;
        }
    }

    // Closing-window rate, per side, from the per-attacker rates.
    let (mut p_closing, mut e_closing) = (0.0, 0.0);
    for (uuid, rate) in closing_rates {
        let fac = faction_of.get(uuid).copied().flatten();
        if is_enemy_side(fac) {
            e_closing += rate;
        } else {
            p_closing += rate;
        }
    }

    let player = SideMargins::new(p_hull, p_hull_max, p_dealt, p_taken, p_closing);
    let enemy = SideMargins::new(e_hull, e_hull_max, e_dealt, e_taken, e_closing);
    classify(is_game_over, outcome_flag, player, enemy)
}

impl RunReport {
    /// Ndjson runs print the captured stream ahead of the summary, so a reader
    /// sees events in order and the summary last.
    fn tap_stream(self, app: &App, args: &HeadlessArgs) -> Self {
        if args.report_format == ReportFormat::Ndjson {
            for line in &app.world().resource::<RunTelemetry>().stream {
                println!("{line}");
            }
            // One trailing station-activity record (issue #1147) so a streaming
            // consumer sees the always-on per-station series inline, stamped at
            // the run's final tick / sim-time like every other stream line. The
            // full report printed after the stream carries the same series under
            // `station_activity`; this line puts it in the ndjson flow too.
            println!(
                "{{\"tick\":{},\"sim_t\":{:.4},\"station_activity\":{}}}",
                self.ticks,
                self.sim_seconds,
                crate::core::codec::encode_station_activity(&self.station_activity),
            );
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::balance::VictimKind;

    /// Guards against regressing to JSON key-scraping, which reported every
    /// message as `"type"` because `ServerMessage` is internally tagged.
    #[test]
    fn variant_name_reports_the_variant_not_the_serde_tag() {
        let name = variant_name(&ServerMessage::GameStarted);
        assert_eq!(name, "GameStarted");
        assert_ne!(name, "type");
    }

    /// Zeroed timings are a claim of replayability, so only the one seed tier
    /// that also pins the scheduler (`--seed`, which implies `--deterministic`)
    /// earns them. A world-TOML seed still runs on the default thread pool, so
    /// it keeps its real, honestly-varying figures.
    #[test]
    fn only_a_cli_seed_zeroes_the_host_clock_timings() {
        assert_eq!(reported_wall_seconds("cli", 1.25), 0.0);
        assert_eq!(reported_wall_seconds("world", 1.25), 1.25);
        assert_eq!(reported_wall_seconds("random", 1.25), 1.25);
        // No `SimRng` in the world at all — nothing to replay, so nothing to
        // hide.
        assert_eq!(reported_wall_seconds("absent", 1.25), 1.25);
    }

    /// Anti-vacuity for the test above: the string it keys off is the one
    /// `SeedSource` actually stamps into the report.
    #[test]
    fn the_reproducible_seed_source_is_the_one_the_cli_tier_reports() {
        assert_eq!(SeedSource::Cli.as_str(), "cli");
        assert_eq!(reported_wall_seconds(SeedSource::Cli.as_str(), 3.0), 0.0);
        assert_eq!(reported_wall_seconds(SeedSource::World.as_str(), 3.0), 3.0);
    }

    #[test]
    fn report_json_is_parseable_and_carries_the_headline_numbers() {
        let report = RunReport {
            ticks: 601,
            sim_seconds: 10.0,
            seed: 42,
            seed_source: "cli".into(),
            wall_seconds: 0.5,
            ticks_per_second: 1202.0,
            final_phase: "InProgress".into(),
            game_over_reason: None,
            entity_count: 4,
            ship: Some(ShipSummary {
                name: Some("Alliance Cruiser".into()),
                x: 1.5,
                z: -2.5,
                hull_current: 90.0,
                hull_max: 100.0,
                damaged_systems: [("helm".to_string(), "Damaged".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            message_counts: [("SimState".to_string(), 100u64)].into_iter().collect(),
            damage_by_ship: BTreeMap::new(),
            // Budget-exhausted with a live closing window → timeout, carrying
            // both sides' margins (AC1 + AC2).
            outcome_report: crate::core::balance::classify(
                false,
                None,
                SideMargins::new(90.0, 100.0, 200.0, 40.0, 3.0),
                SideMargins::new(0.0, 100.0, 40.0, 200.0, 1.0),
            ),
            ai_doctrine: String::new(),
            station_activity: StationActivityPayload::default(),
        };
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));
        assert_eq!(parsed["ticks"], 601);
        assert_eq!(parsed["seed"], 42);
        assert_eq!(parsed["seed_source"], "cli");
        assert_eq!(parsed["speedup_vs_realtime"], 20.0);
        assert_eq!(parsed["ship"]["name"], "Alliance Cruiser");
        assert_eq!(parsed["ship"]["damaged_systems"]["helm"], "Damaged");
        assert_eq!(parsed["message_counts"]["SimState"], 100);
        assert!(parsed["game_over_reason"].is_null());
        // An unset doctrine surface (no projection ran) renders as null, keeping
        // the report valid JSON (issue #1149).
        assert!(parsed["ai_doctrine"].is_null());
        // Outcome + per-side margins are always present.
        assert_eq!(parsed["outcome"], "timeout");
        assert_eq!(parsed["sides"]["player"]["remaining_hull_fraction"], 0.9);
        assert_eq!(parsed["sides"]["player"]["damage_dealt"], 200.0);
        assert_eq!(parsed["sides"]["enemy"]["closing_damage_rate"], 1.0);
    }

    #[test]
    fn report_json_is_parseable_with_no_ship() {
        let report = RunReport {
            ticks: 1,
            sim_seconds: 0.0,
            seed: 7,
            seed_source: "world".into(),
            wall_seconds: 0.0,
            ticks_per_second: 0.0,
            final_phase: "GameOver".into(),
            game_over_reason: Some("hull breach".into()),
            entity_count: 0,
            ship: None,
            message_counts: BTreeMap::new(),
            damage_by_ship: BTreeMap::new(),
            // Reached GameOver via the player-death latch → defeat.
            outcome_report: crate::core::balance::classify(
                true,
                Some(crate::core::balance::Outcome::Defeat),
                SideMargins::default(),
                SideMargins::default(),
            ),
            ai_doctrine: String::new(),
            station_activity: StationActivityPayload::default(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert!(parsed["ship"].is_null());
        assert_eq!(parsed["game_over_reason"], "hull breach");
        assert_eq!(parsed["outcome"], "defeat");
        assert!(report.ended_in_game_over());
    }

    /// The per-ship section is built from the balance-event log, so it has to
    /// survive into the report for ships the world no longer contains.
    #[test]
    fn report_json_carries_per_ship_damage_ledgers() {
        let events = [
            BalanceEvent::DamageApplied {
                attacker: Some("player".into()),
                victim: "raider".into(),
                victim_kind: VictimKind::Ship,
                weapon: "fore_phaser".into(),
                amount: 20.0,
                shield_absorbed: 5.0,
                hull_damage: 15.0,
                system_hit: None,
            },
            BalanceEvent::DamageApplied {
                attacker: Some("raider".into()),
                victim: "player".into(),
                victim_kind: VictimKind::Ship,
                weapon: "torpedo".into(),
                amount: 9.0,
                shield_absorbed: 9.0,
                hull_damage: 0.0,
                system_hit: None,
            },
        ];
        let names: BTreeMap<String, String> = [("player".to_string(), "Ironveil".to_string())]
            .into_iter()
            .collect();
        let report = RunReport {
            ticks: 10,
            sim_seconds: 1.0,
            seed: 0,
            seed_source: "random".into(),
            wall_seconds: 1.0,
            ticks_per_second: 10.0,
            final_phase: "InProgress".into(),
            game_over_reason: None,
            entity_count: 2,
            ship: None,
            message_counts: BTreeMap::new(),
            damage_by_ship: crate::core::balance::aggregate_damage(events.iter(), &names),
            outcome_report: crate::core::balance::classify(
                false,
                None,
                SideMargins::default(),
                SideMargins::default(),
            ),
            ai_doctrine: String::new(),
            station_activity: StationActivityPayload::default(),
        };
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));

        assert_eq!(parsed["damage_by_ship"]["player"]["name_id"], "Ironveil");
        assert_eq!(parsed["damage_by_ship"]["player"]["damage_dealt"], 20.0);
        assert_eq!(parsed["damage_by_ship"]["player"]["damage_taken"], 9.0);
        assert_eq!(parsed["damage_by_ship"]["raider"]["damage_dealt"], 9.0);
        assert_eq!(parsed["damage_by_ship"]["raider"]["damage_taken"], 20.0);
        assert!(parsed["damage_by_ship"]["raider"]["name_id"].is_null());
    }

    /// The always-on station-activity series serialises into the report as a
    /// per-station, per-bucket, per-control-source object (issue #1147). This is
    /// the schema the balance-runs merge folds and the report-integration tests
    /// assert on — a run's report carries it whether or not any debug flag was
    /// set, because `build_report` reads the tracker's projection directly.
    #[test]
    fn report_json_carries_the_station_activity_series() {
        use crate::debug::payload::{StationActivityBucket, StationActivityEntry};

        let payload = StationActivityPayload {
            schema_version: crate::debug::payload::DEBUG_SCHEMA_VERSION,
            bucket_ticks: 900,
            bucket_secs: 15.0,
            buckets: vec![StationActivityBucket {
                start_tick: 0,
                stations: vec![
                    StationActivityEntry {
                        station: "helm".into(),
                        human: 12,
                        ai: 3,
                        offline: 0,
                    },
                    StationActivityEntry {
                        station: "weapons".into(),
                        human: 0,
                        ai: 21,
                        offline: 0,
                    },
                ],
            }],
        };
        let report = RunReport {
            ticks: 900,
            sim_seconds: 15.0,
            seed: 1,
            seed_source: "cli".into(),
            wall_seconds: 0.0,
            ticks_per_second: 0.0,
            final_phase: "InProgress".into(),
            game_over_reason: None,
            entity_count: 2,
            ship: None,
            message_counts: BTreeMap::new(),
            damage_by_ship: BTreeMap::new(),
            outcome_report: crate::core::balance::classify(
                false,
                None,
                SideMargins::default(),
                SideMargins::default(),
            ),
            station_activity: payload,
        };
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));

        let sa = &parsed["station_activity"];
        assert_eq!(sa["schema_version"], 1);
        assert_eq!(sa["bucket_ticks"], 900);
        assert_eq!(sa["bucket_secs"], 15.0);
        let stations = &sa["buckets"][0]["stations"];
        // Sorted by station id: helm before weapons, split by control source.
        assert_eq!(stations[0]["station"], "helm");
        assert_eq!(stations[0]["human"], 12);
        assert_eq!(stations[0]["ai"], 3);
        assert_eq!(stations[1]["station"], "weapons");
        assert_eq!(stations[1]["ai"], 21);
        assert_eq!(stations[1]["human"], 0);
    }
}
