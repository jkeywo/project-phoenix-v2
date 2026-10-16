use super::*;

#[test]
fn listening_proof_fit_is_an_accepted_cruiser_with_only_opted_radio_and_feed_changes() {
    use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
    let load = |path| crate::test_fixtures::load_entity_config(path).unwrap();
    let base = load("assets/entities/alliance_cruiser.toml");
    let configured = load("tests/fixtures/listening-cruiser.toml");
    let original = base.ship_config.as_ref().unwrap();
    let ship = configured.ship_config.as_ref().unwrap();
    assert_eq!(ship.stations, original.stations);
    assert_eq!(ship.systems, original.systems);
    assert_eq!(configured.energy, base.energy);
    assert_eq!(configured.interior, base.interior);
    assert_eq!(ship.consoles.len(), original.consoles.len());
    for console in &original.consoles {
        let actual = ship
            .consoles
            .iter()
            .find(|row| row.id == console.id)
            .unwrap();
        let mut expected = console.clone();
        if console.id.0 == "comms" {
            expected.feeds.push(ConsoleFeed {
                source: SystemId("captain".into()),
                kind: ConsoleObservationKind::Captain,
            });
        }
        assert_eq!(actual, &expected);
    }
    let traffic = configured
        .comms
        .as_ref()
        .unwrap()
        .radio
        .as_ref()
        .unwrap()
        .traffic
        .as_ref()
        .unwrap();
    assert_eq!(traffic.listening.as_ref().unwrap().simultaneous_channels, 1);
    assert_eq!(
        traffic.listening.as_ref().unwrap().priority,
        ["public", "fleet"]
    );
    assert_eq!(traffic.alert.as_ref().unwrap().source.0, "captain");
    validate_radio(&configured).unwrap();
}

#[test]
fn all_shipped_seat_counts_bind_real_paid_radio_equipment() {
    for hull in ["courier", "destroyer", "cruiser", "battleship"] {
        let source = crate::test_fixtures::resolve_from_disk(&format!(
            "assets/entities/alliance_{hull}.toml"
        ))
        .unwrap();
        let config = EntityConfig::from_toml(&source.toml).unwrap();
        validate_radio(&config).unwrap();
        assert_eq!(
            config
                .comms
                .as_ref()
                .unwrap()
                .radio
                .as_ref()
                .unwrap()
                .channels
                .len(),
            2
        );
        for continuous in [false, true] {
            let mut unpaid = config.clone();
            let cost = unpaid
                .energy
                .as_mut()
                .unwrap()
                .consumers
                .iter_mut()
                .find(|consumer| consumer.system.0 == "comms")
                .unwrap();
            if continuous {
                cost.continuous_cost_per_sec = 0.0;
            } else {
                cost.discrete_cost = 0.0;
            }
            assert!(
                validate_radio(&unpaid).is_err(),
                "{hull} must not turn authored paid work into a free grant"
            );
        }
        let mut wrong = config.clone();
        wrong
            .comms
            .as_mut()
            .unwrap()
            .radio
            .as_mut()
            .unwrap()
            .system
            .0 = "helm-thrust".into();
        assert!(validate_radio(&wrong).is_err());
    }
}

#[test]
fn contextual_radio_requires_exact_explicit_own_feed_and_public_channel() {
    use phoenix_model::{
        comms_traffic::{CommsAlertTrafficConfig, CommsTrafficConfig},
        console::{ConsoleFeed, ConsoleObservationKind},
    };
    let source =
        crate::test_fixtures::resolve_from_disk("assets/entities/alliance_cruiser.toml").unwrap();
    let mut config = EntityConfig::from_toml(&source.toml).unwrap();
    let owner = SystemId("comms".into());
    let source = SystemId("captain".into());
    config
        .ship_config
        .as_mut()
        .unwrap()
        .consoles
        .iter_mut()
        .find(|console| console.id.0 == "comms")
        .unwrap()
        .feeds
        .push(ConsoleFeed {
            source: source.clone(),
            kind: ConsoleObservationKind::Captain,
        });
    config
        .comms
        .as_mut()
        .unwrap()
        .radio
        .as_mut()
        .unwrap()
        .traffic = Some(CommsTrafficConfig {
        listening: None,
        alert: Some(CommsAlertTrafficConfig {
            source,
            channel: "public".into(),
            minimum_spacing_ticks: 90,
            routine_text: "We have selected routine alert.".into(),
            red_alert_text: "We have selected red alert.".into(),
        }),
    });
    validate_radio(&config).unwrap();
    for fault in 0..5 {
        let mut bad = config.clone();
        match fault {
            0 => bad
                .ship_config
                .as_mut()
                .unwrap()
                .consoles
                .iter_mut()
                .find(|console| console.id.0 == "comms")
                .unwrap()
                .feeds
                .retain(|feed| feed.kind != ConsoleObservationKind::Captain),
            1 => {
                bad.comms
                    .as_mut()
                    .unwrap()
                    .radio
                    .as_mut()
                    .unwrap()
                    .traffic
                    .as_mut()
                    .unwrap()
                    .alert
                    .as_mut()
                    .unwrap()
                    .source = owner.clone()
            }
            2 => {
                bad.comms
                    .as_mut()
                    .unwrap()
                    .radio
                    .as_mut()
                    .unwrap()
                    .traffic
                    .as_mut()
                    .unwrap()
                    .alert
                    .as_mut()
                    .unwrap()
                    .channel = "fleet".into()
            }
            3 => bad
                .ship_config
                .as_mut()
                .unwrap()
                .systems
                .retain(|system| system.kind != crate::ship::system_registry::RED_ALERT_KIND),
            _ => {
                let mut duplicate = bad
                    .ship_config
                    .as_ref()
                    .unwrap()
                    .systems
                    .iter()
                    .find(|system| system.kind == crate::ship::system_registry::RED_ALERT_KIND)
                    .unwrap()
                    .clone();
                duplicate.id = SystemId("other-alert".into());
                bad.ship_config.as_mut().unwrap().systems.push(duplicate);
            }
        }
        assert!(
            validate_radio(&bad).is_err(),
            "context source fault {fault}"
        );
    }
}
