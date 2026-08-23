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
//! | `SimRng`'s six stream positions | [`PhoenixSnapshot::rng`] (`SimRngState`) |
//! | `WorldIdMint`'s tick + per-namespace counters | [`PhoenixSnapshot::mint`] |
//! | `GamePhase` | [`PhoenixSnapshot::phase`] |
//! | `GameOverReason` (reason + `Outcome`) | [`PhoenixSnapshot::game_over`] |
//! | `CaptainPriorityBoost`'s sorted pairs | [`PhoenixSnapshot::captain_boosts`] |
//! | the `WorldResource` projection | [`PhoenixSnapshot::world`] (the whole `WorldData`) |
//! | the `EntityUuid` namespace | [`PhoenixSnapshot::entities`] |
//! | the `AsteroidUuid` namespace | [`PhoenixSnapshot::asteroids`] |
//! | collision attribution from `RunTelemetry` | [`PhoenixSnapshot::collisions`] |
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
//!   `wave_3_cleared` is authoritative even though nothing folds it yet. It is
//!   named by this issue's own acceptance criteria, and `FlagStore` gained
//!   serde for exactly this.
//! * `scenario` ([`ScenarioState`], issue #864) is in for the same reason
//!   widened one step: flags are what a scenario *remembers*, and this is what
//!   it has already *done* and is still *owed* — every trigger's single-shot
//!   latch, the mission clock those latches are timed against, and the queue of
//!   deferred `after(n, |ctx| …)` script callbacks. Nothing folds any of it
//!   either, and a resumed scenario without it replays its own opening: spent
//!   triggers re-arm and fire a second time, pending callbacks are forgotten,
//!   and `on_timer` thresholds are measured from the age of the fresh app
//!   rather than of the run.
//! * `comms` ([`CommsState`], issue #984's S8) is the same widening applied to
//!   the conversation a scenario is in the middle of *having*: the inbox, the
//!   dialogue entries that make its messages answerable, the template latches
//!   that stop them being injected twice, and the scripted `open_comms`
//!   requests queued but not yet materialised. Nothing folds any of it, and
//!   without it a save taken mid-thread comes back to an empty Comms console
//!   with a scenario waiting on an answer that can no longer be given.
//!
//! **Excluded, and the exclusion is the design.** Browser UI state, PeerJS
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
use crate::console::weapons::beam::{
    ActiveBeam, ActiveBeamSlot, LastShipAttacker, PhaserCooldown, TacticalRadarSelection,
};
use crate::console::weapons::torpedo::TorpedoSystemResource;
use crate::core::balance::{BalanceEvent, StampedBalanceEvent, VictimKind, WEAPON_KIND_COLLISION};
use crate::core::messages::{CommsMessage, GamePhase, SystemId, TeamSlot, WorldData};
use crate::core::telemetry::RunTelemetry;
use crate::dossier::evidence::EvidenceLog;
use crate::entities::spawner::{EntityShipArcHull, EntitySystemHull, EntityUuid};
use crate::lobby::WorldResource;
use crate::server_app::{AsteroidUuid, CaptainPriorityBoost, GameOverReason};
use crate::ship::components::LastHelmInput;
use crate::ship::components::RepairHumanAlerted;
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
use crate::ship::helm_ai::{
    HelmBoostAiPolicyState, HelmEnginesAiPolicyState, HelmSteeringAiPolicyState,
};
use crate::ship::impulse::ImpulsePhase;
use crate::ship::state::{ShipPhysics, ShipRedAlert, ShipWeaponsHold};
use crate::sim_rng::{SimRng, SimRngState};
use crate::sim_tick::SimTick;
use crate::weapons::torpedo::{Torpedo, TubeBurstState, TubeLoadState};
use crate::world::commitments::CommitmentLedger;
use crate::world::content::WorldEvent;
use crate::world::deadlines::DeadlineTable;
use crate::world::flags::FlagStore;
use crate::world::script::schedule::{PendingCallbacks, ScheduledCall, TickBudget};
use crate::world::server::{WorldContentRuntime, WorldScriptRuntime};
use crate::world_id::{WorldIdMint, WorldIdMintState};

// ── The three version dimensions ─────────────────────────────────────────────

/// The payload's byte layout, bumped by hand whenever [`PhoenixSnapshot`]'s
/// shape changes in a way an older save cannot be read as.
///
/// This is a *phoenix* constant that `vellum_save::Versions` carries, not a
/// phoenix version field: the comparison, the ordering of the three checks, and
/// the refusal all belong to `Versions::check`.
///
/// `2` — issue #864 added [`PhoenixSnapshot::scenario`]. Every new field
/// carries `#[serde(default)]`, so a format-1 save still *parses*; the bump is
/// here because parsing it is exactly the wrong outcome. A format-1 payload
/// carries no trigger fired-state, no mission clock and no pending script
/// callbacks, so restoring one would silently re-arm every scenario trigger the
/// run had already spent and drop every deferred callback it was waiting on —
/// the same class of silent gap [`RestoreGap`] exists to refuse out loud. A
/// save this build cannot honour is refused by `Versions::check`, which names
/// the dimension.
///
/// `3` — issue #984's S8 added [`PhoenixSnapshot::comms`], and the same
/// reasoning applies unchanged. Every new field defaults, so a format-2 save
/// parses, and a format-2 save of a comms-quiet world would even restore
/// *correctly* — which is precisely why the version cannot be left at 2. The
/// payload has no way to distinguish "this world had no conversation open" from
/// "this save predates conversations being recorded", and the second one
/// restores a scenario mid-thread into a world with an empty inbox, no dialogue
/// to answer, and every `open_comms` request the run had queued discarded. That
/// is silently wrong in exactly the way a re-armed trigger latch was, so it is
/// refused on the same dimension.
///
/// `4` - issue #985 (Rhai M7) deleted the declarative `[[comms]]` front-end, and
/// [`CommsState`] lost the three fields that only described it: `template_fired`
/// (a `[[comms]]` template's single-shot latch), `uncarried_follow_ups` (the
/// `PendingFollowUp` queue's placeholder ids) and `uncarried_dialogues` (the
/// count of nodes whose responses carried `TriggerAction`s). [`DialogueState`]
/// also lost `speaker`, and its `script` became required rather than optional -
/// every live dialogue is a scripted one now.
///
/// The bump is the same argument the previous two made, run the other way. A
/// format-3 save still *parses* here: serde ignores the three retired fields and
/// defaults the rest. That is precisely the wrong outcome. A format-3 payload
/// recorded mid-conversation on a declarative thread carries a spent template
/// latch this build has nowhere to put and a `script: None` dialogue this build
/// cannot answer, so restoring it would re-arm a broadcast that had already
/// fired and seat a message no `on_pick` exists for. Nothing in the payload
/// distinguishes that save from one whose world simply had no declarative
/// content, so both are refused by `Versions::check`, which names the dimension.
///
/// `5` — issue #1024 added [`ScenarioState::deadlines`], and this is the format-2
/// argument run again on a new vocabulary. Every new field carries
/// `#[serde(default)]`, so a format-4 save still parses — and a format-4 save of
/// a deadline-free world would restore correctly, which is exactly why the
/// constant cannot be left at 4. A format-4 payload carries no deadline record at
/// all, so restoring one into a world that authors `[[deadline]]` blocks re-arms
/// every deadline the run had cancelled, rewinds every one it had slipped back to
/// its authored due time, and un-fires the ones already spent — the same class of
/// silent re-arming that moved this constant to 2 for trigger latches. Nothing in
/// the payload distinguishes that save from one whose world simply authored no
/// deadlines, so both are refused by `Versions::check`, which names the dimension.
///
/// `6` — issue #1029 added [`ScenarioState::commitments`], and the argument is
/// the same one a third time with one twist that makes it sharper. A promise is
/// a RUNTIME artifact: no world file declares one, so nothing in the content
/// digest changes when a run happens to make one, and a format-5 payload of a
/// run that had given its word is byte-indistinguishable from a format-5 payload
/// of a run that had not. Restoring the first into a #1029 build resumes a
/// captain who has promised nothing — which is not merely missing state, it is a
/// *plausible* state, and the ledger's own "unknown" answer is what a scenario
/// guards a duplicate promise with. So the resumed run would re-offer a word
/// already given, and every campaign flag that promise was going to write would
/// be written twice or not at all. Nothing in the payload distinguishes that
/// save from one whose run simply had not reached the negotiation yet, so both
/// are refused by `Versions::check`, which names the dimension.
///
/// `7` — issue #1031 added [`ScenarioState::evidence`], and it is the #1029
/// argument with the twist turned one notch further. A finding is a runtime
/// artifact for the same reason a promise is — no world file declares one, so
/// the content digest is silent about it — but where a missing promise resumes a
/// captain who has said nothing, a missing FINDING resumes a crew who never
/// learned something, and the whole Thin Margin arc is about what the crew know
/// and how they came to know it. A format-6 payload of a run that had scanned the
/// skyhook is byte-indistinguishable from one that had not; restoring the first
/// into a #1031 build hands the crew back a blank intelligence file, and because
/// the store deduplicates on `(subject, provenance, text)` rather than on a
/// counter, a re-scan would re-stamp the finding at the resumed tick and quietly
/// rewrite when they found out. Nothing in the payload distinguishes that save
/// from one whose run had simply not scanned anything yet, so both are refused by
/// `Versions::check`, which names the dimension.
///
/// `8` — issue #1035 added [`ScenarioState::workforce`], and this is #1024's
/// argument rather than #1029's: a strike IS declared in the world file, so it
/// might look as though the content digest could stand in. It cannot, and the
/// reason is one line in [`crate::world::config`]: `RawWorld` sets no
/// `deny_unknown_fields`. A build that predates this vocabulary therefore loads
/// a world authoring `[[workforce]]` perfectly happily, drops the table on the
/// floor, and writes a format-7 save of the SAME files with the SAME content
/// digest — a save that says nothing about a dispute the world is entirely
/// about. Restoring it here arms an empty register, so every structure the
/// strike was gating reads as worked: the depot that was refusing transfers
/// takes one, and the repair that was running unassisted comes back at full
/// rate. Nothing in the payload distinguishes that save from one whose world
/// simply declared no sides, so both are refused by `Versions::check`, which
/// names the dimension.
///
/// `9` — issue #1041 added [`EntityState::weapons_hold`], and it is the
/// simplest of these arguments: the field is on every ship row, so the payload
/// shape moved, and what it records is an ORDER. A format-8 save of a ship
/// whose captain had called a weapons hold is byte-indistinguishable from one
/// whose captain had not; restoring the first into a #1041 build resumes a crew
/// who had chosen restraint with their guns live, on the very tick the scenario
/// is weighing what they chose. The alert beside it has been persisted from the
/// beginning for exactly this reason, and half a firing posture is not a
/// posture. Nothing in the payload distinguishes that save from one taken with
/// the lever never pulled, so both are refused by `Versions::check`, which names
/// the dimension.
///
/// `10` — issue #863 added [`EntityState::spawn`] and
/// [`ScenarioState::name_to_uuid`], and this is the argument #1035 made turned
/// all the way around: not "the content digest cannot see this", but "the
/// content digest is *identical* and the world is still different".
///
/// A format-9 save of a Combat Test run carries every wave NPC's hull, helm,
/// weapons and blackboards — and nothing at all about where those ships came
/// from. That was survivable while a restore was only ever an overwrite, because
/// a fresh boot of the same scenario re-ran the same script from the same seed
/// and re-minted the same uuids, so the roster the capture named was always
/// standing by the time the restore ran. What it was silently relying on is a
/// *replay*, and a resumed browser session is precisely where the replay does
/// not happen: the fresh app boots with nobody at the consoles, so a wave
/// released by a player's action is a wave the bootstrap never releases, and
/// every ship in it comes back as a [`RestoreGap::MissingEntity`] — or, worse,
/// as a `ready_to_restore` that never becomes true and a resume that simply
/// hangs. Two saves of the same scenario, the same files, the same content
/// digest, and one of them describes a raid this build cannot rebuild.
///
/// So the payload now records what each mid-run spawn was made from, and the
/// restore builds it. Both new fields carry `#[serde(default)]`, so a format-9
/// save still parses — and a format-9 save of a world that never spawns anything
/// at runtime would even restore correctly, which is exactly why the constant
/// cannot be left at 9. Nothing in the payload distinguishes that save from one
/// whose run had a whole raid on the board, so both are refused by
/// `Versions::check`, which names the dimension.
///
/// `11` — issues #1107–#1109 added [`EntityState::station_stances`], and it is
/// #1041's argument on a fresh lever. The per-ship Command stance map
/// (`ShipStationStances`) IS folded into the sim digest by
/// `sim_digest::fold_station_stances_namespace`, but nothing in the payload
/// carried it, so a resume dropped every stance an AI, human or objective order
/// had put in force and folded a different number than the capture: a destroyer
/// that had chosen its `ai_engaged` stance at red alert comes back undirected,
/// on the very tick the fold counts it. The field carries `#[serde(default)]`,
/// so a format-10 save still parses — and a format-10 save of a run in which no
/// stance was ever selected would even restore correctly, which is exactly why
/// the constant cannot be left at 10. Nothing in the payload distinguishes that
/// save from one whose crew had directed a Station, so both are refused by
/// `Versions::check`, which names the dimension.
pub const SNAPSHOT_FORMAT: u32 = 11;

/// The simulation, as a string because "0.1-pre" says more in a bug report than
/// "1" and because nothing compares these for order.
///
/// Bump it whenever the simulation's rules change, however slightly. A save
/// recorded under other rules is not a save this build can honour, and the only
/// available honesty is to refuse it — `Run` has no migration hook and is not
/// meant to.
pub const SIMULATION_RULES: &str = "0.1";

/// The authored data, computed rather than remembered.
///
/// Issue #935: this used to hash the scenario TOML text alone, which left
/// entity templates, fragments, and sidecars free to drift under a save. It
/// now folds a [`crate::content_ledger::ContentLedger`] — every authored file
/// the world/entity loader actually consumed for this load, scenario TOML
/// included, keyed by canonical path. Taking the ledger rather than a lone
/// string is deliberate: the digest can only be honest about what the caller
/// hands it, and a bare `&str` parameter invited exactly the narrow read that
/// caused this issue.
///
/// `fnv1a`/`fold_digest` rather than a phoenix-local hash, because
/// `vellum-digest` is already the fleet's digest primitive and a second one
/// would be a second answer.
pub fn content_digest(ledger: &crate::content_ledger::ContentLedger) -> u64 {
    ledger.fold()
}

/// The three dimensions this build writes and reads saves against.
pub fn versions(ledger: &crate::content_ledger::ContentLedger) -> Versions {
    Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, content_digest(ledger))
}

/// The stored artifact's full type: `vellum-save`'s envelope, phoenix's payload.
///
/// Named because it appears in four signatures and spelling it out invites the
/// two parameters being swapped. `LoggedCommand` is the log's element type even
/// though this issue always stores an empty log — the type is what makes #849's
/// continuation log a filled-in field rather than a new artifact.
pub type StoredRun = Run<LoggedCommand, PhoenixSnapshot>;

// ── The payload ──────────────────────────────────────────────────────────────

/// One `EntityUuid`-bearing entity's authoritative state.
///
/// Keyed by the uuid string, never by an ECS handle — see the module docs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    pub uuid: String,
    /// What a **script spawned this entity from** (issue #863), or `None` for
    /// every authored `[[entity]]` block.
    ///
    /// The one field on this row that is not state the entity *has* but the
    /// recipe it was *made by*, and it is here because the row is otherwise
    /// unusable when the target world has no such entity to write into. A fresh
    /// app re-spawns the authored roster from the world file, so an authored
    /// ship is always there to be overwritten; a mid-run spawn is there only if
    /// the bootstrap happened to replay the run that produced it, which a
    /// resumed browser session — booting with nobody at the consoles — does not.
    ///
    /// With the origin, [`restore`] builds the ship instead of reporting a
    /// [`RestoreGap::MissingEntity`]; the rest of this row then lands on it
    /// exactly as it lands on a bootstrapped one. Without it, the gap is still
    /// the honest answer: see [`crate::world::spawn_origin::SpawnOrigin`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn: Option<crate::world::spawn_origin::SpawnOrigin>,
    /// `ShipPhysics`' eight fields, in the order the digest folds them:
    /// `x, y, z, yaw, forward_speed, roll, lateral_speed, vertical_speed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<[f32; 8]>,
    /// `(SystemId, current, max)` per system, in the hull's own stable
    /// insertion order — the same walk `fold_hull` takes. The tier thresholds
    /// and display names are NOT stored: they are authored config the fresh
    /// world rebuilds from TOML, and storing them would put authored data in a
    /// save that a content-digest change is already meant to invalidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_alert: Option<bool>,
    /// The captain's weapons hold (issue #1041) — the restraint lever layered
    /// under the alert above. Stored beside it because the two are one firing
    /// posture: a save that remembered the alert and forgot the hold would
    /// resume a ship that had been ordered to hold fire with its guns live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapons_hold: Option<bool>,
    /// The ship's Command stance selections (issues #1107–#1109) as
    /// `(station id, stance id)`, sorted by station id.
    ///
    /// The per-ship `ShipStationStances` map, which is folded into the sim
    /// digest by `sim_digest::fold_station_stances_namespace` but — before this
    /// field — did not travel, so a resume dropped every stance an AI, human or
    /// objective order had put in force and folded a different number than the
    /// capture. Stored as sorted scalar pairs for `fold`'s own reason: the map's
    /// `HashMap` iteration order is not stable across instances, and the capture
    /// has to be byte-identical whatever order the entries were inserted in.
    ///
    /// EMPTY is the load-bearing default — a hull nobody commands carries an
    /// empty map — so an absent or empty vec restores to an empty map, which is
    /// byte-identical to a never-commanded ship and folds to the same number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub station_stances: Vec<(String, String)>,
    /// The helm axes as they stood at the capture — see [`ControlState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlState>,
    /// The weapon state machines — see [`WeaponState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapons: Option<WeaponState>,
    /// The repair crew — see [`RepairState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<RepairState>,
    /// The Weapons→Helm arc-bearing seam — see [`ArcRequestState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_request: Option<ArcRequestState>,
    /// The reactor allocation — see [`PowerState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerState>,
    /// The ship's fly-through/orbit **pass surface** (`HelmPassSurface`), the
    /// derived helm-leg selection the motion planner steers from.
    ///
    /// This is the state issue #997's duel divergence actually hinged on. The
    /// surface is republished from scratch every AI tick by `ai_policy_state_tick`
    /// — but that system runs `.after(helm_motion_planner)`, so the planner reads
    /// the surface the *previous* tick left behind. A resumed ship whose surface
    /// came back at the bootstrap's value (its own fight, not the captured one)
    /// therefore has its first continuation planner tick select a different leg —
    /// inbound vs escape vs orbit — and steer onto a different bearing two ticks
    /// later, a steering-intent change the digest cannot see until helm
    /// integrates it into yaw.
    ///
    /// The one place a whole component travels rather than a scalar projection,
    /// and the exception is narrow: [`crate::ship::helm_ai::HelmPassSurface`] is a
    /// pure-scalar `Copy` struct with no enum among its fields, so the
    /// variant-order hazard the rest of this module writes scalars to avoid
    /// (see [`WeaponState`]) does not apply — serde's field-name-keyed form is
    /// stable, and a save from a build with a field this one lacks is refused by
    /// the content-version gate, not misread here. Storing it whole is what keeps
    /// the ~25 authored + derived fields impossible to drift out of sync one at a
    /// time. It is a derivation, but one read a tick before it is recomputed, so
    /// the capture-tick value has to survive the restore for the first planner
    /// tick to read — it cannot be rebuilt in time the way `WorldSnapshot` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_surface: Option<crate::ship::helm_ai::HelmPassSurface>,
    /// The entity's infrastructure condition track (issue #1025), whole.
    ///
    /// The second component stored whole rather than projected, and for a
    /// stronger reason than `pass_surface`'s: the track carries *which
    /// operational flags are currently down* alongside the number, and the two
    /// have to come back together. Restore the condition alone and the first
    /// tick after resume re-detects every crossing the mission already spent —
    /// re-firing `on_flag_cleared` on a skyhook that failed twenty minutes ago.
    /// Its `last_hull` is in for the same class of reason: a track that forgot
    /// it would book the structure's entire remaining hull as fresh damage on
    /// the tick after resume.
    ///
    /// The variant-order hazard does not apply — every field is a scalar, a
    /// `String`, or a `Vec` of those.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// It looks like it should: a save written without this field restores a
    /// degraded structure as intact, while the world flag store — which IS in
    /// the payload — comes back still holding the flags that structure dropped.
    /// That is exactly the "silently wrong, and indistinguishable from correct"
    /// shape a bump exists for.
    ///
    /// It cannot happen. The only saves that lack this field are saves of
    /// worlds written before it existed, and a world gains a structure by
    /// gaining an `[infrastructure]` table in its entity TOML — which moves
    /// `content_digest` and gets the older save refused as content-moved long
    /// before this field is read. A save that could reach the bad state is not
    /// loadable for an unrelated and stronger reason, so the format stays at 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infrastructure: Option<crate::infrastructure::InfrastructureState>,
    /// The ship's tractor-beam state (issue #1156) — whether the beam is engaged
    /// and what it is holding.
    ///
    /// A **projection**, not the whole [`crate::tractor::TractorBeam`] component,
    /// for `infrastructure`'s reason: the authored `[tractor]` coupling terms and
    /// the resolved power group ride the component too and are re-derived from the
    /// template on spawn, so writing them into a save would put content into the
    /// one artefact `content_digest` is answerable for. What travels is the pair
    /// the fold cannot recover — the engage state and the coupled target — which
    /// [`fold_tractor_namespace`](crate::sim_digest) folds and which a resume
    /// would otherwise drop, restoring a hulk as adrift when the crew had it
    /// under tow.
    ///
    /// The variant-order hazard does not apply: every field on
    /// `TractorSaveState` is a `bool`, a `String` or an `Option` of one.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// `scan`'s argument above, unchanged. A save written without this field
    /// restores a ship as holding nothing when the mission had it towing —
    /// silently wrong — but it cannot happen: a world gains a tractor by its hull
    /// TOML gaining a `[tractor]` table and a `kind = "tractor"` `[[system]]`,
    /// and `EntityConfig` sets `deny_unknown_fields`, so a build that predates
    /// this vocabulary refuses the template outright rather than loading it and
    /// writing a same-content-digest save to disagree with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tractor: Option<crate::tractor::TractorSaveState>,
    /// The ship's dock control state (issue #1159) — whether a dock is engaged,
    /// whether the two hulls are mated, and which berth is held.
    ///
    /// A **projection**, not the whole [`crate::dock::DockControl`] component, for
    /// `tractor`'s reason: the authored `[dock]` terms and the resolved power
    /// group ride the component and are re-derived from the template on spawn, so
    /// writing them into a save would put content into the one artefact
    /// `content_digest` is answerable for. What travels is the pair the fold
    /// cannot recover — the engage/dock state and the docked target — which
    /// [`fold_dock_namespace`](crate::sim_digest) folds and which a resume would
    /// otherwise drop, restoring two mated hulls as adrift.
    ///
    /// Did not bump [`SNAPSHOT_FORMAT`] for `tractor`'s reason: a world gains
    /// docking by its hull TOML gaining a `[dock]` table (and a `kind = "dock"`
    /// system), and `EntityConfig` sets `deny_unknown_fields`, so a build that
    /// predates this vocabulary refuses the template outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<crate::dock::DockSaveState>,
    /// The ship's external repair-dispatch state (issue #1161) — which ally or
    /// structure a repair team is working abroad, if any.
    ///
    /// A **projection**, not the whole
    /// [`crate::console::repair::external_server::ExternalRepairDispatch`]
    /// component, for `tractor`'s reason: the authored reach and rate ride the
    /// template and are re-derived on spawn. What travels is the one thing the
    /// fold cannot recover — the dispatched target — which
    /// [`fold_external_repair_namespace`](crate::sim_digest) folds and which a
    /// resume would otherwise drop, restoring a team as home when the crew had it
    /// out helping an ally (and, with it, one more free team than the hull really
    /// has). `deny_unknown_fields` on the repair table means a build predating
    /// `[repair.external_dispatch]` refuses the template outright rather than
    /// writing a same-content-digest save to disagree with, which is why this did
    /// not bump [`SNAPSHOT_FORMAT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_repair: Option<crate::console::repair::ExternalRepairSaveState>,
    /// The ship's transfer-umbilical state (issue #1160) — whether the flow is
    /// running.
    ///
    /// A **projection**, not the whole [`crate::umbilical::TransferUmbilical`]
    /// component, for `dock`'s reason: the authored `[umbilical]` terms and the
    /// resolved power group ride the component and are re-derived from the template
    /// on spawn. What travels is the one thing the fold cannot otherwise recover —
    /// the running intent, which [`fold_umbilical_namespace`](crate::sim_digest)
    /// folds and which a resume would otherwise drop, restoring a live resupply as
    /// stopped. The carry and the last refusal are not persisted: both are
    /// projections the next tick re-derives from the resumed world.
    ///
    /// Did not bump [`SNAPSHOT_FORMAT`] for `dock`'s reason: a world gains an
    /// umbilical by its hull TOML gaining an `[umbilical]` table (and a `kind =
    /// "umbilical"` system), and `EntityConfig` sets `deny_unknown_fields`, so a
    /// build that predates this vocabulary refuses the template outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umbilical: Option<crate::umbilical::UmbilicalSaveState>,
    /// The ship's scan record (issue #1032) — the last reading its sensor suite
    /// took, or why the last scan returned nothing.
    ///
    /// A **projection**, not the whole component, for `operations`' reason: the
    /// hull's authored `[scan]` fidelity ladder rides on the component too and
    /// is re-derived from the template on the tick the ship spawns.
    ///
    /// This is the one piece of scan state a fold cannot recover, and that is
    /// exactly why it is here. Everything else about a scan is re-derivable —
    /// the structure's condition, the ship's range, the grid's level — but a
    /// *reading* is what the crew saw when they looked, at the fidelity that
    /// moment bought them, and the structure has moved on since. #1031's
    /// evidence log is stored for the same reason and states it at more length.
    ///
    /// The variant-order hazard does not apply: `ScanRefusal` serialises by
    /// NAME (`#[serde(rename_all = "snake_case")]`), not by index, and every
    /// other field is a scalar, a `String`, or a `Vec` of those.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// #1025's argument, unchanged and for the same shape of field. A save
    /// written without it would restore a crew as never having scanned when the
    /// mission had them holding a reading — silently wrong, and
    /// indistinguishable from a crew who genuinely had not looked. It cannot
    /// happen: a world gains a scanning ship by its hull TOML gaining a `[scan]`
    /// table, which moves `content_digest` and gets the older save refused as
    /// content-moved long before this field is read.
    ///
    /// The contrast with the two bumps this sits between is worth stating,
    /// because both look similar and neither argument reaches here.
    ///
    /// #1031's evidence is written by a *script call*, so a world could gain
    /// findings with no template change at all and a format-6 payload of a run
    /// that had scanned was byte-indistinguishable from one that had not.
    /// #1035's workforce IS declared in the world file, and still had to bump,
    /// because [`RawWorld`](crate::world::config) sets no `deny_unknown_fields`
    /// — so an older build loads a world authoring `[[workforce]]`, drops the
    /// table on the floor, and writes a save with the SAME content digest.
    ///
    /// That second loophole is precisely the one this field does not have.
    /// `EntityConfig` **does** set `deny_unknown_fields`, so a build that
    /// predates this vocabulary does not quietly ignore a hull's `[scan]` table
    /// — it refuses to load the template at all, and never gets as far as
    /// writing a save to disagree with. A scan reading cannot exist without the
    /// table that produced it, and that table is content the loader is strict
    /// about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::science::ScanSaveState>,
    /// The entity's civilian traffic state (issue #1028), whole.
    ///
    /// Stored whole for the same reason the condition track is: the parts have
    /// to come back together. A civilian's lane, its leg, its standing order,
    /// where it stands with that order and the tick that stage is due on are one
    /// fact about a negotiation in progress. Restore the order without the
    /// compliance state and a craft that had already refused starts obeying;
    /// restore the compliance state without the due tick and a craft frozen
    /// mid-acknowledgement answers on the first tick after resume, or never.
    ///
    /// The variant-order hazard applies to exactly one field — `order` is an
    /// enum — and it is handled the way `RON` handles every other: by name, not
    /// by index. Adding a fourth order verb is additive; RENAMING one is not,
    /// and would need the same format bump any other renamed variant does.
    ///
    /// The **route assignment** is not here twice. `CivilianSection` is authored
    /// configuration, re-derived from the entity's TOML at spawn; what the state
    /// carries is the lane it is *currently* on, which a complied divert may
    /// have changed. That is why the section is not captured and this is.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// Exactly the [`EntityState::infrastructure`] argument, one feature later.
    /// A save that lacks this field is a save of a world written before it
    /// existed; a world gains civilian traffic by gaining a `[civilian]` table
    /// in an entity TOML and `[[route]]` blocks in its world TOML, both of which
    /// move `content_digest` and get the older save refused as content-moved
    /// long before this field is read. The format stays at 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civilian: Option<crate::civilian::CivilianState>,
    /// `ObjectiveCursors` as `(objective id, waypoint index, settled)`.
    ///
    /// Where a patrolling ship is *around its route*, which is not derivable
    /// from where it is in space: a route crosses itself, and the cursor is the
    /// only thing that says which leg the ship is on. A wave NPC restored at
    /// index 0 steers for the start of a lap it was halfway around — and
    /// `simulate_low_lod_ships` snaps its yaw straight at that waypoint, so the
    /// error is instant and total rather than a drift.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patrol_cursors: Vec<(String, u32, bool)>,
    /// `ShipSystemBlackboards`, sorted by system id — see the type note below.
    ///
    /// # Why a blackboard is authoritative and not a client projection
    ///
    /// AC5 excludes client projections, and a blackboard is *broadcast* to a
    /// console, which makes this look like the same thing. It is not, and the
    /// difference is which direction the arrow points: the console copy is a
    /// projection OF this map, and this map is the **frozen cross-system read
    /// surface** the ship's own AI decides from. `helm_shared_target_view`
    /// reads the Viewscreen blackboard's `combat_lock` and `science_target`
    /// deliberately through this freeze rather than off Tactical's live
    /// selection (issue #829), precisely so a cross-system read cannot reach
    /// another system's synchronous state. Restoring the wire copies to the
    /// browser is not this field's job; restoring what the helm reads next tick
    /// is.
    ///
    /// The measured consequence of leaving it out is written down in
    /// `tests/snapshot_resume.rs`: a resumed ship whose blackboards were the
    /// *bootstrap's* found its own target lock naming a ship its frozen view
    /// had never seen, resolved no travel target, cleared its recovery windows
    /// and fell out of `torpedo_run` into `acquire` on the first AI tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blackboards: Vec<(String, crate::core::messages::SystemBlackboard)>,
}

