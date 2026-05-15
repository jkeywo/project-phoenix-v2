use bevy::prelude::*;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::lobby::{CurrentPhase, OutboundMessage, Sessions};
use crate::messages::{GamePhase, ServerMessage};

// ── Registration types ─────────────────────────────────────────────────────

/// A boxed producer function: given read-only world access, yields a
/// `ServerMessage` if it has something to send this tick, or `None` to skip.
pub type Producer = Arc<dyn Fn(&World) -> Option<ServerMessage> + Send + Sync>;

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
        // `OnEvent` producers are called every frame and choose whether to
        // emit via the `Option` return of the producer closure.
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
    pub fn register<F>(mut self, audience: Audience, cadence: Cadence, producer: F) -> Self
    where
        F: Fn(&World) -> Option<ServerMessage> + Send + Sync + 'static,
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
    fn build(&self, app: &mut App) {
        let mut registry = SimBroadcastRegistry::new();
        for reg in &self.pending {
            registry.add(SimRegistration {
                audience: reg.audience.clone(),
                cadence: reg.cadence.clone(),
                producer: reg.producer.clone(),
            });
        }
        app.insert_resource(registry)
            .add_systems(Update, dispatch_sim_broadcasts);
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
/// Skips the whole system when the game is not in `InProgress`.
fn dispatch_sim_broadcasts(world: &mut World) {
    // Gate: only active during simulation.
    let phase = world.resource::<CurrentPhase>().0.clone();
    if phase != GamePhase::InProgress {
        return;
    }

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

    // Collect (index, target, producer) for entries that should fire this tick.
    // We clone Arcs so we can release the borrow on `registry` before calling
    // into the world (producers may need immutable world access).
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
