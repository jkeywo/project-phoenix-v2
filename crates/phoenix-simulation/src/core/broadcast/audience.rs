use crate::core::messages::{StationId, SystemId};
use crate::lobby::handler::Target;
use crate::lobby::session::SessionManager;
use crate::ship::config::ShipConfig;

/// Who receives a broadcast message.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Holding(StationId),
    /// Targets the current host of the controlling Console; missing placement skips delivery.
    HoldingSystem(SystemId),
    /// All instances must resolve to one current Console host; split or missing hosts skip delivery.
    HoldingSystemKind(String),
    /// Targets the shared current host of the declared weapons suite, without home-seat fallback.
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
        hosts: Option<&crate::ship::components::ConsoleHosts>,
    ) -> Option<Target> {
        match self {
            Audience::All => Some(Target::All),
            Audience::Holding(station_id) => sessions
                .holder_for_station(station_id)
                .map(|t| Target::Token(t.to_string())),
            Audience::HoldingSystem(system_id) => {
                let config = ship_config?;
                let station = current_system_host(config, hosts?, system_id)?;
                sessions
                    .holder_for_station(station)
                    .map(|token| Target::Token(token.into()))
            }
            Audience::HoldingSystemKind(kind) => {
                let config = ship_config?;
                let station = common_system_host(
                    config,
                    hosts?,
                    config
                        .systems
                        .iter()
                        .filter(|system| &system.kind == kind)
                        .map(|system| &system.id),
                )?;
                sessions
                    .holder_for_station(station)
                    .map(|token| Target::Token(token.into()))
            }
            Audience::HoldingWeapons => {
                use crate::ship::system_registry::*;
                let config = ship_config?;
                let station = common_system_host(
                    config,
                    hosts?,
                    config
                        .systems
                        .iter()
                        .filter(|system| {
                            [
                                PHASER_BANK_KIND,
                                BLASTER_BANK_KIND,
                                TORPEDO_TUBE_KIND,
                                TORPEDO_MAGAZINE_KIND,
                                TACTICAL_RADAR_KIND,
                            ]
                            .contains(&system.kind.as_str())
                        })
                        .map(|system| &system.id),
                )?;
                sessions
                    .holder_for_station(station)
                    .map(|token| Target::Token(token.into()))
            }
            Audience::Token(t) => Some(Target::Token(t.clone())),
            Audience::AllExcept(t) => Some(Target::AllExcept(t.clone())),
        }
    }
}

fn current_system_host<'a>(
    config: &ShipConfig,
    hosts: &'a crate::ship::components::ConsoleHosts,
    system: &SystemId,
) -> Option<&'a StationId> {
    let console = config.console_for_system(system)?;
    hosts
        .0
        .iter()
        .find(|host| host.console == console.id)?
        .host
        .as_ref()
}
fn common_system_host<'a, 's>(
    config: &ShipConfig,
    hosts: &'a crate::ship::components::ConsoleHosts,
    mut systems: impl Iterator<Item = &'s SystemId>,
) -> Option<&'a StationId> {
    let first = current_system_host(config, hosts, systems.next()?)?;
    systems
        .all(|system| current_system_host(config, hosts, system) == Some(first))
        .then_some(first)
}

#[cfg(test)]
#[path = "audience_tests.rs"]
mod tests;
