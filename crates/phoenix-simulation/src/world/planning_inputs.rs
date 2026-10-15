//! Dated participation/control inputs for the sole World plan choice. World
//! never reads host-local Sessions, participant tokens or private job progress.
use crate::ship_plugin::{
    ActiveStationRatings, HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources,
};
use phoenix_sim_world::world::planning::{temporal::*, Decomposition};

pub(crate) struct ForecastSources<'a> {
    pub config: &'a ShipConfigComponent,
    pub sources: &'a ShipSystemControlSources,
    pub ratings: &'a ActiveStationRatings,
    pub presence: &'a crate::lobby::crew_replication::ShipCrewPresence,
    pub hosts: Option<&'a HumanSeekingHosts>,
    pub tick_nanos: u64,
}
impl ForecastSources<'_> {
    pub fn project(&self, graph: &Decomposition) -> ForecastInput {
        use crate::ship::control_source::ControlSource;
        let owners = graph
            .leaves
            .values()
            .map(|row| {
                let owner = row.request.owner();
                let value = if !self.sources.0.entries().any(|(id, _)| id == owner) {
                    OwnerInput::Unavailable {
                        reason: UnavailableReason::MissingSource,
                    }
                } else if self.sources.0.is_offline(owner) {
                    OwnerInput::Unavailable {
                        reason: UnavailableReason::Offline,
                    }
                } else {
                    match self.sources.0.source_for(owner) {
                        ControlSource::Ai => OwnerInput::Ai {},
                        ControlSource::Simplified => OwnerInput::Simplified {},
                        ControlSource::Offline => OwnerInput::Unavailable {
                            reason: UnavailableReason::Offline,
                        },
                        ControlSource::Human => self.human(owner),
                    }
                };
                (owner.clone(), value)
            })
            .collect();
        ForecastInput {
            tick_nanos: self.tick_nanos,
            owners,
        }
    }
    fn human(&self, owner: &phoenix_model::identity::SystemId) -> OwnerInput {
        let Some(station) =
            crate::command_admission::policy::station_for_system(&self.config.0, self.hosts, owner)
        else {
            return OwnerInput::Unavailable {
                reason: UnavailableReason::MissingHost,
            };
        };
        if !self.presence.0.contains(&station) {
            return OwnerInput::Unavailable {
                reason: UnavailableReason::Unoccupied,
            };
        }
        let Some(rating) = self
            .ratings
            .0
            .get(&station)
            .filter(|rating| supported_rating(&self.config.0, &station, rating))
        else {
            return OwnerInput::Unavailable {
                reason: UnavailableReason::InvalidRating,
            };
        };
        OwnerInput::Human {
            station,
            rating: rating.clone(),
        }
    }
}
fn supported_rating(
    config: &crate::ship::config::ShipConfig,
    station: &phoenix_model::identity::StationId,
    rating: &str,
) -> bool {
    rating != crate::ship::rating::BACKFILL_RATING
        && config.station(station).is_some_and(|row| {
            row.ratings.iter().any(|choice| choice.name == rating)
                || (row.ratings.is_empty() && rating == "Std")
        })
}
/// Historical scope validation uses only accepted vocabulary/bindings. Today's
/// presence, policy, ratings and resource levels are deliberately absent.
pub(crate) fn validate_original(
    input: &ForecastInput,
    config: &crate::ship::config::ShipConfig,
) -> Result<(), String> {
    for (owner, value) in &input.owners {
        if config.system(owner).is_none() {
            return Err("original forecast owner is outside accepted equipment".into());
        }
        if let OwnerInput::Human { station, rating } = value {
            let allowed_host = config.console_for_system(owner).is_some_and(|console| {
                console.screen.is_some()
                    && (console.home_station.as_ref() == Some(station)
                        || (console.human_seeking && console.host_order.contains(station)))
            });
            if !allowed_host || !supported_rating(config, station, rating) {
                return Err(
                    "original forecast operator/rating is outside accepted vocabulary".into(),
                );
            }
        }
    }
    Ok(())
}
