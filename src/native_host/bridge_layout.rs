//! The bridge **layout law** (issue #1327) — **pure, Bevy-free**.
//!
//! [`super::bridge_profile`] is the *file*: what an operator wrote down, and
//! whether it is well-formed. This is the *law*: what a bridge arrangement may
//! become, one action at a time, while a host is running. Every path that
//! rearranges a bridge — the lobby's monitor buttons and per-station screen rows,
//! a saved per-ship-class layout being pre-applied, a hand-authored `--profile` —
//! goes through the transitions here, so there is exactly one place the three
//! rules live:
//!
//! 1. **One viewscreen.** A bridge has exactly one shared viewscreen, always.
//!    It is not optional and it is not a `None` waiting to be filled: the lobby
//!    itself is drawn on it, so a layout is constructed with one and can only
//!    ever move it.
//! 2. **No overlay.** A station's console never opens on the viewscreen's
//!    monitor. Assigning one there is refused
//!    ([`LayoutRefusal::StationOnViewscreenMonitor`]), and so is the mirror
//!    action — moving the viewscreen onto a monitor that is holding stations
//!    ([`LayoutRefusal::ViewscreenMonitorHoldsStations`]). The second refusal is
//!    the one worth stating out loud: the alternative is silently evicting
//!    somebody's console to make room, and this module never rearranges anything
//!    the operator did not ask for. Unassign them first. A console an
//!    **authored** profile opened counts too, even though this layout cannot
//!    move it — see [`BridgeLayout::reserved_on`].
//! 3. **Two per screen.** A non-viewscreen monitor holds at most
//!    [`MAX_STATIONS_PER_MONITOR`] **consoles**, split deterministically — along
//!    the screen's own authored [`split_on`](BridgeLayout::split_on), or
//!    [`LAYOUT_SPLIT`] where no profile authored one. That is
//!    [`MAX_PANES_PER_STATION`] — the same legibility bound the
//!    profile enforces at author time, restated as a runtime precondition rather
//!    than duplicated as a second number. A console is a console whoever opened
//!    it (issue #1332): the stations this layout seated *and* the authored
//!    surfaces it merely knows about ([`BridgeLayout::reserved_on`]) are counted
//!    together, so a screen carrying a hand-authored `--pane` has one free slot
//!    and one carrying two has none.
//!
//! # Stations are keyed by station id, not by participant name
//!
//! A console the lobby opens on a wall monitor is claimable by anyone — crew
//! symmetry is not decided by where a screen is — so at layout time there is no
//! participant to name. What is being placed is a **[`StationId`]**, the ship's
//! own authoring key for a claimable station, and that is what this model keys
//! on end to end. It survives persistence because [`PaneSlot`] carries the id
//! beside the label; see that type's note for why it is a field of its own
//! rather than an overloaded `label`.
//!
//! # What the UI has to work out for itself: nothing
//!
//! The lobby greys buttons, and it must not re-derive *why* one is greyed —
//! that is how two implementations of one rule start disagreeing. So the model
//! answers it directly: [`BridgeLayout::occupancy`] says what each monitor is
//! holding and how much room is left, and [`BridgeLayout::eligibility`] gives,
//! for every station, one entry per monitor that is either
//! [`Selected`](MonitorChoice::Selected), [`Eligible`](MonitorChoice::Eligible),
//! or [`Excluded`](MonitorChoice::Excluded) with the reason —
//! [`is-viewscreen`](ExclusionReason::IsViewscreen) or
//! [`full`](ExclusionReason::Full). A button row is a `map` over that list.
//!
//! # When the bridge itself changes under a running host
//!
//! Screens are unplugged mid-session, and a ship's roster changes with its class.
//! [`BridgeLayout::reconcile`] rebuilds a layout against a new monitor set and a
//! new roster: every seat that still makes sense is kept, in its own order, and
//! everything else degrades to unassigned. None of that is an error — a bridge
//! whose screens changed is still a bridge — but none of it is silent either.
//! Each degradation is reported as a [`LayoutAdoption`], including the one that
//! would otherwise pass unnoticed: the operator's chosen viewscreen falling back
//! to the primary because the monitor they picked is gone.
//!
//! # Not the file, and not the window
//!
//! Nothing here opens a window, and nothing here reads a monitor. The winit
//! adapter is [`super::bridge_display`]; the monitors this model knows about are
//! whatever identities it was constructed with, which is what makes the whole law
//! testable in the ordinary `cargo test` CI runs on a machine with one headless
//! display.

use crate::core::messages::StationId;

use super::bridge_profile::{
    pane_rects, BridgeProfile, DiscoveredMonitor, DisplayEntry, DisplayRole, MonitorGeometry,
    MonitorIdentity, PaneRect, PaneSlot, PaneSplit, ValidatedProfile, MAX_PANES_PER_STATION,
    ROLE_STATION, ROLE_VIEWSCREEN,
};

/// The most station consoles one non-viewscreen monitor may hold.
///
/// Defined as [`MAX_PANES_PER_STATION`] rather than as a second `2`: a station
/// the layout seats on a monitor becomes a pane on that monitor's Station
/// surface, so the runtime cap and the author-time density rule are one rule
/// seen from two ends. Changing the legibility bound must move both together, so
/// there is only one place to change it.
pub const MAX_STATIONS_PER_MONITOR: usize = MAX_PANES_PER_STATION;

/// How a monitor the **lobby** filled divides itself between two consoles.
///
/// Side by side — the PRD's "two consoles on one monitor split it side by side
/// automatically". It is a constant rather than a lobby control because the
/// lobby offers none: an operator picks *which* screen, never how it is carved,
/// so a per-press choice would be a setting nothing sets.
///
/// It is the **default**, not the only answer. A monitor a `--profile` authored
/// as a Station carries that entry's own `split`, and
/// [`adopt_profile`](BridgeLayout::adopt_profile) records it
/// ([`split_on`](BridgeLayout::split_on)) so the arrangement an operator wrote
/// down is the arrangement that is drawn — issue #1332's carried defect was that
/// the adapter re-tiled an authored `stacked` screen side by side on the first
/// frame after boot. A screen no profile authored — every screen on a host with
/// no `--profile`, and any other screen on a host with one — has this.
pub const LAYOUT_SPLIT: PaneSplit = PaneSplit::SideBySide;

// ── actions ─────────────────────────────────────────────────────────────────

/// One thing an operator can do to a bridge layout.
///
/// The whole vocabulary: pick the viewscreen's monitor, put a station's console
/// on a screen, take it off again. There is deliberately no separate "move"
/// action — assigning a station that is already seated *is* the move (the PRD's
/// "move a station's console from one monitor to another by pressing a different
/// screen button"), so the lobby presses the same button either way and cannot
/// pick the wrong verb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutAction {
    /// Make `monitor` the viewscreen. Refused while it is holding stations.
    SetViewscreen { monitor: MonitorIdentity },
    /// Open (or re-seat) `station`'s console on `monitor`.
    AssignStation {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// Close `station`'s console, freeing its slot.
    UnassignStation { station: StationId },
}

/// Why a [`LayoutAction`] was refused.
///
/// Every variant names the parties, because the lobby has to *say* what happened
/// — "the lobby never silently ignores me" is a user story, and a boolean cannot
/// satisfy it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutRefusal {
    /// The action names a monitor this bridge does not have — a stale button, or
    /// a display unplugged between the click and the apply.
    UnknownMonitor { monitor: MonitorIdentity },
    /// The action names a station that is not on this ship's roster.
    UnknownStation { station: StationId },
    /// A station's console would open on the monitor showing the viewscreen,
    /// covering the one surface the whole bridge watches.
    StationOnViewscreenMonitor {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// The monitor already holds [`MAX_STATIONS_PER_MONITOR`] consoles.
    MonitorFull {
        monitor: MonitorIdentity,
        /// The consoles this layout seated there.
        occupants: Vec<StationId>,
        /// The labels of surfaces an authored profile opened there that this
        /// layout does not own — see [`BridgeLayout::reserved_on`]. Named beside
        /// the seats for [`ViewscreenMonitorHoldsStations`](Self::ViewscreenMonitorHoldsStations)'s
        /// reason, and counted with them since issue #1332: a console occupies a
        /// screen whoever opened it, so a refusal that named only the seats would
        /// report "1 console" on a screen the operator can see two on.
        panes: Vec<String>,
    },
    /// The viewscreen was asked to move onto a monitor that is holding station
    /// consoles. Refused rather than evicting them: the operator unassigns them
    /// first, so nothing they arranged disappears without them asking.
    ViewscreenMonitorHoldsStations {
        monitor: MonitorIdentity,
        /// The consoles this layout seated there.
        stations: Vec<StationId>,
        /// The labels of surfaces an authored profile opened there that this
        /// layout does not own — see [`BridgeLayout::reserved_on`]. Named
        /// beside the seats rather than folded into them because they are a
        /// different kind of thing: the operator cannot unassign one, and a
        /// `StationId` here would be an id no roster has.
        panes: Vec<String>,
    },
}

