use super::*;
use crate::{core::messages::SystemId, ship::composition::SystemComponentConfig};
use phoenix_model::transit::{LocalJumpConfig, TransitFuel};

fn equipped() -> EntityConfig {
    let mut config =
        crate::test_fixtures::load_entity_config("assets/entities/broadside_cruiser.toml").unwrap();
    let ship = config.ship_config.as_mut().unwrap();
    let system = ship
        .systems
        .iter_mut()
        .find(|system| system.id.0 == "blaster-side")
        .unwrap();
    system.kind = TRANSIT_DRIVE_KIND.into();
    system.components = vec![
        SystemComponentConfig::PowerInput { id: "power".into() },
        SystemComponentConfig::TransitDrive {
            id: "drive".into(),
            power_input: "power".into(),
            navigation_source: SystemId("navigation".into()),
            config: LocalJumpConfig {
                preparation_work_secs: 2.0,
                max_distance: 100.0,
                scatter_radius: 1.0,
                gravity_response: 1.0,
                fuel: TransitFuel {
                    store: "local-kinetic-rounds".into(),
                    amount: 2,
                },
                reading_ticks: 120,
            },
        },
    ];
    system.channels.clear();
    let console = system.console.clone().unwrap();
    ship.consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: SystemId("navigation".into()),
            kind: ConsoleObservationKind::Navigation,
        });
    let energy = config
        .energy
        .as_mut()
        .unwrap()
        .consumers
        .iter_mut()
        .find(|row| row.system.0 == "blaster-side")
        .unwrap();
    energy.continuous_cost_per_sec = 1.0;
    energy.discrete_cost = 2.0;
    config
}

#[test]
fn transit_binds_exact_observed_navigation_and_real_local_stock_without_copying_counts() {
    let config = equipped();
    validate_entity(&config).unwrap();
    let system = config
        .ship_config
        .as_ref()
        .unwrap()
        .system(&SystemId("blaster-side".into()))
        .unwrap();
    let encoded = toml::to_string(system).unwrap();
    assert_eq!(
        toml::from_str::<crate::ship::config::SystemInstanceConfig>(&encoded).unwrap(),
        *system
    );
    let fuel = transit_composition(&system.components)
        .unwrap()
        .config
        .fuel
        .clone();
    assert_ne!(
        fuel.store, "side-kinetic-rounds",
        "managed store ID is not the capacity ID"
    );
    assert_eq!(fuel.amount, 2);
}

#[test]
fn unpermitted_or_wrong_kind_course_and_unowned_or_unpayable_fuel_refuse() {
    for fault in 0..9 {
        let mut config = equipped();
        match fault {
            0 | 1 => {
                let ship = config.ship_config.as_mut().unwrap();
                let console = ship
                    .system(&SystemId("blaster-side".into()))
                    .unwrap()
                    .console
                    .clone()
                    .unwrap();
                let feeds = &mut ship
                    .consoles
                    .iter_mut()
                    .find(|row| row.id == console)
                    .unwrap()
                    .feeds;
                feeds.retain(|feed| feed.source.0 != "navigation");
                if fault == 1 {
                    feeds.push(phoenix_model::console::ConsoleFeed {
                        source: SystemId("navigation".into()),
                        kind: ConsoleObservationKind::Helm,
                    });
                }
            }
            2 => {
                config
                    .ship_config
                    .as_mut()
                    .unwrap()
                    .systems
                    .iter_mut()
                    .find(|system| system.id.0 == "navigation")
                    .unwrap()
                    .kind = DOCK_KIND.into()
            }
            3 | 4 | 5 => {
                let store = config
                    .logistics
                    .as_mut()
                    .unwrap()
                    .stores
                    .iter_mut()
                    .find(|row| row.id == "local-kinetic-rounds")
                    .unwrap();
                if fault == 3 {
                    store.system = Some(SystemId("other-drive".into()));
                }
                if fault == 4 {
                    store.role = StoreRole::Main;
                }
                if fault == 5 {
                    store.endpoint = ResourceEndpoint::TorpedoMagazine {};
                }
            }
            6 | 7 => {
                let energy = config
                    .energy
                    .as_mut()
                    .unwrap()
                    .consumers
                    .iter_mut()
                    .find(|row| row.system.0 == "blaster-side")
                    .unwrap();
                if fault == 6 {
                    energy.continuous_cost_per_sec = 0.0;
                } else {
                    energy.discrete_cost = 0.0;
                }
            }
            _ => config
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .iter_mut()
                .find(|system| system.id.0 == "blaster-side")
                .unwrap()
                .components
                .pop()
                .map(|_| ())
                .unwrap(),
        }
        assert!(validate_entity(&config).is_err(), "fault{fault}");
    }
}
