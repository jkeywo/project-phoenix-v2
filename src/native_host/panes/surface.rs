//! The per-frame pane loop, and the surface it drives (issue #1122).
//!
//! [`PaneSurface`] is everything the loop needs from a document: navigate it,
//! ask whether it has finished loading, hand it one encoded `ServerMessage`, and
//! collect whatever it has asked for. Ultralight satisfies it
//! ([`super::ultralight`], feature `ultralight`); so does [`RecordingSurface`],
//! which is how the loop is tested on a machine with no SDK, no GPU and no
//! window.
//!
//! [`pump_pane`] is the loop itself, and it is deliberately *here* rather than
//! inside the Ultralight adapter: everything it decides — when a document is
//! ready to be pushed to, what happens to a push that fails, what a drained
//! record turns into — is policy, and policy that only runs with a GPU attached
//! is policy nothing ever checks.
//!
//! # A failed push is a message put back, not a message lost
//!
//! `load_url` returns before the document's own scripts are guaranteed to have
//! run, and "the document reports it has finished loading" is not the same
//! instant as "every module has evaluated". So a push can throw for an entirely
//! ordinary reason — `window.__phoenixPaneApply` does not exist yet — and the
//! message it carried must survive that. [`pump_pane`] stops at the first
//! failure and returns the rest of the batch to the front of the pane's queue,
//! in order, so the next frame retries from exactly where it stopped.
//!
//! Dropping instead would be invisible and permanent: the message most likely to
//! be in that first batch is `Welcome`, and a pane that missed its `Welcome`
//! sits in the lobby forever with a completely clean log.
//!
//! # Every push costs the simulation, so a frame may only make so many
//!
//! This is the cost worth stating out loud rather than discovering. A push is a
//! **synchronous** `evaluate_script` into a real browser engine, and the loop
//! that calls it (`super::ultralight::drive_panes`) runs in `Update` on the Bevy
//! main thread — the same thread `FixedUpdate` runs `SimSet` on (AGENTS.md rule
//! 7). Page JavaScript execution time is therefore *simulation* time, for every
//! participant on the ship, including the network clients issue #1112 will add.
//!
//! An unbounded frame is easy to reach without anything being wrong: the first
//! frame after a document loads drains the whole load-time backlog at once, and
//! that backlog is sized by [`super::registry::DEFAULT_OUTBOUND_CAP`]. So
//! [`pump_pane`] pushes at most [`MAX_PUSHES_PER_FRAME`] per pane per frame and
//! leaves the rest queued, by the same `requeue_front` a failed push uses. The
//! backlog then arrives over several frames in order, which is what a page can
//! render anyway.
//!
//! A page-side watchdog — a single push that never returns — is issues
//! #1123/#1124's, and is not solved here.

use super::registry::{PaneDispatch, PaneId};
use super::transport::{PaneBus, PaneInputRefusal};

/// How many messages one pane may be handed in one frame.
///
/// Not a gameplay value and not a designer's knob: it is a bound on how much of
/// a frame the main thread may spend inside a browser engine, and the module
/// note above says why that is the simulation's business. Sized to keep a
/// steady-state pane (a snapshot and a handful of transitions per tick) well
/// clear of it, so the budget only ever bites on a backlog.
pub const MAX_PUSHES_PER_FRAME: usize = 32;

/// Why a surface refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneSurfaceError {
    /// The document could not be navigated to.
    Load(String),
    /// A script could not be evaluated, or threw. The ordinary cause is a page
    /// whose own scripts have not finished running.
    Script(String),
    /// A frame copy failed (issue #1404): a surface that could not be locked, a
    /// buffer of the wrong length, a view that has stopped answering. Ordinarily
    /// transient — a run of them in a row is the crashed-view signal.
    Frame(String),
}

impl std::fmt::Display for PaneSurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaneSurfaceError::Load(detail) => write!(f, "load failed: {detail}"),
            PaneSurfaceError::Script(detail) => write!(f, "script failed: {detail}"),
            // Bare, unlike its siblings: every caller of a frame copy already
            // says "frame copy failed" and how many in a row, so a prefix here
            // would only repeat them.
            PaneSurfaceError::Frame(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for PaneSurfaceError {}

/// One pane's document, as the frame loop sees it.
pub trait PaneSurface {
    /// Navigate to `url`.
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError>;
    /// Whether the document has finished loading and may be pushed to.
    fn is_ready(&self) -> bool;
    /// Hand the page one encoded `ServerMessage`.
    fn push(&mut self, json: &str) -> Result<(), PaneSurfaceError>;
    /// Collect every record the page has queued since the last call.
    fn drain(&mut self) -> Vec<String>;
}

/// What one frame of [`pump_pane`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanePumpReport {
    /// Messages handed to the page.
    pub pushed: usize,
    /// Messages returned to the queue because a push failed.
    pub deferred: usize,
    /// Records the page asked for that reached the simulation.
    pub accepted: usize,
    /// Records the page asked for that did not. Each one is a page saying
    /// something it is not entitled to say, or saying it wrongly.
    pub refusals: Vec<PaneInputRefusal>,
    /// The push failure that stopped this frame's batch, if there was one.
    pub push_failure: Option<PaneSurfaceError>,
    /// Whether this frame stopped because it had spent its per-frame push
    /// budget rather than because anything went wrong. Ordinary on the first
    /// frame after a load; a pane that reports it every frame is one whose page
    /// cannot keep up with the ship.
    pub budget_exhausted: bool,
}

