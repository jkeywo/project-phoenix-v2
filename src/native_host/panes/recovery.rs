//! Recovering a failed pane as an ordinary participant disconnect (issue #1125).
//!
//! A local pane is "just another logical client" (the #1122 doctrine), so its
//! failure must ride the *same* disconnect → Backfill → reconnect machinery a
//! dropped phone does, not a native-only special case. This module is the small
//! amount of glue that routes a pane's fault into that path and, where the fault
//! warrants it, brings the pane back on the **same identity** — the in-process
//! analogue of the browser client's automatic redial (AGENTS.md "Getting on the
//! wire": "the same session token re-sent as `Identify`").
//!
//! Everything here is pure and Bevy-free: it operates on a [`PaneBus`], which is
//! the whole of how a pane talks to the simulation, so the fault-injection
//! acceptance criteria are checked by the ordinary `cargo test` CI runs rather
//! than only under the `ultralight` feature. The Ultralight half
//! ([`super::ultralight`]) does two things this module cannot: it *notices* a
//! real view crash, and it *builds the view* for a recreated pane. What a fault
//! then means — a disconnect, a Backfill, a reconnect on the same token — is all
//! here.
//!
//! # Why a crash recreates and an overflow does not
//!
//! Both faults close the pane, which is the whole of "the failed local
//! participant disconnects through the ordinary Session path" (issue #1125,
//! AC2): [`PaneBus::close`] owes the lobby one `PlayerDisconnected`, the station
//! keeps its holder and flips to `Backfill`, and — because
//! `SessionManager::holder_for_station` gates on `connected` — every audience
//! projection stops resolving to the failed token the instant it disconnects, so
//! no surviving or recreated pane inherits it (AC3).
//!
//! They differ in what comes *after* the close. A [`PaneFault::ViewCrashed`] is a
//! transient — the display is fine, the browser engine hiccuped — so the faithful
//! analogue of a network drop is to rebuild the view and let the page rejoin on
//! the same token, restoring its held station and current projection (AC4). A
//! [`PaneFault::ReliableOverflow`] is a page that stopped draining; recreating it
//! blindly would risk a rebuild loop against a page that wedges again, so that
//! one closes and stays closed — the operator decides. ([ai] decision:
//! auto-recreate is scoped to the crash case for exactly this reason.)
//!
//! # The crash recreate is BOUNDED, or it flaps forever
//!
//! Auto-recreate for a view crash is not unconditional. A view that *loads then
//! crashes* would otherwise be closed and rebuilt on every crash-detect cycle —
//! flapping its station between a human and Backfill roughly every half second,
//! forever, with no operator ever getting a stable console to repair. So
//! recreation is rate-limited **per identity** (issue #1125): after
//! [`MAX_RECREATIONS_PER_WINDOW`] rebuilds of one identity inside
//! [`RECREATION_WINDOW`] the fault is no longer treated as transient, and the
//! pane is left closed on Backfill for the operator — the same conclusion a
//! [`ReliableOverflow`](PaneFault::ReliableOverflow) reaches immediately. The
//! counter is keyed on the session token, which survives the [`PaneId`] change a
//! recreation makes, so every rebuild of a flapping view counts against the one
//! identity; a window rather than a lifetime cap so a pane that crashed once,
//! recovered and ran healthily is not denied a fresh recreation much later.

use std::time::Duration;

use super::registry::PaneId;
use super::transport::PaneBus;

/// The most times one pane identity is auto-recreated within [`RECREATION_WINDOW`]
/// before it is left closed for the operator (issue #1125).
///
/// See the [module note](self#the-crash-recreate-is-bounded-or-it-flaps-forever):
/// past this many rebuilds of the same identity in the window, a view crash is
/// not a transient worth rebuilding into — it flaps — so the pane stays closed on
/// Backfill, matching the reasoning that makes a reliable overflow refuse to
/// recreate at all. Not a gameplay tunable: a small fault-recovery bound.
pub const MAX_RECREATIONS_PER_WINDOW: u32 = 3;

/// The rolling window [`MAX_RECREATIONS_PER_WINDOW`] recreations are counted over
/// (issue #1125).
///
/// Wide enough to catch a ~0.5 s crash-detect flap — several cycles land well
/// inside it — but short enough that a pane which crashed once, recovered, and ran
/// for the rest of a long mission is granted a fresh recreation if it crashes
/// again later, its earlier crash having aged out of the window.
pub const RECREATION_WINDOW: Duration = Duration::from_secs(10);

/// Why a pane failed and its participant must disconnect.
///
/// A pane's fault is not a native-host error — it is a participant dropping, and
/// this names *which kind* so the recovery policy can tell a transient view
/// crash (rebuild it) from a wedged page (leave it closed). Both flip the
/// station to `Backfill`; that half is identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFault {
    /// The page stopped draining and no queued message can safely be discarded.
    /// The historical name also covers unsuperseded snapshot deltas and
    /// change-only complete projections; see [`super::registry`]'s cap note.
    ReliableOverflow,
    /// The pane's Ultralight view stopped answering — a crash, a lost surface, a
    /// frame copy that will not complete. The display itself is fine, so this is
    /// a transient the view can be rebuilt from.
    ViewCrashed,
}

