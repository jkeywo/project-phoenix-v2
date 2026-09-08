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
/// distinguishes, and THIS is the one place that distinction is defined — every
/// other site that needs the rule (`tests/authoritative_state_enumeration.rs`,
/// the declaration comments in `server_app::registration`) points back here
/// rather than restating it:
///
/// * **`Folded`** means `sim_digest::world_digest` walks EVERY field of the
///   type — not most of it, not "the fields that currently matter". And
///   `world_digest` does not run every tick: no Bevy schedule registers it, and
///   its only callers are `headless::replay`'s digest sampler, the
///   cross-target probe, the resume tests and `server::bridge`'s save/restore
///   pair (`sim_digest::world_digest`'s own doc comment, "Cheapness, and the
///   empty-walk affordance"), so "folded" is a claim about what a sample/save/
///   restore walks, never about tick frequency. A per-tick exchange is #1118's
///   to introduce.
/// * **`DeferredFold`** means anything LESS than the whole type is walked —
///   anywhere from zero folded fields (e.g. `WorldContentRuntime`'s pending
///   queues: authoritative, and deliberately kept out of the fold) to every
///   field but one. Either way, the `app.declare_state::<T>(StateClass::
///   DeferredFold, ..)` call carries an adjacent comment naming exactly which
///   fields fold and which do not, and why —
///   `comms::server::{CommsInboxRes, CommsRuntime}` and
///   `world::server::WorldLayerMap` in `server_app::registration` are the
///   worked examples. Declaring `Folded` for a type that is only partly walked
///   is the exact explicit-and-wrong claim this registry exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateClass {
    /// Authoritative simulation state with EVERY field folded into the digest
    /// (`src/sim_digest.rs::world_digest`, run per digest sample and on
    /// save/restore — not per tick). See this enum's own doc comment above for
    /// the full rule: a type only partly walked is `DeferredFold`, never this.
    Folded,
    /// Authoritative state captured in the snapshot (`src/snapshot.rs`) with
    /// LESS than every field folded into the digest — anywhere from none of it
    /// to all but one field. The declaring plugin's call site names the
    /// folded/unfolded split; see this enum's own doc comment above.
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
    aliases: BTreeMap<&'static str, &'static str>,
}

impl StateCensus {
    /// Record that `type_path` is authoritative-state of class `class`, recorded
    /// by PASM entity `pasm`. Overwrites any prior declaration of the same path
    /// (see the type docs on idempotency). Prefer [`App::declare_state`], which
    /// resolves `type_path` from `T` for you.
    pub fn declare(&mut self, type_path: &'static str, class: StateClass, pasm: &'static str) {
        assert!(
            !self.aliases.contains_key(type_path),
            "canonical declaration cannot shadow an ownership alias"
        );
        self.entries.insert(type_path, (class, pasm));
    }

    /// The declaration for `type_path`, if any.
    pub fn get(&self, type_path: &str) -> Option<(StateClass, &'static str)> {
        let owner = self.aliases.get(type_path).copied().unwrap_or(type_path);
        self.entries.get(owner).copied()
    }

    /// Declare physical storage access as an alias of an existing canonical
    /// owner. Aliases inherit its classification/PASM and never add fold entries.
    pub fn declare_alias(
        &mut self,
        alias: &'static str,
        owner: &'static str,
    ) -> Result<(), &'static str> {
        if self.entries.contains_key(alias) {
            return Err("alias shadows canonical owner");
        }
        if !self.entries.contains_key(owner) {
            return Err("alias needs an existing canonical owner; chains are forbidden");
        }
        if let Some(existing) = self.aliases.get(alias) {
            return if *existing == owner {
                Ok(())
            } else {
                Err("conflicting alias owner")
            };
        }
        self.aliases.insert(alias, owner);
        Ok(())
    }

    /// Exact full-path physical aliases, separate from canonical fold entries.
    pub fn aliases(&self) -> &BTreeMap<&'static str, &'static str> {
        &self.aliases
    }

    /// Resolve only an explicitly declared physical alias; unknown types are
    /// never inferred from a generic prefix, short name or shared PASM label.
    pub fn alias_owner(&self, alias: &str) -> Option<&'static str> {
        self.aliases.get(alias).copied()
    }

    /// Every canonical declaration, in full-path order (`BTreeMap` iteration).
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
    /// Bind one physical handle to an already declared canonical owner.
    fn declare_state_alias<Alias: 'static, Owner: 'static>(&mut self) -> &mut Self;
}

