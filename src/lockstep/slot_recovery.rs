//! Recovering a disconnected fixed ship slot on another machine (issue #1120).
//!
//! This is the final piece of PRD #1093's host mesh: the privileged server code on
//! a REPLACEMENT machine reclaiming one disconnected fixed slot during an active
//! mission. It builds directly on the three merged issues before it and reuses
//! their machinery rather than re-inventing it:
//!
//! * #1117 — the whole-payload snapshot transfer ([`crate::lockstep::snapshot_relay`]).
//! * #1118 — the rollback-able gate-and-restore and the [`MeshRestoreArm`] receiver
//!   arm, whose own docs name this issue as "the other legitimate arm; it will set
//!   this for a joining host the same way". This module IS that arm-setter.
//! * #1119 — [`crate::lockstep::LockstepSession::depart`] leaves the vanished slot
//!   in the roster marked disconnected, which is precisely the object recovered
//!   here; [`crate::lockstep::LockstepSession::rejoin`] is its inverse.
//!
//! # The pure decision, and where determinism lives (AC2)
//!
//! A replacement types the fleet code and claims a disconnected slot. Every claim
//! passes through the fleet owner — the star centre
//! (`p2p-delta-transport-is-a-star-today`) — which stamps it with a monotonic
//! `claim_seq` in arrival order and broadcasts a [`crate::lockstep::SlotClaimFrame`]
//! to the whole fleet. [`PendingSlotClaims`] is the pure resolver every host runs
//! over those frames: the winner of a race for one slot is the LOWEST `claim_seq`,
//! so two hosts that hear two competing claims agree the same winner from the
//! shared value — never from who-processed-first locally, the skew-prone mistake
//! #1119's two failed fix rounds are the cautionary tale for. The recovery boundary
//! and the leader that transfers the canonical record are likewise pure functions
//! of the winning claim and the frozen roster, identical on every host.
//!
//! # The execution, mirrored on #1118 (AC1, AC3)
//!
//! Once a winning claim is seen, [`drive_slot_recovery`] opens a recovery whose
//! shape is #1118's boundary dance:
//!
//! * every host withholds ticks past the boundary ([`SlotRecoveryHold`], read by
//!   [`crate::lockstep::gate_lockstep_ticks`] beside the peer-stall and the #1118
//!   hold);
//! * the leader — the lowest roster slot that is NOT the recovering one — captures
//!   its canonical record at the boundary and transfers it through #1117;
//! * the replacement ARMS [`MeshRestoreArm`] naming that leader and restores through
//!   #1118's rollback-able [`crate::lockstep::gate_and_restore`], committing only
//!   when the restored world folds to the record's own digest — the post-restore
//!   agreement AC3 requires — so a failed restore leaves its world clean;
//! * the replacement suppresses its own mesh egress while it bootstraps and waits
//!   for the transfer, so its throwaway pre-restore world publishes no divergent
//!   digest and no premature watermark to the survivors.
//!
//! AC1 — that server-code entry may select ONLY a disconnected fixed slot, and may
//! not add a ship, change a loadout or displace a connected host — is the JS lobby
//! layer's (`gui/host-mesh.js`'s `admitHost`/claim path); the [`PendingSlotClaims`]
//! resolver is the Rust half that a granted claim must have passed, and its
//! refusals are unit-tested there and in `tests/lockstep_slot_recovery.rs`.
//!
//! # AGENTS.md rule 10
//!
//! The pure decision ([`PendingSlotClaims`], [`leader_for`], [`recovery_boundary`])
//! is Bevy-free and unit-tested with no `World`; the Bevy adapter
//! ([`drive_slot_recovery`] and the resources it drives) is co-located here, the
//! same shape [`crate::lockstep::host_loss`] keeps for the same reason.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use crate::command_admission::log::HostSlot;
use crate::logging::LogCat;
use crate::sim_tick::SimTick;
use crate::world::server::BridgeWorldSource;

