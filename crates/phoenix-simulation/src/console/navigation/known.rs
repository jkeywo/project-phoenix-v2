//! Permitted local chart choices; foreign Transit arrivals stay a separate feed.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use phoenix_model::navigation_point::NavigationPointView;

pub fn charted(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    reader: &SystemId,
    source: &SystemId,
) -> Vec<NavigationPointView> {
    if config
        .0
        .system(source)
        .is_none_or(|system| system.kind != crate::ship::system_registry::NAVIGATION_KIND)
    {
        return Vec::new();
    }
    let permitted = config.0.observations_for_system(reader, &boards.0);
    let Some(SystemBlackboard::Navigation(board)) = permitted.get(source) else {
        return Vec::new();
    };
    board.charted_points.clone()
}
