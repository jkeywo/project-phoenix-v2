//! The host-lobby bridge: lobby state in, records out (issue #1325).
//!
//! The same two-direction shape [`panes::transport`] gives a pane — the host
//! pushes with `evaluate_script`, the page queues records the host drains once a
//! frame — over the same [`PaneSurface`] trait, so this loop is testable with no
//! SDK, no GPU and no window exactly as `pump_pane` is.
//!
//! [`panes::transport`]: crate::native_host::panes::transport
//!
//! # Why it is a bridge of its own and not another pane
//!
//! A pane is **a participant**: it has a minted session token, it sends
//! `Identify`, it claims a Station, and everything it says crosses command
//! admission (`PRD #1093`: in-process delivery may skip network serialisation
//! and nothing else). The lobby surface is none of that. It has no identity, it
//! sends nothing, and what it shows is what the host is *already* broadcasting
//! to every phone in the room.
//!
//! Putting it on the pane bus would have meant giving it a token to be refused
//! for, an entry in the registry whose `Welcome` nobody wants, and a projection
//! (`routing::pane_receives`) that would have to answer "which participant is
//! the viewscreen" — a question with no honest answer. So it gets its own
//! bridge, and the pane bus keeps meaning exactly one thing.
//!
//! # Latest-wins, not a queue
//!
//! `pump_pane` carries a *backlog* and is careful never to lose a message,
//! because `Welcome`/`StationAssigned`/`GameStarted` are one-shot transitions
//! and a pane that missed one sits in the lobby forever.
//!
//! Nothing here is one-shot. A `LobbyStatePayload` is a snapshot of the whole
//! lobby, pushed every frame the lobby changes, and the reveal flag is a
//! current state rather than an edge — so an older value has nothing to say
//! that the newest one does not, and holding a backlog of them would only cost
//! main-thread time inside a browser engine (see [`super::super::panes::surface`]'s
//! note on why that time is the *simulation's*). Two slots, latest wins, at most
//! two pushes a frame.
//!
//! A push that fails is still not lost: it goes back in its slot unless
//! something newer has already taken it, which is the same "the page's modules
//! have not run yet" case `pump_pane` handles and the same answer — retry next
//! frame with the value that is current *then*.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::document::{host_lobby_apply_script, host_lobby_reveal_script};
use crate::native_host::panes::{PaneSurface, PaneSurfaceError};

/// How many records the page may queue before the oldest are dropped.
///
/// Nothing rides the page→host direction in this slice, so this bounds a
/// surface that has started talking to a host that has stopped listening —
/// which is a bug rather than load, and the honest response to it is to keep
/// the newest evidence and not to grow.
const RECORD_CAP: usize = 256;

#[derive(Default)]
struct Inner {
    /// The newest encoded `LobbyStatePayload` not yet handed to the page.
    payload: Option<String>,
    /// The newest payload this bridge has ACCEPTED, whether or not it has been
    /// pushed yet. See [`HostLobbyBridge::push_lobby_state`] for why an
    /// identical one is dropped rather than queued.
    last_accepted: Option<String>,
    /// The newest reveal flag not yet handed to the page.
    reveal: Option<bool>,
    /// What the page has asked for, awaiting a reader.
    records: VecDeque<String>,
}

/// The host ↔ lobby-surface bridge.
///
/// Cloneable and internally synchronised, like `PaneBus`: the Bevy plugin holds
/// one clone as a resource and the frame loop holds another, and both run on the
/// main thread today. The lock is what keeps that an implementation detail
/// rather than a promise.
#[derive(Clone, Default)]
pub struct HostLobbyBridge {
    inner: Arc<Mutex<Inner>>,
}

impl HostLobbyBridge {
    /// A bridge with nothing pending.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand the surface the newest lobby state.
    ///
    /// The **exact JSON the web host's `lobby` channel carries**. Replaces
    /// whatever was pending — see the module note on latest-wins.
    ///
    /// A payload byte-identical to the last one accepted is **dropped**, and
    /// that is not an optimisation to be tidied away later. `push_lobby_state`
    /// in `server::viewscreen_border` writes a `LobbyStateChanged` every
    /// `Update`, changed or not — which is free for a browser channel that
    /// hands JS a string, and is not free here: every push is a synchronous
    /// `evaluate_script` into a browser engine on the Bevy main thread, which
    /// is the thread `FixedUpdate` runs `SimSet` on (AGENTS.md rule 7). Without
    /// this, a lobby nobody is touching would cost a JSON parse and a full
    /// station-grid rebuild sixty times a second, of the *simulation's* time,
    /// for the whole mission.
    pub fn push_lobby_state(&self, json: impl Into<String>) {
        let json = json.into();
        let mut inner = self.lock();
        if inner.last_accepted.as_deref() == Some(json.as_str()) {
            return;
        }
        inner.last_accepted = Some(json.clone());
        inner.payload = Some(json);
    }