use super::recovery_plan::recovery_boundary;
use super::snapshot_relay::{
    send_snapshot, MeshRestoreArm, MeshRestoreOutcome, MeshSnapshotReceiver,
};
use super::{FleetLockstep, FleetRoster, MeshAgreement};

/// Every replacement claim on a disconnected slot this host has heard, and the
/// deterministic winner of each (issue #1120, AC2).
///
/// Pure and total, so the whole of "which claim won" is decided in one tested
/// place. The winner for a slot is the lowest `claim_seq` ever observed for it —
/// the first the owner minted — which is a function of the shared claim VALUES and
/// not of the order this host happened to receive them, so every host that has
/// heard the same claims agrees the same winner.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingSlotClaims {
    /// slot -> (winning claim_seq, the tick the owner stamped the winning claim).
    winners: BTreeMap<HostSlot, (u64, u64)>,
    /// Slots whose recovery [`drive_slot_recovery`] has already opened, so a later
    /// duplicate claim frame is inert rather than a second recovery.
    opened: BTreeSet<HostSlot>,
}

impl PendingSlotClaims {
    /// Record a granted claim on `slot`, sequenced `claim_seq`, stamped at `tick`.
    ///
    /// The winner is the lowest `claim_seq` seen so far, so a lower one arriving
    /// later supersedes a higher one already recorded — the resolution is a
    /// function of the values, deterministic under any arrival order. The winning
    /// claim's `tick` travels with its seq, because the recovery boundary is
    /// derived from it and must be the same everywhere.
    pub fn observe(&mut self, slot: HostSlot, claim_seq: u64, tick: u64) {
        match self.winners.get_mut(&slot) {
            Some(entry) if claim_seq < entry.0 => *entry = (claim_seq, tick),
            Some(_) => {}
            None => {
                self.winners.insert(slot, (claim_seq, tick));
            }
        }
    }

    /// The winning `claim_seq` for `slot`, if any claim has been heard.
    pub fn winning_seq(&self, slot: HostSlot) -> Option<u64> {
        self.winners.get(&slot).map(|(seq, _)| *seq)
    }

    /// Whether `claim_seq` is the winning claim for `slot` — "did this claim win?".
    pub fn wins(&self, slot: HostSlot, claim_seq: u64) -> bool {
        self.winning_seq(slot) == Some(claim_seq)
    }

    /// Whether a recovery for `slot` has already been opened.
    pub fn is_opened(&self, slot: HostSlot) -> bool {
        self.opened.contains(&slot)
    }

    /// The next winning claim whose recovery has not been opened yet, as
    /// `(slot, claim_seq, stamped_tick)`, lowest slot first for a stable order.
    fn next_unopened(&self) -> Option<(HostSlot, u64, u64)> {
        self.winners
            .iter()
            .find(|(slot, _)| !self.opened.contains(slot))
            .map(|(slot, (seq, tick))| (*slot, *seq, *tick))
    }

    fn mark_opened(&mut self, slot: HostSlot) {
        self.opened.insert(slot);
    }
}

/// This host's part in a slot recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotRecoveryRole {
    /// This host IS the replacement claiming the slot: it arms, restores, resumes.
    Recovering,
    /// The lowest roster slot that is not the recovering one: it transfers the
    /// canonical record.
    Leader,
    /// Any other surviving host: it holds at the boundary and waits.
    Bystander,
}

/// The leader that transfers the canonical record for a recovery of `recovering`
/// (issue #1120): the lowest roster slot that is NOT the recovering one.
///
/// Pure and shared — every host computes the same leader from the frozen roster
/// and the recovering slot alone. In practice this is the owner (`slot-1`, the star
/// centre, which a member's socket close never removes), unless the owner itself is
/// the slot being recovered, which host migration — out of this issue's scope —
/// would own.
pub fn leader_for(roster: &FleetRoster, recovering: HostSlot) -> HostSlot {
    roster
        .ships()
        .iter()
        .map(|ship| ship.host)
        .filter(|&host| host != recovering)
        .min()
        .unwrap_or_else(|| roster.lead())
}

