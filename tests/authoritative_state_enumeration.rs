//! Deny-by-default enumeration guard for the authoritative-state digest
//! boundary (issue #894, parent #849).
//!
//! # What this proves
//!
//! #894 decided and recorded WHICH types a future digest folds over — the
//! record lives at `pasm/spec/architecture/deterministic-simulation.yaml`
//! (the fold policy itself) plus `implementation.symbols` back-filled onto
//! every `classification: authoritative` `state` entity across
//! `pasm/spec/architecture/*.yaml` (73 entities, 25 files). A record nobody
//! checks against the running app is a record that silently stops being true
//! the first time someone adds a new component and forgets it exists.
//!
//! This guard builds the real headless sim app, reads back EVERY
//! crate-local component and resource Bevy actually registered
//! (`app.world().components()` — Bevy 0.18 stores components and resources in
//! the same registry, see `bevy_ecs::component::ComponentInfo`'s own doc
//! comment), and requires each one to be accounted for by exactly one of:
//!
//! 1. The declaration registry ([`StateCensus`], read here via
//!    [`census_authoritative_short_names`]) — authoritative simulation state,
//!    each type declared at its OWNING `build()` site via
//!    `App::declare_state::<T>(class, pasm)` as one of the two authoritative fold
//!    shapes: `Folded` (state `src/sim_digest.rs` walks every tick) or
//!    `DeferredFold` (authoritative state the record classifies as in-the-fold
//!    but that `world_digest` does not walk yet). Before issue #1222 this was a
//!    hand-maintained `AUTHORITATIVE_SYMBOLS` const in this file transcribed from
//!    the `implementation.symbols` lists on PASM's `classification: authoritative`
//!    `state` entities; the guard now reads the set back out of the census the
//!    built app populates. The traceability-only symbols that const also carried
//!    (function/accessor names, and value types that ride a larger folded
//!    resource rather than register in their own right) stay in PASM as
//!    `implementation.symbols` back-references — they could never key a type
//!    registry — and are not declared here.
//! 2. The same declaration registry ([`StateCensus`], read here via
//!    [`census_excluded_short_names`]) — real, legitimately-registered
//!    non-authoritative state, each type declared at its OWNING plugin's
//!    `build()` via `App::declare_state::<T>(class, pasm)` (issue #1221) with
//!    the reason class the PASM record (`deterministic-simulation.yaml`'s
//!    `digest-exclusion-classes` entity) uses, now carried as a [`StateClass`]:
//!    `Presentation` / `Cache` / `Timer` / `Derived` / `ClearedAtFold` /
//!    `TestInfra`. Before #1221 this was a hand-maintained `EXCLUSIONS` const in
//!    this file; the guard now reads the set back out of the census the built
//!    app populates.
//! 3. [`UNCLASSIFIED_BASELINE`] — the honest remainder. Issue #894's own
//!    scope note: 113 conceptual PASM state entities against 171 Rust
//!    components and 134 resources is a real granularity mismatch, and fully
//!    classifying all of it is bigger than this issue's acceptance criteria.
//!    Rather than pretend otherwise, the gap is made COUNTABLE — the same
//!    discipline `entities::ai_declaration_manifest::EXPECTED_UNDECLARED`
//!    already uses for PRD #774's undeclared-AI worklist. The baseline is a
//!    ratchet: it may shrink (someone classifies a type into (1) or (2) and
//!    removes its entry) but a computed set with anything NOT in the
//!    committed baseline fails immediately, naming the type and pointing
//!    here and at `pasm/spec/architecture/deterministic-simulation.yaml`.
//!
//! A type that is new since this issue and unclassified in every one of the
//! three lists is exactly the failure #894 exists to prevent: "someone adds
//! `CloakState` next quarter, nobody adds it to the list, and the digest
//! silently stops covering divergent state" (deterministic-simulation.yaml).
//!
//! # Why Bevy's own registry rather than a derive or marker trait
//!
//! A hand-written allowlist of ~40 types out of 171 components fails the
//! exact way this issue exists to prevent — the list is explicit AND wrong.
//! Reading `app.world().components()` instead means the source of truth is
//! what the sim app ACTUALLY registers, not what a crate-wide grep finds:
//! viewer-only and editor-only components never enter the headless sim app
//! and self-exclude, without this guard needing to know they exist.
//!
//! # Why this is its own test binary
//!
//! Same reason as `tests/registration_order_determinism.rs` and
//! `tests/rng_determinism.rs`: `--deterministic` pins Bevy's `TaskPoolPlugin`
//! to a single thread, and task pools are process-global, initialised by
//! whichever app builds first. Sharing a binary with another headless test
//! would mean inheriting a pool a neighbour already created.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::authoritative::{StateCensus, StateClass};
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};

/// `rng_coverage.toml` (issue #837), same as
/// `tests/registration_order_determinism.rs`: two NPCs in weapons range, an
/// asteroid field, a radiation zone — beam, blaster, torpedo, collision and
/// region damage all fire inside the window below, which is what registers
/// the widest realistic set of components/resources a single run can reach.
const WORLD: &str = "assets/worlds/rng_coverage.toml";
const TICKS: u64 = 300;
const SEED: u64 = 20260894;

