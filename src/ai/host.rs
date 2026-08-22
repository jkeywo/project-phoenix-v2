//! The single Admission-facing AI host spine (issue #1205).
//!
//! Every fine-system AI operator on the bridge — helm axes, weapon banks,
//! shields, power, captain, comms, navigation, repair, sensors — runs the same
//! four-step spine: **gate** on the Control Source being AI, **check** that the
//! fine system declares a policy, **resolve** the winning verb for a channel
//! against the tick's immutable facts and read-only flags, and **emit** the
//! resulting command through the shared admission seam. Until this module the
//! first three steps were hand-inlined at the top of every host body — twenty-one
//! near-identical `policy_for(sid).operate_ai` / `let Some(policy) else continue`
//! / `resolve_channel` preambles — and the fourth went through one of several
//! byte-identical `emit_*_ai_command` shims.
//!
//! This module is the place all four live once. It is purely additive: no host
//! calls it yet (issue #1205 lands the spine; later slices flip hosts onto it).
//!
//! ## Two halves, one deliberate split
//!
//! [`decide`] is the **pure** half: a Bevy-free function of a
//! [`ControlSourceResolver`], an optional [`AiPolicy`] and a hand-built
//! [`HostTick`], returning a [`HostOutcome`]. It is unit-testable with no `App`,
//! which is the whole point — the gate/declare/resolve logic that used to be
//! smeared across host bodies (and therefore only reachable through a full Bevy
//! fixture) now has a direct test surface.
//!
//! [`AiHostEnv`] is the **Bevy** half: a [`SystemParam`](bevy::ecs::system::SystemParam)
//! bundling the read-only world context every host needs to seed [`decide`]'s
//! inputs — the scenario flag/counter runtime, the loaded sub-world layer map,
//! the session table, and the per-entity origin-layer stamp — behind the
//! [`AiHostEnv::flag_chain`] helper. It holds **bare** [`Res`], not
//! `Option<Res<..>>`, on purpose: a fixture that runs a host through this env
//! must register the same resources production does (via [`register_ai_host_env`])
//! or fail loudly at schedule build, so a fixture cannot silently take a
//! different code path than the shipped app. [`AiEmitter`] wraps the admission
//! seam so the emit half rides the same typed input path a human's command
//! crosses.

use bevy::prelude::*;

// This `use` is the observed PASM admission edge the spine carries. Because
// `src/ai/host.rs` is owned by exactly one PASM entity (`ai-policy-host-spine`),
// PASM attributes this direct edge to `host-command-admission` unambiguously —
// which is the entire reason the emit path is re-homed here rather than left as
// per-host shims in files that other entities co-own.
use crate::command_admission::ai_emit::emit_ai_command;

use crate::ai::policy::{AiPolicy, AiPolicyVerb};
use crate::ship::control_source::ControlSourceResolver;
use crate::world::flags::{AiFacts, AiPolicyMemory, FlagStore};

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
    /// spawned the ship (see [`AiHostEnv::flag_chain`]). Empty for a ship in a
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
/// 1. **Control Source.** `sources.policy_for(&tick.system).operate_ai` must
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
    if !sources.policy_for(&tick.system).operate_ai {
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

/// The Control-Source gate alone, decided through the same [`decide`] spine the
/// policy hosts walk (issue #1208).
///
/// A host whose RESOLUTION the spine does not model — a **selector** (Sensors,
/// Navigation, Repair, the Comms hail selector, Tactical target selection) or a
/// **ranked** channel (Power allocation) — still shares exactly one step with
/// the policy hosts: the gate on its fine system's Control Source being AI. This
/// runs [`decide`]'s gate 1 with a `None` policy (so the declaration and
/// resolution gates are moot and the tick's facts/flags/channel go unread) and
/// returns whether the AI may proceed. Routing it here keeps that gate in the
/// spine for the selector/ranked hosts too, rather than hand-inlined as a
/// twenty-second copy of `policy_for(sid).operate_ai`.
pub fn ai_operates(
    sources: &ControlSourceResolver,
    system: crate::core::messages::SystemId,
) -> bool {
    let facts = AiFacts::new();
    let tick = HostTick {
        system,
        channel: "",
        facts: &facts,
        flags: &[],
        state: None,
    };
    !matches!(decide(sources, None, &tick), HostOutcome::NotAiOperated)
}

/// The read-only world context every AI host reads to seed [`decide`]'s inputs,
/// bundled as one [`SystemParam`](bevy::ecs::system::SystemParam).
///
/// The three resources are **bare** [`Res`], not `Option<Res<..>>`, and that is
/// the deliberate interface choice this module exists to make. A host that takes
/// this env cannot run in a fixture that has not registered the same resources
/// production registers — Bevy's parameter validation panics at schedule build —
/// so [`register_ai_host_env`] is the ONE call that makes the env usable, and a
/// fixture that calls it takes the identical code path the shipped app does. The
/// pre-existing hosts each carry their own `Option<Res<..>>` copies precisely so
/// a bare `App` could skip them silently; consolidating here ends that.
///
/// The env exposes behaviour, not fields: [`flag_chain`](Self::flag_chain)
/// resolves a ship's layered flag store, [`emitter`](Self::emitter) hands back
/// the admission [`AiEmitter`], and [`content_runtime`](Self::content_runtime)
/// serves the raw base-world store for the name-resolution reads that live
/// outside the flag chain. Nothing outside this module borrows the resource
/// handles themselves.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AiHostEnv<'w, 's> {
    /// The base-world scenario flag/counter store and trigger runtime.
    runtime: Res<'w, crate::world::server::WorldContentRuntime>,
    /// The loaded sub-world layer map, walked by `parent:` from a ship's origin.
    layers: Res<'w, crate::world::server::WorldLayerMap>,
    /// The session table — who (if anyone) holds each station — consulted by the
    /// admission seam the [`AiEmitter`] routes through.
    sessions: Res<'w, crate::lobby::Sessions>,
    /// The per-entity origin-layer stamp: an O(1) read of which loaded layer
    /// spawned a ship, anchoring its flag chain.
    origins: Query<'w, 's, &'static crate::world::server::EntityOriginLayer>,
}

