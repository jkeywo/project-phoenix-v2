//! The winit/Bevy adapter for bridge-display profiles (issue #1123).
//!
//! [`super::bridge_profile`] is the pure model; this is the layer that reads real
//! monitors off Bevy's [`Monitor`](bevy::window::Monitor) components and opens
//! one borderless-fullscreen surface per configured monitor. It is testable only
//! under the `#[ignore]`d integration test on a machine with real displays
//! (`tests/native_bridge_displays.rs`), exactly as issue #1121's and #1122's GPU
//! proofs are — CI is GPU-less and headless, so nothing here runs there.
//!
//! # What it does, once monitors are known
//!
//! * The **viewscreen** role goes on the process's **primary window** (the one
//!   `native_render_stack` already opened and the game cameras already target),
//!   put into [`WindowMode::BorderlessFullscreen`] on the viewscreen monitor. So
//!   the #1121 viewscreen render lands on the right display with no camera
//!   retargeting at all.
//! * Each **Station** role spawns an *additional* borderless-fullscreen
//!   [`Window`] on its monitor, tagged with [`BridgeSurface`], and its one or two
//!   pane rectangles are published in [`BridgeStationSurfaces`] for the pane host
//!   to compose onto (see the note there).
//! * A monitor the profile assigns but that is **not present** is logged, not
//!   re-homed; a present monitor with **no assignment** is logged too. The pure
//!   [`resolve`](super::bridge_profile::resolve) makes that judgement; this layer
//!   only reports it.
//!
//! # Why an exclusive system, and why it retries
//!
//! `bevy_winit` fills the [`Monitor`](bevy::window::Monitor) entities a frame or
//! two into the run, not at `Startup`. So [`apply_bridge_profile`] is an
//! exclusive system that runs every frame and does its **boot** work once: it
//! returns quietly while no monitors are known yet, then opens the surfaces and
//! inserts [`BridgeDisplayApplied`] so it never runs again. It is exclusive
//! because it reads the monitor entities, mutates the primary window, spawns new
//! windows and inserts resources in one pass — the same shape
//! `panes::ultralight::init_pane_host` uses for the same reason.
//!
//! # Boot is once; the arrangement is apply-on-change (issues #1330, #1331)
//!
//! The **boot** pass is a one-shot: it seeds the live layout and applies an
//! authored `--profile` exactly as issue #1123 did. Everything after it is
//! change-driven, by two followers that read the one live
//! [`BridgeLayoutResource`] and nothing else:
//!
//! * [`follow_layout_viewscreen`] compares the layout's viewscreen against the
//!   one [`BridgeDisplayApplied`] records and moves the primary window when —
//!   and only when — those differ;
//! * [`follow_layout_stations`] compares the layout's **seating** against
//!   [`BridgeStationSurfaces`], opens a Station window and a console pane for a
//!   station the layout has just seated, and closes them for one it has not. It
//!   runs on a change to *either* of its two inputs — the layout, and the
//!   monitors a seat is made of — because a display that left and came back has
//!   to get its Station window rebuilt for a console the layout never stopped
//!   seating.
//! * [`reconcile_seated_consoles`] then checks that the applier's work actually
//!   landed: a seated station whose console is not open on the bus, or has no
//!   surface to be composited onto, is rebuilt boundedly and — when that budget
//!   is spent — has its seat given back through the law, with a notice. It is
//!   what stops a failure below this layer showing as a lit black screen.
//!
//! Station windows are therefore spawned and despawned *while the host runs*,
//! which is what makes "press a screen button and a console opens on that
//! monitor" true. Nothing else in this module opens one, and the lobby does not:
//! a press and an unplug both arrive as a lawful transition on the layout, and
//! two places that opened consoles would disagree the first time either ran.
//!
//! That is what makes the added machinery free to a host nobody touched. At boot
//! the recorded identity is set to whatever the layout was *seeded* with,
//! whether or not a window was moved to reach it, so:
//!
//! * an authored `--profile` is applied exactly as it was before, by the code
//!   below, and then recorded — so the follower has nothing to do;
//! * a host with **no** `--profile` seeds its layout from the monitors it found
//!   ([`BridgeLayout::from_discovered`]) and touches no window at all: the
//!   window stays where the OS opened it, which is the #1121 behaviour this
//!   module promised when it had no config to run under.
//!
//! The promise therefore moved from "no config, so nothing runs" to "a config
//! with no action, so nothing changes": **only a press or an unplug moves the
//! window**, which is what AGENTS.md tells an operator, so a run in which
//! neither happens produces the same window as it did before this issue. The
//! two are one rule and not two, because they reach the follower by one route —
//! a lawful transition on [`BridgeLayoutResource`]'s layout. The unplug half is
//! narrower than it sounds: the viewscreen moves only when the monitor it is
//! actually on stops being reported for a whole
//! [`DISPLAY_LOSS_DEBOUNCE_FRAMES`] window, and a display arriving, leaving,
//! moving or changing its resolution beside it moves nothing — see
//! [`reconcile_layout`], which is where that is made true rather than merely
//! intended.
//!
//! The synthesised config is what lets [`watch_runtime_displays`] run on every
//! native host rather than only on a `--profile` one, which is what the monitor
//! row needs to follow a cable.

use bevy::prelude::*;
use bevy::window::{
    Monitor, MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, Window, WindowMode,
};

use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::panes::frame_stats::PaneExperiments;

use super::bridge_layout::{BridgeLayout, LayoutAction, LayoutAdoption};
use super::bridge_profile::{
    identify, identify_stable, present_assigned_identities, resolve, runtime_display_losses,
    runtime_display_returns, DiscoveredMonitor, DisplayRole, MonitorGeometry, MonitorIdentity,
    PaneRect, PaneSlot, RawMonitor, RuntimeDisplayLoss, ValidatedProfile,
};
use super::console_assignment::{self, PendingConsoleClaims};
use super::host_lobby::LayoutNotice;

/// The validated bridge profile a native host applies to its displays.
///
/// Inserted by `native_host::app` when `--profile` gave one and it validated —
/// a bad profile fails at the prompt (issue #1123), so by the time this
/// resource exists its roles and density are already sound.
///
/// Since issue #1330 a host given **no** `--profile` gets one too, synthesised
/// by [`apply_bridge_profile`] from the monitors it found, so that
/// [`watch_runtime_displays`] runs on every native host rather than only on a
/// configured one. [`authored`](Self::authored) is the difference, and it is
/// load-bearing rather than informational — see its note.
#[derive(Resource, Clone, Debug)]
pub struct BridgeDisplayConfig {
    pub profile: ValidatedProfile,
    /// Whether an operator wrote this profile (`--profile`) or the host
    /// synthesised it from the displays it enumerated.
    ///
    /// An authored profile is an **instruction**: it moves the primary window
    /// into borderless fullscreen on the monitor it names, which is what
    /// issue #1123 built. A synthesised one is a **description** of where the
    /// windows already are, and applying it would put a host that was launched
    /// with no display arguments at all into borderless fullscreen — a change
    /// of behaviour nobody asked for and the one thing issue #1330's second
    /// acceptance criterion forbids. So this decides whether the viewscreen is
    /// *placed* at boot or merely *recorded*.
    ///
    /// Since issue #1334 it decides a second thing, and that one is a data
    /// guard rather than a placement question: it is the gate on the saved
    /// per-ship-class layouts ([`super::layout_store_systems`]). An authored run
    /// has nothing pre-applied over its profile and **writes nothing back** —
    /// the operator's file wins for that run, and a save would silently drop its
    /// `--pane` participant slots. See
    /// [`BridgeLayout::reserved_on`](super::bridge_layout::BridgeLayout::reserved_on).
    pub authored: bool,
}

/// Tags a window this adapter placed on a specific bridge monitor — the primary
/// window for the viewscreen, or a spawned window for a Station.
///
/// Carries the monitor's stable identity and a human role summary so the
/// integration test (and any diagnostics) can tell which surface is which
/// without re-deriving it.
#[derive(Component, Clone, Debug)]
pub struct BridgeSurface {
    pub identity: String,
    pub role: String,
}

/// One pane's home on a Station window: who sits at it and the rectangle it
/// occupies, in that window's physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct StationPane {
    pub label: String,
    pub rect: PaneRect,
    /// The station whose console this pane shows, when the **layout** placed it
    /// (issue #1331) — the `PaneSlot::station` of the slot it came from.
    ///
    /// `None` for a hand-authored `--pane <NAME>` slot, which belongs to a
    /// person rather than to a station. That is the whole difference
    /// [`follow_layout_stations`] works on: a pane with a station is one the
    /// lobby owns and may close, and a pane without one is the operator's and is
    /// never touched.
    pub station: Option<crate::core::messages::StationId>,
}

/// A Station surface this adapter opened: its monitor identity, the window
/// entity, the monitor geometry it covers, and the pane rectangles laid out
/// across it.
#[derive(Clone, Debug)]
pub struct BridgeStationSurface {
    pub identity: String,
    pub window: Entity,
    /// The [`Monitor`] entity this window's `WindowMode::BorderlessFullscreen`
    /// was spawned against (issue #1331).
    ///
    /// `bevy_winit` despawns a `Monitor` entity when its display stops being
    /// reported and spawns a **new** one when it comes back, so a display that
    /// blipped through a dock or a GPU reset keeps its stable identity while
    /// changing its entity id. The window's mode still names the OLD entity, and
    /// `bevy_winit` re-applies fullscreen only on a change of `Window::mode` —
    /// so without this the window would stay wherever the OS parked it while the
    /// compositing and the input routing used the RETURNED monitor's geometry.
    /// [`follow_layout_stations`] compares the two and rewrites the mode; this is
    /// what makes "differs" a question the adapter can answer.
    pub monitor: Entity,
    /// The monitor's live geometry — its scale factor and its top-left on the
    /// virtual desktop. Carried so the pane host (issue #1124) can composite each
    /// pane at the right physical size and route input in the monitor's own
    /// coordinate space without re-querying the winit monitor.
    pub geometry: MonitorGeometry,
    pub panes: Vec<StationPane>,
}

/// Every Station surface open right now — the ones an authored `--profile`
/// opened at boot, and the ones the lobby's screen rows opened since
/// (issue #1331).
///
/// The seam the pane host composites onto: `panes::ultralight`'s
/// `init_pane_host` seats a `--pane <NAME>` on the slot carrying its label, and
/// `open_pending_views` seats a station's console on the slot carrying its
/// station id. Both read this one list, so there is one answer to "where does
/// this pane live" rather than a boot answer and a runtime answer.
///
/// It is **live**, not a boot record: [`follow_layout_stations`] rewrites the
/// station panes of every surface whenever the layout moves, spawns a Station
/// window for a monitor that gains its first console and despawns one that
/// loses its last.
#[derive(Resource, Clone, Debug, Default)]
pub struct BridgeStationSurfaces(pub Vec<BridgeStationSurface>);

impl BridgeStationSurfaces {
    /// The surface open on `identity`, if any.
    pub fn on(&self, identity: &str) -> Option<&BridgeStationSurface> {
        self.0.iter().find(|s| s.identity == identity)
    }

    /// The slot a pane named `label` should be composited into: its monitor's
    /// surface and the pane's own rectangle.
    ///
    /// One lookup for both kinds of pane — a `--pane` participant label and a
    /// station id — because [`StationPane::label`] carries both (a lobby-opened
    /// console's label *is* its station id; see `PaneSlot::for_station`).
    ///
    /// # A duplicate label resolves to the STATION's slot
    ///
    /// Two slots can only carry one label if an authored `--profile` named a
    /// participant pane for a station this hull has, and
    /// `app::install_world_selection` refuses exactly that at load — on the
    /// `--world` path and on the `--lobby` one — so a duplicate is not reachable
    /// from authored input (issue #1331). This tie-break is what makes the
    /// unreachable case *decided* rather than positional: a first-match by
    /// iteration order would resolve the station's console onto the authored
    /// participant's rectangle on the authored monitor, so the console would be
    /// built on a screen nobody chose while the chosen one stayed black — and
    /// `reconcile_seated_consoles`' health check, which asks only whether SOME
    /// slot carries the name, would be satisfied by the wrong one and never fire.
    pub fn slot_for(&self, label: &str) -> Option<(&BridgeStationSurface, &StationPane)> {
        let named = || {
            self.0
                .iter()
                .flat_map(|s| s.panes.iter().map(move |p| (s, p)))
                .filter(|(_, p)| p.label == label)
        };
        named()
            .find(|(_, p)| p.station.is_some())
            .or_else(|| named().next())
    }
}

