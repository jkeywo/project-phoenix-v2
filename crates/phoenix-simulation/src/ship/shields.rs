use crate::core::broadcast::sim::SimProducer;
use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::ship_plugin::{CoordinationEnqueue, DeliveredCoordination, ShipConfigComponent};
#[path = "shield_evidence.rs"]
mod shield_evidence;
pub(crate) use shield_evidence::track_damage as track_shield_damage_history;

// Human and individual arc-tree commands share the admitted System boundary
// and the single handle_shields_messages applier.

// ── Components ─────────────────────────────────────────────────────────────────

// ── Resources ──────────────────────────────────────────────────────────────────

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipShieldsPlugin;

impl Plugin for ShipShieldsPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_shields_messages`
        // reads every generated `shield-arc-*` instance (one id per authored
        // facing), so the claim names both its kind and generated-id prefix.
        app.register_admitted_consumer(
            ConsumerMatcher::prefix(crate::ship::system_registry::SHIELD_ARC_KIND, "shield-arc-")
                .with_feedback(
                    crate::command_admission::FeedbackAddress::LowercasePrefixSpelling,
                    &[
                        Payload::SetShieldArcFocus,
                        Payload::SetShieldRaised,
                        Payload::SetShieldRecovery,
                    ],
                ),
        );
        register_shields_replication_lifecycle(app);
        app.add_message::<CoordinationEnqueue>()
            .add_message::<DeliveredCoordination>()
            .init_resource::<ShieldsAiConfigResource>()
            .add_systems(
                FixedUpdate,
                (
                    track_shield_damage_history
                        .in_set(crate::sim_sets::FixedStep::TrackShieldDamageHistory)
                        .in_set(crate::sim_sets::SimSet::Input)
                        .before(emit_shields_coordination),
                    // Apply admitted arc-tree decisions in the same Physics step.
                    // Canonical fixed-step ordering places this before shield work
                    // and damage delivery; publication follows that work.
                    handle_shields_messages
                        .in_set(crate::sim_sets::FixedStep::HandleShieldsMessages)
                        .in_set(crate::sim_sets::SimSet::Physics),
                    emit_shields_coordination
                        .in_set(crate::sim_sets::FixedStep::EmitShieldsCoordination)
                        .in_set(crate::sim_sets::SimSet::Input),
                    // Shared power intake runs before paid shield work, which
                    // completes before projectiles resolve this tick.
                    tick_shields
                        .in_set(crate::sim_sets::FixedStep::TickShields)
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(crate::sim_sets::FixedStep::TickPowerSystem)
                        .after(crate::sim_sets::FixedStep::HandleShieldsMessages)
                        .before(crate::sim_sets::FixedStep::BuildTorpedoTargetSnapshot)
                        .before(crate::sim_sets::FixedStep::TickTorpedoLifecycle),
                    receive_shields_coordination
                        .in_set(crate::sim_sets::SimSet::Modifiers)
                        .after(crate::ship_plugin::process_coordination_lag),
                    // Repair writes a distinct registered key. The proof retains
                    // physical mutable-container incompatibility and Publish order.
                    publish_shields_blackboard
                        .in_set(crate::sim_sets::FixedStep::PublishShieldsBlackboard)
                        .in_set(crate::sim_sets::SimSet::Publish)
                        .ambiguous_with(crate::console::repair::server::publish_repair_blackboard),
                ),
            )
            .add_plugins(shields_state_broadcaster());
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

/// Stable lifecycle key for the Shields console's current-state projection.
pub(crate) const SHIELDS_REPLICATION_KEY: &str = "shields";

fn shields_status_audience() -> Audience {
    Audience::HoldingSystemKind(crate::ship::system_registry::SHIELDS_KIND.into())
}

fn current_shield_status(world: &mut World) -> Option<crate::core::messages::ServerMessage> {
    let shields = world
        .query_filtered::<&ShipShields, With<crate::server_app::LocalShip>>()
        .single(world)
        .ok()?;
    Some(shield_status_message(shields))
}

fn shield_status_message(shields: &ShipShields) -> crate::core::messages::ServerMessage {
    crate::core::messages::ServerMessage::ShieldStatus {
        facings: shield_facing_statuses(&shields.0.snapshot()),
        frequency: shields.frequency(),
    }
}

fn register_shields_replication_lifecycle(app: &mut App) {
    crate::core::broadcast::register_reconnect_projection::<
        ShieldsReconnect,
        ShieldsReconnectParams<'static, 'static>,
    >(app, SHIELDS_REPLICATION_KEY, |params| {
        let (requests, sessions, configs, shields) = params
            .downcast::<ShieldsReconnectParams>()
            .expect("registered owner parameter type");
        reconnect_shields_projection(requests, sessions, configs, shields)
    });
}

/// Project Shields only when `token` is the holder resolved by the same
/// authored-System audience as the periodic live broadcaster. No cache exists
/// or is mutated by this one-shot reconnect projection.
struct ShieldsReconnect;
type ShieldsReconnectParams<'w, 's> = (
    Res<'w, crate::core::broadcast::ReconnectRequests>,
    Option<Res<'w, crate::lobby::Sessions>>,
    Query<
        'w,
        's,
        (
            &'static ShipConfigComponent,
            Option<&'static crate::ship::components::ConsoleHosts>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    Query<'w, 's, &'static ShipShields, With<crate::server_app::LocalShip>>,
);
fn reconnect_shields_projection(
    requests: Res<crate::core::broadcast::ReconnectRequests>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    configs: Query<
        (
            &ShipConfigComponent,
            Option<&crate::ship::components::ConsoleHosts>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    shields: Query<&ShipShields, With<crate::server_app::LocalShip>>,
) -> crate::core::broadcast::ReconnectBatch {
    let config = configs.single().ok();
    requests
        .0
        .iter()
        .map(|token| {
            let holder = sessions
                .as_ref()
                .and_then(|s| {
                    shields_status_audience().resolve(
                        &s.0,
                        config.map(|(c, _)| &c.0),
                        config.and_then(|(_, hosts)| hosts),
                    )
                })
                .is_some_and(|target| target == crate::lobby::Target::Token(token.clone()));
            if !holder {
                return Vec::new();
            }
            let Ok(shields) = shields.single() else {
                return Vec::new();
            };
            vec![shield_status_message(shields)]
        })
        .collect()
}

pub fn shields_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::for_producer(SimProducer::Shields).register(
        shields_status_audience(),
        Cadence::Hz(10.0),
        |world: &mut World| current_shield_status(world).into_iter().collect(),
    )
}

// Individual arc trees consume their permitted Console publications and emit
// through ordinary command admission. Physical damage history is produced in
// Input independently of controller ownership and visual fidelity.

#[cfg(test)]
#[path = "shields_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::ship::shields::*;
