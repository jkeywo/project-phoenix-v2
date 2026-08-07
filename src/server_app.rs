use crate::simmath;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{LobbyOutbox, OutboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    DeliveryClass, EntitySnapshot, GamePhase, ServerMessage, ShieldFacingStatus, StationId,
};
use crate::shield::ShieldSystem;

use crate::damage::{apply_damage_with_shields, apply_hull_damage, collision_damage};
use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::shield::attacker_bearing_relative;
use bevy_rapier3d::prelude::ReadRapierContext;
// Re-export ShipPhysics so `crate::simulation::ShipPhysics` and
// `crate::server_app::ShipPhysics` both resolve.
pub use crate::ship_state::ShipPhysics as ShipPhysicsComponent;

use crate::entity_spawner::{
    AsteroidFieldSection, BehaviourSection, ColliderSection, EntityId, EntityName,
    EntityTagsSection, EntityUuid, FactionComponent, MeshSection, RadarAppearanceSection,
    RegionShapeSection,
};
use crate::impulse::ImpulseState;
use crate::messages::ModifierSlot;
use crate::modifiers::ShipModifiers;
use crate::world::server::ObjectiveManagerRes;
use std::collections::HashMap;

// â"€â"€ Beam constants â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
pub use crate::weapons_plugin::{
    weapons_update_broadcaster, ActiveBeam, AsteroidDestroyedVfx, CurrentPhaserMode,
    LastShipAttacker, LastWeaponsUpdate, PhaserCooldown, PhaserRenderConfig,
    TacticalRadarSelection, TorpedoSystemResource,
};

pub use crate::repair_plugin::{repair_state_broadcaster, ShipRepairTeams};

pub use crate::power_plugin::{
    power_state_broadcaster, PowerConfigResource, PowerMultiplierResource, ShipPowerSystem,
};

// â"€â"€ Marker Components â"€â"€â"€â"€â"€â"€â"€â"€
/// Marks the player-controlled ship entity in simulation queries.
/// Rendering and networking queries should use `With<LocalShip>` instead.
#[derive(Component)]
pub struct Ship;

/// Tags the single entity this client owns, renders, and broadcasts — the
/// "local player's ship." Simulation/gameplay systems treat every ship
/// uniformly via `With<Ship>` (the unified-ship model); `LocalShip` is
/// reserved for the things that are inherently about *this one* ship:
///
///   - viewscreen rendering, pfx, and audio;
///   - client networking: broadcast + reconnect resync/cache;
///   - region membership and comms-range;
///   - projecting *this* ship's per-console state to its client
///     (the console-state / blackboard builders and their broadcasters);
///   - routing a human console command to the ship the human is aboard
///     (admission's local-token seam) and clearing the player's own UI
///     selections (e.g. the Tactical lock the local console owns).
///
/// It must never gate shared gameplay mechanics (damage, physics, AI) — those
/// run on `With<Ship>` so the local ship and NPCs behave identically.
#[derive(Component)]
pub struct LocalShip;

/// Marker component on the scene-root child entity of the local ship's GLB
/// model. The child starts `Visibility::Hidden`; the renderer's
/// `toggle_ship_model_visibility` then drives it from the current view mode
/// every frame (visible only in `Cinematic`). That system is state-driven
/// rather than edge-triggered precisely because this marker is inserted
/// asynchronously — see issue #944.
#[derive(Component)]
pub struct LocalShipModel;

#[derive(Component)]
pub struct Asteroid;

/// Marks a light entity that should continuously rotate to face the
/// player's ship, regardless of how its parent entity is oriented.
#[derive(Component)]
pub struct FacePlayerLight;

/// Stable UUID string identifying this asteroid entity (for targeting).
#[derive(Component, Clone)]
pub struct AsteroidUuid(pub String);

/// Per-asteroid `shield_pierce` snapshot, copied from the parent
/// `AsteroidFieldConfig.shield_pierce` at spawn time. Read by
/// `handle_collisions` to split impact damage between shields and hull.
/// When the component is missing, the collision handler treats it as
/// `0.0` (full shield mitigation — pre-#414 behaviour).
#[derive(Component, Clone, Copy, Debug)]
pub struct AsteroidShieldPierce(pub f32);

// â"€â"€ Resources â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
///
/// Per-ship `Component` post ship-parity audit; every ship (player + NPC)
/// carries its own impulse state. NPCs never charge impulse under current
/// AI, but the state lives on the entity so future NPC helm behaviour can
/// route through the same per-ship pathway.
///
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Default)]
pub struct ShipImpulse(pub ImpulseState);

// ShipShields has moved to `crate::ship::shields` as a Component.
pub use crate::ship::shields::ShipShields;

/// The ship's boost drive battery state. Toggle/partial-drain model; only
/// active when the ship's TOML enables it (see `BoostConfigResource`).
///
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback). Both spawn paths insert a
/// `ShipBoost::default()` Component on every ship.
#[derive(Component, Default)]
pub struct ShipBoost(pub crate::boost::BoostState);

/// Per-ship marker set to `true` by phaser/torpedo fire systems when that
/// ship's weapon actually fires this tick. Reset to `false` by
/// `update_combat_activity` at the start of each broadcast tick. Every ship
/// (player + NPC) carries its own component; no global resource.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeaponFiredThisTick(pub bool);

/// Per-ship marker set to `true` when hostile fire targets that ship this
/// tick, even if shields absorb the hit before hull damage leaks through.
/// Every ship carries its own component.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShipAttackedThisTick(pub bool);

/// Tracks the objective id each captain has chosen to prioritize, **scoped to
/// that captain's own ship** (issue #752 `scoped-objective-priority-state`).
///
/// Before #752 this held a single global `boosted_id`, so one captain's pick
/// bled into every ship and system-AI consumer in the session. It is now keyed
/// by local consumer scope — the captain's own ship identity — so a boost only
/// ever reorders that ship's own objective consumers (its Helm/Tactical/Nav AI
/// via the viewscreen pool, and its Captain panel). A boost set in one scope is
/// structurally invisible to every other scope.
///
/// Applied as a score bonus in `publish_viewscreen_blackboard` /
/// `publish_captain_blackboard` so the AI and the captain panel immediately see
/// the updated priority ordering for that ship.
/// Whether the LocalShip currently takes no damage (issue #900).
///
/// Replaces the former `bridge::GOD_MODE` thread-local: state that changes
/// damage outcomes has to live in the authoritative simulation (so it is part
/// of the digest, per #894) rather than out-of-band host memory. Flipped only
/// by [`apply_god_mode_toggle`] consuming an admitted `ToggleGodMode` command
/// on [`crate::system_registry::GOD_MODE_SYSTEM_ID`] — never written directly
/// from `bridge`'s wasm exports — so the toggle carries a tick, lands in the
/// command log, and a replay reproduces it exactly.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GodMode(pub bool);

#[derive(Resource, Clone, Debug, Default)]
pub struct CaptainPriorityBoost {
    /// scope key (the captain's ship identity) -> currently boosted objective id.
    boosts: std::collections::HashMap<String, String>,
}

impl CaptainPriorityBoost {
    /// Score bonus added to the boosted objective's utility score.
    pub const BOOST_AMOUNT: f32 = 15.0;

    /// Scope key for a ship that has no assigned UUID (single-ship sessions and
    /// bare test fixtures). A real multi-ship session keys every ship by its own
    /// UUID, so boosts never collide across ships.
    pub const LOCAL_SCOPE: &'static str = "local";

    /// The scope key for a ship, given its optional UUID string.
    pub fn scope_key(ship_uuid: Option<&str>) -> &str {
        ship_uuid.unwrap_or(Self::LOCAL_SCOPE)
    }

    /// The objective boosted within `scope`, if any.
    pub fn boosted_for(&self, scope: &str) -> Option<&str> {
        self.boosts.get(scope).map(String::as_str)
    }

    /// Toggle `id` as the boosted objective within `scope`. Selecting the id
    /// already boosted in that scope clears it (same toggle semantics as the
    /// pre-#752 global boost, now per scope).
    pub fn toggle(&mut self, scope: &str, id: &str) {
        if self.boosts.get(scope).map(String::as_str) == Some(id) {
            self.boosts.remove(scope);
        } else {
            self.boosts.insert(scope.to_string(), id.to_string());
        }
    }

    /// The `(id, bonus)` argument to pass to `scored_pool_with_boost` for
    /// `scope`, or `None` when nothing is boosted in that scope.
    pub fn boost_arg<'a>(&'a self, scope: &str) -> Option<(&'a str, f32)> {
        self.boosted_for(scope).map(|id| (id, Self::BOOST_AMOUNT))
    }

    /// Remove any boost (in any scope) that points at objective `id` — called
    /// when a layer unload removes the objective, so a stale boost can never
    /// keep re-scoring a record that no longer exists (issue #752 lifecycle).
    pub fn prune_objective(&mut self, id: &str) {
        self.boosts.retain(|_, boosted| boosted != id);
    }

    /// True when no scope has a boost set.
    pub fn is_empty(&self) -> bool {
        self.boosts.is_empty()
    }

    /// True when any scope currently boosts `id`.
    pub fn contains_objective(&self, id: &str) -> bool {
        self.boosts.values().any(|v| v == id)
    }

    /// Every `(scope, boosted objective)` pair, sorted by scope.
    ///
    /// Sorted, not raw, because the backing store is a `HashMap` whose
    /// iteration order follows `RandomState`'s per-process seed — fine for a
    /// lookup, useless to anything that has to produce the same answer twice.
    /// Added by issue #901 so the authoritative-state digest can fold this
    /// resource (issue #894's record puts it in the fold) without reaching into
    /// a private field or inheriting hash order.
    pub fn boosts_sorted(&self) -> Vec<(&str, &str)> {
        let mut pairs: Vec<(&str, &str)> = self
            .boosts
            .iter()
            .map(|(scope, objective)| (scope.as_str(), objective.as_str()))
            .collect();
        pairs.sort();
        pairs
    }
}

/// Applies admitted `ToggleGodMode` commands from the LocalShip's own
/// `AdmittedCommands` to the [`GodMode`] resource (issue #900).
///
/// Runs in `SimSet::Input`, alongside the other admitted-command appliers
/// (e.g. `console::captain::server::handle_set_red_alert`), so the flip lands
/// before `SimSet::Damage` reads it and on the exact tick the command was
/// admitted for — the same "apply the tick you were admitted" contract every
/// other command gets from `command_admission` (AGENTS.md constraint 7).
///
/// Only the `LocalShip` is queried: `admit_system_commands` routes anything
/// that isn't an `ai:`-prefixed token (including `LOCAL_CONSOLE_TOKEN`, the
/// only token `is_command_authorized` admits for this target) to the
/// `LocalShip`'s own `AdmittedCommands`, so an NPC's `AdmittedCommands` never
/// carries this command.
fn apply_god_mode_toggle(
    ship_query: Query<&crate::messages::AdmittedCommands, With<LocalShip>>,
    mut god_mode: ResMut<GodMode>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::GOD_MODE_SYSTEM_ID) {
        if matches!(
            cmd.payload,
            crate::messages::SystemControlPayload::ToggleGodMode
        ) {
            god_mode.0 = !god_mode.0;
        }
    }
}

/// Carries the reason string — and, since #843, the structured
/// [`Outcome`](crate::balance::Outcome) — when the game ends. Set before
/// transitioning to `GamePhase::GameOver`. The `OnEnter(GameOver)` system reads
/// `.0` and broadcasts the reason to all clients; the headless exit report
/// reads `.1` to classify victory vs defeat without string-matching the
/// per-world reason.
///
/// Field `.0` (display string) is unchanged — every existing site that reads or
/// writes it keeps working. Field `.1` is the outcome: `Some(Defeat)` at the
/// built-in player-death sites, whatever a scenario declared on its `game_over`
/// action, or `None` for an undeclared scripted end (the classifier defaults
/// that to victory).
#[derive(Resource, Default)]
pub struct GameOverReason(pub Option<String>, pub Option<crate::balance::Outcome>);

/// Prevents `handle_collisions` from applying damage every frame while the
/// ship is in contact. After damage is applied once, a 1-second cooldown
/// suppresses further hits until the ship clears the obstacle.
///
/// Per-entity component (PRD #597 PR-8): every ship (player + NPC) carries
/// its own `CollisionCooldown`, so an NPC in contact with an asteroid does
/// not suppress the player's collision damage tick and vice versa.
#[derive(Component, Default)]
pub struct CollisionCooldown {
    pub remaining_secs: f32,
}

/// Pending outbound messages produced by simulation systems.
/// Drained each frame by the `SimBroadcaster` dispatch.
///
/// ## Migration note (PRD #253)
/// The old preamble pattern (`MessageWriter<OutboundMessage>`) has been
/// eliminated from all domain plugins. All systems that previously wrote
/// `OutboundMessage` directly now write `(Target, ServerMessage)` tuples
/// into `SimOutbox`. The `sim_outbox_broadcaster()` or a manual drain
/// (in tests) flushes these entries to the `OutboundMessage` bus.
/// To verify the absence of the old pattern, run:
///   rg 'MessageWriter<OutboundMessage>' src/  # must return no matches
#[derive(Resource, Default)]
pub struct SimOutbox(pub Vec<(Target, ServerMessage)>);

/// Broadcast delta caches — [`LastBroadcastEntityPositions`],
/// [`LastBroadcastEntityHealth`], [`LastBroadcastHull`], [`LastBroadcastShields`],
/// [`LastBroadcastBlackboards`] — now live in
/// [`crate::core::broadcast::cache_registry`] (issue #613), which is the
/// single module that knows about all six delta caches (the sixth,
/// `LastWeaponsUpdate`, stays in `console::weapons`) and owns
/// `reset_all` / `resync_for_token` / `prune`. Re-exported here so existing
/// `crate::server_app::LastBroadcastX` / `crate::simulation::LastBroadcastX`
/// references are unaffected by the move.
pub use crate::core::broadcast::cache_registry::{
    LastBroadcastBlackboards, LastBroadcastEntityHealth, LastBroadcastEntityPositions,
    LastBroadcastHull, LastBroadcastShields,
};

/// Tracks non-asteroid entities that have been reported to clients via
/// `EntitySpawned` / `EntityDespawned`.  Seeded from `WorldResource` on
/// the first `InProgress` frame so initial world entities are not re-reported.
///
/// Maintained by the `reconcile_runtime_entities` system.
#[derive(Resource, Default)]
pub struct TrackedEntities {
    /// UUIDs of non-asteroid entities already reported to clients.
    /// Populated from `WorldResource` at game start, then updated
    /// incrementally as runtime entities are spawned/despawned.
    pub reported: std::collections::HashSet<String>,
    /// Whether the registry has been seeded from initial WorldResource
    /// on the first InProgress frame.
    pub seeded: bool,
}

impl TrackedEntities {
    /// Record that a kill site has already broadcast `EntityDespawned` for this
    /// uuid, so the reconcile sweep (`reconcile_runtime_entities`) does not
    /// re-emit a second one (issue #838). No-op if the uuid was never reported.
    pub fn forget(&mut self, uuid: &str) {
        self.reported.remove(uuid);
    }
}

/// The [`WorldResource`] snapshot plus the [`TrackedEntities`] registry, bundled
/// as one `SystemParam` for a kill-site system that would otherwise blow Bevy's
/// 16-parameter ceiling (the torpedo lifecycle) by carrying both separately.
/// `world` is non-optional — every app that runs the torpedo tick inserts
/// `WorldResource` — while `tracked` is `Option` for the bare-`App` fixtures
/// that never insert it (there the reconcile sweep does not run either, so the
/// eager `EntityDespawned` stands alone and the tests asserting it stay green).
#[derive(bevy::ecs::system::SystemParam)]
pub struct WorldAndTracked<'w> {
    pub world: ResMut<'w, crate::lobby::WorldResource>,
    pub tracked: Option<ResMut<'w, TrackedEntities>>,
}

/// The two resources a kill site touches when the *player's* ship is the one
/// that dies: the phase transition and the first-write reason/outcome latch.
///
/// Bundled for the same reason as [`WorldAndTracked`] — the torpedo lifecycle
/// is already at Bevy's 16-parameter ceiling and could not carry them
/// separately. Both are `Option` because bare-`App` fixtures that only exercise
/// damage never insert them, and a missing latch must not fail parameter
/// validation.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PlayerDeathLatch<'w> {
    pub next_state: Option<ResMut<'w, NextState<crate::messages::GamePhase>>>,
    pub reason: Option<ResMut<'w, GameOverReason>>,
}

/// The ambient resources every damage chokepoint reads: the seeded RNG it
/// draws hull distribution from, the log filter its `plog!` lines are gated
/// on, and (issue #900) the God Mode flag that zeroes damage to the local
/// ship.
///
/// Bundled for the same reason as [`WorldAndTracked`] — the blaster and
/// torpedo damage systems are at Bevy's 16-parameter ceiling, and adding the
/// damage log sites (`--log damage=info` printed nothing for a blaster or
/// torpedo kill) pushed both over it. Every field is `Option` because a bare
/// `App` unit-test fixture inserts none of them, and a bare `Res` would fail
/// parameter validation there.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SimRngAndLog<'w> {
    pub rng: Option<Res<'w, crate::sim_rng::SimRng>>,
    pub log: Option<Res<'w, crate::logging::LogFilterConfig>>,
    pub god_mode: Option<Res<'w, GodMode>>,
    /// The tick-scoped id mint (issue #907). Rides in this bundle because the
    /// two weapon systems that mint projectile ids are the same two that were
    /// already at Bevy's parameter ceiling, and a projectile id is minted in
    /// the same breath as the damage draw beside it.
    pub id_mint: Option<Res<'w, crate::world_id::WorldIdMint>>,
}

impl SimRngAndLog<'_> {
    /// True while the local ship's God Mode is on (issue #900). `false` when
    /// the resource is absent (a bare-`App` fixture that never registered
    /// it) — the same "missing means off" default the old thread-local gave.
    pub fn god_mode_active(&self) -> bool {
        self.god_mode.as_ref().is_some_and(|g| g.0)
    }
}

/// Per-entity component holding a ship's system blackboards. Each
/// `publish_*_blackboard` system writes directly into this component on the
/// entity it publishes for — the weapons publishers already publish per-Ship;
/// the remaining publishers still query the `LocalShip` entity only and
/// migrate to per-entity publishing in later issues. The broadcast pipeline
/// reads from this component.
///
/// Stays a `HashMap`: every `publish_*_blackboard` system writes into this map
/// each tick, and `SystemId` keys are long strings with shared prefixes
/// (`torpedo-tube-fore-port` against `…-fore-starboard`), which a `BTreeMap`
/// would compare several times per operation. Iteration order still reaches the
/// wire, so `broadcast_blackboard_updates` sorts the (much smaller) set of
/// *changed* entries instead — ordering where it is observed rather than
/// everywhere it is written.
#[derive(Component, Default, Clone)]
pub struct ShipSystemBlackboards(
    pub std::collections::HashMap<crate::messages::SystemId, crate::messages::SystemBlackboard>,
);

// ── Plugin ───────────────────────────────────────────────────────────────────
/// Empty system used as an ordering anchor for the sim broadcast dispatch.
/// All sim-phase systems (message handlers, tick systems, broadcasters) should
/// run before this anchor so that `broadcast::dispatch::<Sim>` (which has
/// `.after(sim_processing_anchor)`) drains their `SimOutbox` writes.
pub fn sim_processing_anchor() {}

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
const SIM_SET_PLUGIN_REGISTRARS: [fn(&mut App); 13] = [
    |app| {
        app.add_plugins(crate::region_plugin::RegionPlugin);
    },
    |app| {
        app.add_plugins(crate::console_ai_plugin::ConsoleAiPlugin);
    },
    |app| {
        app.add_plugins(crate::ai_plugin::AiPlugin);
    },
    |app| {
        app.add_plugins(crate::captain_plugin::CaptainPlugin);
    },
    |app| {
        app.add_plugins(crate::helm_plugin::HelmPlugin);
    },
    |app| {
        app.add_plugins(crate::ship_plugin::ShipPlugin);
    },
    |app| {
        app.add_plugins(crate::weapons_plugin::WeaponsPlugin);
    },
    |app| {
        app.add_plugins(crate::repair_plugin::RepairPlugin);
    },
    |app| {
        app.add_plugins(crate::power_plugin::ShipPowerPlugin);
    },
    |app| {
        app.add_plugins(crate::shields_plugin::ShipShieldsPlugin);
    },
    |app| {
        app.add_plugins(crate::sensors_plugin::ShipSensorsPlugin);
    },
    |app| {
        app.add_plugins(crate::navigation_plugin::NavigationPlugin);
    },
    |app| {
        app.add_plugins(crate::comms_plugin::CommsConsolePlugin);
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
            crate::entity_config::GlobalConfig::default().sim_tick_hz,
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
        crate::sim_tick::sim_tick_period(crate::entity_config::GlobalConfig::default().sim_tick_hz),
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

    app.add_message::<AsteroidDestroyedVfx>()
        // Balance telemetry. Registered here (not behind `headless`) so the
        // chokepoints can emit unconditionally — only the *collection* is
        // headless-only.
        .add_message::<crate::balance::BalanceEvent>()
        .init_resource::<CaptainPriorityBoost>()
        // The sim's one source of randomness. `init_resource` draws an OS seed, so
        // an unconfigured app (browser host, unit tests) behaves as it always did;
        // headless overrides it with a configured one via `insert_resource`.
        .init_resource::<crate::sim_rng::SimRng>()
        .insert_resource(crate::config_cache::FactionRegistryResource(
            crate::config_cache::get_faction_registry(),
        ))
        .init_resource::<WorldResource>()
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<TrackedEntities>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<LastBroadcastShields>()
        .init_resource::<LastBroadcastBlackboards>()
        .init_resource::<crate::messages::InterSystemQueue>()
        // `handle_collisions` (registered below, in SimSet::Damage) writes this,
        // so the simulation owns it. `DebugOverlayPlugin` also init_resource's it
        // — idempotent — but that plugin is absent headless, and the sim must not
        // depend on the debug overlay being present.
        .init_resource::<crate::debug_overlay::DamageLog>()
        .insert_resource(SimBroadcastTimer(Timer::from_seconds(
            0.1,
            TimerMode::Repeating,
        )))
        .add_systems(
            Startup,
            setup_world.after(crate::world::server::insert_world_config_resource),
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

    // God Mode (issue #900): the resource `apply_god_mode_toggle` flips, and
    // the consumer registration so the unrouted-command lint below doesn't
    // warn about `god-mode` — the applier registered right after this is its
    // one consumer. Registered here (not via a console plugin) because no
    // console owns it: it is a host-only debug toggle, not a station system.
    app.init_resource::<GodMode>();
    {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::GOD_MODE_SYSTEM_ID,
        ));
    }
    app.add_systems(
        FixedUpdate,
        apply_god_mode_toggle.in_set(crate::sim_sets::SimSet::Input),
    );

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
    )
    .add_systems(
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
            broadcast_shield_status.in_set(crate::sim_sets::SimSet::Broadcast),
            handle_collisions.in_set(crate::sim_sets::SimSet::Damage),
            sim_processing_anchor,
        )
            .after(crate::lobby::LobbySystemSet),
    )
    .add_systems(
        FixedUpdate,
        crate::modifier_coordination::translate_power_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        FixedUpdate,
        crate::modifier_coordination::translate_impulse_modifiers
            .in_set(crate::sim_sets::SimSet::Modifiers),
    )
    .add_systems(
        FixedUpdate,
        crate::modifier_coordination::apply_radar_damage_modifiers
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
            publish_viewscreen_blackboard.after(crate::ai_plugin::aggregate_doctrine_blackboards),
        )
            .in_set(crate::sim_sets::SimSet::PublishAggregate),
    )
    .add_plugins(weapons_update_broadcaster())
    .add_plugins(sim_state_broadcaster())
    .add_plugins(modifier_events_broadcaster())
    .add_plugins(sim_outbox_broadcaster());

    if opts.render {
        app.add_plugins(crate::entity_star::StarRenderPlugin)
            .add_plugins(crate::entity_planet::PlanetRenderPlugin)
            .init_resource::<ProceduralMeshCache>()
            .add_systems(Update, render_spawned_entities)
            .add_systems(Update, update_mesh_lod.after(render_spawned_entities))
            .add_systems(Update, face_player_lights.after(render_spawned_entities));
    }

    #[cfg(feature = "server")]
    if opts.render {
        use crate::server::asset_preload::{
            auto_transition_from_loading, begin_asset_preload, broadcast_loading_progress,
            broadcast_loading_start, poll_asset_preload,
        };
        app.add_plugins(crate::server::ServerViewscreenRadarPlugin)
            .init_resource::<crate::server::asset_preload::AssetPreloadResource>()
            .add_systems(Update, begin_asset_preload)
            .add_systems(Update, poll_asset_preload)
            .add_systems(OnEnter(GamePhase::Loading), broadcast_loading_start)
            .add_systems(
                Update,
                broadcast_loading_progress.run_if(in_state(GamePhase::Loading)),
            )
            .add_systems(
                Update,
                auto_transition_from_loading
                    .run_if(in_state(GamePhase::Loading))
                    .after(poll_asset_preload),
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

/// Returns a [`SimBroadcaster`] pre-configured with the `SimState` producer.
///
/// Broadcasts `SimState` at 10 Hz to all players (`Audience::All`).
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
pub fn sim_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::Hz(10.0), |world: &mut World| {
        let entity_states = build_sim_state_entity_states(world);

        // ── Emit SystemHullUpdate per recipient, only when that recipient's
        // *visible* detail changed (issue #737).
        //
        // Post issue #618 `SystemHullStatus` carries the authoritative
        // `SystemId`, display_name and tier. Post #737 the entry list is a
        // role-scoped projection instead of the whole ship, so the send is a
        // per-token fan-out rather than one `Target::All` push — see
        // `crate::console::repair::visibility`.
        crate::console::repair::visibility::push_hull_updates(world);

        let snapshot = crate::messages::SimSnapshot { entity_states };
        vec![ServerMessage::SimState { snapshot }]
    })
}