/// The bridge arrangement this host is **running**, as opposed to the file it
/// booted from (issue #1330).
///
/// Seeded by [`apply_bridge_profile`] the frame winit first reports its
/// monitors, edited by the lobby's monitor row
/// (`host_lobby::drain_surface_records`), rebuilt by
/// [`watch_runtime_displays`] when a cable moves, re-seated once from this ship
/// class's remembered bridge the moment the hull becomes known
/// ([`super::layout_store_systems`], issue #1334 — which also files every
/// accepted change back to that class's saved layout), and read by
/// [`follow_layout_viewscreen`] to decide whether a window has to move.
///
/// It is the single place a live arrangement lives, so the row the operator is
/// looking at and the window they are looking at cannot disagree: both are
/// projections of this one [`BridgeLayout`].
#[derive(Resource, Clone, Debug)]
pub struct BridgeLayoutResource {
    /// The lawful arrangement. Only ever replaced by a
    /// [`BridgeLayout`] transition, never edited field-wise.
    pub layout: BridgeLayout,
    /// The monitors it was last built against — the geometry and OS names the
    /// row's buttons are drawn from, which the layout itself does not carry.
    pub monitors: Vec<DiscoveredMonitor>,
    /// What the lobby is **owed** about the presses and bridge changes it has
    /// not been told about yet.
    ///
    /// **Appended to, never assigned, and drained by the publisher**
    /// ([`host_lobby::publish_bridge_layout`](super::host_lobby)). Three systems
    /// in two unordered chains write here — the lobby's `drain_surface_records`
    /// (in `PreUpdate`), and this module's [`reconcile_layout`] and
    /// [`reconcile_seated_consoles`] — so whichever ran last used to erase what
    /// the others had just said. The frame in which that matters most is exactly
    /// the frame worth reporting: a press landing on the same frame a console's
    /// seat is surrendered would drop `ConsoleCouldNotOpen`, the one notice the
    /// reconciler exists to deliver. Every writer therefore extends, and a
    /// frame's notices surface together.
    ///
    /// Draining at the publisher rather than at the next write is what keeps
    /// them from accumulating: they are owed until they have been pushed, and
    /// then they are not. The drain deliberately does not mark the resource
    /// changed, so it cannot schedule a second push that blanks the row it just
    /// filled.
    ///
    /// **Boot-time** adoption notes are deliberately *not* here — they are logged
    /// instead. A `--profile` full of `--pane` participant slots produces one
    /// note per slot, and opening the lobby with a wall of them would bury the one
    /// thing this row is for.
    pub notices: Vec<LayoutNotice>,
}

/// What [`apply_bridge_profile`] has already put on screen.
///
/// Its presence is still the "boot has happened" latch it was in issue #1123 —
/// [`apply_bridge_profile`] returns immediately once it exists, and
/// `panes::ultralight::init_pane_host` waits for it. What it now also carries is
/// the **viewscreen's applied monitor**, which is what turns the viewscreen role
/// from a once-only application into a change-driven one: see
/// [`follow_layout_viewscreen`] and the [module note](self#boot-is-once-the-viewscreen-is-apply-on-change-issue-1330).
#[derive(Resource, Debug, Default, Clone)]
pub struct BridgeDisplayApplied {
    /// The monitor the viewscreen is on, as far as this adapter is concerned.
    ///
    /// Set at boot to the identity the layout was **seeded** with, whether or
    /// not a window was moved to reach it — that is precisely what makes a host
    /// with no `--profile` and no lobby press keep the window the OS gave it.
    /// `None` only in the moment before the first seed.
    pub viewscreen: Option<MonitorIdentity>,
}

/// Authored Station seats whose hull was not known at display boot. The
/// reservations, pane indices, splits and viewscreen have already been adopted;
/// only these seats wait for selection, and each is attempted exactly once.
#[derive(Resource, Default)]
struct DeferredProfileStations(Vec<(crate::core::messages::StationId, MonitorIdentity)>);

#[cfg(test)]
#[path = "bridge_display_roster_tests.rs"]
mod roster_tests;

/// Installs the bridge-display adapter.
///
/// [`apply_bridge_profile`] is no longer gated on a [`BridgeDisplayConfig`]
/// existing (issue #1330): it is the system that *synthesises* one for a host
/// launched without `--profile`, so gating it on the thing it creates would
/// leave that host with no monitor watcher and no monitor row.
///
/// It is gated on the two conditions that make it genuinely free instead, and
/// they are run conditions rather than early `return`s on purpose. It is an
/// **exclusive** system, and `World::query` builds a fresh `QueryState` every
/// call — so a host it can never do anything for (every
/// `NativeRenderSurface::Contract` and headless composition, which has no
/// [`Monitor`] entities and so never inserts [`BridgeDisplayApplied`] to latch
/// itself off) would otherwise build one every frame for the life of the
/// process. `any_with_component` caches its state; the early `return`s inside
/// stay as the belt to this braces.
pub struct BridgeDisplayPlugin;

/// Everything [`BridgeDisplayPlugin`] runs, as one ordering handle.
///
/// The chain inside is already total, so this exists for what is *outside* it:
/// another plugin that reads or edits [`BridgeLayoutResource`] in `Update` needs
/// to say where it stands relative to the whole adapter, and naming a private
/// member of the chain is not something it can do.
///
/// [`super::layout_store_systems`] is the caller (issue #1334) and shows what
/// the alternative costs: it takes `ResMut<BridgeLayoutResource>`, so Bevy would
/// serialise it against [`follow_layout_stations`] and [`watch_runtime_displays`]
/// in an order that is *arbitrary but silent* — the consoles a remembered layout
/// seats would open on the seed frame or the one after it depending on how the
/// executor felt, which is the kind of difference that shows up once on somebody
/// else's machine. Ordering after this set makes it always the frame after, on
/// purpose.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BridgeDisplaySet;

impl Plugin for BridgeDisplayPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.init_resource::<PendingConsoleClaims>();
        app.init_resource::<super::console_assignment::ConsoleAssignments>();
        app.declare_state::<super::console_assignment::ConsoleAssignments>(
            StateClass::Timer,
            "native-assigned-station-reservation",
        );
        // Native auto-claim: seat each console's station once its session
        // registers. Gated on the lobby inbound stream so a host built without
        // it (a bare display test) stands the plugin up without the writer
        // panicking on a missing `Messages` resource.
        app.add_systems(
            Update,
            (
                console_assignment::apply_pending_console_claims,
                super::console_assignment::sync_console_assignments,
            )
                .chain()
                // A removed seat cancels its pending claim before a late
                // registration can turn that old intent into a command.
                .after(BridgeDisplaySet)
                .run_if(
                    resource_exists::<bevy::ecs::message::Messages<crate::lobby::InboundMessage>>,
                ),
        );
        app.add_systems(
            Update,
            (
                apply_bridge_profile
                    // Boot is once, and the latch is a resource: once it exists
                    // there is nothing left to do, so do not even enter.
                    .run_if(not(resource_exists::<BridgeDisplayApplied>))
                    // And nothing to do at all until winit reports a display.
                    .run_if(any_with_component::<Monitor>),
                sync_station_roster.run_if(resource_exists::<BridgeLayoutResource>),
                // Both of these need the boot seed to exist: the follower diffs
                // against what boot recorded, and the watcher's first
                // observation is the baseline it diffs against.
                follow_layout_viewscreen.run_if(resource_exists::<BridgeDisplayApplied>),
                watch_runtime_displays
                    .run_if(resource_exists::<BridgeDisplayConfig>)
                    .run_if(resource_exists::<BridgeDisplayApplied>),
                // LAST, and chained after the watcher on purpose (issue #1331):
                // an unplug reconciles a station's console away inside
                // `watch_runtime_displays`, and this is what closes it. Running
                // it first would leave that console open for a frame on a
                // display that is gone.
                follow_layout_stations.run_if(resource_exists::<BridgeDisplayApplied>),
                super::console_assignment::remember_console_assignments,
                // And LAST of all: the applier above says what should be on
                // screen, this checks that it is. Ordered after it so a console
                // opened this frame is never judged before it exists — the
                // grace window makes that safe either way, but the order makes
                // it true rather than merely tolerated.
                reconcile_seated_consoles.run_if(resource_exists::<BridgeDisplayApplied>),
            )
                .chain()
                .in_set(BridgeDisplaySet),
        );
    }
}

/// Lift one Bevy [`Monitor`](bevy::window::Monitor) into a [`RawMonitor`].
pub(crate) fn raw_from_monitor(monitor: &Monitor, primary: bool) -> RawMonitor {
    RawMonitor {
        name: monitor.name.clone(),
        physical_width: monitor.physical_width,
        physical_height: monitor.physical_height,
        position_x: monitor.physical_position.x,
        position_y: monitor.physical_position.y,
        scale_factor: monitor.scale_factor,
        primary,
    }
}

/// The winit geometry to spawn a Station window with — a [`MonitorGeometry`] the
/// pane rects are computed against.
pub(crate) fn geometry_of(monitor: &Monitor) -> MonitorGeometry {
    MonitorGeometry {
        physical_width: monitor.physical_width,
        physical_height: monitor.physical_height,
        position_x: monitor.physical_position.x,
        position_y: monitor.physical_position.y,
        scale_factor: monitor.scale_factor,
    }
}

/// The present monitors, in a deterministic order, with the identity each one
/// answers to.
///
/// Sorted by virtual-desktop position before identities are assigned, so that
/// two identical monitors get their `#x,y` suffixes the same way whichever order
/// the ECS happened to iterate them in. Every reader of a live monitor set goes
/// through here so that no two of them can disagree about which display is
/// which.
///
/// `known` is what the caller already believed the bridge was — the identities
/// it has published to the row and judged presses against. A display still
/// plugged in keeps the identity it was known by rather than being re-derived
/// out from under itself; see [`identify_stable`]. Pass an empty slice at boot,
/// when nothing is known yet, which makes this exactly [`identify`].
pub(crate) fn identify_present(
    mut monitors: Vec<(Entity, RawMonitor, MonitorGeometry)>,
    known: &[DiscoveredMonitor],
) -> Vec<(Entity, DiscoveredMonitor, MonitorGeometry)> {
    monitors.sort_by_key(|(_, r, _)| (r.position_x, r.position_y));
    let raws: Vec<RawMonitor> = monitors.iter().map(|(_, r, _)| r.clone()).collect();
    let discovered = identify_stable(&raws, known);
    monitors
        .into_iter()
        .zip(discovered)
        .map(|((entity, _, geometry), found)| (entity, found, geometry))
        .collect()
}

/// The claimable stations of the hull this host is flying, as the layout law
/// keys them.
///
/// Empty when no hull has been resolved — a delivery-only host, or a fixture.
/// That is a lawful bridge with an empty roster: the viewscreen still moves,
/// and there is simply nothing to seat.
fn station_roster(world: &World) -> Vec<crate::core::messages::StationId> {
    world
        .get_resource::<crate::ship::components::PendingShipConfig>()
        .map(|c| c.0.stations.iter().map(|s| s.id.clone()).collect())
        // GameStart consumes PendingShipConfig. A display first reported after
        // that edge still needs the selected hull's roster, never the empty
        // lobby's fallback. SelectedShipResource distinguishes the two.
        .or_else(|| {
            world
                .get_resource::<crate::lobby::SelectedShipResource>()
                .and_then(|_| world.get_resource::<crate::lobby::stations_config::ShipStations>())
                .map(|c| c.stations.iter().map(|s| s.id.clone()).collect())
        })
        .unwrap_or_default()
}

/// Keep the display law on the selected hull independently of saved layouts.
/// In particular an authored profile deliberately has no layout store. Reusing
/// the live monitor snapshot retains the watcher's debounce and stable identity.
fn sync_station_roster(
    mut layout: ResMut<BridgeLayoutResource>,
    hull: Option<Res<crate::ship::components::PendingShipConfig>>,
    selected: Option<Res<crate::lobby::SelectedShipResource>>,
    stations: Option<Res<crate::lobby::stations_config::ShipStations>>,
    mut deferred: Option<ResMut<DeferredProfileStations>>,
) {
    if let Some(deferred) = deferred.as_mut() {
        let missing: Vec<_> = deferred
            .0
            .iter()
            .filter(|(_, monitor)| !layout.layout.monitors().contains(monitor))
            .cloned()
            .collect();
        if !missing.is_empty() {
            deferred
                .0
                .retain(|(_, monitor)| layout.layout.monitors().contains(monitor));
            // A settled unplug abandons the boot instruction. Replugging is
            // an explicit operator repair, and its new layout no longer carries
            // the original participant reservations or split.
            layout
                .notices
                .extend(missing.into_iter().map(|(station, monitor)| {
                    LayoutNotice::Adopted(LayoutAdoption::SeatRefused {
                        station,
                        refusal: super::bridge_layout::LayoutRefusal::UnknownMonitor {
                            monitor: monitor.clone(),
                        },
                        monitor,
                    })
                }));
        }
    }
    let owes_seats = deferred.as_ref().is_some_and(|d| !d.0.is_empty());
    if !layout.is_added()
        && !hull.as_ref().is_some_and(|h| h.is_changed())
        && !selected.as_ref().is_some_and(|s| s.is_changed())
        && !stations.as_ref().is_some_and(|s| s.is_changed())
        && !owes_seats
    {
        return;
    }
    let roster: Vec<_> = if let Some(hull) = hull {
        hull.0.stations.iter().map(|s| s.id.clone()).collect()
    } else if let (Some(_), Some(stations)) = (selected, stations) {
        stations.stations.iter().map(|s| s.id.clone()).collect()
    } else {
        return;
    };
    if layout.layout.roster() == roster && !owes_seats {
        return;
    }

    let (mut next, mut notes) = layout.layout.reconcile(&layout.monitors, roster);
    if let Some(deferred) = deferred.as_mut() {
        let mut seen = Vec::new();
        for (station, monitor) in std::mem::take(&mut deferred.0) {
            if seen.contains(&station) {
                notes.push(LayoutAdoption::StationNamedTwice { station, monitor });
                continue;
            }
            seen.push(station.clone());
            // An existing seat is a live choice. Never move it back to a boot
            // instruction; unassigned seats are attempted in authored order.
            if next.monitor_of(&station).is_some() {
                continue;
            }
            match next.apply(&LayoutAction::AssignStation {
                station: station.clone(),
                monitor: monitor.clone(),
            }) {
                Ok(placed) => next = placed,
                Err(refusal) => notes.push(LayoutAdoption::SeatRefused {
                    station,
                    monitor,
                    refusal,
                }),
            }
        }
    }
    layout.layout = next;
    layout
        .notices
        .extend(notes.into_iter().map(LayoutNotice::Adopted));
}

