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
//! left that intact for a solo host: a logged command carries the tick it
//! applies on explicitly, and [`CommandDelay`] — the lockstep input delay — is
//! `0` when there is nobody to wait for, so the apply tick *is* the admission
//! tick and the command lands in `AdmittedCommands` in the same run of the same
//! system, in the same order, as it did before this module existed.
//!
//! **Issue #1116 made the amendment constraint 7 named in advance.** A host in a
//! fleet runs with a non-zero delay authored as `[global] command_delay_ticks`,
//! so a crew's command applies `delay` ticks after it is admitted — the same
//! tick on every host in the fleet, which is the whole of what lockstep buys.
//! The amendment is a change of *value*, not of plumbing: the tick a command
//! applies on is still written on the command, still the key the queue drains
//! on, and still the tick the log records. AGENTS.md rule 7 carries the amended
//! wording; [`crate::lockstep`] is the only writer of the resource.
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
//! [`crate::entities::spawner::EntityUuid`], which is already the vocabulary
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
//! The log **is** the order. Entries are keyed on `(apply tick, `[`CommandOrder`]`)`:
//!
//! - *apply tick* is `SimTick` at admission plus [`CommandDelay`], so a command
//!   stamped for a future tick sorts after everything already due.
//! - [`CommandOrder`] is `(origin host slot, per-origin sequence)` — the
//!   **peer-independent** tiebreak issue #1116 needs. A single host mints every
//!   order from its own slot with a run-wide monotonic sequence, so it
//!   degenerates exactly to the pre-#1116 arrival counter; a fleet of hosts each
//!   mints from its OWN slot, and every host sorts the merged set the same way
//!   because the key says who issued the command rather than when this
//!   particular receiver happened to decode it.
//!
//! `p2p-delta-command-log-shape` states the rule this satisfies:
//! `must_not_be: [ordered across peers by local arrival index, a second command
//! queue beside PendingCommands]`. The key widened; the queue did not fork.
//!
//! [`PendingCommands`] is a `BTreeMap` on that key, so draining it is ordered by
//! construction rather than by a sort that could be made unstable.
//!
//! ## The log is written when a command APPLIES, not when it is accepted
//!
//! #898 recorded at the moment of acceptance, which was the same moment as the
//! apply while [`CommandDelay`] was zero. Under a fleet delay they are different
//! moments, and acceptance is the wrong one: a host accepts its own crew's
//! commands as its clock reaches each tick and a peer's whenever the frame
//! arrives, so two hosts that apply an identical tick in an identical order
//! would still have *recorded* it in two different orders — and a host running
//! a tick or two behind would record a later stamp before an earlier one, which
//! [`CommandLog::ticks_are_monotonic`] correctly calls unreplayable.
//!
//! So [`PendingCommands::drain_due`] takes the log and writes each entry as it
//! hands the command over. Recording and applying remain **one act**, which is
//! the property #898 introduced [`stamp_accepted_command`] for; it is now the
//! stronger act, because the resulting log is byte-identical on every host in
//! the fleet rather than merely equivalent. A command stamped for a future tick
//! is in the queue and not yet in the log, which is the honest answer: it has
//! not happened yet.
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

use crate::core::messages::{AdmittedCommand, SystemControlPayload, SystemId};
use crate::entities::spawner::EntityUuid;

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

/// Which host in the frozen fleet roster issued a command (issue #1116).
///
/// The fleet owner mints slot ids as `slot-N` (`gui/host-mesh.js`), and the
/// ordinal `N` is what crosses the simulation boundary: it is small, totally
/// ordered, and — because one machine minted every id in one monotonic sequence
/// — it means the same thing on every host. `p2p-delta-identity-is-minted` is
/// the reason the ordinal cannot be self-assigned: "an id is a function of when
/// and in what order a peer minted it".
///
/// A host with no fleet — the single-host case, and every fixture — is
/// [`HostSlot::SOLO`]. That is a real slot rather than an absence, so the
/// ordering key has the same shape whether or not a mesh is running and the
/// solo path is not a second code path.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct HostSlot(pub u32);

impl HostSlot {
    /// The slot a host with no fleet issues from.
    ///
    /// Deliberately `0` and not `1`: `gui/host-mesh.js` numbers fleet slots from
    /// `slot-1`, so a solo host's key can never collide with a fleet member's,
    /// and a log carrying solo entries beside fleet entries (a fleet formed
    /// mid-session would be exactly that) still sorts unambiguously.
    pub const SOLO: HostSlot = HostSlot(0);

