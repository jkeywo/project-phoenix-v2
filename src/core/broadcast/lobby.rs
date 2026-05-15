use bevy::prelude::*;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::lobby::{CurrentPhase, OutboundMessage, Sessions};
use crate::messages::{GamePhase, ServerMessage};

// ── Registration types ─────────────────────────────────────────────────────

/// A boxed producer function for lobby broadcasts.
pub type LobbyProducer = Arc<dyn Fn(&World) -> Option<ServerMessage> + Send + Sync>;

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
    pub fn register<F>(mut self, audience: Audience, cadence: Cadence, producer: F) -> Self
    where
        F: Fn(&World) -> Option<ServerMessage> + Send + Sync + 'static,
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
    fn build(&self, app: &mut App) {
        let mut registry = LobbyBroadcastRegistry::new();
        for reg in &self.pending {
            registry.add(LobbyRegistration {
                audience: reg.audience.clone(),
                cadence: reg.cadence.clone(),
                producer: reg.producer.clone(),
            });
        }
        app.insert_resource(registry)
            .add_systems(Update, dispatch_lobby_broadcasts);
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
/// Skips the whole system when the game is not in the `Lobby` phase.
fn dispatch_lobby_broadcasts(world: &mut World) {
    // Gate: only active during lobby.
    let phase = world.resource::<CurrentPhase>().0.clone();
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
                let target = reg.audience.resolve(&sessions.0)?;
                Some((target, reg.producer.clone()))
            })
            .collect()
    };

    // Call producers and write resulting messages.
    for (target, producer) in ready {
        if let Some(msg) = producer(world) {
            world.write_message(OutboundMessage { target, msg });
        }
    }
}