    /// Hand the surface the current reveal decision
    /// ([`super::reveal::SurfacePresence::force_chrome`]).
    pub fn push_reveal(&self, force_chrome: bool) {
        self.lock().reveal = Some(force_chrome);
    }

    /// Whether anything is waiting to be pushed. Diagnostic, and what a test
    /// asserts on to show that a failed push was kept.
    pub fn has_pending(&self) -> bool {
        let inner = self.lock();
        inner.payload.is_some() || inner.reveal.is_some()
    }

    /// Take everything the surface has asked for since the last call, in order.
    ///
    /// Empty in this slice — the lobby is read-only — and drained anyway, so a
    /// surface that started sending would not silently fill memory.
    pub fn take_records(&self) -> Vec<String> {
        self.lock().records.drain(..).collect()
    }

    fn record(&self, json: &str) {
        let mut inner = self.lock();
        if inner.records.len() >= RECORD_CAP {
            inner.records.pop_front();
        }
        inner.records.push_back(json.to_string());
    }

    /// Take the pending pair, leaving both slots empty.
    fn take_pending(&self) -> (Option<bool>, Option<String>) {
        let mut inner = self.lock();
        (inner.reveal.take(), inner.payload.take())
    }

    /// Put a value back **only if nothing newer has arrived** while the push was
    /// being attempted. Newer always wins: this is a snapshot, not a backlog.
    fn restore_payload(&self, json: String) {
        let mut inner = self.lock();
        if inner.payload.is_none() {
            inner.payload = Some(json);
        }
    }

    fn restore_reveal(&self, flag: bool) {
        let mut inner = self.lock();
        if inner.reveal.is_none() {
            inner.reveal = Some(flag);
        }
    }
}

/// What one frame of [`pump_host_lobby`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostLobbyPumpReport {
    /// Scripts handed to the surface — at most two, the reveal and the state.
    pub pushed: usize,
    /// Values put back because a push failed.
    pub deferred: usize,
    /// Records the surface asked for.
    pub records: Vec<String>,
    /// The push failure that stopped this frame, if there was one.
    pub push_failure: Option<PaneSurfaceError>,
}

