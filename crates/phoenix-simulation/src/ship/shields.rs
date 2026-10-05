use crate::core::broadcast::sim::SimProducer;
use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::ship_plugin::{CoordinationEnqueue, DeliveredCoordination, ShipConfigComponent};

// `ShieldArcCmd` / `ShieldArcIntents` (issue #692's decide/apply transport)
// were retired by issue #826: `console_ai::server::ai_shield_focus` now emits
// admitted `SetShieldArcFocus` payloads through
// `command_admission::validate_and_admit`, and `handle_shields_messages`
// below is the single applier for human and AI commands alike.

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
                    &[Payload::SetShieldArcFocus],
                ),
        );
        register_shields_replication_lifecycle(app);
        app.add_message::<CoordinationEnqueue>()
            .add_message::<DeliveredCoordination>()
            .init_resource::<ShieldsAiConfigResource>()
            .add_systems(
                FixedUpdate,
                (
                    // In `SimSet::Physics`, not Input (issue #826):
                    // `admit_system_commands` clears every ship's
                    // `AdmittedCommands` before Input each tick, and the AI
                    // decide system (`console_ai::server::ai_shield_focus`,
                    // Physics) refills it same-tick via `validate_and_admit`
                    // — so the applier must consume in Physics *after* the AI
                    // emit or AI commands would be silently lost.
                    // `ConsoleAiPlugin` declares the explicit
                    // `ai_shield_focus.before(handle_shields_messages)` edge;
                    // set ordering keeps this before `tick_shields`
                    // (Modifiers) and `publish_shields_blackboard` (Publish).
                    handle_shields_messages
                        .in_set(crate::sim_sets::FixedStep::HandleShieldsMessages)
                        .in_set(crate::sim_sets::SimSet::Physics),
                    emit_shields_coordination
                        .in_set(crate::sim_sets::FixedStep::EmitShieldsCoordination)
                        .in_set(crate::sim_sets::SimSet::Input),
                    // `translate_power_modifiers` is ALSO in `Modifiers`, so
                    // set membership alone leaves their order unspecified and
                    // `tick_shields` would read a one-tick-stale
                    // `ModifierSlot::ShieldRegen` (issue #952). The explicit
                    // edge makes a same-tick reallocation land on this tick's
                    // regen. Dropped harmlessly in harnesses that register
                    // `ShipShieldsPlugin` without the simulation's modifier
                    // translators.
                    tick_shields
                        .in_set(crate::sim_sets::FixedStep::TickShields)
                        .in_set(crate::sim_sets::SimSet::Modifiers)
                        .after(crate::modifiers::coordination::translate_power_modifiers),
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
    Query<'w, 's, &'static ShipConfigComponent, With<crate::server_app::LocalShip>>,
    Query<'w, 's, &'static ShipShields, With<crate::server_app::LocalShip>>,
);
fn reconnect_shields_projection(
    requests: Res<crate::core::broadcast::ReconnectRequests>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    configs: Query<&ShipConfigComponent, With<crate::server_app::LocalShip>>,
    shields: Query<&ShipShields, With<crate::server_app::LocalShip>>,
) -> crate::core::broadcast::ReconnectBatch {
    let config = configs.single().ok();
    requests
        .0
        .iter()
        .map(|token| {
            let holder = sessions
                .as_ref()
                .and_then(|s| shields_status_audience().resolve(&s.0, config.map(|c| &c.0)))
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

// ── Systems ────────────────────────────────────────────────────────────────────

// ── Wire conversion ──────────────────────────────────────────────────────────

// ── Blackboard publish ─────────────────────────────────────────────────────────

// ── Tests ──────────────────────────────────────────────────────────────────────

// ── AI controller ────────────────────────────────────────────────────────────────
//
// The fused decide+mutate `operate_shields_ai` system (damage tracking,
// health monitoring, focus decision, threat-bearing override) was split in
// issue #692 into a decide system + apply adapter. Issue #826 retired the
// adapter and its `ShieldArcIntents` transport: `console_ai::server::
// ai_shield_focus` (decision, unchanged) now emits admitted
// `SetShieldArcFocus` payloads through `command_admission::validate_and_admit`
// with the ship's own `ai:<uuid>` token, and `handle_shields_messages` above
// applies them — the single truth-integration point for human and AI alike.
// `ShieldsDamageHistory`, `ShieldsAiConfigResource`, and
// `PendingShieldsThreatBearing` remains here since it is shield-domain state:
// the Shields-owned coordination receiver writes it and the decide system
// consumes it.

#[cfg(test)]
#[path = "shields_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::ship::shields::*;
