use bevy::prelude::*;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::lobby::{OutboundMessage, Sessions};
use crate::messages::{DeliveryClass, ServerMessage};

// ── Registration types ─────────────────────────────────────────────────────

/// A boxed producer function: given exclusive world access, yields zero or more
/// `ServerMessage`s to send this tick.  Returning an empty `Vec` skips the
/// broadcast for this tick.
///
/// Exclusive (`&mut World`) access lets producers drain mutable resources (e.g.
/// event queues) without needing a separate drain system.
pub type Producer = Arc<dyn Fn(&mut World) -> Vec<ServerMessage> + Send + Sync>;

/// A single registered broadcast entry.
pub struct Registration {
    pub audience: Audience,
    pub cadence: Cadence,
    pub producer: Producer,
}

// ── Phase kind: the three axes on which broadcast phases differ ────────────

/// Zero-sized marker trait parameterising [`Broadcaster`] over the three axes
/// on which broadcast phases actually differ:
///
/// 1. **Delivery class** — [`BroadcastKind::delivery`] stamps every message
///    the phase's producers emit.
/// 2. **Phase gate** — [`BroadcastKind::phase_allows`] is an optional inline
///    predicate evaluated at the top of dispatch; the default is ungated
///    (external scheduling, e.g. a set-level `run_if`, provides the gate).
/// 3. **Schedule** — [`BroadcastKind::add_dispatch`] registers
///    [`dispatch::<Self>`] with the phase's ordering constraints.
///
/// Adding a third phase is a new marker type, not a new file.
pub trait BroadcastKind: Send + Sync + 'static {
    /// Delivery class for every message this phase's producers return.
    fn delivery() -> DeliveryClass;

    /// Inline phase gate. Return `false` to skip dispatch entirely this frame.
    /// Defaults to ungated.
    fn phase_allows(_world: &World) -> bool {
        true
    }

    /// Register `dispatch::<Self>` into the app's `Update` schedule with this
    /// phase's ordering constraints.
    fn add_dispatch(app: &mut App);
}

// ── Resource: live registry per phase ──────────────────────────────────────

/// Live registry of broadcast entries for phase `M`.  The marker keeps each
/// phase's registry a distinct `Resource` identity in one `World`.
#[derive(Resource)]
pub struct BroadcastRegistry<M: BroadcastKind> {
    pub registrations: Vec<Registration>,
    /// Per-registration cadence timers (index-matched to `registrations`).
    pub timers: Vec<Option<Timer>>,
    _marker: PhantomData<M>,
}

impl<M: BroadcastKind> BroadcastRegistry<M> {
    fn new() -> Self {
        Self {
            registrations: Vec::new(),
            timers: Vec::new(),
            _marker: PhantomData,
        }
    }

    fn add(&mut self, reg: Registration) {
        let timer = cadence_timer(&reg.cadence);
        self.registrations.push(reg);
        self.timers.push(timer);
    }
}

