//! Simulation integration facade; implementation ownership is split below.
//! The authoritative world snapshot (issue #862) — phoenix's half of a save.
//!
//! # What this module owns, and what it deliberately does not
//!
//! Phoenix supplies exactly one thing: the **payload**. [`PhoenixSnapshot`] is
//! the captured authoritative state, and [`capture`]/[`restore`] are the two
//! walks that get it out of and back into a live ECS world.
//!
//! Everything around it comes from `vellum-save` and is not re-invented here:
//!
//! | concern | who answers it |
//! |---|---|
//! | the envelope a save is written in | `vellum_save::Run` |
//! | the three version dimensions | `vellum_save::Versions` |
//! | "why won't this load?" | `vellum_save::Moved` |
//! | where the bytes live | `vellum_save::Store` |
//! | "did the restore reproduce the capture?" | `vellum_save::verify` |
//!
//! There is no phoenix envelope, no phoenix version field, and no phoenix
//! compatibility validator, and that is a constraint rather than an omission: a
//! second validator would be a second, quietly disagreeing answer to a question
//! `Versions::check` has already settled, and a host shown a phoenix-invented
//! status would learn less than one shown `Moved` — which names *which*
//! dimension moved and to what.
//!
//! # What the payload covers, and why exactly that
//!
//! The boundary is issue #894's authoritative-state record, and this module
//! does not get to have its own opinion about where it runs. `world_digest`
//! (`crate::sim_digest`) is that record made executable — it is the enumerated
//! list of what a divergence is *defined over* — so the payload is built to be
//! the same list, walked in the same order:
//!
//! | `world_digest` folds | [`PhoenixSnapshot`] carries |
//! |---|---|
//! | `SimTick` | [`PhoenixSnapshot::tick`] |
//! | `SimulationPaused` | [`PhoenixSnapshot::paused`] |
//! | `GmActionJournal` | [`PhoenixSnapshot::gm_actions`] |
//! | `StationPuppets` | [`PhoenixSnapshot::gm_puppets`] |
//! | accepted pending GM Station commands | [`PhoenixSnapshot::gm_station_commands`] |
//! | armed GM direct damage/heal effects (with their Station/System scope) | [`PhoenixSnapshot::gm_direct_effects`] |
//! | GM-attributed faction hostility overrides | [`PhoenixSnapshot::gm_faction_overrides`] |
//! | sensor exposure of GM placements, with its permanent undo latch | [`PhoenixSnapshot::gm_spawn_exposure`] |
//! | per-NPC elapsed idleness behind the GM idle advisory | [`PhoenixSnapshot::gm_idle_npcs`] |
//! | `SimRng`'s six stream positions | [`PhoenixSnapshot::rng`] (`SimRngState`) |
//! | `WorldIdMint`'s tick + per-namespace counters | [`PhoenixSnapshot::mint`] |
//! | `GamePhase` | [`PhoenixSnapshot::phase`] |
//! | `GameOverReason` (reason + `Outcome`) | [`PhoenixSnapshot::game_over`] |
//! | `CaptainPriorityBoost`'s sorted pairs | [`PhoenixSnapshot::captain_boosts`] |
//! | the `WorldResource` projection | [`PhoenixSnapshot::world`] (the whole `WorldData`) |
//! | the `EntityUuid` namespace | [`PhoenixSnapshot::entities`] |
//! | the `AsteroidUuid` namespace | [`PhoenixSnapshot::asteroids`] |
//! | shared collision attribution | [`PhoenixSnapshot::collisions`] |
//!
//! Two entries are deliberately *wider* than the fold rather than equal to it,
//! and both are widened toward the record, never away from it:
//!
//! * `world` stores the whole `WorldData`, not the seven-field projection the
//!   digest narrows to. The record lists `WorldResource` as IN unqualified; the
//!   narrowing is `sim_digest`'s own honest under-coverage, and a payload that
//!   copied the narrowing would drop authored geometry a resumed session still
//!   has to broadcast to its clients.
//! * `asteroids` carries each rock's config path, orientation and shield
//!   pierce alongside the position the digest folds, and
//!   [`PhoenixSnapshot::asteroid_window`] carries the streamer's own progress.
//!   The record's asteroid namespace is about rocks that *exist*; on a world
//!   with streamed belts, which rocks exist is a fact about the streamer, and a
//!   restore that could not rebuild one would be short of exactly the rocks the
//!   capture's digest counted.
//! * `flags` (the world `FlagStore`s, base and per-layer) is in because a
//!   scenario's trigger state is what makes a bounded Combat Test *bounded* —
//!   `wave_3_cleared` is authoritative, and was authoritative here before
//!   anything folded it. It is named by this issue's own acceptance criteria,
//!   and `FlagStore` gained serde for exactly this. Issue #1086 later folded it
//!   too (`sim_digest::fold_scenario_flags`), which is the payload's field list
//!   being used as the fold's membership test rather than the other way round.
//! * `scenario` ([`ScenarioState`], issue #864) is in for the same reason
//!   widened one step: flags are what a scenario *remembers*, and this is what
//!   it has already *done* and is still *owed* — every trigger's single-shot
//!   latch, the mission clock those latches are timed against, and the queue of
//!   deferred `after(n, |ctx| …)` script callbacks. Nothing folded any of it
//!   when it landed — issue #1086 does now — and a resumed scenario without it
//!   replays its own opening: spent
//!   triggers re-arm and fire a second time, pending callbacks are forgotten,
//!   and `on_timer` thresholds are measured from the age of the fresh app
//!   rather than of the run.
//! * `comms` ([`CommsState`], issue #984's S8) is the same widening applied to
//!   the conversation a scenario is in the middle of *having*: the inbox, the
//!   dialogue entries that make its messages answerable, the template latches
//!   that stop them being injected twice, and the scripted `open_comms`
//!   requests queued but not yet materialised. Nothing folded any of it when it
//!   landed either — `sim_digest::fold_comms_scope` does now — and without it a
//!   save taken mid-thread comes back to an empty Comms console with a scenario
//!   waiting on an answer that can no longer be given.
//!
//! **Excluded, and the exclusion is the design.** Browser UI state, transport
//! sessions, renderer caches, client projections, and raw ECS `Entity` handles
//! are all absent. Every per-entity row here is keyed by its `EntityUuid` or
//! `AsteroidUuid` string, never by a handle — the same discipline the command
//! log applied when it refused to record session tokens (#898), for a related
//! reason: a handle is a slot in *this* process's ECS, so a stored handle is a
//! number that means something different every time it is read.
//!
//! # Wider than the digest, and why that is not a re-blessing
//!
//! The payload also carries the **weapon and repair state machines** that AC2
//! names by hand: [`WeaponState`] (live beams with their fractional damage
//! debt, per-bank phaser cooldowns, tube contents and load timers, torpedoes in
//! flight, pending burst volleys, per-arc shield hull) and [`RepairState`]
//! (each team's slot, the request queue, the "already told the crew" latch).
//! Alongside them it carries the AI state a continuation turns out to hang off
//! just as hard: [`PhoenixSnapshot::ai_policy_clock`], each ship's
//! [`RecoveryHistory`] windows, its [`EntityState::patrol_cursors`], and its
//! frozen [`EntityState::blackboards`]. Every one of those was found the same
//! way — by measuring where a restored world stopped agreeing with the live one
//! and asking why, rather than by reasoning about what a save "should" hold.
//!
//! None of that is folded by `world_digest`, and none of it moves the digest by
//! being here. That distinction is the whole reason this is allowed:
//! `sim_digest` is the definition of what a *divergence* is, and widening it is
//! a re-blessing event under #894's AC4. A payload is the definition of what a
//! *continuation* needs, and the two are not the same list. A restored ship
//! whose beams were extinguished, whose tubes were emptied and whose repair
//! teams were sent home stands at a matching digest and then behaves
//! differently on the very next tick — which is a divergence the digest
//! discovers a tick late and cannot explain.
//!
//! # Honestly *still* not covered, and what that costs
//!
//! Not captured today: power allocation, modifier caches, the doctrine
//! objective evaluator's own per-ship derivation (which is what still bounds a
//! low-LOD ship's continuation — see `tests/snapshot_resume.rs`), and rapier's
//! own rigid-body velocities (phoenix's `ShipPhysics` is restored; the solver's
//! internal state is not).
//!
//! A restore reproduces the captured *digest* exactly — that is what
//! [`vellum_save::verify`] checks — and then continues from state that is
//! complete over the record, complete over the machines above, and default over
//! everything else. `tests/snapshot_resume.rs` measures how far that carries
//! and writes the number down rather than tuning around it.
//!
//! # A restore is mostly not a `spawn`
//!
//! [`restore`] does not build a world from nothing. It is handed a world that
//! the *same scenario* has already bootstrapped — `Run::scenario` and
//! `Run::seed` are what say which — and it overwrites that world's
//! authoritative state with the capture's. Entities are matched by uuid, and
//! anything the bootstrap spawned that the capture did not have is despawned.
//!
//! The exception is **what the bootstrap cannot make** (issue #863). A world
//! file's `[[entity]]` blocks come back with any fresh boot; a ship a *script*
//! spawned mid-run does not, unless the fresh app happens to replay the same run
//! to the same point — which a browser session resuming with nobody at the
//! consoles does not do. So a captured row that names no live entity is built
//! from its [`EntityState::spawn`] record if it has one, and reported as a
//! [`RestoreGap`] if it does not. A silent gap is the failure mode worth
//! engineering against either way: it restores *most* of a world and then
//! diverges for a reason nothing in the save points at.
//!
//! # Destroyed streamed rocks, and the respawn policy this payload declares
//!
//! Combat Test's belts are streamed, so "which rocks exist" is a fact about the
//! streamer rather than about the world file, and destruction inside a streamed
//! cell is recorded by *absence* in two places at once: the rock is not in
//! [`PhoenixSnapshot::asteroids`], and its cell's slot in
//! [`PhoenixSnapshot::asteroid_window`] is empty. Both travel, so a restore
//! despawns the rock the fresh app had streamed in alive and installs a window
//! that agrees the cell is empty.
//!
//! **Identity is the cell, never the handle and never a mint.** A streamed rock's
//! `AsteroidUuid` is `deterministic_cell_uuid(0, gx, gz, gx mod size, gz mod size)`
//! (`crate::asteroids::lifecycle`) — a pure function of its lattice cell, because
//! ring addressing makes the slot a pure function of the cell too. Two runs of
//! the same content name the same rock the same thing, and so does a re-stream
//! after a resume, which is what lets absence mean "destroyed" rather than "some
//! other rock". Nothing here stores an ECS handle or draws from
//! [`WorldIdMint`], and neither could work: a handle is a slot in one process,
//! and a mint counter is a fact about when the rock happened to be streamed.
//!
//! **The declared policy for leaving and re-entering a cell after a restore is
//! the same one the live simulation follows: the rock respawns, whole.** That is
//! AGENTS.md's Key Constraint 8 — "destroyed asteroids respawn fresh when the
//! player leaves the cell and returns" — and a resume does not get to have a
//! second opinion about it. Destruction of a streamed rock is a fact about the
//! *current residency* of its cell in the window, not about the world, and it is
//! exactly as durable in a resumed run as in a live one: it survives for as long
//! as the cell stays streamed in, and no longer. A payload that made it durable
//! would be a resumed world that plays differently from the one it resumed —
//! belts that thin out permanently where a live run's refill themselves, and a
//! save file that grows without bound, one record per rock ever shot. Hand-placed
//! rocks (an authored `[[entity]]`, not a streamed cell) are a different thing
//! and stay destroyed the way any other authored entity does, through the same
//! surplus sweep.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use vellum_save::{Ledger, Run, Snapshot, Versions};

