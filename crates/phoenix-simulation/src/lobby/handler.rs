pub use phoenix_sim_session::lobby::handler::{
    derive_game_state, handle_release_station, handle_return_to_lobby, handle_select_station,
    handle_set_afk, handle_set_name, handle_set_ready, handle_set_spectator,
    handle_set_station_rating, is_reserved_token, process_disconnect, return_to_lobby_authority,
    CountdownAction, LobbyHandlerResult, ReturnToLobbyAuthority, Target,
};

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;

use crate::core::messages::{
    GamePhase, GmOperator, ServerMessage, ShipClientConfig, StationId, StringCatalogueSource,
    WorldData,
};
use crate::lobby::{session::SessionManager, stations_config::ShipStations};
use crate::ship::{config::ShipConfig, control_source::ControlSourceResolver, rating};
use std::collections::HashMap;

fn string_catalogues() -> Vec<StringCatalogueSource> {
    crate::entities::config_cache::active_packs()
        .into_iter()
        .filter_map(|pack| {
            pack.files
                .get(crate::world::mod_pack::STRING_CATALOGUE_PATH)
                .cloned()
                .map(|csv| StringCatalogueSource {
                    source: pack.id,
                    csv,
                })
        })
        .collect()
}

pub fn welcome_message(
    sessions: &SessionManager,
    phase: &GamePhase,
    world: Option<&WorldData>,
    ship_stations: &ShipStations,
    ship_config: &ShipClientConfig,
    station_ratings: &HashMap<StationId, String>,
    gm_operators: &[GmOperator],
) -> ServerMessage {
    phoenix_sim_session::lobby::handler::welcome_message(
        sessions,
        phase,
        world,
        ship_stations,
        ship_config,
        station_ratings,
        gm_operators,
        string_catalogues(),
    )
}

pub(crate) fn handle_identify(
    id_token: &str,
    name: &str,
    sessions: &mut SessionManager,
    phase: GamePhase,
    world: Option<&WorldData>,
    ship_stations: &ShipStations,
    ship_config: &ShipClientConfig,
    station_ratings: &HashMap<StationId, String>,
    gm_operators: &[GmOperator],
) -> LobbyHandlerResult {
    phoenix_sim_session::lobby::handler::handle_identify(
        id_token,
        name,
        sessions,
        phase,
        world,
        ship_stations,
        ship_config,
        station_ratings,
        gm_operators,
        string_catalogues(),
    )
}

/// Compatibility adapter for callers that apply disconnect directly to a Ship.
/// The ECS host uses the pure decision and applies its result at the agreed tick.
pub fn process_disconnect_with_stations(
    token: &str,
    sessions: &mut SessionManager,
    ship_stations: &ShipStations,
    ship_config: &ShipConfig,
    resolver: &mut ControlSourceResolver,
    station_ratings: &HashMap<StationId, String>,
    phase: GamePhase,
    preload_complete: bool,
) -> LobbyHandlerResult {
    let result = phoenix_sim_session::lobby::handler::process_disconnect_with_stations(
        token,
        sessions,
        ship_stations,
        station_ratings,
        phase,
        preload_complete,
    );
    if let Some((station, name)) = &result.station_rating_update {
        rating::apply_rating(ship_config, station, name, resolver);
    }
    result
}
