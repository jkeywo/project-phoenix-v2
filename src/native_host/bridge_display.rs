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

use super::bridge_layout::BridgeLayout;
use super::bridge_profile::{
    identify, identify_stable, present_assigned_identities, resolve, runtime_display_losses,
    runtime_display_returns, DiscoveredMonitor, DisplayRole, MonitorGeometry, MonitorIdentity,
    PaneRect, PaneSlot, RawMonitor, RuntimeDisplayLoss, ValidatedProfile,
};
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
        app.init_resource::<PendingConsoleClaims>();
        // Native auto-claim: seat each console's station once its session
        // registers. Gated on the lobby inbound stream so a host built without
        // it (a bare display test) stands the plugin up without the writer
        // panicking on a missing `Messages` resource.
        app.add_systems(
            Update,
            apply_pending_console_claims.run_if(
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

/// Consoles opened on a screen that still owe their station a claim.
///
/// Putting a station on a screen should ALSO claim it (native), so an operator
/// who assigns a console to their glass is seated without a second manual press.
/// The console pane joins as an ordinary participant on a freshly minted token
/// (`transport::PaneBus::open_console`), and a claim can only name a session the
/// lobby has already registered — so [`reconcile_seated_consoles`] records the
/// intent here at open, and [`apply_pending_console_claims`] emits the
/// `SelectStation` the frame that session appears. Nothing here is a reserved
/// token, so admission still cannot tell the console from a phone.
#[derive(Resource, Default)]
struct PendingConsoleClaims(Vec<PendingConsoleClaim>);

struct PendingConsoleClaim {
    /// The console pane's own session token.
    token: String,
    /// The station to claim — an id, which `lobby::stations_config::get_station`
    /// resolves the same as a name.
    station: String,
    /// Frames waited for the session to register, bounded so a console whose page
    /// never connected does not keep a claim pending for the life of the host.
    waited: u32,
}

/// Frames a pending claim waits for its console's session before it is dropped —
/// ~10s at 60 fps, generous for a slow page load and bounded so a failed pane
/// stops being retried.
const PENDING_CLAIM_MAX_FRAMES: u32 = 600;

/// Claim each console's station the frame its participant's session registers,
/// so putting a station on a screen seats it (native auto-claim).
///
/// Deferred rather than sent at open because the console's page has to connect
/// and `Identify` first; until its token is a connected session the lobby
/// handler would run against an unknown token and drop the claim. Sending it on
/// the console's OWN token keeps the seat attributed to that console, exactly as
/// a phone's claim is, and `handle_select_station` no-ops harmlessly if the seat
/// was taken in the meantime.
fn apply_pending_console_claims(
    claims: Option<ResMut<PendingConsoleClaims>>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    mut inbound: MessageWriter<crate::lobby::InboundMessage>,
) {
    let (Some(mut claims), Some(sessions)) = (claims, sessions) else {
        return;
    };
    if claims.0.is_empty() {
        return;
    }
    claims.0.retain_mut(|claim| {
        let registered = sessions
            .0
            .players()
            .iter()
            .any(|p| p.connected && p.token == claim.token);
        if registered {
            inbound.write(crate::lobby::InboundMessage {
                token: claim.token.clone(),
                msg: crate::core::messages::ClientMessage::SelectStation {
                    station: claim.station.clone(),
                },
            });
            return false;
        }
        claim.waited += 1;
        claim.waited < PENDING_CLAIM_MAX_FRAMES
    });
}

/// Lift one Bevy [`Monitor`](bevy::window::Monitor) into a [`RawMonitor`].
fn raw_from_monitor(monitor: &Monitor, primary: bool) -> RawMonitor {
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
fn geometry_of(monitor: &Monitor) -> MonitorGeometry {
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
fn identify_present(
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
        .unwrap_or_default()
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
    let (layout, adoption) = match &authored {
        Some(config) => base.adopt_profile(&config.profile),
        None => (base, Vec::new()),
    };
    // Boot notes are LOGGED, never pushed to the lobby: a hand-authored
    // `--pane` profile produces one per participant slot, and opening the
    // monitor row with a wall of them would bury the answers to actual presses.
    for note in &adoption {
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
/// A layout naming a monitor that is not present *this frame* is left alone
/// rather than retried into a loop: that only happens in the window between a
/// display going away and [`watch_runtime_displays`] believing it, and the
/// reconcile at the end of that window is what moves the layout somewhere real.
fn follow_layout_viewscreen(
    monitors: Query<(Entity, &Monitor, Has<PrimaryMonitor>)>,
    layout: Option<Res<BridgeLayoutResource>>,
    mut applied: ResMut<BridgeDisplayApplied>,
    mut primary: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(layout) = layout else {
        return;
    };
    let wanted = layout.layout.viewscreen().clone();
    if applied.viewscreen.as_ref() == Some(&wanted) {
        return;
    }

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
        return;
    };
    let Ok((window_entity, mut window)) = primary.single_mut() else {
        return;
    };
    window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Entity(*monitor_entity));
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
    // Asked of the BUS ∩ the LAW — every station on this bridge's roster that
    // the layout does not seat, whose console the bus still has open — rather
    // than of `carried`, which is this adapter's own bookkeeping and can have
    // been emptied by the retain above before the reconcile got round to
    // unseating what was on it. A console outliving its seat is the one failure
    // with no way back: the pane never closes, so its station never flips to
    // `Backfill`, and the open sweep below finds a pane already open and never
    // rebuilds a view for it. Asking the two sources of truth directly cannot
    // miss it.
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
    for station in layout
        .layout
        .roster()
        .iter()
        .filter(|s| !seated.contains(s))
    {
        let Some(pane) = bus.0.open_pane_for_name(&station.0) else {
            continue;
        };
        // The dropped-phone path, deliberately: a plain `close`, which owes the
        // lobby one `PlayerDisconnected` and flips the station to `Backfill`.
        // NOT a fault — a fault asks the pane host to rebuild the view, and this
        // console was closed on purpose.
        bus.0.close(pane);
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
        if let Some(note) = cause.note(console, monitor) {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge display: {note}; its view is rebuilt below"
            );
            retile_notices.push(LayoutNotice::Adopted(note));
        }
        bus.0.close(pane);
        match bus.0.recreate(pane) {
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
        let (pane, _url) = bus.0.open_console(&station.0);
        // Putting a station on a screen also CLAIMS it (native auto-claim):
        // record the intent against this console's OWN freshly minted token, to
        // be sent as a `SelectStation` once its session registers. Without this
        // the console opens on the glass but sits in the lobby until the operator
        // presses claim — a second step the physical bridge should not need.
        if let (Some(claims), Some(token)) = (auto_claims.as_mut(), bus.0.token_of(pane)) {
            claims.0.push(PendingConsoleClaim {
                token,
                station: station.0.clone(),
                waited: 0,
            });
        }
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
/// # And when the budget is spent, the seat is given back
///
/// The bound has to end somewhere, and "leave it closed for the operator" —
/// which is the right answer for a `--pane` — would here leave the LAW still
/// seating a station whose card claims a screen it is not on. So the seat is
/// surrendered through the law itself (`UnassignStation`), which frees the
/// screen everywhere at once: the row draws it as free, the Station window
/// closes, the viewscreen may move onto it, and the station is on `Backfill`
/// honestly rather than by accident. The operator is told with a
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
        bus.0.close(pane);
        if !bus.0.record_recreation_within_budget(pane) {
            surrender.push((station.clone(), monitor));
            continue;
        }
        match bus.0.recreate(pane) {
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
    app.add_systems(Update, setup_enumerate);
    match app.run() {
        AppExit::Success => 0,
        AppExit::Error(code) => code.get() as i32,
    }
}

/// Enumerate monitors, print the setup report, and exit. Waits (up to
/// [`SETUP_FRAME_BUDGET`] frames) for winit to populate the monitor list.
fn setup_enumerate(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    profile: Res<SetupProfile>,
    mut frames: ResMut<SetupFrames>,
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
    let report = super::bridge_profile::render_setup_report(&discovered, profile.0.as_ref());
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
mod tests {
    use super::*;

    use crate::native_host::bridge_layout::LayoutAction;

    #[test]
    fn a_monitor_component_lifts_into_a_raw_monitor() {
        let monitor = Monitor {
            name: Some("DELL U2720Q".to_string()),
            physical_width: 3840,
            physical_height: 2160,
            physical_position: IVec2::new(0, 0),
            refresh_rate_millihertz: Some(60_000),
            scale_factor: 1.5,
            video_modes: Vec::new(),
        };
        let raw = raw_from_monitor(&monitor, true);
        assert_eq!(raw.name.as_deref(), Some("DELL U2720Q"));
        assert_eq!(raw.physical_width, 3840);
        assert_eq!(raw.physical_height, 2160);
        assert!(raw.primary);
        assert_eq!(raw.scale_factor, 1.5);
        // And its identity comes out as the documented scheme.
        let discovered = identify(std::slice::from_ref(&raw));
        assert_eq!(discovered[0].identity.as_str(), "DELL U2720Q@3840x2160");
    }

    use super::super::bridge_profile::{
        BridgeProfile, DisplayEntry, PROFILE_VERSION, ROLE_VIEWSCREEN,
    };

    fn dell_raw() -> RawMonitor {
        RawMonitor {
            name: Some("DELL U2720Q".to_string()),
            physical_width: 3840,
            physical_height: 2160,
            position_x: 0,
            position_y: 0,
            scale_factor: 1.0,
            primary: true,
        }
    }

    fn viewscreen_profile(id: &str) -> BridgeProfile {
        BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![DisplayEntry {
                id: id.to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            }],
            touch: Vec::new(),
            media: Vec::new(),
        }
    }

    #[test]
    fn setup_exit_is_clean_with_no_profile() {
        assert!(setup_profile_is_clean(None, &[]));
    }

    #[test]
    fn setup_exit_is_clean_with_a_matching_profile() {
        let discovered = identify(&[dell_raw()]);
        let profile = viewscreen_profile("DELL U2720Q@3840x2160");
        assert!(setup_profile_is_clean(Some(&profile), &discovered));
    }

    #[test]
    fn setup_exit_is_dirty_for_an_invalid_profile() {
        // `from_toml` only parses; a bad schema version (or an unknown role, or
        // a Station of three panes) must fail `--setup`'s exit code exactly as
        // it fails the authoritative `--world` path at the prompt.
        let mut profile = viewscreen_profile("DELL U2720Q@3840x2160");
        profile.version = PROFILE_VERSION + 1;
        assert!(!setup_profile_is_clean(Some(&profile), &[]));
    }

    #[test]
    fn setup_exit_is_dirty_when_the_profile_does_not_match_the_connected_displays() {
        // The profile validates fine on its own, but the monitor it assigns is
        // not actually connected — exactly what `--setup --profile` exists to
        // catch, so it must not report success.
        let profile = viewscreen_profile("DELL U2720Q@3840x2160");
        assert!(!setup_profile_is_clean(Some(&profile), &[]));
    }

    // ── runtime display loss watcher (issue #1125) ──────────────────────────

    /// A Bevy [`Monitor`] component as winit would report it. Built by hand — no
    /// display needed — so the runtime-loss watcher can be driven in CI by
    /// spawning and despawning these, which is exactly what `bevy_winit` does to
    /// the entities when a monitor is plugged in or unplugged.
    fn monitor(name: &str, w: u32, h: u32, x: i32, y: i32) -> Monitor {
        Monitor {
            name: Some(name.to_string()),
            physical_width: w,
            physical_height: h,
            physical_position: IVec2::new(x, y),
            refresh_rate_millihertz: Some(60_000),
            scale_factor: 1.0,
            video_modes: Vec::new(),
        }
    }

    /// A validated profile: the Dell is the viewscreen (primary), the BenQ a
    /// Station carrying a pane for `station_label`.
    fn viewscreen_and_station(station_label: &str) -> ValidatedProfile {
        use super::super::bridge_profile::{PaneSlot, ROLE_STATION};
        BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                DisplayEntry {
                    id: "DELL U2720Q@3840x2160".to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: "BenQ EX@1920x1080".to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_participant(station_label)],
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn losing_a_station_monitor_at_runtime_closes_the_pane_it_carried() {
        // The runtime extension of the #1123 missing-display report, tested
        // against the ACTUAL adapter system rather than only its pure core: a
        // Station monitor that was present and driving a pane is unplugged
        // mid-run (its `Monitor` entity despawned, as bevy_winit does), and the
        // watcher closes the pane it carried — the ordinary dropped-participant
        // path, its station flipping to Backfill. No physical display is
        // involved: the fake monitors ARE the hardware here.
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;
        use crate::native_host::transport::NativeTransport;

        let mut app = App::new();
        app.insert_resource(BridgeDisplayConfig {
            profile: viewscreen_and_station("Ada"),
            authored: true,
        });
        let bus = PaneBus::default();
        let pane =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        bus.mark_live(pane);
        let token = bus.token_of(pane).unwrap();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.add_systems(Update, watch_runtime_displays);

        let _dell = app
            .world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor))
            .id();
        let benq = app
            .world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0))
            .id();

        // First frame establishes the baseline; nothing is lost yet.
        app.update();
        assert_eq!(bus.open_count(), 1, "the baseline frame closes no pane");

        // The Station monitor is unplugged. The loss is debounced, so one missing
        // frame is a blip and closes nothing.
        app.world_mut().entity_mut(benq).despawn();
        app.update();
        assert_eq!(
            bus.open_count(),
            1,
            "one missing frame is a winit blip, not a disconnect"
        );

        // Only a loss that persists the whole debounce window is believed.
        for _ in 1..DISPLAY_LOSS_DEBOUNCE_FRAMES {
            app.update();
        }
        assert_eq!(
            bus.open_count(),
            0,
            "the pane on the persistently-lost Station monitor is closed"
        );
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token }],
            "and the lobby is owed exactly the disconnect a dropped phone would produce"
        );
        // A display loss never recreates — that would be a silent re-home.
        assert!(bus.take_pending_views().is_empty());
    }

    #[test]
    fn losing_the_viewscreen_monitor_at_runtime_closes_no_pane() {
        // The viewscreen carries no participant, so unplugging it leaves the
        // shared 3-D view nowhere to draw and touches no station — the mission
        // and every pane carry on.
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let mut app = App::new();
        app.insert_resource(BridgeDisplayConfig {
            profile: viewscreen_and_station("Ada"),
            authored: true,
        });
        let bus = PaneBus::default();
        let pane =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        bus.mark_live(pane);
        app.insert_resource(PaneBusResource(bus.clone()));
        app.add_systems(Update, watch_runtime_displays);

        let dell = app
            .world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor))
            .id();
        let _benq = app
            .world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0))
            .id();
        app.update();
        app.world_mut().entity_mut(dell).despawn();
        // Past the debounce window, so the viewscreen loss is confirmed — and
        // still closes no pane, because the viewscreen carries no participant.
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
            app.update();
        }

        assert_eq!(
            bus.open_count(),
            1,
            "losing the viewscreen touches no station's pane"
        );
    }

    /// A validated profile of two IDENTICAL Station monitors, disambiguated by
    /// position, each carrying its own participant pane.
    fn two_identical_stations() -> ValidatedProfile {
        use super::super::bridge_profile::{PaneSlot, ROLE_STATION};
        BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                // A viewscreen this fixture never connects: a profile that
                // assigns monitors must name one (issue #1327), and a monitor
                // that is never present is simply never a runtime loss — which is
                // what keeps this a test about the two identical Stations.
                DisplayEntry {
                    id: "DELL U2720Q@3840x2160".to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: "ACME 1080@1920x1080#0,0".to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_participant("Ada")],
                },
                DisplayEntry {
                    id: "ACME 1080@1920x1080#1920,0".to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_participant("Grace")],
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn losing_one_of_two_identical_monitors_closes_only_its_own_pane() {
        // The finding-1 regression, at the adapter: two identical monitors are
        // told apart only by a `#x,y` suffix, so when one leaves the survivor's
        // live-computed identity must NOT shift and read as lost too. Exactly one
        // pane — the removed monitor's — closes; the survivor's stays up.
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;
        use crate::native_host::transport::NativeTransport;

        let mut app = App::new();
        app.insert_resource(BridgeDisplayConfig {
            profile: two_identical_stations(),
            authored: true,
        });
        let bus = PaneBus::default();
        let ada =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        bus.mark_live(ada);
        let ada_token = bus.token_of(ada).unwrap();
        let grace =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000002", "Grace").unwrap());
        bus.mark_live(grace);
        app.insert_resource(PaneBusResource(bus.clone()));
        app.add_systems(Update, watch_runtime_displays);

        // Ada sits at (0,0), Grace at (1920,0) — same model, same mode.
        let ada_monitor = app
            .world_mut()
            .spawn((monitor("ACME 1080", 1920, 1080, 0, 0), PrimaryMonitor))
            .id();
        let _grace_monitor = app
            .world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 1920, 0))
            .id();

        app.update();
        assert_eq!(bus.open_count(), 2, "the baseline frame closes no pane");

        // Ada's monitor is unplugged; Grace's stays exactly where it was.
        app.world_mut().entity_mut(ada_monitor).despawn();
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES {
            app.update();
        }

        assert_eq!(
            bus.open_count(),
            1,
            "only the lost monitor's pane closes — the survivor's stays up"
        );
        assert!(
            bus.open_pane_for_name("Grace").is_some(),
            "Grace's pane on the surviving twin is untouched"
        );
        assert!(
            bus.open_pane_for_name("Ada").is_none(),
            "Ada's pane on the removed twin is closed"
        );
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token: ada_token }],
            "exactly Ada's token disconnects — Grace's never does"
        );
    }

    // ── the apply-on-change viewscreen (issue #1330) ────────────────────────
    //
    // Every one of these runs the REAL plugin against injected `Monitor`
    // entities — the same fake-hardware path the #1125 tests above use. What a
    // real display adds is only the pixels; which window is placed where is
    // decided entirely by what follows.

    const DELL: &str = "DELL U2720Q@3840x2160";
    const BENQ: &str = "BenQ EX@1920x1080";
    /// A third screen, for the tests that need somewhere the viewscreen can go
    /// that is neither the primary nor the one about to be unplugged.
    const ACME: &str = "ACME 1080@1920x1080";

    /// A host with the plugin, a primary window and two monitors, one frame in.
    /// `profile` is an operator's `--profile`; `None` is a plain
    /// `phoenix-host --world …`.
    fn booted(profile: Option<ValidatedProfile>) -> (App, Entity) {
        booted_on(
            profile,
            vec![
                (monitor("DELL U2720Q", 3840, 2160, 0, 0), true),
                (monitor("BenQ EX", 1920, 1080, 3840, 0), false),
            ],
        )
    }

    /// [`booted`] over a monitor set of the test's own choosing — `(monitor,
    /// is_primary)`, in whatever order, exactly as `bevy_winit` would have
    /// spawned them.
    fn booted_on(
        profile: Option<ValidatedProfile>,
        monitors: Vec<(Monitor, bool)>,
    ) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        if let Some(profile) = profile {
            app.insert_resource(BridgeDisplayConfig {
                profile,
                authored: true,
            });
        }
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        for (m, primary) in monitors {
            let mut entity = app.world_mut().spawn(m);
            if primary {
                entity.insert(PrimaryMonitor);
            }
        }
        app.update();
        (app, window)
    }

    /// The entity of the monitor the OS named `name` and put at `x` — so a test
    /// can unplug or re-mode exactly the display it means, the way `bevy_winit`
    /// does. Position as well as name because a twin test has two of each name.
    fn monitor_entity(app: &mut App, name: &str, x: i32) -> Entity {
        let mut found = None;
        let mut query = app.world_mut().query::<(Entity, &Monitor)>();
        for (entity, m) in query.iter(app.world()) {
            if m.name.as_deref() == Some(name) && m.physical_position.x == x {
                found = Some(entity);
            }
        }
        found.expect("the monitor this test named is there")
    }

    /// The entity of the second monitor, so a test can unplug it the way
    /// `bevy_winit` does.
    fn benq_entity(app: &mut App) -> Entity {
        monitor_entity(app, "BenQ EX", 3840)
    }

    /// The lobby's monitor row, as the surface would receive it this frame.
    fn row(app: &App) -> super::super::host_lobby::layout::BridgeLayoutPayload {
        let live = app.world().resource::<BridgeLayoutResource>();
        super::super::host_lobby::bridge_layout_payload(&live.layout, &live.monitors, &live.notices)
    }

    /// The window mode of the process's primary window.
    fn window_mode(app: &App, window: Entity) -> WindowMode {
        app.world().entity(window).get::<Window>().unwrap().mode
    }

    /// Run the whole settle window, so a roster change is believed.
    fn settle(app: &mut App) {
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
            app.update();
        }
    }

    fn viewscreen_identity(app: &App) -> String {
        app.world()
            .resource::<BridgeLayoutResource>()
            .layout
            .viewscreen()
            .as_str()
            .to_string()
    }

    /// Move the live layout's viewscreen, as a lobby button press does.
    fn choose(app: &mut App, identity: &str) {
        let moved = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new(identity),
            })
            .expect("a free monitor takes the viewscreen");
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .layout = moved;
    }

    /// Open a station's console on a monitor, as a lobby button press does.
    fn seat(app: &mut App, station: &str, identity: &str) {
        let placed = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::AssignStation {
                station: crate::core::messages::StationId(station.to_string()),
                monitor: MonitorIdentity::new(identity),
            })
            .expect("a free monitor takes a console");
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .layout = placed;
    }

    #[test]
    fn a_host_with_no_profile_gains_a_config_and_a_layout_and_keeps_its_window() {
        // The whole of issue #1330's second acceptance criterion. The applier
        // and the watcher now run on a host that was given no display arguments
        // at all — which is what the monitor row needs — and the window that
        // host opens is the one #1121 opened.
        let (app, window) = booted(None);

        assert!(
            !app.world().resource::<BridgeDisplayConfig>().authored,
            "a synthesised config describes the displays; it does not instruct"
        );
        let layout = &app.world().resource::<BridgeLayoutResource>().layout;
        assert_eq!(layout.monitors().len(), 2);
        assert_eq!(layout.viewscreen().as_str(), DELL, "the OS primary");

        let placed = app.world().entity(window);
        assert!(
            matches!(placed.get::<Window>().unwrap().mode, WindowMode::Windowed),
            "no lobby action, so the window is exactly where the OS opened it"
        );
        assert!(
            placed.get::<BridgeSurface>().is_none(),
            "and it is not tagged as a placed bridge surface either"
        );
        assert!(
            app.world().resource::<BridgeStationSurfaces>().0.is_empty(),
            "a layout with no seated console opens no Station window"
        );
        assert_eq!(
            app.world()
                .resource::<BridgeDisplayApplied>()
                .viewscreen
                .as_ref()
                .map(|m| m.as_str()),
            Some(DELL),
            "the baseline is the SEEDED viewscreen, which is what leaves the follower idle"
        );
    }

    #[test]
    fn a_host_with_no_profile_still_never_touches_its_window_on_later_frames() {
        // The follower runs every frame forever. "Apply-on-change" has to mean
        // it does nothing on all of them, not that it settles down eventually.
        let (mut app, window) = booted(None);
        for _ in 0..20 {
            app.update();
        }
        assert!(matches!(
            app.world().entity(window).get::<Window>().unwrap().mode,
            WindowMode::Windowed
        ));
    }

    #[test]
    fn an_authored_profile_still_places_the_viewscreen_at_boot() {
        // Issue #1123, unchanged: `--profile` is an instruction, and it wins at
        // boot — seeding the layout the lobby then edits.
        let (app, window) = booted(Some(viewscreen_and_station("Ada")));

        let placed = app.world().entity(window);
        assert!(matches!(
            placed.get::<Window>().unwrap().mode,
            WindowMode::BorderlessFullscreen(_)
        ));
        assert_eq!(placed.get::<BridgeSurface>().unwrap().identity, DELL);
        assert_eq!(
            app.world().resource::<BridgeStationSurfaces>().0.len(),
            1,
            "the profile's Station monitor still gets its own window"
        );
        assert_eq!(viewscreen_identity(&app), DELL);
    }

    #[test]
    fn choosing_another_monitor_moves_the_viewscreen_window_live() {
        // The lobby's monitor row, at the model boundary: the press itself is
        // `host_lobby::drain_surface_records`, and what it does is exactly this
        // — one lawful transition on the live layout. No restart.
        let (mut app, window) = booted(None);
        choose(&mut app, BENQ);
        app.update();

        let placed = app.world().entity(window);
        assert!(
            matches!(
                placed.get::<Window>().unwrap().mode,
                WindowMode::BorderlessFullscreen(_)
            ),
            "the viewscreen window moves onto the chosen display"
        );
        assert_eq!(placed.get::<BridgeSurface>().unwrap().identity, BENQ);
        assert_eq!(
            app.world()
                .resource::<BridgeDisplayApplied>()
                .viewscreen
                .as_ref()
                .map(|m| m.as_str()),
            Some(BENQ)
        );

        // …and having moved once, it does not keep moving.
        let before = app.world().entity(window).get::<Window>().unwrap().mode;
        app.update();
        assert_eq!(
            app.world().entity(window).get::<Window>().unwrap().mode,
            before
        );
    }

    #[test]
    fn unplugging_a_monitor_rebuilds_the_layout_the_row_is_drawn_from() {
        // Issue #1330's third acceptance criterion, headlessly: the roster the
        // lobby shows follows the cable, through the watcher #1125 built.
        let (mut app, _) = booted(None);
        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();

        app.update();
        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitors()
                .len(),
            2,
            "one missing frame is a winit blip, not an unplug"
        );

        for _ in 1..DISPLAY_LOSS_DEBOUNCE_FRAMES {
            app.update();
        }
        let layout = app.world().resource::<BridgeLayoutResource>();
        assert_eq!(layout.layout.monitors().len(), 1);
        assert_eq!(layout.monitors.len(), 1, "and the row's geometry with it");
        assert!(
            layout.notices.is_empty(),
            "losing a monitor nothing was on degrades nothing, so there is nothing to say"
        );
    }

    #[test]
    fn a_monitor_plugged_in_joins_the_row_once_the_roster_settles() {
        // The other direction, and the reason the settle is on the ROSTER
        // rather than only on losses: a display that arrives has to appear as a
        // button, or the operator cannot choose it.
        let (mut app, _) = booted(None);
        app.world_mut()
            .spawn(monitor("Acer VG", 1280, 1024, 5760, 0));
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES {
            app.update();
        }
        let layout = app.world().resource::<BridgeLayoutResource>();
        assert_eq!(layout.layout.monitors().len(), 3);
        assert_eq!(
            layout.layout.viewscreen().as_str(),
            DELL,
            "a new screen is no reason to overrule where the viewscreen is"
        );
    }

    #[test]
    fn unplugging_the_chosen_viewscreen_falls_back_visibly_and_the_window_follows() {
        // The degradation that would otherwise look like nothing happening: a
        // working viewscreen on another screen. The operator is told, in a
        // sentence the row can render, and the window goes where the note says.
        let (mut app, window) = booted(None);
        choose(&mut app, BENQ);
        app.update();
        assert_eq!(viewscreen_identity(&app), BENQ);

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
            app.update();
        }

        assert_eq!(viewscreen_identity(&app), DELL, "primary-else-first");
        let notices = &app.world().resource::<BridgeLayoutResource>().notices;
        assert_eq!(notices.len(), 1);
        let LayoutNotice::Adopted(note) = &notices[0] else {
            panic!("a roster change reports an adoption note: {notices:?}");
        };
        assert_eq!(
            note.string_id(),
            "server.bridge_layout.adopt_viewscreen_gone",
            "the fallback is reported, not silent"
        );
        assert_eq!(
            app.world()
                .entity(window)
                .get::<BridgeSurface>()
                .unwrap()
                .identity,
            DELL,
            "and the window followed the layout onto the surviving display"
        );
    }

    // ── the window moves only when somebody asks (issue #1330) ──────────────
    //
    // `identify`'s answer depends on the set it is given, and the layout stores
    // its answer. Three ordinary events therefore used to read as "the monitor
    // the viewscreen is on has gone": a twin arriving, that twin leaving, and a
    // display renegotiating its mode. Each one fired ViewscreenMonitorGone,
    // discarded the operator's choice and slammed the primary window into
    // borderless fullscreen on a host nobody had touched. These three are the
    // proof that it does not, and they fail on the commit before this one.

    /// Neither the window nor the viewscreen moved, and nothing was reported.
    fn nothing_moved(app: &App, window: Entity, viewscreen: &str) {
        assert_eq!(
            viewscreen_identity(app),
            viewscreen,
            "the viewscreen stayed on the display it was on"
        );
        let notices = &app.world().resource::<BridgeLayoutResource>().notices;
        assert!(
            notices.is_empty(),
            "nothing degraded, so there is nothing to report: {notices:?}"
        );
        assert!(
            matches!(window_mode(app, window), WindowMode::Windowed),
            "and no lobby press happened, so the window is where the OS opened it"
        );
    }

    #[test]
    fn plugging_in_an_identical_twin_leaves_the_viewscreens_own_display_alone() {
        // The survivor of a collision keeps the key it was known by. Without
        // that, BOTH twins become `…#x,y` the moment the second one arrives,
        // the layout's short-key viewscreen matches neither, and the shared
        // view is dragged onto a display nobody chose.
        let (mut app, window) = booted_on(
            None,
            vec![
                (monitor("ACME 1080", 1920, 1080, 0, 0), true),
                (monitor("BenQ EX", 1920, 1080, 1920, 0), false),
            ],
        );
        assert_eq!(viewscreen_identity(&app), "ACME 1080@1920x1080");

        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 3840, 0));
        settle(&mut app);

        nothing_moved(&app, window, "ACME 1080@1920x1080");
        let layout = app.world().resource::<BridgeLayoutResource>();
        assert_eq!(
            layout
                .layout
                .monitors()
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>(),
            vec![
                "ACME 1080@1920x1080",
                "BenQ EX@1920x1080",
                // Only the NEWCOMER pays the disambiguator: it is the one
                // nothing was known about.
                "ACME 1080@1920x1080#3840,0",
            ],
            "the display that was already there kept its key; the new one joined"
        );
        let row = row(&app);
        assert_eq!(row.monitors.len(), 3, "and every one of them has a button");
        assert!(row.monitors[0].viewscreen);
    }

    #[test]
    fn unplugging_one_identical_twin_leaves_the_survivor_where_it_was() {
        // The mirror, and the one that reaches a live crew: the viewscreen is
        // on a display whose identity is only suffixed BECAUSE its twin is
        // there. When the twin leaves, re-deriving would collapse the survivor
        // to the short key — so the layout would decide its own viewscreen had
        // been unplugged while the operator was looking at it.
        let (mut app, window) = booted_on(
            None,
            vec![
                (monitor("ACME 1080", 1920, 1080, 0, 0), true),
                (monitor("ACME 1080", 1920, 1080, 1920, 0), false),
            ],
        );
        assert_eq!(viewscreen_identity(&app), "ACME 1080@1920x1080#0,0");

        let twin = monitor_entity(&mut app, "ACME 1080", 1920);
        app.world_mut().entity_mut(twin).despawn();
        settle(&mut app);

        nothing_moved(&app, window, "ACME 1080@1920x1080#0,0");
        let layout = app.world().resource::<BridgeLayoutResource>();
        assert_eq!(layout.layout.monitors().len(), 1, "the twin did leave");
        let row = row(&app);
        assert_eq!(row.monitors.len(), 1, "and the survivor still has a button");
        assert_eq!(
            row.monitors[0].identity, "ACME 1080@1920x1080#0,0",
            "carrying the identity the press round-trips on, which is exact string equality"
        );
        assert!(row.monitors[0].viewscreen);
    }

    #[test]
    fn a_display_that_renegotiates_its_resolution_is_still_the_same_display() {
        // A television waking, or an EDID handshake settling, rewrites the
        // `WxH` half of an identity outright. It is plainly the same screen in
        // the same place, and treating it as a new one threw away whichever
        // display the operator had chosen.
        let (mut app, window) = booted(None);
        assert_eq!(viewscreen_identity(&app), DELL);

        let dell = monitor_entity(&mut app, "DELL U2720Q", 0);
        {
            let mut m = app.world_mut().entity_mut(dell);
            let mut m = m.get_mut::<Monitor>().unwrap();
            m.physical_width = 1920;
            m.physical_height = 1080;
        }
        settle(&mut app);

        nothing_moved(&app, window, DELL);
        let row = row(&app);
        assert_eq!(row.monitors.len(), 2);
        assert_eq!(
            row.monitors[0].identity, DELL,
            "the key it has been known by all session"
        );
        assert_eq!(
            (row.monitors[0].width, row.monitors[0].height),
            (1920, 1080),
            "while the row shows the size it is ACTUALLY running at"
        );
    }

    // ── consoles opened and closed while the host runs (issue #1331) ────────
    //
    // Everything below drives the REAL plugin against injected `Monitor`
    // entities, exactly as the #1125 and #1330 tests above do — so the
    // open/close transitions, which are the whole of this slice, are checked by
    // the ordinary `cargo test` runs rather than only on a machine with three
    // screens. What a real display adds is the pixels.

    fn station(id: &str) -> crate::core::messages::StationId {
        crate::core::messages::StationId(id.to_string())
    }

    /// A three-screen host with a two-station hull, one frame in: a pane bus, a
    /// primary window, and the plugin. The shape a `phoenix-host --client-dir …`
    /// with no `--profile` boots into.
    fn console_host() -> (App, crate::native_host::panes::transport::PaneBus) {
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        let bus = PaneBus::default();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"

                [[station]]
                id = "weapons"
                name = "Tactical"
                description = "-"
                rank = "Crew"
                "#,
            )
            .expect("a two-station hull parses"),
        ));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 5760, 0));
        app.update();
        (app, bus)
    }

    /// Close a station's console, as the row's off button does.
    fn unseat(app: &mut App, station_id: &str) {
        let closed = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::UnassignStation {
                station: station(station_id),
            })
            .expect("closing a console the roster has is always lawful");
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .layout = closed;
    }

    /// The Station surfaces open right now, by monitor identity.
    fn surfaces(app: &App) -> Vec<String> {
        app.world()
            .resource::<BridgeStationSurfaces>()
            .0
            .iter()
            .map(|s| s.identity.clone())
            .collect()
    }

    #[test]
    fn a_profile_that_seats_a_station_gets_a_console_and_not_just_a_window() {
        // The case a diff would have missed. An authored `--profile` may seat a
        // STATION as well as a participant (`PaneSlot::for_station`), and its
        // Station window and its pane slot exist from boot — so "open the
        // consoles the layout has just gained" finds nothing new and leaves the
        // operator looking at an empty borderless-fullscreen screen. Asking the
        // BUS which seated stations have no console is what closes it.
        use crate::native_host::bridge_profile::{PaneSlot, ROLE_STATION};
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let profile = BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                DisplayEntry {
                    id: DELL.to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: BENQ.to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_station("helm")],
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .unwrap();

        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        let bus = PaneBus::default();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.insert_resource(BridgeDisplayConfig {
            profile,
            authored: true,
        });
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"
                "#,
            )
            .unwrap(),
        ));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.update();

        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .map(|m| m.as_str()),
            Some(BENQ),
            "the profile's seat was adopted by the law"
        );
        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
        assert_eq!(
            bus.open_count(),
            1,
            "and the boot pass opened its console, so the screen is not merely lit"
        );
        assert!(bus.open_pane_for_name("helm").is_some());
    }

    #[test]
    fn a_station_record_from_the_surface_moves_the_law_and_opens_the_console() {
        // The whole page->host path for issue #1331's two verbs, headless and
        // end to end: the JSON `host_lobby_link.js` actually sends, over the ONE
        // record queue and through the ONE drain issue #1328 left behind, out to
        // the layout law, and on into the console the display layer opens for it.
        //
        // Every other console test in this file seats through `LayoutAction`
        // directly, which is the right altitude for what they claim. This is the
        // one that pins the RECORD — because the fold of #1331's station verbs
        // into `HostLobbyRecord` is exactly where a tag, a drain arm or an
        // action constructor could be wrong with nothing else failing.
        use crate::native_host::host_lobby::{
            drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
        };
        use crate::native_host::panes::RecordingSurface;

        let (mut app, bus) = console_host();
        let bridge = HostLobbyBridge::new();
        app.insert_resource(HostLobbyBridgeResource(bridge.clone()));
        // The drain writes the operator's picks onto this bus; nothing here
        // sends one, but the parameter is validated when the system runs.
        app.add_message::<crate::lobby::InboundMessage>();
        // Where `HostLobbyPlugin` puts it, so the layout this frame's press
        // moves is the layout the followers read in the SAME frame.
        app.add_systems(PreUpdate, drain_surface_records);
        assert_eq!(bus.open_count(), 0, "nothing is pre-opened");

        let mut surface = RecordingSurface::ready();
        surface.queue_record(
            r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#,
        );
        pump_host_lobby(&bridge, &mut surface);
        app.update();

        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .map(|m| m.as_str()),
            Some(BENQ),
            "the record moved the law"
        );
        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
        assert_eq!(bus.open_count(), 1, "and the console opened for it");
        assert!(bus.open_pane_for_name("helm").is_some());

        // …and the row's off button closes it again, over the same queue, the
        // same drain arm and the same law.
        surface.queue_record(r#"{"kind":"unassign-station","station":"helm"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();

        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .monitor_of(&station("helm"))
            .is_none());
        assert_eq!(bus.open_count(), 0, "the console closed");
        assert!(surfaces(&app).is_empty(), "and the screen was given back");
    }

    #[test]
    fn seating_a_station_opens_a_station_window_and_a_console_on_it() {
        // The acceptance criterion, headlessly: a press seats the station, and
        // the follower opens the window and the pane — at RUNTIME, frames after
        // boot, with nothing pre-opened.
        let (mut app, bus) = console_host();
        assert!(
            surfaces(&app).is_empty(),
            "nothing is pre-opened: a bridge nobody has arranged has no Station window"
        );
        assert_eq!(bus.open_count(), 0);

        seat(&mut app, "helm", BENQ);
        app.update();

        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
        assert_eq!(
            bus.open_count(),
            1,
            "and a console pane opened for it, which is what reaches the client join flow"
        );
        let pane = bus
            .open_pane_for_name("helm")
            .expect("the pane is named for its station, which is the shared key");
        let token = bus.token_of(pane).expect("an ordinary session token");
        assert!(
            !crate::lobby::handler::is_reserved_token(&token),
            "an ordinary participant: admission cannot tell it from a phone"
        );

        // The pane's slot is the whole monitor, which is what the pane host
        // composites it into.
        let surface = app.world().resource::<BridgeStationSurfaces>();
        let (_, slot) = surface.slot_for("helm").expect("the console has a home");
        assert_eq!(
            (slot.rect.width, slot.rect.height),
            (1920, 1080),
            "one console is the whole screen"
        );
        assert_eq!(slot.station.as_ref(), Some(&station("helm")));
    }

    #[test]
    fn unassigning_closes_the_console_frees_the_screen_and_shuts_its_window() {
        // The off button: the pane closes through the ordinary dropped-phone
        // path (its station falls back to AI control), the Station window goes,
        // and the monitor reads as free everywhere.
        use crate::native_host::transport::NativeTransport;

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        bus.mark_live(pane);
        let token = bus.token_of(pane).unwrap();
        let window = app.world().resource::<BridgeStationSurfaces>().0[0].window;
        // Drain the view the open queued, as the pane host does once a frame,
        // so what is left below is only what the CLOSE queued.
        assert_eq!(bus.take_pending_views().len(), 1);

        unseat(&mut app, "helm");
        app.update();

        assert_eq!(bus.open_count(), 0, "the console closed");
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token }],
            "and the lobby is owed exactly the disconnect a dropped phone would produce"
        );
        assert!(
            surfaces(&app).is_empty(),
            "the Station surface went with it"
        );
        assert!(
            bus.take_pending_views().is_empty(),
            "and nothing was queued to rebuild it: this was a close, not a fault"
        );

        // The window is despawned a frame later, so the pane host has a frame to
        // tear down the view and the Station camera that were rendering to it.
        app.update();
        assert!(
            app.world().get_entity(window).is_err(),
            "the Station window is closed and the screen is free again"
        );

        // Occupancy updates everywhere: the monitor row draws no occupant, and
        // the viewscreen may now move onto the screen the console had.
        let row = row(&app);
        assert!(row.monitors[1].stations.is_empty());
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new(BENQ),
            })
            .is_ok());
    }

    #[test]
    fn moving_a_console_to_another_screen_keeps_whoever_claimed_it() {
        // Assigning a seated station elsewhere IS the move (the law has no move
        // action). A view is built at one size on one window, so the move is a
        // REBUILD — but on the same identity, through the #1125 recreate path,
        // so the page's `Identify` is a reconnect the lobby answers by restoring
        // the held station rather than a stranger arriving.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        let token = bus.token_of(pane).unwrap();
        bus.mark_live(pane);
        bus.take_pending_views();

        seat(&mut app, "helm", ACME);
        app.update();

        assert_eq!(surfaces(&app), vec![ACME.to_string()]);
        assert_eq!(bus.open_count(), 1, "one console, not two");
        let moved = bus
            .open_pane_for_name("helm")
            .expect("the console is still open");
        assert_ne!(moved, pane, "a rebuilt view is a new handle");
        assert_eq!(
            bus.token_of(moved).as_deref(),
            Some(token.as_str()),
            "on the SAME session token, so whoever claimed it keeps it across the move"
        );
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![moved],
            "and the pane host is asked to build its view — against the surfaces this pass \
             rewrote, so it lands on the screen the operator chose"
        );
    }

    #[test]
    fn two_consoles_on_one_screen_divide_it_side_by_side() {
        // The law caps a screen at two, and `surface_rects` is what turns that
        // into geometry. The 2-up polish is issue #1332's; the tiling is free
        // here and refusing to draw it would be a rule this module invented.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        seat(&mut app, "weapons", BENQ);
        app.update();

        assert_eq!(
            surfaces(&app),
            vec![BENQ.to_string()],
            "one window, two panes"
        );
        assert_eq!(bus.open_count(), 2);
        let surface = app.world().resource::<BridgeStationSurfaces>();
        let (_, helm) = surface.slot_for("helm").unwrap();
        let (_, weapons) = surface.slot_for("weapons").unwrap();
        assert_eq!((helm.rect.x, helm.rect.width), (0, 960));
        assert_eq!((weapons.rect.x, weapons.rect.width), (960, 960));
    }

    #[test]
    fn seating_a_second_console_beside_one_rebuilds_the_first_at_its_new_half() {
        // The operator's two presses, a moment apart — the ordinary way a screen
        // comes to hold two. The console that was already there had a view built
        // for the whole screen, so it is rebuilt at its half rather than left
        // overlapping the newcomer.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let helm_token = bus
            .token_of(bus.open_pane_for_name("helm").unwrap())
            .unwrap();
        bus.take_pending_views();

        seat(&mut app, "weapons", BENQ);
        app.update();

        let surface = app.world().resource::<BridgeStationSurfaces>();
        assert_eq!(surface.slot_for("helm").unwrap().1.rect.width, 960);
        assert_eq!(surface.slot_for("weapons").unwrap().1.rect.x, 960);
        assert_eq!(bus.open_count(), 2);
        assert_eq!(
            bus.token_of(bus.open_pane_for_name("helm").unwrap())
                .as_deref(),
            Some(helm_token.as_str()),
            "the first console keeps its identity: its view moved, nobody was dropped"
        );
        assert_eq!(
            bus.take_pending_views().len(),
            2,
            "one view to build for the newcomer and one to rebuild for the console beside it"
        );
    }

    #[test]
    fn closing_one_of_two_consoles_gives_the_other_the_whole_screen() {
        // The re-layout the move above only hinted at: a monitor's rectangles
        // are recomputed for the WHOLE monitor, so the survivor grows rather
        // than staying in its half beside a black one — and its VIEW is rebuilt
        // at the new size, on the same identity, because a view is created at
        // one size and cannot be resized into place.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        seat(&mut app, "weapons", BENQ);
        app.update();
        let helm_pane = bus.open_pane_for_name("helm").unwrap();
        let helm_token = bus.token_of(helm_pane).unwrap();
        bus.mark_live(helm_pane);
        bus.take_pending_views();

        unseat(&mut app, "weapons");
        app.update();

        assert_eq!(bus.open_count(), 1);
        let surface = app.world().resource::<BridgeStationSurfaces>();
        let (_, helm) = surface.slot_for("helm").unwrap();
        assert_eq!((helm.rect.x, helm.rect.width), (0, 1920));
        let grown = bus.open_pane_for_name("helm").unwrap();
        assert_eq!(
            bus.token_of(grown).as_deref(),
            Some(helm_token.as_str()),
            "the survivor's view is rebuilt at its new size, and whoever was at it stays"
        );
        assert_eq!(bus.take_pending_views().len(), 1);
    }

    #[test]
    fn a_bridge_nobody_rearranged_opens_and_closes_nothing_forever() {
        // Apply-on-change has to mean the follower does nothing on every frame,
        // not that it settles down eventually — it runs for the life of the
        // process on every windowed host.
        let (mut app, bus) = console_host();
        for _ in 0..20 {
            app.update();
        }
        assert!(surfaces(&app).is_empty());
        assert_eq!(bus.open_count(), 0);
    }

    // ── an unplugged screen, and the two halves of the #1330 tripwire ───────
    //
    // Issue #1330 left the no-`--profile` host's unplug-closes-nothing holding
    // by accident: the synthesised `BridgeDisplayConfig` is the BOOT layout, so
    // it carries no pane labels and `watch_runtime_displays` resolves every loss
    // to an empty one. #1331 settled that it STAYS the boot layout — see the
    // note in `apply_bridge_profile` — and that a lobby-opened console is closed
    // by the LAW instead: the reconcile unseats it and `follow_layout_stations`
    // closes it. These two are that decision, split into its two claims.

    #[test]
    fn unplugging_a_monitor_with_no_console_on_it_closes_no_pane() {
        // Half one: the watcher still closes nothing of its own. The pane on the
        // bus here is a `--pane`-shaped one the layout does not own — nothing
        // seated it, nothing may close it — and the screen that is unplugged is
        // one the layout has no console on. A config rebuilt from the live
        // layout would start naming panes here, and this is what would go red.
        use crate::native_host::panes::identity::PaneIdentity;

        let (mut app, bus) = console_host();
        let stray =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        bus.mark_live(stray);
        // The operator has arranged the bridge — viewscreen moved, a console
        // open — so this is not the trivial host that would seat nothing under
        // any implementation. What it has NOT done is put a console on the BenQ.
        choose(&mut app, BENQ);
        seat(&mut app, "helm", ACME);
        app.update();
        assert!(!app.world().resource::<BridgeDisplayConfig>().authored);

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        settle(&mut app);

        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitors()
                .len(),
            2,
            "the unplug itself was believed"
        );
        assert_eq!(
            bus.open_count(),
            2,
            "and nobody's console closed: neither the stray pane nor the console on \
             the screen that is still plugged in"
        );
        assert!(bus.open_pane_for_name("Ada").is_some());
        assert!(bus.open_pane_for_name("helm").is_some());
    }

    #[test]
    fn unplugging_a_monitor_holding_a_runtime_console_closes_exactly_it() {
        // Half two, and the behaviour #1331 wants: a console open on the screen
        // that is unplugged closes — its token disconnects and its station falls
        // back to AI control, the #1125 display-loss semantics — while a console
        // on a screen that is still there does not.
        use crate::native_host::transport::NativeTransport;

        let (mut app, bus) = console_host();
        choose(&mut app, DELL);
        seat(&mut app, "helm", BENQ);
        seat(&mut app, "weapons", ACME);
        app.update();
        let helm_pane = bus.open_pane_for_name("helm").unwrap();
        bus.mark_live(helm_pane);
        let helm_token = bus.token_of(helm_pane).unwrap();
        bus.mark_live(bus.open_pane_for_name("weapons").unwrap());
        assert_eq!(bus.open_count(), 2);

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        settle(&mut app);

        let live = app.world().resource::<BridgeLayoutResource>();
        assert!(
            live.layout.monitor_of(&station("helm")).is_none(),
            "the law left helm's console unassigned, as it does for any lost screen"
        );
        assert_eq!(
            live.layout
                .monitor_of(&station("weapons"))
                .map(|m| m.as_str()),
            Some(ACME),
            "and weapons kept its seat on the screen that is still there"
        );
        assert_eq!(
            bus.open_count(),
            1,
            "exactly the console on the unplugged screen closed"
        );
        assert!(bus.open_pane_for_name("helm").is_none());
        assert!(bus.open_pane_for_name("weapons").is_some());
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token: helm_token }],
            "through the ordinary dropped-participant path, so its station goes to Backfill"
        );
        assert_eq!(
            surfaces(&app),
            vec![ACME.to_string()],
            "and the lost screen's Station surface went with the display"
        );
    }

    #[test]
    fn a_replugged_monitor_can_take_its_console_back_without_ending_the_mission() {
        // Story 27's half that a headless test can hold: the layout leaves a
        // console unassigned rather than re-homing it, and re-seating it on the
        // returned display opens a fresh console — a new participant on an
        // ordinary token, claiming through the normal flow. (The other half —
        // that the row is REACHABLE mid-mission — is the revealed surface's, in
        // `host_lobby::reveal`.)
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        settle(&mut app);
        assert_eq!(bus.open_count(), 0);

        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        settle(&mut app);
        assert!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .is_none(),
            "a returned display is never silently re-homed onto"
        );

        seat(&mut app, "helm", BENQ);
        app.update();
        assert_eq!(bus.open_count(), 1, "and an explicit press opens it again");
        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
    }

    // ── a surface is the LAW's to drop, not this frame's winit report ────────
    //
    // Every other reader of a lost display waits out the debounce window before
    // believing it. The surface sweep did not: it retained on the monitors winit
    // reported THIS frame, so a single absent frame despawned a live console's
    // Station window. It was masked by the apply-on-change gate, and any lobby
    // press unmasked it — `drain_surface_records` writes its notices on every
    // layout record, so every press marks the layout changed whether or not
    // anything moved.

    /// A lobby press that only *reports* — the state every refused press, and
    /// every press for something already true, leaves behind: nothing moved,
    /// and `BridgeLayoutResource` was written to all the same.
    fn press_that_only_reports(app: &mut App) {
        let notices = app
            .world()
            .resource::<BridgeLayoutResource>()
            .notices
            .clone();
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .notices = notices;
    }

    #[test]
    fn an_unplug_and_a_press_inside_the_settle_window_does_not_strand_a_console() {
        // The blocker, exactly as it happens: a console is open on the BenQ, the
        // BenQ is unplugged, and the operator presses ANY button before the
        // reconcile has believed the unplug.
        //
        // The surface must survive the blip — the law still names that monitor —
        // and the console with it. Then, when the reconcile does unseat it, the
        // console closes through the ordinary dropped-participant path. Before
        // this fix the surface was dropped on the press, and the close sweep,
        // driven by the surfaces it had just emptied, then had nothing to close:
        // the pane stayed open forever, its station never reached Backfill, and
        // the camera went on rendering to a despawned window.
        use crate::native_host::transport::NativeTransport;

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        bus.mark_live(pane);
        let token = bus.token_of(pane).unwrap();
        bus.take_pending_views();

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();

        // Inside the settle window, and pressing all the way through it.
        for _ in 1..DISPLAY_LOSS_DEBOUNCE_FRAMES {
            press_that_only_reports(&mut app);
            app.update();
            assert_eq!(
                surfaces(&app),
                vec![BENQ.to_string()],
                "the law still names that monitor, so its surface stands"
            );
            assert!(
                bus.open_pane_for_name("helm").is_some(),
                "and the console on it is untouched: a blip is not an unplug"
            );
            assert!(
                bus.transport().poll().is_empty(),
                "nobody has been disconnected yet"
            );
        }

        // The reconcile believes it, and NOW the console closes — once.
        settle(&mut app);
        assert!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .is_none(),
            "the law unseated it"
        );
        assert_eq!(bus.open_count(), 0, "and the adapter closed it");
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token }],
            "through the ordinary dropped-participant path, so its station goes to Backfill"
        );
        assert!(surfaces(&app).is_empty(), "and the surface went with it");
    }

    #[test]
    fn a_frame_reporting_no_monitors_at_all_closes_nothing() {
        // winit between hot-plug events, not a bridge that lost every display —
        // the judgement `watch_runtime_displays` already made on the same
        // observation, which the surface sweep did not. Acting on it would tear
        // down every Station window on the machine.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let window = app.world().resource::<BridgeStationSurfaces>().0[0].window;

        let monitors: Vec<Entity> = {
            let mut query = app.world_mut().query_filtered::<Entity, With<Monitor>>();
            query.iter(app.world()).collect()
        };
        for monitor in monitors {
            app.world_mut().entity_mut(monitor).despawn();
        }
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
            press_that_only_reports(&mut app);
            app.update();
        }

        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
        assert_eq!(bus.open_count(), 1);
        assert!(
            app.world().get_entity(window).is_ok(),
            "the Station window is still there, because nothing was reported to be gone"
        );
    }

    #[test]
    fn re_seating_after_a_stranded_unplug_gives_a_console_a_real_seat_again() {
        // The other end of the blocker. The stranded console left a pane open
        // under the station's own name, so the open sweep — which asks the bus
        // whether a seated station already has one — found it and opened
        // nothing, and no view was ever built for the new Station window: a
        // permanently black screen a press could not repair. With the console
        // closed honestly, a re-seat is an ordinary open with a real slot and a
        // view queued against it.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        bus.mark_live(bus.open_pane_for_name("helm").unwrap());
        bus.take_pending_views();

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        press_that_only_reports(&mut app);
        settle(&mut app);
        assert_eq!(bus.open_count(), 0, "the stranding is over");

        seat(&mut app, "helm", ACME);
        app.update();

        assert_eq!(surfaces(&app), vec![ACME.to_string()]);
        let reopened = bus
            .open_pane_for_name("helm")
            .expect("a seated station has a console");
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![reopened],
            "and a view is queued for it, which is what a LIVE console is"
        );
        let surface = app.world().resource::<BridgeStationSurfaces>();
        let (seat, slot) = surface
            .slot_for("helm")
            .expect("built against a real Station surface, not a black window");
        assert_eq!(seat.identity, ACME);
        assert_eq!((slot.rect.width, slot.rect.height), (1920, 1080));
    }

    #[test]
    fn a_host_with_no_pane_bus_says_so_rather_than_opening_a_black_window() {
        // A host with no `--client-dir` bundle has nothing to load a console
        // from. It also has no lobby surface to press, so this is unreachable in
        // production — but the layout is still lawful, so the follower must
        // decline rather than spawn a fullscreen window showing nothing.
        let (mut app, _) = booted(None);
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"
                "#,
            )
            .unwrap(),
        ));
        // Re-seed the layout with the roster, as a runtime world load would.
        let seeded = BridgeLayout::from_discovered(
            &app.world()
                .resource::<BridgeLayoutResource>()
                .monitors
                .clone(),
            [station("helm")],
        )
        .unwrap();
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .layout = seeded;
        seat(&mut app, "helm", BENQ);
        app.update();

        assert!(
            surfaces(&app).is_empty(),
            "no bus, no console — and therefore no window"
        );
    }

    // ── an authored station console belongs to the LAW, not the watcher ─────

    /// A three-screen host booted from an AUTHORED `--profile` that seats
    /// `helm` on the BenQ, with a pane bus and a two-station hull — the one
    /// shape in which the boot profile and the live layout can disagree about
    /// where a console is.
    fn authored_console_host() -> (App, crate::native_host::panes::transport::PaneBus) {
        use crate::native_host::bridge_profile::{PaneSlot, ROLE_STATION};
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let profile = BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                DisplayEntry {
                    id: DELL.to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: BENQ.to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_station("helm")],
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .expect("a viewscreen and a one-station display validate");

        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        let bus = PaneBus::default();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.insert_resource(BridgeDisplayConfig {
            profile,
            authored: true,
        });
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"

                [[station]]
                id = "weapons"
                name = "Tactical"
                description = "-"
                rank = "Crew"
                "#,
            )
            .expect("a two-station hull parses"),
        ));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 5760, 0));
        app.update();
        (app, bus)
    }

    #[test]
    fn unplugging_the_screen_an_authored_console_has_left_does_not_close_it() {
        // `PaneSlot::for_station` names its pane for its station, so an authored
        // station slot used to put a STATION ID into the #1125 watcher's
        // `pane_labels`. That list is the BOOT profile's and never moves; the
        // console does. So unplugging the monitor the profile named resolved
        // "helm" against the LIVE bus, found the console on the screen the
        // operator had since moved it to, and closed it — ending a human's watch
        // over a display their console was not on, and minting a fresh token in
        // its place.
        use crate::native_host::transport::NativeTransport;

        let (mut app, bus) = authored_console_host();
        let booted = bus
            .open_pane_for_name("helm")
            .expect("the authored seat opened its console at boot");
        bus.mark_live(booted);
        let token = bus.token_of(booted).expect("an ordinary session token");
        bus.take_pending_views();

        // The lobby moves it. A move IS a rebuild, on the same identity.
        seat(&mut app, "helm", ACME);
        app.update();
        let moved = bus
            .open_pane_for_name("helm")
            .expect("the console survived the move");
        assert_eq!(
            bus.token_of(moved).as_deref(),
            Some(token.as_str()),
            "whoever claimed it kept it across the move"
        );
        bus.take_pending_views();
        let _ = bus.transport().poll();

        // Now unplug the screen it is NOT on — the one the boot profile named.
        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        settle(&mut app);

        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .map(|m| m.as_str()),
            Some(ACME),
            "the law never unseated it: its screen is still there"
        );
        assert_eq!(bus.open_count(), 1);
        assert_eq!(
            bus.open_pane_for_name("helm"),
            Some(moved),
            "the same handle — not closed, not even rebuilt"
        );
        assert_eq!(
            bus.token_of(moved).as_deref(),
            Some(token.as_str()),
            "and therefore the same identity: the token guarantee holds"
        );
        assert!(
            bus.transport().poll().is_empty(),
            "nobody was disconnected by the unplug of a screen they were not on"
        );
    }

    // ── two consoles on one screen (issue #1332) ────────────────────────────
    //
    // #1331 tiled two SEATED consoles because the geometry was free, and carried
    // two things forward. First the overlap: the law counted only seats, so a
    // screen already holding a hand-authored `--pane` console offered a station a
    // slot and the adapter then laid the newcomer across the whole monitor on top
    // of it. Second the neighbour rebuild: a console nobody moved loses its
    // rectangle when one arrives beside it or leaves, so its page reloads and its
    // crew member spends that load on `Backfill`.

    /// A three-screen host booted from an AUTHORED `--profile` that opens a
    /// **participant** pane for `Ada` on the BenQ — the `--pane`-shaped profile.
    ///
    /// The one shape in which a screen carries a console the layout may lay out
    /// but never seat, move or close.
    fn participant_pane_host() -> (App, crate::native_host::panes::transport::PaneBus) {
        use crate::native_host::bridge_profile::{PaneSlot, ROLE_STATION};
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let profile = BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                DisplayEntry {
                    id: DELL.to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: BENQ.to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot::for_participant("Ada")],
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .expect("a viewscreen and a one-participant display validate");

        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        let bus = PaneBus::default();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.insert_resource(BridgeDisplayConfig {
            profile,
            authored: true,
        });
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"

                [[station]]
                id = "weapons"
                name = "Tactical"
                description = "-"
                rank = "Crew"
                "#,
            )
            .expect("a two-station hull parses"),
        ));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 5760, 0));
        app.update();
        // The `--pane` participant's own console, as `LocalPanes` opens it at
        // boot: an ordinary pane on the bus under her name, which the layout
        // never seated and may never close.
        let ada =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000042", "Ada").unwrap());
        bus.mark_live(ada);
        bus.take_pending_views();
        (app, bus)
    }

    /// The rectangle a named pane occupies on the surface it is composited onto.
    fn rect_of(app: &App, label: &str) -> PaneRect {
        app.world()
            .resource::<BridgeStationSurfaces>()
            .slot_for(label)
            .unwrap_or_else(|| panic!("{label} has a slot"))
            .1
            .rect
    }

    /// The notices the row is owed this frame, by their `strings.csv` id.
    fn notice_ids(app: &App) -> Vec<String> {
        row(app).notices.into_iter().map(|n| n.id).collect()
    }

    /// A three-screen host booted from an AUTHORED `--profile` that gives the
    /// BenQ `panes` under `split`, with every participant pane in it **already
    /// open on the bus before the first frame** — which is what `LocalPanes`
    /// does for a `--pane <NAME>` at boot.
    ///
    /// That last part is the whole point of the fixture. A re-tile is only
    /// announced for a console the bus actually has, so a fixture that opened
    /// the participant's pane *after* boot could not see the boot frame closing
    /// and recreating it.
    fn authored_benq_host(
        split: Option<super::super::bridge_profile::PaneSplit>,
        panes: Vec<PaneSlot>,
    ) -> (App, crate::native_host::panes::transport::PaneBus) {
        use crate::native_host::bridge_profile::ROLE_STATION;
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::PaneBusResource;

        let profile = BridgeProfile {
            version: PROFILE_VERSION,
            displays: vec![
                DisplayEntry {
                    id: DELL.to_string(),
                    role: ROLE_VIEWSCREEN.to_string(),
                    split: None,
                    panes: Vec::new(),
                },
                DisplayEntry {
                    id: BENQ.to_string(),
                    role: ROLE_STATION.to_string(),
                    split,
                    panes: panes.clone(),
                },
            ],
            touch: Vec::new(),
            media: Vec::new(),
        }
        .validate()
        .expect("the fixture is a valid profile");

        let mut app = App::new();
        app.add_plugins(BridgeDisplayPlugin);
        let bus = PaneBus::default();
        app.insert_resource(PaneBusResource(bus.clone()));
        app.insert_resource(BridgeDisplayConfig {
            profile,
            authored: true,
        });
        app.insert_resource(crate::ship::components::PendingShipConfig(
            toml::from_str(
                r#"
                [[station]]
                id = "helm"
                name = "Helm"
                description = "-"
                rank = "Crew"

                [[station]]
                id = "weapons"
                name = "Tactical"
                description = "-"
                rank = "Crew"
                "#,
            )
            .expect("a two-station hull parses"),
        ));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 5760, 0));

        // Before the first frame, as `LocalPanes` opens them.
        for (index, slot) in panes.iter().enumerate() {
            if slot.station.is_some() {
                continue;
            }
            let uuid = format!("3f1a6c2e-0a11-4b3c-9d55-0000000000{:02}", index + 1);
            let pane = bus.open(
                PaneIdentity::adopt(&uuid, &slot.label).expect("a well-formed pane identity"),
            );
            bus.mark_live(pane);
        }
        bus.take_pending_views();

        app.update();
        (app, bus)
    }

    #[test]
    fn a_boot_frame_draws_the_authored_arrangement_and_re_tiles_nothing() {
        // THE MINIMUM BAR of issue #1332's fix round, and the defect it closes.
        //
        // There used to be two tilings. `apply_bridge_profile` laid this screen
        // out from the file — helm first, because that is what the operator
        // wrote — and `follow_layout_stations`, chained immediately after it in
        // the same `Update`, re-laid it from the law, which put every reserved
        // surface first. So the boot frame drew the operator's arrangement and
        // then inverted it: Ada's console lost the half it had been given, was
        // closed and recreated for a rectangle change nobody had asked for, and
        // the row was handed a re-tiling notice — routing straight around the
        // deliberate rule that boot notes are LOGGED and never pushed.
        //
        // A pure boot frame must re-tile nothing at all.
        let (app, bus) = authored_benq_host(
            None,
            vec![
                PaneSlot::for_station("helm"),
                PaneSlot::for_participant("Ada"),
            ],
        );
        let ada = bus.open_pane_for_name("Ada").expect("her console is open");

        assert!(
            notice_ids(&app).is_empty(),
            "nobody moved anything, so the row is owed nothing"
        );
        let helm = rect_of(&app, "helm");
        let ada_rect = rect_of(&app, "Ada");
        assert_eq!(
            (helm.x, helm.width),
            (0, 960),
            "the file put helm in the first pane, so helm is the left half"
        );
        assert_eq!((ada_rect.x, ada_rect.width), (960, 960));
        assert_eq!(
            bus.open_pane_for_name("Ada"),
            Some(ada),
            "and her console was never closed and rebuilt: the same handle it booted with"
        );
    }

    #[test]
    fn a_boot_frame_honours_the_other_authored_order_too() {
        // The mirror, so the fix cannot be "seats first" — a blanket rule the
        // other way round, wrong for exactly the profiles the old one was right
        // for. It is the file's own order, whichever order that is.
        let (app, _bus) = authored_benq_host(
            None,
            vec![
                PaneSlot::for_participant("Ada"),
                PaneSlot::for_station("helm"),
            ],
        );
        assert!(notice_ids(&app).is_empty());
        assert_eq!((rect_of(&app, "Ada").x, rect_of(&app, "helm").x), (0, 960));
    }

    #[test]
    fn a_boot_frame_keeps_an_authored_stacked_screen_stacked() {
        // The second thing the follower's tiling overrode: it knew only
        // `LAYOUT_SPLIT`, so a screen the operator authored `stacked` was drawn
        // stacked at boot and re-carved side by side one system later — two
        // consoles rebuilt, and the arrangement in the file simply ignored.
        use super::super::bridge_profile::PaneSplit;

        let (app, bus) = authored_benq_host(
            Some(PaneSplit::Stacked),
            vec![
                PaneSlot::for_participant("Ada"),
                PaneSlot::for_participant("Grace"),
            ],
        );
        let handles = (
            bus.open_pane_for_name("Ada"),
            bus.open_pane_for_name("Grace"),
        );

        assert!(notice_ids(&app).is_empty());
        let ada = rect_of(&app, "Ada");
        let grace = rect_of(&app, "Grace");
        assert_eq!((ada.y, ada.height), (0, 540));
        assert_eq!((grace.y, grace.height), (540, 540));
        assert_eq!(
            (ada.width, grace.width),
            (1920, 1920),
            "full width, stacked"
        );
        assert_eq!(
            (
                bus.open_pane_for_name("Ada"),
                bus.open_pane_for_name("Grace")
            ),
            handles,
            "and neither console was rebuilt to reach the arrangement it booted in"
        );
    }

    #[test]
    fn an_off_roster_authored_station_opens_no_window_and_tells_the_row_nothing() {
        // The BOOT SIDE EFFECT this fix round removed, pinned so it cannot come
        // back. The old path laid a Station out from the FILE, so a `--profile`
        // naming a station this hull does not have — a cruiser's profile
        // launched on a destroyer — spawned a borderless-fullscreen window for a
        // console the law had already refused: a black screen with nothing
        // behind it, and (once the follower re-tiled the same monitor from the
        // law and found it empty) a spawned-then-closed window nobody had asked
        // for. Boot reads `surface_rects` now, so a refused seat is simply not
        // on the screen and no window is opened for it.
        let (app, _bus) = authored_benq_host(None, vec![PaneSlot::for_station("flight-deck")]);

        let layout = &app.world().resource::<BridgeLayoutResource>().layout;
        assert!(
            layout.occupants_on(&MonitorIdentity::new(BENQ)).is_empty(),
            "the law refused the seat — the destroyer has no flight-deck"
        );
        assert!(
            surfaces(&app).is_empty(),
            "so nothing is drawn on that screen, and no Station window is opened for it"
        );
        assert!(
            notice_ids(&app).is_empty(),
            "and the refusal is a BOOT note: logged, never pushed onto the lobby's row"
        );
    }

    #[test]
    fn a_console_seated_beside_an_authored_one_tiles_rather_than_covering_it() {
        // The carried defect, end to end. Before this the law offered the BenQ
        // (it counted only seats), took the press, and this pass then handed helm
        // `pane_rects(count = 1)` — the WHOLE monitor — while Ada's authored slot
        // was retained at the whole monitor too. Two consoles, one rectangle.
        let (mut app, bus) = participant_pane_host();
        assert_eq!(
            rect_of(&app, "Ada").width,
            1920,
            "Ada has the screen to herself at boot"
        );

        seat(&mut app, "helm", BENQ);
        app.update();

        assert_eq!(
            surfaces(&app),
            vec![BENQ.to_string()],
            "one window, as ever — a monitor has exactly one Station surface, so \
             sharing it is the only honest answer"
        );
        let ada = rect_of(&app, "Ada");
        let helm = rect_of(&app, "helm");
        assert_eq!(
            (ada.x, ada.width),
            (0, 960),
            "the authored one keeps the left"
        );
        assert_eq!((helm.x, helm.width), (960, 960));
        assert_eq!(ada.width + helm.width, 1920, "they tile the screen exactly");
        assert_eq!(bus.open_count(), 2, "and both consoles are live");
    }

    #[test]
    fn an_authored_console_is_rebuilt_at_its_new_half_and_keeps_its_identity() {
        // Laying it out is not the same as owning it. The pane is never closed
        // by the layout and never re-minted — but its VIEW was built at one size
        // on one window, so yielding half the screen costs it the same
        // close+recreate a station's console pays, on the same session token.
        let (mut app, bus) = participant_pane_host();
        let before = bus.open_pane_for_name("Ada").expect("her console is open");
        let token = bus.token_of(before).expect("an ordinary session token");

        seat(&mut app, "helm", BENQ);
        app.update();

        let after = bus
            .open_pane_for_name("Ada")
            .expect("her console is still open");
        assert_ne!(after, before, "a rebuilt view is a new handle");
        assert_eq!(
            bus.token_of(after).as_deref(),
            Some(token.as_str()),
            "on the SAME identity, so she reconnects as herself rather than as a stranger"
        );
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![after, bus.open_pane_for_name("helm").unwrap()],
            "one view to rebuild for her and one to build for the newcomer"
        );
    }

    #[test]
    fn the_console_that_was_re_tiled_is_named_on_the_row_and_the_one_that_moved_is_not() {
        // The neighbour flap, made visible (issue #1332). The operator watched
        // themselves seat `weapons`; what they did not ask for — and would
        // otherwise never be told — is that `helm`'s page is reloading and
        // `helm` is on AI control until it finishes.
        let (mut app, _bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        assert!(
            notice_ids(&app).is_empty(),
            "the first console on an empty screen re-tiles nothing"
        );

        seat(&mut app, "weapons", BENQ);
        app.update();

        assert_eq!(
            notice_ids(&app),
            vec!["server.bridge_layout.adopt_console_retiling".to_string()],
            "exactly one line, and it is about the neighbour"
        );
        let notice = &row(&app).notices[0];
        assert_eq!(
            notice.params.get("console").map(String::as_str),
            Some("helm"),
            "the console nobody asked to move"
        );
        assert_eq!(notice.params.get("monitor").map(String::as_str), Some(BENQ));
    }

    #[test]
    fn a_console_the_operator_moved_between_screens_is_not_announced_as_a_surprise() {
        // The other half of the same rule, and why the two are told apart by the
        // evidence rather than by a flag: a console whose MONITOR changed was
        // moved by the press the operator just made. Narrating that back to them
        // would bury the neighbour's line, which is the one that is news.
        let (mut app, _bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();

        seat(&mut app, "helm", ACME);
        app.update();

        assert!(
            notice_ids(&app).is_empty(),
            "the move says nothing; the operator watched themselves make it"
        );
    }

    #[test]
    fn moving_a_console_off_a_shared_screen_regrows_the_one_that_stayed_and_says_so() {
        // The move at the 2-up boundary, which is the acceptance criterion's
        // other direction: leaving a shared screen re-tiles the survivor to the
        // whole of it, and the survivor is a neighbour nobody asked to move.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        seat(&mut app, "weapons", BENQ);
        app.update();
        let helm_token = bus
            .token_of(bus.open_pane_for_name("helm").unwrap())
            .unwrap();
        bus.take_pending_views();
        assert_eq!(rect_of(&app, "helm").width, 960);

        seat(&mut app, "weapons", ACME);
        app.update();

        assert_eq!(
            surfaces(&app),
            vec![BENQ.to_string(), ACME.to_string()],
            "two screens now, one console each"
        );
        assert_eq!(
            (rect_of(&app, "helm").x, rect_of(&app, "helm").width),
            (0, 1920),
            "the console that stayed grows into the whole screen"
        );
        assert_eq!(rect_of(&app, "weapons").width, 1920);
        assert_eq!(
            bus.token_of(bus.open_pane_for_name("helm").unwrap())
                .as_deref(),
            Some(helm_token.as_str()),
            "and whoever was at it keeps it across the re-tile"
        );
        assert_eq!(
            notice_ids(&app),
            vec!["server.bridge_layout.adopt_console_retiling".to_string()],
        );
        assert_eq!(
            row(&app).notices[0]
                .params
                .get("console")
                .map(String::as_str),
            Some("helm"),
        );

        // The vacated slot is offered again, everywhere at once. The law decides
        // it and the row is a `map` over the law, so this is the whole of "the
        // vacated monitor un-greys on every other station's row".
        let benq_for = |station: &str| {
            row(&app)
                .stations
                .into_iter()
                .find(|r| r.station == station)
                .unwrap_or_else(|| panic!("{station} has a row"))
                .monitors
                .into_iter()
                .find(|s| s.identity == BENQ)
                .expect("the BenQ has an entry")
                .choice
        };
        assert_eq!(benq_for("helm"), "selected", "helm is still on it");
        assert_eq!(
            benq_for("weapons"),
            "eligible",
            "and the station that left is offered it back — the screen has a slot again"
        );
        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .occupancy_of(&MonitorIdentity::new(BENQ))
                .unwrap()
                .free_slots,
            Some(1)
        );
    }

    #[test]
    fn a_console_rebuilt_because_its_screen_changed_size_is_not_told_the_split_changed() {
        // The honest attribution (issue #1332's fix round). This pass used to
        // call every in-place rectangle change "the split changed", which is a
        // sentence the code cannot support for a display that renegotiated its
        // mode: nothing joined that screen and nothing left it. The console IS
        // rebuilt — a view is built at one size — so it still earns a notice;
        // what it must not earn is a wrong reason, which sends the operator
        // hunting for the console that arrived.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let before = bus.open_pane_for_name("helm").expect("its console opened");
        bus.mark_live(before);
        bus.take_pending_views();
        assert!(
            notice_ids(&app).is_empty(),
            "the first console on an empty screen re-tiles nothing"
        );

        // The BenQ renegotiates its resolution in place — a television waking,
        // an EDID handshake settling. Same name, same corner, new pixels, so
        // `identify_stable` carries its identity and the LAW sees the same
        // bridge with the same console on the same screen.
        let benq = benq_entity(&mut app);
        {
            let mut entity = app.world_mut().entity_mut(benq);
            let mut display = entity.get_mut::<Monitor>().expect("it is a monitor");
            display.physical_width = 1280;
            display.physical_height = 1024;
        }
        app.update();

        assert_eq!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitors()
                .iter()
                .map(|m| m.as_str().to_string())
                .collect::<Vec<_>>(),
            vec![DELL.to_string(), BENQ.to_string(), ACME.to_string()],
            "the same three screens, the BenQ's identity carried across its new mode"
        );
        let helm = rect_of(&app, "helm");
        assert_eq!(
            (helm.width, helm.height),
            (1280, 1024),
            "and its console now fills the screen at its new size"
        );
        assert_eq!(
            notice_ids(&app),
            vec!["server.bridge_layout.adopt_console_resized".to_string()],
            "the true cause: the screen changed size, holding exactly what it held"
        );
        let notice = &row(&app).notices[0];
        assert_eq!(
            notice.params.get("console").map(String::as_str),
            Some("helm")
        );
        assert_eq!(notice.params.get("monitor").map(String::as_str), Some(BENQ));
        assert_ne!(
            bus.open_pane_for_name("helm"),
            Some(before),
            "and it really was rebuilt — the notice is not decoration"
        );
    }

    #[test]
    fn a_re_tiling_notice_settles_rather_than_flapping() {
        // The notice marks the layout changed, which is how it reaches the row —
        // and the pass therefore runs once more. That pass must find the
        // rectangles it just wrote, re-tile nothing and say nothing, or the host
        // would rebuild a console on every frame for the rest of the run.
        //
        // The seats are taken one frame at a time so there IS a re-tile to
        // settle: helm has the screen to itself, then weapons joins it and helm
        // — which nobody moved — loses half. That is the one notice, and the
        // twenty frames after it must add none.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        bus.take_pending_views();
        seat(&mut app, "weapons", BENQ);
        app.update();
        assert_eq!(
            notice_ids(&app),
            vec!["server.bridge_layout.adopt_console_retiling".to_string()],
            "helm was re-tiled by the newcomer, and said so once"
        );
        let handles = |b: &crate::native_host::panes::transport::PaneBus| {
            (
                b.open_pane_for_name("helm"),
                b.open_pane_for_name("weapons"),
            )
        };
        let settled = handles(&bus);

        for _ in 0..20 {
            app.update();
        }

        assert_eq!(handles(&bus), settled, "nothing was rebuilt again");
        assert_eq!(bus.open_count(), 2);
        assert_eq!(
            row(&app).notices.len(),
            1,
            "and the row was told once, not once a frame"
        );
    }

    // ── the law and the adapter are reconciled (issue #1331) ────────────────
    //
    // `follow_layout_stations` applies the layout; it cannot see whether the
    // application worked. A view that will not build, or a seat with no surface
    // to be composited onto, leaves a station card claiming a screen that is
    // black, with nothing retrying and nothing saying so. These pin the repair:
    // a bounded rebuild on the same identity, and then an honest Backfill.

    #[test]
    fn a_healthy_console_is_never_touched_by_the_reconciler() {
        // First, the frames it must NOT act on — which is all of them, on a
        // bridge where everything worked.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        bus.mark_live(pane);
        bus.take_pending_views();

        for _ in 0..(CONSOLE_MISSING_GRACE_FRAMES * 4) {
            app.update();
        }

        assert_eq!(
            bus.open_pane_for_name("helm"),
            Some(pane),
            "the same handle, never rebuilt"
        );
        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
        assert!(
            bus.take_pending_views().is_empty(),
            "and nothing was queued to rebuild"
        );
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .notices
            .is_empty());
    }

    #[test]
    fn a_console_with_nowhere_to_be_built_is_rebuilt_boundedly_then_gives_its_seat_back() {
        // A seated station whose console has no `BridgeStationSurfaces` slot:
        // the pane host has no window and no rectangle for it, so it builds the
        // view on the wrong window or not at all. Poked in directly, because the
        // ways to reach it — a seat made while its monitor was between hot-plug
        // frames, a placement that failed — all leave exactly this state.
        use crate::native_host::panes::recovery::MAX_RECREATIONS_PER_WINDOW;

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let mut current = bus.open_pane_for_name("helm").unwrap();
        let token = bus.token_of(current).unwrap();
        bus.mark_live(current);
        bus.take_pending_views();

        app.world_mut()
            .resource_mut::<BridgeStationSurfaces>()
            .0
            .iter_mut()
            .for_each(|s| s.panes.clear());

        let mut rebuilds = 0u32;
        for _ in 0..(MAX_RECREATIONS_PER_WINDOW + 1) {
            for _ in 0..CONSOLE_MISSING_GRACE_FRAMES {
                app.update();
            }
            let Some(next) = bus.open_pane_for_name("helm") else {
                break;
            };
            assert_ne!(next, current, "a rebuild is a new handle");
            assert_eq!(
                bus.token_of(next).as_deref(),
                Some(token.as_str()),
                "rebuilt on the SAME identity, so a console that does come back is \
                 the same participant"
            );
            rebuilds += 1;
            current = next;
        }

        assert_eq!(
            rebuilds, MAX_RECREATIONS_PER_WINDOW,
            "bounded by #1125's own per-identity budget, not retried forever"
        );
        assert_eq!(bus.open_count(), 0, "and then left closed");

        // The seat is GIVEN BACK, which is the half a `--pane` does not need:
        // the law must stop claiming a screen the adapter cannot use.
        let live = app.world().resource::<BridgeLayoutResource>();
        assert!(
            live.layout.monitor_of(&station("helm")).is_none(),
            "the station is on Backfill honestly, not by accident"
        );
        let [LayoutNotice::Adopted(note)] = live.notices.as_slice() else {
            panic!("the operator is told, in a sentence: {:?}", live.notices);
        };
        assert_eq!(
            note.string_id(),
            "server.bridge_layout.adopt_console_could_not_open"
        );

        // …and the row renders it, which is the only place an operator sees it.
        let row = row(&app);
        let notice = row
            .notices
            .iter()
            .find(|n| n.id == "server.bridge_layout.adopt_console_could_not_open")
            .expect("the notice crosses to the surface");
        assert_eq!(
            notice.params.get("station").map(String::as_str),
            Some("helm")
        );
        assert_eq!(notice.params.get("monitor").map(String::as_str), Some(BENQ));
        assert!(
            row.monitors[1].stations.is_empty(),
            "and the screen reads as free again, so it can be used for something"
        );

        // The Station window goes with the seat.
        app.update();
        app.update();
        assert!(surfaces(&app).is_empty());
    }

    #[test]
    fn a_console_whose_view_will_not_build_reaches_backfill_rather_than_a_black_screen() {
        // The pane host's half, at the seam #1125 built for exactly this: a
        // failed `make_pane_view` faults the pane, `service_faults` closes it
        // (one honest disconnect) and rebuilds it on the same token, and past
        // the budget it stays closed. Injected here, as #1125's own tests inject
        // one, because building a real Ultralight view needs an SDK and a GPU.
        //
        // What this adds is the end of that story: the LAW still seated a
        // station whose console the pane host has given up on, and something has
        // to say so.
        use crate::native_host::panes::recovery::{
            service_faults, PaneFault, MAX_RECREATIONS_PER_WINDOW,
        };

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let mut current = bus.open_pane_for_name("helm").unwrap();

        let mut exhausted = false;
        for _ in 0..(MAX_RECREATIONS_PER_WINDOW + 1) {
            bus.fault(current, PaneFault::ViewCrashed);
            let outcome = service_faults(&bus).pop().expect("one fault serviced");
            match outcome.recreated {
                Some((next, _url)) => current = next,
                None => exhausted = outcome.recreation_exhausted,
            }
            app.update();
        }
        assert!(exhausted, "the pane host gave up, as #1125 says it must");
        assert_eq!(bus.open_count(), 0);
        assert!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitor_of(&station("helm"))
                .is_some(),
            "and the law is still seating it — which is the divergence"
        );

        for _ in 0..CONSOLE_MISSING_GRACE_FRAMES {
            app.update();
        }

        let live = app.world().resource::<BridgeLayoutResource>();
        assert!(
            live.layout.monitor_of(&station("helm")).is_none(),
            "the seat is surrendered, so the row stops claiming a black screen"
        );
        assert!(
            live.notices.iter().any(|n| matches!(
                n,
                LayoutNotice::Adopted(note)
                    if note.string_id() == "server.bridge_layout.adopt_console_could_not_open"
            )),
            "and the operator is told why: {:?}",
            live.notices
        );
    }

    // ── an authored console is not free room (issue #1330) ──────────────────

    #[test]
    fn the_viewscreen_may_not_move_onto_an_authored_participants_console() {
        // `--pane`-shaped profiles seat NOTHING: their panes name a person, not
        // a station, so `adopt_profile` has no `StationId` to place. The
        // adapter opens the Station window all the same — so a layout that
        // recorded nothing would read that screen as free and let the
        // viewscreen cover Ada's live console.
        let (app, _) = booted(Some(viewscreen_and_station("Ada")));
        assert_eq!(
            app.world().resource::<BridgeStationSurfaces>().0.len(),
            1,
            "the profile's Station window really is open on the BenQ"
        );

        let refusal = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new(BENQ),
            })
            .expect_err("Ada's console is on it");
        assert_eq!(
            refusal.string_id(),
            "server.bridge_layout.viewscreen_holds_stations"
        );
        assert_eq!(
            refusal
                .params()
                .iter()
                .find(|(k, _)| *k == "stations")
                .map(|(_, v)| v.as_str()),
            Some("Ada"),
            "and the notice names who is on it, which is the only way to act on it"
        );
    }

    #[test]
    fn the_row_draws_an_authored_participants_console_as_an_occupant() {
        // The other half of the same fact: a press the law will refuse must be
        // legible on the button BEFORE it is pressed, not only in the sentence
        // that comes back.
        let (app, _) = booted(Some(viewscreen_and_station("Ada")));
        let row = row(&app);
        assert_eq!(row.monitors[1].identity, BENQ);
        assert_eq!(row.monitors[1].stations, vec!["Ada".to_string()]);
        assert!(
            row.monitors[0].stations.is_empty(),
            "the viewscreen holds none"
        );
    }

    // ── a returned display, a frame with none, and two writers in one frame ──

    #[test]
    fn a_duplicate_pane_label_resolves_to_the_stations_own_slot() {
        // `slot_for` was a first-match by iteration order, and the only way two
        // slots can carry one label is an authored `--profile` participant pane
        // named for a station this hull has — which
        // `app::install_world_selection` now refuses at load, on both boot
        // paths. This is the tie-break that makes the unreachable case DECIDED:
        // a positional answer would build the station's console on the authored
        // monitor at the authored rectangle, leaving the screen the operator
        // chose black — and `reconcile_seated_consoles`, which asks only whether
        // SOME slot carries the name, would be satisfied by the wrong one and
        // never fire.
        let rect = |x: u32| PaneRect {
            x,
            y: 0,
            width: 960,
            height: 1080,
        };
        let surfaces = BridgeStationSurfaces(vec![
            BridgeStationSurface {
                identity: BENQ.to_string(),
                window: Entity::PLACEHOLDER,
                monitor: Entity::PLACEHOLDER,
                geometry: MonitorGeometry {
                    physical_width: 1920,
                    physical_height: 1080,
                    position_x: 3840,
                    position_y: 0,
                    scale_factor: 1.0,
                },
                // The authored participant pane, first in iteration order.
                panes: vec![StationPane {
                    label: "helm".to_string(),
                    rect: rect(0),
                    station: None,
                }],
            },
            BridgeStationSurface {
                identity: ACME.to_string(),
                window: Entity::PLACEHOLDER,
                monitor: Entity::PLACEHOLDER,
                geometry: MonitorGeometry {
                    physical_width: 1920,
                    physical_height: 1080,
                    position_x: 5760,
                    position_y: 0,
                    scale_factor: 1.0,
                },
                // The station's own console, on the screen the lobby chose.
                panes: vec![StationPane {
                    label: "helm".to_string(),
                    rect: rect(960),
                    station: Some(station("helm")),
                }],
            },
        ]);

        let (surface, slot) = surfaces.slot_for("helm").expect("the label resolves");
        assert_eq!(
            surface.identity, ACME,
            "the station-bearing slot wins, wherever it sits in the list"
        );
        assert_eq!(slot.station.as_ref(), Some(&station("helm")));

        // A label only one slot carries is unaffected, which is every label a
        // refused-at-load bridge actually has.
        assert!(surfaces.slot_for("weapons").is_none());
    }

    #[test]
    fn a_display_that_left_and_came_back_re_anchors_its_station_window() {
        // `bevy_winit` despawns a `Monitor` entity when a display stops being
        // reported and spawns a NEW one when it returns, so a blip inside the
        // settle window — which the law-based retain exists to survive — leaves
        // the Station window's `BorderlessFullscreen(Entity(old))` naming an
        // entity that is gone. winit re-applies fullscreen only when the mode
        // CHANGES, so without rewriting it the window sits wherever the OS
        // parked it while the geometry, the pane rects and the input router all
        // use the returned monitor's coordinates: drawn on one screen, clicked
        // on another.
        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let (window, anchored) = {
            let surface = &app.world().resource::<BridgeStationSurfaces>().0[0];
            (surface.window, surface.monitor)
        };
        let booted_on = benq_entity(&mut app);
        assert_eq!(anchored, booted_on);
        assert_eq!(
            window_mode(&app, window),
            WindowMode::BorderlessFullscreen(MonitorSelection::Entity(booted_on))
        );

        // Out and back, well inside the settle window: the law never unseats
        // the console, so the surface and the console are the same ones.
        app.world_mut().entity_mut(booted_on).despawn();
        app.update();
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.update();

        let replugged = benq_entity(&mut app);
        assert_ne!(replugged, booted_on, "winit hands back a new handle");
        let surface = &app.world().resource::<BridgeStationSurfaces>().0[0];
        assert_eq!(surface.window, window, "the same window, not a new one");
        assert_eq!(
            surface.monitor, replugged,
            "and the surface now knows which display it is anchored to"
        );
        assert_eq!(
            window_mode(&app, window),
            WindowMode::BorderlessFullscreen(MonitorSelection::Entity(replugged)),
            "the mode is rewritten, which is the only thing winit acts on"
        );
        assert_eq!(bus.open_count(), 1, "and nobody was dropped for a blip");
    }

    #[test]
    fn a_seat_pressed_on_a_frame_with_no_monitors_is_not_surrendered_for_it() {
        // The applier declines a frame reporting no monitors at all — winit
        // between hot-plug events — and leaves its pass owed, so a station
        // seated on such a frame has neither a surface nor a console through no
        // fault of anything below this layer. The reconciler used to count those
        // frames against the grace and give the seat back after ten of them,
        // reporting a failure that never happened.
        let (mut app, bus) = console_host();
        let monitors: Vec<Entity> = {
            let mut query = app.world_mut().query_filtered::<Entity, With<Monitor>>();
            query.iter(app.world()).collect()
        };
        for monitor in monitors {
            app.world_mut().entity_mut(monitor).despawn();
        }

        seat(&mut app, "helm", BENQ);
        for _ in 0..(CONSOLE_MISSING_GRACE_FRAMES * 2) {
            app.update();
        }

        let live = app.world().resource::<BridgeLayoutResource>();
        assert_eq!(
            live.layout.monitor_of(&station("helm")).map(|m| m.as_str()),
            Some(BENQ),
            "the seat is still the operator's: nothing has had a chance to fail"
        );
        assert!(
            live.notices.is_empty(),
            "and nothing was reported: {:?}",
            live.notices
        );
        assert_eq!(bus.open_count(), 0, "the applier's pass is still owed");

        // And when the displays are reported again, that owed pass runs.
        app.world_mut()
            .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        app.world_mut()
            .spawn(monitor("ACME 1080", 1920, 1080, 5760, 0));
        app.update();
        assert_eq!(
            bus.open_count(),
            1,
            "the console opens on the screen chosen"
        );
        assert_eq!(surfaces(&app), vec![BENQ.to_string()]);
    }

    #[test]
    fn a_press_and_a_seat_surrender_in_one_frame_both_reach_the_row() {
        // `BridgeLayoutResource::notices` is written by two UNORDERED chains —
        // the lobby's own record drain, and this module's reconcilers — and both
        // used to assign. So whichever ran last erased the other, and the frame
        // where that decides something is exactly the frame worth reporting: a
        // press landing as a seat is surrendered dropped `ConsoleCouldNotOpen`,
        // the one notice the reconciler exists to deliver. Both orders are driven
        // here, because the order is the bug.
        //
        // The lobby's writer is `drain_surface_records` — the surface's ONE
        // reader since #1328 folded every page->host verb into one vocabulary —
        // and it runs in `PreUpdate` for real. It is registered beside the
        // reconcilers here on purpose: what is being pinned is the two writers
        // landing in the SAME frame in EITHER order, and one schedule is how a
        // test can choose the order.
        use crate::native_host::host_lobby::{
            drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
        };
        use crate::native_host::panes::transport::PaneBus;
        use crate::native_host::panes::{PaneBusResource, RecordingSurface};

        for lobby_first in [true, false] {
            let bridge = HostLobbyBridge::new();
            let mut app = App::new();
            app.insert_resource(HostLobbyBridgeResource(bridge.clone()));
            // The drain writes the operator's picks onto this bus. Nothing here
            // sends one, but a `MessageWriter` parameter is validated when the
            // system RUNS, so the bus has to exist.
            app.add_message::<crate::lobby::InboundMessage>();
            // A seated station with NO console on the bus and NO slot on any
            // surface: the divergence the reconciler ends.
            app.insert_resource(PaneBusResource(PaneBus::default()));
            app.insert_resource(BridgeStationSurfaces::default());
            // Present, so the zero-monitor guard does not decline the frame.
            app.world_mut()
                .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor));
            app.world_mut()
                .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));

            let discovered = identify(&[
                RawMonitor {
                    name: Some("DELL U2720Q".to_string()),
                    physical_width: 3840,
                    physical_height: 2160,
                    position_x: 0,
                    position_y: 0,
                    scale_factor: 1.0,
                    primary: true,
                },
                RawMonitor {
                    name: Some("BenQ EX".to_string()),
                    physical_width: 1920,
                    physical_height: 1080,
                    position_x: 3840,
                    position_y: 0,
                    scale_factor: 1.0,
                    primary: false,
                },
            ]);
            let layout = BridgeLayout::from_discovered(&discovered, [station("helm")])
                .expect("two monitors are a bridge")
                .apply(&LayoutAction::AssignStation {
                    station: station("helm"),
                    monitor: MonitorIdentity::new(BENQ),
                })
                .expect("a free non-viewscreen monitor takes a console");
            app.insert_resource(BridgeLayoutResource {
                layout,
                monitors: discovered,
                notices: Vec::new(),
            });

            if lobby_first {
                app.add_systems(
                    Update,
                    (drain_surface_records, reconcile_seated_consoles).chain(),
                );
            } else {
                app.add_systems(
                    Update,
                    (reconcile_seated_consoles, drain_surface_records).chain(),
                );
            }

            // Every frame but the last of the grace window, with nobody pressing.
            for _ in 1..CONSOLE_MISSING_GRACE_FRAMES {
                app.update();
            }
            assert!(
                app.world()
                    .resource::<BridgeLayoutResource>()
                    .notices
                    .is_empty(),
                "nothing has failed yet"
            );

            // The press lands on the very frame the grace runs out.
            let mut surface = RecordingSurface::ready();
            surface.queue_record(r#"{"kind":"set-viewscreen","monitor":"Unplugged@1920x1080"}"#);
            pump_host_lobby(&bridge, &mut surface);
            app.update();

            let notices = &app.world().resource::<BridgeLayoutResource>().notices;
            assert!(
                notices.iter().any(|n| matches!(
                    n,
                    LayoutNotice::Refused(refusal)
                        if refusal.string_id() == "server.bridge_layout.unknown_monitor"
                )),
                "the operator's press is answered (lobby_first = {lobby_first}): {notices:?}"
            );
            assert!(
                notices.iter().any(|n| matches!(
                    n,
                    LayoutNotice::Adopted(note)
                        if note.string_id() == "server.bridge_layout.adopt_console_could_not_open"
                )),
                "and the surrendered seat is reported too (lobby_first = {lobby_first}): \
                 {notices:?}"
            );
            assert_eq!(notices.len(), 2, "exactly the two, once each: {notices:?}");
        }
    }

    #[test]
    fn a_press_for_a_monitor_that_vanished_is_refused_at_the_law() {
        // The stale press: the row was drawn, the operator reached for it, and
        // the cable came out in between. The layout law answers with a sentence
        // the lobby renders rather than moving anything.
        let (mut app, _) = booted(None);
        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
            app.update();
        }

        let refusal = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(&LayoutAction::SetViewscreen {
                monitor: MonitorIdentity::new(BENQ),
            })
            .expect_err("the monitor is gone");
        assert_eq!(
            refusal.string_id(),
            "server.bridge_layout.unknown_monitor",
            "and it is a sentence, not a silence"
        );
    }

    // ── a crashed console reopens on its own monitor (issue #1333) ──────────
    //
    // The placement rule itself is pure and lives in `panes::placement`, which
    // is what makes it checkable by CI at all: before #1333 it was six lines
    // inside `open_pending_views`, behind `--features ultralight`, which no job
    // in this repository compiles. These ask that rule of a REAL running
    // bridge — the surfaces `follow_layout_stations` wrote this frame and the
    // law the row edits — at each step of a crash, a flap, an unplug and a move.
    //
    // Every one of them hands the decision a TEMPTING TILE: a stored
    // primary-window rectangle under the console's own name, which is precisely
    // what the pre-#1333 fallback would have reached for. The assertions are
    // that it never does.

    /// Where the pane host would build `name`'s view, decided from the live
    /// bridge exactly as `open_pending_views` decides it.
    fn home(
        app: &App,
        name: &str,
        tiles: &[crate::native_host::panes::PaneTile],
    ) -> crate::native_host::panes::PaneHome {
        let live = app.world().resource::<BridgeLayoutResource>();
        crate::native_host::panes::home_for_pane(
            name,
            Some(app.world().resource::<BridgeStationSurfaces>()),
            Some(&live.layout),
            tiles,
        )
    }

    /// A stored primary-window tile under `name` — the thing a seated console
    /// must never be rebuilt onto. On a real host a station id can never have
    /// one (`app::install_world_selection` refuses a `--pane` label that
    /// shadows a station id, and a runtime console records no tile at all), so
    /// this is the trap made reachable on purpose: with it present, "the seated
    /// console was not tiled" is a claim about the RULE rather than about an
    /// empty list.
    fn tempting_tile(name: &str) -> Vec<crate::native_host::panes::PaneTile> {
        vec![crate::native_host::panes::PaneTile {
            name: name.to_string(),
            origin: (0, 0),
            size: (1280, 720),
        }]
    }

    /// The Station window a surface is open on, by monitor identity.
    fn station_window(app: &App, identity: &str) -> Entity {
        app.world()
            .resource::<BridgeStationSurfaces>()
            .on(identity)
            .expect("a surface is open on that monitor")
            .window
    }

    #[test]
    fn a_crashed_seated_console_is_rebuilt_on_its_own_station_window() {
        // The acceptance criterion, at the seam #1125 built and against the
        // surfaces #1331 keeps live: a console seated on the BenQ has its view
        // crash, is recreated on the same session token, and the pane host is
        // told to build it on the BenQ's OWN window — not tiled onto the
        // viewscreen, whose window is `primary` and whose home would be a
        // `PrimaryTile`.
        use crate::native_host::panes::recovery::{service_faults, PaneFault};
        use crate::native_host::panes::PaneHome;

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        let token = bus.token_of(pane).unwrap();
        bus.mark_live(pane);
        bus.take_pending_views();
        let benq_window = station_window(&app, BENQ);

        bus.fault(pane, PaneFault::ViewCrashed);
        let (recreated, _url) = service_faults(&bus)
            .pop()
            .expect("one fault serviced")
            .recreated
            .expect("a view crash recreates the console");
        assert_eq!(
            bus.token_of(recreated).as_deref(),
            Some(token.as_str()),
            "on the same identity, so whoever was at it reconnects to it"
        );
        assert_eq!(
            bus.name_of(recreated).as_deref(),
            Some("helm"),
            "and under the same name — which is the key `open_pending_views` \
             actually feeds to `home_for_pane`, so the placement asserted below \
             is this pane's and not a coincidence of the string"
        );
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![recreated],
            "and exactly one view is queued for the pane host to build"
        );

        let PaneHome::Station { window, size, .. } = home(&app, "helm", &tempting_tile("helm"))
        else {
            panic!(
                "a crashed console is rebuilt on its own Station window: {:?}",
                home(&app, "helm", &tempting_tile("helm"))
            );
        };
        assert_eq!(
            window, benq_window,
            "the BenQ's window, not the primary one"
        );
        assert_eq!(size, (1920, 1080), "at the seat the layout gives it now");
    }

    #[test]
    fn a_console_that_crashed_and_moved_in_one_frame_lands_on_the_screen_it_moved_to() {
        // The crash-during-a-move edge. `service_faults` closes and recreates
        // (queueing view A), and before the pane host has drained that queue the
        // operator's move closes THAT pane and recreates it again (queueing view
        // B) — the same `close` + `recreate` pair, used deliberately.
        //
        // Two entries, one console: `open_pending_views` skips A because
        // `is_open` says its pane was closed in the interval, and builds B on the
        // surfaces the move rewrote. Nothing double-builds, nothing is orphaned,
        // and the session token survives both hops.
        use crate::native_host::panes::recovery::{service_faults, PaneFault};
        use crate::native_host::panes::{PaneHome, PaneId};

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let original = bus.open_pane_for_name("helm").unwrap();
        let token = bus.token_of(original).unwrap();
        bus.mark_live(original);
        bus.take_pending_views();

        // The crash, serviced but not yet built.
        bus.fault(original, PaneFault::ViewCrashed);
        let (after_crash, _) = service_faults(&bus)
            .pop()
            .expect("one fault serviced")
            .recreated
            .expect("a view crash recreates");

        // The move, in the gap.
        seat(&mut app, "helm", ACME);
        app.update();

        let queued: Vec<PaneId> = bus
            .take_pending_views()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(queued.len(), 2, "one entry from each close+recreate");
        assert_eq!(queued[0], after_crash);
        let open: Vec<PaneId> = queued
            .iter()
            .copied()
            .filter(|id| bus.is_open(*id))
            .collect();
        assert_eq!(
            open.len(),
            1,
            "and exactly one of them is still open, so exactly one view is built"
        );
        assert_eq!(
            bus.open_count(),
            1,
            "one console on the bus, not two racing for the same seat"
        );
        assert_eq!(
            bus.token_of(open[0]).as_deref(),
            Some(token.as_str()),
            "the token survived the crash AND the move, so nobody had to claim again"
        );

        let PaneHome::Station { window, .. } = home(&app, "helm", &tempting_tile("helm")) else {
            panic!("the surviving console still belongs on a Station window");
        };
        assert_eq!(
            window,
            station_window(&app, ACME),
            "on the screen the operator moved it to, not the one it crashed on \
             and not the viewscreen"
        );
    }

    #[test]
    fn a_seated_console_with_nowhere_to_go_is_left_unbuilt_rather_than_put_on_the_viewscreen() {
        // The failure this issue exists to close, made reachable directly: the
        // law still seats helm, and the adapter has no slot for it — a monitor
        // between hot-plug frames, a Station window not yet rebuilt. The
        // pre-#1333 fallback would have taken the stored tile and drawn a wall
        // console over the shared view.
        //
        // The honest answer is that nothing is built, and #1331's reconciler
        // then repairs it boundedly and gives the seat back with a notice — which
        // is the second half asserted here, so "not built" cannot quietly mean
        // "a station card claiming a black screen forever".
        use crate::native_host::panes::recovery::MAX_RECREATIONS_PER_WINDOW;
        use crate::native_host::panes::{NoHome, PaneHome};

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        bus.mark_live(bus.open_pane_for_name("helm").unwrap());
        bus.take_pending_views();

        app.world_mut()
            .resource_mut::<BridgeStationSurfaces>()
            .0
            .iter_mut()
            .for_each(|s| s.panes.clear());

        assert_eq!(
            home(&app, "helm", &tempting_tile("helm")),
            PaneHome::Nowhere(NoHome::SeatedButUnplaced),
            "never the viewscreen, however tempting the tile"
        );

        // And it does not sit there: the reconciler rebuilds it on its own
        // identity up to the #1125 budget and then surrenders the seat.
        for _ in 0..((MAX_RECREATIONS_PER_WINDOW + 2) * CONSOLE_MISSING_GRACE_FRAMES) {
            app.update();
        }
        let live = app.world().resource::<BridgeLayoutResource>();
        assert!(
            live.layout.monitor_of(&station("helm")).is_none(),
            "the seat is given back, so the station is on AI control honestly"
        );
        assert!(
            live.notices.iter().any(|n| matches!(
                n,
                LayoutNotice::Adopted(note)
                    if note.string_id() == "server.bridge_layout.adopt_console_could_not_open"
            )),
            "and the operator is told: {:?}",
            live.notices
        );
    }

    #[test]
    fn a_rebuild_this_pass_could_not_place_is_faulted_rather_than_quietly_dropped() {
        // The one-frame interleaving that would make "not built" mean "forgotten
        // for ever", built against the real reconciler:
        //
        //   frame N   this pass reaches its grace, closes and recreates the
        //             console — and RESETS its strike counter as it does;
        //   frame N   the pane host drains that pending view later in the SAME
        //             frame, with the slot still missing, and `home_for_pane`
        //             answers `Nowhere(SeatedButUnplaced)`;
        //   frame N+1 the slot comes back, so the health check below (pane open
        //             AND slot present) reads healthy — for ever, over a black
        //             screen, with no retry and no surrender.
        //
        // Which is why the answer carries a RETRY decision, pinned per reason in
        // `panes::placement` because the drain itself is behind
        // `--features ultralight`. This test drives the two halves that ARE
        // compilable — the reconciler, and the fault the decision asks for —
        // across that interleaving, and ends in a retry rather than in the stuck
        // state. The other terminus, a retry budget spent and the seat given
        // back, is the two tests either side of this one.
        use crate::native_host::panes::recovery::{service_faults, PaneFault};
        use crate::native_host::panes::{NoHome, PaneHome};

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let opened = bus
            .open_pane_for_name("helm")
            .expect("the seat opened a console");
        let token = bus.token_of(opened).expect("an open console has a token");
        bus.mark_live(opened);
        bus.take_pending_views();

        // The slot goes away — a monitor between hot-plug frames, a Station
        // window not yet rebuilt — and is kept so it can come back mid-scenario.
        let slots: Vec<Vec<StationPane>> = app
            .world()
            .resource::<BridgeStationSurfaces>()
            .0
            .iter()
            .map(|s| s.panes.clone())
            .collect();
        app.world_mut()
            .resource_mut::<BridgeStationSurfaces>()
            .0
            .iter_mut()
            .for_each(|s| s.panes.clear());

        // Frame N, first half: the grace runs out and the console is rebuilt.
        let mut rebuilt = None;
        for _ in 0..=CONSOLE_MISSING_GRACE_FRAMES {
            app.update();
            if let Some((id, _url)) = bus.take_pending_views().pop() {
                rebuilt = Some(id);
                break;
            }
        }
        let rebuilt = rebuilt.expect("the reconciler rebuilds the console it cannot see");
        assert_eq!(
            bus.name_of(rebuilt).as_deref(),
            Some("helm"),
            "on the same name the pane host looks its home up by"
        );

        // Frame N, second half: the pane host drains that entry, and this is the
        // decision it makes — no home, and a retry rather than a skip.
        let unbuilt = home(&app, "helm", &tempting_tile("helm"));
        assert_eq!(
            unbuilt,
            PaneHome::Nowhere(NoHome::SeatedButUnplaced),
            "never the viewscreen, however tempting the tile"
        );
        let PaneHome::Nowhere(reason) = unbuilt else {
            unreachable!("just asserted")
        };
        assert!(
            reason.should_retry(),
            "and a seated console is faulted, not dropped: the pending entry is \
             already drained, so a skip is the end of the story"
        );

        // Frame N+1: the slot returns, and with it the trap. Nothing is queued
        // to build a view, yet both halves of the health check now pass — so a
        // pane host that had skipped would leave the station card claiming a
        // screen with nothing on it and this pass would never look again.
        for (surface, panes) in app
            .world_mut()
            .resource_mut::<BridgeStationSurfaces>()
            .0
            .iter_mut()
            .zip(slots)
        {
            surface.panes = panes;
        }
        assert!(
            bus.take_pending_views().is_empty(),
            "nothing is queued to build the view that was dropped"
        );
        assert!(
            bus.open_pane_for_name("helm").is_some()
                && app
                    .world()
                    .resource::<BridgeStationSurfaces>()
                    .slot_for("helm")
                    .is_some(),
            "and the reconciler's health check would read HEALTHY: open pane, live \
             slot, black screen"
        );

        // What the retry decision actually does, on #1125's own path: the pane is
        // closed and reopened on the same token, and a view is queued again — for
        // the screen it belongs on.
        bus.fault(rebuilt, PaneFault::ViewCrashed);
        let (retried, _url) = service_faults(&bus)
            .pop()
            .expect("one fault serviced")
            .recreated
            .expect("within the per-identity budget the console is rebuilt");
        assert_eq!(
            bus.token_of(retried).as_deref(),
            Some(token.as_str()),
            "still the same participant, across the reconciler's rebuild and this one"
        );
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![retried],
            "so a view IS queued again: the console retries rather than sitting \
             healthy and black"
        );
        let PaneHome::Station { window, .. } = home(&app, "helm", &tempting_tile("helm")) else {
            panic!("and the retry goes to its own Station window");
        };
        assert_eq!(window, station_window(&app, BENQ));
    }

    #[test]
    fn a_flapping_seated_console_never_touches_the_viewscreen_on_any_of_its_rebuilds() {
        // The bounded-flap path of #1125, asked the #1333 question on every hop.
        // A view that loads then crashes is rebuilt at most
        // MAX_RECREATIONS_PER_WINDOW times and then left closed — and not one of
        // those rebuilds, nor the give-up that follows, is ever aimed at the
        // primary window.
        use crate::native_host::panes::recovery::{
            service_faults, PaneFault, MAX_RECREATIONS_PER_WINDOW,
        };
        use crate::native_host::panes::PaneHome;

        let (mut app, bus) = console_host();
        seat(&mut app, "helm", BENQ);
        app.update();
        let mut current = bus.open_pane_for_name("helm").unwrap();
        let token = bus.token_of(current).unwrap();
        let benq_window = station_window(&app, BENQ);

        let mut rebuilds = 0u32;
        let mut gave_up = false;
        for _ in 0..(MAX_RECREATIONS_PER_WINDOW + 1) {
            bus.fault(current, PaneFault::ViewCrashed);
            let outcome = service_faults(&bus).pop().expect("one fault serviced");
            match outcome.recreated {
                Some((next, _)) => {
                    rebuilds += 1;
                    current = next;
                    bus.mark_live(current);
                    assert_eq!(bus.token_of(current).as_deref(), Some(token.as_str()));
                    assert_eq!(
                        home(&app, "helm", &tempting_tile("helm")),
                        PaneHome::Station {
                            window: benq_window,
                            origin: (0, 0),
                            size: (1920, 1080),
                            scale: 1.0,
                            window_origin: (3840, 0),
                        },
                        "every rebuild goes back to the same screen"
                    );
                }
                None => {
                    assert!(outcome.recreation_exhausted);
                    gave_up = true;
                }
            }
            app.update();
        }
        assert_eq!(rebuilds, MAX_RECREATIONS_PER_WINDOW);
        assert!(gave_up, "and then it stopped, rather than flapping forever");
        assert_eq!(bus.open_count(), 0, "left closed for the operator");

        // The seat is surrendered through the law, with the notice the row
        // renders — the end of the story a `--pane` does not need.
        for _ in 0..CONSOLE_MISSING_GRACE_FRAMES {
            app.update();
        }
        let live = app.world().resource::<BridgeLayoutResource>();
        assert!(live.layout.monitor_of(&station("helm")).is_none());
        assert!(live.notices.iter().any(|n| matches!(
            n,
            LayoutNotice::Adopted(note)
                if note.string_id() == "server.bridge_layout.adopt_console_could_not_open"
        )));
    }

    #[test]
    fn an_unplugged_console_does_not_respawn_and_a_replug_puts_it_back_on_that_screen() {
        // The unplug half, unchanged from #1331 and now stated: the console
        // closes through the ordinary dropped-participant path, NOTHING is
        // queued to rebuild it (which is what "it does not respawn on the
        // viewscreen" actually means at this seam — a display loss is not a
        // fault), and the operator's own row is what brings it back, onto the
        // replugged monitor.
        use crate::native_host::host_lobby::{
            drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
        };
        use crate::native_host::panes::PaneHome;
        use crate::native_host::panes::RecordingSurface;
        use crate::native_host::transport::NativeTransport;

        let (mut app, bus) = console_host();
        let bridge = HostLobbyBridge::new();
        app.insert_resource(HostLobbyBridgeResource(bridge.clone()));
        app.add_message::<crate::lobby::InboundMessage>();
        app.add_systems(PreUpdate, drain_surface_records);

        seat(&mut app, "helm", BENQ);
        app.update();
        let pane = bus.open_pane_for_name("helm").unwrap();
        bus.mark_live(pane);
        let token = bus.token_of(pane).unwrap();
        bus.take_pending_views();

        let benq = benq_entity(&mut app);
        app.world_mut().entity_mut(benq).despawn();
        settle(&mut app);

        assert_eq!(bus.open_count(), 0, "the console closed with its screen");
        assert_eq!(
            bus.transport().poll(),
            vec![crate::native_host::transport::TransportEvent::Disconnected { token }],
            "its crew drops to Backfill through the ordinary disconnect, with no crash"
        );
        assert!(
            bus.take_pending_views().is_empty(),
            "and nothing at all is queued to rebuild it: an unplug is a close, not a fault, \
             so there is no view for the viewscreen to catch"
        );

        // Replug. Still nothing is re-homed automatically.
        app.world_mut()
            .spawn(monitor("BenQ EX", 1920, 1080, 3840, 0));
        settle(&mut app);
        assert_eq!(
            bus.open_count(),
            0,
            "a returned display opens nothing by itself"
        );

        // The operator presses the row's button — the real record, over the real
        // drain — and the console comes back on the screen they replugged.
        let mut surface = RecordingSurface::ready();
        surface.queue_record(
            r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#,
        );
        pump_host_lobby(&bridge, &mut surface);
        app.update();

        let reopened = bus
            .open_pane_for_name("helm")
            .expect("the press opened a console again");
        assert_eq!(
            bus.take_pending_views()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![reopened],
            "with a view queued, which is what a live console is"
        );
        let PaneHome::Station { window, .. } = home(&app, "helm", &tempting_tile("helm")) else {
            panic!("the reopened console belongs on the replugged monitor's window");
        };
        assert_eq!(window, station_window(&app, BENQ));
    }

    #[test]
    fn a_legacy_tiled_pane_still_rebuilds_on_the_primary_window() {
        // Acceptance criterion 3, stated as a test rather than assumed from a
        // suite staying green: a `--pane` on a host with no `--profile` has no
        // Station window anywhere and is seated by no law, so it keeps issue
        // #1125's home — its own tile on the primary window, at exactly the
        // rectangle `init_pane_host` recorded. Nothing #1333 narrowed reaches it.
        use crate::native_host::panes::identity::PaneIdentity;
        use crate::native_host::panes::recovery::{service_faults, PaneFault};
        use crate::native_host::panes::{PaneHome, PaneTile};

        let (mut app, bus) = console_host();
        let ada =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        bus.mark_live(ada);
        // A console is open on a real Station window beside it, so this is not
        // the degenerate host in which every home would be a tile.
        seat(&mut app, "helm", BENQ);
        app.update();

        let tiles = vec![PaneTile {
            name: "Ada".to_string(),
            origin: (960, 0),
            size: (960, 1080),
        }];
        assert_eq!(
            home(&app, "Ada", &tiles),
            PaneHome::PrimaryTile {
                origin: (960, 0),
                size: (960, 1080)
            },
            "the legacy tiling is untouched"
        );

        // And it survives the crash path exactly as it did: closed, recreated on
        // the same identity, and rebuilt on the same tile.
        bus.fault(ada, PaneFault::ViewCrashed);
        let (recreated, _) = service_faults(&bus)
            .pop()
            .expect("one fault serviced")
            .recreated
            .expect("a view crash recreates a tiled pane too");
        assert_eq!(
            bus.token_of(recreated),
            bus.token_of(ada),
            "the same participant"
        );
        assert_eq!(
            home(&app, "Ada", &tiles),
            PaneHome::PrimaryTile {
                origin: (960, 0),
                size: (960, 1080)
            }
        );
    }
}