use crate::asteroids::lifecycle::{AsteroidData, AsteroidEntityMap, AsteroidWindow};
use crate::command_admission::log::LoggedCommand;
use crate::comms::content::{
    ActiveDialogue, CommsDialogueNode, CommsResponse, OpenCommsRequest, ScriptedDialogue,
};
use crate::comms::server::{CommsInboxRes, CommsRuntime};
use crate::console::repair::server::{RepairQueueEntry, RepairRequestQueue, ShipRepairTeams};
use crate::console::weapons::beam::{ActiveBeam, ActiveBeamSlot, PhaserCooldown};
use crate::console::weapons::torpedo::TorpedoSystemResource;
pub use crate::core::collision_history::CollisionRecord;
use crate::core::messages::{CommsMessage, GamePhase, SystemId, WorldData};
use crate::core::telemetry::RunTelemetry;
use crate::dossier::evidence::EvidenceLog;
use crate::entities::spawner::{EntityShipArcHull, EntitySystemHull, EntityUuid};
use crate::lobby::WorldResource;
use crate::server_app::{
    AsteroidUuid, CaptainPriorityBoost, GameOverReason, ShipBoost, ShipImpulse,
};
use crate::ship::components::RepairHumanAlerted;
use crate::ship::impulse::ImpulsePhase;
use crate::ship::state::{ShipPhysics, ShipRedAlert};
use crate::sim_rng::{SimRng, SimRngState};
use crate::sim_tick::SimTick;
use crate::weapons::torpedo::{Torpedo, TubeBurstState, TubeLoadState};
use crate::world::commitments::CommitmentLedger;
use crate::world::content::WorldEvent;
use crate::world::deadlines::DeadlineTable;
use crate::world::flags::FlagStore;
use crate::world::script::schedule::{PendingCallbacks, ScheduledCall};
use crate::world::server::{WorldContentRuntime, WorldScriptRuntime};
use crate::world_id::{WorldIdMint, WorldIdMintState};