impl std::fmt::Display for LayoutRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutRefusal::UnknownMonitor { monitor } => write!(
                f,
                "monitor {monitor} is not one of this bridge's displays; it may have been \
                 unplugged since the layout was drawn"
            ),
            LayoutRefusal::UnknownStation { station } => write!(
                f,
                "station {:?} is not on this ship's roster, so it has no console to place",
                station.0
            ),
            LayoutRefusal::StationOnViewscreenMonitor { station, monitor } => write!(
                f,
                "monitor {monitor} is showing the viewscreen, so station {:?}'s console may not \
                 open on it — a console never covers the shared view. Pick another screen, or \
                 move the viewscreen first",
                station.0
            ),
            LayoutRefusal::MonitorFull {
                monitor,
                occupants,
                panes,
            } => write!(
                f,
                "monitor {monitor} already holds {} console(s) ({}), which is the maximum of \
                 {MAX_STATIONS_PER_MONITOR}: a console is authored to be read one, or \
                 side-by-side two, to a screen. Free a slot or pick another screen",
                occupants.len() + panes.len(),
                occupant_list(occupants, panes),
            ),
            LayoutRefusal::ViewscreenMonitorHoldsStations {
                monitor,
                stations,
                panes,
            } => write!(
                f,
                "monitor {monitor} is holding the console(s) for {}, so the viewscreen may not \
                 move onto it; unassign them first — they are not evicted to make room",
                occupant_list(stations, panes),
            ),
        }
    }
}

impl std::error::Error for LayoutRefusal {}

impl LayoutRefusal {
    /// The `strings.csv` id a **player-visible** surface renders this refusal
    /// through (issue #1330).
    ///
    /// [`Display`](std::fmt::Display) above is the *operator log*'s sentence —
    /// Rust-composed English, which is exactly what AGENTS.md rule 11 says may
    /// never reach a screen a player reads. The lobby's monitor row does reach
    /// one, so what crosses the bridge to it is this id plus
    /// [`params`](Self::params), and `gui/strings.js`'s `t()` resolves the
    /// sentence on the page. One refusal, two renderings, and neither is the
    /// other's fallback.
    pub fn string_id(&self) -> &'static str {
        match self {
            LayoutRefusal::UnknownMonitor { .. } => "server.bridge_layout.unknown_monitor",
            LayoutRefusal::UnknownStation { .. } => "server.bridge_layout.unknown_station",
            LayoutRefusal::StationOnViewscreenMonitor { .. } => {
                "server.bridge_layout.station_on_viewscreen"
            }
            LayoutRefusal::MonitorFull { .. } => "server.bridge_layout.monitor_full",
            LayoutRefusal::ViewscreenMonitorHoldsStations { .. } => {
                "server.bridge_layout.viewscreen_holds_stations"
            }
        }
    }

    /// The `{placeholder}` values [`string_id`](Self::string_id)'s row
    /// interpolates, in a fixed order.
    pub fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            LayoutRefusal::UnknownMonitor { monitor } => {
                vec![("monitor", monitor.as_str().to_string())]
            }
            LayoutRefusal::UnknownStation { station } => vec![("station", station.0.clone())],
            LayoutRefusal::StationOnViewscreenMonitor { station, monitor } => vec![
                ("station", station.0.clone()),
                ("monitor", monitor.as_str().to_string()),
            ],
            LayoutRefusal::MonitorFull {
                monitor,
                occupants,
                panes,
            } => vec![
                ("monitor", monitor.as_str().to_string()),
                ("stations", occupant_params(occupants, panes)),
                ("max", MAX_STATIONS_PER_MONITOR.to_string()),
            ],
            LayoutRefusal::ViewscreenMonitorHoldsStations {
                monitor,
                stations,
                panes,
            } => vec![
                ("monitor", monitor.as_str().to_string()),
                ("stations", occupant_params(stations, panes)),
            ],
        }
    }
}

/// Everything on one monitor, for an operator log line: its seated consoles
/// then the authored surfaces it also carries — `"helm", "Ada"`.
///
/// Quoted, unlike [`occupant_params`]: an operator log line is naming machine
/// keys, and the quotes say so.
///
/// Both refusals that name a monitor's occupants go through this **one** pair of
/// functions (issue #1332 brought [`LayoutRefusal::MonitorFull`] here from a
/// seats-only formatter). A refusal that counted or listed only the seats would
/// tell an operator a screen holds one console while they are looking at two.
fn occupant_list(stations: &[StationId], panes: &[String]) -> String {
    join_occupants(
        stations.iter().map(|s| format!("{:?}", s.0)),
        panes.iter().map(|p| format!("{p:?}")),
    )
}

/// Everything on one monitor, for a **player-visible** parameter — `helm, Ada`.
///
/// Unquoted: the quotes in an operator log line read as "this is a machine key",
/// and on the lobby's own surface they read as stray punctuation in a sentence.
fn occupant_params(stations: &[StationId], panes: &[String]) -> String {
    join_occupants(stations.iter().map(|s| s.0.clone()), panes.iter().cloned())
}

fn join_occupants(
    stations: impl Iterator<Item = String>,
    panes: impl Iterator<Item = String>,
) -> String {
    stations.chain(panes).collect::<Vec<_>>().join(", ")
}

// ── the layout ──────────────────────────────────────────────────────────────

/// One console an **authored** profile opened on a monitor that this layout does
/// not own — see [`BridgeLayout::reserved_on`].
///
/// It carries the pane's position in the `[[display]]` entry that authored it as
/// well as its label, and that index is load-bearing (issue #1332): it is where
/// the console is *drawn* on its screen. A profile authoring
/// `[helm(station), Ada(participant)]` puts helm on the left and Ada on the
/// right, and the layout has nowhere else to remember that — the station went
/// into `seats` and the participant into `reserved`, so the pair's authored
/// interleaving is lost the moment the two lists are read back in their own
/// order. Blanket "reserved first" is what that loss looked like: boot drew the
/// operator's order and the first follower pass flipped it, closing and
/// recreating a console nobody had touched.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reservation {
    /// The participant label the pane was authored under — the key the pane bus
    /// resolves it by, and the name a person reads.
    label: String,
    /// This pane's index within its authored `[[display]]` entry, which is its
    /// slot in the screen's left-to-right (or top-to-bottom) tiling.
    index: usize,
}

/// A bridge arrangement: which monitor is the viewscreen, and which stations sit
/// on which of the others.
///
/// Every value of this type is a **lawful** bridge — there is no constructor and
/// no transition that produces one which is not, so nothing downstream re-checks
/// the rules. [`apply`](Self::apply) is pure: it answers a *new* layout or a
/// refusal and never mutates the one it was given, so a lobby can offer a
/// prospective arrangement without committing to it.
///
/// Seating is stored grouped by monitor, in monitor order, and **within a
/// monitor in the order the consoles were assigned**. That order is not
/// incidental: it is the left-to-right order the panes are drawn in
/// ([`station_rects`](Self::station_rects)), it is what
/// [`write_displays_into`](Self::write_displays_into) records in the file, and it
/// is part of `PartialEq`. So two layouts are equal when they seat the same
/// stations on the same monitors *in the same order* — which is what makes the
/// profile round-trip an equality assertion rather than a set comparison — and
/// moving a console off a shared screen and back swaps the two halves, because
/// the console that stayed put is now the earlier of the two. That is the
/// operator's own two actions showing up on the screen they are watching, not a
/// rearrangement behind their back, so the order is left exactly as they made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeLayout {
    /// This bridge's monitors, in discovery order. Deduplicated.
    monitors: Vec<MonitorIdentity>,
    /// The ship class's claimable stations. Deduplicated.
    roster: Vec<StationId>,
    /// Index into `monitors` of the monitor showing the viewscreen.
    viewscreen: usize,
    /// Parallel to `monitors`: the stations seated on each, in seat order. The
    /// viewscreen's entry is always empty — that is rule 2, held as an invariant
    /// rather than re-derived.
    seats: Vec<Vec<StationId>>,
    /// Parallel to `monitors`: how each divides itself between two consoles.
    ///
    /// [`LAYOUT_SPLIT`] for a screen no profile authored, and the authored
    /// entry's own `split` for one a `--profile` named as a Station
    /// ([`adopt_profile`](Self::adopt_profile) records it). It is stored rather
    /// than looked up because the profile is not kept: adoption is the only
    /// moment the file is in hand, and [`surface_rects`](Self::surface_rects) is
    /// read on every frame of the run.
    ///
    /// Normalised, never optional: a screen with no authored split holds
    /// [`LAYOUT_SPLIT`] itself rather than a `None` meaning the same thing, so
    /// two layouts that tile identically compare equal and the profile
    /// round-trip stays an equality assertion.
    splits: Vec<PaneSplit>,
    /// Parallel to `monitors`: the surfaces an **authored** profile opened on
    /// each, which this layout does not own (issue #1330).
    ///
    /// A hand-authored `--pane <NAME>` Station belongs to a named crew member
    /// rather than to a claimable station ([`PaneSlot::for_participant`]), so
    /// there is no [`StationId`] to seat and [`adopt_profile`](Self::adopt_profile)
    /// seats nothing for it. The window is opened all the same — the display
    /// adapter spawns one borderless-fullscreen surface per configured monitor —
    /// so a layout that recorded nothing would read that monitor as **free** and
    /// let the viewscreen move on top of somebody's live console. That is rule
    /// 2's whole point, defeated by a bookkeeping gap.
    ///
    /// A reserved label is therefore an occupant for **both** density rules. It
    /// blocks the viewscreen moving onto its screen (rule 2's mirror,
    /// [`set_viewscreen`](Self::set_viewscreen)) and it **consumes one of that
    /// screen's two console slots** (rule 3, issue #1332): a monitor carrying one
    /// authored pane has one free slot, one carrying two has none, and
    /// [`assign`](Self::assign), [`occupancy`](Self::occupancy) and
    /// [`eligibility`](Self::eligibility) all count it.
    ///
    /// It took #1332 to make that true, and the gap it closed was not
    /// theoretical: #1331 counted only `seats`, so the screen row *offered* a
    /// monitor already holding an authored console, the law *accepted* the press,
    /// and the adapter then laid the new console out across the whole monitor —
    /// the two consoles **overlapping** rather than tiling. Counting is half the
    /// fix; the other half is that the adapter lays every pane on a monitor out
    /// together, reserved ones included (`bridge_display::follow_layout_stations`).
    ///
    /// It is still **not a seat**: it is not on the roster, it has no
    /// [`eligibility`](Self::eligibility) row of its own, and no
    /// [`LayoutAction`] can move or free it — only the `--profile` that authored
    /// it, and only by being re-adopted. Faking a `StationId` for one instead
    /// would have put a station no ship has on the roster-driven rows.
    ///
    /// # It does not survive a round trip through a profile (issue #1334)
    ///
    /// [`write_displays_into`](Self::write_displays_into) emits seats and only
    /// seats, so `adopt_profile(to_validated_profile(L))` is **not** `L` whenever
    /// `L` reserves anything: the participant panes go in and do not come back
    /// out. That is harmless today, and only today — the one production caller
    /// that writes a profile ([`super::bridge_display`]'s synthesised runtime
    /// config) does it on a boot layout whose `reserved` is empty, and there is
    /// no save-layout path at all for an operator to lose anything through.
    ///
    /// The moment one lands it becomes DATA LOSS with a face: an operator saves a
    /// bridge that was seeded from a hand-authored `--profile`, the participant
    /// panes are silently dropped from the file, and the next boot reads those
    /// screens as free and moves the viewscreen onto a crew member's live
    /// console — the exact failure `reserved` exists to prevent, re-introduced by
    /// the save. Issue #1334 must therefore do one of two things: re-emit the
    /// participant slots it read, or refuse to persist a `--profile`-seeded
    /// layout at all. It may not simply write the file.
    reserved: Vec<Vec<Reservation>>,
}

