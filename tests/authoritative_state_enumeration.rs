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
//! 1. [`AUTHORITATIVE_SYMBOLS`] — transcribed, not invented, from the
//!    `implementation.symbols` lists this issue back-filled onto PASM. Every
//!    name here is literally readable out of a `pasm/spec/architecture/*.yaml`
//!    file today.
//! 2. [`EXCLUSIONS`] — real, legitimately-registered non-authoritative state,
//!    with the reason class the PASM record
//!    (`deterministic-simulation.yaml`'s `digest-exclusion-classes` entity)
//!    uses: `presentation` / `cache` / `timer` / `derived` / `test-infra`.
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
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};

/// `rng_coverage.toml` (issue #837), same as
/// `tests/registration_order_determinism.rs`: two NPCs in weapons range, an
/// asteroid field, a radiation zone — beam, blaster, torpedo, collision and
/// region damage all fire inside the window below, which is what registers
/// the widest realistic set of components/resources a single run can reach.
const WORLD: &str = "assets/worlds/rng_coverage.toml";
const TICKS: u64 = 300;
const SEED: u64 = 20260894;

/// Every Rust type name that appears in `implementation.symbols` on a
/// `classification: authoritative` `state` entity, across every file in
/// `pasm/spec/architecture/*.yaml`, as of this issue's symbol-backfill pass.
///
/// Transcribed, not curated: every entry here is one this guard's own author
/// could point at inside a committed `.yaml` file. A handful of PASM entries
/// could not honestly name a dedicated Rust TYPE (a `thread_local!` pair, a
/// field on a larger resource, a value composed fresh on every read rather
/// than stored) — see the inline comment next to that entity's `symbols:` key
/// in its `.yaml` file for which and why. Those entries' function/accessor
/// names are still listed here for traceability; they simply never match a
/// registered Bevy component or resource, which is harmless — this list is a
/// superset check, not an exact-membership one.
///
/// This is the FULL union across all 73 entities (the 21 this issue
/// back-filled plus the 52 that already had `implementation.symbols` before
/// it) — regenerated mechanically from the `.yaml` files themselves, not
/// hand-curated, so it cannot silently drift from what PASM actually says.
/// `WorldResource` (the AC5 "IN" call) has no dedicated PASM `state` entity of
/// its own as of this issue — it is named directly in
/// `digest-boundary-reviewer-answers` instead — so it is listed separately
/// below rather than mixed into this transcription.
#[rustfmt::skip]
const AUTHORITATIVE_SYMBOLS: &[&str] = &[
    "ActiveBeam", "ActiveDialogue", "ActiveStationRatings", "AiDirective", "AiHistory",
    "AiPolicyMemory", "AiPolicyRuntimeState", "AiWorldEntity", "AssetPreloadResource",
    "AsteroidData", "AsteroidEntityMap", "AsteroidWindow", "BlasterVolleyState", "BoostCommand",
    "BoostState", "CaptainPriorityBoost", "CivilianConfig", "CivilianSection", "CivilianState",
    "CivilianTraffic", "CommsHailable", "CommsInbox", "CommsInboxRes", "CommsRange",
    "CommsRuntime", "ControlSourceResolver", "CoordinationLagQueue", "CoordinationQueue",
    "CurrentPhaserMode", "DamageRecord", "DesiredMotion", "EntityConfig",
    "EntityId", "EntityName", "EntityOriginLayer", "EntityUuid", "FactionConfig",
    "FactionRegistry", "FieldOrigin", "FlagStore", "GameOverReason", "GamePhase", "GodMode",
    "HazardAssessment", "HazardAssessmentRaw", "HazardContribution", "HelmAiShipFrame",
    "HelmAiSurfacesFrame", "HelmBoostAiPolicy", "HelmCapabilityConfig", "HelmCapabilitySection",
    "HelmImpulseAiPolicy", "INTENT_NARRATION_SPAWN_SITES", "ImpulseCommand", "ImpulsePhase",
    "ImpulseState", "InfrastructureCondition", "InfrastructureState",
    "LastHelmInput", "LateralThrustInput", "LodBubble", "Manifest", "MergeStep",
    "ModelMarkers", "ModelRig", "NavigationWaypoint", "ObjectiveManager",
    "OperationHold", "OperationsSaveState", "ShipOperations", "ProgressRate",
    // Issue #1027. `ResolvedCapacity` is the live half of a structure's named
    // capacity (a level and a ceiling, resolved at load the way a threshold is)
    // and `CapacityAdjustment` is one queued move of it — both transcribed from
    // `infrastructure-condition-state` and `infrastructure-condition-tracker`.
    //
    // This slice registered NO new Bevy component or resource, which is why the
    // guard's own computed set is unchanged: the four verbs are authored fields
    // on a component #1026 already registered, and the tow deliberately does
    // NOT carry an "under tow" marker — the rig is derived from the live hold
    // every tick, so there was nothing to register and nothing extra to
    // classify.
    "ResolvedCapacity", "CapacityAdjustment",
    // Issue #1032, the science scan. `ShipScanRecord` is the one new Bevy
    // COMPONENT — the per-ship survey suite and the last reading it took — and
    // the reading is the part no fold can recover: it is what the crew saw when
    // they looked, at the fidelity that moment bought them, and the structure
    // has moved on since (#1031's evidence log is stored for the same reason).
    // `ScanReading`, `ScanRefusal` and `ScanSaveState` are the value types
    // behind it, listed for traceability the way `OperationHold` is; `ScanConfig`
    // is the authored ladder, which is content re-derived at spawn rather than
    // saved. Transcribed from `science-scan-state` in
    // pasm/spec/architecture/world-files.yaml.
    "ShipScanRecord", "ScanReading", "ScanRefusal", "ScanSaveState", "ScanConfig",
    "PendingArcBearingRequest", "PendingWorldLayerChanges", "PhaserCooldown",
    "Player", "PowerBlackboard", "Provenance", "QualifiedRef", "RecentCombatActivity",
    "RegionEffectKind", "RepairBlackboard", "RepairTeams", "ResolvedTemplate", "ScenarioCatalog",
    "ScenarioCatalogWire", "ScoredObjective", "SensorRadarSelection", "SessionManager",
    "Severity", "ShieldSystem", "ShieldsDamageHistory", "ShipBoost", "ShipConfig", "ShipImpulse",
    "ShipIntentNarration", "ShipModifiers", "ShipPhysics", "ShipRedAlert",
    // Issue #1041's tactical restraint lever. Authoritative and FOLDED, in its
    // own `fold_weapons_hold_namespace` — transcribed from the
    // `authoritative-weapons-hold-state` entity in
    // pasm/spec/architecture/red-alert.yaml.
    "ShipWeaponsHold",
    // Issue #863's spawn provenance. `EntitySpawnOrigin` is a new Bevy
    // COMPONENT — what a mid-run scripted spawn was made from, so a resume can
    // rebuild the ship no fresh boot re-derives — and `SpawnOrigin` is the
    // Bevy-free record it wraps. Authoritative and NOT folded, like the deadline
    // table and the commitments ledger beside it.
    //
    // The guard's own computed set does NOT move for this entry, and the reason
    // is worth writing down rather than leaving as a coincidence: the component
    // is registered the first time a world runs a scripted `spawn_entity`, and
    // this guard's world (`rng_coverage.toml`) authors none. So the entry is a
    // FORWARD declaration — transcribed from the `runtime-spawn-origin-state`
    // entity in pasm/spec/architecture/trigger-pipeline.yaml — rather than a
    // response to a failure, and the day a spawning world becomes the guard's
    // world it is already accounted for.
    "EntitySpawnOrigin", "SpawnOrigin",
    "SimRng",
    "SimRngState", "SimulationPaused", "SourceLocation", "StationConfig", "SteeringInput",
    "SystemBlackboard",
    "StaticPointDefence", "SystemHull", "TacticalRadarSelection", "TeamSlot", "ThrustInput", "Torpedo",
    "TorpedoDetonation", "TorpedoSystem", "TorpedoTube", "TubeBurstState", "VerticalMovementMode",
    "ViewscreenArbiter", "ViewscreenResolution", "WaypointMode", "WeaponsDoctrineAiPolicy",
    "WorldConfig", "WorldEntity",
    "WorldEventBuffer", "WorldFinding", "WorldLayerChange", "WorldLayerMap", "WorldRuntime",
    "WorldSnapshot", "WorldSource", "WorldView",
    // Issue #907's tick-scoped id mint. Authoritative, and FOLDED: its
    // per-namespace counters are in the digest's run-scope preamble for the
    // same reason `SimRng`'s stream positions are, so a divergent spawn count
    // is caught on the tick it happens. Backed by the `world-id-mint-state`
    // entity in deterministic-simulation.yaml.
    "WorldIdMint", "WorldId", "IdNamespace",
    // Function/accessor names transcribed for traceability even though they
    // never match a registered component/resource (see the doc comment
    // above) — harmless in a superset check.
    "apply_arc_bearing_request", "apply_world_layer_changes", "assess_hazards",
    "assign_named_entity_uuids", "build_catalog", "build_helm_ai_surfaces_frame",
    "clear_mod_pack_overlay", "encode_local_facing", "encode_local_velocity", "is_instagib",
    "mod_pack_overlay_get", "on_site_systems", "seed_helm_actuator_facts",
    "push_mod_pack", "remove_mod_pack", "reorder_mod_packs", "active_packs",
    "spawn_immediate_entities_internal", "sync_ship_position",
    "tick_boost", "tick_impulse", "visible_entities", "wasm_load_world", "wasm_select_ship",
    // Named directly in `digest-boundary-reviewer-answers` (AC5's verbatim
    // "IN" calls) rather than backed by its own PASM `state` entity's
    // `implementation.symbols` as of this issue.
    "WorldResource",
    // Named in `digest-fold-order-policy` (the namespace-sequence rule) as
    // the second minted-id namespace, sibling to `EntityUuid` above.
    "AsteroidUuid",
    // The tick counter itself — the digest-exclusion-classes rationale
    // already depends on this being in the fold ("the tick counter is
    // already in the fold via SimTick") to justify excluding the AI cadence
    // latches as pure functions of it; stated as an explicit member here too.
    "SimTick",
];

