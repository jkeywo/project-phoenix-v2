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
mod tests {
    use super::*;
    use crate::native_host::panes::RecordingSurface;

    const LOBBY: &str = r#"{"phase":"Lobby","crew_count":0}"#;

    fn publish_lane(bridge: &HostLobbyBridge, lane: usize, newer: bool) -> String {
        let json = if newer {
            r#"{"value":2}"#
        } else {
            r#"{"value":1}"#
        };
        match lane {
            0 => {
                bridge.push_reveal(newer);
                host_lobby_reveal_script(newer)
            }
            1 => {
                bridge.push_join(json);
                host_lobby_join_script(json)
            }
            2 => {
                bridge.push_landing(json);
                host_lobby_landing_script(json)
            }
            3 => {
                bridge.push_packs(json);
                host_lobby_packs_script(json)
            }
            4 => {
                bridge.push_scenario(json);
                host_lobby_scenario_script(json)
            }
            5 => {
                bridge.push_layout(json);
                host_lobby_layout_script(json)
            }
            6 => {
                bridge.push_audio(json.into());
                super::super::document::host_lobby_audio_script(json)
            }
            7 => {
                bridge.push_lobby_state(json);
                host_lobby_apply_script(json)
            }
            _ => unreachable!(),
        }
    }

    struct FailingLaneSurface {
        recorded: RecordingSurface,
        fail_at: usize,
        attempted: usize,
        replace: Option<(HostLobbyBridge, usize)>,
    }

