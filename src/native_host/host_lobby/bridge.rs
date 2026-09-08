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
//! (`session_connections::ConnectionRegistry`) that would have to answer "which participant is
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
//! lobby, pushed every frame the lobby changes; the scenario-panel state is a
//! snapshot of the whole picker (issue #1328); the monitor row is a snapshot of
//! the whole bridge layout (issue #1330); and the reveal flag is a current
//! state rather than an edge — so an older value has nothing to say that the
//! newest one does not, and holding a backlog of them would only cost
//! main-thread time inside a browser engine (see [`super::super::panes::surface`]'s
//! note on why that time is the *simulation's*). Seven latest-wins slots — the
//! reveal, the join invitation, the landing screen, the mod-pack shelf, the
//! picker, the monitor row and the lobby state — at most one push each a frame.
//!
//! The one exception is the QR toggle, which is an *edge* and is counted rather
//! than collapsed — see [`HostLobbyBridge::push_qr_toggle`].
//!
//! A push that fails is still not lost: it goes back in its slot unless
//! something newer has already taken it, which is the same "the page's modules
//! have not run yet" case `pump_pane` handles and the same answer — retry next
//! frame with the value that is current *then*.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::document::{
    host_lobby_apply_script, host_lobby_join_script, host_lobby_landing_script,
    host_lobby_layout_script, host_lobby_packs_script, host_lobby_qr_toggle_script,
    host_lobby_reveal_script, host_lobby_scenario_script,
};
use crate::native_host::panes::{PaneSurface, PaneSurfaceError};