/// How far past the boundary a host withholds ticks while a slot recovery it has
/// not yet resolved is in flight (issue #1120).
///
/// The #1120 twin of [`crate::lockstep::recovery::RecoveryHold`]: a separate
/// resource the barrier reads BESIDE its own peer-stall, so the #1116 barrier
/// decision is unchanged and this is a clearly-separable addition. `None` = no hold.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct SlotRecoveryHold {
    /// Ticks strictly greater than this are withheld; `None` runs freely.
    pub withhold_beyond: Option<u64>,
}

/// This host's view of a slot recovery in flight, if any.
#[derive(Resource, Default)]
pub struct SlotRecoveryState {
    active: Option<ActiveSlotRecovery>,
}

impl SlotRecoveryState {
    /// Whether a recovery is currently tracked.
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The recovering slot of the tracked recovery, if any.
    pub fn recovering_slot(&self) -> Option<HostSlot> {
        self.active.as_ref().map(|a| a.slot)
    }

    /// The tick this host must not run past yet, or `None` when it is free. Every
    /// role holds at the boundary until it resolves — the leader captures there, the
    /// replacement restores there, the survivors wait there.
    fn hold_boundary(&self) -> Option<u64> {
        match &self.active {
            Some(a) if !a.resolved => Some(a.boundary),
            _ => None,
        }
    }

    /// Whether this host must publish no mesh egress right now: it is the recovering
    /// replacement, bootstrapping and awaiting the transfer, so its throwaway
    /// pre-restore world must leak neither a divergent digest nor a premature
    /// watermark to the survivors.
    pub fn suppresses_egress(&self) -> bool {
        matches!(
            &self.active,
            Some(a) if a.role == SlotRecoveryRole::Recovering && !a.resolved
        )
    }
}

/// One tracked slot recovery.
struct ActiveSlotRecovery {
    slot: HostSlot,
    leader: HostSlot,
    boundary: u64,
    claim_seq: u64,
    role: SlotRecoveryRole,
    /// The leader's capture tick, once it has transferred; the tick the replacement
    /// restores to. `None` until sent.
    record_tick: Option<u64>,
    resolved: bool,
}

/// One resolved slot-recovery event, as this host recorded it (issue #1120).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotRecoveryRecord {
    /// The slot that was recovered.
    pub slot: HostSlot,
    /// The leader that transferred the canonical record.
    pub leader: HostSlot,
    /// The boundary the fleet held at, captured at, and restored to.
    pub boundary: u64,
    /// The winning claim's owner-minted sequence.
    pub claim_seq: u64,
    /// How the event resolved, from this host's vantage.
    pub result: SlotRecoveryResult,
}

/// How a slot recovery resolved, from the recording host's vantage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotRecoveryResult {
    /// This host led: it transferred its canonical record at `record_tick`.
    Led { record_tick: u64 },
    /// This host was a surviving bystander; it waited and resumed.
    Witnessed,
    /// This host was the replacement and restored the leader's record, folding to
    /// it — the takeover succeeded.
    Recovered { record_tick: u64, digest: u64 },
    /// This host was the replacement but the leader's record was refused (a bad
    /// build, a corrupt payload, an incomplete restore). Its world was rolled back
    /// to the bootstrap, never left half-restored, and the slot stays disconnected.
    NoValidRecord { refusal: String },
}

/// Every slot-recovery event this host has recorded (issue #1120).
#[derive(Resource, Default)]
pub struct SlotRecoveryLog {
    entries: Vec<SlotRecoveryRecord>,
}

impl SlotRecoveryLog {
    /// The recorded events, oldest first.
    pub fn entries(&self) -> &[SlotRecoveryRecord] {
        &self.entries
    }