pub(crate) fn cadence_timer(cadence: &Cadence) -> Option<Timer> {
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

/// Bevy plugin that broadcasts `ServerMessage`s for phase `M`.
///
/// Use [`Broadcaster::register`] before adding the plugin to `App` to enqueue
/// producers. Each producer is called at the requested cadence and its output
/// is routed to the `Target` resolved from the `Audience`.
pub struct Broadcaster<M: BroadcastKind> {
    pending: Vec<Registration>,
    _marker: PhantomData<M>,
}

impl<M: BroadcastKind> Default for Broadcaster<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: BroadcastKind> Broadcaster<M> {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Register a producer that fires at `cadence` to `audience` during this
    /// broadcaster's phase.
    ///
    /// The producer receives exclusive `&mut World` access so it can drain
    /// mutable resources (e.g. event queues).  Read-only producers may simply
    /// call `world.resource::<T>()` as usual.
    pub fn register<F>(mut self, audience: Audience, cadence: Cadence, producer: F) -> Self
    where
        F: Fn(&mut World) -> Vec<ServerMessage> + Send + Sync + 'static,
    {
        self.pending.push(Registration {
            audience,
            cadence,
            producer: Arc::new(producer),
        });
        self
    }
}

impl<M: BroadcastKind> Plugin for Broadcaster<M> {
    fn is_unique(&self) -> bool {
        false
    }

    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<BroadcastRegistry<M>>() {
            app.insert_resource(BroadcastRegistry::<M>::new());
            M::add_dispatch(app);
        }
        let mut registry = app.world_mut().resource_mut::<BroadcastRegistry<M>>();
        for reg in &self.pending {
            registry.add(Registration {
                audience: reg.audience.clone(),
                cadence: reg.cadence.clone(),
                producer: reg.producer.clone(),
            });
        }
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
///
/// Gating is per-phase: `M::phase_allows` may short-circuit inline, and/or the
/// schedule position chosen by `M::add_dispatch` may carry an external
/// `run_if` (e.g. the SimSet chain's `in_state(GamePhase::InProgress)`).
pub fn dispatch<M: BroadcastKind>(world: &mut World) {
    if !M::phase_allows(world) {
        return;
    }

    // Tick all cadence timers.
    let dt = world.resource::<Time>().delta();
    {
        let mut registry = world.resource_mut::<BroadcastRegistry<M>>();
        for timer_opt in registry.timers.iter_mut() {
            if let Some(t) = timer_opt.as_mut() {
                t.tick(dt);
            }
        }
    }

    // Collect ship config before borrowing registry/sessions to avoid borrow conflicts.
    let ship_config_opt: Option<crate::ship_plugin::ShipConfigComponent> = world
        .query_filtered::<&crate::ship_plugin::ShipConfigComponent, With<crate::simulation::LocalShip>>()
        .single(world)
        .ok()
        .cloned();

    // Collect (target, producer) for entries that should fire this tick.
    // We clone Arcs so we can release the borrow on `registry` before calling
    // into the world (producers need exclusive world access).
    let ready: Vec<(crate::lobby_handler::Target, Producer)> = {
        let registry = world.resource::<BroadcastRegistry<M>>();
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
                delivery: M::delivery(),
            });
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::broadcast::lobby::LobbyBroadcaster;
    use crate::core::broadcast::sim::SimBroadcaster;
    use crate::core::broadcast::{Lobby, Sim};
    use crate::lobby::{LobbyPlugin, OutboundMessage, Sessions};
    use crate::messages::{GamePhase, ServerMessage};
    use std::time::Duration;

    // ── cadence_timer unit tests ──────────────────────────────────────────

    #[test]
    fn cadence_timer_hz_builds_repeating_timer_with_inverse_period() {
        let t = cadence_timer(&Cadence::Hz(10.0)).expect("Hz > 0 should build a timer");
        assert_eq!(t.mode(), TimerMode::Repeating);
        assert!((t.duration().as_secs_f32() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn cadence_timer_non_positive_hz_yields_none() {
        assert!(cadence_timer(&Cadence::Hz(0.0)).is_none());
        assert!(cadence_timer(&Cadence::Hz(-1.0)).is_none());
    }

    #[test]
    fn cadence_timer_period_builds_repeating_timer() {
        let d = Duration::from_millis(250);
        let t = cadence_timer(&Cadence::Period(d)).expect("Period should build a timer");
        assert_eq!(t.mode(), TimerMode::Repeating);
        assert_eq!(t.duration(), d);
    }

    #[test]
    fn cadence_timer_on_event_yields_none() {
        assert!(cadence_timer(&Cadence::OnEvent).is_none());
    }

    #[test]
    fn cadence_timer_once_builds_zero_duration_one_shot() {
        let t = cadence_timer(&Cadence::Once).expect("Once should build a timer");
        assert_eq!(t.mode(), TimerMode::Once);
        assert_eq!(t.duration(), Duration::ZERO);
    }

    // ── Coexistence: both parameterisations in one App ────────────────────

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    /// Sim and Lobby broadcasters must coexist in one `App` with fully
    /// independent registries: a registration on one must never fire through
    /// the other's dispatch (which would show up as a duplicate message or a
    /// message with the wrong `DeliveryClass`).
    #[test]
    fn sim_and_lobby_registries_coexist_independently() {
        let sim = SimBroadcaster::new().register(Audience::All, Cadence::Once, |_: &mut World| {
            vec![ServerMessage::GameStarted]
        });
        let lobby =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_: &mut World| {
                vec![ServerMessage::PlayerLeft { token: "t1".into() }]
            });

        let mut app = App::new();
        app.add_plugins(LobbyPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(sim);
        app.add_plugins(lobby);

        // Lobby phase so the lobby gate passes (sim dispatch is internally
        // ungated in this harness — its gate lives on the SimSet chain).
        app.world_mut()
            .insert_resource(State::new(GamePhase::Lobby));
        {
            let mut sm = app.world_mut().resource_mut::<Sessions>();
            sm.0.register("alice".to_string(), "Alice".to_string())
                .unwrap();
        }
        app.init_resource::<Outbox>();
        app.add_systems(PostUpdate, collect);

        // Distinct Resource identities, one registration each.
        assert_eq!(
            app.world()
                .resource::<BroadcastRegistry<Sim>>()
                .registrations
                .len(),
            1
        );
        assert_eq!(
            app.world()
                .resource::<BroadcastRegistry<Lobby>>()
                .registrations
                .len(),
            1
        );

        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();

        // Sim's registration fired exactly once, stamped Snapshot.
        let game_started: Vec<_> = msgs
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .collect();
        assert_eq!(
            game_started.len(),
            1,
            "sim registration must fire exactly once (no cross-dispatch)"
        );
        assert_eq!(game_started[0].delivery, DeliveryClass::Snapshot);

        // Lobby's registration fired exactly once, stamped Reliable.
        let player_left: Vec<_> = msgs
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::PlayerLeft { .. }))
            .collect();
        assert_eq!(
            player_left.len(),
            1,
            "lobby registration must fire exactly once (no cross-dispatch)"
        );
        assert_eq!(player_left[0].delivery, DeliveryClass::Reliable);
    }
}
