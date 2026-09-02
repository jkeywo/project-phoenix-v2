//! Plugin registration for the simulation app assembly (issue #1199).
//!
//! Public surface: [`SimPluginOptions`], [`RegistrationOrder`],
//! [`RegistrationProbes`], and the two composition entry points
//! [`add_simulation_plugins`] / [`add_simulation_plugins_with`]. Re-exported
//! through `crate::server_app` so callers (bridge, headless, tests) keep their
//! existing paths.
//!
//! Role: the one place that wires the whole simulation onto an `App` — the
//! `SimSet` chain's `configure_sets` edges, physics registration, the per-table
//! plugins, the authoritative-state census declarations, resource init, and the
//! system registrations in their exact schedules and sets.
//!
//! Load-bearing invariant: registration ORDER and set membership are the
//! digest's inputs. Bevy's single-threaded executor breaks ordering ties by
//! registration order, so every `add_systems` / `add_plugins` call here must
//! stay in the sequence and `in_set(..)` it had before — this module is pure
//! motion of that sequence, not a re-ordering of it (see `register_physics`,
//! the admission-seam placement note, and `SIM_SET_PLUGIN_REGISTRARS`).

use super::*;

/// Which optional slices of the simulation registration to include.
///
/// The simulation proper is renderer-agnostic, but a handful of plugins and
/// systems registered alongside it (`StarRenderPlugin`, `PlanetRenderPlugin`,
/// `render_spawned_entities` and friends, the viewscreen radar, the asset
/// preloader) need meshes, materials and a `GameCamera`. Callers that run
/// without a `RenderPlugin` — the headless binary, and eventually the WASM
/// automation branch in `bridge.rs` — set `render: false` to skip them.
#[derive(Clone, Copy, Debug)]
pub struct SimPluginOptions {
    /// Register the render-coupled plugins and systems. `true` for the browser
    /// host; `false` for headless runs with no camera and no GPU.
    pub render: bool,
    /// Register [`RapierPhysicsPlugin`] **after** every simulation system
    /// instead of before them (issue #896's acceptance hook).
    ///
    /// Not a gameplay option and not reachable from any command line: the sole
    /// caller that sets it is the test that drives one colliding run each way
    /// and requires the two to agree bit for bit. That equality is the evidence
    /// that physics is ordered against the `SimSet` chain by the explicit
    /// `configure_sets` edges below, and not by the accident of which
    /// `add_plugins` call happened to come first.
    pub physics_last: bool,
    /// Which order to register the `SimSet`-chain plugins in (issue #899).
    ///
    /// `Canonical` reproduces the order below, unchanged. `Shuffled(seed)`
    /// deterministically permutes it — same seed, same permutation, every
    /// time — while leaving the `configure_sets` edges on `SimSet` itself,
    /// and every plugin's own internal `.after`/`.before` wiring, untouched.
    /// Those edges are exactly what SHOULD pin behaviour; this knob exists to
    /// prove that they do, and that nothing is quietly leaning on registration
    /// order instead. Not reachable from any command line, like `physics_last`
    /// above — the sole callers are `tests/registration_order_determinism.rs`.
    pub registration_order: RegistrationOrder,
    /// Two extra, mutually-unordered systems to fold into the same shuffled
    /// group, for the mutation-proof half of issue #899's guard only. `None`
    /// in every real call site and in every other test. See
    /// `tests/registration_order_determinism.rs` for what they prove.
    pub extra_registration_probes: Option<RegistrationProbes>,
}

/// A pair of `fn(&mut App)` registrars — see
/// [`SimPluginOptions::extra_registration_probes`]. Its own alias purely to
/// keep that field (and its `HeadlessArgs` twin) under clippy's
/// `type_complexity` threshold.
pub type RegistrationProbes = (fn(&mut App), fn(&mut App));

impl Default for SimPluginOptions {
    fn default() -> Self {
        Self {
            render: true,
            physics_last: false,
            registration_order: RegistrationOrder::Canonical,
            extra_registration_probes: None,
        }
    }
}

/// See [`SimPluginOptions::registration_order`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RegistrationOrder {
    /// The order the plugins are listed in `add_simulation_plugins_with`.
    #[default]
    Canonical,
    /// A deterministic permutation of that order, keyed by `seed`. Two calls
    /// with the same seed always produce the same permutation; different
    /// seeds are not guaranteed to differ from each other or from
    /// `Canonical`, so a test wanting a *changed* order should not assume a
    /// single seed proves anything — it should compare against `Canonical`.
    Shuffled(u64),
}

/// The `SimSet`-chain plugins, in their canonical registration order.
///
/// Each entry is a non-capturing closure — coerced to a plain `fn(&mut App)`
/// pointer — so the array is `Copy` and cheap to clone into a `Vec` for
/// shuffling. Extracted to its own array (rather than a chain of
/// `.add_plugins` calls) precisely so [`register_sim_set_plugins`] has
/// something it can reorder; the systems and edges each plugin registers are
/// unchanged either way.
const SIM_SET_PLUGIN_REGISTRARS: [fn(&mut App); 14] = [
    |app| {
        app.add_plugins(crate::regions::server::RegionPlugin);
    },
    |app| {
        app.add_plugins(crate::console_ai::server::ConsoleAiPlugin);
    },
    |app| {
        app.add_plugins(crate::ai::server::AiPlugin);
    },
    |app| {
        app.add_plugins(crate::console::captain::server::CaptainPlugin);
    },
    |app| {
        app.add_plugins(crate::console::command::server::CommandPlugin);
    },
    |app| {
        app.add_plugins(crate::console::helm::server::HelmPlugin);
    },
    |app| {
        app.add_plugins(crate::ship_plugin::ShipPlugin);
    },
    |app| {
        app.add_plugins(crate::console::weapons::WeaponsPlugin);
    },
    |app| {
        app.add_plugins(crate::console::repair::server::RepairPlugin);
    },
    |app| {
        app.add_plugins(crate::ship::power::ShipPowerPlugin);
    },
    |app| {
        app.add_plugins(crate::ship::shields::ShipShieldsPlugin);
    },
    |app| {
        app.add_plugins(crate::ship::sensors::ShipSensorsPlugin);
    },
    |app| {
        app.add_plugins(crate::console::navigation::NavigationPlugin);
    },
    |app| {
        app.add_plugins(crate::console::comms::server::CommsConsolePlugin);
    },
];

/// Register the `SimSet`-chain plugins listed in [`SIM_SET_PLUGIN_REGISTRARS`],
/// in either their canonical order or a deterministic shuffle of it (issue
/// #899), plus the mutation-proof probes when a test supplies them.
///
/// This is the coarsest granularity that satisfies the guard's acceptance
/// criteria: each plugin's internal systems keep whatever order the plugin
/// itself wires, and the `SimSet` chain's own `configure_sets` edges
/// (registered by the caller before this runs) are untouched. Only the
/// relative order these 13 (or 15, with probes) top-level registrations run
/// in is permuted — which is exactly the ambiguity `SimSet` leaves open for
/// Bevy to resolve, and exactly what a newly-added, un-gated system without
/// an explicit edge would otherwise be at the mercy of.
fn register_sim_set_plugins(app: &mut App, opts: SimPluginOptions) {
    let mut registrars: Vec<fn(&mut App)> = SIM_SET_PLUGIN_REGISTRARS.to_vec();
    if let Some((probe_a, probe_b)) = opts.extra_registration_probes {
        registrars.push(probe_a);
        registrars.push(probe_b);
    }
    if let RegistrationOrder::Shuffled(seed) = opts.registration_order {
        use rand::seq::SliceRandom;
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        registrars.shuffle(&mut rng);
    }
    for register in registrars {
        register(app);
    }
}