/// Read the present monitors, seed the live layout, and open one
/// borderless-fullscreen surface per **configured** monitor.
///
/// Runs once — see the [module note](self#why-an-exclusive-system-and-why-it-retries)
/// — and what it does depends on whether an operator authored a profile:
///
/// * **With `--profile`** it is issue #1123: resolve the profile against the
///   present displays, put the viewscreen on the primary window in borderless
///   fullscreen, and spawn one Station window per monitor the profile put a
///   console on. Since issue #1332's fix round those windows' panes are laid out
///   by [`BridgeLayout::surface_rects`](super::bridge_layout::BridgeLayout::surface_rects)
///   over the adopted layout rather than by a second tiling of the file — see
///   the note where the spawns are built.
/// * **Without one** it synthesises a [`BridgeDisplayConfig`] from the displays
///   it found so the runtime watcher has something to watch, seeds the layout
///   from the same discovery, and **touches no window** — see the
///   [module note](self#boot-is-once-the-viewscreen-is-apply-on-change-issue-1330).
pub fn apply_bridge_profile(world: &mut World) {
    if world.get_resource::<BridgeDisplayApplied>().is_some() {
        return;
    }
    let log = world.get_resource::<LogFilterConfig>().cloned();
    // The `novsync` frame experiment (`panes::frame_stats`) opens Station
    // windows without a vblank wait; every other run takes the default.
    let station_present_mode = world.get_resource::<PaneExperiments>().map_or(
        PresentMode::default(),
        PaneExperiments::station_present_mode,
    );

    // The monitors as winit reports them, paired with their entities. Cloned out
    // so no query borrow is held while we mutate windows and spawn below.
    let monitors = identify_present(
        world
            .query::<(Entity, &Monitor, Has<PrimaryMonitor>)>()
            .iter(world)
            .map(|(e, m, primary)| (e, raw_from_monitor(m, primary), geometry_of(m)))
            .collect(),
        // Boot: nothing is known yet, so there is nothing to carry forward.
        &[],
    );
    if monitors.is_empty() {
        // winit has not populated the monitor list yet — try again next frame.
        return;
    }
    let discovered: Vec<DiscoveredMonitor> = monitors.iter().map(|(_, d, _)| d.clone()).collect();

    // The live layout's seed. `from_discovered` puts the viewscreen on the
    // primary monitor — where the OS opened this process's window, and so where
    // the lobby is already being drawn — and an authored profile is then adopted
    // on top of it, which is what "an explicit --profile still wins at boot"
    // means: it seeds the arrangement the lobby then edits.
    let base = BridgeLayout::from_discovered(&discovered, station_roster(world))
        .expect("a non-empty discovery always yields a bridge");
    let authored = world.get_resource::<BridgeDisplayConfig>().cloned();
    if let Some(config) = authored.as_ref().filter(|c| c.authored) {
        if !world.contains_resource::<crate::ship::components::PendingShipConfig>()
            && !world.contains_resource::<crate::lobby::SelectedShipResource>()
        {
            world.insert_resource(DeferredProfileStations(
                config
                    .profile
                    .displays
                    .iter()
                    // Only the hull is deferred. An absent monitor was not
                    // adopted at boot and is never silently adopted on replug.
                    .filter(|display| base.monitors().contains(&display.identity))
                    .flat_map(|display| match &display.role {
                        DisplayRole::Station { panes, .. } => panes
                            .iter()
                            .filter_map(|p| p.station.as_ref())
                            .map(|s| {
                                (
                                    crate::core::messages::StationId(s.clone()),
                                    display.identity.clone(),
                                )
                            })
                            .collect::<Vec<_>>(),
                        _ => Vec::new(),
                    })
                    .collect(),
            ));
        }
    }
    let (layout, adoption) = match &authored {
        Some(config) => base.adopt_profile(&config.profile),
        None => (base, Vec::new()),
    };
    // Boot notes are LOGGED, never pushed to the lobby: a hand-authored
    // `--pane` profile produces one per participant slot, and opening the
    // monitor row with a wall of them would bury the answers to actual presses.
    for note in &adoption {
        if world.contains_resource::<DeferredProfileStations>()
            && matches!(
                note,
                LayoutAdoption::SeatRefused {
                    refusal: super::bridge_layout::LayoutRefusal::UnknownStation { .. },
                    ..
                }
            )
        {
            // This is not an invalid hull reference yet. Selection will try
            // the deferred seat once and publish any actual refusal then.
            continue;
        }
        crate::pwarn!(log, LogCat::Lobby, "bridge display: {note}");
    }

    let config = match authored {
        Some(config) => config,
        None => {
            // The runtime display config (issue #1330). A *description* of the
            // displays as found, not an instruction: `authored` is false, so
            // nothing below places a window from it.
            //
            // IT IS THE BOOT LAYOUT AND IT STAYS THAT WAY (settled in
            // issue #1331). It carries no pane labels, because `layout` has
            // seated nothing at boot, so `watch_runtime_displays` resolves every
            // unplug on this host to a loss with an empty `pane_labels` and
            // closes no pane. #1330 left that holding by accident and flagged a
            // rebuild-from-the-live-layout as the obvious next move. It is not
            // the move, and here is why it was not taken:
            //
            //   * A console the lobby opened is closed on an unplug ALREADY,
            //     through the law rather than through this config.
            //     `BridgeLayout::reconcile` leaves a station whose monitor is
            //     gone unassigned, and `follow_layout_stations` closes exactly
            //     what the layout no longer seats — the same debounce window,
            //     the same `PaneBus::close`, the same flip to `Backfill`. A
            //     rebuilt config would close it a second time.
            //   * The two lists mean different things. `assigned_surfaces`'s
            //     `pane_labels` are the AUTHORED profile's participant names —
            //     issue #1125's question, "was a monitor this host was
            //     *configured* for lost, and whose panes were on it". Folding
            //     lobby-opened station ids into them would make one list answer
            //     two questions, and the answer to neither would be checkable.
            //
            // So the watcher keeps the boot arrangement and the layout keeps the
            // live one. The pair of tests that pins this is
            // `unplugging_a_monitor_with_no_console_on_it_closes_no_pane` and
            // `unplugging_a_monitor_holding_a_runtime_console_closes_exactly_it`
            // — read them before changing this.
            //
            // WHAT THAT DOES NOT MEAN, and what a first reading of it got wrong:
            // that the two lists cannot *overlap*. They can, and a `--profile`
            // is how — `PaneSlot::for_station` names its pane for its station,
            // so an AUTHORED station slot put a station id straight into
            // `pane_labels` and the watcher then resolved it against the LIVE
            // bus, closing a console that had since been moved to another
            // screen. The lists are kept apart at the SOURCE instead:
            // `assigned_surfaces` excludes a station-bearing slot outright
            // (issue #1331), so `pane_labels` really is participants only, and a
            // station's console is the law's on every host — authored or not.
            let config = BridgeDisplayConfig {
                profile: layout.to_validated_profile(),
                authored: false,
            };
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge display: no --profile, so the layout is the {} monitor(s) as found, with \
                 the viewscreen on {} (the window is left exactly where it opened)",
                discovered.len(),
                layout.viewscreen()
            );
            world.insert_resource(config.clone());
            config
        }
    };

    let resolved = resolve(&config.profile, &discovered);

    // identity → (monitor entity, geometry), for turning a resolved surface back
    // into the winit monitor it names.
    let by_identity: std::collections::HashMap<String, (Entity, MonitorGeometry)> = monitors
        .iter()
        .map(|(e, d, g)| (d.identity.as_str().to_string(), (*e, g.clone())))
        .collect();

    // Only an AUTHORED profile has problems worth reporting. A synthesised one
    // names the viewscreen and whatever monitors hold consoles — which, on a
    // host nobody has arranged yet, is one monitor out of however many are
    // plugged in. Every other display is then a `MonitorUnassigned`, so a
    // three-screen host would WARN twice at every boot about a state that is
    // simply "the operator has not put anything there yet". `resolve` is still
    // run: it is what turns the profile into the surfaces below.
    if config.authored {
        for problem in &resolved.problems {
            crate::pwarn!(log, LogCat::Lobby, "bridge display: {problem}");
        }
    }

    // The viewscreen goes on the primary window; the Stations get their own.
    let mut station_surfaces: Vec<BridgeStationSurface> = Vec::new();
    let mut viewscreen_monitor: Option<(Entity, String, String)> = None;
    struct StationSpawn {
        monitor: Entity,
        identity: String,
        role: String,
        geometry: MonitorGeometry,
        panes: Vec<StationPane>,
    }
    let mut station_spawns: Vec<StationSpawn> = Vec::new();

    // The viewscreen is the profile's to place — it is what puts this process's
    // primary window on a screen, and the layout has already adopted the same
    // entry.
    for surface in &resolved.surfaces {
        let Some((monitor_entity, _)) = by_identity.get(surface.identity.as_str()) else {
            continue;
        };
        if surface.role == DisplayRole::Viewscreen {
            viewscreen_monitor = Some((
                *monitor_entity,
                surface.identity.as_str().to_string(),
                surface.role.summary(),
            ));
        }
    }

    // ── the Station windows come from the LAYOUT, not from the file ──────────
    //
    // ONE TILING, FROM THE BOOT FRAME ONWARD (issue #1332's fix round). This
    // used to lay a Station out from the profile directly — `pane_rects` over
    // that entry's own pane order and its own split — while
    // `follow_layout_stations` laid the same screen out from
    // `BridgeLayout::surface_rects`. Two answers to one question, and the second
    // overwrote the first on the very next system in the chain:
    //
    //   * an authored `[helm(station), Ada(participant)]` booted helm-left and
    //     was re-tiled Ada-left one frame later, which closed and recreated a
    //     console nobody had touched and put a re-tiling notice on the row that
    //     nobody had earned — routing straight around the deliberate rule a few
    //     lines up that boot notes are logged and never pushed;
    //   * an authored `split = "stacked"` screen was re-tiled side by side,
    //     because the follower's tiling knew only the constant.
    //
    // Both are gone by construction rather than by agreement: the law now
    // carries the authored split (`BridgeLayout::split_on`) and each authored
    // pane's authored index (`BridgeLayout::occupants_at`), and this is a READER
    // of the same `surface_rects` the follower reads. There is one tiling, so
    // there is nothing for a second one to disagree with.
    for (entity, found, geometry) in &monitors {
        let identity = &found.identity;
        let panes: Vec<StationPane> = layout
            .surface_rects(identity, geometry)
            .into_iter()
            .map(|(occupant, rect)| StationPane {
                label: occupant.name().to_string(),
                rect,
                // An authored profile may seat a STATION as well as a
                // participant (`PaneSlot::for_station`). Carrying which it is
                // here is what lets `follow_layout_stations` treat the two
                // differently: it owns the station consoles and never touches
                // the participants'.
                station: occupant.station().cloned(),
            })
            .collect();
        if panes.is_empty() {
            continue;
        }
        // The diagnostic summary the `BridgeSurface` tag carries, rebuilt from
        // the arrangement actually being drawn rather than from the file — a
        // pane the law refused (a station off this ship's roster, say) is
        // reported as a `LayoutAdoption` above and must not then be named on a
        // window as though it were open.
        let role = DisplayRole::Station {
            split: layout.split_on(identity),
            panes: panes
                .iter()
                .map(|pane| match &pane.station {
                    Some(station) => PaneSlot::for_station(station.0.clone()),
                    None => PaneSlot::for_participant(pane.label.clone()),
                })
                .collect(),
        }
        .summary();
        station_spawns.push(StationSpawn {
            monitor: *entity,
            identity: identity.as_str().to_string(),
            role,
            geometry: geometry.clone(),
            panes,
        });
    }

    // Place the viewscreen on the primary window — but only for a profile an
    // operator actually wrote. A synthesised one describes where the window
    // already is, and acting on it would put a host launched with no display
    // arguments into borderless fullscreen it never asked for.
    if config.authored {
        if let Some((monitor_entity, identity, role)) = viewscreen_monitor {
            if let Some(primary) = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .iter(world)
                .next()
            {
                if let Some(mut window) = world.entity_mut(primary).get_mut::<Window>() {
                    window.mode =
                        WindowMode::BorderlessFullscreen(MonitorSelection::Entity(monitor_entity));
                }
                world.entity_mut(primary).insert(BridgeSurface {
                    identity: identity.clone(),
                    role,
                });
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "bridge display: viewscreen on monitor {identity} (primary window, borderless \
                     fullscreen)"
                );
            }
        } else {
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "bridge display: the profile assigns no present monitor the viewscreen role, so \
                 the shared 3-D view has nowhere to draw. Check the profile against `--setup`."
            );
        }
    }

    // Open one borderless-fullscreen window per Station.
    for spawn in station_spawns {
        let window = world
            .spawn((
                Window {
                    title: format!("{} — Station", super::WINDOW_TITLE),
                    name: Some(format!("phoenix-station-{}", spawn.identity)),
                    mode: WindowMode::BorderlessFullscreen(MonitorSelection::Entity(spawn.monitor)),
                    present_mode: station_present_mode,
                    ..default()
                },
                BridgeSurface {
                    identity: spawn.identity.clone(),
                    role: spawn.role.clone(),
                },
            ))
            .id();
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "bridge display: station window on monitor {} for {} pane(s) (borderless fullscreen)",
            spawn.identity,
            spawn.panes.len()
        );
        station_surfaces.push(BridgeStationSurface {
            identity: spawn.identity,
            window,
            monitor: spawn.monitor,
            geometry: spawn.geometry,
            panes: spawn.panes,
        });
    }

    world.insert_resource(BridgeStationSurfaces(station_surfaces));
    // The baseline the follower diffs against is the layout's viewscreen — the
    // identity this host is *seeded* on — whether or not a window was moved to
    // reach it. That is the whole of the "a run with no lobby action produces
    // today's window" promise: with nothing to differ from, the follower never
    // fires, and the window stays wherever it opened.
    world.insert_resource(BridgeDisplayApplied {
        viewscreen: Some(layout.viewscreen().clone()),
    });
    world.insert_resource(BridgeLayoutResource {
        layout,
        monitors: discovered,
        notices: Vec::new(),
    });
}