    /// Parse the ordinal out of a `slot-N` id from the host-mesh roster.
    ///
    /// `None` for anything that is not one, because a roster id this side
    /// cannot parse is a protocol disagreement rather than a slot to guess at.
    pub fn from_slot_id(id: &str) -> Option<Self> {
        id.strip_prefix("slot-")?.parse().ok().map(HostSlot)
    }

    /// The `slot-N` id this ordinal renders as, for the host-mesh roster.
    pub fn slot_id(&self) -> String {
        format!("slot-{}", self.0)
    }
}

/// The peer-independent tiebreak between commands that apply on the same tick
/// (issue #1116).
///
/// Ordering is the derived field order — origin first, then that origin's own
/// sequence — so the whole fleet's traffic for one tick has exactly one order,
/// and every host computes it from the key alone without knowing what arrived
/// when. The pre-#1116 `arrival` counter was the opposite: a local wire-decode
/// index, which is a fact about the receiver rather than about the command.
///
/// `seq` restarts at zero on the run boundary along with the log
/// ([`reset_command_log`]), on every host, because a run boundary is a tick
/// every host agrees on. Trap T8 of the #1116 brief is exactly this: an
/// ordering key that survived the reset on one host and not another would
/// tiebreak round two differently from round one.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CommandOrder {
    /// The host that admitted this command from its own crew.
    pub origin: HostSlot,
    /// That host's own monotonic counter, restarted at each run boundary.
    pub seq: u64,
}

