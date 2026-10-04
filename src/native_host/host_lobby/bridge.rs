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
//! # Presentation snapshots and ordered fleet traffic
//!
//! `pump_pane` carries a *backlog* and is careful never to lose a message,
//! because `Welcome`/`StationAssigned`/`GameStarted` are one-shot transitions
//! and a pane that missed one sits in the lobby forever.
//!
//! A `LobbyStatePayload` is a snapshot of the whole
//! lobby, pushed every frame the lobby changes; the scenario-panel state is a
//! snapshot of the whole picker (issue #1328); the monitor row is a snapshot of
//! the whole bridge layout (issue #1330); and the reveal flag is a current
//! state rather than an edge — so an older value has nothing to say that the
//! newest one does not, and holding a backlog of them would only cost
//! main-thread time inside a browser engine (see [`super::super::panes::surface`]'s
//! note on why that time is the *simulation's*). Eight latest-wins lanes — the
//! reveal, the join invitation, the landing screen, the mod-pack shelf, the
//! picker, the monitor row, audio settings and the lobby state — at most one
//! push each a frame.
//!
//! The QR toggle is an *edge* and is counted rather
//! than collapsed — see [`HostLobbyBridge::push_qr_toggle`].
//! Fleet updates and wire frames have separate ordered, bounded queues: updates
//! carry one-shot join/launch requests and drained simulation frames. Overflow
//! faults the fleet explicitly; it must never discard those records as stale paint.
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
const AUDIO_LANE: usize = 6;

/// Latest-value lanes in their page-observable delivery order.
#[derive(Clone, PartialEq)]
enum Projection {
    Reveal(bool),
    Join(String),
    Landing(String),
    Packs(String),
    Scenario(String),
    Layout(String),
    Audio(String),
    Lobby(String),
}

impl Projection {
    fn lane(&self) -> usize {
        match self {
            Self::Reveal(_) => 0,
            Self::Join(_) => 1,
            Self::Landing(_) => 2,
            Self::Packs(_) => 3,
            Self::Scenario(_) => 4,
            Self::Layout(_) => 5,
            Self::Audio(_) => AUDIO_LANE,
            Self::Lobby(_) => 7,
        }
    }

    fn script(&self) -> String {
        match self {
            Self::Reveal(flag) => host_lobby_reveal_script(*flag),
            Self::Join(json) => host_lobby_join_script(json),
            Self::Landing(json) => host_lobby_landing_script(json),
            Self::Packs(json) => host_lobby_packs_script(json),
            Self::Scenario(json) => host_lobby_scenario_script(json),
            Self::Layout(json) => host_lobby_layout_script(json),
            Self::Audio(json) => super::document::host_lobby_audio_script(json),
            Self::Lobby(json) => host_lobby_apply_script(json),
        }
    }
}

/// Snapshots are replaced, never queued. Only frame-fed lanes deduplicate;
/// the others keep their existing event-fed publication behavior.
#[derive(Default)]
struct ProjectionMailbox {
    pending: [Option<Projection>; 8],
    last_accepted: [Option<Projection>; 8],
}

impl ProjectionMailbox {
    fn publish(&mut self, value: Projection) {
        let lane = value.lane();
        if matches!(
            value,
            Projection::Lobby(_) | Projection::Layout(_) | Projection::Audio(_)
        ) {
            if self.last_accepted[lane].as_ref() == Some(&value) {
                return;
            }
            self.last_accepted[lane] = Some(value.clone());
        }
        self.pending[lane] = Some(value);
    }

    fn restore(&mut self, value: Projection) {
        // A publisher may have filled this lane while the surface was pushing.
        let lane = value.lane();
        self.pending[lane].get_or_insert(value);
    }

    fn take(&mut self) -> [Option<Projection>; 8] {
        std::mem::take(&mut self.pending)
    }

    fn has_pending(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    fn republish_audio(&mut self) {
        self.pending[AUDIO_LANE] = self.last_accepted[AUDIO_LANE].clone();
    }
}

#[derive(Default)]
struct Inner {
    projections: ProjectionMailbox,
    /// Counted edges: two QR presses must remain two flips.
    qr_toggles: usize,
    records: VecDeque<String>,
    /// Reliable overflow is terminal until the surface replaces its bridge.
    fleet_faulted: bool,
    hud_reading: Option<String>,
    hud_locale: Option<String>,
    fleet_config: Option<String>,
    fleet_updates: VecDeque<String>,
    fleet_wire: VecDeque<String>,
    hud_os_defaults: crate::native_host::panes::os_prefs::OsAccessibilityPrefs,
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

