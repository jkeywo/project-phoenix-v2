use super::*;
use crate::ship::disablement::{publish_ship_disablement, DisablementObservation};

#[test]
fn paid_enquiry_event_codec_preserves_every_observer_field_and_refuses_bad_pending_state() {
    use crate::world::config::EnquiryPhase;
    for phase in [EnquiryPhase::Started, EnquiryPhase::Completed] {
        let event = WorldEvent::Enquiry {
            target_uuid: "subject".into(),
            observer_uuid: "observer".into(),
            system: phoenix_model::identity::SystemId("science".into()),
            aim: "anomaly".into(),
            intrusive: true,
            phase,
        };
        let record = super::super::capture::world_event_record(&event);
        let stored: WorldEventRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(
            super::super::application::world_event_from_record(&stored),
            Some(event)
        );
        for change in [0, 1, 2, 3, 4] {
            let mut bad = record.clone();
            match change {
                0 => bad.aim = "secret".into(),
                1 => bad.system.clear(),
                2 => bad.other.clear(),
                3 => bad.subject.clear(),
                _ => bad.enquiry_phase = None,
            }
            assert!(super::super::application::world_event_from_record(&bad).is_none());
            let mut world = World::new();
            let before = capture(&world);
            let mut saved = before.clone();
            saved.scenario = Some(ScenarioState {
                pending_events: vec![bad],
                ..Default::default()
            });
            let report = restore(&mut world, &saved);
            assert!(!report.is_complete());
            assert_eq!(
                capture(&world),
                before,
                "invalid event must refuse before any writes"
            );
        }
    }
}

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
    // The accepted ShipConfig carries the ordinary crew continuation contract.
    // Use the same Backfill seed as an actual NPC installation, without adding
    // human occupancy or changing this isolated disablement event fixture.
    let ship_config = config.ship_config.unwrap();
    let (sources, ratings) = crate::ship::rating::seed_boot_ratings(&ship_config, |_| {
        crate::ship::rating::BACKFILL_RATING.into()
    });
    let entity = world
        .spawn((
            EntityUuid("actor".into()),
            damage,
            ShipRedAlert(false),
            crate::ship_plugin::ShipConfigComponent(ship_config),
            // Match assignments::install for this accepted armed hull. Empty
            // retained intent still has a required canonical owner.
            crate::console::weapons::assignments::ShipWeaponTargets::default(),
            crate::ship_plugin::ShipSystemControlSources(sources),
            crate::ship_plugin::ActiveStationRatings(ratings),
            crate::lobby::crew_replication::ShipCrewPresence::default(),
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
    assert!(saved.entities[0]
        .weapon_targets
        .as_ref()
        .unwrap()
        .0
        .is_empty());
    assert!(saved.entities[0].disablement.as_ref().unwrap().disabled);
    let crew = saved.entities[0]
        .mesh_crew
        .as_ref()
        .expect("ordinary accepted-config crew owner");
    assert!(
        crew.occupied.is_empty(),
        "Backfill does not invent Human occupancy"
    );
    assert!(!crew.sources.is_empty(), "actual Backfill seed is retained");
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
