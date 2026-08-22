//! The `ai` log category's decision-trace slice (issue #1146, PRD #1144).
//!
//! # What this proves
//!
//! Two things, both about the `ai`-category decision traces this issue added to
//! the doctrine aggregator (`ai::server::aggregate_doctrine_blackboards`), the
//! Tactical target selector (`console::weapons::ai_target_selection`) and the
//! Captain host (`console::captain::operate_captain_ai`):
//!
//! 1. **Determinism (the headline AC).** A seeded headless run is byte-identical
//!    — every RNG stream position, every ship's physics and hull, every recorded
//!    collision — whether `ai` logging is OFF (the default `warn` floor), ON
//!    unfiltered (`ai=debug`), or ON narrowed to one hull (`ai=debug` +
//!    `--log-entity`). Enabling the traces, and actually firing every emitter,
//!    moves not one byte of the digest. The traces are a read-only projection of
//!    authoritative state that never touches the sim or its RNG.
//!
//! 2. **The trace has real content to carry.** `cargo test` installs no
//!    `tracing` subscriber, so the emitted log LINES cannot be captured here (see
//!    the note in `logging::macros`). The codebase's convention — used by
//!    `command_admission::router`'s unrouted-lint tests — is to prove the
//!    DECISION that drives the event through the same pure function the emitter
//!    calls, against the same authoritative state the emitter reads. So the test
//!    below runs the identical `ai=debug --log-entity` configuration a tuner
//!    would, then reconstructs, from the surviving authoritative blackboard, the
//!    directive the doctrine trace would have logged for the filtered hull —
//!    proving the emit path had a real directive to name, not that it stayed
//!    silent. The exact field VALUES (`prev`/`new`/`target`/`score`) are unit-
//!    tested in `project_phoenix::ai::decision_trace`.
//!
//! # Why this is its own test binary
//!
//! Same reason as `tests/registration_order_determinism.rs`: `--deterministic`
//! pins Bevy's `TaskPoolPlugin` to a single thread, and task pools are
//! process-global, initialised by whichever app builds first. Sharing a binary
//! with another headless test means inheriting a pool a neighbour created.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

mod common;

use bevy::prelude::*;
use common::SimFixture;
use project_phoenix::ai::decision_trace::top_directive;
use project_phoenix::core::messages::SystemBlackboard;
use project_phoenix::entities::spawner::EntityName;
use project_phoenix::headless::fingerprint::{fingerprint, RunFingerprint};
use project_phoenix::headless::HeadlessArgs;
use project_phoenix::logging::{parse_log_entities, parse_log_spec};
use project_phoenix::server_app::ShipSystemBlackboards;
use project_phoenix::ship::system_registry::viewscreen_system_id;

/// `rng_coverage.toml` (issue #837): two AI-driven NPC lancers in weapons range
/// plus the AI-flown player destroyer, so the doctrine aggregator, the Tactical
/// target selector and the Captain host all run and actually reselect targets
/// and change directives inside the window below — exactly the emitters this
/// slice populates. The same world the determinism guards already lean on.
const WORLD: &str = "assets/worlds/rng_coverage.toml";

/// Long enough that an NPC has scored a directive and a fight has broken out
/// (mirrors `tests/registration_order_determinism.rs`), short enough to stay
/// fast.
const TICKS: u64 = 300;

const SEED: u64 = 20261146;

/// A substring of the world-authored name of one NPC lancer
/// (`world.entity.lancer_alpha.name`), matched case-insensitively as a substring
/// by the entity filter — the `--log-entity <ship>` a tuner would type.
const FILTER_SHIP: &str = "lancer_alpha";

/// Build args for `WORLD` at `SEED`, then apply `mutate` to fold in a log config.
fn args_with(mutate: impl FnOnce(&mut HeadlessArgs)) -> HeadlessArgs {
    let mut args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };
    mutate(&mut args);
    args
}

fn fingerprint_of(args: HeadlessArgs) -> RunFingerprint {
    let mut app = SimFixture::new(args).build_and_run();
    fingerprint(&mut app)
}