impl CommandOrder {
    pub fn new(origin: HostSlot, seq: u64) -> Self {
        Self { origin, seq }
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
    /// Where this command sits in the fleet's agreed order for that tick
    /// (issue #1116).
    ///
    /// Carried explicitly for the same reason `tick` is: the log's own `Vec`
    /// order is the applied order *on the host that wrote it*, and a recovering
    /// host (#1118) merging two partial logs has nothing but this key to
    /// reconstruct the order from. It also names the host a divergent command
    /// came from, which is the difference between "the fleet disagreed at tick
    /// 240" and a diagnostic somebody can act on.
    pub order: CommandOrder,
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
    /// Append one applied command.
    ///
    /// Private to the module on purpose: the one recording site is
    /// [`PendingCommands::drain_due`], which cannot record without also
    /// handing the command to the ship it applies on.
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
/// `0` for a host with nobody to wait for, which is still the default and still
/// the only correct value for a solo run.
///
/// # The amendment (issue #1116)
///
/// This used to say the value was "deliberately not TOML data — no designer
/// tunes it". Issue #1116 is the amendment that entry anticipated, and it moved
/// the number rather than the reasoning: a fleet's delay IS authored, as
/// `[global] command_delay_ticks` in the world TOML, because it is a property of
/// the mission the fleet agreed to play — a scenario meant for players on one
/// LAN wants a shorter delay than one meant for players on separate mobile
/// networks, and that is a choice about the mission, not a constant about the
/// engine. What has not changed is that a *wrong* value is a stall or a desync
/// rather than a balance change, so the authored number is validated at world
/// load like the tick ratios beside it.
///
/// [`crate::lockstep`] is what sets it: joining a fleet applies the authored
/// value, and leaving one puts it back to zero. Nothing else writes it, so the
/// solo path cannot acquire a delay by accident — which is the property
/// AGENTS.md rule 7 asks for.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandDelay(pub u64);

/// One accepted command waiting for the tick it applies on.
#[derive(Clone, Debug)]
pub struct PendingCommand {
    /// The logical tick this applies on.
    pub tick: u64,
    /// Where this sits in the fleet's agreed order for that tick.
    pub order: CommandOrder,
    /// The routed ship, named the way anything outside this process names it.
    /// Carried alongside `route` so the log entry written on apply describes
    /// the ship admission resolved, not one re-derived a tick later.
    pub ship: ShipKey,
    /// The ship whose `AdmittedCommands` this lands in, resolved by admission's
    /// routing rule when the command arrived.
    pub route: Entity,
    pub command: AdmittedCommand,
}

/// Commands accepted at the boundary and not yet due, ordered by
/// `(apply tick, `[`CommandOrder`]`)`.
///
/// With [`CommandDelay`] at zero every command enqueued during a tick drains
/// again inside the same run of [`super::admit_system_commands`], so this is a
/// pass-through for a solo host and the observable behaviour is exactly what it
/// was before #898. The queue exists so that it stays a pass-through *by
/// configuration* rather than by construction: a future-stamped command slots
/// in with no redesign of the admission path.
///
/// Issue #1116 widened the key and left the queue alone, which is the shape
/// `p2p-delta-command-log-shape` requires — a second queue beside this one is
/// on its `must_not_be` list. A solo host's [`CommandOrder`]s all carry
/// [`HostSlot::SOLO`], so `(tick, order)` sorts identically to the old
/// `(tick, arrival)` and no solo behaviour moved.
#[derive(Resource, Default, Debug)]
pub struct PendingCommands {
    /// The slot this host mints its own commands' order from. Written once when
    /// a fleet forms; [`HostSlot::SOLO`] otherwise.
    origin: HostSlot,
    next_seq: u64,
    queue: BTreeMap<(u64, CommandOrder), PendingCommand>,
}

impl PendingCommands {
    /// Adopt this host's fleet slot, so the commands it admits from its own
    /// crew are ordered under that slot on every host in the fleet.
    ///
    /// Idempotent, and deliberately does **not** reset `next_seq`: a fleet is
    /// joined before the mission starts, and [`reset_command_log`] at the run
    /// boundary is the one place a sequence restarts.
    pub fn set_origin(&mut self, origin: HostSlot) {
        self.origin = origin;
    }

    /// The slot this host mints from.
    pub fn origin(&self) -> HostSlot {
        self.origin
    }

    /// The [`CommandOrder`] this host's next own-crew command will carry.
    fn next_order(&mut self) -> CommandOrder {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        CommandOrder::new(self.origin, seq)
    }

    /// Queue one accepted command for `tick`, returning the order it carries.
    ///
    /// `order` is `None` for a command this host admitted from its own crew —
    /// the order is minted here, from this host's slot. It is `Some` only for a
    /// command that arrived from another host already carrying the order that
    /// host minted, which is the whole of what makes the total order
    /// peer-independent: a receiver never renumbers a sender's traffic.
    ///
    /// Private for the same reason [`CommandLog::record`] is: the queue is
    /// reached only through [`stamp_accepted_command`], which is only ever
    /// called on the accepted branch of the authority check.
    fn enqueue(
        &mut self,
        tick: u64,
        order: Option<CommandOrder>,
        route: Entity,
        ship: ShipKey,
        command: AdmittedCommand,
    ) -> CommandOrder {
        let order = match order {
            Some(order) => order,
            None => self.next_order(),
        };
        self.queue.insert(
            (tick, order),
            PendingCommand {
                tick,
                order,
                ship,
                route,
                command,
            },
        );
        order
    }

    /// Drop every queued command and restart this host's sequence counter.
    ///
    /// The counter restarts because `seq` is "the *n*th command this host
    /// issued this run", and [`reset_command_log`] is where a run ends. A
    /// second round that kept counting would tiebreak identical inputs on a
    /// different number from the first, which is exactly the kind of hidden
    /// run-to-run state a replay cannot reproduce — and, across a fleet, a
    /// number two hosts would restart differently (trap T8).
    ///
    /// The fleet slot is NOT cleared: it is who this host is, not what it did.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.next_seq = 0;
    }

    /// Remove and return everything due at or before `now`, in
    /// `(tick, `[`CommandOrder`]`)` order, **recording each one in `log` as it
    /// goes**.
    ///
    /// The log takes its entries here rather than at acceptance so that the
    /// record is the applied sequence — see the module docs. Draining and
    /// recording are one act for the same reason queueing and recording used to
    /// be: there is no way to apply a command without writing it down, and no
    /// way to write one down that did not apply.
    ///
    /// "At or before" rather than "exactly": a command stamped for a tick that
    /// has somehow already passed is applied late rather than stranded in the
    /// queue forever. That cannot happen on a solo host — `SimTick` advances
    /// one step at a time and the stamp is never in the past — but a stranded
    /// command would be a silent, permanent input loss, and a late one is at
    /// least visible in the log it was recorded in.
    pub fn drain_due(&mut self, now: u64, log: &mut CommandLog) -> Vec<PendingCommand> {
        let later = self
            .queue
            .split_off(&(now.saturating_add(1), CommandOrder::default()));
        let due = std::mem::replace(&mut self.queue, later);
        due.into_values()
            .inspect(|pending| {
                // Built here, from the command about to be applied, so the
                // projection can never describe a different command from the
                // one that lands. The token stays on the `AdmittedCommand`;
                // only the non-secret half is recorded.
                log.record(LoggedCommand {
                    tick: pending.tick,
                    order: pending.order,
                    ship: pending.ship.clone(),
                    target: pending.command.target.clone(),
                    payload: pending.command.payload.clone(),
                });
            })
            .collect()
    }

    /// How many commands are waiting for a future tick.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Stamp one accepted command for the tick it applies on and queue it there.
///
/// The log is written by [`PendingCommands::drain_due`] when the command
/// actually applies, so the record is the applied sequence and every host in a
/// fleet writes the same one — see the module docs for why acceptance is the
/// wrong moment once [`CommandDelay`] is non-zero.
///
/// Only ever called on the accepted branch of admission's authority check, which
/// is what keeps refusals out of the queue and therefore out of the log
/// (`vellum_replay`'s third rule).
///
/// `route` and `ship` are the same destination said twice, in the two
/// vocabularies that need it: the `Entity` the queue delivers to inside this
/// process, and the [`ShipKey`] the log names for anything outside it. They are
/// taken together, here, because this is the one site that has admission's
/// resolved route in hand — deriving the key anywhere else would mean a second
/// copy of the routing rule.
///
/// `order` is `None` for a command this host's own crew issued and `Some` for
/// one that arrived from a peer already ordered. Either way this stays the ONE
/// recording site, which is what lets issue #1116's mesh merge sit *at* this
/// call rather than after it (`p2p-delta-command-log-shape`).
pub fn stamp_accepted_command(
    pending: &mut PendingCommands,
    apply_tick: u64,
    order: Option<CommandOrder>,
    route: Entity,
    ship: ShipKey,
    command: AdmittedCommand,
) -> CommandOrder {
    pending.enqueue(apply_tick, order, route, ship, command)
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
    hash = fnv1a(hash, &entry.order.origin.0.to_le_bytes());
    hash = fnv1a(hash, &entry.order.seq.to_le_bytes());
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
            feedback_correlation: None,
        }
    }

