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
        ["first", "second", "third"].map(super::super::document::host_lobby_fleet_update_script)
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
