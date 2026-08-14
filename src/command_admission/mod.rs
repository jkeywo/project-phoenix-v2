//! Host command admission — the authoritative gate every `ControlSystem`
//! request passes through before any console router observes it.
//!
//! This module is the single seam named by the PASM entity
//! `host-command-admission`. It owns [`AdmissionSet`], [`AdmissionPlugin`],
//! the [`admit_system_commands`] system, and the pure authority predicate
//! [`is_command_authorized`].
//!
//! Admission is the only place that knows *who* sent a command. Once a
//! command lands in `AdmittedCommands` it carries no source identity, so
//! downstream routers (helm, weapons, repair, ...) can never branch on
//! human-vs-AI origin. See AGENTS.md "Humans and AI are symmetric".
//!
//! The pure "may this token do this?" predicate lives in [`policy`]; this
//! module owns the once-per-tick Bevy seam that applies it.
//!
//! Admission is also where a run's inputs are *written down*: [`log`] holds the
//! tick-stamped, ordered record of everything that crossed the network boundary
//! (issue #898), which — with the master seed — is the whole of what a replay
//! needs. Read that module's docs for what is recorded and what deliberately is
//! not.
//!
//! Extracted from `src/server_app.rs` (issue #736) so that the admission
//! seam is an explicitly importable module rather than an inlined block;
//! `server_app` re-exports these items so existing call sites are unchanged.

use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::messages::ClientMessage;
use crate::server_app::LocalShip;

pub mod ai_emit;
pub mod debug_route;
pub mod log;
pub mod policy;
pub mod router;

// NOTE: `ai_emit` is deliberately NOT re-exported here. It is a `pub mod`, so
// `crate::command_admission::ai_emit::emit_ai_command` is the one public path
// every AI operator imports — a second flattened path would let two spellings
// of the same item drift apart in imports and in the PASM observed edges.
pub use log::{
    reset_command_log, CommandDelay, CommandLog, CommandLogReplay, LoggedCommand, PendingCommands,
    ShipKey,
};
pub use policy::{is_command_authorized, station_for_system};
pub use router::{
    unrouted_command_targets, warn_unrouted_admitted_commands, AdmittedConsumerRegistry,
    ConsumerMatcher, RegisterAdmittedConsumer,
};

/// System set that `admit_system_commands` belongs to. Handlers that run in
/// `FixedUpdate` but outside `SimSet::Input` can use `.after(AdmissionSet)` to
/// guarantee they see a fully-populated `AdmittedCommands`. Admission lives in
/// the fixed schedule with the sim it gates (issue #895): inbound messages are
/// drained per FRAME in `PreUpdate`, and admitting there would clear-and-refill
/// `AdmittedCommands` zero or several times per logical tick.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdmissionSet;

/// Plugin that registers the admission gate and `AdmittedCommands` resource.
/// Include this in plugin-level test apps so handlers have a populated
/// `AdmittedCommands` to read from.
pub struct AdmissionPlugin;

impl Plugin for AdmissionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::InterSystemQueue>()
            .init_resource::<crate::ai::server::AiTokenRegistry>()
            .configure_sets(
                FixedUpdate,
                AdmissionSet
                    .after(crate::lobby::LobbySystemSet)
                    .before(crate::sim_sets::SimSet::Input),
            )
            // Unrouted-command lint (issue #833): warning-only, ordered after
            // every consumer set so it observes the full tick's admitted set
            // before next tick's `admit_system_commands` clears it. Not in
            // `AdmissionSet` (which runs `.before(SimSet::Input)`). Production
            // `server_app` adds the twin system directly since it wires the
            // admission seam inline rather than via this plugin.
            .add_systems(
                FixedUpdate,
                warn_unrouted_admitted_commands.after(crate::sim_sets::SimSet::Broadcast),
            );
        // The seam itself: the command log, the future-tick queue, and the
        // system that writes both (issue #898). Ungated because a plugin-level
        // fixture never leaves `GamePhase::Lobby`.
        register_admission_seam(app, AdmissionGate::EveryTick);
    }
}