/// A ship's **weapon state machines**, named by this issue's AC2.
///
/// Every field here is a machine that is *mid-something* at the capture tick,
/// and a resumed ship whose machines came back cold is a different ship: a beam
/// that was two thirds through its burn stops burning, a bank that was on
/// cooldown is free to fire, a tube that was three seconds into a load is
/// empty, and a shield arc that had taken a broadside is whole again. Each of
/// those changes what happens on the *first* tick after a restore, which is
/// exactly the window a digest match at the instant of restore cannot see.
///
/// # Runtime only, never authored
///
/// The rule [`EntityState::hull`] states is kept here throughout: what is
/// stored is what the run *changed*, never what the TOML said. A tube's
/// `facing_deg`, `fire_arc_deg`, `volley_max`, `load_time`, barrel names and
/// firing pattern are all authored, the fresh world rebuilt them, and a save
/// that disagreed about them is a save the content-version gate refuses. Only
/// the load state, the counts and the last-fired markers travel.
///
/// # Written as scalars
///
/// For [`ControlState`]'s reason: a `derive` on `TubeLoadState` would silently
/// make its variant *order* stored surface, and a scalar written out at the
/// call site makes that commitment visible where it is made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeaponState {
    /// Every live phaser beam as `(bank, target uuid, remaining_secs,
    /// damage_accumulator)`, in the bank order `ActiveBeam` already keeps.
    ///
    /// `damage_accumulator` is in the tuple deliberately. It is the fractional
    /// damage carried between ticks so that 5 HP/s applies accurately at any
    /// frame rate; a beam restored without it is a beam whose sub-tick debt was
    /// forgiven, and `ActiveBeam::restore_live_banks` exists so that a restore
    /// cannot round-trip through `start` and lose it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beams: Vec<(String, String, f32, f32)>,
    /// `(bank, remaining_secs)` for every bank still cooling, in bank order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phaser_cooldowns: Vec<(String, f32)>,
    /// Every tube's contents and load machine, in the tube order the hull
    /// authored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tubes: Vec<TubeState>,
    /// The shared magazine. `None` when this ship has no torpedo system at all,
    /// which is how a hull with no tubes stays distinguishable from one that
    /// has shot itself dry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torpedoes_remaining: Option<u32>,
    /// Torpedoes in the air, which are authoritative in the plainest sense —
    /// they are moving, steering and about to detonate on somebody.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub torpedoes_in_flight: Vec<TorpedoInFlight>,
    /// Volleys part-way through their burst cadence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bursts: Vec<BurstState>,
    /// Per-arc shield hull as `(arc id, current, max)` in the arc order the
    /// TOML declared — `ShipArcHull`'s own iteration order, which it keeps
    /// separately from its map for exactly this determinism reason.
    ///
    /// This is the STRUCTURAL hull behind the emitters, not the shield CHARGE
    /// the arcs are currently holding — those are two different quantities and
    /// `shield_charge` below carries the other one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arc_hull: Vec<(String, f32, f32)>,
    /// Per-facing shield CHARGE as `(arc id, hp, hp_frac, offline_remaining,
    /// is_focused)`, in `ShieldSystem.facings` order — the fractional
    /// deterministic sim state a resume was silently dropping (issue #997
    /// follow-up: a restored world defaulted every arc back to full charge).
    ///
    /// `hp_frac` rides along for `beams`' `damage_accumulator` reason: it is the
    /// sub-tick regen/decay carried between fixed-timestep ticks, and a charge
    /// restored without it is a charge whose fractional debt was forgiven.
    /// `max_hp` is deliberately absent — it is not runtime charge but a
    /// focus-derived value `ShieldSystem::restore_facings` rebuilds from each
    /// arc's TOML baseline, so storing it would commit a derived quantity to
    /// the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shield_charge: Vec<(String, i32, f32, f32, bool)>,
    /// Every blaster bank's volley/cooldown cycle and bolts, in the authored
    /// bank order the component keeps — see [`BlasterRuntime`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blasters: Vec<BlasterRuntime>,
}

/// One torpedo tube's runtime state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TubeState {
    pub id: String,
    /// `TubeLoadState` as `0` = `Unloaded`, `1` = `Loading`, `2` = `Loaded`,
    /// `3` = `Unloading`. Anything else restores as `Unloaded` rather than
    /// panicking, for [`ControlState::impulse_phase`]'s reason.
    pub load_phase: u8,
    /// The `Loading`/`Unloading` timer, `(remaining, total)`. Zeroes for the
    /// two settled phases.
    pub load_timer: [f32; 2],
    pub loaded_count: u32,
    pub target_count: u32,
    /// Barrel indices the most recently launched round left from, and the
    /// 1-based pattern step it came from. Both are read to render the Tactical
    /// indicator and to pick the *next* barrel, so a tube restored without them
    /// resumes its firing pattern from the wrong place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_barrels: Vec<u32>,
    pub pattern_step: u32,
}

/// One torpedo mid-flight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TorpedoInFlight {
    pub uuid: String,
    pub position: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
    pub lifespan_remaining: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    pub tube_id: String,
    /// The firing tube's `shield_pierce` as it stood at launch. Carried by the
    /// round rather than re-resolved at detonation, so it has to be stored with
    /// it.
    pub shield_pierce: f32,
}

/// One tube's pending burst.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BurstState {
    pub tube_id: String,
    pub pending: u32,
    pub timer: f32,
    /// The launch origin and heading captured at fire time; every shot of the
    /// volley leaves from here, so it is state and not a derivation.
    pub launch: [f32; 3],
    pub launch_heading: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_origins: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_sequence: Vec<(u32, u32)>,
    pub next_shot_index: u32,
}

/// One blaster bank's runtime **volley + cooldown** cycle and its bolts in the
/// air (issue #997).
///
/// The blaster analogue of [`WeaponState::phaser_cooldowns`] and the tube/burst
/// state — and the piece whose absence issue #997's duel divergence turned out
/// to hinge on. The WEAPONS DOCTRINE (`tick_weapons_arc_request`) chooses which
/// family to ask Helm to bring an arc onto by walking the ship's families and
/// stopping at the first whose emitters are ONLINE and USABLE; a bank on
/// cooldown is not usable. A resumed destroyer whose blasters came back at the
/// authored default — ready, no cooldown — is usable when the live ship's are
/// still cooling, so its doctrine picks a different family, emits a different
/// arc-bearing request, and its Helm steers onto a different bearing. That is
/// the destroyer's yaw parting company two ticks after a restore whose digest
/// matched exactly.
///
/// Only the run-changed volley cycle and the bolts travel; the bank's authored
/// `config` (arc, cadence, damage, barrels, pattern) is rebuilt from TOML, the
/// rule [`WeaponState`] keeps throughout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlasterRuntime {
    /// Steps left to fire in the active volley (`0` when idle).
    pub pending_volley: u32,
    /// The resolved active firing schedule — `(barrel indices, at_secs)` steps.
    /// Stored rather than re-derived because the cursor below indexes it, and a
    /// re-derivation could disagree with a config that changed between builds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<(Vec<u32>, f32)>,
    pub next_step: u32,
    pub volley_elapsed: f32,
    /// Barrels that fired on the most recent step — read for the Tactical
    /// indicator and to pick the next barrel, so a bank restored without them
    /// resumes its pattern from the wrong place, as a tube does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_barrels: Vec<u32>,
    pub current_step: u32,
    pub on_cooldown: bool,
    pub cooldown_remaining: f32,
    pub charging: bool,
    pub charge_elapsed: f32,
    /// Bolts this bank has in flight — moving, and about to hit somebody, so
    /// authoritative in the plainest sense (as torpedoes in flight are).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_flight: Vec<BlasterBolt>,
}

/// One blaster bolt mid-flight — see [`BlasterRuntime::in_flight`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlasterBolt {
    pub id: String,
    pub x: f32,
    pub z: f32,
    pub heading: f32,
    pub speed: f32,
    pub lifespan_remaining: f32,
    pub collision_radius: f32,
    pub damage: i32,
    pub shield_pierce: f32,
    pub source_uuid: String,
}

/// A ship's **repair state**, the other half of AC2's "weapon/repair state".
///
/// A repair team is a timer with a destination. Restored idle, every team that
/// was three seconds into a five-second walk arrives late — or never, because
/// the dispatch that sent it is not re-issued — and the systems they were
/// mending stop mending. The queue and the alert latch travel with them for the
/// same reason: a queue restored empty re-raises requests the capture had
/// already spent, and `RepairHumanAlerted` is precisely the latch that stops a
/// crew being told twice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairState {
    /// One `TeamSlot` per team, in slot order.
    ///
    /// The single place this payload stores a *type* rather than scalars, and
    /// the exception is narrow on purpose: `TeamSlot` is already a wire type
    /// with derived serde that the client renders from, so its variant order is
    /// stored surface the repository had committed to before this module
    /// existed. Copying it into scalars here would not remove that commitment,
    /// only duplicate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamSlot>,
    /// Pending requests as `(station id, label, tier, deficit)`, in the
    /// severity order the queue keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<(String, String, crate::ship::damage::DamageTier, f32)>,
    /// The "this crew has already been told" latch, as `(system id, tier)`
    /// sorted by id — the component is a `HashMap`, and a payload may not
    /// inherit its iteration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerted: Vec<(String, crate::ship::damage::DamageTier)>,
}

/// The Weapons→Helm **arc-bearing seam** (issues #677/#767), both halves of it.
///
/// This is the divergence issue #997 was opened for. The seam is a debounce on
/// the Weapons side and a pending bearing on the Helm side, and the two are one
/// piece of state: `tick_weapons_arc_request` only emits a channel-3
/// [`CoordinationPayload::ArcBearingRequest`] when its debounce key *changes*,
/// and the request it emits lands in Helm's [`PendingArcBearingRequest`], which
/// `ai_helm_steering` folds into the steering bias every tick until the geometry
/// self-clears.
///
/// A ship captured mid-engagement holds a *settled* debounce (`last = Some(key)`,
/// already emitted) and a *pending* bearing Helm is still folding in. A resumed
/// ship that booted both `default()` has `last = None` — so the first
/// `ai_tick_ready` cadence tick after the restore re-fires a request the live
/// world suppresses — and an empty pending bearing, so its helm steers without
/// the bias the live ship still carries. Either alone changes steering intent,
/// which the digest cannot see (it folds `ShipPhysics`, not steering input) until
/// helm integrates it into yaw ~2 ticks later. That is exactly the lagged, opt-
/// level-independent divergence #997 measured.
///
/// # Stored as its wire types, not scalars
///
/// [`WeaponFamily`] and [`WeaponEmitterArc`] already derive serde — they are the
/// channel-3 payload types, so their shape was stored surface the repository had
/// committed to before this module existed. Copying them into scalars here would
/// not remove that commitment, only duplicate it — the [`RepairState::teams`]
/// exception, for the same reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArcRequestState {
    /// `WeaponsArcRequestState.last` — the settled debounce key
    /// `(family, target uuid, usable arcs)`. `Some` means a request for this key
    /// has already been emitted and must *not* re-fire on the first cadence tick
    /// after the restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<(
        crate::core::messages::WeaponFamily,
        String,
        Vec<crate::core::messages::WeaponEmitterArc>,
    )>,
    /// `PendingArcBearingRequest.target` — the uuid Helm is biasing to bring a
    /// weapon arc onto, or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_target: Option<String>,
    /// `PendingArcBearingRequest.arcs` — the emitting family's usable ONLINE
    /// emitter arcs Helm folds into its steering bias and self-clears against.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_arcs: Vec<crate::core::messages::WeaponEmitterArc>,
}

/// A ship's **reactor allocation** — the per-group power levels, the battery
/// reserve, and the exhaustion lock (issue #997).
///
/// This is the state issue #997's duel divergence was actually about. The
/// `PhaserDamage`, `MaxSpeed`, `MaxYawRate` and `ShieldRegen` modifiers are
/// recomputed *every tick* from the power levels (see
/// `modifiers::coordination::apply_power_modifiers`), so a resumed ship whose
/// reactor came back at the seeded default — every group at level 2 — burns its
/// beams, drives its engines and regenerates its shields at a different
/// intensity than the live ship from the first tick after the restore. The duel
/// cruiser held WEAPONS at level 3 (a `PhaserDamage` of 1.25); the resumed one
/// reverted to level 2 (1.0), so its beam accumulated a fifth less damage per
/// tick, applied one fewer whole point two ticks later, and the hull the digest
/// folds parted company — exactly the lagged, opt-level-independent divergence
/// #997 measured.
///
/// The battery reserve and the lock travel too: the reserve because
/// `PowerSystem::tick` integrates the modifiers off the *current* charge and
/// browns the reactor out at zero, and the lock because a reactor restored
/// unlocked would let its allocation controls move on a tick the live ship had
/// them frozen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerState {
    /// `(power group id, level)` in the reactor's own insertion order — the
    /// order `PowerSystem` walks for deterministic wire output, preserved so the
    /// restore rebuilds the same order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allocations: Vec<(String, u8)>,
    pub battery_charge: f32,
    /// The exhaustion lock — see [`PowerState`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

/// A ship's **active control state**: what its helm was being told to do.
///
/// Named by this issue's acceptance criteria, and not optional in practice even
/// though `world_digest` does not fold it. `ShipPhysics` records where a ship
/// *is* and how fast it is going; these six axes record what it is being asked
/// to do next, and `integrate_ship_physics` reads them on the very first step
/// after a restore. A resumed ship without them keeps its captured velocity and
/// then immediately coasts, which reads as a divergence one frame after a
/// restore that was otherwise exact — the first thing this slice's continuation
/// test caught.
///
/// Stored as plain scalars rather than by giving `ImpulsePhase`, `ImpulseState`
/// and `BoostState` serde derives. The reason is the type-shape constraint
/// `sim_digest` documents: a `derive` on an enum silently makes its variant
/// *order* stored surface, and a scalar written out at the call site makes that
/// commitment visible where it is made. Three fewer types become save format.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub thrust: f32,
    pub steering: f32,
    pub lateral: f32,
    pub vertical: f32,
    pub boost: bool,
    /// `ImpulsePhase` as `0` = `Idle`, `1` = `Charging`, `2` = `Active`.
    /// Anything else restores as `Idle` rather than panicking — an unknown
    /// phase is a save from a build that had one this one does not, and the
    /// content/format gate is what refuses that, not a `match` arm here.
    pub impulse_phase: u8,
    /// `LastHelmInput`'s `(thrust, steering, lateral)`. Distinct from the three
    /// axes above: those are the *desired* input, this is what the integrator
    /// last actually applied, and the helm AI's rate limiting reads the
    /// difference.
    pub last_helm: [f32; 3],
    /// `TacticalRadarSelection` — the uuid this ship's Tactical radar is locked
    /// on, or `None`.
    ///
    /// Targeting is radar-owned, and the lock is what every downstream decision
    /// hangs off: a restored ship without it has no target, so its helm AI
    /// steers nowhere and its weapons hold fire. That is a whole ship behaving
    /// differently, one tick after a restore whose digest matched exactly —
    /// which is precisely the class of silent gap this payload exists to close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_lock: Option<String>,
    /// `LastShipAttacker` — who last shot this ship, the AI's fallback target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attacker: Option<String>,
    /// `SensorRadarSelection` — the uuid this ship's Sensors radar has locked as
    /// its **Science Target**, or `None`. The sibling of [`Self::target_lock`]
    /// (which is the Tactical radar's Combat Lock): both are per-ship radar
    /// selections a run *chose*, and both feed the ship's own AI through the
    /// frozen viewscreen read surface. `PublishAggregate` lifts this into
    /// `ViewscreenBlackboard::science_target` (issue #829), which
    /// `helm_shared_target_view` and the weapons doctrine both decide from — so a
    /// resumed ship whose Sensors radar came back empty resolves a different
    /// shared target on its first cadence tick and steers differently, the same
    /// silent one-tick divergence `target_lock` was captured to close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_lock: Option<String>,
    /// The three stateful helm policies' runtime state, in the fixed order
    /// `(engines, steering, boost)` — see [`PolicyState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_policies: Option<[PolicyState; 3]>,
    /// `HelmRecoveryHistory` — see [`RecoveryHistory`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_recovery: Option<RecoveryHistory>,
}