impl PaneFault {
    /// A one-line reason for the operator log.
    ///
    /// Operator diagnostics on the same footing as the rest of the pane host's
    /// log lines (inline English, not `strings.csv`): this is a file, a
    /// scrollback and a screenshot for whoever is running the bridge, never
    /// player-visible console text.
    pub fn reason(&self) -> &'static str {
        match self {
            PaneFault::ReliableOverflow => {
                "stopped draining its console and overflowed its state backlog"
            }
            PaneFault::ViewCrashed => "its console view stopped answering (crash or lost surface)",
        }
    }

    /// Whether a fault of this kind should recreate the pane, so the human
    /// reconnects on the same identity.
    ///
    /// True for a view crash (a transient — rebuild the view, the pane analogue
    /// of a network redial); false for a reliable overflow (a wedged page — leave
    /// it closed rather than risk a rebuild loop). A `true` here is *permission*
    /// to recreate, not a guarantee: [`service_faults`] still rate-limits the
    /// crash case per identity — see [`MAX_RECREATIONS_PER_WINDOW`].
    pub fn recreates(&self) -> bool {
        matches!(self, PaneFault::ViewCrashed)
    }
}

/// What servicing one faulted pane did — for the operator log.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultOutcome {
    /// The pane that failed and was closed.
    pub failed: PaneId,
    /// Why it failed.
    pub fault: PaneFault,
    /// The recreated pane and the URL its view should navigate to, when the
    /// fault warranted a fresh view on the same identity. `None` for a fault that
    /// only closes, and `None` too when the bus was never armed for recreation
    /// (a test with no HTTP server — the new pane still opens, but with no URL).
    pub recreated: Option<(PaneId, String)>,
    /// True when the fault *would* have recreated (a view crash) but the identity
    /// has flapped past [`MAX_RECREATIONS_PER_WINDOW`], so it was deliberately left
    /// closed on Backfill for the operator instead. Distinguishes that give-up
    /// from an overflow (which never recreates) in the operator log.
    pub recreation_exhausted: bool,
}

/// Close every pane the bus reported faulted, and recreate the ones whose fault
/// warrants it.
///
/// This is the seam-level whole of issue #1125's fault handling, drivable from a
/// test with no SDK, no GPU and no window — which is where its acceptance
/// criteria live. [`super::ultralight::drive_pane_host`] calls it once per frame
/// after it has *detected* faults (a page over its reliable budget, a view that
/// stopped answering); a CI test calls it after injecting one through
/// [`PaneBus::fault`].
///
/// Closing hands the lobby the ordinary `PlayerDisconnected`; recreating opens a
/// fresh pane on the same identity and enqueues its view for the Ultralight host
/// to build (see [`PaneBus::recreate`] / [`PaneBus::take_pending_views`]). The
/// close comes first so the disconnect is queued ahead of any reconnect
/// `Identify` the recreated pane will later send.
pub fn service_faults(bus: &PaneBus) -> Vec<FaultOutcome> {
    let mut outcomes = Vec::new();
    for (failed, fault) in bus.take_faulted() {
        // Close before recreate: `close` owes the lobby its disconnect now, and
        // the recreated pane's `Identify` cannot be polled until its view has
        // loaded — frames later — so the ordinary disconnect-then-reconnect
        // order is preserved. `recreate` reads the failed pane's identity from
        // its (now `Closed`) record, which the registry keeps.
        let mut recreated = None;
        let mut recreation_exhausted = false;
        if fault.recreates() {
            // Bounded recreation (issue #1125): a view that loads-then-crashes
            // would flap its station between human and Backfill forever. Past the
            // per-identity budget, stop rebuilding into the same crash and leave
            // the pane closed for the operator — the ReliableOverflow conclusion.
            if bus.record_recreation_within_budget(failed) {
                recreated = bus.rebuild(failed);
            } else {
                recreation_exhausted = true;
            }
        }
        if recreated.is_none() {
            bus.close(failed);
        }
        outcomes.push(FaultOutcome {
            failed,
            fault,
            recreated,
            recreation_exhausted,
        });
    }
    outcomes
}

/// A dead renderer cannot recreate any view. Close every live bus entry,
/// including consoles whose creation was still pending, and consume queued
/// faults so the ordinary per-seat recovery cannot reopen them.
pub fn close_after_thread_failure(bus: &PaneBus) {
    for id in bus.open_pane_ids() {
        bus.fault(id, PaneFault::ViewCrashed);
        bus.close(id);
    }
    let _ = bus.take_faulted();
    let _ = bus.take_pending_views();
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
