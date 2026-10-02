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

use crate::core::messages::{
    ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommand, ClientMessage, DeliveryClass,
    ServerMessage,
};
use crate::lobby::{InboundMessage, OutboundMessage, Sessions, Target};
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
    reset_command_log, CommandDelay, CommandLog, CommandLogReplay, CommandOrder, HostSlot,
    LoggedCommand, PendingCommands, ShipKey,
};
pub use policy::{
    authorize_station_command, effective_target_for_command, is_command_authorized,
    station_for_system, StationCommandPolicyFailure,
};
pub use router::{
    unrouted_command_targets, unrouted_commandable_systems, warn_unrouted_admitted_commands,
    AdmittedConsumerRegistry, ConsumerMatcher, RegisterAdmittedConsumer,
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

/// Exact target/payload pairs whose owning consumers complete the correlated
/// operator lifecycle. A correlation never widens command authority: this is
/// only the protocol allowlist for commands that can promise a terminal reply.
pub(crate) fn supports_correlated_action_feedback_for_kind(
    target: &crate::core::messages::SystemId,
    payload: &crate::core::messages::SystemControlPayload,
    target_kind: Option<&str>,
) -> bool {
    use crate::core::messages::SystemControlPayload;

    let authored_terminal_owner = match target_kind {
        Some(crate::ship::system_registry::HELM_IMPULSE_KIND) => matches!(
            payload,
            SystemControlPayload::StartImpulseCharge | SystemControlPayload::CancelImpulse
        ),
        Some(crate::ship::system_registry::HELM_BOOST_KIND) => matches!(
            payload,
            SystemControlPayload::SetBoost { .. } | SystemControlPayload::ToggleBoost
        ),
        Some(crate::ship::system_registry::VIEWSCREEN_KIND) => {
            matches!(payload, SystemControlPayload::SetView { .. })
        }
        Some(crate::ship::system_registry::DOCK_KIND) => {
            matches!(
                payload,
                SystemControlPayload::Dock | SystemControlPayload::Undock
            )
        }
        _ => false,
    };

    authored_terminal_owner
        || (target.0 == crate::ship::system_registry::RED_ALERT_SYSTEM_ID
            && matches!(payload, SystemControlPayload::SetRedAlert { .. }))
        || (target.0 == crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID
            && matches!(payload, SystemControlPayload::SetView { .. }))
        || (target.0 == crate::ship::system_registry::CAPTAIN_SYSTEM_ID
            && matches!(payload, SystemControlPayload::SetObjectivePriority { .. }))
        || (target.0 == crate::ship::system_registry::NAVIGATION_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::SetNavigationWaypoint { .. }
                    | SystemControlPayload::ClearNavigationWaypoint
                    | SystemControlPayload::OrderCivilian { .. }
            ))
        || (target.0 == crate::ship::system_registry::TACTICAL_RADAR_SYSTEM_ID
            && matches!(payload, SystemControlPayload::SetTarget { .. }))
        || (target.0 == crate::ship::system_registry::PHASER_CONTROL_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::SetPhaserMode { .. }
                    | SystemControlPayload::SetStrikeBoost { .. }
            ))
        || (target.0.starts_with("phaser-") && matches!(payload, SystemControlPayload::FirePhaser))
        || (target.0.starts_with("blaster-")
            && matches!(
                payload,
                SystemControlPayload::FireBlaster
                    | SystemControlPayload::ChargeBlasterStart
                    | SystemControlPayload::ChargeBlasterCancel
            ))
        || (target.0.starts_with("torpedo-tube-")
            && matches!(
                payload,
                SystemControlPayload::FireTorpedo { .. }
                    | SystemControlPayload::SetTorpedoVolleyTarget { .. }
            ))
        || (target.0 == crate::ship::system_registry::COMMS_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::Hail { .. }
                    | SystemControlPayload::RespondToMessage { .. }
                    | SystemControlPayload::ClearComms
                    | SystemControlPayload::ShowOnScreen { .. }
            ))
        || (target.0 == crate::ship::system_registry::SENSORS_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::SetScienceTarget { .. }
                    | SystemControlPayload::ScanTarget { .. }
            ))
        || (target.0 == crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::StartImpulseCharge | SystemControlPayload::CancelImpulse
            ))
        || (target.0 == crate::ship::system_registry::HELM_BOOST_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::SetBoost { .. } | SystemControlPayload::ToggleBoost
            ))
        || (target.0 == crate::ship::system_registry::DOCK_KIND
            && matches!(
                payload,
                SystemControlPayload::Dock | SystemControlPayload::Undock
            ))
        || (is_shield_arc_target(&target.0)
            && matches!(payload, SystemControlPayload::SetShieldArcFocus { .. }))
        || (target.0 == crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::SetPowerGroupAllocation { .. }
            ))
        || (target.0 == crate::ship::system_registry::REPAIR_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::DispatchRepairTeam { .. }
                    | SystemControlPayload::RecallRepairTeam { .. }
                    | SystemControlPayload::SetRepairTargetPriority { .. }
                    | SystemControlPayload::DispatchExternalRepair
                    | SystemControlPayload::RecallExternalRepair
            ))
        || (target.0 == crate::ship::system_registry::TRACTOR_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::EngageTractor | SystemControlPayload::ReleaseTractor
            ))
        || (target.0 == crate::ship::system_registry::UMBILICAL_SYSTEM_ID
            && matches!(
                payload,
                SystemControlPayload::StartTransfer | SystemControlPayload::StopTransfer
            ))
}

