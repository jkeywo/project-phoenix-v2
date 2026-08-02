//! The ordered, tick-stamped record of every command that crossed the host's
//! network boundary (issue #898, PRD #849).
//!
//! # What goes in the log, and why that is the whole decision
//!
//! Issue #898 exists because the parent PRD described a log carrying "human and
//! AI commands both stamped for a future tick", and that cannot be built as
//! written: AI decisions never cross a network boundary. They are emitted
//! mid-tick, in-process, by [`super::ai_emit::emit_ai_command`] straight into
//! the target ship's `AdmittedCommands`.
//!
//! **The decision is Option A: the log records the network boundary.** Nothing
//! that a replaying instance can re-derive for itself is written down. Concretely:
//!
//! - Every command [`super::admit_system_commands`] accepts is recorded, in the
//!   order it was accepted, stamped with the logical tick it applies on.
//! - Every command an AI decider emits through `emit_ai_command` is *absent*,
//!   and keeps its same-tick guarantee untouched.
//!
//! The contract that makes the omission safe: replaying the recorded log through
//! the deterministic simulation from the same seed regenerates identical AI
//! behaviour, so recording AI decisions as well would double-count them — the
//! replay would apply each AI order once from the log and once from the decider
//! that re-derived it. This is what makes AI determinism load-bearing, which is
//! why #895 put the AI cadence on the logical tick and why the RNG had to be
//! seeded per call site (#897) first.
//!
//! The recorder does **not** ask whether a command came from a human or an AI.
//! It records at a *seam*, not by origin — which is the only reading compatible
//! with AGENTS.md constraint 6 ("never branch on human-vs-AI"). In production
//! the only writer of `InboundMessage` is the JS bridge's `drain_inbound`, so
//! that seam is exactly the network boundary and in practice its traffic is
//! human. If #854 later has a peer send an NPC's orders over the wire, those are
//! logged too — correctly, because a remote peer's decisions are not something
//! this instance re-derives.
//!
//! # Consequence for AGENTS.md constraint 7
//!
//! Constraint 7 says "helm commands apply the tick they are admitted". Option A
//! leaves that **intact and unamended**. A logged command carries the tick it
//! applies on explicitly, and [`CommandDelay`] — the lockstep input delay — is
//! `0` for a local host, so the apply tick *is* the admission tick and the
//! command lands in `AdmittedCommands` in the same run of the same system, in
//! the same order, as it did before this module existed.
//!
//! What changes is that the tick is now written down rather than implied, and
//! that admission consumes from an ordered queue keyed on it. A non-zero
//! `CommandDelay` — which only P2P lockstep (#854) has a reason to set, once
//! there is a second peer to negotiate one with — is therefore the *deliberate*
//! amendment point for constraint 7, not an accident waiting in the plumbing.
//!
//! # The session token never enters the log
//!
//! An [`AdmittedCommand`] carries a `response_token` — the sender's session
//! token — so a reply can be addressed back to whoever asked. That token is a
//! **bearer credential**: AGENTS.md constraint 2 makes the UUIDv4 in a client's
//! `localStorage` the whole of its identity, so anything holding the string can
//! impersonate that player. The log's destinations are exactly the two places
//! such a string must not go: a save file on disk, and a peer over the wire.
//!
//! So a [`LoggedCommand`] is not an `AdmittedCommand` with a tick bolted on. It
//! is the *non-secret projection* of one: the tick, the target system, the
//! payload, and a [`ShipKey`] — the routed ship's
//! [`crate::entity_spawner::EntityUuid`], which is already the vocabulary
//! snapshots, balance events and damage ledgers name ships in, and which is
//! derived from the seeded simulation rather than from a client. The raw token
//! stays on the in-process `AdmittedCommand` in [`PendingCommands`] and in the
//! ship's `AdmittedCommands`, and goes no further.
//!
//! That is enough for a replay, because routing is a *destination*, not an
//! *identity*: [`super::admit_system_commands`] resolves a token to one ship's
//! `AdmittedCommands`, and the [`ShipKey`] names that ship directly. What the
//! log deliberately cannot do is re-run the authority check — a replay applies
//! commands an authority check already accepted, which is the same reason
//! refusals are absent (`vellum_replay`'s third rule).
//!
//! # Ordering
//!
//! The log **is** the order. Entries are keyed on `(apply tick, arrival)`:
//!
//! - *apply tick* is `SimTick` at admission plus [`CommandDelay`], so a command
//!   stamped for a future tick sorts after everything already due.
//! - *arrival* is a monotonic counter over the whole run, assigned in the order
//!   [`super::admit_system_commands`] reads `InboundMessage`s — which is the
//!   order the bridge decoded them from the wire.
//!
//! [`PendingCommands`] is a `BTreeMap` on that key, so draining it is ordered by
//! construction rather than by a sort that could be made unstable. Recording and
//! queueing happen in one step ([`stamp_accepted_command`]) precisely so the two
//! orders cannot drift.
//!
//! # What is deliberately not here
//!
//! - **Driving the whole simulation from a log.** That is #901. This module
//!   lands the log, its tick semantics, and [`CommandLogReplay`] — the type
//!   #901 drives — and no more.
//! - **A logged debug-command class.** `bridge.rs`'s god-mode thread-local is
//!   read inline at four sites and is #900's to convert. Nothing here precludes
//!   it: a debug command that reaches `AdmittedCommands` through admission is
//!   recorded like any other, because the recorder does not inspect origin.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::entity_spawner::EntityUuid;
use crate::messages::{AdmittedCommand, SystemControlPayload, SystemId};

