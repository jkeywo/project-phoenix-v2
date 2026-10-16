//! Server-side console AI orchestrator.
//!
//! Complexity preset machinery (ComplexityRules, ConsoleComplexityState,
//! build_complexity_rules, track_complexity_changes) removed in B4 (issue #534).
//! AI behaviour is now gated by StationRatingConfig.ai_tuning.
//!
//! Issue #692 wires the previously-orphaned pure decision functions from
//! `console_ai::core` into Bevy systems here:
//! - Shield arcs use the closed per-System state-tree host in
//!   `shields_tree`, which emits ordinary admitted commands. Physical damage
//!   evidence is produced independently by the Shields domain.
//! - `tick_frequency_hint_high_fidelity` (named `ai_frequency_hint` until issue
//!   #873 took the operator branch out of it) — wires
//!   `console_ai::tick_frequency_hint`, which had no caller anywhere prior to
//!   this issue.
//!
//! Issue #694 (preliminary) added `ai_torpedo_auto_fire` /
//! `integrate_torpedo_intents`, replacing the old fused torpedo sub-block
//! that used to live inside `console::weapons::operate_tactical_ai`
//! with the same decide/`TorpedoIntents`-write + mutate-only-adapter shape.
//! `operate_tactical_ai` kept running for Tactical target selection only.
//!
//! Issue #698 completes that work: `ai_torpedo_auto_fire` is no longer
//! preliminary (it reads real target-lock and target-shield state instead of
//! hardcoding them), and `integrate_torpedo_intents` is gone — its body moved
//! into `console::weapons::integrate_weapons_state`, the single
//! adapter that drains both `TorpedoIntents` and `PhaserIntents`.
//!
//! Issue #700 finished the decomposition: `operate_tactical_ai` is gone
//! entirely, and target selection now lives wholly in
//! `console::weapons::ai_target_selection`.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
#[path = "shields_tree.rs"]
mod shields_tree;

// AI rule keys — match the keys used in [[station.rating]].ai_tuning tables.
pub use phoenix_sim_gameplay::AI_RULE_TORPEDO_AUTO_FIRE;
// `AI_RULE_AUTO_HINT` ("auto_hint") was deleted by issue #873. It gated the
// Sensors frequency hint on whether a *human session* held the Sensors station
// and, if so, on that holder's active rating — so a coordination fact derived
// entirely from authoritative ship state stopped being emitted the moment a
// human sat down. That is the human/AI branch AGENTS.md rule 6 forbids, and no
// shipped hull authored the key anyway. Do not reintroduce it: a station rating
// tunes what a console offers its own operator, never whether the ship's state
// reaches the rest of the bridge.

/// Per-ship persistent state for `tick_frequency_hint_high_fidelity`'s
/// delayed-hint timer.
/// Bevy-facing wrapper around `console_ai::FrequencyHintState`.
///
/// Present only while the ship carries `AiHighFidelity` — bundled alongside
/// that marker at every spawn/promote site (mirrors `ShipPowerAiState`'s
/// scoping; see `ai::server::lod_ai_ships` and the `AiHighFidelity` spawn
/// sites in `server_app.rs` / `ship_plugin.rs` / `ai/server.rs`).
#[derive(Component, Default, Clone, Debug)]
pub struct ShipFrequencyHintState(pub crate::console_ai::FrequencyHintState);

impl ShipFrequencyHintState {
    /// Read the authoritative delayed-hint memory for continuation projection.
    pub(crate) fn continuation(&self) -> (Option<&str>, f32, bool) {
        (
            self.0.current_target.as_deref(),
            self.0.elapsed_secs,
            self.0.hint_sent,
        )
    }

    /// Replace bootstrap memory with a restored continuation.
    pub(crate) fn replace_continuation(
        &mut self,
        current_target: Option<String>,
        elapsed_secs: f32,
        hint_sent: bool,
    ) {
        self.0 = crate::console_ai::FrequencyHintState {
            current_target,
            elapsed_secs,
            hint_sent,
        };
    }
}

/// Console AI orchestrator plugin.
pub struct ConsoleAiPlugin;