impl BridgeLayout {
    /// A bridge of `monitors` and `roster`, with `viewscreen` showing the shared
    /// view and no station consoles open.
    ///
    /// Refuses [`LayoutRefusal::UnknownMonitor`] when `viewscreen` is not one of
    /// `monitors` — including the no-monitors case, which is not a bridge.
    /// Duplicates in either list are dropped, keeping the first occurrence, so a
    /// caller may hand over whatever it enumerated.
    pub fn new(
        monitors: impl IntoIterator<Item = MonitorIdentity>,
        roster: impl IntoIterator<Item = StationId>,
        viewscreen: &MonitorIdentity,
    ) -> Result<Self, LayoutRefusal> {
        let monitors = dedup(monitors);
        let roster = dedup(roster);
        let Some(index) = monitors.iter().position(|m| m == viewscreen) else {
            return Err(LayoutRefusal::UnknownMonitor {
                monitor: viewscreen.clone(),
            });
        };
        let seats = vec![Vec::new(); monitors.len()];
        let reserved = vec![Vec::new(); monitors.len()];
        let splits = vec![LAYOUT_SPLIT; monitors.len()];
        Ok(Self {
            monitors,
            roster,
            viewscreen: index,
            seats,
            splits,
            reserved,
        })
    }

    /// A bridge of the monitors [`identify`](super::bridge_profile::identify)
    /// discovered, with the **primary** monitor showing the viewscreen.
    ///
    /// The lobby's starting point: the primary monitor is where the OS opened the
    /// process's window, which is where the lobby is already being drawn. `None`
    /// when nothing was discovered. If no monitor is flagged primary the first is
    /// used, matching the adapter's own fallback.
    pub fn from_discovered(
        discovered: &[DiscoveredMonitor],
        roster: impl IntoIterator<Item = StationId>,
    ) -> Option<Self> {
        let primary = discovered
            .iter()
            .find(|d| d.primary)
            .or_else(|| discovered.first())?;
        let viewscreen = primary.identity.clone();
        Self::new(
            discovered.iter().map(|d| d.identity.clone()),
            roster,
            &viewscreen,
        )
        .ok()
    }

    /// This bridge's monitors, in discovery order.
    pub fn monitors(&self) -> &[MonitorIdentity] {
        &self.monitors
    }

    /// This ship class's claimable stations.
    pub fn roster(&self) -> &[StationId] {
        &self.roster
    }

    /// The monitor showing the shared viewscreen.
    pub fn viewscreen(&self) -> &MonitorIdentity {
        &self.monitors[self.viewscreen]
    }

    /// The monitor `station`'s console is open on, if any.
    pub fn monitor_of(&self, station: &StationId) -> Option<&MonitorIdentity> {
        self.seat_of(station).map(|i| &self.monitors[i])
    }

    /// The stations seated on `monitor`, in **seat order** — the order the
    /// operator opened them, which is the order they appear left to right among
    /// that screen's panes. Empty for the viewscreen, and for a monitor this
    /// bridge does not have.
    ///
    /// Seat order is the stations' order *relative to each other*, not their
    /// slot numbers: an authored surface ([`reserved_on`](Self::reserved_on))
    /// may sit between them or before them, at the index its profile gave it.
    /// [`occupants_on`](Self::occupants_on) is the whole screen in the order it
    /// is drawn, and [`surface_rects`](Self::surface_rects) is that order with
    /// the rectangles.
    pub fn stations_on(&self, monitor: &MonitorIdentity) -> &[StationId] {
        match self.index_of(monitor) {
            Some(i) => &self.seats[i],
            None => &[],
        }
    }

    /// The labels of authored surfaces on `monitor` this layout does not own, in
    /// the order their profile authored them — see the
    /// [`reserved`](Self::reserved) note. Empty for a monitor this bridge does
    /// not have.
    pub fn reserved_on(&self, monitor: &MonitorIdentity) -> Vec<String> {
        match self.index_of(monitor) {
            Some(i) => self.reserved[i].iter().map(|r| r.label.clone()).collect(),
            None => Vec::new(),
        }
    }

    /// How `monitor` divides itself between two consoles — its authored `split`
    /// when a `--profile` named it a Station, else [`LAYOUT_SPLIT`].
    ///
    /// [`LAYOUT_SPLIT`] for a monitor this bridge does not have, which is the
    /// same answer an empty screen gives and is never drawn against anything.
    pub fn split_on(&self, monitor: &MonitorIdentity) -> PaneSplit {
        match self.index_of(monitor) {
            Some(i) => self.splits[i],
            None => LAYOUT_SPLIT,
        }
    }

    /// Everything open on `monitor`, as the names a person reads, **in the order
    /// it is drawn on that screen**.
    ///
    /// The same order — and the same names — [`surface_rects`](Self::surface_rects)
    /// hands out rectangles in, so the greyed button that lists what a screen is
    /// holding reads left to right (or top to bottom) exactly as the screen does.
    /// Before issue #1332's fix round the two disagreed: this listed seats then
    /// authored surfaces while the tiling drew authored surfaces first, so a
    /// mixed screen's button named them in the opposite order to the glass.
    ///
    /// What the lobby's monitor row draws on a button — one function, so the row
    /// cannot promise a press the law then refuses for a reason the row never
    /// showed.
    pub fn occupants_on(&self, monitor: &MonitorIdentity) -> Vec<String> {
        match self.index_of(monitor) {
            Some(i) => self
                .occupants_at(i)
                .iter()
                .map(|o| o.name().to_string())
                .collect(),
            None => Vec::new(),
        }
    }

    /// Apply `action`, answering the layout it produces — or the typed reason it
    /// was refused. `self` is untouched either way.
    ///
    /// # The no-op doctrine
    ///
    /// An action that asks for what already holds succeeds and answers an
    /// identical layout, rather than being a refusal — **all three** of them:
    /// naming the current viewscreen, seating a station on the monitor it is
    /// already on, and closing a console that is not open. Pressing the button
    /// under your finger is not an error, and two of these are reachable without
    /// anyone pressing anything twice: a lobby's off button is pressed by two
    /// clients at once, or a station is unassigned by one operator between
    /// another's read and their click. Refusing the second one would make a race
    /// look like a fault. Note the boundary — this is about a *lawful* action
    /// that changes nothing; an action naming a monitor this bridge does not have
    /// or a station off this ship's roster is still refused, because that is a
    /// stale button rather than a settled state.
    pub fn apply(&self, action: &LayoutAction) -> Result<Self, LayoutRefusal> {
        match action {
            LayoutAction::SetViewscreen { monitor } => self.set_viewscreen(monitor),
            LayoutAction::AssignStation { station, monitor } => self.assign(station, monitor),
            LayoutAction::UnassignStation { station } => self.unassign(station),
        }
    }

    fn set_viewscreen(&self, monitor: &MonitorIdentity) -> Result<Self, LayoutRefusal> {
        let index = self.require_monitor(monitor)?;
        if index == self.viewscreen {
            return Ok(self.clone());
        }
        // Rule 2's mirror: no silent eviction. A monitor holding consoles keeps
        // them, and the operator is told to unassign them first. An authored
        // surface counts — it is a live console on that screen too, and the
        // fact that this layout cannot move it makes covering it worse, not
        // more allowable (see `reserved`).
        if !self.seats[index].is_empty() || !self.reserved[index].is_empty() {
            return Err(LayoutRefusal::ViewscreenMonitorHoldsStations {
                monitor: monitor.clone(),
                stations: self.seats[index].clone(),
                panes: self.reserved_labels(index),
            });
        }
        let mut next = self.clone();
        next.viewscreen = index;
        Ok(next)
    }