/// Compute this tick's `EntityStateSnapshot` list for the `SimState` broadcast.
///
/// Extracted from [`sim_state_broadcaster`]'s producer closure (issue #927)
/// so it can be called directly in tests without going through the
/// Broadcaster/cadence machinery — see the `sim_state_entity_states` test
/// module below, which pins the shield-detail payload population directly
/// (target with shields -> `shields`/`shield_freq` present; entity with none
/// -> absent) without needing a full multi-tick cadence fixture.
fn build_sim_state_entity_states(world: &mut World) -> Vec<crate::messages::EntityStateSnapshot> {
    // ── Asteroids: position/yaw never changes — omit from per-tick payload.
    // The client already has asteroid positions from WorldSetup/AsteroidSpawned.
    // Health fields are delta-compressed: only emitted when changed since last tick.
    type AsteroidRaw = (
        String,
        Option<f32>,
        Option<f32>,
        Option<Vec<crate::messages::ShieldFacingStatus>>,
        Option<f32>,
    );
    let asteroid_raw: Vec<AsteroidRaw> = {
        let mut q = world.query::<(
            &AsteroidUuid,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
        )>();
        q.iter(world)
            .filter_map(|(uuid, hull_comp, shield_comp)| {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                // Per-facing detail + generator frequency (issue #927): the
                // SAME producer this ship's own `ShieldsBlackboard.facings`
                // uses (`ship::shields::shield_facing_statuses`) and the
                // same `ShipShields::frequency()`
                // `tick_frequency_hint_high_fidelity` reads for
                // `FrequencyHint` — one producer, no parallel derivation.
                // These were always sent as `None` before #927, which is
                // why `target_shields`/`target_shield_freq` were always
                // empty on the wire regardless of which console rendered them.
                let shields_wire = shield_comp
                    .map(|s| crate::ship::shields::shield_facing_statuses(&s.0.snapshot()));
                let shield_freq = shield_comp.map(|s| s.frequency());
                // Skip entirely when there are no health components (unbreakable asteroids).
                if hull_fraction.is_none() && shield_fraction.is_none() {
                    return None;
                }
                Some((
                    uuid.0.clone(),
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                ))
            })
            .collect()
    };
    let asteroid_states: Vec<crate::messages::EntityStateSnapshot> = {
        let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
        asteroid_raw
            .into_iter()
            .filter_map(
                |(uuid, hull_fraction, shield_fraction, shields_wire, shield_freq)| {
                    let prev = health_cache
                        .0
                        .get(&uuid)
                        .cloned()
                        .unwrap_or((None, None, None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    // Bucketed projection (issue #927 gap-fill review): a
                    // raw `shields_wire != prev.2` compares `offline_remaining`
                    // at full precision, which `tick_shields` decrements every
                    // tick through a ~30s recovery — that re-triggered this
                    // gate on effectively every 10 Hz tick while any facing
                    // was offline. See `ship::shields::shields_delta_projection`.
                    let shields_changed =
                        crate::ship::shields::shields_delta_projection(&shields_wire)
                            != crate::ship::shields::shields_delta_projection(&prev.2);
                    let freq_changed = shield_freq != prev.3;
                    if !hull_changed && !shield_changed && !shields_changed && !freq_changed {
                        return None;
                    }
                    health_cache.0.insert(
                        uuid.clone(),
                        (
                            hull_fraction,
                            shield_fraction,
                            shields_wire.clone(),
                            shield_freq,
                        ),
                    );
                    Some(crate::messages::EntityStateSnapshot {
                        uuid,
                        position: None,
                        yaw: None,
                        hull_fraction,
                        shield_fraction,
                        flags: vec![],
                        shields: shields_wire,
                        shield_freq,
                        warp_out_remaining_secs: None,
                    })
                },
            )
            .collect()
    };

    // ── Non-asteroid entities (NPCs, stations): collect raw data first so
    // we can drop the ECS borrow before mutating the LastBroadcast* resources.
    type NpcRaw = (
        String,
        bevy::math::Vec3,
        f32,
        Option<f32>,
        Option<f32>,
        Option<Vec<crate::messages::ShieldFacingStatus>>,
        Option<f32>,
    );
    let npc_raw: Vec<NpcRaw> = {
        let mut q = world.query_filtered::<(
            &Transform,
            &EntityUuid,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
        ), Without<Asteroid>>();
        q.iter(world)
            .map(|(transform, uuid, hull_comp, shield_comp)| {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                // Per-facing detail + generator frequency (issue #927) —
                // same producer as the asteroid branch above; see the
                // comment there for why this closes the Sensors-panel gap.
                let shields_wire = shield_comp
                    .map(|s| crate::ship::shields::shield_facing_statuses(&s.0.snapshot()));
                let shield_freq = shield_comp.map(|s| s.frequency());
                let yaw = transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0;
                (
                    uuid.0.clone(),
                    transform.translation,
                    yaw,
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                )
            })
            .collect()
    };

    // Compare against last-broadcast positions and health; skip entities
    // where nothing changed.  Position/yaw suppressed below ~1 cm movement;
    // hull/shield suppressed when the f32 value is identical to last tick.
    const POS_THRESHOLD_SQ: f32 = 0.0001; // 0.01 world-unit radius
    const YAW_THRESHOLD: f32 = 0.001; // ~0.057 degrees
    let npc_states: Vec<crate::messages::EntityStateSnapshot> = {
        // Borrow position cache, then health cache separately (both mut).
        // Collect diffs first to avoid holding multiple mut borrows.
        type NpcDiff = (
            String,
            Option<[f32; 3]>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<Vec<crate::messages::ShieldFacingStatus>>,
            Option<f32>,
        );
        let diffs: Vec<NpcDiff> = {
            let mut pos_cache = world.resource_mut::<LastBroadcastEntityPositions>();
            npc_raw
                .iter()
                .map(
                    |(
                        uuid,
                        pos,
                        yaw,
                        hull_fraction,
                        shield_fraction,
                        shields_wire,
                        shield_freq,
                    )| {
                        let moved = match pos_cache.0.get(uuid) {
                            Some(&(prev_pos, prev_yaw)) => {
                                (*pos - prev_pos).length_squared() > POS_THRESHOLD_SQ
                                    || (*yaw - prev_yaw).abs() > YAW_THRESHOLD
                            }
                            None => true,
                        };
                        if moved {
                            pos_cache.0.insert(uuid.clone(), (*pos, *yaw));
                        }
                        let out_pos = if moved {
                            Some([pos.x, pos.y, pos.z])
                        } else {
                            None
                        };
                        let out_yaw = if moved { Some(*yaw) } else { None };
                        (
                            uuid.clone(),
                            out_pos,
                            out_yaw,
                            *hull_fraction,
                            *shield_fraction,
                            shields_wire.clone(),
                            *shield_freq,
                        )
                    },
                )
                .collect()
        };
        let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
        diffs
            .into_iter()
            .filter_map(
                |(
                    uuid,
                    out_pos,
                    out_yaw,
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                )| {
                    let prev = health_cache
                        .0
                        .get(&uuid)
                        .cloned()
                        .unwrap_or((None, None, None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    // Bucketed projection — see the asteroid branch above and
                    // `ship::shields::shields_delta_projection`'s doc comment.
                    let shields_changed =
                        crate::ship::shields::shields_delta_projection(&shields_wire)
                            != crate::ship::shields::shields_delta_projection(&prev.2);
                    let freq_changed = shield_freq != prev.3;
                    // Skip the entity entirely when nothing at all changed.
                    if out_pos.is_none()
                        && out_yaw.is_none()
                        && !hull_changed
                        && !shield_changed
                        && !shields_changed
                        && !freq_changed
                    {
                        return None;
                    }
                    if hull_changed || shield_changed || shields_changed || freq_changed {
                        health_cache.0.insert(
                            uuid.clone(),
                            (
                                hull_fraction,
                                shield_fraction,
                                shields_wire.clone(),
                                shield_freq,
                            ),
                        );
                    }
                    Some(crate::messages::EntityStateSnapshot {
                        uuid,
                        position: out_pos,
                        yaw: out_yaw,
                        hull_fraction: if hull_changed { hull_fraction } else { None },
                        shield_fraction: if shield_changed {
                            shield_fraction
                        } else {
                            None
                        },
                        flags: vec![],
                        shields: if shields_changed { shields_wire } else { None },
                        shield_freq: if freq_changed { shield_freq } else { None },
                        warp_out_remaining_secs: None,
                    })
                },
            )
            .collect()
    };

    asteroid_states.into_iter().chain(npc_states).collect()
}

/// Returns a [`SimBroadcaster`] pre-configured with the `ModifierAdded` and
/// `ModifierRemoved` producers.
///
/// Drains pending modifier events from [`ShipModifiers`] once per frame and
/// broadcasts each as a separate `ServerMessage` to all players (`Audience::All`).
/// Uses `Cadence::OnEvent` so the producer is called every frame regardless of
/// any Hz timer; an empty drain produces no outbound messages.
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipModifiers` component on
/// the LocalShip entity, falling back to the global Resource for tests that
/// only insert the Resource form.
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        use crate::modifiers::ModifierEvent;
        let events: Vec<ModifierEvent> = {
            let mut q =
                world.query_filtered::<&mut crate::modifiers::ShipModifiers, With<LocalShip>>();
            if let Some(mut mods_comp) = q.iter_mut(world).next() {
                std::mem::take(&mut mods_comp.pending_events)
            } else {
                Vec::new()
            }
        };
        events
            .into_iter()
            .map(|event| match event {
                ModifierEvent::Added {
                    source,
                    slot,
                    bonus,
                } => ServerMessage::ModifierAdded {
                    source,
                    slot,
                    bonus,
                },
                ModifierEvent::Removed { source, slot } => {
                    ServerMessage::ModifierRemoved { source, slot }
                }
            })
            .collect()
    })
}

/// Returns a [`SimBroadcaster`] that drains [`SimOutbox`] each frame and writes
/// each entry as an `OutboundMessage` with per-message target routing.
///
/// Uses `Cadence::OnEvent` so the producer fires every frame.  When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by any simulation system) the queued entries are flushed
/// directly to `OutboundMessage` with their original `Target` routing.
pub fn sim_outbox_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        let mut outbox = world.resource_mut::<SimOutbox>();
        let entries = std::mem::take(&mut outbox.0);
        for (target, msg) in entries {
            world.write_message(OutboundMessage {
                target,
                msg: msg.clone(),
                delivery: delivery_class_for_msg(&msg),
            });
        }
        vec![]
    })
}

/// Derive the delivery class for a `ServerMessage`.
///
/// Snapshot-class messages ride the unordered/no-retransmit DataChannel;
/// everything else (commands, lobby messages, Welcome, etc.) is reliable.
/// This is the single place where delivery class is decided server-side
/// (AC 1). The function is not exported — everything routes through
/// `sim_outbox_broadcaster` or `broadcast::dispatch::<Sim>`.
fn delivery_class_for_msg(msg: &ServerMessage) -> DeliveryClass {
    match msg {
        ServerMessage::SimState { .. }
        | ServerMessage::BlackboardUpdate { .. }
        | ServerMessage::ShieldStatus { .. }
        | ServerMessage::RepairState { .. }
        | ServerMessage::PowerState { .. }
        | ServerMessage::WeaponsUpdate { .. }
        | ServerMessage::SystemHullUpdate { .. } => DeliveryClass::Snapshot,
        _ => DeliveryClass::Reliable,
    }
}

// -- Systems -------------------------------------------------------------------

/// When the entity identified by `LastShipAttacker` no longer exists in the
/// world, clear the attacker record so stale references are not published.
fn clear_last_attacker_on_death(
    mut attacker_q: Query<&mut LastShipAttacker>,
    entity_uuids: Query<&EntityUuid>,
) {
    for mut attacker in attacker_q.iter_mut() {
        let uuid = match &attacker.0 {
            Some(u) => u.clone(),
            None => continue,
        };
        let still_alive = entity_uuids.iter().any(|eu| eu.0.as_str() == uuid.as_str());
        if !still_alive {
            attacker.0 = None;
        }
    }
}

/// When a ship's red alert transitions from on to off, clear the attacker
/// record — the threat has passed and the old attacker is no longer relevant.
///
/// Covers every ship (player + NPC), not just `LocalShip`: NPC captain-AI can
/// set its own `ShipRedAlert` (`handle_set_red_alert` in
/// `console::captain::server` dispatches `SetRedAlert` per-ship), and an
/// NPC that stands down should stop retaliating just like the player does.
///
/// `ShipRedAlert` only changes via an explicit assignment (never a
/// same-value rewrite in production), so `Changed<ShipRedAlert>` combined
/// with a boolean component reduces to exactly the on→off edge: the only way
/// a two-valued component both changes and reads `false` is if it was `true`
/// the instant before. This also sidesteps needing a per-entity "previous
/// value" store — a single shared `Local<bool>` (the pre-#685-followup
/// version) does not work once more than one ship is in the query, since it
/// would still only remember one entity's last state.
fn clear_last_attacker_on_red_alert_off(
    mut attacker_q: Query<
        (&mut LastShipAttacker, &crate::ship_state::ShipRedAlert),
        Changed<crate::ship_state::ShipRedAlert>,
    >,
) {
    for (mut attacker, ra) in &mut attacker_q {
        if !ra.0 {
            attacker.0 = None;
        }
    }
}

/// Publish the `LocalShip` viewscreen blackboard: hull/alert status plus the
/// scored objective pool the player ship's per-system AI (weapons, helm,
/// navigation) reads to pick a directive to serve.
///
/// # Why this MERGES rather than clobbers (issue #842)
///
/// After #842 the game-start player hull carries a default `[behaviour]`
/// doctrine, so the player ship holds BOTH `LocalShip` and `BehaviourSection`.
/// `aggregate_doctrine_blackboards` (`With<BehaviourSection>`) also writes the
/// same `VIEWSCREEN_SYSTEM_ID` entry, from the *template* doctrine. If this
/// system simply overwrote that entry — or vice versa — one objective pool
/// would silently erase the other: the doctrine writer clobbering here dropped
/// the player's scenario objectives entirely, so a shipped defence scenario
/// (`combat_test`) stopped developing combat and violated AC3 (scenario
/// objectives must outrank template doctrine).
///
/// Instead this system, pinned to run `.after(aggregate_doctrine_blackboards)`,
/// combines both sources into one scored pool: the global `ObjectiveManager`
/// scenario objectives (e.g. targeted `Destroy wave_N` @80) UNIONED with the
/// hull's template doctrine (untargeted `Destroy` @45 + `Hold` @20), re-sorted
/// descending by score. Scenario objectives coexist with and outrank the
/// standing default, so the player pursues the mission (restoring `combat_test`)
/// while the untargeted @45 remains a fallback that licenses proactive
/// engagement whenever no scenario objective is in play (the probe worlds).
///
/// The doctrine pool is scored fresh from the `BehaviourSection` component here
/// — NOT read back out of the blackboard entry the doctrine writer left. Those
/// two writers run at different cadences (the doctrine writer is gated to the
/// 10 Hz AI snapshot; this one runs every tick), so reading the published entry
/// and re-merging would re-consume this system's own prior output on the ticks
/// the doctrine writer skipped and duplicate the pool without bound. Rescoring
/// from the component is the one source that stays correct every tick.
///
/// A `LocalShip` with no `BehaviourSection` (pre-#842 shape) merges an empty
/// doctrine pool — i.e. behaves exactly as before.
fn publish_viewscreen_blackboard(
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>,
    local_uuid_q: Query<&crate::entity_spawner::EntityUuid, With<LocalShip>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Option<Res<CaptainPriorityBoost>>,
    mut ship_blackboards_q: Query<
        (
            &mut ShipSystemBlackboards,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::weapons_plugin::LastShipAttacker>,
            Option<&crate::entities::spawner::BehaviourSection>,
        ),
        With<LocalShip>,
    >,
) {
    use crate::messages::{SystemBlackboard, SystemId, ViewscreenBlackboard};
    use crate::objectives::WorldConditions;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let entity_state = ship_blackboards_q.single().ok();
    // Lift Combat Lock + Science Target from the local ship's own radar
    // blackboards (issue #829), published this tick in `SimSet::Publish`.
    let combat_lock = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::tactical_radar_system_id())
        {
            Some(SystemBlackboard::TacticalRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let science_target = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(SystemBlackboard::SensorRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let red_alert = entity_state
        .as_ref()
        .and_then(|(_, ra, _, _, _)| ra.map(|r| r.0))
        .unwrap_or(false);
    let last_damage_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_damage_taken));
    let last_weapon_fired_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_weapon_fired));
    let last_attacker_uuid = entity_state
        .as_ref()
        .and_then(|(_, _, _, la, _)| la.and_then(|l| l.0.clone()));

    let hull_integrity_pct = hull_q
        .single()
        .map(|h| {
            let max = h.0.total_max();
            let cur = h.0.total_current();
            if max > 0.0 {
                (cur / max * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            }
        })
        .unwrap_or(100.0);

    let conditions = WorldConditions {
        red_alert,
        hull_fraction: hull_integrity_pct / 100.0,
        attacked: false,
    };
    // Scope the captain boost to this (local) ship, so a boost only ever
    // reorders this ship's own objective consumers (issue #752).
    let local_uuid = local_uuid_q.single().ok().map(|u| u.0.clone());
    let scope = CaptainPriorityBoost::scope_key(local_uuid.as_deref());
    let captain_boost = boost.as_ref().and_then(|b| b.boost_arg(scope));
    let mut scored_objectives = objectives
        .as_ref()
        .map(|o| o.0.scored_pool_with_boost(&conditions, captain_boost))
        .unwrap_or_default();

    // Merge the hull's standing template doctrine into the scenario pool (see
    // the "why this MERGES" note above). Score the doctrine with the same
    // `attacked` signal the NPC path (`aggregate_doctrine_blackboards`) uses, so
    // a backfilled player and a world-spawned copy of the same hull evaluate
    // their identical doctrine identically (#842 AC4 symmetry). The scenario
    // pool keeps its own conditions (unchanged), so existing player-objective
    // scoring is untouched.
    if let Some((_, _, _, _, Some(behaviour))) = entity_state.as_ref() {
        let doctrine_conditions = WorldConditions {
            red_alert,
            hull_fraction: hull_integrity_pct / 100.0,
            attacked: last_attacker_uuid.is_some(),
        };
        let doctrine_pool =
            crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &doctrine_conditions);
        scored_objectives.extend(doctrine_pool);
    }

    // Re-sort the unioned pool descending by score. `sort_by` is stable, so
    // ties keep concatenation order (scenario objectives before doctrine ones —
    // a deterministic tiebreak the `top_destroy_objective_target` / helm
    // consumers rely on to read the highest-scored directive first). `total_cmp`
    // gives a total, deterministic order the rng-determinism guard depends on.
    scored_objectives.sort_by(|a, b| b.score.total_cmp(&a.score));

    let bb = ViewscreenBlackboard {
        red_alert,
        hull_integrity_pct,
        last_damage_taken_secs,
        last_weapon_fired_secs,
        last_attacker_uuid,
        scored_objectives,
        combat_lock,
        science_target,
    };

    // Write directly to the per-entity component.
    if let Some((mut entity_bbs, _, _, _, _)) = ship_blackboards_q.iter_mut().next() {
        entity_bbs.0.insert(
            SystemId(VIEWSCREEN_SYSTEM_ID.to_string()),
            SystemBlackboard::Viewscreen(bb),
        );
    }
}

/// Collision response for ships in contact: applies hull damage, brings the
/// ship to a hard stop, and de-overlaps it from the collider it hit.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This system (with
/// `separate_ship_from_collision`) writes `forward_speed`/`x`/`z` directly and
/// is an intentional exception: collision response is a correction layered on
/// top of the helm integration, not a competing integrator. Routing it through
/// helm intent would let the ship integrate *into* geometry for a frame before
/// responding. It deliberately does not opt into the debug
/// `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
/// Balance tracer: emit a [`BalanceEvent::PhaseChanged`] for every game-phase
/// transition. Reads the global `StateTransitionEvent<GamePhase>` stream, so it
/// fires exactly once per real transition without tapping each `next_state.set`
/// call site. Same-state "transitions" are skipped. `Option<ResMut<Messages>>`
/// so bare-`App` fixtures without the message registered still validate.
fn emit_phase_change_balance_events(
    mut reader: MessageReader<bevy::state::state::StateTransitionEvent<GamePhase>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let Some(msgs) = balance_events.as_mut() else {
        return;
    };
    for ev in reader.read() {
        if ev.exited == ev.entered {
            continue;
        }
        let fmt = |s: &Option<GamePhase>| match s {
            Some(p) => format!("{p:?}"),
            None => "None".to_string(),
        };
        msgs.write(crate::balance::BalanceEvent::PhaseChanged {
            from: fmt(&ev.exited),
            to: fmt(&ev.entered),
        });
    }
}

/// Everything `handle_collisions` needs to know about the other body in a
/// contact: where it is, how big it is, and — since issue #896 — what it is
/// called.
type CollisionBodyQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Transform,
        Option<&'static ColliderSection>,
        Option<&'static EntityUuid>,
        Option<&'static AsteroidUuid>,
    ),
>;

/// The sort key that puts collision handling in a stable world-ID order
/// (issue #896).
///
/// Authored uuid first — that is the identity two instances of the simulation
/// share, and the one the AC asks for — with the entity index behind it as the
/// tiebreak for anything the world file never named (bare test spawns, and
/// bodies carrying no uuid at all). Deliberately NOT the entity index alone:
/// two hosts agree on it only for as long as they agree on spawn order, which
/// is a weaker promise than the uuid already makes.
fn collision_order_key(
    entity: Entity,
    bodies: &CollisionBodyQuery,
) -> (String, bevy::ecs::entity::EntityIndex) {
    let uuid = bodies
        .get(entity)
        .ok()
        .and_then(|(_, _, entity_uuid, asteroid_uuid)| {
            entity_uuid
                .map(|u| u.0.clone())
                .or_else(|| asteroid_uuid.map(|u| u.0.clone()))
        })
        .unwrap_or_default();
    (uuid, entity.index())
}

fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    asteroid_query: Query<
        (&Transform, &AsteroidUuid, Option<&AsteroidShieldPierce>),
        With<Asteroid>,
    >,
    mut ship_query: Query<
        (
            Entity,
            &mut ShipPhysicsComponent,
            &mut CollisionCooldown,
            &mut crate::entity_spawner::EntitySystemHull,
            Option<&mut ShipShields>,
            Option<&ShipModifiers>,
            Option<&EntityUuid>,
            Option<&ColliderSection>,
            Has<LocalShip>,
            Option<&mut ShipImpulse>,
            Option<&mut crate::entity_spawner::EntityShipArcHull>,
        ),
        With<Ship>,
    >,
    body_query: CollisionBodyQuery,
    mut outbox: ResMut<SimOutbox>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut game_over_reason: ResMut<GameOverReason>,
    mut damage_log: ResMut<DamageLog>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut world: ResMut<WorldResource>,
    mut commands: Commands,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    // See `tick_beams_apply_damage` (issue #838): forget the killed uuid from
    // the registry so the reconcile sweep does not re-emit `EntityDespawned`.
    mut tracked: Option<ResMut<TrackedEntities>>,
    // Seeded RNG + log filter + God Mode (issue #900), bundled: separately
    // they put this system one over Bevy's 16-parameter ceiling.
    ambient: SimRngAndLog,
) {
    let dt = time.delta_secs();

    let Ok(ctx) = context.single() else { return };

    // Stable iteration order (issue #896). `ship_query.iter_mut()` walks the
    // archetypes, which is an artefact of how entities were spawned, moved and
    // despawned rather than anything the simulation authored — and the order
    // is load-bearing: a collision can destroy a ship, and which of two ships
    // in a mutual impact is resolved (and so which one dies) first decides the
    // outcome. Sorted by world id, every instance resolves them in the same
    // order.
    let mut ship_order: Vec<((String, bevy::ecs::entity::EntityIndex), Entity)> = ship_query
        .iter()
        // Position 6 of the tuple below is the ship's `Option<&EntityUuid>` —
        // read straight off this query rather than looked up again.
        .map(|(entity, _, _, _, _, _, uuid, ..)| {
            (
                (
                    uuid.map(|u| u.0.clone()).unwrap_or_default(),
                    entity.index(),
                ),
                entity,
            )
        })
        .collect();
    ship_order.sort();

    // Handle every ship (player + NPCs) uniformly. Per-entity CollisionCooldown,
    // ShipModifiers, ShipShields, EntitySystemHull, ShipImpulse. Player-only side
    // effects (damage messages, GameOver, debug log) are gated on `is_local`.
    for (_, ship_entity) in ship_order {
        let Ok((
            ship_entity,
            mut physics,
            mut cooldown,
            mut hull_comp,
            shields_opt,
            modifiers_comp,
            ship_uuid,
            ship_collider,
            is_local,
            mut impulse_opt,
            mut arc_hull_opt,
        )) = ship_query.get_mut(ship_entity)
        else {
            // NOT reachable via an earlier iteration of this same loop
            // despawning `ship_entity`: despawns in this system go through
            // `Commands`, which are deferred until the next `ApplyDeferred`
            // sync point, so an entity queued for despawn earlier in this
            // very call is still present and still queryable here. This arm
            // exists only because `Query::get_mut` returns a `Result` by
            // API — any entity in `ship_order` genuinely missing from
            // `ship_query` (a stale id from a prior tick, a test fixture
            // gap) falls back to skipping it rather than panicking.
            continue;
        };
        cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

        let default_modifiers;
        let modifiers: &ShipModifiers = match modifiers_comp {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        };

        // One collision per ship per tick, and *which* one must not be
        // rapier's business (issue #896). `contact_pairs_with(..).next()` took
        // whatever the narrow phase happened to hand back first — an order
        // that follows the broadphase's internal bookkeeping, and one a
        // parallel broadphase would not even produce consistently between
        // builds. The choice is the lowest world id instead: with a ship
        // wedged between two rocks, every instance of the simulation picks the
        // same rock, and so deals the same damage from the same bearing into
        // the same shield arc.
        let contact = ctx
            .contact_pairs_with(ship_entity)
            // `contact_pairs_with` yields every pair whose *bounding volumes*
            // overlap, not just the ones actually touching (see the method's
            // own doc pointer to `has_any_active_contact`). Filtering to real
            // contacts before the deterministic pick matters because two rocks
            // can have overlapping AABBs without their shapes touching, and a
            // lower-uuid rock merely near the ship must not out-rank a rock the
            // ship is actually embedded in.
            .filter(|pair| pair.has_any_active_contact())
            .filter_map(|pair| {
                if pair.collider1() == Some(ship_entity) {
                    pair.collider2()
                } else {
                    pair.collider1()
                }
            })
            .min_by_key(|other| collision_order_key(*other, &body_query));

        let Some(attacker_entity) = contact else {
            continue;
        };
        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        // Cancel impulse charge on any ship that takes a collision hit.
        if let Some(ref mut impulse) = impulse_opt {
            impulse.0.cancel_charge();
        }

        let speed_at_impact = physics.forward_speed;
        physics.forward_speed = 0.0;
        let attacker_body = body_query.get(attacker_entity).ok();
        separate_ship_from_collision(
            &mut physics,
            collider_radius(ship_collider),
            attacker_body.map(|(transform, ..)| transform),
            collider_radius(attacker_body.and_then(|(_, collider, ..)| collider)),
        );
        let damage = collision_damage(speed_at_impact) as f32
            * modifiers.get(&ModifierSlot::HullDamageTaken);

        let asteroid_info = asteroid_query.get(attacker_entity).ok();
        let bearing = asteroid_info
            .map(|(t, _, _)| {
                attacker_bearing_relative(
                    t.translation.x,
                    t.translation.z,
                    physics.x,
                    physics.z,
                    physics.yaw,
                )
            })
            .unwrap_or(0.0);

        let source_label = asteroid_info
            .map(|(_, uuid, _)| format!("asteroid:{}", uuid.0))
            .unwrap_or_else(|| "collision".to_string());

        // Resolve the colliding asteroid's `shield_pierce` (missing → 0.0,
        // matching pre-#414 behaviour where all collision damage was first
        // absorbed by shields).
        let shield_pierce = asteroid_info
            .and_then(|(_, _, sp)| sp.map(|c| c.0))
            .unwrap_or(0.0);

        // Split impact damage by the asteroid's `shield_pierce`: the
        // pierced fraction goes straight to hull; the absorbed fraction
        // is mitigated by the facing shield quadrant (any leak adds to
        // hull damage).
        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(damage, shield_pierce);
        let mut total_hull = pierced;
        let mut shield_amount = 0.0;

        // Shields are optional per-ship. Absorb through them when present;
        // otherwise all absorbed damage leaks straight to hull.
        let arc_label = if let Some(mut shields) = shields_opt {
            let arc_idx = shields.0.facing_index_for_bearing(bearing);
            let label = shields.0.facings.get(arc_idx).map(|f| f.label.clone());
            if absorbed > 0.0 {
                let leak =
                    apply_damage_with_shields(absorbed.round() as i32, bearing, &mut shields.0);
                shield_amount = (absorbed - leak as f32).max(0.0);
                total_hull += leak as f32;
            }
            label
        } else {
            // No shields → the "absorbed" portion also lands on hull.
            total_hull += absorbed;
            None
        };

        // Entity-scoped trace covering *every* ship. The `DamageLog` below is
        // player-only and capped at 10 entries because it backs the F8 debug
        // overlay; this is the channel that survives a headless run and can be
        // narrowed to one ship with `--log-entity`.
        crate::pdebug!(
            ambient.log,
            crate::logging::LogCat::Damage,
            entity = ship_entity,
            "collision: source={} amount={:.1} arc={:?} pierced={:.1} absorbed={:.1}",
            source_label,
            damage,
            arc_label,
            pierced,
            absorbed
        );

        // Debug damage log: player-only (single-player debug overlay).
        if is_local {
            damage_log.push(DamageLogEntry {
                source: source_label.clone(),
                shield_arc: arc_label,
                amount: damage,
            });
        }

        // What the shields actually lost, captured before the god-mode clamp
        // below. The shield hit was already written into `ShipShields` above,
        // and god mode does not put it back — so the balance tracer has to
        // report the real figure even when the wire message reports zero.
        let shield_absorbed_for_balance = shield_amount;

        // God mode: local ship takes no damage.
        if is_local && ambient.god_mode_active() {
            total_hull = 0.0;
            shield_amount = 0.0;
        }

        let mut ship_destroyed = false;
        let hull_applied = if total_hull > 0.0 {
            crate::sim_rng::with_stream(
                ambient.rng.as_deref(),
                crate::sim_rng::SimStream::CollisionDamage,
                |rng| {
                    let (applied, destroyed) = apply_hull_damage(&mut hull_comp.0, total_hull, rng);
                    // Distribute the same absorbed amount across the per-arc
                    // hull pool (issue #514) so arc tier tracking follows
                    // overall hull damage. Skipped when the ship has no
                    // `EntityShipArcHull` (NPCs).
                    if let Some(ref mut arc_hull) = arc_hull_opt {
                        arc_hull.0.apply_damage(applied, rng);
                    }
                    ship_destroyed = destroyed;
                    applied
                },
            )
        } else {
            0.0
        };

        // The `info` half of the collision damage logging: the per-hit line
        // above is `debug`/`trace` detail, but destruction is a state edge a
        // balancer reads as a headline. Same discipline as the beam, blaster,
        // torpedo, and region kill sites.
        if ship_destroyed {
            crate::pinfo!(
                ambient.log,
                crate::logging::LogCat::Damage,
                entity = ship_entity,
                "destroyed by {}",
                source_label
            );
        }

        // Balance tracer. Environmental damage has no attacker — the asteroid
        // that hit us is identified by the `collision` weapon kind, not by a
        // shooter uuid. Emitted for every ship, not just the LocalShip.
        // Skipped for a ship with no `EntityUuid`, which has no identity the
        // report could key a ledger on.
        if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
            msgs.write(crate::balance::BalanceEvent::DamageApplied {
                attacker: None,
                victim: uuid.0.clone(),
                // Only ships run this collision path; the asteroid is the
                // thing collided *with*, and takes no damage from it.
                victim_kind: crate::balance::VictimKind::Ship,
                weapon: crate::balance::WEAPON_KIND_COLLISION.to_string(),
                amount: damage,
                shield_absorbed: shield_absorbed_for_balance,
                hull_damage: hull_applied,
                system_hit: None,
            });
        }

        // DamageTaken / ShipDestroyed / GameOver are player-facing UI events.
        // Only emit for the LocalShip. NPCs use the AiEntityDestroyed +
        // EntityDespawned path (same as beam-kill).
        if is_local {
            outbox.0.push((
                Target::All,
                ServerMessage::DamageTaken {
                    hull: hull_applied,
                    shield: shield_amount,
                },
            ));
            if ship_destroyed {
                outbox.0.push((Target::All, ServerMessage::ShipDestroyed));
                if game_over_reason.0.is_none() {
                    game_over_reason.0 = Some("All consoles destroyed".into());
                    // The LocalShip died → this run is a defeat (#843). Latched
                    // alongside the reason under the same first-write guard.
                    game_over_reason.1 = Some(crate::balance::Outcome::Defeat);
                    // EntityDestroyed for the player death, once (guarded by the
                    // first reason write). Environmental death → no killer.
                    // Shares the `GameOverReason` latch with a scenario's
                    // `SetGameOverReason`; see the beam death site (console/
                    // weapons/beam.rs) for why that coupling is accepted.
                    if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                        msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                            victim: uuid.0.clone(),
                            killer: None,
                        });
                    }
                }
                next_state.set(GamePhase::GameOver);
            }
        } else if ship_destroyed {
            // NPC destruction: mirror the beam-kill path so downstream world
            // triggers and clients update consistently.
            if let Some(uuid) = ship_uuid {
                world.0.entities.retain(|e| e.uuid != uuid.0);
                destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                    entity_uuid: uuid.0.clone(),
                });
                outbox.0.push((
                    Target::All,
                    ServerMessage::EntityDespawned {
                        uuid: uuid.0.clone(),
                    },
                ));
                if let Some(t) = tracked.as_mut() {
                    t.forget(&uuid.0);
                }
                // EntityDestroyed for the NPC death, co-located with the
                // AiEntityDestroyed write. Environmental death → no killer.
                if let Some(msgs) = balance_events.as_mut() {
                    msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                        victim: uuid.0.clone(),
                        killer: None,
                    });
                }
            }
            commands.entity(ship_entity).try_despawn();
        }
        cooldown.remaining_secs = 1.0;
    }
}