/// Which ship's `AdmittedCommands` a logged command lands in, named by that
/// ship's [`EntityUuid`].
///
/// The log's routing key, and deliberately the *only* identity in it. A uuid is
/// non-secret, deterministic (the seeded simulation mints it), and already the
/// vocabulary a snapshot, a balance event and a damage ledger name ships in, so
/// a replay or a peer resolves it with machinery that already exists. See the
/// module docs for why the sender's session token stays behind.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipKey(pub String);

impl ShipKey {
    /// The key for the ship admission resolved a command to.
    ///
    /// `None` — a ship with no [`EntityUuid`] — yields the empty key. Every
    /// ship production spawns carries one (the generic spawner mints it, and
    /// `spawn_game_start_entities` gives the player ship one too), so this arm
    /// exists for bare-`App` fixtures that spawn a ship out of loose
    /// components. An empty key records the command rather than dropping it:
    /// losing a run's input silently is worse than recording one whose route a
    /// replay cannot resolve, and [`ShipKey::is_named`] is how a consumer tells
    /// the two apart.
    pub fn from_uuid(uuid: Option<&EntityUuid>) -> Self {
        ShipKey(uuid.map(|u| u.0.clone()).unwrap_or_default())
    }

    /// Whether this key names a ship a replay could resolve.
    pub fn is_named(&self) -> bool {
        !self.0.is_empty()
    }
}

/// One accepted command, as the log records it: the logical tick it applies on,
/// the ship it applies to, and what it asks for.
///
/// The non-secret projection of an [`AdmittedCommand`] — same target, same
/// payload, but the sender's session token replaced by a [`ShipKey`]. The
/// module docs say why: this type is written to saves and sent to peers, and a
/// session token is a bearer credential.
///
/// The tick is carried explicitly rather than inferred from position, because
/// the two are only the same thing while [`CommandDelay`] is zero. A peer
/// receiving this entry has to know which tick to apply it on without knowing
/// what delay the sender ran with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoggedCommand {
    /// The `SimTick` value this command applies on.
    pub tick: u64,
    /// The ship whose `AdmittedCommands` this lands in, as admission's routing
    /// rule resolved it when the command arrived.
    pub ship: ShipKey,
    /// The system the command addresses.
    pub target: SystemId,
    /// What it asks that system to do.
    pub payload: SystemControlPayload,
}