impl Plugin for ConsoleAiPlugin {
    fn build(&self, app: &mut App) {
        // The ONE shared AI decision cadence (issues #889, #895). Every decider
        // below used to be UNGATED, and while `SimSet` was configured in Bevy's
        // `Update` that meant one decision per rendered frame — at display
        // refresh rate, over a `WorldSnapshot` rebuilt on an unrelated clock.
        // They now share the helm axes' latch, derived from the logical tick.
        crate::ai::cadence::register_ai_cadence(app);
        // The shared AI host spine's read-only world context (issue #1205). No
        // host in this plugin consumes `AiHostEnv` yet — this is the additive
        // wiring point so the bare-`Res` env is registered wherever the AI host
        // plugin runs, mirrored by `ship::test_support` for fixtures.
        crate::ai::host::register_ai_host_env(app);
        app.add_systems(
            FixedUpdate,
            (
                // Decide only (issue #826): emits admitted SetShieldArcFocus
                // payloads; the single applier is `ship::shields::
                // handle_shields_messages` (registered by ShipShieldsPlugin
                // in this same Physics set). The `.before` edge is the one
                // explicit ordering between them — admission clears
                // AdmittedCommands before Input each tick, so the applier
                // must consume same-tick after this emit.
                shields_tree::ai_shields
                    .in_set(crate::sim_sets::FixedStep::AiShieldFocus)
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(crate::ship::shields::handle_shields_messages)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Decide only (issue #831, mirroring shields #826): emits
                // admitted SetPowerGroupAllocation payloads; the single applier
                // is `ship::power::handle_power_messages` (registered by
                // ShipPowerPlugin in this same Physics set). The `.before` edge
                // is the one explicit ordering between them — admission clears
                // AdmittedCommands before Input each tick, so the applier must
                // consume same-tick after this emit.
                ai_power_allocation
                    .in_set(crate::sim_sets::FixedStep::AiPowerAllocation)
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(crate::ship::power::handle_power_messages)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Decide only. The apply half is
                // `console::weapons::handle_fire_torpedo` (issue #846 —
                // previously `integrate_weapons_state`, which drained the
                // retired `TorpedoIntents`), registered by `WeaponsPlugin` in
                // this same `Physics` set.
                //
                // The `.before` edge is NOT decoration, and its absence was a
                // live production bug: both systems sat in `SimSet::Physics`
                // with no edge between them, and the resolved order put the
                // CONSUMER first. The admitted `FireTorpedo` therefore sat in
                // `AdmittedCommands` untouched until `clear_before_input`
                // wiped it at the top of the next tick, so an AI-crewed ship
                // never launched a torpedo in a real run — every unit test
                // passed only because the weapons harness adds this edge
                // itself. Same class as #881 (an AI-emitted command whose
                // applier was never ordered against the emitter); the
                // shields/power siblings above carry the identical edge for
                // the identical reason.
                ai_torpedo_auto_fire
                    .in_set(crate::sim_sets::FixedStep::AiTorpedoAutoFire)
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(crate::console::weapons::handle_fire_torpedo)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // The loading half of the torpedo AI. `Input`, not `Physics`,
                // and explicitly before the volley-target handler: the command
                // it emits has to be consumed in the SAME tick, exactly as
                // `operate_captain_ai` is ordered before
                // `handle_set_red_alert`. That also puts `target_count` in
                // place before `advance_torpedo_flights` (Physics) runs its
                // auto-load block, so a tube starts loading the tick the order
                // is given rather than the tick after.
                //
                // Gated on the shared base cadence like its launch half. The
                // two run in different sets (`Input` load, `Physics` launch)
                // but on the SAME latch, so a tube that is loaded and ready
                // still launches in the tick the launch decider next fires —
                // quantising both halves does not insert an extra tick between
                // them.
                ai_torpedo_load
                    .in_set(crate::sim_sets::FixedStep::AiTorpedoLoad)
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(crate::console::weapons::handle_set_torpedo_volley_target)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Not an AI-operator decider despite living in this plugin
                // (issue #873): it emits the ship's Sensors frequency advisory
                // for every high-fidelity hull regardless of who holds Sensors.
                // It stays under `ai_tick_ready` because its reaction-delay
                // model advances by the authored tick period, not `Time::delta`.
                tick_frequency_hint_high_fidelity
                    .in_set(crate::sim_sets::FixedStep::TickFrequencyHintHighFidelity)
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready),
            ),
        );
    }
}

// ── Power AI ─────────────────────────────────────────────────────────────────

// `emit_power_ai_command` (issue #831) likewise collapsed into the shared
// `command_admission::ai_emit::emit_ai_command` seam (issue #738).

