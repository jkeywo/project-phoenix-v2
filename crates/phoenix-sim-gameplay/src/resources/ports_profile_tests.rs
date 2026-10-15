use super::*;

#[test]
fn actual_free_and_manual_facilities_bind_existing_paid_dock_without_granting_stock() {
    for path in [
        "assets/entities/station_axiom.toml",
        "assets/entities/manual_trade_port.toml",
    ] {
        let config = crate::test_fixtures::load_entity_config(path).unwrap();
        validate_entity_port(&config).unwrap();
        assert_eq!(
            config
                .port_service
                .as_ref()
                .unwrap()
                .system
                .as_ref()
                .unwrap()
                .0,
            "dock"
        );
        let inventory = config.infrastructure.as_ref().unwrap();
        assert_eq!(
            inventory
                .capacities
                .iter()
                .find(|row| row.id == "torpedoes")
                .unwrap()
                .amount,
            1000
        );
        assert_eq!(
            inventory
                .capacities
                .iter()
                .find(|row| row.id == "reserve_fuel")
                .unwrap()
                .amount,
            0
        );
    }
}

#[test]
fn binding_refuses_missing_wrong_kind_unfunded_or_nonphysical_producer() {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/station_axiom.toml").unwrap();
    for failure in 0..7 {
        let mut candidate = config.clone();
        match failure {
            0 => {
                candidate.port_service.as_mut().unwrap().system =
                    Some(crate::core::messages::SystemId("absent".into()))
            }
            1 => {
                candidate
                    .ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|row| row.id.0 == "dock")
                    .unwrap()
                    .kind = "sensors".into()
            }
            2 => candidate
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .iter_mut()
                .find(|row| row.id.0 == "dock")
                .unwrap()
                .components
                .clear(),
            3 => {
                candidate
                    .energy
                    .as_mut()
                    .unwrap()
                    .consumers
                    .iter_mut()
                    .find(|row| row.system.0 == "dock")
                    .unwrap()
                    .continuous_cost_per_sec = 0.0
            }
            4 => candidate
                .interior
                .as_mut()
                .unwrap()
                .placements
                .remove(&crate::core::messages::SystemId("dock".into()))
                .map(|_| ())
                .unwrap(),
            5 => candidate
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .iter_mut()
                .find(|row| row.id.0 == "dock")
                .unwrap()
                .damage_components
                .clear(),
            _ => candidate.damage = None,
        }
        assert!(validate_entity_port(&candidate).is_err(), "case {failure}");
    }
    let mut legacy = config;
    legacy.port_service.as_mut().unwrap().system = None;
    legacy.damage = None;
    validate_entity_port(&legacy).unwrap();
}