/// Real, legitimately-registered non-authoritative state, with the reason
/// class `deterministic-simulation.yaml`'s `digest-exclusion-classes` entity
/// records: `presentation` | `cache` | `timer` | `derived` | `test-infra`.
///
/// `EntitySnapshot` (`src/core/messages.rs`) is deliberately ABSENT from both
/// this list and [`AUTHORITATIVE_SYMBOLS`] rather than excluded here: it
/// carries no `#[derive(Component)]`/`#[derive(Resource)]` at all (a plain
/// wire-message struct), so it can never appear in the registry this guard
/// scans. Its rejection as a digest-boundary shortcut is recorded in
/// `deterministic-simulation.yaml` (`digest-boundary-reviewer-answers`) for a
/// reviewer reading the PASM record, not re-proven here.
#[rustfmt::skip]
const EXCLUSIONS: &[(&str, &str)] = &[
    // Presentation — the local hull's frame-time interpolation lives directly
    // on the simulation entity but never feeds authoritative fixed-tick state.
    ("RenderInterp", "presentation"),
    //
    // The debug overlay flags (`src/debug_overlay.rs`) are presentation in the
    // strictest sense: each one decides whether a wireframe, a log or an
    // inspector is DRAWN, and nothing reads them to decide anything else. They
    // reach the registry as of issue #940 because the phone client can now flip
    // them, so the systems that do it name the resources even in a headless run
    // that has no `DebugOverlayPlugin` to insert them.
    //
    // Their sibling `SimulationPaused` is deliberately NOT here — it is in
    // AUTHORITATIVE_SYMBOLS, because pausing stops the clock the whole
    // simulation advances on. The line between the two is exactly "does it
    // change what the sim computes".
    ("DebugRegionsEnabled", "presentation"),
    ("DebugOverlayEnabled", "presentation"),
    ("DebugDamageEnabled", "presentation"),
    ("DebugEntitiesEnabled", "presentation"),
    ("DebugEntityInspectorEnabled", "presentation"),

    // Broadcast caches — one-directional delta-suppression mirrors of
    // already-authoritative state, not a second copy of simulation truth.
    // (`src/core/broadcast/cache_registry.rs` — only the first two carry an
    // `Entity` segment in their real name; `LastBroadcastHull`,
    // `LastBroadcastShields` and `LastBroadcastBlackboards` do not.)
    ("LastBroadcastEntityPositions", "cache"),
    ("LastBroadcastEntityHealth", "cache"),
    ("LastBroadcastHull", "cache"),
    ("LastBroadcastShields", "cache"),
    ("LastBroadcastBlackboards", "cache"),
    // Same family for the phone client's debug/session read-back (issue #940):
    // `debug_overlay::report_debug_state` compares against it to skip
    // re-announcing flags nothing moved. A mirror of resources that are
    // themselves presentation, so it can never be a second copy of sim truth.
    ("LastReportedDebugState", "cache"),
    // Same family, a different console (`src/console/weapons/blackboard.rs`):
    // "The broadcaster compares against this to skip identical ticks."
    ("LastWeaponsUpdate", "cache"),
    // The broadcaster's own per-phase live delivery registry
    // (`src/core/broadcast/broadcaster.rs`) — transport bookkeeping, not a
    // second copy of simulation truth.
    ("BroadcastRegistry", "cache"),

    // Timers / outboxes.
    ("SimBroadcastTimer", "timer"),
    ("SimOutbox", "timer"),
    ("DamageLog", "timer"),

    // Derived — AI cadence latches, pure functions of SimTick (already in
    // the fold via SimRng/GamePhase's own tick-scoped siblings).
    ("AiTickReady", "derived"),
    ("AiSnapshotReady", "derived"),
    // Derived — the shared AI base-cadence interval `tick_ai_cadence` writes
    // alongside the two latches above, same tick-derivation, same family.
    ("AiBaseInterval", "derived"),
    // Derived — human-seeking host routing (issue #984 S7): recomputed every
    // tick as a pure function of ShipConfig + Sessions + control sources, none
    // of which is digest-folded; spawn-required on LocalShip so it never causes
    // a mid-run archetype move (the S7 digest-regression fix).
    ("HumanSeekingHosts", "derived"),

    // Cleared-at-fold (the one new classification term this issue adds,
    // deterministic-simulation.yaml's `digest-exclusion-classes`):
    // structurally empty by the RenderInterp fold point on every
    // correctly-running instance.
    ("InterSystemQueue", "cleared-at-fold"),
];

