use super::*;
use crate::core::messages::SystemId;
use phoenix_model::console::ConsoleObservationKind;

fn fixture() -> EntityConfig {
    let mut entity =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    // Use the actual fitted tool, ownership, tree, energy and room. These
    // profile tests vary only the acquisition range and supported material.
    let recovery = entity.recovery.as_mut().expect("shipped recovery profile");
    recovery.range = 20.0;
    recovery.resources = vec!["alloy".into()];
    entity
}

#[test]
fn fitted_tool_requires_actual_energy_managed_local_room_and_permitted_contacts() {
    let good = fixture();
    assert_eq!(compile_entity_recovery(&good).unwrap(), good.recovery);
    for mode in 0..6 {
        let mut bad = good.clone();
        let id = bad.recovery.as_ref().unwrap().system.clone();
        match mode {
            0 => bad.recovery = None,
            1 => bad
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .retain(|c| c.system != id),
            2 => {
                bad.interior
                    .as_mut()
                    .unwrap()
                    .placements
                    .insert(id.clone(), "engineering".into());
            }
            3 => {
                let ship = bad.ship_config.as_mut().unwrap();
                let console = ship.system(&id).unwrap().console.clone().unwrap();
                ship.consoles
                    .iter_mut()
                    .find(|c| c.id == console)
                    .unwrap()
                    .feeds
                    .retain(|feed| {
                        !matches!(
                            feed.kind,
                            ConsoleObservationKind::SensorRadar
                                | ConsoleObservationKind::TacticalRadar
                        )
                    });
            }
            4 => {
                let ship = bad.ship_config.as_mut().unwrap();
                let mut second = ship.system(&id).unwrap().clone();
                second.id = SystemId("second-recovery".into());
                ship.systems.push(second);
            }
            _ => {
                let ship = bad.ship_config.as_mut().unwrap();
                let console = ship.system(&id).unwrap().console.clone().unwrap();
                let mut private = ship
                    .consoles
                    .iter()
                    .find(|c| c.id == console)
                    .unwrap()
                    .clone();
                private.id = crate::core::messages::ConsoleId("recovery-private".into());
                private
                    .feeds
                    .retain(|feed| feed.kind != ConsoleObservationKind::Logistics);
                ship.systems
                    .iter_mut()
                    .find(|s| s.id == id)
                    .unwrap()
                    .console = Some(private.id.clone());
                ship.consoles.push(private);
            }
        }
        assert!(compile_entity_recovery(&bad).is_err(), "case {mode}");
    }
}

#[test]
fn radar_feed_uses_fixed_publication_identity_even_with_a_custom_machinery_id() {
    let mut entity = fixture();
    let ship = entity.ship_config.as_mut().unwrap();
    let radar = ship
        .systems
        .iter_mut()
        .find(|s| s.kind == crate::ship::system_registry::SENSOR_RADAR_KIND)
        .unwrap();
    let original = radar.id.clone();
    radar.id = SystemId("custom-scanner".into());
    let room = entity
        .interior
        .as_mut()
        .unwrap()
        .placements
        .remove(&original)
        .unwrap();
    entity
        .interior
        .as_mut()
        .unwrap()
        .placements
        .insert(SystemId("custom-scanner".into()), room);
    assert!(compile_entity_recovery(&entity).unwrap().is_some());
}

#[test]
fn recoverable_source_requires_real_capacity_or_magazine_and_a_physical_disabled_owner() {
    let mut entity = fixture();
    entity.recoverable = Some(RecoverableSourceConfig {
        availability: RecoveryAvailability::Disabled,
        resources: std::collections::BTreeMap::from([(
            "alloy".into(),
            phoenix_model::logistics::ResourceEndpoint::Capacity { id: "alloy".into() },
        )]),
    });
    validate_recoverable_source(&entity).unwrap();
    entity.damage = None;
    assert!(validate_recoverable_source(&entity).is_err());
    entity.recoverable.as_mut().unwrap().availability = RecoveryAvailability::Always;
    validate_recoverable_source(&entity).unwrap();
    entity.infrastructure = None;
    assert!(validate_recoverable_source(&entity).is_err());
    entity.recoverable.as_mut().unwrap().resources = std::collections::BTreeMap::from([(
        "torpedoes".into(),
        phoenix_model::logistics::ResourceEndpoint::TorpedoMagazine {},
    )]);
    validate_recoverable_source(&entity).unwrap();
    entity.torpedoes = None;
    assert!(validate_recoverable_source(&entity).is_err());
}