    fn assign(
        &self,
        station: &StationId,
        monitor: &MonitorIdentity,
    ) -> Result<Self, LayoutRefusal> {
        // The subject before the object: an action naming a station this ship
        // does not have is refused as that, whatever monitor it also named.
        self.require_station(station)?;
        let index = self.require_monitor(monitor)?;
        if index == self.viewscreen {
            return Err(LayoutRefusal::StationOnViewscreenMonitor {
                station: station.clone(),
                monitor: monitor.clone(),
            });
        }
        let current = self.seat_of(station);
        if current == Some(index) {
            return Ok(self.clone());
        }
        // Capacity is judged AFTER the no-op case above, so re-pressing a full
        // monitor's own button for a station already on it is not a refusal —
        // the station is one of the occupants it would be counted against.
        if self.is_full(index) {
            return Err(LayoutRefusal::MonitorFull {
                monitor: monitor.clone(),
                occupants: self.seats[index].clone(),
                panes: self.reserved_labels(index),
            });
        }
        let mut next = self.clone();
        if let Some(from) = current {
            next.seats[from].retain(|s| s != station);
        }
        next.seats[index].push(station.clone());
        Ok(next)
    }

    fn unassign(&self, station: &StationId) -> Result<Self, LayoutRefusal> {
        self.require_station(station)?;
        // The third no-op: a console that is not open is already closed. See
        // `apply`'s note — the station is still checked against the roster first,
        // so a stale button from another ship class is refused as that.
        let Some(index) = self.seat_of(station) else {
            return Ok(self.clone());
        };
        let mut next = self.clone();
        next.seats[index].retain(|s| s != station);
        Ok(next)
    }

    fn index_of(&self, monitor: &MonitorIdentity) -> Option<usize> {
        self.monitors.iter().position(|m| m == monitor)
    }

    fn require_monitor(&self, monitor: &MonitorIdentity) -> Result<usize, LayoutRefusal> {
        self.index_of(monitor)
            .ok_or_else(|| LayoutRefusal::UnknownMonitor {
                monitor: monitor.clone(),
            })
    }

    fn require_station(&self, station: &StationId) -> Result<(), LayoutRefusal> {
        if self.roster.contains(station) {
            Ok(())
        } else {
            Err(LayoutRefusal::UnknownStation {
                station: station.clone(),
            })
        }
    }

    fn seat_of(&self, station: &StationId) -> Option<usize> {
        self.seats.iter().position(|s| s.contains(station))
    }

    /// The labels of one monitor's authored surfaces, in the order their profile
    /// authored them.
    fn reserved_labels(&self, index: usize) -> Vec<String> {
        self.reserved[index]
            .iter()
            .map(|r| r.label.clone())
            .collect()
    }

    /// Everything on one monitor, **in the order it is drawn on that screen**
    /// (issue #1332's fix round) — the one place that order is decided.
    ///
    /// Each authored surface takes the slot its profile gave it
    /// ([`Reservation::index`]) and the seats fill what is left, in seat order.
    /// So a profile authoring `[helm(station), Ada(participant)]` draws helm on
    /// the left and Ada on the right — the operator's own order — and one
    /// authoring `[Ada, helm]` draws them the other way round. A station the
    /// **lobby** later seats beside an authored console lands in the slot the
    /// authored one did not take, which is the same "the console that stayed put
    /// keeps its half" rule seat order already follows, extended to the one
    /// occupant seat order cannot express.
    ///
    /// An authored index can be out of range — a two-pane profile whose *other*
    /// pane named a station this ship does not have leaves one reservation at
    /// index 1 on a screen with one slot — so it is clamped to the last slot and
    /// then walked forward (wrapping) to the first free one. Total and
    /// deterministic: there is exactly one slot per occupant, so a free one
    /// always remains.
    fn occupants_at(&self, index: usize) -> Vec<SurfaceOccupant> {
        let total = self.occupant_count(index);
        if total == 0 {
            return Vec::new();
        }
        let mut slots: Vec<Option<SurfaceOccupant>> = vec![None; total];
        for reservation in &self.reserved[index] {
            let target = reservation.index.min(total - 1);
            let slot = (target..total)
                .chain(0..target)
                .find(|i| slots[*i].is_none())
                .expect("one slot per occupant, so a free one always remains");
            slots[slot] = Some(SurfaceOccupant::Reserved(reservation.label.clone()));
        }
        let mut seats = self.seats[index].iter();
        slots
            .into_iter()
            .map(|slot| {
                slot.unwrap_or_else(|| {
                    SurfaceOccupant::Station(
                        seats
                            .next()
                            .expect("the free slots are exactly the seats' count")
                            .clone(),
                    )
                })
            })
            .collect()
    }

    /// How many consoles a monitor is carrying — its seats **and** the authored
    /// surfaces it holds (issue #1332).
    ///
    /// The one arithmetic behind rule 3, so [`assign`](Self::assign)'s refusal,
    /// [`eligibility`](Self::eligibility)'s greying and
    /// [`occupancy`](Self::occupancy)'s `free_slots` cannot come to disagree
    /// about what "full" means. See the [`reserved`](Self::reserved) note.
    fn occupant_count(&self, index: usize) -> usize {
        self.seats[index].len() + self.reserved[index].len()
    }

    /// Whether a monitor can take no further console — see
    /// [`occupant_count`](Self::occupant_count).
    fn is_full(&self, index: usize) -> bool {
        self.occupant_count(index) >= MAX_STATIONS_PER_MONITOR
    }

    // ── reporting ───────────────────────────────────────────────────────────

    /// What every monitor is holding, in monitor order.
    pub fn occupancy(&self) -> Vec<MonitorOccupancy> {
        (0..self.monitors.len())
            .map(|i| self.occupancy_at(i))
            .collect()
    }

    /// What one monitor is holding. `None` for a monitor this bridge lacks.
    pub fn occupancy_of(&self, monitor: &MonitorIdentity) -> Option<MonitorOccupancy> {
        self.index_of(monitor).map(|i| self.occupancy_at(i))
    }

    fn occupancy_at(&self, index: usize) -> MonitorOccupancy {
        let is_viewscreen = index == self.viewscreen;
        MonitorOccupancy {
            monitor: self.monitors[index].clone(),
            is_viewscreen,
            stations: self.seats[index].clone(),
            reserved: self.reserved_labels(index),
            free_slots: (!is_viewscreen)
                .then(|| MAX_STATIONS_PER_MONITOR.saturating_sub(self.occupant_count(index))),
        }
    }

    /// For every station on the roster, which monitors its console may open on
    /// and why each of the others may not — everything a greyed button row needs.
    pub fn eligibility(&self) -> Vec<StationEligibility> {
        self.roster
            .iter()
            .map(|station| self.eligibility_at(station))
            .collect()
    }

    /// [`eligibility`](Self::eligibility) for one station. Refuses
    /// [`LayoutRefusal::UnknownStation`] for a station off the roster.
    pub fn eligibility_of(&self, station: &StationId) -> Result<StationEligibility, LayoutRefusal> {
        self.require_station(station)?;
        Ok(self.eligibility_at(station))
    }

    fn eligibility_at(&self, station: &StationId) -> StationEligibility {
        let seat = self.seat_of(station);
        StationEligibility {
            station: station.clone(),
            assigned_to: seat.map(|i| self.monitors[i].clone()),
            monitors: (0..self.monitors.len())
                .map(|i| StationMonitorChoice {
                    monitor: self.monitors[i].clone(),
                    choice: if seat == Some(i) {
                        MonitorChoice::Selected
                    } else if i == self.viewscreen {
                        MonitorChoice::Excluded(ExclusionReason::IsViewscreen)
                    } else if self.is_full(i) {
                        MonitorChoice::Excluded(ExclusionReason::Full)
                    } else {
                        MonitorChoice::Eligible
                    },
                })
                .collect(),
        }
    }

    /// Where **everything** on `monitor` is drawn on it, in the order it is drawn
    /// — [`occupants_on`](Self::occupants_on)'s order, with the rectangles
    /// (issue #1332).
    ///
    /// The deterministic split, resolved against real geometry: one console is
    /// the whole monitor, two divide it along that screen's own
    /// [`split_on`](Self::split_on) with no gap and no overlap. Empty for the
    /// viewscreen and for a monitor this bridge lacks.
    ///
    /// # One tiling, for both kinds of console and from boot onward
    ///
    /// It is **one** [`pane_rects`] call over the whole occupancy, not a seat
    /// tiling laid beside a reserved one. Two calls is exactly the shape that
    /// produced the #1331 overlap this replaces: an authored pane spawned at boot
    /// across its whole monitor, a station later seated on the same screen, and
    /// both handed the full rectangle. Since rule 3 now counts the two together
    /// ([`reserved`](Self::reserved)) their count is bounded by
    /// [`MAX_STATIONS_PER_MONITOR`] like any other pair, so the *existing* split
    /// geometry tiles them with nothing new invented.
    ///
    /// It is also the **only** tiling. `bridge_display::apply_bridge_profile`
    /// used to lay a `--profile`'s Station out from the file directly — the
    /// profile's own pane order, the profile's own split — and the first
    /// follower pass then re-laid the same screen from here. Two answers, and
    /// this one won: an authored `[helm, Ada]` booted helm-left and flipped to
    /// Ada-left one frame later, closing and recreating a console nobody had
    /// touched, and an authored `stacked` screen was re-tiled side by side. Boot
    /// now reads its rectangles from here, so there is one answer and the
    /// authored arrangement is what a boot frame draws — see
    /// [`occupants_at`](Self::occupants_at) for the order and
    /// [`splits`](Self::splits) for the axis.
    pub fn surface_rects(
        &self,
        monitor: &MonitorIdentity,
        geometry: &MonitorGeometry,
    ) -> Vec<(SurfaceOccupant, PaneRect)> {
        let Some(index) = self.index_of(monitor) else {
            return Vec::new();
        };
        let occupants = self.occupants_at(index);
        let rects = pane_rects(geometry, self.splits[index], occupants.len());
        occupants.into_iter().zip(rects).collect()
    }

