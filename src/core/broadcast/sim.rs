use bevy::prelude::*;

use crate::core::broadcast::broadcaster::{dispatch, BroadcastKind, Broadcaster};
use crate::core::messages::DeliveryClass;

/// Marker for the simulation (`InProgress`) broadcast phase.
///
/// - Delivery: `Snapshot` (lossy, latest-wins).
/// - Phase gate: none inline — the SimSet chain's
///   `.run_if(in_state(GamePhase::InProgress))` gates the whole set.
/// - Schedule: `FixedUpdate` (where `SimSet` lives since issue #895), inside
///   `SimSet::Broadcast`.
pub struct Sim;

impl BroadcastKind for Sim {
    fn delivery() -> DeliveryClass {
        DeliveryClass::Snapshot
    }

    fn add_dispatch(app: &mut App) {
        app.add_systems(
            FixedUpdate,
            dispatch::<Sim>
                .in_set(crate::sim_sets::SimSet::Broadcast)
                .in_set(crate::sim_sets::FixedStep::SimDispatch),
        );
    }
}

/// Bevy plugin that broadcasts `ServerMessage`s during the `InProgress` phase.
///
/// Use [`Broadcaster::register`] before adding the plugin to `App` to enqueue
/// producers. Each producer is called at the requested cadence and its output
/// is routed to the `Target` resolved from the `Audience`.
pub type SimBroadcaster = Broadcaster<Sim>;

/// Production callback order inside the one simulation dispatch (#1400).
/// Preserve the canonical composition's wire order even when the owning
/// plugins register in a different order. This does not order ECS systems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimProducer {
    Repair,
    Power,
    Shields,
    Weapons,
    SimState,
    Modifier,
    Outbox,
}

impl SimProducer {
    fn rank(self) -> usize {
        match self {
            Self::Repair => 0,
            Self::Power => 1,
            Self::Shields => 2,
            Self::Weapons => 3,
            Self::SimState => 4,
            Self::Modifier => 5,
            Self::Outbox => 6,
        }
    }
}

impl SimBroadcaster {
    /// Build the registration for one production owner. Its single callback
    /// keeps its own audience, cadence and message sequence. Duplicate owners
    /// are refused; unlabelled generic registrations retain insertion order.
    pub(crate) fn for_producer(owner: SimProducer) -> Self {
        Self::for_order(owner.rank())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::broadcast::audience::Audience;
    use crate::core::broadcast::cadence::Cadence;
    use crate::core::messages::ServerMessage;
    use crate::lobby::{LobbyPlugin, OutboundMessage, Sessions};

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
        // Sim dispatch does not gate internally (SimSet handles it).
        // No State<GamePhase> setup needed for these tests.

        // Outbox + collector.
        app.init_resource::<Outbox>();
        app.add_systems(PostUpdate, collect);

        // One fixed step per update (issue #895) — see the Lobby twin.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(1),
        );

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

    fn named_producer(owner: SimProducer, cadence: Cadence) -> SimBroadcaster {
        SimBroadcaster::for_producer(owner).register(Audience::All, cadence, move |world| {
            world.resource_mut::<Calls>().0.push(owner);
            vec![ServerMessage::PlayerLeft {
                token: format!("{owner:?}"),
            }]
        })
    }

    #[derive(Resource, Default)]
    struct Calls(Vec<SimProducer>);

    fn tokens(messages: &[OutboundMessage]) -> Vec<&str> {
        messages
            .iter()
            .map(|message| {
                assert_eq!(message.delivery, DeliveryClass::Snapshot);
                let ServerMessage::PlayerLeft { token } = &message.msg else {
                    panic!("fixture producers emit their own identity");
                };
                token.as_str()
            })
            .collect()
    }

    #[test]
    fn production_producers_keep_canonical_call_and_delivery_order_when_registered_backwards() {
        use SimProducer::{
            Modifier, Outbox as OutboxProducer, Power, Repair, Shields, SimState, Weapons,
        };
        let expected = [
            Repair,
            Power,
            Shields,
            Weapons,
            SimState,
            Modifier,
            OutboxProducer,
        ];
        let mut app = dispatch_app(SimBroadcaster::new());
        app.init_resource::<Calls>();
        for owner in expected.into_iter().rev() {
            app.add_plugins(named_producer(owner, Cadence::OnEvent));
        }
        for _ in 0..2 {
            app.world_mut().resource_mut::<Calls>().0.clear();
            app.world_mut().resource_mut::<Outbox>().0.clear();
            let messages = tick_and_collect(&mut app);
            assert_eq!(app.world().resource::<Calls>().0, expected);
            assert_eq!(
                tokens(&messages),
                ["Repair", "Power", "Shields", "Weapons", "SimState", "Modifier", "Outbox"]
            );
        }
    }

    #[test]
    fn ranked_insertion_keeps_existing_cadence_timers_attached_to_their_producers() {
        use std::time::Duration;
        let mut app = dispatch_app(named_producer(
            SimProducer::Power,
            Cadence::Period(Duration::from_millis(3)),
        ));
        app.init_resource::<Calls>();
        // Insert on both sides of Power. A timer-index mismatch changes which
        // real callback runs at one of the following observed millisecond ticks.
        app.add_plugins(named_producer(SimProducer::Repair, Cadence::Once));
        app.add_plugins(named_producer(
            SimProducer::Shields,
            Cadence::Period(Duration::from_millis(2)),
        ));
        for expected in [
            vec!["Repair"],
            vec!["Shields"],
            vec!["Power"],
            vec!["Shields"],
            vec![],
            vec!["Power", "Shields"],
        ] {
            app.world_mut().resource_mut::<Outbox>().0.clear();
            let messages = tick_and_collect(&mut app);
            assert_eq!(tokens(&messages), expected);
        }
    }

    #[test]
    fn generic_registrations_preserve_insertion_order_across_plugins_and_around_ranked_owners() {
        let build = || {
            let first = SimBroadcaster::new()
                .register(Audience::All, Cadence::OnEvent, |_| {
                    vec![ServerMessage::PlayerLeft { token: "z".into() }]
                })
                .register(Audience::All, Cadence::OnEvent, |_| {
                    vec![ServerMessage::PlayerLeft { token: "a".into() }]
                });
            let mut app = dispatch_app(first);
            app.add_plugins(SimBroadcaster::new().register(
                Audience::All,
                Cadence::OnEvent,
                |_| vec![ServerMessage::PlayerLeft { token: "m".into() }],
            ));
            app
        };
        let mut plain = build();
        let messages = tick_and_collect(&mut plain);
        assert_eq!(tokens(&messages), ["z", "a", "m"]);

        let mut app = build();
        app.init_resource::<Calls>();
        app.add_plugins(named_producer(SimProducer::Power, Cadence::OnEvent));
        app.add_plugins(named_producer(SimProducer::Repair, Cadence::OnEvent));
        let messages = tick_and_collect(&mut app);
        assert_eq!(tokens(&messages), ["Repair", "Power", "z", "a", "m"]);
    }

    #[test]
    #[should_panic(expected = "a production broadcast owner must register exactly one producer")]
    fn duplicate_production_owner_is_refused() {
        let mut app = dispatch_app(named_producer(SimProducer::Repair, Cadence::OnEvent));
        app.add_plugins(named_producer(SimProducer::Repair, Cadence::Once));
    }
}