/// The host-side bounded range windows a ship's helm policies read through
/// `fact(safe_distance_held)` and the pressed detector.
///
/// `HelmPolicyRuntime`'s own docs call its five components "one thing", and the
/// payload had three of them. The two it did not have are not alike, and only
/// one of them is state: `HelmPassSurface` is republished from scratch by
/// `ai_policy_state_tick` every AI tick, so a restored ship rebuilds it on its
/// first tick and storing it would store a derivation. These windows are the
/// opposite — they are an *accumulation* over the last N shared AI ticks, and
/// there is no tick on which they are recomputed from the world. A ship
/// restored without them has held its safe distance for zero samples, which is
/// a different answer to a question its transitions are gated on.
///
/// The capacities are authored (`safe_distance_window_ticks`,
/// `pressed_window_ticks`) and re-applied every tick from config, so they are
/// stored only so the samples can be replayed into a window of the right size
/// before the first tick re-authors it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecoveryHistory {
    /// The uuid both windows were measured against. A target switch clears
    /// them, so restoring the samples without the identity they belong to
    /// would credit a new threat with distance held against the old one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The level window (`safe_distance_held`), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<f64>,
    pub ranges_capacity: u32,
    /// The trend window (the pressed detector), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub separation: Vec<f64>,
    pub separation_capacity: u32,
}

/// One stateful AI policy's runtime state (issue #882's `AiPolicyRuntimeState`).
///
/// Captured because a cold policy runtime is a ship that behaves differently:
/// the state id and `entered_at_secs` are what `state_time` is measured
/// against, so a restored ship whose policy was reset evaluates every
/// time-gated transition from zero and takes a different branch on the first
/// tick after the restore. That is the second silent gap this slice's
/// continuation test caught, after the helm axes.
///
/// Written out field-by-field rather than by deriving serde on
/// `AiPolicyRuntimeState` itself, for the reason [`ControlState`] gives:
/// `AiPolicyMemory` already carries serde (added for this payload), and the two
/// scalars beside it do not need a third type's shape pinned as save format to
/// travel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyState {
    /// The currently-entered state id.
    pub current: String,
    /// The tick-derived clock reading `current` was entered at.
    pub entered_at_secs: f64,
    /// The fine system's typed private memory.
    pub memory: crate::world::flags::AiPolicyMemory,
}

/// One asteroid's authoritative state.
///
/// A rock's position is authoritative because it is what a collision resolves
/// against — the digest's own reason for folding it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidState {
    pub uuid: String,
    pub translation: [f32; 3],
    /// The rock's orientation, `[x, y, z, w]`. Not folded by the digest — a
    /// tumbling rock collides the same either way — but stored because a
    /// restore now *spawns* rocks, and a spawned rock with a default rotation
    /// is a visibly different rock from the one that was saved.
    #[serde(default)]
    pub rotation: [f32; 4],
    /// `(SystemId, current, max)`, as [`EntityState::hull`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    /// The rock's entity TOML path, and the reason it is here is [`restore`]'s
    /// spawn path: to *build* a missing rock the restore needs its collider,
    /// mesh, tags and radar appearance, and all four are read from this file
    /// rather than stored. It is joined from the streaming window's slot data,
    /// which is the only place a rock's config path survives after spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    /// The rock's `AsteroidShieldPierce`, which is per-*field* tuning resolved
    /// at spawn from whichever contribution the composed evaluator picked. That
    /// makes it neither authored-per-rock nor recomputable without re-running
    /// the evaluator, so it travels with the rock.
    #[serde(default)]
    pub shield_pierce: f32,
}

/// The asteroid streamer's own progress, and the reason AC1's world needs it.
///
/// Combat Test's belts are *streamed*: a rock exists when the player's cell
/// window covers it, so a fresh app bootstrapped at the spawn point has a
/// different rock population than a capture taken after the player has flown
/// somewhere. Restoring the rocks without restoring the window that owns them
/// closes half the gap and opens a worse one — the streamer would still believe
/// it was anchored where the fresh boot left it, and the very next
/// `update_asteroid_window` tick would full-rebuild the belt out from under the
/// restore.
///
/// With the anchor, the player cell and the composition key all put back, that
/// tick recomputes the same cell from the restored ship's position, finds it
/// unchanged, and returns without touching anything. The streamer resumes
/// rather than restarting.
///
/// Cosmetic slots are **not** here: they hold raw `Entity` handles, which this
/// payload never stores (see the module docs), and they are set dressing with
/// no uuid, no hull and no collider. [`restore`] clears them so the streamer
/// repopulates them on its next scroll.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidWindowState {
    pub arena_gx: i32,
    pub arena_gz: i32,
    pub despawn_cells: u32,
    pub spawn_cells: u32,
    pub resolution: f32,
    /// The player's lattice cell as of the last streamer tick. `None` before
    /// the first one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_grid: Option<(i32, i32)>,
    /// The fingerprint of the contribution set the window's contents were built
    /// from. Restored so the streamer does not read its own live fields as a
    /// composition change and rebuild.
    pub composition_key: u64,
    pub needs_init: bool,
    /// The occupied slots, sorted by `(z, x)` so the payload is byte-stable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<WindowSlot>,
}

/// One occupied ring-buffer slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowSlot {
    pub x: u32,
    pub z: u32,
    pub uuid: String,
    pub config_path: String,
    pub hp: i32,
    pub max_hp: i32,
    pub y: f32,
}

/// One collision the run applied, in the shape the digest attributes it.
///
/// Only collisions are stored, not the whole `RunTelemetry` stream: the rest of
/// that resource is a *report* artifact (message counts, ndjson lines, name
/// tables), and a report is something a resumed run rebuilds rather than
/// something it inherits. Collisions are here because `fold_collisions` puts
/// them in the authoritative fold — #896's finding that contact attribution is
/// the part of physics a divergence shows up in first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionRecord {
    pub tick: u64,
    pub sim_t: f64,
    pub victim: String,
    pub victim_is_asteroid: bool,
    pub amount: f32,
    pub shield_absorbed: f32,
    pub hull_damage: f32,
}

/// One world layer's flag store, keyed by the layer's TOML path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerFlags {
    pub path: String,
    pub flags: FlagStore,
}

/// The **scenario-progression** state a scripted world resumes from (issue
/// #864).
///
/// `flags` above is the half of a scenario's memory that already travelled: the
/// counters a `when` predicate reads. This is the other half — *what has
/// already happened*, and *what is still owed*. A resumed scenario without it
/// stands at a matching digest and then replays its own opening: every
/// single-shot trigger the run had spent is re-armed and fires a second time,
/// every `after(n, |ctx| …)` callback the run was waiting on has been forgotten,
/// and the mission clock those firings are measured against has been rewound to
/// the age of whatever fresh app was booted to restore into.
///
/// That last one is the piece the rest hangs off, and it is stored **relative**
/// rather than absolute for the reason [`AsteroidWindowState`] stores the
/// streamer's anchor: `mission_clock_anchor_secs` is a reading of
/// `Time<Fixed>::elapsed_secs()` taken in *this process*, and a fresh app's
/// clock started at a different moment. What is authoritative is not the anchor
/// but the distance from it — the mission-elapsed seconds every `on_timer`
/// threshold and every `action_delays` fire time is authored against — so the
/// capture stores the distance and the restore re-derives an anchor that
/// reproduces it against the resumed app's own clock.
///
/// # Honestly not covered
///
/// `pending_delayed_actions` (the declarative `action_delays` queue, which a
/// scripted `ctx.schedule.in_seconds(n).<verb>(…)` also feeds) is **not** here.
/// Carrying it means giving [`crate::world::config::TriggerAction`] — 22
/// variants over `AiDirective`, `UtilityConfig`, `ObjectiveSource`,
/// `ModifierSlot`, `IntModifierSlot` and `crate::core::balance::Outcome` — a serde
/// derive, which would pin six *authored-config* types' shape as save format.
/// This module refuses that commitment everywhere else it comes up (see
/// [`PhoenixSnapshot::game_over`], which stores `Outcome` as a label precisely
/// so its variant order does not become stored surface), and no shipped world
/// authors `action_delays` or `in_seconds` today — the deferral vocabulary the
/// shipped scripted set actually uses is `after(..)`, which is
/// [`Self::script_callbacks`]. Widening to the action queue is a separate
/// issue's commitment, not a line to slip in here.
///
/// A layer's own trigger state is not stored separately either, for a narrower
/// reason: a layer's states are *merged into* the base `trigger_states` vec when
/// its `[script]` set compiles at load (issue #1045) and removed from it at
/// unload, so [`Self::triggers`] already carries every trigger that can fire.
/// What identifies them is the `origin_layer` tag on each state — matched by
/// `world::server::remove_layer_script_triggers` — and a resumed world rebuilds
/// them by re-running the same layer loads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioState {
    /// Mission-elapsed seconds at the capture — see the type docs for why this
    /// is a distance and not the anchor itself. `None` when the mission clock
    /// was not yet anchored (no world, or a run that had not started).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_elapsed_secs: Option<f32>,
    /// One row per live trigger state, in `WorldContentRuntime::trigger_states`
    /// order — see [`TriggerRuntimeState`]. Every row is written, not just the
    /// fired ones, so the count is itself the alignment check the restore makes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerRuntimeState>,
    /// `(group, member names)` sorted by group, members sorted — the membership
    /// an `on_all_destroyed group = "…"` condition is judged against. A
    /// `HashMap` of `HashSet`s in the runtime; a payload may not inherit either
    /// iteration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_groups: Vec<(String, Vec<String>)>,
    /// `(entity name, uuid)` sorted by name: `WorldContentRuntime::name_to_uuid`,
    /// the map every name-carrying command in `world::dispatch` resolves through
    /// (issue #863).
    ///
    /// It belongs beside `entity_groups` for the reason that field is here — both
    /// are what a scenario knows about *which* entity it means — and it was left
    /// out until now for a reason that stops being true the moment a restore can
    /// spawn: a fresh boot rebuilt the map by re-running the same spawns, and the
    /// mint being deterministic meant it rebuilt it with the same uuids. A
    /// restore-spawned entity has no such re-run behind it, so the name it
    /// answers to has to travel with it or the resumed scenario cannot destroy
    /// it, cannot target it, and cannot notice it dying.
    ///
    /// Sorted, because a `HashMap`'s iteration order is not a payload's to
    /// inherit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_to_uuid: Vec<(String, String)>,
    /// `(uuid, aggregate hull fraction)` sorted by uuid: the last sample
    /// `collect_world_events` compares against to decide whether a hull crossed
    /// *downward*.
    ///
    /// Restored because leaving it out manufactures an event. A fresh app's
    /// ships are whole, so its last sample is ~1.0; the restore then writes the
    /// capture's mauled hull underneath it, and the very next tick reads that
    /// as a fresh downward crossing and fires every `on_hull_below` template the
    /// captured run had already spent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_hull_fractions: Vec<(String, f32)>,
    /// `WorldContentRuntime::pending_world_events`, in queue order — see
    /// [`WorldEventRecord`]. Usually empty at a tick boundary; non-empty exactly
    /// when the previous tick's delayed actions or script callbacks queued a
    /// chaining event for the next one, which is the tick a capture can land on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_events: Vec<WorldEventRecord>,
    /// `WorldScriptRuntime::pending_callbacks`, **in queue order**.
    ///
    /// Not sorted, and that is the deliberate reading of this module's
    /// stable-key rule rather than an exception to it. The rule exists because a
    /// payload must not inherit a `HashMap`'s iteration order; this queue is a
    /// `Vec` whose order is the order the scripts scheduled into it, which is
    /// already a deterministic function of the run every peer reproduces — and
    /// it is *load-bearing*, because `PendingCallbacks::drain_due` fires due
    /// callbacks in exactly this order and their effects apply in that order.
    /// Sorting the payload would hand the resumed world a different firing
    /// order than the live one it is being compared against. Each entry is a
    /// [`ScheduledCall`] — `(fire_tick, script_path, fn_name)` — which is
    /// already the stable key the rule asks for: no AST, no closure, no handle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_callbacks: Vec<ScheduledCall>,
    /// `WorldContentRuntime::deadlines`, whole (issue #1024).
    ///
    /// The named half of the queue above. [`Self::script_callbacks`] already
    /// carries the `ScheduledCall` a pending deadline is waiting on — but the key
    /// is `(fire_tick, script_path, fn_name)` and nothing in it says *which*
    /// deadline, whether the crew can see it, or whether the two that are NOT in
    /// the queue were cancelled or already fired. Restoring the queue without the
    /// table therefore resumes a mission whose deadlines have no names and no
    /// history: a cancelled `stabiliser_failure` comes back armed at its authored
    /// time, and a window the crew bought two minutes on comes back due when it
    /// originally was.
    ///
    /// Stored as the whole [`DeadlineTable`] rather than a per-field projection,
    /// which is [`EntityState::pass_surface`]'s exception, taken for its reason:
    /// every field is a scalar, a `String`, or the already-stored
    /// [`ScheduledCall`], with one small enum
    /// ([`DeadlineState`](crate::world::deadlines::DeadlineState)) that travels by
    /// its own `snake_case` serde name rather than by variant order. There is
    /// nothing here whose shape a save would pin that this payload does not pin
    /// already — and writing it whole is what keeps the table impossible to drift
    /// out of sync with the queue one field at a time.
    ///
    /// In authored order, never sorted, for [`Self::script_callbacks`]' reason:
    /// the order is the world file's, it is what the panel renders, and it is
    /// already deterministic across peers.
    #[serde(default, skip_serializing_if = "DeadlineTable::is_empty")]
    pub deadlines: DeadlineTable,
    /// `WorldContentRuntime::commitments`, whole (issue #1029).
    ///
    /// The promises the run made, with who each was made to, its terms, its
    /// stated resolution condition, whether it ended up kept or broken, and the
    /// ticks at both ends.
    ///
    /// Unlike every other field on this struct, **nothing in the world file
    /// predicts whether this is empty**: a promise exists because of what the
    /// player said, not because of what an author declared. That is why the
    /// content digest cannot stand in for the format bump here — see
    /// [`SNAPSHOT_FORMAT`] — and it is also why the whole ledger is stored
    /// rather than a projection of the open ones: a resumed run that could not
    /// tell a kept promise from one never made would let the crew give the same
    /// word twice.
    ///
    /// In the order the promises were made, never sorted, for
    /// [`Self::deadlines`]' reason.
    #[serde(default, skip_serializing_if = "CommitmentLedger::is_empty")]
    pub commitments: CommitmentLedger,
    /// `WorldContentRuntime::evidence`, whole (issue #1031).
    ///
    /// What the crew found out, what each finding was about, how they learned it
    /// and on which tick.
    ///
    /// Stored for [`Self::commitments`]' reason — nothing in the world file
    /// predicts whether it is empty, because a finding exists because of what the
    /// crew *did* — and stored WHOLE for [`Self::deadlines`]' reason: every field
    /// is a `String`, a `u64`, or one small enum travelling by its own
    /// `snake_case` serde name.
    ///
    /// This is the only dossier state a save carries, and the distinction is the
    /// whole of `src/dossier/`: the facts on a sheet are re-folded from the
    /// condition track, the ledger and the roster every tick and would disagree
    /// with them if persisted, whereas what the crew learned is recoverable from
    /// nothing at all.
    ///
    /// In the order the findings were made, never sorted, for
    /// [`Self::deadlines`]' reason — and here the order is also what the fact
    /// sheet renders.
    #[serde(default, skip_serializing_if = "EvidenceLog::is_empty")]
    pub evidence: EvidenceLog,
    /// `WorldContentRuntime::workforce`, whole (issue #1035).
    ///
    /// Which sides of the world's labour dispute are out right now, what each
    /// makes of the crew, and whether the register has been armed at all.
    ///
    /// The `armed` latch is the field that makes this necessary rather than
    /// merely tidy. Without it a resumed mission's first tick re-arms from the
    /// world file and puts a settled strike straight back on — the loudest
    /// possible way to lose a negotiation the crew already won, and the same
    /// class of silent re-arming that carried [`Self::deadlines`] into the
    /// payload. The two live facts have to come back with it: restore the latch
    /// without the records and the register is armed and empty, so every
    /// structure reads as worked.
    ///
    /// Stored as the whole register rather than a per-side projection, for
    /// [`Self::deadlines`]' reason: every field is a scalar, a `String` or a
    /// `bool`, there is no enum among them, and writing it whole is what keeps
    /// three facts about one side impossible to drift apart one at a time.
    ///
    /// In authored order, never sorted — the world file's order, which every
    /// peer reads the same way.
    #[serde(
        default,
        skip_serializing_if = "crate::world::workforce::WorkforceRegister::is_empty"
    )]
    pub workforce: crate::world::workforce::WorkforceRegister,
}

/// One scenario trigger's runtime state — the three fields a run *changes*.
///
/// The trigger itself (condition, actions, `when` predicate, `repeat`,
/// `cooldown_secs`, authored id) is not here, for [`EntityState::hull`]'s rule:
/// it is authored config the fresh world rebuilds from TOML — or, for a scripted
/// trigger, from the `[script]` block the content digest is bound to.
///
/// # Scripted and declarative triggers are the same row
///
/// `merge_script_triggers` appends one `TriggerState` per compiled
/// `ScriptTrigger` to the SAME `WorldContentRuntime::trigger_states` vec the
/// declarative triggers live in — a scripted trigger's `.trigger` is
/// byte-identical to its TOML equivalent, and only the parallel
/// `WorldScriptRuntime::handlers` entry says where its effects come from. So
/// there is one fired-state list to capture, not two, and this row covers both
/// kinds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TriggerRuntimeState {
    /// Position in `trigger_states`. See [`restore`]'s scenario walk for why an
    /// index is a stable key *here* while it would not be for an ECS entity: the
    /// table is rebuilt by a deterministic replay of the same load, and a table
    /// of a different length is refused rather than written into.
    pub index: u32,
    /// The single-shot latch. The whole point of the row.
    pub fired: bool,
    /// The `OnAllDestroyed` accumulation, sorted — a `HashSet` in the runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen_destroyed: Vec<String>,
    /// Mission-elapsed seconds of the last fire, which is what a `repeat`
    /// trigger's `cooldown_secs` is measured from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_elapsed: Option<f32>,
}

/// One queued `WorldEvent`, written as a tag plus its fields.
///
/// Written out rather than by deriving serde on
/// [`crate::world::content::WorldEvent`], for [`ControlState`]'s reason: a
/// derive would make that enum's shape stored surface, and a tag written at the
/// call site makes the commitment visible where it is made. An unrecognised
/// `kind` is dropped on restore rather than panicking — the same rule
/// [`ControlState::impulse_phase`] states.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldEventRecord {
    /// `0` Destroyed, `1` Attacked, `2` HullDroppedBelow, `3` TimerElapsed,
    /// `4` Hailed, `5` FlagSet, `6` FlagCleared, `7` WorldLoaded,
    /// `8` EnteredRegion, `9` ExitedRegion, `10` WaypointReached.
    pub kind: u8,
    /// The event's primary name: an entity uuid, a region uuid, or a flag name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    /// The event's secondary name: an attacker uuid or a waypoint anchor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub other: String,
    /// `FlagSet`/`FlagCleared`'s owning layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    /// `HullDroppedBelow`'s `(previous, current)` fractions, or
    /// `TimerElapsed`'s elapsed seconds in `[0]`.
    #[serde(default)]
    pub numbers: [f32; 2],
}

/// The **comms** state a mid-conversation save resumes from (issue #984, S8).
///
/// [`ScenarioState`] is what the scenario has done and is still owed. This is
/// the conversation it is in the middle of *having*: the inbox the Comms
/// officer is looking at, the dialogue entries that make those messages
/// answerable, and the scripted `open_comms` requests that have been queued but
/// not yet materialised into threads.
///
/// Nothing comms-shaped was in this payload before that issue - not the inbox,
/// not the dialogues - so a save taken mid-thread came back to a world with an
/// empty Comms console and a scenario waiting for an answer that could no longer
/// be given. That is the whole gap.
///
/// # Every dialogue reduces losslessly, and the reason is structural
///
/// [`ActiveDialogue::current_node`] is a [`CommsDialogueNode`], and while
/// `[[comms]]` existed a *declarative* one's responses could carry
/// `Vec<TriggerAction>` and a nested `follow_up` tree. Serialising those is the
/// commitment [`ScenarioState`] refuses for `pending_delayed_actions`, so such a
/// node was left out and the loss reported. Issue #985 deleted the front-end
/// that produced them: a node is now built only by `project_node`, whose
/// responses are `(text, important)` and nothing else, so [`DialogueState`] is a
/// faithful copy of every node there can be and the refusal path is gone with
/// the shape that needed it.
///
/// # Honestly not covered
///
/// `contacts`, `range_flags` and `range_active` are **derived**, not
/// progression: the hail roster is rebuilt every tick from the live entities
/// that carry `[comms] hailable = true` (`update_comms_range_flags`), which also
/// recomputes the range map from ship and entity transforms. A resumed world
/// derives all three from state this payload *does* restore. `needs_broadcast`
/// is set true by the restore rather than carried, because after a restore it is
/// unconditionally true.
///
/// `OnScreenMessage` - which message the Comms officer put on the viewscreen -
/// is a presentation choice, not scenario progression, and falls under this
/// module's standing exclusion of client projections. Nothing folds it and it is
/// re-established by the next `ShowOnScreen`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommsState {
    /// `CommsInboxRes`, **in inbox order** - which is injection order, the order
    /// `CommsInbox::messages()` projects onto the wire and the order
    /// `operate_comms_response_ai` decides in. Not sorted, for
    /// [`ScenarioState::script_callbacks`]' reason: it is a `Vec` whose order is
    /// already a deterministic function of the run, and it is read in that
    /// order by an actor that emits commands from it.
    ///
    /// [`crate::core::messages::CommsMessage`] is the wire type and was already
    /// `Serialize`/`Deserialize` - this stores it verbatim rather than
    /// projecting it, because every field on it (`is_read`, `selected_response`,
    /// `is_urgent`, `thread_id`, `sender_uuid`) is authoritative state the
    /// response handler reads back out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbox: Vec<CommsMessage>,
    /// Every `CommsRuntime::active_dialogues` entry, sorted by message id - the
    /// standing rule for anything that is a `HashMap` in the runtime, and the
    /// message id is the map's own key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogues: Vec<DialogueState>,
    /// `CommsRuntime::open_hails`, already ordered (a `BTreeSet`). The record of
    /// which targets this ship has hailed and not cleared; without it a resumed
    /// Backfill comms officer re-hails a contact it had already hailed, which
    /// seats a duplicate thread in the restored inbox.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_hails: Vec<String>,
    /// `WorldScriptRuntime::pending_comms_opens`, **in queue order**.
    ///
    /// Not sorted, and the justification is [`ScenarioState::script_callbacks`]'
    /// verbatim, one step stronger: `open_scripted_comms_threads` drains this
    /// `Vec` front-to-back and mints a message id per request from the
    /// tick-scoped [`WorldIdMint`], so a reordered queue does not merely apply
    /// effects in a different order - it hands different threads different ids,
    /// which `world_digest` folds through the mint's per-namespace counters. The
    /// order is already deterministic (it is the order the scripts pushed), so
    /// there is nothing to normalise away.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_opens: Vec<OpenCommsRequest>,
}