    /// The HUD has its own document; retain its current reading preferences
    /// beside this endpoint's existing bridge, independently of lobby pumping.
    pub fn set_hud_presentation(
        &self,
        record: &crate::native_host::viewscreen_presentation::ViewscreenPresentation,
        os: Option<crate::native_host::panes::os_prefs::OsAccessibilityPrefs>,
    ) {
        let mut inner = self.lock();
        if let Some(os) = os {
            inner.hud_os_defaults = os;
        }
        inner.hud_reading = Some(
            crate::native_host::viewscreen_presentation::hud_reading_script(
                record,
                &inner.hud_os_defaults,
            ),
        );
    }

    pub fn hud_reading_script(&self) -> Option<String> {
        let inner = self.lock();
        if inner.hud_reading.is_none() && inner.hud_locale.is_none() {
            return None;
        }
        Some(format!(
            "{}{}",
            inner.hud_reading.as_deref().unwrap_or(""),
            inner.hud_locale.as_deref().unwrap_or("")
        ))
    }

    pub fn set_hud_locale(&self, locale: Option<&str>) {
        self.lock().hud_locale = Some(format!(
            "window.__phoenixSetHudLocale?.({});",
            crate::native_host::viewscreen_locale::locale_script_value(locale),
        ));
    }

    /// Used by the typed SetPresentation handler before its best-effort file
    /// write, so a storage failure still leaves the whole live Viewscreen legible.
    pub fn apply_hud_presentation_record(
        &self,
        record: &super::HostLobbyRecord,
    ) -> Option<crate::native_host::viewscreen_presentation::ViewscreenPresentation> {
        let super::HostLobbyRecord::SetPresentation {
            text_scale_percent,
            contrast,
            shake_percent,
            flash_percent,
            decorative_motion_percent,
        } = record
        else {
            return None;
        };
        let presentation = crate::native_host::viewscreen_presentation::ViewscreenPresentation {
            text_scale_percent: *text_scale_percent,
            contrast: *contrast,
            shake_percent: *shake_percent,
            flash_percent: *flash_percent,
            decorative_motion_percent: *decorative_motion_percent,
        }
        .sanitised();
        self.set_hud_presentation(&presentation, None);
        Some(presentation)
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
        self.lock()
            .projections
            .publish(Projection::Lobby(json.into()));
    }

    /// Hand the surface the current reveal decision
    /// ([`super::reveal::SurfacePresence::force_chrome`]).
    pub fn push_reveal(&self, force_chrome: bool) {
        self.lock()
            .projections
            .publish(Projection::Reveal(force_chrome));
    }

    /// Hand the surface the crew's join invitation
    /// ([`super::join::JoinInvite`], already encoded).
    pub fn push_join(&self, json: impl Into<String>) {
        self.lock()
            .projections
            .publish(Projection::Join(json.into()));
    }

    /// Hand the surface the scenario picker's state
    /// ([`super::scenario::ScenarioPanelPayload`], already encoded) — issue
    /// #1328.
    pub fn push_scenario(&self, json: impl Into<String>) {
        self.lock()
            .projections
            .publish(Projection::Scenario(json.into()));
    }

    /// Hand the surface the landing screen's state
    /// ([`super::landing::LandingPanelPayload`], already encoded) - issue
    /// #1361.
    pub fn push_landing(&self, json: impl Into<String>) {
        self.lock()
            .projections
            .publish(Projection::Landing(json.into()));
    }

    /// Hand the surface the mod-pack shelf
    /// ([`super::packs::ModPackPanelPayload`], already encoded) - issue #1366.
    pub fn push_packs(&self, json: impl Into<String>) {
        self.lock()
            .projections
            .publish(Projection::Packs(json.into()));
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
        self.lock()
            .projections
            .publish(Projection::Layout(json.into()));
    }

