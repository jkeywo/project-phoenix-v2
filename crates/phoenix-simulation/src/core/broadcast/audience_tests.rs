use super::*;
use crate::core::messages::{PowerGroupId, StationId, SystemId};
use crate::ship::config::{PowerGroupConfig, StationConfig, SystemInstanceConfig};

fn ship_config() -> ShipConfig {
    ShipConfig {
        stations: vec![
            StationConfig {
                id: StationId("power".into()),
                name: "Power".into(),
                description: "Power".into(),
                rank: "".into(),
                short_code: "P".into(),
                ratings: vec![],
                manual_overview: None,
                tutorials: vec![],
                stances: vec![],
            },
            StationConfig {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Helm".into(),
                rank: "".into(),
                short_code: "H".into(),
                ratings: vec![],
                manual_overview: None,
                tutorials: vec![],
                stances: vec![],
            },
        ],
        systems: vec![
            SystemInstanceConfig {
                components: Vec::new(),
                channels: Vec::new(),
                console: Some(phoenix_model::identity::ConsoleId("background".into())),
                id: SystemId("dummy".into()),
                kind: "dummy".into(),
                ai_only: true,

                power_group: None,
                marker: None,
                config: None,
            },
            SystemInstanceConfig {
                components: Vec::new(),
                channels: Vec::new(),
                console: Some(phoenix_model::identity::ConsoleId("power".into())),
                id: SystemId("power-reactor".into()),
                kind: "power_reactor".into(),
                ai_only: true,

                power_group: None,
                marker: None,
                config: None,
            },
            SystemInstanceConfig {
                components: Vec::new(),
                channels: Vec::new(),
                console: Some(phoenix_model::identity::ConsoleId("helm".into())),
                id: SystemId("helm-thruster".into()),
                kind: "helm".into(),
                ai_only: true,

                power_group: None,
                marker: None,
                config: None,
            },
        ],
        power_groups: std::iter::once((
            PowerGroupId("ops".into()),
            PowerGroupConfig {
                label: "Ops".into(),
                default_level: 2,
                min_level: 1,
                max_level: 4,
            },
        ))
        .collect(),
        consoles: vec![
            phoenix_model::console::ConsoleConfig {
                id: phoenix_model::identity::ConsoleId("background".into()),
                ..Default::default()
            },
            fixture_console("power"),
            fixture_console("helm"),
        ],
        coordination_lag_secs: 2.0,
    }
}

fn fixture_console(station: &str) -> phoenix_model::console::ConsoleConfig {
    phoenix_model::console::ConsoleConfig {
        id: phoenix_model::identity::ConsoleId(station.into()),
        home_station: Some(StationId(station.into())),
        screen: Some("fixture.html".into()),
        ..Default::default()
    }
}

fn sm_with_holder(token: &str, station: StationId) -> SessionManager {
    let mut sm = SessionManager::new();
    sm.register(token.to_string(), "Player".to_string())
        .unwrap();
    sm.set_station(token, Some(station));
    sm
}

#[test]
fn audience_all_always_resolves() {
    let sm = SessionManager::new();
    assert_eq!(
        Audience::All.resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::All)
    );
}

#[test]
fn audience_holding_returns_none_when_no_holder() {
    let sm = SessionManager::new();
    assert_eq!(
        Audience::Holding(StationId("power".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        None
    );
}

#[test]
fn audience_holding_returns_token_when_station_held() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    assert_eq!(
        Audience::Holding(StationId("power".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::Token("tok1".to_string()))
    );
}

#[test]
fn audience_holding_returns_none_for_different_station() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    assert_eq!(
        Audience::Holding(StationId("helm".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        None
    );
}

#[test]
fn audience_holding_system_resolves_to_station_holder() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    assert_eq!(
        Audience::HoldingSystem(SystemId("power-reactor".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::Token("tok1".to_string()))
    );
}

#[test]
fn audience_holding_system_kind_uses_authored_kind_not_instance_id() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    assert_eq!(
        Audience::HoldingSystemKind("power_reactor".into()).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::Token("tok1".to_string()))
    );
}

#[test]
fn audience_holding_system_returns_none_when_no_config() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    assert_eq!(
        Audience::HoldingSystem(SystemId("power-reactor".into())).resolve(&sm, None, None),
        None
    );
}