/// Keep the viewscreen on the monitor the **live layout** names (issue #1330).
///
/// The change-driven half of the applier: the lobby's monitor row and
/// [`watch_runtime_displays`]'s reconcile both move
/// [`BridgeLayoutResource`]'s viewscreen, and this is what makes the window
/// follow — `BorderlessFullscreen` on the chosen display, with the primary
/// window re-tagged so the same [`BridgeSurface`] that told the #1123
/// integration test which surface is which keeps telling the truth.
///
/// **It does nothing at all unless the layout's viewscreen differs from the one
/// [`BridgeDisplayApplied`] recorded.** Every frame of every host that nobody
/// has touched takes the first `return` below.
///
/// A missing target normally waits for the monitor watcher's fallback. If a GM
/// role exists, hide the old window while it waits so winit cannot park the
/// viewscreen over the dedicated GM. A returning lawful target unhides and
/// reanchors it, including a replacement Monitor entity at the same geometry.
#[allow(clippy::too_many_arguments)]
fn follow_layout_viewscreen(
    monitors: Query<(Entity, &Monitor, Has<PrimaryMonitor>)>,
    layout: Option<Res<BridgeLayoutResource>>,
    mut applied: ResMut<BridgeDisplayApplied>,
    mut primary: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    mut hidden_for_gm: Local<bool>,
    mut anchored_to: Local<Option<Entity>>,
) {
    let Some(layout) = layout else {
        return;
    };
    let wanted = layout.layout.viewscreen().clone();
    // Resolved against the identities the LAYOUT knows, not against freshly
    // derived ones: a display whose identity was carried across a roster change
    // answers to the key the row published and the press named, and re-deriving
    // here would leave a lawful press finding no monitor and silently doing
    // nothing. See `identify_stable`.
    let present = identify_present(
        monitors
            .iter()
            .map(|(e, m, is_primary)| (e, raw_from_monitor(m, is_primary), geometry_of(m)))
            .collect(),
        &layout.monitors,
    );
    let Some((monitor_entity, _, _)) = present.iter().find(|(_, d, _)| d.identity == wanted) else {
        // winit can park a disconnected fullscreen window on another monitor.
        // Hide it while its only lawful alternative is the dedicated GM screen.
        // The law keeps the absent identity, so a later non-GM monitor restores
        // the viewscreen without enabling/disabling either role.
        if layout.layout.game_master_monitor().is_some() {
            if let Ok((_, mut window)) = primary.single_mut() {
                window.visible = false;
                *hidden_for_gm = true;
            }
        }
        return;
    };
    if applied.viewscreen.as_ref() == Some(&wanted)
        && !*hidden_for_gm
        && anchored_to.is_none_or(|entity| entity == *monitor_entity)
    {
        *anchored_to = Some(*monitor_entity);
        return;
    }
    let Ok((window_entity, mut window)) = primary.single_mut() else {
        return;
    };
    window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Entity(*monitor_entity));
    if *hidden_for_gm {
        window.visible = true;
        *hidden_for_gm = false;
    }
    *anchored_to = Some(*monitor_entity);
    commands.entity(window_entity).insert(BridgeSurface {
        identity: wanted.as_str().to_string(),
        role: DisplayRole::Viewscreen.summary(),
    });
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "bridge display: viewscreen moved to monitor {wanted} (primary window, borderless \
         fullscreen)"
    );
    applied.viewscreen = Some(wanted);
}

// ── consoles opened and closed while the host runs (issue #1331) ────────────

/// Spawn one borderless-fullscreen Station window on `monitor`, tagged so the
/// #1123 integration test and any diagnostics can tell which surface is which.
///
/// The same window [`apply_bridge_profile`] opens at boot, built through
/// `Commands` rather than `&mut World` because [`follow_layout_stations`] is an
/// ordinary system. One constructor, so a console opened at boot and one opened
/// from the lobby are the same kind of window.
fn spawn_station_window(
    commands: &mut Commands,
    monitor: Entity,
    identity: &str,
    present_mode: PresentMode,
) -> Entity {
    commands
        .spawn((
            Window {
                title: format!("{} — Station", super::WINDOW_TITLE),
                name: Some(format!("phoenix-station-{identity}")),
                mode: WindowMode::BorderlessFullscreen(MonitorSelection::Entity(monitor)),
                present_mode,
                ..default()
            },
            BridgeSurface {
                identity: identity.to_string(),
                role: DisplayRole::Station {
                    split: super::bridge_layout::LAYOUT_SPLIT,
                    panes: Vec::new(),
                }
                .summary(),
            },
        ))
        .id()
}

/// Why one console's view has to be built again — and therefore what
/// [`follow_layout_stations`] says about it (issue #1332's fix round).
///
/// All three do the identical work: an Ultralight view is created at one size on
/// one window, so a console whose rectangle is no longer the one its view was
/// built for goes through `close` + `recreate` on the same session token. The
/// distinction is the **sentence**, and the sentence has to be true — an
/// operator reading "the split changed" about a display that renegotiated its
/// mode learns something that did not happen, and goes looking for the console
/// that joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rebuild {
    /// The operator moved this console to another screen. Not announced: they
    /// watched themselves do it, and narrating it back would bury the
    /// neighbour's line, which is the one that is news.
    Moved,
    /// A neighbour arrived on this console's screen or left it, so the screen is
    /// divided differently and this console — which nobody asked to move — has a
    /// different share of it.
    Retiled,
    /// The screen itself changed size while holding exactly the consoles it
    /// already held, so every rectangle on it moved with nothing having joined
    /// or left.
    Resized,
}

impl Rebuild {
    /// The note the lobby's row is owed for this rebuild, or `None` for the one
    /// the operator asked for.
    fn note(
        self,
        console: &str,
        monitor: &MonitorIdentity,
    ) -> Option<super::bridge_layout::LayoutAdoption> {
        use super::bridge_layout::LayoutAdoption;
        match self {
            Rebuild::Moved => None,
            Rebuild::Retiled => Some(LayoutAdoption::ConsoleRetiling {
                console: console.to_string(),
                monitor: monitor.clone(),
            }),
            Rebuild::Resized => Some(LayoutAdoption::ConsoleResized {
                console: console.to_string(),
                monitor: monitor.clone(),
            }),
        }
    }
}