impl AiHostEnv<'_, '_> {
    /// The read-only scenario flag chain for `ship`, anchored at the layer that
    /// spawned it and terminating at the base [`WorldContentRuntime`](crate::world::server::WorldContentRuntime)
    /// store.
    ///
    /// `chain[0]` is the origin layer's own store, each outer layer follows, and
    /// the base store terminates it; a `parent:` prefix on a flag name steps one
    /// entry outward. A ship with no origin stamp (base-world entity) resolves to
    /// the base store alone. This is the same walk `entity_flag_chain` runs for
    /// every current host, exposed here once behind the env.
    pub fn flag_chain(&self, ship: Entity) -> Vec<&FlagStore> {
        crate::world::server::entity_flag_chain(
            self.origins.get(ship).ok(),
            Some(&self.runtime),
            Some(&self.layers),
        )
    }

    /// [`flag_chain`](Self::flag_chain) for a host that already holds the ship's
    /// origin stamp from its OWN query, rather than looking it up by entity.
    ///
    /// A handful of hosts read `Option<&EntityOriginLayer>` as a member of their
    /// per-ship query tuple (it rides alongside the other components they iterate)
    /// instead of taking the env's [`origins`](Self::origins) lookup. They pass
    /// that borrow straight through here, so the chain is anchored at exactly the
    /// same layer `flag_chain` would resolve — the env still supplies the runtime
    /// and layer stores the walk terminates against.
    pub fn flag_chain_from(
        &self,
        origin: Option<&crate::world::server::EntityOriginLayer>,
    ) -> Vec<&FlagStore> {
        crate::world::server::entity_flag_chain(origin, Some(&self.runtime), Some(&self.layers))
    }

    /// The base-world content runtime, for the few reads a host makes of it
    /// OUTSIDE the layered flag chain — chiefly the `name_to_uuid` map a target
    /// selector resolves an authored objective/hail target name through.
    ///
    /// This is deliberately the raw store, not another `flag_chain`-style walk:
    /// the entity name table is a single base-world map with no per-layer twin,
    /// so a host resolving a name reads it here directly. Under production the
    /// store is fully loaded; under a bare-`App` fixture that ran
    /// [`register_ai_host_env`] it is the empty default, which every name lookup
    /// misses exactly as an absent `Option<Res<..>>` used to — behaviour the
    /// callers already fall through on.
    pub fn content_runtime(&self) -> &crate::world::server::WorldContentRuntime {
        &self.runtime
    }

    /// The admission emitter bound to this env's session table.
    ///
    /// Each per-ship emit still supplies the ship-specific context (its uuid,
    /// control sources, config and `AdmittedCommands`); the env supplies the one
    /// piece shared across the tick — the sessions the seam authorises against.
    pub fn emitter(&self) -> AiEmitter<'_> {
        AiEmitter {
            sessions: &self.sessions,
        }
    }
}

/// The AI side of the typed input path — a thin wrapper over the shared
/// admission seam ([`emit_ai_command`]).
///
/// A host builds one from its [`AiHostEnv`] and calls [`emit`](Self::emit) per
/// ship. Routing an AI decision through here (rather than writing authoritative
/// state directly) is what keeps AI and human commands symmetric: the same
/// `validate_and_admit` seam re-checks `operate_ai` on the exact target system,
/// so the AI can express nothing a human's console could not send (AGENTS.md #6).
pub struct AiEmitter<'a> {
    sessions: &'a crate::lobby::Sessions,
}

