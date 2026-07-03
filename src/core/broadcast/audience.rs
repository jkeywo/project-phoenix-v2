use crate::lobby_handler::Target;
use crate::messages::StationId;
use crate::session::SessionManager;
use crate::ship::config::ShipConfig;

/// Who receives a broadcast message.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Holding(StationId),
    Token(String),
    AllExcept(String),
}

impl Audience {
    /// Resolve this audience to a `Target` given current session state.
    /// Returns `None` when `Holding` names a station with no current holder,
    /// signalling the caller to skip this broadcast.
    ///
    /// The `ship_config` parameter is retained for API stability with the
    /// broadcaster dispatchers but is unused for the new station-keyed
    /// `Holding` variant.
    pub fn resolve(
        &self,
        sessions: &SessionManager,
        _ship_config: Option<&ShipConfig>,
    ) -> Option<Target> {
        match self {
            Audience::All => Some(Target::All),
            Audience::Holding(station_id) => sessions
                .holder_for_station(station_id)
                .map(|t| Target::Token(t.to_string())),
            Audience::Token(t) => Some(Target::Token(t.clone())),
            Audience::AllExcept(t) => Some(Target::AllExcept(t.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{PowerGroupId, StationId, SystemId};
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
                },
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm".into(),
                    rank: "".into(),
                    short_code: "H".into(),
                    ratings: vec![],
                },
            ],
            systems: vec![SystemInstanceConfig {
                id: SystemId("dummy".into()),
                kind: "dummy".into(),
                station: None,
                ai_only: true,
                power_group: None,
                marker: None,
                config: None,
            }],
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