/// Derive the AUTHORITATIVE set — the guard's "list 1" — from the
/// authoritative-state declaration registry ([`StateCensus`]) the running sim
/// app populated, rather than from a hand-maintained `const`.
///
/// Since issue #1222 (Track 3 step C10) every authoritative type that used to
/// live in the local `AUTHORITATIVE_SYMBOLS` const is declared at its owning
/// `build()` site via `App::declare_state::<T>(class, pasm)` (see
/// `src/authoritative.rs` and the block in `server_app::add_simulation_plugins_with`),
/// and this guard reads the set back out of the census — the mirror image of
/// [`census_excluded_short_names`] below. The two authoritative fold shapes count
/// here: `Folded` (state `src/sim_digest.rs`'s `world_digest` walks every tick)
/// and `DeferredFold` (authoritative state the record classifies as in-the-fold
/// but that `world_digest` does not walk yet). Every declared EXCLUSION class
/// (`Presentation`/`Cache`/`Timer`/`Derived`/`ClearedAtFold`/`TestInfra`) is
/// filtered OUT — those are list 2, read by [`census_excluded_short_names`].
///
/// The ~100 traceability names the old const also carried — function/accessor
/// names, `thread_local!` pairs, and value types that live on a larger folded
/// resource rather than register in their own right (`ScanReading`, `WorldId`,
/// `SpawnOrigin`, `SimRngState`, `InfrastructureState`, the `apply_*`/`tick_*`
/// systems, `INTENT_NARRATION_SPAWN_SITES`, …) — could never key a type registry,
/// so they are NOT declared here; they stay as `implementation.symbols` back-
/// references on their owning `state` entities in `pasm/spec/architecture/*.yaml`
/// (and the consolidating `digest-census-traceability-symbols` note in
/// `deterministic-simulation.yaml`), exactly where they were transcribed from.
///
/// `GamePhase` and `EntitySpawnOrigin` are declared as forward declarations
/// (real authoritative types this world never registers), so their short names
/// appear here even though no registered type matches them — harmless in a
/// superset check, and what keeps [`ac5_reviewer_answers_match_the_pasm_record`]'s
/// `GamePhase` IN-call answerable from the census alone.
///
/// Reduced to SHORT names for the same reason the exclusion set is: the two
/// superset lists are consulted by short name, only [`UNCLASSIFIED_BASELINE`]
/// keys on the full path (see [`short_name`]).
fn census_authoritative_short_names(app: &App) -> std::collections::BTreeSet<String> {
    app.world()
        .get_resource::<StateCensus>()
        .map(|census| {
            census
                .entries()
                .iter()
                .filter(|(_, (class, _))| !is_digest_exclusion(*class))
                .map(|(full, _)| short_name(full))
                .collect()
        })
        .unwrap_or_default()
}