    /// The most recent event, if any.
    pub fn last(&self) -> Option<&SlotRecoveryRecord> {
        self.entries.last()
    }
}

/// Drive the slot recovery a winning claim implies (issue #1120).
///
/// Exclusive because the leader's transfer captures the whole world and the
/// replacement's restore overwrites it. Wired into the mesh chain
/// (`register_lockstep`) after [`crate::lockstep::recovery::drive_recovery`] and
/// before [`crate::lockstep::gate_lockstep_ticks`] — so the hold it sets is honoured
/// this same frame — and before
/// [`crate::lockstep::snapshot_relay::drain_mesh_restore`], so an arm it sets takes
/// effect this frame.
pub fn drive_slot_recovery(world: &mut World) {
    let Some(session) = world.get_resource::<FleetLockstep>() else {
        return;
    };
    if session.is_alone() {
        if let Some(mut hold) = world.get_resource_mut::<SlotRecoveryHold>() {
            hold.withhold_beyond = None;
        }
        return;
    }
    let local = session.local();
    let delay = session.delay();
    let sim_tick = world.get_resource::<SimTick>().map_or(0, |t| t.0);
    let interval = world.resource::<MeshAgreement>().local.interval;

    if world.resource::<SlotRecoveryState>().active.is_none() {
        open_slot_recovery(world, local, interval, delay);
    }
    advance_slot_recovery(world, local, delay, sim_tick);

    let hold = world.resource::<SlotRecoveryState>().hold_boundary();
    world.resource_mut::<SlotRecoveryHold>().withhold_beyond = hold;
}

/// Open a recovery for the next winning claim, if one is due and nothing else is in
/// flight.
fn open_slot_recovery(world: &mut World, local: HostSlot, interval: u64, delay: u64) {
    // One recovery at a time: a divergence recovery (#1118) in flight defers this,
    // and #1118's own begin defers to a slot recovery in flight — so the two never
    // arm the same host at once.
    if world
        .resource::<super::recovery::RecoveryState>()
        .is_active()
    {
        return;
    }
    let Some((slot, claim_seq, claim_tick)) = world.resource::<PendingSlotClaims>().next_unopened()
    else {
        return;
    };

    let role = if local == slot {
        SlotRecoveryRole::Recovering
    } else if local == leader_for(world.resource::<FleetRoster>(), slot) {
        SlotRecoveryRole::Leader
    } else {
        SlotRecoveryRole::Bystander
    };

    // AC1's "cannot displace a connected host", enforced on every survivor: a
    // survivor opens a recovery only for a slot IT HAS DEPARTED (issue #1119's
    // `depart`). A claim naming a slot this survivor still holds a live connection
    // to is DEFERRED, not opened — never marked opened either, so if the slot does
    // later depart (the host-loss simply arrived after the claim) the recovery
    // still opens, while a claim for a genuinely-live slot never displaces it. The
    // replacement itself (local == slot) is the one host for which this is a
    // genuine recovery of its own identity, so it is exempt.
    if role != SlotRecoveryRole::Recovering && !world.resource::<FleetLockstep>().has_departed(slot)
    {
        return;
    }

    let boundary = recovery_boundary(claim_tick, interval, delay);
    let leader = leader_for(world.resource::<FleetRoster>(), slot);
    world.resource_mut::<PendingSlotClaims>().mark_opened(slot);

    match role {
        SlotRecoveryRole::Recovering => {
            // Arm to accept the canonical record from the elected leader, and only
            // that leader (#1118's arm). Clear any stale outcome so this host acts
            // on THIS recovery's restore.
            world.resource_mut::<MeshRestoreArm>().arm(leader);
            if let Some(mut rx) = world.get_resource_mut::<MeshSnapshotReceiver>() {
                rx.clear_outcome();
            }
        }
        SlotRecoveryRole::Leader | SlotRecoveryRole::Bystander => {
            // Re-admit the recovering slot to the barrier at the boundary, so it is
            // a full lockstep member again once the replacement resumes (which is
            // what lets its crew's future commands land at an agreed tick). The
            // SlotRecoveryHold below is what actually holds this host at the
            // boundary meanwhile.
            if let Some(mut session) = world.get_resource_mut::<FleetLockstep>() {
                session.rejoin(slot, boundary);
            }
        }
    }

    let log = world
        .get_resource::<crate::logging::LogFilterConfig>()
        .cloned();
    crate::pwarn!(
        log,
        LogCat::Admit,
        "host-mesh slot recovery: slot {} claimed (seq {claim_seq}), leader {}, \
         boundary tick {boundary} (this host is {role:?})",
        slot.slot_id(),
        leader.slot_id(),
    );
    world.resource_mut::<SlotRecoveryState>().active = Some(ActiveSlotRecovery {
        slot,
        leader,
        boundary,
        claim_seq,
        role,
        record_tick: None,
        resolved: false,
    });
}