/// Open and close station consoles as the **live layout** seats and unseats
/// them (issue #1331).
///
/// The station half of the apply-on-change applier, and the exact counterpart of
/// [`follow_layout_viewscreen`]: the lobby's screen rows and
/// [`watch_runtime_displays`]'s reconcile both move
/// [`BridgeLayoutResource`]'s seating, and this is what makes the *screens*
/// follow — a Station window on the chosen monitor, a pane seated on it, and
/// the console going through the ordinary client join flow from there.
///
/// # A console is opened here, never by the lobby
///
/// `host_lobby::drain_surface_records`'s layout arm only moves the layout. That is
/// deliberate: an unplug reconciles a station's console away with nobody
/// pressing anything, and if the press path opened consoles the unplug path
/// would have to learn to close them separately — two implementations of one
/// rule, disagreeing the first time either was touched. Both changes reach the
/// layout, and the layout is the only thing this reads.
///
/// It is also what makes the **display-loss** semantics fall out rather than be
/// re-stated: [`BridgeLayout::reconcile`] leaves a station whose monitor is gone
/// unassigned, so its console is closed here on the same frame — the pane's
/// token disconnects and its station flips to `Backfill` through the ordinary
/// dropped-participant path, exactly as issue #1125 does it for a `--pane`.
/// [`watch_runtime_displays`]'s own pane closing is untouched and stays what it
/// always was: the **authored** profile's participant panes. See the invariant
/// note in [`apply_bridge_profile`].
///
/// # What a console is
///
/// A pane on the bus, minted with a fresh ordinary session token and named for
/// its station (`PaneBus::open_console`), loading the same client document a
/// `--pane` loads and a phone loads. The station id in the layout decides only
/// **which console document opens on which glass** — never who may sit there:
/// the page claims its station through the ordinary lobby flow, and may be
/// released and re-claimed by anyone.
///
/// # Every console on a monitor is laid out together (issue #1332)
///
/// A monitor's rectangles come from ONE
/// [`BridgeLayout::surface_rects`](super::bridge_layout::BridgeLayout::surface_rects)
/// call over its whole occupancy — the authored surfaces a `--profile` opened
/// *and* the stations the layout seats — rather than a station tiling laid
/// beside whatever was already there. Two tilings is precisely what produced
/// issue #1331's carried defect: an authored `--pane` was spawned across its
/// whole monitor at boot, the law let a station be seated on that same screen
/// because it counted only seats, and this pass then handed the newcomer the
/// full rectangle too — two consoles **overlapping** instead of tiling.
///
/// The fix has two halves and they only work together. The law counts a reserved
/// surface against the two-per-screen cap, so a mixed monitor holds at most two;
/// and this pass rewrites **all** of a monitor's panes, so the authored one
/// yields half its screen instead of being tiled around. An authored pane is
/// still not the layout's to open, move or close — only to *place*.
///
/// # Moving one is rebuilding it, on the same identity
///
/// An Ultralight view is created at one size on one window, so a console that
/// moved screens — or whose rectangle changed because a second console joined
/// its screen or left it — cannot be re-placed and has to be built again. That
/// goes through `PaneBus::close` + `recreate`, which is issue #1125's crash path
/// used deliberately rather than a second mechanism: the **same session token**
/// survives, so the rebuilt page's `Identify` is a reconnect the lobby answers
/// by restoring the held station. Whoever claimed that console keeps it across
/// the move, with the same moment on `Backfill` a view crash costs.
///
/// # …and a console nobody moved says so out loud
///
/// That rebuild is the price of the operator's own press for the console they
/// pressed for. For its **neighbour** — a person who pressed nothing, whose
/// screen blinks and whose seat is claimable by somebody else for one page load
/// — it is a surprise, and the PRD's "reassignment must not steal seats
/// gratuitously" is why it is announced rather than absorbed. The three are told
/// apart by the evidence this pass already has ([`Rebuild`]): a console whose
/// *monitor* changed was moved by the press the operator just made; one whose
/// rectangle changed on a screen whose **occupancy** also changed was re-tiled
/// by a neighbour arriving or leaving
/// ([`ConsoleRetiling`](super::bridge_layout::LayoutAdoption::ConsoleRetiling));
/// and one whose rectangle changed on a screen holding exactly what it held
/// before was resized with its monitor
/// ([`ConsoleResized`](super::bridge_layout::LayoutAdoption::ConsoleResized)).
/// The last two both reach the lobby's row — the console really did reload
/// either way — with the cause that actually happened.
// Nine parameters, and every one is a distinct thing this pass reads, writes or
// remembers — the same reason `watch_runtime_displays` carries nine.
#[allow(clippy::too_many_arguments)]
fn follow_layout_stations(
    monitors: Query<(Entity, &Monitor, Has<PrimaryMonitor>)>,
    // `ResMut` only for the re-tiling notice at the end (issue #1332). Nothing
    // here edits the arrangement — that is the law's, and this pass follows it —
    // and the resource is deref'd immutably everywhere else, so a frame with
    // nothing to report does not mark it changed and does not schedule a push.
    layout: Option<ResMut<BridgeLayoutResource>>,
    surfaces: Option<ResMut<BridgeStationSurfaces>>,
    bus: Option<Res<crate::native_host::panes::PaneBusResource>>,
    // The `novsync` frame experiment, for a Station window opened here — the
    // same present mode `apply_bridge_profile` gives one opened at boot.
    experiments: Option<Res<PaneExperiments>>,
    // The Station windows this pass has already opened, so one whose display
    // came back on a new `Monitor` entity can be re-anchored to it — see the
    // existing-surface arm below.
    mut windows: Query<&mut Window>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    // A console opened here owes its station a claim (native auto-claim); the
    // intent is recorded and applied once the console's session registers.
    mut auto_claims: Option<ResMut<PendingConsoleClaims>>,
    // Station windows that emptied on an earlier pass, despawned on this one.
    //
    // One frame of grace, and it is not tidiness: the pane host tears a closed
    // pane's view, its canvas and its Station camera down in
    // `retire_closed_panes` on its own schedule, and that system and this one
    // are unordered within `Update`. Despawning the window in the same pass
    // therefore leaves a live camera rendering to a window that is gone for as
    // long as it takes the pane host to notice. Draining a list next frame costs
    // an `is_empty` on every other frame of the run.
    mut pending_close: Local<Vec<Entity>>,
    // A pass that was owed and could not be run — see the deferral note below.
    // `Res::is_changed` is answered against the frame this system last *ran*,
    // so a change observed on a frame this pass declines to act on is gone by
    // the next one unless it is remembered here.
    mut pass_due: Local<bool>,
    // The monitor identities this pass last placed against.
    mut placed_against: Local<Vec<String>>,
    // The last roster this pass actually applied. A removed Station is absent
    // from the current roster but still owes its old console a close.
    mut applied_roster: Local<Vec<crate::core::messages::StationId>>,
) {
    for window in pending_close.drain(..) {
        commands.entity(window).try_despawn();
    }
    let (Some(mut layout), Some(mut surfaces)) = (layout, surfaces) else {
        return;
    };

    // Resolved against the identities the LAYOUT knows rather than freshly
    // derived ones, for `follow_layout_viewscreen`'s reason: a display whose
    // identity was carried across a twin arriving or a mode change answers to
    // the key the row published and the press named.
    let present = identify_present(
        monitors
            .iter()
            .map(|(e, m, is_primary)| (e, raw_from_monitor(m, is_primary), geometry_of(m)))
            .collect(),
        &layout.monitors,
    );
    let live: Vec<String> = present
        .iter()
        .map(|(_, d, _)| d.identity.as_str().to_string())
        .collect();

    // Apply-on-change, like the viewscreen follower — but on a change to
    // EITHER of this pass's two inputs, because it has two. The layout says
    // which station sits where; the monitors say what a seat is made of (which
    // window, at what geometry), and a display that left and came back has to
    // get its Station window rebuilt or the console the layout still seats has
    // nowhere to be composited. A host nobody has rearranged, whose displays
    // nobody has touched, still takes this return on every frame of its life.
    if layout.is_changed() || *placed_against != live {
        *pass_due = true;
    }
    if !*pass_due {
        return;
    }

    // A frame with NO monitors at all is winit between hot-plug events, not a
    // bridge that lost every display — the same judgement `watch_runtime_displays`
    // makes on the same observation. Acting on it would tear down every Station
    // window on the machine. The pass stays owed, so the change that made it due
    // is not lost with the frame.
    if present.is_empty() {
        return;
    }

    // A console is a pane, and without a bus there is nothing to open one on: a
    // host with no `--client-dir` bundle has no client document to load. It has
    // no lobby surface to press either, so this is unreachable in production —
    // but the layout is still lawful, and spawning a borderless-fullscreen
    // window showing nothing would be worse than saying so.
    let Some(bus) = bus else {
        *pass_due = false;
        *placed_against = live;
        let seated: Vec<&crate::core::messages::StationId> = layout
            .layout
            .monitors()
            .iter()
            .flat_map(|m| layout.layout.stations_on(m))
            .collect();
        if !seated.is_empty() {
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "bridge display: the layout seats {} console(s), but this host has no pane bus \
                 to open one on — it needs a --client-dir bundle",
                seated.len()
            );
        }
        return;
    };
    *pass_due = false;
    *placed_against = live;

    // Every console the surfaces are carrying, and **where** — the monitor and
    // the rectangle its view was built for. Both halves matter: a console that
    // changed either has a view sized and placed for a seat it no longer has,
    // and rebuilding it is what makes a move a move.
    //
    // Keyed by the pane's LABEL and covering authored panes as well as seated
    // ones (issue #1332), because a re-tile moves an authored `--pane`'s
    // rectangle exactly as it moves a station's, and its view is just as unable
    // to follow. The label is the one key the pane bus resolves either kind by.
    let carried: Vec<(String, String, PaneRect)> = surfaces
        .0
        .iter()
        .flat_map(|s| {
            s.panes
                .iter()
                .map(|p| (p.label.clone(), s.identity.clone(), p.rect))
        })
        .collect();

    // ── the layout's occupancy, monitor by monitor ──────────────────────────
    //
    // The one thing worked out here is which consoles have a view built for a
    // seat they no longer have, and which of the three reasons it is for
    // ([`Rebuild`]): the operator MOVED this one (its monitor changed), a
    // neighbour arriving or leaving RE-TILED it (its rectangle changed on a
    // screen whose occupancy changed), or its monitor was RESIZED under it (its
    // rectangle changed on a screen holding exactly what it held before).
    // Whether a console needs OPENING is not a diff at all — it is asked of the
    // bus, below, for every station the layout seats.
    let mut rebuilds: Vec<(String, MonitorIdentity, Rebuild)> = Vec::new();
    for (entity, discovered, geometry) in &present {
        let identity = &discovered.identity;
        // The deterministic split, resolved against this monitor's real
        // geometry: one console is the whole screen, two divide it side by side.
        // Recomputed for the WHOLE monitor and over its WHOLE occupancy — the
        // authored surfaces it carries as well as the stations the law seats —
        // because seating a second console re-lays out the first whichever kind
        // the first is, and a tiling that skipped the authored one would hand two
        // panes the same pixels.
        let occupancy = layout.layout.surface_rects(identity, geometry);
        let existing = surfaces
            .0
            .iter()
            .position(|s| s.identity == identity.as_str());
        if occupancy.is_empty() && existing.is_none() {
            continue;
        }
        let panes: Vec<StationPane> = occupancy
            .into_iter()
            .map(|(occupant, rect)| StationPane {
                label: occupant.name().to_string(),
                rect,
                station: occupant.station().cloned(),
            })
            .collect();
        // WHAT THIS SCREEN WAS HOLDING, before the rewrite below — the evidence
        // that tells a re-tile from a resize (issue #1332's fix round). A
        // rectangle that changed says a console has to be rebuilt; it does not
        // say why, and this pass used to assert "the split changed" for every
        // one of them. A monitor that renegotiated its mode in place keeps its
        // identity (`identify_stable`) and its consoles and reports new pixels,
        // so every pane on it moves with nothing having joined or left — and
        // "the split changed" is then a sentence the code cannot support. The
        // occupancy is the discriminator, and it is compared as the labels in
        // their drawn order rather than as a count, so a swap is a re-tile too.
        let held_before: Option<Vec<String>> = existing.map(|index| {
            surfaces.0[index]
                .panes
                .iter()
                .map(|p| p.label.clone())
                .collect()
        });
        let now_holding: Vec<String> = panes.iter().map(|p| p.label.clone()).collect();
        let occupancy_changed = held_before.as_ref() != Some(&now_holding);
        for pane in &panes {
            // A console this pass has never seen is simply opened below; one it
            // has seen elsewhere, or at another size, has a view built for that
            // seat and cannot be re-placed into this one.
            let Some((_, was_on, was_at)) =
                carried.iter().find(|(label, _, _)| label == &pane.label)
            else {
                continue;
            };
            let cause = if was_on != identity.as_str() {
                Rebuild::Moved
            } else if was_at != &pane.rect {
                if occupancy_changed {
                    Rebuild::Retiled
                } else {
                    Rebuild::Resized
                }
            } else {
                continue;
            };
            rebuilds.push((pane.label.clone(), identity.clone(), cause));
        }
        match existing {
            Some(index) => {
                let surface = &mut surfaces.0[index];
                surface.geometry = geometry.clone();
                // RE-ANCHOR a surface whose display went away and came back.
                //
                // `bevy_winit` despawns a `Monitor` entity when the display stops
                // being reported and spawns a new one when it returns, so a blip
                // inside the settle window — which the law-based retain above
                // exists to survive — leaves this window's
                // `BorderlessFullscreen(Entity(old))` naming an entity that no
                // longer exists. `bevy_winit` re-applies fullscreen only when
                // `Window::mode` CHANGES, so the window would sit wherever the OS
                // parked it while `geometry` above, the pane rects and the input
                // router all used the returned monitor's coordinates: the console
                // drawn on one screen and clicked on another. Rewriting the mode
                // is exactly what `follow_layout_viewscreen` does for the primary
                // window, on the same evidence.
                if surface.monitor != *entity {
                    surface.monitor = *entity;
                    if let Ok(mut window) = windows.get_mut(surface.window) {
                        window.mode =
                            WindowMode::BorderlessFullscreen(MonitorSelection::Entity(*entity));
                    }
                    crate::pinfo!(
                        log,
                        LogCat::Lobby,
                        "bridge display: monitor {identity} came back on a new handle, so its \
                         station window is re-anchored to it"
                    );
                }
                // REPLACED WHOLESALE, authored panes included (issue #1332).
                //
                // It used to retain the authored panes and rebuild only the
                // station ones around them, which is what let a station's console
                // be laid across an authored one. The law's occupancy is now the
                // whole truth about what is on a screen, so the surface is its
                // rendering and nothing more: `surface_rects` above already
                // carried the authored slots forward, at their share of the
                // screen, in the order they have held since boot.
                //
                // The two lists cannot drift apart. A Station surface exists only
                // for a monitor an authored `--profile` named, `adopt_profile`
                // reserved that profile's participant panes on the same monitors
                // at boot, and `reconcile` carries a surviving monitor's
                // reservations forward — a monitor that goes away loses its
                // surface, its reservation and (through the #1125 watcher) its
                // authored panes together.
                surface.panes = panes;
            }
            None => {
                let window = spawn_station_window(
                    &mut commands,
                    *entity,
                    identity.as_str(),
                    experiments.as_deref().map_or(
                        PresentMode::default(),
                        PaneExperiments::station_present_mode,
                    ),
                );
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "bridge display: station window opened on monitor {identity} for {} \
                     console(s) (borderless fullscreen)",
                    panes.len()
                );
                surfaces.0.push(BridgeStationSurface {
                    identity: identity.as_str().to_string(),
                    window,
                    monitor: *entity,
                    geometry: geometry.clone(),
                    panes,
                });
            }
        }
    }

    // A monitor the LAW no longer names — unplugged, and the reconcile believed
    // it — keeps no surface: its window went with the display.
    //
    // The judgement is the LAYOUT'S, and deliberately not this frame's winit
    // report. Every other reader of a lost display waits out
    // [`DISPLAY_LOSS_DEBOUNCE_FRAMES`] before believing it, because winit's
    // monitor list blips through a GPU reset, a display waking and a dock;
    // dropping a surface on a single absent frame would despawn a live console's
    // window under a crew member, leave a camera rendering to a window that is
    // gone, and leave the layout still seating a station the adapter has no
    // surface for. The reconcile already does that waiting, so following it —
    // rather than re-deciding beside it on different evidence — is what makes
    // the two agree by construction. A monitor absent for a blip keeps its
    // surface and its panes untouched; when the reconcile finally unseats its
    // consoles, this drops the surface and the sweep below closes them.
    let lawful: Vec<&str> = layout
        .layout
        .monitors()
        .iter()
        .map(|m| m.as_str())
        .collect();
    surfaces.0.retain(|surface| {
        if lawful.contains(&surface.identity.as_str()) {
            return true;
        }
        pending_close.push(surface.window);
        false
    });

    // ── close what the layout no longer seats ───────────────────────────────
    //
    // Asked of the BUS ∩ the LAW — every station on the current or last-applied
    // roster that the layout does not seat, whose console the bus still has open — rather
    // than of `carried`, which is this adapter's own bookkeeping and can have
    // been emptied by the retain above before the reconcile got round to
    // unseating what was on it. A console outliving its seat is the one failure
    // with no way back: the pane never closes, so its station never flips to
    // `Backfill`, and the open sweep below finds a pane already open and never
    // rebuilds a view for it. Asking the two sources of truth directly cannot
    // miss it. Retaining the last applied roster also covers a Station the new
    // hull removed entirely. The no-monitor/no-bus returns above leave that
    // history untouched, so a deferred pass cannot forget the close it owes.
    //
    // (The roster is the layout's, so a `--pane <NAME>` participant is out of
    // scope by construction — a hand-authored label that shadowed a station id
    // would put it back in, which is why `app::install_world_selection` refuses
    // one at boot.)
    let seated: Vec<&crate::core::messages::StationId> = layout
        .layout
        .monitors()
        .iter()
        .flat_map(|m| layout.layout.stations_on(m))
        .collect();
    let current_roster = layout.layout.roster().to_vec();
    let mut known_stations = std::mem::replace(&mut *applied_roster, current_roster.clone());
    for station in current_roster {
        if !known_stations.contains(&station) {
            known_stations.push(station);
        }
    }
    for station in known_stations.iter().filter(|s| !seated.contains(s)) {
        let Some(_pane) =
            console_assignment::close_console(&bus.0, auto_claims.as_deref_mut(), station)
        else {
            continue;
        };
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "bridge display: station {:?}'s console is closed; its station falls back \
             to AI control until somebody claims it again",
            station.0
        );
    }

    // ── rebuild what moved, on the same identity ────────────────────────────
    //
    // A view is created at one size, on one window. So a console the operator
    // moved to another screen — and one whose rectangle changed because a second
    // console joined its screen or left it — has a view that no longer fits its
    // seat, and there is no re-place: it has to be built again.
    //
    // Through `close` + `recreate`, which is issue #1125's crash path used
    // deliberately rather than a second mechanism. It keeps the SAME session
    // token, so the page's `Identify` is a reconnect the lobby answers by
    // restoring the held station and pushing the current projection — whoever
    // claimed that console keeps it across the move. `open_pending_views` builds
    // the new view against the surfaces this pass just rewrote, so it lands on
    // the screen the operator chose. The gap in between is the same one a view
    // crash has: a moment on `Backfill` while the page loads.
    //
    // Moved, re-tiled and resized consoles are rebuilt by the SAME two calls —
    // the work is identical and the reason is the same lost rectangle. What
    // differs is only what is *said* about them, and each says its own true
    // cause: a move the operator made is not announced at all, and the two kinds
    // of surprise ([`Rebuild::note`]) are announced as what they are. The notice
    // is raised HERE rather than from the classification directly, so it is only
    // ever said about a console that really was rebuilt — a pane the bus does
    // not have yet reconnects nothing, and promising a reconnect for it would be
    // a sentence the code does not honour.
    let mut retile_notices: Vec<LayoutNotice> = Vec::new();
    for (console, monitor, cause) in &rebuilds {
        let Some(pane) = bus.0.open_pane_for_name(console) else {
            // Nothing to rebuild. The open sweep below asks the bus about every
            // seated station, so this one is simply opened there.
            continue;
        };
        if bus.0.is_superseded(pane) {
            // Resizing its monitor or retiling its neighbour is not an
            // operator request to reconnect an identity that moved elsewhere.
            continue;
        }
        if let Some(note) = cause.note(console, monitor) {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge display: {note}; its view is rebuilt below"
            );
            retile_notices.push(LayoutNotice::Adopted(note));
        }
        match bus.0.rebuild(pane) {
            Some((rebuilt, _url)) => crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge display: {console:?}'s console is now on monitor {monitor}; its view is \
                 rebuilt as {rebuilt} on the same identity, so whoever claimed it keeps it"
            ),
            // `recreate` refuses a pane the registry cannot resolve, or one that
            // is not `Closed` — neither of which the line above can produce, so
            // this is a defensive arm rather than a reachable state. What it says
            // is nonetheless what the CODE then does, which is the only thing an
            // operator log may say: the console was closed, and only a STATION's
            // comes back on its own — the open sweep immediately below finds the
            // layout seating it with no pane and mints a FRESH one, on the screen
            // the operator chose but not on the old identity. An authored
            // `--pane`'s console is nothing's to reopen, so it says so rather
            // than promising a return.
            None => crate::pwarn!(
                log,
                LogCat::Lobby,
                "bridge display: {console:?}'s console is now on monitor {monitor} but its own \
                 identity could not be carried across; a station's console reopens there as a \
                 fresh one and must be claimed again, and a --pane participant's must be \
                 restarted with the host"
            ),
        }
    }

    // ── open a console for every seated station that has none ───────────────
    //
    // Asked of the BUS rather than derived from a diff, so the rule is "every
    // station the layout seats has a console" rather than "a console is opened
    // when a press adds one". The difference is what a **`--profile` that seats
    // a station** gets: its Station window and its pane slot exist from boot, so
    // a diff would find nothing new and leave the operator looking at an empty
    // screen. It also makes the sweep idempotent — running it twice opens one
    // console, which is what lets the pass above close-and-recreate without
    // having to tell this one what it did.
    for station in &seated {
        if bus.0.open_pane_for_name(&station.0).is_some() {
            continue;
        }
        let monitor = layout
            .layout
            .monitor_of(station)
            .expect("a seated station is on one of this bridge's monitors");
        let (pane, _url) =
            console_assignment::open_console(&bus.0, auto_claims.as_deref_mut(), station);
        // The URL is NOT logged: it carries this console's session token in its
        // fragment, and an operator log is a file, a scrollback and a screenshot.
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "bridge display: station {:?}'s console is opening on monitor {monitor} as {pane}; \
             it joins and auto-claims its station",
            station.0
        );
    }

    // ── close the windows nothing is left on ────────────────────────────────
    surfaces.0.retain(|surface| {
        if !surface.panes.is_empty() {
            return true;
        }
        pending_close.push(surface.window);
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "bridge display: monitor {} is holding no console, so its station window closes and \
             the screen is free again",
            surface.identity
        );
        false
    });

    // ── and say so, for the consoles nobody asked to move (issue #1332) ──────
    //
    // APPENDED, like every other writer of this list, and it marks the resource
    // changed — which is exactly how the notice reaches the row, because
    // `publish_bridge_layout` only pushes a layout that changed. The cost is one
    // extra pass next frame, which finds the rectangles it just wrote, re-tiles
    // nothing, says nothing and settles.
    //
    // LAST in the pass, after the window sweep, so a frame that surrenders every
    // console on a screen has already finished with it before the row is told.
    if !retile_notices.is_empty() {
        layout.notices.extend(retile_notices);
    }
}

