//! Admitted commands and the synchronous inter-System command channel.
pub use phoenix_model::messages::*;

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedCommand {
    pub target: SystemId,
    pub payload: SystemControlPayload,
    /// Opaque host-local target used to address a reply to the origin route.
    /// Handlers must not branch on this for any behavioral decision.
    pub response_token: Option<String>,
    /// Transient correlated-feedback identity.  It survives command delay so
    /// the owning consumer can acknowledge actual consumption, but is never
    /// projected into `LoggedCommand`, mesh traffic, snapshots or replay.
    pub feedback_correlation: Option<ActionCorrelationId>,
}

/// Cleared and refilled each tick by `admit_system_commands` (runs before
/// `SimSet::Input`). Handlers read from this instead of `InboundMessage`.
///
/// Pure per-ship Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<AdmittedCommands>`.
#[derive(bevy::prelude::Component, Default)]
pub struct AdmittedCommands(pub Vec<AdmittedCommand>);

impl AdmittedCommands {
    /// Iterate admitted commands targeting the given system ID string.
    pub fn for_target<'a>(&'a self, target: &'a str) -> impl Iterator<Item = &'a AdmittedCommand> {
        self.0.iter().filter(move |c| c.target.0.as_str() == target)
    }
}

/// Payloads that one system may send to another within the same Simulate tick.
///
/// Inter-system commands originate inside Simulate and are applied immediately
/// (same-tick) by the target system. They are invariant-gated: valid by
/// construction, not by control-state check. The sender mutates only its own
/// state; the target mutates only its own.
#[derive(Clone, Debug)]
pub enum InterSystemPayload {
    /// Joystick input published by the Helm Joystick fine system (issue #511)
    /// for consumption by each Helm Engine fine system. Channels thrust and
    /// steering so each engine can independently gate on its own online state.
    JoystickState { thrust: f32, steering: f32 },
    /// A torpedo tube is requesting a round from the shared magazine (issue #512).
    ///
    /// Sent by the tube's `handle_load_tube` handler during `SimSet::Input`
    /// and consumed by the magazine handler `handle_torpedo_magazine_inter_system`
    /// during `SimSet::Physics` on the same tick. The magazine consumer:
    ///
    /// 1. Refuses the claim (no-op) if the magazine is offline (Disabled /
    ///    Destroyed hull tier), leaving the tube unloaded.
    /// 2. Refuses the claim if the magazine's `torpedoes_remaining == 0`.
    /// 3. Otherwise decrements the magazine counter and begins loading the
    ///    named tube (via `TorpedoSystem::start_load_reserved`).
    ///
    /// The `tube` field carries the tube's TOML `id` (e.g. `"fore_port"`).
    ClaimTorpedoRound { tube: TorpedoTube },
}

/// An inter-system command: one system commanding another to mutate its own
/// state this tick. See [`InterSystemPayload`] for invariants.
///
/// `source_entity` identifies which ship the message applies to so
/// per-entity handlers can route the mutation to the correct ship's
/// per-entity state. `None` means "target
/// the LocalShip" — used by legacy paths and tests that never spawned a
/// specific ship.
#[derive(Clone, Debug)]
pub struct InterSystemMsg {
    pub target: SystemId,
    pub payload: InterSystemPayload,
    pub source_entity: Option<bevy::prelude::Entity>,
}

/// Cleared at the start of each Simulate phase (before `SimSet::Input`) and
/// filled during Simulate by systems that need to mutate a peer system's state.
/// Handlers read from this without authority checks — valid by construction.
#[derive(bevy::prelude::Resource, Default)]
pub struct InterSystemQueue(pub Vec<InterSystemMsg>);

impl InterSystemQueue {
    /// Iterate messages targeting the given system ID string.
    pub fn for_target<'a>(&'a self, target: &'a str) -> impl Iterator<Item = &'a InterSystemMsg> {
        self.0.iter().filter(move |m| m.target.0.as_str() == target)
    }
}