/// Register Rapier on the logical tick, ordered explicitly against the
/// `SimSet` chain (issue #896).
///
/// # The clock
/// Rapier used to run its `PhysicsSet` chain in `PostUpdate`, i.e. once per
/// rendered FRAME, stepping by whatever the host's frame period happened to be.
/// Two instances agreeing on the logical tick could still disagree on how many
/// times physics had stepped, so every collision the simulation consumed was a
/// function of frame pacing. `in_fixed_schedule()` moves the whole chain into
/// `FixedUpdate` alongside the simulation, and `TimestepMode::Fixed` with a
/// period derived from `[global] sim_tick_hz` makes each of those runs advance
/// physics by exactly one logical tick: N ticks step rapier N times, whatever
/// the frame rate. The resource is inserted BEFORE the plugin so the plugin's
/// own "you are in `FixedUpdate` without a fixed timestep" warning never fires,
/// and `sim_tick::reconcile_fixed_timestep` keeps it following a `WorldConfig`
/// swapped in at runtime, exactly as it does for `Time<Fixed>`.
///
/// `substeps: 1` because a substep is a solver subdivision, not a tick: the
/// simulation's own integration lives in the pure `ship::physics` module and
/// rapier is here for contacts and raycasts, so extra substeps would buy
/// nothing but float noise and time.
///
/// # The order within the tick
/// Physics has to sit between the two halves of the simulation that talk to it:
///
/// - `sync_ship_position` (`SimSet::Physics`) writes each ship's `Transform`
///   from the `ShipPhysics` this tick just integrated. `PhysicsSet::SyncBackend`
///   is what copies those transforms into rapier's bodies, so it must run
///   after — otherwise rapier steps last tick's positions.
/// - `handle_collisions` (`SimSet::Damage`) reads the contact pairs the step
///   produced, so `PhysicsSet::Writeback` — the end of rapier's chain — must
///   run before it.
///
/// Both edges are declared, rather than left to the schedule's defaults: with
/// the sets merely coexisting in `FixedUpdate` the graph would be free to
/// interleave them either way round, which is precisely the ambiguity this
/// issue exists to remove. Note the semantic shift this makes explicit — with
/// physics in `PostUpdate` a tick's collisions were resolved from the transforms
/// of the *previous* frame; now every tick sees its own.
fn register_physics(app: &mut App) {
    // `dt` comes from `sim_tick::sim_tick_period(hz).as_secs_f32()` rather
    // than a second, independent `1.0 / hz` division, so this and `Time<Fixed>`
    // (`reconcile_fixed_timestep`, `sim_tick.rs`) both derive rapier's step
    // from the identical nanosecond-quantized `Duration` — the one conversion
    // `sim_tick_period`'s own doc comment requires every driver to share. The
    // one remaining step, `Duration::as_secs_f32`, is a lossy f64→f32 cast
    // rapier's own `f32` dt forces; it is not claimed to be bit-identical to
    // `Time<Fixed>`'s f64-precision accumulator, only to agree with the other
    // rapier dt call site in `reconcile_fixed_timestep`.
    app.insert_resource(TimestepMode::Fixed {
        dt: crate::sim_tick::sim_tick_period(
            crate::entities::config::GlobalConfig::default().sim_tick_hz,
        )
        .as_secs_f32(),
        substeps: 1,
    })
    .add_plugins(RapierPhysicsPlugin::<()>::default().in_fixed_schedule())
    .configure_sets(
        FixedUpdate,
        (
            PhysicsSet::SyncBackend.after(crate::sim_sets::SimSet::Physics),
            PhysicsSet::Writeback.before(crate::sim_sets::SimSet::Damage),
        ),
    );
}

/// Compose all per-table simulation plugins onto `app`, including the
/// render-coupled ones.
///
/// This is the canonical registration point for the server simulation.
/// Call this from `wasm_init()` (bridge) instead of using a `SimulationPlugin`.
pub fn add_simulation_plugins(app: &mut App) {
    add_simulation_plugins_with(app, SimPluginOptions::default());
}

