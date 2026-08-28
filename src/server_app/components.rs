//! ECS vocabulary for the simulation app assembly (issue #1199).
//!
//! Public surface: the marker components (`Ship`, `LocalShip`, `Asteroid`, …),
//! the sim resources (`GodMode`, `Instagib`, `GameOverReason`, `SimOutbox`,
//! `TrackedEntities`, `CaptainPriorityBoost`, …), the `#[derive(SystemParam)]`
//! bundles used by wide sim systems (`WorldAndTracked`, `PlayerDeathLatch`,
//! `SimRngAndLog`), `ShipSystemBlackboards`, and the `sim_processing_anchor`
//! ordering marker. Re-exported through `crate::server_app` so every existing
//! `crate::server_app::X` path resolves unchanged.
//!
//! Role: the data types the simulation defines and shares — no systems logic
//! lives here beyond the tiny `apply_god_mode_toggle` applier that owns
//! [`GodMode`], and the empty `sim_processing_anchor`.
//!
//! Load-bearing invariant: `LocalShip` REQUIRES `HumanSeekingHosts` /
//! `VisitingStationHosts` / `ScenarioDetailFloor` so those components arrive in
//! the spawn-burst archetype transition — a mid-run archetype move here would
//! re-order archetype ids and move the authoritative digest (see `LocalShip`).

use super::*;

// ── Marker Components ────────
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
///
/// # Why it REQUIRES `HumanSeekingHosts` (issue #984)
///
/// `resolve_human_seeking_hosts` is the only writer of that map, and it runs on
/// exactly this marker. If it had to `Commands::insert` the component the first
/// time it ran, the player ship would perform an ARCHETYPE MOVE on a mid-run
/// tick — long after the world settled — and that is not a private bookkeeping
/// detail: Bevy allocates archetype ids in creation order and every query
/// iterates its matched archetypes in that order, so one extra archetype
/// created at that moment re-orders the archetype ids the NPC hulls land in.
/// Two NPC hull groups then swap places in every query that matches both, the
/// per-entity RNG draws and command inserts interleave differently, and the
/// authoritative digest moves — measured: `duel` and `rng_coverage` both moved
/// on nothing but the move itself (a zero-sized dummy marker inserted in the
/// same place reproduced it byte for byte). Requiring the component makes it
/// arrive in the SAME transition as the marker, during the spawn burst, so no
/// mid-run archetype move ever happens and the resolver needs no `Commands`.
#[derive(Component)]
#[require(crate::ship_plugin::HumanSeekingHosts)]
#[require(crate::ship_plugin::VisitingStationHosts)]
#[require(crate::ship_plugin::ScenarioDetailFloor)]
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

// ── Resources ────────────────
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
pub struct ShipBoost(pub crate::ship::boost::BoostState);

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
/// on [`crate::ship::system_registry::GOD_MODE_SYSTEM_ID`] — never written directly
/// from `bridge`'s wasm exports — so the toggle carries a tick, lands in the
/// command log, and a replay reproduces it exactly.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GodMode(pub bool);

/// Instagib cheat: the LocalShip deals 100× damage (issue #1181, formerly the
/// `INSTAGIB` thread-local read ambiently by `console::weapons::beam`).
///
/// Toggled from the host settings cog's Debug/Cheat tab. On native it is never
/// inserted — the toggle is a `#[wasm_bindgen]` export with no native caller —
/// so `tick_beams_apply_damage`'s `Option<Res<Instagib>>` resolves to `None`
/// (off), exactly as the old `is_instagib()` returned a hard-coded `false`
/// there. A wasm-only host debug simulation override, the sibling of [`GodMode`]
/// and [`crate::debug_overlay::SimulationPaused`]; it is declared into the
/// `StateCensus` in `add_simulation_plugins_with` so the enumeration guard
/// accounts for it.
///
/// Lives here — the always-compiled simulation app assembly, beside its sibling
/// [`GodMode`] — rather than in `crate::server::bridge` (issue #1194): it is
/// sim-visible state read by always-compiled weapon code, so the `--server`
/// feature gate must not be able to compile it out. The wasm bridge only
/// mirrors and drains it (`drain_instagib_toggle` / `publish_instagib`).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instagib(pub bool);

#[derive(Resource, Clone, Debug, Default)]
pub struct CaptainPriorityBoost {
    /// scope key (the captain's ship identity) -> currently boosted objective id.
    boosts: std::collections::HashMap<String, String>,
}

impl CaptainPriorityBoost {
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