/// How many consecutive frames a seated station may have no console on screen
/// before [`reconcile_seated_consoles`] acts on it (issue #1331).
///
/// The same window every other reader of a transient waits, and for the same
/// reason: the systems that open a console, build its view and place its Station
/// window are unordered against each other within `Update`, so a station is
/// legitimately half-seated for a frame or two after every press. Acting inside
/// that window would tear down a console that was about to work.
const CONSOLE_MISSING_GRACE_FRAMES: u32 = DISPLAY_LOSS_DEBOUNCE_FRAMES;

/// Reconcile what the **law** seats against what is actually **on screen**, and
/// surrender a seat the adapter cannot honour (issue #1331).
///
/// [`follow_layout_stations`] applies the layout; it does not check that the
/// application worked. Two things below it can fail after it has returned
/// happily, and both leave the same picture — a station card showing a screen,
/// a black borderless-fullscreen window, and nobody able to say why:
///
///  * the pane host's `make_pane_view` can fail (Ultralight refuses the view, the
///    document will not load), which leaves the pane open on the bus with no
///    surface behind it;
///  * a station can be seated on a monitor that is not present at the moment the
///    pass runs, which leaves the console with no [`BridgeStationSurfaces`] slot
///    to be composited into.
///
/// So this asks the only question that matters and asks it of reality: does every
/// seated station have a console *open on the bus* AND *a slot on a Station
/// surface*? A station that fails that for [`CONSOLE_MISSING_GRACE_FRAMES`]
/// consecutive frames is repaired the way issue #1125 repairs a crashed view —
/// `close` + `recreate` on the same session token, **bounded** by
/// [`PaneBus::record_recreation_within_budget`](crate::native_host::panes::transport::PaneBus::record_recreation_within_budget),
/// so a console that cannot be built does not flap forever.
///
/// # Exhausted recovery frees the physical screen and retains the assignment
///
/// The bound has to end somewhere, and "leave it closed for the operator" —
/// which is the right answer for a `--pane` — would here leave the LAW still
/// seating a station whose card claims a screen it is not on. So the seat is
/// surrendered through the law itself (`UnassignStation`), which frees the
/// physical screen: the Station window closes and the viewscreen may move onto
/// it. `ConsoleAssignments` retains the station reservation and its identity,
/// while `Backfill` operates during disconnection. The row keeps Off and move
/// available and explains that the console is unavailable. The operator sees a
/// [`LayoutNotice`] the row renders — the same channel an unplugged viewscreen
/// reports through — because a console that silently never appeared is exactly
/// the failure this whole slice exists to make impossible.
fn reconcile_seated_consoles(
    monitors: Query<&Monitor>,
    layout: Option<ResMut<BridgeLayoutResource>>,
    surfaces: Option<Res<BridgeStationSurfaces>>,
    bus: Option<Res<crate::native_host::panes::PaneBusResource>>,
    log: Option<Res<LogFilterConfig>>,
    // Consecutive frames each seated station has been missing its console.
    mut missing: Local<std::collections::HashMap<crate::core::messages::StationId, u32>>,
) {
    let (Some(mut layout), Some(surfaces), Some(bus)) = (layout, surfaces, bus) else {
        return;
    };
    // A frame reporting NO monitors at all is winit between hot-plug events, and
    // both the applier and the watcher decline to act on one. So must this: with
    // no monitors `follow_layout_stations` returns with its pass still owed, so a
    // station seated on such a frame has no surface and no console through no
    // fault of anything below this layer — and counting those frames against the
    // grace would surrender, after ten of them, a seat the applier never got a
    // chance to attempt. The counters are left exactly as they are, so a genuine
    // failure already part-way through its window neither restarts nor advances.
    if monitors.is_empty() {
        return;
    }
    let seated: Vec<crate::core::messages::StationId> = layout
        .layout
        .monitors()
        .iter()
        .flat_map(|m| layout.layout.stations_on(m))
        .cloned()
        .collect();
    // A bridge nobody has put a console on has nothing to reconcile, which is
    // every frame of a host nobody rearranged.
    if seated.is_empty() {
        if !missing.is_empty() {
            missing.clear();
        }
        return;
    }
    missing.retain(|station, _| seated.contains(station));

    let mut surrender: Vec<(crate::core::messages::StationId, MonitorIdentity)> = Vec::new();
    for station in &seated {
        let pane = bus.0.open_pane_for_name(&station.0);
        if pane.is_some_and(|id| bus.0.is_superseded(id)) {
            // The Session moved to another connection. This is neither a
            // crashed view nor authority to change the operator's screen plan.
            missing.remove(station);
            continue;
        }
        // BOTH halves, because either alone is a lie: a pane with no slot is
        // built on the wrong window (or not at all), and a slot with no pane is
        // a lit screen with nothing on it.
        if pane.is_some() && surfaces.slot_for(&station.0).is_some() {
            missing.remove(station);
            continue;
        }
        let strikes = missing.entry(station.clone()).or_insert(0);
        *strikes += 1;
        if *strikes < CONSOLE_MISSING_GRACE_FRAMES {
            continue;
        }
        missing.remove(station);
        let monitor = layout
            .layout
            .monitor_of(station)
            .cloned()
            .expect("a seated station is on one of this bridge's monitors");
        let Some(pane) = pane else {
            // Nothing on the bus at all. Either the rebuilds above have already
            // spent the budget and the pane host left it closed, or a fault did
            // — both are "this console is not coming back on its own".
            surrender.push((station.clone(), monitor));
            continue;
        };
        // Issue #1125's crash path, used deliberately: the same session token
        // survives, so a console that DOES come back is still the same
        // participant. The budget is the same per-identity one a flapping view
        // crash is held to, and it is what stops this becoming a rebuild loop.
        if !bus.0.record_recreation_within_budget(pane) {
            bus.0.close(pane);
            surrender.push((station.clone(), monitor));
            continue;
        }
        match bus.0.rebuild(pane) {
            Some((rebuilt, _url)) => crate::pwarn!(
                log,
                LogCat::Lobby,
                "bridge display: station {:?}'s console is seated on monitor {monitor} but has \
                 nothing on screen; rebuilding it as {rebuilt} on the same identity",
                station.0
            ),
            None => surrender.push((station.clone(), monitor)),
        }
    }

    if surrender.is_empty() {
        return;
    }
    // APPENDED, not assigned: this chain and the lobby's are unordered within
    // `Update`, and a press landing on the same frame as a surrender would
    // otherwise erase whichever of the two ran first — most damagingly the
    // `ConsoleCouldNotOpen` below, which is the whole reason this system talks to
    // the row at all. See `BridgeLayoutResource::notices`.
    let mut notices: Vec<LayoutNotice> = Vec::new();
    for (station, monitor) in surrender {
        let unseated = layout
            .layout
            .apply(&super::bridge_layout::LayoutAction::UnassignStation {
                station: station.clone(),
            });
        match unseated {
            Ok(next) => {
                layout.layout = next;
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "bridge display: station {:?}'s console could not be put on monitor \
                     {monitor} after {} attempt(s), so its seat is given back and the screen \
                     is free again; the station is on AI control until somebody opens it \
                     somewhere that works",
                    station.0,
                    crate::native_host::panes::recovery::MAX_RECREATIONS_PER_WINDOW
                );
                notices.push(LayoutNotice::Adopted(
                    super::bridge_layout::LayoutAdoption::ConsoleCouldNotOpen { station, monitor },
                ));
            }
            // Unreachable: unassigning a station this layout seats is always
            // lawful. Reported rather than swallowed, because a seat that could
            // be neither honoured nor given back is worth a sentence.
            Err(refusal) => {
                crate::pwarn!(log, LogCat::Lobby, "bridge display: {refusal}");
                notices.push(LayoutNotice::Refused(refusal));
            }
        }
    }
    layout.notices.extend(notices);
}

