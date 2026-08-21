//! The authoritative-state declaration registry (issue #1220, Track 3 step C8).
//!
//! # What this is, and what it is not
//!
//! `tests/authoritative_state_enumeration.rs` (issue #894) already proves that
//! every crate-local component/resource the sim app registers is *accounted
//! for* — folded, excluded with a reason, or on the honest unclassified
//! baseline. It does that by reading Bevy's own component registry and checking
//! it against three hand-maintained `const` lists. Those lists are transcribed
//! from `pasm/spec/architecture/*.yaml`, which means the authoritative record
//! lives in PASM and the *code* only asserts against a copy of it.
//!
//! This module is the other direction: a place for an owning plugin to state,
//! **in Rust, at the site that owns the type**, "this type is authoritative and
//! folded / deferred / presentation / …, and here is the PASM `state` entity id
//! that records it". A plugin calls [`App::declare_state`] in its `build()`, the
//! same way it calls `app.register_admitted_consumer(..)` (see
//! `command_admission::router`) — the declaration lands in the [`StateCensus`]
//! resource, keyed by the type's **full path**.
//!
//! # This issue declares NOTHING
//!
//! Per its acceptance criteria, #1220 adds only the *mechanism*. No production
//! plugin calls [`App::declare_state`] yet, so in a real headless run
//! [`StateCensus`] is never even initialised (its `init_resource` is on-first-
//! use), never registered, and therefore invisible to the enumeration guard
//! that scans the registry. The census test still reads its existing `const`
//! lists; this registry does not feed it. Wiring declarations in, and then
//! deriving those lists from the census rather than from a transcription, is
//! later Track-3 work.
//!
//! # Determinism: the registry is inert to the digest
//!
//! [`StateCensus`] is a diagnostic/coverage surface. Nothing in
//! `src/sim_digest.rs` (`world_digest`) or `src/snapshot.rs` reads it, so the
//! order plugins declare their state in — and whether they declare it at all —
//! cannot move a single byte of the authoritative-state digest. The map is a
//! `BTreeMap`, so its *contents* are a pure function of the SET of declarations
//! regardless of insertion order; `tests/authoritative_state_enumeration.rs`'s
//! `permuting_declaration_order_leaves_the_digest_identical` proves the digest
//! consequence directly, mirroring `tests/registration_order_determinism.rs`.

use bevy::prelude::*;
use std::collections::BTreeMap;

/// How a declared authoritative-state type relates to the #894 digest boundary.
///
/// The four exclusion classes (`Presentation` / `Cache` / `Timer` / `Derived`)
/// and `ClearedAtFold` mirror the reason vocabulary
/// `pasm/spec/architecture/deterministic-simulation.yaml`'s
/// `digest-exclusion-classes` entity records and that
/// `tests/authoritative_state_enumeration.rs`'s `EXCLUSIONS` list already uses;
/// `TestInfra` is the fifth (state a test harness registers). `Folded` and
/// `DeferredFold` are the two authoritative shapes the fold record
/// distinguishes: state walked by `world_digest` every tick, versus
/// authoritative state captured in the snapshot but deliberately deferred out
/// of the per-tick fold (e.g. `WorldContentRuntime`'s pending queues).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateClass {
    /// Authoritative simulation state folded into the per-tick digest
    /// (`src/sim_digest.rs::world_digest`).
    Folded,
    /// Authoritative state captured in the snapshot (`src/snapshot.rs`) but
    /// deliberately deferred out of the per-tick fold.
    DeferredFold,
    /// Presentation-only: it decides how something is DRAWN, and nothing reads
    /// it to decide what the fixed tick computes.
    Presentation,
    /// A one-directional delta-suppression mirror of already-authoritative
    /// state — never a second copy of simulation truth.
    Cache,
    /// A timer or outbox: wall-clock / transport bookkeeping, not simulation
    /// state.
    Timer,
    /// Recomputed every tick as a pure function of digest-free inputs, so
    /// folding it would fold its inputs a second time.
    Derived,
    /// Structurally empty by the `RenderInterp` fold point on every
    /// correctly-running instance (e.g. an inter-system queue drained each
    /// tick).
    ClearedAtFold,
    /// Registered only by a test harness or dev tool, never by the sim itself.
    TestInfra,
}

