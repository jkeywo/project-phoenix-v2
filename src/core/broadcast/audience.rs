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
#[path = "audience_tests.rs"]
mod tests;