    fn entry(tick: u64, target: &str) -> LoggedCommand {
        LoggedCommand {
            tick,
            order: CommandOrder::default(),
            ship: ShipKey(SHIP.into()),
            target: SystemId(target.into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        }
    }

    /// Queue one command from this host's own crew. The `log` argument is kept
    /// so every call site still reads as "log and queue"; nothing is written
    /// until [`PendingCommands::drain_due`] applies it.
    fn stamp(_log: &mut CommandLog, pending: &mut PendingCommands, tick: u64, target: &str) {
        stamp_accepted_command(
            pending,
            tick,
            None,
            Entity::from_raw_u32(1).unwrap(),
            ShipKey(SHIP.into()),
            command(target, "t1"),
        );
    }

    /// The ordering key is `(tick, order)`, not the order alone: a command
    /// stamped for a later tick waits behind one stamped for an earlier tick
    /// even though it was issued first.
    #[test]
    fn the_queue_drains_in_tick_then_order() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();

        stamp(&mut log, &mut pending, 7, "late");
        stamp(&mut log, &mut pending, 3, "early");
        stamp(&mut log, &mut pending, 3, "also-3");

        let due = pending.drain_due(3, &mut log);
        let targets: Vec<&str> = due.iter().map(|p| p.command.target.0.as_str()).collect();
        assert_eq!(
            targets,
            vec!["early", "also-3"],
            "tick 3's commands drain in issue order and tick 7's stays behind"
        );
        assert_eq!(pending.len(), 1);

        let due = pending.drain_due(7, &mut log);
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

        let due = pending.drain_due(0, &mut log);
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

    /// Applying and recording are one act: the log is the drain order.
    ///
    /// Stamping alone writes nothing — the three commands below are recorded
    /// only as the two drains hand them over, which is why the log ends up in
    /// apply order rather than in the order they happened to be accepted in.
    #[test]
    fn the_log_records_every_applied_command_in_apply_order() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        for (tick, target) in [(5_u64, "c"), (0, "a"), (0, "b")] {
            stamp(&mut log, &mut pending, tick, target);
        }
        assert!(
            log.is_empty(),
            "nothing has applied yet, so nothing is written down yet"
        );
        pending.drain_due(0, &mut log);
        pending.drain_due(5, &mut log);

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
            &mut pending,
            0,
            None,
            route,
            ShipKey(SHIP.into()),
            command("helm", "session-token-aaaa"),
        );
        stamp_accepted_command(
            &mut pending,
            12,
            None,
            route,
            ShipKey(SHIP.into()),
            command("power", "session-token-bbbb"),
        );
        pending.drain_due(12, &mut log);

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
        assert_eq!(
            pending.drain_due(0, &mut log).len(),
            1,
            "tick 0's command applies"
        );
        assert_eq!(log.len(), 1, "and only the applied one is written down");
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
        let due = pending.drain_due(0, &mut log);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].order, CommandOrder::new(HostSlot::SOLO, 0));
    }

    // ── Issue #1116: the peer-independent total order ────────────────────────

    /// The ordering key is a fact about the ISSUER, not about the receiver.
    ///
    /// Two hosts admit each other's traffic in opposite arrival orders — which
    /// is the normal case, because each one reads its own crew's command
    /// locally and the other's off a socket — and still drain the tick in the
    /// same order, because the key says who issued each command and with what
    /// sequence. Under the pre-#1116 arrival counter these two queues would
    /// have drained in mirror-image orders and the two hosts would have applied
    /// the same tick's input differently.
    #[test]
    fn two_hosts_drain_a_tick_in_the_same_order_whatever_order_they_heard_it_in() {
        let alpha = CommandOrder::new(HostSlot(1), 0);
        let beta = CommandOrder::new(HostSlot(2), 0);
        let route = Entity::from_raw_u32(1).unwrap();

        let drained = |first: CommandOrder, second: CommandOrder| -> Vec<CommandOrder> {
            let mut log = CommandLog::default();
            let mut pending = PendingCommands::default();
            for order in [first, second] {
                stamp_accepted_command(
                    &mut pending,
                    4,
                    Some(order),
                    route,
                    ShipKey(SHIP.into()),
                    command("helm", "t1"),
                );
            }
            pending
                .drain_due(4, &mut log)
                .into_iter()
                .map(|p| p.order)
                .collect()
        };

        assert_eq!(
            drained(alpha, beta),
            drained(beta, alpha),
            "the drain order must not depend on which host's traffic arrived \
             first — that is the whole of `peer-independent`"
        );
        assert_eq!(
            drained(beta, alpha),
            vec![alpha, beta],
            "slot 1 sorts first"
        );
    }

    /// A receiver never renumbers a sender: an order that arrived is the order
    /// that is queued and the order that is recorded, and it does not consume
    /// this host's own sequence.
    #[test]
    fn a_peers_order_is_carried_not_reassigned() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        pending.set_origin(HostSlot(1));

        let peer = CommandOrder::new(HostSlot(2), 77);
        stamp_accepted_command(
            &mut pending,
            0,
            Some(peer),
            Entity::from_raw_u32(1).unwrap(),
            ShipKey(SHIP.into()),
            command("helm", "t1"),
        );
        stamp(&mut log, &mut pending, 0, "shields");
        pending.drain_due(0, &mut log);

        assert_eq!(
            log.entries().iter().map(|e| e.order).collect::<Vec<_>>(),
            vec![CommandOrder::new(HostSlot(1), 0), peer],
            "both apply on tick 0, and the FLEET order decides which lands \
             first — slot 1 before slot 2 — not which of them arrived first"
        );
        assert_eq!(
            log.entries()[1].order,
            peer,
            "the peer's order is carried, never reassigned: a receiver that \
             renumbered a sender's traffic would give two hosts different \
             orders for the same tick"
        );
        assert_eq!(
            log.entries()[0].order.seq,
            0,
            "and this host's own first command is still seq 0 — a peer's \
             traffic must not advance a counter that means 'the nth command \
             THIS host issued'"
        );
    }

    /// A `slot-N` roster id round-trips to the ordinal the simulation orders on,
    /// and anything else is refused rather than guessed at.
    #[test]
    fn a_roster_slot_id_round_trips_to_its_ordinal() {
        assert_eq!(HostSlot::from_slot_id("slot-3"), Some(HostSlot(3)));
        assert_eq!(HostSlot(3).slot_id(), "slot-3");
        assert_eq!(HostSlot::from_slot_id("slot-x"), None);
        assert_eq!(HostSlot::from_slot_id("3"), None);
        assert_eq!(
            HostSlot::from_slot_id("slot-1"),
            Some(HostSlot(1)),
            "the fleet owner is slot-1, and it must never collide with SOLO"
        );
        assert_ne!(HostSlot::SOLO, HostSlot(1));
    }

    /// The run boundary restarts the sequence but not the identity: round two
    /// numbers from zero again, still under this host's own slot.
    #[test]
    fn a_run_boundary_restarts_the_sequence_and_keeps_the_slot() {
        let mut log = CommandLog::default();
        let mut pending = PendingCommands::default();
        pending.set_origin(HostSlot(2));
        stamp(&mut log, &mut pending, 0, "helm");
        stamp(&mut log, &mut pending, 0, "power");
        pending.drain_due(0, &mut log);
        assert_eq!(log.entries()[1].order, CommandOrder::new(HostSlot(2), 1));

        log.clear();
        pending.clear();
        stamp(&mut log, &mut pending, 0, "shields");
        pending.drain_due(0, &mut log);
        assert_eq!(
            log.entries()[0].order,
            CommandOrder::new(HostSlot(2), 0),
            "round two must number from zero, under the same slot"
        );
        assert_eq!(pending.origin(), HostSlot(2));
    }
}
