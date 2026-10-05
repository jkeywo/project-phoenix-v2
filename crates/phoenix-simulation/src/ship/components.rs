pub use phoenix_sim_gameplay::ship::components::*;

/// Load `ShipConfigComponent` from `assets/entities/alliance_battleship.toml`.
///
/// Through the include resolver rather than `include_str!` (issue #876). That
/// hull is COMPOSED, so the bytes on disk are only half of it, and a baked site
/// can never see resolution: `include_str!` runs at compile time and the
/// resolver needs a fragment source at run time.
/// [`crate::entities::include_resolve::HostFragmentSource`] is the one source that
/// compiles on both targets — on native it falls through to the filesystem, on
/// WASM it reads the raw templates the host has already delivered.
///
/// This is a FALLBACK: every real boot inserts `PendingShipConfig` ahead of it
/// (`server::bridge::wasm_init` from `wasm_validate_stations`, `headless::app`
/// from `--ship`), so it is reached only by explicit fallback fixtures and
/// by a lobby that was handed no selection at all.
///
/// Panics if the file fails to compose or validate — the server cannot start
/// without a valid ship configuration, which is the contract this always had.
pub(crate) fn load_ship_config_from_disk() -> ShipConfigComponent {
    const HULL: &str = "assets/entities/alliance_battleship.toml";
    let resolved = crate::entities::include_resolve::resolve_template(
        HULL,
        &crate::entities::include_resolve::HostFragmentSource,
    )
    .unwrap_or_else(|e| panic!("ship_config: {HULL} failed to compose: {e}"));
    let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("core system registry must be valid");
    let kinds: Vec<&str> = registry.kinds().collect();
    match crate::ship::config::parse_and_validate(&resolved.toml, &kinds) {
        Ok(config) => {
            bevy::log::info!(
                "ship_config: loaded {} stations, {} systems",
                config.stations.len(),
                config.systems.len()
            );
            ShipConfigComponent(config)
        }
        Err(e) => panic!("ship_config: failed validation: {e}"),
    }
}

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;

#[cfg(test)]
use bevy::prelude::*;