/// The run's ordered command log: everything that crossed the network boundary
/// and was accepted, in apply order.
///
/// Serialisable for the same two reasons `SimRngState` is: #901 replays it, and
/// the snapshot boundary (#862) stores it. Together with the master seed it is
/// the whole of a run's input — the pair is what "replayable in principle"
/// means here.
///
/// Nothing removes entries. A run's log is its history, and human traffic is
/// sparse enough (a few commands per second at the very most) that unbounded
/// growth is not a live concern at demo length; bounding or flushing it belongs
/// with the snapshot boundary that would define what a truncated log means.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandLog {
    entries: Vec<LoggedCommand>,
}

impl CommandLog {
    /// Append one accepted command.
    ///
    /// Private to the module on purpose: everything that records goes through
    /// [`stamp_accepted_command`], which cannot record without also queueing.
    fn record(&mut self, entry: LoggedCommand) {
        self.entries.push(entry);
    }

    /// Forget everything: a new run starts a new log.
    ///
    /// Called at the run boundary by [`reset_command_log`], never mid-run —
    /// see that function for why a second round must not inherit the first
    /// one's inputs.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The log, in order.
    pub fn entries(&self) -> &[LoggedCommand] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every command that applies on `tick`, in order.
    pub fn for_tick(&self, tick: u64) -> impl Iterator<Item = &LoggedCommand> {
        self.entries.iter().filter(move |e| e.tick == tick)
    }

    /// The tick the last recorded command applies on, if any.
    pub fn last_recorded_tick(&self) -> Option<u64> {
        self.entries.last().map(|e| e.tick)
    }

    /// Whether the recorded ticks never go backwards.
    ///
    /// The cheap smoke check that a log is replayable *in principle*: entries
    /// are consumed in recorded order against a clock that only advances, so a
    /// tick that went backwards would be a command the replay could not apply
    /// on the tick it claims. [`CommandLogReplay`] is the same rule stated as a
    /// `vellum_replay::Simulation`.
    pub fn ticks_are_monotonic(&self) -> bool {
        self.entries.windows(2).all(|w| w[0].tick <= w[1].tick)
    }
}

/// Lockstep input delay, in logical ticks: how far ahead of the tick it is
/// admitted on a command is stamped to apply.
///
/// `0` — the shipped value, and the only correct one for a single host, where
/// there is nobody to wait for. It is a resource rather than a constant because
/// it is the knob P2P lockstep (#854) turns: peers agree a delay so that every
/// instance has every peer's input for tick *T* before it simulates *T*. It is
/// deliberately *not* TOML gameplay data — no designer tunes it, and a wrong
/// value is a desync rather than a balance change.
///
/// See the module docs for what a non-zero value means for AGENTS.md
/// constraint 7.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandDelay(pub u64);

/// One accepted command waiting for the tick it applies on.
#[derive(Clone, Debug)]
pub struct PendingCommand {
    /// The logical tick this applies on.
    pub tick: u64,
    /// Monotonic arrival index — the tiebreak within a tick.
    pub arrival: u64,
    /// The ship whose `AdmittedCommands` this lands in, resolved by admission's
    /// routing rule when the command arrived.
    pub route: Entity,
    pub command: AdmittedCommand,
}

/// Commands accepted at the boundary and not yet due, ordered by
/// `(apply tick, arrival)`.
///
/// With [`CommandDelay`] at zero every command enqueued during a tick drains
/// again inside the same run of [`super::admit_system_commands`], so this is a
/// pass-through today and the observable behaviour is exactly what it was
/// before #898. The queue exists so that it stays a pass-through *by
/// configuration* rather than by construction: a future-stamped command slots
/// in with no redesign of the admission path.
#[derive(Resource, Default, Debug)]
pub struct PendingCommands {
    next_arrival: u64,
    queue: BTreeMap<(u64, u64), PendingCommand>,
}

impl PendingCommands {
    /// Queue one accepted command for `tick`, returning the log entry that
    /// records it.
    ///
    /// Private for the same reason [`CommandLog::record`] is: queueing and
    /// recording are one act, and [`stamp_accepted_command`] is where it
    /// happens.
    fn enqueue(
        &mut self,
        tick: u64,
        route: Entity,
        ship: ShipKey,
        command: AdmittedCommand,
    ) -> LoggedCommand {
        let arrival = self.next_arrival;
        self.next_arrival = self.next_arrival.wrapping_add(1);
        // The log entry is built here, from the command about to be queued, so
        // the projection can never describe a different command from the one
        // that applies. The token stays on the `AdmittedCommand` this queue
        // holds; only the non-secret half is returned for recording.
        let entry = LoggedCommand {
            tick,
            ship,
            target: command.target.clone(),
            payload: command.payload.clone(),
        };
        self.queue.insert(
            (tick, arrival),
            PendingCommand {
                tick,
                arrival,
                route,
                command,
            },
        );
        entry
    }