    /// Whether anything is waiting to be pushed. Diagnostic, and what a test
    /// asserts on to show that a failed push was kept.
    pub fn has_pending(&self) -> bool {
        let inner = self.lock();
        inner.fleet_config.is_some()
            || !inner.fleet_updates.is_empty()
            || !inner.fleet_wire.is_empty()
            || inner.projections.has_pending()
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
        match crate::core::codec::to_json(record) {
            Ok(json) => {
                self.record(&json);
                true
            }
            Err(_) => false,
        }
    }

    fn record(&self, json: &str) {
        let mut inner = self.lock();
        let incoming_fleet = is_fleet_record(json);
        if incoming_fleet && inner.fleet_faulted {
            return;
        }
        if inner.records.len() >= RECORD_CAP {
            // Fleet records are the reliable native control-plane lane. They
            // may share this typed bridge, but they may not inherit the
            // operator-button queue's oldest-wins shedding rule: losing one
            // mesh frame can deadlock a lockstep barrier. Prefer shedding an
            // ordinary UI record; a queue consisting only of fleet records is
            // already bounded upstream by the rendezvous reliable budget and
            // is drained on the next PreUpdate.
            if let Some(index) = inner.records.iter().position(|raw| !is_fleet_record(raw)) {
                inner.records.remove(index);
            } else if incoming_fleet {
                // Reliable records cannot be shed. Convert the whole doomed
                // backlog into one explicit terminal fault; the fleet system
                // closes the Rust socket when it consumes this record.
                inner.records.clear();
                inner.fleet_faulted = true;
                inner.records.push_back(
                    r#"{"kind":"fleet_fault","reason":"bridge-overflow","detail":"native fleet reliable bridge capacity exceeded"}"#.to_string(),
                );
                return;
            } else if !incoming_fleet {
                return;
            }
        }
        inner.records.push_back(json.to_string());
    }

    /// Take everything pending, leaving every slot empty.
    fn take_pending(&self) -> Pending {
        let mut inner = self.lock();
        Pending {
            fleet_config: inner.fleet_config.take(),
            fleet_updates: std::mem::take(&mut inner.fleet_updates),
            fleet_wire: std::mem::take(&mut inner.fleet_wire),
            projections: inner.projections.take(),
            qr_toggles: std::mem::take(&mut inner.qr_toggles),
        }
    }

