pub use phoenix_sim_contracts::logging::*;
mod filter;
pub use filter::refresh_log_entity_filter;

use bevy::prelude::*;
/// Registers the logging resource and the entity-filter maintenance system.
///
/// Insert a configured [`LogFilterConfig`] *before* adding this plugin to
/// override the default (warn everywhere, no entity filter).
pub struct LoggingPlugin;

impl Plugin for LoggingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogFilterConfig>().add_systems(
            PreUpdate,
            refresh_log_entity_filter.run_if(|cfg: Res<LogFilterConfig>| cfg.has_entity_filter()),
        );
    }
}
