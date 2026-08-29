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
    identify, pane_rects, present_assigned_identities, resolve, runtime_display_losses,
    runtime_display_returns, DisplayRole, MonitorGeometry, MonitorIdentity, PaneRect, RawMonitor,
    RuntimeDisplayLoss, ValidatedProfile,
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
/// entity, the monitor geometry it covers, and the pane rectangles laid out
/// across it.
#[derive(Clone, Debug)]
pub struct BridgeStationSurface {
    pub identity: String,
    pub window: Entity,
    /// The monitor's live geometry — its scale factor and its top-left on the
    /// virtual desktop. Carried so the pane host (issue #1124) can composite each
    /// pane at the right physical size and route input in the monitor's own
    /// coordinate space without re-querying the winit monitor.
    pub geometry: MonitorGeometry,
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
        )
        .add_systems(
            Update,
            // Only after the profile has been applied, so the watcher's first
            // observation is the baseline it diffs against — see the system's
            // own note. A no-op until then, and on a host with no profile.
            watch_runtime_displays
                .run_if(resource_exists::<BridgeDisplayConfig>)
                .run_if(resource_exists::<BridgeDisplayApplied>),
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
        geometry: MonitorGeometry,
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
                    geometry: geometry.clone(),
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
            geometry: spawn.geometry,
            panes: spawn.panes,
        });
    }

    world.insert_resource(BridgeStationSurfaces(station_surfaces));
    world.insert_resource(BridgeDisplayApplied);
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
/// [`VIEW_CRASH_COPY_FAILURES`], deliberately shorter because a genuine unplug
/// should still reach Backfill promptly, and a returning monitor before the
/// window is up costs nothing but a cleared counter.
///
/// (The echoed threshold is the pane host's own `VIEW_CRASH_COPY_FAILURES`, a
/// `ultralight`-gated const, so it is named in prose rather than intra-doc
/// linked from this always-compiled module.)
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
fn watch_runtime_displays(
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    config: Res<BridgeDisplayConfig>,
    bus: Option<Res<crate::native_host::panes::PaneBusResource>>,
    log: Option<Res<LogFilterConfig>>,
    mut baseline: Local<Option<std::collections::HashSet<MonitorIdentity>>>,
    mut absent_streak: Local<std::collections::HashMap<MonitorIdentity, u32>>,
) {
    let raws: Vec<RawMonitor> = monitors
        .iter()
        .map(|(m, primary)| raw_from_monitor(m, primary))
        .collect();
    if raws.is_empty() {
        // No monitors reported this frame — winit has not populated them yet, or
        // a transient empty frame during a hot-plug. Treating that as "every
        // display was lost" would be wrong, so wait for a frame that has some.
        return;
    }

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
                    panes: vec![PaneSlot {
                        label: station_label.to_string(),
                    }],
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
                DisplayEntry {
                    id: "ACME 1080@1920x1080#0,0".to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot {
                        label: "Ada".to_string(),
                    }],
                },
                DisplayEntry {
                    id: "ACME 1080@1920x1080#1920,0".to_string(),
                    role: ROLE_STATION.to_string(),
                    split: None,
                    panes: vec![PaneSlot {
                        label: "Grace".to_string(),
                    }],
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
}
