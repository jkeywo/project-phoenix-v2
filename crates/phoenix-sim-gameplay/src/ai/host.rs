use crate::ai::policy::{AiPolicy, AiPolicyVerb};
use crate::core::messages::SystemId;
use crate::ship::control_source::ControlSourceResolver;
use crate::world::flags::{AiFacts, AiPolicyMemory, FlagStore};
use bevy::prelude::*;
/// The verdict [`decide`] reaches for one fine system on one channel this tick.
///
/// The three non-acting variants are kept distinct rather than folded into a
/// single "no output" because a host (and its tests) care WHY nothing was
/// emitted: a human holds the console ([`NotAiOperated`](HostOutcome::NotAiOperated)),
/// the system is AI-run but authors no policy
/// ([`Undeclared`](HostOutcome::Undeclared)), or it authors one that chose not
/// to act this tick ([`Held`](HostOutcome::Held)). Only the last is a normal
/// steady-state; the first two are structural facts about the ship.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostOutcome<'a> {
    /// The fine system's Control Source is not AI — a human holds it, or damage
    /// / a station rating has driven it offline. The host stands down and emits
    /// nothing; there is no human-versus-AI branch past this gate (AGENTS.md #6).
    NotAiOperated,
    /// AI-operated, but the fine system declares no [`AiPolicy`]. Under strict
    /// AI-declaration mode (issue #885) an undeclared system takes no action at
    /// all — behaviour is never invented for it — so the host emits nothing.
    Undeclared,
    /// AI-operated and declared, but no rule fired on the resolved channel this
    /// tick (or the policy is an explicit idle). The actuator holds its last
    /// input — distinct from emitting a zeroing command.
    Held,
    /// AI-operated, declared, and a rule won the channel: apply this verb. The
    /// borrow is of the policy handed to [`decide`], so the caller reads the
    /// winning verb without cloning.
    Act(&'a AiPolicyVerb),
}

/// The read-only stateful-resolution context for one tick (issue #882).
///
/// Present in a [`HostTick`] only for a stateful policy: `current` is the state
/// id the host is holding this tick and `memory` is that system's private
/// memory bag (with `state_time` already filled in by the host). Absent, the
/// tick resolves the stateless path.
#[derive(Clone, Copy, Debug)]
pub struct HostState<'a> {
    /// The currently-entered state id, committed before any output resolves.
    pub current: &'a str,
    /// This fine system's private memory bag for the tick.
    pub memory: &'a AiPolicyMemory,
}

/// The immutable per-tick inputs [`decide`] resolves a policy against.
///
/// Hand-buildable with no `App`: a test constructs the [`SystemId`](crate::core::messages::SystemId)
/// it wants gated, the channel it wants resolved, a seeded [`AiFacts`], a flag
/// chain (often empty), and — for a stateful policy — a [`HostState`]. The host
/// builds the same value from live components each tick.
#[derive(Clone, Debug)]
pub struct HostTick<'a> {
    /// The fine system being operated. Gates the Control Source: [`decide`]
    /// returns [`HostOutcome::NotAiOperated`] unless
    /// `sources.policy_for(&system).operate_ai`.
    pub system: crate::core::messages::SystemId,
    /// The output channel to resolve on the policy this tick (e.g. `"red_alert"`,
    /// `"yaw"`, a power-group id).
    pub channel: &'a str,
    /// The immutable typed fact snapshot the host seeded for this tick. Guards
    /// that reference an unseeded fact read it absent (the #779 empty-facts
    /// lesson): a host that fails to seed a fact simply never fires its guard.
    pub facts: &'a AiFacts,
    /// The read-only scenario flag/counter chain, anchored at the layer that
    /// spawned the ship (see [`AiWorldView::flag_chain`]). Empty for a ship in a
    /// world with no flags — every flag then reads false/0.
    pub flags: &'a [&'a FlagStore],
    /// Optional stateful-resolution context. `None` runs the stateless path
    /// ([`AiPolicy::resolve_channel`]); `Some` runs the per-state path
    /// ([`AiPolicy::resolve_channel_in_state`]) for the named current state.
    pub state: Option<HostState<'a>>,
}