    /// Where each of `monitor`'s **seated** consoles is drawn on it, in seat
    /// order — [`surface_rects`](Self::surface_rects) with the authored surfaces
    /// filtered out, *not* a tiling of its own.
    ///
    /// A caller that has to place every pane on a screen wants `surface_rects`;
    /// this is for one that only asks where the stations are.
    pub fn station_rects(
        &self,
        monitor: &MonitorIdentity,
        geometry: &MonitorGeometry,
    ) -> Vec<(StationId, PaneRect)> {
        self.surface_rects(monitor, geometry)
            .into_iter()
            .filter_map(|(occupant, rect)| match occupant {
                SurfaceOccupant::Station(station) => Some((station, rect)),
                SurfaceOccupant::Reserved(_) => None,
            })
            .collect()
    }

    // ── conversion to and from a display profile ────────────────────────────

    /// Write this layout over `profile`'s `[[display]]` list, leaving its
    /// `[[touch]]` and `[[media]]` assignments alone.
    ///
    /// Split from [`to_profile`](Self::to_profile) because a bridge profile is
    /// one file with more than displays in it: saving a layout back over an
    /// operator's existing profile must not quietly drop the touch mapping and
    /// the room's cameras and microphones.
    ///
    /// The emitted order is the operator's: the viewscreen first, then each
    /// monitor holding consoles in monitor order — which is the order
    /// [`BridgeProfile`] documents, and the order this layout reads back in.
    pub fn write_displays_into(&self, profile: &mut BridgeProfile) {
        let mut displays = vec![DisplayEntry {
            id: self.viewscreen().as_str().to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        }];
        for (index, stations) in self.seats.iter().enumerate() {
            if stations.is_empty() {
                continue;
            }
            displays.push(DisplayEntry {
                id: self.monitors[index].as_str().to_string(),
                role: ROLE_STATION.to_string(),
                // The screen's OWN split (issue #1332's fix round), not the
                // constant: a monitor a `--profile` authored `stacked` and the
                // operator then filled from the lobby is still stacked, and a
                // file that wrote `side_by_side` over it would re-arrange the
                // bridge on the next boot.
                //
                // Only a two-console monitor is actually split; a single console
                // is the whole screen and the field is ignored for it, so it is
                // left out rather than written as noise an operator must read
                // past.
                split: (stations.len() > 1).then(|| self.splits[index]),
                panes: stations
                    .iter()
                    .map(|s| PaneSlot::for_station(s.0.clone()))
                    .collect(),
            });
        }
        profile.displays = displays;
    }

    /// This layout as a standalone display profile — an empty profile with
    /// [`write_displays_into`](Self::write_displays_into) applied.
    pub fn to_profile(&self) -> BridgeProfile {
        let mut profile = BridgeProfile::empty();
        self.write_displays_into(&mut profile);
        profile
    }

    /// This layout as a **validated** display profile — the shape
    /// [`adopt_profile`](Self::adopt_profile), [`resolve`](super::bridge_profile::resolve)
    /// and the winit adapter all take.
    ///
    /// Infallible, and that is the point: a lawful layout cannot write a profile
    /// [`validate`](BridgeProfile::validate) rejects, so a consumer handed a
    /// `Result` here would have to invent a story for an error that cannot
    /// happen — and the stories are all bad (an `unwrap` that looks like a
    /// panic waiting to happen, or a swallowed error that hides a real bug).
    /// Every refusal is closed off by construction: the version is this build's,
    /// the roles are the two words this module writes, monitors are deduplicated
    /// so no id repeats, every emitted Station holds one or
    /// [`MAX_STATIONS_PER_MONITOR`] panes, each pane's label and station id are
    /// the station's own unique id, and there is always exactly one viewscreen
    /// (rule 1). The tests hold the proof directly —
    /// `a_layouts_profile_always_names_the_viewscreen` and
    /// `a_layouts_validated_profile_is_the_same_arrangement`.
    ///
    /// Valid is not the same as **lossless**, and this is not the latter:
    /// participant slots are not re-emitted, so
    /// [`adopt_profile`](Self::adopt_profile) of what this writes differs from
    /// the layout it was written from whenever that layout reserves anything.
    /// See [`reserved`](Self::reserved) — nothing today can reach the loss, and
    /// issue #1334 is where it stops being free. A screen's
    /// [`split_on`](Self::split_on) is lossy in the same one direction and for
    /// the file's own reason: a `[[display]]` holding fewer than two panes has no
    /// split to write (a one-pane Station's is ignored by definition), so a
    /// screen the operator authored `stacked` and then emptied comes back
    /// [`LAYOUT_SPLIT`].
    pub fn to_validated_profile(&self) -> ValidatedProfile {
        self.to_profile()
            .validate()
            // Unreachable by the construction argued above; if it ever fires, a
            // transition has produced an unlawful layout and the law itself is
            // wrong — which is a panic, not a value to hand a caller.
            .expect("a lawful layout writes a valid profile")
    }

    /// Read `profile`'s arrangement onto this bridge's monitors and roster,
    /// answering the layout it produces and everything that did not fit.
    ///
    /// Adoption **replaces** the seating rather than merging into it: a saved
    /// layout being pre-applied, or a `--profile` taking precedence over one, is
    /// the operator saying "this arrangement", not "these additions".
    ///
    /// Every seat is placed through [`apply`](Self::apply), so a profile cannot
    /// smuggle in an arrangement the law forbids: a monitor that is no longer
    /// connected, a station that is not on this ship's roster, a monitor that is
    /// now the viewscreen, a third console on a screen — each is refused by the
    /// same rule a button press would meet, and reported as a
    /// [`LayoutAdoption`] rather than silently dropped. A station whose monitor
    /// is missing is simply left unassigned, which is the graceful degradation a
    /// LAN party with different screens needs.
    ///
    /// A pane that names **no** station is reported the same way and is also
    /// **reserved** on its monitor (see the [`reserved`](Self::reserved) note):
    /// there is no seat to take, but the screen is not free either, and a
    /// viewscreen that moved onto it would cover a crew member's live console.
    /// It keeps the **index** its entry gave it, because that is where it is
    /// drawn — see [`Reservation`].
    ///
    /// A Station entry's `split` is adopted too, onto the monitor it names
    /// ([`splits`](Self::splits)): this is the one moment the file is in hand,
    /// and the arrangement an operator wrote down is the arrangement the screen
    /// is carved into from the boot frame onward.
    pub fn adopt_profile(&self, profile: &ValidatedProfile) -> (Self, Vec<LayoutAdoption>) {
        let mut notes = Vec::new();
        let mut next = Self {
            monitors: self.monitors.clone(),
            roster: self.roster.clone(),
            viewscreen: self.viewscreen,
            seats: vec![Vec::new(); self.monitors.len()],
            // Replaced along with the seating: adoption is "this arrangement",
            // and a profile that no longer authors a screen no longer carves it
            // or reserves it.
            splits: vec![LAYOUT_SPLIT; self.monitors.len()],
            reserved: vec![Vec::new(); self.monitors.len()],
        };

        // The viewscreen first: every seat below is judged against it, so
        // adopting the stations before the viewscreen moved would refuse the
        // ones that are lawful under the profile's own arrangement.
        if let Some(entry) = profile
            .displays
            .iter()
            .find(|d| d.role == DisplayRole::Viewscreen)
        {
            match next.set_viewscreen(&entry.identity) {
                Ok(moved) => next = moved,
                Err(refusal) => notes.push(LayoutAdoption::ViewscreenRefused {
                    monitor: entry.identity.clone(),
                    refusal,
                }),
            }
        }

        let mut seated: Vec<StationId> = Vec::new();
        for display in &profile.displays {
            let DisplayRole::Station { panes, split } = &display.role else {
                continue;
            };
            // How this screen is carved, from the entry that carves it. Recorded
            // before the panes below, and whether or not any of them is adopted:
            // the split is the operator's statement about the SCREEN, and a
            // station they later seat on it from the lobby lands on the axis they
            // authored.
            if let Some(index) = next.index_of(&display.identity) {
                next.splits[index] = *split;
            }
            for (position, pane) in panes.iter().enumerate() {
                let Some(id) = pane.station.as_deref() else {
                    // Nothing to seat — but the authored profile opens a
                    // surface here all the same, so the screen is recorded as
                    // taken rather than left looking free to rule 2's mirror.
                    // At the index the file gave it, because that is the half of
                    // the screen it is drawn on.
                    if let Some(index) = next.index_of(&display.identity) {
                        next.reserved[index].push(Reservation {
                            label: pane.label.clone(),
                            index: position,
                        });
                    }
                    notes.push(LayoutAdoption::PaneNamesNoStation {
                        monitor: display.identity.clone(),
                        label: pane.label.clone(),
                    });
                    continue;
                };
                let station = StationId(id.to_string());
                if seated.contains(&station) {
                    // Assigning it again would silently MOVE it, which reads as
                    // "the last mention won" — a rearrangement nobody asked for.
                    notes.push(LayoutAdoption::StationNamedTwice {
                        station,
                        monitor: display.identity.clone(),
                    });
                    continue;
                }
                match next.assign(&station, &display.identity) {
                    Ok(placed) => {
                        next = placed;
                        seated.push(station);
                    }
                    Err(refusal) => notes.push(LayoutAdoption::SeatRefused {
                        station,
                        monitor: display.identity.clone(),
                        refusal,
                    }),
                }
            }
        }

        (next, notes)
    }

