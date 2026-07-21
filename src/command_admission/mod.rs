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
//! Extracted from `src/server_app.rs` (issue #736) so that the admission
//! seam is an explicitly importable module rather than an inlined block;
//! `server_app` re-exports these items so existing call sites are unchanged.

use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::messages::ClientMessage;
use crate::server_app::LocalShip;

pub mod policy;
pub mod router;

pub use policy::{is_command_authorized, station_for_system};
pub use router::{
    unrouted_command_targets, warn_unrouted_admitted_commands, AdmittedConsumerRegistry,
    ConsumerMatcher, RegisterAdmittedConsumer,
};

/// System set that `admit_system_commands` belongs to. Handlers that run in
/// `Update` but outside `SimSet::Input` can use `.after(AdmissionSet)` to
/// guarantee they see a fully-populated `AdmittedCommands`.
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
                Update,
                AdmissionSet
                    .after(crate::lobby::LobbySystemSet)
                    .before(crate::sim_sets::SimSet::Input),
            )
            .add_systems(
                Update,
                (admit_system_commands, clear_inter_system_queue).in_set(AdmissionSet),
            )
            // Unrouted-command lint (issue #833): warning-only, ordered after
            // every consumer set so it observes the full tick's admitted set
            // before next tick's `admit_system_commands` clears it. Not in
            // `AdmissionSet` (which runs `.before(SimSet::Input)`). Production
            // `server_app` adds the twin system directly since it wires the
            // admission seam inline rather than via this plugin.
            .add_systems(
                Update,
                warn_unrouted_admitted_commands.after(crate::sim_sets::SimSet::Broadcast),
            );
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
pub fn validate_and_admit(
    token: &str,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    control_sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &Sessions,
    config: &crate::ship::config::ShipConfig,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    if !is_command_authorized(token, &target, &payload, control_sources, sessions, config) {
        return false;
    }
    admitted.0.push(crate::messages::AdmittedCommand {
        target,
        payload,
        response_token: Some(token.to_string()),
    });
    true
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
pub fn admit_system_commands(
    mut reader: MessageReader<InboundMessage>,
    mut ship_query: Query<(
        Entity,
        &crate::ship_plugin::ShipSystemControlSources,
        &mut crate::messages::AdmittedCommands,
        &crate::ship_plugin::ShipConfigComponent,
        Has<LocalShip>,
    )>,
    sessions: Res<Sessions>,
    ai_registry: Res<crate::ai::server::AiTokenRegistry>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    // Clear every ship's admitted commands: the AI decide systems refill
    // their own ship's queue later in the same tick via `validate_and_admit`.
    let mut local_ship: Option<Entity> = None;
    for (entity, _, mut admitted, _, is_local) in ship_query.iter_mut() {
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
        let Ok((ship_entity, control_sources, mut admitted, ship_config, _)) =
            ship_query.get_mut(route)
        else {
            continue;
        };
        if validate_and_admit(
            &ev.token,
            target.clone(),
            payload.clone(),
            control_sources,
            &sessions,
            &ship_config.0,
            &mut admitted,
        ) {
            crate::ptrace!(
                log,
                LogCat::Admit,
                entity = ship_entity,
                "admitted {:?} → {:?} from token={}",
                target.0,
                std::mem::discriminant(payload),
                &ev.token[..ev.token.len().min(8)],
            );
        } else {
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