/// Ordinary per-System tree AI uses exactly the Console's published readings.
fn ai_power_allocation(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut crate::core::messages::AdmittedCommands,
    )>,
) {
    use crate::core::messages::{SystemBlackboard, SystemControlPayload as Control};
    use phoenix_sim_gameplay::ai::host::{decide_tree_with_numbers, TreeHostOutcome};
    use std::collections::BTreeMap;
    let system = crate::ship::system_registry::power_reactor_system_id();
    for (uuid, sources, config, boards, programs, mut runtimes, mut admitted) in &mut ships {
        if !crate::ai::host::ai_operates(&sources.0, &system) {
            continue;
        }
        let view = config.0.observations_for_system(&system, &boards.0);
        let Some(power_view) = view
            .get(&crate::core::messages::SystemId(
                crate::ship::system_registry::POWER_SYSTEM_ID.into(),
            ))
            .and_then(|board| match board {
                SystemBlackboard::Power(board) => Some(board),
                _ => None,
            })
        else {
            continue;
        };
        let Some(energy) = power_view.energy.as_ref() else {
            continue;
        };
        let Some(runtime) = runtimes.0.get_mut(&system) else {
            continue;
        };
        let mut facts = BTreeMap::from([
            (
                "strike_boost_enabled".into(),
                if power_view
                    .strike_reserve
                    .as_ref()
                    .is_some_and(|reading| reading.enabled)
                {
                    1.0
                } else {
                    0.0
                },
            ),
            (
                "reactor_reserve_fraction".into(),
                (energy.reactor.energy / energy.reactor.capacity) as f64,
            ),
            (
                "reactor_heat_fraction".into(),
                (energy.reactor.heat / energy.reactor.trip_heat) as f64,
            ),
            (
                "output_actual".into(),
                energy.reactor.output_actual_per_sec as f64,
            ),
        ]);
        for (group, reserve, heat) in power_group_observations(energy) {
            facts.insert(format!("reserve-{group}"), reserve as f64);
            facts.insert(format!("heat-{group}"), heat as f64);
        }
        let result = match decide_tree_with_numbers(
            &sources.0,
            &system,
            programs.0.get(&system),
            &BTreeMap::new(),
            &facts,
            &BTreeMap::from([
                (
                    "reactor-generation".into(),
                    energy.reactor.generation_enabled,
                ),
                (
                    "priority-allocation".into(),
                    energy.mode == phoenix_model::energy::EnergyAllocationMode::Minimal,
                ),
            ]),
            runtime,
            tick.0,
        ) {
            Ok(TreeHostOutcome::Evaluated(result)) => result,
            Ok(_) => continue,
            Err(error) => {
                warn!(%error, "power.tree_refused");
                continue;
            }
        };
        let number = |name: &str, fallback: f32| {
            result
                .numeric_intents
                .get(name)
                .map(|value| *value as f32)
                .unwrap_or(fallback)
        };
        let output = number("reactor-output", energy.reactor.output_target_per_sec);
        let cooling = number("reactor-cooling", energy.reactor.cooling_per_sec);
        if output != energy.reactor.output_target_per_sec
            || cooling != energy.reactor.cooling_per_sec
        {
            emit_ai_command(
                uuid,
                system.clone(),
                Control::SetEnergyReactor {
                    output_per_sec: output,
                    cooling_per_sec: cooling,
                },
                sources,
                &sessions,
                Some(config),
                &mut admitted,
            );
        }
        if let Some(enabled) = result.intents.get("reactor-generation") {
            if *enabled != energy.reactor.generation_enabled {
                emit_ai_command(
                    uuid,
                    system.clone(),
                    Control::SetEnergyGeneration { enabled: *enabled },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
        if let Some(priority) = result.intents.get("priority-allocation") {
            use phoenix_model::energy::EnergyAllocationMode;
            let mode = if *priority {
                EnergyAllocationMode::Minimal
            } else {
                EnergyAllocationMode::Manual
            };
            if mode != energy.mode {
                emit_ai_command(
                    uuid,
                    system.clone(),
                    Control::SetEnergyMode {
                        mode,
                        cooling_usage: energy.cooling_usage,
                    },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
        for group in &energy.groups {
            let flow = number(&format!("flow-{}", group.id), group.desired_flow_per_sec);
            let cooling = number(
                &format!("cooling-{}", group.id),
                group.desired_cooling_per_sec,
            );
            if flow != group.desired_flow_per_sec || cooling != group.desired_cooling_per_sec {
                emit_ai_command(
                    uuid,
                    system.clone(),
                    Control::SetEnergyGroup {
                        group: crate::core::messages::PowerGroupId(group.id.clone()),
                        flow_per_sec: flow,
                        cooling_per_sec: cooling,
                    },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

// ── Torpedo AI (issue #694, completed by #698) ──────────────────────────────

/// AI torpedo auto-fire decision system (issue #694; promoted to full in #698).
///
/// Wires the previously-orphaned `console_ai::auto_fire_torpedo`: for ships
/// whose Tactical target is already locked (`TacticalRadarSelection`, written by
/// `console::weapons::ai_target_selection`), decides which loaded,
/// in-arc torpedo tubes to fire and writes the
/// decision into `TorpedoIntents` for
/// `console::weapons::integrate_weapons_state` to apply, rather than
/// calling `TorpedoSystem::launch` directly.
///
/// Replaces the old fused torpedo sub-block that used to run inline inside
/// `operate_tactical_ai` (banner-marked "TORPEDO AUTO-FIRE (future: split to
/// torpedo_tube system)").
///
/// # Real inputs (issue #698)
/// #694 landed with `TorpedoAiInput { target_locked: true, target_shields: 0 }`
/// hardcoded, because target selection still lived inside `operate_tactical_ai`
/// and there was no settled place to read a lock from. #697 split selection out,
/// so both now come from real state:
///
/// - `target_locked` — the locked target actually resolves to a live entity in
///   the world this tick. A `TacticalRadarSelection` naming a destroyed entity is not a
///   lock. (Pre-#698 this position lookup happened anyway, purely to compute
///   bearing; the difference is that failing it is now expressed as
///   `target_locked = false` rather than an early `continue`.)
/// - `target_facing_shields` — the HP of the single shield arc a torpedo
///   arriving from this ship would strike, resolved by handing the attack
///   bearing to the target's own `ShieldSystem::facing_index_for_bearing` —
///   the same bearing→arc resolver the damage path uses, so the gate asks
///   about the arc the shot is on course to meet. (The gate predicts from the
///   launcher's bearing; `tick_torpedo_lifecycle` routes the hit from the
///   torpedo's own impact point, so a torpedo that homes far enough around a
///   moving target can still land on a neighbouring arc. One resolver, two
///   moments in the shot's life.) Since issue #956 nothing in Rust compares it
///   to anything: it is seeded into the tube's launch snapshot and the tube's
///   own authored guard is what holds fire. Phasers strip the shields, torpedoes
///   finish the hull — said in TOML, and therefore sayable differently by
///   different hulls. Every armed tube currently authors
///   `fact(target_facing_shields) <= 0`, and it is worth more than it used to be:
///   since issue #929 a round delivers `damage_hull` only into a DOWN arc and the
///   far smaller `damage_shields` into a live one, so the gate decides the
///   payload rather than merely the timing. Do not read a fleet-wide rule out of
///   this seeding all the same — #929's first pass had `alliance_cruiser`'s three
///   tubes compare against a `param(max_striking_shield_hp)` set past any arc
///   reading, switching the conjunct off, and its second pass put the fleet text
///   back and raised that hull's guns instead.
///   `torpedo_launch_shield_gate_truth_table` is the enumeration; this seeding
///   takes no position.
///
///   It is deliberately *not* the sum over all arcs. Summing let three
///   healthy REAR arcs veto a shot into a collapsed FRONT arc while the
///   attacker was dead ahead — the hull is exposed exactly where the torpedo
///   would land. With per-arc regen and short offline windows a four-arc
///   Alliance hull practically never has every arc down at once, so the
///   summed gate meant AI crews on those hulls never fired a torpedo. A
///   single-omni-arc NPC is unaffected: its one arc is the facing arc for
///   every bearing. An offline arc reports 0 because it passes damage through
///   to the hull and so is not blocking the shot, and a target with no
///   `ShipShields` at all (asteroids, debris) reports 0 and stays
///   torpedo-eligible — which is what preserves the pre-#698 behaviour for
///   every non-ship target.
///
/// # Gating
/// - `ShipSystemControlSources.policy_for(torpedo_magazine_system_id()).operate_ai`
///   — new constraint vs. the old fused block, which had no torpedo-specific
///   gate of its own beyond the combined `any_tactical_system_operates_ai`
///   check that ran before the old `operate_tactical_ai` (that check still
///   gates `ai_target_selection`; this is an *additional*, torpedo-specific
///   gate). The torpedo magazine is the shared bottleneck
///   resource across tubes, so its policy is the natural per-system gate
///   (no single unified `torpedo_system_id()` exists).
/// - `AiHighFidelity` (query filter, new constraint vs. the old system).
/// - The firing ship's canonical Tactical rating: Backfill operates normally;
///   a chosen rating requires its authored `torpedo_auto_fire` tuning. Other
///   ships' session claims and this peer's local projection cannot gate it.
///   This chosen-rating distinction is what
///   `console::weapons`'s `ai_stops_firing_when_rating_switches_to_std`
///   test asserts on.
pub(crate) fn ai_torpedo_auto_fire(
    sessions: Res<crate::lobby::Sessions>,
    aims: crate::console::weapons::assignments::WeaponAimInputs,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    guidance: crate::console::weapons::guidance_link::GuidanceMeasurements,
    // Objective-contributed Command stances (issue #1110). `Option<Res<_>>` so a
    // bare-`App` weapons fixture with no world plugin reads no contributions.
    active_objective_stances: Option<Res<crate::console::command::server::ActiveObjectiveStances>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &crate::ship::state::ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::console::weapons::TorpedoSystemResource>,
            Option<&crate::console::weapons::TorpedoTubeAiPolicies>,
            Option<&crate::console::weapons::guidance_link::ShipGuidanceKnowledge>,
            &mut crate::core::messages::AdmittedCommands,
            // Issue #872: this ship's own red-alert state, seeded as a typed
            // fact for the tube's authored LAUNCH predicate. `Option<&_>` for
            // fixtures that spawn a ship without it; absent reads `false`.
            Option<&crate::ship::state::ShipRedAlert>,
            // Issue #1107: the ship's Command stance selections, so a directed
            // AI weapons Station's stance decides the launch posture in place of
            // the ship's own Red Alert. Absent reads as no direction.
            Option<&crate::console::command::server::ShipStationStances>,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    asteroid_q: Query<
        (&crate::server_app::AsteroidUuid, &Transform),
        With<crate::server_app::Asteroid>,
    >,
    other_ships_q: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &Transform,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::ship::state::ShipPhysics>,
        ),
        Without<crate::server_app::Asteroid>,
    >,
) {
    let policy_sid = crate::ship::system_registry::torpedo_magazine_system_id();

    for (
        ship_entity,
        entity_uuid,
        ship_config,
        control_sources,
        active_ratings,
        physics,
        blackboards,
        torpedo_sys_comp,
        tube_policies,
        guidance_knowledge,
        mut admitted,
        red_alert_opt,
        stances_opt,
    ) in ships.iter_mut()
    {
        // Read once per ship; folded into every tube's posture below. No Rust
        // rule consults it — the gate is the tube's authored predicate (#872),
        // and the restraint lever beside it rides that same predicate (#1396).
        // The Command stance override (#1107) rides the same fact;
        // absent a direction it is `None` and the seeded value is unchanged.
        let stance_override = crate::console::command::server::weapons_station_stance_high_alert(
            stances_opt,
            active_objective_stances
                .as_deref()
                .map(|a| a.for_ship(entity_uuid.map_or("", |u| u.0.as_str())))
                .as_ref(),
            &ship_config.0,
            &control_sources.0,
            red_alert_opt.is_some_and(|r| r.0),
        );
        // One tube's posture (issue #1396). The alert, the hold and the stance
        // are the ship's; COLD is a reading of the power group THIS tube's
        // `[[system]]` entry authors, so it is resolved per tube inside the
        // launch loop rather than once per ship.
        let posture_for = |tube_id: &str| {
            crate::console::weapons::WeaponsAlertPosture::from_parts(
                red_alert_opt,
                crate::ship::system_registry::torpedo_tube_system_id(tube_id).is_some_and(|sid| {
                    crate::console::weapons::observed_weapons_restraint(
                        Some(ship_config),
                        &sid,
                        Some(blackboards),
                    )
                }),
                stance_override,
            )
        };
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);
        // Control-Source gate through the shared AI host spine (issue #1208): the
        // torpedo MAGAZINE's own operate_ai is the natural per-ship gate — the
        // shared bottleneck resource across tubes, and there is no unified
        // torpedo_system_id. The per-tube LAUNCH resolution stays in
        // `torpedo_tube_launch_policy_fires`.
        if !crate::ai::host::ai_operates(&control_sources.0, &policy_sid) {
            continue;
        }

        // The station owning this ship's weapons — resolved per-ship rather
        // than assumed to be named "tactical". Only legacy hulls without a
        // declared weapons Station use the canonical fallback identity.
        let tactical_station = ship_config.0.weapons_station().unwrap_or_else(|| {
            crate::core::messages::StationId(
                crate::ship::system_registry::TACTICAL_STATION_ID.into(),
            )
        });

        // Crew claims are host-local, while these ratings belong to this ship
        // on every peer. Never let a different ship's Tactical holder suppress
        // an ordinary Backfill tube. Bare legacy fixtures omit the rating map;
        // accepted stationed ships seed an explicit Backfill or chosen rating.
        let auto_fire_enabled = active_ratings
            .0
            .get(&tactical_station)
            .is_none_or(|rating| {
                rating == crate::ship::rating::BACKFILL_RATING
                    || ship_config.0.has_ai_rule(
                        &tactical_station,
                        rating,
                        AI_RULE_TORPEDO_AUTO_FIRE,
                    )
            });
        if !auto_fire_enabled {
            continue;
        }

        // Per-entity component only (issue #738). This used to fall back to
        // the global `TorpedoSystemResource` Resource, which mirrors the
        // LOCAL ship's magazine and tube state — so an NPC with no `[torpedoes]`
        // block decided its auto-fire from the player's tubes. A ship with no
        // torpedo system has nothing to fire.
        let Some(torpedo_sys_comp) = torpedo_sys_comp else {
            continue;
        };
        let torpedo_sys: &crate::weapons::torpedo::TorpedoSystem = &torpedo_sys_comp.0;
        // Ship-wide "every tube is at its volley capacity" (issue #791). A
        // doctrine that spends a shield gap on one full salvo needs this rather
        // than the per-tube `loaded` (which is `loaded_count > 0`), and it is a
        // reading of the ship's own tubes rather than an authored number — the
        // capacity itself is `[[torpedoes.tubes]] volley_max`. A hull with no
        // tubes at all cannot reach here (`tubes` would be empty and
        // `auto_fire_torpedo` would return nothing), so the vacuous-true case of
        // `all` is unreachable rather than merely unlikely.
        let tubes_full = torpedo_sys
            .tubes
            .iter()
            .all(|tube| tube.loaded_count >= tube.volley_max);

        for tube in &torpedo_sys.tubes {
            let tube_id = &tube.id;
            let Some(target) = crate::ship::system_registry::torpedo_tube_system_id(tube_id) else {
                continue;
            };
            // Maintain the last explicit own order, independently of today's
            // primary target. Only a newer original permitted reading triggers
            // another paid issue; explicit stop is never undone by maintenance.
            if torpedo_sys.config.receiver.is_some()
                && crate::ai::host::ai_operates(&control_sources.0, &target)
            {
                if let Some(knowledge) = guidance_knowledge {
                    for (round_uuid, target_uuid) in
                        phoenix_sim_gameplay::weapons::guided_orders::maintenance_requests(
                            &knowledge.0,
                            &target,
                            |contact| {
                                aims.resolve_reference(ship_entity, &target, contact, |_| None)
                                    .filter(|aim| aim.allows(tube.targets.as_deref()))?;
                                guidance
                                    .sample(ship_entity, &target, contact)
                                    .map(|reading| reading.sampled_tick)
                            },
                        )
                    {
                        crate::command_admission::ai_emit::emit_ai_command(
                            entity_uuid,
                            target.clone(),
                            crate::core::messages::SystemControlPayload::RetargetGuidedRound {
                                round_uuid,
                                target_uuid,
                            },
                            control_sources,
                            &sessions,
                            Some(ship_config),
                            &mut admitted,
                        );
                    }
                }
            }
            let Some(aim) = aims.resolve(ship_entity, &target, |route| {
                asteroid_q
                    .iter()
                    .find_map(|(uuid, transform)| {
                        (uuid.0 == route)
                            .then_some([transform.translation.x, transform.translation.z])
                    })
                    .or_else(|| {
                        other_ships_q.iter().find_map(|(uuid, transform, _, _)| {
                            (uuid.0 == route)
                                .then_some([transform.translation.x, transform.translation.z])
                        })
                    })
            }) else {
                continue;
            };
            if !aim.allows(tube.targets.as_deref()) {
                continue;
            }
            let geometry = aims.geometry(
                ship_entity,
                &target,
                &crate::weapons::mount::MountConfig {
                    facing_deg: tube.facing_deg,
                    fire_arc_deg: tube.fire_arc_deg,
                    range: torpedo_sys.config.speed * torpedo_sys.config.lifespan,
                },
                &aim,
                physics,
            );
            if !tube.is_loaded() || !geometry.in_arc || !geometry.in_range {
                continue;
            }
            // A permitted position is not a shield measurement. Legacy fixtures
            // retain their old inputs; fitted instruments never read live enemy
            // shields to decide whether to launch.
            let target_facing_shields = if aim.measured {
                aims.observed_shield_cover(ship_entity, &target, &aim, [physics.x, physics.z])
            } else {
                Some(
                    other_ships_q
                        .iter()
                        .find_map(|(uuid, _, shields, target_physics)| {
                            (uuid.0 == aim.route).then(|| {
                                shields
                                    .map(|shields| {
                                        shields.0.hp_facing_attacker(
                                            physics.x,
                                            physics.z,
                                            aim.position[0],
                                            aim.position[1],
                                            target_physics.map_or(0.0, |physics| physics.yaw),
                                        )
                                    })
                                    .unwrap_or(0)
                            })
                        })
                        .unwrap_or(0),
                )
            };
            // Per-tube LAUNCH policy gate (issue #782): `auto_fire_torpedo`
            // already resolved this tube's host readiness (loaded, in arc, target
            // locked — NOT the magazine, whose rounds are the ones left to reload
            // with and were already spent for whatever is in the tubes, and NOT
            // the striking arc's shields, which issue #956 moved into the guard
            // below); now resolve
            // the tube's own authored launch policy over a seeded snapshot. Only a
            // tube whose policy fires `LaunchTorpedo` launches — an idle tube (or
            // one whose guard holds) is skipped, leaving other tubes free to fire
            // (per-tube independence). The candidates all passed the host gates,
            // so those facts are `true`; `target_facing_shields` carries its live
            // per-arc HP reading, and is now the ONLY gate on the shield state.
            //
            // A tube with NO entry does not launch: since #885b stage 5d there
            // is no synthesised stand-in, and strict AI-declaration mode rejects
            // a tube that authors no inline `ai` block at load.
            let Some(launch_policy) = tube_policies.and_then(|p| p.0.get(tube_id)) else {
                continue;
            };
            let facts = crate::console::weapons::seed_torpedo_tube_launch_facts(
                true,
                true,
                true,
                true,
                target_facing_shields,
                tubes_full,
                posture_for(tube_id),
            );
            if !crate::console::weapons::torpedo_tube_launch_policy_fires(
                launch_policy,
                &facts,
                &flag_chain,
            ) {
                continue;
            }
            // Emit as an admitted command through the shared AI seam (issue
            // #846), instead of the retired `TorpedoIntents` buffer.
            crate::command_admission::ai_emit::emit_ai_command(
                entity_uuid,
                target,
                crate::core::messages::SystemControlPayload::FireTorpedo { target_uuid: None },
                control_sources,
                &sessions,
                Some(ship_config),
                &mut admitted,
            );
        }
    }
}

/// AI torpedo *loading* system — the missing half of the torpedo AI.
///
/// `ai_torpedo_auto_fire` decides whether to fire an already-loaded tube, but
/// nothing ever asked for one to be loaded: `TorpedoSystem::from_configs`
/// starts every tube at `target_count: 0` and the auto-load block in
/// `TorpedoSystem::tick` is gated on `target_count > 0`, so an AI-crewed ship
/// never launched a torpedo in its life. This system is the console operator
/// nobody in an AI-crewed run is.
///
/// # Why it goes through `AdmittedCommands`
///
/// It emits `SetTorpedoVolleyTarget` into the ship's own `AdmittedCommands`
/// rather than writing `target_count` directly, so the AI issues *the same
/// command a human's console sends* and `handle_set_torpedo_volley_target`
/// stays the single writer of the tube's volley target (AGENTS.md — "humans
/// and AI are symmetric"; admission is the only place that knows who sent a
/// command). The alternative — a non-zero `target_count` default in
/// `from_configs` — would silently pre-load every *human* player's tubes and
/// drain their magazine with no order given.
///
/// This is the `operate_captain_ai` → `handle_set_red_alert` pattern: push
/// into each ship's own `AdmittedCommands` in `SimSet::Input`, ordered before
/// the handler that drains it, and let the handler iterate every ship so NPC
/// commands are not dropped.
///
/// # Gating
/// - Per-tube: the tube's own fine `torpedo-tube-<id>` system must be
///   `operate_ai` (an unregistered tube id falls back to the default-source
///   policy, matching `handle_load_tube`'s treatment — issue #801).
/// - The shared magazine's fine system must also be `operate_ai`, mirroring
///   `ai_torpedo_auto_fire`'s magazine gate: the magazine is the bottleneck
///   resource every tube draws from, and a magazine no AI is operating should
///   not be emptied into the tubes.
///
/// How many rounds to keep loaded is TOML, never a constant: the per-tube
/// `[[torpedoes.tubes]] ai_target_count`, the ship-wide `[torpedoes]
/// ai_volley_target`, or the tube's `volley_max` — resolved at construction
/// into `TorpedoTube::ai_target_count`.
///
/// Deliberately *not* gated on `AiHighFidelity`: loading is a slow standing
/// order (seconds of load time), and a ship demoted to low LOD mid-load would
/// otherwise stop maintaining tubes it is about to need on promotion.
pub(crate) fn ai_torpedo_load(
    sessions: Res<crate::lobby::Sessions>,
    // `Option<Res<_>>`, never bare — bare-`App` fixtures never insert it.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &crate::console::weapons::TorpedoSystemResource,
            Option<&crate::console::weapons::TorpedoTubeAiPolicies>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let magazine_id = crate::ship::system_registry::torpedo_magazine_system_id();

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        ship_config,
        torpedo_sys,
        tube_policies,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Control-Source gate through the shared AI host spine (issue #1208): the
        // torpedo MAGAZINE's own operate_ai. The per-tube LOAD resolution stays in
        // `torpedo_tube_load_policy_fires`.
        if !crate::ai::host::ai_operates(&control_sources.0, &magazine_id) {
            continue;
        }

        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);

        let tubes: Vec<crate::console_ai::TubeLoadSummary> = torpedo_sys
            .0
            .tubes
            .iter()
            .map(|tube| {
                let tube_system_id = crate::ship::system_registry::torpedo_tube_system_id(&tube.id)
                    .filter(|id| {
                        crate::console::weapons::shared::system_is_registered(control_sources, id)
                    });
                let policy = match &tube_system_id {
                    Some(id) => control_sources.0.policy_for(id),
                    // Unregistered fine system → default-source policy
                    // (issue #801 — no coarse fallback).
                    None => crate::ship::control_source::control_tick_policy(
                        crate::ship::control_source::ControlSource::default(),
                    ),
                };
                crate::console_ai::TubeLoadSummary {
                    id: tube.id.clone(),
                    target_count: tube.target_count,
                    ai_target_count: tube.ai_target_count,
                    operates_ai: policy.operate_ai,
                }
            })
            .collect();

        for (tube_id, count) in crate::console_ai::torpedo_load_orders(&tubes) {
            // Per-tube LOAD policy gate (issue #782): `torpedo_load_orders`
            // decided this tube is AI-operated and off its configured volley
            // target; now resolve the tube's own authored load policy over a
            // seeded fact snapshot. Only a tube whose policy fires `LoadTorpedo`
            // emits the volley order — an idle tube (or one whose guard holds)
            // is skipped, leaving other tubes free to load (per-tube
            // independence).
            //
            // A tube with NO entry does not load: since #885b stage 5d there is
            // no synthesised stand-in, and strict AI-declaration mode rejects a
            // tube that authors no inline `ai` block at load.
            let Some(policy) = tube_policies.and_then(|p| p.0.get(&tube_id)) else {
                continue;
            };
            let tube_ref = torpedo_sys.0.tube(&tube_id);
            let facts = crate::console::weapons::seed_torpedo_tube_load_facts(
                tube_ref.map(|t| t.loaded_count).unwrap_or(0),
                tube_ref.map(|t| t.target_count).unwrap_or(0),
                count,
                torpedo_sys.0.torpedoes_remaining(),
                true,
            );
            if !crate::console::weapons::torpedo_tube_load_policy_fires(policy, &facts, &flag_chain)
            {
                continue;
            }
            let Some(target) = crate::ship::system_registry::torpedo_tube_system_id(&tube_id)
            else {
                continue;
            };
            // Through the shared AI-emit seam (issue #738), never a raw
            // `admitted.0.push`: admission is the only place that decides what
            // an `ai:` token may do, and it re-checks `operate_ai` on this
            // exact tube SystemId.
            let admitted_ok = emit_ai_command(
                entity_uuid,
                target.clone(),
                crate::core::messages::SystemControlPayload::SetTorpedoVolleyTarget { count },
                control_sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
            // A refusal here is silent by construction — no panic, no order, no
            // torpedo — and the symptom (an AI crew that never fires) is a long
            // way from the cause. The decide gate above already required this
            // tube to be `operate_ai`, so a refusal means the tube's fine
            // system is not registered in this ship's `ControlSourceResolver`
            // (an authored `[[torpedoes.tubes]]` with no matching `[[system]]`
            // block) and the two gates are reading different worlds. Warn.
            if !admitted_ok {
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Weapons,
                    entity = ship_entity,
                    "torpedo load order for {} refused at admission — no such \
                     fine system on this ship's control sources",
                    target.0
                );
            }
        }
    }
}

// `integrate_torpedo_intents` (issue #694) lived here until issue #698 folded
// it into `console::weapons::integrate_weapons_state`, which drains
// `TorpedoIntents` and `PhaserIntents` in one adapter. Nothing else may drain
// `TorpedoIntents`: two systems both draining it would race for the launch.

// ── Frequency-hint AI ─────────────────────────────────────────────────────────

/// High-fidelity Sensors frequency-hint emitter (issues #692, #873).
///
/// Publish an advisory only after a permitted paid Shields reading exists for
/// the selected public subject. The retained finding, including its observed
/// configuration, is the entire answer; no target machinery is queried here.
/// Human and AI holders share this acquisition and delivery path. High-fidelity
/// hulls retain the authored advisory delay; the low-fidelity twin deduplicates
/// the same recorded evidence without performing a second acquisition.
pub(crate) fn tick_frequency_hint_high_fidelity(
    contact_runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // The AUTHORED tick period, not `Time::delta` (issue #889). This system is
    // gated by `run_if(ai_tick_ready)`, so it observes one shared AI tick per
    // run — feeding it the frame delta would accumulate only the frames it
    // happened to run on and stretch the authored hint delay by the
    // frame-rate-to-tick-rate ratio (2x at 60 Hz, ~4.8x at 144 Hz), reversing
    // the frame-independence the gate exists to provide. Same shape as
    // `ai_policy_state_tick`'s `AiPolicyTickClock`.
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &crate::ship_plugin::ShipConfigComponent,
            &mut ShipFrequencyHintState,
            Option<&crate::ship::sensors::SensorsAiConfigResource>,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    pictures: Query<(
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
    )>,
    mut writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_tick_hz)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().ai_tick_hz);
    let dt = if hz > 0.0 { 1.0 / hz } else { 0.0 };

    for (
        entity,
        observer_uuid,
        control_sources,
        blackboards,
        ship_config,
        mut hint_state,
        ai_config_comp,
    ) in ships.iter_mut()
    {
        let (tracks, controls) = pictures.get(entity).unwrap_or((None, None));
        let Some(owner) = crate::science::broad::science_owner(controls, tracks.is_some()) else {
            continue;
        };
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3),
        // identical to how the low-fidelity twin `tick_sensors_frequency_hint`
        // and the firing paths read it — never the tactical radar's live
        // selection.
        let locked_target = console_combat_lock(&ship_config.0, &owner, blackboards);

        let locked_target = locked_target.filter(|target| {
            observer_uuid
                .and_then(|observer| {
                    contact_runtime.as_ref().map(|runtime| {
                        crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &observer.0,
                            target,
                        )
                    })
                })
                .unwrap_or_default()
                != crate::gm::operations::contact::ContactMode::Conceal
        });
        let public_target = locked_target.as_deref().and_then(|target| {
            crate::science::contact_routing::alias_for_source(
                &ship_config.0,
                blackboards,
                tracks,
                &owner,
                target,
            )
        });
        let acquired = locked_target.as_deref().and_then(|target| {
            let aliases = crate::science::contact_routing::aliases_for_source(
                &ship_config.0,
                blackboards,
                tracks,
                &owner,
                target,
            );
            crate::science::shield_enquiry::latest_permitted_finding_for_subjects(
                &ship_config.0,
                blackboards,
                &owner,
                &aliases,
            )
        });
        let Some((subject, finding)) = public_target.zip(acquired) else {
            hint_state.0 = Default::default();
            continue;
        };
        let correct_frequency = finding.frequency;
        // The debounce key is held evidence, never the target's current configuration.
        let locked_target = Some(format!(
            "{}:{}/{}",
            subject.len(),
            subject,
            finding.configuration
        ));

        // Per-ship tuning only (issue #738). This used to fall back to the
        // global `SensorsAiConfigResource` Resource while iterating every ship,
        // so whatever last wrote that Resource applied fleet-wide. Nothing in
        // the shipped app writes it — unlike the shields Resource, which
        // `server_app` dual-writes from the PLAYER ship's TOML — so the leak
        // was latent rather than live: it is registered purely for structural
        // symmetry with shields/power, and never seeded. The fallback is now
        // the parse-time default the type already supplies for a ship TOML that
        // omits the section.
        let default_ai_cfg;
        let ai_cfg: &crate::ship::sensors::SensorsAiConfigResource = match ai_config_comp {
            Some(c) => c,
            None => {
                default_ai_cfg = crate::ship::sensors::SensorsAiConfigResource::default();
                &default_ai_cfg
            }
        };

        let input = crate::console_ai::FrequencyHintInput {
            locked_target,
            correct_frequency,
            dt,
            delay_secs: ai_cfg.frequency_hint_delay_secs,
        };

        let output = crate::console_ai::tick_frequency_hint(&mut hint_state.0, &input);

        if let crate::console_ai::FrequencyHintOutput::Hint { frequency } = output {
            let sender_origin = control_sources.0.source_for(&owner);
            let Some(address) = crate::ship::coordination::address_for_system(
                &ship_config.0,
                &crate::science::broad::combat_owner(controls, tracks.is_some())
                    .unwrap_or_else(crate::ship::system_registry::tactical_radar_system_id),
            ) else {
                continue;
            };
            writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                address,
                payload: crate::core::messages::CoordinationPayload::FrequencyHint { frequency },
                presentation: crate::core::messages::CoordinationPresentation::new(
                    "coordination.frequency_hint.title",
                    "coordination.frequency_hint.body",
                )
                .with_body_param("frequency", frequency),
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
                sender_system: owner.clone(),
            });
        }
    }
}

/// Read the existing frozen coordination publication through this System's
/// Console. Neither another Console's host nor another tube's permissions
/// broaden this view. This borrows observations without touching AI memory.
fn console_combat_lock(
    config: &crate::ship::config::ShipConfig,
    system: &crate::core::messages::SystemId,
    blackboards: &crate::server_app::ShipSystemBlackboards,
) -> Option<String> {
    match config
        .observations_for_system(system, &blackboards.0)
        .get(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(board)) => {
            board.combat_lock.clone()
        }
        _ => None,
    }
}

/// Fold each permitted consumer once into each of its ancestors. Consumer
/// order and f32 min/max operations match the original per-group traversal.
fn power_group_observations(
    energy: &phoenix_model::energy::PowerEnergyBlackboard,
) -> Vec<(&str, f32, f32)> {
    use std::collections::BTreeMap;
    // First occurrence supplies the parent, matching the old linear `find`
    // even for malformed duplicate group IDs in diagnostic observations.
    let mut groups = BTreeMap::new();
    for group in &energy.groups {
        groups
            .entry(group.id.as_str())
            .or_insert((group.parent.as_deref(), 1.0f32, 0.0f32, None));
    }
    for (consumer_index, consumer) in energy.consumers.iter().enumerate() {
        let mut cursor = consumer.group.as_ref().map(|id| id.0.as_str());
        for _ in 0..=energy.groups.len() {
            let Some(group) = cursor.and_then(|id| groups.get_mut(id)) else {
                break;
            };
            // A cycle must not apply the same consumer twice to an ancestor.
            if group.3 == Some(consumer_index) {
                break;
            }
            group.3 = Some(consumer_index);
            group.1 = group.1.min(consumer.energy / consumer.capacity);
            group.2 = group.2.max(consumer.heat / consumer.trip_heat);
            cursor = group.0;
        }
    }
    energy
        .groups
        .iter()
        .map(|group| {
            let (_, reserve, heat, _) = groups[group.id.as_str()];
            (group.id.as_str(), reserve, heat)
        })
        .collect()
}

#[cfg(test)]
#[path = "power_group_observations_tests.rs"]
mod power_group_observations_tests;

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