    /// Rebuild this arrangement against a changed bridge: a different set of
    /// monitors (a screen unplugged, another plugged in) and/or a different
    /// station roster (a different ship class, or one whose stations changed).
    ///
    /// **Never an error.** A bridge whose screens changed is still a bridge, and
    /// there is nothing for an operator to fix at the moment a cable comes out —
    /// so every seat that still makes sense is kept, exactly where and in the
    /// order it was, and everything else degrades to unassigned. What this does
    /// *not* do is degrade quietly: each degradation comes back as a
    /// [`LayoutAdoption`], in the same vocabulary
    /// [`adopt_profile`](Self::adopt_profile) speaks.
    ///
    /// - The **viewscreen** is kept when its monitor survived — even when it is
    ///   not the primary, because that was the operator's choice and a re-plug of
    ///   some *other* screen is no reason to overrule it. Only when its own
    ///   monitor is gone does it fall back, and the fallback obeys rule 2 like
    ///   every other move: it prefers a screen holding **nothing** — the primary
    ///   if that one is free, else the first free one in monitor order — because
    ///   landing on an occupied screen covers a live console, and an authored one
    ///   ([`reserved`](Self::reserved)) it cannot even be moved off afterwards.
    ///   The fallback is always reported
    ///   ([`ViewscreenMonitorGone`](LayoutAdoption::ViewscreenMonitorGone)): it is
    ///   the one change here that would otherwise look like nothing happened. When
    ///   **no** screen is free the shared view still has to be somewhere, so it
    ///   lands primary-else-first and names what it landed on top of
    ///   ([`ViewscreenCoversOccupants`](LayoutAdoption::ViewscreenCoversOccupants)).
    /// - A **station whose monitor is gone** is left unassigned and named
    ///   ([`StationMonitorGone`](LayoutAdoption::StationMonitorGone)); it is not
    ///   re-homed onto some other screen, for the same reason a missing display's
    ///   role is not ([`ProfileProblem`](super::bridge_profile::ProfileProblem)).
    /// - A **station that left the roster** has its console closed and named
    ///   ([`StationOffRoster`](LayoutAdoption::StationOffRoster)).
    /// - A station that stays on a monitor that stays keeps its seat, and a
    ///   station **new** to the roster arrives unassigned — that is the ordinary
    ///   state of an unplaced console, not a degradation, so it gets no note.
    /// - A seat the law refuses for any other reason is a
    ///   [`SeatRefused`](LayoutAdoption::SeatRefused), exactly as in adoption.
    ///   The one that actually happens: the viewscreen fell back onto a monitor
    ///   that was holding consoles, which by rule 2 it may no longer.
    ///
    /// Handed **no** monitors, it keeps this layout whole and says so
    /// ([`NoMonitorsReported`](LayoutAdoption::NoMonitorsReported)) — a bridge is
    /// at least one screen (the lobby is drawn on it), so there is no lawful
    /// layout to degrade to, and an empty enumeration is far more likely a
    /// display driver restarting than a bridge that ceased to exist.
    pub fn reconcile(
        &self,
        monitors: &[DiscoveredMonitor],
        roster: impl IntoIterator<Item = StationId>,
    ) -> (Self, Vec<LayoutAdoption>) {
        let mut notes = Vec::new();
        let identities = dedup(monitors.iter().map(|d| d.identity.clone()));
        let roster = dedup(roster);

        let Some(first) = identities.first() else {
            notes.push(LayoutAdoption::NoMonitorsReported {
                kept: self.monitors.clone(),
            });
            return (self.clone(), notes);
        };

        // The viewscreen first, as in adoption: every seat below is judged
        // against it.
        let viewscreen = if identities.contains(self.viewscreen()) {
            self.viewscreen().clone()
        } else {
            // Rule 2 governs the fallback too. Choosing primary-else-first
            // *directly* — rather than through the transition that enforces the
            // rule — drops the shared view on top of whatever a surviving screen
            // is already holding, and an authored console is the worst case
            // because nothing here can move it out from under afterwards. So a
            // screen holding NOTHING is preferred: the primary when it is free,
            // else the first free one in monitor order.
            let free = |m: &MonitorIdentity| self.occupants_on(m).is_empty();
            let primary = monitors.iter().find(|d| d.primary).map(|d| &d.identity);
            let replacement = primary
                .filter(|m| free(m))
                .or_else(|| identities.iter().find(|m| free(m)))
                .cloned()
                // Nowhere free at all. The shared view still has to be
                // somewhere — a bridge is at least one screen — so the plain
                // primary-else-first fallback stands, and what it covers is
                // named below instead of being covered in silence.
                .unwrap_or_else(|| primary.cloned().unwrap_or_else(|| first.clone()));
            notes.push(LayoutAdoption::ViewscreenMonitorGone {
                monitor: self.viewscreen().clone(),
                replacement: replacement.clone(),
            });
            let occupants = self.occupants_on(&replacement);
            if !occupants.is_empty() {
                notes.push(LayoutAdoption::ViewscreenCoversOccupants {
                    monitor: replacement.clone(),
                    occupants,
                });
            }
            replacement
        };
        let index = identities
            .iter()
            .position(|m| m == &viewscreen)
            .expect("the viewscreen is one of the monitors it was chosen from");

        // An authored surface follows its screen: a monitor that is still here
        // is still carrying whatever the profile opened on it, and one that is
        // gone took its surface with it. Carried before the seats below so the
        // occupancy a seat is judged against is the whole of it. A screen's
        // authored split follows it the same way — a cable coming out of some
        // OTHER display is no reason to re-carve this one — and a monitor this
        // bridge has just gained arrives on `LAYOUT_SPLIT`, because nothing
        // authored it.
        let reserved: Vec<Vec<Reservation>> = identities
            .iter()
            .map(|m| match self.index_of(m) {
                Some(i) => self.reserved[i].clone(),
                None => Vec::new(),
            })
            .collect();
        let splits: Vec<PaneSplit> = identities.iter().map(|m| self.split_on(m)).collect();
        let mut next = Self {
            seats: vec![Vec::new(); identities.len()],
            monitors: identities,
            roster,
            viewscreen: index,
            splits,
            reserved,
        };

        // Re-seat in the order this layout holds them — monitor by monitor, seat
        // by seat — so a surviving screen's two halves stay in the order the
        // operator is looking at.
        for (seat, stations) in self.seats.iter().enumerate() {
            let monitor = &self.monitors[seat];
            for station in stations {
                if !next.roster.contains(station) {
                    notes.push(LayoutAdoption::StationOffRoster {
                        station: station.clone(),
                        monitor: monitor.clone(),
                    });
                    continue;
                }
                if !next.monitors.contains(monitor) {
                    notes.push(LayoutAdoption::StationMonitorGone {
                        station: station.clone(),
                        monitor: monitor.clone(),
                    });
                    continue;
                }
                match next.assign(station, monitor) {
                    Ok(placed) => next = placed,
                    Err(refusal) => notes.push(LayoutAdoption::SeatRefused {
                        station: station.clone(),
                        monitor: monitor.clone(),
                        refusal,
                    }),
                }
            }
        }

        (next, notes)
    }
}

/// Drop repeats, keeping the first occurrence and the input order.
fn dedup<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

// ── reporting types ─────────────────────────────────────────────────────────

/// One console occupying a slot on a monitor (issue #1332) — what
/// [`BridgeLayout::surface_rects`] hands back a rectangle for.
///
/// The two kinds are kept apart rather than flattened to a name, because the
/// adapter has to *treat* them differently: a station's console is the layout's
/// to open, move and close, and an authored one is only ever laid out. Flattening
/// them would have made the display layer re-derive which is which from a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceOccupant {
    /// A console this layout seated, keyed by its station.
    Station(StationId),
    /// A surface a hand-authored `--profile` opened, by its participant label —
    /// see [`BridgeLayout::reserved_on`].
    Reserved(String),
}

impl SurfaceOccupant {
    /// The name a person reads for this console — the station id, or the
    /// participant label. The same string [`BridgeLayout::occupants_on`] lists
    /// and the same one a `PaneSlot`'s `label` carries, which is what lets the
    /// pane bus resolve either kind by one key.
    pub fn name(&self) -> &str {
        match self {
            SurfaceOccupant::Station(station) => &station.0,
            SurfaceOccupant::Reserved(label) => label,
        }
    }

    /// The station this console belongs to, or `None` for an authored one.
    pub fn station(&self) -> Option<&StationId> {
        match self {
            SurfaceOccupant::Station(station) => Some(station),
            SurfaceOccupant::Reserved(_) => None,
        }
    }
}

