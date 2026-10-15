use super::*;

#[test]
fn neither_real_emitter_can_create_bus_advice_without_an_acquired_finding() {
    for high in [false, true] {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);
        if high {
            arm_real_high_fidelity_sensors_emitter(&mut app, "unprobed", 0.83);
        } else {
            arm_real_sensors_frequency_emitter(&mut app, "unprobed", 0.83);
        }
        let ship = find_ship_entity(&mut app);
        let source = crate::science::scan_blackboard_key_for(
            &crate::ship::system_registry::sensors_system_id(),
        );
        // Keep the physical target and frozen lock, revoke only acquired knowledge.
        app.world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
            .unwrap()
            .0
            .remove(&source);
        let before = ship_phaser_frequency(&mut app);
        for _ in 0..authored_reaction_delay_runs() + 4 {
            tick(&mut app);
        }
        assert_eq!(
            ship_phaser_frequency(&mut app),
            before,
            "high fidelity={high}"
        );
    }
}
