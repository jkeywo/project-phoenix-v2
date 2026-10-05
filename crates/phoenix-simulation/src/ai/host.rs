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
//! This module owns the shared spine. Selector and ranked-channel hosts also
//! use its Control Source gate through [`ai_operates`].
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

// This adapter borrows live mission context for the Gameplay decision spine.
// Command authorization itself is shared through the simulation contracts.

use crate::world::flags::FlagStore;

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
    /// Preserve the original schedule read of session state. AI admission uses
    /// the shared AI claimant policy and needs no crew lookup.
    _sessions: Res<'w, crate::lobby::Sessions>,
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

    /// The same-tick admission emitter for an AI claimant.
    ///
    /// Each per-ship emit still supplies the ship-specific context (its uuid,
    /// control sources, config and `AdmittedCommands`); the env supplies the one
    /// policy shared with human command admission.
    pub fn emitter(&self) -> AiEmitter {
        AiEmitter
    }
}

/// Register every resource [`AiHostEnv`] borrows as a bare [`Res`] — the single
/// wiring point for the spine.
///
/// Called by both the AI host plugin and `crates/phoenix-simulation/src/ship/test_support.rs`, so a test
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
#[path = "host_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::ai::host::{
    ai_operates, decide, AiEmitter, HostOutcome, HostState, HostTick,
};
impl phoenix_sim_gameplay::ai::host::AiWorldView for AiHostEnv<'_, '_> {
    fn flag_chain(&self, ship: Entity) -> Vec<&FlagStore> {
        AiHostEnv::flag_chain(self, ship)
    }
    fn flag_chain_from(
        &self,
        origin: Option<&crate::world::server::EntityOriginLayer>,
    ) -> Vec<&FlagStore> {
        AiHostEnv::flag_chain_from(self, origin)
    }
    fn names(&self) -> &std::collections::HashMap<String, String> {
        &self.runtime.name_to_uuid
    }
}
