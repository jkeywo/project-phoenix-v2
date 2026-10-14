use super::*;
use phoenix_sim_gameplay::ship::disablement::DisablementObservation;

fn attach_observation(app: &mut App, uuid: &str, disabled: bool) -> Entity {
    let mut q = app
        .world_mut()
        .query::<(Entity, &crate::entities::spawner::EntityUuid)>();
    let entity = q.iter(app.world()).find(|(_, id)| id.0 == uuid).unwrap().0;
    app.world_mut()
        .entity_mut(entity)
        .insert(DisablementObservation { disabled });
    entity
}

#[test]
fn public_defeat_excludes_combat_but_preserves_towing_and_recovers_normally() {
    let mut app = test_app();
    let uuid = "cc000000-0000-0000-0000-000000001101";
    set_tactical_radar_range(&mut app, 200.0);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, uuid, 0.0, -30.0);
    let actor = attach_observation(&mut app, uuid, true);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("hulk".into(), uuid.into());
    insert_destroy_objective_blackboard(&mut app, "hulk", 90.0);
    set_local_last_attacker(&mut app, None);
    tick(&mut app);
    assert!(
        get_weapons_target(&mut app).is_none(),
        "even a named combat order cannot auto-acquire confirmed defeat"
    );
    insert_operate_objective_blackboard(&mut app, "hulk", 80.0);
    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(uuid),
        "same retained entity can still be towed"
    );
    app.world_mut()
        .get_mut::<DisablementObservation>(actor)
        .unwrap()
        .disabled = false;
    insert_destroy_objective_blackboard(&mut app, "hulk", 90.0);
    tick(&mut app);
    assert_eq!(get_weapons_target(&mut app).as_deref(), Some(uuid));
    assert!(app.world().get_entity(actor).is_ok());
}

#[test]
fn human_radar_receives_the_same_coarse_fact_only_for_visible_contacts() {
    let mut app = test_app();
    let uuid = "cc000000-0000-0000-0000-000000001102";
    setup_ship_contact_world(&mut app, uuid, 0.0, -50.0);
    start_game(&mut app);
    let actor = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(0.0, 0.0, -50.0),
            DisablementObservation { disabled: true },
        ))
        .id();
    tick(&mut app);
    assert!(blip_for(&tactical_blips(&mut app), uuid).neutralized);
    app.world_mut()
        .get_mut::<DisablementObservation>(actor)
        .unwrap()
        .disabled = false;
    tick(&mut app);
    assert!(!blip_for(&tactical_blips(&mut app), uuid).neutralized);
    app.world_mut()
        .get_mut::<Transform>(actor)
        .unwrap()
        .translation
        .z = -100_000.0;
    tick(&mut app);
    assert!(
        !tactical_blips(&mut app)
            .iter()
            .any(|blip| blip.uuid == uuid),
        "lifecycle information does not reveal unseen contacts"
    );
}