/// Whether the admission seam runs only while a game is in progress.
///
/// Production says [`AdmissionGate::InProgressOnly`]: outside `InProgress`
/// there are no ships to route to and the `SimSet` chain that consumes admitted
/// commands is itself gated, so admitting would fill a buffer nothing reads.
/// Fixtures say [`AdmissionGate::EveryTick`], because a bare-`App` harness
/// spawns its ship by hand and never runs the lobby's countdown to `InProgress`
/// — gating them would silently switch admission off and every assertion below
/// would pass vacuously.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionGate {
    /// Only while `GamePhase::InProgress` — the production wiring.
    InProgressOnly,
    /// Every fixed step, whatever the phase — `AdmissionPlugin` and the
    /// `server_app` test fixture.
    EveryTick,
}

/// Register the whole admission seam in one call: the tick-stamped
/// [`log::CommandLog`], the [`log::PendingCommands`] queue it drains,
/// [`log::CommandDelay`], and [`admit_system_commands`] itself.
///
/// One call rather than four, because the four are not independently useful and
/// three of the four ways to get it wrong are silent (issue #898 review). A
/// [`admit_system_commands`] added without the resources fails Bevy's parameter
/// validation and *skips the whole system* — every command silently unadmitted;
/// resources added without the system leave an always-empty log that reads as
/// "this run had no input"; and either half added twice double-counts. Nothing
/// downstream notices any of those, so the fix is to remove the choice: every
/// call site — production, [`AdmissionPlugin`], and the `server_app` fixture —
/// goes through here.
///
/// What is deliberately *not* folded in is [`log::reset_command_log`]. It hangs
/// on `OnEnter(GamePhase::InProgress)` in `server_app` alongside the other
/// run-start systems, because it is a property of the run boundary rather than
/// of the seam, and only an app with a real game phase has one.
pub fn register_admission_seam(app: &mut App, gate: AdmissionGate) {
    log::register_command_log(app);
    let systems = (admit_system_commands, clear_inter_system_queue)
        .in_set(AdmissionSet)
        .after(crate::lobby::LobbySystemSet)
        .before(crate::sim_sets::SimSet::Input);
    match gate {
        AdmissionGate::InProgressOnly => {
            app.add_systems(
                FixedUpdate,
                systems.run_if(in_state(crate::messages::GamePhase::InProgress)),
            );
        }
        AdmissionGate::EveryTick => {
            app.add_systems(FixedUpdate, systems);
        }
    }
}

pub(crate) fn clear_inter_system_queue(mut queue: ResMut<crate::messages::InterSystemQueue>) {
    queue.0.clear();
}

/// The one validate+enqueue seam every admitted command passes through
/// (issue #824). Both callers use it:
///
/// - [`admit_system_commands`] for network `ControlSystem` messages (human
///   tokens and `ai:` tokens alike), and
/// - the console/system AI decide systems (e.g. the per-axis helm AI in
///   `ship::helm_ai`), which emit their decisions as admitted
///   `SystemControlPayload`s into their own ship's `AdmittedCommands` in the
///   same tick rather than round-tripping through the inbound queue.
///
/// Validation is the target ship's own `ControlSourceResolver` via
/// [`is_command_authorized`]: an `ai:` token requires `operate_ai` on the
/// target system; a human token requires `accept_human_input` plus station
/// tenure. On success the command is pushed with its source identity reduced
/// to `response_token` (reply routing only — never behavioural).
///
/// This overload carries no human-seeking host map (issue #984) and does not
/// need one: its only production caller is [`ai_emit::emit_ai_command`], and an
/// `ai:` token is decided on `operate_ai` alone — it returns from
/// [`is_command_authorized`] several branches before station tenure is looked
/// up at all. The network path, which does reach tenure, goes through
/// [`validate_command`] from [`admit_system_commands`] with the routed ship's
/// map in hand.
pub fn validate_and_admit(
    token: &str,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &Sessions,
    config: &crate::ship::config::ShipConfig,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    match validate_command(
        token,
        target,
        payload,
        control_sources,
        sessions,
        config,
        None,
    ) {
        Some(command) => {
            admitted.0.push(command);
            true
        }
        None => false,
    }
}

