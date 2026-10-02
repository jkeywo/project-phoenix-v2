use super::*;
use crate::native_host::panes::{PaneId, RecordingSurface};

#[test]
fn save_requests_report_phase_and_missing_store_refusals_to_the_private_surface() {
    let mut world = World::new();
    world.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    let bridge = NativeGmBridge::default();
    bridge.activate(PaneId(1));
    let mut surface = RecordingSurface::ready();
    request(
        &mut world,
        &bridge,
        "before-start".into(),
        "create",
        Some("Bookmark".into()),
    );
    bridge.pump(PaneId(1), &mut surface);
    assert!(
        surface
            .pushed
            .iter()
            .any(|value| value.contains("before-start")
                && value.contains("while the game is running"))
    );
    request(&mut world, &bridge, "catalogue".into(), "list", None);
    bridge.pump(PaneId(1), &mut surface);
    assert!(surface
        .pushed
        .iter()
        .any(|value| value.contains("catalogue") && value.contains("unavailable")));
}

#[test]
fn completion_receipts_survive_logger_drain_and_delayed_surface_pumps() {
    let bridge = NativeGmBridge::default();
    bridge.activate(PaneId(1));
    bridge.retain_save_outcomes(vec![SaveOutcome {
        slot: "first-save".into(),
        tick: 12,
        ok: true,
        error: None,
    }]);
    bridge.retain_save_outcomes(Vec::new());
    bridge.retain_save_outcomes(vec![SaveOutcome {
        slot: "second-save".into(),
        tick: 13,
        ok: false,
        error: Some("disk full".into()),
    }]);
    let mut surface = RecordingSurface::ready();
    bridge.pump(PaneId(1), &mut surface);
    assert!(surface
        .pushed
        .iter()
        .any(|value| value.contains("first-save")
            && value.contains("second-save")
            && value.contains("disk full")));
}