/// The honest remainder: every crate-local component/resource the sim app
/// registers that neither [`AUTHORITATIVE_SYMBOLS`] nor [`EXCLUSIONS`]
/// reaches yet, computed from a real run and committed here as a ratchet.
///
/// This is NOT a claim that any of these 171-components/134-resources-worth
/// of remaining state SHOULD stay out of the digest forever — issue #894's
/// own scope note says classifying all of it is bigger than this issue's
/// acceptance criteria. It is the worklist: shrinking this list means moving
/// an entry into `AUTHORITATIVE_SYMBOLS` (with a PASM `implementation.symbols`
/// entry to match) or into `EXCLUSIONS` (with a reason). Growing it silently
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
    "AdmittedCommands", "AdmittedConsumerRegistry", "AiHighFidelity",
    "AiPolicyTickClock", "AiProfile", "AiTokenRegistry", "Asteroid", "AsteroidFieldSection",
    "AsteroidShieldPierce", "BankConfigResource", "BeamContext", "BehaviourSection",
    "BlasterBankAiPolicies", "BlasterSystemResource", "BoostConfigResource", "CaptainAiPolicy",
    "CinematicCameraSection", "ColliderSection", "CollisionCooldown", "CommandDelay",
    "CommandLog", "CommsResponseAiCadence", "CommsResponseAiPolicy", "CommsTargetSelector",
    "CountdownTimer", "DockingMotionIntent", "EntityShipArcHull", "EntitySystemHull",
    "EntityTagsSection", "EntityTarget", "FactionComponent", "FactionRegistryResource",
    "GameStateCache", "HelmBoostAiPolicyState", "HelmConsoleSection", "HelmEnginesAiPolicy",
    "HelmEnginesAiPolicyState", "HelmLateralAiPolicy", "HelmMotionPlan", "HelmPassSurface",
    "HelmPhysicsFrame", "HelmPhysicsWriteGuard", "HelmRecoveryHistory", "HelmSteeringAiPolicy",
    "HelmSteeringAiPolicyState", "HelmVerticalAiPolicy", "HelmWaypointClearance",
    "ImpulseConfigResource", "LastShipAttacker", "LastSystemTiers", "LastVisibleRepairBlackboard",
    "LobbyOutbox", "LocalShip", "LodTransitionTimer", "LogFilterConfig", "MeshSection",
    "NavClearanceIssueState", "NavigationTargetSelector", "NpcFrequencyMatchStates",
    "ObjectiveCursors", "ObjectiveManagerRes", "OnScreenMessage", "PendingCommands",
    "PendingScenarioLoad", "PendingShieldsThreatBearing", "PendingShipConfig",
    "PendingTacticalFrequencyHint", "PhaserBankAiPolicies", "PhaserCombatConfigResource",
    "PhaserRenderConfig", "PowerAiCadence", "PowerAiPolicy", "PowerBrownoutState",
    "PowerConfigResource", "PowerMultiplierResource", "RadarAppearanceSection",
    "RegionEffectsSection", "RegionMembership", "RegionShapeSection", "RepairHumanAlerted",
    "RepairRequestQueue", "RepairTargetSelector", "RunTelemetry", "ScenariosBeingUnloaded",
    "SelectedShipResource", "SensorsAiConfigResource", "SensorsFrequencyState",
    "SensorsTargetSelector", "SensorsThreatState", "Sessions", "ShakeState",
    "ShieldsAiConfigResource", "ShieldsCoordinationState", "ShieldsFocusAiPolicy", "Ship",
    "ShipAttackedThisTick", "ShipAudioSection", "ShipClientConfigResource", "ShipConfigComponent",
    "ShipFrequencyHintState", "ShipManualResource", "ShipPhaserFrequency",
    "ShipPhysicsConfigResource", "ShipPowerSystem", "ShipRepairTeams", "ShipShields",
    "ShipStations", "ShipSystemBlackboards", "ShipSystemControlSources", "ShipViewMode",
    "TacticalTargetSelector", "TorpedoMagazineAiPolicy", "TorpedoSystemResource",
    "TorpedoTargetSnapshot", "TorpedoTubeAiPolicies", "TrackedEntities", "VerticalThrustInput",
    "WeaponFiredThisTick", "WeaponsArcRequestState", "WeaponsConsoleSection",
    "WeaponsUpdateFirstTick", "WorldContentRuntime", "WorldSetupBroadcast",
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
    "RawWorldSource", "WorldScriptRuntime",
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
/// `Foo<Bar>` and `Foo` are the same lookup key.
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