    impl PaneSurface for FailingLaneSurface {
        fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
            self.recorded.load(url)
        }
        fn is_ready(&self) -> bool {
            self.recorded.is_ready()
        }
        fn drain(&mut self) -> Vec<String> {
            self.recorded.drain()
        }
        fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
            let attempt = self.attempted;
            self.attempted += 1;
            if attempt == self.fail_at {
                if let Some((bridge, lane)) = &self.replace {
                    publish_lane(bridge, *lane, true);
                }
                return Err(PaneSurfaceError::Script("injected lane failure".into()));
            }
            self.recorded.push(script)
        }
    }

    #[test]
    fn every_projection_failure_preserves_order_counts_and_edges() {
        for fail_at in 0..8 {
            let bridge = HostLobbyBridge::new();
            let expected: Vec<_> = (0..8)
                .map(|lane| publish_lane(&bridge, lane, false))
                .collect();
            bridge.push_qr_toggle();
            bridge.push_qr_toggle();
            let mut surface = FailingLaneSurface {
                recorded: RecordingSurface::ready(),
                fail_at,
                attempted: 0,
                replace: None,
            };
            let report = pump_host_lobby(&bridge, &mut surface);
            assert_eq!(report.pushed, fail_at);
            assert_eq!(report.deferred, 10 - fail_at);
            assert_eq!(surface.attempted, fail_at + 1);
            assert!(report.push_failure.is_some());
            let retry = pump_host_lobby(&bridge, &mut surface);
            assert_eq!(retry.pushed, 10 - fail_at);
            assert_eq!(retry.deferred, 0);
            assert_eq!(&surface.recorded.pushed[..8], expected.as_slice());
            assert_eq!(
                &surface.recorded.pushed[8..],
                &[host_lobby_qr_toggle_script(), host_lobby_qr_toggle_script()]
            );
        }
    }

    #[test]
    fn every_projection_retains_a_newer_value_published_during_failure() {
        for lane in 0..8 {
            let bridge = HostLobbyBridge::new();
            publish_lane(&bridge, lane, false);
            let mut surface = FailingLaneSurface {
                recorded: RecordingSurface::ready(),
                fail_at: 0,
                attempted: 0,
                replace: Some((bridge.clone(), lane)),
            };
            assert_eq!(pump_host_lobby(&bridge, &mut surface).deferred, 1);
            assert_eq!(pump_host_lobby(&bridge, &mut surface).pushed, 1);
            assert_eq!(
                surface.recorded.pushed,
                vec![publish_lane(&bridge, lane, true)]
            );
        }
    }

    #[test]
    fn only_frame_fed_projection_lanes_deduplicate() {
        for lane in 0..8 {
            let bridge = HostLobbyBridge::new();
            publish_lane(&bridge, lane, false);
            let mut surface = RecordingSurface::ready();
            assert_eq!(pump_host_lobby(&bridge, &mut surface).pushed, 1);
            publish_lane(&bridge, lane, false);
            assert_eq!(
                pump_host_lobby(&bridge, &mut surface).pushed,
                usize::from(lane < 5)
            );
        }
    }

    #[test]
    fn fleet_configuration_is_delivered_before_an_early_ready_frame() {
        let bridge = HostLobbyBridge::new();
        bridge.push_fleet_wire(r#"{"type":"ready"}"#);
        bridge.push_fleet_config(r#"{"base":"https://fleet.test"}"#);
        let mut surface = RecordingSurface::ready();

        let report = pump_host_lobby(&bridge, &mut surface);

        assert_eq!(report.pushed, 2);
        let pushed = &surface.pushed;
        assert!(pushed[0].contains("__phoenixHostFleetConfigure"));
        assert!(pushed[1].contains("__phoenixHostFleetWire"));
    }

    #[test]
    fn fleet_record_flood_is_bounded_and_becomes_a_terminal_fault() {
        let bridge = HostLobbyBridge::new();
        let frame = r#"{"kind":"fleet_wire_send","frame":"{}"}"#;
        for _ in 0..=RECORD_CAP {
            bridge.record(frame);
        }
        // Further reliable input after the fault cannot grow the queue again.
        for _ in 0..RECORD_CAP {
            bridge.record(frame);
        }

        let records = bridge.take_records();
        assert_eq!(records.len(), 1);
        assert!(matches!(
            super::super::HostLobbyRecord::decode(&records[0]),
            Some(super::super::HostLobbyRecord::FleetFault { reason, .. })
                if reason == "bridge-overflow"
        ));
    }

    #[test]
    fn fleet_wire_flood_is_bounded_and_becomes_a_terminal_fault() {
        let bridge = HostLobbyBridge::new();
        for _ in 0..=RECORD_CAP {
            bridge.push_fleet_wire(r#"{"type":"ready"}"#);
        }

        assert!(bridge.fleet_faulted());
        let pending = bridge.take_pending();
        assert!(pending.fleet_wire.is_empty());
        let records = bridge.take_records();
        assert_eq!(records.len(), 1);
        assert!(records[0].contains("bridge-overflow"));
    }

    #[test]
    fn fleet_join_and_frames_survive_delayed_surface_loading_in_order() {
        let bridge = HostLobbyBridge::new();
        let updates = [
            r#"{"join_request":{"code":"ABCDEFGH","role":"ship"},"frames":[]}"#,
            r#"{"join_request":null,"frames":["tick-first"]}"#,
            r#"{"force_start":true,"frames":["tick-second"]}"#,
            r#"{"force_start":false,"frames":[]}"#,
        ];
        for update in updates {
            bridge.push_fleet_update(update);
        }
        let mut surface = RecordingSurface::default();
        pump_host_lobby(&bridge, &mut surface);
        assert!(surface.pushed.is_empty());
        surface = RecordingSurface::ready();
        assert_eq!(pump_host_lobby(&bridge, &mut surface).pushed, 4);
        assert_eq!(
            surface.pushed,
            updates.map(super::super::document::host_lobby_fleet_update_script)
        );
    }

    #[test]
    fn deferred_fleet_updates_precede_newer_updates() {
        let bridge = HostLobbyBridge::new();
        bridge.push_fleet_update("first");
        bridge.push_fleet_update("second");
        let mut surface = RecordingSurface::ready();
        surface.failing_pushes = 1;
        assert_eq!(pump_host_lobby(&bridge, &mut surface).deferred, 2);
        bridge.push_fleet_update("third");
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(
            surface.pushed,
            ["first", "second", "third"]
                .map(super::super::document::host_lobby_fleet_update_script)
        );
    }

    #[test]
    fn fleet_update_overflow_and_concurrent_restore_are_terminal_and_bounded() {
        let bridge = HostLobbyBridge::new();
        bridge.push_fleet_update("in flight");
        let pending = bridge.take_pending();
        for _ in 0..RECORD_CAP {
            bridge.push_fleet_update("queued");
        }
        bridge.restore_fleet_updates(pending.fleet_updates);
        assert!(bridge.fleet_faulted());
        bridge.push_fleet_update("refused after fault");
        assert!(bridge.take_pending().fleet_updates.is_empty());
        let records = bridge.take_records();
        assert_eq!(records.len(), 1);
        assert!(records[0].contains("bridge-overflow"));

        let bridge = HostLobbyBridge::new();
        for _ in 0..=RECORD_CAP {
            bridge.push_fleet_update("queued");
        }
        assert!(bridge.fleet_faulted());
        assert!(bridge.take_pending().fleet_updates.is_empty());
    }
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
    fn audio_document_observation_republishes_only_current_status_without_playback() {
        let bridge = HostLobbyBridge::new();
        let mut surface = RecordingSurface::ready();
        bridge.push_audio("{\"status\":\"playing\"}".into());
        pump_host_lobby(&bridge, &mut surface);
        bridge.push_audio("{\"status\":\"playing\"}".into());
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 1);
        bridge.republish_audio();
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 2);
        assert!(surface
            .pushed
            .iter()
            .all(|script| script.contains("__phoenixHostLobbyAudio")));
        assert!(bridge.take_records().is_empty());
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
