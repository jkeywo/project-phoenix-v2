use super::*;
use phoenix_model::console::{
    ConsoleConfig, ConsoleFeed, ConsoleHostSnapshot, ConsoleObservationKind,
};
use phoenix_model::identity::ConsoleId;

#[test]
fn handoff_revokes_previous_recipient_and_feeds_do_not_pool_between_tabs() {
    let mut config: crate::ship::config::ShipConfig =
        toml::from_str("coordination_lag_secs = 0.0").unwrap();
    let first = ConsoleId("first".into());
    let second = ConsoleId("second".into());
    let source = SystemId("observations".into());
    config.consoles = vec![
        ConsoleConfig {
            id: first.clone(),
            feeds: vec![ConsoleFeed {
                source: source.clone(),
                kind: ConsoleObservationKind::Viewscreen,
            }],
            ..Default::default()
        },
        ConsoleConfig {
            id: second.clone(),
            ..Default::default()
        },
    ];
    let home = StationId("home".into());
    let visitor = StationId("visitor".into());
    let mut hosts = ConsoleHosts(vec![
        ConsoleHostSnapshot {
            console: first,
            host: Some(home.clone()),
            rating: "Std".into(),
        },
        ConsoleHostSnapshot {
            console: second,
            host: Some(home.clone()),
            rating: "Std".into(),
        },
    ]);
    let boards = HashMap::from([(
        source.clone(),
        SystemBlackboard::Viewscreen(Default::default()),
    )]);
    assert_eq!(
        permitted_publications(&config, &hosts, Some(&home), &boards).len(),
        1
    );
    assert!(permitted_publications(&config, &hosts, Some(&visitor), &boards).is_empty());
    hosts.0[0].host = Some(visitor.clone());
    assert!(
        permitted_publications(&config, &hosts, Some(&home), &boards).is_empty(),
        "remaining cohosted tab cannot retain another Console's feed"
    );
    assert_eq!(
        permitted_publications(&config, &hosts, Some(&visitor), &boards).len(),
        1
    );
    assert!(permitted_publications(&config, &hosts, None, &boards).is_empty());
    let wrong_kind = HashMap::from([(source, SystemBlackboard::Helm(Default::default()))]);
    assert!(
        permitted_publications(&config, &hosts, Some(&visitor), &wrong_kind).is_empty(),
        "binding is typed, not only an id allowlist"
    );
}