// ── runtime display loss (issue #1125) ──────────────────────────────────────

/// How many consecutive frames a configured monitor must be absent before its
/// panes are disconnected (issue #1125).
///
/// `bevy_winit`'s `create_monitors` despawns a [`Monitor`](bevy::window::Monitor)
/// entity on any event-loop iteration where winit's `available_monitors()` stops
/// reporting it — and on Windows that set can blip for a frame or two during a
/// GPU reset (TDR), a monitor waking from DPMS, or a dock/undock, then recover.
/// Acting on a single change-frame would flip a live human's station to Backfill
/// for a transient that never really disconnected. So a loss must persist this
/// many consecutive observations first — the display-side echo of
/// [`VIEW_CRASH_COPY_FAILURES`](crate::native_host::panes::mirror::VIEW_CRASH_COPY_FAILURES),
/// deliberately shorter because a genuine unplug should still reach Backfill
/// promptly, and a returning monitor before the window is up costs nothing but a
/// cleared counter.
const DISPLAY_LOSS_DEBOUNCE_FRAMES: u32 = 10;

/// Watch the live monitor set and react to a configured display lost — or
/// returned — **mid-mission** (issue #1125).
///
/// [`apply_bridge_profile`] runs once, at setup, and reports a profile-vs-hardware
/// mismatch it finds then ([`super::bridge_profile::ProfileProblem`]). This is its
/// runtime companion: a monitor that *was* present and driving panes and is
/// unplugged while the mission runs. `bevy_winit` despawns a [`Monitor`] entity
/// when its display disconnects, so this diffs the identities present this frame
/// against the previous frame's and, on a change:
///
/// * names each lost configured monitor exactly (an authored
///   [`RuntimeDisplayLoss`](super::bridge_profile::RuntimeDisplayLoss)), never
///   re-homing its role — the same doctrine [`resolve`] holds at setup;
/// * closes the panes a lost **Station** carried, so their tokens disconnect and
///   their stations flip to Backfill through the ordinary dropped-participant
///   path — *not* a fault, so there is no auto-recreate: the display is gone,
///   there is nowhere to rebuild a view, and bringing it back is an explicit
///   repair (see below);
/// * reports a **returned** monitor for that explicit repair, and does nothing
///   automatic — re-applying the profile is the deliberate act that places a
///   surface, so no pane is ever silently moved onto reappeared hardware (AC5).
///
/// The `Local` baseline starts unset and is filled on the first frame that sees
/// any monitor, so the very first observation establishes the ground truth
/// rather than reporting every present monitor as "new". Gated on
/// [`BridgeDisplayApplied`] so that baseline is the applied state.
///
/// Its pure half — which monitors were lost or returned, and which panes a loss
/// names — is [`runtime_display_losses`]/[`runtime_display_returns`], tested by
/// the ordinary `cargo test` runs; this adapter only reads the live monitors and
/// closes the named panes on the bus. The unplug-and-replug proof itself needs
/// real hardware and is the `#[ignore]`d `tests/native_bridge_displays.rs`.
///
/// # It also rebuilds the live layout (issue #1330)
///
/// The same frame's monitor list is what the lobby's monitor row is drawn from,
/// so a cable that moves has to reach [`BridgeLayoutResource`] as well as the
/// panes. It does, through [`BridgeLayout::reconcile`] — but only once the new
/// roster has **settled**, on the same
/// [`DISPLAY_LOSS_DEBOUNCE_FRAMES`] window the pane closures use and for a
/// sharper reason: a reconcile whose viewscreen monitor is missing moves the
/// viewscreen to the primary, and acting on a one-frame winit blip would
/// therefore drag the shared view across the room and back.
///
/// # The two debounces count the same observation, and in a fixed order
///
/// This system runs two windows over one frame's monitor list — the roster
/// settle above, and the per-monitor absence streak the pane closures wait for —
/// and they used to reset on *different* events: the settle restarted whenever
/// the roster changed, while a streak only cleared when its own monitor came
/// back. On a roster still churning around a genuinely absent display, the
/// streak could therefore cross its threshold while the settle kept restarting,
/// and a pane would be closed for a monitor the layout still seated.
///
/// So both now restart on the same thing — **the observed roster changing** —
/// which makes them reach their thresholds on the same frame, and
/// [`reconcile_layout`] runs first within that frame. The unseat always precedes
/// the close, rather than usually preceding it.
// Nine parameters, and every one is a distinct thing this frame's observation
// is judged against or written to. A Bevy system's parameter list IS its
// dependency declaration to the scheduler, so bundling them into a struct would
// hide what it reads rather than simplify anything.
#[allow(clippy::too_many_arguments)]
fn watch_runtime_displays(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    config: Res<BridgeDisplayConfig>,
    bus: Option<Res<crate::native_host::panes::PaneBusResource>>,
    layout: Option<ResMut<BridgeLayoutResource>>,
    log: Option<Res<LogFilterConfig>>,
    mut baseline: Local<Option<std::collections::HashSet<MonitorIdentity>>>,
    mut absent_streak: Local<std::collections::HashMap<MonitorIdentity, u32>>,
    mut settling_roster: Local<Option<(Vec<MonitorIdentity>, u32)>>,
    // The previous frame's observed roster, which is what both windows restart
    // on — see the note above.
    mut last_observed: Local<Option<Vec<MonitorIdentity>>>,
) {
    let mut raws: Vec<RawMonitor> = monitors
        .iter()
        .map(|(m, primary)| raw_from_monitor(m, primary))
        .collect();
    if raws.is_empty() {
        // No monitors reported this frame — winit has not populated them yet, or
        // a transient empty frame during a hot-plug. Treating that as "every
        // display was lost" would be wrong, so wait for a frame that has some.
        return;
    }
    // The one order every reader of a live monitor set uses, so the identities
    // this frame derives are the identities the applier seeded the layout with.
    // The set below is a `HashSet` and the two diffs are set operations, so
    // sorting changes nothing about the #1125 half.
    raws.sort_by_key(|r| (r.position_x, r.position_y));

    // Derived ONCE, here, and handed to both halves: the roster settle and the
    // absence streak must be counting the same observation, or they reach their
    // thresholds on different frames — see the note on this system.
    let known: Vec<DiscoveredMonitor> = layout
        .as_ref()
        .map(|l| l.monitors.clone())
        .unwrap_or_default();
    let discovered = identify_stable(&raws, &known);
    let observed: Vec<MonitorIdentity> = discovered.iter().map(|d| d.identity.clone()).collect();

    reconcile_layout(layout, discovered, &log, &mut settling_roster);

    // A roster that CHANGED restarts every absence streak, exactly as it
    // restarts the settle above. The whole point of both windows is "wait until
    // the hardware picture has stopped moving before acting on it", and a streak
    // that survived a change would let a pane close for a monitor the layout was
    // still, lawfully, seating a console on.
    if last_observed.as_ref() != Some(&observed) {
        absent_streak.clear();
    }
    *last_observed = Some(observed);

    // The BOOT profile's assignments, deliberately — this is #1125's question
    // ("was a monitor this host was configured for lost, and whose panes were on
    // it"), which is about the arrangement the host was started with. It does
    // NOT follow the lobby: after a `SetViewscreen` press the viewscreen role
    // here still names the monitor the host booted on, so a loss/return report
    // can name the wrong screen as "the viewscreen". Left as it is because the
    // consequence is confined to two log lines — the pane closures below are
    // driven by `pane_labels`, which a viewscreen entry never has, and on a
    // no-`--profile` host no entry has any at all.
    //
    // A console the LOBBY opened on a screen that is unplugged is closed all the
    // same, and by the layout rather than by this: `reconcile_layout` above
    // unseats it and `follow_layout_stations` closes it on the same frame. And
    // an AUTHORED profile's station console goes the same way, because
    // `assigned_surfaces` excludes a station-bearing slot from `pane_labels`
    // (issue #1331) — see the settled note in `apply_bridge_profile`.
    let assigned = config.profile.assigned_surfaces();
    // The assigned monitors present THIS frame, matched STABLY against the raw
    // monitors rather than re-derived with `identify` — so the survivor of two
    // identical monitors keeps its own suffixed identity when its twin leaves,
    // instead of shifting to the short key and being misread as also lost (issue
    // #1125). See `present_assigned_identities`.
    let current = present_assigned_identities(&assigned, &raws);

    // Establish the committed baseline on the first observed frame, then diff.
    let committed = match baseline.as_mut() {
        Some(committed) => committed,
        None => {
            *baseline = Some(current);
            return;
        }
    };

    // A returned monitor is reported immediately — it only logs, moves no pane —
    // and clears any pending absence streak it had accumulated.
    for returned in runtime_display_returns(&assigned, committed, &current) {
        crate::pinfo!(log, LogCat::Lobby, "bridge display: {returned}");
        absent_streak.remove(&returned.identity);
    }

    // A loss is DEBOUNCED before its panes are closed: a monitor absent this frame
    // is only a *candidate*, and its streak must reach `DISPLAY_LOSS_DEBOUNCE_FRAMES`
    // consecutive frames before we believe it — a one- or two-frame winit blip
    // (a GPU reset, a monitor waking) then disconnects no station. Candidates that
    // reappear before the window is up have their streak cleared below.
    let candidates = runtime_display_losses(&assigned, committed, &current);
    absent_streak.retain(|id, _| candidates.iter().any(|c| &c.identity == id));

    let mut confirmed: Vec<RuntimeDisplayLoss> = Vec::new();
    for loss in candidates {
        let streak = absent_streak.entry(loss.identity.clone()).or_insert(0);
        *streak += 1;
        if *streak >= DISPLAY_LOSS_DEBOUNCE_FRAMES {
            absent_streak.remove(&loss.identity);
            confirmed.push(loss);
        }
    }

    for loss in &confirmed {
        crate::pwarn!(log, LogCat::Lobby, "bridge display: {loss}");
        if let Some(bus) = &bus {
            for label in &loss.pane_labels {
                if let Some(id) = bus.0.open_pane_for_name(label) {
                    // The dropped-phone path, deliberately: a plain `close`, which
                    // owes the lobby one `PlayerDisconnected` and flips the
                    // station to Backfill. NOT `fault` — a fault would ask the
                    // pane host to rebuild the view, and there is no display to
                    // rebuild it on.
                    bus.0.close(id);
                }
            }
        }
    }

    // Commit the new ground truth: monitors present this frame join the committed
    // set (a return, or one still present); confirmed losses leave it. An
    // UNconfirmed candidate stays committed so it keeps being detected next frame
    // until it either returns or crosses the debounce.
    for id in &current {
        committed.insert(id.clone());
    }
    for loss in &confirmed {
        committed.remove(&loss.identity);
    }
}