/// One live dialogue - see [`CommsState`] for why every one of them travels.
///
/// The node is stored as body and per-response `(text, important)` rather than
/// as a [`CommsDialogueNode`]. Since issue #985 that IS the whole of a node, so
/// this is a faithful copy and not a lossy shortcut, and nothing here pins a
/// runtime type's shape as stored surface.
// No `Default`: a dialogue row is always built whole from a live
// `ActiveDialogue`, and there is no meaningful empty one - a `message_id` of
// `""` addresses nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogueState {
    /// The `CommsMessage::id` this dialogue answers - `active_dialogues`' key,
    /// and what a `RespondToMessage` addresses.
    pub message_id: String,
    /// The thread every message in this conversation shares.
    pub thread_id: String,
    /// The shown node's body text, as a `strings.csv` id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The runtime values interpolated into `body`'s `{placeholder}` tokens.
    ///
    /// Captured rather than re-derived, because there is nothing to re-derive
    /// from: the figures were computed by the script at the moment the node was
    /// entered, off state that has since moved on. A resumed save that dropped
    /// them would re-render the node with its placeholders bare.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub body_params: std::collections::BTreeMap<String, String>,
    /// `(text, important)` per shown response, in the order the player sees them
    /// - the index a `RespondToMessage` submits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responses: Vec<(String, bool)>,
    /// The Rhai tree that answers this dialogue.
    ///
    /// It was `Option` while declarative threads existed; issue #985 made it
    /// required, because [`ActiveDialogue::script`] is no longer optional either.
    ///
    /// [`ScriptedDialogue`] is strings only (`script_path`, `node_fn`, the
    /// parallel `on_pick` names) and already derives serde for this. Those names
    /// resolve against the **recompiled** script set, and what makes that safe is
    /// issue #864's content binding rather than anything here:
    /// the load returns `CompiledScripts::content_hash` as a ledger record and
    /// its caller applies it (issue #1241), `content_digest` folds it, and
    /// `Versions::check` refuses
    /// a save whose scripts moved - so a restored `node_fn`/`on_pick` pair is
    /// always read against the identical compiled units it was captured from.
    /// The runtime backstop is still there underneath
    /// (`EnterError::Unresolved` refuses the pick visibly rather than acting on
    /// a name that no longer exists), and so is the load-time `on_pick` lint.
    pub script: ScriptedDialogue,
}

/// Copy one live dialogue into a [`DialogueState`].
///
/// Infallible since issue #985: the only node shape left is the one
/// `project_node` builds, whose responses are `(text, important)`. It used to
/// return `None` for a declarative node whose responses carried `TriggerAction`s
/// or a nested `follow_up` - authored-config trees whose serialisation is the
/// commitment [`ScenarioState`] refuses for `pending_delayed_actions` - and the
/// front-end that could author one is gone.
fn reduce_dialogue_node(message_id: &str, dialogue: &ActiveDialogue) -> DialogueState {
    let node = &dialogue.current_node;
    DialogueState {
        message_id: message_id.to_string(),
        thread_id: dialogue.thread_id.clone(),
        body: node.body.clone(),
        body_params: node.body_params.clone(),
        responses: node
            .responses
            .iter()
            .map(|r| (r.text.clone(), r.important))
            .collect(),
        script: dialogue.script.clone(),
    }
}

/// Captured authoritative world state: everything issue #894's record says a
/// divergence is defined over, at one tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhoenixSnapshot {
    /// The logical tick the capture was taken between. Mirrors
    /// `vellum_save::Snapshot::tick`, which is the envelope's copy; this is the
    /// resource's own value, and [`restore`] writes it back.
    pub tick: u64,
    pub rng: Option<SimRngState>,
    pub mint: Option<WorldIdMintState>,
    pub phase: Option<GamePhase>,
    /// `(reason, outcome label)`. The outcome is a label rather than the
    /// `Outcome` enum because `Outcome` is not `Serialize` — and the labels are
    /// already this run's report vocabulary, so nothing is lost and one fewer
    /// enum's variant order becomes stored surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_over: Option<(Option<String>, Option<String>)>,
    /// `(scope, objective)` pairs in sorted key order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captain_boosts: Vec<(String, String)>,
    /// The whole `WorldResource` payload — see the module docs for why this is
    /// wider than the digest's projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldData>,
    /// The base world's flag store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<FlagStore>,
    /// Per-layer flag stores, sorted by path so the payload is byte-stable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_flags: Vec<LayerFlags>,
    /// The scenario's *progression*: what has already fired, what is still
    /// scheduled, and the mission clock both are measured against — see
    /// [`ScenarioState`]. `None` only for a world with no `WorldContentRuntime`
    /// at all (a bare-`App` fixture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<ScenarioState>,
    /// The conversation the scenario is in the middle of having — the inbox, the
    /// dialogues that make it answerable, and the scripted thread opens still
    /// queued. See [`CommsState`]. `None` only for a world with no
    /// `CommsRuntime` at all (a bare-`App` fixture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comms: Option<CommsState>,
    /// `AiPolicyTickClock` — the tick-derived clock every stateful AI policy
    /// measures `state_time` against (issue #882's AC4).
    ///
    /// One `f64`, and the most load-bearing one in this payload.
    /// [`PolicyState::entered_at_secs`] is a reading *of this clock*, so
    /// restoring the policies without it hands every ship a state entered three
    /// seconds into a clock that now reads a sixteenth of a second:
    /// `memory_at` clamps the resulting negative `state_time` to zero, every
    /// time-gated transition evaluates as though the state had only just been
    /// entered, and the next AI tick walks a different edge. That was the whole
    /// of the tick-2 divergence this slice first measured and attributed to
    /// cold weapon machines — it was not the weapons. It was two ships falling
    /// out of `torpedo_run` and `inbound` back into `acquire`, one tick after a
    /// restore whose digest had matched exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_policy_clock: Option<f64>,
    /// Sorted by uuid — a payload must not inherit ECS iteration order any more
    /// than the digest may.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asteroids: Vec<AsteroidState>,
    /// The streamer's window over those rocks — see [`AsteroidWindowState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asteroid_window: Option<AsteroidWindowState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collisions: Vec<CollisionRecord>,
}

// ── Capture ──────────────────────────────────────────────────────────────────

