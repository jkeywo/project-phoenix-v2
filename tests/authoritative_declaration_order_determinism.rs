//! The standing guard that the authoritative-state declaration registry
//! (`src/authoritative.rs`, issue #1220) is INERT to the #901 digest.
//!
//! # What this proves
//!
//! `App::declare_state::<T>(class, pasm)` records a type in the [`StateCensus`]
//! resource, keyed by its full path. That census is a coverage/diagnostic
//! surface: nothing in `src/sim_digest.rs`'s `world_digest` or `src/snapshot.rs`
//! reads it. This guard turns that claim into a test — the order plugins declare
//! their state in (and whether they declare it at all) must not move a single
//! byte of the authoritative-state digest.
//!
//! It mirrors `tests/registration_order_determinism.rs` for the mechanism: the
//! same `SimPluginOptions::extra_registration_probes` seam folds a pair of
//! declaration-only registrars into the very machinery that registers the 13
//! `SimSet`-chain plugins, and `RegistrationOrder` permutes it. Two techniques
//! are used together — an explicit flip of the two probes (`(a, b)` vs `(b, a)`
//! under `Canonical`, the guaranteed permutation the #899 mutation-proof test
//! uses) and two deterministic `Shuffled` seeds — and every resulting
//! `world_digest` is compared byte-for-byte against a no-declaration baseline.
//!
//! # Why its own test binary
//!
//! The same reason as `tests/registration_order_determinism.rs` and
//! `tests/authoritative_state_enumeration.rs`: `--deterministic` pins Bevy's
//! `TaskPoolPlugin` to one thread, task pools are process-global, and sharing a
//! binary with a neighbour would inherit whichever pool built first.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::marker::PhantomData;

use bevy::prelude::*;
use project_phoenix::authoritative::{DeclareState, StateCensus, StateClass};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::server_app::{RegistrationOrder, RegistrationProbes};
use project_phoenix::sim_digest::world_digest;

/// `rng_coverage.toml` (issue #837), the same world the other two determinism
/// guards use: two NPCs in weapons range, an asteroid field, a radiation zone —
/// so `world_digest` folds a rich, non-trivial authoritative state rather than
/// an idle one, which is what makes "the registry did not move it" meaningful.
const WORLD: &str = "assets/worlds/rng_coverage.toml";
const TICKS: u64 = 300;
const SEED: u64 = 20261220;

/// Distinct marker types the probes declare. They register no systems and no
/// entities — a declaration touches only [`StateCensus`] — so they cannot
/// perturb the simulation the digest measures.
struct ProbeFolded;
struct ProbePresentation;
struct ProbeGeneric<T>(PhantomData<T>);

/// First declaration-only probe: two states, one of them a generic
/// instantiation, so [`ProbeGeneric<u8>`] and [`ProbeGeneric<u16>`] below prove
/// the full-path census key keeps distinct instantiations distinct.
fn declare_probe_a(app: &mut App) {
    app.declare_state::<ProbeFolded>(StateClass::Folded, "probe-folded-state")
        .declare_state::<ProbeGeneric<u8>>(StateClass::Cache, "probe-generic-u8-state");
}

/// Second declaration-only probe, declared in the opposite half of the flip.
fn declare_probe_b(app: &mut App) {
    app.declare_state::<ProbePresentation>(StateClass::Presentation, "probe-presentation-state")
        .declare_state::<ProbeGeneric<u16>>(StateClass::Derived, "probe-generic-u16-state");
}

/// Build and run the seeded world under `order`, optionally injecting the
/// declaration probes, and return `(world_digest, the StateCensus if any)`.
fn digest_and_census(
    order: RegistrationOrder,
    probes: Option<RegistrationProbes>,
) -> (u64, Option<StateCensus>) {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        registration_order: order,
        extra_registration_probes: probes,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    let digest = world_digest(app.world());
    let census = app.world().get_resource::<StateCensus>().cloned();
    (digest, census)
}

/// Issue #1220's headline acceptance criterion: permuting declaration order
/// leaves `world_digest` byte-identical, because the census is digest-inert.
#[test]
fn permuting_declaration_order_leaves_the_authoritative_digest_identical() {
    // No declarations at all: since this issue wires none into any production
    // plugin, StateCensus is never even initialised in a real run.
    let (baseline_digest, baseline_census) = digest_and_census(RegistrationOrder::Canonical, None);
    assert!(
        baseline_census.is_none(),
        "no production plugin declares yet, so StateCensus must be absent when \
         no probe declares — got {baseline_census:?}"
    );

    // The same world, now with four states declared during build, in permuted
    // orders: an explicit flip of the two probes (guaranteed to change the
    // order the four declarations arrive in) plus two deterministic shuffles
    // that also move the probes among the 13 SimSet-chain plugins.
    let (d_ab, c_ab) =
        digest_and_census(RegistrationOrder::Canonical, Some((declare_probe_a, declare_probe_b)));
    let (d_ba, c_ba) =
        digest_and_census(RegistrationOrder::Canonical, Some((declare_probe_b, declare_probe_a)));
    let (d_s1, c_s1) =
        digest_and_census(RegistrationOrder::Shuffled(1), Some((declare_probe_a, declare_probe_b)));
    let (d_s2, c_s2) = digest_and_census(
        RegistrationOrder::Shuffled(0xC0FFEE),
        Some((declare_probe_a, declare_probe_b)),
    );

    let c_ab = c_ab.expect("the probes declare, so StateCensus must exist");
    let c_ba = c_ba.expect("the probes declare, so StateCensus must exist");
    let c_s1 = c_s1.expect("the probes declare, so StateCensus must exist");
    let c_s2 = c_s2.expect("the probes declare, so StateCensus must exist");

    // Precondition: all four declarations landed, and the two generic
    // instantiations did NOT collapse to one entry (the full-path census key).
    assert_eq!(
        c_ab.len(),
        4,
        "expected four declared probe states (two of them distinct generic \
         instantiations); got {:?}",
        c_ab.entries()
    );

    // The census content is a pure function of the SET of declarations, never
    // the order they arrived in — a `BTreeMap` keyed by full path.
    assert_eq!(
        c_ab.entries(),
        c_ba.entries(),
        "census content changed when the two probes were flipped — it is not \
         order-independent"
    );
    assert_eq!(c_ab.entries(), c_s1.entries());
    assert_eq!(c_ab.entries(), c_s2.entries());

    // The headline property: declaring state — and the order it is declared in
    // — is inert to the authoritative-state digest. Byte-identical to the
    // no-declaration baseline, and across every permutation.
    assert_eq!(
        baseline_digest, d_ab,
        "declaring authoritative state at all moved world_digest — the census \
         registry is NOT inert to the digest; something in sim_digest.rs or \
         snapshot.rs is reading StateCensus"
    );
    assert_eq!(
        d_ab, d_ba,
        "flipping the two declaration probes changed world_digest — declaration \
         ORDER is leaking into the digest"
    );
    assert_eq!(
        d_ab, d_s1,
        "shuffling declaration order (seed 1) changed world_digest — declaration \
         order is leaking into the digest"
    );
    assert_eq!(
        d_ab, d_s2,
        "shuffling declaration order (seed 0xC0FFEE) changed world_digest — \
         declaration order is leaking into the digest"
    );
}
