use crate::ship::impulse::ImpulseState;
use bevy::prelude::*;
pub use phoenix_sim_contracts::lifecycle::{GameOverReason, SimOutbox, SimOutboxEntry};
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
/// # The cross-host rule (issue #1116)
///
/// With two ship hosts running one mission, **this marker is on a different
/// ship on each of them**: it says "the ship whose crew is on this machine".
/// That makes the sentence above load-bearing rather than tidy, and sharpens it
/// into a rule a test can hold the code to:
///
/// > **Nothing `LocalShip` gates may reach the authoritative digest.**
///
/// Anything that does is, by construction, a value two hosts compute
/// differently from tick zero — and a fleet whose hosts disagree from tick zero
/// has no mission. `tests/local_ship_neutrality.rs` is the guard: it runs the
/// same seeded world twice, moving the marker to a different fleet ship, and
/// compares the fold on every tick. Two sites failed it when it was written and
/// were fixed rather than blessed (visual banking in `integrate_ship_physics`,
/// which now runs for every ship; and the human-seeking/detail-floor resolvers,
/// which now run for every ship in the fleet off the frozen roster).
///
/// # Why it no longer REQUIREs `HumanSeekingHosts` (issue #984, revised #1116)
///
/// #984 made this marker `#[require]` `HumanSeekingHosts` /
/// `VisitingStationHosts` / `ScenarioDetailFloor` because
/// `resolve_human_seeking_hosts` ran on exactly this marker and would otherwise
/// have had to `Commands::insert` the map on its first run — a MID-RUN
/// ARCHETYPE MOVE, which measurably moved `duel` and `rng_coverage` (proven, not
/// inferred: a zero-sized dummy marker in the same place reproduced both
/// digests byte for byte).
///
/// The requirement has moved rather than gone. Those three components are now
/// inserted by `world_setup::insert_player_core_bundle` on **every ship in the
/// fleet**, in the same spawn burst, whether or not that ship is the local one
/// — so the resolvers still need no `Commands`, no mid-run move can happen, and
/// every fleet hull carries the identical component set regardless of which
/// host is looking at it. A `#[require]` on this marker would have done the
/// opposite: it would have made the *presence* of those components depend on
/// which ship a host tagged, which is precisely the cross-host asymmetry the
/// rule above forbids.
///
/// The standing guard against archetype-creation order moving the digest at all
/// is `tests/archetype_order_determinism.rs` (issue #1052), which reverses the
/// hull groups' archetype ids mid-run and asserts the fold does not care.
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

/// Previous Input-phase hull sample for this drive. A hit applied later in
/// Damage remains pending across a save and is consumed by the next Input pass.
/// None is an unsampled drive; Some(0.0) is a sampled destroyed hull.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ImpulseHullHistory(pub Option<f32>);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
///
/// Per-ship `Component` post ship-parity audit; every ship (player + NPC)
/// carries its own impulse state, used by admitted crew commands and the
/// per-axis AI through the same per-ship pathway.
///
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Default)]
#[require(ImpulseHullHistory)]
pub struct ShipImpulse(pub ImpulseState);

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
pub fn apply_god_mode_toggle(
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

/// The ambient resources every damage chokepoint uses: the seeded RNG it
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
pub struct SimRngAndLog<
    'w,
    const STREAM: usize = { crate::sim_rng::SimStream::CollisionDamage as usize },
> {
    pub rng: crate::sim_rng::LiveStream<'w, STREAM>,
    pub log: Option<Res<'w, crate::logging::LogFilterConfig>>,
    pub god_mode: Option<Res<'w, GodMode>>,
}

impl<const STREAM: usize> SimRngAndLog<'_, STREAM> {
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