/// [`add_simulation_plugins`] with explicit control over the optional slices.
pub fn add_simulation_plugins_with(app: &mut App, opts: SimPluginOptions) {
    // The logical simulation tick (issue #895). The whole `SimSet` chain lives
    // in `FixedUpdate`, so the simulation advances zero or more whole steps per
    // rendered frame on the `[global] sim_tick_hz` clock rather than once per
    // frame. The default timestep is applied here so an app that never loads a
    // world still steps at the shipped rate; `reconcile_fixed_timestep` (in
    // `First`, before the fixed loop runs) applies the authored rate once a
    // `WorldConfig` exists.
    app.insert_resource(Time::<Fixed>::from_duration(
        crate::sim_tick::sim_tick_period(
            crate::entities::config::GlobalConfig::default().sim_tick_hz,
        ),
    ));
    crate::sim_tick::register_sim_tick(app);
    app.add_systems(First, crate::sim_tick::reconcile_fixed_timestep);

    // Physics first, unless the caller asked for it last — see
    // `SimPluginOptions::physics_last`. Which of the two it is must not matter,
    // and that is asserted rather than assumed.
    if !opts.physics_last {
        register_physics(app);
    }

    app.configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain()
            .run_if(in_state(GamePhase::InProgress))
            .after(crate::lobby::LobbySystemSet),
    );

    // The SimSet-chain plugins (issue #899's shuffle knob). Broken out of the
    // `.add_plugins` chain above into their own function so a test can permute
    // the order they register in — see `register_sim_set_plugins`.
    register_sim_set_plugins(app, opts);

    // Authoritative-state EXCLUSION declarations (issue #1221, Track 3 step C9).
    // Every type below is non-authoritative state this function OWNS — it
    // `init_resource`s / `insert_resource`s each of them further down (or, for the
    // Debug Surface flags, adds `DebugPlugin`, which inserts every catalogue
    // adapter resource on every assembled target, including headless). Each is
    // declared at this owning site via `App::declare_state`,
    // replacing the hand-maintained `EXCLUSIONS` const that used to live in
    // `tests/authoritative_state_enumeration.rs`; the enumeration guard now reads
    // the exclusion set back out of `StateCensus`. The declaration is inert to the
    // digest (see `src/authoritative.rs`), so nothing here moves a byte of the
    // authoritative-state digest — the determinism guard proves that directly.
    {
        use crate::authoritative::{DeclareState, StateClass};
        app
            // Timers / outboxes: wall-clock / transport bookkeeping, not sim state.
            .declare_state::<SimOutbox>(StateClass::Timer, "digest-exclusion-classes")
            .declare_state::<crate::debug_overlay::DamageLog>(
                StateClass::Timer,
                "digest-exclusion-classes",
            )
            // Cache of the presentation-only debug flags (issue #940):
            // `report_debug_state` compares against it to skip re-announcing.
            // "debug-overlay-state" is the PASM state entity in
            // pasm/spec/architecture/viewscreen-cameras-debug.yaml owned by
            // `debug-overlay-controller` (src/debug_overlay.rs) — every
            // `crate::debug_overlay` type below shares it (issue #1241
            // reconciled a drifted "debug-overlay-flags-state" spelling here
            // back onto the real entity rather than adding a duplicate one).
            .declare_state::<crate::debug_overlay::LastReportedDebugState>(
                StateClass::Cache,
                "debug-overlay-state",
            )
            // Cleared-at-fold: real inter-system-message state, but structurally
            // empty by the `RenderInterp` fold point on every correct instance.
            .declare_state::<crate::core::messages::InterSystemQueue>(
                StateClass::ClearedAtFold,
                "inter-system-message-state",
            )
            // The authored-beat / scripted-removal queue (issue #1338): pushed
            // by the shared dispatch applier, drained in full every tick by
            // `narrative::emit_authored_and_marked_entity_narrative`, so it is
            // empty at every fold/snapshot boundary — the same `ClearedAtFold`
            // contract every other `EffectQueue<T>` carries.
            .declare_state::<
                crate::effect_queue::EffectQueue<crate::core::narrative::NarrativeRequest>,
            >(StateClass::ClearedAtFold, "digest-exclusion-classes")
            // The post-mission report-row queue (issue #1344): same shape and
            // same contract as the narrative queue above — pushed by the shared
            // dispatch applier, drained in full every tick by
            // `mission_report::apply_report_rows`, so it is empty at every
            // fold/snapshot boundary. The REPORT it drains into is authoritative
            // and declared `Folded` below; the queue between them is not.
            .declare_state::<crate::effect_queue::EffectQueue<crate::core::report::ReportRow>>(
                StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            )
            // Presentation: the authored narrative mark (issue #1338). Scenario
            // data that decides only what the after-action timeline SHOWS —
            // nothing in the fixed tick branches on it, and neither
            // `sim_digest` nor `snapshot` walks it.
            .declare_state::<crate::core::narrative::NarrativeMark>(
                StateClass::Presentation,
                "narrative-event-stream",
            )
            // The `show_message(..)` request queue (issue #1342): pushed by the
            // shared dispatch applier, drained in full every tick by
            // `narrative::tick_computer_message` — the same `ClearedAtFold`
            // contract every other `EffectQueue<T>` carries.
            .declare_state::<
                crate::effect_queue::EffectQueue<
                    crate::core::computer_message::ComputerMessageRequest,
                >,
            >(StateClass::ClearedAtFold, "digest-exclusion-classes")
            // Presentation: the authoritative "one message at a time" state
            // (issue #1342). Viewscreen-only and presentation-only by the
            // issue's own contract — nothing in the fixed tick branches on a
            // message's text or severity, and neither `sim_digest` nor
            // `snapshot` walks it.
            .declare_state::<crate::core::computer_message::ActiveComputerMessage>(
                StateClass::Presentation,
                "computer-message-state",
            )
            // The continuous-task lifecycle queue (issue #1341): pushed by the
            // tractor and scan sites, drained in full every tick by
            // `narrative::emit_task_lifecycle_narrative`, so it is empty at every
            // fold/snapshot boundary — the same `ClearedAtFold` contract every
            // other `EffectQueue<T>` carries.
            .declare_state::<
                crate::effect_queue::EffectQueue<crate::core::task_lifecycle::TaskLifecycleRequest>,
            >(StateClass::ClearedAtFold, "digest-exclusion-classes")
            // Presentation: the live task-activation registry (issue #1341). It
            // decides only what the after-action timeline SHOWS — nothing in the
            // fixed tick reads it, and neither `sim_digest` nor `snapshot` walks
            // it. A resource rather than a `Local` on the emitter so the report
            // boundary can close whatever was still running when the run stopped.
            .declare_state::<crate::core::task_lifecycle::TaskLifecycles>(
                StateClass::Presentation,
                "task-lifecycle-registry",
            )
            // Presentation: the host per-Station attention surface (issue #1101);
            // it drives which tab asks for attention, never what the tick computes.
            .declare_state::<StationImportanceRes>(
                StateClass::Presentation,
                "station-importance-state",
            )
            // Presentation Debug Surface flags (issues #940/#1267).
            // `DebugPlugin` inserts them on every assembled target so each
            // catalogue adapter and native readback is live; this remains the
            // site that owns their presence in the authoritative-state census.
            .declare_state::<crate::debug_overlay::DebugRegionsEnabled>(
                StateClass::Presentation,
                "debug-overlay-state",
            )
            .declare_state::<crate::debug_overlay::DebugOverlayEnabled>(
                StateClass::Presentation,
                "debug-overlay-state",
            )
            .declare_state::<crate::debug_overlay::DebugDamageEnabled>(
                StateClass::Presentation,
                "debug-overlay-state",
            )
            .declare_state::<crate::debug_overlay::DebugEntitiesEnabled>(
                StateClass::Presentation,
                "debug-overlay-state",
            )
            .declare_state::<crate::debug_overlay::DebugEntityInspectorEnabled>(
                StateClass::Presentation,
                "debug-overlay-state",
            )
            // Derived: mass is inserted once at spawn from `EntityConfig.mass`
            // (`src/entities/spawner.rs`) and only ever read — it rides the content
            // digest, not the sim digest.
            .declare_state::<crate::entities::spawner::EntityMass>(
                StateClass::Derived,
                "entity-mass-state",
            );
    }

    // Authoritative-state DECLARATIONS (issue #1222, Track 3 step C10). This
    // block finishes the census migration #1220/#1221 began: the authoritative
    // half of the digest-boundary record — the types the census's old
    // `AUTHORITATIVE_SYMBOLS` const named — is now declared here in Rust, at the
    // sim-assembly owning site, exactly as #1221 declared the exclusion half in
    // the block just above (and where this function natively owns `SimRng`,
    // `CaptainPriorityBoost`, `GameOverReason`, `GodMode`, `AsteroidUuid`,
    // `ShipBoost` and `ShipImpulse` a few lines further down). Full type paths so
    // the census keys on the canonical name regardless of the alias written here.
    //
    // Two authoritative shapes, split exactly as `src/sim_digest.rs` folds them:
    //
    // * `Folded` — state `world_digest` walks every tick (`fold_run_scope`, the
    //   entity/infrastructure/civilian/weapons-hold/station-stances/tractor/dock/
    //   external-repair/umbilical/asteroid namespaces). A divergence in one of
    //   these is caught on the tick it happens.
    // * `DeferredFold` — authoritative state the record classifies as in-the-fold
    //   but that `world_digest` does NOT walk today (the honest "deferred, and the
    //   digest may grow to cover it" list in `sim_digest`'s module docs: weapons
    //   state machines, AI policy surfaces, selections, per-ship input, world
    //   layer/runtime tables, and the like). It is authoritative — never an
    //   exclusion — so the enumeration guard reads it into the authoritative set
    //   beside `Folded`, but marking it `Folded` would claim a fold that does not
    //   exist yet.
    //
    // The declaration is inert to the digest (`src/authoritative.rs`): nothing in
    // `world_digest` or the snapshot reads `StateCensus`, and the declaration-order
    // determinism guard proves it directly. `GamePhase` and `EntitySpawnOrigin` are
    // forward declarations — real authoritative types this world never registers
    // (`GamePhase` folds through `State<GamePhase>`, whose registry entry is a Bevy
    // path the crate-local scan filters out; `EntitySpawnOrigin` registers only
    // once a world runs a scripted spawn) — declared so the record stays complete
    // the day either one enters the scanned registry.
    {
        use crate::authoritative::{DeclareState, StateClass};
        app
            // ---- Folded: walked by `world_digest` every tick. ----
            // Run-scope preamble (`fold_run_scope`).
            .declare_state::<crate::sim_tick::SimTick>(
                StateClass::Folded,
                "digest-render-interp-fold-point",
            )
            .declare_state::<crate::sim_rng::SimRng>(StateClass::Folded, "sim-rng-state")
            .declare_state::<crate::world_id::WorldIdMint>(
                StateClass::Folded,
                "world-id-mint-state",
            )
            .declare_state::<GamePhase>(StateClass::Folded, "game-phase-state")
            .declare_state::<GameOverReason>(StateClass::Folded, "game-over-reason-state")
            // The structured post-mission report (issue #1344). Authoritative,
            // not presentation: a scenario SCRIPT writes it, so two instances on
            // the same seed must hold the same rows in the same order or they
            // have diverged. `Folded` and not `DeferredFold` because
            // `fold_run_scope` walks EVERY field of every row — id, both String
            // Ids, the state label and the hidden score — and `snapshot` captures
            // and restores all of them.
            .declare_state::<crate::core::report::MissionReport>(
                StateClass::Folded,
                "post-mission-report-state",
            )
            .declare_state::<CaptainPriorityBoost>(
                StateClass::Folded,
                "captain-objective-priority-state",
            )
            .declare_state::<crate::lobby::server::WorldResource>(
                StateClass::Folded,
                "digest-boundary-reviewer-answers",
            )
            // Entity namespace (`fold_entity_namespace`).
            .declare_state::<crate::entities::spawner::EntityUuid>(
                StateClass::Folded,
                "spawned-entity-state",
            )
            .declare_state::<crate::ship::state::ShipPhysics>(
                StateClass::Folded,
                "authoritative-ship-motion-state",
            )
            .declare_state::<crate::ship::state::ShipRedAlert>(
                StateClass::Folded,
                "authoritative-red-alert-state",
            )
            // Per-namespace folds (issues #1025/#1028/#1041/#1107/#1143/#907).
            .declare_state::<crate::infrastructure::InfrastructureCondition>(
                StateClass::Folded,
                "infrastructure-condition-state",
            )
            .declare_state::<crate::civilian::server::CivilianTraffic>(
                StateClass::Folded,
                "civilian-traffic-state",
            )
            .declare_state::<crate::ship::state::ShipWeaponsHold>(
                StateClass::Folded,
                "authoritative-weapons-hold-state",
            )
            .declare_state::<crate::console::command::server::ShipStationStances>(
                StateClass::Folded,
                "command-stance-selection-state",
            )
            .declare_state::<crate::tractor::server::TractorBeam>(
                StateClass::Folded,
                "tractor-beam-state",
            )
            .declare_state::<crate::dock::server::DockControl>(
                StateClass::Folded,
                "dock-relationship-state",
            )
            .declare_state::<crate::console::repair::external_server::ExternalRepairDispatch>(
                StateClass::Folded,
                "external-repair-dispatch-state",
            )
            .declare_state::<crate::umbilical::server::TransferUmbilical>(
                StateClass::Folded,
                "umbilical-flow-state",
            )
            // Security teams (issue #1346), the umbilical's shape exactly: what
            // `fold_security_namespace` walks is the authoritative half — which
            // teams are out, in what state, against which target — while the
            // authored `[security]` terms are content `content_digest` answers
            // for and the risk/elapsed/last-refusal fields are projections the
            // next tick re-derives.
            .declare_state::<crate::security::ShipSecurityTeams>(
                StateClass::Folded,
                "security-team-state",
            )
            // The target-side authored table is `DeferredFold` with ZERO folded
            // fields, which is the honest reading of that class: it is
            // authoritative — a host that thought a compartment offered a
            // different action would dispatch differently — but every byte of it
            // came out of the entity template and nothing ever writes it, so
            // `content_digest` already answers for it and `world_digest` walks
            // none of it.
            .declare_state::<crate::security::SecurityTargetActions>(
                StateClass::DeferredFold,
                "security-target-state",
            )
            .declare_state::<AsteroidUuid>(StateClass::Folded, "digest-fold-order-policy")
            // ---- DeferredFold: authoritative, not yet walked by `world_digest`. ----
            .declare_state::<crate::ai::server::LodBubble>(
                StateClass::DeferredFold,
                "npc-ai-controller-state",
            )
            .declare_state::<crate::ai::server::WorldSnapshot>(
                StateClass::DeferredFold,
                "world-snapshot-state",
            )
            .declare_state::<crate::ai::server::AiHighFidelity>(
                StateClass::DeferredFold,
                "npc-ai-fidelity-state",
            )
            .declare_state::<crate::ai::server::LodTransitionTimer>(
                StateClass::DeferredFold,
                "npc-ai-fidelity-state",
            )
            .declare_state::<crate::asteroids::lifecycle::AsteroidEntityMap>(
                StateClass::DeferredFold,
                "asteroid-window-state",
            )
            .declare_state::<crate::asteroids::lifecycle::AsteroidWindow>(
                StateClass::DeferredFold,
                "asteroid-window-state",
            )
            .declare_state::<crate::civilian::server::CivilianSection>(
                StateClass::DeferredFold,
                "civilian-traffic-adapter",
            )
            .declare_state::<crate::comms::component::CommsHailable>(
                StateClass::DeferredFold,
                "comms-range-state",
            )
            .declare_state::<crate::comms::component::CommsRange>(
                StateClass::DeferredFold,
                "comms-range-state",
            )
            // Issue #1086 put both types into `sim_digest::fold_comms_scope`,
            // and both land on `DeferredFold` — the standard stated once in
            // tests/authoritative_state_enumeration.rs's `WorldContentRuntime`
            // paragraph and applied here: `Folded` means the whole resource is
            // walked, and a partly-walked one keeps `DeferredFold` plus a comment
            // naming the halves.
            //
            // `CommsInboxRes` wraps `console::comms::inbox::CommsInbox`, which
            // has TWO fields, and only one is folded: `records` — every field of
            // every `CommsMessage`, the stored `sender_in_range` and per-response
            // `available` included (they are written once at injection and
            // carried verbatim by `snapshot::CommsState::inbox` — the per-tick
            // stamping happens on clones, never on the stored message) — is
            // walked by `fold_comms_scope`. `dirty` is not: it is broadcast
            // bookkeeping (whether a client push is owed since the last
            // broadcast), it is not carried by the snapshot payload, and
            // `snapshot::restore_comms` re-establishes it unconditionally via
            // `mark_dirty()` rather than restoring a captured value — the same
            // shape `CommsRuntime`'s demotion below uses for its own broadcast
            // bookkeeping.
            .declare_state::<crate::comms::server::CommsInboxRes>(
                StateClass::DeferredFold,
                "comms-inbox-state",
            )
            // `CommsRuntime` is folded in HALF: `active_dialogues` and
            // `open_hails` are walked; `contacts`, `range_flags` and
            // `range_active` are rebuilt every tick by
            // `update_comms_range_flags` from live entities and transforms the
            // entity namespace already folds, and `needs_broadcast` /
            // `last_broadcast_host` are broadcast bookkeeping (which peer was
            // last sent a `CommsState`, and whether one is owed) that the restore
            // re-establishes rather than carries.
            .declare_state::<crate::comms::server::CommsRuntime>(
                StateClass::DeferredFold,
                "comms-dialogue-state",
            )
            .declare_state::<crate::console::navigation::NavigationWaypoint>(
                StateClass::DeferredFold,
                "navigation-waypoint-state",
            )
            .declare_state::<crate::console::navigation::NavClearanceIssueState>(
                StateClass::DeferredFold,
                "navigation-clearance-issue-state",
            )
            .declare_state::<crate::ship::components::HelmWaypointClearance>(
                StateClass::DeferredFold,
                "helm-waypoint-clearance-state",
            )
            .declare_state::<crate::console::weapons::WeaponsDoctrineAiPolicy>(
                StateClass::DeferredFold,
                "weapon-family-arc-bearing-coordination",
            )
            .declare_state::<crate::console::weapons::NpcFrequencyMatchStates>(
                StateClass::DeferredFold,
                "npc-frequency-match-state",
            )
            .declare_state::<crate::console::weapons::beam::ActiveBeam>(
                StateClass::DeferredFold,
                "phaser-beam-state",
            )
            .declare_state::<crate::console::weapons::beam::CurrentPhaserMode>(
                StateClass::DeferredFold,
                "phaser-beam-state",
            )
            .declare_state::<crate::console::weapons::beam::PhaserCooldown>(
                StateClass::DeferredFold,
                "phaser-beam-state",
            )
            .declare_state::<crate::console::weapons::beam::TacticalRadarSelection>(
                StateClass::DeferredFold,
                "weapons-target-state",
            )
            .declare_state::<crate::debug_overlay::SimulationPaused>(
                StateClass::DeferredFold,
                "host-debug-simulation-override-state",
            )
            .declare_state::<crate::entities::model_rig::ModelMarkers>(
                StateClass::DeferredFold,
                "model-marker-runtime-state",
            )
            .declare_state::<crate::entities::spawner::EntityId>(
                StateClass::DeferredFold,
                "spawned-entity-state",
            )
            .declare_state::<crate::entities::spawner::EntityName>(
                StateClass::DeferredFold,
                "spawned-entity-state",
            )
            .declare_state::<crate::entities::spawner::HelmCapabilitySection>(
                StateClass::DeferredFold,
                "vertical-movement-mode-state",
            )
            .declare_state::<crate::entities::spawner::StaticPointDefence>(
                StateClass::DeferredFold,
                "spawned-entity-state",
            )
            .declare_state::<crate::entities::spawner::EntitySpawnOrigin>(
                StateClass::DeferredFold,
                "runtime-spawn-origin-state",
            )
            .declare_state::<GodMode>(
                StateClass::DeferredFold,
                "host-debug-simulation-override-state",
            )
            // Instagib (issue #1181): the sibling host-debug simulation override,
            // read by `tick_beams_apply_damage` via `Option<Res<Instagib>>`, so it
            // registers in the headless app the enumeration guard scans. Same
            // classification as `GodMode` / `SimulationPaused` — a wasm-only host
            // cheat, off and uninserted on native, that alters damage when on.
            .declare_state::<Instagib>(
                StateClass::DeferredFold,
                "host-debug-simulation-override-state",
            )
            .declare_state::<ShipBoost>(StateClass::DeferredFold, "boost-drive-state")
            .declare_state::<ShipImpulse>(StateClass::DeferredFold, "impulse-drive-state")
            .declare_state::<TrackedEntities>(
                StateClass::DeferredFold,
                "runtime-entity-projection-state",
            )
            .declare_state::<crate::ship_plugin::ShipSystemControlSources>(
                StateClass::DeferredFold,
                "system-control-source-state",
            )
            .declare_state::<crate::modifiers::cache::ShipModifiers>(
                StateClass::DeferredFold,
                "ship-modifier-state",
            )
            .declare_state::<crate::science::server::ShipScanRecord>(
                StateClass::DeferredFold,
                "science-scan-state",
            );
        // `AssetPreloadResource` is a presentation resource (`crate::server::
        // asset_preload`), init'd only in the `#[cfg(feature = "server")] if
        // opts.render` block below, so its declaration is gated the same way
        // (issue #1194): the always-compiled assembly must not name the presentation
        // module with the `server` feature off. Split out of the chain because a
        // single `.declare_state` link cannot carry a `#[cfg]`. Census-neutral for
        // every config that runs the enumeration guard — all of them build with
        // `server` on (headless = default + headless).
        #[cfg(feature = "server")]
        app.declare_state::<crate::server::asset_preload::AssetPreloadResource>(
            StateClass::DeferredFold,
            "asset-loading-state",
        );
        app.declare_state::<crate::ship::combat_activity::RecentCombatActivity>(
            StateClass::DeferredFold,
            "recent-combat-activity-state",
        )
        .declare_state::<crate::ship::components::ActiveStationRatings>(
            StateClass::DeferredFold,
            "active-station-rating-state",
        )
        .declare_state::<crate::ship::components::CoordinationQueue>(
            StateClass::DeferredFold,
            "coordination-lag-queue-state",
        )
        .declare_state::<crate::ship::components::LastHelmInput>(
            StateClass::DeferredFold,
            "last-helm-input-state",
        )
        .declare_state::<crate::ship::components::LastSystemTiers>(
            StateClass::DeferredFold,
            "system-damage-tier-memory-state",
        )
        .declare_state::<crate::ship::components::PendingArcBearingRequest>(
            StateClass::DeferredFold,
            "pending-arc-bearing-request-state",
        )
        .declare_state::<crate::ship::components::PendingTacticalFrequencyHint>(
            StateClass::DeferredFold,
            "tactical-frequency-hint-inbox-state",
        )
        .declare_state::<crate::ship::helm::BoostCommand>(
            StateClass::DeferredFold,
            "helm-actuator-input-state",
        )
        .declare_state::<crate::ship::helm::ImpulseCommand>(
            StateClass::DeferredFold,
            "helm-actuator-input-state",
        )
        .declare_state::<crate::ship::helm::LateralThrustInput>(
            StateClass::DeferredFold,
            "helm-actuator-input-state",
        )
        .declare_state::<crate::ship::helm::SteeringInput>(
            StateClass::DeferredFold,
            "helm-actuator-input-state",
        )
        .declare_state::<crate::ship::helm::ThrustInput>(
            StateClass::DeferredFold,
            "helm-actuator-input-state",
        )
        .declare_state::<crate::ship::helm_ai::HelmAiSurfacesFrame>(
            StateClass::DeferredFold,
            "helm-ai-surfaces-frame-state",
        )
        .declare_state::<crate::ship::intent_narration_systems::ShipIntentNarration>(
            StateClass::DeferredFold,
            "intent-narration-state",
        )
        .declare_state::<crate::ship::power::PowerBrownoutState>(
            StateClass::DeferredFold,
            "power-brownout-coordination-state",
        )
        .declare_state::<crate::ship::state::ShipPhaserFrequency>(
            StateClass::DeferredFold,
            "ship-phaser-frequency-state",
        )
        .declare_state::<crate::console_ai::server::ShipFrequencyHintState>(
            StateClass::DeferredFold,
            "sensors-frequency-hint-state",
        )
        .declare_state::<crate::ship::sensors::SensorRadarSelection>(
            StateClass::DeferredFold,
            "sensors-target-state",
        )
        .declare_state::<crate::ship::sensors::SensorsThreatState>(
            StateClass::DeferredFold,
            "sensors-threat-debounce-state",
        )
        .declare_state::<crate::ship::shields::ShieldsDamageHistory>(
            StateClass::DeferredFold,
            "shields-damage-history-state",
        )
        .declare_state::<crate::ship::shields::PendingShieldsThreatBearing>(
            StateClass::DeferredFold,
            "shields-threat-bearing-inbox-state",
        )
        .declare_state::<crate::ship::shields::ShieldsCoordinationState>(
            StateClass::DeferredFold,
            "shields-coordination-debounce-state",
        )
        .declare_state::<crate::world::config::WorldConfig>(
            StateClass::DeferredFold,
            "world-configuration-state",
        )
        .declare_state::<crate::world::server::EntityOriginLayer>(
            StateClass::DeferredFold,
            "world-layer-runtime-state",
        )
        .declare_state::<crate::world::server::PendingWorldLayerChanges>(
            StateClass::DeferredFold,
            "world-layer-runtime-state",
        )
        .declare_state::<crate::world::server::WorldEventBuffer>(
            StateClass::DeferredFold,
            "world-event-buffer-state",
        )
        // Partly folded since issue #1086, so it keeps `DeferredFold` — the
        // standard `CommsRuntime` above and `WorldContentRuntime` in
        // tests/authoritative_state_enumeration.rs are held to.
        // `sim_digest::fold_scenario_flags` walks every ACTIVE layer's path, its
        // `loader_path` and its POSITION in the activation order, plus its
        // `FlagStore` — the composition topology a snapshot recreates. The rest
        // of a `WorldRuntime` row stays outside the fold: `anchors`,
        // `delayed_unload_resolve` and `script_units` are authored content
        // (`snapshot::content_digest` answers for them), `spawned_entities` are
        // ECS handles whose identities the entity namespace already folds,
        // `owned_objective_ids` are re-derived from the layer's own trigger
        // replay, and the raw `activation_order` is deliberately not folded at
        // all — see `fold_scenario_flags` for why the payload cannot round-trip
        // it.
        .declare_state::<crate::world::server::WorldLayerMap>(
            StateClass::DeferredFold,
            "world-layer-runtime-state",
        );
    }

    app.add_message::<AsteroidDestroyedVfx>()
        // Balance telemetry. Registered here (not behind `headless`) so the
        // chokepoints can emit unconditionally — only the *collection* is
        // headless-only.
        .add_message::<crate::core::balance::BalanceEvent>()
        // Narrative telemetry (issue #1338, PRD #1337). Registered beside the
        // balance message and for the same reason: the emitters run on every
        // target so a beat is produced whether or not anyone is collecting, and
        // only the *collection* (`headless::report::collect_narrative_events`)
        // is headless-only. Kept a SEPARATE message from `BalanceEvent` because
        // the PRD keeps the story surface beside the combat ledger rather than
        // inside it — the per-ship ledgers must stay a fold of the balance
        // stream alone.
        .add_message::<crate::core::narrative::NarrativeEvent>()
        // The authored-beat / authored-entity-outcome / scripted-removal queue
        // the shared dispatch applier pushes onto (issue #1223's pattern, issue
        // #1338's payload).
        .init_resource::<
            crate::effect_queue::EffectQueue<crate::core::narrative::NarrativeRequest>,
        >()
        // The ship's-computer message state and its request queue (issue
        // #1342). Registered unconditionally for the same reason the
        // narrative queue above is: the emitter runs on every target, and
        // only the headless collection of its narrative events is
        // conditional.
        .init_resource::<crate::core::computer_message::ActiveComputerMessage>()
        .init_resource::<
            crate::effect_queue::EffectQueue<
                crate::core::computer_message::ComputerMessageRequest,
            >,
        >()
        // The continuous-task lifecycle queue and its activation registry (issue
        // #1341): the tractor and scan sites push start/end reports, and the
        // emitter turns them into the paired timeline beats.
        .init_resource::<
            crate::effect_queue::EffectQueue<crate::core::task_lifecycle::TaskLifecycleRequest>,
        >()
        .init_resource::<crate::core::task_lifecycle::TaskLifecycles>()
        // The post-mission report (issue #1344) and the script-boundary queue
        // that feeds it. `insert_resource` of the default rather than
        // `init_resource` for the report itself, matching `GameOverReason`
        // below: a run starts with no rows, and that is a stated fact rather
        // than a type default nobody wrote down.
        .init_resource::<crate::effect_queue::EffectQueue<crate::core::report::ReportRow>>()
        .insert_resource(crate::core::report::MissionReport::default())
        .init_resource::<CaptainPriorityBoost>()
        // The sim's one source of randomness. `init_resource` draws an OS seed, so
        // an unconfigured app (browser host, unit tests) behaves as it always did;
        // headless overrides it with a configured one via `insert_resource`.
        .init_resource::<crate::sim_rng::SimRng>()
        .insert_resource(crate::entities::config_cache::FactionRegistryResource(
            crate::entities::config_cache::get_faction_registry(),
        ))
        .init_resource::<WorldResource>()
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<TrackedEntities>()
        .init_resource::<SimOutbox>()
        .init_resource::<crate::core::messages::InterSystemQueue>()
        // `handle_collisions` (registered below, in SimSet::Damage) writes this,
        // so the simulation owns it. `DebugOverlayPlugin` also init_resource's it
        // — idempotent — but that plugin is absent headless, and the sim must not
        // depend on the debug overlay being present.
        .init_resource::<crate::debug_overlay::DamageLog>()
        .add_systems(
            Startup,
            setup_world
                .after(crate::world::server::insert_world_config_resource)
                // Issue #984 (Rhai M6 phase 2a), atomic-activation gate. This
                // spawner reaches `world_activation_blocked` (via
                // `spawn_anonymous_entities_internal`), which reads the
                // script-activation flag `compile_world_scripts` sets. On the
                // browser `setup_world` and the WorldPlugin's Startup chain are
                // independent, so without this `.after` a script-error world could
                // spawn its anonymous stars/planets before the script gate is set
                // — violating the "zero entities on script error" invariant.
                // `spawn_world_entities` (the named-entity half) is already gated
                // via the WorldPlugin `.chain()`.
                .after(crate::world::server::compile_world_scripts)
                // Determinism pin (issue #984, Rhai M6 phase 2a). `setup_world`
                // (anonymous stars/planets) and `spawn_world_entities`
                // (named/asteroid) both mint `IdNamespace::Entity` from the shared
                // `WorldIdMint` and conflict on `WorldConfig` (Res vs ResMut), so
                // Bevy serialises them — but with no edge between them their order
                // is a topological-sort tie-break. Pre-2a that tie-break happened
                // to run `setup_world` first; adding the `.after(compile_world_
                // scripts)` edge above shifted it, flipping the entity-mint order
                // for any world with anonymous entities (combat_test) and moving
                // its authoritative digest — a determinism regression for the whole
                // script-free shipped set, which must be a byte-identical no-op.
                // This `.before` restores the pre-2a relative order explicitly, so
                // the mint order no longer depends on the tie-break. It sits below
                // the gate edge, so the invariant is `compile_world_scripts <
                // setup_world < spawn_world_entities`: the script gate is still set
                // before either spawner runs, and the mint order matches the parent
                // commit exactly.
                .before(crate::world::server::spawn_world_entities),
        )
        .add_systems(
            OnEnter(GamePhase::InProgress),
            (
                // The run boundary for the command log (issue #898). First in the
                // chain because it is the *start* of a run's input record: every
                // command the systems after it cause to be admitted belongs to the
                // round that is beginning, and a second round reached through
                // `ReturnToLobby` must not inherit the first round's log. Sits
                // beside `reset_broadcast_caches_on_start` because it is the same
                // kind of thing — per-run state that a multi-game session has to
                // hand back.
                crate::command_admission::reset_command_log,
                // The run boundary for the post-mission report (issue #1344),
                // beside the command log's and for exactly the same reason: the
                // report is per-run state, and a second round reached through
                // `ReturnToLobby` must not inherit the first round's rows —
                // least of all into a scenario that authored no report and has
                // to keep its ordinary victory/defeat ending (AC5).
                crate::mission_report::reset_mission_report,
                reset_broadcast_caches_on_start,
                crate::world::server::seed_ship_power_counter,
                spawn_game_start_entities,
                dump_tracked_entities,
            )
                .chain(),
        )
        .add_systems(OnEnter(GamePhase::GameOver), on_game_over_enter)
        // Balance tracer for game-phase transitions. One global reader, one emit
        // per transition — inherently unconditional, no per-`next_state.set` taps.
        // In the fixed schedule so its events land in tick order with the rest of
        // the balance stream.
        .add_systems(FixedUpdate, emit_phase_change_balance_events)
        // The authored mission timeline (issue #1338). Chained and ordered after
        // `SimSet::Broadcast` — the last set of the tick — so every emitter sees
        // the state this tick finished with: the Objective the trigger pipeline
        // just completed, the deadline the callback drain just fired, the
        // `EntityDestroyed` the damage set just wrote, and the beat a script
        // queued anywhere in between. Chained among themselves so the monotonic
        // sequence `collect_narrative_events` assigns is a fixed function of the
        // tick rather than of executor order.
        .add_systems(
            FixedUpdate,
            (
                crate::narrative::emit_scenario_narrative,
                crate::narrative::emit_authored_and_marked_entity_narrative,
                // The ship's-computer message (issue #1342): applies this
                // tick's `show_message(..)` requests, checks the active
                // message's simulation-time expiry, and emits the
                // shown/superseded/expired narrative trio. Chained after the
                // other two so a message authored by the SAME trigger that
                // also posts a beat this tick reports in a stable order.
                crate::narrative::tick_computer_message,
                // Last of the narrative emitters (issue #1341): a task's
                // terminal beat reads as the consequence of the tick, so it
                // sequences after the Objective, deadline and entity beats that
                // share it.
                crate::narrative::emit_task_lifecycle_narrative,
                // The report accumulator (issue #1344) joins the same chain, and
                // last of all: a row a script wrote this tick lands after the
                // beats that caused it, so an after-action reading sees the
                // story move before it sees the report move.
                crate::mission_report::apply_report_rows,
            )
                .chain()
                .after(crate::sim_sets::SimSet::Broadcast),
        )
        // Clear the active ship's-computer message on the two transitions the
        // issue names: mission end and a return to the lobby. Not a narrative
        // beat — the shown/superseded/expired trio is the whole of AC5,
        // and this is neither.
        .add_systems(
            OnEnter(GamePhase::GameOver),
            crate::narrative::clear_active_computer_message,
        )
        .add_systems(
            OnEnter(GamePhase::Lobby),
            crate::narrative::clear_active_computer_message,
        )
        .insert_resource(GameOverReason(None, None))
        .add_systems(
            FixedUpdate,
            (reconcile_runtime_entities, broadcast_world_setup_on_start)
                .chain()
                .after(crate::lobby::LobbySystemSet)
                .before(crate::sim_sets::SimSet::Input),
        );

    // The command admission seam (issue #898): the tick-stamped command log,
    // the future-tick queue it drains, `CommandDelay`, and
    // `admit_system_commands` itself — one call, because a half-wired seam
    // fails silently in three different ways. See `register_admission_seam`.
    //
    // Admission moves with the sim into `FixedUpdate` (issue #895): inbound
    // messages are drained once per FRAME in `PreUpdate`, so admitting per
    // frame would clear-and-refill `AdmittedCommands` zero or several times per
    // tick. The helper places it exactly once per tick, before `SimSet::Input`,
    // whatever the frame rate.
    //
    // **Registered exactly here on purpose.** `admit_system_commands` and the
    // `reconcile_runtime_entities` block above are both `.after(LobbySystemSet)
    // .before(SimSet::Input)` with no edge between them, so their relative order
    // is a tie — and Bevy's single-threaded executor (which `--deterministic`
    // selects) breaks ties by REGISTRATION order. Reconcile spawns and despawns
    // ships; admission resolves commands to ships. Hoisting this call earlier in
    // the function reverses that tie, and the headless duel probes — which are
    // combat-chaotic — change outcome. Keep the call where the systems it
    // registers used to be added.
    crate::command_admission::register_admission_seam(
        app,
        crate::command_admission::AdmissionGate::InProgressOnly,
    );

    // Host-to-host lockstep (issue #1116). Registered immediately after the
    // admission seam because it is the same seam seen from the fleet's side: it
    // owns the slot admission orders this host's commands under, the queue the
    // peers' commands enter, and the barrier that decides whether the next tick
    // may run at all. Everything it installs is inert for a lone host — the
    // default `FleetRoster` is a fleet of one, no `LockstepSession` exists until
    // one forms, and `CommandDelay` stays at zero — so a single-player run is
    // byte-identical to a pre-#1116 one.
    crate::lockstep::register_lockstep(app);

    // God Mode (issue #900): the resource `apply_god_mode_toggle` flips, and
    // the consumer registration so the unrouted-command lint below doesn't
    // warn about `god-mode` — the applier registered right after this is its
    // one consumer. Registered here (not via a console plugin) because no
    // console owns it: it is a host-only debug toggle, not a station system.
    app.init_resource::<GodMode>();
    {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        app.register_admitted_consumer(ConsumerMatcher::undeclared_exact(
            crate::ship::system_registry::GOD_MODE_SYSTEM_ID,
        ));
    }
    app.add_systems(
        FixedUpdate,
        apply_god_mode_toggle.in_set(crate::sim_sets::SimSet::Input),
    );

    // The phone client's settings route (issue #940): drain the debug flags and
    // the pause a connected phone asks for, then report the resulting state
    // back to every client. `PreUpdate`, ordered, and deliberately NOT in
    // `FixedUpdate` — pausing starves the fixed schedule, so a fixed drain
    // could never undo a pause and a fixed reporter could never announce one.
    // See `debug_overlay::drain_client_pause`.
    //
    // Gated on the two plugins that own what they touch. `DebugOverlayPlugin`
    // is added by `wasm_init` and owns `SimulationPaused`; `LobbyPlugin` brings
    // `Sessions` and the `OutboundMessage` stream. A headless run deliberately
    // omits `DebugOverlayPlugin` / `SimulationPaused` even though its lobby
    // supplies `Sessions`, and it has no phone debug route to serve. Check the
    // absent owner resource directly: since issue #1267
    // `DebugRegionsEnabled` is inserted on every assembled target for the
    // all-build Debug Surface catalogue, so it is no longer a browser-owner
    // proxy.
    //
    // The reporter is registered unconditionally and the two drains are not:
    // in a demo build the phone has no route to either, but it is still told
    // what the host's own cog did. Two `add_systems` calls rather than one
    // `#[cfg]`-riddled tuple, so the demo build's schedule is a strictly
    // smaller thing rather than a differently-shaped one.
    // Host-side Station importance (issue #1101). Registered unconditionally so
    // the broadcaster's ingest/read always has a resource; the visit drain is
    // gated on `Sessions` existing (a headless run has no phone to serve and no
    // inbound stream to read).
    app.init_resource::<StationImportanceRes>();
    app.add_systems(
        PreUpdate,
        drain_station_visited.run_if(resource_exists::<crate::lobby::Sessions>),
    );

    app.init_resource::<crate::debug_overlay::LastReportedDebugState>();
    // `drain_client_debug_flags` moved to the bridge/marshalling side in issue
    // #1193 (it calls `apply_pending_toggles`); its pause sibling stays sim-side
    // in `debug_overlay`. The whole chain is `server`-gated (issue #1194): the
    // debug-flag drain names `crate::server::bridge`, presentation the always-
    // compiled assembly must not reference with the feature off. The pause drain
    // rides along inside the gate rather than being split into its own ungated
    // call — every config that runs the client-facing debug route builds with
    // `server` on (a phone only reaches the host through the bridge), and keeping
    // both in one `.chain()` preserves the exact schedule shape server-on builds
    // already had, so the sim digest is untouched.
    #[cfg(all(not(phoenix_demo_build), feature = "server"))]
    app.add_systems(
        PreUpdate,
        (
            crate::server::bridge::drain_client_debug_flags,
            crate::debug_overlay::drain_client_pause,
        )
            .chain()
            .before(crate::debug::catalogue::refresh_readback)
            .run_if(
                resource_exists::<crate::debug_overlay::SimulationPaused>
                    .and(resource_exists::<crate::lobby::Sessions>),
            ),
    );
    app.add_systems(
        PreUpdate,
        crate::debug_overlay::report_debug_state
            .after(crate::debug::catalogue::refresh_readback)
            .run_if(
                resource_exists::<crate::debug_overlay::SimulationPaused>
                    .and(resource_exists::<crate::lobby::Sessions>),
            ),
    );

    // Console input-to-feedback latency, client half (issue #1169). Connected
    // clients measure their OWN console round trips — both stamps on one device's
    // clock — and report the durations; this folds them into the same tracker the
    // host's own admission→broadcast window lands in, so one payload carries the
    // whole picture.
    //
    // `PreUpdate` and not `FixedUpdate`, deliberately: this is a session
    // diagnostic, it changes no simulation outcome, and putting it in the fixed
    // schedule would tie a non-deterministic reading to the tick a replay
    // re-derives. Gated on the flag and on `Sessions` for the same reason its
    // `drain_client_debug_flags` neighbour is — a headless run has no phone to
    // hear from — and compiled out of a demo build with the message it reads.
    #[cfg(not(phoenix_demo_build))]
    app.add_systems(
        PreUpdate,
        crate::debug::console_latency::drain_console_latency_reports
            .run_if(resource_exists::<crate::lobby::Sessions>)
            .run_if(|flag: Res<crate::debug::DebugConsoleLatencyEnabled>| flag.0),
    );

    // Structured debug observability (PRD #1144, issue #1145). Always-on
    // station-activity counters read the tick's fully-populated `AdmittedCommands`
    // after `SimSet::Broadcast` — the same window the unrouted lint below uses,
    // and the only tap that sees both network-admitted and in-process AI-emitted
    // commands (see `crate::debug::station_activity`). Added on every target so
    // headless and native get the same counters; the JSON publish behind it is
    // flag-gated, the counters are not. Read-only, so it never moves the digest.
    app.add_plugins(crate::debug::DebugPlugin);

    // Unrouted-command lint (issue #833). Production wires the admission seam
    // through `register_admission_seam` (above) rather than via
    // `AdmissionPlugin`, so the lint is added here too. Warning-only, ordered
    // after every consumer set; observes the tick's admitted set before next
    // tick's clear. The `AdmittedConsumerRegistry` it reads is populated by each
    // consumer plugin's `register_admitted_consumer` call at build time.
    app.add_systems(
        FixedUpdate,
        crate::command_admission::warn_unrouted_admitted_commands
            .after(crate::sim_sets::SimSet::Broadcast)
            .run_if(in_state(GamePhase::InProgress)),
    );

    // Replication owners register their lifecycle adapters beside their live
    // publishers. The composition root only calls the owner registrars;
    // neither lifecycle runner learns cache resources or payload shapes.
    register_entity_state_replication_lifecycle(app);
    register_blackboard_replication_lifecycle(app);

    app.add_systems(
        FixedUpdate,
        broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::PublishAggregate),
    )
    .add_systems(
        FixedUpdate,
        refresh_caches_on_midgame_reconnect
            .after(crate::lobby::LobbySystemSet)
            .before(crate::lobby::server::drain_lobby_outbox)
            .before(crate::sim_sets::SimSet::Broadcast),
    )
    .add_systems(
        FixedUpdate,
        (
            handle_collisions.in_set(crate::sim_sets::SimSet::Damage),
            sim_processing_anchor,
        )
            .after(crate::lobby::LobbySystemSet),
    )
    .add_systems(
        FixedUpdate,
        crate::modifiers::coordination::translate_power_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        FixedUpdate,
        crate::modifiers::coordination::translate_impulse_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        FixedUpdate,
        crate::modifiers::coordination::apply_radar_damage_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        FixedUpdate,
        (
            clear_last_attacker_on_death,
            clear_last_attacker_on_red_alert_off,
            // `publish_viewscreen_blackboard` (LocalShip) and
            // `aggregate_doctrine_blackboards` (BehaviourSection) both write the
            // SAME viewscreen blackboard entry, and after #842 the game-start
            // player carries BOTH markers. Without a defined order they raced and
            // last-writer-wins CLOBBERED — the doctrine writer dropped the
            // player's scenario objectives entirely (a defence scenario stopped
            // developing combat). Pin the LocalShip writer to run *after* the
            // doctrine writer so it can MERGE the two objective pools (scenario ∪
            // template doctrine) instead of one silently erasing the other.
            publish_viewscreen_blackboard.after(crate::ai::server::aggregate_doctrine_blackboards),
        )
            .in_set(crate::sim_sets::SimSet::PublishAggregate),
    )
    .add_plugins(weapons_update_broadcaster())
    .add_plugins(sim_state_broadcaster())
    .add_plugins(modifier_events_broadcaster())
    .add_plugins(sim_outbox_broadcaster());

    if opts.render {
        app.add_plugins(crate::entities::star::StarRenderPlugin)
            .add_plugins(crate::entities::planet::PlanetRenderPlugin)
            .init_resource::<ProceduralMeshCache>()
            // The authored `[render]` calibration (PRD #1023). Initialised to
            // its own defaults here so the LOD swap always has one, and
            // overwritten from the world's block at `PostStartup` by the
            // renderer plugin.
            .init_resource::<crate::render_setup::RenderTuning>()
            .add_systems(Update, render_spawned_entities)
            .add_systems(Update, update_mesh_lod.after(render_spawned_entities))
            .add_systems(
                Update,
                // After the fade driver, so a billboard mid-cross-fade folds
                // THIS frame's fade alpha into its pose weights rather than
                // last frame's — the two share one alpha channel and
                // `orient_lod_billboards` is its only writer.
                crate::entities::billboard::orient_lod_billboards::<
                    crate::render_setup::GameCamera,
                >
                    .after(update_mesh_lod)
                    .after(crate::entities::visual_fade::drive_visual_fades),
            )
            .add_systems(
                Update,
                crate::entities::visual_fade::drive_visual_fades.after(update_mesh_lod),
            )
            .add_systems(Update, face_player_lights.after(render_spawned_entities));
    }

    #[cfg(feature = "server")]
    if opts.render {
        use crate::server::asset_preload::{
            auto_transition_from_loading, begin_asset_preload, broadcast_loading_progress,
            broadcast_loading_start, poll_asset_preload,
        };
        app.add_plugins(crate::server::ServerViewscreenRadarPlugin)
            // The reference grid reads the SAME hull config the viewscreen
            // radar above does, through the same `SelectedShipResource` +
            // config-cache path, so the two can never disagree about which hull
            // the player is flying. It attaches nothing to any simulation
            // entity — see the module note on why that matters for the digest.
            .add_plugins(crate::server::ReferenceGridPlugin)
            .init_resource::<crate::server::asset_preload::AssetPreloadResource>()
            .add_systems(Update, begin_asset_preload)
            .add_systems(Update, poll_asset_preload)
            .add_systems(OnEnter(GamePhase::Loading), broadcast_loading_start)
            .add_systems(
                Update,
                broadcast_loading_progress.run_if(in_state(GamePhase::Loading)),
            )
            // `FixedUpdate`, not `Update` (issue #907, applied to this system in
            // #1121's fix round). The readiness POLL stays frame-paced — it is
            // asset streaming, which is a function of disk and GPU and has no
            // business on the logical tick — but the `NextState<GamePhase>`
            // WRITE moves onto the tick, before `SimSet::Input`, alongside
            // `tick_countdown` and `native_host::solo_auto_start`, the two other
            // systems that start a mission.
            //
            // A `NextState` write from a frame schedule applies at the
            // frame-level `StateTransition`, so `OnEnter(GamePhase::InProgress)`
            // — and the player-ship mint inside it — would fire at a point whose
            // relationship to `SimTick` depends on frame pacing. That is exactly
            // what #907 ruled out, and until this moved it was the ONLY route to
            // `InProgress` for a crewed session: `solo_auto_start` had the
            // treatment and the path every real crew takes did not.
            //
            // The `.after(poll_asset_preload)` edge goes with the move — an
            // ordering edge is only real inside one schedule — and its loss
            // costs a frame of latency and nothing else: the poll writes
            // `preload.complete` in `Update`, this reads it on the next tick,
            // and `broadcast_loading_progress` keeps the client's bar moving in
            // the meantime.
            .add_systems(
                FixedUpdate,
                auto_transition_from_loading
                    .run_if(in_state(GamePhase::Loading))
                    .before(crate::sim_sets::SimSet::Input),
            );
    }

    // The reversed half of the registration-order pair. Everything physics
    // needs to be ordered against is already in the graph by now, so if the
    // `configure_sets` edges in `register_physics` are doing the work, this
    // app and the default one are the same simulation.
    if opts.physics_last {
        register_physics(app);
    }
}