/// How many records the page may queue before the oldest are dropped.
///
/// The page→host direction carries an operator's presses — a scenario, a hull,
/// an AI launch (issue #1328), a monitor for the viewscreen (issue #1330) —
/// which arrive at human speed and are drained every frame. So this bounds a
/// surface talking to a host that has stopped listening, which is a bug rather
/// than load, and the honest response to it is to keep the newest evidence and
/// not to grow.
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
    /// The newest join invitation not yet handed to the page (issue #1329).
    ///
    /// Latest-wins like the lobby state, and for the same reason: an invitation
    /// is a statement of where the crew should go NOW, and a reclaimed or
    /// rotated code makes the previous one wrong rather than merely older.
    join: Option<String>,
    /// The newest scenario-panel state not yet handed to the page (issue
    /// #1328).
    ///
    /// Latest-wins like the lobby state, and for the same reason: it is a
    /// snapshot of the whole picker — the catalogue, what the arbiter has
    /// locked, and whether a world has landed and closed the picker for good —
    /// so an older one has nothing to say the newest does not.
    ///
    /// Unlike [`Self::payload`] it carries **no dedupe**, because its feed does
    /// not push every frame: `host_lobby::feed_scenario_panel` pushes only when
    /// the selection moved or a world arrived, exactly as `join` is pushed only
    /// when a code is issued.
    scenario: Option<String>,
    /// The newest landing-screen state not yet handed to the page (issue
    /// #1361).
    ///
    /// Latest-wins like the picker beside it, and carrying no dedupe for the
    /// same reason: its feed pushes on the first frame and on the frame a World
    /// lands, and nothing in between.
    landing: Option<String>,
    /// The newest mod-pack shelf not yet handed to the page (issue #1366).
    ///
    /// Latest-wins like the landing above it, and carrying no dedupe for the
    /// same reason: its feed pushes on the first frame and on the frame an
    /// install attempt finished, and nothing in between.
    packs: Option<String>,
    /// QR toggles asked for but not yet applied (issue #1329).
    ///
    /// A COUNT, not a flag, because this is the one thing on this bridge that is
    /// an edge rather than a state: the page owns whether the panel is on
    /// screen (`gui/host-qr.js` reads `#overlay`), and what crosses is "somebody
    /// pressed the button". Two presses in a frame are two flips — collapsing
    /// them to "somebody asked" would turn a double-press into a single one.
    qr_toggles: usize,
    /// The newest encoded `BridgeLayoutPayload` not yet handed to the page
    /// (issue #1330).
    layout: Option<String>,
    /// The newest monitor row this bridge has ACCEPTED. Deduped for the same
    /// reason `last_accepted` is: the row is republished whenever the layout
    /// resource is touched, and a bridge nobody rearranged must cost the
    /// simulation nothing.
    last_accepted_layout: Option<String>,
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

    /// Hand the surface the crew's join invitation
    /// ([`super::join::JoinInvite`], already encoded).
    pub fn push_join(&self, json: impl Into<String>) {
        self.lock().join = Some(json.into());
    }

    /// Hand the surface the scenario picker's state
    /// ([`super::scenario::ScenarioPanelPayload`], already encoded) — issue
    /// #1328.
    pub fn push_scenario(&self, json: impl Into<String>) {
        self.lock().scenario = Some(json.into());
    }

    /// Hand the surface the landing screen's state
    /// ([`super::landing::LandingPanelPayload`], already encoded) - issue
    /// #1361.
    pub fn push_landing(&self, json: impl Into<String>) {
        self.lock().landing = Some(json.into());
    }

    /// Hand the surface the mod-pack shelf
    /// ([`super::packs::ModPackPanelPayload`], already encoded) - issue #1366.
    pub fn push_packs(&self, json: impl Into<String>) {
        self.lock().packs = Some(json.into());
    }

    /// Somebody asked for the join QR to be flipped (issue #1329).
    ///
    /// A phone's `ClientMessage::ToggleQrCode`, arriving over the relay at a
    /// host with no page in front of its simulation. The surface's OWN control
    /// does not come through here — it is a click inside the document, on the
    /// state the document already owns, and a round trip through the host would
    /// only add a frame of latency to a decision nobody else needs to know.
    pub fn push_qr_toggle(&self) {
        self.lock().qr_toggles += 1;
    }

    /// Hand the surface the current monitor row (issue #1330) — an encoded
    /// [`BridgeLayoutPayload`](super::layout::BridgeLayoutPayload).
    ///
    /// Deduped exactly as [`push_lobby_state`](Self::push_lobby_state) is, and
    /// for the same reason: the row is republished from a resource the applier
    /// touches, and a bridge nobody rearranged must not spend the simulation's
    /// thread re-rendering an unchanged button row.
    pub fn push_layout(&self, json: impl Into<String>) {
        let json = json.into();
        let mut inner = self.lock();
        if inner.last_accepted_layout.as_deref() == Some(json.as_str()) {
            return;
        }
        inner.last_accepted_layout = Some(json.clone());
        inner.layout = Some(json);
    }

    /// Whether anything is waiting to be pushed. Diagnostic, and what a test
    /// asserts on to show that a failed push was kept.
    pub fn has_pending(&self) -> bool {
        let inner = self.lock();
        inner.payload.is_some()
            || inner.reveal.is_some()
            || inner.join.is_some()
            || inner.scenario.is_some()
            || inner.landing.is_some()
            || inner.packs.is_some()
            || inner.layout.is_some()
            || inner.qr_toggles > 0
    }

    /// Take everything the surface has asked for since the last call, in order.
    ///
    /// This carries the operator's scenario and hull picks and their AI-launch
    /// press (issue #1328) and their monitor presses (issue #1330) — see
    /// [`super::HostLobbyRecord`], the one vocabulary all four are in, which
    /// `host_lobby::drain_surface_records` decodes them into. Drained
    /// unconditionally, so a surface talking to a host that has stopped
    /// listening cannot fill memory.
    ///
    /// # One reader, and it has to stay one
    ///
    /// This is a `drain`: what it returns, nobody else will see. So the surface
    /// gets exactly ONE consumer — `drain_surface_records`, in `PreUpdate` —
    /// and every kind of record is a variant it dispatches rather than a second
    /// system reading the same queue. A second `take_records` caller would not
    /// fail loudly: whichever ran first would swallow the other's records and
    /// warn about a vocabulary it does not speak, and the other would see an
    /// empty queue forever. That is the whole reason the picks and the monitor
    /// row share an enum instead of having one each.
    pub fn take_records(&self) -> Vec<String> {
        self.lock().records.drain(..).collect()
    }

    /// Host-local recovery uses the same typed layout record and one reader as
    /// a lobby button. It never edits layout state from a second dispatch path.
    pub(crate) fn submit_record(&self, record: &super::HostLobbyRecord) -> bool {
        match crate::core::codec::encode_host_lobby_record(record) {
            Ok(json) => {
                self.record(&json);
                true
            }
            Err(_) => false,
        }
    }

    fn record(&self, json: &str) {
        let mut inner = self.lock();
        if inner.records.len() >= RECORD_CAP {
            inner.records.pop_front();
        }
        inner.records.push_back(json.to_string());
    }

    /// Take everything pending, leaving every slot empty.
    fn take_pending(&self) -> Pending {
        let mut inner = self.lock();
        Pending {
            reveal: inner.reveal.take(),
            join: inner.join.take(),
            scenario: inner.scenario.take(),
            landing: inner.landing.take(),
            packs: inner.packs.take(),
            layout: inner.layout.take(),
            payload: inner.payload.take(),
            qr_toggles: std::mem::take(&mut inner.qr_toggles),
        }
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

    fn restore_join(&self, json: String) {
        let mut inner = self.lock();
        if inner.join.is_none() {
            inner.join = Some(json);
        }
    }

    fn restore_scenario(&self, json: String) {
        let mut inner = self.lock();
        if inner.scenario.is_none() {
            inner.scenario = Some(json);
        }
    }

    fn restore_landing(&self, json: String) {
        let mut inner = self.lock();
        if inner.landing.is_none() {
            inner.landing = Some(json);
        }
    }

    fn restore_packs(&self, json: String) {
        let mut inner = self.lock();
        if inner.packs.is_none() {
            inner.packs = Some(json);
        }
    }

    fn restore_layout(&self, json: String) {
        let mut inner = self.lock();
        if inner.layout.is_none() {
            inner.layout = Some(json);
        }
    }

    /// Give back toggles that were not delivered.
    ///
    /// Added rather than replaced, unlike every other slot here: these are
    /// edges, and one that failed to cross plus one that arrived while it was
    /// failing are two presses, both of which the operator made.
    fn restore_qr_toggles(&self, count: usize) {
        self.lock().qr_toggles += count;
    }
}

