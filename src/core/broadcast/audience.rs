use crate::core::messages::{StationId, SystemId};
use crate::lobby::handler::Target;
use crate::lobby::session::SessionManager;
use crate::ship::config::ShipConfig;

/// Who receives a broadcast message.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Holding(StationId),
    /// Dynamically resolves the station that owns the given system from the
    /// ship config, then targets that station's holder.  Falls back to
    /// `None` (skip broadcast) when no config is available.
    HoldingSystem(SystemId),
    /// Dynamically resolves the single station shared by every authored System
    /// of the given kind, then targets that station's holder. Instance ids are
    /// author-defined, so capability-level publishers use this instead of
    /// freezing one hull's id. Missing, ownerless, or split ownership skips the
    /// broadcast.
    HoldingSystemKind(String),
    /// Resolves the station owning this ship's weapons suite from the ship
    /// config, then targets that station's holder. `None` (skip broadcast)
    /// when there's no config, no weapons owner, or the station is unheld.
    ///
    /// Distinct from `Holding(StationId("tactical"))` because the owner is not
    /// always named "tactical" — the single-station Courier puts its blaster on
    /// "pilot". See [`ShipConfig::weapons_station`].
    HoldingWeapons,
    Token(String),
    AllExcept(String),
}

impl Audience {
    /// Resolve this audience to a `Target` given current session state.
    /// Returns `None` when `Holding` names a station with no current holder,
    /// or when a System-derived audience cannot determine one owning station
    /// from the ship config, signalling the caller to skip this broadcast.
    pub fn resolve(
        &self,
        sessions: &SessionManager,
        ship_config: Option<&ShipConfig>,
    ) -> Option<Target> {
        match self {
            Audience::All => Some(Target::All),
            Audience::Holding(station_id) => sessions
                .holder_for_station(station_id)
                .map(|t| Target::Token(t.to_string())),
            Audience::HoldingSystem(system_id) => {
                let station_id = ship_config
                    .and_then(|config| config.system(system_id))
                    .and_then(|sys| sys.station.clone())?;
                sessions
                    .holder_for_station(&station_id)
                    .map(|t| Target::Token(t.to_string()))
            }
            Audience::HoldingSystemKind(kind) => {
                let station_id = ship_config?.station_for_system_kind(kind)?;
                sessions
                    .holder_for_station(&station_id)
                    .map(|t| Target::Token(t.to_string()))
            }
            Audience::HoldingWeapons => {
                let station_id = ship_config.and_then(|config| config.weapons_station())?;
                sessions
                    .holder_for_station(&station_id)
                    .map(|t| Target::Token(t.to_string()))
            }
            Audience::Token(t) => Some(Target::Token(t.clone())),
            Audience::AllExcept(t) => Some(Target::AllExcept(t.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
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
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm".into(),
                    rank: "".into(),
                    short_code: "H".into(),
                    ratings: vec![],
                    console: None,
                    manual_overview: None,
                    tutorials: vec![],
                    human_seeking: false,
                    host_order: vec![],
                    visiting_rating: None,
                    auxiliary: false,
                    command_target: None,
                    stances: vec![],
                },
            ],
            systems: vec![
                SystemInstanceConfig {
                    id: SystemId("dummy".into()),
                    kind: "dummy".into(),
                    station: None,
                    ai_only: true,
                    human_seeking: false,
                    seek_order: Vec::new(),
                    power_group: None,
                    marker: None,
                    config: None,
                },
                SystemInstanceConfig {
                    id: SystemId("power-reactor".into()),
                    kind: "power_reactor".into(),
                    station: Some(StationId("power".into())),
                    ai_only: true,
                    human_seeking: false,
                    seek_order: Vec::new(),
                    power_group: None,
                    marker: None,
                    config: None,
                },
                SystemInstanceConfig {
                    id: SystemId("helm-thruster".into()),
                    kind: "helm".into(),
                    station: Some(StationId("helm".into())),
                    ai_only: true,
                    human_seeking: false,
                    seek_order: Vec::new(),
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
            coordination_lag_secs: 2.0,
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
            Audience::All.resolve(&sm, Some(&ship_config())),
            Some(Target::All)
        );
    }

    #[test]
    fn audience_holding_returns_none_when_no_holder() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::Holding(StationId("power".into())).resolve(&sm, Some(&ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_returns_token_when_station_held() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        assert_eq!(
            Audience::Holding(StationId("power".into())).resolve(&sm, Some(&ship_config())),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_returns_none_for_different_station() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        assert_eq!(
            Audience::Holding(StationId("helm".into())).resolve(&sm, Some(&ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_system_resolves_to_station_holder() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        assert_eq!(
            Audience::HoldingSystem(SystemId("power-reactor".into()))
                .resolve(&sm, Some(&ship_config())),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_system_kind_uses_authored_kind_not_instance_id() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        assert_eq!(
            Audience::HoldingSystemKind("power_reactor".into()).resolve(&sm, Some(&ship_config())),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_system_returns_none_when_no_config() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        assert_eq!(
            Audience::HoldingSystem(SystemId("power-reactor".into())).resolve(&sm, None),
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
            Audience::HoldingSystem(SystemId("power-reactor".into()))
                .resolve(&sm, Some(&ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_system_returns_none_for_unknown_system() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::HoldingSystem(SystemId("nonexistent".into()))
                .resolve(&sm, Some(&ship_config())),
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
            console: None,
            manual_overview: None,
            tutorials: vec![],
            human_seeking: false,
            host_order: vec![],
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: vec![],
        });
        config.systems.push(SystemInstanceConfig {
            id: SystemId("blaster-fore".into()),
            kind: "blaster_bank".into(),
            station: Some(StationId("pilot".into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        });
        config
    }

    #[test]
    fn audience_holding_weapons_resolves_to_the_weapons_station_holder() {
        let sm = sm_with_holder("tok1", StationId("pilot".into()));
        assert_eq!(
            Audience::HoldingWeapons.resolve(&sm, Some(&pilot_ship_config())),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_weapons_returns_none_when_weapons_station_unheld() {
        let sm = sm_with_holder("tok1", StationId("helm".into()));
        assert_eq!(
            Audience::HoldingWeapons.resolve(&sm, Some(&pilot_ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_weapons_returns_none_when_ship_has_no_weapons_owner() {
        let sm = sm_with_holder("tok1", StationId("power".into()));
        // The base fixture declares no weapon systems and no tactical station,
        // which is the NPC shape.
        assert_eq!(
            Audience::HoldingWeapons.resolve(&sm, Some(&ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_weapons_returns_none_when_no_config() {
        let sm = sm_with_holder("tok1", StationId("pilot".into()));
        assert_eq!(Audience::HoldingWeapons.resolve(&sm, None), None);
    }

    #[test]
    fn audience_token_always_resolves() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::Token("abc".to_string()).resolve(&sm, Some(&ship_config())),
            Some(Target::Token("abc".to_string()))
        );
    }

    #[test]
    fn audience_all_except_always_resolves() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::AllExcept("abc".to_string()).resolve(&sm, Some(&ship_config())),
            Some(Target::AllExcept("abc".to_string()))
        );
    }
}
