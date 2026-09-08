//! Register authoritative World materialization in the caller's schedule.
//!
//! Startup and a native deferred load run these same systems, with the same
//! deferred-command boundaries. They remain ordinary systems in the supplied
//! schedule, so other plugins' function-identity ordering edges still apply.
//! The caller owns ingest, hull selection and the ID mint's lifecycle. GameStart
//! spawning and render-only setup are separate phases, not part of this pass.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

/// Ordering anchor for work that consumes the completed World materialization.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldMaterialization;

/// Install one materialization pass. Register once per supplied schedule after
/// installing the simulation resources and World plugin dependencies.
pub fn register(app: &mut App, schedule: impl ScheduleLabel) -> &mut App {
    app.add_systems(
        schedule,
        (
            super::server::insert_world_config_resource,
            super::server::insert_raw_world_source_resource,
            super::server::compile_world_scripts,
            super::server::freeze_host_preloaded_content,
            // The script activation gate must precede BOTH spawn halves.
            // Anonymous entries mint before named/asteroid entries; reversing
            // them assigns the same IDs to different entities (#984).
            crate::server_app::setup_world,
            super::server::spawn_world_entities,
            super::server::init_world_runtime,
            super::server::load_extra_worlds,
        )
            .chain()
            .in_set(WorldMaterialization),
    )
}