/// What one monitor is holding — see [`BridgeLayout::occupancy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorOccupancy {
    pub monitor: MonitorIdentity,
    /// Whether this monitor is showing the shared viewscreen.
    pub is_viewscreen: bool,
    /// The stations seated on it, in the order their panes are laid out.
    pub stations: Vec<StationId>,
    /// The labels of authored surfaces on it this layout does not own — see
    /// [`BridgeLayout::reserved_on`]. Since issue #1332 they are consoles for
    /// every purpose but ownership: they block the viewscreen moving here **and**
    /// they are already subtracted from `free_slots`. Reported beside it all the
    /// same, because a consumer that wants to *name* what a screen is holding
    /// needs both lists and neither is derivable from the count.
    pub reserved: Vec<String>,
    /// How many more consoles it can take — `None` for the viewscreen, which
    /// takes none at all.
    ///
    /// `None` rather than `0`, because those are different facts and a zero here
    /// reads as the wrong one. "Full" is a monitor already holding
    /// [`MAX_STATIONS_PER_MONITOR`] consoles the operator can free a slot on; the
    /// viewscreen is a screen no console ever opens on. A consumer drawing
    /// occupied slots as `MAX_STATIONS_PER_MONITOR - free_slots` would have drawn
    /// two full slots on the shared view. `is_viewscreen` is the distinction, and
    /// [`ExclusionReason`] is how it reaches a button.
    pub free_slots: Option<usize>,
}

/// Why a monitor is not offered for a station's console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionReason {
    /// It is showing the shared viewscreen; a console never covers it.
    IsViewscreen,
    /// It already holds [`MAX_STATIONS_PER_MONITOR`] consoles — the stations
    /// seated on it *and* the authored surfaces it carries (issue #1332).
    Full,
}

impl ExclusionReason {
    /// A stable machine-readable token, for a log line or a UI state attribute.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExclusionReason::IsViewscreen => "is-viewscreen",
            ExclusionReason::Full => "full",
        }
    }
}

impl std::fmt::Display for ExclusionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one monitor's button says for one station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorChoice {
    /// This station's console is already on this monitor.
    Selected,
    /// Pressing this would open the console here.
    Eligible,
    /// Greyed, for this reason.
    Excluded(ExclusionReason),
}

impl MonitorChoice {
    /// Whether pressing this would be accepted (a re-press of the selected
    /// monitor is accepted too, as a no-op).
    pub fn is_offered(&self) -> bool {
        matches!(self, MonitorChoice::Selected | MonitorChoice::Eligible)
    }

    /// Why this is greyed, if it is.
    pub fn exclusion(&self) -> Option<ExclusionReason> {
        match self {
            MonitorChoice::Excluded(reason) => Some(*reason),
            _ => None,
        }
    }
}

/// One monitor's entry in a station's screen row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationMonitorChoice {
    pub monitor: MonitorIdentity,
    pub choice: MonitorChoice,
}

/// A whole station's screen row — see [`BridgeLayout::eligibility`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationEligibility {
    pub station: StationId,
    /// The monitor its console is on, if any.
    pub assigned_to: Option<MonitorIdentity>,
    /// One entry per monitor, in monitor order.
    pub monitors: Vec<StationMonitorChoice>,
}

impl StationEligibility {
    /// The monitors this station's console could move to — excluding the one it
    /// is already on.
    pub fn eligible(&self) -> impl Iterator<Item = &MonitorIdentity> {
        self.monitors
            .iter()
            .filter(|c| c.choice == MonitorChoice::Eligible)
            .map(|c| &c.monitor)
    }

    /// This row's entry for one monitor.
    pub fn choice_for(&self, monitor: &MonitorIdentity) -> Option<MonitorChoice> {
        self.monitors
            .iter()
            .find(|c| &c.monitor == monitor)
            .map(|c| c.choice)
    }
}

