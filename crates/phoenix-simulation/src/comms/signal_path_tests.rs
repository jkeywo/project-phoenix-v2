use super::*;
use crate::entities::spawner::{EntityUuid, RegionShapeSection};
use crate::regions::{
    field::{FieldConfig, FieldEvolution, FieldFalloff, RegionFieldState},
    shape::RegionShape,
    signal_path::SignalMaterialConfig,
};

fn app() -> App {
    let mut app = App::new();
    app.init_resource::<CommsRuntime>()
        .insert_resource(crate::sim_tick::SimTick(0))
        .insert_resource(Time::<Fixed>::from_seconds(1.0))
        .add_systems(Update, update_comms_range_flags);
    app.world_mut().spawn((
        crate::server_app::LocalShip,
        crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot(0)),
        Transform::from_xyz(-20.0, 0.0, 0.0),
        crate::comms::CommsRange(100.0),
    ));
    app.world_mut().spawn((
        EntityUuid("contact".into()),
        Transform::from_xyz(20.0, 0.0, 0.0),
        crate::comms::CommsRange(100.0),
        crate::comms::CommsHailable::default(),
    ));
    app
}
fn reachable(app: &App) -> bool {
    app.world().resource::<CommsRuntime>().range_flags["contact"]
}
fn profile() -> FieldConfig {
    FieldConfig {
        falloff: FieldFalloff::Step,
        strength: 1.0,
        sensor_absorption_per_unit: 0.0,
        comms_absorption_per_unit: 0.0,
        evolution: None,
    }
}

#[test]
fn substantial_terrain_blocks_real_local_and_fleet_reach_until_cleared() {
    let mut app = app();
    app.update();
    assert!(reachable(&app));
    let terrain = app
        .world_mut()
        .spawn((
            EntityUuid("planet".into()),
            Transform::default(),
            RegionShapeSection(RegionShape::Sphere { radius: 5.0 }),
            SignalMaterialConfig {
                sensor_absorption_per_unit: 0.0,
                comms_absorption_per_unit: 0.0,
                sensor_opaque: false,
                comms_opaque: true,
            },
        ))
        .id();
    app.update();
    assert!(!reachable(&app));
    let comms = app.world().resource::<CommsRuntime>();
    assert!(!comms.contacts[0].in_range);
    assert!(!comms.fleet_range_flags[&crate::command_admission::HostSlot(0)]["contact"]);
    app.world_mut().despawn(terrain);
    app.update();
    assert!(reachable(&app));
}

#[test]
fn endpoint_clear_cloud_affects_only_authored_signal_class() {
    let mut app = app();
    let mut field = profile();
    field.sensor_absorption_per_unit = 1.0;
    let cloud = app
        .world_mut()
        .spawn((
            EntityUuid("cloud".into()),
            Transform::default(),
            RegionShapeSection(RegionShape::Sphere { radius: 5.0 }),
            RegionFieldState::new(field, 0, [0.0; 3]).unwrap(),
        ))
        .id();
    app.update();
    assert!(reachable(&app));
    app.world_mut()
        .get_mut::<RegionFieldState>(cloud)
        .unwrap()
        .profile
        .comms_absorption_per_unit = 1.0;
    app.update();
    assert!(!reachable(&app));
    // Neither participant is inside the cloud: this is actual path loss.
    assert!(app.world().resource::<CommsRuntime>().range_active);
}

#[test]
fn logical_field_evolution_changes_the_same_real_reach_without_frame_integration() {
    let mut app = app();
    let mut field = profile();
    field.comms_absorption_per_unit = 1.0;
    field.evolution = Some(FieldEvolution {
        duration_secs: 10.0,
        translation: [0.0, 0.0, 30.0],
        final_scale: 1.0,
        final_strength: 1.0,
    });
    app.world_mut().spawn((
        EntityUuid("cloud".into()),
        Transform::default(),
        RegionShapeSection(RegionShape::Sphere { radius: 5.0 }),
        RegionFieldState::new(field, 0, [0.0; 3]).unwrap(),
    ));
    app.update();
    assert!(!reachable(&app));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 10;
    app.update();
    assert!(reachable(&app));
    app.update();
    assert!(reachable(&app));
}