#[test]
fn audience_holding_system_returns_none_when_station_unheld() {
    let mut sm = SessionManager::new();
    sm.register("tok1".to_string(), "Player".to_string())
        .unwrap();
    // tok1 holds no station, so the system's station is unheld.
    assert_eq!(
        Audience::HoldingSystem(SystemId("power-reactor".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        None
    );
}

#[test]
fn audience_holding_system_returns_none_for_unknown_system() {
    let sm = SessionManager::new();
    assert_eq!(
        Audience::HoldingSystem(SystemId("nonexistent".into())).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        None
    );
}

/// A hull whose guns live on a station that isn't named "tactical". This
/// is the Courier shape and the reason `HoldingWeapons` exists.
fn pilot_ship_config() -> ShipConfig {
    let mut config = ship_config();
    config.stations.push(StationConfig {
        id: StationId("pilot".into()),
        name: "Pilot".into(),
        description: "Everything".into(),
        rank: "".into(),
        short_code: "PLT".into(),
        ratings: vec![],
        manual_overview: None,
        tutorials: vec![],
        stances: vec![],
    });
    config.systems.push(SystemInstanceConfig {
        components: Vec::new(),
        channels: Vec::new(),
        console: Some(phoenix_model::identity::ConsoleId("pilot".into())),
        id: SystemId("blaster-fore".into()),
        kind: "blaster_bank".into(),
        ai_only: false,

        power_group: None,
        marker: None,
        config: None,
    });
    config.consoles.push(fixture_console("pilot"));
    config
}

#[test]
fn audience_holding_weapons_resolves_to_the_weapons_station_holder() {
    let sm = sm_with_holder("tok1", StationId("pilot".into()));
    assert_eq!(
        Audience::HoldingWeapons.resolve(
            &sm,
            Some(&pilot_ship_config()),
            Some(&fixture_hosts(&pilot_ship_config()))
        ),
        Some(Target::Token("tok1".to_string()))
    );
}

#[test]
fn audience_holding_weapons_returns_none_when_weapons_station_unheld() {
    let sm = sm_with_holder("tok1", StationId("helm".into()));
    assert_eq!(
        Audience::HoldingWeapons.resolve(
            &sm,
            Some(&pilot_ship_config()),
            Some(&fixture_hosts(&pilot_ship_config()))
        ),
        None
    );
}

#[test]
fn audience_holding_weapons_returns_none_when_ship_has_no_weapons_owner() {
    let sm = sm_with_holder("tok1", StationId("power".into()));
    // The base fixture declares no weapon systems and no tactical station,
    // which is the NPC shape.
    assert_eq!(
        Audience::HoldingWeapons.resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        None
    );
}

#[test]
fn audience_holding_weapons_returns_none_when_no_config() {
    let sm = sm_with_holder("tok1", StationId("pilot".into()));
    assert_eq!(Audience::HoldingWeapons.resolve(&sm, None, None), None);
}

#[test]
fn audience_token_always_resolves() {
    let sm = SessionManager::new();
    assert_eq!(
        Audience::Token("abc".to_string()).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::Token("abc".to_string()))
    );
}

#[test]
fn audience_all_except_always_resolves() {
    let sm = SessionManager::new();
    assert_eq!(
        Audience::AllExcept("abc".to_string()).resolve(
            &sm,
            Some(&ship_config()),
            Some(&fixture_hosts(&ship_config()))
        ),
        Some(Target::AllExcept("abc".to_string()))
    );
}

fn fixture_hosts(config: &ShipConfig) -> crate::ship::components::ConsoleHosts {
    crate::ship::components::ConsoleHosts(
        config
            .consoles
            .iter()
            .map(|console| phoenix_model::console::ConsoleHostSnapshot {
                console: console.id.clone(),
                host: console.home_station.clone(),
                rating: "Std".into(),
            })
            .collect(),
    )
}
#[test]
fn system_audiences_follow_live_console_host_and_refuse_missing_or_split_hosts() {
    let config = pilot_ship_config();
    let mut hosts = fixture_hosts(&config);
    let mut sessions = sm_with_holder("home", StationId("pilot".into()));
    sessions
        .register("visitor".into(), "Visitor".into())
        .unwrap();
    sessions.set_station("visitor", Some(StationId("helm".into())));
    for audience in [
        Audience::HoldingWeapons,
        Audience::HoldingSystem(SystemId("blaster-fore".into())),
        Audience::HoldingSystemKind("blaster_bank".into()),
    ] {
        assert_eq!(audience.resolve(&sessions, Some(&config), None), None);
        assert_eq!(
            audience.resolve(&sessions, Some(&config), Some(&hosts)),
            Some(Target::Token("home".into()))
        );
    }
    hosts
        .0
        .iter_mut()
        .find(|host| host.console.0 == "pilot")
        .unwrap()
        .host = Some(StationId("helm".into()));
    for audience in [
        Audience::HoldingWeapons,
        Audience::HoldingSystem(SystemId("blaster-fore".into())),
        Audience::HoldingSystemKind("blaster_bank".into()),
    ] {
        assert_eq!(
            audience.resolve(&sessions, Some(&config), Some(&hosts)),
            Some(Target::Token("visitor".into()))
        );
    }
    hosts
        .0
        .iter_mut()
        .find(|host| host.console.0 == "pilot")
        .unwrap()
        .host = None;
    assert_eq!(
        Audience::HoldingWeapons.resolve(&sessions, Some(&config), Some(&hosts)),
        None
    );
}

#[test]
fn aggregate_system_audiences_reject_split_console_hosts() {
    let mut config = pilot_ship_config();
    let mut second = config.systems.last().unwrap().clone();
    second.id = SystemId("blaster-aft".into());
    second.console = Some(phoenix_model::identity::ConsoleId("helm".into()));
    config.systems.push(second);
    let mut sessions = sm_with_holder("pilot", StationId("pilot".into()));
    sessions.register("helm".into(), "Helm".into()).unwrap();
    sessions.set_station("helm", Some(StationId("helm".into())));
    let mut hosts = fixture_hosts(&config);
    for audience in [
        Audience::HoldingWeapons,
        Audience::HoldingSystemKind("blaster_bank".into()),
    ] {
        assert_eq!(
            audience.resolve(&sessions, Some(&config), Some(&hosts)),
            None
        );
    }
    hosts
        .0
        .iter_mut()
        .find(|h| h.console.0 == "helm")
        .unwrap()
        .host = Some(StationId("pilot".into()));
    for audience in [
        Audience::HoldingWeapons,
        Audience::HoldingSystemKind("blaster_bank".into()),
    ] {
        assert_eq!(
            audience.resolve(&sessions, Some(&config), Some(&hosts)),
            Some(Target::Token("pilot".into()))
        );
    }
}