#[cfg(test)]
fn supports_correlated_action_feedback(
    target: &crate::core::messages::SystemId,
    payload: &crate::core::messages::SystemControlPayload,
) -> bool {
    supports_correlated_action_feedback_for_kind(target, payload, None)
}

fn is_shield_arc_target(target: &str) -> bool {
    target.strip_prefix("shield-arc-").is_some_and(|arc| {
        !arc.is_empty()
            && arc
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

/// Complete one correlated action only after its owning consumer has actually
/// handled it. AI and legacy commands carry no correlation/token and are a
/// deliberate no-op here.
pub(crate) fn finish_action_feedback(
    cmd: &AdmittedCommand,
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    outcome: ActionFeedbackOutcome,
) {
    let (Some(correlation), Some(token)) = (
        cmd.feedback_correlation.as_ref(),
        cmd.response_token.as_deref(),
    ) else {
        return;
    };
    write_action_feedback(outbound, token, correlation, outcome);
}

impl Plugin for AdmissionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::core::messages::InterSystemQueue>()
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
    let systems = (
        admit_system_commands.in_set(crate::sim_sets::FixedStep::AdmitSystemCommands),
        clear_inter_system_queue,
    )
        .in_set(AdmissionSet)
        .after(crate::lobby::LobbySystemSet)
        .before(crate::sim_sets::SimSet::Input);
    match gate {
        AdmissionGate::InProgressOnly => {
            app.add_systems(
                FixedUpdate,
                systems.run_if(in_state(crate::core::messages::GamePhase::InProgress)),
            );
        }
        AdmissionGate::EveryTick => {
            app.add_systems(FixedUpdate, systems);
        }
    }
}

pub(crate) fn clear_inter_system_queue(mut queue: ResMut<crate::core::messages::InterSystemQueue>) {
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
    target: crate::core::messages::SystemId,
    payload: crate::core::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &Sessions,
    config: &crate::ship::config::ShipConfig,
    admitted: &mut crate::core::messages::AdmittedCommands,
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
    target: crate::core::messages::SystemId,
    payload: crate::core::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &Sessions,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
) -> Option<crate::core::messages::AdmittedCommand> {
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
    Some(crate::core::messages::AdmittedCommand {
        target,
        payload,
        response_token: Some(token.to_string()),
        feedback_correlation: None,
    })
}

/// Payload-aware admission for an authenticated GM acting through one
/// authoritatively puppeted Station. This shares effective-target and live
/// availability policy with ordinary human/AI admission, then constructs the
/// same source-stripped command shape. GM attribution remains at the caller's
/// journal/activity seam and never reaches System consumers.
pub fn validate_station_command(
    station: &crate::core::messages::StationId,
    target: crate::core::messages::SystemId,
    payload: crate::core::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    config: &crate::ship::config::ShipConfig,
    hosts: Option<&crate::ship_plugin::HumanSeekingHosts>,
) -> Result<crate::core::messages::AdmittedCommand, StationCommandPolicyFailure> {
    authorize_station_command(station, &target, &payload, control_sources, config, hosts)?;
    Ok(crate::core::messages::AdmittedCommand {
        target,
        payload,
        response_token: None,
        feedback_correlation: None,
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
/// # Applying by `ShipKey` (issue #1116)
///
/// The *acceptance* route above is unchanged — this host's own crew is aboard
/// this host's own ship, which is what `LocalShip` means. What changed is the
/// **apply** route: a due command is delivered to the ship its
/// [`log::ShipKey`] names, resolved at the instant it lands rather than at the
/// instant it was accepted. That closes the gap `src/headless/replay.rs`
/// recorded in so many words ("reading `ShipKey` back out and resolving *it* to
/// a destination … is issue #854's work"), and it is what lets a peer's command
/// — which names a ship this host did not route it to, and may not even have
/// spawned when the frame arrived — reach the right hull. An unnamed key (the
/// bare-`App` fixture shape) falls back to the `Entity` recorded at acceptance,
/// so nothing about a fixture moved.
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
        &mut crate::ship_plugin::ShipSystemControlSources,
        &mut crate::core::messages::AdmittedCommands,
        &crate::ship_plugin::ShipConfigComponent,
        Has<LocalShip>,
        Option<&crate::entities::spawner::EntityUuid>,
        // The human-seeking host map (issue #984), absent on a hull that
        // authors no `human_seeking` system and on any ship before
        // `resolve_human_seeking_hosts` has run once.
        Option<&crate::ship_plugin::HumanSeekingHosts>,
        Option<&crate::entities::spawner::EntitySystemHull>,
    )>,
    sessions: Res<Sessions>,
    ai_registry: Res<crate::ai::server::AiTokenRegistry>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    delay: Res<log::CommandDelay>,
    mut command_log: ResMut<log::CommandLog>,
    mut pending: ResMut<log::PendingCommands>,
    mut mesh_outbox: Option<ResMut<crate::lockstep::MeshOutbox>>,
    mut outbound: Option<ResMut<Messages<OutboundMessage>>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    let now = sim_tick.map_or(0, |t| t.0);
    let apply_tick = now.saturating_add(delay.0);

    // Clear every ship's admitted commands: the AI decide systems refill
    // their own ship's queue later in the same tick via `validate_and_admit`.
    // The uuid→entity index is built in the same walk, because the apply pass
    // below routes by `ShipKey` and a second walk to find that out would be a
    // second copy of "which ship is which".
    let mut local_ship: Option<Entity> = None;
    let mut by_ship_key: std::collections::HashMap<String, Entity> =
        std::collections::HashMap::new();
    for (entity, mut sources, mut admitted, _, is_local, uuid, _, hull) in ship_query.iter_mut() {
        // Restore hull-derived availability before any command can enter.
        sources
            .0
            .set_destroyed(hull.is_some_and(|h| h.0.is_destroyed()));
        admitted.0.clear();
        if is_local {
            local_ship = Some(entity);
        }
        if let Some(uuid) = uuid {
            by_ship_key.insert(uuid.0.clone(), entity);
        }
    }
    for ev in reader.read() {
        let (target, payload, correlation) = match &ev.msg {
            ClientMessage::ControlSystem { target, payload } => (target, payload, None),
            ClientMessage::ControlSystemCorrelated {
                correlation,
                target,
                payload,
            } => (target, payload, Some(correlation.clone())),
            _ => continue,
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
            if let Some(correlation) = correlation.as_ref() {
                write_action_feedback(
                    &mut outbound,
                    &ev.token,
                    correlation,
                    ActionFeedbackOutcome::Refused,
                );
            }
            continue;
        };
        // Read-only: the accepted command is queued for its apply tick rather
        // than pushed here, so this borrow never needs to be mutable.
        let Ok((ship_entity, control_sources, _, ship_config, _, ship_uuid, seeking_hosts, _)) =
            ship_query.get(route)
        else {
            if let Some(correlation) = correlation.as_ref() {
                write_action_feedback(
                    &mut outbound,
                    &ev.token,
                    correlation,
                    ActionFeedbackOutcome::Refused,
                );
            }
            continue;
        };
        // Resolve the authored instance before evaluating the correlation
        // allowlist. Discrete Helm, viewscreen and dock SystemIds are
        // designer-owned; their registered kind identifies the terminal
        // consumer. Continuous Helm axes deliberately stay uncorrelated.
        let target_kind = ship_config
            .0
            .systems
            .iter()
            .find(|system| system.id == *target)
            .map(|system| system.kind.as_str());
        if let Some(correlation) = correlation
            .as_ref()
            .filter(|_| !supports_correlated_action_feedback_for_kind(target, payload, target_kind))
        {
            write_action_feedback(
                &mut outbound,
                &ev.token,
                correlation,
                ActionFeedbackOutcome::Refused,
            );
            continue;
        }
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
            Some(mut command) => {
                command.feedback_correlation = correlation.clone();
                // Accepted: stamped and queued for the tick it applies on. A
                // refused command reaches neither branch of this — which is the
                // whole of "a rejection never enters the log", since the log is
                // written when the queue applies.
                let order = log::stamp_accepted_command(
                    &mut pending,
                    apply_tick,
                    None,
                    route,
                    ship_key.clone(),
                    command.clone(),
                );
                // …and, if this host is in a fleet, told to the fleet. Staged
                // here rather than derived later because this is the one place
                // that has the accepted command, its agreed order and its
                // routed ship in hand at once; `seal_tick_frame` turns the
                // tick's staging into one frame.
                if let Some(outbox) = mesh_outbox.as_deref_mut() {
                    outbox.stage(crate::lockstep::mesh_command(
                        apply_tick, order, ship_key, &command,
                    ));
                }
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
                if let Some(correlation) = correlation.as_ref() {
                    write_action_feedback(
                        &mut outbound,
                        &ev.token,
                        correlation,
                        ActionFeedbackOutcome::Refused,
                    );
                }
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
    // now, in `(tick, order)` order — and is written into the log as it lands,
    // so the record is the applied sequence and every host in the fleet writes
    // the same one.
    for due in pending.drain_due(now, &mut command_log) {
        // The ship is named, not remembered: `ShipKey` is resolved against this
        // tick's world, and the `Entity` captured at acceptance is only the
        // fallback for a ship that never had a uuid to be named by.
        let route = by_ship_key.get(&due.ship.0).copied().unwrap_or(due.route);
        let Ok((_, _, mut admitted, _, _, _, _, hull)) = ship_query.get_mut(route) else {
            if let (Some(correlation), Some(token)) = (
                due.command.feedback_correlation.as_ref(),
                due.command.response_token.as_deref(),
            ) {
                write_action_feedback(
                    &mut outbound,
                    token,
                    correlation,
                    ActionFeedbackOutcome::Refused,
                );
            }
            crate::pwarn!(
                log,
                LogCat::Admit,
                "dropping a command stamped for tick {} on ship {:?} — that ship \
                 is not in this world",
                due.tick,
                due.ship.0,
            );
            continue;
        };
        if hull.is_some_and(|h| h.0.is_destroyed()) {
            if let (Some(correlation), Some(token)) = (
                due.command.feedback_correlation.as_ref(),
                due.command.response_token.as_deref(),
            ) {
                write_action_feedback(
                    &mut outbound,
                    token,
                    correlation,
                    ActionFeedbackOutcome::Refused,
                );
            }
            continue;
        }
        admitted.0.push(due.command);
    }
}

/// Emit one reliable, token-targeted lifecycle result.  `Option` keeps the
/// admission system valid in reduced apps that do not register outbound
/// messages; production's LobbyPlugin always does.
fn write_action_feedback(
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    token: &str,
    correlation: &ActionCorrelationId,
    outcome: ActionFeedbackOutcome,
) {
    let Some(messages) = outbound.as_deref_mut() else {
        return;
    };
    messages.write(OutboundMessage {
        target: Target::Token(token.to_string()),
        msg: ServerMessage::ActionFeedback {
            correlation: correlation.clone(),
            outcome,
        },
        delivery: DeliveryClass::Reliable,
    });
}

/// Complete one admitted semantic action at the system that owns its gameplay
/// result. Commands without a response token/correlation are ordinary AI or
/// legacy traffic and deliberately produce no lifecycle message.
pub(crate) fn finish_admitted_action_feedback(
    outbound: &mut Option<ResMut<Messages<OutboundMessage>>>,
    command: &crate::core::messages::AdmittedCommand,
    outcome: ActionFeedbackOutcome,
) {
    let (Some(token), Some(correlation)) = (
        command.response_token.as_deref(),
        command.feedback_correlation.as_ref(),
    ) else {
        return;
    };
    write_action_feedback(outbound, token, correlation, outcome);
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