/// Compatibility path for the unchanged saved Power payload. The reactor owns
/// its projection and conversion; this module owns traversal and restore order.
pub use crate::modifiers::power_system::PowerState;

/// Compatibility paths for the unchanged saved control payloads. Component
/// conversion belongs to the Ship; this module retains identity joins and order.
pub use crate::ship::continuation::{ControlState, PolicyState, RecoveryHistory};

/// One scenario trigger's runtime state — the three fields a run *changes*.
///
pub use crate::world::trigger_registry::TriggerRuntimeState;

mod artifact;
#[cfg(test)]
mod physical_preflight_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod shield_tests;
#[cfg(test)]
#[path = "../snapshot_tests.rs"]
mod tests;
pub use artifact::content_digest;
pub use artifact::export_artifact;
pub use artifact::import_artifact;
pub use artifact::load_from;
pub use artifact::peek_artifact_scenario;
pub use artifact::required_boot_identity;
pub use artifact::run_for;
pub use artifact::save_to;
pub use artifact::versions;
pub use artifact::LoadRefusal;
pub use artifact::NoContinuationLog;
pub use artifact::SavedGame;
pub use artifact::StoredRun;
pub use artifact::TransferStore;
pub use artifact::DEFAULT_SLOT;
pub use artifact::EXPORT_FILE_NAME;
pub use artifact::SIMULATION_RULES;
pub use artifact::SNAPSHOT_FORMAT;
pub use artifact::STORAGE_NAMESPACE;

