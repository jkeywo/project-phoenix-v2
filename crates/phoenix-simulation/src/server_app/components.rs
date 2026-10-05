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
//! Load-bearing invariant: `HumanSeekingHosts` / `VisitingStationHosts` /
//! `ScenarioDetailFloor` arrive in the spawn-burst archetype transition on
//! EVERY fleet ship — a mid-run archetype move would re-order archetype ids,
//! and making their presence depend on which ship a host tagged `LocalShip`
//! would give two hosts different worlds from tick zero (see `LocalShip`).

use super::*;

// ── Marker Components ────────

// ── Resources ────────────────

// ShipShields has moved to `crate::ship::shields` as a Component.
pub use crate::ship::shields::ShipShields;

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
    pub(crate) mission: crate::crew_spectator::CrewMissionPolicy<'w>,
    pub next_state: Option<ResMut<'w, NextState<crate::core::messages::GamePhase>>>,
    pub reason: Option<ResMut<'w, GameOverReason>>,
}

// ── Plugin ───────────────────────────────────────────────────────────────────
/// Empty system used as an ordering anchor for the sim broadcast dispatch.
/// All sim-phase systems (message handlers, tick systems, broadcasters) should
/// run before this anchor so that `broadcast::dispatch::<Sim>` (which has
/// `.after(sim_processing_anchor)`) drains their `SimOutbox` writes.
pub fn sim_processing_anchor() {}

#[cfg(test)]
#[path = "components_interior_write_access_tests.rs"]
mod interior_write_access_tests;

pub use phoenix_sim_contracts::lifecycle::{GameOverReason, SimOutbox, SimOutboxEntry};

pub use phoenix_sim_gameplay::server_app::*;
