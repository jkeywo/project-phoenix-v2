use bevy::prelude::*;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::lobby::{OutboundMessage, Sessions};
use crate::messages::ServerMessage;

// ── Registration types ─────────────────────────────────────────────────────

/// A boxed producer function: given exclusive world access, yields zero or more
/// `ServerMessage`s to send this tick.  Returning an empty `Vec` skips the
/// broadcast for this tick.
///
/// Exclusive (`&mut World`) access lets producers drain mutable resources (e.g.
/// event queues) without needing a separate drain system.
pub type Producer = Arc<dyn Fn(&mut World) -> Vec<ServerMessage> + Send + Sync>;

/// A single registered broadcast entry for the simulation phase.
pub struct SimRegistration {
    pub audience: Audience,
    pub cadence: Cadence,
    pub producer: Producer,
}

// ── Resource: live registry kept during simulation ─────────────────────────

#[derive(Resource)]
pub struct SimBroadcastRegistry {
    pub registrations: Vec<SimRegistration>,
    /// Per-registration cadence timers (index-matched to `registrations`).
    pub timers: Vec<Option<Timer>>,
}

impl SimBroadcastRegistry {
    fn new() -> Self {
        Self {
            registrations: Vec::new(),
            timers: Vec::new(),
        }
    }

    fn add(&mut self, reg: SimRegistration) {
        let timer = cadence_timer(&reg.cadence);
        self.registrations.push(reg);
        self.timers.push(timer);
    }
}

fn cadence_timer(cadence: &Cadence) -> Option<Timer> {
    match cadence {
        Cadence::Hz(hz) => {
            if *hz > 0.0 {
                Some(Timer::from_seconds(1.0 / hz, TimerMode::Repeating))
            } else {
                None
            }
        }
        Cadence::Period(d) => Some(Timer::new(*d, TimerMode::Repeating)),
        // `OnEvent` producers are called every frame and emit by returning a
        // non-empty Vec.
        Cadence::OnEvent => None,
        // `Once` fires on the very first tick (zero-duration timer).
        Cadence::Once => Some(Timer::from_seconds(0.0, TimerMode::Once)),
    }
}

// ── Plugin builder ─────────────────────────────────────────────────────────

/// Bevy plugin that broadcasts `ServerMessage`s during the `InProgress` phase.
///
/// Use [`SimBroadcaster::register`] before adding the plugin to `App` to
/// enqueue producers. Each producer is called at the requested cadence and its
/// output is routed to the `Target` resolved from the `Audience`.
pub struct SimBroadcaster {
    pending: Vec<SimRegistration>,
}

impl Default for SimBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl SimBroadcaster {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Register a producer that fires at `cadence` to `audience` during the
    /// simulation (`InProgress`) phase.
    ///
    /// The producer receives exclusive `&mut World` access so it can drain
    /// mutable resources (e.g. event queues).  Read-only producers may simply
    /// call `world.resource::<T>()` as usual.
    pub fn register<F>(mut self, audience: Audience, cadence: Cadence, producer: F) -> Self
    where
        F: Fn(&mut World) -> Vec<ServerMessage> + Send + Sync + 'static,
    {
        self.pending.push(SimRegistration {
            audience,
            cadence,
            producer: Arc::new(producer),
        });
        self
    }
}