mod schema;
pub use schema::AiFidelityState;
pub use schema::ArcRequestState;
pub use schema::AsteroidState;
pub use schema::AsteroidWindowState;
pub use schema::BlasterBolt;
pub use schema::BlasterRuntime;
pub use schema::BootIdentity;
pub use schema::BurstState;
pub use schema::CommsState;
pub use schema::CoordinationEnqueueState;
pub use schema::CoordinationQueueState;
pub use schema::DialogueState;
pub use schema::DriveState;
pub use schema::EntityState;
pub use schema::FrequencyHintContinuationState;
pub use schema::GameStartEntityUuid;
pub use schema::HelmWaypointClearanceState;
pub use schema::IntentNarrationState;
pub use schema::IntentSnapshotState;
pub use schema::LastSystemTiersState;
pub use schema::LayerFlags;
pub use schema::MeshControlSource;
pub use schema::MeshCrewState;
pub use schema::MotionPlanState;
pub use schema::NavigationClearanceIssueState;
pub use schema::NavigationWaypointState;
pub use schema::NpcFrequencyMatchState;
pub use schema::NpcFrequencyMatchStatesState;
pub use schema::PhoenixSnapshot;
pub use schema::PowerBrownoutContinuationState;
pub use schema::QueuedCoordinationState;
pub use schema::RecentCombatActivityState;
pub use schema::RepairState;
pub use schema::ScenarioState;
pub use schema::SensorsThreatMemoryState;
pub use schema::ShieldsAiState;
pub use schema::ShieldsCoordinationContinuationState;
pub use schema::ShipPhaserFrequencyState;
pub use schema::SnapshotReportRow;
pub use schema::TacticalFrequencyHintInboxState;
pub use schema::TorpedoInFlight;
pub use schema::TrackedEntitiesState;
pub use schema::TubeState;
pub use schema::WeaponState;
pub use schema::WindowSlot;
pub use schema::WorldEventRecord;