/// Resolve one fine system's AI verdict for one channel this tick — the pure,
/// Bevy-free spine.
///
/// The three gates run in order and each short-circuits:
///
/// 1. **Control Source.** [`ai_operates`] must
///    hold, or the outcome is [`HostOutcome::NotAiOperated`]. This is the one
///    place a human (or an offline system) suppresses the AI, and it reads the
///    same per-system resolver a human command is admitted against.
/// 2. **Declaration.** `policy` must be `Some`, or the outcome is
///    [`HostOutcome::Undeclared`] (strict AI-declaration, issue #885).
/// 3. **Resolution.** The channel is resolved — stateless when `tick.state` is
///    `None`, per-state otherwise — through the frozen `ai::policy` evaluator.
///    A fired rule yields [`HostOutcome::Act`]; no rule (or an idle policy)
///    yields [`HostOutcome::Held`].
///
/// The returned [`HostOutcome::Act`] borrows the winning verb from `policy`, so
/// the outcome's lifetime is tied to the policy's, not the tick's.
pub fn decide<'p>(
    sources: &ControlSourceResolver,
    policy: Option<&'p AiPolicy>,
    tick: &HostTick<'_>,
) -> HostOutcome<'p> {
    // Gate 1 — the Control Source must be AI. A human holder or a damage/rating
    // offline both resolve `operate_ai == false` here (see `control_tick_policy`).
    if !ai_operates(sources, &tick.system) {
        return HostOutcome::NotAiOperated;
    }

    // Gate 2 — the fine system must declare a policy. No synthesised stand-in
    // since #885b stage 5d: an undeclared AI-operated system does nothing.
    let Some(policy) = policy else {
        return HostOutcome::Undeclared;
    };

    // Gate 3 — resolve the channel. Both arms call the same frozen evaluator; a
    // `None` verb ("hold") is the ordinary steady-state, not an error.
    let verb = match tick.state {
        None => policy.resolve_channel(tick.channel, tick.facts, tick.flags),
        Some(state) => policy.resolve_channel_in_state(
            state.current,
            tick.channel,
            tick.facts,
            state.memory,
            tick.flags,
        ),
    };

    match verb {
        Some(verb) => HostOutcome::Act(verb),
        None => HostOutcome::Held,
    }
}

/// The Control Source gate shared by [`decide`] and selector/ranked hosts.
///
/// A host whose RESOLUTION the spine does not model — a **selector** (Sensors,
/// Navigation, Repair, the Comms hail selector, Tactical target selection) or a
/// **ranked** channel (Power allocation) — still shares exactly one step with
/// the policy hosts: the gate on its fine system's Control Source being AI.
/// The resolver applies the same damage, GM-disable and destroyed-hull overrides
/// for every caller, without needing policy-resolution inputs.
pub fn ai_operates(sources: &ControlSourceResolver, system: &SystemId) -> bool {
    sources.policy_for(system).operate_ai
}

/// Borrowed live mission facts for one operator invocation. Implemented by composition.
/// No copied resource is refreshed between systems, so prior same-tick writes are visible.
pub trait AiWorldView {
    fn flag_chain(&self, ship: Entity) -> Vec<&FlagStore>;
    fn flag_chain_from(
        &self,
        origin: Option<&phoenix_sim_contracts::identity::EntityOriginLayer>,
    ) -> Vec<&FlagStore>;
    fn names(&self) -> &std::collections::HashMap<String, String>;
    fn emitter(&self) -> AiEmitter {
        AiEmitter
    }
}
pub struct AiEmitter;
impl AiEmitter {
    pub fn emit(
        &self,
        entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
        target: SystemId,
        payload: crate::core::messages::SystemControlPayload,
        sources: &crate::ship_plugin::ShipSystemControlSources,
        ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
        admitted: &mut crate::core::messages::AdmittedCommands,
    ) -> bool {
        crate::command_admission::ai_emit::emit_ai_command(
            entity_uuid,
            target,
            payload,
            sources,
            ship_config,
            admitted,
        )
    }
}