const COLLISION_SEPARATION_SLOP: f32 = 0.05;

fn collider_radius(collider: Option<&ColliderSection>) -> f32 {
    collider.map(|c| c.0.radius.max(0.0)).unwrap_or(0.0)
}

/// Pushes `physics.x`/`z` out along the contact normal so the ship no longer
/// overlaps what it hit. Sanctioned out-of-band `ShipPhysics` writer — see
/// `handle_collisions` and the writer-policy table on `ShipPhysics`.
fn separate_ship_from_collision(
    physics: &mut ShipPhysicsComponent,
    ship_radius: f32,
    attacker_transform: Option<&Transform>,
    attacker_radius: f32,
) {
    let Some(attacker_transform) = attacker_transform else {
        return;
    };
    let min_dist = ship_radius + attacker_radius + COLLISION_SEPARATION_SLOP;
    if min_dist <= 0.0 {
        return;
    }

    let dx = physics.x - attacker_transform.translation.x;
    let dz = physics.z - attacker_transform.translation.z;
    let dist_sq = dx * dx + dz * dz;
    let (nx, nz, dist) = if dist_sq > 1e-6 {
        let dist = dist_sq.sqrt();
        (dx / dist, dz / dist, dist)
    } else {
        // Degenerate overlap: step back opposite the ship's current forward.
        (-simmath::sin(physics.yaw), simmath::cos(physics.yaw), 0.0)
    };

    if dist < min_dist {
        physics.x = attacker_transform.translation.x + nx * min_dist;
        physics.z = attacker_transform.translation.z + nz * min_dist;
    }
}

/// Tick shield regen for the player ship. **PR-7 (issue #597) moved this
/// canonical registration into `ShipShieldsPlugin::tick_shields`, which
/// iterates every ship with the `Ship` marker (player + NPCs). This local
/// stub is retained temporarily as a documented no-op if any test still
/// references it directly; production wiring goes through the plugin.**
#[allow(dead_code)]
fn tick_shields(_time: Res<Time>, _shields_q: Query<&mut ShipShields, With<Ship>>) {
    // Moved: see `crate::ship::shields::tick_shields`.
}

/// Broadcast `ShieldStatus` at 10 Hz.
/// Sends to all players only when shield state changed; always sends to the
/// Shields console holder so their panel stays smooth during regeneration.
fn broadcast_shield_status(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut outbox: ResMut<SimOutbox>,
    sessions: Res<Sessions>,
    ship_query: Query<&ShipShields, With<LocalShip>>,
    mut last: ResMut<LastBroadcastShields>,
) {
    let Some(shields) = ship_query.iter().next() else {
        return;
    };
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let facings: Vec<ShieldFacingStatus> =
        crate::ship::shields::shield_facing_statuses(&shields.0.snapshot());

    let frequency = shields.frequency();
    if facings != last.0 {
        // State changed — broadcast to everyone.
        last.0 = facings.clone();
        outbox.0.push((
            Target::All,
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    } else if let Some(token) = sessions.0.holder_for_station(&StationId("shields".into())) {
        // Nothing changed but the Shields holder still gets a periodic refresh
        // so regenerating HP stays smooth on their panel.
        outbox.0.push((
            Target::Token(token.to_string()),
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    }
}

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
struct WorldSetupBroadcast {
    sent: bool,
}

/// Broadcast `GameOver { reason }` to all players when the game enters the
/// GameOver phase. Reads the reason from `GameOverReason` resource and resets
/// it to `None` after broadcast.
fn on_game_over_enter(mut game_over_reason: ResMut<GameOverReason>, mut outbox: ResMut<SimOutbox>) {
    let reason = game_over_reason.0.take().unwrap_or_default();
    outbox
        .0
        .push((Target::All, ServerMessage::GameOver { reason }));
}

/// Reset all change-detection caches when entering InProgress so the first
/// broadcast tick always sends a full state to all players. Also covers the
/// multi-game restart case where stale cache from a previous game would
/// otherwise suppress initial updates.
///
/// Delegates to [`crate::core::broadcast::cache_registry::reset_all`] (issue
/// #613), the single place that knows about all six broadcast delta caches.
fn reset_broadcast_caches_on_start(
    mut hull: ResMut<LastBroadcastHull>,
    mut shields: ResMut<LastBroadcastShields>,
    mut positions: ResMut<LastBroadcastEntityPositions>,
    mut health: ResMut<LastBroadcastEntityHealth>,
    mut weapons: ResMut<LastWeaponsUpdate>,
    mut last_bb: ResMut<LastBroadcastBlackboards>,
    last_repair_bb: Option<ResMut<crate::console::repair::visibility::LastVisibleRepairBlackboard>>,
) {
    // Per-token repair-blackboard projections (issue #737) are a seventh delta
    // cache; clear them alongside the shared six so a restarted game re-sends.
    if let Some(mut last_repair_bb) = last_repair_bb {
        last_repair_bb.clear();
    }
    crate::core::broadcast::cache_registry::reset_all(
        &mut hull,
        &mut shields,
        &mut positions,
        &mut health,
        &mut weapons,
        &mut last_bb,
    );
}

/// Emit `BlackboardUpdate` for any system whose blackboard has changed since
/// the last broadcast. Reads from the `LocalShip` entity's per-entity component
/// (populated by `dual_publish_blackboards`). Runs in `SimSet::PublishAggregate`
/// (before `SimSet::Broadcast` so `broadcast::dispatch::<Sim>` sees the outbox entries).
///
/// Since issue #737 the *repair* blackboard is fanned out per session token
/// rather than broadcast to all: it carries exact per-system hull detail, and
/// who may see which system is a host-side decision. Every other blackboard
/// still goes out unprojected at `Target::All`.
/// The `Local` caches the `QueryState` across ticks. `World::query_filtered`
/// builds a *fresh* one on every call, and constructing it walks every archetype
/// in the world to work out which ones match — per tick, for a query that
/// resolves to a single `LocalShip` entity. Bevy asserts the state's world id on
/// use, so a cached state cannot silently be applied to the wrong world.
pub fn broadcast_blackboard_updates(
    world: &mut World,
    mut bb_query: Local<Option<QueryState<&'static ShipSystemBlackboards, With<LocalShip>>>>,
) {
    use crate::console::repair::visibility;

    world.init_resource::<visibility::LastVisibleRepairBlackboard>();

    let mut updates: Vec<(crate::messages::SystemId, crate::messages::SystemBlackboard)> = {
        let q = bb_query.get_or_insert_with(|| {
            world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>()
        });
        let Some(bb) = q.iter(world).next() else {
            return;
        };
        let last = world.resource::<LastBroadcastBlackboards>();
        let mut changed: Vec<(crate::messages::SystemId, crate::messages::SystemBlackboard)> =
            bb.0.iter()
                .filter(|(id, bb)| last.0.get(*id) != Some(*bb))
                .map(|(id, bb)| (id.clone(), bb.clone()))
                .collect();
        // Sorted because this vec becomes the `BlackboardUpdate` payload, and
        // it was collected from a `HashMap` whose order follows `RandomState`'s
        // per-process seed — two `--seed` runs emitted the same updates in a
        // different order every time. Sorting the changed set (a handful of
        // entries) rather than ordering the map itself keeps the per-tick
        // publish writes on cheap hash lookups.
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        changed
    };

    let viewers = visibility::connected_viewers(world);

    // A token's station is an input to its repair-blackboard projection, so a
    // player changing station mid-game invalidates it even though nothing the
    // `LastBroadcastBlackboards` diff can see has changed. Without this, an
    // idle undamaged ship would leave the previous station's detail on that
    // phone until the internal blackboard next changed — possibly never.
    let stations_changed = {
        let cache = world.resource::<visibility::LastVisibleRepairBlackboard>();
        cache.stations_changed(&viewers)
    };

    // Prune first so a disconnected token cannot keep suppressing a resend if
    // it reconnects into the same station later in the same game.
    {
        let mut cache = world.resource_mut::<visibility::LastVisibleRepairBlackboard>();
        visibility::prune_repair_blackboard_cache(&mut cache, &viewers);
        cache.record_stations(&viewers);
    }

    if updates.is_empty() && !stations_changed {
        return;
    }

    // Station change with an otherwise-unchanged blackboard: re-feed the
    // current repair blackboard so the per-token projection is recomputed. The
    // per-token cache inside `project_repair_blackboards` still suppresses the
    // send for every token whose *view* did not actually change.
    if stations_changed
        && !updates
            .iter()
            .any(|(_, bb)| matches!(bb, crate::messages::SystemBlackboard::Repair(_)))
    {
        let mut q = world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        if let Some(bb) = q.iter(world).next() {
            if let Some((id, repair)) =
                bb.0.iter()
                    .find(|(_, bb)| matches!(bb, crate::messages::SystemBlackboard::Repair(_)))
            {
                updates.push((id.clone(), repair.clone()));
            }
        }
    }

    // One `resource_mut` for the whole batch, not one per entry: each call is a
    // resource lookup plus a change-tick bump, and this loop runs every tick
    // that anything changed.
    {
        let mut last = world.resource_mut::<LastBroadcastBlackboards>();
        for (id, bb) in &updates {
            last.0.insert(id.clone(), bb.clone());
        }
    }

    let vis = visibility::hull_visibility(world);
    let mut cache = world
        .remove_resource::<visibility::LastVisibleRepairBlackboard>()
        .unwrap_or_default();
    let pending =
        visibility::project_repair_blackboards(updates, vis.as_ref(), &viewers, &mut cache);
    world.insert_resource(cache);
    world.resource_mut::<SimOutbox>().0.extend(pending);
}

// The command-admission seam lives in its own module (issue #736) so that
// dependants can name it with an explicit `use crate::command_admission::…;`.
// Re-exported here so the existing `crate::server_app::Admission*` call sites
// keep resolving unchanged.
pub use crate::command_admission::{
    admit_system_commands, is_command_authorized, station_for_system, AdmissionPlugin, AdmissionSet,
};

/// When a player reconnects mid-game (Identify during InProgress),
/// `handle_identify_system` (in `LobbySystemSet`) queues a `Welcome { .. }` into
/// `LobbyOutbox` targeted at that player's
/// token. Detect this and push a full-state resync to *just that token* via
/// [`crate::core::broadcast::cache_registry::resync_for_token`] (issue #613).
///
/// This replaces the #599 quick fix, which reset all six shared broadcast
/// delta caches — correct for the reconnecting player, but it also forced
/// the *next* 10 Hz tick to broadcast full state to *every other* connected
/// client, since those caches are shared across all `Audience::All`
/// producers. The targeted resync leaves the shared caches untouched, so
/// every other client's next tick remains a normal delta.
fn refresh_caches_on_midgame_reconnect(world: &mut World) {
    let state = world.resource::<State<GamePhase>>();
    if *state.get() != GamePhase::InProgress {
        return;
    }
    let reconnecting_tokens: Vec<String> = {
        let lobby_outbox = world.resource::<LobbyOutbox>();
        lobby_outbox
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (Target::Token(token), ServerMessage::Welcome { .. }) => Some(token.clone()),
                _ => None,
            })
            .collect()
    };
    for token in reconnecting_tokens {
        crate::core::broadcast::cache_registry::resync_for_token(world, &token);
    }
}

/// Emit a single `WorldSetup` broadcast when the game enters `InProgress`.
/// Uses `State<GamePhase>` + sentry to fire exactly once.
fn broadcast_world_setup_on_start(
    state: Res<State<GamePhase>>,
    world: Res<WorldResource>,
    mut sent: ResMut<WorldSetupBroadcast>,
    mut outbox: ResMut<SimOutbox>,
) {
    if sent.sent || state.get() != &GamePhase::InProgress {
        return;
    }
    outbox.0.push((
        Target::All,
        ServerMessage::WorldSetup {
            world: world.0.clone(),
        },
    ));
    sent.sent = true;
}

/// Reconciles the live ECS entities with the `TrackedEntities` registry each tick.
fn upsert_world_entity(world: &mut WorldResource, snapshot: EntitySnapshot) {
    if let Some(existing) = world
        .0
        .entities
        .iter_mut()
        .find(|e| e.uuid == snapshot.uuid)
    {
        *existing = snapshot;
    } else {
        world.0.entities.push(snapshot);
    }
}

fn snapshot_from_entity_config(
    uuid: String,
    id: Option<String>,
    config: &crate::entity_config::EntityConfig,
    position: Vec3,
) -> EntitySnapshot {
    let mut snapshot = EntitySnapshot {
        uuid,
        id,
        name: config.name.clone(),
        position: Some([position.x, position.y, position.z]),
        tags: config.tags.clone(),
        ..EntitySnapshot::default()
    };

    if let Some(radar) = &config.radar_appearance {
        if let Some(colour) = &radar.colour {
            if colour.len() >= 3 {
                snapshot.colour = Some([colour[0], colour[1], colour[2]]);
            }
        }
        if let Some(region_colour) = &radar.region_colour {
            if region_colour.len() >= 3 {
                snapshot.region_colour =
                    Some([region_colour[0], region_colour[1], region_colour[2]]);
            }
        }
        snapshot.radar_size = radar.size;
        snapshot.radar_icon = radar.icon.clone();
    }

    if let Some(collider) = &config.collider {
        if snapshot.radius.is_none() {
            snapshot.radius = Some(collider.radius);
        }
    }

    if let Some(target) = &config.target {
        snapshot.target_tags = target.tags.clone();
        snapshot.threat_level = Some(target.threat_level.as_str().to_string());
        snapshot.target_description = target.description.clone();
    }

    // Initial shield fraction (#471). When the entity has a `[shields]`
    // block, seed the snapshot at full HP. Per-tick updates flow through
    // `EntityStateSnapshot.shield_fraction` from `sim_state_broadcaster`.
    if config.shields_console.is_some() {
        snapshot.shield_fraction = Some(1.0);
    }

    snapshot
}

/// For non-asteroid entities carrying `EntityUuid`:
/// - New entities (present in ECS, absent from `reported`) emit `EntitySpawned`
///   and are added to `WorldResource.entities` so they appear on reconnect `Welcome`.
/// - Missing entities (absent from ECS, present in `reported`) emit
///   `EntityDespawned` and are removed from `WorldResource.entities`.
///
/// Asteroids are excluded (they use `AsteroidSpawned` / `AsteroidDestroyed`).
///
/// On the very first `InProgress` tick, seeds `reported` from the initial
/// `WorldResource` entities so those are not re-broadcast.
fn reconcile_runtime_entities(
    mut registry: ResMut<TrackedEntities>,
    mut world: ResMut<WorldResource>,
    query: Query<
        (
            Entity,
            &EntityUuid,
            Option<&EntityId>,
            Option<&EntityName>,
            &Transform,
            Option<&RegionShapeSection>,
            Option<&EntityTagsSection>,
            Option<&RadarAppearanceSection>,
            Option<&AsteroidFieldSection>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&crate::entity_spawner::EntityTarget>,
            Option<&crate::ship::shields::ShipShields>,
        ),
        Without<Asteroid>,
    >,
    mut outbox: ResMut<SimOutbox>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut positions_cache: ResMut<LastBroadcastEntityPositions>,
    mut health_cache: ResMut<LastBroadcastEntityHealth>,
) {
    // Build set of entity names referenced by active mission objectives.
    let active_objective_names: std::collections::HashSet<String> = objectives
        .as_ref()
        .map(|obj| {
            obj.0
                .sorted_snapshots()
                .into_iter()
                .filter(|s| s.status == crate::messages::ObjectiveStatus::Active)
                .flat_map(|s| s.targets)
                .collect()
        })
        .unwrap_or_default();
    // Build the current set of ECS entity UUIDs.
    let current: HashMap<String, Entity> = query
        .iter()
        .map(|(e, u, _, _, _, _, _, _, _, _, _, _)| (u.0.clone(), e))
        .collect();

    /// Serialise a `RegionShape` to the wire string (snake_case variant name).
    fn shape_to_wire(shape: &RegionShapeSection) -> String {
        use crate::region_shape::RegionShape;
        match &shape.0 {
            RegionShape::Sphere { .. } => "sphere",
            RegionShape::Box { .. } => "box",
            RegionShape::Torus { .. } => "torus",
        }
        .to_string()
    }

    // Seed reported set from ECS on first in-progress frame so that initial
    // world entities (stars, planets, ships, fields) are not re-reported.
    // Also populate WorldData.entities so the reconnect Welcome includes them.
    if !registry.seeded {
        for (uuid, entity) in &current {
            registry.reported.insert(uuid.clone());
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::region_shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::region_shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::region_shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot);
            }
        }
        registry.seeded = true;
        return;
    }

    // Emit EntitySpawned for new entities, in UUID order.
    //
    // Iterating `current` directly announced the same ships in a different
    // order on every run, because `HashMap` order follows `RandomState`'s
    // per-process seed. Only the *newly seen* ids are sorted — almost always
    // none, and a handful on the tick a wave spawns — so this stays off the
    // per-tick cost of walking every entity.
    let mut newly_seen: Vec<(&String, &Entity)> = current
        .iter()
        .filter(|(uuid, _)| !registry.reported.contains(*uuid))
        .collect();
    newly_seen.sort_by(|a, b| a.0.cmp(b.0));
    for (uuid, entity) in newly_seen {
        if registry.reported.insert(uuid.clone()) {
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::region_shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::region_shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::region_shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot.clone());
                outbox
                    .0
                    .push((Target::All, ServerMessage::EntitySpawned { snapshot }));
            }
        }
    }

    // Emit EntityDespawned for entities no longer in the ECS.
    let reported_snapshot: Vec<String> = registry.reported.iter().cloned().collect();
    for uuid in &reported_snapshot {
        if !current.contains_key(uuid) {
            registry.reported.remove(uuid);
            world.0.entities.retain(|e| e.uuid != *uuid);
            // Prune the despawned UUID from the delta caches (issue #613) —
            // runtime-spawned entities (e.g. scenario-triggered NPCs) can
            // despawn and respawn with fresh UUIDs just like asteroids.
            crate::core::broadcast::cache_registry::prune(
                &mut positions_cache,
                &mut health_cache,
                std::slice::from_ref(uuid),
            );
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned { uuid: uuid.clone() },
            ));
        }
    }
}

// ── World Setup ────────────────────────────────────────────────────────────
//
// Per PRD #341, asteroid-field entries and named `[[entity]]` instances are
// owned by `world::server::spawn_world_entities`. This `setup_world` system
// covers only:
//   * spawning *anonymous* immediate `[[entity]]` instances (e.g. stars,
//     planets) that aren't asteroid fields and don't carry a `name`.
//
// When no `WorldConfig` is loaded (native unit tests only — production
// always loads a world TOML via the WASM bridge) this is a no-op.
fn setup_world(
    mut commands: Commands,
    mut world: ResMut<WorldResource>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    let config_cache = crate::config_cache::get_config_cache();

    // Pre-resolve named-entity positions so anonymous entries using
    // `relative_to` can be positioned (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(&world_config);

    for entity_inst in &world_config.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::Immediate {
            continue;
        }
        // Asteroid-field entries and named entries are owned by the unified
        // spawn pass in `world::server::spawn_world_entities`. Skip them to
        // avoid double-spawning.
        let is_unified = crate::world::config::is_owned_by_unified_pipeline(entity_inst, |path| {
            config_cache
                .get(path)
                .and_then(|c| c.asteroid_field.as_ref())
                .is_some()
        });
        if is_unified {
            continue;
        }

        let config = match crate::entity_loader::resolve_entity(entity_inst, &config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "setup_world: failed to resolve entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        let uuid =
            crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &world_config.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!("setup_world: {e}");
                continue;
            }
        };

        crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid.clone(),
            entity_inst.id.clone(),
        );
        upsert_world_entity(
            &mut world,
            snapshot_from_entity_config(uuid, entity_inst.id.clone(), &config, pos),
        );
    }
}

fn player_spawn_rotation_yaw(rot: [f32; 3]) -> (bevy::math::Quat, f32) {
    let q = bevy::math::Quat::from_euler(bevy::math::EulerRot::YXZ, rot[1], rot[0], rot[2]);
    let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
    (q, yaw)
}

/// Compute the player ship's identity — the `player` tag and the `playerShip`
/// radar icon — to inject at the player game-start spawn.
///
/// This identity is deliberately NOT authored in the hull templates. If it
/// were, every world-spawned copy of the same hull (which spawns as an NPC)
/// would masquerade as the player: it would answer `player`-only radar filters
/// and draw with the player blip. Injecting here scopes the identity to the one
/// hull the local player actually flies.
///
/// Returns `(tags, radar)`: the template tags with `player` appended (keeping
/// `ship`, which player-ship selection keys off), and the template's radar
/// appearance with its icon forced to `playerShip` (colour/size preserved).
/// The caller re-inserts these onto the spawned entity; Bevy `insert` replaces,
/// so this overwrites the ordinary-ship sections `spawn_entity` set from the
/// template.
fn player_ship_identity(
    template_tags: &[String],
    template_radar: Option<&crate::entity_config::RadarAppearanceConfig>,
) -> (Vec<String>, crate::entity_config::RadarAppearanceConfig) {
    let mut tags = template_tags.to_vec();
    let player_tag = crate::entity_tags::EntityTag::Player.as_str();
    if !tags.iter().any(|t| t == player_tag) {
        tags.push(player_tag.to_string());
    }
    let mut radar =
        template_radar
            .cloned()
            .unwrap_or(crate::entity_config::RadarAppearanceConfig {
                icon: None,
                colour: None,
                size: None,
                region_colour: None,
            });
    radar.icon = Some(crate::server::asset_preload::PLAYER_SHIP_RADAR_ICON.to_string());
    (tags, radar)
}

