use super::*;
use crate::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use crate::lobby::handler::Target;
use crate::native_host::panes::identity::PaneIdentity;
use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

fn identity(n: u8) -> PaneIdentity {
    PaneIdentity::adopt(
        format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
        format!("crew-{n}"),
    )
    .unwrap()
}

#[test]
fn renderer_death_closes_live_and_pending_consoles_without_recreation() {
    let bus = PaneBus::default();
    bus.arm_recreation("http://localhost".into(), "<html></html>".into());
    let live = bus.open(identity(1));
    bus.mark_live(live);
    let (pending, _) = bus.open_console("pending");
    bus.fault(live, PaneFault::ViewCrashed);
    close_after_thread_failure(&bus);
    assert!(!bus.is_open(live));
    assert!(!bus.is_open(pending));
    assert_eq!(bus.open_count(), 0);
    assert!(service_faults(&bus).is_empty());
    assert!(bus.take_pending_views().is_empty());
    // The adapter may observe further screen requests after terminal failure.
    let (late, _) = bus.open_console("late");
    close_after_thread_failure(&bus);
    assert!(!bus.is_open(late));
    assert!(service_faults(&bus).is_empty());
}

#[test]
fn a_view_crash_closes_the_pane_and_recreates_it_on_the_same_identity() {
    // The crash case, at the seam. The failed pane is closed (its token owes
    // the lobby a disconnect) and a fresh pane opens carrying the SAME token,
    // which is what lets the page rejoin its held station on reconnect.
    let bus = PaneBus::default();
    let original = bus.open(identity(1));
    let token = bus.token_of(original).unwrap();
    bus.mark_live(original);
    super::super::transport::identify_test_pane(&bus, original);

    bus.fault(original, PaneFault::ViewCrashed);
    let outcomes = service_faults(&bus);

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].failed, original);
    assert_eq!(outcomes[0].fault, PaneFault::ViewCrashed);
    let (recreated, _url) = outcomes[0]
        .recreated
        .clone()
        .expect("a view crash recreates the pane");
    assert_ne!(recreated, original, "a recreated pane is a new handle");
    assert_eq!(
        bus.token_of(recreated).as_deref(),
        Some(token.as_str()),
        "and carries the same session token, so it reconnects as the same participant"
    );

    // The lobby is owed exactly one disconnect, on the shared token.
    assert_eq!(
        bus.transport().poll(),
        vec![TransportEvent::Disconnected { token }]
    );
}

#[test]
fn a_flapping_view_crash_is_recreated_a_bounded_number_of_times_then_left_closed() {
    // The finding: unbounded auto-recreate lets a load-then-crash view flap
    // its station between human and Backfill forever. The bound recreates the
    // same identity at most MAX_RECREATIONS_PER_WINDOW times in the window,
    // then leaves the pane closed for the operator — the ReliableOverflow
    // conclusion. Here every crash is consecutive (well inside the window).
    let bus = PaneBus::default();
    let mut current = bus.open(identity(1));
    let token = bus.token_of(current).unwrap();
    bus.mark_live(current);

    let mut recreations = 0u32;
    let mut left_closed = false;
    // One more crash than the budget: the extra one must NOT recreate.
    for _ in 0..(MAX_RECREATIONS_PER_WINDOW + 1) {
        bus.fault(current, PaneFault::ViewCrashed);
        let outcome = service_faults(&bus).pop().expect("one fault serviced");
        match outcome.recreated {
            Some((new_id, _)) => {
                assert!(
                    !outcome.recreation_exhausted,
                    "a recreation is not also an exhaustion"
                );
                recreations += 1;
                current = new_id;
                bus.mark_live(current);
                assert_eq!(
                    bus.token_of(current).as_deref(),
                    Some(token.as_str()),
                    "each rebuild carries the SAME identity, so the count keys on it"
                );
            }
            None => {
                assert!(
                    outcome.recreation_exhausted,
                    "the crash gave up after the budget, not silently"
                );
                assert_eq!(
                    bus.open_count(),
                    0,
                    "past the budget the pane is left closed on Backfill, not flapping"
                );
                left_closed = true;
            }
        }
    }
    assert_eq!(
        recreations, MAX_RECREATIONS_PER_WINDOW,
        "recreated exactly the budget's worth of times"
    );
    assert!(left_closed, "and then stopped, leaving the pane closed");
}

