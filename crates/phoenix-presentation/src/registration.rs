//! Registers render systems over the simulation's public projections.
#[cfg(feature = "server")]
use crate::core::messages::GamePhase;
use crate::server_app_render::{
    face_player_lights, render_spawned_entities, update_mesh_lod, ProceduralMeshCache,
};
use bevy::prelude::*;

pub fn add_presentation_plugins(app: &mut App) {
    #[cfg(feature = "server")]
    use crate::authoritative::{DeclareState, StateClass};
    #[cfg(feature = "server")]
    app.declare_state::<crate::server::asset_preload::AssetPreloadResource>(
        StateClass::Presentation,
        "asset-loading-state",
    )
    .declare_state::<crate::presentation_contracts::PresentationReadiness>(
        StateClass::Presentation,
        "asset-loading-state",
    );
    {
        app.add_plugins(crate::entities::star::StarRenderPlugin)
            .add_plugins(crate::entities::planet::PlanetRenderPlugin)
            .init_resource::<ProceduralMeshCache>()
            // The authored `[render]` calibration (PRD #1023). Initialised to
            // its own defaults here so the LOD swap always has one, and
            // overwritten from the world's block at `PostStartup` by the
            // renderer plugin.
            .init_resource::<crate::render_setup::RenderTuning>()
            .add_systems(Update, render_spawned_entities)
            .add_systems(Update, update_mesh_lod.after(render_spawned_entities))
            .add_systems(
                Update,
                // After the fade driver, so a billboard mid-cross-fade folds
                // THIS frame's fade alpha into its pose weights rather than
                // last frame's — the two share one alpha channel and
                // `orient_lod_billboards` is its only writer.
                crate::entities::billboard::orient_lod_billboards::<
                    crate::render_setup::GameCamera,
                >
                    .after(update_mesh_lod)
                    .after(crate::entities::visual_fade::drive_visual_fades),
            )
            .add_systems(
                Update,
                crate::entities::visual_fade::drive_visual_fades.after(update_mesh_lod),
            )
            .add_systems(Update, face_player_lights.after(render_spawned_entities));
    }

    #[cfg(feature = "server")]
    {
        use crate::server::asset_preload::{
            auto_transition_from_loading, begin_asset_preload, broadcast_loading_progress,
            broadcast_loading_start, poll_asset_preload,
        };
        app.init_resource::<crate::presentation_contracts::PresentationReadiness>()
            .add_systems(FixedFirst, publish_preload)
            .add_plugins(crate::server::ServerViewscreenRadarPlugin)
            // The reference grid reads the SAME hull config the viewscreen
            // radar above does, through the same `SelectedShipResource` +
            // config-cache path, so the two can never disagree about which hull
            // the player is flying. It attaches nothing to any simulation
            // entity — see the module note on why that matters for the digest.
            .add_plugins(crate::server::ReferenceGridPlugin)
            .init_resource::<crate::server::asset_preload::AssetPreloadResource>()
            .add_systems(Update, begin_asset_preload)
            .add_systems(Update, poll_asset_preload)
            .add_systems(OnEnter(GamePhase::Loading), broadcast_loading_start)
            .add_systems(
                Update,
                broadcast_loading_progress.run_if(in_state(GamePhase::Loading)),
            )
            // `FixedUpdate`, not `Update` (issue #907, applied to this system in
            // #1121's fix round). The readiness POLL stays frame-paced — it is
            // asset streaming, which is a function of disk and GPU and has no
            // business on the logical tick — but the `NextState<GamePhase>`
            // WRITE moves onto the tick, before `SimSet::Input`, alongside
            // `tick_countdown` and `native_host::solo_auto_start`, the two other
            // systems that start a mission.
            //
            // A `NextState` write from a frame schedule applies at the
            // frame-level `StateTransition`, so `OnEnter(GamePhase::InProgress)`
            // — and the player-ship mint inside it — would fire at a point whose
            // relationship to `SimTick` depends on frame pacing. That is exactly
            // what #907 ruled out, and until this moved it was the ONLY route to
            // `InProgress` for a crewed session: `solo_auto_start` had the
            // treatment and the path every real crew takes did not.
            //
            // The `.after(poll_asset_preload)` edge goes with the move — an
            // ordering edge is only real inside one schedule — and its loss
            // costs a frame of latency and nothing else: the poll writes
            // `preload.complete` in `Update`, this reads it on the next tick,
            // and `broadcast_loading_progress` keeps the client's bar moving in
            // the meantime.
            .add_systems(
                FixedUpdate,
                auto_transition_from_loading
                    .run_if(in_state(GamePhase::Loading))
                    .before(crate::sim_sets::SimSet::Input),
            );
    }
}

#[cfg(feature = "server")]
pub fn publish_preload(
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
    mut readiness: ResMut<crate::presentation_contracts::PresentationReadiness>,
) {
    readiness.started = preload.as_ref().is_some_and(|p| p.started);
    readiness.complete = preload.as_ref().is_some_and(|p| p.complete);
}
