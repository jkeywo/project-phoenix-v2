use super::*;

/// A bare `App` with the plugin's own resources and messages, and nothing
/// else. `NativeRenderSurface::Contract` is a real composition with no
/// `InputPlugin`, so the key system must be absent here rather than
/// panicking — which is the arrangement this fixture reproduces.
fn app_with_lobby() -> (App, HostLobbyBridge) {
    let bridge = HostLobbyBridge::new();
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .add_message::<LobbyStateChanged>()
        .add_message::<crate::lobby::InboundMessage>()
        .init_state::<GamePhase>()
        .insert_resource(HostLobbyBridgeResource(bridge.clone()))
        .add_plugins(HostLobbyPlugin);
    (app, bridge)
}

/// Read every `InboundMessage` this frame wrote, without consuming the
/// reader the systems under test share.
fn inbound(app: &mut App) -> Vec<crate::lobby::InboundMessage> {
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::InboundMessage>>();
    messages.iter_current_update_messages().cloned().collect()
}

#[test]
fn a_pick_made_on_the_surface_arrives_as_the_host_pages_own_picker_would() {
    // Issue #1328. The surface is not a participant, so what it sends is a
    // `HostLobbyRecord` rather than a `ClientMessage` — but what reaches the
    // arbiter has to be indistinguishable from the host PAGE's own picker,
    // which submits under `LOCAL_CONSOLE_TOKEN` through
    // `__localConsoleSend`. Anything else would be a second sender the
    // first-valid-wins rule had never heard of.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
    surface.queue_record(r#"{"kind":"select_ship","template_path":"assets/a.toml"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    let sent = inbound(&mut app);
    assert_eq!(sent.len(), 2);
    for message in &sent {
        assert_eq!(message.token, crate::console_bridge::LOCAL_CONSOLE_TOKEN);
    }
    assert!(matches!(
        &sent[0].msg,
        crate::core::messages::ClientMessage::SelectScenario { scenario_id }
            if scenario_id == "combat_test"
    ));
    assert!(matches!(
        &sent[1].msg,
        crate::core::messages::ClientMessage::SelectPlayerShip { template_path }
            if template_path == "assets/a.toml"
    ));
}

#[test]
fn a_record_this_bridge_does_not_speak_reaches_no_message_bus() {
    // A `ClientMessage` envelope is the shape most likely to arrive here by
    // mistake, and is the one that must NOT be forwarded: the surface holds
    // no session token, so admitting one would be admitting a participant
    // nobody identified.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"type":"SetReady","data":{"ready":true}}"#);
    surface.queue_record("not json at all");
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    assert!(inbound(&mut app).is_empty());
}

#[test]
fn the_launch_control_sets_the_latch_the_browser_button_sets() {
    // One force-start rule, not two: the record only raises
    // `PendingForceStart`, and `server::bridge::apply_force_start` — the
    // browser host's own, de-wasm-gated — decides everything else.
    let (mut app, bridge) = app_with_lobby();
    app.init_resource::<crate::server::bridge::PendingForceStart>();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"force_start"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    assert!(
        app.world()
            .resource::<crate::server::bridge::PendingForceStart>()
            .0
    );
    assert!(
        inbound(&mut app).is_empty(),
        "a launch is a host-side latch, not a participant's command"
    );
}

/// Every `AppExit` this frame wrote, without consuming the reader the app
/// itself uses to decide it is finished.
fn exits(app: &mut App) -> Vec<AppExit> {
    app.world()
        .resource::<Messages<AppExit>>()
        .iter_current_update_messages()
        .cloned()
        .collect()
}

#[test]
fn a_confirmed_exit_writes_the_ordinary_app_exit_and_nothing_else() {
    // Issue #1365. The whole mechanism: the surface's confirmed press
    // becomes `AppExit::Success` — the same message
    // `bridge_display::setup_enumerate` writes to end `--setup` — and the
    // binary's existing teardown then runs unchanged, because the run ends
    // through the door that was already there. Nothing here withdraws a
    // document or stops the delivery service, and this asserts that too:
    // a second teardown path would be a second thing to keep in step.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"exit_desktop"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    assert_eq!(exits(&mut app), vec![AppExit::Success]);
    assert!(
        inbound(&mut app).is_empty(),
        "quitting is the host's own act, not a participant's command"
    );
}

#[test]
fn opening_the_exit_route_asks_rather_than_quitting() {
    // The confirmation is the point of the slice: the ENTRY only opens the
    // stage that asks (`landing_open`), and it is the stage's own control
    // that sends `exit_desktop`. A host that quit on the menu press would
    // be a host with no confirmation at all, whatever the page drew.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"landing_open","entry":"exit_desktop"}"#);
    surface.queue_record(r#"{"kind":"landing_close"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    assert!(exits(&mut app).is_empty());
}

/// A shelf pointed at a folder that cannot exist.
///
/// No temp directory, and none needed: every claim below is about what
/// happens to a NAME the surface sent, and the shelf that name is looked up
/// in is empty either way. The rules about which files reach a shelf are
/// `native_host::mod_packs`'s, tested there without a filesystem at all.
fn empty_shelf() -> packs::ModPackShelfResource {
    packs::ModPackShelfResource::new(
        "no-such-mod-folder-for-issue-1366",
        ".",
        "assets/scenarios.toml",
    )
}

#[test]
fn a_shelf_is_put_on_the_surface_on_the_first_frame_and_not_again() {
    // Issue #1366's push rule: the landing must show the folder without
    // waiting for anybody to touch it, and an idle host must then cost the
    // surface nothing — the same arrangement the picker and the landing
    // beside it make.
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(empty_shelf());
    let mut surface = crate::native_host::panes::RecordingSurface::ready();

    let shelves = |surface: &crate::native_host::panes::RecordingSurface| {
        surface
            .pushed
            .iter()
            .filter(|s| s.contains("__phoenixHostLobbyPacks("))
            .cloned()
            .collect::<Vec<_>>()
    };

    app.update();
    pump_host_lobby(&bridge, &mut surface);
    let first = shelves(&surface);
    assert_eq!(first.len(), 1);
    assert!(first[0].contains("no-such-mod-folder-for-issue-1366"));
    // The folder is not there, so the panel says WHICH emptiness this is
    // rather than showing an empty list that reads as "there are no packs".
    assert!(first[0].contains("scan_error"));

    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert_eq!(shelves(&surface).len(), 1, "an idle shelf pushes nothing");
}

#[test]
fn a_host_with_no_shelf_never_offers_one() {
    // The whole of how the landing's Load-mod-pack row stays inert on a host
    // started without `--mod-pack-dir`: no resource, no push, and therefore
    // nothing that tells the surface it can answer the row. Not a flag, and
    // not a check on the page.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert!(surface
        .pushed
        .iter()
        .all(|s| !s.contains("__phoenixHostLobbyPacks(")));
}

#[test]
fn a_pack_this_host_never_offered_is_refused_and_says_so_on_the_panel() {
    // The gate, end to end. A name off the bridge is looked up in the shelf
    // THIS host produced and never joined onto the scanned directory, so
    // "never offered", "invented by a newer page" and "deleted since the
    // scan" are one refusal — and it is a refusal the operator can read,
    // not a silent drop.
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(empty_shelf());
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"install_mod_pack","pack":"../../secrets.zip"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    let shelf = app.world().resource::<packs::ModPackShelfResource>();
    assert_eq!(shelf.attempted.as_deref(), Some("../../secrets.zip"));
    assert!(!shelf.accepted);
    assert_eq!(shelf.findings.len(), 1);
    assert_eq!(shelf.findings[0].category, "unknown-pack");
    assert_eq!(shelf.findings[0].severity, "error");
    assert!(
        shelf.pending.is_none(),
        "the latch is answered, not left set"
    );
    // …and the answer reaches the surface in the same frame the press did.
    pump_host_lobby(&bridge, &mut surface);
    let packs_push = surface
        .pushed
        .iter()
        .rev()
        .find(|s| s.contains("__phoenixHostLobbyPacks("))
        .expect("the panel is told what happened");
    assert!(packs_push.contains("unknown-pack"));
    assert!(packs_push.contains(r#""accepted":false"#));
}

#[test]
fn a_pack_chosen_on_a_host_with_no_shelf_is_dropped_rather_than_panicking() {
    // The resource is `Option` in the drain for the reason every other one
    // is: a bundle can be newer than the binary serving it, and a record for
    // a feature this host was not started with must be a line in the log
    // rather than a missing-resource panic on the simulation's own thread.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"install_mod_pack","pack":"anything.zip"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert!(inbound(&mut app).is_empty());
    assert!(exits(&mut app).is_empty());
}

#[test]
fn choosing_a_pack_is_not_a_participants_command_and_never_becomes_one() {
    // The surface holds no session token. An install rearranges this host's
    // own content overlay, exactly as a layout press rearranges its own
    // screens — it is not something a participant may say, and nothing here
    // may quietly turn it into one.
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(empty_shelf());
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"install_mod_pack","pack":"thin-margin.zip"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert!(inbound(&mut app).is_empty());
}

#[test]
fn a_composition_with_no_force_start_latch_ignores_the_press_rather_than_panicking() {
    // `app_with_lobby` is deliberately bare: the resource is `Option` in the
    // drain because a headless-shaped composition need not carry it, and a
    // missing resource on a system that RUNS is a Bevy panic rather than an
    // inert system.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"force_start"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
}

/// The primary window's mode, read the way `bridge_display`'s own tests
/// read it — by the entity the fixture spawned, so a second window arriving
/// later could never make this ambiguous.
fn window_mode(app: &App, window: Entity) -> bevy::window::WindowMode {
    app.world()
        .entity(window)
        .get::<bevy::window::Window>()
        .unwrap()
        .mode
}

#[test]
fn the_fullscreen_control_moves_the_primary_window_and_moves_it_back() {
    // Issue #1367, and the half `fullscreen.rs`'s unit tests cannot reach:
    // those prove what the two pure functions DECIDE, and this proves the
    // wiring around them — the record latches in `PreUpdate`, and
    // `apply_window_mode_toggle` spends the latch on the primary window in
    // `Update` of the SAME frame. Without it, deleting the dispatcher's arm
    // or dropping the applier out of `HostLobbyPlugin`'s chain leaves the
    // whole suite green and the answer to "does the window actually move?"
    // written down nowhere.
    use bevy::window::{MonitorSelection, PrimaryWindow, Window, WindowMode};

    let (mut app, bridge) = app_with_lobby();
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    assert_eq!(window_mode(&app, window), WindowMode::Windowed);

    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
    pump_host_lobby(&bridge, &mut surface);

    // ONE frame: the drain latches in `PreUpdate` and the applier spends it
    // in `Update`, so an operator's press is answered in the frame it
    // arrived rather than the next one.
    app.update();
    assert_eq!(
        window_mode(&app, window),
        // `Current` because nothing has assigned this window a display:
        // the pure functions' own tests say why that is the only honest
        // answer, and this says the applier asks them.
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    );
    assert!(
        !app.world()
            .resource::<fullscreen::WindowModeToggle>()
            .pending,
        "the frame that applies a press clears the latch, or the window would flip again"
    );
    assert!(
        inbound(&mut app).is_empty(),
        "a window mode is this host's own, never a participant's command"
    );

    // And back, which is the whole control: a press is a TOGGLE, and the
    // second one has to read the mode the first one wrote.
    surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert_eq!(window_mode(&app, window), WindowMode::Windowed);
}

#[test]
fn a_host_with_no_primary_window_drops_the_fullscreen_press_rather_than_panicking() {
    // The sibling of the force-start case above, and `app_with_lobby` is
    // bare in the same way: a delivery-only or headless composition carries
    // the lobby surface and no window at all, so the applier's window query
    // matches nothing every time it runs. That is one warning, not a panic
    // — and the latch is spent either way, because a press held until the
    // day a window appeared would be a control acting minutes after it was
    // pressed.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert!(
        !app.world()
            .resource::<fullscreen::WindowModeToggle>()
            .pending
    );
}

#[test]
fn a_composition_with_no_window_mode_latch_ignores_the_press_rather_than_panicking() {
    // The drain's OTHER guard: `window_mode` is `Option<ResMut<_>>` there
    // for the reason `force_start` is, and while `HostLobbyPlugin` always
    // inserts the latch today, the arm that answers its absence is reached
    // by any composition that runs `drain_surface_records` without it. A
    // dispatcher that panicked on a record it could not answer would take
    // every other record in the same batch down with it.
    let (mut app, bridge) = app_with_lobby();
    app.world_mut()
        .remove_resource::<fullscreen::WindowModeToggle>();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
}

#[test]
fn a_host_with_no_catalogue_never_publishes_a_picker() {
    // A `--world` host holds no `LobbyScenarioCatalog`, which is what leaves
    // its `#scenario-panel` at the `display: none` the document assembled it
    // with — the flag that decides the world skipping the stage it decides.
    let (mut app, bridge) = app_with_lobby();
    app.update();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    assert!(
        !surface
            .pushed
            .iter()
            .any(|s| s.contains("__phoenixHostLobbyScenario(")),
        "no catalogue, no picker: {:?}",
        surface.pushed
    );
}

#[test]
fn a_host_with_no_catalogue_never_publishes_a_landing_either() {
    // Issue #1361, and the same fact doing the same job one layer out: a
    // `--world` host holds no `LobbyScenarioCatalog`, so it never pushes a
    // landing and `#landing-panel` stays at the `display: none` the
    // document assembled it with. An operator who was told what they are
    // flying at the prompt is not shown a menu asking.
    let (mut app, bridge) = app_with_lobby();
    app.update();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    assert!(
        !surface
            .pushed
            .iter()
            .any(|s| s.contains("__phoenixHostLobbyLanding(")),
        "no catalogue, no front door: {:?}",
        surface.pushed
    );
}

#[test]
fn a_world_less_host_puts_the_landing_on_screen_and_a_landed_world_takes_it_away() {
    // The acceptance criterion, as the two pushes that make it true. The
    // first frame reveals the front door; the frame a `WorldConfig` lands
    // dismisses it, because the crew lobby underneath sits at z-index 180
    // and a landing left up would cover the thing the room is watching.
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(LobbyScenarioCatalog::default());
    app.insert_resource(LobbySelection::default());
    app.update();

    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    let first = surface
        .pushed
        .iter()
        .find(|s| s.contains("__phoenixHostLobbyLanding("))
        .expect("a world-less host shows its front door")
        .clone();
    assert!(first.contains(r#""dismissed":false"#), "{first}");
    assert!(
        first.contains(landing::BUILD_ID),
        "the status bar names THIS binary's build: {first}"
    );

    // Nothing moved: no second push, because the surface already has the
    // answer and every push is main-thread time inside a browser engine.
    surface.pushed.clear();
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert!(
        !surface
            .pushed
            .iter()
            .any(|s| s.contains("__phoenixHostLobbyLanding(")),
        "an idle landing costs the simulation nothing: {:?}",
        surface.pushed
    );

    // …and the world lands.
    app.insert_resource(crate::world::config::WorldConfig::default());
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    let last = surface
        .pushed
        .iter()
        .rev()
        .find(|s| s.contains("__phoenixHostLobbyLanding("))
        .expect("a committed world dismisses the landing")
        .clone();
    assert!(last.contains(r#""dismissed":true"#), "{last}");
}

#[test]
fn a_landing_press_is_drained_by_the_one_reader_and_reaches_no_message_bus() {
    // Issue #1361. The menu's two records ride the same queue as the picks
    // beside them — `take_records` is a drain with one reader, so a new
    // control is a variant and never a second queue. What they must NOT do
    // is reach the arbiter: opening a route on the front door is not a
    // participant's command, and New Game's actual effect is the
    // `#scenario-panel` this surface already carries, whose picks take the
    // world-load path they always took.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"landing_open","entry":"new_game"}"#);
    surface.queue_record(r#"{"kind":"landing_close"}"#);
    surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
    pump_host_lobby(&bridge, &mut surface);

    app.update();
    let sent = inbound(&mut app);
    assert_eq!(
        sent.len(),
        1,
        "only the pick is a message; the two menu presses are not"
    );
    assert!(matches!(
        &sent[0].msg,
        crate::core::messages::ClientMessage::SelectScenario { scenario_id }
            if scenario_id == "combat_test"
    ));
    // Drained rather than left on the queue: a record the host does not act
    // on still has to be taken, or the surface sits on a growing backlog.
    assert!(bridge.take_records().is_empty());
}

#[test]
fn a_lobby_state_push_reaches_the_bridge_as_the_bytes_the_web_host_reads() {
    let (mut app, bridge) = app_with_lobby();
    app.world_mut().write_message(LobbyStateChanged {
        json: r#"{"phase":"Lobby","crew_count":2}"#.to_string(),
    });
    app.update();
    assert!(bridge.has_pending());
}

#[test]
fn the_plugin_runs_without_an_input_plugin_because_a_contract_host_has_none() {
    // Bevy validates a system's parameters when it runs, so a bare
    // `Res<ButtonInput<KeyCode>>` in an app with no `InputPlugin` is a
    // panic in a host that would otherwise be fine.
    let (mut app, _) = app_with_lobby();
    app.update();
    app.update();
}

#[test]
fn mission_start_reaches_the_reveal_state_from_the_simulations_own_phase() {
    let (mut app, _) = app_with_lobby();
    app.update();
    assert!(
        app.world()
            .resource::<HostLobbyRevealResource>()
            .0
            .presence()
            .composited
    );

    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    app.update();
    let reveal = app.world().resource::<HostLobbyRevealResource>().0.clone();
    assert_eq!(reveal.phase(), &GamePhase::InProgress);
    assert!(!reveal.presence().composited);
}

#[test]
fn the_reveal_flag_is_pushed_only_when_it_changes() {
    // A state, not an edge: re-asserting it every frame would be a repaint
    // a frame of a decision nobody made.
    let (mut app, bridge) = app_with_lobby();
    app.update();
    assert!(bridge.has_pending(), "the first frame states the baseline");
    // Drain it the way the frame loop would.
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    assert!(!bridge.has_pending());

    app.update();
    app.update();
    assert!(!bridge.has_pending(), "an unchanged reveal says nothing");
}

#[test]
fn a_phones_qr_toggle_reaches_the_surface() {
    // Issue #1329's AC3, end to end on the Rust side: the same button on
    // the same phone that a browser host answers in JavaScript arrives here
    // as a decoded message and leaves as a push the document answers.
    let (mut app, bridge) = app_with_lobby();
    app.update();
    // Drain the baseline reveal the first frame states.
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    surface.pushed.clear();

    app.world_mut().write_message(crate::lobby::InboundMessage {
        token: "phone-1".to_string(),
        msg: crate::core::messages::ClientMessage::ToggleQrCode,
    });
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert_eq!(
        surface.pushed,
        vec!["window.__phoenixHostLobbyQrToggle()".to_string()]
    );
}

#[test]
fn two_phone_presses_in_one_frame_are_two_flips() {
    // Which is a no-op, and is exactly what the person pressing twice
    // expects. Collapsing them to "somebody asked" would turn a double-tap
    // into a single flip.
    let (mut app, bridge) = app_with_lobby();
    app.update();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    surface.pushed.clear();

    for _ in 0..2 {
        app.world_mut().write_message(crate::lobby::InboundMessage {
            token: "phone-1".to_string(),
            msg: crate::core::messages::ClientMessage::ToggleQrCode,
        });
    }
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert_eq!(surface.pushed.len(), 2);
}

#[test]
fn a_phones_qr_toggle_does_not_uncover_the_surface_mid_mission() {
    // A phone in a player's pocket must not be able to drop a black sheet
    // over a running viewscreen: the surface composites into an OPAQUE
    // texture, so revealing it in play covers the mission. F9 stays the
    // only thing that uncovers it; the press sets what the operator finds
    // when they do.
    let (mut app, _bridge) = app_with_lobby();
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    app.update();

    app.world_mut().write_message(crate::lobby::InboundMessage {
        token: "phone-1".to_string(),
        msg: crate::core::messages::ClientMessage::ToggleQrCode,
    });
    app.update();
    assert!(
        !app.world()
            .resource::<HostLobbyRevealResource>()
            .0
            .presence()
            .composited
    );
}

#[test]
fn other_client_messages_are_not_mistaken_for_the_qr_toggle() {
    let (mut app, bridge) = app_with_lobby();
    app.update();
    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    surface.pushed.clear();

    app.world_mut().write_message(crate::lobby::InboundMessage {
        token: "phone-1".to_string(),
        msg: crate::core::messages::ClientMessage::ReleaseStation,
    });
    app.update();
    pump_host_lobby(&bridge, &mut surface);
    assert!(surface.pushed.is_empty());
}

#[test]
fn the_surface_id_is_not_one_the_pane_registry_can_mint() {
    // The router, the focus ring and the touch-capture map are keyed by
    // PaneId, and the lobby surface has to appear in them without being a
    // participant. Ids are minted from 0 upward and never reused.
    use crate::native_host::panes::{PaneBus, PaneIdentity};
    let bus = PaneBus::default();
    for i in 0..8 {
        let id = bus.open(
            PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", format!("p{i}")).unwrap(),
        );
        assert_ne!(id, HOST_LOBBY_SURFACE_ID);
        assert!(id.0 < 1_000, "ids are minted from zero upward: {id}");
    }
}

#[test]
fn the_document_path_and_url_agree_with_each_other() {
    let lobby = LocalHostLobby::open("0.0.0.0:8080");
    assert_eq!(
        lobby.host_addr, "127.0.0.1:8080",
        "nothing can dial the documented default bind"
    );
    assert!(lobby.url().ends_with(&lobby.path()));
    assert!(lobby.path().starts_with("/host-lobby-"));
}

#[test]
fn the_address_a_phone_is_sent_to_is_not_the_one_the_view_dialled() {
    // Issue #1329, and the whole reason `join_base` exists beside
    // `host_addr`: the surface loads over loopback because that is what an
    // embedded view on this machine can dial, and a QR built from THAT
    // encodes the one address in the room no phone can open.
    //
    // Which address the discovery finds depends on the machine running the
    // test, so what is asserted is the shape and the port — the parts that
    // are decisions rather than environment.
    let lobby = LocalHostLobby::open("0.0.0.0:8080");
    assert!(lobby.join_base.starts_with("http://"));
    assert!(
        lobby.join_base.ends_with(":8080/"),
        "the port the listener bound, and the trailing slash the URL builder needs: {}",
        lobby.join_base
    );

    // A bind that names an address is the operator's own answer and is used
    // as given — which is also the one case with no environment in it.
    let pinned = LocalHostLobby::open("192.168.1.5:8080");
    assert_eq!(pinned.join_base, "http://192.168.1.5:8080/");
}

#[test]
fn an_issued_join_code_reaches_the_surface_as_the_page_can_use_it() {
    // The bridge plumbing for issue #1329's AC1: the code the relay was
    // issued becomes a push the document's own `__phoenixHostLobbyJoin`
    // answers, carrying the letters, the structured code and the address a
    // phone should be sent to.
    let lobby = LocalHostLobby::open("192.168.1.5:8080");
    let code = crate::core::rendezvous::JoinCode {
        full: "PHX-1-ABCDE".to_string(),
        suffix: "ABCDE".to_string(),
        ..Default::default()
    };
    let join = HostLobbyJoinResource::from_lobby(&lobby, None);
    lobby.publish_join(&join.invite(&code));

    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&lobby.bridge, &mut surface);
    assert_eq!(surface.pushed.len(), 1);
    let pushed = &surface.pushed[0];
    assert!(pushed.starts_with("window.__phoenixHostLobbyJoin("));
    assert!(pushed.contains("ABCDE"));
    assert!(pushed.contains("PHX-1-ABCDE"));
    assert!(pushed.contains("http://192.168.1.5:8080/"));
}

#[test]
fn a_directly_joinable_host_shows_its_own_code_and_not_the_clouds() {
    // Issue #1353. A host running BOTH legs holds two codes, and they are
    // not interchangeable: the QR carries the PAGE as well as the code, the
    // page a LAN phone loads is served by this host, and so the service it
    // dials is this host. The worker's letters on that QR would send every
    // phone in the room to a service that never heard of them.
    let lobby = LocalHostLobby::open("192.168.1.5:8080");
    let mine = crate::core::rendezvous::JoinCode {
        full: "PHX_1_ABCDE".to_string(),
        suffix: "ABCDE".to_string(),
        ..Default::default()
    };
    let cloud = crate::core::rendezvous::JoinCode {
        full: "PHX_1_ZZZZZ".to_string(),
        suffix: "ZZZZZ".to_string(),
        ..Default::default()
    };
    let join = HostLobbyJoinResource::from_lobby(&lobby, Some("https://worker.example"))
        .with_direct_code(mine.clone());
    assert_eq!(join.viewscreen_invite(&cloud), None);
    let invite = join.viewscreen_invite(&mine).expect("its own code shows");
    // And with no `?rendezvous=` in it: the page dials the origin that
    // served it, so naming a service would be both redundant and wrong.
    assert_eq!(
        invite,
        JoinInvite::Code {
            code: "ABCDE".to_string(),
            full: "PHX_1_ABCDE".to_string(),
            page_base: "http://192.168.1.5:8080/".to_string(),
            rendezvous: None,
        }
    );

    // A cloud-only host is unchanged: whatever the service issues goes up,
    // with the service named for `gui/join-url.js` to judge.
    let cloud_only = HostLobbyJoinResource::from_lobby(&lobby, Some("https://worker.example"));
    assert!(cloud_only.viewscreen_invite(&cloud).is_some());
}

#[test]
fn a_host_with_no_join_service_says_so_rather_than_showing_a_dead_qr() {
    // `--solo`, or no `--rendezvous` (issue #1329 AC2). The crew must not be
    // stood in front of the viewscreen scanning something that can never
    // work, and "no code yet" must not look like "no code ever".
    let lobby = LocalHostLobby::open("0.0.0.0:8080");
    lobby.publish_join(&JoinInvite::Off);

    let mut surface = crate::native_host::panes::RecordingSurface::ready();
    pump_host_lobby(&lobby.bridge, &mut surface);
    assert_eq!(
        surface.pushed,
        vec![r#"window.__phoenixHostLobbyJoin('{"kind":"off"}')"#.to_string()]
    );
}

#[test]
fn publishing_serves_the_lobby_at_that_path_and_withdrawing_stops() {
    let documents = HostedDocuments::default();
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    let page = std::fs::read_to_string("server.html").unwrap();
    lobby.publish(&page, &documents).unwrap();

    let body = documents
        .get(&lobby.path())
        .expect("the document is served");
    assert!(body.contains("id=\"station-grid\""));
    // The OS accessibility default layer every embedded surface gets
    // (issue #1127): an Ultralight view has no matchMedia to read.
    assert!(body.contains("window.PhoenixOsAccessibilityDefaults ="));

    lobby.withdraw(&documents);
    assert!(documents.get(&lobby.path()).is_none());
}

#[test]
fn a_bundle_with_no_lobby_refuses_by_name_instead_of_publishing_a_blank_page() {
    let documents = HostedDocuments::default();
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    assert_eq!(
        lobby.publish("<html><body></body></html>", &documents),
        Err(HostLobbyDocumentError::NoLobbyPanel)
    );
    assert!(documents.is_empty());
}

// ── the monitor row's two directions (issue #1330) ──────────────────────
//
// The surface half of the row is `gui/host-lobby-{view,render}.js` and its
// vitest suites; the law is `bridge_layout`'s. What is left — and what is
// here — is the loop between them: a record the page queued becomes a
// transition on the live layout, and the layout that results becomes the
// row the page is handed back.

use crate::native_host::bridge_layout::BridgeLayout;
use crate::native_host::bridge_profile::{identify, DiscoveredMonitor, RawMonitor};
use crate::native_host::panes::RecordingSurface;

const DELL: &str = "DELL U2720Q@3840x2160";
const BENQ: &str = "BenQ EX@1920x1080";

fn two_monitors() -> Vec<DiscoveredMonitor> {
    identify(&[
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
    ])
}

/// A two-monitor bridge with the viewscreen on the primary, as
/// `apply_bridge_profile` seeds one.
fn seeded_layout() -> BridgeLayoutResource {
    let monitors = two_monitors();
    let layout = BridgeLayout::from_discovered(
        &monitors,
        [crate::core::messages::StationId("helm".to_string())],
    )
    .expect("two monitors are a bridge");
    BridgeLayoutResource {
        layout,
        monitors,
        notices: Vec::new(),
    }
}

/// An app with the lobby plugin and a seeded bridge layout, plus a surface
/// that has already taken the opening row.
fn app_with_layout() -> (App, HostLobbyBridge, RecordingSurface) {
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(seeded_layout());
    app.update();
    let mut surface = RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    surface.pushed.clear();
    (app, bridge, surface)
}

/// Queue one record on the surface and run **one** frame that answers it.
///
/// Exactly one `app.update()`, which is what makes every test below an
/// assertion about the schedule as well as about the rule: the drain is in
/// `PreUpdate` and `publish_bridge_layout` in `Update`, so a press and the
/// row that answers it land in the same frame. Move the drain after the
/// publish and every one of these fails, because the row would be a frame
/// late and this helper never runs that frame.
fn press(app: &mut App, bridge: &HostLobbyBridge, surface: &mut RecordingSurface, json: &str) {
    surface.queue_record(json);
    pump_host_lobby(bridge, surface);
    app.update();
    pump_host_lobby(bridge, surface);
}

fn viewscreen(app: &App) -> String {
    app.world()
        .resource::<BridgeLayoutResource>()
        .layout
        .viewscreen()
        .as_str()
        .to_string()
}

#[test]
fn the_opening_row_is_pushed_as_soon_as_a_layout_exists() {
    let (mut app, bridge) = app_with_lobby();
    app.insert_resource(seeded_layout());
    app.update();
    let mut surface = RecordingSurface::ready();
    pump_host_lobby(&bridge, &mut surface);
    let row = surface
        .pushed
        .iter()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the row reaches the surface");
    assert!(row.contains(DELL));
    assert!(row.contains(BENQ));
}

#[test]
fn a_button_press_moves_the_live_viewscreen_and_the_row_says_so() {
    // The acceptance criterion, end to end on the host side: a record the
    // page queued becomes one lawful transition, and the row that comes
    // back marks the display the operator chose.
    let (mut app, bridge, mut surface) = app_with_layout();
    assert_eq!(viewscreen(&app), DELL);

    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
    );

    assert_eq!(viewscreen(&app), BENQ);
    let row = surface
        .pushed
        .iter()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the moved row is pushed back");
    // The mark travels as data — `viewscreen: true` on the display that
    // was pressed and nowhere else — and the row's words are the page's.
    assert!(
            row.contains(r#"{"identity":"BenQ EX@1920x1080","name":"BenQ EX","width":1920,"height":1080,"primary":false,"viewscreen":true}"#),
            "the pressed display is marked: {row}"
        );
    assert!(
            row.contains(r#""identity":"DELL U2720Q@3840x2160","name":"DELL U2720Q","width":3840,"height":2160,"primary":true,"viewscreen":false"#),
            "and the one it left is not: {row}"
        );
    assert!(
        !row.contains("bridge_layout"),
        "an accepted press has nothing to say: {row}"
    );
}

#[test]
fn a_press_for_a_monitor_that_is_gone_is_refused_with_something_to_read() {
    // The stale press. "The lobby never silently ignores me" is the user
    // story, and a boolean cannot satisfy it — so the refusal comes back as
    // the id of a sentence the row renders.
    let (mut app, bridge, mut surface) = app_with_layout();

    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"kind":"set-viewscreen","monitor":"Unplugged@1920x1080"}"#,
    );

    assert_eq!(viewscreen(&app), DELL, "nothing moved");
    let row = surface
        .pushed
        .iter()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the refusal is pushed back");
    assert!(row.contains("server.bridge_layout.unknown_monitor"));
    assert!(row.contains("Unplugged@1920x1080"));
    // And having been pushed, it is no longer OWED. The notices list is
    // what the surface has not been told yet (issue #1331 made every writer
    // append to it, so it has to be emptied somewhere); this is where.
    assert!(
        app.world()
            .resource::<BridgeLayoutResource>()
            .notices
            .is_empty(),
        "the publisher drains what it has pushed"
    );
}

#[test]
fn moving_the_viewscreen_onto_a_monitor_holding_a_console_is_refused_not_resolved() {
    // Rule 2's mirror, reaching a person: the consoles are not evicted to
    // make room, and the row says which ones are in the way.
    let (mut app, bridge, mut surface) = app_with_layout();
    let seated = app
        .world()
        .resource::<BridgeLayoutResource>()
        .layout
        .apply(
            &crate::native_host::bridge_layout::LayoutAction::AssignStation {
                station: crate::core::messages::StationId("helm".to_string()),
                monitor: crate::native_host::bridge_profile::MonitorIdentity::new(BENQ),
            },
        )
        .expect("a free non-viewscreen monitor takes a console");
    app.world_mut()
        .resource_mut::<BridgeLayoutResource>()
        .layout = seated;

    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
    );

    assert_eq!(viewscreen(&app), DELL);
    let row = surface
        .pushed
        .iter()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the refusal is pushed back");
    assert!(row.contains("server.bridge_layout.viewscreen_holds_stations"));
    assert!(row.contains("helm"));
}

#[test]
fn an_accepted_press_clears_the_refusal_the_one_before_it_earned() {
    // The row shows what happened to the LAST thing the operator did.
    //
    // Asserted on the ROWS rather than on the resource, which is where the
    // claim actually lives: since issue #1331 every writer of `notices`
    // appends (so a press cannot erase a console surrender it raced) and the
    // publisher drains what it pushed — so "the refusal is gone" is a fact
    // about the row the surface is handed next, not about a field.
    let (mut app, bridge, mut surface) = app_with_layout();
    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"kind":"set-viewscreen","monitor":"Unplugged@1920x1080"}"#,
    );
    let refused = surface
        .pushed
        .iter()
        .rev()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the refusal is pushed back")
        .clone();
    assert!(refused.contains("server.bridge_layout.unknown_monitor"));

    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
    );
    let accepted = surface
        .pushed
        .iter()
        .rev()
        .find(|s| s.contains("__phoenixHostLobbyLayout"))
        .expect("the moved row is pushed back");
    assert_ne!(&refused, accepted, "a new row, not the old one repeated");
    assert!(
        !accepted.contains("bridge_layout"),
        "and it carries no notice at all: {accepted}"
    );
    assert!(app
        .world()
        .resource::<BridgeLayoutResource>()
        .notices
        .is_empty());
}

#[test]
fn a_record_this_build_cannot_read_moves_nothing_and_says_nothing_to_the_row() {
    // A page/host mismatch is an operator's problem, not an answer to a
    // press — rendering it as feedback would tell the crew their button is
    // broken when what is broken is the bundle.
    let (mut app, bridge, mut surface) = app_with_layout();
    press(
        &mut app,
        &bridge,
        &mut surface,
        r#"{"monitor":"BenQ EX@1920x1080"}"#,
    );

    assert_eq!(viewscreen(&app), DELL);
    assert!(app
        .world()
        .resource::<BridgeLayoutResource>()
        .notices
        .is_empty());
    assert!(
        !surface
            .pushed
            .iter()
            .any(|s| s.contains("__phoenixHostLobbyLayout")),
        "and nothing changed, so there is no row to push"
    );
}

#[test]
fn a_bridge_nobody_rearranged_pushes_no_row_at_all() {
    // Every push is a synchronous evaluate_script on the thread the fixed
    // tick runs on, and a monitor row changes about once a session.
    let (mut app, bridge, _) = app_with_layout();
    for _ in 0..10 {
        app.update();
    }
    assert!(!bridge.has_pending());
}

#[test]
fn a_press_that_arrives_before_a_layout_exists_is_dropped_rather_than_queued() {
    // A host whose winit has not enumerated its displays yet has no layout
    // to judge a press against. Draining anyway is what stops a surface
    // that started talking to it sitting on a queue for the whole mission.
    let (mut app, bridge) = app_with_lobby();
    let mut surface = RecordingSurface::ready();
    surface.queue_record(r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    assert!(bridge.take_records().is_empty());
}

#[test]
fn one_drain_serves_every_verb_the_surface_speaks_in_one_frame() {
    // The claim this whole arrangement exists for. `take_records` is a
    // DRAIN, so a second reader would swallow the first's records and warn
    // about a vocabulary it does not speak while the first saw an empty
    // queue for the rest of the run — with a clean log at both ends, which
    // is why nothing else here would catch it.
    //
    // A frame carrying a pick, a launch and a monitor press proves all
    // three arrive: the pick as an InboundMessage the arbiter reads, the
    // launch on the force-start latch, the press on the live layout — and
    // the row reporting it published in that SAME frame, which is the
    // PreUpdate-drain → Update-publish edge stated as a claim rather than
    // left to the other tests to imply.
    let (mut app, bridge, mut surface) = app_with_layout();
    app.insert_resource(crate::server::bridge::PendingForceStart(false));
    assert_eq!(viewscreen(&app), DELL);

    surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
    surface.queue_record(r#"{"kind":"force_start"}"#);
    surface.queue_record(r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#);
    pump_host_lobby(&bridge, &mut surface);
    app.update();
    pump_host_lobby(&bridge, &mut surface);

    let picks = inbound(&mut app);
    assert_eq!(picks.len(), 1, "the pick reached the arbiter's bus");
    assert_eq!(picks[0].token, LOCAL_CONSOLE_TOKEN);
    assert!(matches!(
        picks[0].msg,
        ClientMessage::SelectScenario { ref scenario_id } if scenario_id == "combat_test"
    ));
    assert!(
        app.world()
            .resource::<crate::server::bridge::PendingForceStart>()
            .0,
        "the launch reached the latch in the same frame as the pick"
    );
    assert_eq!(
        viewscreen(&app),
        BENQ,
        "and the monitor press reached the live layout, rather than being \
             swallowed by whichever reader ran first"
    );
    assert!(
        surface
            .pushed
            .iter()
            .any(|s| s.contains("__phoenixHostLobbyLayout") && s.contains(BENQ)),
        "the row that answers the press is published in that same frame: {:?}",
        surface.pushed
    );
}
#[test]
fn a_saved_shake_reaches_the_renderer_at_launch_not_at_the_first_press() {
    // Issue #1428. The renderer reads its two effect intensities off a
    // process-global latch, and until this test the ONLY writer was the
    // `SetPresentation` arm below — a press. Nothing published the record
    // this display had already saved, and `presentation.apply()` on the page
    // cannot: the browser motion channel it publishes to is a no-op on an
    // Ultralight view. So a room that had turned the hull shake off came
    // back at FULL shake every launch, because `sync_viewscreen_motion` saw
    // "nothing published" and fell back to the reduced-motion default.
    use crate::server::bridge::{
        clear_native_effect_intensities, published_effect_intensities,
        NATIVE_EFFECT_LATCH_TEST_LOCK,
    };
    let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_native_effect_intensities();
    assert_eq!(
        published_effect_intensities(),
        (None, None, None),
        "a launch starts with nothing published — that is the state this test is about"
    );

    // What the room left on this machine last session, arriving the way the
    // host reads it: through the store, sanitised, not hand-built.
    let dir = std::env::temp_dir().join("phoenix-1428-launch-seed");
    let _ = std::fs::remove_dir_all(&dir);
    let store = super::super::viewscreen_presentation::ViewscreenPresentationStore::at(&dir);
    store
        .save(
            &super::super::viewscreen_presentation::ViewscreenPresentation {
                shake_percent: Some(0),
                flash_percent: Some(0),
                decorative_motion_percent: Some(40),
                ..Default::default()
            },
        )
        .expect("save");

    // The launch itself. The host page is deliberately unusable: the claim
    // is that the renderer is told what this display saved, and that is not
    // contingent on the lobby markup assembling.
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    let _ = lobby.publish_with_presentation(
        "<html><body></body></html>",
        &crate::delivery::serve::HostedDocuments::default(),
        store.load(),
        None,
    );

    let (shake, flash, decorative) = published_effect_intensities();
    assert_eq!(
        shake,
        Some(0.0),
        "a saved off-state reaches the renderer before anybody presses anything"
    );
    // Flashes = Off is the case the Display tab's copy makes a promise
    // about on this runtime: from here it reaches the shield-flash uniform
    // AND, through `sync_viewscreen_motion` and `cache_hud_state`, the
    // `data-flash` band the native HUD overlay's vignette rule reads.
    assert_eq!(
        flash,
        Some(0.0),
        "a saved Flashes = Off crosses at launch, not at the first press"
    );
    // The third percent has no uniform: it is CSS, and on native it reaches
    // a document (`gui/viewscreen-hud.html`) that no head injection does. It
    // crosses on this same call because this is the only seam that carries
    // it into the process.
    let decorative = decorative.expect("the saved decorative band crosses too");
    assert!(
        (decorative - 0.4).abs() < 1e-6,
        "decorative was {decorative}"
    );

    clear_native_effect_intensities();
    let _ = std::fs::remove_dir_all(&dir);
}
