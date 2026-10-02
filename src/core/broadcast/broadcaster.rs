use bevy::prelude::*;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::core::broadcast::audience::Audience;
use crate::core::broadcast::cadence::Cadence;
use crate::core::messages::{DeliveryClass, ServerMessage};
use crate::lobby::{OutboundMessage, Sessions};

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
    /// Explicit production ranks, index-matched to entries and timers. Generic
    /// registrations have no rank and retain insertion order after ranked ones.
    orders: Vec<Option<usize>>,
    _marker: PhantomData<M>,
}

impl<M: BroadcastKind> BroadcastRegistry<M> {
    fn new() -> Self {
        Self {
            registrations: Vec::new(),
            timers: Vec::new(),
            orders: Vec::new(),
            _marker: PhantomData,
        }
    }

    fn add(&mut self, reg: Registration, order: Option<usize>) {
        let timer = cadence_timer(&reg.cadence);
        let at = match order {
            Some(rank) => {
                assert!(
                    !self.orders.contains(&Some(rank)),
                    "a production broadcast owner must register exactly one producer"
                );
                self.orders
                    .iter()
                    .position(|other| other.is_none_or(|other| other > rank))
                    .unwrap_or(self.orders.len())
            }
            None => self.orders.len(),
        };
        // Inserting all three together keeps every timer attached to its owner,
        // including when a later plugin inserts before an existing producer.
        self.registrations.insert(at, reg);
        self.timers.insert(at, timer);
        self.orders.insert(at, order);
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
    order: Option<usize>,
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
            order: None,
            _marker: PhantomData,
        }
    }

    /// Internal mechanism for a phase's typed production-owner constructor.
    /// Generic callers keep `new().register(..)` and insertion order.
    pub(super) fn for_order(order: usize) -> Self {
        Self {
            order: Some(order),
            ..Self::new()
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
        // Authoritative-state exclusion declaration (issue #1221, Track 3 step C9).
        // The per-phase broadcast registry is a CACHE — the broadcaster's own live
        // delivery bookkeeping, never a second copy of simulation truth. Declared
        // per-instantiation at this owning site (each `BroadcastRegistry<M>` keys
        // distinctly by full path yet all share the short name `BroadcastRegistry`
        // the guard consults), replacing the `EXCLUSIONS` const in
        // `tests/authoritative_state_enumeration.rs`; inert to the digest.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<BroadcastRegistry<M>>(
                StateClass::Cache,
                "digest-exclusion-classes",
            );
        }
        let mut registry = app.world_mut().resource_mut::<BroadcastRegistry<M>>();
        for reg in &self.pending {
            registry.add(
                Registration {
                    audience: reg.audience.clone(),
                    cadence: reg.cadence.clone(),
                    producer: reg.producer.clone(),
                },
                self.order,
            );
        }
    }
}

// ── Dispatch system (exclusive: needs &mut World for write_message) ────────

/// Each frame, tick cadence timers and call producers that are ready.
///
/// Gating is per-phase: `M::phase_allows` may short-circuit inline, and/or the
/// schedule position chosen by `M::add_dispatch` may carry an external
/// `run_if` (e.g. the SimSet chain's `in_state(GamePhase::InProgress)`).
pub fn dispatch<M: BroadcastKind>(
    world: &mut World,
    mut config_query: Local<
        Option<
            QueryState<
                &'static crate::ship_plugin::ShipConfigComponent,
                With<crate::server_app::LocalShip>,
            >,
        >,
    >,
) {
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

    // Deliberately *not* gated on "is any registration ready": every phase in
    // this codebase registers at least one `Cadence::OnEvent` producer (see
    // `server_app::sim_*_broadcaster`), whose timer is `None` and which is
    // therefore always ready. A pre-check would scan the registry every tick and
    // never once short-circuit — measured as a net loss.
    //
    // Collect ship config before borrowing registry/sessions to avoid borrow
    // conflicts. The `QueryState` is cached in a `Local` rather than rebuilt per
    // tick by `World::query_filtered` — building one walks every archetype in
    // the world, which this system was paying for on every single tick.
    let ship_config_opt: Option<crate::ship_plugin::ShipConfigComponent> = {
        let q = config_query.get_or_insert_with(|| {
            world.query_filtered::<&crate::ship_plugin::ShipConfigComponent, With<crate::server_app::LocalShip>>()
        });
        q.single(world).ok().cloned()
    };

    // Collect (target, producer) for entries that should fire this tick.
    // We clone Arcs so we can release the borrow on `registry` before calling
    // into the world (producers need exclusive world access).
    let ready: Vec<(crate::lobby::handler::Target, Producer)> = {
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
#[path = "broadcaster_tests.rs"]
mod tests;