#[test]
fn an_overflow_closes_the_pane_and_does_not_recreate_it() {
    // The wedged-page case: closed, and left closed. Recreating a page that
    // stopped draining risks a rebuild loop; the operator decides.
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    bus.mark_live(id);

    bus.fault(id, PaneFault::ReliableOverflow);
    let outcomes = service_faults(&bus);

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].fault, PaneFault::ReliableOverflow);
    assert!(
        outcomes[0].recreated.is_none(),
        "a reliable overflow closes but does not rebuild"
    );
    assert_eq!(bus.open_count(), 0, "nothing is open after the close");
}

#[test]
fn a_recreated_pane_receives_its_own_projection_and_the_failed_one_never_does() {
    // AC3/AC4 at the seam: after a crash, a projection addressed to the
    // shared token reaches the recreated pane and no other, and the failed
    // (closed) pane receives nothing at all.
    let bus = PaneBus::default();
    let failed = bus.open(identity(1));
    let bystander = bus.open(identity(2));
    let token = bus.token_of(failed).unwrap();
    bus.mark_live(failed);
    super::super::transport::identify_test_pane(&bus, failed);
    bus.mark_live(bystander);
    super::super::transport::identify_test_pane(&bus, bystander);

    bus.fault(failed, PaneFault::ViewCrashed);
    let (recreated, _) = service_faults(&bus)[0]
        .recreated
        .clone()
        .expect("a crash recreates");
    bus.mark_live(recreated);
    super::super::transport::identify_test_pane(&bus, recreated);

    bus.transport().dispatch(TransportDispatch {
        target: &Target::Token(token),
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    assert_eq!(
        bus.take_outbound(recreated).len(),
        1,
        "the recreated pane receives its own token's projection"
    );
    assert!(
        bus.take_outbound(failed).is_empty(),
        "the failed pane is closed and inherits nothing"
    );
    assert!(
        bus.take_outbound(bystander).is_empty(),
        "and a neighbouring pane never sees another participant's projection"
    );
}

#[test]
fn servicing_with_nothing_faulted_does_nothing() {
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    bus.mark_live(id);
    assert!(service_faults(&bus).is_empty());
    assert_eq!(bus.open_count(), 1);
}

#[test]
fn a_recreated_pane_can_take_the_seat_back_by_re_identifying() {
    // The transport-visible half: the recreated pane presents the shared
    // token through the ordinary `Identify` path, exactly as a reconnecting
    // phone does. (The lobby's reconnect-yield that restores the held station
    // is proved end to end in `tests/native_host_panes.rs`.)
    let bus = PaneBus::default();
    let original = bus.open(identity(1));
    let token = bus.token_of(original).unwrap();
    bus.mark_live(original);
    super::super::transport::identify_test_pane(&bus, original);
    bus.fault(original, PaneFault::ViewCrashed);
    let (recreated, _) = service_faults(&bus)[0]
        .recreated
        .clone()
        .expect("a crash recreates");

    // The disconnect is polled in its own frame, before the recreated view
    // has loaded — exactly as it is in production, where a fresh view takes
    // frames to load and identify. Draining it here is what puts the station
    // on Backfill before the reconnect that restores it.
    assert_eq!(
        bus.transport().poll(),
        vec![TransportEvent::Disconnected {
            token: token.clone()
        }],
        "the failed pane's disconnect comes first, on its own"
    );

    // Frames later: the recreated view has loaded and its page rejoins on the
    // shared token, through the ordinary `Identify` path.
    bus.mark_live(recreated);
    bus.submit(
        recreated,
        ClientMessage::Identify {
            token: token.clone(),
            name: "crew-1".to_string(),
        },
    )
    .expect("a recreated pane may identify as its own shared token");
    assert_eq!(
        bus.transport().poll(),
        vec![TransportEvent::Received {
            token: token.clone(),
            msg: ClientMessage::Identify {
                token,
                name: "crew-1".to_string(),
            },
        }],
        "then the reconnect Identify arrives on that same token"
    );
}