/// Walk a live world and take its authoritative state.
///
/// Takes `&World`, not `&mut World`, for the same reason `world_digest` does:
/// capturing must not perturb the run it is capturing. Every read goes through
/// `get_resource`/`try_query`, so a bare-`App` fixture with half the world
/// unregistered produces a partial payload rather than a panic.
///
/// Call this between `App::update()` calls — outside `SimSet`, at a tick
/// boundary. `SimRng::state`'s own docs say why: mid-tick, some systems for the
/// step have drawn and others have not, so "all six streams right now" is not a
/// point any system agrees on.
pub fn capture(world: &World) -> PhoenixSnapshot {
    PhoenixSnapshot {
        tick: world.get_resource::<SimTick>().map_or(0, |t| t.0),
        rng: world.get_resource::<SimRng>().map(SimRng::state),
        mint: world.get_resource::<WorldIdMint>().map(WorldIdMint::state),
        phase: world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone()),
        game_over: world
            .get_resource::<GameOverReason>()
            .map(|reason| (reason.0.clone(), reason.1.map(|o| o.as_str().to_string()))),
        captain_boosts: world
            .get_resource::<CaptainPriorityBoost>()
            .map(|boosts| {
                boosts
                    .boosts_sorted()
                    .into_iter()
                    .map(|(scope, objective)| (scope.to_string(), objective.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        world: world.get_resource::<WorldResource>().map(|w| w.0.clone()),
        flags: world
            .get_resource::<WorldContentRuntime>()
            .map(|rt| rt.flags.clone()),
        layer_flags: capture_layer_flags(world),
        scenario: capture_scenario(world),
        comms: capture_comms(world),
        ai_policy_clock: world
            .get_resource::<crate::ship::helm_ai::AiPolicyTickClock>()
            .map(|clock| clock.0),
        entities: capture_entities(world),
        asteroids: capture_asteroids(world),
        asteroid_window: capture_asteroid_window(world),
        collisions: capture_collisions(world),
    }
}

fn capture_layer_flags(world: &World) -> Vec<LayerFlags> {
    let Some(layers) = world.get_resource::<crate::world::server::WorldLayerMap>() else {
        return Vec::new();
    };
    let mut rows: Vec<LayerFlags> = layers
        .0
        .iter()
        .map(|(path, runtime)| LayerFlags {
            path: path.clone(),
            flags: runtime.flags.clone(),
        })
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

/// The fixed-step clock the mission anchor is a reading of.
///
/// `Time<Fixed>` explicitly, not the context-sensitive `Time`: `anchor_mission_clock`
/// runs inside `FixedUpdate`, where `Time` *is* `Time<Fixed>`, and a capture taken
/// between `App::update()` calls would read `Time<Virtual>` instead — two clocks
/// that disagree by up to a timestep, which is exactly the drift issue #960 spent
/// a whole system removing from this anchor.
fn fixed_elapsed_secs(world: &World) -> Option<f32> {
    world
        .get_resource::<Time<bevy::time::Fixed>>()
        .map(|t| t.elapsed_secs())
}

/// Walk the scenario's progression state — see [`ScenarioState`].
fn capture_scenario(world: &World) -> Option<ScenarioState> {
    let runtime = world.get_resource::<WorldContentRuntime>()?;

    // The anchor is a reading of THIS process's clock; the distance from it is
    // what a resumed run needs. See `ScenarioState`.
    let mission_elapsed_secs = match (runtime.mission_clock_anchor_secs, fixed_elapsed_secs(world))
    {
        (Some(anchor), Some(now)) => Some((now - anchor).max(0.0)),
        _ => None,
    };

    let triggers = runtime
        .trigger_states
        .iter()
        .enumerate()
        .map(|(index, state)| {
            let mut seen_destroyed: Vec<String> = state.seen_destroyed.iter().cloned().collect();
            seen_destroyed.sort();
            TriggerRuntimeState {
                index: index as u32,
                fired: state.fired,
                seen_destroyed,
                last_fired_elapsed: state.last_fired_elapsed,
            }
        })
        .collect();

    let mut entity_groups: Vec<(String, Vec<String>)> = runtime
        .entity_groups
        .iter()
        .map(|(group, members)| {
            let mut names: Vec<String> = members.iter().cloned().collect();
            names.sort();
            (group.clone(), names)
        })
        .collect();
    entity_groups.sort_by(|a, b| a.0.cmp(&b.0));

    let mut name_to_uuid: Vec<(String, String)> = runtime
        .name_to_uuid
        .iter()
        .map(|(name, uuid)| (name.clone(), uuid.clone()))
        .collect();
    name_to_uuid.sort_by(|a, b| a.0.cmp(&b.0));

    let mut observed_hull_fractions: Vec<(String, f32)> = runtime
        .observed_hull_fractions
        .iter()
        .map(|(uuid, fraction)| (uuid.clone(), *fraction))
        .collect();
    observed_hull_fractions.sort_by(|a, b| a.0.cmp(&b.0));

    Some(ScenarioState {
        mission_elapsed_secs,
        triggers,
        entity_groups,
        // Which entity each authored name means (issue #863) — see
        // `ScenarioState::name_to_uuid`.
        name_to_uuid,
        observed_hull_fractions,
        pending_events: runtime
            .pending_world_events
            .iter()
            .map(world_event_record)
            .collect(),
        // The scripted half. Absent for every script-free world, which is what
        // keeps their payloads shaped exactly as they were before this issue.
        script_callbacks: world
            .get_resource::<WorldScriptRuntime>()
            .map(|script| script.pending_callbacks.0.clone())
            .unwrap_or_default(),
        // The named half (issue #1024). Empty — and so absent from the payload —
        // for every world that authors no `[[deadline]]`, which is every shipped
        // world today.
        deadlines: runtime.deadlines.clone(),
        // The promises (issue #1029). Empty — and so absent from the payload —
        // for every run that never reached a beat where the captain gave their
        // word, which is every shipped world today.
        commitments: runtime.commitments.clone(),
        // What the crew found out (issue #1031). Empty — and so absent from the
        // payload — for every run whose scenario never said the crew learned
        // anything, which is every shipped world today.
        evidence: runtime.evidence.clone(),
        // The labour dispute (issue #1035). Empty — and so absent from the
        // payload — for every world that authors no `[[workforce]]`, which is
        // every shipped world but Falling Skyway.
        workforce: runtime.workforce.clone(),
    })
}

/// Walk the comms state a mid-conversation save resumes from — see
/// [`CommsState`].
///
/// Read-only, like every other `capture_*` here, so a save-free run is
/// digest-neutral by construction: nothing in this function takes a `&mut` on
/// any resource, so no change-detection tick flips and no system downstream sees
/// a different world because a capture was taken.
fn capture_comms(world: &World) -> Option<CommsState> {
    let comms = world.get_resource::<CommsRuntime>()?;

    // Sorted by message id — the map's own key. See `CommsState::dialogues`.
    let mut dialogues: Vec<DialogueState> = comms
        .active_dialogues
        .iter()
        .map(|(message_id, dialogue)| reduce_dialogue_node(message_id, dialogue))
        .collect();
    dialogues.sort_by(|a, b| a.message_id.cmp(&b.message_id));

    Some(CommsState {
        inbox: world
            .get_resource::<CommsInboxRes>()
            .map(|inbox| inbox.0.messages())
            .unwrap_or_default(),
        dialogues,
        open_hails: comms.open_hails.iter().cloned().collect(),
        // The scripted half, absent for every script-free world — the same
        // shape-preserving property `ScenarioState::script_callbacks` has.
        pending_opens: world
            .get_resource::<WorldScriptRuntime>()
            .map(|script| script.pending_comms_opens.clone())
            .unwrap_or_default(),
    })
}

/// Tag one queued `WorldEvent` — see [`WorldEventRecord`].
fn world_event_record(event: &WorldEvent) -> WorldEventRecord {
    let mut row = WorldEventRecord::default();
    match event {
        WorldEvent::Destroyed { uuid } => {
            row.kind = 0;
            row.subject = uuid.clone();
        }
        WorldEvent::Attacked {
            uuid,
            attacker_uuid,
        } => {
            row.kind = 1;
            row.subject = uuid.clone();
            row.other = attacker_uuid.clone();
        }
        WorldEvent::HullDroppedBelow {
            uuid,
            previous_fraction,
            current_fraction,
        } => {
            row.kind = 2;
            row.subject = uuid.clone();
            row.numbers = [*previous_fraction, *current_fraction];
        }
        WorldEvent::TimerElapsed { elapsed_secs } => {
            row.kind = 3;
            row.numbers = [*elapsed_secs, 0.0];
        }
        WorldEvent::Hailed { target_uuid } => {
            row.kind = 4;
            row.subject = target_uuid.clone();
        }
        WorldEvent::FlagSet { name, origin_layer } => {
            row.kind = 5;
            row.subject = name.clone();
            row.origin_layer = origin_layer.clone();
        }
        WorldEvent::FlagCleared { name, origin_layer } => {
            row.kind = 6;
            row.subject = name.clone();
            row.origin_layer = origin_layer.clone();
        }
        WorldEvent::WorldLoaded => row.kind = 7,
        WorldEvent::EnteredRegion { uuid } => {
            row.kind = 8;
            row.subject = uuid.clone();
        }
        WorldEvent::ExitedRegion { uuid } => {
            row.kind = 9;
            row.subject = uuid.clone();
        }
        WorldEvent::WaypointReached { uuid, waypoint } => {
            row.kind = 10;
            row.subject = uuid.clone();
            row.other = waypoint.clone();
        }
    }
    row
}

/// The inverse of [`world_event_record`]. An unrecognised tag is `None` — a save
/// from a build with an event kind this one does not have, which the version
/// gate is what refuses, not a `match` arm here.
fn world_event_from_record(row: &WorldEventRecord) -> Option<WorldEvent> {
    Some(match row.kind {
        0 => WorldEvent::Destroyed {
            uuid: row.subject.clone(),
        },
        1 => WorldEvent::Attacked {
            uuid: row.subject.clone(),
            attacker_uuid: row.other.clone(),
        },
        2 => WorldEvent::HullDroppedBelow {
            uuid: row.subject.clone(),
            previous_fraction: row.numbers[0],
            current_fraction: row.numbers[1],
        },
        3 => WorldEvent::TimerElapsed {
            elapsed_secs: row.numbers[0],
        },
        4 => WorldEvent::Hailed {
            target_uuid: row.subject.clone(),
        },
        5 => WorldEvent::FlagSet {
            name: row.subject.clone(),
            origin_layer: row.origin_layer.clone(),
        },
        6 => WorldEvent::FlagCleared {
            name: row.subject.clone(),
            origin_layer: row.origin_layer.clone(),
        },
        7 => WorldEvent::WorldLoaded,
        8 => WorldEvent::EnteredRegion {
            uuid: row.subject.clone(),
        },
        9 => WorldEvent::ExitedRegion {
            uuid: row.subject.clone(),
        },
        10 => WorldEvent::WaypointReached {
            uuid: row.subject.clone(),
            waypoint: row.other.clone(),
        },
        _ => return None,
    })
}

fn hull_rows(hull: &crate::ship::damage::SystemHull) -> Vec<(String, f32, f32)> {
    hull.iter()
        .map(|(id, entry)| (id.0.clone(), entry.current, entry.max))
        .collect()
}

/// The helm axes, in a query of their own.
///
/// Separate from [`capture_entities`]' walk because Bevy's query tuples do not
/// stretch that far, and joined back by uuid rather than by handle — the same
/// rule the rest of this module keeps.
fn capture_controls(world: &World) -> Vec<(String, ControlState)> {
    // A row is emitted only for an entity that actually carries the helm axes.
    // The distinction is load-bearing: `map_or(0.0, ..)` over an absent
    // component and a genuinely centred stick produce the same numbers, so
    // without this an entity with no helm at all would be captured as one
    // holding neutral — and `ready_to_restore` would then have no way to tell
    // that a freshly-spawned ship had not yet been given its controls.
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ThrustInput>,
        Option<&SteeringInput>,
        Option<&LateralThrustInput>,
        Option<&VerticalThrustInput>,
        Option<&BoostCommand>,
        Option<&ImpulseCommand>,
        Option<&LastHelmInput>,
        Option<&TacticalRadarSelection>,
        Option<&LastShipAttacker>,
        Option<&HelmEnginesAiPolicyState>,
        Option<&HelmSteeringAiPolicyState>,
        Option<&HelmBoostAiPolicyState>,
        Option<&crate::ship::helm_ai::HelmRecoveryHistory>,
    )>() else {
        return Vec::new();
    };
    query
        .iter(world)
        .filter(|(_, thrust, ..)| thrust.is_some())
        .map(
            |(
                uuid,
                thrust,
                steering,
                lateral,
                vertical,
                boost,
                impulse,
                last,
                lock,
                attacker,
                engines_policy,
                steering_policy,
                boost_policy,
                recovery,
            )| {
                (
                    uuid.0.clone(),
                    ControlState {
                        thrust: thrust.map_or(0.0, |t| t.0),
                        steering: steering.map_or(0.0, |s| s.0),
                        lateral: lateral.map_or(0.0, |l| l.0),
                        vertical: vertical.map_or(0.0, |v| v.0),
                        boost: boost.is_some_and(|b| b.0),
                        impulse_phase: impulse.map_or(0, |i| match i.0 {
                            ImpulsePhase::Idle => 0,
                            ImpulsePhase::Charging => 1,
                            ImpulsePhase::Active => 2,
                        }),
                        last_helm: last.map_or([0.0; 3], |l| [l.thrust, l.steering, l.lateral]),
                        target_lock: lock.and_then(|l| l.0.clone()),
                        last_attacker: attacker.and_then(|a| a.0.clone()),
                        // Joined in by uuid in `capture_entities` — see
                        // `capture_sensor_locks`; the helm-axes query is at
                        // Bevy's width already.
                        sensor_lock: None,
                        helm_policies: Some([
                            policy_state(engines_policy.map(|p| &p.0)),
                            policy_state(steering_policy.map(|p| &p.0)),
                            policy_state(boost_policy.map(|p| &p.0)),
                        ]),
                        helm_recovery: recovery.map(|r| RecoveryHistory {
                            target: r.target.map(|t| t.to_string()),
                            ranges: r.ranges.iter().collect(),
                            ranges_capacity: r.ranges.capacity() as u32,
                            separation: r.separation.iter().collect(),
                            separation_capacity: r.separation.capacity() as u32,
                        }),
                    },
                )
            },
        )
        .collect()
}

/// The Sensors radar's Science Target lock, in a query of its own.
///
/// Kept out of [`capture_controls`]' tuple — which is already at Bevy's query
/// width — and joined back by uuid, the rule the module keeps. A row is emitted
/// only for a ship whose Sensors radar actually holds a lock, so an unlocked
/// radar stores nothing rather than a `Some(None)` that restore would have to
/// distinguish from absence.
fn capture_sensor_locks(world: &World) -> Vec<(String, String)> {
    let Some(mut query) =
        world.try_query::<(&EntityUuid, &crate::ship::sensors::SensorRadarSelection)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .filter_map(|(uuid, lock)| lock.0.clone().map(|target| (uuid.0.clone(), target)))
        .collect()
}

fn policy_state(runtime: Option<&crate::ai::policy::AiPolicyRuntimeState>) -> PolicyState {
    runtime.map_or_else(PolicyState::default, |r| PolicyState {
        current: r.current.clone(),
        entered_at_secs: r.entered_at_secs,
        memory: r.memory.clone(),
    })
}

fn apply_policy_state(runtime: &mut crate::ai::policy::AiPolicyRuntimeState, stored: &PolicyState) {
    runtime.current = stored.current.clone();
    runtime.entered_at_secs = stored.entered_at_secs;
    runtime.memory = stored.memory.clone();
}

/// The weapon state machines and the repair crew, in a query of their own.
///
/// Separate from [`capture_entities`] for [`capture_controls`]' reason — Bevy's
/// query tuples do not stretch that far — and joined back by uuid rather than
/// by handle, which is the rule the whole module keeps.
type WeaponRepairRow = (
    String,
    Option<WeaponState>,
    Option<RepairState>,
    Vec<(String, crate::core::messages::SystemBlackboard)>,
    Vec<(String, u32, bool)>,
);

fn capture_weapons_and_repair(world: &World) -> Vec<WeaponRepairRow> {
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ActiveBeam>,
        Option<&PhaserCooldown>,
        Option<&TorpedoSystemResource>,
        Option<&EntityShipArcHull>,
        Option<&ShipRepairTeams>,
        Option<&RepairRequestQueue>,
        Option<&RepairHumanAlerted>,
        Option<&crate::server_app::ShipSystemBlackboards>,
        Option<&crate::ai::server::ObjectiveCursors>,
        Option<&crate::console::weapons::blaster::BlasterSystemResource>,
        Option<&crate::ship::shields::ShipShields>,
    )>() else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(
            |(
                uuid,
                beam,
                cooldown,
                torpedoes,
                arcs,
                teams,
                queue,
                alerted,
                blackboards,
                cursors,
                blasters,
                shields,
            )| {
                // A row is emitted only for an entity that carries at least one
                // of these, for `capture_controls`' reason: an all-defaults
                // `WeaponState` and a genuinely idle one are the same bytes, so
                // storing one for every entity would make an asteroid look like
                // a ship with its weapons cold.
                let weapons = (beam.is_some()
                    || cooldown.is_some()
                    || torpedoes.is_some()
                    || arcs.is_some()
                    || blasters.is_some()
                    || shields.is_some())
                .then(|| weapon_state(beam, cooldown, torpedoes, arcs, blasters, shields));
                let repair = (teams.is_some() || queue.is_some() || alerted.is_some())
                    .then(|| repair_state(teams, queue, alerted));
                let mut boards: Vec<(String, crate::core::messages::SystemBlackboard)> =
                    blackboards
                        .map(|b| {
                            b.0.iter()
                                .map(|(id, board)| (id.0.clone(), board.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                // The component is a `HashMap` on purpose (see its own docs);
                // a payload may not inherit that order.
                boards.sort_by(|a, b| a.0.cmp(&b.0));
                let cursors = cursors
                    .map(|c| {
                        c.0.iter()
                            .map(|cursor| {
                                (
                                    cursor.objective_id.clone(),
                                    cursor.index() as u32,
                                    cursor.settled(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (uuid.0.clone(), weapons, repair, boards, cursors)
            },
        )
        .collect()
}

/// The Weapons→Helm arc-bearing seam, in a query of its own.
///
/// Separate from [`capture_weapons_and_repair`] for that helper's reason — the
/// query tuple is already at its limit — and joined back by uuid. A row is
/// emitted only when at least one half of the seam carries something the restore
/// must put back, so an idle ship (no debounce, no pending bearing) stores
/// nothing rather than a default `ArcRequestState`.
fn capture_arc_requests(world: &World) -> Vec<(String, ArcRequestState)> {
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&crate::console::weapons::WeaponsArcRequestState>,
        Option<&crate::ship_plugin::PendingArcBearingRequest>,
    )>() else {
        return Vec::new();
    };
    query
        .iter(world)
        .filter_map(|(uuid, weapons_state, pending)| {
            let last = weapons_state.and_then(|s| {
                s.last
                    .as_ref()
                    .map(|(family, target, arcs)| (*family, target.clone(), arcs.clone()))
            });
            let pending_target = pending.and_then(|p| p.target.map(|t| t.to_string()));
            let pending_arcs = pending.map(|p| p.arcs.clone()).unwrap_or_default();
            (last.is_some() || pending_target.is_some() || !pending_arcs.is_empty()).then(|| {
                (
                    uuid.0.clone(),
                    ArcRequestState {
                        last,
                        pending_target,
                        pending_arcs,
                    },
                )
            })
        })
        .collect()
}

/// The reactor allocation, in a query of its own, joined back by uuid.
///
/// A row is emitted for every ship carrying a [`ShipPowerSystem`] — unlike the
/// weapon and arc rows there is no "all defaults look idle" ambiguity to guard
/// against, because a ship either has a reactor or it does not, and a defaulted
/// reactor (every group at 2, full battery, unlocked) is a genuinely different
/// state from the boosted one this restore exists to reinstate.
fn capture_power(world: &World) -> Vec<(String, PowerState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::ship::power::ShipPowerSystem)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, power)| {
            (
                uuid.0.clone(),
                PowerState {
                    allocations: power
                        .0
                        .iter()
                        .map(|(id, level)| (id.0.clone(), level))
                        .collect(),
                    battery_charge: power.0.battery_charge,
                    locked: power.0.locked(),
                },
            )
        })
        .collect()
}

/// The helm pass surface, in a query of its own, joined by uuid — see
/// [`EntityState::pass_surface`]. A row is emitted for every ship carrying one;
/// the planner reads it every tick, so there is no "idle looks default"
/// ambiguity to guard against.
fn capture_pass_surfaces(world: &World) -> Vec<(String, crate::ship::helm_ai::HelmPassSurface)> {
    let Some(mut query) =
        world.try_query::<(&EntityUuid, &crate::ship::helm_ai::HelmPassSurface)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, surface)| (uuid.0.clone(), *surface))
        .collect()
}

fn weapon_state(
    beam: Option<&ActiveBeam>,
    cooldown: Option<&PhaserCooldown>,
    torpedoes: Option<&TorpedoSystemResource>,
    arcs: Option<&EntityShipArcHull>,
    blasters: Option<&crate::console::weapons::blaster::BlasterSystemResource>,
    shields: Option<&crate::ship::shields::ShipShields>,
) -> WeaponState {
    let system = torpedoes.map(|t| &t.0);
    WeaponState {
        beams: beam
            .map(|b| {
                b.live_banks()
                    .map(|(bank, slot)| {
                        (
                            bank.clone(),
                            slot.target_uuid.clone(),
                            slot.remaining_secs,
                            slot.damage_accumulator,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        phaser_cooldowns: cooldown
            .map(PhaserCooldown::active_banks_sorted)
            .unwrap_or_default(),
        tubes: system
            .map(|s| {
                s.tubes
                    .iter()
                    .map(|tube| {
                        let (load_phase, load_timer) = match &tube.load_state {
                            TubeLoadState::Unloaded => (0, [0.0, 0.0]),
                            TubeLoadState::Loading { remaining, total } => {
                                (1, [*remaining, *total])
                            }
                            TubeLoadState::Loaded => (2, [0.0, 0.0]),
                            TubeLoadState::Unloading { remaining, total } => {
                                (3, [*remaining, *total])
                            }
                        };
                        TubeState {
                            id: tube.id.clone(),
                            load_phase,
                            load_timer,
                            loaded_count: tube.loaded_count,
                            target_count: tube.target_count,
                            active_barrels: tube.active_barrels.clone(),
                            pattern_step: tube.pattern_step,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        torpedoes_remaining: system.map(|s| s.torpedoes_remaining),
        torpedoes_in_flight: system
            .map(|s| {
                s.in_flight
                    .iter()
                    .map(|t| TorpedoInFlight {
                        uuid: t.uuid.clone(),
                        position: [t.x, t.y, t.z],
                        heading: t.heading,
                        pitch: t.pitch,
                        lifespan_remaining: t.lifespan_remaining,
                        target_uuid: t.target_uuid.clone(),
                        source_uuid: t.source_uuid.clone(),
                        tube_id: t.tube_id.clone(),
                        shield_pierce: t.shield_pierce,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        bursts: system
            .map(|s| {
                s.burst_states
                    .iter()
                    .map(|b| BurstState {
                        tube_id: b.tube_id.clone(),
                        pending: b.pending,
                        timer: b.timer,
                        launch: [b.launch_x, b.launch_y, b.launch_z],
                        launch_heading: b.launch_heading,
                        target_uuid: b.target_uuid.clone(),
                        source_uuid: b.source_uuid.clone(),
                        barrel_origins: b
                            .barrel_origins
                            .iter()
                            .map(|(x, y, z)| [*x, *y, *z])
                            .collect(),
                        barrel_sequence: b.barrel_sequence.clone(),
                        next_shot_index: b.next_shot_index,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        arc_hull: arcs
            .map(|a| {
                a.0.iter()
                    .map(|(id, entry)| (id.to_string(), entry.current, entry.max))
                    .collect()
            })
            .unwrap_or_default(),
        shield_charge: shields
            .map(|s| {
                s.0.facings
                    .iter()
                    .map(|f| {
                        (
                            f.id.clone(),
                            f.hp,
                            f.hp_frac(),
                            f.offline_remaining,
                            f.is_focused,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        blasters: blasters
            .map(|b| {
                b.0.iter()
                    .map(|bank| {
                        let v = &bank.volley;
                        BlasterRuntime {
                            pending_volley: v.pending_volley,
                            schedule: v.schedule.clone(),
                            next_step: v.next_step as u32,
                            volley_elapsed: v.volley_elapsed,
                            active_barrels: v.active_barrels.clone(),
                            current_step: v.current_step,
                            on_cooldown: v.on_cooldown,
                            cooldown_remaining: v.cooldown_remaining,
                            charging: v.charging,
                            charge_elapsed: v.charge_elapsed,
                            in_flight: bank
                                .in_flight
                                .iter()
                                .map(|p| BlasterBolt {
                                    id: p.id.clone(),
                                    x: p.x,
                                    z: p.z,
                                    heading: p.heading,
                                    speed: p.speed,
                                    lifespan_remaining: p.lifespan_remaining,
                                    collision_radius: p.collision_radius,
                                    damage: p.damage,
                                    shield_pierce: p.shield_pierce,
                                    source_uuid: p.source_uuid.clone(),
                                })
                                .collect(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn repair_state(
    teams: Option<&ShipRepairTeams>,
    queue: Option<&RepairRequestQueue>,
    alerted: Option<&RepairHumanAlerted>,
) -> RepairState {
    let mut alerted_rows: Vec<(String, crate::ship::damage::DamageTier)> = alerted
        .map(|a| {
            a.0.iter()
                .map(|(system, tier)| (system.clone(), *tier))
                .collect()
        })
        .unwrap_or_default();
    alerted_rows.sort_by(|a, b| a.0.cmp(&b.0));
    RepairState {
        teams: teams.map(|t| t.0.slots().to_vec()).unwrap_or_default(),
        queue: queue
            .map(|q| {
                q.entries
                    .iter()
                    .map(|e| {
                        (
                            e.station_id.clone(),
                            e.station_label.clone(),
                            e.tier,
                            e.deficit,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        alerted: alerted_rows,
    }
}

/// The infrastructure condition tracks, in a query of their own, joined by uuid
/// — see [`EntityState::infrastructure`]. Only entities that authored
/// `[infrastructure]` carry one, so most worlds capture an empty list.
fn capture_infrastructure(
    world: &World,
) -> Vec<(String, crate::infrastructure::InfrastructureState)> {
    let Some(mut query) =
        world.try_query::<(&EntityUuid, &crate::infrastructure::InfrastructureCondition)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, condition)| (uuid.0.clone(), condition.0.clone()))
        .collect()
}

/// The tractor-beam states, in a query of their own, joined by uuid — see
/// [`EntityState::tractor`]. Only hulls that authored `[tractor]` carry one, so
/// most worlds capture an empty list.
///
/// A beam that authored a tractor and is engaging nothing captures nothing — the
/// same reading `fold_tractor_namespace` takes: an idle beam is byte-for-byte the
/// state of a hull built before tractors existed, so writing it would charge
/// every world fielding a tractor-capable hull for a feature its crew never used.
fn capture_tractors(world: &World) -> Vec<(String, crate::tractor::TractorSaveState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::tractor::TractorBeam)>() else {
        return Vec::new();
    };
    let idle = crate::tractor::TractorSaveState::default();
    query
        .iter(world)
        .map(|(uuid, beam)| (uuid.0.clone(), beam.save_state()))
        .filter(|(_, state)| *state != idle)
        .collect()
}

/// The dock control states, in a query of their own, joined by uuid — see
/// [`EntityState::dock`]. Only hulls that authored a `kind = "dock"` system carry
/// one, so most worlds capture an empty list. An idle control (engaging nothing,
/// docked to nothing) captures nothing, the same reading `fold_dock_namespace`
/// takes.
fn capture_docks(world: &World) -> Vec<(String, crate::dock::DockSaveState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::dock::DockControl)>() else {
        return Vec::new();
    };
    let idle = crate::dock::DockSaveState::default();
    query
        .iter(world)
        .map(|(uuid, control)| (uuid.0.clone(), control.save_state()))
        .filter(|(_, state)| *state != idle)
        .collect()
}

/// The external repair-dispatch states, in a query of their own, joined by uuid
/// — see [`EntityState::external_repair`]. Only hulls that authored
/// `[repair.external_dispatch]` carry one, so most worlds capture an empty list.
///
/// A hull that CAN dispatch and has sent nobody captures nothing — the same
/// reading `fold_external_repair_namespace` takes: no dispatched target is
/// byte-for-byte the state of a hull built before external dispatch existed, so
/// writing it would charge every world fielding a capable hull for a feature its
/// crew never used.
fn capture_external_repair(
    world: &World,
) -> Vec<(String, crate::console::repair::ExternalRepairSaveState)> {
    let Some(mut query) =
        world.try_query::<(&EntityUuid, &crate::console::repair::ExternalRepairDispatch)>()
    else {
        return Vec::new();
    };
    let idle = crate::console::repair::ExternalRepairSaveState::default();
    query
        .iter(world)
        .map(|(uuid, dispatch)| (uuid.0.clone(), dispatch.save_state()))
        .filter(|(_, state)| *state != idle)
        .collect()
}

/// The transfer-umbilical states, in a query of their own, joined by uuid — see
/// [`EntityState::umbilical`]. Only hulls that authored a `kind = "umbilical"`
/// system carry one, so most worlds capture an empty list. An idle umbilical (not
/// running) captures nothing, the same reading `fold_umbilical_namespace` takes.
fn capture_umbilicals(world: &World) -> Vec<(String, crate::umbilical::UmbilicalSaveState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::umbilical::TransferUmbilical)>()
    else {
        return Vec::new();
    };
    let idle = crate::umbilical::UmbilicalSaveState::default();
    query
        .iter(world)
        .map(|(uuid, umbilical)| (uuid.0.clone(), umbilical.save_state()))
        .filter(|(_, state)| *state != idle)
        .collect()
}

/// The scan records, in a query of their own, joined by uuid — see
/// [`EntityState::scan`]. Only hulls that authored `[scan]` carry one, so most
/// worlds capture an empty list.
fn capture_scans(world: &World) -> Vec<(String, crate::science::ScanSaveState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::science::ShipScanRecord)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, record)| (uuid.0.clone(), record.save_state()))
        .collect()
}

/// The spawn origins, in a query of their own, joined by uuid — see
/// [`EntityState::spawn`] (issue #863).
///
/// A query of its own for the reason every sibling here has one, and this type
/// is the sharpest case of it: `try_query` yields `None` when *any* component it
/// names is unregistered, `Option<&T>` included, and `EntitySpawnOrigin` is only
/// registered once a world has actually run a scripted spawn. Folding it into
/// the main walk therefore made every capture of a spawn-free world — which is
/// most of them — return no entity rows at all.
fn capture_spawn_origins(world: &World) -> Vec<(String, crate::world::spawn_origin::SpawnOrigin)> {
    let Some(mut query) =
        world.try_query::<(&EntityUuid, &crate::entities::spawner::EntitySpawnOrigin)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, origin)| (uuid.0.clone(), origin.0.clone()))
        .collect()
}

/// The civilian traffic states, in a query of their own, joined by uuid — see
/// [`EntityState::civilian`]. Only entities that authored `[civilian]` carry
/// one, so most worlds capture an empty list.
fn capture_civilians(world: &World) -> Vec<(String, crate::civilian::CivilianState)> {
    let Some(mut query) = world.try_query::<(&EntityUuid, &crate::civilian::CivilianTraffic)>()
    else {
        return Vec::new();
    };
    query
        .iter(world)
        .map(|(uuid, traffic)| (uuid.0.clone(), traffic.0.clone()))
        .collect()
}

fn capture_entities(world: &World) -> Vec<EntityState> {
    let controls = capture_controls(world);
    let machines = capture_weapons_and_repair(world);
    let arc_requests = capture_arc_requests(world);
    let power = capture_power(world);
    let sensor_locks = capture_sensor_locks(world);
    let pass_surfaces = capture_pass_surfaces(world);
    let infrastructure = capture_infrastructure(world);
    let tractors = capture_tractors(world);
    let docks = capture_docks(world);
    let external_repair = capture_external_repair(world);
    let umbilicals = capture_umbilicals(world);
    let scans = capture_scans(world);
    let civilians = capture_civilians(world);
    let spawn_origins = capture_spawn_origins(world);
    let Some(mut query) = world.try_query::<(
        &EntityUuid,
        Option<&ShipPhysics>,
        Option<&EntitySystemHull>,
        Option<&ShipRedAlert>,
        Option<&ShipWeaponsHold>,
        Option<&crate::console::command::server::ShipStationStances>,
    )>() else {
        return Vec::new();
    };
    let mut rows: Vec<EntityState> = query
        .iter(world)
        .map(|(uuid, physics, hull, alert, hold, stances)| EntityState {
            control: controls
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| {
                    let mut state = state.clone();
                    state.sensor_lock = sensor_locks
                        .iter()
                        .find(|(id, _)| id == &uuid.0)
                        .map(|(_, target)| target.clone());
                    state
                }),
            weapons: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .and_then(|(_, weapons, ..)| weapons.clone()),
            repair: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .and_then(|(_, _, repair, ..)| repair.clone()),
            arc_request: arc_requests
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            power: power
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            pass_surface: pass_surfaces
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, surface)| *surface),
            infrastructure: infrastructure
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            tractor: tractors
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            dock: docks
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            external_repair: external_repair
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            umbilical: umbilicals
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            scan: scans
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            civilian: civilians
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, state)| state.clone()),
            blackboards: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .map(|(_, _, _, boards, _)| boards.clone())
                .unwrap_or_default(),
            patrol_cursors: machines
                .iter()
                .find(|(id, ..)| id == &uuid.0)
                .map(|(.., cursors)| cursors.clone())
                .unwrap_or_default(),
            uuid: uuid.0.clone(),
            // Read off the entity rather than joined out of a ledger — see
            // [`crate::world::spawn_origin`] for why the record rides there.
            // Absent on every authored `[[entity]]`, which is the signal a
            // restore reads it for.
            spawn: spawn_origins
                .iter()
                .find(|(id, _)| id == &uuid.0)
                .map(|(_, origin)| origin.clone()),
            physics: physics.map(|p| {
                [
                    p.x,
                    p.y,
                    p.z,
                    p.yaw,
                    p.forward_speed,
                    p.roll,
                    p.lateral_speed,
                    p.vertical_speed,
                ]
            }),
            hull: hull.map(|h| hull_rows(&h.0)),
            red_alert: alert.map(|a| a.0),
            weapons_hold: hold.map(|h| h.0),
            // Sorted by station id, the same walk `fold_station_stances_namespace`
            // takes, so the capture is byte-identical whatever order the map's
            // entries were inserted in. An empty map yields an empty vec, which
            // `skip_serializing_if` drops — a never-commanded hull carries no row.
            station_stances: stances
                .map(|s| {
                    let mut pairs: Vec<(String, String)> =
                        s.0.iter()
                            .map(|(station, stance)| (station.0.clone(), stance.clone()))
                            .collect();
                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                    pairs
                })
                .unwrap_or_default(),
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

fn capture_asteroids(world: &World) -> Vec<AsteroidState> {
    // The streaming window is the only place a live rock's config path
    // survives — nothing on the entity carries it — and [`restore`] needs it to
    // rebuild a rock the target world never streamed.
    let config_paths: Vec<(String, String)> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .slots
                .iter()
                .flatten()
                .flatten()
                .map(|data| (data.uuid.clone(), data.config_path.clone()))
                .collect()
        })
        .unwrap_or_default();

    let Some(mut query) = world.try_query::<(
        &AsteroidUuid,
        Option<&Transform>,
        Option<&EntitySystemHull>,
        Option<&crate::server_app::AsteroidShieldPierce>,
    )>() else {
        return Vec::new();
    };
    let mut rows: Vec<AsteroidState> = query
        .iter(world)
        .map(|(uuid, transform, hull, pierce)| {
            let t = transform.map(|t| t.translation).unwrap_or(Vec3::ZERO);
            let r = transform.map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            AsteroidState {
                config_path: config_paths
                    .iter()
                    .find(|(id, _)| id == &uuid.0)
                    .map(|(_, path)| path.clone()),
                uuid: uuid.0.clone(),
                translation: [t.x, t.y, t.z],
                rotation: [r.x, r.y, r.z, r.w],
                hull: hull.map(|h| hull_rows(&h.0)),
                shield_pierce: pierce.map_or(0.0, |p| p.0),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

fn capture_asteroid_window(world: &World) -> Option<AsteroidWindowState> {
    let window = world.get_resource::<AsteroidWindow>()?;
    let mut slots = Vec::new();
    for (z, row) in window.slots.iter().enumerate() {
        for (x, slot) in row.iter().enumerate() {
            let Some(data) = slot else { continue };
            slots.push(WindowSlot {
                x: x as u32,
                z: z as u32,
                uuid: data.uuid.clone(),
                config_path: data.config_path.clone(),
                hp: data.hp,
                max_hp: data.max_hp,
                y: data.y,
            });
        }
    }
    Some(AsteroidWindowState {
        arena_gx: window.arena_gx,
        arena_gz: window.arena_gz,
        despawn_cells: window.despawn_cells,
        spawn_cells: window.spawn_cells,
        resolution: window.resolution,
        player_grid: window.player_grid,
        composition_key: window.composition_key,
        needs_init: window.needs_init,
        slots,
    })
}

fn capture_collisions(world: &World) -> Vec<CollisionRecord> {
    let Some(telemetry) = world.get_resource::<RunTelemetry>() else {
        return Vec::new();
    };
    telemetry
        .balance_events
        .iter()
        .filter_map(|stamped| match &stamped.event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                victim_kind,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == WEAPON_KIND_COLLISION => Some(CollisionRecord {
                tick: stamped.tick,
                sim_t: stamped.sim_t,
                victim: victim.clone(),
                victim_is_asteroid: matches!(victim_kind, VictimKind::Asteroid),
                amount: *amount,
                shield_absorbed: *shield_absorbed,
                hull_damage: *hull_damage,
            }),
            _ => None,
        })
        .collect()
}

// ── The stored artifact ──────────────────────────────────────────────────────

/// Build the `vellum_save::Run` a save is written as.
///
/// The log is empty and the ledger holds only the capture: this is a saved
/// game, not yet a recording. `vellum-save`'s own
/// `a_snapshot_with_an_empty_log_is_a_saved_game_that_verifies` is the shape
/// this mirrors, and the continuation log is #849's to fill in — at which point
/// nothing here changes, because `Run` already has the field.
///
/// `digest` is the caller's, deliberately: it is `sim_digest::world_digest` of
/// the same world at the same instant, and taking it here would mean this
/// module deciding when a digest is meaningful. That decision belongs to the
/// caller who knows it is standing between `update()` calls.
pub fn run_for(
    payload: PhoenixSnapshot,
    digest: u64,
    seed: u64,
    scenario: impl Into<String>,
    versions: Versions,
) -> StoredRun {
    let tick = payload.tick;
    Run {
        versions,
        scenario: scenario.into(),
        seed,
        snapshot: Some(Snapshot {
            tick,
            digest,
            state: payload,
        }),
        commands: Vec::new(),
        ledger: Ledger {
            every: 0,
            samples: Vec::new(),
            final_tick: tick,
            final_digest: digest,
        },
    }
}

/// The slot a host's one save lives in.
///
/// `vellum_save::is_slot` is what decides whether a name is usable at all, and
/// it is checked by the backends rather than trusted — this is just phoenix's
/// default choice of name.
pub const DEFAULT_SLOT: &str = "autosave";

/// The `localStorage` namespace the browser backend keys under.
///
/// Namespaced because two games served from one origin — which is exactly what
/// a GitHub Pages account is — must not read or overwrite each other's saves.
pub const STORAGE_NAMESPACE: &str = "phoenix";

/// The file name a host is offered when it exports a save (issue #866).
///
/// `.ron` and not `.sav`, because the extension is a promise about the contents
/// and this one is keeping it: `Store` moves `String`, the record is RON text,
/// and an exported save is a file a human can open in an editor and read. That
/// is the property to lean on in a bug report — "paste me the first twenty
/// lines" is a diagnosis, and an opaque blob is not.
pub const EXPORT_FILE_NAME: &str = "phoenix-save.ron";

// ── The portable artifact (issue #866) ───────────────────────────────────────

/// A [`vellum_save::Store`] that holds its slots in memory, for the moment a
/// save is in transit rather than at rest.
///
/// # Why a third backend and not two lines of `to_ron`/`from_ron`
///
/// A browser has no filesystem, so "export a file" cannot be `FileStore` and
/// "import a file" cannot be `FileStore` either: the bytes arrive from a
/// `<input type="file">` and leave through a download, and the only durable
/// thing in between is the RON text itself. The obvious implementation is to
/// call `Run::to_ron` on the way out and `Run::from_ron` plus `versions.check`
/// on the way in — and that is exactly what this type exists to avoid.
///
/// [`load_from`]'s three lines are not three lines: they are an ORDER — read,
/// parse, gate — and the module's own docs explain why the gate runs before a
/// single component is written. A second copy of that sequence for the file
/// path would be a second place for it to be got wrong, and the way it would be
/// got wrong is silent (a host half-adopts a world it is about to be refused).
/// So the file path goes through the same [`save_to`] and [`load_from`] every
/// slot goes through, and what changes is only *where the string lives* — which
/// is the one thing `Store` is for.
///
/// That is also what makes this issue's "no second snapshot schema" true by
/// construction rather than by review: there is nowhere to put one. Three
/// backends, one record, one gate.
///
/// # Not a cache and not a save
///
/// It keeps nothing beyond the call that made it. Each export builds one, writes
/// one slot into it and takes the string straight back out; each import builds
/// one holding the text it was handed and reads it once. Nothing polls it,
/// nothing persists it, and it is deliberately not a `Resource`.
#[derive(Debug, Default)]
pub struct TransferStore {
    slots: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
}

impl TransferStore {
    /// An empty store, for an export about to be written into it.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A store already holding `text` in `slot`, for an import about to be read
    /// out of it.
    pub fn holding(slot: &str, text: String) -> Self {
        let store = Self::empty();
        store.slots.borrow_mut().insert(slot.to_string(), text);
        store
    }

    /// Take a slot's text out, leaving the store without it.
    pub fn take(&self, slot: &str) -> Option<String> {
        self.slots.borrow_mut().remove(slot)
    }
}

impl vellum_save::Store for TransferStore {
    /// Nothing here can fail: a `BTreeMap` in this process always answers.
    type Error = std::convert::Infallible;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.slots.borrow().get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.slots
            .borrow_mut()
            .insert(slot.to_string(), contents.to_string());
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.slots.borrow_mut().remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(self.slots.borrow().keys().cloned().collect())
    }
}

/// Serialise a run into the text of a portable save file (issue #866).
///
/// Byte-identical to what [`save_to`] hands `LocalStorage` for the same run,
/// because it *is* [`save_to`] — see [`TransferStore`]. An exported file and a
/// browser slot are the same record in the same encoding, which is what makes
/// importing one into another host a resume rather than an import format.
pub fn export_artifact(run: &StoredRun) -> Result<String, String> {
    let store = TransferStore::empty();
    save_to(&store, DEFAULT_SLOT, run)?;
    store
        .take(DEFAULT_SLOT)
        .ok_or_else(|| "the exported save was not written".to_string())
}

/// Read the text of a portable save file and put it through the same gate a
/// local slot goes through (issue #866).
///
/// The mirror of [`export_artifact`], and the same statement about the gate:
/// this is [`load_from`] over a store that happens to hold the file's text, so
/// the refusal it returns is the one a `localStorage` slot would have returned
/// for the same bytes.
///
/// The two refusals a host actually meets are different kinds of bad news and
/// stay different values here rather than collapsing into one sentence:
/// [`LoadRefusal::Unparsable`] means the FILE is damaged — truncated, edited,
/// or not a save at all — and [`LoadRefusal::Moved`] means the file is intact
/// and this BUILD cannot honour it, naming the dimension that moved. "Pick
/// another file" and "this save is from an older build" are different
/// instructions, and a host told the wrong one goes looking in the wrong place.
pub fn import_artifact(text: &str, current: &Versions) -> Result<StoredRun, LoadRefusal> {
    let store = TransferStore::holding(DEFAULT_SLOT, text.to_string());
    load_from(&store, DEFAULT_SLOT, current)
}

/// Which scenario a portable save belongs to, read WITHOUT the version gate.
///
/// The browser needs this before it can run the gate at all, and the ordering is
/// forced rather than chosen: the content dimension is a digest over the files a
/// world load consumed, so there is nothing to check a save against until its
/// world has been loaded — and which world that is, is written in the save. So
/// an import reads the scenario first, loads it, and only then asks
/// [`import_artifact`] whether this build can honour the file.
///
/// It is exactly `Run::from_ron` and one field read: parsing is the only thing
/// that can fail here, which is why a damaged file is caught at THIS step and
/// reported as damaged before any world is loaded on its behalf.
pub fn peek_artifact_scenario(text: &str) -> Result<String, LoadRefusal> {
    StoredRun::from_ron(text)
        .map(|run| run.scenario)
        .map_err(|e| LoadRefusal::Unparsable(e.to_string()))
}

/// Why a stored save did not become a resumable session.
///
/// [`LoadRefusal::Moved`] carries `vellum_save::Moved` **unchanged**, and its
/// `Display` is that type's own sentence. That is the acceptance criterion, not
/// a convenience: a phoenix-worded status would be a second answer to a
/// question the version gate has already answered, and it would lose the one
/// thing the gate exists to report — *which* dimension moved, and to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRefusal {
    /// Nothing is stored in that slot. A first run has no save, which is not an
    /// error.
    Empty,
    /// The store itself would not answer.
    Unreadable(String),
    /// The bytes are not a `Run` this build can parse.
    Unparsable(String),
    /// The version gate refused it.
    Moved(vellum_save::Moved),
}

impl std::fmt::Display for LoadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadRefusal::Empty => f.write_str("there is no save in that slot"),
            LoadRefusal::Unreadable(why) => write!(f, "the save could not be read: {why}"),
            LoadRefusal::Unparsable(why) => write!(f, "the save could not be parsed: {why}"),
            // Verbatim. See the type's docs.
            LoadRefusal::Moved(moved) => write!(f, "{moved}"),
        }
    }
}

/// Write a run to a slot, through `vellum-save`'s store and nothing else.
pub fn save_to<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    run: &StoredRun,
) -> Result<(), String> {
    let text = run.to_ron().map_err(|e| e.to_string())?;
    store.write(slot, &text).map_err(|e| e.to_string())
}

/// Read a run back and put it through the version gate before anything is
/// activated.
///
/// The gate runs *here*, before a single component is written, because that
/// ordering is the whole reason it exists: restoring first and refusing second
/// would mean a host had already half-adopted a world it is about to be told it
/// cannot have.
pub fn load_from<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    current: &Versions,
) -> Result<StoredRun, LoadRefusal> {
    let text = store
        .read(slot)
        .map_err(|e| LoadRefusal::Unreadable(e.to_string()))?
        .ok_or(LoadRefusal::Empty)?;
    let run = StoredRun::from_ron(&text).map_err(|e| LoadRefusal::Unparsable(e.to_string()))?;
    run.versions.check(current).map_err(LoadRefusal::Moved)?;
    Ok(run)
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// Something the capture named that the bootstrapped world did not have.
///
/// Reported rather than skipped. A restore that quietly drops a ship produces a
/// world that looks right and diverges for a reason nothing in the save points
/// at, which is strictly worse than a restore that says what it could not do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreGap {
    /// A captured `EntityUuid` no entity in the target world carries.
    MissingEntity(String),
    /// A captured `AsteroidUuid` no asteroid in the target world carries.
    MissingAsteroid(String),
    /// The captured `SimRngState` has a different number of streams than this
    /// build declares — a save from before a stream was added. Refused rather
    /// than mapped by position, which would hand one call site another's
    /// sequence.
    RngStreamsMoved,
    /// The bootstrapped world's trigger table is a different length than the
    /// capture's, so the capture's per-trigger rows cannot be trusted to name
    /// the same triggers.
    ///
    /// Reported rather than written in by position, and the distinction is the
    /// whole reason an index is usable as a key here at all: the table is
    /// rebuilt by a deterministic replay of the same load — the declarative
    /// states in `the order `init_world_runtime` builds them in
    /// appended by `merge_script_triggers` in compile order — so two runs of the
    /// same content produce the same table. A table of a *different* length is
    /// therefore not a table whose rows shifted; it is a world that loaded a
    /// different layer set, and writing fired-state into it by position would
    /// arm and disarm triggers at random.
    ScenarioTriggersMoved { saved: usize, found: usize },
    /// The capture was waiting on scripted `after(n, |ctx| …)` callbacks and the
    /// bootstrapped world has no `WorldScriptRuntime` to queue them on — a save
    /// from a scripted world being restored into one whose scripts did not
    /// compile. The content dimension is what should have refused this; the gap
    /// is here so it is never silent if it does not.
    ScriptRuntimeAbsent { pending_callbacks: usize },
    /// The capture was holding scripted comms state — queued `open_comms`
    /// requests, or live scripted dialogues — and the bootstrapped world has no
    /// `WorldScriptRuntime` to run them against.
    ///
    /// [`Self::ScriptRuntimeAbsent`]'s comms twin and reported separately
    /// because the loss is different: a dropped callback is deferred work that
    /// never happens, while this is a conversation the player is *in* that
    /// cannot be answered. The content dimension is what should have refused the
    /// save; this is here so it is never silent if it does not.
    CommsScriptRuntimeAbsent {
        pending_opens: usize,
        scripted_dialogues: usize,
    },
}

impl std::fmt::Display for RestoreGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreGap::MissingEntity(uuid) => {
                write!(f, "the world has no entity `{uuid}` to restore into")
            }
            RestoreGap::MissingAsteroid(uuid) => {
                write!(f, "the world has no asteroid `{uuid}` to restore into")
            }
            RestoreGap::RngStreamsMoved => f.write_str(
                "this save's generator streams do not match this build's; \
                 mapping them by position would misroute a call site's sequence",
            ),
            RestoreGap::ScenarioTriggersMoved { saved, found } => write!(
                f,
                "this save records {saved} scenario trigger(s) and the world has \
                 {found}; writing fired state in by position would arm and \
                 disarm the wrong triggers"
            ),
            RestoreGap::ScriptRuntimeAbsent { pending_callbacks } => write!(
                f,
                "this save is waiting on {pending_callbacks} scripted callback(s) \
                 and the world compiled no scripts to run them"
            ),
            RestoreGap::CommsScriptRuntimeAbsent {
                pending_opens,
                scripted_dialogues,
            } => write!(
                f,
                "this save holds {pending_opens} queued comms open(s) and \
                 {scripted_dialogues} scripted dialogue(s) and the world \
                 compiled no scripts to answer them"
            ),
        }
    }
}

/// What a restore actually managed to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Captured entity rows that found a home — bootstrapped or built.
    pub entities_restored: usize,
    /// How many of those this restore had to **build** because the bootstrapped
    /// world had no such entity (issue #863).
    ///
    /// Counted separately because the two are different claims about the same
    /// number: a restore that matched everything is a resume of a run the fresh
    /// app could have replayed, and a restore that built ships is a resume of one
    /// it could not. A host that wants to know whether the save is carrying the
    /// run or merely correcting it reads this.
    pub entities_spawned: usize,
    pub asteroids_restored: usize,
    /// Entities the bootstrap spawned that the capture did not have. Despawned
    /// — a resumed world must not carry a ship the save never saw.
    pub despawned: usize,
    pub gaps: Vec<RestoreGap>,
}

impl RestoreReport {
    /// Whether every captured row found a home. A clean restore is the only
    /// one whose digest can be expected to match the capture's.
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

/// Write a captured payload back over a bootstrapped world.
///
/// The world handed in must already be the *same scenario*, freshly built and
/// run far enough that its authored entities exist — see the module docs on why
/// this is an overwrite and not a spawn. `Run::scenario` and `Run::seed` are
/// what tell a host which world that is.
pub fn restore(world: &mut World, snapshot: &PhoenixSnapshot) -> RestoreReport {
    let mut report = RestoreReport::default();

    restore_entities(world, snapshot, &mut report);
    restore_asteroids(world, snapshot, &mut report);
    restore_run_scope(world, snapshot, &mut report);
    rebuild_power_modifiers(world);
    rebuild_ai_world_snapshot(world);

    report
}

/// Rebuild each ship's `ShipModifiers` from the reactor allocation this restore
/// just wrote, for [`rebuild_ai_world_snapshot`]'s reason: the modifier cache is
/// a per-tick DERIVATION of power (issue #952 — WEAPONS buys `PhaserDamage`,
/// SHIELDS buys `ShieldRegen`, HELM buys `MaxSpeed`/`MaxYawRate`), and a fresh
/// app's bootstrap left it at the seeded rest values while [`PowerState`]
/// restored the allocation those modifiers derive from. Without this, the first
/// tick after the restore integrates beam damage and shield regen at the wrong
/// intensity for exactly one tick — invisible to `world_digest`, which leaves
/// shield charge deferred from the fold, and caught instead by the shield-charge
/// continuation in `tests/snapshot_resume.rs`, which reads `ShipShields` off the
/// live and resumed worlds directly and parts within a frame if this cache is
/// left at its seeded default. The system is a pure
/// function of restored power, so running it once here settles the cache before
/// the first tick reads it; the per-tick `translate_power_modifiers` recomputes
/// the identical values from then on. Errors are swallowed for the same
/// partial-world reason as the snapshot rebuild below.
fn rebuild_power_modifiers(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    let _ = world.run_system_once(crate::modifiers::coordination::translate_power_modifiers);
}

/// Rebuild the AI's `WorldSnapshot` from the world this restore just wrote.
///
/// The one derivation a restore has to force, and the reason is the *cadence*
/// rather than the data. `build_world_snapshot` runs under
/// `run_if(ai_snapshot_ready)`, a latch that is a pure function of `SimTick`
/// (issue #895's anchor) — and [`restore_run_scope`] has just moved `SimTick`
/// to the capture's. So the resumed world's next arm lands on the same tick the
/// live world's does, which is right, but every tick *between* the restore and
/// that arm is spent steering from whatever snapshot the bootstrap happened to
/// leave behind — for a fresh app stopped the moment its roster appeared, an
/// empty one.
///
/// The measured consequence, and the last one this slice's continuation test
/// found: the resumed ships' radar-gated world view held no contacts at all, so
/// `seed_helm_travel_facts` resolved no target, `HelmRecoveryHistory` cleared
/// itself on the target switch, and both machines fell out of `torpedo_run` and
/// `inbound` into `acquire` on the first AI tick after a restore whose digest
/// had matched exactly.
///
/// Rebuilt rather than *stored*: the snapshot is a pure function of the world
/// at its tick, and after this restore the world already stands at that tick.
/// Putting a derivation in the save would be storing an answer the payload can
/// recompute — and one that a later build might compute differently.
fn rebuild_ai_world_snapshot(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    // Errors are swallowed on purpose: a bare-`App` fixture with the AI plugin
    // absent has no `WorldSnapshot` to rebuild, and that is the same
    // "partial world, partial restore" contract `capture` keeps.
    let _ = world.run_system_once(crate::ai::server::build_world_snapshot);
}

/// The run-scope resources, written last so the entity walks above cannot see a
/// half-updated tick.
fn restore_run_scope(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    world.insert_resource(SimTick(snapshot.tick));

    if let Some(state) = snapshot.rng.clone() {
        match SimRng::from_state(state) {
            Some(rng) => world.insert_resource(rng),
            None => report.gaps.push(RestoreGap::RngStreamsMoved),
        }
    }

    if let Some(state) = snapshot.mint.clone() {
        world.insert_resource(WorldIdMint::from_state(state));
    }

    let mut restored_phase_entry: Option<GamePhase> = None;
    if let Some(phase) = snapshot.phase.clone() {
        // `State::new` rather than `NextState`: a queued transition applies on
        // the next `StateTransition`, which is a step this restore has not run
        // yet and must not depend on. The captured phase is where the world
        // already *is*.
        //
        // But `State::new` also skips `OnEnter`/`OnExit` entirely (issue #934),
        // and for a phase actually changing under this restore that silently
        // drops whatever that phase's entry effects do. Note the transition
        // here — before the direct write below — and let
        // `run_restored_phase_entry_effects` decide, once the rest of this
        // function has finished writing the resources those effects read.
        let previous = world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone());
        if previous.as_ref() != Some(&phase) {
            restored_phase_entry = Some(phase.clone());
        }
        world.insert_resource(State::new(phase));
    }

    if let Some((reason, outcome)) = snapshot.game_over.clone() {
        let outcome = outcome
            .as_deref()
            .and_then(|label| crate::core::balance::Outcome::parse(label).ok());
        world.insert_resource(GameOverReason(reason, outcome));
    }

    if !snapshot.captain_boosts.is_empty() {
        // `toggle` on an empty store inserts; there is no bulk setter and this
        // needs none — `CaptainPriorityBoost::default()` is empty by
        // construction, so one toggle per pair reproduces the map exactly.
        let mut boosts = CaptainPriorityBoost::default();
        for (scope, objective) in &snapshot.captain_boosts {
            boosts.toggle(scope, objective);
        }
        world.insert_resource(boosts);
    }

    if let Some(data) = snapshot.world.clone() {
        world.insert_resource(WorldResource(data));
    }

    if let Some(flags) = snapshot.flags.clone() {
        if let Some(mut runtime) = world.get_resource_mut::<WorldContentRuntime>() {
            runtime.flags = flags;
        }
    }

    if let Some(secs) = snapshot.ai_policy_clock {
        world.insert_resource(crate::ship::helm_ai::AiPolicyTickClock(secs));
    }

    if !snapshot.layer_flags.is_empty() {
        if let Some(mut layers) = world.get_resource_mut::<crate::world::server::WorldLayerMap>() {
            for layer in &snapshot.layer_flags {
                if let Some(runtime) = layers.0.get_mut(&layer.path) {
                    runtime.flags = layer.flags.clone();
                }
            }
        }
    }

    restore_scenario(world, snapshot, report);
    restore_comms(world, snapshot, report);

    restore_collisions(world, snapshot);

    // Last, now that every resource an entry effect might read (`GameOverReason`
    // above included) carries the restored value.
    if let Some(phase) = restored_phase_entry {
        run_restored_phase_entry_effects(world, phase);
    }
}

/// Put the scenario's progression back — see [`ScenarioState`].
///
/// Every write here is a wholesale replacement, [`apply_weapons`]' rule and for
/// its reason: the fresh app ran its *own* opening on the way to the restore
/// point, so merging would leave the resumed scenario holding a firing the
/// capture never made.
///
/// # The handlers realign, and nothing here realigns them
///
/// `WorldScriptRuntime::handlers` is the vec parallel to `trigger_states` that
/// says which script fn supplies a scripted trigger's effects. It is **not** in
/// the payload and must not be: it holds `(script_path, fn_name)` pairs that
/// only mean anything against the retained ASTs, and ASTs are not serialisable
/// at all. It does not need to be, either. `compile_world_scripts` runs at
/// `Startup` on the bootstrapped world and `init_world_runtime` calls
/// `merge_script_triggers` immediately after, which builds `handlers` beside the
/// table it fills: one `Some` per compiled `ScriptTrigger`, appended in compile
/// order — the same two deterministic walks (`merge_script_triggers` over the
/// compiled script set, then the load's registration order) that produced the
/// captured table. So the resumed world's index *i* names the same trigger and
/// the same handler the capture's did, before this function writes a single byte,
/// and the only thing left to check is that the two tables are the same length —
/// which is [`RestoreGap::ScenarioTriggersMoved`].
///
/// A world that had a scripted LAYER loaded at capture (issue #1045) rebuilds the
/// same way and for the same reason: the layer's own `[script]` set compiles at
/// `LoadWorld` and appends through the same `merge_script_triggers`, so a resumed
/// world that re-runs the same layer loads reaches the same table in the same
/// order. Until it has, the length check is what refuses the mismatch — the same
/// answer [`ScenarioState::triggers`] already gives for per-layer state.
///
/// # The per-tick budget is reset, not restored
///
/// `WorldScriptRuntime::budget` is a per-tick circuit breaker and a capture is
/// taken at a tick boundary, with that tick's script work already complete — so
/// there is no partially-spent budget to preserve, and the live world's next
/// tick starts a fresh one. What IS written back is `budget_tick`, re-based to
/// the captured tick, and that is not cosmetic. Both script systems reset the
/// budget by comparing `budget_tick` against the *current* `SimTick`; the fresh
/// app's `budget_tick` is whatever tick its own bootstrap reached, and if that
/// number happened to equal the resumed world's next tick — a coincidence a
/// world whose fresh app runs hundreds of ticks can absolutely produce — the
/// resumed world would skip the reset and run its first continuation tick with
/// the bootstrap's operations already charged against it while the live world
/// ran with an empty budget. Re-basing makes the reset decision a function of
/// the captured state rather than of how far the fresh app happened to get.
fn restore_scenario(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(stored) = snapshot.scenario.as_ref() else {
        return;
    };
    let now = fixed_elapsed_secs(world);

    if let Some(mut runtime) = world.get_resource_mut::<WorldContentRuntime>() {
        // The anchor that reproduces the captured mission elapsed against THIS
        // app's clock. Left alone when either side has no clock reading to work
        // from, rather than guessed at.
        runtime.mission_clock_anchor_secs = match (stored.mission_elapsed_secs, now) {
            (Some(elapsed), Some(now)) => Some(now - elapsed),
            (Some(_), None) => runtime.mission_clock_anchor_secs,
            (None, _) => None,
        };

        if stored.triggers.len() == runtime.trigger_states.len() {
            for row in &stored.triggers {
                let Some(state) = runtime.trigger_states.get_mut(row.index as usize) else {
                    continue;
                };
                state.fired = row.fired;
                state.seen_destroyed = row.seen_destroyed.iter().cloned().collect();
                state.last_fired_elapsed = row.last_fired_elapsed;
            }
        } else {
            report.gaps.push(RestoreGap::ScenarioTriggersMoved {
                saved: stored.triggers.len(),
                found: runtime.trigger_states.len(),
            });
        }

        runtime.entity_groups = stored
            .entity_groups
            .iter()
            .map(|(group, members)| (group.clone(), members.iter().cloned().collect()))
            .collect();
        // Wholesale replacement (issue #863), on this walk's rule and with the
        // sharper reason `restore_entities` gives: the bootstrap's map names the
        // ships the bootstrap spawned, and the surplus sweep has just despawned
        // whichever of those the capture did not have. Merging would leave the
        // resumed scenario resolving a name to an entity that no longer exists,
        // which is how an `on_destroyed` handler waits forever for a ship that
        // has already been taken off the board.
        //
        // Empty in the payload means empty here, not "leave what you found":
        // this is the map a capture *had*, and a run with no named entities is a
        // real state a resumed world must be able to stand in.
        runtime.name_to_uuid = stored
            .name_to_uuid
            .iter()
            .map(|(name, uuid)| (name.clone(), uuid.clone()))
            .collect();
        runtime.observed_hull_fractions = stored
            .observed_hull_fractions
            .iter()
            .map(|(uuid, fraction)| (uuid.clone(), *fraction))
            .collect();
        runtime.pending_world_events = stored
            .pending_events
            .iter()
            .filter_map(world_event_from_record)
            .collect();
        // Wholesale replacement, this module's rule throughout: the fresh app ran
        // its own `arm_mission_deadlines` on the way to the restore point, so
        // merging would leave the resumed world holding the bootstrap's due ticks
        // beside the capture's. Taking the table whole also carries its `armed`
        // latch, which is what stops the arming system re-arming over the top.
        runtime.deadlines = stored.deadlines.clone();
        // Wholesale replacement for the same rule (issue #1029), and here it is not
        // even a choice: a promise is only ever written by a script call, so the
        // fresh app's ledger is empty on the way to the restore point and merging
        // would be indistinguishable from replacing. Taking it whole is what stays
        // correct once #849's continuation log replays commands INTO a restored run.
        runtime.commitments = stored.commitments.clone();
        // Wholesale replacement again (issue #1031), on the ledger's terms and
        // with one extra reason of its own: the store deduplicates on
        // `(subject, provenance, text)` and keeps the FIRST tick, so a merge that
        // let the fresh app's own appends land first would re-stamp a finding at
        // the bootstrap's tick and quietly rewrite when the crew learned it.
        runtime.evidence = stored.evidence.clone();
        // And the workforce register (issue #1035), wholesale for the deadline
        // table's rule. The `armed` latch travels with it, which is what stops
        // `arm_mission_workforces` running on the resumed mission's first tick
        // and putting a settled strike back on.
        runtime.workforce = stored.workforce.clone();
    }

    match world.get_resource_mut::<WorldScriptRuntime>() {
        Some(mut script) => {
            script.pending_callbacks = PendingCallbacks(stored.script_callbacks.clone());
            script.budget = TickBudget::new();
            script.budget_tick = snapshot.tick;
        }
        None if !stored.script_callbacks.is_empty() => {
            report.gaps.push(RestoreGap::ScriptRuntimeAbsent {
                pending_callbacks: stored.script_callbacks.len(),
            });
        }
        None => {}
    }
}

/// Put the conversation back — see [`CommsState`].
///
/// Wholesale replacement throughout, [`restore_scenario`]'s rule and for its
/// reason: the fresh app ran its own opening on the way to the restore point, so
/// merging would leave the resumed world holding a thread the capture never
/// opened. The inbox is rebuilt from empty rather than injected into, which is
/// the same statement made about a container `CommsInbox` has no bulk setter for
/// — `inject` skips duplicate ids, so injecting the captured rows in order into
/// a *cleared* inbox reproduces the record vec exactly.
///
/// # The scripted names resolve against the recompiled set
///
/// A [`DialogueState::script`] carries `(script_path, node_fn, on_pick[])` and
/// nothing else — no `AST`, no handle — so answering a restored dialogue means
/// looking `script_path` up in the *bootstrapped* world's
/// `WorldScriptRuntime::asts` and calling the named fn. What makes that the same
/// tree the capture was reading is issue #864's content binding, not anything
/// here: the load returns the compiled set's `content_hash` as a ledger record
/// for its caller to apply (issue #1241), `content_digest` folds it, and
/// `Versions::check` refuses a
/// save whose scripts moved before `restore` is ever reached. Editing a single
/// `on_pick` body therefore refuses the save rather than resolving the name
/// against a different fn. Underneath that, `enter_node` still refuses an
/// unresolvable name visibly (`EnterError::Unresolved` → the control flashes
/// red), and `validate_on_pick_fns` still lints the authored names at load.
fn restore_comms(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(stored) = snapshot.comms.as_ref() else {
        return;
    };

    if let Some(mut inbox) = world.get_resource_mut::<CommsInboxRes>() {
        inbox.0 = crate::console::comms::inbox::CommsInbox::new();
        for message in &stored.inbox {
            inbox.0.inject(message.clone());
        }
        // A restore always owes its clients a push, whatever the inbox's own
        // dirty flag happened to be at the capture.
        inbox.0.mark_dirty();
    }

    if let Some(mut comms) = world.get_resource_mut::<CommsRuntime>() {
        comms.active_dialogues = stored
            .dialogues
            .iter()
            .map(|row| {
                (
                    row.message_id.clone(),
                    ActiveDialogue {
                        // The exact inverse of `reduce_dialogue_node` — see
                        // [`CommsState`] for why that is a copy rather than a
                        // reduction.
                        current_node: CommsDialogueNode {
                            body: row.body.clone(),
                            body_params: row.body_params.clone(),
                            responses: row
                                .responses
                                .iter()
                                .map(|(text, important)| CommsResponse {
                                    text: text.clone(),
                                    important: *important,
                                })
                                .collect(),
                        },
                        thread_id: row.thread_id.clone(),
                        script: row.script.clone(),
                    },
                )
            })
            .collect();

        comms.open_hails = stored.open_hails.iter().cloned().collect();
        // `range_flags` / `range_active` / `contacts` are all recomputed by
        // `update_comms_range_flags` on the next tick; what they need is for the
        // resumed world to push a fresh `CommsState` to its clients.
        comms.needs_broadcast = true;
    }

    let scripted_dialogues = stored.dialogues.len();
    match world.get_resource_mut::<WorldScriptRuntime>() {
        Some(mut script) => {
            script.pending_comms_opens = stored.pending_opens.clone();
        }
        // The dialogues are counted here too: a thread with no runtime behind it
        // is answerable by nothing, which is a loss the caller has to hear about
        // even when no open was queued.
        None if !stored.pending_opens.is_empty() || scripted_dialogues > 0 => {
            report.gaps.push(RestoreGap::CommsScriptRuntimeAbsent {
                pending_opens: stored.pending_opens.len(),
                scripted_dialogues,
            });
        }
        None => {}
    }
}

/// Re-run the observable entry effects of a restored phase transition that the
/// direct `State::new` write above skips (issue #934).
///
/// Not a blanket "run every restored phase's `OnEnter`" — that is wrong for
/// `InProgress` specifically. `ready_to_restore` (below) gates every restore on
/// the fresh app's own roster already existing, which only happens after that
/// app ran its *own* `OnEnter(InProgress)` for its own game start — the mint,
/// the spawns, the command-log reset. Re-running that schedule here would
/// re-spawn and re-reset exactly what the entity/asteroid restore above just
/// wrote. So `InProgress` gets nothing, on purpose. `Lobby` has no `OnEnter`
/// registered at all. `Loading` does (`broadcast_loading_start`), but it is a
/// transient phase a resumed run has no business landing in — capture refuses
/// to record it (see the guard where `capture` is called) — so there is
/// nothing to re-enter here either.
///
/// `GameOver` is the case the issue was filed for: a fresh app still
/// `InProgress` restoring a captured `GameOver` needs `on_game_over_enter`
/// (`server_app.rs`) and `push_game_over_hud_state`
/// (`server/viewscreen_border.rs`) to run, or the host never emits the
/// `GameOver` message and the HUD never leaves its live state. Both are
/// audited safe to re-run: they only *read* `GameOverReason` (restored above,
/// before this call) and *write* an outbox message / HUD resource — neither
/// spawns, despawns, or resets anything the rest of this restore depends on.
/// Run via `OnEnter(GameOver)` itself rather than by naming the two systems,
/// so a future addition to that schedule is covered by construction — but that
/// also means a system landing in `OnEnter(GameOver)` later needs this same
/// audit before it can be trusted here.
fn run_restored_phase_entry_effects(world: &mut World, phase: GamePhase) {
    if phase == GamePhase::GameOver {
        let _ = world.try_run_schedule(OnEnter(GamePhase::GameOver));
    }
}

fn restore_collisions(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(mut telemetry) = world.get_resource_mut::<RunTelemetry>() else {
        return;
    };
    // Every non-collision event the bootstrap produced goes too. The resumed
    // run's telemetry is the capture's, not the capture's plus whatever the
    // bootstrap happened to generate on its way to the restore point.
    telemetry.balance_events.clear();
    for record in &snapshot.collisions {
        telemetry.balance_events.push(StampedBalanceEvent {
            tick: record.tick,
            sim_t: record.sim_t,
            event: BalanceEvent::DamageApplied {
                attacker: None,
                victim: record.victim.clone(),
                victim_kind: if record.victim_is_asteroid {
                    VictimKind::Asteroid
                } else {
                    VictimKind::Ship
                },
                weapon: WEAPON_KIND_COLLISION.to_string(),
                amount: record.amount,
                shield_absorbed: record.shield_absorbed,
                hull_damage: record.hull_damage,
                system_hit: None,
            },
        });
    }
}

/// Overwrite each captured entity's state, despawning anything the capture did
/// not have.
fn restore_entities(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(mut query) = world.try_query::<(Entity, &EntityUuid)>() else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut writes: Vec<(Entity, EntityState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.entities.iter().find(|row| &row.uuid == uuid) {
            Some(row) => writes.push((*entity, row.clone())),
            None => surplus.push(*entity),
        }
    }

    // Issue #863. A row with nothing to write into is either a ship the
    // bootstrap will never make — a mid-run spawn — or a genuine gap, and the
    // difference is whether the capture recorded what the spawn was made from.
    // Built here rather than waited for: waiting is what a fresh app booted with
    // nobody at the consoles does forever.
    for row in &snapshot.entities {
        if present.iter().any(|(_, uuid)| uuid == &row.uuid) {
            continue;
        }
        match row
            .spawn
            .as_ref()
            .and_then(|origin| spawn_from_origin(world, row, origin))
        {
            Some(entity) => {
                report.entities_spawned += 1;
                writes.push((entity, row.clone()));
            }
            None => report
                .gaps
                .push(RestoreGap::MissingEntity(row.uuid.clone())),
        }
    }

    // Both kinds together: `entities_restored` is "captured rows that found a
    // home", and a row that found a home this restore *built* is as restored as
    // one that found a bootstrapped ship waiting.
    report.entities_restored = writes.len();

    for (entity, row) in writes {
        let mut entity_mut = world.entity_mut(entity);
        if let Some(p) = row.physics {
            if let Some(mut physics) = entity_mut.get_mut::<ShipPhysics>() {
                physics.x = p[0];
                physics.y = p[1];
                physics.z = p[2];
                physics.yaw = p[3];
                physics.forward_speed = p[4];
                physics.roll = p[5];
                physics.lateral_speed = p[6];
                physics.vertical_speed = p[7];
            }
            // The renderer and the physics solver both read `Transform`, so a
            // restored ship that only moved its `ShipPhysics` would sit in one
            // place and be drawn in another until the next helm integration.
            //
            // The ROTATION is written for a sharper reason than drawing:
            // `build_helm_ai_surfaces_frame` reads a target's facing straight
            // off `Transform::rotation`, not off its `ShipPhysics`. A resumed
            // world that restored only the translation therefore had every ship
            // steering against a target whose heading was the *bootstrap's* —
            // and a Harrow inbound on a bearing of pi was read as facing 0.
            //
            // Derived here rather than stored, and by the same expression
            // `physics_systems::apply_ship_physics` uses, so the two can only
            // ever agree: the transform is a projection of `ShipPhysics`, and a
            // save that stored it separately could contradict the thing it is a
            // projection of.
            if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
                transform.translation = Vec3::new(p[0], p[1], p[2]);
                transform.rotation = Quat::from_euler(bevy::math::EulerRot::YXZ, -p[3], 0.0, p[5]);
            }
        }
        if let Some(rows) = &row.hull {
            if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
                apply_hull(&mut hull.0, rows);
            }
        }
        if let Some(active) = row.red_alert {
            if let Some(mut alert) = entity_mut.get_mut::<ShipRedAlert>() {
                alert.0 = active;
            }
        }
        // Issue #1041. Restored beside the alert, because the two are one
        // firing posture and a resumed ship that had been ordered to hold fire
        // must come back holding it.
        if let Some(held) = row.weapons_hold {
            if let Some(mut hold) = entity_mut.get_mut::<ShipWeaponsHold>() {
                hold.0 = held;
            }
        }
        // Issues #1107–#1109. The per-ship Command stance map IS folded into the
        // sim digest, so a resume that dropped it stood at a different digest
        // than the capture the instant any stance was in force. Overwrite the
        // authoritative map from the row — an empty row clears it, byte-identical
        // to a never-commanded hull. Re-wrapped into `StationId` from the sorted
        // scalar pairs the capture stored.
        if let Some(mut stances) =
            entity_mut.get_mut::<crate::console::command::server::ShipStationStances>()
        {
            stances.0 = row
                .station_stances
                .iter()
                .map(|(station, stance)| {
                    (
                        crate::core::messages::StationId(station.clone()),
                        stance.clone(),
                    )
                })
                .collect();
        }
        // Reseed the #1108 edge scratch (`LastDirectedControl`) from the restored
        // control sources. It is NOT folded into the digest (it is a pure
        // function of already-authoritative control state, classified `derived`),
        // so this does not move the digest-at-restore assertion — but the scratch
        // is what makes the Human→AI stance-resume trigger fire on an EDGE rather
        // than every tick, and a restored map is empty. An empty scratch treats
        // the first post-restore tick as a first OBSERVATION and fires nothing;
        // a continuous host would have carried `Some(prev)` and could fire the
        // edge that tick. Recording the directed Station's CURRENT
        // `station_is_ai_controlled` result makes the first tick a continuation
        // of the state the resumed world actually restored into, not a spurious
        // first-observation no-op. The captured session-level human/AI split on
        // the target is deliberately NOT recoverable — control sources are
        // derived from who is at a console, which the snapshot excludes — so the
        // honest reseed is the restored world's own current reading. See the
        // WARNING on `command_plugin::LastDirectedControl`.
        let reseed = {
            let config = entity_mut.get::<crate::ship_plugin::ShipConfigComponent>();
            let sources = entity_mut.get::<crate::ship_plugin::ShipSystemControlSources>();
            match (config, sources) {
                (Some(config), Some(sources)) => {
                    crate::console::command::server::command_station(&config.0)
                        .and_then(|command| command.command_target.clone())
                        .map(|target| {
                            let now_ai = crate::console::command::server::station_is_ai_controlled(
                                &config.0, &sources.0, &target,
                            );
                            (target, now_ai)
                        })
                }
                _ => None,
            }
        };
        if let Some((target, now_ai)) = reseed {
            if let Some(mut last) =
                entity_mut.get_mut::<crate::console::command::server::LastDirectedControl>()
            {
                last.0.clear();
                last.0.insert(target, now_ai);
            }
        }
        if let Some(control) = &row.control {
            if let Some(mut thrust) = entity_mut.get_mut::<ThrustInput>() {
                thrust.0 = control.thrust;
            }
            if let Some(mut steering) = entity_mut.get_mut::<SteeringInput>() {
                steering.0 = control.steering;
            }
            if let Some(mut lateral) = entity_mut.get_mut::<LateralThrustInput>() {
                lateral.0 = control.lateral;
            }
            if let Some(mut vertical) = entity_mut.get_mut::<VerticalThrustInput>() {
                vertical.0 = control.vertical;
            }
            if let Some(mut boost) = entity_mut.get_mut::<BoostCommand>() {
                boost.0 = control.boost;
            }
            if let Some(mut impulse) = entity_mut.get_mut::<ImpulseCommand>() {
                impulse.0 = match control.impulse_phase {
                    1 => ImpulsePhase::Charging,
                    2 => ImpulsePhase::Active,
                    // Including anything this build does not recognise — see
                    // `ControlState::impulse_phase`.
                    _ => ImpulsePhase::Idle,
                };
            }
            if let Some(mut last) = entity_mut.get_mut::<LastHelmInput>() {
                last.thrust = control.last_helm[0];
                last.steering = control.last_helm[1];
                last.lateral = control.last_helm[2];
            }
            if let Some(policies) = &control.helm_policies {
                if let Some(mut state) = entity_mut.get_mut::<HelmEnginesAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[0]);
                }
                if let Some(mut state) = entity_mut.get_mut::<HelmSteeringAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[1]);
                }
                if let Some(mut state) = entity_mut.get_mut::<HelmBoostAiPolicyState>() {
                    apply_policy_state(&mut state.0, &policies[2]);
                }
            }
            if let Some(stored) = &control.helm_recovery {
                if let Some(mut history) =
                    entity_mut.get_mut::<crate::ship::helm_ai::HelmRecoveryHistory>()
                {
                    history.target = stored
                        .target
                        .as_deref()
                        .and_then(|t| uuid::Uuid::parse_str(t).ok());
                    history.ranges.set_capacity(stored.ranges_capacity as usize);
                    history.ranges.clear();
                    for sample in &stored.ranges {
                        history.ranges.push(*sample);
                    }
                    history
                        .separation
                        .set_capacity(stored.separation_capacity as usize);
                    history.separation.clear();
                    for sample in &stored.separation {
                        history.separation.push(*sample);
                    }
                }
            }
            if let Some(mut lock) = entity_mut.get_mut::<TacticalRadarSelection>() {
                lock.0 = control.target_lock.clone();
            }
            if let Some(mut sensor_lock) =
                entity_mut.get_mut::<crate::ship::sensors::SensorRadarSelection>()
            {
                sensor_lock.0 = control.sensor_lock.clone();
            }
            if let Some(mut attacker) = entity_mut.get_mut::<LastShipAttacker>() {
                // `set_if_neq` semantics matter here: `LastShipAttacker`'s
                // change detection is the rising-edge latch behind
                // `on_entity_attacked` triggers, so a blind write on restore
                // would re-fire a scenario trigger the capture had already
                // spent.
                let restored = control.last_attacker.clone();
                if attacker.0 != restored {
                    attacker.0 = restored;
                }
            }
        }
        if let Some(weapons) = &row.weapons {
            apply_weapons(&mut entity_mut, weapons);
        }
        if let Some(repair) = &row.repair {
            apply_repair(&mut entity_mut, repair);
        }
        if let Some(arc) = &row.arc_request {
            apply_arc_request(&mut entity_mut, arc);
        }
        if let Some(power) = &row.power {
            if let Some(mut reactor) = entity_mut.get_mut::<crate::ship::power::ShipPowerSystem>() {
                let allocations: Vec<(crate::core::messages::PowerGroupId, u8)> = power
                    .allocations
                    .iter()
                    .map(|(id, level)| (crate::core::messages::PowerGroupId(id.clone()), *level))
                    .collect();
                reactor
                    .0
                    .restore(&allocations, power.battery_charge, power.locked);
            }
        }
        if let Some(surface) = row.pass_surface {
            if let Some(mut pass) = entity_mut.get_mut::<crate::ship::helm_ai::HelmPassSurface>() {
                *pass = surface;
            }
        }
        if let Some(infrastructure) = &row.infrastructure {
            if let Some(mut condition) =
                entity_mut.get_mut::<crate::infrastructure::InfrastructureCondition>()
            {
                condition.0 = infrastructure.clone();
            }
        }
        if let Some(tractor) = &row.tractor {
            if let Some(mut beam) = entity_mut.get_mut::<crate::tractor::TractorBeam>() {
                beam.restore(tractor);
            }
        }
        if let Some(dock) = &row.dock {
            if let Some(mut control) = entity_mut.get_mut::<crate::dock::DockControl>() {
                control.restore(dock);
            }
        }
        if let Some(external_repair) = &row.external_repair {
            if let Some(mut dispatch) =
                entity_mut.get_mut::<crate::console::repair::ExternalRepairDispatch>()
            {
                dispatch.restore(external_repair);
            }
        }
        if let Some(umbilical) = &row.umbilical {
            if let Some(mut control) = entity_mut.get_mut::<crate::umbilical::TransferUmbilical>() {
                control.restore(umbilical);
            }
        }
        if let Some(scan) = &row.scan {
            if let Some(mut record) = entity_mut.get_mut::<crate::science::ShipScanRecord>() {
                record.restore(scan);
            }
        }
        if let Some(civilian) = &row.civilian {
            if let Some(mut traffic) = entity_mut.get_mut::<crate::civilian::CivilianTraffic>() {
                traffic.0 = civilian.clone();
            }
        }
        if !row.patrol_cursors.is_empty() {
            if let Some(mut cursors) = entity_mut.get_mut::<crate::ai::server::ObjectiveCursors>() {
                cursors.0 = row
                    .patrol_cursors
                    .iter()
                    .map(|(id, index, settled)| {
                        crate::ai::patrol_cursor::PatrolCursor::restored(
                            id.clone(),
                            *index as usize,
                            *settled,
                        )
                    })
                    .collect();
            }
        }
        if !row.blackboards.is_empty() {
            if let Some(mut boards) =
                entity_mut.get_mut::<crate::server_app::ShipSystemBlackboards>()
            {
                boards.0 = row
                    .blackboards
                    .iter()
                    .map(|(id, board)| (SystemId(id.clone()), board.clone()))
                    .collect();
            }
        }
    }

    report.despawned += surplus.len();
    for entity in surplus {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }
}

/// Rebuild one mid-run spawn the bootstrapped world does not have (issue #863).
///
/// The same three steps the live spawn took, through the same two functions, so
/// a rebuilt ship and a streamed-in one cannot come out different: resolve the
/// template and overrides ([`SpawnOrigin::resolve`]), build the entity
/// ([`crate::entities::spawner::spawn_entity`]), then re-declare the layer
/// ownership and the placement the applier's own arm declares.
///
/// `None` — and therefore a [`RestoreGap::MissingEntity`] — when the template
/// does not resolve. That is the honest answer for the same reason
/// [`restore_asteroids`] gives it for a rock with no config path: an entity
/// built without its template is one with an invented collider and no weapons,
/// and a resumed world carrying one is worse than a resumed world that says what
/// it could not rebuild.
///
/// # Why the fidelity tier is read off the row
///
/// The helm axes are not spawn components — they arrive with
/// `ai_high_fidelity_components()` when the LOD system promotes a ship inside the
/// player's bubble — so a freshly built ship has none of them and every
/// [`ControlState`] write below would land nowhere. That is the exact silent
/// failure `ready_to_restore` was written to prevent, and the capture already
/// answers it: a row carries `control` if and only if the ship it was taken from
/// was high-fidelity. So the tier is *read*, not guessed. It is also
/// self-correcting either way — `update_ai_lod` re-derives the tier from range on
/// its next tick, with its own dwell timer — whereas a dropped write is not.
fn spawn_from_origin(
    world: &mut World,
    row: &EntityState,
    origin: &crate::world::spawn_origin::SpawnOrigin,
) -> Option<Entity> {
    let mut warnings = Vec::new();
    let config = origin.resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings);
    for warning in &warnings {
        bevy::log::warn!(
            target: crate::logging::LogCat::World.target(),
            "snapshot restore: {warning}"
        );
    }
    let config = config?;

    let position = Vec3::new(origin.position[0], origin.position[1], origin.position[2]);
    let entity = {
        let mut commands = world.commands();
        crate::entities::spawner::spawn_entity(
            &mut commands,
            &config,
            position,
            row.uuid.clone(),
            None,
        )
    };
    // `spawn_entity` builds through `Commands`, so the components are queued
    // rather than present. Everything below — and every state write in
    // `restore_entities` afterwards — reads the entity directly, so the queue
    // has to be applied before any of it runs.
    world.flush();

    let mut entity_mut = world.entity_mut(entity);
    // The applier's own placement step, on its terms: `spawn_entity` set the
    // translation, and a rotation or scale replaces the whole `Transform`
    // through the canonical `TransformConfig` conversions rather than a second
    // Euler expression.
    if origin.rotation.is_some() || origin.scale.is_some() {
        let transform_config = crate::world::config::TransformConfig {
            rotation: origin.rotation,
            scale: origin.scale,
            ..Default::default()
        };
        entity_mut.insert(Transform {
            translation: position,
            rotation: transform_config.quat(),
            scale: transform_config.scale_vec(),
        });
    }
    // Put the record back on the ship it describes, so a save taken *after* this
    // resume can rebuild it again. A resumed run that could only be resumed once
    // is a continuation with an expiry date.
    entity_mut.insert(crate::entities::spawner::EntitySpawnOrigin(origin.clone()));
    if row.control.is_some() {
        entity_mut.insert(crate::ai::server::ai_high_fidelity_components());
    }

    if let Some(path) = &origin.layer_path {
        // Layer ownership is what makes `UnloadWorld` despawn a layer's ad-hoc
        // spawns. Declared only when the target world actually has that layer
        // loaded — the applier's own guard — because pushing a handle into a
        // layer the resumed world never loaded would record an ownership nothing
        // will ever act on.
        let owned = world
            .get_resource_mut::<crate::world::server::WorldLayerMap>()
            .and_then(|mut layers| {
                layers.0.get_mut(path).map(|layer| {
                    layer.spawned_entities.push(entity);
                })
            })
            .is_some();
        if owned {
            world
                .entity_mut(entity)
                .insert(crate::world::server::EntityOriginLayer(path.clone()));
        }
    }

    Some(entity)
}

/// Put a ship's weapon state machines back mid-cycle.
///
/// Every write here is a wholesale replacement rather than a merge, and that is
/// the point: a fresh app's bootstrap ran its own tubes and its own beams on
/// the way to the restore point, and merging would leave the resumed ship
/// carrying a shot the capture never fired.
fn apply_weapons(entity: &mut EntityWorldMut<'_>, stored: &WeaponState) {
    if let Some(mut beam) = entity.get_mut::<ActiveBeam>() {
        beam.restore_live_banks(stored.beams.iter().map(
            |(bank, target, remaining, accumulator)| {
                (
                    bank.clone(),
                    ActiveBeamSlot {
                        target_uuid: target.clone(),
                        remaining_secs: *remaining,
                        damage_accumulator: *accumulator,
                    },
                )
            },
        ));
    }
    if let Some(mut cooldown) = entity.get_mut::<PhaserCooldown>() {
        cooldown.restore_banks(stored.phaser_cooldowns.iter().cloned());
    }
    if let Some(mut arcs) = entity.get_mut::<EntityShipArcHull>() {
        for (id, current, _max) in &stored.arc_hull {
            arcs.0.set_hp(id, *current);
        }
    }
    if let Some(mut shields) = entity.get_mut::<crate::ship::shields::ShipShields>() {
        // Charge, not structural hull (that is `arc_hull` above): overwrite the
        // per-facing hp / fractional accumulator / offline timer / focus, then
        // let the system re-derive its focus-dependent fields so the restored
        // system is field-identical to the captured one (issue #997 follow-up).
        // Verified directly off `ShipShields` in `snapshot_resume`, since the
        // digest deliberately defers shield charge from the fold. An arc the
        // save omits is left as the bootstrap built it.
        if !stored.shield_charge.is_empty() {
            shields.0.restore_facings(&stored.shield_charge);
        }
    }
    if let Some(mut torpedoes) = entity.get_mut::<TorpedoSystemResource>() {
        let system = &mut torpedoes.0;
        if let Some(remaining) = stored.torpedoes_remaining {
            system.torpedoes_remaining = remaining;
        }
        for tube in system.tubes.iter_mut() {
            let Some(row) = stored.tubes.iter().find(|t| t.id == tube.id) else {
                // A tube the save does not mention is left alone rather than
                // emptied, for `apply_hull`'s reason: an unmentioned tube is a
                // save written against a different hull, and the content digest
                // is what refuses that.
                continue;
            };
            tube.load_state = match row.load_phase {
                1 => TubeLoadState::Loading {
                    remaining: row.load_timer[0],
                    total: row.load_timer[1],
                },
                2 => TubeLoadState::Loaded,
                3 => TubeLoadState::Unloading {
                    remaining: row.load_timer[0],
                    total: row.load_timer[1],
                },
                // Including anything this build does not recognise — see
                // `TubeState::load_phase`.
                _ => TubeLoadState::Unloaded,
            };
            tube.loaded_count = row.loaded_count;
            tube.target_count = row.target_count;
            tube.active_barrels = row.active_barrels.clone();
            tube.pattern_step = row.pattern_step;
        }
        system.in_flight = stored
            .torpedoes_in_flight
            .iter()
            .map(|t| Torpedo {
                uuid: t.uuid.clone(),
                x: t.position[0],
                y: t.position[1],
                z: t.position[2],
                heading: t.heading,
                pitch: t.pitch,
                lifespan_remaining: t.lifespan_remaining,
                target_uuid: t.target_uuid.clone(),
                source_uuid: t.source_uuid.clone(),
                tube_id: t.tube_id.clone(),
                shield_pierce: t.shield_pierce,
            })
            .collect();
        system.burst_states = stored
            .bursts
            .iter()
            .map(|b| TubeBurstState {
                tube_id: b.tube_id.clone(),
                pending: b.pending,
                timer: b.timer,
                launch_x: b.launch[0],
                launch_y: b.launch[1],
                launch_z: b.launch[2],
                launch_heading: b.launch_heading,
                target_uuid: b.target_uuid.clone(),
                source_uuid: b.source_uuid.clone(),
                barrel_origins: b
                    .barrel_origins
                    .iter()
                    .map(|o| (o[0], o[1], o[2]))
                    .collect(),
                barrel_sequence: b.barrel_sequence.clone(),
                next_shot_index: b.next_shot_index,
            })
            .collect();
    }
    if let Some(mut blasters) =
        entity.get_mut::<crate::console::weapons::blaster::BlasterSystemResource>()
    {
        // Joined by authored bank order, the order the component keeps and the
        // capture walked. A bank the save does not reach (fewer stored than the
        // hull carries) is left alone, `apply_hull`'s rule: an unmentioned bank
        // is a save written against a different hull, which the content digest
        // refuses.
        for (bank, row) in blasters.0.iter_mut().zip(stored.blasters.iter()) {
            let v = &mut bank.volley;
            v.pending_volley = row.pending_volley;
            v.schedule = row.schedule.clone();
            v.next_step = row.next_step as usize;
            v.volley_elapsed = row.volley_elapsed;
            v.active_barrels = row.active_barrels.clone();
            v.current_step = row.current_step;
            v.on_cooldown = row.on_cooldown;
            v.cooldown_remaining = row.cooldown_remaining;
            v.charging = row.charging;
            v.charge_elapsed = row.charge_elapsed;
            bank.in_flight = row
                .in_flight
                .iter()
                .map(|p| crate::weapons::blaster::BlasterProjectile {
                    id: p.id.clone(),
                    x: p.x,
                    z: p.z,
                    heading: p.heading,
                    speed: p.speed,
                    lifespan_remaining: p.lifespan_remaining,
                    collision_radius: p.collision_radius,
                    damage: p.damage,
                    shield_pierce: p.shield_pierce,
                    source_uuid: p.source_uuid.clone(),
                })
                .collect();
        }
    }
}

/// Put a ship's repair crew back where it was standing.
fn apply_repair(entity: &mut EntityWorldMut<'_>, stored: &RepairState) {
    if let Some(mut teams) = entity.get_mut::<ShipRepairTeams>() {
        teams.0.restore_slots(&stored.teams);
    }
    if let Some(mut queue) = entity.get_mut::<RepairRequestQueue>() {
        queue.entries = stored
            .queue
            .iter()
            .map(
                |(station_id, station_label, tier, deficit)| RepairQueueEntry {
                    station_id: station_id.clone(),
                    station_label: station_label.clone(),
                    tier: *tier,
                    deficit: *deficit,
                },
            )
            .collect();
    }
    if let Some(mut alerted) = entity.get_mut::<RepairHumanAlerted>() {
        alerted.0 = stored.alerted.iter().cloned().collect();
    }
}

/// Put the Weapons→Helm arc-bearing seam back — see [`ArcRequestState`].
///
/// Both halves are wholesale replacements, [`apply_weapons`]' rule: a settled
/// debounce is restored so the first cadence tick after the restore does not
/// re-fire a request the capture had already spent, and the pending bearing is
/// restored so Helm resumes folding the same bias it was folding at the capture
/// rather than steering as though no request were outstanding.
fn apply_arc_request(entity: &mut EntityWorldMut<'_>, stored: &ArcRequestState) {
    if let Some(mut weapons_state) =
        entity.get_mut::<crate::console::weapons::WeaponsArcRequestState>()
    {
        weapons_state.last = stored
            .last
            .as_ref()
            .map(|(family, target, arcs)| (*family, target.clone(), arcs.clone()));
    }
    if let Some(mut pending) = entity.get_mut::<crate::ship_plugin::PendingArcBearingRequest>() {
        pending.target = stored
            .pending_target
            .as_deref()
            .and_then(|t| uuid::Uuid::parse_str(t).ok());
        pending.arcs = stored.pending_arcs.clone();
    }
}

/// Overwrite the streamed belt: keep the rocks the capture knows, despawn the
/// ones it does not, and **spawn** the ones the target world never streamed.
///
/// The spawn half is what makes a restore authoritative over the belt rather
/// than merely corrective, and it is what Combat Test needs. A capture taken
/// after the player has flown somewhere names rocks whose cells the fresh app —
/// bootstrapped at the spawn point and stepped only far enough to raise its
/// roster — has never had in window. Reporting those as
/// [`RestoreGap::MissingAsteroid`]s and carrying on would leave the resumed
/// world short of exactly the rocks the capture's digest counted, so the digest
/// would not match and nothing in the save would say why.
///
/// A rock is spawned through `asteroid_lifecycle::rock_bundle` — the same
/// component set the streamer itself builds — so a restored rock and a streamed
/// one are the same entity, down to the `ColliderSection` that collision
/// avoidance reads an obstacle's size from.
fn restore_asteroids(world: &mut World, snapshot: &PhoenixSnapshot, report: &mut RestoreReport) {
    let Some(mut query) = world.try_query::<(Entity, &AsteroidUuid)>() else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut writes: Vec<(Entity, AsteroidState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.asteroids.iter().find(|row| &row.uuid == uuid) {
            Some(row) => writes.push((*entity, row.clone())),
            None => surplus.push((*entity, uuid.clone())),
        }
    }
    let missing: Vec<AsteroidState> = snapshot
        .asteroids
        .iter()
        .filter(|row| !present.iter().any(|(_, uuid)| uuid == &row.uuid))
        .cloned()
        .collect();

    report.asteroids_restored = writes.len();

    for (entity, row) in writes {
        let mut entity_mut = world.entity_mut(entity);
        if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
            transform.translation =
                Vec3::new(row.translation[0], row.translation[1], row.translation[2]);
            // Not re-normalised. The stored quaternion came off a live
            // `Transform` and is already unit; normalising it again is a
            // divide that moves the low bits, and the capture this restore is
            // checked against folds bit patterns.
            transform.rotation = Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            );
        }
        if let Some(rows) = &row.hull {
            if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
                apply_hull(&mut hull.0, rows);
            }
        }
    }

    for (entity, uuid) in &surplus {
        if let Ok(entity_mut) = world.get_entity_mut(*entity) {
            entity_mut.despawn();
        }
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.remove(uuid);
        }
    }
    report.despawned += surplus.len();

    for row in &missing {
        // Without a config path there is nothing to build the rock *from* — no
        // collider, no hull maximum, no mesh — so this stays the honest gap it
        // always was rather than becoming a rock with invented dimensions. In
        // practice it means a hand-placed rock the target world does not have,
        // which is a scenario difference the content digest is the answer to.
        let Some(config_path) = row.config_path.as_deref() else {
            report
                .gaps
                .push(RestoreGap::MissingAsteroid(row.uuid.clone()));
            continue;
        };
        let config = crate::asteroids::lifecycle::rock_config(config_path);
        let current_hp = row
            .hull
            .as_ref()
            .and_then(|rows| rows.first().map(|(_, current, _)| *current))
            .unwrap_or(config.max_hp);
        let mut spawned = world.spawn(crate::asteroids::lifecycle::rock_bundle(
            &row.uuid,
            &config,
            Vec3::new(row.translation[0], row.translation[1], row.translation[2]),
            Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            ),
            row.shield_pierce,
            current_hp,
        ));
        if let Some(mesh) = &config.mesh {
            spawned.insert(crate::entities::spawner::MeshSection(mesh.clone()));
        }
        let entity = spawned.id();
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.insert(row.uuid.clone(), entity);
        }
        report.asteroids_restored += 1;
    }

    restore_asteroid_window(world, snapshot);
}

/// Put the streamer's own progress back, so its next tick resumes rather than
/// rebuilding. See [`AsteroidWindowState`].
fn restore_asteroid_window(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return;
    };
    // Cosmetic handles belong to the app that spawned them, and the arena the
    // restore is about to install may not be the one their slots were indexed
    // against. They carry no uuid, no hull and no collider, so despawning them
    // costs a frame of set dressing and buys a window whose every remaining
    // handle is one this restore put there.
    let cosmetics: Vec<Entity> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .cosmetic_upper_slots
                .iter()
                .chain(window.cosmetic_lower_slots.iter())
                .flatten()
                .flatten()
                .copied()
                .collect()
        })
        .unwrap_or_default();
    for entity in cosmetics {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }

    let Some(mut window) = world.get_resource_mut::<AsteroidWindow>() else {
        return;
    };
    let size = (2 * stored.despawn_cells + 1) as usize;
    window.slots = vec![vec![None; size]; size];
    window.cosmetic_upper_slots = vec![vec![None; size]; size];
    window.cosmetic_lower_slots = vec![vec![None; size]; size];
    for slot in &stored.slots {
        let (x, z) = (slot.x as usize, slot.z as usize);
        let Some(cell) = window.slots.get_mut(z).and_then(|row| row.get_mut(x)) else {
            continue;
        };
        *cell = Some(AsteroidData {
            uuid: slot.uuid.clone(),
            config_path: slot.config_path.clone(),
            hp: slot.hp,
            max_hp: slot.max_hp,
            y: slot.y,
        });
    }
    window.arena_gx = stored.arena_gx;
    window.arena_gz = stored.arena_gz;
    window.despawn_cells = stored.despawn_cells;
    window.spawn_cells = stored.spawn_cells;
    window.resolution = stored.resolution;
    window.player_grid = stored.player_grid;
    window.composition_key = stored.composition_key;
    window.needs_init = stored.needs_init;
}

/// Whether a bootstrapped world is far enough along to be restored into.
///
/// A fresh app does not have the scenario's ships at tick 0 — the lobby's
/// collective auto-start has to run first, and the world spawns on the phase
/// transition. So both callers that restore (the browser boot path and the
/// integration test) step the fresh app until this is true and only then
/// overwrite. It is the same question [`restore`] would otherwise answer too
/// late, as a list of [`RestoreGap::MissingEntity`]s.
///
/// # It waits for the whole roster; [`ready_to_rebuild`] is the other half
///
/// This predicate is unchanged by issue #863 and deliberately so: a bootstrapped
/// ship is a *better* ship than one [`restore`] builds, because the bootstrap ran
/// the world's own systems over it — faction registration, AI token, LOD
/// promotion and its dwell timer, the power seed — and the payload does not
/// cover every one of those. The duel's 120-frame continuation claim is measured
/// against a fully bootstrapped roster and is the standing proof: restoring
/// earlier, onto hulls this module had built instead, parts the two worlds one
/// frame after a matching digest.
///
/// So the order of preference is: wait for the bootstrap, and build only what
/// never arrives. [`ready_to_rebuild`] is the second half of that sentence, and
/// the caller's own deadline is what decides when to ask it.
pub fn ready_to_restore(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(mut query) = world.try_query::<(&EntityUuid, Option<&ThrustInput>)>() else {
        return false;
    };
    // Both halves matter. A ship's `EntityUuid` appears at spawn, but its helm
    // axes are inserted a beat later, and a restore that fired in that window
    // found no `ThrustInput` to write to and silently left the ship coasting —
    // a world whose digest matched the capture exactly and diverged one tick
    // afterwards. Waiting for the controls is what closes it.
    let present: Vec<(&str, bool)> = query
        .iter(world)
        .map(|(uuid, thrust)| (uuid.0.as_str(), thrust.is_some()))
        .collect();
    let roster_ready = snapshot.entities.iter().all(|row| {
        present.iter().any(|(uuid, has_controls)| {
            *uuid == row.uuid && (*has_controls || row.control.is_none())
        })
    });
    roster_ready && belt_ready(world, snapshot)
}

/// Whether a world that has stopped becoming [`ready_to_restore`] can be
/// restored into *anyway*, by building what the bootstrap never produced (issue
/// #863).
///
/// # Why this is a second predicate and not a looser first one
///
/// [`ready_to_restore`] answers "is the bootstrap done?" and a caller loops on it
/// until a deadline. This answers the question that deadline used to end the
/// session on: **is what is still missing something this build can make?**
///
/// The two have to stay apart because a mid-run spawn is genuinely ambiguous
/// while the wait is still running. `duel` spawns its whole NPC roster through
/// script effects at t=0, and a fresh app produces every one of those ships
/// within a frame or two — so a predicate that treated "spawned" as "do not
/// wait" would build ten hulls it was about to be handed, which is measurably
/// worse (see [`ready_to_restore`]). `probe_reinforce` spawns two at t=3 s, and a
/// fresh app driven by nobody produces them never. Nothing in the payload
/// distinguishes those two futures, so the only honest discriminator is *time*:
/// wait, and if the ships have still not arrived, build them.
///
/// True when every captured row is either standing with its controls — the
/// ordinary case, and the same rule as above — or carries a
/// [`SpawnOrigin`](crate::world::spawn_origin::SpawnOrigin) that
/// [`restore`] can build it from. A row that is standing *without* its controls
/// makes this false as well: building does not help a ship that is already there,
/// so a caller in that state should keep waiting or give up, not restore over a
/// half-built hull.
pub fn ready_to_rebuild(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(mut query) = world.try_query::<(&EntityUuid, Option<&ThrustInput>)>() else {
        return false;
    };
    let present: Vec<(&str, bool)> = query
        .iter(world)
        .map(|(uuid, thrust)| (uuid.0.as_str(), thrust.is_some()))
        .collect();
    let roster_rebuildable = snapshot.entities.iter().all(|row| {
        match present.iter().find(|(uuid, _)| *uuid == row.uuid) {
            Some((_, has_controls)) => *has_controls || row.control.is_none(),
            None => row.spawn.is_some(),
        }
    });
    roster_rebuildable && belt_ready(world, snapshot)
}

/// Whether the target world's asteroid streamer has settled onto the same
/// composition the capture was taken against.
///
/// [`restore_asteroids`] is authoritative over the *rocks* — it spawns the ones
/// the capture names and despawns the ones it does not — but it cannot be
/// authoritative over a field entity that has not loaded yet.
/// `update_asteroid_window` recomputes its composition key from the live
/// `AsteroidFieldSection`s every tick, and a key that disagrees with the
/// window's is its signal that a world layer loaded or unloaded a field, which
/// it answers by clearing the window wholesale. Restoring into a world whose
/// fields were still arriving would therefore be undone by the very next tick:
/// the belt wiped, the digest silently short of every rock the capture counted.
///
/// So a capture whose streamer had settled waits for one whose streamer has
/// too. A capture with no streamed field at all — the duel arena — waits for
/// nothing, because there is nothing to disagree about.
fn belt_ready(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return true;
    };
    if stored.needs_init {
        return true;
    }
    world
        .get_resource::<AsteroidWindow>()
        .is_some_and(|live| !live.needs_init && live.composition_key == stored.composition_key)
}

// ── Verification ─────────────────────────────────────────────────────────────

/// The `Sampling` simulation [`vellum_save::verify`] checks a restore against.
///
/// Deliberately tiny, and deliberately not `headless::replay::PhoenixSim`: that
/// type is native-only (it lives under the `headless` feature) and a browser
/// host is precisely the thing that has to be told its save will not load. This
/// adapter compiles on both targets because it does nothing but hold a world
/// and hash it.
///
/// # `apply` refuses, and that is not a stub
///
/// This issue's artifact has an **empty log** by construction — a snapshot with
/// no commands is a saved game, which is the whole of what #862 stores — so
/// `replay_into` never reaches `apply`. Refusing rather than pretending is the
/// honest encoding of that: if a log ever does arrive here, it arrived from
/// #849's continuation work, and the right answer is a named refusal rather
/// than a silent no-op that would make an unreplayed command look replayed.
/// When #849 lands, the verifier for a run *with* a log is `PhoenixSim`, which
/// already crosses the production admission boundary.
pub struct SavedGame<'a> {
    world: &'a World,
    ledger: Ledger,
}

/// Why [`SavedGame`] will not replay a command. See its docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoContinuationLog;

impl std::fmt::Display for NoContinuationLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a saved game carries no command log to replay (issue #849 adds one)")
    }
}

impl<'a> SavedGame<'a> {
    /// Wrap a restored world for verification.
    ///
    /// The ledger is empty on purpose. `verify` reads this side's ledger only
    /// to look for a *sampled* disagreement, and a saved game samples nothing;
    /// the numbers it actually compares — the capture digest and the final
    /// digest — both come from `run`, checked against `digest()` recomputed
    /// live. Handing the recorded digest in here would make the check confirm
    /// itself.
    pub fn new(world: &'a World) -> SavedGame<'a> {
        SavedGame {
            world,
            ledger: Ledger::default(),
        }
    }
}

impl vellum_replay::Simulation for SavedGame<'_> {
    type Command = LoggedCommand;
    type Rejection = NoContinuationLog;

    fn apply(&mut self, _command: &LoggedCommand) -> Result<(), NoContinuationLog> {
        Err(NoContinuationLog)
    }

    fn is_over(&self) -> bool {
        false
    }

    fn digest(&self) -> u64 {
        crate::sim_digest::world_digest(self.world)
    }
}

impl vellum_save::Sampling for SavedGame<'_> {
    fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// A restored world already stands at the capture tick, so there is no tail
    /// to run out. Stepping here would need `&mut World` and a schedule, and
    /// would be running the simulation *inside* a verification — which is the
    /// one thing a check of "did the restore land?" must not do.
    fn advance_to(&mut self, _tick: u64) {}
}

/// Write captured per-system HP onto a hull the fresh world built from config.
///
/// `set_hp` rather than replacing the whole `SystemHull`: the tier
/// thresholds, display names and insertion order are authored config, and the
/// bootstrapped hull already has them right. A system the capture does not
/// mention is left alone rather than zeroed — an unmentioned system is a save
/// written against a different hull, which the content digest is what refuses.
fn apply_hull(hull: &mut crate::ship::damage::SystemHull, rows: &[(String, f32, f32)]) {
    for (id, current, _max) in rows {
        hull.set_hp(&SystemId(id.clone()), *current);
    }
}