    /// Drop every queued command and restart the arrival counter.
    ///
    /// The counter restarts because arrival is "the *n*th command of this run",
    /// and [`reset_command_log`] is where a run ends. A second round that kept
    /// counting would tiebreak identical inputs on a different number from the
    /// first, which is exactly the kind of hidden run-to-run state a replay
    /// cannot reproduce.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.next_arrival = 0;
    }

    /// Remove and return everything due at or before `now`, in
    /// `(tick, arrival)` order.
    ///
    /// "At or before" rather than "exactly": a command stamped for a tick that
    /// has somehow already passed is applied late rather than stranded in the
    /// queue forever. That cannot happen on a local host — `SimTick` advances
    /// one step at a time and the stamp is never in the past — but a stranded
    /// command would be a silent, permanent input loss, and a late one is at
    /// least visible in the log it was recorded in.
    pub fn drain_due(&mut self, now: u64) -> Vec<PendingCommand> {
        let later = self.queue.split_off(&(now.saturating_add(1), 0));
        let due = std::mem::replace(&mut self.queue, later);
        due.into_values().collect()
    }

    /// How many commands are waiting for a future tick.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Stamp one accepted command for the tick it applies on: queue it for that
/// tick *and* record it in the log, in one step.
///
/// The two happen together so the log order and the apply order cannot drift —
/// the log is not a commentary on the queue, it is the same sequence written
/// down. There is deliberately no way to record without queueing.
///
/// Only ever called on the accepted branch of admission's authority check, which
/// is what keeps refusals out of the log (`vellum_replay`'s third rule).
///
/// `route` and `ship` are the same destination said twice, in the two
/// vocabularies that need it: the `Entity` the queue delivers to inside this
/// process, and the [`ShipKey`] the log names for anything outside it. They are
/// taken together, here, because this is the one site that has admission's
/// resolved route in hand — deriving the key anywhere else would mean a second
/// copy of the routing rule.
pub fn stamp_accepted_command(
    log: &mut CommandLog,
    pending: &mut PendingCommands,
    apply_tick: u64,
    route: Entity,
    ship: ShipKey,
    command: AdmittedCommand,
) {
    let entry = pending.enqueue(apply_tick, route, ship, command);
    log.record(entry);
}

/// Install the command-log resources. Idempotent (`init_resource` is).
///
/// Not called directly: [`super::register_admission_seam`] is the only caller,
/// so the resources and the system that writes them cannot be registered apart.
pub(super) fn register_command_log(app: &mut App) {
    app.init_resource::<CommandLog>()
        .init_resource::<PendingCommands>()
        .init_resource::<CommandDelay>();
}

/// Start a new run's log: clear the record and the future-tick queue.
///
/// Registered in `OnEnter(GamePhase::InProgress)` by
/// `server_app::add_simulation_plugins_with`, which is the run boundary — both
/// the first game and every later one reached by `ReturnToLobby` from
/// `GameOver` (`lobby::handler::handle_return_to_lobby`).
///
/// Without this a second round inherits round one's log, and the pair "master
/// seed + command log" stops describing *a* run. Note what it does **not**
/// do: `SimTick` counts steps for the life of the app, so round two's stamps
/// carry on upward and the log stays perfectly monotonic. It simply describes
/// two runs at once, and a replay would apply round one's commands to round
/// two's world. There is no check that catches that afterwards, which is why
/// the boundary is drawn here instead.
pub fn reset_command_log(mut log: ResMut<CommandLog>, mut pending: ResMut<PendingCommands>) {
    log.clear();
    pending.clear();
}

// ── The replay contract ───────────────────────────────────────────────────────

