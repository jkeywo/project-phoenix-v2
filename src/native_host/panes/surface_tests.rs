use super::*;
use crate::core::codec::JsonCodec;
use crate::core::messages::{DeliveryClass, ServerMessage};
use crate::lobby::handler::Target;
use crate::native_host::panes::identity::PaneIdentity;
use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

fn bus_with_pane() -> (PaneBus, PaneId) {
    let bus = PaneBus::default();
    let id = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    super::super::transport::identify_test_pane(&bus, id);
    (bus, id)
}

fn broadcast(bus: &PaneBus, msg: ServerMessage) {
    bus.transport().dispatch(TransportDispatch {
        target: &Target::All,
        msg: &msg,
        delivery: DeliveryClass::Reliable,
    });
}

fn broadcast_snapshot(bus: &PaneBus, msg: ServerMessage) {
    bus.transport().dispatch(TransportDispatch {
        target: &Target::All,
        msg: &msg,
        delivery: DeliveryClass::Snapshot,
    });
}

/// Deterministically model the simulation publishing while the pane thread
/// is inside evaluate_script, without relying on scheduler timing.
struct EnqueueThenFail {
    bus: PaneBus,
    messages: Vec<(ServerMessage, DeliveryClass)>,
}

impl PaneSurface for EnqueueThenFail {
    fn load(&mut self, _url: &str) -> Result<(), PaneSurfaceError> {
        Ok(())
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn push(&mut self, _json: &str) -> Result<(), PaneSurfaceError> {
        for (msg, delivery) in self.messages.drain(..) {
            self.bus.transport().dispatch(TransportDispatch {
                target: &Target::All,
                msg: &msg,
                delivery,
            });
        }
        Err(PaneSurfaceError::Script("page inbox full".into()))
    }

    fn drain(&mut self) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_failed_push_reconciles_a_newer_snapshot_enqueued_during_evaluation() {
    let bus = PaneBus::with_capacity(2);
    let id = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    super::super::transport::identify_test_pane(&bus, id);
    let snapshot = |frequency| ServerMessage::ShieldStatus {
        facings: Vec::new(),
        frequency,
    };
    broadcast(&bus, ServerMessage::GameStarted);
    broadcast_snapshot(&bus, snapshot(0.1));
    let mut surface = EnqueueThenFail {
        bus: bus.clone(),
        messages: vec![(snapshot(0.9), DeliveryClass::Snapshot)],
    };

    let report = pump_pane(&bus, id, &mut surface);
    assert!(report.push_failure.is_some());
    assert_eq!(report.deferred, 2);
    assert!(bus.take_faulted().is_empty());
    let queued = bus.take_outbound(id);
    assert_eq!(queued.len(), 2, "the requeued batch still respects the cap");
    assert!(queued[0].json.contains("GameStarted"));
    assert_eq!(
        JsonCodec.decode_server(&queued[1].json).unwrap(),
        snapshot(0.9)
    );
}

#[test]
fn a_failed_push_reports_overflow_when_a_concurrent_projection_cannot_fit() {
    let bus = PaneBus::with_capacity(2);
    let id = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    super::super::transport::identify_test_pane(&bus, id);
    broadcast(&bus, ServerMessage::GameStarted);
    broadcast(&bus, ServerMessage::ShipDestroyed);
    let mut surface = EnqueueThenFail {
        bus: bus.clone(),
        messages: vec![(
            ServerMessage::RepairState { teams: Vec::new() },
            DeliveryClass::Snapshot,
        )],
    };

    let report = pump_pane(&bus, id, &mut surface);
    assert!(report.push_failure.is_some());
    assert_eq!(report.deferred, 2);
    assert!(bus.is_open(id));
    assert_eq!(
        bus.take_faulted(),
        vec![(id, super::super::recovery::PaneFault::ReliableOverflow)]
    );
    let queued = bus.take_outbound(id);
    assert_eq!(queued.len(), 2, "only the two reliable messages survive");
    assert!(queued[0].json.contains("GameStarted"));
    assert!(queued[1].json.contains("ShipDestroyed"));
}

#[test]
fn a_failed_push_reports_overflow_from_reliable_messages_enqueued_during_evaluation() {
    let bus = PaneBus::with_capacity(2);
    let id = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    super::super::transport::identify_test_pane(&bus, id);
    broadcast(&bus, ServerMessage::GameStarted);
    broadcast(&bus, ServerMessage::ShipDestroyed);
    let mut surface = EnqueueThenFail {
        bus: bus.clone(),
        messages: vec![
            (ServerMessage::ReturnedToLobby, DeliveryClass::Reliable),
            (ServerMessage::GameStarted, DeliveryClass::Reliable),
        ],
    };

    let report = pump_pane(&bus, id, &mut surface);
    assert!(report.push_failure.is_some());
    assert_eq!(report.deferred, 2);
    assert_eq!(
        bus.take_faulted(),
        vec![(id, super::super::recovery::PaneFault::ReliableOverflow)],
        "one fault for the pane, even when multiple messages overflow"
    );
    let queued = bus.take_outbound(id);
    assert_eq!(queued.len(), 2);
    assert!(queued[0].json.contains("GameStarted"));
    assert!(queued[1].json.contains("ShipDestroyed"));
}

#[test]
fn a_burst_of_snapshots_of_one_kind_reaches_the_page_as_one_push() {
    // The phone's lossy channel drops stale snapshots; the pane bus
    // coalesces them at push time (see `registry`), so a frame that
    // unpacked into many ticks costs the page one evaluation per kind
    // rather than a budget's worth of stale ones (issue #1403).
    let (bus, id) = bus_with_pane();
    for _ in 0..(MAX_PUSHES_PER_FRAME * 2) {
        broadcast_snapshot(
            &bus,
            ServerMessage::ShieldStatus {
                facings: Vec::new(),
                frequency: 0.5,
            },
        );
    }
    broadcast_snapshot(&bus, ServerMessage::RepairState { teams: Vec::new() });
    let mut surface = RecordingSurface::ready();
    let report = pump_pane(&bus, id, &mut surface);
    assert_eq!(report.pushed, 2, "one push per kind");
    assert_eq!(report.deferred, 0);
    assert!(!report.budget_exhausted);
}

#[test]
fn a_document_that_has_not_loaded_is_not_pushed_to_and_keeps_its_backlog() {
    let (bus, id) = bus_with_pane();
    broadcast(&bus, ServerMessage::GameStarted);
    let mut surface = RecordingSurface::default();
    assert_eq!(pump_pane(&bus, id, &mut surface), PanePumpReport::default());
    assert!(surface.pushed.is_empty());

    surface.ready = true;
    let report = pump_pane(&bus, id, &mut surface);
    assert_eq!(
        report.pushed, 1,
        "the backlog arrives once the page can take it"
    );
    assert!(surface.pushed[0].starts_with("window.__phoenixPaneApply("));
}

#[test]
fn a_push_that_throws_puts_its_batch_back_in_order_rather_than_losing_it() {
    // The window between "the document loaded" and "its modules have run".
    // The message most likely to be in that first batch is `Welcome`, and a
    // pane that missed it sits in the lobby forever with a clean log.
    let (bus, id) = bus_with_pane();
    broadcast(&bus, ServerMessage::GameStarted);
    broadcast(
        &bus,
        ServerMessage::GameOver {
            reason: String::new(),
            outcome: None,
            report: Vec::new(),
        },
    );
    let mut surface = RecordingSurface::ready();
    surface.failing_pushes = 2;

    let first = pump_pane(&bus, id, &mut surface);
    assert_eq!(first.pushed, 0);
    assert_eq!(first.deferred, 2);
    assert!(first.push_failure.is_some());

    // Second frame: the first push still fails, the second succeeds, and
    // the remaining message is deferred once more.
    let second = pump_pane(&bus, id, &mut surface);
    assert_eq!(second.pushed, 0);
    assert_eq!(second.deferred, 2);

    let third = pump_pane(&bus, id, &mut surface);
    assert_eq!(third.pushed, 2, "nothing was lost across three frames");
    assert_eq!(third.deferred, 0);
    assert!(surface.pushed[0].contains("GameStarted"));
    assert!(surface.pushed[1].contains("GameOver"));
}

#[test]
fn a_frame_pushes_at_most_its_budget_and_leaves_the_rest_queued_in_order() {
    // Every push is a synchronous evaluate_script on the pane thread, so
    // one unbounded console would delay the other views. The first frame
    // after a document loads drains the whole load-time backlog.
    let (bus, id) = bus_with_pane();
    let total = MAX_PUSHES_PER_FRAME + 5;
    for _ in 0..total {
        broadcast(&bus, ServerMessage::GameStarted);
    }
    let mut surface = RecordingSurface::ready();

    let first = pump_pane(&bus, id, &mut surface);
    assert_eq!(first.pushed, MAX_PUSHES_PER_FRAME);
    assert_eq!(first.deferred, 5);
    assert!(first.budget_exhausted);
    assert!(
        first.push_failure.is_none(),
        "spending the budget is not a failure"
    );

    let second = pump_pane(&bus, id, &mut surface);
    assert_eq!(second.pushed, 5, "the remainder arrives on the next frame");
    assert_eq!(second.deferred, 0);
    assert!(!second.budget_exhausted);
    assert_eq!(surface.pushed.len(), total, "and nothing was lost");
}

#[test]
fn what_the_page_asks_for_reaches_the_simulation_through_the_bus() {
    let (bus, id) = bus_with_pane();
    let token = bus.token_of(id).unwrap();
    let mut surface = RecordingSurface::ready();
    surface.queue_record(format!(
        r#"{{"type":"Identify","data":{{"token":"{token}","name":"Ada"}}}}"#
    ));
    surface.queue_record(r#"{"type":"SelectStation","data":{"station":"helm"}}"#);

    let report = pump_pane(&bus, id, &mut surface);
    assert_eq!(report.accepted, 2);
    assert!(report.refusals.is_empty());
    let events = bus.transport().poll();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        TransportEvent::Received { token: t, .. } if t == &token
    ));
}

#[test]
fn a_page_that_asks_for_something_it_may_not_have_is_refused_and_reported() {
    // A pane may only identify as itself. The refusal is reported rather
    // than swallowed, so the operator sees a page misbehaving instead of a
    // pane that silently never joins.
    let (bus, id) = bus_with_pane();
    let mut surface = RecordingSurface::ready();
    surface.queue_record(
        r#"{"type":"Identify","data":{"token":"__local_console__","name":"impostor"}}"#,
    );
    let report = pump_pane(&bus, id, &mut surface);
    assert_eq!(report.accepted, 0);
    assert!(matches!(
        report.refusals.as_slice(),
        [PaneInputRefusal::Impersonation { .. }]
    ));
    assert!(bus.transport().poll().is_empty());
}