/// The declaration registry: every type an owning plugin has declared via
/// [`App::declare_state`], keyed by its **full type path**
/// (`std::any::type_name::<T>()`), mapping to its [`StateClass`] and the PASM
/// `state` entity id (under `pasm/spec/architecture/`) that records it.
///
/// # Why the full path, not a short name
///
/// The key is the full path precisely so two distinct generic instantiations —
/// the canonical `EffectQueue<A>` / `EffectQueue<B>` case — are distinct keys
/// rather than collapsing at the first `<`, the exact truncation
/// `tests/authoritative_state_enumeration.rs`'s old `short_name` census key
/// suffered and this issue also fixes there.
///
/// # Idempotent by construction
///
/// Declaring the same type twice with the same classification is a harmless
/// overwrite (a plugin added twice in a test harness cannot corrupt the map),
/// and because the store is a `BTreeMap` its final contents do not depend on
/// the order declarations arrived in — see the module docs on digest inertness.
#[derive(Resource, Default, Debug, Clone)]
pub struct StateCensus {
    entries: BTreeMap<&'static str, (StateClass, &'static str)>,
}

impl StateCensus {
    /// Record that `type_path` is authoritative-state of class `class`, recorded
    /// by PASM entity `pasm`. Overwrites any prior declaration of the same path
    /// (see the type docs on idempotency). Prefer [`App::declare_state`], which
    /// resolves `type_path` from `T` for you.
    pub fn declare(&mut self, type_path: &'static str, class: StateClass, pasm: &'static str) {
        self.entries.insert(type_path, (class, pasm));
    }

    /// The declaration for `type_path`, if any.
    pub fn get(&self, type_path: &str) -> Option<(StateClass, &'static str)> {
        self.entries.get(type_path).copied()
    }

    /// Every declaration, in full-path order (`BTreeMap` iteration).
    pub fn entries(&self) -> &BTreeMap<&'static str, (StateClass, &'static str)> {
        &self.entries
    }

    /// Number of declared types.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been declared yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One-line declaration API: `app.declare_state::<T>(class, pasm)` in the owning
/// plugin's `build`. Initialises [`StateCensus`] on first use, so no plugin owns
/// the `init_resource` and the order plugins build in does not matter — the same
/// shape `command_admission::router`'s `RegisterAdmittedConsumer` uses.
pub trait DeclareState {
    /// Declare that `T` is authoritative-state of class `class`, recorded by
    /// PASM `state` entity `pasm`. Returns `&mut Self` for chaining.
    fn declare_state<T: 'static>(&mut self, class: StateClass, pasm: &'static str) -> &mut Self;
}

impl DeclareState for App {
    fn declare_state<T: 'static>(&mut self, class: StateClass, pasm: &'static str) -> &mut Self {
        if !self.world().contains_resource::<StateCensus>() {
            self.init_resource::<StateCensus>();
        }
        let type_path = std::any::type_name::<T>();
        self.world_mut()
            .resource_mut::<StateCensus>()
            .declare(type_path, class, pasm);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Alpha;
    struct Beta<T>(#[allow(dead_code)] T);

    #[test]
    fn declare_state_keys_on_the_full_type_path() {
        let mut app = App::new();
        app.declare_state::<Alpha>(StateClass::Folded, "alpha-state");

        let census = app.world().resource::<StateCensus>();
        assert_eq!(census.len(), 1);
        // The key is the FULL path, so it carries the module, not just `Alpha`.
        let (key, (class, pasm)) = census.entries().iter().next().unwrap();
        assert!(
            key.ends_with("::Alpha") && key.contains("authoritative"),
            "expected a full module path ending in ::Alpha, got {key}"
        );
        assert_eq!(*class, StateClass::Folded);
        assert_eq!(*pasm, "alpha-state");
    }

    #[test]
    fn distinct_generic_instantiations_do_not_collapse() {
        let mut app = App::new();
        app.declare_state::<Beta<Alpha>>(StateClass::Cache, "beta-alpha")
            .declare_state::<Beta<u32>>(StateClass::Derived, "beta-u32");

        // Two DISTINCT keys — the whole reason the census keys on the full path
        // rather than a short name truncated at the first `<`.
        let census = app.world().resource::<StateCensus>();
        assert_eq!(census.len(), 2, "generic instantiations collapsed: {census:?}");
    }

    #[test]
    fn redeclaring_the_same_type_is_idempotent() {
        let mut app = App::new();
        app.declare_state::<Alpha>(StateClass::Folded, "alpha-state")
            .declare_state::<Alpha>(StateClass::Folded, "alpha-state");
        assert_eq!(app.world().resource::<StateCensus>().len(), 1);
    }
}
