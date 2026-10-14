use super::*;
use crate::ship::disablement::{publish_ship_disablement, DisablementObservation};

fn actor() -> (App, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let mut app = App::new();
    app.add_message::<crate::ai::server::AiEntityDisabled>();
    let world = app.world_mut();
    world.register_component::<ShipPhysics>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    let entity = world
        .spawn((
            EntityUuid("actor".into()),
            damage,
            ShipRedAlert(false),
            crate::ship_plugin::ShipConfigComponent(config.ship_config.unwrap()),
        ))
        .id();
    (app, entity)
}

#[test]
fn reversible_world_events_roundtrip_entries_and_recoveries() {
    for disabled in [false, true] {
        let event = WorldEvent::Disabled {
            uuid: "actor".into(),
            disabled,
        };
        let record = super::super::capture::world_event_record(&event);
        assert_eq!(record.kind, 11);
        assert_eq!(record.disabled, disabled);
        let stored: WorldEventRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(
            matches!(super::super::application::world_event_from_record(&stored),Some(WorldEvent::Disabled{uuid,disabled:value}) if uuid=="actor" && value==disabled)
        );
    }
}

#[test]
fn full_snapshot_preserves_the_actor_frontier_and_next_recovery_occurs_once() {
    let (mut live, entity) = actor();
    live.world_mut()
        .get_mut::<DisablementObservation>(entity)
        .unwrap()
        .disabled = true;
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    assert_eq!(saved.entities.len(), 1);
    assert!(saved.entities[0].disablement.as_ref().unwrap().disabled);
    let (mut resumed, restored) = actor();
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    assert!(
        resumed
            .world()
            .get::<DisablementObservation>(restored)
            .unwrap()
            .disabled
    );
    use bevy::ecs::system::RunSystemOnce;
    for app in [&mut live, &mut resumed] {
        app.world_mut()
            .run_system_once(publish_ship_disablement)
            .unwrap();
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<crate::ai::server::AiEntityDisabled>>()
            .drain()
            .map(|event| event.disabled)
            .collect();
        assert_eq!(events, [false]);
        app.world_mut()
            .run_system_once(publish_ship_disablement)
            .unwrap();
        assert!(app
            .world()
            .resource::<Messages<crate::ai::server::AiEntityDisabled>>()
            .is_empty());
    }
    assert_eq!(
        serde_json::to_value(capture(live.world())).unwrap(),
        serde_json::to_value(capture(resumed.world())).unwrap()
    );
}

#[test]
fn missing_required_frontier_refuses_before_alert_or_clock_writes() {
    let (source, _) = actor();
    let mut invalid = capture(source.world());
    invalid.entities[0].disablement = None;
    invalid.entities[0].red_alert = Some(true);
    invalid.tick = 999;
    let (mut target, _) = actor();
    let before = serde_json::to_value(capture(target.world())).unwrap();
    let report = restore(target.world_mut(), &invalid);
    assert!(!report.is_complete());
    assert_eq!(report.entities_restored, 0);
    assert_eq!(
        serde_json::to_value(capture(target.world())).unwrap(),
        before
    );
}