mod capture;
mod crew_presence;
mod flight_carriers;
mod guided_rounds;
mod historical_clock;
pub use capture::capture;
pub(crate) use capture::capture_entity_state;
use capture::fixed_elapsed_secs;

mod application;
pub(crate) use application::apply_entity_state;
use application::apply_entity_state_with;
use application::apply_hull;
use application::rebuild_power_modifiers;
use application::restore_ai_cadence;
use application::restore_ai_world_snapshot;
use application::restore_coordination_staging;
pub(crate) use application::restore_mesh_crew;
use application::restore_motion_plans;
use application::restore_npc_frequency_matches;
use application::restore_run_scope;
use application::validate_entity_repair_jobs;

mod fitting;
mod ports;
mod preflight;
pub(crate) use ports::assert_settled as assert_ports_settled;
#[cfg(test)]
mod ports_tests;
use preflight::inactive_layer_paths;
use preflight::layer_topology_gap;
use preflight::live_layer_topology;
use preflight::pending_layer_changes;
pub use preflight::ready_to_rebuild;
pub use preflight::ready_to_restore;
use preflight::saved_layer_topology;
pub use preflight::validate_boot_identity_for_world;
pub use preflight::RestoreGap;

mod restore;
pub use restore::restore;
pub use restore::RestoreReport;

mod materialization;
pub use materialization::reconcile_world_layers;
use materialization::restore_asteroids;
use materialization::restore_entities;
pub(crate) use materialization::spawn_from_origin;
pub use materialization::LayerReconcileStatus;

mod activities;
pub(crate) use activities::validate_single_row as validate_activity_single_row;
mod craft;
mod emp;
mod helm_movement;
#[cfg(test)]
mod mission_clock_tests;
mod mounts;
#[cfg(test)]
mod navigation_sharing_tests;
mod personnel;
mod sensing_publications;
mod shield_enquiry;
mod spatial_fields;
mod transit;

mod guided_source;
mod illumination;
mod jamming;
mod protection;
#[cfg(test)]
mod sustained_beam_tests;
mod weapon_targets;

#[cfg(test)]
mod blaster_spatial_tests;

mod comms_requests;
