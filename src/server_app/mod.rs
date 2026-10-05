//! Host composition of simulation and optional presentation.
use bevy::prelude::*;
#[cfg(feature = "viewer")]
pub use phoenix_presentation::server_app_render::procedural_mesh_material;
pub use phoenix_presentation::server_app_render::ProceduralMeshCache;
pub use phoenix_simulation::server_app::*;

pub fn add_simulation_plugins(app: &mut App) {
    add_simulation_plugins_with(app, SimPluginOptions::default());
}
pub fn add_simulation_plugins_with(app: &mut App, opts: SimPluginOptions) {
    phoenix_simulation::server_app::add_simulation_plugins_with(app, opts);
    #[cfg(feature = "server")]
    {
        use crate::authoritative::{DeclareState, StateClass};
        // Optional adapter inputs register their types even on renderer-less hosts.
        app.declare_state::<crate::presentation_contracts::TestWindowVisibility>(
            StateClass::TestInfra,
            "gm-milestone-integrated-workshop",
        )
        .declare_state::<crate::server::viewscreen_border::ViewscreenEndpointInput>(
            StateClass::Presentation,
            "viewscreen-motion-state",
        )
        .declare_state::<crate::server::asset_preload::AssetPreloadResource>(
            StateClass::Presentation,
            "asset-loading-state",
        );
    }
    #[cfg(all(not(phoenix_demo_build), feature = "server"))]
    app.add_systems(
        PreUpdate,
        (
            crate::server::bridge::drain_client_debug_flags,
            crate::debug_overlay::drain_client_pause,
        )
            .chain()
            .before(crate::debug::catalogue::refresh_readback)
            .run_if(
                resource_exists::<crate::debug_overlay::SimulationPaused>
                    .and(resource_exists::<crate::lobby::Sessions>),
            ),
    );

    #[cfg(feature = "server")]
    app.add_systems(
        First,
        crate::presentation_adapters::apply_test_visibility
            .after(crate::workshop::test_clock::apply_controls),
    );
    if opts.render {
        phoenix_presentation::registration::add_presentation_plugins(app);
    }
    #[cfg(feature = "server")]
    app.add_systems(
        Update,
        crate::presentation_adapters::sample_endpoint_input
            .before(crate::server::viewscreen_border::ResolveMotion),
    );
    #[cfg(all(feature = "server", target_arch = "wasm32"))]
    app.add_systems(
        PostUpdate,
        crate::presentation_adapters::mirror_debug_readbacks
            .after(crate::debug::modifiers::publish_modifier_debug)
            .after(crate::debug::damage::publish_damage_debug)
            .after(crate::debug::entities::publish_entity_behavior_debug)
            .after(crate::debug::inspector::publish_entity_inspector_debug)
            .before(crate::server::bridge::flush_host_channels),
    );
}