/// One frame's worth of everything waiting to cross.
struct Pending {
    reveal: Option<bool>,
    join: Option<String>,
    landing: Option<String>,
    packs: Option<String>,
    scenario: Option<String>,
    layout: Option<String>,
    payload: Option<String>,
    qr_toggles: usize,
}

/// What one frame of [`pump_host_lobby`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostLobbyPumpReport {
    /// Scripts handed to the surface — the reveal, the invitation, the landing
    /// screen, the mod-pack shelf, the picker, the monitor row and the lobby
    /// state, plus one for every QR toggle.
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
/// The order within a frame is the order the page has to hear them in:
///
/// 1. **the reveal**, before the state whose meaning it changes, so a frame
///    carrying both paints once with both answers rather than painting the
///    phase's answer and then correcting it;
/// 2. **the join invitation**, before the state that decides whether the panel
///    carrying it is on screen;
/// 3. **the landing screen** (issue #1361), before the picker it reveals: of
///    the three panels that cover one another — lobby, then picker, then
///    landing — it is the outermost, and the same "decide the covering first"
///    rule that puts the picker before the lobby puts the front door before
///    both. (The join overlay is a fourth panel and is not in that stack:
///    `document::GROUND_CSS` lifts it clear of all three, so nothing in this
///    order decides what covers it — which is why its push sits at 2 for an
///    unrelated reason;)
/// 4. **the mod-pack shelf** (issue #1366), which is a STAGE INSIDE the landing
///    — one of the panels `#landing-mid` holds — so it goes after the landing it
///    is drawn in and before everything the landing covers. A frame carrying
///    both a dismissal and a shelf paints the front door's absence once, rather
///    than filling a panel on a screen that is going away;
/// 5. **the scenario picker** (issue #1328), before the lobby it covers: the
///    picker is a full-screen panel over the crew lobby, so a frame that both
///    closes it and fills the lobby behind it decides the covering first. No
///    element is written by both renderers, so nothing here can clobber
///    anything — the order is fixed and stated so it stays that way;
/// 6. **the monitor row** (issue #1330), for the same reason the five above it
///    go first: the state push is what repaints the whole lobby, so the row has
///    to be in place when it lands rather than corrected after it;
/// 7. **the lobby state**, which carries the phase, and with it the join
///    panel's show/hide law;
/// 8. **QR toggles last**, because an operator's press is their answer to the
///    phase, not the other way round. Applied before the state, a toggle in the
///    same frame as a `Lobby` push would be silently overwritten by it.
///
/// The rule the list encodes: **every snapshot before the state that repaints
/// around it, and the one edge after it.** A slot added later goes with the
/// snapshots unless it is an edge, in which case it joins the toggles.
///
/// (`host_lobby_boot.js` renders nothing until it has something to render, so a
/// lone reveal, a lone picker, a lone shelf, a lone monitor row or a lone toggle
/// on the first frame is free.)
pub fn pump_host_lobby(
    bridge: &HostLobbyBridge,
    surface: &mut dyn PaneSurface,
) -> HostLobbyPumpReport {
    let mut report = HostLobbyPumpReport::default();
    if !surface.is_ready() {
        return report;
    }

    let pending = bridge.take_pending();
    // Once one push has thrown, the page has no bridge yet and the rest of the
    // frame would throw identically — replacing the reported failure with a
    // copy of itself and spending an `evaluate_script` call per remaining slot
    // on the simulation's own thread. Everything after it is deferred instead.
    let mut failed = false;

    if let Some(flag) = pending.reveal {
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

    if let Some(json) = pending.join {
        if failed {
            report.deferred += 1;
            bridge.restore_join(json);
        } else {
            match surface.push(&host_lobby_join_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_join(json);
                    failed = true;
                }
            }
        }
    }

    if let Some(json) = pending.landing {
        if failed {
            report.deferred += 1;
            bridge.restore_landing(json);
        } else {
            match surface.push(&host_lobby_landing_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_landing(json);
                    failed = true;
                }
            }
        }
    }

    if let Some(json) = pending.packs {
        if failed {
            report.deferred += 1;
            bridge.restore_packs(json);
        } else {
            match surface.push(&host_lobby_packs_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_packs(json);
                    failed = true;
                }
            }
        }
    }

    if let Some(json) = pending.scenario {
        if failed {
            report.deferred += 1;
            bridge.restore_scenario(json);
        } else {
            match surface.push(&host_lobby_scenario_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_scenario(json);
                    failed = true;
                }
            }
        }
    }

    if let Some(json) = pending.layout {
        if failed {
            report.deferred += 1;
            bridge.restore_layout(json);
        } else {
            match surface.push(&host_lobby_layout_script(&json)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    bridge.restore_layout(json);
                    failed = true;
                }
            }
        }
    }

    if let Some(json) = pending.payload {
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
                    failed = true;
                }
            }
        }
    }

    if pending.qr_toggles > 0 {
        if failed {
            report.deferred += pending.qr_toggles;
            bridge.restore_qr_toggles(pending.qr_toggles);
        } else {
            for applied in 0..pending.qr_toggles {
                match surface.push(&host_lobby_qr_toggle_script()) {
                    Ok(()) => report.pushed += 1,
                    Err(e) => {
                        report.push_failure = Some(e);
                        let unsent = pending.qr_toggles - applied;
                        report.deferred += unsent;
                        bridge.restore_qr_toggles(unsent);
                        break;
                    }
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
    fn the_join_invitation_crosses_before_the_state_that_decides_it_is_on_screen() {
        // Issue #1329. Both in one frame must paint once: the panel's contents
        // before the phase law that shows or hides the panel.
        let bridge = HostLobbyBridge::new();
        bridge.push_join(r#"{"kind":"code","code":"ABCDE"}"#);
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 2);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyJoin("));
        assert!(surface.pushed[0].contains("ABCDE"));
        assert!(surface.pushed[1].contains("__phoenixHostLobbyApply("));
    }

    #[test]
    fn a_newer_invitation_replaces_the_one_that_had_not_crossed_yet() {
        // A rotated or reclaimed code makes the previous one WRONG, not merely
        // older: a snapshot, like the lobby state beside it.
        let bridge = HostLobbyBridge::new();
        bridge.push_join(r#"{"kind":"off"}"#);
        bridge.push_join(r#"{"kind":"code","code":"ABCDE"}"#);
        let mut surface = RecordingSurface::ready();

        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 1);
        assert!(surface.pushed[0].contains("ABCDE"));
    }

    #[test]
    fn the_picker_crosses_before_the_lobby_it_covers() {
        // Issue #1328. `#scenario-panel` is a full-screen panel over the crew
        // lobby, so a frame that both closes the picker and fills the lobby
        // behind it decides the covering first.
        let bridge = HostLobbyBridge::new();
        bridge.push_scenario(r#"{"scenarios":[],"locked":true}"#);
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 2);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyScenario("));
        assert!(surface.pushed[1].contains("__phoenixHostLobbyApply("));
    }

    #[test]
    fn a_newer_picker_state_replaces_the_one_that_had_not_crossed_yet() {
        // A snapshot of the whole picker, like the lobby state beside it: once
        // the arbiter has locked a scenario, the state that said it was open is
        // WRONG rather than merely older.
        let bridge = HostLobbyBridge::new();
        bridge.push_scenario(r#"{"locked_scenario":null}"#);
        bridge.push_scenario(r#"{"locked_scenario":"combat_test"}"#);
        let mut surface = RecordingSurface::ready();

        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 1);
        assert!(surface.pushed[0].contains("combat_test"));
    }

    #[test]
    fn a_picker_state_that_could_not_cross_is_kept_and_defers_the_lobby_behind_it() {
        // The window between "the document loaded" and "its module island ran".
        // Dropping here would leave the viewscreen showing a picker that cannot
        // be clicked until the next time somebody happened to change the
        // selection — which on a fresh `--lobby` host is never.
        let bridge = HostLobbyBridge::new();
        bridge.push_scenario(r#"{"locked_scenario":null}"#);
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;

        let first = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(first.pushed, 0);
        assert_eq!(first.deferred, 2);
        assert!(bridge.has_pending());

        let second = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(second.pushed, 2, "nothing was lost across the two frames");
        assert!(surface.pushed[0].contains("__phoenixHostLobbyScenario("));
    }

    #[test]
    fn a_phones_qr_toggle_is_applied_after_the_phase_it_is_answering() {
        // The operator's press is their answer to the phase, not the other way
        // round. Pushed before the state, a toggle in the same frame as a
        // `Lobby` push would be silently overwritten by the phase law.
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        bridge.push_qr_toggle();
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 2);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyApply("));
        assert_eq!(surface.pushed[1], "window.__phoenixHostLobbyQrToggle()");
    }

    #[test]
    fn two_presses_in_one_frame_are_two_flips_and_not_one() {
        // The one edge on this bridge. Collapsing them the way the snapshot
        // slots collapse would turn a double-press into a single one — and a
        // double-press is how an operator lands back where they started.
        let bridge = HostLobbyBridge::new();
        bridge.push_qr_toggle();
        bridge.push_qr_toggle();
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 2);
        assert_eq!(surface.pushed.len(), 2);
        assert!(!bridge.has_pending());
    }

    #[test]
    fn a_toggle_that_could_not_cross_is_kept_rather_than_swallowed() {
        // A press that vanished into a document still loading its modules is a
        // press the operator made and the room never saw.
        let bridge = HostLobbyBridge::new();
        bridge.push_qr_toggle();
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;

        let first = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(first.pushed, 0);
        assert_eq!(first.deferred, 1);
        assert!(bridge.has_pending());

        // …and a second press while it was failing is a SECOND flip, added to
        // the one held back rather than replacing it.
        bridge.push_qr_toggle();
        let second = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(second.pushed, 2);
    }

    #[test]
    fn what_the_surface_asks_for_is_collected_rather_than_dropped() {
        // Since issue #1328 the records are the operator's own scenario and
        // hull picks and their AI-launch press. This asserts only the pipe —
        // that what the page queued reaches a reader exactly once, in order —
        // because what the records MEAN is `HostLobbyRecord`'s, and what they
        // do is `drain_surface_records`'s.
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"force_start"}"#);

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(
            report.records,
            vec![r#"{"kind":"force_start"}"#.to_string()]
        );
        assert_eq!(
            bridge.take_records(),
            vec![r#"{"kind":"force_start"}"#.to_string()]
        );
        assert!(bridge.take_records().is_empty());
    }

    const ROW: &str = r#"{"monitors":[{"identity":"BRAVIA@3840x2160"}]}"#;
    const MOVED_ROW: &str = r#"{"monitors":[{"identity":"BenQ@1920x1080"}]}"#;

    #[test]
    fn the_monitor_row_rides_the_same_latest_wins_slot_the_lobby_state_does() {
        // A row is a snapshot of the whole bridge layout, so an older one has
        // nothing to say the newest does not (issue #1330).
        let bridge = HostLobbyBridge::new();
        bridge.push_layout(ROW);
        bridge.push_layout(MOVED_ROW);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 1);
        assert!(surface.pushed[0].starts_with("window.__phoenixHostLobbyLayout("));
        assert!(surface.pushed[0].contains("BenQ@1920x1080"));
    }

    #[test]
    fn an_unchanged_monitor_row_costs_the_simulation_nothing() {
        // The row is republished whenever the layout resource is touched, which
        // is every frame the applier looks at it. Re-pushing it would spend the
        // simulation's own thread rebuilding a button row nobody moved.
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        for _ in 0..10 {
            bridge.push_layout(ROW);
            pump_host_lobby(&bridge, &mut surface);
        }
        assert_eq!(surface.pushed.len(), 1);

        bridge.push_layout(MOVED_ROW);
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 2);
    }

    #[test]
    fn a_frame_carrying_all_three_paints_the_state_last() {
        // The state push is what repaints the whole lobby, so the reveal and
        // the row must already be in place when it lands — otherwise the
        // surface paints the phase's answer and then corrects itself twice.
        let bridge = HostLobbyBridge::new();
        bridge.push_reveal(true);
        bridge.push_layout(ROW);
        bridge.push_lobby_state(PLAYING);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 3);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyReveal('true')"));
        assert!(surface.pushed[1].contains("__phoenixHostLobbyLayout("));
        assert!(surface.pushed[2].contains("__phoenixHostLobbyApply("));
    }

    #[test]
    fn a_monitor_row_that_throws_keeps_itself_and_the_state_for_the_next_frame() {
        let bridge = HostLobbyBridge::new();
        bridge.push_layout(ROW);
        bridge.push_lobby_state(LOBBY);
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;

        let first = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(first.pushed, 0);
        assert_eq!(first.deferred, 2);
        assert!(first.push_failure.is_some());
        assert!(bridge.has_pending());

        let second = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(second.pushed, 2, "nothing was lost across the two frames");
        assert!(surface.pushed[0].contains("__phoenixHostLobbyLayout("));
        assert!(surface.pushed[1].contains(r#""phase":"Lobby""#));
    }

    #[test]
    fn a_frame_carrying_every_slot_pins_the_documented_pump_order() {
        // Every snapshot before the state that repaints around it, and the one
        // edge after it — the rule the doc comment on `pump_host_lobby` states,
        // asserted whole rather than pairwise, because the pairwise tests above
        // each leave the slots they do not mention free to drift.
        //
        // All EIGHT, since the picker (issue #1328) joined the row (issue
        // #1330), the landing (issue #1361) joined both and the mod-pack shelf
        // (issue #1366) joined all three: four slices adding a snapshot each is
        // exactly the situation the stated rule exists for, and an assertion one
        // slot short leaves the newest to be placed by whichever slice lands
        // next. The landing goes with the snapshots and ahead of the picker it
        // reveals, because of the three panels that cover one another — lobby,
        // picker, landing — it is the outermost; the shelf goes immediately
        // after it because it is a stage drawn INSIDE it. The join overlay sits
        // above all three (`document::GROUND_CSS`) and so is not placed by this
        // order at all.
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state(LOBBY);
        bridge.push_qr_toggle();
        bridge.push_layout(ROW);
        bridge.push_scenario(r#"{"scenarios":[],"locked":false}"#);
        bridge.push_landing(r#"{"build":"0.1.0","dismissed":false}"#);
        bridge.push_packs(r#"{"dir":"mods","offered":[]}"#);
        bridge.push_join(r#"{"kind":"code","code":"ABCDE"}"#);
        bridge.push_reveal(true);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);
        assert_eq!(report.pushed, 8);
        assert!(surface.pushed[0].contains("__phoenixHostLobbyReveal('true')"));
        assert!(surface.pushed[1].contains("__phoenixHostLobbyJoin("));
        assert!(surface.pushed[2].contains("__phoenixHostLobbyLanding("));
        assert!(surface.pushed[3].contains("__phoenixHostLobbyPacks("));
        assert!(surface.pushed[4].contains("__phoenixHostLobbyScenario("));
        assert!(surface.pushed[5].contains("__phoenixHostLobbyLayout("));
        assert!(surface.pushed[6].contains("__phoenixHostLobbyApply("));
        assert_eq!(surface.pushed[7], "window.__phoenixHostLobbyQrToggle()");
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