/// Derive the digest EXCLUSION set — the guard's "list 2" — from the
/// authoritative-state declaration registry ([`StateCensus`]) the running sim
/// app populated, rather than from a hand-maintained `const`.
///
/// Since issue #1221 (Track 3 step C9) every non-authoritative type that used to
/// live in a local `EXCLUSIONS` const is declared at its OWNING plugin's
/// `build()` via `App::declare_state::<T>(class, pasm)` (see
/// `src/authoritative.rs`), and this guard reads the set back out of the census
/// instead. The reason classes are unchanged — presentation / cache / timer /
/// derived / cleared-at-fold, exactly the `deterministic-simulation.yaml`
/// `digest-exclusion-classes` vocabulary — they now live as a [`StateClass`] at
/// each declaration site rather than a string beside a name here.
///
/// `RenderInterp` and `ViewscreenMotion` are declared at their render /
/// viewscreen plugins, which a headless run never adds; they never register in
/// this census either, so their absence from this set is correct, not a gap (the
/// old const listed them only as forward documentation). Everything the headless
/// sim app actually registers is declared at a plugin the headless app DOES add,
/// so it lands here.
///
/// The census keys on the FULL type path so two distinct generic instantiations
/// (`BroadcastRegistry<Sim>` / `BroadcastRegistry<Lobby>`) cannot collapse; the
/// guard consults the two superset lists and this set by the SHORT name (see
/// [`short_name`]), so this reduces each declared exclusion's full path to its
/// short name, exactly as the old const listed them.
///
/// Only the true digest EXCLUSION classes count as "list 2": a future
/// `Folded` / `DeferredFold` declaration is authoritative state IN the digest and
/// must never be read here as an exclusion (see [`is_digest_exclusion`]). As of
/// C9 nothing declares those, so the filter is belt-and-suspenders — but it keeps
/// the set honest the day a folded declaration lands beside these.
///
/// `EntitySnapshot` (`src/core/messages.rs`) is deliberately covered by NEITHER
/// this set nor the census-derived authoritative set
/// ([`census_authoritative_short_names`]): it carries no
/// `#[derive(Component)]`/`#[derive(Resource)]` at all (a plain wire-message
/// struct), so it can never appear in the registry this guard scans. Its
/// rejection as a digest-boundary shortcut is recorded in
/// `deterministic-simulation.yaml` (`digest-boundary-reviewer-answers`) and
/// re-proven by [`ac5_reviewer_answers_match_the_pasm_record`], not here.
fn census_excluded_short_names(app: &App) -> std::collections::BTreeSet<String> {
    app.world()
        .get_resource::<StateCensus>()
        .map(|census| {
            census
                .entries()
                .iter()
                .filter(|(_, (class, _))| is_digest_exclusion(*class))
                .map(|(full, _)| short_name(full))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a declared [`StateClass`] is a digest EXCLUSION — real but
/// non-authoritative state that must stay OUT of the fold — rather than one of
/// the two authoritative fold shapes (`Folded` / `DeferredFold`).
fn is_digest_exclusion(class: StateClass) -> bool {
    !matches!(class, StateClass::Folded | StateClass::DeferredFold)
}

/// The honest remainder: every crate-local component/resource the sim app
/// registers that neither the census-derived authoritative set
/// ([`census_authoritative_short_names`]) nor the census-derived exclusion set
/// ([`census_excluded_short_names`]) reaches yet, computed from a real run and
/// committed here as a ratchet.
///
/// This is NOT a claim that any of these 171-components/134-resources-worth
/// of remaining state SHOULD stay out of the digest forever — issue #894's
/// own scope note says classifying all of it is bigger than this issue's
/// acceptance criteria. It is the worklist: shrinking this list means declaring
/// an entry authoritative (`Folded`/`DeferredFold`, with a PASM
/// `implementation.symbols` entry to match) or as an `App::declare_state`
/// exclusion at its owning plugin (with a reason class). Growing it silently
/// is exactly what `unclassified_types_match_the_committed_baseline` exists
/// to prevent — a brand-new type lands here as a test FAILURE naming it,
/// never as a quiet pass.
///
/// Computed from a real run of `WORLD` at `TICKS` (issue #894's own pass, the
/// day the enumeration guard was written) via
/// `every_registered_type_maps_to_the_digest_record`'s own failure message —
/// run it, copy the printed list, and replace this array when the change is
/// intentional.
#[rustfmt::skip]
const UNCLASSIFIED_BASELINE: &[&str] = &[
    // Full module paths, not short names (issue #1220): the census key is the
    // full type path so two distinct generic instantiations cannot collapse to
    // one entry. The two superset lists above stay short names; only this
    // exhaustive remainder is keyed on the full path. Regenerate mechanically
    // from a real run — see `every_registered_type_maps_to_the_digest_record`'s
    // failure message — never hand-edit.
    "project_phoenix::ai::server::AiHighFidelity",
    "project_phoenix::ai::server::AiProfile",
    "project_phoenix::ai::server::AiTokenRegistry",
    "project_phoenix::ai::server::LodTransitionTimer",
    "project_phoenix::ai::server::ObjectiveCursors",
    // The authoritative-state declaration registry itself (issue #1220's
    // `StateCensus`, `src/authoritative.rs`). It first ENTERS the registry as of
    // issue #1221: `declare_state` `init_resource`s it on first use, and #1221 is
    // when production plugins begin declaring, so a real run now inserts it. It is
    // a build-time coverage/diagnostic surface — populated once as plugins build,
    // never read by the fixed tick, never folded (the determinism guard proves
    // exactly that) — so it is non-authoritative. It fits no single digest
    // exclusion class cleanly (it is neither presentation, cache, timer, derived,
    // nor cleared-at-fold), so rather than misclassify it, it sits on the honest
    // baseline beside `AdmittedConsumerRegistry` below — the other build-time
    // registry populated by plugin `build()` calls.
    "project_phoenix::authoritative::StateCensus",
    // The boot render-surrogate seam marker (issue #1218). A zero-sized
    // `#[derive(Resource)]` that `boot::render_surrogate` inserts so boot's
    // three-profile parity test can assert which renderer path a profile took;
    // it now rides into the headless sim app because `build_headless_app` composes
    // through `boot::build`. Build-time diagnostic state — inserted once as the
    // app composes, never read by the fixed tick, never folded — so it sits on the
    // honest baseline beside `StateCensus` above rather than being misclassified
    // into a digest-exclusion class it does not fit.
    "project_phoenix::boot::RenderSurrogateApplied",
    "project_phoenix::command_admission::log::CommandDelay",
    "project_phoenix::command_admission::log::CommandLog",
    "project_phoenix::command_admission::log::PendingCommands",
    "project_phoenix::command_admission::router::AdmittedConsumerRegistry",
    "project_phoenix::comms::server::OnScreenMessage",
    "project_phoenix::console::captain::server::CaptainAiPolicy",
    "project_phoenix::console::comms::server::CommsResponseAiCadence",
    "project_phoenix::console::comms::server::CommsResponseAiPolicy",
    "project_phoenix::console::comms::server::CommsTargetSelector",
    "project_phoenix::console::navigation::server::NavClearanceIssueState",
    "project_phoenix::console::navigation::server::NavigationTargetSelector",
    "project_phoenix::console::repair::server::RepairRequestQueue",
    "project_phoenix::console::repair::server::RepairTargetSelector",
    "project_phoenix::console::repair::server::ShipRepairTeams",
    "project_phoenix::console::repair::visibility::LastVisibleRepairBlackboard",
    "project_phoenix::console::weapons::server::NpcFrequencyMatchStates",
    "project_phoenix::console::weapons::server::PhaserRenderConfig",
    "project_phoenix::console::weapons::server::WeaponsArcRequestState",
    "project_phoenix::console::weapons::beam::LastShipAttacker",
    "project_phoenix::console::weapons::beam::PhaserBankAiPolicies",
    "project_phoenix::console::weapons::beam::PhaserCombatConfigResource",
    "project_phoenix::console::weapons::beam::TacticalTargetSelector",
    "project_phoenix::console::weapons::blackboard::WeaponsUpdateFirstTick",
    "project_phoenix::console::weapons::blaster::BlasterBankAiPolicies",
    "project_phoenix::console::weapons::blaster::BlasterSystemResource",
    "project_phoenix::console::weapons::shared::BeamContext",
    "project_phoenix::console::weapons::shared::TorpedoTargetSnapshot",
    "project_phoenix::console::weapons::torpedo::TorpedoMagazineAiPolicy",
    "project_phoenix::console::weapons::torpedo::TorpedoSystemResource",
    "project_phoenix::console::weapons::torpedo::TorpedoTubeAiPolicies",
    "project_phoenix::console_ai::server::ShipFrequencyHintState",
    "project_phoenix::core::messages::AdmittedCommands",
    "project_phoenix::core::telemetry::RunTelemetry",
    "project_phoenix::entities::config_cache::FactionRegistryResource",
    "project_phoenix::entities::spawner::AsteroidFieldSection",
    "project_phoenix::entities::spawner::BehaviourSection",
    "project_phoenix::entities::spawner::CinematicCameraSection",
    "project_phoenix::entities::spawner::ColliderSection",
    "project_phoenix::entities::spawner::EntityShipArcHull",
    "project_phoenix::entities::spawner::EntitySystemHull",
    "project_phoenix::entities::spawner::EntityTagsSection",
    "project_phoenix::entities::spawner::EntityTarget",
    "project_phoenix::entities::spawner::FactionComponent",
    "project_phoenix::entities::spawner::HelmConsoleSection",
    "project_phoenix::entities::spawner::MeshSection",
    "project_phoenix::entities::spawner::RadarAppearanceSection",
    "project_phoenix::entities::spawner::RegionEffectsSection",
    "project_phoenix::entities::spawner::RegionShapeSection",
    "project_phoenix::entities::spawner::ShipAudioSection",
    "project_phoenix::entities::spawner::WeaponsConsoleSection",
    "project_phoenix::lobby::server::CountdownTimer",
    "project_phoenix::lobby::server::GameStateCache",
    "project_phoenix::lobby::server::LobbyOutbox",
    "project_phoenix::lobby::server::SelectedShipResource",
    "project_phoenix::lobby::server::Sessions",
    "project_phoenix::lobby::server::ShipClientConfigResource",
    "project_phoenix::lobby::server::ShipManualResource",
    "project_phoenix::lobby::stations_config::ShipStations",
    "project_phoenix::logging::filter::LogFilterConfig",
    "project_phoenix::regions::server::RegionMembership",
    "project_phoenix::server::viewscreen_border::ShakeState",
    "project_phoenix::server_app::broadcast_publish::WorldSetupBroadcast",
    "project_phoenix::server_app::components::Asteroid",
    "project_phoenix::server_app::components::AsteroidShieldPierce",
    "project_phoenix::server_app::components::CollisionCooldown",
    "project_phoenix::server_app::components::LocalShip",
    "project_phoenix::server_app::components::Ship",
    "project_phoenix::server_app::components::ShipAttackedThisTick",
    "project_phoenix::server_app::components::ShipSystemBlackboards",
    "project_phoenix::server_app::components::TrackedEntities",
    "project_phoenix::server_app::components::WeaponFiredThisTick",
    "project_phoenix::ship::components::BankConfigResource",
    "project_phoenix::ship::components::BoostConfigResource",
    "project_phoenix::ship::components::DockingMotionIntent",
    "project_phoenix::ship::components::HelmWaypointClearance",
    "project_phoenix::ship::components::ImpulseConfigResource",
    "project_phoenix::ship::components::LastSystemTiers",
    "project_phoenix::ship::components::PendingShipConfig",
    "project_phoenix::ship::components::PendingTacticalFrequencyHint",
    "project_phoenix::ship::components::RepairHumanAlerted",
    "project_phoenix::ship::components::ShipConfigComponent",
    "project_phoenix::ship::components::ShipPhysicsConfigResource",
    "project_phoenix::ship::components::ShipSystemControlSources",
    "project_phoenix::ship::helm::HelmPhysicsFrame",
    "project_phoenix::ship::helm::HelmPhysicsWriteGuard",
    "project_phoenix::ship::helm::VerticalThrustInput",
    "project_phoenix::ship::helm_ai::AiPolicyTickClock",
    // Issue #1209 collapsed the six per-axis `Helm*AiPolicy` newtypes
    // (Engines/Steering/Lateral/Vertical/Impulse/Boost) into this ONE keyed
    // component. It is authored-immutable and not snapshotted, so it sits on the
    // honest baseline exactly as `PhaserBankAiPolicies` above does — not folded
    // and not a declared exclusion. The two axes previously declared
    // `DeferredFold` (Boost/Impulse policies) lost their `declare_state` with the
    // newtypes; the STATE twins below are unchanged (LOD-carried + snapshotted).
    "project_phoenix::ship::helm_ai::FineSystemAiPolicies",
    "project_phoenix::ship::helm_ai::boost::HelmBoostAiPolicyState",
    "project_phoenix::ship::helm_ai::engines::HelmEnginesAiPolicyState",
    "project_phoenix::ship::helm_ai::surfaces::HelmPassSurface",
    "project_phoenix::ship::helm_ai::surfaces::HelmRecoveryHistory",
    "project_phoenix::ship::helm_ai::steering::HelmSteeringAiPolicyState",
    "project_phoenix::ship::helm_planner::HelmMotionPlan",
    "project_phoenix::ship::power::PowerAiCadence",
    "project_phoenix::ship::power::PowerAiPolicy",
    "project_phoenix::ship::power::PowerBrownoutState",
    "project_phoenix::ship::power::PowerConfigResource",
    "project_phoenix::ship::power::PowerMultiplierResource",
    "project_phoenix::ship::power::ShipPowerSystem",
    "project_phoenix::ship::sensors::SensorsAiConfigResource",
    "project_phoenix::ship::sensors::SensorsFrequencyState",
    "project_phoenix::ship::sensors::SensorsTargetSelector",
    "project_phoenix::ship::sensors::SensorsThreatState",
    "project_phoenix::ship::shields::PendingShieldsThreatBearing",
    "project_phoenix::ship::shields::ShieldsAiConfigResource",
    "project_phoenix::ship::shields::ShieldsCoordinationState",
    "project_phoenix::ship::shields::ShieldsFocusAiPolicy",
    "project_phoenix::ship::shields::ShipShields",
    "project_phoenix::ship::state::ShipPhaserFrequency",
    "project_phoenix::ship::state::ShipViewMode",
    "project_phoenix::world::server::ObjectiveManagerRes",
    "project_phoenix::world::server::PendingScenarioLoad",
    "project_phoenix::world::server::WorldContentRuntime",
    // The Rhai scripting seam (issue #984, Rhai M6 phase 2a/2b). Both are
    // authoritative-but-deferred, exactly like `WorldContentRuntime` above:
    // `RawWorldSource` is the world TOML the script loader reads at `Startup`
    // (as loaded, after any headless duel-side transform);
    // `WorldScriptRuntime` holds the compiled handler ASTs, the
    // per-tick script budget, the content hash and — since phase 2b — the live
    // `PendingCallbacks` queue of deferred `after(n, |ctx| …)` callbacks that
    // `tick_script_callbacks` drains each tick. That `PendingCallbacks` is now
    // POPULATED authoritative future work (a serialisable `(fire_tick,
    // script_path, fn_name)` vec inside `WorldScriptRuntime`), so it belongs in
    // the same digest fold as `WorldContentRuntime`'s own deferred state
    // (`pending_delayed_actions` / `pending_world_events`). It carries no
    // `#[derive(Resource)]` of its own — it lives inside `WorldScriptRuntime` —
    // so it never appears as a distinct registry entry; this one baseline line
    // covers it. Registered in the census run even for a script-free world (the
    // systems reference them via `Option<Res>` / `Option<ResMut>`), but never
    // instantiated there.
    //
    // Issue #1024's named mission deadlines are covered by these same two lines
    // and by `WorldContentRuntime` above, and register NOTHING of their own —
    // which is the whole reason the state was put where it was. The live table
    // (`DeadlineTable`, with its `DeadlineRecord`/`DeadlineState`) is a FIELD on
    // `WorldContentRuntime`; the `on_deadline` declarations
    // (`Vec<DeadlineHandler>`) are a FIELD on `WorldScriptRuntime`; and a
    // deadline's deferred firing is an ordinary `ScheduledCall` on the
    // `PendingCallbacks` this comment already accounts for. So the slice adds no
    // `#[derive(Resource)]` and no `#[derive(Component)]`, nothing new appears in
    // the registry this guard scans, and both baseline entries keep covering
    // exactly what they did before — with more state behind them. See
    // `src/world/deadlines.rs` for why a deadline is a record over the existing
    // queue rather than a scheduler (and a resource) of its own, and
    // pasm/spec/architecture/scenario-scripting.yaml's `mission-deadline-state`
    // entity for the `implementation.symbols` naming those Rust types.
    //
    // Issue #1029's commitments ledger is covered the same way, by
    // `WorldContentRuntime` above: `CommitmentLedger` — with its `Commitment`,
    // `CommitmentState` and the `CommitmentChange`/`CommitmentMutation` a script
    // call buffers — is a FIELD on that resource, sitting beside the deadline
    // table for the reason the deadline table sits there. The slice registers no
    // `#[derive(Resource)]` and no `#[derive(Component)]`, so nothing new appears
    // in the registry this guard scans. Unlike deadlines it adds nothing to
    // `WorldScriptRuntime` either: there is no `[[commitment]]` block and so no
    // load-time declaration table to hold — a promise exists because of what a
    // player said, not because of what an author wrote down. See
    // `src/world/commitments.rs`, and
    // pasm/spec/architecture/scenario-scripting.yaml's `commitment-ledger-state`
    // entity for the `implementation.symbols` naming those Rust types.
    //
    // Issue #1035's workforce register is covered the same way again, by
    // `WorldContentRuntime` above: `WorkforceRegister` — with its
    // `WorkforceRecord`, the authored `Workforce` row it is armed from, and the
    // `WorkforceMutation`/`FlagMirror` a settlement travels as — is a FIELD on
    // that resource, sitting beside the commitments ledger for the reason the
    // ledger sits beside the deadline table. The slice registers no
    // `#[derive(Resource)]` and no `#[derive(Component)]`, so nothing new
    // appears in the registry this guard scans, and it adds nothing to
    // `WorldScriptRuntime` either: the `[[workforce]]` blocks are parsed onto
    // `WorldConfig` (authored content, not runtime state) and a settlement is
    // an ordinary `ActionCmd` on the effect buffer that already existed.
    //
    // The one place it DOES touch a scanned type is `InfrastructureState`,
    // which gained an authored, immutable `workforce` naming the side that
    // staffs a structure — a field on state `InfrastructureCondition` already
    // covers, not a registration. See `src/world/workforce.rs`, and
    // pasm/spec/architecture/scenario-scripting.yaml's `workforce-register-state`
    // entity for the `implementation.symbols` naming those Rust types.
    //
    // Issue #1030's dossiers add nothing here either, and for a stronger reason
    // than either of the two above: they add no STATE at all, anywhere. A dossier
    // is a projection folded fresh every tick out of state other subsystems
    // already own — the condition track, the commitments ledger, the comms
    // roster, the faction registry, an entity's own name — so there is nothing to
    // register, nothing to fold (folding it would fold those numbers twice) and
    // nothing for #863 to persist (a save carrying it could disagree with the
    // state it was derived from). `DossierPlugin` adds one publisher system and
    // no resource; the subject roster is DERIVED from `CommsHailable` and
    // `InfrastructureCondition` rather than authored into a component of its own,
    // which was a deliberate rejection — see the module docs in
    // `src/dossier/server.rs` — and is what leaves this list untouched. The wire
    // types (`DossierBlackboard` and friends) are plain message structs with no
    // `#[derive(Component)]`/`#[derive(Resource)]`, so like `EntitySnapshot` they
    // can never appear in the registry this guard scans. See
    // pasm/spec/architecture/world-files.yaml's `dossier-published-state` entity.
    //
    // Issue #1031's gathered evidence is the exception that sharpens the
    // paragraph above rather than contradicting it. It DOES add state — an
    // `EvidenceLog` of what the crew found out, which no fold can recover
    // because it exists only where a scenario said the crew went and did
    // something — and that state is snapshotted (`ScenarioState::evidence`,
    // `SNAPSHOT_FORMAT` 7) precisely because it is not derived. It still adds
    // nothing to this list, and for the ledger's reason: `EvidenceLog` (with its
    // `EvidenceEntry` and `EvidenceProvenance`) is a FIELD on
    // `WorldContentRuntime` above, sitting beside the commitments ledger, so the
    // slice registers no `#[derive(Resource)]` and no `#[derive(Component)]` and
    // nothing new appears in the registry this guard scans. The one script verb
    // that writes it (`ctx.dossier.append`) buffers an ordinary `ActionCmd` on
    // the existing effect sink and adds no `WorldScriptRuntime` field either:
    // there is no `[[evidence]]` block, so there is no load-time declaration
    // table to hold. See `src/dossier/evidence.rs`, and
    // pasm/spec/architecture/world-files.yaml's `dossier-evidence-state` entity
    // for the `implementation.symbols` naming those Rust types.
    //
    // Issue #1038's scan-versus-dossier diff registers nothing at all, which is
    // the whole reason it was built the way it was. It adds ONE bit — "this crew
    // have read that structure" — and that bit is written into the base-world
    // `FlagStore`, which is a FIELD on `WorldContentRuntime` above and has been
    // authoritative, snapshotted and accounted for here since long before this
    // slice. `science::scan::scanned_flag` is a free function composing the key;
    // `science::server::tick_scans` is an existing system that gained a
    // `ResMut<WorldContentRuntime>` parameter. No `#[derive(Resource)]`, no
    // `#[derive(Component)]`, no `WorldScriptRuntime` field, and no new
    // `ActionCmd`: the comparison itself lives in world script and its output is
    // an ordinary `ctx.dossier.append` onto the #1031 log two paragraphs up.
    //
    // The alternative — a `ScannedSubjects` resource, or a component on the
    // structure — was rejected for exactly the reason this list exists: it would
    // have been a second authoritative record of something the flag store can
    // already hold, needing its own classification, its own snapshot field and
    // its own fold decision, to answer a question a counter answers. See
    // `src/science/scan.rs`'s `scanned_flag` docs for the mirror argument, and
    // #1035's `FlagMirror` two paragraphs up for the precedent.
    //
    // Issue #1043's CAMPAIGN FLAG HANDOFF registers nothing either, and it is the
    // largest test of that reading so far. Six families of fact that Falling
    // Skyway hands to whatever mission comes after it — who was carried in the
    // transfer window and who was left, how the strike ended, how deep the
    // evidence went, what the casualties came to, whether the skyhook held, and
    // how many promises were kept and broken — are ordinary named counters in
    // that same base-world `FlagStore`, written by world script under a
    // `campaign.<mission>.<family>.<fact>` prefix. There is no `CampaignRecord`
    // resource and no per-mission component, for the reason the paragraph above
    // rejected `ScannedSubjects`: it would be a second authoritative copy of
    // facts the store already holds. The prefix is a naming convention over
    // existing state, not a type — see
    // pasm/spec/architecture/world-files.yaml's `campaign-flag-handoff-state`
    // for the contract it does carry (exactly one member of each exclusive
    // family, always written) and the Rust types that hold it. The slice adds no
    // Rust at all, so it moved no `SNAPSHOT_FORMAT`: the names land in a map
    // `ScenarioState` already saves.
    //
    // Issue #1214's `PreCompiledScripts` joins the same seam and for the same
    // reason. It is the boot→Startup handoff that carries the world's scripts,
    // compiled ONCE in `world::load::load` (the headless path), to
    // `compile_world_scripts` — which consumes it instead of re-parsing
    // `RawWorldSource` and compiling a second time. It is authoritative-but-
    // deferred exactly as `RawWorldSource` is: it holds the same `CompiledScripts`
    // the runtime is built from, its content is bound into the CONTENT digest via
    // the content ledger (never the authoritative fold), and it is emptied
    // (`Option::take`) the moment the runtime is built. Registered in the census
    // run because `compile_world_scripts` references it via `Option<ResMut>`
    // (headless also inserts it), so it appears here for the same reason
    // `RawWorldSource` does.
    // Issue #1181's `BridgeWorldSource` joins the same seam. It is the wasm
    // bridge's hand-off of the loaded world's raw `(path, TOML)` into the World
    // — the de-globalised replacement for the `get_raw_world_source()` free
    // function `insert_raw_world_source_resource` used to read. It registers here
    // because that `Startup` system now references it via `Option<Res>` on both
    // targets (only the browser ever inserts it), exactly as `RawWorldSource`
    // below registers via `compile_world_scripts`'s `Option<Res>` while only the
    // browser/duel paths insert it. Authoritative-but-deferred for the same
    // reason: it carries world content bound into the CONTENT digest (never the
    // authoritative fold) and is consumed at `Startup` to build `RawWorldSource`.
    //
    // It moved from `crate::server::bridge` to `crate::world::server` beside that
    // consumer in issue #1194 (the sim→presentation boundary lift), so its full
    // type-path key is the `world::server::` one below, not the old `server::`
    // bridge one.
    //
    // Issue #1045's SCRIPT-IN-LAYERS registers nothing new, and it is the seam's
    // most invasive slice so far. A layer's `[script]` block now compiles at
    // `LoadWorld` and merges into the SAME `WorldScriptRuntime` this list already
    // covers — its ASTs, its `handlers` vec, its `deadline_handlers` — and its
    // trigger states into `WorldContentRuntime::trigger_states` above. The one new
    // field is `WorldRuntime::script_units`, the AST keys a layer added so its
    // unload can retract exactly them; `WorldRuntime` carries no
    // `#[derive(Resource)]` of its own — it lives inside `WorldLayerMap`, already
    // covered — so nothing new appears in the registry this guard scans. The empty
    // `WorldScriptRuntime` a scripted layer inserts when the base world authored
    // none is that same registered type, instantiated where a script-free world
    // previously had nothing. See `src/world/layers.rs` and the parallel-vec
    // invariant documented on `WorldScriptRuntime::handlers`.
    "project_phoenix::world::server::BridgeWorldSource",
    "project_phoenix::world::server::PreCompiledScripts",
    "project_phoenix::world::server::RawWorldSource",
    "project_phoenix::world::server::WorldScriptRuntime",
];

fn build_and_run() -> App {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("app should build");
    run(&mut app, args.max_ticks);
    app
}

/// The crate's own module-path prefix, matching `Cargo.toml`'s package name.
/// Filters out every Bevy-internal and third-party type — `Time<Fixed>`,
/// `Transform`, `RapierContext`, and friends — none of which this issue's
/// boundary is about; only what THIS crate defines and the sim app actually
/// registers is in scope.
const CRATE_PREFIX: &str = "project_phoenix::";

/// The short type name Bevy's `type_name::<T>()`-derived `DebugName` reports
/// (e.g. `project_phoenix::ship::state::ShipRedAlert`), stripped to its last
/// path segment and truncated before any generic parameter list so
/// `Foo<Bar>` and `Foo` share a short name.
///
/// # No longer the census key
///
/// This USED to be the census key — the string every registered type was
/// reduced to and every list compared against. That was wrong for exactly the
/// reason the truncation exists: two distinct generic instantiations
/// (`EffectQueue<A>` and `EffectQueue<B>`, or `BroadcastRegistry<Sim>` and a
/// hypothetical `BroadcastRegistry<Other>`) collapse to one key and one can
/// silently hide the other from the unclassified ratchet. The census key is now
/// the FULL path ([`registered_crate_local_type_names`]); this short name
/// survives only as the lookup into the two census-derived superset sets — the
/// authoritative set ([`census_authoritative_short_names`]) and the exclusion set
/// ([`census_excluded_short_names`]) — both consulted by short name, each reduced
/// to short names from the census's full-path keys. Only [`UNCLASSIFIED_BASELINE`]
/// — the exhaustive remainder
/// that must distinguish collapsing generics — keys on the full path.
fn short_name(full: &str) -> String {
    // Strip the OUTER generic parameter list FIRST, by truncating at the
    // first '<' in the whole path — not the last "::" segment. A naive
    // rsplit("::").next() on a generic like
    // `project_phoenix::core::broadcast::broadcaster::BroadcastRegistry<project_phoenix::core::broadcast::sim::Sim>`
    // splits on every "::" INSIDE the generic parameter too, yielding the
    // nonsense fragment `Sim>` instead of `BroadcastRegistry`.
    let without_generics = match full.find('<') {
        Some(idx) => &full[..idx],
        None => full,
    };
    without_generics
        .rsplit("::")
        .next()
        .unwrap_or(without_generics)
        .to_string()
}

/// Every crate-local component/resource name Bevy actually registered, as its
/// FULL type path (the census key), de-duplicated and sorted for a stable diff.
///
/// The full path — not [`short_name`] — is the key precisely so two distinct
/// generic instantiations do not collapse to one entry and hide each other from
/// the unclassified ratchet. The two superset lists are still consulted by short
/// name (see [`short_name`]); only this key and [`UNCLASSIFIED_BASELINE`] are
/// full paths.
fn registered_crate_local_type_names(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app
        .world()
        .components()
        .iter_registered()
        .map(|info| info.name().to_string())
        .filter(|full| full.starts_with(CRATE_PREFIX))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Issue #894's enumeration AC, made concrete: every registered crate-local
/// type maps to exactly one of the fold set, an explicit reasoned exclusion,
/// or the committed unclassified baseline — never to nothing.
#[test]
fn every_registered_type_maps_to_the_digest_record() {
    let app = build_and_run();
    let registered = registered_crate_local_type_names(&app);
    assert!(
        !registered.is_empty(),
        "precondition: the sim app registered no crate-local component or \
         resource at all — CRATE_PREFIX or the registry lookup is wrong"
    );

    // "List 1": the authoritative (folded / deferred-fold) set, now derived from
    // the declaration registry (`StateCensus`) the built app populated, not a
    // local `AUTHORITATIVE_SYMBOLS` const (issue #1222). Consulted by SHORT name,
    // as it always was.
    let authoritative: std::collections::BTreeSet<String> = census_authoritative_short_names(&app);
    // "List 2": the exclusion set, likewise derived from the declaration registry
    // (`StateCensus`), not a local `EXCLUSIONS` const (issue #1221). Still
    // consulted by SHORT name, as it always was.
    let excluded: std::collections::BTreeSet<String> = census_excluded_short_names(&app);
    let baseline: std::collections::BTreeSet<&str> =
        UNCLASSIFIED_BASELINE.iter().copied().collect();

    let mut newly_unclassified: Vec<&str> = registered
        .iter()
        .map(String::as_str)
        .filter(|full| {
            // The two superset lists are consulted by SHORT name (transcribed
            // from PASM as short names); the exhaustive baseline is consulted by
            // the FULL path, so collapsing generics can never hide one another.
            let short = short_name(full);
            !authoritative.contains(short.as_str())
                && !excluded.contains(short.as_str())
                && !baseline.contains(*full)
        })
        .collect();
    newly_unclassified.sort();
    newly_unclassified.dedup();

    assert!(
        newly_unclassified.is_empty(),
        "{} crate-local type(s) registered by the sim app are UNCLASSIFIED by \
         the #894 digest-boundary record: {newly_unclassified:?}\n\
         Classify each one by declaring it at its OWNING plugin's `build()` via \
         `app.declare_state::<T>(class, pasm)` (issue #1220's registry):\n\
         \x20 - if it is authoritative simulation state, declare it \
         `StateClass::Folded` when `src/sim_digest.rs` folds it or \
         `StateClass::DeferredFold` when it does not yet — the census feeds this \
         authoritative set — and add its name to the owning PASM \
         `classification: authoritative` state entity's `implementation.symbols` \
         under pasm/spec/architecture/*.yaml;\n\
         \x20 - otherwise declare it with the right exclusion StateClass \
         (Presentation/Cache/Timer/Derived/ClearedAtFold/TestInfra) — the census \
         feeds the exclusion set — and if the reason class is new, record it in \
         pasm/spec/architecture/deterministic-simulation.yaml's \
         digest-exclusion-classes entity too.\n\
         See pasm/spec/architecture/deterministic-simulation.yaml for the fold \
         policy this guard enforces.",
        newly_unclassified.len()
    );
}

/// The ratchet direction: the committed baseline may only ever describe types
/// the sim app actually still registers unclassified. A stale entry (the type
/// was renamed, removed, or got classified some OTHER way without this file
/// being updated) means the baseline is over-claiming coverage it no longer
/// has — the opposite failure from the guard above, and just as silent if
/// unchecked.
#[test]
fn the_committed_baseline_names_only_types_still_registered_and_unclassified() {
    let app = build_and_run();
    let registered: std::collections::BTreeSet<String> = registered_crate_local_type_names(&app)
        .into_iter()
        .collect();
    // Both superset lists are now derived from the declaration registry
    // (`StateCensus`): the authoritative set (issue #1222) and the exclusion set
    // (issue #1221) — same short-name "is it now classified elsewhere" semantics
    // as when both were hand-maintained consts.
    let authoritative: std::collections::BTreeSet<String> = census_authoritative_short_names(&app);
    let excluded: std::collections::BTreeSet<String> = census_excluded_short_names(&app);

    let mut stale: Vec<&str> = UNCLASSIFIED_BASELINE
        .iter()
        .copied()
        .filter(|full| {
            // Baseline entries are FULL paths now, so `registered` (also full
            // paths) is checked directly; the two short-name superset lists are
            // checked against the entry's short name, preserving the exact
            // "is it now classified elsewhere" semantics.
            let short = short_name(full);
            !registered.contains(*full)
                || authoritative.contains(short.as_str())
                || excluded.contains(short.as_str())
        })
        .collect();
    stale.sort();

    assert!(
        stale.is_empty(),
        "UNCLASSIFIED_BASELINE names type(s) that are no longer registered, or \
         are now classified elsewhere in this file, and should be removed: \
         {stale:?}. Trim UNCLASSIFIED_BASELINE — this is the shrink half of \
         the ratchet, and it is always safe."
    );
}

/// AC5's own reviewer test, executable rather than only readable: every
/// settled in/out call from the #894 HITL thread, checked against the record
/// this file enforces.
#[test]
fn ac5_reviewer_answers_match_the_pasm_record() {
    // Read the answers back out of the census the built app populates (issue
    // #1222), not a hand-maintained const: the IN calls must be declared
    // `Folded`/`DeferredFold` at their owning sites, the OUT calls must be absent
    // from the authoritative set entirely.
    let app = build_and_run();
    let authoritative: std::collections::BTreeSet<String> = census_authoritative_short_names(&app);
    for must_be_in in [
        "SimRng",
        "GamePhase",
        "GameOverReason",
        "WorldResource",
        "CaptainPriorityBoost",
        "ShipPhysics",
    ] {
        assert!(
            authoritative.contains(must_be_in),
            "{must_be_in} must be declared authoritative (`Folded`/`DeferredFold`) \
             into StateCensus — the #894 HITL thread settled this as an explicit \
             IN call, quoted verbatim in \
             pasm/spec/architecture/deterministic-simulation.yaml \
             (digest-boundary-reviewer-answers)."
        );
    }
    // RenderInterp OUT: it must never be declared authoritative. It is declared
    // `Presentation` at `RendererPlugin::build`, a plugin the headless run never
    // adds, so it never registers here (see `census_excluded_short_names` above),
    // and the only check possible ahead of it being built is this negative one.
    assert!(
        !authoritative.contains("RenderInterp"),
        "RenderInterp is the #894 HITL thread's explicit OUT call — it folds \
         frame-time-interpolated presentation data and must never be declared \
         authoritative. See digest-render-interp-fold-point in \
         pasm/spec/architecture/deterministic-simulation.yaml."
    );
    // EntitySnapshot rejected as a shortcut: it must never be treated as the
    // digest boundary.
    assert!(
        !authoritative.contains("EntitySnapshot"),
        "EntitySnapshot (src/core/messages.rs) is the #894 HITL thread's \
         rejected shortcut — it carries authored presentation fields \
         (radar_icon, region_colour, colour, radar_size) and must never stand \
         in for the digest boundary. See digest-boundary-reviewer-answers in \
         pasm/spec/architecture/deterministic-simulation.yaml."
    );
}