impl DeclareState for App {
    fn declare_state_alias<Alias: 'static, Owner: 'static>(&mut self) -> &mut Self {
        self.world_mut()
            .resource_mut::<StateCensus>()
            .declare_alias(
                std::any::type_name::<Alias>(),
                std::any::type_name::<Owner>(),
            )
            .expect("invalid physical state owner binding");
        self
    }
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
        assert_eq!(
            census.len(),
            2,
            "generic instantiations collapsed: {census:?}"
        );
    }

    #[test]
    fn redeclaring_the_same_type_is_idempotent() {
        let mut app = App::new();
        app.declare_state::<Alpha>(StateClass::Folded, "alpha-state")
            .declare_state::<Alpha>(StateClass::Folded, "alpha-state");
        assert_eq!(app.world().resource::<StateCensus>().len(), 1);
    }
}

#[cfg(test)]
mod owner_alias_tests {
    use super::*;
    struct Owner;
    struct OtherOwner;
    struct Handle<const I: usize>;

    #[test]
    fn physical_aliases_leave_the_exact_canonical_map_unchanged() {
        let mut app = App::new();
        app.declare_state::<Owner>(StateClass::Folded, "owner-state");
        let before = app.world().resource::<StateCensus>().entries().clone();
        app.declare_state_alias::<Handle<0>, Owner>()
            .declare_state_alias::<Handle<1>, Owner>()
            .declare_state_alias::<Handle<0>, Owner>();
        let census = app.world().resource::<StateCensus>();
        assert_eq!(census.entries(), &before);
        assert_eq!(census.len(), before.len());
        assert_eq!(census.aliases().len(), 2);
        for alias in [
            std::any::type_name::<Handle<0>>(),
            std::any::type_name::<Handle<1>>(),
        ] {
            assert_eq!(
                census.alias_owner(alias),
                Some(std::any::type_name::<Owner>())
            );
            assert_eq!(
                census.get(alias),
                census.get(std::any::type_name::<Owner>())
            );
        }
        assert_eq!(census.get(std::any::type_name::<Handle<2>>()), None);
        assert_eq!(census.alias_owner("Handle<0>"), None);
    }

    #[test]
    fn alias_rejects_unknown_owners_chains_shadowing_and_conflicting_ownership() {
        let mut census = StateCensus::default();
        let owner = std::any::type_name::<Owner>();
        let other = std::any::type_name::<OtherOwner>();
        let first = std::any::type_name::<Handle<0>>();
        let second = std::any::type_name::<Handle<1>>();
        assert!(census.declare_alias(first, owner).is_err());
        census.declare(owner, StateClass::Folded, "owner");
        census.declare(other, StateClass::Cache, "other");
        census.declare_alias(first, owner).unwrap();
        let before = census.aliases().clone();
        assert!(census.declare_alias(first, other).is_err());
        assert!(census.declare_alias(second, first).is_err());
        assert!(census.declare_alias(owner, other).is_err());
        assert!(census.declare_alias(second, second).is_err());
        assert_eq!(census.aliases(), &before);
        // Classification is inherited, never copied into a separately mutable row.
        census.declare(owner, StateClass::DeferredFold, "owner-revised");
        assert_eq!(
            census.get(first),
            Some((StateClass::DeferredFold, "owner-revised"))
        );
    }

    #[test]
    #[should_panic(expected = "canonical declaration cannot shadow an ownership alias")]
    fn canonical_declaration_cannot_reclassify_an_alias() {
        let mut census = StateCensus::default();
        census.declare("owner", StateClass::Folded, "owner");
        census.declare_alias("alias", "owner").unwrap();
        census.declare("alias", StateClass::Derived, "different");
    }
}