/// The authority check on its own, returning the source-stripped
/// `AdmittedCommand` it produces — or `None` if the command is refused.
///
/// [`validate_and_admit`] is this plus the push, and remains the seam the AI
/// deciders use because they want the command to land in `AdmittedCommands`
/// *now*. [`admit_system_commands`] needs the accepted command in hand instead:
/// a network command is stamped for the tick it applies on and queued for it
/// (issue #898), which for a non-zero [`log::CommandDelay`] is not this tick.
///
/// Splitting the two keeps one authority call and one place that builds an
/// `AdmittedCommand` — the property #824 introduced `validate_and_admit` for.
pub fn validate_command(
    token: &str,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &Sessions,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
) -> Option<crate::messages::AdmittedCommand> {
    if !is_command_authorized(
        token,
        &target,
        &payload,
        control_sources,
        sessions,
        config,
        hosts,
    ) {
        return None;
    }
    Some(crate::messages::AdmittedCommand {
        target,
        payload,
        response_token: Some(token.to_string()),
    })
}

/// Authority gate for intra-system commands. Runs once per tick before
/// `SimSet::Input`, clearing and refilling every ship's per-entity
/// `AdmittedCommands`.
///
/// Ship-aware (issue #824, per
/// `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §2): human tokens
/// route to the LocalShip's `AdmittedCommands` as before; a registered
/// `ai:` token resolves through `AiTokenRegistry` to the owning entity and
/// is admitted into THAT entity's `AdmittedCommands`, validated by that
/// entity's own `ControlSourceResolver` (`operate_ai` must hold). An
/// unregistered `ai:` token (player Backfill AI, synthetic test tokens)
/// still routes to the LocalShip.
///
/// A network `ControlSystem` message is admitted iff its token is the live
/// controller of the target system on the routed ship: AI tokens require
/// `operate_ai`; human tokens require `accept_human_input` AND holding the
/// console for that system. Once admitted the command carries no source
/// identity — handlers must not branch on the origin.
///
/// # The command log (issue #898)
///
/// This is the seam where a run's inputs are written down. An accepted command
/// is stamped for the tick it applies on — `SimTick` plus [`log::CommandDelay`],
/// which is zero on a local host — then *queued* for that tick and *recorded*
/// in the [`log::CommandLog`] in one step. The system then drains everything now
/// due out of [`log::PendingCommands`] into the routed ships' `AdmittedCommands`.
///
/// With a zero delay the enqueue and the drain happen in the same run of this
/// system, in arrival order, so what a downstream handler observes is exactly
/// what it observed before the log existed. See [`log`] for the ordering rule
/// and for why AI emissions are not recorded here.
///
/// This is also the only place that knows both halves of a command's
/// destination, so it is where they part company: the routed `Entity` goes to
/// [`log::PendingCommands`] with the token-bearing `AdmittedCommand`, and the
/// routed ship's [`log::ShipKey`] goes to the log with everything else. The
/// sender's token stays in this process — see [`log`] for why.
///
/// `SimTick` is taken as `Option<Res<_>>` for the same reason `LogFilterConfig`
/// is: a bare-`App` fixture that never registered the tick would otherwise fail
/// Bevy's parameter validation and skip admission entirely. The fallback is
/// tick 0, which stamps and drains in one step exactly as a zero delay does —
/// it degrades the *stamp*, never the admission.
pub fn admit_system_commands(
    mut reader: MessageReader<InboundMessage>,
    mut ship_query: Query<(
        Entity,
        &crate::ship_plugin::ShipSystemControlSources,
        &mut crate::messages::AdmittedCommands,
        &crate::ship_plugin::ShipConfigComponent,
        Has<LocalShip>,
        Option<&crate::entity_spawner::EntityUuid>,
        // The human-seeking host map (issue #984), absent on a hull that
        // authors no `human_seeking` system and on any ship before
        // `resolve_human_seeking_hosts` has run once.
        Option<&crate::ship_plugin::HumanSeekingHosts>,
    )>,
    sessions: Res<Sessions>,
    ai_registry: Res<crate::ai::server::AiTokenRegistry>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    delay: Res<log::CommandDelay>,
    mut command_log: ResMut<log::CommandLog>,
    mut pending: ResMut<log::PendingCommands>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    let now = sim_tick.map_or(0, |t| t.0);
    let apply_tick = now.saturating_add(delay.0);

    // Clear every ship's admitted commands: the AI decide systems refill
    // their own ship's queue later in the same tick via `validate_and_admit`.
    let mut local_ship: Option<Entity> = None;
    for (entity, _, mut admitted, _, is_local, _, _) in ship_query.iter_mut() {
        admitted.0.clear();
        if is_local {
            local_ship = Some(entity);
        }
    }
    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        // Route: a registered NPC `ai:` token belongs to its own entity's
        // AdmittedCommands; everything else (humans, host page, unregistered
        // `ai:` backfill tokens) belongs to the LocalShip.
        let route = if ev.token.starts_with("ai:") {
            ai_registry.bevy_entity_for_token(&ev.token).or(local_ship)
        } else {
            local_ship
        };
        let Some(route) = route else {
            continue;
        };
        // Read-only: the accepted command is queued for its apply tick rather
        // than pushed here, so this borrow never needs to be mutable.
        let Ok((ship_entity, control_sources, _, ship_config, _, ship_uuid, seeking_hosts)) =
            ship_query.get(route)
        else {
            continue;
        };
        // The log's routing key, taken here because this is where the route was
        // resolved. The `Entity` above delivers inside this process; this names
        // the same ship for anything outside it (issue #898 review) — see
        // `log::ShipKey`.
        let ship_key = log::ShipKey::from_uuid(ship_uuid);
        match validate_command(
            &ev.token,
            target.clone(),
            payload.clone(),
            control_sources,
            &sessions,
            &ship_config.0,
            seeking_hosts,
        ) {
            Some(command) => {
                // Accepted: stamped, queued and recorded together. A refused
                // command reaches neither branch of this — which is the whole
                // of "a rejection never enters the log".
                log::stamp_accepted_command(
                    &mut command_log,
                    &mut pending,
                    apply_tick,
                    route,
                    ship_key,
                    command,
                );
                crate::ptrace!(
                    log,
                    LogCat::Admit,
                    entity = ship_entity,
                    "admitted {:?} → {:?} from token={} for tick {}",
                    target.0,
                    std::mem::discriminant(payload),
                    &ev.token[..ev.token.len().min(8)],
                    apply_tick,
                );
            }
            None => {
                crate::pwarn!(
                    log,
                    LogCat::Admit,
                    entity = ship_entity,
                    "rejected {:?} → {:?} from token={}",
                    target.0,
                    std::mem::discriminant(payload),
                    &ev.token[..ev.token.len().min(8)],
                );
            }
        }
    }

    // Everything stamped for this tick (or, defensively, an earlier one) lands
    // now, in `(tick, arrival)` order — the order the log records.
    for due in pending.drain_due(now) {
        let Ok((_, _, mut admitted, _, _, _, _)) = ship_query.get_mut(due.route) else {
            crate::pwarn!(
                log,
                LogCat::Admit,
                "dropping a command stamped for tick {} — its ship is gone",
                due.tick,
            );
            continue;
        };
        admitted.0.push(due.command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::LobbyPlugin;
    use crate::messages::{
        AdmittedCommands, RepairTarget, StationId, SystemControlPayload, SystemId,
    };
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    use crate::ship::test_support::{drive_one_fixed_step_per_update, TEST_TICK};
    use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

    /// The token whose player holds the repair station in every fixture here.
    /// Deliberately a distinctive string rather than `t1`: the log tests below
    /// assert it appears NOWHERE in a recorded entry, and a two-character token
    /// could match some unrelated substring by luck.
    const HOLDER: &str = "human-session-token";

    /// The fixture ship's `EntityUuid`, and therefore the [`log::ShipKey`] the
    /// log must record for anything routed to it. Spawned deliberately: a ship
    /// without one would exercise the unnamed-key fallback rather than the
    /// production shape.
    const SHIP_UUID: &str = "uuid-fixture-ship";

    /// A one-station hull, so `station_for_system` resolves `repair` → the
    /// `repair` station. Same shape as the `policy` unit tests use.
    fn config() -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "repair"
name = "Engineering"
description = "Damage control."
rank = "Ltn."

[[system]]
id = "repair"
kind = "repair_control"
station = "repair"
"#,
            &["repair_control"],
        )
        .unwrap()
    }

    fn sessions_with_repair_holder(token: &str) -> Sessions {
        let mut sm = crate::lobby::session::SessionManager::new();
        sm.register(token.into(), "Engineer".into()).unwrap();
        sm.set_station(token, Some(StationId("repair".into())));
        Sessions(sm)
    }

    fn dispatch(team_idx: u8) -> SystemControlPayload {
        SystemControlPayload::DispatchRepairTeam {
            team_idx,
            target: RepairTarget::Core,
        }
    }

    fn sources(source: ControlSource) -> ShipSystemControlSources {
        let mut resolver = ControlSourceResolver::new();
        resolver.set(SystemId("repair".into()), source);
        ShipSystemControlSources(resolver)
    }

    /// A `LocalShip` whose repair system answers to `source`, plus the
    /// admission seam, on the fixed clock and driving one logical tick per
    /// `update()`.
    fn admission_app(source: ControlSource) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin);
        crate::sim_tick::register_sim_tick(&mut app);
        app.insert_resource(sessions_with_repair_holder(HOLDER));
        drive_one_fixed_step_per_update(&mut app, TEST_TICK);
        let ship = app
            .world_mut()
            .spawn((
                LocalShip,
                AdmittedCommands::default(),
                ShipConfigComponent(config()),
                sources(source),
                crate::entity_spawner::EntityUuid(SHIP_UUID.into()),
            ))
            .id();
        (app, ship)
    }

    fn send(app: &mut App, token: &str, payload: SystemControlPayload) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg: ClientMessage::ControlSystem {
                    target: SystemId("repair".into()),
                    payload,
                },
            });
    }

    fn admitted(app: &mut App, ship: Entity) -> Vec<SystemControlPayload> {
        app.world()
            .entity(ship)
            .get::<AdmittedCommands>()
            .expect("the ship has an AdmittedCommands")
            .0
            .iter()
            .map(|c| c.payload.clone())
            .collect()
    }

    fn command_log(app: &App) -> &log::CommandLog {
        app.world().resource::<log::CommandLog>()
    }

    /// The baseline: an accepted command applies on the tick it was admitted
    /// on, and the log records it stamped with that same tick.
    ///
    /// `SimTick` advances in `FixedLast`, so the step that admits reads tick 0
    /// and the counter reads 1 once the frame is over — which is why the
    /// recorded stamp is 0 while `SimTick` afterwards is 1.
    #[test]
    fn an_admitted_command_applies_on_the_tick_it_is_stamped_for() {
        let (mut app, ship) = admission_app(ControlSource::Human);
        send(&mut app, HOLDER, dispatch(0));
        app.update();

        assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
        let entries = command_log(&app).entries();
        assert_eq!(entries.len(), 1, "the accepted command must be recorded");
        assert_eq!(entries[0].tick, 0, "stamped with the tick it applied on");
        assert_eq!(entries[0].payload, dispatch(0));
        assert_eq!(
            entries[0].ship,
            log::ShipKey(SHIP_UUID.into()),
            "the log carries the ROUTED SHIP's uuid, which is what makes the \
             entry re-routable on replay — and not the sender's session token, \
             which is a bearer credential"
        );
        assert!(
            !format!("{entries:?}").contains(HOLDER),
            "the holder's session token must not appear anywhere in the log"
        );
        assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 1);
    }

    /// The vellum contract's third rule, at the phoenix seam: a command the
    /// authority gate refuses reaches neither `AdmittedCommands` nor the log.
    #[test]
    fn a_rejected_command_never_enters_the_log() {
        let (mut app, ship) = admission_app(ControlSource::Human);
        send(&mut app, "intruder", dispatch(0));
        app.update();

        assert!(admitted(&mut app, ship).is_empty());
        assert!(
            command_log(&app).is_empty(),
            "a refused command in the log would refuse again on replay, where \
             refusal is a hard error"
        );
        assert!(app.world().resource::<log::PendingCommands>().is_empty());
    }

    /// Two commands arriving in one tick keep their arrival order in both the
    /// admitted buffer and the log — the log *is* the order.
    #[test]
    fn arrival_order_within_a_tick_is_the_recorded_order() {
        let (mut app, ship) = admission_app(ControlSource::Human);
        send(&mut app, HOLDER, dispatch(0));
        send(&mut app, HOLDER, dispatch(1));
        app.update();

        assert_eq!(admitted(&mut app, ship), vec![dispatch(0), dispatch(1)]);
        let recorded: Vec<SystemControlPayload> = command_log(&app)
            .entries()
            .iter()
            .map(|e| e.payload.clone())
            .collect();
        assert_eq!(recorded, vec![dispatch(0), dispatch(1)]);
    }

    /// The future-tick path, which a zero `CommandDelay` otherwise hides: with
    /// a delay of two ticks the command is recorded immediately, stamped for
    /// tick 2, and does not reach `AdmittedCommands` until tick 2 comes round.
    ///
    /// This is the test that makes "logged commands carry the tick they apply
    /// on, and apply on that tick" a claim about the plumbing rather than a
    /// tautology about a delay of nought.
    #[test]
    fn a_delayed_command_waits_for_the_tick_it_is_stamped_for() {
        let (mut app, ship) = admission_app(ControlSource::Human);
        app.insert_resource(log::CommandDelay(2));
        send(&mut app, HOLDER, dispatch(0));

        // Tick 0: recorded and queued, but not yet applied.
        app.update();
        assert!(
            admitted(&mut app, ship).is_empty(),
            "a command stamped for tick 2 must not apply on tick 0"
        );
        assert_eq!(command_log(&app).entries().len(), 1);
        assert_eq!(command_log(&app).entries()[0].tick, 2);
        assert_eq!(app.world().resource::<log::PendingCommands>().len(), 1);

        // Tick 1: still waiting.
        app.update();
        assert!(admitted(&mut app, ship).is_empty());

        // Tick 2: applies, on exactly the tick it was stamped for.
        app.update();
        assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 3);
        assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
        assert!(app.world().resource::<log::PendingCommands>().is_empty());
        assert_eq!(
            command_log(&app).entries().len(),
            1,
            "applying must not record a second time"
        );
    }

    /// Ticks are recorded in non-decreasing order across a run — the smoke-level
    /// property a replay driver depends on.
    #[test]
    fn recorded_ticks_never_go_backwards() {
        let (mut app, _) = admission_app(ControlSource::Human);
        for team in 0..4 {
            send(&mut app, HOLDER, dispatch(team));
            app.update();
        }
        let ticks: Vec<u64> = command_log(&app).entries().iter().map(|e| e.tick).collect();
        assert_eq!(ticks, vec![0, 1, 2, 3]);
        assert!(command_log(&app).ticks_are_monotonic());
    }

    /// What the probe below saw *inside* the fixed step, per step.
    #[derive(Resource, Default)]
    struct SameStepWitness {
        /// One entry per fixed step: how many of that step's `AdmittedCommands`
        /// carried the AI decider's emission at the moment the paired applier
        /// would have run.
        seen_per_step: Vec<usize>,
    }

    /// Option A's load-bearing half: an AI decider's emission lands in
    /// `AdmittedCommands` in the same tick it was decided — the guarantee
    /// `emit_ai_command` documents — and is *not* recorded.
    ///
    /// It is absent because a replay re-derives it: the log plus the seed
    /// regenerate this decision, so logging it would apply it twice.
    ///
    /// # Why the emitter is scheduled rather than called
    ///
    /// The point at issue is "same *tick*", and a tick is a run of the fixed
    /// schedule. Driving `update()` and then poking the emitter in with
    /// `run_system_cached` proves something weaker and differently shaped: it
    /// reads `AdmittedCommands` from *outside* any fixed step, where the buffer
    /// happens to survive between steps, so it would keep passing even if
    /// admission had cleared the emission away mid-step. Here the emitter and
    /// the probe are both registered in `FixedUpdate` `.after(AdmissionSet)`
    /// and chained, so the probe stands exactly where the real paired applier
    /// stands — after the decider, inside the same step, after that step's
    /// admission has done its clear. The assertion is on what the probe
    /// recorded, not on what survived to the end of the frame.
    #[test]
    fn an_ai_emission_keeps_its_same_tick_guarantee_and_stays_out_of_the_log() {
        fn emit(
            mut ships: Query<(
                &ShipSystemControlSources,
                &mut AdmittedCommands,
                Option<&ShipConfigComponent>,
            )>,
            sessions: Res<Sessions>,
        ) {
            for (sources, mut admitted, config) in ships.iter_mut() {
                assert!(
                    ai_emit::emit_ai_command(
                        None,
                        SystemId("repair".into()),
                        SystemControlPayload::DispatchRepairTeam {
                            team_idx: 7,
                            target: RepairTarget::Core,
                        },
                        sources,
                        &sessions,
                        config,
                        &mut admitted,
                    ),
                    "the AI token must be admitted on an AI-controlled system"
                );
            }
        }

        /// Stands where the paired applier stands: same fixed step, after the
        /// decider, after admission.
        fn probe(ships: Query<&AdmittedCommands>, mut witness: ResMut<SameStepWitness>) {
            let seen = ships
                .iter()
                .flat_map(|a| a.0.iter())
                .filter(|c| {
                    matches!(
                        c.payload,
                        SystemControlPayload::DispatchRepairTeam { team_idx: 7, .. }
                    )
                })
                .count();
            witness.seen_per_step.push(seen);
        }

        let (mut app, _) = admission_app(ControlSource::Ai);
        app.init_resource::<SameStepWitness>()
            .add_systems(FixedUpdate, (emit, probe).chain().after(AdmissionSet));

        // Three steps, so the claim is about every tick rather than about one
        // lucky one — a decider that emitted into a buffer the next tick's
        // admission wiped before the applier ran would show a zero here.
        for _ in 0..3 {
            app.update();
        }

        let seen = &app.world().resource::<SameStepWitness>().seen_per_step;
        assert_eq!(seen.len(), 3, "precondition: three fixed steps ran");
        assert!(
            seen.iter().all(|&n| n == 1),
            "every fixed step must show the decider's emission still in \
             AdmittedCommands when the applier runs, in that same step — got \
             {seen:?}"
        );
        assert!(
            command_log(&app).is_empty(),
            "an AI emission never crossed the network boundary, so the log \
             must not carry it — replay re-derives it from the seed"
        );
    }

    /// The recorder keys on the *seam*, not on the origin: a command that
    /// arrives over the inbound boundary is recorded whatever its token looks
    /// like. Branching on `ai:` here would be exactly the human-vs-AI branch
    /// AGENTS.md constraint 6 forbids, and it would drop a remote peer's
    /// orders — which this instance cannot re-derive — from the log.
    #[test]
    fn the_recorder_does_not_ask_where_an_inbound_command_came_from() {
        let (mut app, ship) = admission_app(ControlSource::Ai);
        send(&mut app, ai_emit::AI_BACKFILL_TOKEN, dispatch(0));
        app.update();

        assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
        assert_eq!(
            command_log(&app).entries().len(),
            1,
            "an inbound command is recorded on arrival, not on its token shape"
        );
    }
}
