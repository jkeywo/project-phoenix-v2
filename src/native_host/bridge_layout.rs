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
//!    [`MAX_STATIONS_PER_MONITOR`] stations, split deterministically side by
//!    side. That is [`MAX_PANES_PER_STATION`] — the same legibility bound the
//!    profile enforces at author time, restated as a runtime precondition rather
//!    than duplicated as a second number.
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

/// How a monitor holding two station consoles is divided.
///
/// Side by side, always — the PRD's "two consoles on one monitor split it side by
/// side automatically". It is a constant rather than a per-monitor choice because
/// the lobby offers no control for it: an operator picks *which* screen, never
/// how it is carved, and a layout that could differ per monitor would be a
/// setting nothing sets.
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
        occupants: Vec<StationId>,
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
            LayoutRefusal::MonitorFull { monitor, occupants } => write!(
                f,
                "monitor {monitor} already holds {} console(s) ({}), which is the maximum of \
                 {MAX_STATIONS_PER_MONITOR}: a console is authored to be read one, or \
                 side-by-side two, to a screen. Free a slot or pick another screen",
                occupants.len(),
                station_list(occupants),
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
            LayoutRefusal::MonitorFull { monitor, occupants } => vec![
                ("monitor", monitor.as_str().to_string()),
                ("stations", station_params(occupants)),
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

/// `"helm", "weapons"` — station ids, quoted and joined, for a refusal message.
fn station_list(stations: &[StationId]) -> String {
    stations
        .iter()
        .map(|s| format!("{:?}", s.0))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `helm, weapons` — station ids joined for a **player-visible** parameter.
///
/// Unquoted, unlike [`station_list`]: the quotes in an operator log line read as
/// "this is a machine key", and on the lobby's own surface they read as stray
/// punctuation in a sentence.
fn station_params(stations: &[StationId]) -> String {
    stations
        .iter()
        .map(|s| s.0.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Everything on one monitor, for an operator log line: its seated consoles
/// then the authored surfaces it also carries.
fn occupant_list(stations: &[StationId], panes: &[String]) -> String {
    join_occupants(
        stations.iter().map(|s| format!("{:?}", s.0)),
        panes.iter().map(|p| format!("{p:?}")),
    )
}

/// Everything on one monitor, for a **player-visible** parameter. See
/// [`station_params`] for why nothing is quoted here.
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
    /// Parallel to `monitors`: the labels of surfaces an **authored** profile
    /// opened on each, which this layout does not own (issue #1330).
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
    /// A reserved label is therefore an occupant for the purpose of rule 2's
    /// mirror ([`set_viewscreen`](Self::set_viewscreen)) and nothing else. It is
    /// **not** a seat: it is not on the roster, it has no
    /// [`eligibility`](Self::eligibility) row, no action can move or free it, and
    /// it does not consume a monitor's console capacity — seating a station
    /// beside an authored pane is the pane host's compositing question
    /// (issue #1124), not this law's, and the viewscreen is the only surface this
    /// module actually moves today. Faking a `StationId` for one instead would
    /// have put a station no ship has on the roster-driven rows.
    reserved: Vec<Vec<String>>,
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
        Ok(Self {
            monitors,
            roster,
            viewscreen: index,
            seats,
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

    /// The stations seated on `monitor`, in the order their panes are laid out.
    /// Empty for the viewscreen, and for a monitor this bridge does not have.
    pub fn stations_on(&self, monitor: &MonitorIdentity) -> &[StationId] {
        match self.index_of(monitor) {
            Some(i) => &self.seats[i],
            None => &[],
        }
    }

    /// The labels of authored surfaces on `monitor` this layout does not own —
    /// see the [`reserved`](Self::reserved) note. Empty for a monitor this
    /// bridge does not have.
    pub fn reserved_on(&self, monitor: &MonitorIdentity) -> &[String] {
        match self.index_of(monitor) {
            Some(i) => &self.reserved[i],
            None => &[],
        }
    }

    /// Everything open on `monitor`, as the names a person reads: the stations
    /// this layout seated, then the authored surfaces it merely knows about.
    ///
    /// What the lobby's monitor row draws on a button, and the same list a
    /// refusal names — one function, so the row cannot promise a press the law
    /// then refuses for a reason the row never showed.
    pub fn occupants_on(&self, monitor: &MonitorIdentity) -> Vec<String> {
        self.stations_on(monitor)
            .iter()
            .map(|s| s.0.clone())
            .chain(self.reserved_on(monitor).iter().cloned())
            .collect()
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
                panes: self.reserved[index].clone(),
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
        if self.seats[index].len() >= MAX_STATIONS_PER_MONITOR {
            return Err(LayoutRefusal::MonitorFull {
                monitor: monitor.clone(),
                occupants: self.seats[index].clone(),
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
            reserved: self.reserved[index].clone(),
            free_slots: (!is_viewscreen)
                .then(|| MAX_STATIONS_PER_MONITOR - self.seats[index].len()),
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
                    } else if self.seats[i].len() >= MAX_STATIONS_PER_MONITOR {
                        MonitorChoice::Excluded(ExclusionReason::Full)
                    } else {
                        MonitorChoice::Eligible
                    },
                })
                .collect(),
        }
    }

    /// Where each of `monitor`'s seated consoles is drawn on it, in seat order.
    ///
    /// The deterministic split, resolved against real geometry: one console is
    /// the whole monitor, two divide it side by side with no gap and no overlap.
    /// Empty for the viewscreen and for a monitor this bridge lacks.
    pub fn station_rects(
        &self,
        monitor: &MonitorIdentity,
        geometry: &MonitorGeometry,
    ) -> Vec<(StationId, PaneRect)> {
        let stations = self.stations_on(monitor);
        stations
            .iter()
            .cloned()
            .zip(pane_rects(geometry, LAYOUT_SPLIT, stations.len()))
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
                // Only a two-console monitor is actually split; a single console
                // is the whole screen and the field is ignored for it, so it is
                // left out rather than written as noise an operator must read
                // past.
                split: (stations.len() > 1).then_some(LAYOUT_SPLIT),
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
    pub fn adopt_profile(&self, profile: &ValidatedProfile) -> (Self, Vec<LayoutAdoption>) {
        let mut notes = Vec::new();
        let mut next = Self {
            monitors: self.monitors.clone(),
            roster: self.roster.clone(),
            viewscreen: self.viewscreen,
            seats: vec![Vec::new(); self.monitors.len()],
            // Replaced along with the seating: adoption is "this arrangement",
            // and a profile that no longer authors a participant pane no longer
            // reserves the screen it was on.
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
            let DisplayRole::Station { panes, .. } = &display.role else {
                continue;
            };
            for pane in panes {
                let Some(id) = pane.station.as_deref() else {
                    // Nothing to seat — but the authored profile opens a
                    // surface here all the same, so the screen is recorded as
                    // taken rather than left looking free to rule 2's mirror.
                    if let Some(index) = next.index_of(&display.identity) {
                        next.reserved[index].push(pane.label.clone());
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
    ///   monitor is gone does it fall back to primary-else-first, and that
    ///   fallback is reported ([`ViewscreenMonitorGone`](LayoutAdoption::ViewscreenMonitorGone)):
    ///   it is the one change here that would otherwise look like nothing
    ///   happened.
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
            let replacement = monitors
                .iter()
                .find(|d| d.primary)
                .map(|d| d.identity.clone())
                .unwrap_or_else(|| first.clone());
            notes.push(LayoutAdoption::ViewscreenMonitorGone {
                monitor: self.viewscreen().clone(),
                replacement: replacement.clone(),
            });
            replacement
        };
        let index = identities
            .iter()
            .position(|m| m == &viewscreen)
            .expect("the viewscreen is one of the monitors it was chosen from");

        // An authored surface follows its screen: a monitor that is still here
        // is still carrying whatever the profile opened on it, and one that is
        // gone took its surface with it. Carried before the seats below so the
        // occupancy a seat is judged against is the whole of it.
        let reserved: Vec<Vec<String>> = identities
            .iter()
            .map(|m| self.reserved_on(m).to_vec())
            .collect();
        let mut next = Self {
            seats: vec![Vec::new(); identities.len()],
            monitors: identities,
            roster,
            viewscreen: index,
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

/// What one monitor is holding — see [`BridgeLayout::occupancy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorOccupancy {
    pub monitor: MonitorIdentity,
    /// Whether this monitor is showing the shared viewscreen.
    pub is_viewscreen: bool,
    /// The stations seated on it, in the order their panes are laid out.
    pub stations: Vec<StationId>,
    /// The labels of authored surfaces on it this layout does not own — see
    /// [`BridgeLayout::reserved_on`]. They block the viewscreen moving here and
    /// nothing else, so they are reported beside `free_slots` rather than
    /// subtracted from it.
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
    /// It already holds [`MAX_STATIONS_PER_MONITOR`] consoles.
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
            LayoutAdoption::StationMonitorGone { .. } => {
                "server.bridge_layout.adopt_station_monitor_gone"
            }
            LayoutAdoption::StationOffRoster { .. } => {
                "server.bridge_layout.adopt_station_off_roster"
            }
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
            | LayoutAdoption::StationOffRoster { station, monitor } => vec![
                ("station", station.0.clone()),
                ("monitor", monitor.as_str().to_string()),
            ],
            LayoutAdoption::ViewscreenMonitorGone {
                monitor,
                replacement,
            } => vec![
                ("monitor", monitor.as_str().to_string()),
                ("replacement", replacement.as_str().to_string()),
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