impl Plugin for SimBroadcaster {
    fn is_unique(&self) -> bool {
        false
    }

    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<SimBroadcastRegistry>() {
            app.insert_resource(SimBroadcastRegistry::new());
            app.add_systems(
                Update,
                dispatch_sim_broadcasts.in_set(crate::sim_sets::SimSet::Broadcast),
            );
        }
        let mut registry = app.world_mut().resource_mut::<SimBroadcastRegistry>();
        for reg in &self.pending {
            registry.add(SimRegistration {
                audience: reg.audience.clone(),
                cadence: reg.cadence.clone(),
                producer: reg.producer.clone(),
            });
        }
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
/// Gated by the SimSet chain's `.run_if(in_state(GamePhase::InProgress))`.
fn dispatch_sim_broadcasts(world: &mut World) {
    // Tick all cadence timers.
    let dt = world.resource::<Time>().delta();
    {
        let mut registry = world.resource_mut::<SimBroadcastRegistry>();
        for timer_opt in registry.timers.iter_mut() {
            if let Some(t) = timer_opt.as_mut() {
                t.tick(dt);
            }
        }
    }

    // Collect ship config before borrowing registry/sessions to avoid borrow conflicts.
    let ship_config_opt: Option<crate::ship_plugin::ShipConfigComponent> = world
        .query_filtered::<&crate::ship_plugin::ShipConfigComponent, With<crate::simulation::Ship>>()
        .single(world)
        .ok()
        .cloned();

    // Collect (target, producer) for entries that should fire this tick.
    // We clone Arcs so we can release the borrow on `registry` before calling
    // into the world (producers need exclusive world access).
    let ready: Vec<(crate::lobby_handler::Target, Producer)> = {
        let registry = world.resource::<SimBroadcastRegistry>();
        let sessions = world.resource::<Sessions>();
        registry
            .registrations
            .iter()
            .enumerate()
            .filter_map(|(i, reg)| {
                // Check cadence timer.
                let should_fire = match &registry.timers[i] {
                    Some(t) => t.just_finished(),
                    None => true, // OnEvent: always let the producer decide
                };
                if !should_fire {
                    return None;
                }
                // Resolve audience → target.
                let target = reg
                    .audience
                    .resolve(&sessions.0, ship_config_opt.as_ref().map(|c| &c.0))?;
                Some((target, reg.producer.clone()))
            })
            .collect()
    };

    // Call producers and write resulting messages.
    for (target, producer) in ready {
        for msg in producer(world) {
            world.write_message(OutboundMessage {
                target: target.clone(),
                msg,
            });
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::broadcast::audience::Audience;
    use crate::core::broadcast::cadence::Cadence;
    use crate::lobby::{LobbyPlugin, OutboundMessage, Sessions};
    use crate::messages::ServerMessage;

    // ── Test harness ──────────────────────────────────────────────────────

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    /// Build a minimal `App` for dispatch tests.
    ///
    /// - `LobbyPlugin` registers the message bus (`add_message::<OutboundMessage>()`).
    /// - `CurrentPhase` is set to `InProgress` so the dispatch gate passes.
    /// - One player is registered so `Audience::All` can resolve a `Target`.
    /// - A `collect` PostUpdate system drains outbound messages into `Outbox`.
    fn dispatch_app(broadcaster: SimBroadcaster) -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin);
        app.add_plugins(bevy::time::TimePlugin);

        // Wire the broadcaster under test.
        app.add_plugins(broadcaster);

        // Register one player and advance to InProgress.
        {
            let mut sm = app.world_mut().resource_mut::<Sessions>();
            sm.0.register("alice".to_string(), "Alice".to_string())
                .unwrap();
        }
        // dispatch_sim_broadcasts no longer gates internally (SimSet handles it).
        // No State<GamePhase> setup needed for these tests.

        // Outbox + collector.
        app.init_resource::<Outbox>();
        app.add_systems(PostUpdate, collect);

        app
    }

    /// Advance one update tick and return all outbound messages collected.
    fn tick_and_collect(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        msgs
    }

    // ── Audience::All resolution ───────────────────────────────────────────

    /// A producer registered with `Audience::All` and `Cadence::Once` must
    /// broadcast to all connected sessions.  The key assertion is that the
    /// message is present at all — i.e. `Audience::All` resolves to a valid
    /// `Target` with at least one connected player.
    #[test]
    fn audience_all_resolves_and_delivers_message() {
        let broadcaster =
            SimBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });

        let mut app = dispatch_app(broadcaster);
        let msgs = tick_and_collect(&mut app);

        assert!(
            msgs.iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "expected GameStarted from Audience::All producer, got: {:?}",
            msgs.iter().map(|m| &m.msg).collect::<Vec<_>>(),
        );
    }

    // ── Cadence::OnEvent drain-once semantics ─────────────────────────────

    /// A producer registered with `Cadence::OnEvent` is called every frame.
    /// When it has pending work it emits messages; when the queue is empty it
    /// returns nothing.  This test verifies the drain-once contract:
    ///
    /// - Events queued before a tick are all broadcast in that tick.
    /// - A second tick with no new events produces no messages.
    #[test]
    fn on_event_producer_drains_once_per_frame() {
        use std::sync::{Arc, Mutex};
        let queue: Arc<Mutex<Vec<ServerMessage>>> = Arc::new(Mutex::new(vec![]));
        let queue_clone = queue.clone();

        let broadcaster = SimBroadcaster::new().register(
            Audience::All,
            Cadence::OnEvent,
            move |_world: &mut World| {
                let mut q = queue_clone.lock().unwrap();
                std::mem::take(&mut *q)
            },
        );

        let mut app = dispatch_app(broadcaster);

        // Pre-load two events.
        {
            let mut q = queue.lock().unwrap();
            q.push(ServerMessage::GameStarted);
            q.push(ServerMessage::GameStarted);
        }

        // Tick 1: both events should be drained and broadcast.
        let msgs1 = tick_and_collect(&mut app);
        let count1 = msgs1
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count();
        assert_eq!(
            count1, 2,
            "tick 1: expected 2 GameStarted messages, got {count1}"
        );

        // Clear the outbox, then tick again with an empty queue.
        app.world_mut().resource_mut::<Outbox>().0.clear();
        let msgs2 = tick_and_collect(&mut app);
        let count2 = msgs2
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count();
        assert_eq!(
            count2, 0,
            "tick 2: expected 0 messages after drain, got {count2}"
        );
    }
}