/// Why a replayed [`LoggedCommand`] was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayRejection {
    /// The entry is stamped for a tick the replay clock has already left
    /// behind. Recording is monotonic, so this means the log was reordered,
    /// merged wrongly, or written by a build that disagreed about the stamp.
    TickWentBackwards { stamped: u64, clock: u64 },
}

impl std::fmt::Display for ReplayRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayRejection::TickWentBackwards { stamped, clock } => write!(
                f,
                "a command stamped for tick {stamped} arrived after the replay \
                 clock had reached tick {clock} — the log is out of order"
            ),
        }
    }
}

/// The log's own ordering contract, stated as a [`vellum_replay::Simulation`].
///
/// **This is not the phoenix simulation.** Driving the real world from a log is
/// #901's scope, and it is a much larger thing: it needs the whole app, the
/// seed, and the snapshot boundary. What this models is the one invariant #898
/// owns — commands arrive stamped with the tick they apply on, they apply in
/// `(tick, arrival)` order, and a stamp that goes backwards is refused rather
/// than applied late.
///
/// Modelling that much is what lets `vellum_replay::contract::check_all` say
/// something true about phoenix rather than about a toy counter: it exercises
/// the actual [`LoggedCommand`] type, and it proves the two rules that are easy
/// to break by accident — that a refusal leaves the state byte-identical, and
/// that a refused command never reaches the log.
#[derive(Clone, Debug)]
pub struct CommandLogReplay {
    /// The furthest tick applied so far.
    clock: u64,
    applied: usize,
    /// Rolling fingerprint of the accepted sequence.
    digest: u64,
}

impl Default for CommandLogReplay {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandLogReplay {
    pub fn new() -> Self {
        Self {
            clock: 0,
            applied: 0,
            digest: FNV_OFFSET,
        }
    }

    /// The furthest tick this replay has reached.
    pub fn clock(&self) -> u64 {
        self.clock
    }

    /// How many entries have been applied.
    pub fn applied(&self) -> usize {
        self.applied
    }
}

impl vellum_replay::Simulation for CommandLogReplay {
    type Command = LoggedCommand;
    type Rejection = ReplayRejection;

    fn apply(&mut self, command: &LoggedCommand) -> Result<(), ReplayRejection> {
        // Checked before anything moves: a refusal must leave the clock, the
        // count and the digest exactly as they were.
        if command.tick < self.clock {
            return Err(ReplayRejection::TickWentBackwards {
                stamped: command.tick,
                clock: self.clock,
            });
        }
        self.clock = command.tick;
        self.applied += 1;
        self.digest = fold_command(self.digest, command);
        Ok(())
    }

    /// A command log has no ending of its own — it ends when it runs out.
    fn is_over(&self) -> bool {
        false
    }

