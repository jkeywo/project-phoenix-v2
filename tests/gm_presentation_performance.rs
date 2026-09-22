//! Presentation demand must not change the authoritative continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::{
    gm_projection::*,
    headless::{build_headless_app, HeadlessArgs},
};

fn app(present: bool) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/combat_test.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        seed: Some(42),
        deterministic: true,
        log_spec: "off".into(),
        log: project_phoenix::logging::parse_log_spec("off").unwrap(),
        ..Default::default()
    })
    .unwrap();
    app.add_plugins(GmProjectionPlugin);
    if present {
        app.insert_resource(NativeGmPresentation);
    }
    app.finish();
    app.cleanup();
    app
}

#[test]
fn presentation_and_subscriptions_preserve_tick_digests_and_journal() {
    let mut baseline = app(false);
    let mut observed = app(true);
    for step in 0..120 {
        baseline.update();
        observed.update();
        assert_eq!(
            baseline
                .world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0,
            observed
                .world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0
        );
        assert_eq!(
            project_phoenix::sim_digest::state_digest(&baseline),
            project_phoenix::sim_digest::state_digest(&observed),
            "update {step}"
        );
        assert_eq!(
            baseline
                .world()
                .resource::<project_phoenix::gm_action::GmActionLog>()
                .entries(),
            observed
                .world()
                .resource::<project_phoenix::gm_action::GmActionLog>()
                .entries()
        );
        if step == 30 {
            observed.world_mut().resource_mut::<GmInspectorInterest>().0 = Some(Default::default());
        }
    }
}

#[test]
fn console_demand_preserves_topology_and_invalidates_on_world_replacement() {
    let mut app = app(true);
    for _ in 0..5 {
        app.update();
    }
    fn take(app: &mut App) -> GmStationProjectionPayload {
        app.world_mut()
            .resource_mut::<Messages<project_phoenix::console_bridge::GmStationProjectionChanged>>()
            .drain()
            .last()
            .unwrap()
            .payload
    }
    let initial = take(&mut app);
    let ship = initial
        .ships
        .iter()
        .find(|ship| {
            ship.stations
                .iter()
                .any(|station| station.station_id.0 == "helm")
        })
        .unwrap()
        .ship_id
        .clone();
    let request = GmConsoleInterest {
        consumer: GmConsoleConsumer::Console,
        ship: ship.clone(),
        station: "helm".into(),
        visible: true,
        mount_generation: 7,
        world_generation: initial.presentation_generation,
    };
    app.world_mut()
        .resource_mut::<GmConsoleSubscriptions>()
        .requests
        .insert(request.consumer, request.clone());
    app.update();
    let selected = take(&mut app);
    assert_eq!(selected.detail_ships, Some(vec![ship]));
    assert_eq!(selected.ships.len(), initial.ships.len());
    app.world_mut()
        .resource_mut::<GmConsoleSubscriptions>()
        .requests
        .get_mut(&request.consumer)
        .unwrap()
        .visible = false;
    app.update();
    let hidden = take(&mut app);
    assert_eq!(hidden.detail_ships, Some(Vec::new()));
    assert!(hidden.entities.is_empty());
    assert_eq!(hidden.ships.len(), selected.ships.len());
    app.world_mut()
        .resource_mut::<GmConsoleSubscriptions>()
        .requests
        .insert(request.consumer, request);
    app.world_mut()
        .resource_mut::<project_phoenix::world::config::WorldConfig>()
        .set_changed();
    app.update();
    let replaced = take(&mut app);
    assert!(replaced.presentation_generation > hidden.presentation_generation);
    assert_eq!(
        replaced.detail_ships,
        Some(Vec::new()),
        "old world interest cannot receive new-world detail"
    );
}