/// Issue #1146's headline AC: enabling `ai` decision logging — unfiltered or
/// narrowed to one hull — leaves a seeded run's digest byte-identical to the
/// logging-off run. The traces never perturb the simulation.
#[test]
fn ai_logging_on_off_and_filtered_reach_the_identical_digest() {
    // (1) Default floor: no `ai` traces emit at all.
    let off = fingerprint_of(args_with(|_| {}));

    // (2) `ai=debug` unfiltered: every emitter fires, on every hull, every tick.
    let on_unfiltered = fingerprint_of(args_with(|a| {
        a.log = parse_log_spec("ai=debug").expect("valid spec");
        a.log_spec = "ai=debug".into();
    }));

    // (3) `ai=debug --log-entity lancer_alpha`: the filtered emit path, the exact
    //     configuration a tuner uses.
    let on_filtered = fingerprint_of(args_with(|a| {
        a.log = parse_log_spec("ai=debug").expect("valid spec");
        a.log_spec = "ai=debug".into();
        a.log.entity_filter = parse_log_entities(FILTER_SHIP);
    }));

    assert!(
        !off.ships.is_empty(),
        "precondition: the fingerprint covers no ship — the comparison would be \
         vacuous"
    );
    assert!(
        !off.collisions.is_empty(),
        "precondition: no collision recorded in {TICKS} ticks of {WORLD}; the \
         run is too quiet to exercise the physics-adjacent state the fingerprint \
         guards. Ships: {:?}",
        off.ships
    );

    assert_eq!(
        off, on_unfiltered,
        "enabling `ai=debug` decision logging moved the seeded digest — an `ai` \
         emitter is perturbing the simulation (drawing from SimRng, mutating \
         authoritative state, or reordering work). The traces must be a \
         read-only projection; see src/ai/decision_trace.rs."
    );
    assert_eq!(
        off, on_filtered,
        "enabling `ai=debug --log-entity` moved the seeded digest — the \
         per-entity-filtered emit path is perturbing the simulation. See \
         src/ai/decision_trace.rs and src/logging/."
    );
}

/// The trace has a real decision to carry: under the exact `ai=debug
/// --log-entity lancer_alpha` configuration, the filtered NPC reaches a scored
/// top directive on its viewscreen blackboard — the very value the doctrine
/// decision trace reads and logs. Reconstructed from surviving authoritative
/// state because `cargo test` installs no subscriber to capture the log line
/// itself (see the module docs); the field mapping from this directive to the
/// event's `prev`/`new`/`target`/`score` is unit-tested in
/// `ai::decision_trace`.
#[test]
fn the_filtered_hull_reaches_a_directive_the_doctrine_trace_would_log() {
    let args = args_with(|a| {
        a.log = parse_log_spec("ai=debug").expect("valid spec");
        a.log_spec = "ai=debug".into();
        a.log.entity_filter = parse_log_entities(FILTER_SHIP);
    });
    let mut app = SimFixture::new(args).build_and_run();

    let vs_id = viewscreen_system_id();
    let mut named_pool_directives: Vec<(String, Option<String>)> = app
        .world_mut()
        .query::<(&EntityName, &ShipSystemBlackboards)>()
        .iter(app.world())
        .filter_map(|(name, blackboards)| match blackboards.0.get(&vs_id) {
            Some(SystemBlackboard::Viewscreen(v)) => Some((
                name.0.clone(),
                top_directive(&v.scored_objectives).map(|o| format!("{:?}", o.directive)),
            )),
            _ => None,
        })
        .collect();
    named_pool_directives.sort();

    assert!(
        !named_pool_directives.is_empty(),
        "precondition: no BehaviourSection hull published a viewscreen \
         blackboard in {TICKS} ticks — the doctrine aggregator never ran, so \
         there was nothing for its `ai` trace to project"
    );

    let filtered = named_pool_directives
        .iter()
        .find(|(name, _)| name.to_lowercase().contains(FILTER_SHIP));
    let (name, top) = filtered.unwrap_or_else(|| {
        panic!(
            "the filtered hull matching {FILTER_SHIP:?} was not among the ships \
             that published a viewscreen blackboard: {:?}. The `--log-entity` \
             config would then narrow the `ai` trace to a hull that emits \
             nothing.",
            named_pool_directives
        )
    });
    assert!(
        top.is_some(),
        "the filtered hull {name:?} published a viewscreen blackboard but scored \
         no top directive after {TICKS} ticks, so the doctrine decision trace \
         would have nothing but `none` to log for it — pick a longer window or a \
         hull that actually acts."
    );
}