    /// Current settings/status only. This lane never carries playback events.
    pub fn push_audio(&self, json: String) {
        self.lock().projections.publish(Projection::Audio(json));
    }
    pub fn push_fleet_config(&self, json: impl Into<String>) {
        self.lock().fleet_config = Some(json.into());
    }
    pub fn push_fleet_update(&self, json: impl Into<String>) {
        let mut inner = self.lock();
        if inner.fleet_faulted {
            return;
        }
        if inner.fleet_updates.len() >= RECORD_CAP {
            Self::fault_fleet_updates(&mut inner);
            return;
        }
        // Unlike lobby paint snapshots, these updates carry one-shot join and
        // launch requests and drained MeshOutbox frames. Keep them in order
        // while the asynchronous embedded surface loads or defers a push.
        inner.fleet_updates.push_back(json.into());
    }
    fn restore_fleet_updates(&self, mut updates: VecDeque<String>) {
        let mut inner = self.lock();
        if inner.fleet_faulted {
            return;
        }
        if updates.len() + inner.fleet_updates.len() > RECORD_CAP {
            Self::fault_fleet_updates(&mut inner);
            return;
        }
        updates.append(&mut inner.fleet_updates);
        inner.fleet_updates = updates;
    }
    fn fault_fleet_updates(inner: &mut Inner) {
        inner.fleet_updates.clear();
        inner.fleet_faulted = true;
        inner.records.clear();
        inner.records.push_back(
            r#"{"kind":"fleet_fault","reason":"bridge-overflow","detail":"native fleet update capacity exceeded"}"#.to_string(),
        );
    }
    pub fn push_fleet_wire(&self, frame: impl Into<String>) {
        let mut inner = self.lock();
        if inner.fleet_faulted {
            return;
        }
        if inner.fleet_wire.len() >= RECORD_CAP {
            inner.fleet_wire.clear();
            inner.fleet_faulted = true;
            inner.records.clear();
            inner.records.push_back(
                r#"{"kind":"fleet_fault","reason":"bridge-overflow","detail":"native fleet reliable bridge capacity exceeded"}"#.to_string(),
            );
            return;
        }
        inner.fleet_wire.push_back(frame.into());
    }
    pub fn fleet_faulted(&self) -> bool {
        self.lock().fleet_faulted
    }
    fn restore_fleet_wire(&self, mut frames: VecDeque<String>) {
        let mut inner = self.lock();
        frames.append(&mut inner.fleet_wire);
        inner.fleet_wire = frames;
    }
    pub fn republish_audio(&self) {
        self.lock().projections.republish_audio();
    }
    fn restore_qr_toggles(&self, count: usize) {
        self.lock().qr_toggles += count;
    }
}

fn is_fleet_record(json: &str) -> bool {
    matches!(
        super::HostLobbyRecord::decode(json),
        Some(
            super::HostLobbyRecord::FleetCode { .. }
                | super::HostLobbyRecord::FleetRoster { .. }
                | super::HostLobbyRecord::FleetFrame { .. }
                | super::HostLobbyRecord::FleetStartGrant { .. }
                | super::HostLobbyRecord::FleetHostLost { .. }
                | super::HostLobbyRecord::FleetSlotClaimed { .. }
                | super::HostLobbyRecord::FleetWireSend { .. }
                | super::HostLobbyRecord::FleetWireAdopt
                | super::HostLobbyRecord::FleetWireOpen { .. }
                | super::HostLobbyRecord::FleetWireClose { .. }
                | super::HostLobbyRecord::FleetContinuation { .. }
                | super::HostLobbyRecord::FleetContinuationFrame { .. }
                | super::HostLobbyRecord::FleetFault { .. }
                | super::HostLobbyRecord::FleetIdentity { .. }
                | super::HostLobbyRecord::FleetGmBootstrap { .. }
                | super::HostLobbyRecord::FleetStartPolicy { .. }
                | super::HostLobbyRecord::FleetForceResult { .. }
                | super::HostLobbyRecord::FleetGmJoinPending { .. }
                | super::HostLobbyRecord::FleetGmJoinStatus { .. }
                | super::HostLobbyRecord::FleetJoinStatus { .. }
        )
    )
}

/// One frame's worth of everything waiting to cross.
struct Pending {
    fleet_config: Option<String>,
    fleet_updates: VecDeque<String>,
    fleet_wire: VecDeque<String>,
    projections: [Option<Projection>; 8],
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

    if let Some(json) = pending.fleet_config {
        match surface.push(&super::document::host_lobby_fleet_config_script(&json)) {
            Ok(()) => report.pushed += 1,
            Err(e) => {
                report.push_failure = Some(e);
                report.deferred += 1;
                bridge.push_fleet_config(json);
                failed = true;
            }
        }
    }

    let mut unsent_updates = VecDeque::new();
    for json in pending.fleet_updates {
        if failed {
            unsent_updates.push_back(json);
            continue;
        }
        match surface.push(&super::document::host_lobby_fleet_update_script(&json)) {
            Ok(()) => report.pushed += 1,
            Err(e) => {
                report.push_failure = Some(e);
                unsent_updates.push_back(json);
                failed = true;
            }
        }
    }
    report.deferred += unsent_updates.len();
    if !unsent_updates.is_empty() {
        bridge.restore_fleet_updates(unsent_updates);
    }
    if failed {
        report.deferred += pending.fleet_wire.len();
        bridge.restore_fleet_wire(pending.fleet_wire);
    } else {
        let mut unsent = VecDeque::new();
        for frame in pending.fleet_wire {
            if failed {
                unsent.push_back(frame);
                continue;
            }
            match surface.push(&super::document::host_lobby_fleet_wire_script(&frame)) {
                Ok(()) => report.pushed += 1,
                Err(e) => {
                    report.push_failure = Some(e);
                    report.deferred += 1;
                    unsent.push_back(frame);
                    failed = true;
                }
            }
        }
        if !unsent.is_empty() {
            report.deferred += unsent.len().saturating_sub(1);
            bridge.restore_fleet_wire(unsent);
        }
    }

    for projection in pending.projections.into_iter().flatten() {
        if !failed {
            match surface.push(&projection.script()) {
                Ok(()) => {
                    report.pushed += 1;
                    continue;
                }
                Err(error) => {
                    report.push_failure = Some(error);
                    failed = true;
                }
            }
        }
        report.deferred += 1;
        bridge.lock().projections.restore(projection);
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
#[path = "bridge_tests.rs"]
mod tests;