/// One frame for the lobby surface: push what is pending, collect what was
/// asked for.
///
/// Does nothing at all while the document is still loading — the boot script
/// that owns `window.__phoenixHostLobbyApply` has not run, so every push would
/// throw and the state it carried would have to be retried anyway.
///
/// The reveal is pushed **before** the state, so that a frame carrying both
/// paints once with both answers rather than painting the phase's answer and
/// then correcting it. (`host_lobby_boot.js` renders nothing until a payload has
/// arrived, so a lone reveal on the first frame is free.)
pub fn pump_host_lobby(
    bridge: &HostLobbyBridge,
    surface: &mut dyn PaneSurface,
) -> HostLobbyPumpReport {
    let mut report = HostLobbyPumpReport::default();
    if !surface.is_ready() {
        return report;
    }

    let (reveal, payload) = bridge.take_pending();
    let mut failed = false;

    if let Some(flag) = reveal {
        match surface.push(&host_lobby_reveal_script(flag)) {
            Ok(()) => report.pushed += 1,
            Err(e) => {
                report.push_failure = Some(e);
                report.deferred += 1;
                bridge.restore_reveal(flag);
                failed = true;
            }
        }
    }

    if let Some(json) = payload {
        // A failed reveal means the page has no bridge yet, so the state push
        // would fail too — and attempting it would replace the reported failure
        // with an identical second one. Defer it instead.
        if failed {
            report.deferred += 1;
            bridge.restore_payload(json);
        } else {
            match surface.push(&host_lobby_apply_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_payload(json);
                }
            }
        }
    }

    for record in surface.drain() {
        bridge.record(&record);
        report.records.push(record);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::panes::RecordingSurface;

    const LOBBY: &str = r#"{"phase":"Lobby","crew_count":0}"#;
    const PLAYING: &str = r#"{"phase":"InProgress","crew_count":3}"#;

    #[test]
    fn a_document_that_has_not_loaded_is_not_pushed_to_and_keeps_its_state() {
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::default();
        assert_eq!(
            pump_host_lobby(&bridge, &mut surface),
            HostLobbyPumpReport::default()
        );
        assert!(surface.pushed.is_empty());
        assert!(bridge.has_pending());

        surface.ready = true;
        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 1);
        assert!(surface.pushed[0].starts_with("window.__phoenixHostLobbyApply("));
        assert!(!bridge.has_pending());
    }

    #[test]
    fn only_the_newest_lobby_state_is_pushed_because_it_is_a_snapshot() {
        // The difference from `pump_pane`, and the reason for it: an older
        // lobby snapshot has nothing to say the newest one does not, and every
        // push is main-thread time inside a browser engine.
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        bridge.push_lobby_state(PLAYING);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 1);
        assert_eq!(surface.pushed.len(), 1);
        assert!(surface.pushed[0].contains(r#""phase":"InProgress""#));
        assert!(
            !surface.pushed[0].contains(r#""phase":"Lobby""#),
            "the superseded snapshot never reaches the page: {}",
            surface.pushed[0]
        );
    }

    #[test]
    fn a_quiet_frame_pushes_nothing_at_all() {
        // The common case, sixty times a second: the lobby has not changed, so
        // there is nothing to say and no script to evaluate.
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 0);
        assert!(surface.pushed.is_empty());
    }

    #[test]
    fn an_unchanged_lobby_costs_the_simulation_nothing() {
        // `viewscreen_border::push_lobby_state` writes a LobbyStateChanged
        // every Update whether or not anything moved. Every push here is a
        // synchronous evaluate_script on the thread FixedUpdate runs SimSet on,
        // so re-pushing an identical snapshot would spend the simulation's own
        // time re-rendering a lobby nobody touched, sixty times a second, for
        // the whole mission.
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        for _ in 0..10 {
            bridge.push_lobby_state(LOBBY);
            pump_host_lobby(&bridge, &mut surface);
        }
        assert_eq!(surface.pushed.len(), 1);

        // …and a real change still gets through.
        bridge.push_lobby_state(PLAYING);
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 2);
        assert!(surface.pushed[1].contains(r#""phase":"InProgress""#));
    }

    #[test]
    fn a_push_that_throws_keeps_its_state_for_the_next_frame() {
        // The window between "the document loaded" and "its module island has
        // run". Dropping here would leave the viewscreen showing an empty lobby
        // until the next time the roster happened to change.
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;

        let first = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(first.pushed, 0);
        assert_eq!(first.deferred, 1);
        assert!(first.push_failure.is_some());
        assert!(bridge.has_pending());

        let second = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(second.pushed, 1);
        assert!(surface.pushed[0].contains(r#""phase":"Lobby""#));
    }

    #[test]
    fn a_state_that_arrived_while_a_push_failed_is_not_overwritten_by_the_old_one() {
        // Restoring unconditionally would put a stale snapshot back over a
        // fresh one — the one way a latest-wins slot can go backwards.
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;
        pump_host_lobby(&bridge, &mut surface);

        bridge.push_lobby_state(PLAYING);
        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 1);
        assert!(surface.pushed[0].contains(r#""phase":"InProgress""#));
    }

    #[test]
    fn the_reveal_is_pushed_before_the_state_it_changes_the_meaning_of() {
        // Both in one frame must paint once with both answers, not paint the
        // phase's answer and then correct it.
        let bridge = HostLobbyBridge::new();
        bridge.push_reveal(true);
        bridge.push_lobby_state(PLAYING);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 2);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyReveal('true')"));
        assert!(surface.pushed[1].contains("__phoenixHostLobbyApply("));
    }

    #[test]
    fn a_failed_reveal_defers_the_state_rather_than_reporting_the_same_fault_twice() {
        // A reveal that threw means the page has no bridge yet, so the state
        // push cannot succeed either; attempting it would only overwrite the
        // failure being reported with an identical one.
        let bridge = HostLobbyBridge::new();
        bridge.push_reveal(true);
        bridge.push_lobby_state(PLAYING);
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 0);
        assert_eq!(report.deferred, 2);
        assert!(surface.pushed.is_empty());
        assert!(bridge.has_pending());

        let second = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(second.pushed, 2, "nothing was lost across the two frames");
    }

    #[test]
    fn what_the_surface_asks_for_is_collected_rather_than_dropped() {
        // Nothing sends in this slice. The drain exists so that the day
        // something does — a QR toggle, a layout row — the records are already
        // arriving somewhere a reader can be attached to, and a surface that
        // started talking to a deaf host cannot grow without bound.
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"hello"}"#);

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.records, vec![r#"{"kind":"hello"}"#.to_string()]);
        assert_eq!(
            bridge.take_records(),
            vec![r#"{"kind":"hello"}"#.to_string()]
        );
        assert!(bridge.take_records().is_empty());
    }

    #[test]
    fn a_surface_talking_to_a_host_that_never_reads_drops_its_oldest_records() {
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        for i in 0..(RECORD_CAP + 5) {
            surface.queue_record(format!("{{\"n\":{i}}}"));
        }
        pump_host_lobby(&bridge, &mut surface);
        let held = bridge.take_records();
        assert_eq!(held.len(), RECORD_CAP);
        assert_eq!(held[0], format!("{{\"n\":{}}}", 5), "the newest survive");
    }
}
