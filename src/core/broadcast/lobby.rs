use bevy::prelude::*;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::lobby::{OutboundMessage, Sessions};
use crate::messages::{DeliveryClass, GamePhase, ServerMessage};

// ── Registration types ─────────────────────────────────────────────────────

/// A boxed producer function for lobby broadcasts.
///
/// The producer receives exclusive `&mut World` access so it can drain mutable
/// resources (e.g. the lobby outbox).  Returning an empty `Vec` skips the
/// broadcast for this tick.
pub type LobbyProducer = Arc<dyn Fn(&mut World) -> Vec<ServerMessage> + Send + Sync>;

/// A single registered broadcast entry for the lobby phase.
pub struct LobbyRegistration {
    pub audience: Audience,
    pub cadence: Cadence,
    pub producer: LobbyProducer,
}

// ── Resource: live registry kept during lobby ──────────────────────────────

#[derive(Resource)]
pub struct LobbyBroadcastRegistry {
    pub registrations: Vec<LobbyRegistration>,
    /// Per-registration cadence timers (index-matched to `registrations`).
    pub timers: Vec<Option<Timer>>,
}

impl LobbyBroadcastRegistry {
    fn new() -> Self {
        Self {
            registrations: Vec::new(),
            timers: Vec::new(),
        }
    }

    fn add(&mut self, reg: LobbyRegistration) {
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
        Cadence::OnEvent => None,
        Cadence::Once => Some(Timer::from_seconds(0.0, TimerMode::Once)),
    }
}

// ── Plugin builder ─────────────────────────────────────────────────────────

/// Bevy plugin that broadcasts `ServerMessage`s during the `Lobby` phase.
///
/// Use [`LobbyBroadcaster::register`] before adding the plugin to `App` to
/// enqueue producers. Each producer is called at the requested cadence and its
/// output is routed to the `Target` resolved from the `Audience`.
pub struct LobbyBroadcaster {
    pending: Vec<LobbyRegistration>,
}

impl Default for LobbyBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl LobbyBroadcaster {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Register a producer that fires at `cadence` to `audience` during the
    /// lobby phase.
    ///
    /// The producer receives exclusive `&mut World` access so it can drain
    /// mutable resources (e.g. the lobby outbox).
    pub fn register<F>(mut self, audience: Audience, cadence: Cadence, producer: F) -> Self
    where
        F: Fn(&mut World) -> Vec<ServerMessage> + Send + Sync + 'static,
    {
        self.pending.push(LobbyRegistration {
            audience,
            cadence,
            producer: Arc::new(producer),
        });
        self
    }
}

impl Plugin for LobbyBroadcaster {
    fn is_unique(&self) -> bool {
        false
    }

    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<LobbyBroadcastRegistry>() {
            app.insert_resource(LobbyBroadcastRegistry::new());
            app.add_systems(
                Update,
                dispatch_lobby_broadcasts.after(crate::lobby::LobbySystemSet),
            );
        }
        let mut registry = app.world_mut().resource_mut::<LobbyBroadcastRegistry>();
        for reg in &self.pending {
            registry.add(LobbyRegistration {
                audience: reg.audience.clone(),
                cadence: reg.cadence.clone(),
                producer: reg.producer.clone(),
            });
        }
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
/// Only active during the `Lobby` phase.
fn dispatch_lobby_broadcasts(world: &mut World) {
    let phase = match world.get_resource::<State<GamePhase>>() {
        Some(s) => s.get().clone(),
        None => return,
    };
    if phase != GamePhase::Lobby {
        return;
    }

    // Tick all cadence timers.
    let dt = world.resource::<Time>().delta();
    {
        let mut registry = world.resource_mut::<LobbyBroadcastRegistry>();
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

    // Collect (target, producer) for entries ready to fire this tick.
    let ready: Vec<(crate::lobby_handler::Target, LobbyProducer)> = {
        let registry = world.resource::<LobbyBroadcastRegistry>();
        let sessions = world.resource::<Sessions>();
        registry
            .registrations
            .iter()
            .enumerate()
            .filter_map(|(i, reg)| {
                let should_fire = match &registry.timers[i] {
                    Some(t) => t.just_finished(),
                    None => true,
                };
                if !should_fire {
                    return None;
                }
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
                delivery: DeliveryClass::Reliable,
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
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::{GamePhase, ServerMessage};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn dispatch_app(broadcaster: LobbyBroadcaster) -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(broadcaster);

        app.world_mut()
            .insert_resource(State::new(GamePhase::Lobby));
        app.init_resource::<Outbox>();
        app.add_systems(PostUpdate, collect);
        app
    }

    fn tick_and_collect(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        msgs
    }

    #[test]
    fn once_cadence_delivers_message_on_first_tick() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        let msgs = tick_and_collect(&mut app);
        assert!(
            msgs.iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "Cadence::Once should deliver message on first tick"
        );
    }

    #[test]
    fn once_cadence_does_not_fire_again() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        let _ = tick_and_collect(&mut app);
        app.world_mut().resource_mut::<Outbox>().0.clear();
        let msgs = tick_and_collect(&mut app);
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "Cadence::Once should not fire on second tick"
        );
    }

    #[test]
    fn on_event_producer_drains_per_frame() {
        use std::sync::{Arc, Mutex};
        let queue: Arc<Mutex<Vec<ServerMessage>>> = Arc::new(Mutex::new(vec![]));
        let q2 = queue.clone();

        let broadcaster = LobbyBroadcaster::new().register(
            Audience::All,
            Cadence::OnEvent,
            move |_world: &mut World| {
                let mut q = q2.lock().unwrap();
                std::mem::take(&mut *q)
            },
        );

        let mut app = dispatch_app(broadcaster);

        queue.lock().unwrap().push(ServerMessage::GameStarted);
        queue.lock().unwrap().push(ServerMessage::GameStarted);

        let msgs1 = tick_and_collect(&mut app);
        let count1 = msgs1
            .iter()
            .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
            .count();
        assert_eq!(count1, 2, "tick 1: expected 2 messages, got {count1}");

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

    #[test]
    fn multiple_registrations_all_fire() {
        let broadcaster = LobbyBroadcaster::new()
            .register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            })
            .register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::PlayerLeft { token: "t1".into() }]
            });

        let mut app = dispatch_app(broadcaster);
        let msgs = tick_and_collect(&mut app);
        let game_started = msgs
            .iter()
            .any(|m| matches!(m.msg, ServerMessage::GameStarted));
        let player_left = msgs
            .iter()
            .any(|m| matches!(m.msg, ServerMessage::PlayerLeft { .. }));
        assert!(game_started, "first registration should fire");
        assert!(player_left, "second registration should fire");
    }

    #[test]
    fn dispatch_skips_when_not_in_lobby_phase() {
        let broadcaster =
            LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_world: &mut World| {
                vec![ServerMessage::GameStarted]
            });
        let mut app = dispatch_app(broadcaster);
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        let msgs = tick_and_collect(&mut app);
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m.msg, ServerMessage::GameStarted)),
            "broadcasts should not fire outside lobby phase"
        );
    }
}
