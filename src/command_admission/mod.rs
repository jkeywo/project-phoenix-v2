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

pub use policy::{is_command_authorized, station_for_system};

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
            );
    }
}

pub(crate) fn clear_inter_system_queue(mut queue: ResMut<crate::messages::InterSystemQueue>) {
    queue.0.clear();
}

/// Authority gate for intra-system commands. Runs once per tick before
/// `SimSet::Input`, clearing and refilling `AdmittedCommands`.
///
/// A network `ControlSystem` message is admitted iff its token is the live
/// controller of the target system: AI tokens require `operate_ai`; human
/// tokens require `accept_human_input` AND holding the console for that system.
/// Once admitted the command carries no source identity — handlers must not
/// branch on the origin.
pub fn admit_system_commands(
    mut reader: MessageReader<InboundMessage>,
    mut ship_query: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<LocalShip>,
    >,
    sessions: Res<Sessions>,
    ai_registry: Res<crate::ai::server::AiTokenRegistry>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    let Some((ship_entity, control_sources, mut admitted, ship_config)) =
        ship_query.iter_mut().next()
    else {
        return;
    };
    admitted.0.clear();
    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        // Reject registered NPC ai: tokens that don't belong to the player ship.
        // Only tokens present in AiTokenRegistry are NPC-owned; unregistered ai:
        // tokens (player Backfill AI or synthetic test tokens) pass through.
        if ev.token.starts_with("ai:") {
            if let Some(entity) = ai_registry.bevy_entity_for_token(&ev.token) {
                if entity != ship_entity {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        entity = ship_entity,
                        "rejected NPC ai: token {} → {:?}",
                        &ev.token[..ev.token.len().min(12)],
                        std::mem::discriminant(payload),
                    );
                    continue;
                }
            }
        }
        if is_command_authorized(
            &ev.token,
            target,
            payload,
            control_sources,
            &sessions,
            &ship_config.0,
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
            admitted.0.push(crate::messages::AdmittedCommand {
                target: target.clone(),
                payload: payload.clone(),
                response_token: Some(ev.token.clone()),
            });
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