    /// The selected objective ID to pass to `scored_pool_with_boost` for
    /// `scope`, or `None` when nothing is selected in that scope.
    pub fn boost_arg(&self, scope: &str) -> Option<&str> {
        self.boosted_for(scope)
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
pub(crate) fn apply_god_mode_toggle(
    ship_query: Query<&crate::core::messages::AdmittedCommands, With<LocalShip>>,
    mut god_mode: ResMut<GodMode>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::ship::system_registry::GOD_MODE_SYSTEM_ID) {
        if matches!(
            cmd.payload,
            crate::core::messages::SystemControlPayload::ToggleGodMode
        ) {
            god_mode.0 = !god_mode.0;
        }
    }
}

/// Carries the reason string — and, since #843, the structured
/// [`Outcome`](crate::core::balance::Outcome) — when the game ends. Set before
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
pub struct GameOverReason(
    pub Option<String>,
    pub Option<crate::core::balance::Outcome>,
);

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
/// Drained each logical fixed tick by the `SimBroadcaster` dispatch in
/// `SimSet::Broadcast` while the in-progress fixed loop advances.
///
/// ## Migration note (T1 Architecture PRD #1249, issue #1262)
/// The old preamble pattern (`MessageWriter<OutboundMessage>`) has been
/// eliminated from simulation domain plugins. Producers now choose
/// [`SimOutbox::push_snapshot`] or [`SimOutbox::push_reliable`] at the point
/// where cadence and loss semantics are known. The raw queue is private, so a
/// new producer cannot compile without making that choice. The
/// `sim_outbox_broadcaster()` drains the already-classified entries to the
/// `OutboundMessage` bus without inspecting their payload variants.
/// The intentional direct-message exception is
/// `debug_overlay::report_debug_state`: it emits Reliable `DebugState` from
/// `PreUpdate`, because a pause stops the fixed loop that drains this outbox
/// and the client still has to receive confirmation.
/// PRD #253 introduced the earlier broadcaster seam; #1249 is the parent PRD
/// for this explicit producer-owned delivery classification.
#[derive(Resource, Default)]
pub struct SimOutbox {
    entries: Vec<SimOutboxEntry>,
}

pub(crate) struct SimOutboxEntry {
    pub(crate) target: Target,
    pub(crate) message: ServerMessage,
    pub(crate) delivery: DeliveryClass,
}

impl SimOutbox {
    /// Queue a lossy latest-state projection for the snapshot DataChannel.
    pub fn push_snapshot(&mut self, (target, message): (Target, ServerMessage)) {
        self.push(target, message, DeliveryClass::Snapshot);
    }

    /// Queue an ordered event that must be delivered reliably.
    pub fn push_reliable(&mut self, (target, message): (Target, ServerMessage)) {
        self.push(target, message, DeliveryClass::Reliable);
    }

    /// Queue a batch of latest-state projections with one explicit class.
    pub fn extend_snapshot(&mut self, entries: impl IntoIterator<Item = (Target, ServerMessage)>) {
        self.entries
            .extend(entries.into_iter().map(|(target, message)| SimOutboxEntry {
                target,
                message,
                delivery: DeliveryClass::Snapshot,
            }));
    }

    /// Queue a batch of ordered events with one explicit class.
    pub fn extend_reliable(&mut self, entries: impl IntoIterator<Item = (Target, ServerMessage)>) {
        self.entries
            .extend(entries.into_iter().map(|(target, message)| SimOutboxEntry {
                target,
                message,
                delivery: DeliveryClass::Reliable,
            }));
    }

    /// Read queued entries without exposing a mutable raw insertion path.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Target, &ServerMessage)> {
        self.entries
            .iter()
            .map(|entry| (&entry.target, &entry.message))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn push(&mut self, target: Target, message: ServerMessage, delivery: DeliveryClass) {
        self.entries.push(SimOutboxEntry {
            target,
            message,
            delivery,
        });
    }

    pub(crate) fn drain(&mut self) -> Vec<SimOutboxEntry> {
        std::mem::take(&mut self.entries)
    }
}

/// Owner-local Hull delta-cache compatibility export.
pub use crate::console::repair::visibility::LastBroadcastHull;

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
    pub next_state: Option<ResMut<'w, NextState<crate::core::messages::GamePhase>>>,
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
    pub  std::collections::HashMap<
        crate::core::messages::SystemId,
        crate::core::messages::SystemBlackboard,
    >,
);

// ── Plugin ───────────────────────────────────────────────────────────────────
/// Empty system used as an ordering anchor for the sim broadcast dispatch.
/// All sim-phase systems (message handlers, tick systems, broadcasters) should
/// run before this anchor so that `broadcast::dispatch::<Sim>` (which has
/// `.after(sim_processing_anchor)`) drains their `SimOutbox` writes.
pub fn sim_processing_anchor() {}
