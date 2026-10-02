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

use super::super::bridge_profile::{BridgeProfile, DisplayEntry, PROFILE_VERSION, ROLE_VIEWSCREEN};

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
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
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
    let ada = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    bus.mark_live(ada);
    crate::native_host::panes::transport::identify_test_pane(&bus, ada);
    let ada_token = bus.token_of(ada).unwrap();
    let grace =
        bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000002", "Grace").unwrap());
    bus.mark_live(grace);
    crate::native_host::panes::transport::identify_test_pane(&bus, grace);
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
fn booted_on(profile: Option<ValidatedProfile>, monitors: Vec<(Monitor, bool)>) -> (App, Entity) {
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

/// Drive the same logical transition the lobby drain uses, without needing its
/// page bridge in tests whose subject is display following.
fn station_press(app: &mut App, action: LayoutAction) {
    use super::super::console_assignment::{apply_host_action, ConsoleAssignments};
    use super::super::panes::PaneBusResource;
    let layout = app
        .world()
        .resource::<BridgeLayoutResource>()
        .layout
        .clone();
    let bus = app
        .world()
        .get_resource::<PaneBusResource>()
        .map(|bus| bus.0.clone());
    app.world_mut()
        .resource_scope(|world, mut assignments: Mut<ConsoleAssignments>| {
            world.resource_scope(|world, mut claims: Mut<PendingConsoleClaims>| {
                let (next, _) = {
                    let mut sessions = world.get_resource_mut::<crate::lobby::Sessions>();
                    apply_host_action(
                        &layout,
                        &action,
                        Some(&mut assignments),
                        Some(&mut claims),
                        bus.as_ref(),
                        sessions.as_mut().map(|sessions| &mut sessions.0),
                    )
                    .expect("the host accepts the Station action")
                };
                world.resource_mut::<BridgeLayoutResource>().layout = next;
            });
        });
}

/// Open a station's console on a monitor, as a lobby button press does.
fn seat(app: &mut App, station: &str, identity: &str) {
    station_press(
        app,
        LayoutAction::AssignStation {
            station: crate::core::messages::StationId(station.to_owned()),
            monitor: MonitorIdentity::new(identity),
        },
    );
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

#[test]
fn a_gm_only_monitor_keeps_its_role_and_the_viewscreen_hides_until_a_screen_returns() {
    let (mut app, window) = booted(None);
    let mut layout = app.world_mut().resource_mut::<BridgeLayoutResource>();
    layout.layout = layout
        .layout
        .apply(&super::super::bridge_layout::LayoutAction::SetGameMaster {
            monitor: Some(MonitorIdentity::new(BENQ)),
        })
        .unwrap();
    app.update();
    let lost = monitor_entity(&mut app, "DELL U2720Q", 0);
    app.world_mut().entity_mut(lost).despawn();
    for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
        app.update();
    }
    let layout = app.world().resource::<BridgeLayoutResource>();
    assert_eq!(layout.layout.viewscreen().as_str(), DELL);
    assert_eq!(
        layout
            .layout
            .game_master_monitor()
            .map(MonitorIdentity::as_str),
        Some(BENQ)
    );
    assert!(!app.world().entity(window).get::<Window>().unwrap().visible);
    let returned = app
        .world_mut()
        .spawn((monitor("DELL U2720Q", 3840, 2160, 0, 0), PrimaryMonitor))
        .id();
    for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 1 {
        app.update();
    }
    let primary = app.world().entity(window).get::<Window>().unwrap();
    assert!(primary.visible);
    assert_eq!(
        primary.mode,
        WindowMode::BorderlessFullscreen(MonitorSelection::Entity(returned))
    );
    assert_eq!(
        app.world()
            .resource::<BridgeLayoutResource>()
            .layout
            .game_master_monitor()
            .map(MonitorIdentity::as_str),
        Some(BENQ)
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

/// Close a station's console through the production host Off transition.
fn unseat(app: &mut App, station_id: &str) {
    station_press(
        app,
        LayoutAction::UnassignStation {
            station: station(station_id),
        },
    );
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
fn native_screen_reservation_survives_monitor_loss_move_and_ends_only_at_off() {
    use super::super::console_assignment::ConsoleAssignments;
    use crate::native_host::host_lobby::{
        drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
    };
    use crate::native_host::panes::RecordingSurface;
    let (mut app, bus) = console_host();
    app.add_message::<crate::lobby::InboundMessage>();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    let bridge = HostLobbyBridge::new();
    app.insert_resource(HostLobbyBridgeResource(bridge.clone()));
    app.add_systems(PreUpdate, drain_surface_records);
    let mut surface = RecordingSurface::ready();
    surface.queue_record(
        r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#,
    );
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    let pane = bus.open_pane_for_name("helm").unwrap();
    let token = bus.token_of(pane).unwrap();
    assert_eq!(
        app.world()
            .resource::<crate::lobby::Sessions>()
            .0
            .native_station_for_token(&token),
        Some(&station("helm"))
    );
    let monitor = app
        .world_mut()
        .query::<(Entity, &Monitor)>()
        .iter(app.world())
        .find(|(_, m)| m.name.as_deref() == Some("BenQ EX"))
        .unwrap()
        .0;
    app.world_mut().entity_mut(monitor).despawn();
    for _ in 0..=DISPLAY_LOSS_DEBOUNCE_FRAMES {
        app.update();
    }
    assert!(bus.open_pane_for_name("helm").is_none());
    assert_eq!(
        app.world()
            .resource::<ConsoleAssignments>()
            .monitor_for(&station("helm")),
        Some(&MonitorIdentity::new(BENQ))
    );
    assert_eq!(
        app.world()
            .resource::<crate::lobby::Sessions>()
            .0
            .native_station_for_token(&token),
        Some(&station("helm"))
    );
    surface.queue_record(
        r#"{"kind":"assign-station","station":"helm","monitor":"ACME 1080@1920x1080"}"#,
    );
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert_eq!(
        bus.token_of(bus.open_pane_for_name("helm").unwrap())
            .as_deref(),
        Some(token.as_str())
    );
    surface.queue_record(r#"{"kind":"unassign-station","station":"helm"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert!(bus.open_pane_for_name("helm").is_none());
    assert!(bus.console_assignments().is_empty());
    assert!(app.world().resource::<ConsoleAssignments>().is_empty());
    assert!(app
        .world()
        .resource::<crate::lobby::Sessions>()
        .0
        .native_station_for_token(&token)
        .is_none());
    assert!(
        bus.recreate(pane).is_none(),
        "a stale fault cannot resurrect an Off screen"
    );
}

#[test]
fn native_screen_assignment_refuses_to_displace_a_connected_phone() {
    use crate::native_host::host_lobby::{
        drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
    };
    let (mut app, bus) = console_host();
    app.add_message::<crate::lobby::InboundMessage>();
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("phone".into(), "Ada".into()).unwrap();
    sessions.set_station("phone", Some(station("helm")));
    app.insert_resource(crate::lobby::Sessions(sessions));
    let bridge = HostLobbyBridge::new();
    app.insert_resource(HostLobbyBridgeResource(bridge.clone()));
    app.add_systems(PreUpdate, drain_surface_records);
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(
        r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#,
    );
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert_eq!(bus.open_count(), 0);
    assert!(app
        .world()
        .resource::<BridgeLayoutResource>()
        .notices
        .iter()
        .any(
            |notice| matches!(notice, LayoutNotice::StationHeld { holder, .. } if holder == "Ada")
        ));
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
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
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
    crate::native_host::panes::transport::identify_test_pane(&bus, helm_pane);
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
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
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
    let ada = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000042", "Ada").unwrap());
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
        let pane =
            bus.open(PaneIdentity::adopt(&uuid, &slot.label).expect("a well-formed pane identity"));
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
fn a_superseded_console_is_not_recreated_or_unseated_when_its_slot_is_missing() {
    use crate::native_host::connections::SharedConnections;
    use crate::native_host::transport::NativeTransport;
    let (mut app, bus) = console_host();
    let shared = SharedConnections::default();
    bus.transport().share_connections(shared.clone());
    seat(&mut app, "helm", BENQ);
    app.update();
    let pane = bus.open_pane_for_name("helm").unwrap();
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
    bus.mark_live(pane);
    bus.take_pending_views();
    let token = bus.token_of(pane).unwrap();
    {
        let mut connections = shared.lock();
        let leg = connections.new_leg();
        let phone = connections.open(leg);
        connections.bind(phone, &token).unwrap();
    }
    app.world_mut()
        .resource_mut::<BridgeStationSurfaces>()
        .0
        .iter_mut()
        .for_each(|surface| surface.panes.clear());
    for _ in 0..CONSOLE_MISSING_GRACE_FRAMES * 5 {
        app.update();
    }
    assert!(bus.is_superseded(pane));
    assert_eq!(bus.open_pane_for_name("helm"), Some(pane));
    assert!(bus.take_pending_views().is_empty());
    let layout = app.world().resource::<BridgeLayoutResource>();
    assert_eq!(
        layout.layout.monitor_of(&station("helm")),
        Some(&MonitorIdentity::new(BENQ))
    );
    assert!(layout.notices.is_empty());

    // A neighbour changes the split, but nobody reopened the superseded
    // console. The geometry rebuild sweep must not mint a fresh participant.
    seat(&mut app, "weapons", BENQ);
    app.update();
    assert_eq!(bus.open_pane_for_name("helm"), Some(pane));
    let new_views = bus.take_pending_views();
    assert_eq!(new_views.len(), 1, "only the new neighbour gets a view");
    assert_eq!(bus.name_of(new_views[0].0).as_deref(), Some("weapons"));
    unseat(&mut app, "weapons");
    app.update();
    assert_eq!(bus.open_pane_for_name("helm"), Some(pane));
    assert!(bus.take_pending_views().is_empty());

    let benq = benq_entity(&mut app);
    {
        let mut entity = app.world_mut().entity_mut(benq);
        let mut monitor = entity.get_mut::<Monitor>().unwrap();
        monitor.physical_width = 1280;
        monitor.physical_height = 1024;
    }
    app.update();
    assert_eq!(bus.open_pane_for_name("helm"), Some(pane));
    assert!(bus.is_superseded(pane));
    assert!(bus.take_pending_views().is_empty());
    assert_eq!(rect_of(&app, "helm").width, 1280);
    assert!(app
        .world()
        .resource::<BridgeLayoutResource>()
        .notices
        .is_empty());

    // Only an explicit off/on creates a new console, with a new identity.
    unseat(&mut app, "helm");
    app.update();
    seat(&mut app, "helm", BENQ);
    app.update();
    let reopened = bus.open_pane_for_name("helm").unwrap();
    assert_ne!(reopened, pane);
    assert_ne!(bus.token_of(reopened).as_deref(), Some(token.as_str()));
    assert!(!bus.is_superseded(reopened));
}

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

    let PaneHome::Station { window, size, .. } = home(&app, "helm", &tempting_tile("helm")) else {
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
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
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
    let ada = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
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