/// Advance the tracked recovery: do this host's role work and, when done, record
/// the event and clear the tracking.
fn advance_slot_recovery(world: &mut World, local: HostSlot, delay: u64, sim_tick: u64) {
    let Some(mut active) = world.resource_mut::<SlotRecoveryState>().active.take() else {
        return;
    };
    let boundary = active.boundary;
    let mut resolution: Option<SlotRecoveryResult> = None;

    match active.role {
        SlotRecoveryRole::Leader => {
            // Held AT the boundary by the hold, so `sim_tick == boundary` when it
            // captures — the same tick the replacement restores to.
            if sim_tick >= boundary && active.record_tick.is_none() {
                active.record_tick = try_send_record(world, local, active.slot, active.boundary);
            }
            if let Some(tick) = active.record_tick {
                if replacement_resumed(world, active.slot, boundary, delay) {
                    resolution = Some(SlotRecoveryResult::Led { record_tick: tick });
                }
            }
        }
        SlotRecoveryRole::Bystander => {
            if replacement_resumed(world, active.slot, boundary, delay) {
                resolution = Some(SlotRecoveryResult::Witnessed);
            }
        }
        SlotRecoveryRole::Recovering => match restore_result(world) {
            RestoreResolution::Pending => {}
            RestoreResolution::Recovered { tick, digest } => {
                resolution = Some(SlotRecoveryResult::Recovered {
                    record_tick: tick,
                    digest,
                });
            }
            RestoreResolution::Failed(refusal) => {
                resolution = Some(SlotRecoveryResult::NoValidRecord { refusal });
            }
        },
    }

    if let Some(result) = resolution {
        finish_slot_recovery(world, local, &active, result);
        return;
    }

    world.resource_mut::<SlotRecoveryState>().active = Some(active);
}

/// Capture and queue the leader's canonical record for the replacement. Returns the
/// tick captured (the tick the replacement restores to), or `None` if it could not
/// be framed.
fn try_send_record(
    world: &mut World,
    local: HostSlot,
    slot: HostSlot,
    boundary: u64,
) -> Option<u64> {
    let scenario = world
        .get_resource::<BridgeWorldSource>()
        .map(|source| source.path.clone())
        .unwrap_or_default();
    let captured_at = world.get_resource::<SimTick>().map_or(0, |t| t.0);
    // Stable per recovery so a re-sent chunk is never mistaken for a different
    // transfer: the boundary in the high bits, the recovered slot in the low.
    let transfer_id = (boundary << 16) ^ u64::from(slot.0);
    let log = world
        .get_resource::<crate::logging::LogFilterConfig>()
        .cloned();
    match send_snapshot(world, local, transfer_id, scenario) {
        Ok(chunks) => {
            crate::pinfo!(
                log,
                LogCat::Admit,
                "host-mesh slot recovery leader {} transferred its canonical record in \
                 {chunks} chunk(s) at tick {captured_at} for slot {}",
                local.slot_id(),
                slot.slot_id(),
            );
            Some(captured_at)
        }
        Err(why) => {
            crate::perror!(
                log,
                LogCat::Admit,
                "host-mesh slot recovery leader {} could not capture its record: {why}",
                local.slot_id(),
            );
            None
        }
    }
}