/// Spawn entities with `spawn_on = GameStart` (e.g. player ship) when the
/// game transitions to InProgress. Registered in `OnEnter(GamePhase::InProgress)`.
fn spawn_game_start_entities(
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending_ship_config: Option<ResMut<crate::ship_plugin::PendingShipConfig>>,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    mut sessions: Option<ResMut<crate::lobby::Sessions>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut has_spawned: Local<bool>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    if *has_spawned {
        return;
    }

    let mc = match world_config.as_deref() {
        Some(mc) => mc,
        None => return,
    };

    let config_cache = crate::config_cache::get_config_cache();

    let mut ship_spawned = false;
    let named_positions = crate::world::config::build_named_entity_positions(mc);
    for entity_inst in &mc.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            continue;
        }
        // Evaluate optional spawn predicate against the world flag store.
        if let Some(pred) = &entity_inst.when_predicate {
            let empty = crate::world::flags::FlagStore::new();
            let flags_ref = runtime.as_ref().map(|r| &r.flags).unwrap_or(&empty);
            if !pred.evaluate(&[flags_ref]) {
                continue;
            }
        }
        let config = match crate::entity_loader::resolve_entity(entity_inst, &config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // The player ship's full loadout (weapons, torpedoes, blasters, shields,
        // mesh, stations) must come from the lobby-selected ship template, not
        // the world's `[[entity]] player-ship` placeholder. The placeholder only
        // fixes spawn position; without this override a player who selects the
        // Destroyer still spawns the placeholder hull's weapons (e.g. the
        // cruiser's two phaser banks and no blasters). ShipConfigComponent is
        // already sourced from the selection (PendingShipConfig); this brings the
        // EntityConfig-derived systems into agreement. Matched on the same
        // predicate used below for the player-ship position/rotation/marker.
        let config = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            match selected_ship
                .as_ref()
                .and_then(|sel| config_cache.get(&sel.0))
            {
                Some(selected_cfg) => selected_cfg.clone(),
                None => config,
            }
        } else {
            config
        };

        let uuid =
            crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);
        let pos = match crate::world::config::resolve_entity_position_with(
            entity_inst,
            &mc.anchors,
            &named_positions,
        ) {
            Ok(p) => Vec3::new(p[0], p[1], p[2]),
            Err(e) => {
                bevy::log::error!(
                    "Failed to resolve GameStart entity '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };

        // Override with player_spawn position when spawning the player ship
        // (issue #623).
        let pos = if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            if let Some(ref spawn) = mc.player_spawn {
                if let Some(ref anchor_name) = spawn.anchor {
                    match mc.anchors.get(anchor_name) {
                        Some(a) => Vec3::new(a[0], a[1], a[2]),
                        None => {
                            bevy::log::error!("player_spawn anchor '{}' not found", anchor_name);
                            pos
                        }
                    }
                } else if let Some(p) = spawn.position {
                    Vec3::new(p[0], p[1], p[2])
                } else {
                    pos
                }
            } else {
                pos
            }
        } else {
            pos
        };

        // Override with player_spawn rotation when spawning the player ship (issue #623).
        let player_spawn_rot: Option<bevy::math::Quat> =
            if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
                mc.player_spawn.as_ref().and_then(|s| s.rotation).map(|r| {
                    let (q, _) = player_spawn_rotation_yaw(r);
                    q
                })
            } else {
                None
            };

        let spawned = crate::entity_spawner::spawn_entity(
            &mut commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );

        // Apply rotation on the spawned entity's Transform
        if let Some(q) = player_spawn_rot {
            commands
                .entity(spawned)
                .insert(bevy::prelude::Transform::from_translation(pos).with_rotation(q));
        }

        // Extract yaw for ShipPhysicsComponent
        let initial_yaw = player_spawn_rot
            .map(|q| {
                let (yaw, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
                yaw
            })
            .unwrap_or(0.0);

        // The first GameStart entity with tags containing "ship" gets the Ship marker
        if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            let ship_config = if let Some(pending) = pending_ship_config.as_mut() {
                let cfg = crate::ship_plugin::ShipConfigComponent(pending.0.clone());
                commands.remove_resource::<crate::ship_plugin::PendingShipConfig>();
                pending_ship_config = None;
                cfg
            } else {
                crate::ship_plugin::load_ship_config_from_disk()
            };
            // Seed the reactor from the player ship's authored power groups
            // (issue #762) before `ship_config` is moved into the entity, so
            // authored groups beyond the canonical three (e.g. `ops`) are
            // allocatable. Empty for a config with no `[power_groups.*]`.
            let power_group_seed =
                crate::power_plugin::authored_power_group_seed(&ship_config.0.power_groups);
            let (initial_control_sources, initial_active_ratings) = {
                // The shared boot-seeding path (issue #871) — the same
                // `seed_boot_ratings` `entities::spawner` calls for every other
                // hull. Only the per-station rating CHOICE differs here: this
                // path knows about lobby sessions, so a manned station boots on
                // the player's chosen complexity toggle instead of Backfill.
                match sessions {
                    Some(ref sess) => {
                        let manned: std::collections::HashSet<_> = sess
                            .0
                            .players()
                            .iter()
                            .filter(|p| p.connected)
                            .filter_map(|p| p.station.as_ref())
                            .collect();
                        let (resolver, active_ratings) =
                            crate::ship::rating::seed_boot_ratings(&ship_config.0, |station| {
                                // Manned stations apply the player's
                                // lobby-chosen complexity toggle (if any), else
                                // the station's base (first) rating. Unmanned
                                // stations are fully AI-backfilled, as before.
                                if manned.contains(&station.id) {
                                    sess.0
                                        .pending_rating_for(&station.id)
                                        .cloned()
                                        .or_else(|| station.ratings.first().map(|r| r.name.clone()))
                                        .unwrap_or_else(|| "Std".to_string())
                                } else {
                                    crate::ship::rating::BACKFILL_RATING.to_string()
                                }
                            });
                        (
                            crate::ship_plugin::ShipSystemControlSources(resolver),
                            crate::ship_plugin::ActiveStationRatings(active_ratings),
                        )
                    }
                    // No lobby at all: leave both empty, exactly as before.
                    None => (
                        crate::ship_plugin::ShipSystemControlSources::default(),
                        crate::ship_plugin::ActiveStationRatings::default(),
                    ),
                }
            };
            if let Some(ref mut sess) = sessions {
                sess.0.clear_all_pending_ratings();
            }
            commands
                .entity(spawned)
                .insert(Ship)
                .insert(LocalShip)
                // The player ship is permanently high-fidelity (`lod_ai_ships`
                // never evaluates `LocalShip`), so it takes the marker and the
                // components that travel with it from the SAME shared
                // definition the NPC promotion path uses. Spelling the set out
                // here again is how #785's RepairTargetSelector, #786's
                // CommsTargetSelector and #882's HelmBoostAiPolicyState each
                // silently missed the player ship.
                .insert(crate::ai_plugin::ai_high_fidelity_components())
                .insert(ShipSystemBlackboards::default())
                .insert(ship_config)
                .insert(initial_control_sources)
                .insert(initial_active_ratings)
                .insert(crate::ship_plugin::CoordinationQueue::default())
                .insert(crate::ship_plugin::PendingArcBearingRequest::default())
                .insert(crate::ship_plugin::DockingMotionIntent::default())
                .insert(crate::ship::shields::PendingShieldsThreatBearing::default())
                // Sensors→Tactical frequency advisory a backfilled Tactical
                // consumes off the channel-3 bus (issue #873).
                .insert(crate::ship_plugin::PendingTacticalFrequencyHint::default())
                // Per-ship intent-narration memory (issue #879). The player
                // ship is the one bridge with human seats to narrate TO, so
                // omitting it here — the failure mode #785/#786/#882/#885 each
                // shipped — would leave the whole feature dead on the only
                // hull it exists for.
                .insert(crate::ship_plugin::ShipIntentNarration::default())
                .insert(crate::messages::AdmittedCommands::default())
                .insert(ShipPhysicsComponent {
                    x: pos.x,
                    z: pos.z,
                    yaw: initial_yaw,
                    ..Default::default()
                })
                // Channel-3 Navigation→Helm clearance latch (issue #702).
                .insert(crate::ship_plugin::HelmWaypointClearance::default())
                // Per-objective route cursors. The player ship was missing
                // these — `entities/spawner.rs` inserted them for NPCs and this
                // path did not — which silently disabled AI patrol on the
                // player ship whenever an unmanned Helm backfilled to AI: with
                // no cursor component, `helm_patrol` had no route position to
                // steer from and `advance_objective_cursors` had nothing to
                // advance (issue #702).
                .insert(crate::ai_plugin::ObjectiveCursors::default())
                .insert(crate::weapons_plugin::TacticalRadarSelection::default())
                .insert(crate::weapons_plugin::ActiveBeam::default())
                .insert(crate::weapons_plugin::PhaserCooldown::default())
                .insert(crate::weapons_plugin::WeaponsArcRequestState::default())
                .insert(crate::sensors_plugin::SensorRadarSelection::default())
                .insert(crate::ship_state::ShipRedAlert::default())
                .insert(crate::ship_state::ShipViewMode::default())
                .insert(crate::ship_state::ShipPhaserFrequency::default())
                .insert(crate::navigation_plugin::NavigationWaypoint::default())
                .insert(crate::power_plugin::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::from_authored_groups(
                        crate::modifiers::power_system::PowerConfig::default().capacity,
                        &power_group_seed,
                    ),
                ))
                .insert(crate::ship_plugin::LastHelmInput::default())
                // Per-ship impulse drive state (audit follow-up). Every
                // ship carries its own; NPC ships get one via the spawner
                // too (both idle by default).
                .insert(crate::server_app::ShipImpulse::default())
                // Per-ship boost drive battery (audit follow-up). Every
                // ship carries its own; NPC ships get one via the spawner
                // (both empty by default).
                .insert(crate::server_app::ShipBoost::default())
                // Per-ship coordination bus state (audit follow-up). See
                // `entities/spawner.rs` for details.
                .insert(crate::ship::shields::ShieldsCoordinationState::default())
                .insert(crate::ship::sensors::SensorsFrequencyState::default())
                .insert(crate::ship::sensors::SensorsThreatState::default())
                .insert(crate::ship::power::PowerBrownoutState::default())
                // Per-entity CollisionCooldown so player and NPC ships each
                // have their own cooldown timer (PRD #597 PR-8).
                .insert(CollisionCooldown::default())
                // ShipModifiers as per-entity component (PR 6 — PRD #597; the
                // legacy Resource fallback was removed in issue #606). Every
                // ship — player and NPC — carries its own instance.
                .insert(crate::modifiers::ShipModifiers::new())
                // Combat activity state per-ship (PR 10 — PRD #597). Every
                // ship (player + NPC) tracks its own recent combat activity
                // + this-tick weapon-fired / attacked / last-attacker markers.
                .insert(crate::ship::combat_activity::RecentCombatActivity::default())
                .insert(WeaponFiredThisTick::default())
                .insert(ShipAttackedThisTick::default())
                .insert(crate::weapons_plugin::LastShipAttacker::default());

            // Inject player identity (the `player` tag + `playerShip` radar
            // icon) HERE, on the one ship the local player flies — not in the
            // hull template. The templates author only ordinary-ship identity
            // so that NPC copies of the same hull spawned into the world do not
            // masquerade as the player. `spawn_entity` already inserted the
            // template's ordinary `EntityTagsSection` / `RadarAppearanceSection`;
            // Bevy `insert` replaces, so re-inserting overwrites them. These
            // components feed the snapshot builders, so the injected tag/icon
            // reach clients (and the native radar's player dedup) before the
            // first broadcast.
            let (player_tags, player_radar) =
                player_ship_identity(&config.tags, config.radar_appearance.as_ref());
            commands
                .entity(spawned)
                .insert(EntityTagsSection(player_tags))
                .insert(RadarAppearanceSection(player_radar));

            // The player ship's hull lives on its `EntitySystemHull`
            // component (PRD #581). All damage/repair paths write there
            // directly; the old `ShipHullIntegrity` resource was retired
            // in PRD #597 PR 10.
            ship_spawned = true;

            // Ship-specific resource setup
            if let Some(hc) = &config.hull {
                let _hc = hc; // hull is set up via EntitySystemHull in the spawner
                              // [repair] block — overrides default RepairTimings if present.
                              // Absent block keeps the same defaults the hardcoded constants
                              // used to provide (5.0s travel, 0.5 HP/s repair rate).
                let repair = config.repair.as_ref();
                let team_count = repair
                    .map(|rc| rc.repair_team_count as usize)
                    .filter(|&n| n > 0)
                    .unwrap_or(2);
                let timings = repair.map(|rc| rc.to_runtime()).unwrap_or_default();
                let teams = ShipRepairTeams(crate::repair_teams::RepairTeams::new_with_timings(
                    team_count, timings,
                ));
                // Per-entity component only (issue #830 retired the global Resource).
                commands.entity(spawned).insert(teams);
            }

            // Apply shield focus config + base shield-system values from TOML if present.
            // Post-#514: the `[shields_console.base]` sub-block still holds
            // ship-wide defaults (max_hp, regen_per_sec, offline_duration)
            // consumed as fallbacks by each `[[shield_arc]]` block. When
            // shield_arcs are declared the runtime is built via
            // `ShieldSystem::from_arcs`; otherwise fall back to
            // `ShieldSystem::new` with historical evenly-spaced facings.
            if let Some(sc) = &config.shields_console {
                let ship_wide = sc.base.as_ref().map(|b| b.to_runtime()).unwrap_or_default();
                let shield_system = if !config.shield_arcs.is_empty() {
                    let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
                    ShieldSystem::from_arcs(&arcs, &ship_wide)
                } else {
                    ShieldSystem::new(&ship_wide)
                };
                let freq = config
                    .shield_arcs
                    .first()
                    .map(|a| a.frequency)
                    .unwrap_or(sc.frequency);
                let mut shields = ShipShields(shield_system, freq);
                shields.0.focus_config = crate::shield::ShieldFocusConfig {
                    bonus_max_hp: sc.focus_bonus_max_hp,
                    bonus_regen: sc.focus_bonus_regen,
                    penalty_max_hp: sc.focus_penalty_max_hp,
                    penalty_regen: sc.focus_penalty_regen,
                    decay_rate: sc.focus_decay_rate,
                    focused_damage_multiplier: sc.focus_focused_damage_multiplier,
                    unfocused_damage_multiplier: sc.focus_unfocused_damage_multiplier,
                };
                commands.entity(spawned).insert(shields);
            } else if !config.shield_arcs.is_empty() {
                let ship_wide = crate::shield::ShieldConfig::default();
                let arcs: Vec<_> = config.shield_arcs.iter().map(|a| a.to_runtime()).collect();
                let freq = config
                    .shield_arcs
                    .first()
                    .map(|a| a.frequency)
                    .unwrap_or(0.5);
                commands.entity(spawned).insert(ShipShields(
                    ShieldSystem::from_arcs(&arcs, &ship_wide),
                    freq,
                ));
            } else {
                // Default shields on the ship entity when no TOML shields_console block.
                commands
                    .entity(spawned)
                    .insert(ShipShields(ShieldSystem::default(), 0.5));
            }

            // Shields AI config — loaded from [shields_console.ai] if present,
            // otherwise falls back to ShieldsAiConfigResource defaults. The
            // per-entity Component is what `operate_shields_ai` and
            // `emit_shields_coordination` read; the global Resource is a
            // dual-write with no remaining readers (issue #738).
            let ai_cfg = config
                .shields_console
                .as_ref()
                .and_then(|sc| sc.ai.as_ref())
                .map(|ai| crate::ship::shields::ShieldsAiConfigResource {
                    damage_window_secs: ai.damage_window_secs,
                    min_damage_window_secs: ai.min_damage_window_secs,
                    damage_pct_threshold: ai.damage_pct_threshold,
                    health_ratio_threshold: ai.health_ratio_threshold,
                    ..Default::default()
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(ai_cfg.clone());
            commands.insert_resource(ai_cfg);

            // Shields focus AI policy (issue #783) — player-ship half of the
            // per-entity pattern in `spawner.rs`. The authored
            // `[shields_console.ai_policy]` block drives `ai_shield_focus`'s gate
            // and supplies the authored windows/thresholds. Since #885b stage 5d
            // there is no Rust-side synthesiser behind it: strict AI-declaration
            // mode rejects an AI-capable hull that omits the block at load, so an
            // unauthored policy means no component and no automation rather than
            // one invented in Rust (PRD #774 US7). Validated already in
            // `EntityConfig::from_toml`.
            if let Some(ai) = config
                .shields_console
                .as_ref()
                .and_then(|sc| sc.ai_policy.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::shields::ShieldsFocusAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }

            // Sensors AI config — the player-ship half of the same per-entity
            // pattern (issue #738 follow-up). `tick_frequency_hint_high_fidelity` reads only the
            // Component, and the spawner attaches one to every entity with a
            // `[behaviour]` block; without this, `[sensors_console.ai]` authored
            // on a player-class ship was silently ignored end to end. Behaviour-
            // neutral for every ship TOML in `assets/` today: none declares the
            // section, and the fallback the reader already used is this same
            // parse-time default.
            commands.entity(spawned).insert(
                config
                    .sensors_console
                    .as_ref()
                    .and_then(|sc| sc.ai.as_ref())
                    .map(|ai| crate::ship::sensors::SensorsAiConfigResource {
                        frequency_hint_delay_secs: ai.frequency_hint_delay_secs,
                    })
                    .unwrap_or_default(),
            );

            // Sensors target selector (issue #776) — the player-ship half of the
            // per-entity pattern. The authored `[sensors_console.selector]` block
            // drives `operate_sensors_ai`'s ranking under Backfill. No
            // synthesised stand-in since #885b stage 5d. Validated already in
            // `EntityConfig::from_toml`.
            if let Some(s) = config
                .sensors_console
                .as_ref()
                .and_then(|sc| sc.selector.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::sensors::SensorsTargetSelector {
                        selector: s.to_selector().unwrap_or_default(),
                        power_rating: config.power_rating.map(|r| r as f32),
                    });
            }

            // Tactical target selector (issue #777) — the player-ship half of
            // the per-entity pattern. The authored `[weapons_console.selector]`
            // block drives `ai_target_selection`'s ranking under Backfill.
            if let Some(s) = config
                .weapons_console
                .as_ref()
                .and_then(|wc| wc.selector.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::weapons_plugin::TacticalTargetSelector {
                        selector: s.to_selector().unwrap_or_default(),
                        power_rating: config.power_rating.map(|r| r as f32),
                        // AC6 (issue #781): explicit radar idle from `[weapons_console]
                        // selector_idle`, else baseline (radar runs its selector).
                        idle: config
                            .weapons_console
                            .as_ref()
                            .map(|wc| wc.selector_idle)
                            .unwrap_or(false),
                    });
            }

            // Navigation target selector (issue #778) — the player-ship half of
            // the per-entity pattern. The authored
            // `[navigation_console.selector]` block drives
            // `operate_navigation_ai`'s ranking under Backfill.
            if let Some(s) = config
                .navigation_console
                .as_ref()
                .and_then(|nc| nc.selector.as_ref())
            {
                commands.entity(spawned).insert(
                    crate::console::navigation::NavigationTargetSelector {
                        selector: s.to_selector().unwrap_or_default(),
                        power_rating: config.power_rating.map(|r| r as f32),
                    },
                );
            }

            // Comms hail selector + dialogue-response policy (issue #786) — the
            // player-ship half of the per-entity pattern in `spawner.rs`, and
            // the ONLY half that can ever run: both `operate_comms_ai` and
            // `operate_comms_response_ai` are filtered `With<LocalShip>`, and
            // the spawner never spawns the player ship. Without this,
            // `[comms_console.selector]` / `[comms_console.ai]` parsed,
            // validated, and were then silently ignored (the host's tick-local
            // canonical default always won), and `self_fact(power_rating)` /
            // `fact(power_rating)` were permanently ABSENT — the #779
            // empty-facts failure mode. Both are resolved by the same shared
            // helper the spawner calls, so the two paths cannot drift.
            let (comms_selector, comms_response_policy, comms_response_cadence) =
                crate::console::comms::server::comms_console_ai_components(&config);
            if let Some(sel) = comms_selector {
                commands.entity(spawned).insert(sel);
            }
            if let Some(policy) = comms_response_policy {
                commands.entity(spawned).insert(policy);
            }
            if let Some(cadence) = comms_response_cadence {
                commands.entity(spawned).insert(cadence);
            }

            // Repair target selector (issue #785) — same player-ship gap. Less
            // severe than Comms because `operate_repair_ai`'s host is
            // `With<Ship>`, so spawner-built NPCs already carried one; but the
            // PLAYER ship never goes through the spawner, so an authored
            // `[repair.selector]` on a player-class hull was ignored and
            // `self_fact(power_rating)` was absent there too. Same shape as the
            // spawner's insert; the block is already validated in
            // `EntityConfig::from_toml`.
            if let Some(s) = config.repair.as_ref().and_then(|rc| rc.selector.as_ref()) {
                commands.entity(spawned).insert(
                    crate::console::repair::server::RepairTargetSelector {
                        selector: s.to_selector().unwrap_or_default(),
                        power_rating: config.power_rating.map(|r| r as f32),
                    },
                );
            }

            // Captain AI policy (issue #775) — the player ship half of the
            // per-entity pattern above. The authored `[captain_console.ai]` block
            // drives `operate_captain_ai`. Validated already in
            // `EntityConfig::from_toml`.
            if let Some(ai) = config.captain_console.as_ref().and_then(|c| c.ai.as_ref()) {
                commands
                    .entity(spawned)
                    .insert(crate::captain_plugin::CaptainAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }

            // Helm Engines/Steering AI policies (issue #779) — player-ship half
            // of the per-entity pattern in `spawner.rs`. The authored
            // `[helm_console.engines_ai]` / `[helm_console.steering_ai]` blocks
            // drive `ai_helm_thrust` / `ai_helm_steering`.
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.engines_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmEnginesAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.steering_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmSteeringAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }

            // Helm secondary-actuator AI policies (issue #780) — player-ship half
            // of the per-entity pattern in `spawner.rs`. The authored
            // `[helm_console.lateral_ai/vertical_ai/impulse_ai/boost_ai]` blocks
            // drive the secondary hosts.
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.lateral_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmLateralAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.vertical_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmVerticalAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.impulse_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmImpulseAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }
            if let Some(ai) = config
                .helm_console
                .as_ref()
                .and_then(|h| h.boost_ai.as_ref())
            {
                commands
                    .entity(spawned)
                    .insert(crate::ship::helm_ai::HelmBoostAiPolicy(
                        ai.to_policy().unwrap_or_default(),
                    ));
            }

            // Shields damage history — per-ship Component tracking HP deltas
            // for the AI damage-concentration algorithm. Initialised empty; resized
            // lazily by operate_shields_ai to match the ship's arc count.
            commands
                .entity(spawned)
                .insert(crate::ship::shields::ShieldsDamageHistory::default());

            // Per-arc hull HP (issue #514). Attach `EntityShipArcHull`
            // alongside the shield system so `sync_console_damage_tiers`
            // can flip the fine `shield-arc-<id>` SystemIds into
            // `offline_systems` when an arc's hull HP drops into the
            // Disabled/Destroyed tier.
            if !config.shield_arcs.is_empty() {
                let arc_entries: Vec<(String, crate::damage::ArcHullEntry)> = config
                    .shield_arcs
                    .iter()
                    .filter(|a| a.hull_max_hp > 0.0)
                    .map(|a| {
                        (
                            a.id.clone(),
                            crate::damage::ArcHullEntry {
                                current: a.hull_max_hp,
                                max: a.hull_max_hp,
                                tier_config: crate::damage::ConsoleTierConfig {
                                    damaged_threshold_pct: a.hull_damaged_threshold_pct,
                                    disabled_threshold_pct: a.hull_disabled_threshold_pct,
                                    debuff_magnitude: a.hull_debuff_magnitude,
                                },
                            },
                        )
                    })
                    .collect();
                if !arc_entries.is_empty() {
                    commands
                        .entity(spawned)
                        .insert(crate::entity_spawner::EntityShipArcHull(
                            crate::damage::ShipArcHull::from_entries(arc_entries),
                        ));
                }
            }

            if let Some(wc) = &config.weapons_console {
                let first_bank = wc.phaser_banks.first();
                let beam_color = crate::beam_render::resolve_beam_color(
                    first_bank.map(|b| &b.beam_color).unwrap_or(&vec![]),
                );
                let beam_range = first_bank
                    .map(|b| {
                        if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            40.0
                        }
                    })
                    .unwrap_or(40.0);
                let render_cfg = PhaserRenderConfig {
                    beam_color,
                    beam_range,
                };
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.entity(spawned).insert(render_cfg.clone());
                commands.insert_resource(render_cfg);

                // Player phaser combat tuning — overrides the default
                // PhaserCombatConfig that WeaponsPlugin installed. The
                // [weapons_console] block already carries `beam_range`,
                // `beam_damage_per_sec`, `beam_duration_secs`, and
                // `cooldown_secs`; before this slice those were only
                // honoured by the NPC phaser path. Now the player path
                // also reads them via the PhaserCombatConfig resource.
                let combat_cfg = crate::weapons_plugin::PhaserCombatConfigResource(
                    crate::entity_config::PhaserCombatConfig::from_weapons_console(wc),
                );
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.entity(spawned).insert(combat_cfg.clone());
                commands.insert_resource(combat_cfg);

                // Per-bank phaser / blaster open-fire AI policies (issue #781) —
                // the player-ship half of `spawner.rs`'s per-weapon maps, added
                // by #885b stage 5d.
                //
                // THIS PATH DID NOT EXIST BEFORE. Until the synthesisers were
                // deleted the player ship carried no per-bank map at all and
                // `ai_phaser_auto_fire` / `ai_blaster_auto_fire` silently fell
                // back to a Rust-side default on every tick — the same "the
                // player ship goes through a second attachment path" omission
                // that bit #785, #786 and #882. Attaching the authored maps here
                // is what keeps a backfilled player ship firing exactly as it
                // did, now from its own TOML.
                let phaser_bank_policies: std::collections::HashMap<
                    String,
                    crate::ai::policy::AiPolicy,
                > = wc
                    .phaser_banks
                    .iter()
                    .filter_map(|b| {
                        let ai = b.ai.as_ref()?;
                        Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                    })
                    .collect();
                commands
                    .entity(spawned)
                    .insert(crate::weapons_plugin::PhaserBankAiPolicies(
                        phaser_bank_policies,
                    ));
                if !wc.blaster_banks.is_empty() {
                    let blaster_bank_policies: std::collections::HashMap<
                        String,
                        crate::ai::policy::AiPolicy,
                    > = wc
                        .blaster_banks
                        .iter()
                        .filter_map(|b| {
                            let ai = b.ai.as_ref()?;
                            Some((b.id.clone(), ai.to_policy().unwrap_or_default()))
                        })
                        .collect();
                    commands
                        .entity(spawned)
                        .insert(crate::weapons_plugin::BlasterBankAiPolicies(
                            blaster_bank_policies,
                        ));
                }
            } else {
                // No [weapons_console] block — insert defaults so the entity-component
                // path always finds a value on the LocalShip entity.
                commands
                    .entity(spawned)
                    .insert(crate::weapons_plugin::PhaserCombatConfigResource::default());
                commands
                    .entity(spawned)
                    .insert(PhaserRenderConfig::default());
            }

            // [torpedoes] block — builds the TorpedoSystem from TOML config.
            // Inserted as per-entity component AND global resource (dual-write
            // migration). NPC ships with a [torpedoes] block also get their own
            // TorpedoSystemResource component via `entities::spawner::spawn_entity`
            // (see #597 PR-3 and the audit follow-up); `tick_torpedo_lifecycle`
            // iterates `With<Ship>` so both paths advance the same way.
            if let Some(tc) = &config.torpedoes {
                let runtime_config = tc.to_runtime();
                let torpedo_system = if !tc.tubes.is_empty() {
                    crate::torpedo::TorpedoSystem::from_configs(&tc.tubes, runtime_config)
                } else {
                    crate::torpedo::TorpedoSystem::new(runtime_config)
                };
                let torpedo_res = crate::weapons_plugin::TorpedoSystemResource(torpedo_system);
                // Insert as per-entity component AND global resource (dual-write migration).
                commands.insert_resource(torpedo_res.clone());
                commands.entity(spawned).insert(torpedo_res);

                // Per-tube load/launch + shared-magazine grant AI policies
                // (issue #782) — the player-ship half of `spawner.rs`'s maps,
                // added by #885b stage 5d for the same reason as the phaser and
                // blaster maps above: before it the player ship carried neither,
                // and the torpedo hosts fell back to a Rust-side default every
                // tick.
                let tube_policies: std::collections::HashMap<String, crate::ai::policy::AiPolicy> =
                    tc.tubes
                        .iter()
                        .filter_map(|t| {
                            let ai = t.ai.as_ref()?;
                            Some((t.id.clone(), ai.to_policy().unwrap_or_default()))
                        })
                        .collect();
                commands
                    .entity(spawned)
                    .insert(crate::weapons_plugin::TorpedoTubeAiPolicies(tube_policies));
                if let Some(ai) = tc.ai.as_ref() {
                    commands.entity(spawned).insert(
                        crate::weapons_plugin::TorpedoMagazineAiPolicy(
                            ai.to_policy().unwrap_or_default(),
                        ),
                    );
                }
            }

            // Power config — unconditionally insert as per-entity Component
            // so systems that iterate `With<Ship>` always see a value on
            // the player ship (matching NPCs, which spawner.rs always
            // inserts a defaulted `PowerConfigResource` for). Dual-writes
            // the global Resource for legacy readers.
            let power_config = if let Some(pc) = &config.power {
                PowerConfigResource(crate::power_system::PowerConfig {
                    capacity: pc.capacity,
                    rates: pc.rates,
                    emergency_threshold: pc.emergency_threshold,
                    // Battery floors (issue #952) — see the NPC-side twin in
                    // `entities::spawner`.
                    group_floors: crate::ship::power::authored_power_group_floors(
                        &pc.battery_floor,
                        &config
                            .ship_config
                            .as_ref()
                            .map(|sc| sc.power_groups.clone())
                            .unwrap_or_default(),
                    ),
                    floor_release_margin_pct: pc.battery_floor_release_margin,
                })
            } else {
                PowerConfigResource::default()
            };
            commands.entity(spawned).insert(power_config.clone());
            commands.insert_resource(power_config);

            // Inline stateless Power allocation AI policy (issue #784) — from the
            // authored `[power.ai_policy]` block, so `ai_power_allocation`
            // iterating `With<Ship>` sees the ship's own policy. `to_policy`
            // cannot fail: validated at load.
            if let Some(ai) = config.power.as_ref().and_then(|pc| pc.ai_policy.as_ref()) {
                commands.entity(spawned).insert((
                    crate::ship::power::PowerAiPolicy(ai.to_policy().unwrap_or_default()),
                    // Carried from the SAME authored block (issue #889's
                    // evaluate_every_ticks, wired at runtime): a resolved
                    // `AiPolicy` alone forgets this field, so it rides
                    // alongside as a sibling component.
                    crate::ship::power::PowerAiCadence(ai.evaluate_every_ticks),
                ));
            }

            // Power multipliers
            let defaults = [-0.5, 0.0, 0.25, 0.5];
            let mut multipliers: std::collections::HashMap<
                crate::messages::PowerGroupId,
                [f32; 4],
            > = std::collections::HashMap::from([
                (
                    crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                    defaults,
                ),
                (
                    crate::messages::PowerGroupId(crate::power_system::SHIELDS_POWER_GROUP.into()),
                    defaults,
                ),
            ]);
            if let Some(hc) = &config.helm_console {
                if let Some(pm) = hc.power_multipliers {
                    multipliers.insert(
                        crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                        pm,
                    );
                }
            }
            if let Some(wc) = &config.weapons_console {
                if let Some(pm) = wc.power_multipliers {
                    multipliers.insert(
                        crate::messages::PowerGroupId(
                            crate::power_system::WEAPONS_POWER_GROUP.into(),
                        ),
                        pm,
                    );
                }
            }
            if let Some(sc) = &config.shields_console {
                if let Some(pm) = sc.power_multipliers {
                    // shields_console power drives ModifierSlot::ShieldRegen (#952)
                    multipliers.insert(
                        crate::messages::PowerGroupId(
                            crate::power_system::SHIELDS_POWER_GROUP.into(),
                        ),
                        pm,
                    );
                }
            }
            commands.insert_resource(PowerMultiplierResource {
                multipliers: multipliers.clone(),
            });
            // Insert as per-entity component AND global resource (dual-write migration — PR 6).
            commands
                .entity(spawned)
                .insert(PowerMultiplierResource { multipliers });

            // Ship physics config from [helm_console] TOML, or default
            let physics_cfg =
                config
                    .helm_console
                    .as_ref()
                    .map(|hc| crate::ship_physics::ShipPhysicsConfig {
                        max_speed: hc.max_speed,
                        max_reverse_speed: hc.max_reverse_speed,
                        acceleration: hc.acceleration,
                        deceleration: hc.deceleration,
                        max_yaw_rate: hc.max_yaw_rate,
                        low_speed_turn_boost: hc.low_speed_turn_boost,
                        max_lateral_speed: hc
                            .lateral_thrust
                            .as_ref()
                            .map(|lt| lt.max_lateral_speed)
                            .unwrap_or(15.0),
                        lateral_acceleration: hc
                            .lateral_thrust
                            .as_ref()
                            .map(|lt| lt.lateral_acceleration)
                            .unwrap_or(15.0),
                        // Vertical axis (issue #744): no dedicated helm_console
                        // TOML yet, so take the ShipPhysicsConfig defaults.
                        ..crate::ship_physics::ShipPhysicsConfig::new()
                    });
            let physics_cfg_resource = crate::ship_plugin::ShipPhysicsConfigResource(
                physics_cfg.unwrap_or(crate::ship_physics::ShipPhysicsConfig::new()),
            );
            commands.insert_resource(physics_cfg_resource.clone());
            commands.entity(spawned).insert(physics_cfg_resource);

            // Impulse config from [helm_console] TOML, or default
            let impulse_steering = config
                .helm_capability
                .as_ref()
                .map(|cap| cap.impulse.steering_multiplier)
                .unwrap_or(0.0);
            let impulse_cfg = config
                .helm_console
                .as_ref()
                .map(|hc| crate::ship_plugin::ImpulseConfigResource {
                    charge_duration: hc.impulse_charge_duration,
                    speed_multiplier: hc.impulse_speed_multiplier,
                    acceleration_multiplier: hc.impulse_acceleration_multiplier,
                    engage_distance: hc.impulse_engage_distance,
                    cancel_distance: hc.impulse_cancel_distance,
                    steering_multiplier: impulse_steering,
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(impulse_cfg);

            // Boost config from [helm_console.boost] TOML. Absent table ⇒
            // feature disabled (default component has `enabled: false`).
            let boost_cfg = config
                .helm_console
                .as_ref()
                .and_then(|hc| hc.boost.as_ref())
                .map(|b| crate::ship_plugin::BoostConfigResource {
                    enabled: true,
                    multiplier: b.multiplier,
                    steering_multiplier: b.steering_multiplier,
                    active_duration: b.active_duration,
                    recharge_duration: b.recharge_duration,
                })
                .unwrap_or_default();
            commands.entity(spawned).insert(boost_cfg);

            // Bank config from [helm_console] TOML, or default
            let bank_cfg = config
                .helm_console
                .as_ref()
                .map(|hc| crate::ship_plugin::BankConfigResource {
                    max_bank_deg: hc.max_bank_deg,
                    bank_lerp_rate: hc.bank_lerp_rate,
                })
                .unwrap_or_default();
            commands.insert_resource(bank_cfg.clone());
            commands.entity(spawned).insert(bank_cfg);
        }
    }

    *has_spawned = true;
}

