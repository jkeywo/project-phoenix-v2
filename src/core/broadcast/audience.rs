use crate::lobby_handler::Target;
use crate::messages::Console;
use crate::session::SessionManager;
use crate::ship::config::ShipConfig;

/// Who receives a broadcast message.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Holding(Console),
    Token(String),
    AllExcept(String),
}

impl Audience {
    /// Resolve this audience to a `Target` given current session state.
    /// Returns `None` when `Holding` names a console with no current holder or
    /// when `ship_config` is `None`, signalling the caller to skip this broadcast.
    pub fn resolve(
        &self,
        sessions: &SessionManager,
        ship_config: Option<&ShipConfig>,
    ) -> Option<Target> {
        match self {
            Audience::All => Some(Target::All),
            Audience::Holding(console) => {
                let cfg = ship_config?;
                sessions
                    .console_holder(console, cfg)
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
                    console: "power".into(),
                    ratings: vec![],
                },
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm".into(),
                    rank: "".into(),
                    short_code: "H".into(),
                    console: "helm".into(),
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

    fn sm_with_holder(token: &str, console: Console) -> SessionManager {
        let mut sm = SessionManager::new();
        sm.register(token.to_string(), "Player".to_string())
            .unwrap();
        sm.set_station(
            token,
            Some(StationId(console.station_console_id().to_string())),
        );
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
            Audience::Holding(Console::Power).resolve(&sm, Some(&ship_config())),
            None
        );
    }

    #[test]
    fn audience_holding_returns_token_when_console_held() {
        let sm = sm_with_holder("tok1", Console::Power);
        assert_eq!(
            Audience::Holding(Console::Power).resolve(&sm, Some(&ship_config())),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_returns_none_for_different_console() {
        let sm = sm_with_holder("tok1", Console::Power);
        assert_eq!(
            Audience::Holding(Console::Helm).resolve(&sm, Some(&ship_config())),
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
