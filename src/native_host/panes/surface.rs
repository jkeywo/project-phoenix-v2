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
//! # Each pane iteration has a bounded push budget
//!
//! A push is a synchronous `evaluate_script` into a browser engine. Since
//! issue #1404 it runs on the dedicated pane thread, so it no longer occupies
//! the simulation's thread. The budget still bounds each pane's share of an
//! iteration and prevents one loading console from delaying every other view.
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
/// Not a gameplay value and not a designer's knob: it bounds one console's
/// share of a pane-thread iteration inside the browser engine. Sized to keep a
/// steady-state pane (a snapshot and a handful of transitions per tick) well
/// clear of it, so the budget only ever bites on a backlog.
pub const MAX_PUSHES_PER_FRAME: usize = 32;

pub use phoenix_platform::surface::*;

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
    if bus.is_superseded(id) {
        // Keep the inert page's own queue bounded without treating an ordinary
        // connection handoff as a stream of page faults or admitting its input.
        surface.drain();
        return report;
    }
    bus.mark_live(id);
    if let Some(script) = bus.private_audio_script(id) {
        if surface.push(&script).is_ok() {
            bus.mark_private_audio_sent(id, script);
            report.pushed += 1;
        }
    }

    let replies = bus.take_operator_replies(id);
    for (index, reply) in replies.iter().enumerate() {
        if let Err(error) = surface.push(reply) {
            bus.requeue_operator_replies(id, replies[index..].to_vec());
            report.push_failure = Some(error);
            break;
        }
        report.pushed += 1;
    }

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
        if bus.submit_private_audio(id, &record) {
            continue;
        }
        if bus.submit_operator_record(id, &record) {
            continue;
        }
        match bus.submit_json(id, &record) {
            Ok(()) => report.accepted += 1,
            Err(refusal) => report.refusals.push(refusal),
        }
    }
    report
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