/// One frame for one pane: push what is queued, collect what was asked for.
///
/// Does nothing at all while the document is still loading — pushing into a page
/// with no bridge is how a `Welcome` gets lost — and everything it moves goes
/// through [`PaneBus`], so a pane's traffic crosses the same admission and
/// projection boundaries whether its surface is Ultralight or a test double.
///
/// Pushes at most [`MAX_PUSHES_PER_FRAME`]; see the module note for why that
/// bound is the simulation's business rather than the pane's.
pub fn pump_pane(bus: &PaneBus, id: PaneId, surface: &mut dyn PaneSurface) -> PanePumpReport {
    let mut report = PanePumpReport::default();
    if !surface.is_ready() {
        return report;
    }
    bus.mark_live(id);

    let batch = bus.take_outbound(id);
    let mut deferred: Vec<PaneDispatch> = Vec::new();
    for (index, dispatch) in batch.iter().enumerate() {
        if report.pushed >= MAX_PUSHES_PER_FRAME {
            report.budget_exhausted = true;
            deferred.extend_from_slice(&batch[index..]);
            break;
        }
        match surface.push(&super::document::pane_apply_script(&dispatch.json)) {
            Ok(()) => report.pushed += 1,
            Err(e) => {
                report.push_failure = Some(e);
                deferred.extend_from_slice(&batch[index..]);
                break;
            }
        }
    }
    if !deferred.is_empty() {
        report.deferred = deferred.len();
        bus.requeue_front(id, deferred);
    }

    for record in surface.drain() {
        match bus.submit_json(id, &record) {
            Ok(()) => report.accepted += 1,
            Err(refusal) => report.refusals.push(refusal),
        }
    }
    report
}

/// A [`PaneSurface`] that records instead of rendering.
///
/// The frame loop's own test double: it never needs an SDK, so everything
/// [`pump_pane`] decides is checked by the ordinary `cargo test` CI runs rather
/// than only by a human on a Windows machine with a GPU.
#[derive(Debug, Default)]
pub struct RecordingSurface {
    /// Whether the document reports itself loaded.
    pub ready: bool,
    /// Every URL this surface was asked to load.
    pub loaded: Vec<String>,
    /// Every script that was successfully pushed.
    pub pushed: Vec<String>,
    /// Records handed back on the next [`PaneSurface::drain`].
    pub queued_records: Vec<String>,
    /// Number of pushes to fail before the first success. Models the window
    /// between "the document loaded" and "its modules have run".
    pub failing_pushes: usize,
}

impl RecordingSurface {
    /// A surface whose document is already loaded.
    pub fn ready() -> Self {
        Self {
            ready: true,
            ..Default::default()
        }
    }

    /// Queue a record for the page to hand back.
    pub fn queue_record(&mut self, record: impl Into<String>) {
        self.queued_records.push(record.into());
    }
}

impl PaneSurface for RecordingSurface {
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
        self.loaded.push(url.to_string());
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn push(&mut self, json: &str) -> Result<(), PaneSurfaceError> {
        if self.failing_pushes > 0 {
            self.failing_pushes -= 1;
            return Err(PaneSurfaceError::Script(
                "window.__phoenixPaneApply is not a function".to_string(),
            ));
        }
        self.pushed.push(json.to_string());
        Ok(())
    }

    fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.queued_records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{DeliveryClass, ServerMessage};
    use crate::lobby::handler::Target;
    use crate::native_host::panes::identity::PaneIdentity;
    use crate::native_host::transport::{NativeTransport, TransportDispatch, TransportEvent};

    fn bus_with_pane() -> (PaneBus, PaneId) {
        let bus = PaneBus::default();
        let id =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
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

    #[test]
    fn a_burst_of_snapshots_of_one_kind_reaches_the_page_as_one_push() {
        // The phone's lossy channel drops stale snapshots; the pane bus
        // coalesces them at push time (see `registry`), so a frame that
        // unpacked into many ticks costs the page one evaluation per kind
        // rather than a budget's worth of stale ones (issue #1403).
        let (bus, id) = bus_with_pane();
        for _ in 0..(MAX_PUSHES_PER_FRAME * 2) {
            broadcast_snapshot(&bus, ServerMessage::GameStarted);
        }
        broadcast_snapshot(&bus, ServerMessage::ShipDestroyed);
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
        // Every push is a synchronous evaluate_script on the Bevy main thread,
        // which is the thread the fixed simulation tick runs on — so an
        // unbounded frame is simulation stall for every participant on the
        // ship. The first frame after a document loads is exactly where an
        // unbounded one would happen: it drains the whole load-time backlog.
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
}