/// Every crate-local component/resource name Bevy actually registered,
/// short-named and de-duplicated, sorted for a stable diff.
fn registered_crate_local_type_names(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app
        .world()
        .components()
        .iter_registered()
        .map(|info| info.name().to_string())
        .filter(|full| full.starts_with(CRATE_PREFIX))
        .map(|full| short_name(&full))
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

    let authoritative: std::collections::BTreeSet<&str> =
        AUTHORITATIVE_SYMBOLS.iter().copied().collect();
    let excluded: std::collections::BTreeSet<&str> =
        EXCLUSIONS.iter().map(|(name, _)| *name).collect();
    let baseline: std::collections::BTreeSet<&str> =
        UNCLASSIFIED_BASELINE.iter().copied().collect();

    let mut newly_unclassified: Vec<&str> = registered
        .iter()
        .map(String::as_str)
        .filter(|name| {
            !authoritative.contains(name) && !excluded.contains(name) && !baseline.contains(name)
        })
        .collect();
    newly_unclassified.sort();
    newly_unclassified.dedup();

    assert!(
        newly_unclassified.is_empty(),
        "{} crate-local type(s) registered by the sim app are UNCLASSIFIED by \
         the #894 digest-boundary record: {newly_unclassified:?}\n\
         Classify each one in tests/authoritative_state_enumeration.rs:\n\
         \x20 - if it is authoritative simulation state, add it to a PASM \
         `classification: authoritative` state entity's `implementation.symbols` \
         under pasm/spec/architecture/*.yaml, then add its name to \
         AUTHORITATIVE_SYMBOLS here;\n\
         \x20 - otherwise add it to EXCLUSIONS here with a reason class \
         (presentation/cache/timer/derived/cleared-at-fold/test-infra), and if \
         the reason is new, record it in \
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
    let authoritative: std::collections::BTreeSet<&str> =
        AUTHORITATIVE_SYMBOLS.iter().copied().collect();
    let excluded: std::collections::BTreeSet<&str> =
        EXCLUSIONS.iter().map(|(name, _)| *name).collect();

    let mut stale: Vec<&str> = UNCLASSIFIED_BASELINE
        .iter()
        .copied()
        .filter(|name| {
            !registered.contains(*name) || authoritative.contains(name) || excluded.contains(name)
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
    let authoritative: std::collections::BTreeSet<&str> =
        AUTHORITATIVE_SYMBOLS.iter().copied().collect();
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
            "{must_be_in} must be in AUTHORITATIVE_SYMBOLS — the #894 HITL \
             thread settled this as an explicit IN call, quoted verbatim in \
             pasm/spec/architecture/deterministic-simulation.yaml \
             (digest-boundary-reviewer-answers)."
        );
    }
    // RenderInterp OUT: it must never be added to AUTHORITATIVE_SYMBOLS. It
    // is not registered as a type today (see the EXCLUSIONS doc comment
    // above), so the only check possible ahead of it being built is this
    // negative one.
    assert!(
        !authoritative.contains("RenderInterp"),
        "RenderInterp is the #894 HITL thread's explicit OUT call — it folds \
         frame-time-interpolated presentation data and must never enter \
         AUTHORITATIVE_SYMBOLS. See digest-render-interp-fold-point in \
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