impl AiEmitter<'_> {
    /// Validate-and-enqueue one AI decision into `target`'s owning ship's own
    /// `AdmittedCommands`, through the shared admission seam.
    ///
    /// A pass-through to [`emit_ai_command`] with the session table already
    /// bound: it builds the ship's `ai:<uuid>` (or backfill) token, stands in an
    /// empty `ShipConfig` when the entity carries none, and calls
    /// `validate_and_admit`. Returns whether admission accepted the command.
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &self,
        entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
        target: crate::core::messages::SystemId,
        payload: crate::core::messages::SystemControlPayload,
        sources: &crate::ship_plugin::ShipSystemControlSources,
        ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
        admitted: &mut crate::core::messages::AdmittedCommands,
    ) -> bool {
        emit_ai_command(
            entity_uuid,
            target,
            payload,
            sources,
            self.sessions,
            ship_config,
            admitted,
        )
    }
}

/// Register every resource [`AiHostEnv`] borrows as a bare [`Res`] — the single
/// wiring point for the spine.
///
/// Called by both the AI host plugin and `src/ship/test_support.rs`, so a test
/// fixture and the shipped app reach the env through exactly one code path. Each
/// registration is idempotent: `WorldContentRuntime` and `WorldLayerMap` are
/// [`init_resource`](App::init_resource)'d (both `Default`, and the world plugin
/// already inserts them in production), and `Sessions` — which has no `Default` —
/// is inserted only when the lobby plugin has not already provided it. Calling
/// this in an app that already has all three is therefore a no-op, and calling
/// it in a bare `App` fixture makes the env usable without pulling in the world
/// or lobby plugins wholesale.
pub fn register_ai_host_env(app: &mut App) {
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<crate::world::server::WorldLayerMap>();
    if !app.world().contains_resource::<crate::lobby::Sessions>() {
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::policy::AiPolicyRule;
    use crate::core::messages::SystemId;
    use crate::ship::control_source::ControlSource;
    use crate::world::flags::parse_predicate;

    fn sid() -> SystemId {
        SystemId("red-alert".into())
    }

    /// A one-rule red-alert policy that fires only while `fact(threat) > 0`, so
    /// the same policy resolves `Act` or `Held` depending purely on the facts.
    fn threat_policy() -> AiPolicy {
        AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "red_alert".into(),
                when: parse_predicate("fact(threat) > 0").unwrap(),
                verb: AiPolicyVerb::SetRedAlert(true),
            }],
            idle: false,
            machine: None,
        }
    }

    fn ai_sources() -> ControlSourceResolver {
        let mut s = ControlSourceResolver::new();
        s.set(sid(), ControlSource::Ai);
        s
    }

    fn tick_with<'a>(facts: &'a AiFacts, flags: &'a [&'a FlagStore]) -> HostTick<'a> {
        HostTick {
            system: sid(),
            channel: "red_alert",
            facts,
            flags,
            state: None,
        }
    }

    #[test]
    fn not_ai_operated_when_a_human_holds_the_system() {
        // Default source is Human → operate_ai is false → the AI stands down
        // before the policy is ever consulted (an armed policy proves the gate,
        // not the resolution, produced this).
        let sources = ControlSourceResolver::new();
        let policy = threat_policy();
        let mut facts = AiFacts::new();
        facts.set("threat", 1.0);
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&facts, &[])),
            HostOutcome::NotAiOperated
        );
    }

    #[test]
    fn not_ai_operated_when_the_system_is_offline() {
        // Damage/rating offline overrides even an explicit Ai source.
        let mut sources = ai_sources();
        sources.set_offline(sid(), true);
        let policy = threat_policy();
        let mut facts = AiFacts::new();
        facts.set("threat", 1.0);
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&facts, &[])),
            HostOutcome::NotAiOperated
        );
    }

    #[test]
    fn undeclared_when_ai_operated_but_no_policy_is_authored() {
        // AI holds the system, but there is no policy: strict declaration mode
        // means it does nothing, and the outcome names WHY.
        let sources = ai_sources();
        assert_eq!(
            decide(&sources, None, &tick_with(&AiFacts::new(), &[])),
            HostOutcome::Undeclared
        );
    }

    #[test]
    fn held_when_declared_but_no_rule_fires_this_tick() {
        // AI + declared, but the guard's fact is unseeded so it reads absent and
        // the only rule holds. This is the ordinary steady-state, distinct from
        // both undeclared and not-AI.
        let sources = ai_sources();
        let policy = threat_policy();
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&AiFacts::new(), &[])),
            HostOutcome::Held
        );
    }

    #[test]
    fn act_returns_the_winning_verb_when_a_rule_fires() {
        // AI + declared + the guard's fact crosses its threshold → the winning
        // verb is handed back by borrow.
        let sources = ai_sources();
        let policy = threat_policy();
        let mut facts = AiFacts::new();
        facts.set("threat", 1.0);
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&facts, &[])),
            HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
        );
    }

    #[test]
    fn a_flag_guard_reads_the_supplied_flag_chain() {
        // The flags handed in the tick actually reach the evaluator: a rule
        // gated on a world flag fires only when that flag is set in the chain.
        let sources = ai_sources();
        let policy = AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "red_alert".into(),
                when: parse_predicate("flag(general_quarters)").unwrap(),
                verb: AiPolicyVerb::SetRedAlert(true),
            }],
            idle: false,
            machine: None,
        };
        let facts = AiFacts::new();

        // Absent flag → Held.
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&facts, &[])),
            HostOutcome::Held
        );

        // Set flag in the chain → Act.
        let mut store = FlagStore::new();
        store.set_flag("general_quarters");
        let chain = [&store];
        assert_eq!(
            decide(&sources, Some(&policy), &tick_with(&facts, &chain)),
            HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
        );
    }

    #[test]
    fn a_stateful_tick_resolves_the_named_state_only() {
        // With a HostState the per-state path runs: the `armed` state fires, and
        // a tick naming a state that authors no rule on the channel holds.
        let sources = ai_sources();
        let state = |id: &str, verb: AiPolicyVerb| crate::ai::policy::AiPolicyState {
            id: id.into(),
            yields_to_arc_requests: true,
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "red_alert".into(),
                when: parse_predicate("true").unwrap(),
                verb,
            }],
            transitions: Vec::new(),
        };
        let policy = AiPolicy {
            params: crate::world::flags::AiParams::new(),
            rules: Vec::new(),
            idle: false,
            machine: Some(crate::ai::policy::AiPolicyMachine {
                initial: "calm".into(),
                initial_memory: AiPolicyMemory::new(),
                states: vec![
                    crate::ai::policy::AiPolicyState {
                        id: "calm".into(),
                        yields_to_arc_requests: true,
                        rules: Vec::new(),
                        transitions: Vec::new(),
                    },
                    state("armed", AiPolicyVerb::SetRedAlert(true)),
                ],
            }),
        };
        let facts = AiFacts::new();
        let memory = AiPolicyMemory::new();

        let armed = HostTick {
            system: sid(),
            channel: "red_alert",
            facts: &facts,
            flags: &[],
            state: Some(HostState {
                current: "armed",
                memory: &memory,
            }),
        };
        assert_eq!(
            decide(&sources, Some(&policy), &armed),
            HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
        );

        let calm = HostTick {
            state: Some(HostState {
                current: "calm",
                memory: &memory,
            }),
            ..armed.clone()
        };
        assert_eq!(decide(&sources, Some(&policy), &calm), HostOutcome::Held);
    }

    /// The point of the whole slice (issue #1207): a fixture that runs a host
    /// through [`AiHostEnv`] but forgot [`register_ai_host_env`] fails LOUDLY,
    /// so a bare `App` cannot silently take a different code path than the
    /// shipped app. The env's fields are bare [`Res`], and Bevy validates a
    /// missing required resource by panicking (`Res` is a required param, unlike
    /// `Option<Res<..>>`), so the same system that runs in production panics at
    /// the first schedule run in an unregistered fixture — and runs cleanly once
    /// the one registration call every real app makes has been made.
    #[test]
    fn a_host_reading_the_env_panics_in_a_bare_app_that_skipped_registration() {
        use bevy::prelude::*;

        // Stand-in for every converted host: it consumes `AiHostEnv` and nothing
        // else, so the only resources its validation can trip over are the ones
        // `register_ai_host_env` installs.
        fn reads_the_env(env: AiHostEnv) {
            // Touch a behaviour so the borrow of the bare `Res` fields is real.
            let _ = env.content_runtime();
        }

        let run = |register: bool| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut app = App::new();
                if register {
                    register_ai_host_env(&mut app);
                }
                app.add_systems(Update, reads_the_env);
                app.update();
            }))
        };

        // Swallow the deliberate panic's backtrace so the passing test is quiet.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let unregistered = run(false);
        let registered = run(true);
        std::panic::set_hook(prev);

        assert!(
            unregistered.is_err(),
            "a host consuming AiHostEnv in a bare App that never called \
             register_ai_host_env must panic at schedule run — the guarantee \
             that a fixture cannot diverge from production"
        );
        assert!(
            registered.is_ok(),
            "the identical host runs cleanly once register_ai_host_env has \
             installed the env's resources"
        );
    }
}