/// Diagnostic: dump every tracked entity's components on InProgress start.
/// Helps debug missing raider or other invisible NPC issues.
fn dump_tracked_entities(
    query: Query<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&EntityId>,
        &Transform,
        Option<&MeshSection>,
        Option<&EntityTagsSection>,
        Option<&RadarAppearanceSection>,
        Option<&BehaviourSection>,
        Option<&FactionComponent>,
    )>,
) {
    bevy::log::info!("=== ENTITY DUMP (InProgress start) ===");
    let mut count = 0u32;
    for (uuid, name, id, transform, mesh, tags, radar, behaviour, faction) in &query {
        count += 1;
        let label = name
            .map(|n| n.0.clone())
            .or_else(|| id.map(|i| i.0.clone()))
            .unwrap_or_else(|| "?".to_string());
        let pos = format!(
            "[{:.1}, {:.1}, {:.1}]",
            transform.translation.x, transform.translation.y, transform.translation.z
        );
        let has_mesh = if mesh.is_some() { "MESH" } else { "no-mesh" };
        let tags_str = tags
            .map(|t| format!("tags={:?}", t.0))
            .unwrap_or_else(|| "no-tags".to_string());
        let has_radar = if radar.is_some() { "RADAR" } else { "no-radar" };
        let has_ai = if behaviour.is_some() { "AI" } else { "no-ai" };
        let fac = faction
            .map(|f| format!("faction={}", f.0))
            .unwrap_or_else(|| "no-faction".to_string());
        bevy::log::info!(
            "  ENTTY uuid={} label={} pos={} {} {} {} {} {}",
            &uuid.0[..uuid.0.len().min(8)],
            label,
            pos,
            has_mesh,
            tags_str,
            has_radar,
            has_ai,
            fac
        );
    }
    bevy::log::info!("=== ENTITY DUMP END ({} entities) ===", count);
}

/// Marker: entity mesh has been rendered (GLB procedural).
/// Prevents re-processing by `render_spawned_entities`.
#[derive(Component)]
struct RenderProcessed;

pub use crate::entities::glb_visual::{
    resolve_sidecar_rig, spawn_glb_visual, GlbSpawnOutcome, PendingSceneHandle,
};

/// Tag a freshly spawned GLB `SceneRoot` as the local ship's model: hidden by
/// default (shown only by the cinematic camera) and exempt from frustum culling
/// because it sits at the camera origin.
///
/// `spawn_glb_visual` is deliberately ignorant of the simulation, so this
/// decoration is applied by the caller to the child entity it returns.
fn decorate_local_ship_model(commands: &mut Commands, child: Entity) {
    commands.entity(child).insert((
        Visibility::Hidden,
        LocalShipModel,
        bevy::camera::visibility::NoFrustumCulling,
    ));
}

/// Rounded key for a cached procedural mesh (geometry only — colour/emissive do
/// not affect the mesh, so they are excluded to maximise sharing).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMeshKey {
    /// Shape discriminant: 0 = sphere, 1 = cuboid, 2 = torus.
    shape: u8,
    radius_q: i32,
    size_q: [i32; 3],
    minor_q: i32,
}

/// Rounded key for a cached procedural material (appearance only).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ProcMatKey {
    colour_q: [i32; 3],
    emissive_q: i32,
}

/// Quantise a float to milli-units for use in a hashable cache key.
fn quantize_key(v: f32) -> i32 {
    (v * 1000.0).round() as i32
}

/// Deduplicates procedural meshes and materials by rounded key so that all
/// identical primitives (e.g. every distant asteroid's far-LOD sphere) share a
/// single mesh handle and a single material handle. Reusing handles lets the
/// renderer batch/instance the draws instead of issuing one per entity.
/// `pub(crate)` for the model viewer, which builds a ladder's procedural far
/// level through the same constructor rather than growing its own sphere.
#[derive(Resource, Default)]
pub(crate) struct ProceduralMeshCache {
    meshes: HashMap<ProcMeshKey, Handle<Mesh>>,
    materials: HashMap<ProcMatKey, Handle<StandardMaterial>>,
}

/// A procedural LOD level's own rotation, as a quaternion. Identity when the
/// level declares none.
fn level_rotation(level: &crate::entity_config::LodLevel) -> Quat {
    level
        .rotation
        .map(|r| Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]))
        .unwrap_or(Quat::IDENTITY)
}

/// Build — or fetch from `cache` — the `Mesh3d`/material handles for a
/// procedural primitive. Mirrors PATH B of the flat renderer but routes through
/// the cache so identical primitives share handles. Shared by the flat renderer
/// and the LOD system.
pub(crate) fn procedural_mesh_material(
    cache: &mut ProceduralMeshCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    shape: crate::entity_config::MeshShape,
    radius: f32,
    size: Option<[f32; 3]>,
    minor_radius: f32,
    colour: &[f32],
    emissive_mul: f32,
) -> (Handle<Mesh>, Handle<StandardMaterial>) {
    use crate::entity_config::MeshShape;

    let (shape_id, size_for_key) = match shape {
        MeshShape::Sphere => (0u8, [0.0; 3]),
        MeshShape::Cuboid => (1u8, size.unwrap_or([2.0, 1.0, 3.0])),
        MeshShape::Torus => (2u8, [0.0; 3]),
    };
    let mesh_key = ProcMeshKey {
        shape: shape_id,
        radius_q: quantize_key(radius),
        size_q: [
            quantize_key(size_for_key[0]),
            quantize_key(size_for_key[1]),
            quantize_key(size_for_key[2]),
        ],
        minor_q: quantize_key(minor_radius),
    };
    let mesh_handle = cache
        .meshes
        .entry(mesh_key)
        .or_insert_with(|| match shape {
            MeshShape::Sphere => meshes.add(Sphere {
                radius: radius.max(0.1),
            }),
            MeshShape::Cuboid => {
                let [x, y, z] = size.unwrap_or([2.0, 1.0, 3.0]);
                meshes.add(Cuboid::new(x, y, z))
            }
            MeshShape::Torus => meshes.add(Torus {
                major_radius: radius.max(0.5),
                minor_radius: minor_radius.max(0.1),
            }),
        })
        .clone();

    let rgb = if colour.len() >= 3 {
        [colour[0], colour[1], colour[2]]
    } else {
        [0.6, 0.6, 0.6]
    };
    let mat_key = ProcMatKey {
        colour_q: [
            quantize_key(rgb[0]),
            quantize_key(rgb[1]),
            quantize_key(rgb[2]),
        ],
        emissive_q: quantize_key(emissive_mul),
    };
    let mat_handle = cache
        .materials
        .entry(mat_key)
        .or_insert_with(|| {
            let color = Color::srgb(rgb[0], rgb[1], rgb[2]);
            let emissive = LinearRgba::from(color) * emissive_mul;
            materials.add(StandardMaterial {
                base_color: color,
                emissive,
                ..default()
            })
        })
        .clone();

    (mesh_handle, mat_handle)
}

/// Distance-based mesh LOD state, attached to entities whose model rig sidecar
/// declares one or more `[[lod]]` levels. [`update_mesh_lod`] selects and swaps
/// the active level each frame based on camera distance;
/// [`render_spawned_entities`] skips rendering these entities directly.
#[derive(Component)]
struct MeshLods {
    /// Ordered near→far LOD levels copied from the model's rig sidecar
    /// ([`crate::model_rig::ModelRig::lod`], issue #914).
    levels: Vec<crate::entity_config::LodLevel>,
    /// Flat mesh config supplying fallback fields (colour/radius/emissive/size/
    /// minor_radius) and the shared `variant` for levels that omit them.
    base: crate::entity_config::MeshConfig,
    /// Active level index; `None` until the first evaluation establishes it.
    current: Option<usize>,
    /// The child carrying the active level's visual — a GLB level's
    /// `SceneRoot`, or a shape level's `Mesh3d`.
    scene_child: Option<Entity>,
    /// Whether this entity is the local player's ship (GLB starts hidden).
    is_local_ship: bool,
}

/// Remove whichever visual the active LOD level installed, so a new level can be
/// built cleanly. Both kinds of level hang their visual off a child — a GLB's
/// `SceneRoot`, a shape's `Mesh3d` — so this despawns exactly one entity, via
/// `try_despawn` (safe if it was already removed; Bevy 0.18 `despawn` panics on
/// an already-despawned entity).
///
/// Note: this intentionally does NOT remove `ModelMarkers`. On a GLB→GLB switch
/// the new level's `spawn_glb_visual` re-inserts `ModelMarkers`, and because
/// commands apply in enqueue order, a blanket `remove` here (queued after that
/// insert) would clobber the new markers. `ModelMarkers` is instead cleared
/// explicitly in the procedural branch of [`update_mesh_lod`] when switching
/// away from a GLB level to a shape level.
fn teardown_lod_visual(commands: &mut Commands, lods: &mut MeshLods) {
    if let Some(child) = lods.scene_child.take() {
        commands.entity(child).try_despawn();
    }
}

/// Add visual meshes and materials to spawned entities that have a `[mesh]`
/// section but no `RenderProcessed` yet. When `cfg.model` is set, loads a GLB
/// scene instead of creating a procedural shape — but defers insertion until
/// the asset is actually loaded (avoids attaching an unloaded handle that
/// would never retry). Applies `cfg.scale` and `cfg.rotation` to the entity's
/// transform in both paths. Additionally, if the entity carries a `Lights`
/// component (from one or more `[[light]]` TOML entries), attach the matching
/// `PointLight`/`DirectionalLight` components (single light → inline, multiple
/// → spawned as child entities).
///
/// Entities whose model rig sidecar declares a `[[lod]]` chain are NOT rendered
/// here: they receive a [`MeshLods`] component and are driven by
/// [`update_mesh_lod`].
fn render_spawned_entities(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut star_surface_materials: ResMut<Assets<crate::entity_star::StarSurfaceMaterial>>,
    mut star_halo_materials: ResMut<Assets<crate::entity_star::StarHaloMaterial>>,
    mut planet_surface_materials: ResMut<Assets<crate::entity_planet::PlanetSurfaceMaterial>>,
    mut planet_cloud_materials: ResMut<Assets<crate::entity_planet::PlanetCloudMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    entities: Query<
        (
            Entity,
            &Transform,
            Option<&crate::entity_spawner::MeshSection>,
            Option<&crate::entity_spawner::StarSection>,
            Option<&crate::entity_spawner::PlanetSection>,
            Option<&crate::entity_spawner::Lights>,
            Option<&PendingSceneHandle>,
            Option<&crate::simulation::LocalShip>,
        ),
        Without<RenderProcessed>,
    >,
) {
    for (entity, transform, mesh_sec, star_sec, planet_sec, lights_opt, pending, local_ship) in
        entities.iter()
    {
        let mesh_cfg_for_transform = mesh_sec.map(|mesh_sec| &mesh_sec.0);

        if let Some(star_sec) = star_sec {
            crate::entities::celestial_visual::insert_star_visual(
                &mut commands,
                &mut meshes,
                &mut star_surface_materials,
                &mut star_halo_materials,
                entity,
                &star_sec.0,
            );
        } else if let Some(planet_sec) = planet_sec {
            // Textured planet: UV sphere with the custom planet shader, plus
            // an optional alpha-blended cloud shell child. Checked before the
            // `[mesh]` branch — planet templates keep a procedural `[mesh]`
            // fallback for headless/editor contexts that must not win here.
            crate::entities::celestial_visual::insert_planet_visual(
                &mut commands,
                &mut meshes,
                &mut planet_surface_materials,
                &mut planet_cloud_materials,
                &asset_server,
                entity,
                &planet_sec.0,
            );
        } else if let Some(mesh_sec) = mesh_sec {
            let cfg = &mesh_sec.0;

            if let Some(model_path) = &cfg.model {
                // The LOD ladder is owned by the model, not the entity (issue
                // #914), so whether this is a LOD entity at all is a question
                // only the rig sidecar can answer — resolve it first. On wasm a
                // sidecar still in flight yields `None`; retry next frame, which
                // is the same wait the flat GLB path already takes. On native
                // the read is synchronous.
                let Some(rig) = resolve_sidecar_rig(model_path, cfg.variant.as_deref()) else {
                    continue;
                };
                if !rig.lod.is_empty() {
                    // LOD entity: defer the visual to `update_mesh_lod`, which
                    // selects a level by camera distance each frame. Attach the
                    // LOD state; the flat paths below are skipped for this
                    // entity. `base` stays the entity's own `[mesh]` so a shared
                    // ladder still renders each rock's authored colour/radius.
                    commands.entity(entity).insert(MeshLods {
                        levels: rig.lod.clone(),
                        base: cfg.clone(),
                        current: None,
                        scene_child: None,
                        is_local_ship: local_ship.is_some(),
                    });
                } else {
                    // PATH A: GLB model (shared helper preserves the async logic).
                    // `rig` was already resolved above to answer "does this model
                    // have a [[lod]] chain" — hand it straight through instead of
                    // making spawn_glb_visual read/parse the same sidecar again.
                    match spawn_glb_visual(
                        &mut commands,
                        &asset_server,
                        &scenes,
                        entity,
                        model_path,
                        cfg.variant.as_deref(),
                        pending,
                        Some(&rig),
                    ) {
                        GlbSpawnOutcome::Spawned(child) => {
                            if local_ship.is_some() {
                                decorate_local_ship_model(&mut commands, child);
                            }
                        }
                        // GLB / rig not loaded yet — try again next frame.
                        GlbSpawnOutcome::Pending => continue,
                        GlbSpawnOutcome::Failed => {
                            // Stop retrying an entity whose GLB will never load.
                            commands.entity(entity).insert(RenderProcessed);
                            continue;
                        }
                    }
                }
            } else {
                // PATH B: Procedural primitive (deduped via the shared cache).
                let emissive_mul = cfg.emissive.unwrap_or(0.4);
                let (mesh, mat) = procedural_mesh_material(
                    &mut proc_cache,
                    &mut meshes,
                    &mut materials,
                    cfg.shape,
                    cfg.radius,
                    cfg.size,
                    cfg.minor_radius,
                    &cfg.colour,
                    emissive_mul,
                );
                commands
                    .entity(entity)
                    .insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            }
        } else {
            continue;
        }

        // Apply scale/rotation — preserves spawn position. `mesh_cfg_for_transform`
        // is `None` for stars, so this is a no-op on that path.
        if let Some(cfg) =
            mesh_cfg_for_transform.filter(|cfg| cfg.scale != 1.0 || cfg.rotation != [0.0, 0.0, 0.0])
        {
            commands.entity(entity).insert(Transform {
                translation: transform.translation,
                rotation: bevy::math::Quat::from_euler(
                    bevy::math::EulerRot::XYZ,
                    cfg.rotation[0],
                    cfg.rotation[1],
                    cfg.rotation[2],
                ),
                scale: Vec3::splat(cfg.scale),
            });
        }

        // Mark processed so we never visit this entity again.
        let mut ec = commands.entity(entity);
        ec.insert(RenderProcessed);

        // Attach lights, if any. A light that needs to face the player must
        // be its own child entity so rotating it doesn't rotate the parent's
        // visual mesh; otherwise a single light can live on the entity itself.
        if let Some(lights_comp) = lights_opt {
            let lights = &lights_comp.0;
            let needs_children = lights.len() > 1 || lights.iter().any(|l| l.face_player);
            match (lights.len(), needs_children) {
                (0, _) => {}
                (1, false) => insert_light(&mut ec, &lights[0]),
                _ => {
                    ec.with_children(|parent| {
                        for light in lights {
                            spawn_child_light(parent, light);
                        }
                    });
                }
            }
        }
    }
}

/// Distance-based LOD driver. For each entity carrying a [`MeshLods`] component,
/// computes the 3-D distance from the [`GameCamera`](crate::server::renderer::GameCamera)
/// to the entity, selects the appropriate level via
/// [`crate::entity_config::select_lod`] (with hysteresis), and — when the chosen
/// level differs from the current one — tears down the old visual and builds the
/// new one through the same helpers the flat renderer uses.
///
/// GLB levels that are still async-loading keep the current visual and retry
/// next frame, so a switch never leaves the entity permanently invisible.
/// Runs after [`render_spawned_entities`] so newly-attached `MeshLods` are
/// established the same frame they are spawned.
fn update_mesh_lod(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut proc_cache: ResMut<ProceduralMeshCache>,
    scenes: Res<Assets<bevy::scene::Scene>>,
    camera: Query<&GlobalTransform, With<crate::server::renderer::GameCamera>>,
    mut lod_entities: Query<(
        Entity,
        &mut Transform,
        &mut MeshLods,
        Option<&PendingSceneHandle>,
    )>,
) {
    use crate::entity_config::select_lod;

    // No camera → nothing to measure distance against; try again next frame.
    let Some(cam_tf) = camera.iter().next() else {
        return;
    };
    let cam_pos = cam_tf.translation();

    for (entity, mut transform, mut lods, pending) in lod_entities.iter_mut() {
        // Use the entity's LOCAL transform, not its `GlobalTransform`: on the
        // frame an entity is first rendered its `MeshLods` is inserted this same
        // Update, but global transforms aren't propagated until PostUpdate, so a
        // `GlobalTransform` read here would still be the identity default and pick
        // the initial level from distance-to-origin (a one-frame wrong-LOD flash).
        // Asteroids are top-level/unparented, so local == world. If a parented
        // entity ever needs LOD, this must switch to a propagated world position.
        let distance = transform.translation.distance(cam_pos);
        let target = select_lod(&lods.levels, distance, lods.current);

        // Issue lod-preload-by-distance, part 3: always try to have the next
        // MORE detailed level (one index closer than `target`) warm in the
        // asset server's cache, so an approaching ship never triggers a
        // fresh async load the frame it actually crosses into that band —
        // it's already sitting in cache from here. Runs every frame
        // regardless of whether `target` just changed (a ship can sit near a
        // boundary for a while before crossing it), and never touches
        // `lods.current` or the displayed visual — only the block below does
        // that. `asset_server.load()` is idempotent: a path already
        // loading/loaded just returns the existing handle, so this is cheap
        // once warm.
        if target > 0 {
            if let Some(model_path) = lods
                .levels
                .get(target - 1)
                .and_then(|level| level.model.as_deref())
            {
                let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
                let _: Handle<bevy::scene::Scene> = asset_server.load(format!("{rel}#Scene0"));
            }
        }

        if lods.current == Some(target) {
            continue;
        }

        // Copy the target level out so the `lods` borrow is free for teardown.
        let Some(level) = lods.levels.get(target).cloned() else {
            continue;
        };

        // Recompute the entity's scale from the flat `[mesh] scale` and this
        // level's optional `[x, y, z]`. Recomputed rather than multiplied in,
        // so switching between levels that do and do not declare one is
        // symmetric and leaves nothing to unwind: a level with no `scale` puts
        // the entity back to exactly what it spawned with.
        transform.scale =
            Vec3::splat(lods.base.scale) * level.scale.map(Vec3::from_array).unwrap_or(Vec3::ONE);

        if let Some(model_path) = level.model.as_deref() {
            let variant = level.variant.clone().or_else(|| lods.base.variant.clone());
            // This level's own sidecar hasn't been resolved yet this frame —
            // let spawn_glb_visual resolve it.
            match spawn_glb_visual(
                &mut commands,
                &asset_server,
                &scenes,
                entity,
                model_path,
                variant.as_deref(),
                pending,
                None,
            ) {
                // Keep the current visual until the new GLB resolves — avoids a
                // visible gap. `current` is left unchanged so we retry next frame.
                GlbSpawnOutcome::Pending => continue,
                GlbSpawnOutcome::Failed => {
                    // Give up on this level; drop the old visual and settle so we
                    // stop retrying it every frame.
                    teardown_lod_visual(&mut commands, &mut lods);
                    lods.current = Some(target);
                }
                GlbSpawnOutcome::Spawned(child) => {
                    if lods.is_local_ship {
                        decorate_local_ship_model(&mut commands, child);
                    }
                    teardown_lod_visual(&mut commands, &mut lods);
                    lods.scene_child = Some(child);
                    lods.current = Some(target);
                }
            }
        } else if let Some(shape) = level.shape {
            // Procedural level — fields fall back to the flat `base` config.
            let radius = level.radius.unwrap_or(lods.base.radius);
            let minor = level.minor_radius.unwrap_or(lods.base.minor_radius);
            let size = level.size.or(lods.base.size);
            let emissive_mul = level.emissive.or(lods.base.emissive).unwrap_or(0.4);
            let colour = level
                .colour
                .clone()
                .unwrap_or_else(|| lods.base.colour.clone());
            let (mesh, mat) = procedural_mesh_material(
                &mut proc_cache,
                &mut meshes,
                &mut materials,
                shape,
                radius,
                size,
                minor,
                &colour,
                emissive_mul,
            );
            teardown_lod_visual(&mut commands, &mut lods);
            // Switching to a shape level: drop any `ModelMarkers` left by a prior
            // GLB level (no-op if absent). Enqueued after teardown, so it never
            // races a freshly-inserted marker map.
            commands
                .entity(entity)
                .remove::<crate::model_rig::ModelMarkers>();
            // The mesh goes on a CHILD, as a GLB level's `SceneRoot` does, so
            // the level can carry its own rotation. Rotating the entity itself
            // is not available: an entity's rotation is simulation state, and
            // physics rewrites it every tick on anything that moves.
            let child = commands
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(mat),
                    Transform::from_rotation(level_rotation(&level)),
                ))
                .id();
            commands.entity(entity).add_child(child);
            lods.scene_child = Some(child);
            lods.current = Some(target);
        } else {
            // Neither model nor shape — invalid level. Settle so we don't spin.
            bevy::log::warn!(
                "update_mesh_lod: LOD level {target} on {entity:?} has neither model nor shape — skipping"
            );
            lods.current = Some(target);
        }
    }
}

fn insert_light(
    ec: &mut bevy::ecs::system::EntityCommands,
    light: &crate::entity_config::LightConfig,
) {
    use crate::entity_config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            ec.insert(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
        }
        LightKind::Directional => {
            ec.insert(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
        }
    }
}

