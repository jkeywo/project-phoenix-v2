use super::*;
use bevy::ecs::system::RunSystemOnce;

#[test]
fn physical_evidence_records_damage_without_ai_and_expires_on_logical_ticks() {
    let mut app = App::new();
    let mut config = crate::world::config::WorldConfig::default();
    config.global.sim_tick_hz = 10.0;
    app.insert_resource(config)
        .insert_resource(crate::sim_tick::SimTick::default());
    let shield = crate::weapons::shield::ShieldSystem::default();
    let mut history = ShieldsDamageHistory::default();
    history.ensure_len(shield.facings.len());
    for (index, facing) in shield.facings.iter().enumerate() {
        history.observe_hp(index, facing.hp);
    }
    let entity = app
        .world_mut()
        .spawn((
            ShipShields(shield, 0.5),
            history,
            ShieldsAiConfigResource {
                damage_window_secs: 0.6,
                min_damage_window_secs: 0.1,
                ..Default::default()
            },
        ))
        .id();
    // No Ship marker, controller, session or AI policy participates in sensing damage.
    app.world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings[0]
        .hp -= 5;
    app.world_mut().run_system_once(track_damage).unwrap();
    assert_eq!(
        app.world()
            .get::<ShieldsDamageHistory>(entity)
            .unwrap()
            .arcs[0][0]
            .amount,
        5
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    app.world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings[0]
        .hp += 3;
    app.world_mut().run_system_once(track_damage).unwrap();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    app.world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings[0]
        .hp -= 2;
    app.world_mut().run_system_once(track_damage).unwrap();
    assert_eq!(
        app.world()
            .get::<ShieldsDamageHistory>(entity)
            .unwrap()
            .arcs[0]
            .len(),
        2
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 6;
    app.world_mut().run_system_once(track_damage).unwrap();
    let history = app.world().get::<ShieldsDamageHistory>(entity).unwrap();
    assert_eq!(history.arcs[0].len(), 1);
    assert_eq!(history.arcs[0][0].amount, 2);
    assert_eq!(history.arcs[0][0].recorded_tick, 2);
}