    fn digest(&self) -> u64 {
        self.digest
            .rotate_left(17)
            .wrapping_add(self.clock)
            .wrapping_mul(31)
            .wrapping_add(self.applied as u64)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Fold one entry into the rolling digest.
///
/// The payload enters through its derived `Debug` rendering, which is a total
/// and deterministic encoding of every variant and field — and, unlike a
/// hand-written match, cannot silently stop covering a variant somebody adds
/// later. This is an ordering fingerprint for comparing two replays of the same
/// log, not a wire format: nothing outside this process reads it, so it is free
/// to change shape whenever the types do.
fn fold_command(seed: u64, entry: &LoggedCommand) -> u64 {
    let mut hash = fnv1a(seed, &entry.tick.to_le_bytes());
    hash = fnv1a(hash, entry.ship.0.as_bytes());
    hash = fnv1a(hash, entry.target.0.as_bytes());
    fnv1a(hash, format!("{:?}", entry.payload).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_replay::Simulation;

    /// The ship every fixture here routes to.
    const SHIP: &str = "uuid-ship-1";

    fn command(target: &str, token: &str) -> AdmittedCommand {
        AdmittedCommand {
            target: SystemId(target.into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
            response_token: Some(token.into()),
        }
    }

    fn entry(tick: u64, target: &str) -> LoggedCommand {
        LoggedCommand {
            tick,
            ship: ShipKey(SHIP.into()),
            target: SystemId(target.into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        }
    }

    fn stamp(log: &mut CommandLog, pending: &mut PendingCommands, tick: u64, target: &str) {
        stamp_accepted_command(
            log,
            pending,
            tick,
            Entity::from_raw_u32(1).unwrap(),
            ShipKey(SHIP.into()),
            command(target, "t1"),
        );
    }

    /// The ordering key is `(tick, arrival)`, not arrival alone: a command
    /// stamped for a later tick waits behind one stamped for an earlier tick
    /// even though it arrived first.
    #[test]
    fn the_queue_drains_in_tick_then_arrival_order() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();

        stamp(&mut log, &mut pending, 7, "late");
        stamp(&mut log, &mut pending, 3, "early");
        stamp(&mut log, &mut pending, 3, "also-3");

        let due = pending.drain_due(3);
        let targets: Vec<&str> = due.iter().map(|p| p.command.target.0.as_str()).collect();
        assert_eq!(
            targets,
            vec!["early", "also-3"],
            "tick 3's commands drain in arrival order and tick 7's stays behind"
        );
        assert_eq!(pending.len(), 1);

        let due = pending.drain_due(7);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].command.target.0, "late");
        assert!(pending.is_empty());
    }

    /// The queue keeps the token so a reply can still be addressed; the log
    /// keeps the ship key instead, and never sees it. This is the split the
    /// whole module exists to make, so it is asserted directly rather than
    /// inferred from the round-trip test below.
    #[test]
    fn the_token_stays_in_the_queue_and_the_log_gets_the_ship_key() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        stamp(&mut log, &mut pending, 0, "helm");

        let due = pending.drain_due(0);
        assert_eq!(
            due[0].command.response_token.as_deref(),
            Some("t1"),
            "the in-process command keeps the token — replies still need it"
        );

        let entry = &log.entries()[0];
        assert_eq!(entry.ship, ShipKey(SHIP.into()));
        assert_eq!(entry.target.0, "helm");
        assert!(
            entry.ship.is_named(),
            "a resolved route must produce a key a replay can look up"
        );
    }

    /// A ship with no `EntityUuid` — the bare-`App` fixture shape — still gets
    /// its command recorded, under a key that says it is unresolvable.
    #[test]
    fn a_ship_with_no_uuid_yields_an_unnamed_key() {
        assert!(!ShipKey::from_uuid(None).is_named());
        let uuid = EntityUuid("abc".into());
        assert_eq!(ShipKey::from_uuid(Some(&uuid)), ShipKey("abc".into()));
    }

    /// Recording and queueing are one act: the log is the drain order.
    #[test]
    fn the_log_records_every_stamp_in_apply_order() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        for (tick, target) in [(0_u64, "a"), (0, "b"), (5, "c")] {
            stamp(&mut log, &mut pending, tick, target);
        }

        let recorded: Vec<(u64, &str)> = log
            .entries()
            .iter()
            .map(|e| (e.tick, e.target.0.as_str()))
            .collect();
        assert_eq!(recorded, vec![(0, "a"), (0, "b"), (5, "c")]);
        assert_eq!(log.for_tick(0).count(), 2);
        assert_eq!(log.last_recorded_tick(), Some(5));
        assert!(log.ticks_are_monotonic());
    }

    /// A log whose stamps go backwards is not replayable, and says so.
    #[test]
    fn a_backwards_stamp_fails_the_monotonic_check() {
        let mut log = CommandLog::default();
        log.record(entry(4, "a"));
        log.record(entry(2, "b"));
        assert!(!log.ticks_are_monotonic());
    }

    /// The whole vellum contract, against the real [`LoggedCommand`] type:
    /// replaying is deterministic, a refusal changes nothing at all, and a
    /// refused command never reaches the log.
    #[test]
    fn the_log_keeps_the_vellum_replay_contract() {
        let script = vec![entry(0, "helm"), entry(0, "shields"), entry(9, "power")];
        // Stamped for a tick the script has already passed — refused, and the
        // only kind of refusal a log replay has.
        let rejected = entry(1, "helm");
        vellum_replay::contract::check_all(CommandLogReplay::new, &script, &rejected);
    }