/// The replacement's read of its restore outcome.
enum RestoreResolution {
    Pending,
    Recovered { tick: u64, digest: u64 },
    Failed(String),
}

fn restore_result(world: &World) -> RestoreResolution {
    let Some(outcome) = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|rx| rx.last_outcome().cloned())
    else {
        return RestoreResolution::Pending;
    };
    match outcome {
        MeshRestoreOutcome::Committed { tick, digest } => {
            RestoreResolution::Recovered { tick, digest }
        }
        MeshRestoreOutcome::NotReady => RestoreResolution::Pending,
        MeshRestoreOutcome::RefusedGate(why) => RestoreResolution::Failed(why),
        MeshRestoreOutcome::RefusedIntegrity { recorded, restored } => {
            RestoreResolution::Failed(format!(
                "the restored world folds to {restored:#018x}, not the {recorded:#018x} the \
                 record recorded"
            ))
        }
        MeshRestoreOutcome::Incomplete { tick, gaps } => {
            RestoreResolution::Failed(format!("the restore left {gaps} gap(s) at tick {tick}"))
        }
        MeshRestoreOutcome::RefusedWrongSender { armed_from, from } => {
            RestoreResolution::Failed(format!(
                "the record came from {from:?}, not the leader {}",
                armed_from.slot_id()
            ))
        }
        MeshRestoreOutcome::RefusedUnarmed => {
            RestoreResolution::Failed("this host was not armed to restore".to_string())
        }
    }
}

/// Whether the replacement has RESUMED past the boundary — its post-restore
/// watermark has advanced beyond the value a host paused at the boundary holds.
///
/// A monotone liveness signal, exactly as #1118's `recovering_all_resumed`: the
/// replacement suppresses egress until it restores, so a watermark past the
/// boundary is proof it has restored and stepped. The barrier, not this, governs
/// which ticks run.
fn replacement_resumed(world: &World, slot: HostSlot, boundary: u64, delay: u64) -> bool {
    let Some(session) = world.get_resource::<FleetLockstep>() else {
        return false;
    };
    let threshold = boundary.saturating_add(delay);
    session
        .observed(slot)
        .is_some_and(|watermark| watermark > threshold)
}

/// Record this host's event, disarm, and clear the tracking.
fn finish_slot_recovery(
    world: &mut World,
    local: HostSlot,
    active: &ActiveSlotRecovery,
    result: SlotRecoveryResult,
) {
    world.resource_mut::<MeshRestoreArm>().disarm();
    let record = SlotRecoveryRecord {
        slot: active.slot,
        leader: active.leader,
        boundary: active.boundary,
        claim_seq: active.claim_seq,
        result: result.clone(),
    };
    world.resource_mut::<SlotRecoveryLog>().entries.push(record);
    // Clearing the tracking lifts this host's hold (SlotRecoveryHold -> None next
    // publish) and, for the replacement, resumes its egress — so its first genuine
    // post-restore frame flows to the survivors and the fleet resumes in lockstep.
    world.resource_mut::<SlotRecoveryState>().active = None;

    let log = world
        .get_resource::<crate::logging::LogFilterConfig>()
        .cloned();
    crate::pinfo!(
        log,
        LogCat::Admit,
        "host-mesh slot recovery for slot {} resolved on {}: {result:?}",
        active.slot.slot_id(),
        local.slot_id(),
    );
}