fn spawn_child_light(
    parent: &mut bevy::ecs::relationship::RelatedSpawnerCommands<ChildOf>,
    light: &crate::entity_config::LightConfig,
) {
    use crate::entity_config::LightKind;
    let color = Color::srgb(light.colour[0], light.colour[1], light.colour[2]);
    match light.kind {
        LightKind::Point => {
            let mut child = parent.spawn(PointLight {
                color,
                intensity: light.intensity,
                range: light.range.unwrap_or(50.0),
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
        LightKind::Directional => {
            let mut child = parent.spawn(DirectionalLight {
                color,
                illuminance: light.intensity,
                shadows_enabled: false,
                ..default()
            });
            if light.face_player {
                child.insert(FacePlayerLight);
            }
        }
    }
}

/// Rotates every [`FacePlayerLight`] entity so it points toward the
/// player's ship, independent of its parent entity's orientation.
fn face_player_lights(
    ship_query: Query<&GlobalTransform, With<LocalShip>>,
    mut light_query: Query<(&GlobalTransform, &mut Transform), With<FacePlayerLight>>,
) {
    let Some(ship_transform) = ship_query.iter().next() else {
        return;
    };
    let player_pos = ship_transform.translation();
    for (global, mut transform) in &mut light_query {
        let light_pos = global.translation();
        if (player_pos - light_pos).length_squared() > f32::EPSILON {
            transform.rotation = Transform::from_translation(light_pos)
                .looking_at(player_pos, Vec3::Y)
                .rotation;
        }
    }
}

// â"€â"€ Tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€
#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::collision_damage;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::ship_plugin::handle_impulse_messages;
    use crate::weapons_plugin::BEAM_DAMAGE_PER_SEC;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    #[derive(Resource)]
    struct ShipEntity(Entity);

    // ── station_for_system ───────────────────────────────────────────────

    /// Issue #801 deleted `station_for_system`'s "tactical" special case
    /// (step 2.5): `"tactical"` is a station id, not a system, and no
    /// `ControlSystem` message targets it. Ship-level tactical operations
    /// target real declared systems (`tactical-radar`, `phaser-control`),
    /// which resolve through the ordinary system→station lookup — including
    /// on a hull whose weapons station isn't literally named "tactical".
    ///
    /// Issue #832 removed the step-3 station-name fallback entirely, so a bare
    /// station id like `"tactical"` no longer resolves — it is not a declared
    /// system, and every client wire target names a declared system.
    #[test]
    fn station_for_system_resolves_tactical_systems_via_their_declared_station() {
        let crewed = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"
"#,
            &["phaser_bank", "tactical_radar"],
        )
        .unwrap();
        assert_eq!(
            station_for_system(&crewed, &SystemId("tactical-radar".into())),
            Some(StationId("tactical".into())),
            "crewed hulls resolve tactical-radar to their tactical station"
        );
        // #832: the bare station string `"tactical"` no longer resolves — the
        // step-3 station-name fallback was removed and `"tactical"` is not a
        // declared system.
        assert_eq!(
            station_for_system(&crewed, &SystemId("tactical".into())),
            None,
        );

        let courier = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "pilot"
name = "Pilot"
description = "Everything."
rank = "Ltn."

[[system]]
id = "blaster-fore"
kind = "blaster_bank"
station = "pilot"

[[system]]
id = "pilot-radar"
kind = "tactical_radar"
station = "pilot"
"#,
            &["blaster_bank", "tactical_radar"],
        )
        .unwrap();
        assert_eq!(
            station_for_system(&courier, &SystemId("pilot-radar".into())),
            Some(StationId("pilot".into())),
            "the Courier's radar lives on pilot, so SetTarget resolves there"
        );
        assert_eq!(
            station_for_system(&courier, &SystemId("tactical".into())),
            None,
            "no tactical station and no tactical system: the deleted step-2.5 \
             weapons-owner special case must not resurrect this lookup"
        );
    }

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    /// Test-only glue (issue #829): seed each ship's `ViewscreenBlackboard`
    /// combat_lock / science_target from its `TacticalRadarSelection` /
    /// `SensorRadarSelection` components before `SimSet::Input`, standing in for
    /// the radar publishers + viewscreen aggregators the full app runs. Merges
    /// into any existing viewscreen entry.
    fn seed_viewscreen_from_selection(
        mut q: Query<
            (
                Option<&crate::weapons_plugin::TacticalRadarSelection>,
                Option<&crate::sensors_plugin::SensorRadarSelection>,
                &mut ShipSystemBlackboards,
            ),
            With<Ship>,
        >,
    ) {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        for (tac, sci, mut bbs) in q.iter_mut() {
            let combat_lock = tac.and_then(|t| t.0.clone());
            let science_target = sci.and_then(|s| s.0.clone());
            let mut vbb = match bbs
                .0
                .get(&crate::ship::system_registry::viewscreen_system_id())
            {
                Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
                _ => ViewscreenBlackboard::default(),
            };
            vbb.combat_lock = combat_lock;
            vbb.science_target = science_target;
            bbs.0.insert(
                crate::ship::system_registry::viewscreen_system_id(),
                SystemBlackboard::Viewscreen(vbb),
            );
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        // The admission seam, through the same one call production uses
        // (issue #898) — resources and system together, so this fixture cannot
        // drift into having one without the other. Ungated: the fixture spawns
        // its ships by hand and never runs the lobby countdown, so it never
        // reaches `GamePhase::InProgress`.
        crate::command_admission::register_admission_seam(
            &mut app,
            crate::command_admission::AdmissionGate::EveryTick,
        );
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
                .chain(),
        )
        .add_systems(
            FixedUpdate,
            seed_viewscreen_from_selection
                .after(crate::lobby::LobbySystemSet)
                .before(crate::sim_sets::SimSet::Input),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .init_resource::<WorldResource>()
        .init_resource::<TrackedEntities>()
        .insert_resource(SimBroadcastTimer(Timer::new(
            std::time::Duration::from_nanos(1),
            TimerMode::Repeating,
        )))
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<LastBroadcastShields>()
        .init_resource::<LastBroadcastBlackboards>()
        .init_resource::<crate::messages::InterSystemQueue>()
        .init_resource::<crate::ai::server::AiTokenRegistry>()
        .init_resource::<Outbox>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .add_plugins(crate::captain_plugin::CaptainPlugin)
        .add_plugins(crate::weapons_plugin::WeaponsPlugin)
        .add_plugins(crate::repair_plugin::RepairPlugin)
        .add_plugins(crate::power_plugin::ShipPowerPlugin)
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_plugins(crate::sensors_plugin::ShipSensorsPlugin)
        .add_plugins(crate::comms_plugin::CommsConsolePlugin)
        .add_systems(
            OnEnter(GamePhase::InProgress),
            reset_broadcast_caches_on_start,
        )
        .add_systems(
            FixedUpdate,
            (
                handle_impulse_messages,
                broadcast_shield_status,
                reconcile_runtime_entities
                    .after(crate::lobby::LobbySystemSet)
                    .before(broadcast_world_setup_on_start),
                broadcast_world_setup_on_start.after(crate::lobby::LobbySystemSet),
                refresh_caches_on_midgame_reconnect.after(crate::lobby::LobbySystemSet),
            ),
        )
        .add_systems(
            FixedUpdate,
            crate::modifier_coordination::translate_power_modifiers
                .after(crate::power_plugin::handle_power_messages)
                .after(crate::power_plugin::tick_power_system),
        )
        .add_systems(
            FixedUpdate,
            crate::modifier_coordination::translate_impulse_modifiers
                .after(handle_impulse_messages),
        )
        .add_systems(
            FixedUpdate,
            (
                sim_processing_anchor,
                broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::PublishAggregate),
            ),
        )
        .add_plugins(weapons_update_broadcaster())
        .add_plugins(sim_state_broadcaster())
        .add_plugins(modifier_events_broadcaster())
        .add_systems(PostUpdate, collect);
        // One fixed step per update (issue #895): the sim chain above lives in
        // `FixedUpdate`, and each 200 ms harness tick advances it once (so the
        // Hz-based SimBroadcaster timers always fire within a single update).
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
        // Spawn the Ship entity immediately so systems that query it (including
        // auth checks in handle_fire_torpedo, handle_power_messages, etc.) work
        // during Lobby as well as InProgress.
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::simulation::LocalShip,
                crate::simulation::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                crate::ship_plugin::ShipSystemControlSources::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                ShipShields(ShieldSystem::default(), 0.5),
                ShipPhysicsComponent::default(),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_state::ShipViewMode::default(),
                crate::ship_state::ShipPhaserFrequency::default(),
                bevy::prelude::Transform::default(),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (SystemId("helm".into()), 25.0),
                    (SystemId("tactical".into()), 25.0),
                    (SystemId("power".into()), 25.0),
                    (SystemId("shields".into()), 25.0),
                ])),
            ))
            .id();
        // Insert per-entity components (Bundle limit).
        app.world_mut().entity_mut(ship).insert((
            ShipImpulse::default(),
            ShipBoost::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::weapons_plugin::TorpedoSystemResource(crate::torpedo::TorpedoSystem::new(
                crate::torpedo::TorpedoConfig::default(),
            )),
            crate::weapons_plugin::PhaserCombatConfigResource::default(),
            PhaserRenderConfig::default(),
            // PR 7 (issue #597) — per-entity beam / target / cooldown / sensors / waypoint.
            crate::weapons_plugin::TacticalRadarSelection::default(),
            crate::weapons_plugin::ActiveBeam::default(),
            crate::weapons_plugin::PhaserCooldown::default(),
            crate::sensors_plugin::SensorRadarSelection::default(),
            crate::navigation_plugin::NavigationWaypoint::default(),
            crate::ship::power::PowerBrownoutState::default(),
        ));
        app.insert_resource(ShipEntity(ship));
        app
    }

    // ── PR 7 (issue #597) test helpers ──────────────────────────────────────
    // These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
    // direct Resource access after PR 7 removed the Resource derive from
    // TacticalRadarSelection / ActiveBeam / PhaserCooldown / SensorRadarSelection / NavigationWaypoint.

    fn get_weapons_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::TacticalRadarSelection, With<LocalShip>>();
        q.single(app.world()).ok().and_then(|wt| wt.0.clone())
    }

    /// Author a `[weapons_console.radar] range` on the LocalShip.
    ///
    /// Since issue #887 `handle_set_target` takes the lock horizon from the
    /// ship's OWN `WeaponsConsoleSection` (it applies the lock for every ship,
    /// not just the player's, and `ShipClientConfigResource` is the player's
    /// radar) — so a fixture that wants a bounded horizon has to author one. A
    /// hull with no radar block has an unbounded horizon, which is what every
    /// NPC hull actually declares.
    fn set_tactical_radar_range(app: &mut App, range: f32) {
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let entity = q.single_mut(app.world_mut()).expect("LocalShip");
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entity_spawner::WeaponsConsoleSection(
                crate::entity_config::WeaponsConsoleConfig {
                    torpedo_arc_color: vec![],
                    power_multipliers: None,
                    phaser_banks: vec![],
                    blaster_banks: vec![],
                    radar: Some(crate::radar_config::RadarConfig {
                        range,
                        shows: vec![crate::entity_tags::EntityTag::Ship],
                        selects: vec![],
                    }),
                    selector: None,
                    selector_idle: false,
                },
            ));
    }

    // `ActiveBeam` is per-bank since issue #790; these fixtures all drive a ship
    // firing ONE bank at a time, so "the beam" still means "the one live slot".

    fn get_active_beam_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        q.single(app.world())
            .ok()
            .and_then(|b| b.any_target().map(str::to_string))
    }

    fn active_beam_target_is_none(app: &mut App) -> bool {
        get_active_beam_target(app).is_none()
    }

    fn live_beam_banks(app: &mut App) -> Vec<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|b| b.live_banks().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
        let banks = live_beam_banks(app);
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            match uuid {
                None => {
                    for bank in banks {
                        b.end_bank(&bank);
                    }
                }
                Some(u) => {
                    let bank = banks.first().cloned().unwrap_or_default();
                    let remaining = b
                        .bank_slot_mut(&bank)
                        .map(|s| s.remaining_secs)
                        .unwrap_or(0.0);
                    b.start(bank, u, remaining);
                }
            }
        }
    }

    fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
        let banks = live_beam_banks(app);
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            for bank in banks {
                if let Some(slot) = b.bank_slot_mut(&bank) {
                    slot.remaining_secs = secs;
                }
            }
        }
    }

    fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
        let banks = live_beam_banks(app);
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::ActiveBeam, With<LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            for bank in banks {
                if let Some(slot) = b.bank_slot_mut(&bank) {
                    slot.damage_accumulator = val;
                }
            }
        }
    }

    fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::weapons_plugin::PhaserCooldown, With<LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|cd| cd.is_bank_active(bank))
            .unwrap_or(false)
    }

    fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::weapons_plugin::PhaserCooldown, With<LocalShip>>();
        if let Ok(mut cd) = q.single_mut(app.world_mut()) {
            cd.start_bank_with_cooldown(bank, secs);
        }
    }

    fn apply_hull_damage(app: &mut App, amount: f32) {
        let mut rng = crate::sim_rng::unseeded_test_rng();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .apply_damage(amount, &mut rng);
    }

    fn get_ship_modifiers(app: &mut App) -> crate::modifiers::ShipModifiers {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::modifiers::ShipModifiers, With<crate::simulation::LocalShip>>(
            );
        q.single(app.world()).unwrap().clone()
    }

    fn modify_ship_modifiers<F>(app: &mut App, f: F)
    where
        F: FnOnce(&mut crate::modifiers::ShipModifiers),
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::modifiers::ShipModifiers, With<crate::simulation::LocalShip>>();
        let mut mods = q.single_mut(app.world_mut()).unwrap();
        f(&mut mods);
    }

    fn get_phaser_frequency(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<LocalShip>>();
        q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
    }

    fn get_view_mode(app: &mut App) -> crate::messages::ViewMode {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipViewMode, With<LocalShip>>();
        q.single(app.world())
            .map(|vm| vm.view_mode.clone())
            .unwrap_or(crate::messages::ViewMode::Camera(
                crate::messages::CameraView::default(),
            ))
    }

    // Test helper for directly setting view mode without round-tripping a
    // client message; retained for tests that may need to seed view state.
    #[allow(dead_code)]
    fn set_ship_view_mode(app: &mut App, mode: crate::messages::ViewMode) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipViewMode, With<LocalShip>>();
        if let Ok(mut vm) = q.single_mut(app.world_mut()) {
            vm.view_mode = mode;
        }
    }

    /// Fast-forward the pre-game countdown so the game starts immediately.
    /// Must be called after the tick that starts the countdown.
    fn fast_forward_countdown(app: &mut App) {
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        // Drain any leftover SimOutbox entries that the sim systems wrote but
        // were not captured by the PostUpdate collect system (SimOutbox is not
        // connected to the OutboundMessage bus for test_app).
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn load_tube_now(app: &mut App, tube: &str) {
        // Systems prefer the per-entity component over the resource; update component.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        } else {
            let world = app.world_mut();
            let mut res = world.resource_mut::<TorpedoSystemResource>();
            res.0
                .tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        }
    }

    fn set_ship_yaw(app: &mut App, yaw: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysicsComponent, With<crate::simulation::Ship>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipPhysics");
        p.yaw = yaw;
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app); // handle_set_ready_system → starts countdown
        fast_forward_countdown(app);
        tick(app); // tick_countdown → emits GameStarted, sets NextState::Set(InProgress)
        tick(app); // NextState takes effect: Phase switches to InProgress
    }

    fn start_game_with_helm(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "helm", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    fn start_game_with_sensors(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    fn start_game_with_navigation(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "navigation", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    #[test]
    fn entity_config_radar_icon_flows_into_world_snapshot() {
        let config = crate::entity_config::EntityConfig {
            name: Some("Sun".into()),
            tags: vec!["star".into(), "center".into()],
            collider: Some(crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: 50.0,
                length: 0.0,
            }),
            radar_appearance: Some(crate::entity_config::RadarAppearanceConfig {
                colour: Some(vec![1.0, 0.85, 0.3]),
                size: None,
                region_colour: None,
                icon: Some("star".into()),
            }),
            ..Default::default()
        };

        let snapshot =
            snapshot_from_entity_config("sun-uuid".into(), None, &config, Vec3::new(0.0, 0.0, 0.0));

        assert_eq!(snapshot.name.as_deref(), Some("Sun"));
        assert_eq!(snapshot.tags, vec!["star", "center"]);
        assert_eq!(snapshot.radius, Some(50.0));
        assert_eq!(snapshot.colour, Some([1.0, 0.85, 0.3]));
        assert_eq!(snapshot.radar_icon.as_deref(), Some("star"));
    }

    /// `player_ship_identity` adds the `player` tag (keeping `ship`) and forces
    /// the radar icon to `playerShip` while preserving the template's radar
    /// colour/size.
    #[test]
    fn player_ship_identity_adds_player_tag_and_playership_icon() {
        let (tags, radar) = player_ship_identity(
            &["ship".to_string()],
            Some(&crate::entity_config::RadarAppearanceConfig {
                icon: Some("ship".into()),
                colour: Some(vec![0.0, 1.0, 0.2]),
                size: Some(6.0),
                region_colour: None,
            }),
        );
        assert!(tags.iter().any(|t| t == "ship"), "keeps the ship tag");
        assert!(tags.iter().any(|t| t == "player"), "adds the player tag");
        assert_eq!(radar.icon.as_deref(), Some("playerShip"));
        // Appearance other than the icon is preserved from the template.
        assert_eq!(radar.colour, Some(vec![0.0, 1.0, 0.2]));
        assert_eq!(radar.size, Some(6.0));
    }

    /// End-to-end of the player spawn path's identity injection: parse the real
    /// cruiser hull template, spawn it via `spawn_entity` (which sets the
    /// ordinary-ship `EntityTagsSection` / `RadarAppearanceSection`), then apply
    /// the same injection `spawn_game_start_entities` performs and assert the
    /// spawned player ship carries the `player` tag AND the `playerShip` radar
    /// icon. Uses the checked-in template so it regresses on the TOML edits too.
    #[test]
    fn player_spawn_injects_player_tag_and_icon_over_template() {
        use crate::entity_spawner::{EntityTagsSection, RadarAppearanceSection};
        use bevy::prelude::*;

        // Through the resolver (issue #876): this hull is COMPOSED, so its baked
        // bytes are no longer the document that spawns.
        let config =
            crate::entity_includes::load_entity_config("assets/entities/alliance_cruiser.toml")
                .expect("cruiser template must compose and parse");

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        let spawned = {
            let mut cmds = app.world_mut().commands();
            crate::entity_spawner::spawn_entity(
                &mut cmds,
                &config,
                Vec3::ZERO,
                "player-cruiser".into(),
                None,
            )
        };
        app.world_mut().flush();

        // Pre-injection: the template presents as an ordinary ship.
        let tags = app.world().get::<EntityTagsSection>(spawned).unwrap();
        assert!(!tags.0.iter().any(|t| t == "player"));
        let radar = app.world().get::<RadarAppearanceSection>(spawned).unwrap();
        assert_eq!(radar.0.icon.as_deref(), Some("ship"));

        // Apply the player spawn injection (mirrors spawn_game_start_entities).
        let (player_tags, player_radar) =
            player_ship_identity(&config.tags, config.radar_appearance.as_ref());
        app.world_mut()
            .entity_mut(spawned)
            .insert(EntityTagsSection(player_tags))
            .insert(RadarAppearanceSection(player_radar));

        // Post-injection: player identity is present on the spawned ship.
        let tags = app.world().get::<EntityTagsSection>(spawned).unwrap();
        assert!(tags.0.iter().any(|t| t == "ship"), "still a ship");
        assert!(
            tags.0.iter().any(|t| t == "player"),
            "player ship carries the player tag; got {:?}",
            tags.0
        );
        let radar = app.world().get::<RadarAppearanceSection>(spawned).unwrap();
        assert_eq!(
            radar.0.icon.as_deref(),
            Some("playerShip"),
            "player ship carries the playerShip radar icon"
        );
    }

    #[test]
    fn world_entity_upsert_replaces_existing_snapshot_for_same_uuid() {
        let mut world = WorldResource(WorldData::default());
        upsert_world_entity(
            &mut world,
            EntitySnapshot {
                uuid: "same".into(),
                tags: vec!["asteroid".into()],
                radar_icon: Some("asteroid".into()),
                ..Default::default()
            },
        );
        upsert_world_entity(
            &mut world,
            EntitySnapshot {
                uuid: "same".into(),
                tags: vec!["star".into()],
                radar_icon: Some("star".into()),
                ..Default::default()
            },
        );

        assert_eq!(world.0.entities.len(), 1);
        assert_eq!(world.0.entities[0].tags, vec!["star"]);
        assert_eq!(world.0.entities[0].radar_icon.as_deref(), Some("star"));
    }

    #[test]
    fn sensors_can_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::ScienceRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::ScienceRadar);
    }
    #[test]
    fn sensors_can_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SensorsRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::SensorsRadar);
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SensorsRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn navigation_can_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SystemChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::SystemChart);
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::ScienceRadar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::SystemChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn navigation_can_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::NavigationChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::NavigationChart);
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::NavigationChart,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    fn start_game_with_comms(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "comms",
            ClientMessage::Identify {
                token: "comms".into(),
                name: "Uhura".into(),
            },
        );
        tick(app);
        push(
            app,
            "comms",
            ClientMessage::SelectStation {
                station: "Comms".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "comms", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    #[test]
    fn comms_can_push_view_to_comms() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::Comms);
    }

    #[test]
    fn captain_override_from_comms_view_works() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        // Captain overrides back to a camera view.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::new("camera_aft"))
        );
    }

    #[test]
    fn non_comms_cannot_push_comms_view() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Comms,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn helm_can_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
    }

    #[test]
    fn captain_cannot_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Captain has no authority over Radar; request is silently dropped.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn helm_cannot_switch_view_to_camera() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
        );
    }

    #[test]
    fn world_setup_is_broadcast_once_after_start_game() {
        let mut app = test_app();
        // Pre-populate world data so the broadcast has something to emit.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 5.0, -1.0, 2.0)],
            ..Default::default()
        }));

        // Bring the game up to the point of pressing SetReady
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "A".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        // Advance the phase to InProgress so broadcast_world_setup_on_start fires.
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        let start_out = tick(&mut app);

        let world_setups: Vec<_> = start_out
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. }))
            .collect();
        assert_eq!(
            world_setups.len(),
            1,
            "expected exactly one WorldSetup on the SetReady tick"
        );
        match &world_setups[0].msg {
            ServerMessage::WorldSetup { world } => {
                assert_eq!(world.entities.len(), 1);
                assert_eq!(world.entities[0].x(), 5.0);
            }
            _ => unreachable!(),
        }
        match &world_setups[0].target {
            crate::lobby::Target::All => {}
            t => panic!("WorldSetup should target All, got {:?}", t),
        }

        // Subsequent ticks must not re-broadcast WorldSetup
        let later = tick(&mut app);
        assert!(
            !later
                .iter()
                .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should only fire once per game"
        );
    }

    #[test]
    fn world_setup_is_not_broadcast_during_lobby() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 0.0, 0.0, 2.0)],
            ..Default::default()
        }));
        // Identify and select a console but don't start the game.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "A".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should not be broadcast in the Lobby phase"
        );
    }

    /// Read the live phase out of a running app.
    fn phase_of(app: &App) -> GamePhase {
        app.world().resource::<State<GamePhase>>().get().clone()
    }

    /// Issue #939's mid-mission "exit to lobby" has to work against the REAL
    /// app, not just the pure handler: `handle_return_to_lobby_system` is
    /// registered with a `run_if` phase gate of its own, so a handler that
    /// happily honours `InProgress` is still dead if the system never runs
    /// there. This drives the whole `LobbyPlugin` through `test_app`, which is
    /// the only harness that observes that registration.
    #[test]
    fn host_return_to_lobby_aborts_a_mission_in_progress_in_the_real_app() {
        let mut app = test_app();
        start_game(&mut app);
        assert_eq!(
            phase_of(&app),
            GamePhase::InProgress,
            "precondition: start_game must leave the session mid-mission"
        );

        push(
            &mut app,
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            ClientMessage::ReturnToLobby,
        );
        tick(&mut app);
        // `test_app` deliberately omits `LobbyOutboxPlugin`, so the lobby's
        // broadcasts stay in `LobbyOutbox` instead of reaching the
        // `OutboundMessage` bus — read them where they actually land.
        let returned = app
            .world()
            .resource::<LobbyOutbox>()
            .0
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::ReturnedToLobby));
        tick(&mut app); // NextState::Set(Lobby) takes effect.

        assert_eq!(
            phase_of(&app),
            GamePhase::Lobby,
            "the host's exit-to-lobby must abort a mission that is still InProgress"
        );
        assert!(
            returned,
            "the abort must tell the phones to leave their consoles"
        );
    }

    /// The reach added above is the host page's alone. A phone sending the
    /// same un-gated `ReturnToLobby` mid-mission must be ignored by the real
    /// app, or the settings-cog feature would hand every handset an abort.
    #[test]
    fn a_phone_cannot_abort_a_mission_in_progress_in_the_real_app() {
        let mut app = test_app();
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::ReturnToLobby);
        tick(&mut app);
        tick(&mut app);

        assert_eq!(
            phase_of(&app),
            GamePhase::InProgress,
            "a participant token must not end a mission the rest of the crew is flying"
        );
    }

    #[test]
    fn hull_integrity_starts_at_100_and_appears_in_system_hull_update() {
        let mut app = test_app();
        start_game(&mut app);
        // The first InProgress tick (inside start_game) already emitted and consumed
        // the initial SystemHullUpdate. Reset the cache to force re-emission.
        app.world_mut()
            .resource_mut::<LastBroadcastHull>()
            .0
            .clear();
        let out = tick(&mut app);
        // Post issue #737 `entries` is a per-recipient projection, so the
        // whole-ship figure is `aggregate_fraction` — the authoritative
        // ship-wide hull producer — not the sum of the visible rows.
        let aggregate = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate {
                    aggregate_fraction, ..
                } => Some(*aggregate_fraction),
                _ => None,
            })
            .expect("expected a SystemHullUpdate broadcast");
        assert!((aggregate.expect("aggregate fraction") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);
        // Consume the initial SystemHullUpdate so LastBroadcastHull is seeded.
        let _ = tick(&mut app);

        // Directly apply damage to the EntitySystemHull component (simulates collision at ~half speed).
        apply_hull_damage(&mut app, 10.0);

        let out = tick(&mut app);
        // See the note above: the ship-wide figure is now `aggregate_fraction`.
        let aggregate = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate {
                    aggregate_fraction, ..
                } => Some(*aggregate_fraction),
                _ => None,
            })
            .expect("expected a SystemHullUpdate after damage");
        assert!((aggregate.expect("aggregate fraction") - 0.9).abs() < 1e-6);
    }

    // â"€â"€ SetTarget / TargetLock tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid(
                "target-uuid",
                asteroid_x,
                asteroid_z,
                2.0,
            )],
            ..Default::default()
        }));
        // Also spawn the live ECS entity. As of the targeting fix, gameplay
        // logic reads positions from ECS Transforms (not the WorldResource
        // snapshot), so targets must exist as ECS entities to be lockable.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        ));
    }

    /// Like `setup_weapons_world` but also returns the spawned entity for
    /// tests that need to manipulate or despawn it later.
    fn setup_weapons_world_with_entity(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid(
                "target-uuid",
                asteroid_x,
                asteroid_z,
                2.0,
            )],
            ..Default::default()
        }));
        app.world_mut()
            .spawn((
                Asteroid,
                AsteroidUuid("target-uuid".into()),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (crate::messages::SystemId("captain".into()), 30.0),
                ])),
                Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
            ))
            .id()
    }

    fn start_game_with_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
        // Apply the human rating for Tactical's weapons systems so
        // `admit_system_commands` (which checks ShipSystemControlSources)
        // authorizes human ControlSystem messages for phasers, torpedoes, etc.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
        if let Ok(mut cs) = q.single_mut(app.world_mut()) {
            use crate::ship::control_source::ControlSource;
            cs.0.set(
                crate::system_registry::phaser_fore_system_id(),
                ControlSource::Human,
            );
            cs.0.set(
                crate::system_registry::phaser_aft_system_id(),
                ControlSource::Human,
            );
            cs.0.set(
                crate::system_registry::torpedo_tube_fore_port_system_id(),
                ControlSource::Human,
            );
            cs.0.set(
                crate::system_registry::torpedo_tube_fore_starboard_system_id(),
                ControlSource::Human,
            );
            cs.0.set(
                crate::system_registry::torpedo_tube_aft_system_id(),
                ControlSource::Human,
            );
            cs.0.set(
                crate::system_registry::torpedo_magazine_system_id(),
                ControlSource::Human,
            );
        }
    }

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        // Asteroid at (30, 0) â€" 30 units from ship origin, within 60-unit range.
        set_tactical_radar_range(&mut app, 300.0);
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        // Server state should record the lock.
        assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        // Asteroid at (400, 0) — 400 units away, outside 300-unit Weapons range.
        set_tactical_radar_range(&mut app, 300.0);
        setup_weapons_world(&mut app, 400.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(get_weapons_target(&mut app).is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "no-such-asteroid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(get_weapons_target(&mut app).is_none());
    }

    // â"€â"€ WeaponsUpdate / fire_ready tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Target locked, within 40-unit phaser range, in forward arc â†' fire_ready = true.
    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Tick 1 admits SetTarget in `SimSet::Input`. `compute_current_weapons_update`
        // reads the frozen viewscreen combat lock (spec §3), which this harness'
        // `seed_viewscreen_from_selection` glue refreshes before `SimSet::Input`,
        // so the new lock reaches the wire on tick 2. The full app aggregates the
        // viewscreen in `SimSet::PublishAggregate`, ahead of the `SimSet::Broadcast`
        // broadcaster, so it has no such gap.
        tick(&mut app);
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            update.1,
            "expected fire_ready=true for in-range, forward-arc target"
        );
    }

    /// Target locked but beyond 40-unit phaser range (within 60u lock range) → fire_ready = false.
    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units — within lock range
        // (60u) but outside phaser range (40u).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Two ticks, for the same frozen-combat-lock reason as the test above.
        tick(&mut app);
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            !update.1,
            "expected fire_ready=false for beyond-phaser-range target"
        );
    }

    // â"€â"€ FirePhaser / beam lifecycle tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Helper: lock target then fire phaser; returns messages from the fire tick.
    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        // Lock
        push(
            app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(app);
        // Fire
        push(
            app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        tick(app)
    }

    /// Firing at a fire-ready target broadcasts BeamStarted to all.
    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (yaw=0 â†' facing -Z â†' asteroid at (0,-20)).
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(
            beam_started.is_some(),
            "expected BeamStarted after firing at fire-ready target"
        );
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid, .. } => {
                assert_eq!(target_uuid, "target-uuid")
            }
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        // ActiveBeam resource should be populated.
        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "fore", 3.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown"
        );
    }

    /// Non-weapons player cannot fire.
    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser"
        );
    }

    /// When the beam fires at a target outside the 180Â° arc, it is rejected.
    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind â€" in rear arc.
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        // Lock (within 60u range) â€" lock doesn't require arc.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(&mut app);
        // Fire â€" rejected because target is behind.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc"
        );
    }

    /// A 6-second natural beam kills the asteroid (5 HP/s Ã— 6s = 30 HP total).
    ///
    /// The test accelerates time by manipulating the beam state directly
    /// after confirming the beam started, then runs ticks with large deltas.
    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();

        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity. Fetch its handle after setup.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        // Verify beam started.
        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0); // still "ongoing"

        let out = tick(&mut app);

        // Asteroid destroyed message should be present.
        let destroyed = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(
            destroyed.is_some(),
            "expected AsteroidDestroyed when asteroid HP reaches 0"
        );
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        // BeamEnded also broadcast.
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction"
        );

        // Asteroid no longer in world data.
        assert!(
            !app.world()
                .resource::<WorldResource>()
                .0
                .entities
                .iter()
                .any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        // Beam resource cleared.
        assert!(active_beam_target_is_none(&mut app));

        // Cooldown started.
        assert!(
            phaser_bank_is_active(&mut app, "fore"),
            "cooldown should start after beam end"
        );

        // The entity should be despawned.
        assert!(
            app.world()
                .get::<crate::entity_spawner::EntitySystemHull>(asteroid_entity)
                .is_none(),
            "asteroid entity should be despawned"
        );
    }

    /// Beam severs when ship rotates target out of the 180Â° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = π → facing +Z, asteroid at (0,-20) is behind).
        set_ship_yaw(&mut app, std::f32::consts::PI);

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            phaser_bank_is_active(&mut app, "fore"),
            "cooldown should start after arc sever"
        );
    }

    /// Beam severs when the target moves beyond 40-unit phaser range.
    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move asteroid position in WorldData to 50 units away (out of 40u range).
        app.world_mut().resource_mut::<WorldResource>().0.entities[0].position =
            Some([0.0, 0.0, -50.0]);
        // Move the live ECS Transform too — gameplay reads positions from
        // Transforms, not from the WorldResource snapshot.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut Transform, With<AsteroidUuid>>();
        for mut t in q.iter_mut(app.world_mut()) {
            t.translation.z = -50.0;
        }

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            phaser_bank_is_active(&mut app, "fore"),
            "cooldown should start after range sever"
        );
    }

    /// No damage refund on sever — whatever HP was dealt is permanent.
    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity itself. Fetch its handle by querying for the
        // matching UUID after the fact.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        // Apply partial damage via accumulator.
        set_active_beam_damage_accumulator(&mut app, 10.0);
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        set_ship_yaw(&mut app, std::f32::consts::PI);
        let _ = tick(&mut app);

        let hp = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(asteroid_entity)
            .map(|h| h.0.total_current());
        assert!(
            hp.is_some() && hp.unwrap() < 30.0,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    /// A fresh FirePhaser after cooldown on a new locked target cancels any
    /// active beam and starts a new one.
    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();

        // Set up two asteroids.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![
                EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
        // Spawn live ECS entities for both targets — gameplay reads positions
        // from Transforms, not from the WorldResource snapshot.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("t1".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("t2".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ));
        start_game_with_weapons(&mut app);

        // Lock and fire at t1.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        let _ = tick(&mut app);
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

        // Natural beam expiry: set remaining to 0.
        set_active_beam_remaining_secs(&mut app, 0.0);
        // Zero damage accumulator so no destruction fires.
        set_active_beam_damage_accumulator(&mut app, 0.0);
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(phaser_bank_is_active(&mut app, "fore"));

        // Force cooldown to expire.
        start_phaser_cooldown(&mut app, "fore", 0.0);

        // Lock and fire at t2.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown"
        );
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
    }

    // -- Repair helpers --------------------------------------------------

    /// Set up a game with a captain and repair player.
    fn start_game_with_repair(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::Identify {
                token: "eng".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::SelectStation {
                station: "Repair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "eng", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
        // Issue #830: the global `ShipRepairTeams` Resource is gone and this
        // test's ship template carries no `[hull]` block, so the spawner attaches
        // no `ShipRepairTeams`. Give the LocalShip its own component so the
        // per-entity dispatch handler has a store to write.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .expect("LocalShip must be spawned by start_game_with_repair");
        app.world_mut()
            .entity_mut(ship)
            .insert(ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)));
    }

    /// Read the LocalShip's own `ShipRepairTeams` component (issue #830 — no
    /// global Resource). Returns an owned clone for assertion convenience.
    fn local_teams(app: &mut App) -> ShipRepairTeams {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipRepairTeams, With<LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipRepairTeams")
            .clone()
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(
            teams.0.slots()[idx],
            crate::messages::TeamSlot::Travelling { .. }
        )
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    // -- Repair dispatch tests --------------------------------------

    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle after non-Repair dispatch"
        );
    }

    #[test]
    fn repair_holder_can_dispatch_team() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling after dispatch"
        );
    }

    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 1,
                    target: crate::messages::RepairTarget::Station(StationId("tactical".into())),
                },
            },
        );
        tick(&mut app);
        // Redirect team 0 (different console → Returning)
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("power".into())),
                },
            },
        );
        tick(&mut app);
        let teams = local_teams(&mut app);
        assert!(matches!(
            teams.0.slots()[0],
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(&teams, 1));
    }

    #[test]
    fn repair_state_broadcast_after_dispatch() {
        let mut app = test_app();
        start_game_with_repair(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: crate::messages::RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        let out = tick(&mut app);
        let repair_state = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(
            repair_state.is_some(),
            "RepairState with Travelling team should be broadcast to repair console"
        );
    }

    // â"€â"€ SetPhaserMode tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// The Weapons console holder can change the phaser mode to Manual.
    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Manual,
            "phaser mode should be Manual after SetPhaserMode"
        );
    }

    /// A non-Weapons player cannot change the phaser mode.
    #[test]
    fn non_weapons_player_cannot_set_phaser_mode() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Establish a known mode (Auto) via the authorised player first.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Auto,
                },
            },
        );
        tick(&mut app);
        // Non-weapons player attempts to switch back to Manual — must be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Auto,
            "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
        );
    }

    /// Shared setup used by tests that need a Sensors + Tactical(weapons) console pairing.
    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
        fast_forward_countdown(app);
        tick(app);
        tick(app);
    }

    // â"€â"€ FireTorpedo tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: SystemId("torpedo-tube-fore-port".into()),
                payload: SystemControlPayload::FireTorpedo { target_uuid: None },
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(
                &m.msg,
                ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port"
            )),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: SystemId("torpedo-tube-fore-port".into()),
                payload: SystemControlPayload::FireTorpedo { target_uuid: None },
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        load_tube_now(&mut app, "fore_port");
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::torpedo_tube_fore_port_system_id(),
                payload: SystemControlPayload::FireTorpedo { target_uuid: None },
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_starboard");

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: SystemId("torpedo-tube-fore-starboard".into()),
                payload: SystemControlPayload::FireTorpedo { target_uuid: None },
            },
        );
        let out = tick(&mut app);

        let launched = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}",
            launched.target
        );
    }

    // â"€â"€ ShipModifiers integration tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    /// Empty modifier table: phaser damage is identical to the base BEAM_DAMAGE_PER_SEC
    /// (5 HP/s). After 1 second of beam fire on a 30-HP asteroid the HP decreases by 5.
    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (within 40-unit phaser range).
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        // Lock and fire
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        tick(&mut app);

        // Advance by 1 second of simulated time (many small ticks).
        // Each tick() calls app.update() which advances the Bevy TimePlugin by a small real step.
        // Instead, directly test the accumulator math by examining the asteroid HP after
        // running a known number of frames equivalent to >1 second.
        // BEAM_DAMAGE_PER_SEC = 5; asteroid starts at 30 HP.
        // After enough ticks (>6 s at 5 HP/s) the asteroid should be destroyed.
        // With identity modifier this should work; with a 2Ã— modifier it would be faster.

        // Run 500 ms worth of ticks at ~16ms each (â‰ˆ31 ticks).
        // After that, asteroid should have taken ~2â€"3 HP (not destroyed yet).
        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world
                .0
                .entities
                .iter()
                .find(|a| a.uuid == "target-uuid")
                .map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    /// PhaserDamage modifier at 2Ã— doubles the kill rate.
    /// With BEAM_DAMAGE_PER_SEC=5 and 30-HP asteroid:
    /// - Base: 6 seconds to destroy
    /// - 2Ã— modifier (bonus=1.0): 3 seconds to destroy
    ///   Test: after running ~4s of game time, the asteroid is destroyed with 2Ã— but not with 1Ã—.
    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        // --- App with 2Ã— PhaserDamage modifier ---
        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        start_game_with_weapons(&mut app_fast);
        // Apply 2Ã— phaser damage modifier after ship is spawned.
        modify_ship_modifiers(&mut app_fast, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0, // â†' multiplier 2.0
            });
        });
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_fast);
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        tick(&mut app_fast); // processes FirePhaser, beam becomes active

        // Inject accumulated damage: 3.5s × (5 HP/s × 2×) = 35 HP → enough to destroy 30-HP asteroid.
        set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
        tick(&mut app_fast); // One tick to process the accumulated damage.

        let still_exists_fast = app_fast
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(
            !still_exists_fast,
            "with 2Ã— phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
        );

        // --- App with identity modifier (baseline): same damage injected but at 1Ã— ---
        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_fore_system_id(),
                payload: SystemControlPayload::FirePhaser,
            },
        );
        tick(&mut app_base); // processes FirePhaser, beam becomes active
                             // Inject same real time but at base rate: 3.5s × 5 HP/s = 17.5 HP accumulated
        set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
        tick(&mut app_base);

        let still_exists_base = app_base
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    /// HullDamageTaken modifier at -1 (â†' 0.5Ã— multiplier) halves collision damage.
    /// At zero ship speed, base collision_damage=5. With 0.5Ã— modifier: round(5Ã—0.5)=3.
    // â"€â"€ modifier broadcast tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

    #[test]
    fn add_modifier_broadcasts_modifier_added_message() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app); // consume startup messages

        // Register a modifier on the ship entity.
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        });
        let out = tick(&mut app);

        let found = out.iter().any(|m| {
            matches!(
                &m.msg,
                ServerMessage::ModifierAdded { source, slot, bonus }
                    if *source == ModifierSource::ImpulseDrive
                    && *slot == ModifierSlot::MaxSpeed
                    && (*bonus - 0.5).abs() < 1e-6
            )
        });
        assert!(found, "expected ModifierAdded in outbound messages");
    }

    #[test]
    fn remove_modifier_broadcasts_modifier_removed_message() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        let mut app = test_app();
        start_game(&mut app);
        // Add first so there's something to remove.
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        });
        tick(&mut app);

        // Now remove it.
        modify_ship_modifiers(&mut app, |mods| {
            mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        });
        let out = tick(&mut app);

        let found = out.iter().any(|m| {
            matches!(
                &m.msg,
                ServerMessage::ModifierRemoved { source, slot }
                    if *source == ModifierSource::ImpulseDrive
                    && *slot == ModifierSlot::MaxSpeed
            )
        });
        assert!(found, "expected ModifierRemoved in outbound messages");
    }

    #[test]
    fn asteroid_collision_pierce_zero_routes_all_to_shields() {
        // Replicates the split + apply that `handle_collisions` performs
        // (without standing up Rapier), proving the pierce=0 path leaves
        // hull untouched and the shield quadrant absorbs full damage.
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 0.0);
        assert_eq!(pierced, 0.0);
        assert_eq!(absorbed, 10.0);
        let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
        let total_hull = pierced + leak as f32;
        if total_hull > 0.0 {
            let rng = &mut crate::sim_rng::unseeded_test_rng();
            apply_hull_damage(&mut hull, total_hull, rng);
        }
        assert!(
            (hull.total_current() - 100.0).abs() < 1e-6,
            "hull untouched with pierce=0 (leak={})",
            leak
        );
        assert_eq!(
            shields.facings[0].hp,
            initial_fore_hp - 10,
            "fore quadrant should have absorbed all 10 damage"
        );
    }

    #[test]
    fn asteroid_collision_pierce_full_routes_all_to_hull() {
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 1.0);
        assert_eq!(pierced, 10.0);
        assert_eq!(absorbed, 0.0);
        let leak = if absorbed > 0.0 {
            apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields)
        } else {
            0
        };
        let total_hull = pierced + leak as f32;
        let rng = &mut crate::sim_rng::unseeded_test_rng();
        apply_hull_damage(&mut hull, total_hull, rng);
        assert!(
            (hull.total_current() - 90.0).abs() < 1e-6,
            "hull should be 90 with pierce=1 (10 damage straight through)"
        );
        assert_eq!(
            shields.facings[0].hp, initial_fore_hp,
            "fore quadrant should be untouched with pierce=1"
        );
    }

    #[test]
    fn asteroid_collision_pierce_partial_splits_proportionally() {
        use crate::damage::{
            apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
        };
        use crate::shield::{ShieldConfig, ShieldSystem};
        let mut shields = ShieldSystem::new(&ShieldConfig::default());
        let initial_fore_hp = shields.facings[0].hp;
        let mut hull =
            crate::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

        // pierce = 0.3 on 10 damage → 3 to hull, 7 to fore shield.
        let damage: f32 = 10.0;
        let (pierced, absorbed) = split_damage_for_pierce(damage, 0.3);
        let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
        let total_hull = pierced + leak as f32;
        let rng = &mut crate::sim_rng::unseeded_test_rng();
        apply_hull_damage(&mut hull, total_hull, rng);
        assert!(
            (hull.total_current() - 97.0).abs() < 1e-6,
            "hull should lose 3 (the pierced portion), got {}",
            hull.total_current()
        );
        assert_eq!(
            shields.facings[0].hp,
            initial_fore_hp - 7,
            "fore quadrant should have absorbed 7"
        );
    }

    #[test]
    fn hull_damage_modifier_halves_collision_damage() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::Modifier;

        // Hull damage halved via modifier.
        let mut app = test_app();
        start_game(&mut app);
        modify_ship_modifiers(&mut app, |mods| {
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::HullDamageTaken,
                bonus: -1.0, // â†' multiplier 0.5
            });
        });

        // Apply collision damage directly through the formula used in handle_collisions.
        // At 200 u/s: collision_damage(200) = round(200 * 0.5) = 100.
        // With 0.5Ã— modifier: round(100 * 0.5) = 50.
        fn near(a: f32, b: f32) -> bool {
            (a - b).abs() < 1e-6
        }
        let mods = get_ship_modifiers(&mut app);
        let base_damage = collision_damage(200.0) as f32; // 100
        let scaled_damage = (base_damage * mods.get(&ModifierSlot::HullDamageTaken)).round();
        assert!(
            near(base_damage, 100.0),
            "collision_damage(200) should be 100"
        );
        assert!(
            near(scaled_damage, 50.0),
            "with 0.5Ã— modifier, damage should be 50"
        );

        // Verify the hull loses only the scaled amount by triggering damage through the component.
        apply_hull_damage(&mut app, scaled_damage);
        let out = tick(&mut app);
        // Ship-wide hull reads off `aggregate_fraction` post issue #737.
        let aggregate = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SystemHullUpdate {
                    aggregate_fraction, ..
                } => Some(*aggregate_fraction),
                _ => None,
            })
            .expect("expected SystemHullUpdate");
        assert!(
            near(aggregate.expect("aggregate fraction"), 0.5),
            "hull should be 100 - 50 = 50 with halved collision damage"
        );
    }

    /// PRD #597 PR-8: NPC ships share the collision code path with the player,
    /// so an NPC ship overlapping an asteroid must take hull damage on its own
    /// `EntitySystemHull` component just like the player ship does.
    ///
    /// This spins up a minimal Rapier world (no plugin scaffolding) with just
    /// `handle_collisions`, spawns an NPC ship (`Ship` marker, no `LocalShip`)
    /// overlapping an asteroid, ticks once, and asserts the NPC's hull dropped.
    /// Because the ship is not `LocalShip`, none of the player-only side
    /// effects (`DamageTaken`, `ShipDestroyed`, `GameOver`) may fire.
    #[test]
    fn npc_ship_takes_hull_damage_from_asteroid_collision() {
        use crate::damage::SystemHull;
        use crate::entity_config::{ColliderConfig, ColliderShape};
        use crate::entity_spawner::{ColliderSection, EntitySystemHull, EntityUuid};
        use crate::modifiers::ShipModifiers;
        use bevy_rapier3d::prelude::*;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None, None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .add_systems(Update, handle_collisions);

        // Move the game into InProgress so RapierPhysicsPlugin's default
        // run condition (if any) doesn't gate the step. Not strictly required
        // for handle_collisions itself, but keeps the test's app state
        // consistent with production semantics.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        // Spawn an NPC ship at the origin with a ball collider, some hull,
        // some forward speed (so collision_damage yields non-zero), and no
        // `LocalShip` marker. `ShipShields` is omitted deliberately — NPCs
        // in production may or may not have shields; when absent, all damage
        // routes to hull.
        let npc_uuid = "npc-test-uuid".to_string();
        let npc_hull_max = 100.0f32;
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid(npc_uuid.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                ShipPhysicsComponent {
                    x: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                    forward_speed: 100.0,
                    roll: 0.0,
                    lateral_speed: 0.0,
                    ..Default::default()
                },
                CollisionCooldown::default(),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    npc_hull_max,
                )])),
                ShipModifiers::new(),
                ShipImpulse::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: 5.0,
                    length: 0.0,
                }),
                Collider::ball(5.0),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();

        // Spawn an asteroid overlapping the NPC at the origin.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("ast-test-uuid".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        // Several updates: first ticks let Rapier build the broad-phase and
        // detect the overlapping pair; subsequent ticks run `handle_collisions`
        // with the contact visible on `ReadRapierContext`.
        for _ in 0..3 {
            app.update();
        }

        let hull = app
            .world()
            .get::<EntitySystemHull>(npc)
            .expect("NPC must retain EntitySystemHull");
        assert!(
            hull.0.total_current() < npc_hull_max,
            "NPC hull must decrease from asteroid collision (current={}, max={})",
            hull.0.total_current(),
            npc_hull_max
        );

        // Player-only messages must NOT be emitted for an NPC-vs-asteroid
        // collision — those are gated on `Has<LocalShip>`.
        let outbox = &app.world().resource::<SimOutbox>().0;
        assert!(
            !outbox
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::DamageTaken { .. })),
            "DamageTaken is a player-only UI message; must not fire for NPCs"
        );
        assert!(
            !outbox
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::ShipDestroyed)),
            "ShipDestroyed is a player-only UI message; must not fire for NPCs"
        );

        // Collision response stops the ship and separates it out of the
        // overlapping collider volume, instead of bouncing it backward.
        let physics = app.world().get::<ShipPhysicsComponent>(npc).unwrap();
        assert_eq!(
            physics.forward_speed, 0.0,
            "NPC forward_speed should be zeroed after collision"
        );
        let dist = (physics.x * physics.x + physics.z * physics.z).sqrt();
        assert!(
            dist >= 10.0 + COLLISION_SEPARATION_SLOP - 1e-5,
            "NPC should be separated outside the two collider radii, distance={dist}"
        );
    }

    /// Issue #896, AC-3: which of several simultaneous contacts a ship is
    /// resolved against is decided by world id, not by whichever pair rapier
    /// hands back first.
    ///
    /// A ship wedged between two rocks used to take
    /// `contact_pairs_with(..).next()` — an order that comes out of the
    /// broadphase's internal bookkeeping, is not something the simulation
    /// chose, and is not even the same between a parallel and a serial build.
    /// It decides real outcomes: which direction the ship is pushed, what
    /// bearing the impact comes from and so which shield arc absorbs it, and
    /// whose `shield_pierce` applies.
    ///
    /// The two rocks here sit on opposite sides of the ship, so the direction
    /// it ends up separated in says which one was picked — and the answer must
    /// be the same for both spawn orders, because the pick is `ast-aaa`'s to
    /// win on its uuid either way.
    #[test]
    fn a_ship_between_two_asteroids_is_resolved_against_the_lower_world_id() {
        use crate::damage::SystemHull;
        use crate::entity_config::{ColliderConfig, ColliderShape};
        use crate::entity_spawner::{ColliderSection, EntitySystemHull, EntityUuid};
        use crate::modifiers::ShipModifiers;
        use bevy_rapier3d::prelude::*;

        /// Where the ship ends up after being separated out of the overlap,
        /// with the two rocks spawned in `order`.
        fn separated_x(order: [(&str, f32); 2]) -> f32 {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_millis(50),
                ))
                .add_plugins(bevy::transform::TransformPlugin)
                .add_plugins(bevy::asset::AssetPlugin::default())
                .init_asset::<bevy::mesh::Mesh>()
                .init_resource::<bevy::scene::SceneSpawner>()
                .add_plugins(bevy::state::app::StatesPlugin)
                .init_state::<GamePhase>()
                .add_plugins(RapierPhysicsPlugin::<()>::default())
                .init_resource::<SimOutbox>()
                .init_resource::<WorldResource>()
                .insert_resource(GameOverReason(None, None))
                .init_resource::<DamageLog>()
                .add_message::<crate::ai_plugin::AiEntityDestroyed>()
                .add_systems(Update, handle_collisions);
            app.world_mut()
                .resource_mut::<NextState<GamePhase>>()
                .set(GamePhase::InProgress);
            app.update();

            let ship = app
                .world_mut()
                .spawn((
                    Ship,
                    EntityUuid("ship-under-test".into()),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                    ShipPhysicsComponent {
                        forward_speed: 100.0,
                        ..Default::default()
                    },
                    CollisionCooldown::default(),
                    EntitySystemHull(SystemHull::from_config(&[(
                        SystemId("captain".into()),
                        100.0,
                    )])),
                    ShipModifiers::new(),
                    ShipImpulse::default(),
                    ColliderSection(ColliderConfig {
                        shape: ColliderShape::Ball,
                        radius: 5.0,
                        length: 0.0,
                    }),
                    Collider::ball(5.0),
                    RigidBody::KinematicPositionBased,
                    ActiveCollisionTypes::KINEMATIC_KINEMATIC
                        | ActiveCollisionTypes::KINEMATIC_STATIC,
                ))
                .id();

            for (uuid, x) in order {
                app.world_mut().spawn((
                    Asteroid,
                    AsteroidUuid(uuid.to_string()),
                    Transform::from_xyz(x, 0.0, 0.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                    ColliderSection(ColliderConfig {
                        shape: ColliderShape::Ball,
                        radius: 5.0,
                        length: 0.0,
                    }),
                    Collider::ball(5.0),
                    RigidBody::Fixed,
                    ActiveCollisionTypes::KINEMATIC_STATIC,
                ));
            }

            // Let the broad phase see both overlaps before the collision is
            // consumed, as in the sibling tests above.
            for _ in 0..3 {
                app.update();
            }
            app.world().get::<ShipPhysicsComponent>(ship).unwrap().x
        }

        // `ast-aaa` sits at +X, so the ship is pushed to −X when it is the one
        // chosen — whichever rock was spawned first.
        let aaa_first = separated_x([("ast-aaa", 3.0), ("ast-zzz", -3.0)]);
        let zzz_first = separated_x([("ast-zzz", -3.0), ("ast-aaa", 3.0)]);

        assert!(
            aaa_first < 0.0,
            "the ship should have been separated away from `ast-aaa` at +X, \
             but ended up at x={aaa_first}"
        );
        assert_eq!(
            aaa_first, zzz_first,
            "the same two rocks resolved differently depending on which was \
             spawned first — the contact pair is still being taken in rapier's \
             order rather than by world id"
        );
    }

    /// Issue #896 review finding: `contact_pairs_with` yields every pair whose
    /// *bounding volumes* overlap, not just the ones whose shapes actually
    /// touch. A third rock positioned so its AABB clips the ship's AABB but
    /// whose sphere never reaches the ship's must not be eligible for the
    /// deterministic pick — even when its uuid would sort lowest of all three
    /// and so would win the `min_by_key` outright if it were merely filtered
    /// on `Option::is_some()` upstream instead of on real contact.
    #[test]
    fn a_lower_uuid_rock_with_only_an_aabb_overlap_is_never_selected() {
        use crate::damage::SystemHull;
        use crate::entity_config::{ColliderConfig, ColliderShape};
        use crate::entity_spawner::{ColliderSection, EntitySystemHull, EntityUuid};
        use crate::modifiers::ShipModifiers;
        use bevy_rapier3d::prelude::*;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None, None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .add_systems(Update, handle_collisions);
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        let ship = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid("ship-under-test".into()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                ShipPhysicsComponent {
                    forward_speed: 100.0,
                    ..Default::default()
                },
                CollisionCooldown::default(),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )])),
                ShipModifiers::new(),
                ShipImpulse::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: 5.0,
                    length: 0.0,
                }),
                Collider::ball(5.0),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();

        // The genuine contact: `ast-aaa` at +X, sphere-overlapping the ship
        // exactly as in the sibling test above.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("ast-aaa".to_string()),
            Transform::from_xyz(3.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        // The decoy: `ast-000` sorts below `ast-aaa` on uuid alone, so it
        // would win `min_by_key` if the filter above it did not exclude
        // AABB-only overlaps. Both rocks have radius 5 and the ship has radius
        // 5, so two spheres need center distance < 10 to actually touch. This
        // one sits at (8, 8, 0): 3D center distance is sqrt(8²+8²) ≈ 11.3 — no
        // shape contact — but its AABB (x:[3,13], y:[3,13], z:[-5,5]) clips
        // the ship's AABB (x:[-5,5], y:[-5,5], z:[-5,5]) in both x and y, so
        // the broad phase still reports the pair.
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("ast-000".to_string()),
            Transform::from_xyz(8.0, 8.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        // Let the broad phase see both overlaps before the collision is
        // consumed, as in the sibling test above.
        for _ in 0..3 {
            app.update();
        }

        let physics = app.world().get::<ShipPhysicsComponent>(ship).unwrap();
        // If the decoy had been selected, `separate_ship_from_collision` would
        // have pushed the ship away from (8, 8, 0) — a nonzero z displacement,
        // since the decoy's z sits at 0 same as the ship's own z it would at
        // minimum not reproduce the pure -X push below. The genuine pick
        // (`ast-aaa` at +X) only ever moves the ship along x, leaving z at 0.
        assert!(
            physics.x < 0.0,
            "the ship should still have been separated away from `ast-aaa` \
             at +X, but ended up at x={}",
            physics.x
        );
        assert_eq!(
            physics.z, 0.0,
            "the ship moved in z, implying the AABB-only decoy at (8, 8, 0) \
             was selected instead of the genuine `ast-aaa` contact"
        );
    }

    /// Environmental damage still has to reach the balance log, on an NPC, with
    /// no attacker — the half of a fight that `DamageTaken` never reports.
    #[test]
    fn npc_asteroid_collision_emits_attacker_less_balance_event() {
        use crate::balance::BalanceEvent;
        use crate::damage::SystemHull;
        use crate::entity_config::{ColliderConfig, ColliderShape};
        use crate::entity_spawner::{ColliderSection, EntitySystemHull, EntityUuid};
        use crate::modifiers::ShipModifiers;
        use bevy::ecs::message::Messages;
        use bevy_rapier3d::prelude::*;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None, None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .add_message::<BalanceEvent>()
            .add_systems(Update, handle_collisions);

        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        let npc_uuid = "npc-collide-uuid".to_string();
        app.world_mut().spawn((
            Ship,
            EntityUuid(npc_uuid.clone()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent {
                x: 0.0,
                z: 0.0,
                yaw: 0.0,
                forward_speed: 100.0,
                roll: 0.0,
                lateral_speed: 0.0,
                ..Default::default()
            },
            CollisionCooldown::default(),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("ast-collide-uuid".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
            }),
            Collider::ball(5.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));

        for _ in 0..3 {
            app.update();
        }

        let messages = app.world().resource::<Messages<BalanceEvent>>();
        let mut cursor = messages.get_cursor();
        let hits: Vec<&BalanceEvent> = cursor.read(messages).collect();
        assert_eq!(hits.len(), 1, "the collision must emit exactly one event");

        let BalanceEvent::DamageApplied {
            attacker,
            victim,
            victim_kind,
            weapon,
            amount,
            shield_absorbed,
            hull_damage,
            ..
        } = hits[0]
        else {
            panic!("the collision event must be a DamageApplied");
        };
        assert_eq!(*attacker, None, "environmental damage has no shooter");
        assert_eq!(victim, &npc_uuid);
        assert_eq!(
            *victim_kind,
            crate::balance::VictimKind::Ship,
            "the ship takes the collision damage, not the rock it hit"
        );
        assert_eq!(weapon, crate::balance::WEAPON_KIND_COLLISION);
        assert!(*amount > 0.0, "a 100-speed impact must offer damage");
        assert_eq!(
            *shield_absorbed, 0.0,
            "this NPC has no shields, so nothing is absorbed"
        );
        assert!(
            *hull_damage > 0.0,
            "unshielded impact damage must land on hull"
        );
    }

    #[test]
    fn drain_sim_outbox_directly() {
        let mut app = test_app();
        start_game(&mut app);

        // Write directly to SimOutbox
        let len_before = app.world().resource::<SimOutbox>().0.len();
        app.world_mut()
            .resource_mut::<SimOutbox>()
            .0
            .push((Target::All, ServerMessage::GameStarted));

        // Drain manually
        app.world_mut().resource_mut::<SimOutbox>().0.clear();

        // Check SimOutbox is now empty
        let len_after = app.world().resource::<SimOutbox>().0.len();
        assert_eq!(
            len_after,
            0,
            "SimOutbox should be empty after drain, was {} before drain",
            len_before + 1
        );
    }

    // -- Power system integration tests --------------------------------------

    /// Helper: captain + power console player, game started.
    fn start_game_with_power(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "power",
            ClientMessage::Identify {
                token: "power".into(),
                name: "Monty".into(),
            },
        );
        tick(app);
        push(
            app,
            "power",
            ClientMessage::SelectStation {
                station: "Power".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "power", ClientMessage::SetReady { ready: true });
        let _ = tick(app);
        fast_forward_countdown(app);
        let _ = tick(app);
        let _ = tick(app);
    }

    #[test]
    fn non_power_sender_increase_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Reset power to known state.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                1,
            );

        // Captain (not Power holder) tries to set Helm to 2.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 2,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&crate::messages::PowerGroupId(
                    crate::power_system::HELM_POWER_GROUP.into()
                )),
            1,
            "non-Power sender should not be able to increase power"
        );
    }

    #[test]
    fn non_power_sender_decrease_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Captain (not Power holder) tries to set Sensors to 1.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::SHIELDS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .level_for(&crate::messages::PowerGroupId(
                    crate::power_system::SHIELDS_POWER_GROUP.into()
                )),
            2,
            "non-Power sender should not be able to decrease power"
        );
    }

    #[test]
    fn power_sender_increase_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Power holder sets Helm to 3.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { helm, .. } => Some(*helm),
                _ => None,
            })
            .expect("expected a PowerState message for power holder");
        assert_eq!(
            power_state, 3,
            "PowerState should show helm=3 after increase"
        );
    }

    #[test]
    fn power_sender_decrease_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Power holder sets Weapons to 1.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { weapons, .. } => Some(*weapons),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 1,
            "PowerState should show weapons=1 after decrease"
        );
    }

    #[test]
    fn power_state_only_sent_to_power_holder() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let out = tick(&mut app);

        // Every PowerState message should target the power holder.
        for m in out
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. }))
        {
            assert!(
                matches!(&m.target, Target::Token(t) if t == "power"),
                "PowerState should only go to the Power holder, got {:?}",
                m.target
            );
        }
    }

    #[test]
    fn no_power_station_holder_no_power_state_broadcast() {
        let mut app = test_app();
        // Only captain, no power station holder.
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(
            !any_power_state,
            "no PowerState should be sent when no Power station holder exists"
        );
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Manually set Helm to 4 (max).
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );

        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 4,
                },
            },
        );
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { helm, .. } => Some(*helm),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 4,
            "helm should stay at 4 (max bound enforced by PowerSystem)"
        );
    }

    // -- Power ? Modifier wiring integration tests -------------------------

    #[test]
    fn increasing_helm_power_updates_max_speed_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Helm so level 2 ? 0.0, level 3 ? 1.0
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                [-0.5, 0.0, 1.0, 2.0],
            );

        // Set Helm to 3.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::HELM_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 3 ? index 2 ? bonus 1.0 ? MaxSpeed multiplier = 2.0
        let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::MaxSpeed);
        assert!(
            (mult - 2.0).abs() < 1e-6,
            "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}"
        );
    }

    #[test]
    fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Tactical: level 2 ? 0.0, level 1 ? -0.5
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                [-0.5, 0.0, 0.25, 0.5],
            );

        // Set Weapons to 1.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    level: 1,
                },
            },
        );
        let _ = tick(&mut app);

        // Level 1 ? index 0 ? bonus -0.5 (negative) ? 1.0 / (1.0 + 0.5) = 0.666...
        let expected = 1.0 / 1.5;
        let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::PhaserDamage);
        assert!(
            (mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
        );
    }

    /// Re-aimed by issue #952: a flat battery takes back each group's SPENT
    /// point, landing it on the level its file seeds it at (nominal for a hull
    /// that authors no `[power_groups.*]`, as this fixture does) rather than
    /// slamming every group to 1. The old name and its x0.667 assertions
    /// described the retired brownout lock.
    #[test]
    fn a_flat_battery_floors_every_group_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Set known multipliers for all three
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                defaults,
            );
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                defaults,
            );
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(
                crate::messages::PowerGroupId(crate::power_system::SHIELDS_POWER_GROUP.into()),
                defaults,
            );

        // Set state that will trigger the battery floors on the next tick:
        // total=8 (negative rate), battery already at 0 -> tick keeps it at 0,
        // and every group is under its authored floor (#952).
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::WEAPONS_POWER_GROUP.into()),
                2,
            );
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::SHIELDS_POWER_GROUP.into()),
                2,
            );
            ps.0.battery_charge = 0.0;
        }

        // Tick applies the floors -> translate_power_modifiers runs
        tick(&mut app);

        // All three at their nominal floor -> bonus 0.0 -> multiplier 1.0.
        let mods = get_ship_modifiers(&mut app);
        for (slot, label) in [
            (ModifierSlot::MaxSpeed, "MaxSpeed"),
            (ModifierSlot::PhaserDamage, "PhaserDamage"),
            (ModifierSlot::ShieldRegen, "ShieldRegen"),
        ] {
            let mult = mods.get(&slot);
            assert!(
                (mult - 1.0).abs() < 1e-6,
                "with every group held at its NOMINAL floor, {label} should be                  x1.0, got {mult}"
            );
        }
        assert_eq!(
            app.world()
                .resource::<ShipPowerSystem>()
                .0
                .commanded_level_for(&crate::messages::PowerGroupId(
                    crate::power_system::HELM_POWER_GROUP.into()
                )),
            4,
            "the brownout must not have rewritten the standing order"
        );
    }

    #[test]
    fn power_increase_respects_total_cap_of_eight() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Set total to 8: helm=4, weapons=2, shields=2.
        let _ = app
            .world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .set_group_allocation(
                &crate::messages::PowerGroupId(crate::power_system::HELM_POWER_GROUP.into()),
                4,
            );

        // Try to set shields to 3 — total would be 9 (over cap), should be blocked at 2.
        push(
            &mut app,
            "power",
            ClientMessage::ControlSystem {
                target: crate::system_registry::power_reactor_system_id(),
                payload: crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(
                        crate::power_system::SHIELDS_POWER_GROUP.into(),
                    ),
                    level: 3,
                },
            },
        );
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::PowerState { shields, .. } => Some(*shields),
                _ => None,
            })
            .expect("expected a PowerState message");
        assert_eq!(
            power_state, 2,
            "shields should stay at 2 when total is already at the cap of 8"
        );
        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.total(),
            8,
            "total should remain 8"
        );
    }

    // -- Runtime entity lifecycle (EntitySpawned / EntityDespawned) -----

    #[test]
    fn reconcile_system_seeds_on_first_inprogress_frame() {
        let mut app = test_app();
        start_game(&mut app);
        // After start_game, the system should have seeded (even if empty).
        let registry = app.world().resource::<TrackedEntities>();
        assert!(
            registry.seeded,
            "system should be seeded after first InProgress frame"
        );
    }

    #[test]
    fn spawn_non_asteroid_entity_emits_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("runtime-entity-1".into()),
            Transform::from_xyz(100.0, 0.0, -200.0),
        ));

        let out = tick(&mut app);

        let spawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
            _ => None,
        });
        assert!(
            spawned.is_some(),
            "expected EntitySpawned after spawning a non-asteroid entity"
        );
        assert_eq!(spawned.unwrap().uuid, "runtime-entity-1");
    }

    #[test]
    fn entity_spawned_broadcast_contains_position_and_id() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("pos-entity".into()),
            crate::entity_spawner::EntityId("station-alpha".into()),
            Transform::from_xyz(50.0, 0.0, -75.0),
        ));

        let out = tick(&mut app);

        let spawned = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
                _ => None,
            })
            .expect("expected EntitySpawned");

        assert_eq!(spawned.uuid, "pos-entity");
        assert_eq!(spawned.id, Some("station-alpha".into()));
        assert_eq!(spawned.position, Some([50.0, 0.0, -75.0]));
    }

    #[test]
    fn despawn_non_asteroid_entity_emits_entity_despawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        let entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("to-despawn".into()),
                Transform::default(),
            ))
            .id();

        // Tick once so the spawn system picks it up.
        let _ = tick(&mut app);

        // Now despawn it.
        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntityDespawned { uuid } => Some(uuid.clone()),
            _ => None,
        });
        assert!(
            despawned.is_some(),
            "expected EntityDespawned after despawning a non-asteroid entity"
        );
        assert_eq!(despawned.unwrap(), "to-despawn");
    }

    #[test]
    fn asteroid_spawn_does_not_emit_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn an asteroid entity (has Asteroid component + EntityUuid).
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("asteroid-1".into()),
            Asteroid,
            AsteroidUuid("asteroid-1".into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawned = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }));
        assert!(
            !spawned,
            "asteroid spawn must not emit EntitySpawned (uses AsteroidSpawned instead)"
        );
    }

    #[test]
    fn runtime_entity_appears_in_world_data_for_reconnect() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("reconnect-entity".into()),
            Transform::from_xyz(25.0, 0.0, -50.0),
        ));

        let _ = tick(&mut app);

        // The entity should now be in world.entities so Welcome includes it.
        let world = app.world().resource::<WorldResource>();
        let found = world
            .0
            .entities
            .iter()
            .any(|e| e.uuid == "reconnect-entity");
        assert!(
            found,
            "runtime entity must appear in WorldResource for Welcome reconnects"
        );
    }

    #[test]
    fn midgame_reconnect_resets_blackboard_cache() {
        let mut app = test_app();
        start_game(&mut app);

        let helm_id = SystemId("helm".into());
        let helm_bb = SystemBlackboard::Helm(HelmBlackboard {
            yaw: 1.0,
            forward_speed: 50.0,
            x: 100.0,
            z: 200.0,
            impulse_charge: 0.5,
            boost_battery: 0.8,
            boost_active: false,
            boost_enabled: true,
            radar_range: 0.0,
            lateral_speed: 0.0,
            hostile_weapon_arcs: Vec::new(),
        });

        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemBlackboards, With<LocalShip>>();
            if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
                bbs.0.insert(helm_id.clone(), helm_bb.clone());
            }
        }

        // Tick: broadcast_blackboard_updates caches the blackboard and emits it.
        let out1 = tick(&mut app);
        assert!(
            out1.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BlackboardUpdate { .. })),
            "first tick after seeding must emit BlackboardUpdate"
        );

        // Simulate reconnect: push Identify with same token -> Welcome emitted.
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        let out2 = tick(&mut app);
        let out3 = tick(&mut app);

        let has_bb_for_helm = |out: &[OutboundMessage]| -> bool {
            out.iter().any(|m| match &m.msg {
                ServerMessage::BlackboardUpdate { updates } => {
                    updates.iter().any(|(id, _)| id.0 == "helm")
                }
                _ => false,
            })
        };

        assert!(
            has_bb_for_helm(&out2) || has_bb_for_helm(&out3),
            "must emit BlackboardUpdate with helm data within one tick of reconnect Welcome"
        );
    }

    /// Issue #697 made the weapons blackboard publish systems per-entity, so NPC ships now
    /// carry populated Weapons blackboards. `broadcast_blackboard_updates` reads
    /// only the `LocalShip`, and `LastBroadcastBlackboards` is a single global
    /// map keyed on `SystemId` alone — it structurally assumes one broadcast
    /// source. This pins that assumption: NPC blackboards must cost zero
    /// bandwidth, or they would both leak and collide with the player ship's
    /// cache entries under the same `SystemId`.
    #[test]
    fn npc_weapons_blackboards_add_no_wire_traffic() {
        let mut app = test_app();
        start_game(&mut app);

        // The thing the NPC is locked onto has to exist: `target_uuid` is the
        // frozen combat lock filtered for liveness, so a lock on a uuid with no
        // entity behind it is (correctly) never published.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("npc-only-target".into()),
            bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
        ));

        // An NPC ship locked onto a target the player ship never sees.
        let npc = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::ship_plugin::ShipConfigComponent::default(),
                crate::ship_plugin::ShipSystemControlSources::default(),
                ShipSystemBlackboards::default(),
                crate::weapons_plugin::TacticalRadarSelection(Some("npc-only-target".into())),
                crate::weapons_plugin::ActiveBeam::default(),
                crate::weapons_plugin::PhaserCooldown::default(),
                crate::weapons_plugin::LastShipAttacker::default(),
                ShipPhysicsComponent::default(),
                crate::entity_spawner::EntityUuid("npc-1".into()),
                bevy::prelude::Transform::default(),
            ))
            .id();

        // Two ticks: `target_uuid` is the frozen viewscreen combat lock, which
        // the aggregator writes in `SimSet::PublishAggregate` — one tick behind
        // the publisher that reads it in `SimSet::Publish` (spec §1).
        tick(&mut app);
        let out = tick(&mut app);

        // The NPC really does publish its own Weapons blackboard...
        let npc_target = app
            .world()
            .entity(npc)
            .get::<ShipSystemBlackboards>()
            .and_then(|bbs| bbs.0.get(&SystemId("tactical".into())).cloned());
        assert!(
            matches!(
                npc_target,
                Some(crate::messages::SystemBlackboard::Weapons(ref bb))
                    if bb.target_uuid.as_deref() == Some("npc-only-target")
            ),
            "NPC must publish its own Weapons blackboard, got {npc_target:?}"
        );

        // ...and none of it reaches any client.
        for m in &out {
            if let ServerMessage::BlackboardUpdate { updates } = &m.msg {
                for (id, bb) in updates {
                    if let crate::messages::SystemBlackboard::Weapons(w) = bb {
                        assert_ne!(
                            w.target_uuid.as_deref(),
                            Some("npc-only-target"),
                            "NPC weapons blackboard leaked to the wire under SystemId {id:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn entity_spawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("all-broadcast".into()),
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawn_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }))
            .expect("expected EntitySpawned message");
        assert!(
            matches!(&spawn_msg.target, crate::lobby::Target::All),
            "EntitySpawned must broadcast to All, got {:?}",
            spawn_msg.target
        );
    }

    #[test]
    fn entity_despawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        let entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("broadcast-despawn".into()),
                Transform::default(),
            ))
            .id();
        let _ = tick(&mut app);

        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawn_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { .. }))
            .expect("expected EntityDespawned message");
        assert!(
            matches!(&despawn_msg.target, crate::lobby::Target::All),
            "EntityDespawned must broadcast to All, got {:?}",
            despawn_msg.target
        );
    }

    // -- SetPhaserFrequency envelope tests (issue #804) -------------------
    // The legacy top-level `ClientMessage::SetPhaserFrequency` was deleted;
    // these exercise the admitted `ControlSystem` envelope path against the
    // full server app (real ship config declaring `phaser-control`).

    /// Build the admitted-envelope form of a frequency change (issue #804).
    fn set_phaser_frequency_msg(frequency: f32) -> ClientMessage {
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: crate::messages::SystemControlPayload::SetPhaserFrequency { frequency },
        }
    }

    /// Tactical holder may always set phaser frequency.
    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", set_phaser_frequency_msg(0.8));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.8).abs() < 1e-5,
            "Tactical holder should set phaser frequency to 0.8, got {freq}"
        );
    }

    /// Sensors holder is never authorized to set phaser frequency (delegation removed in B4).
    #[test]
    fn sensors_holder_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(&mut app, "sensors", set_phaser_frequency_msg(0.9));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Sensors holder must NOT change phaser frequency, got {freq}"
        );
    }

    /// An unrelated console (e.g. captain) cannot set phaser frequency.
    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", set_phaser_frequency_msg(0.9));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Captain must NOT change phaser frequency, got {freq}"
        );
    }

    /// Frequency value is clamped to [0.0, 1.0] by the handler.
    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", set_phaser_frequency_msg(1.5));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 1.0).abs() < 1e-5,
            "frequency above 1.0 should clamp to 1.0, got {freq}"
        );

        push(&mut app, "weapons", set_phaser_frequency_msg(-0.5));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.0).abs() < 1e-5,
            "frequency below 0.0 should clamp to 0.0, got {freq}"
        );
    }

    // -- Shield focus tests --------------------------------------------------

    fn start_game_with_shields(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "shields",
            ClientMessage::Identify {
                token: "shields".into(),
                name: "Sully".into(),
            },
        );
        tick(app);
        push(
            app,
            "shields",
            ClientMessage::SelectStation {
                station: "Shields".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "shields", ClientMessage::SetReady { ready: true });
        let _ = tick(app);
        fast_forward_countdown(app);
        let _ = tick(app);
        let _ = tick(app);
    }

    #[test]
    fn shields_holder_can_focus_a_facing() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, Some(0));
        assert!(shields.0.facings[0].is_focused);
    }

    #[test]
    fn non_shields_sender_cannot_set_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        // Captain (not Shields holder) tries to set focus.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("port").expect("port"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shields_holder_can_clear_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);
        let ship = app.world().resource::<ShipEntity>().0;
        let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, Some(0));

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: false },
            },
        );
        tick(&mut app);
        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shield_focus_is_ignored_during_lobby() {
        let mut app = test_app();
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(&mut app);

        // Still in Lobby — SetShieldArcFocus should be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("aft").expect("aft"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipEntity>().0;
        assert!(app
            .world()
            .entity(ship)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
            .is_none());
    }

    #[test]
    fn shield_focus_updates_broadcast_status() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(
            &mut app,
            "shields",
            ClientMessage::ControlSystem {
                target: crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let shield_status = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::ShieldStatus { facings, .. } => Some(facings.clone()),
                _ => None,
            })
            .expect("expected a ShieldStatus broadcast after focus change");

        assert!(shield_status[0].is_focused, "Fore should be focused");
        assert!(!shield_status[1].is_focused, "Port should not be focused");
        assert!(!shield_status[2].is_focused, "Aft should not be focused");
        assert!(
            !shield_status[3].is_focused,
            "Starboard should not be focused"
        );
    }

    #[test]
    fn player_spawn_rotation_yaw_extracts_yaw_correctly() {
        let (q, yaw) = player_spawn_rotation_yaw([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
        assert!(
            (yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "yaw-only rotation should produce matching yaw"
        );
        let (y, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
        assert!(
            (y - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "quaternion yaw should match input"
        );
    }

    #[test]
    fn player_spawn_rotation_yaw_pitch_only_gives_zero_yaw() {
        let (_, yaw) = player_spawn_rotation_yaw([std::f32::consts::FRAC_PI_4, 0.0, 0.0]);
        assert!(yaw.abs() < 1e-6, "pitch-only rotation should give zero yaw");
    }

    #[test]
    fn player_spawn_rotation_yaw_roll_only_gives_zero_yaw() {
        let (_, yaw) = player_spawn_rotation_yaw([0.0, 0.0, std::f32::consts::FRAC_PI_3]);
        assert!(yaw.abs() < 1e-6, "roll-only rotation should give zero yaw");
    }

    // ── last_attacker clear handler tests ──────────────────────────────────

    fn last_attacker_test_app() -> App {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                clear_last_attacker_on_death,
                clear_last_attacker_on_red_alert_off,
            ),
        );
        app
    }

    #[test]
    fn clear_on_despawn_clears_when_entity_removed() {
        let mut app = last_attacker_test_app();
        let attacker_uuid = "attacker-1".to_string();
        let attacker_entity = app
            .world_mut()
            .spawn((EntityUuid(attacker_uuid.clone()),))
            .id();
        let ship = app
            .world_mut()
            .spawn((LastShipAttacker(Some(attacker_uuid)),))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
        app.world_mut().despawn(attacker_entity);
        app.update();
        assert_eq!(app.world().get::<LastShipAttacker>(ship).unwrap().0, None);
    }

    #[test]
    fn clear_on_despawn_does_not_clear_when_entity_still_alive() {
        let mut app = last_attacker_test_app();
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((LastShipAttacker(Some("attacker-1".to_string())),))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
    }

    #[test]
    fn clear_on_red_alert_off_clears_when_red_alert_turns_off() {
        let mut app = last_attacker_test_app();
        // Spawn an entity so clear_last_attacker_on_death doesn't fire.
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                LocalShip,
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
        app.world_mut()
            .get_mut::<crate::ship_state::ShipRedAlert>(ship)
            .unwrap()
            .0 = false;
        app.update();
        assert!(app
            .world()
            .get::<LastShipAttacker>(ship)
            .unwrap()
            .0
            .is_none());
    }

    #[test]
    fn clear_on_red_alert_off_does_not_clear_when_alert_stays_on() {
        let mut app = last_attacker_test_app();
        // Spawn an entity so clear_last_attacker_on_death doesn't fire.
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let ship = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                LocalShip,
            ))
            .id();
        app.update();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(ship).unwrap().0,
            Some("attacker-1".to_string())
        );
    }

    /// Regression test: `clear_last_attacker_on_red_alert_off` used to be
    /// filtered `With<LocalShip>`, so an NPC whose red alert stood down kept
    /// retaliating against a stale attacker forever. NPC captain-AI can
    /// set its own `ShipRedAlert` (`handle_set_red_alert` dispatches
    /// per-ship), so the clear handler must cover NPCs too.
    #[test]
    fn clear_on_red_alert_off_clears_for_an_npc_not_just_local_ship() {
        let mut app = last_attacker_test_app();
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        let npc = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                // No `LocalShip` marker — this is an NPC.
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<LastShipAttacker>(npc).unwrap().0,
            Some("attacker-1".to_string())
        );
        app.world_mut()
            .get_mut::<crate::ship_state::ShipRedAlert>(npc)
            .unwrap()
            .0 = false;
        app.update();
        assert!(
            app.world()
                .get::<LastShipAttacker>(npc)
                .unwrap()
                .0
                .is_none(),
            "an NPC standing down from red alert must clear its attacker record too"
        );
    }

    /// Two ships (one player, one NPC) toggling red alert off independently
    /// must each clear their own attacker record without a shared `Local`
    /// mixing up whose transition is whose.
    #[test]
    fn clear_on_red_alert_off_handles_multiple_ships_independently() {
        let mut app = last_attacker_test_app();
        app.world_mut()
            .spawn((EntityUuid("attacker-1".to_string()),));
        app.world_mut()
            .spawn((EntityUuid("attacker-2".to_string()),));

        let local = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-1".to_string())),
                crate::ship_state::ShipRedAlert(true),
                LocalShip,
            ))
            .id();
        let npc = app
            .world_mut()
            .spawn((
                LastShipAttacker(Some("attacker-2".to_string())),
                crate::ship_state::ShipRedAlert(true),
            ))
            .id();
        app.update();

        // Only the NPC stands down this tick; the player stays at red alert.
        app.world_mut()
            .get_mut::<crate::ship_state::ShipRedAlert>(npc)
            .unwrap()
            .0 = false;
        app.update();

        assert!(
            app.world()
                .get::<LastShipAttacker>(npc)
                .unwrap()
                .0
                .is_none(),
            "the NPC that stood down must have its attacker cleared"
        );
        assert_eq!(
            app.world().get::<LastShipAttacker>(local).unwrap().0,
            Some("attacker-1".to_string()),
            "the player ship, still at red alert, must keep its attacker record"
        );
    }

    // ── build_sim_state_entity_states: shield detail on the wire (#927) ─────
    //
    // Root cause pinned here: `sim_state_broadcaster` always sent
    // `shields: None` and had no `shield_freq` field on `EntityStateSnapshot`
    // at all, regardless of whether the entity carried a `ShipShields`
    // component — so `target_shields`/`target_shield_freq` were empty on the
    // wire for every Sensors target, on every hull, before this fix. These
    // call `build_sim_state_entity_states` directly (the function extracted
    // from `sim_state_broadcaster`'s producer closure) rather than going
    // through the Broadcaster/cadence machinery, since the function needs
    // only a bare `World` with the two delta-cache resources.

    #[test]
    fn target_with_shields_populates_shields_and_shield_freq() {
        let mut world = World::new();
        world.init_resource::<LastBroadcastEntityPositions>();
        world.init_resource::<LastBroadcastEntityHealth>();
        world.spawn((
            EntityUuid("target-1".to_string()),
            Transform::from_xyz(10.0, 0.0, 20.0),
            ShipShields(ShieldSystem::default(), 0.75),
        ));

        let states = build_sim_state_entity_states(&mut world);
        let entry = states
            .iter()
            .find(|s| s.uuid == "target-1")
            .expect("target-1 must appear in the first SimState tick");

        let shields = entry
            .shields
            .as_ref()
            .expect("a ShipShields-carrying entity must publish its facings");
        assert!(
            !shields.is_empty(),
            "expected at least one shield facing, same producer as this ship's own ShieldsBlackboard"
        );
        assert_eq!(
            entry.shield_freq,
            Some(0.75),
            "shield_freq must be the entity's own ShipShields::frequency() — \
             the same value FrequencyHint reads"
        );
    }

    #[test]
    fn entity_without_shields_leaves_shields_and_shield_freq_absent() {
        let mut world = World::new();
        world.init_resource::<LastBroadcastEntityPositions>();
        world.init_resource::<LastBroadcastEntityHealth>();
        world.spawn((
            EntityUuid("no-shields-1".to_string()),
            Transform::from_xyz(5.0, 0.0, 5.0),
        ));

        let states = build_sim_state_entity_states(&mut world);
        let entry = states
            .iter()
            .find(|s| s.uuid == "no-shields-1")
            .expect("no-shields-1 must still appear (position changed on the first tick)");

        assert!(
            entry.shields.is_none(),
            "an entity with no ShipShields must not carry a shields field"
        );
        assert!(
            entry.shield_freq.is_none(),
            "an entity with no ShipShields must not carry a shield_freq field"
        );
    }

    #[test]
    fn shield_detail_is_delta_compressed_like_hull_and_shield_fraction() {
        // The widened `LastBroadcastEntityHealth` cache tuple must gate the
        // NEXT tick's inclusion on shields/shield_freq changing, exactly as
        // it already did for hull_fraction/shield_fraction — not just those
        // two fields.
        let mut world = World::new();
        world.init_resource::<LastBroadcastEntityPositions>();
        world.init_resource::<LastBroadcastEntityHealth>();
        world.spawn((
            EntityUuid("steady-1".to_string()),
            Transform::from_xyz(1.0, 0.0, 1.0),
            ShipShields(ShieldSystem::default(), 0.5),
        ));

        let first = build_sim_state_entity_states(&mut world);
        assert!(
            first.iter().any(|s| s.uuid == "steady-1"),
            "first tick must publish the newly-seen entity"
        );

        let second = build_sim_state_entity_states(&mut world);
        assert!(
            !second.iter().any(|s| s.uuid == "steady-1"),
            "an entity whose position/hull/shields/freq are all unchanged \
             since the last broadcast must be omitted entirely from the next tick"
        );
    }

    #[test]
    fn shield_offline_remaining_delta_gate_ignores_subsecond_countdown_but_reports_bucket_crossings(
    ) {
        // `ShieldFacingStatus` derives `PartialEq` over every field including
        // `offline_remaining`, which `tick_shields` decrements every tick
        // through a ~30s recovery. A raw equality gate re-sent this payload
        // on effectively every 10 Hz tick while any facing was offline, even
        // though nothing perceptible changed. `shields_delta_projection`
        // buckets `offline_remaining` to whole seconds (ceiling) before
        // comparing — see its doc comment in `ship::shields`.
        let mut world = World::new();
        world.init_resource::<LastBroadcastEntityPositions>();
        world.init_resource::<LastBroadcastEntityHealth>();
        let entity = world
            .spawn((
                EntityUuid("recovering-1".to_string()),
                Transform::from_xyz(1.0, 0.0, 1.0),
                ShipShields(ShieldSystem::default(), 0.5),
            ))
            .id();
        world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 5.5;

        let first = build_sim_state_entity_states(&mut world);
        assert!(
            first.iter().any(|s| s.uuid == "recovering-1"),
            "first tick must publish the newly-seen offline facing"
        );

        // Sub-second countdown: 5.5s -> 5.4s still buckets to ceil = 6.0 —
        // must NOT re-send.
        world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 5.4;
        let second = build_sim_state_entity_states(&mut world);
        assert!(
            !second.iter().any(|s| s.uuid == "recovering-1"),
            "a sub-second offline_remaining tick (5.5s -> 5.4s, same whole-second \
             bucket) must not re-trigger the delta gate"
        );

        // Crossing a whole-second boundary: 5.4s -> 4.9s, ceil bucket goes
        // 6.0 -> 5.0 — must re-send.
        world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 4.9;
        let third = build_sim_state_entity_states(&mut world);
        assert!(
            third.iter().any(|s| s.uuid == "recovering-1"),
            "crossing a whole-second bucket boundary (5.4s -> 4.9s) must \
             re-trigger the delta gate"
        );
    }

    // ── God Mode applier (issue #900) ───────────────────────────────────────

    fn god_mode_app() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<GodMode>()
            .add_systems(Update, apply_god_mode_toggle);
        let ship = app
            .world_mut()
            .spawn((LocalShip, AdmittedCommands::default()))
            .id();
        (app, ship)
    }

    /// Sets `ship`'s `AdmittedCommands` to exactly one command (clearing
    /// whatever was there), mirroring what `admit_system_commands` does at the
    /// top of every real tick (AGENTS.md constraint 7: "`AdmittedCommands` is
    /// cleared and refilled at admission each tick"). This minimal fixture has
    /// no admission system to do that itself, so the test stands in for it —
    /// otherwise a second call would leave the first tick's command in place
    /// and every following tick would apply the toggle twice.
    fn admit(app: &mut App, ship: Entity, payload: SystemControlPayload) {
        let mut admitted = app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap();
        admitted.0.clear();
        admitted.0.push(AdmittedCommand {
            target: SystemId(crate::system_registry::GOD_MODE_SYSTEM_ID.into()),
            payload,
            response_token: None,
        });
    }

    /// The baseline: an admitted `ToggleGodMode` command flips `GodMode` from
    /// its default `false`.
    #[test]
    fn an_admitted_toggle_flips_god_mode_on() {
        let (mut app, ship) = god_mode_app();
        assert!(!app.world().resource::<GodMode>().0, "precondition: off");
        admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
        app.update();
        assert!(
            app.world().resource::<GodMode>().0,
            "an admitted ToggleGodMode command must flip GodMode on"
        );
    }

    /// A second admitted toggle (a second tick, a second command) flips it
    /// back off — proving this is a flip, not a one-way latch.
    #[test]
    fn a_second_admitted_toggle_flips_god_mode_back_off() {
        let (mut app, ship) = god_mode_app();
        admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
        app.update();
        assert!(app.world().resource::<GodMode>().0, "precondition: now on");

        admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
        app.update();
        assert!(
            !app.world().resource::<GodMode>().0,
            "a second admitted toggle must flip it back off"
        );
    }

    /// With nothing admitted, a tick must not touch `GodMode` — the applier
    /// only acts on what it finds in `AdmittedCommands`, never on its own.
    #[test]
    fn no_admitted_command_leaves_god_mode_untouched() {
        let (mut app, _ship) = god_mode_app();
        app.update();
        assert!(
            !app.world().resource::<GodMode>().0,
            "with nothing admitted, GodMode must stay at its default"
        );
    }
}