    /// `Diverged` names the entry that broke the log, which is the whole
    /// diagnostic value of replaying rather than diffing states.
    #[test]
    fn a_reordered_log_names_the_entry_that_broke_it() {
        let mut sim = CommandLogReplay::new();
        let fault =
            vellum_replay::replay_into(&mut sim, &[entry(0, "a"), entry(4, "b"), entry(1, "c")])
                .expect_err("the third entry goes backwards");
        assert_eq!(fault.at_command, 2);
        assert!(
            matches!(
                fault.rejection,
                ReplayRejection::TickWentBackwards {
                    stamped: 1,
                    clock: 4
                }
            ),
            "got {:?}",
            fault.rejection
        );
    }

    /// Two replays of the same log agree; a log with one command changed does
    /// not. Without the second half the digest could be a constant.
    #[test]
    fn the_digest_distinguishes_two_different_logs() {
        let script = vec![entry(0, "helm"), entry(3, "power")];
        let mut first = CommandLogReplay::new();
        vellum_replay::replay_into(&mut first, &script).expect("replays");
        let mut again = CommandLogReplay::new();
        vellum_replay::replay_into(&mut again, &script).expect("replays");
        assert_eq!(first.digest(), again.digest());

        let mut different = CommandLogReplay::new();
        vellum_replay::replay_into(&mut different, &[entry(0, "helm"), entry(3, "shields")])
            .expect("replays");
        assert_ne!(
            first.digest(),
            different.digest(),
            "a different command must produce a different digest, or the \
             contract check above proves nothing"
        );
    }

    /// The log leaves the process the same way `SimRngState` does — RON, the
    /// format the headless side already reads and writes — and what leaves with
    /// it is the ship key, never the session token.
    ///
    /// The negative assertion is the load-bearing one. This is the exact moment
    /// the log becomes a file on disk or a frame on the wire, so it is the
    /// moment a bearer credential in it would escape (AGENTS.md constraint 2).
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_log_round_trips_through_ron_without_the_token() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        let route = Entity::from_raw_u32(1).unwrap();
        stamp_accepted_command(
            &mut log,
            &mut pending,
            0,
            route,
            ShipKey(SHIP.into()),
            command("helm", "session-token-aaaa"),
        );
        stamp_accepted_command(
            &mut log,
            &mut pending,
            12,
            route,
            ShipKey(SHIP.into()),
            command("power", "session-token-bbbb"),
        );

        let text = ron::ser::to_string(&log).expect("the log serialises");
        assert!(
            !text.contains("session-token-"),
            "a session token reached the serialised log — it is a bearer \
             credential and the log's destinations are saves and peers:\n{text}"
        );
        assert!(
            text.contains(SHIP),
            "the ship key is what replaces it, so it has to be there:\n{text}"
        );

        let restored: CommandLog = ron::from_str(&text).expect("and comes back");
        assert_eq!(restored, log);
        assert_eq!(restored.entries()[1].tick, 12);
        assert_eq!(restored.entries()[1].target.0, "power");
    }

    /// The run boundary: a second round starts from an empty log and an empty
    /// queue, arrival counter included.
    #[test]
    fn resetting_starts_a_fresh_run() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        stamp(&mut log, &mut pending, 0, "helm");
        stamp(&mut log, &mut pending, 99, "power");
        assert_eq!(log.len(), 2);
        assert_eq!(pending.drain_due(0).len(), 1, "tick 0's command applies");
        assert_eq!(
            pending.len(),
            1,
            "the tick-99 command is still waiting, which is exactly the state a \
             round boundary must not carry across"
        );

        log.clear();
        pending.clear();
        assert!(log.is_empty());
        assert!(pending.is_empty());
        assert!(
            log.last_recorded_tick().is_none(),
            "a cleared log has no history to answer questions about"
        );

        // Round two's first command is round two's arrival 0: the drain order
        // of identical input must not depend on how much round one saw.
        stamp(&mut log, &mut pending, 0, "shields");
        let due = pending.drain_due(0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].arrival, 0);
    }
}