/// Rebuild [`BridgeLayoutResource`] against the monitors present now, once the
/// roster has stopped changing (issue #1330).
///
/// Split out of [`watch_runtime_displays`] because it answers a different
/// question about the same frame: that system asks "did an *assigned* display
/// go away, and whose console was on it"; this asks "is this still the bridge
/// the layout describes". They share the observation and nothing else.
///
/// # Why it settles instead of reacting
///
/// [`BridgeLayout::reconcile`] moves the viewscreen to the primary when the
/// monitor it was on is gone. That is right for an unplug and disastrous for a
/// blip: winit's monitor list can flicker for a frame or two through a GPU
/// reset or a monitor waking, and reacting to that would drag the shared view
/// onto another screen and back while the crew watched. So a *different* roster
/// has to be reported [`DISPLAY_LOSS_DEBOUNCE_FRAMES`] times **in a row, and be
/// the same roster each time**, before it is believed — a set that is still
/// changing simply restarts the count.
///
/// Every degradation the rebuild causes becomes a
/// [`LayoutNotice`] on the resource, which is how the operator finds out that
/// the screen they chose took the viewscreen's home with it.
///
/// # Why the identities are carried, not re-derived
///
/// [`identify`]'s answer depends on the set it is given: an identical twin
/// arriving suffixes *both* of a pair with `#x,y`, that twin leaving collapses
/// the survivor back to the short key, and a television renegotiating its mode
/// rewrites the `WxH` half outright. Comparing a freshly-derived string against
/// the layout's stored one therefore reads three ordinary events — a plug, an
/// unplug of some *other* screen, a resolution change — as "the monitor the
/// viewscreen is on has gone", which fires
/// [`ViewscreenMonitorGone`](super::bridge_layout::LayoutAdoption::ViewscreenMonitorGone),
/// throws away the operator's choice and has
/// [`follow_layout_viewscreen`] slam the primary window into borderless
/// fullscreen. On a host nobody touched. That is precisely the promise in this
/// module's [note](self#boot-is-once-the-viewscreen-is-apply-on-change-issue-1330).
///
/// So `discovered` is built by [`identify_stable`], which matches the layout's
/// existing identities to the monitors present *by form* — the same technique
/// [`present_assigned_identities`] uses on the pane side, and for the same
/// reason — and carries a survivor's identity forward. Only a display no known
/// identity could claim is genuinely absent, and only that reconciles away. The
/// carried identities are then written back to
/// [`BridgeLayoutResource::monitors`], because the row is drawn from those and
/// the button's round trip is exact string equality: the row must serve the key
/// the layout will judge the press against.
///
/// It is derived by the CALLER and handed in, rather than derived here, so that
/// the absence streak beside it is counting the very same observation — see
/// [`watch_runtime_displays`]'s note on the two windows.
fn reconcile_layout(
    layout: Option<ResMut<BridgeLayoutResource>>,
    discovered: Vec<DiscoveredMonitor>,
    log: &Option<Res<LogFilterConfig>>,
    settling: &mut Local<Option<(Vec<MonitorIdentity>, u32)>>,
) {
    let Some(mut layout) = layout else {
        return;
    };
    let identities: Vec<MonitorIdentity> = discovered.iter().map(|d| d.identity.clone()).collect();
    if identities == layout.layout.monitors() {
        **settling = None;
        // Same bridge, possibly moved or re-moded. The LAW has nothing to
        // rebuild — a monitor's geometry is not part of its identity — but the
        // row is drawn from these geometries and the next roster change is
        // matched against these positions, so a stale copy would show the wrong
        // size on a button and anchor the next carry-forward to a corner the
        // display has left. Written only when it actually differs: an
        // unconditional write would mark the resource changed every frame and
        // re-encode the payload on the thread `FixedUpdate` runs `SimSet` on.
        if discovered != layout.monitors {
            layout.monitors = discovered;
        }
        return;
    }

    let count = match settling.as_ref() {
        Some((pending, count)) if pending == &identities => count + 1,
        // A roster still in motion restarts the count rather than inheriting
        // the previous candidate's.
        _ => 1,
    };
    if count < DISPLAY_LOSS_DEBOUNCE_FRAMES {
        **settling = Some((identities, count));
        return;
    }
    **settling = None;

    let roster = layout.layout.roster().to_vec();
    let (next, notes) = layout.layout.reconcile(&discovered, roster);
    for note in &notes {
        crate::pwarn!(log, LogCat::Lobby, "bridge display: {note}");
    }
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "bridge display: the bridge now has {} monitor(s); the viewscreen is on {}",
        next.monitors().len(),
        next.viewscreen()
    );
    layout.layout = next;
    layout.monitors = discovered;
    // Appended for `reconcile_seated_consoles`' reason, and this is the writer a
    // press is most likely to race: a roster settling and an operator reaching
    // for a button are independent events.
    layout
        .notices
        .extend(notes.into_iter().map(LayoutNotice::Adopted));
}

// ── the --setup enumeration mode ──────────────────────────────────────────

/// How many frames [`run_setup`] waits for winit to report the monitors before
/// giving up. Generous: monitor sync is normally done within a frame or two, and
/// this is a one-shot operator command, so a long ceiling costs nothing and
/// avoids a false "no monitors" on a slow-starting display driver.
const SETUP_FRAME_BUDGET: u32 = 600;

/// Marks whether [`run_setup`] has printed its report, so it does so once.
#[derive(Resource)]
struct SetupProfile(Option<super::bridge_profile::BridgeProfile>);

/// Frame counter for [`run_setup`]'s bounded wait.
#[derive(Resource, Default)]
struct SetupFrames(u32);

/// Run the operator-facing `--setup` mode: open a hidden winit window purely to
/// enumerate the monitors, print the discovery report (validating `profile` if
/// one was given), and exit ([ai] decision: the setup surface for #1123 is this
/// enumeration mode plus the hand-editable profile file, not an interactive UI —
/// the touch/keyboard-operable setup screen is #1124/#1128's).
///
/// Needs a real display and a GPU adapter, like every winit path here, so it is
/// never run in CI; it is the local half of the acceptance criteria. Returns a
/// process exit code: 0 once monitors were reported and (when a profile was
/// given) it checks out; 1 if none were reported after [`SETUP_FRAME_BUDGET`]
/// frames, or if a supplied `--profile` is invalid or does not match the
/// connected displays — see [`setup_profile_is_clean`]. `from_toml` upstream
/// only parses, so this is what makes a script or CI gating on `--setup`'s
/// exit status get an honest verdict instead of always seeing success; the
/// authoritative `--world` path already refuses to boot on an invalid profile
/// (`phoenix_host.rs`), and this brings `--setup` to the same standard.
pub fn run_setup(profile: Option<super::bridge_profile::BridgeProfile>) -> i32 {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("{} — display setup", super::WINDOW_TITLE),
                    visible: false,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "warn".to_string(),
                ..default()
            }),
    );
    app.insert_resource(SetupProfile(profile));
    app.init_resource::<SetupFrames>();
    #[cfg(all(feature = "host", target_os = "windows"))]
    app.insert_non_send_resource(SetupCameraScan::default());
    app.add_systems(Update, setup_enumerate);
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(code) => code.get() as i32,
    }
}

/// Enumerate monitors, print the setup report, and exit. Waits (up to
/// [`SETUP_FRAME_BUDGET`] frames) for winit to populate the monitor list.
#[cfg(all(feature = "host", target_os = "windows"))]
#[derive(Default)]
struct SetupCameraScan {
    scan: Option<super::media_camera::CameraScan>,
    started: Option<std::time::Instant>,
}

fn setup_enumerate(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    profile: Res<SetupProfile>,
    mut frames: ResMut<SetupFrames>,
    #[cfg(all(feature = "host", target_os = "windows"))] mut cameras: NonSendMut<SetupCameraScan>,
    mut exit: MessageWriter<AppExit>,
) {
    frames.0 += 1;
    let raws: Vec<RawMonitor> = monitors
        .iter()
        .map(|(m, primary)| raw_from_monitor(m, primary))
        .collect();
    let budget_spent = frames.0 >= SETUP_FRAME_BUDGET;
    if raws.is_empty() && !budget_spent {
        return;
    }
    let discovered = identify(&raws);
    #[cfg(not(feature = "host"))]
    let report = super::bridge_profile::render_setup_report(&discovered, profile.0.as_ref());
    #[cfg(feature = "host")]
    let report = {
        let mut report =
            super::bridge_profile::render_display_setup_report(&discovered, profile.0.as_ref());
        use super::bridge_media::MediaKind;
        let mut devices = Vec::new();
        let mut supported = Vec::new();
        #[cfg(target_os = "windows")]
        {
            if cameras.started.is_none() {
                cameras.started = Some(std::time::Instant::now());
                match super::media_camera::CameraScan::begin() {
                    Ok(scan) => cameras.scan = Some(scan),
                    Err(error) => {
                        report.push_str(&format!("\nCamera enumeration unavailable: {error}\n"))
                    }
                }
            }
            if let Some(scan) = &cameras.scan {
                match scan.poll() {
                    Ok(Some(found)) => {
                        devices.extend(super::media_camera::discovered_cameras(&found));
                        supported.push(MediaKind::Camera);
                        cameras.scan = None;
                    }
                    Ok(None)
                        if cameras.started.is_some_and(|start| {
                            start.elapsed() < std::time::Duration::from_secs(10)
                        }) =>
                    {
                        return
                    }
                    Ok(None) => {
                        report.push_str("\nCamera enumeration timed out.\n");
                        cameras.scan = None;
                    }
                    Err(error) => {
                        report.push_str(&format!("\nCamera enumeration unavailable: {error}\n"));
                        cameras.scan = None;
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        report.push_str("\nCamera enumeration is supported on Windows only.\n");
        match super::media_output::OutputDevices::scan() {
            Ok(outputs) => {
                devices.extend(outputs.discovered());
                supported.push(MediaKind::Output);
            }
            Err(error) => report.push_str(&format!("\nOutput enumeration unavailable: {error}\n")),
        }
        match super::media_microphone::Microphones::scan() {
            Ok(mics) => {
                devices.extend(mics.discovered());
                supported.push(MediaKind::Microphone);
            }
            Err(error) => {
                report.push_str(&format!("\nMicrophone enumeration unavailable: {error}\n"))
            }
        }
        report.push_str("\nMedia tests: --test-output, --meter-microphone, --preview-camera with --setup --profile and a surface name.\nUnnamed/duplicate audio names cannot be tested safely; assign unique OS names.\n");
        report.push_str(&super::bridge_media::render_available_setup_report(
            &devices,
            profile.0.as_ref(),
            &supported,
        ));
        report
    };
    // The accessibility half of the report (issue #1128): per-pane reflow
    // headroom at the supported scaling extremes, the keyboard-focus order across
    // monitors, and the OS accessibility preferences the panes and reticle start
    // from. Resolved here (not inside `render_setup_report`) because it is the
    // adapter that reads the machine's OS preferences.
    let prefs = super::panes::os_prefs::query_os_accessibility_prefs();
    let resolved = profile.0.as_ref().and_then(|p| {
        p.validate()
            .ok()
            .map(|v| super::bridge_profile::resolve(&v, &discovered))
    });
    let accessibility =
        super::setup_accessibility::render_accessibility_setup_report(resolved.as_ref(), &prefs);
    // Operator output, on the same footing as `phoenix-host`'s other CLI prints:
    // stdout, not the `plog!` family.
    print!("{report}{accessibility}");
    if discovered.is_empty() {
        eprintln!(
            "phoenix-host --setup: no monitors were reported after {} frames",
            frames.0
        );
        exit.write(AppExit::error());
    } else if !setup_profile_is_clean(profile.0.as_ref(), &discovered) {
        // The report above already printed *why* — an invalid profile, or one
        // naming a monitor that is not connected. A non-zero exit here is what
        // makes that a verdict a script can gate on, rather than something only
        // visible to a human reading stdout.
        eprintln!("phoenix-host --setup: --profile does not check out; see the report above");
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

/// Whether a supplied `--profile` is fit to gate `--setup`'s exit code on: it
/// parses, validates (schema version, role vocabulary, pane density, the
/// one-viewscreen rule), and resolves against `discovered` with no
/// [`ProfileProblem`](super::bridge_profile::ProfileProblem) — no monitor the
/// profile assigns is missing, and none of `discovered` is left unassigned.
///
/// `None` (no profile was given — a bare `phoenix-host --setup`) is always
/// clean: with nothing to check, there is nothing to fail. Pure and
/// display-free, unlike [`run_setup`]/[`setup_enumerate`], so it is directly
/// unit-testable without a winit window.
fn setup_profile_is_clean(
    profile: Option<&super::bridge_profile::BridgeProfile>,
    discovered: &[super::bridge_profile::DiscoveredMonitor],
) -> bool {
    let Some(profile) = profile else {
        return true;
    };
    match profile.validate() {
        Ok(validated) => !resolve(&validated, discovered).has_problems(),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "bridge_display_tests.rs"]
mod tests;
