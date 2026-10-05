use super::*;

fn app_with_surface_resources() -> App {
    let mut app = App::new();
    app.init_resource::<crate::debug_overlay::DebugRegionsEnabled>();
    app.init_resource::<crate::debug_overlay::DebugOverlayEnabled>();
    app.init_resource::<crate::debug_overlay::DebugDamageEnabled>();
    app.init_resource::<crate::debug_overlay::DebugEntitiesEnabled>();
    app.init_resource::<crate::debug_overlay::DebugEntityInspectorEnabled>();
    app.init_resource::<crate::debug::DebugStationActivityEnabled>();
    app.init_resource::<crate::debug::DebugAiDoctrineEnabled>();
    app.init_resource::<crate::debug::DebugScenarioStateEnabled>();
    app.init_resource::<crate::debug::DebugConsoleLatencyEnabled>();
    app.init_resource::<DebugSurfaceReadback>();
    app
}

#[test]
fn every_catalogue_row_has_exactly_one_correctly_ordered_adapter() {
    assert_eq!(DEBUG_SURFACE_ADAPTERS.len(), DEBUG_SURFACE_CATALOGUE.len());
    let unique: HashSet<_> = DEBUG_SURFACE_ADAPTERS
        .iter()
        .map(|adapter| adapter.surface)
        .collect();
    assert_eq!(unique.len(), DEBUG_SURFACE_CATALOGUE.len());
    for (adapter, descriptor) in DEBUG_SURFACE_ADAPTERS.iter().zip(DEBUG_SURFACE_CATALOGUE) {
        assert_eq!(adapter.surface, descriptor.surface);
    }
}

#[test]
fn pending_duplicates_collapse_and_readback_uses_stable_catalogue_order() {
    let mut app = app_with_surface_resources();
    apply_pending_toggles(
        app.world_mut(),
        [
            DebugSurface::Damage,
            DebugSurface::Damage,
            DebugSurface::Regions,
        ],
    );
    refresh_readback(app.world_mut());

    let reported = &app.world().resource::<DebugSurfaceReadback>().0;
    assert_eq!(
        reported
            .iter()
            .map(|(surface, _)| *surface)
            .collect::<Vec<_>>(),
        DebugSurface::ALL
    );
    assert!(
        reported
            .iter()
            .find(|(s, _)| *s == DebugSurface::Damage)
            .unwrap()
            .1
    );
    assert!(
        reported
            .iter()
            .find(|(s, _)| *s == DebugSurface::Regions)
            .unwrap()
            .1
    );
}

#[test]
fn absolute_set_is_idempotent_and_uses_the_same_adapter() {
    let mut app = app_with_surface_resources();
    set_surface(app.world_mut(), DebugSurface::ConsoleLatency, true);
    set_surface(app.world_mut(), DebugSurface::ConsoleLatency, true);
    assert!(adapter(DebugSurface::ConsoleLatency).is_enabled(app.world()));
}

#[test]
fn pending_absolute_states_collapse_to_the_latest_value() {
    let mut app = app_with_surface_resources();
    apply_pending_states(
        app.world_mut(),
        [
            (DebugSurface::Damage, true),
            (DebugSurface::Regions, true),
            (DebugSurface::Damage, false),
        ],
    );
    assert!(!adapter(DebugSurface::Damage).is_enabled(app.world()));
    assert!(adapter(DebugSurface::Regions).is_enabled(app.world()));
}

#[test]
fn native_readback_refresh_needs_no_session_or_bridge_resource() {
    let mut app = app_with_surface_resources();
    set_surface(app.world_mut(), DebugSurface::ScenarioState, true);
    app.add_systems(Update, refresh_readback);
    app.update();

    assert_eq!(
        app.world().resource::<DebugSurfaceReadback>().0,
        DebugSurface::ALL
            .into_iter()
            .map(|surface| (surface, surface == DebugSurface::ScenarioState))
            .collect::<Vec<_>>()
    );
}

#[test]
fn host_diagnostic_mutation_route_is_absent_from_a_demo_build() {
    assert_eq!(
        mutation_route_available(),
        !crate::build_flags::is_demo_cfg()
    );
}