/// Something a profile asked for that this bridge could not take
/// ([`BridgeLayout::adopt_profile`]), or something a change to the bridge itself
/// took away ([`BridgeLayout::reconcile`]). Reported, never silent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutAdoption {
    /// The profile's viewscreen monitor could not be used; the layout kept the
    /// one it had.
    ViewscreenRefused {
        monitor: MonitorIdentity,
        refusal: LayoutRefusal,
    },
    /// A `[[display.pane]]` names a participant but no station — a hand-authored
    /// `--pane` profile. There is nothing to seat: the pane belongs to a person,
    /// not to a station.
    PaneNamesNoStation {
        monitor: MonitorIdentity,
        label: String,
    },
    /// The profile seats one station on two screens. The first placement stands;
    /// the second is reported rather than allowed to move it.
    StationNamedTwice {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// A seat the layout law refused — its monitor is gone, or is now the
    /// viewscreen, or is already full, or the station is not on this roster. The
    /// station is left unassigned.
    SeatRefused {
        station: StationId,
        monitor: MonitorIdentity,
        refusal: LayoutRefusal,
    },
    /// The monitor the operator had chosen for the viewscreen is no longer
    /// connected, so the viewscreen fell back to the primary (or, failing that,
    /// the first) screen — see [`BridgeLayout::reconcile`]. Reported because it
    /// is the one degradation that leaves a perfectly ordinary-looking bridge:
    /// without this note the operator's choice would simply be gone, with a
    /// working viewscreen somewhere else to hide it.
    ViewscreenMonitorGone {
        monitor: MonitorIdentity,
        replacement: MonitorIdentity,
    },
    /// The viewscreen's fallback had **no free screen** to land on, so it landed
    /// on one that was holding consoles — see [`BridgeLayout::reconcile`].
    ///
    /// Emitted *beside* [`ViewscreenMonitorGone`](Self::ViewscreenMonitorGone),
    /// not instead of it: that note says the operator's chosen screen is gone,
    /// this one says what the replacement is now sitting on top of. It is not a
    /// refusal, because a bridge is at least one screen and the shared view has
    /// to be somewhere — but it is never silent, and that is the whole reason it
    /// exists. A **seated** station covered this way at least earns its own
    /// [`SeatRefused`](Self::SeatRefused) when rule 2 unseats it below; an
    /// authored surface ([`BridgeLayout::reserved_on`]) earns nothing at all,
    /// cannot be unassigned, and would otherwise be covered without a word.
    ViewscreenCoversOccupants {
        monitor: MonitorIdentity,
        /// What that screen was holding, in the order
        /// [`BridgeLayout::occupants_on`] gives it — the order it is drawn on
        /// the screen, so the list reads the way the operator was looking at it.
        occupants: Vec<String>,
    },
    /// A station's console was open on a monitor that is no longer connected, so
    /// it is left unassigned rather than moved to some other screen — the same
    /// no-silent-re-homing rule a missing display gets one slice up.
    StationMonitorGone {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// A station that had a console open is not on the new roster — a different
    /// ship class, or one whose stations changed. Its console is closed.
    StationOffRoster {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// A station was seated, but its console could not be put on screen and the
    /// bounded rebuild budget is spent, so the seat was **given back**
    /// (issue #1331).
    ///
    /// Raised by `bridge_display::reconcile_seated_consoles` rather than by
    /// [`reconcile`](BridgeLayout::reconcile) or
    /// [`adopt_profile`](BridgeLayout::adopt_profile) — it is the adapter
    /// reporting that it cannot honour a lawful arrangement, which is the one
    /// degradation the law itself cannot see. Reported because the alternative
    /// is a station card claiming a screen that is black.
    ConsoleCouldNotOpen {
        station: StationId,
        monitor: MonitorIdentity,
    },
    /// A console **nobody moved** is being rebuilt, because the screen it is on
    /// gained or lost a neighbour and its half of that screen changed
    /// (issue #1332).
    ///
    /// Raised by `bridge_display::follow_layout_stations`, like
    /// [`ConsoleCouldNotOpen`](Self::ConsoleCouldNotOpen) and for the same
    /// reason: it is the adapter reporting a consequence the law cannot see.
    ///
    /// # Why this is a notice at all
    ///
    /// An Ultralight view is built at one size on one window, so a pane whose
    /// rectangle changes cannot be re-placed — it goes through `close` +
    /// `recreate` on the **same session token**, and the page reloads. Whoever
    /// was at that console therefore spends a page load disconnected, their
    /// station on `Backfill`, and their seat is claimable by somebody else for
    /// exactly that long. The reconnect restores it (`handle_identify`'s
    /// reconnect-yield) *if* nobody took it in the gap — and if somebody did,
    /// the station stays on AI control for the person who was at it.
    ///
    /// For the console the operator **moved**, that is the cost of the move and
    /// it needs no announcement. For its *neighbour* — a person who pressed
    /// nothing and whose screen is about to blink — it is a surprise, and the
    /// PRD's "reassignment must not steal seats gratuitously" is the reason the
    /// blink is announced rather than absorbed. The variant carries the console's
    /// name rather than a [`StationId`] because an authored `--pane` participant
    /// is re-tiled by the same rule and has no station id to carry.
    ///
    /// # Only when the OCCUPANCY changed
    ///
    /// This variant names one cause — a neighbour arrived or left — and it must
    /// only be raised for that cause. A screen whose consoles did not change but
    /// whose *geometry* did (a television renegotiating its mode, its identity
    /// carried across by `identify_stable`) re-tiles every pane on it too, and
    /// saying "the split changed" about that is a sentence the code cannot
    /// support: nothing joined or left. That case is
    /// [`ConsoleResized`](Self::ConsoleResized) — the same rebuild, the true
    /// reason.
    ConsoleRetiling {
        /// The station id, or the participant label of an authored surface —
        /// [`SurfaceOccupant::name`].
        console: String,
        monitor: MonitorIdentity,
    },
    /// A console **nobody moved** is being rebuilt, because the monitor it is on
    /// changed size while holding exactly the consoles it already held
    /// (issue #1332's fix round).
    ///
    /// [`ConsoleRetiling`](Self::ConsoleRetiling)'s sibling, and everything in
    /// that note about *why a rebuild is announced at all* applies here word for
    /// word: the view was built at one size, the page reloads, and the person at
    /// it spends that load on `Backfill`. What differs is only the true cause. A
    /// display that renegotiates its mode in place — a television waking, an
    /// EDID handshake settling — keeps its identity
    /// (`bridge_profile::identify_stable`) and reports new pixels, so every pane
    /// on it gets a new rectangle with the same neighbours it always had.
    ///
    /// It is a separate variant rather than a parameter because the two are
    /// separate **sentences**: `t()` interpolates values into a sentence, and a
    /// translator handed `{cause}` cannot see what grammar is about to land in
    /// it.
    ConsoleResized {
        /// The station id, or the participant label of an authored surface —
        /// [`SurfaceOccupant::name`].
        console: String,
        monitor: MonitorIdentity,
    },
    /// [`reconcile`](BridgeLayout::reconcile) was handed no monitors at all. A
    /// bridge is at least one screen, so the layout was kept exactly as it was;
    /// these are the monitors it still names.
    NoMonitorsReported { kept: Vec<MonitorIdentity> },
}

impl LayoutAdoption {
    /// The `strings.csv` id a **player-visible** surface renders this note
    /// through (issue #1330) — see [`LayoutRefusal::string_id`] for why this is
    /// a separate rendering from [`Display`](std::fmt::Display).
    pub fn string_id(&self) -> &'static str {
        match self {
            LayoutAdoption::ViewscreenRefused { .. } => {
                "server.bridge_layout.adopt_viewscreen_refused"
            }
            LayoutAdoption::PaneNamesNoStation { .. } => {
                "server.bridge_layout.adopt_pane_no_station"
            }
            LayoutAdoption::StationNamedTwice { .. } => "server.bridge_layout.adopt_station_twice",
            LayoutAdoption::SeatRefused { .. } => "server.bridge_layout.adopt_seat_refused",
            LayoutAdoption::ViewscreenMonitorGone { .. } => {
                "server.bridge_layout.adopt_viewscreen_gone"
            }
            LayoutAdoption::ViewscreenCoversOccupants { .. } => {
                "server.bridge_layout.adopt_viewscreen_covers"
            }
            LayoutAdoption::StationMonitorGone { .. } => {
                "server.bridge_layout.adopt_station_monitor_gone"
            }
            LayoutAdoption::StationOffRoster { .. } => {
                "server.bridge_layout.adopt_station_off_roster"
            }
            LayoutAdoption::ConsoleCouldNotOpen { .. } => {
                "server.bridge_layout.adopt_console_could_not_open"
            }
            LayoutAdoption::ConsoleRetiling { .. } => "server.bridge_layout.adopt_console_retiling",
            LayoutAdoption::ConsoleResized { .. } => "server.bridge_layout.adopt_console_resized",
            LayoutAdoption::NoMonitorsReported { .. } => "server.bridge_layout.adopt_no_monitors",
        }
    }

    /// The `{placeholder}` values [`string_id`](Self::string_id)'s row
    /// interpolates, in a fixed order.
    ///
    /// The nested [`LayoutRefusal`] two of these carry is **not** among them —
    /// see [`cause`](Self::cause).
    pub fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            LayoutAdoption::ViewscreenRefused { monitor, .. } => {
                vec![("monitor", monitor.as_str().to_string())]
            }
            LayoutAdoption::PaneNamesNoStation { monitor, label } => vec![
                ("label", label.clone()),
                ("monitor", monitor.as_str().to_string()),
            ],
            LayoutAdoption::StationNamedTwice { station, monitor }
            | LayoutAdoption::SeatRefused {
                station, monitor, ..
            }
            | LayoutAdoption::StationMonitorGone { station, monitor }
            | LayoutAdoption::StationOffRoster { station, monitor }
            | LayoutAdoption::ConsoleCouldNotOpen { station, monitor } => vec![
                ("station", station.0.clone()),
                ("monitor", monitor.as_str().to_string()),
            ],
            LayoutAdoption::ConsoleRetiling { console, monitor }
            | LayoutAdoption::ConsoleResized { console, monitor } => vec![
                ("console", console.clone()),
                ("monitor", monitor.as_str().to_string()),
            ],
            LayoutAdoption::ViewscreenMonitorGone {
                monitor,
                replacement,
            } => vec![
                ("monitor", monitor.as_str().to_string()),
                ("replacement", replacement.as_str().to_string()),
            ],
            LayoutAdoption::ViewscreenCoversOccupants { monitor, occupants } => vec![
                ("monitor", monitor.as_str().to_string()),
                // Already the player-facing list `occupants_on` draws on a
                // button, so it is joined the way `occupant_params` joins one:
                // unquoted, because quotes read as stray punctuation in a
                // sentence somebody is reading off a screen.
                ("occupants", occupants.join(", ")),
            ],
            LayoutAdoption::NoMonitorsReported { kept } => {
                vec![("count", kept.len().to_string())]
            }
        }
    }

    /// The refusal that produced this note, when there was one.
    ///
    /// Reported as its own line beside the note rather than interpolated into
    /// it: `t()` substitutes **values** into a sentence, and a second localised
    /// sentence is not a value — a translator handed `{reason}` cannot see what
    /// grammar is about to land in it, and this build would have to resolve two
    /// ids in a fixed English order to fill it.
    pub fn cause(&self) -> Option<&LayoutRefusal> {
        match self {
            LayoutAdoption::ViewscreenRefused { refusal, .. }
            | LayoutAdoption::SeatRefused { refusal, .. } => Some(refusal),
            _ => None,
        }
    }
}

impl std::fmt::Display for LayoutAdoption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutAdoption::ViewscreenRefused { monitor, refusal } => write!(
                f,
                "the saved layout puts the viewscreen on monitor {monitor}, which this bridge \
                 cannot do ({refusal}); the viewscreen stays where it is"
            ),
            LayoutAdoption::PaneNamesNoStation { monitor, label } => write!(
                f,
                "the pane for {label:?} on monitor {monitor} names no station, so it is a \
                 participant's own pane rather than a station console; the layout leaves it alone"
            ),
            LayoutAdoption::StationNamedTwice { station, monitor } => write!(
                f,
                "the layout puts station {:?} on more than one screen; its first placement stands \
                 and monitor {monitor} is ignored",
                station.0
            ),
            LayoutAdoption::SeatRefused {
                station,
                monitor,
                refusal,
            } => write!(
                f,
                "station {:?}'s console cannot open on monitor {monitor} ({refusal}); it is left \
                 unassigned",
                station.0
            ),
            LayoutAdoption::ViewscreenMonitorGone {
                monitor,
                replacement,
            } => write!(
                f,
                "monitor {monitor} was showing the viewscreen and is no longer connected, so the \
                 viewscreen moved to {replacement}; pick the screen you want it on again once \
                 {monitor} is back"
            ),
            LayoutAdoption::ViewscreenCoversOccupants { monitor, occupants } => write!(
                f,
                "no screen was free for the viewscreen to fall back to, so it moved onto monitor \
                 {monitor} on top of {}; unassign them, or plug the missing display back in and \
                 pick a screen again",
                occupants
                    .iter()
                    .map(|o| format!("{o:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            LayoutAdoption::StationMonitorGone { station, monitor } => write!(
                f,
                "station {:?}'s console was on monitor {monitor}, which is no longer connected; it \
                 is left unassigned rather than moved to another screen",
                station.0
            ),
            LayoutAdoption::StationOffRoster { station, monitor } => write!(
                f,
                "station {:?} is not on this ship's roster, so its console on monitor {monitor} is \
                 closed",
                station.0
            ),
            LayoutAdoption::ConsoleCouldNotOpen { station, monitor } => write!(
                f,
                "station {:?}'s console could not be put on monitor {monitor}, so its seat is \
                 given back and the screen is free again; open it on another screen, or try \
                 that one again",
                station.0
            ),
            LayoutAdoption::ConsoleRetiling { console, monitor } => write!(
                f,
                "monitor {monitor}'s split changed, so {console:?}'s console — which nobody asked \
                 to move — is rebuilt at its new half; whoever is at it reconnects on the same \
                 identity once the page loads, and their station is on AI control until it does \
                 — and stays there if somebody else claimed it in the meantime",
            ),
            LayoutAdoption::ConsoleResized { console, monitor } => write!(
                f,
                "monitor {monitor} changed size, so {console:?}'s console — which nobody asked to \
                 move, and which kept the same neighbours — is rebuilt to fit it; whoever is at \
                 it reconnects on the same identity once the page loads, and their station is on \
                 AI control until it does — and stays there if somebody else claimed it in the \
                 meantime",
            ),
            LayoutAdoption::NoMonitorsReported { kept } => write!(
                f,
                "no monitors were reported at all, and a bridge is at least one screen; the layout \
                 is left as it was, still naming {}",
                match kept.len() {
                    1 => "1 monitor".to_string(),
                    n => format!("{n} monitors"),
                }
            ),
        }
    }
}

#[cfg(test)]
#[path = "bridge_layout_tests.rs"]
mod tests;