/// Install the slot-recovery resources. The [`drive_slot_recovery`] system is wired
/// into the mesh chain by [`crate::lockstep::register_lockstep`].
pub fn register_slot_recovery(app: &mut App) {
    {
        use crate::authoritative::{DeclareState, StateClass};
        // Session bookkeeping about a recovery in flight and the claims heard —
        // never folded, and every honest host derives it from the same shared
        // claims. The log is a report ABOUT the event.
        app.declare_state::<PendingSlotClaims>(StateClass::Timer, "fleet-slot-recovery-state")
            .declare_state::<SlotRecoveryState>(StateClass::Timer, "fleet-slot-recovery-state")
            .declare_state::<SlotRecoveryHold>(StateClass::Timer, "fleet-slot-recovery-state")
            .declare_state::<SlotRecoveryLog>(StateClass::Derived, "fleet-slot-recovery-state");
    }
    app.init_resource::<PendingSlotClaims>()
        .init_resource::<SlotRecoveryState>()
        .init_resource::<SlotRecoveryHold>()
        .init_resource::<SlotRecoveryLog>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockstep::{FleetRoster, FleetShip};

    fn roster(slots: &[HostSlot], local: HostSlot) -> FleetRoster {
        FleetRoster::new(
            slots.iter().map(|&host| FleetShip::new(host)).collect(),
            local,
        )
    }

    /// **AC2, the determinism crown for the claim race.** Two machines claim the
    /// same vacant slot; the winner is the LOWEST owner-minted `claim_seq`, and it
    /// is the same whatever order the two claims are observed in — so every host
    /// that hears both agrees the one winner from the shared value, never from
    /// who-processed-first locally.
    #[test]
    fn the_lowest_claim_seq_wins_regardless_of_arrival_order() {
        // Host A hears claim seq 5 then seq 9.
        let mut a = PendingSlotClaims::default();
        a.observe(HostSlot(3), 5, 100);
        a.observe(HostSlot(3), 9, 101);

        // Host B hears them in the OPPOSITE order.
        let mut b = PendingSlotClaims::default();
        b.observe(HostSlot(3), 9, 101);
        b.observe(HostSlot(3), 5, 100);

        assert_eq!(a.winning_seq(HostSlot(3)), Some(5));
        assert_eq!(
            a.winning_seq(HostSlot(3)),
            b.winning_seq(HostSlot(3)),
            "two hosts that heard the same two claims in opposite orders must elect \
             the SAME winner — the resolution is a function of the seq VALUE, not \
             of arrival"
        );
        assert!(a.wins(HostSlot(3), 5) && !a.wins(HostSlot(3), 9));
        // The winning claim's tick travels with it, so the boundary is shared.
        assert_eq!(a.winners.get(&HostSlot(3)).map(|(_, t)| *t), Some(100));
    }

    /// The leader is the lowest roster slot that is not the one being recovered —
    /// in practice the owner, unless the owner itself is recovered.
    #[test]
    fn the_leader_is_the_lowest_slot_that_is_not_recovering() {
        let roster = roster(&[HostSlot(1), HostSlot(2), HostSlot(3)], HostSlot(1));
        assert_eq!(leader_for(&roster, HostSlot(3)), HostSlot(1));
        assert_eq!(leader_for(&roster, HostSlot(2)), HostSlot(1));
        assert_eq!(
            leader_for(&roster, HostSlot(1)),
            HostSlot(2),
            "recovering the owner falls to the next slot (host migration is out of \
             this issue's scope, but the leader math is still total)"
        );
    }

    /// A recovery is opened once per slot: a duplicate winning claim frame is inert.
    #[test]
    fn a_slot_recovery_opens_once() {
        let mut claims = PendingSlotClaims::default();
        claims.observe(HostSlot(3), 5, 100);
        assert_eq!(claims.next_unopened(), Some((HostSlot(3), 5, 100)));
        claims.mark_opened(HostSlot(3));
        assert!(claims.is_opened(HostSlot(3)));
        // A later duplicate does not re-open it.
        claims.observe(HostSlot(3), 5, 100);
        assert_eq!(claims.next_unopened(), None);
    }
}
