use super::*;
use crate::core::messages::ConsoleId;
use crate::ship::components::ConsoleHosts;
use phoenix_model::console::{ConsoleConfig, ConsoleHostSnapshot};

#[test]
fn screens_follow_actual_hosts_and_screenless_scopes_are_not_interfaces() {
    let mut config: crate::ship::config::ShipConfig = toml::from_str("").unwrap();
    let home = StationId("home".into());
    let other = StationId("other".into());
    config.consoles = vec![
        ConsoleConfig {
            id: ConsoleId("primary".into()),
            home_station: Some(home.clone()),
            screen: Some("gui/battleship/helm.html".into()),
            ..Default::default()
        },
        ConsoleConfig {
            id: ConsoleId("visitor".into()),
            human_seeking: true,
            screen: Some("gui/battleship/helm.html".into()),
            ..Default::default()
        },
        ConsoleConfig {
            id: ConsoleId("ai-scope".into()),
            home_station: Some(other.clone()),
            ..Default::default()
        },
    ];
    let mut hosts = ConsoleHosts(vec![
        ConsoleHostSnapshot {
            console: ConsoleId("primary".into()),
            host: Some(other.clone()),
            rating: String::new(),
        },
        ConsoleHostSnapshot {
            console: ConsoleId("visitor".into()),
            host: Some(home.clone()),
            rating: String::new(),
        },
    ]);
    let ids = |station: &StationId, hosts: &ConsoleHosts| {
        station_consoles(&config, Some(hosts), station)
            .map(|console| console.id.0.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&home, &hosts), ["visitor"]);
    assert_eq!(ids(&other, &hosts), ["primary"]);
    hosts.0[0].host = None;
    hosts.0[1].host = None;
    assert_eq!(ids(&home, &hosts), ["primary"]);
    assert!(ids(&other, &hosts).is_empty());
}
