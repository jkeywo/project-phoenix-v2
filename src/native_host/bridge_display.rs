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
//! exclusive system that runs every frame and does its work once: it returns
//! quietly while no monitors are known yet, then applies the profile and inserts
//! [`BridgeDisplayApplied`] so it never runs again. It is exclusive because it
//! reads the monitor entities, mutates the primary window, spawns new windows and
//! inserts resources in one pass — the same shape `panes::ultralight::init_pane_host`
//! uses for the same reason.

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PrimaryMonitor, PrimaryWindow, Window, WindowMode};

use crate::logging::{LogCat, LogFilterConfig};

use super::bridge_profile::{
    identify, pane_rects, resolve, DisplayRole, MonitorGeometry, PaneRect, RawMonitor,
    ValidatedProfile,
};

/// The validated bridge profile a native host applies to its displays.
///
/// Inserted by `native_host::app` only when `--profile` gave one and it
/// validated — a bad profile fails at the prompt (issue #1123), so by the time
/// this resource exists its roles and density are already sound. Absent means
/// the single-window #1121 behaviour: the viewscreen is the one default window
/// and no Station windows open.
#[derive(Resource, Clone, Debug)]
pub struct BridgeDisplayConfig {
    pub profile: ValidatedProfile,
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
}

/// A Station surface this adapter opened: its monitor identity, the window
/// entity, and the pane rectangles laid out across it.
#[derive(Clone, Debug)]
pub struct BridgeStationSurface {
    pub identity: String,
    pub window: Entity,
    pub panes: Vec<StationPane>,
}

/// Every Station surface the applied profile opened.
///
/// Published for the pane host to consume. **The pane→Station-window compositing
/// is the documented continuation of this work**, sharing #1124's input-routing
/// concern: this slice opens the Station windows and computes each pane's
/// rectangle (both testable — the geometry purely, the windows under the ignored
/// integration test), and leaves the Ultralight pane rendering pointed at those
/// windows for the follow-on. Until then a host launched with both `--profile`
/// and `--pane` opens the Station windows AND tiles the panes on the viewscreen
/// window as #1122 always did; nothing regresses, and this resource is the seam
/// the compositing step reads.
#[derive(Resource, Clone, Debug, Default)]
pub struct BridgeStationSurfaces(pub Vec<BridgeStationSurface>);

/// Set once [`apply_bridge_profile`] has run, so it does its work exactly once.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct BridgeDisplayApplied;

/// Installs the bridge-display adapter.
///
/// A no-op without a [`BridgeDisplayConfig`], so a host can add it
/// unconditionally; `native_host::app` only inserts the config when a profile was
/// given. The system is gated on the config existing so an app with none never
/// touches a window.
pub struct BridgeDisplayPlugin;

impl Plugin for BridgeDisplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            apply_bridge_profile.run_if(resource_exists::<BridgeDisplayConfig>),
        );
    }
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

/// Read the present monitors, resolve the profile against them, and open one
/// borderless-fullscreen surface per configured monitor.
///
/// Runs once — see the [module note](self#why-an-exclusive-system-and-why-it-retries).
pub fn apply_bridge_profile(world: &mut World) {
    if world.get_resource::<BridgeDisplayApplied>().is_some() {
        return;
    }
    let Some(config) = world.get_resource::<BridgeDisplayConfig>().cloned() else {
        return;
    };
    let log = world.get_resource::<LogFilterConfig>().cloned();

    // The monitors as winit reports them, paired with their entities. Cloned out
    // so no query borrow is held while we mutate windows and spawn below.
    let mut monitors: Vec<(Entity, RawMonitor, MonitorGeometry)> = world
        .query::<(Entity, &Monitor, Has<PrimaryMonitor>)>()
        .iter(world)
        .map(|(e, m, primary)| (e, raw_from_monitor(m, primary), geometry_of(m)))
        .collect();
    if monitors.is_empty() {
        // winit has not populated the monitor list yet — try again next frame.
        return;
    }
    // A stable order so identical-monitor disambiguation is deterministic;
    // `identify` disambiguates by position, so this only affects the reported
    // order, never the identities.
    monitors.sort_by_key(|(_, r, _)| (r.position_x, r.position_y));

    let raws: Vec<RawMonitor> = monitors.iter().map(|(_, r, _)| r.clone()).collect();
    let discovered = identify(&raws);
    let resolved = resolve(&config.profile, &discovered);

    // identity → (monitor entity, geometry), for turning a resolved surface back
    // into the winit monitor it names.
    let by_identity: std::collections::HashMap<String, (Entity, MonitorGeometry)> = discovered
        .iter()
        .zip(monitors.iter())
        .map(|(d, (e, _, g))| (d.identity.as_str().to_string(), (*e, g.clone())))
        .collect();

    for problem in &resolved.problems {
        crate::pwarn!(log, LogCat::Lobby, "bridge display: {problem}");
    }

    // The viewscreen goes on the primary window; the Stations get their own.
    let mut station_surfaces: Vec<BridgeStationSurface> = Vec::new();
    let mut viewscreen_monitor: Option<(Entity, String, String)> = None;
    struct StationSpawn {
        monitor: Entity,
        identity: String,
        role: String,
        panes: Vec<StationPane>,
    }
    let mut station_spawns: Vec<StationSpawn> = Vec::new();

    for surface in &resolved.surfaces {
        let Some((monitor_entity, geometry)) = by_identity.get(surface.identity.as_str()) else {
            continue;
        };
        match &surface.role {
            DisplayRole::Viewscreen => {
                viewscreen_monitor = Some((
                    *monitor_entity,
                    surface.identity.as_str().to_string(),
                    surface.role.summary(),
                ));
            }
            DisplayRole::Station { split, panes } => {
                let rects = pane_rects(geometry, *split, panes.len());
                let laid_out: Vec<StationPane> = panes
                    .iter()
                    .zip(rects)
                    .map(|(slot, rect)| StationPane {
                        label: slot.label.clone(),
                        rect,
                    })
                    .collect();
                station_spawns.push(StationSpawn {
                    monitor: *monitor_entity,
                    identity: surface.identity.as_str().to_string(),
                    role: surface.role.summary(),
                    panes: laid_out,
                });
            }
        }
    }

    // Place the viewscreen on the primary window.
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
            "bridge display: the profile assigns no present monitor the viewscreen role, so the \
             shared 3-D view has nowhere to draw. Check the profile against `--setup`."
        );
    }

    // Open one borderless-fullscreen window per Station.
    for spawn in station_spawns {
        let window = world
            .spawn((
                Window {
                    title: format!("{} — Station", super::WINDOW_TITLE),
                    name: Some(format!("phoenix-station-{}", spawn.identity)),
                    mode: WindowMode::BorderlessFullscreen(MonitorSelection::Entity(spawn.monitor)),
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
            panes: spawn.panes,
        });
    }

    world.insert_resource(BridgeStationSurfaces(station_surfaces));
    world.insert_resource(BridgeDisplayApplied);
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
/// process exit code: 0 once monitors were reported, 1 if none were after
/// [`SETUP_FRAME_BUDGET`] frames.
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
    // Operator output, on the same footing as `phoenix-host`'s other CLI prints:
    // stdout, not the `plog!` family.
    print!("{report}");
    if discovered.is_empty() {
        eprintln!(
            "phoenix-host --setup: no monitors were reported after {} frames",
            frames.0
        );
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
