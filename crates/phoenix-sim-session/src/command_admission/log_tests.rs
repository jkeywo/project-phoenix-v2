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
