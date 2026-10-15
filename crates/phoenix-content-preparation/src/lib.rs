//! Offline Phoenix content preparation. All live installation stays with callers.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
pub use phoenix_content::{ledger as content_ledger, sound_cues};
pub use phoenix_sim_gameplay::core;
pub use phoenix_sim_gameplay::{ai, civilian, jobs, objectives, ship};
pub use phoenix_sim_world::{gm_attention, recipients};
pub mod entities {
    pub use phoenix_content::rig as model_rig;
    pub use phoenix_sim_gameplay::entities::{config, entity_override};
    pub mod include_resolve {
        pub use phoenix_content::include_resolve::*;
        pub use phoenix_sim_gameplay::entities::include_resolve::*;
        #[cfg(test)]
        pub struct FixtureFragments;
        #[cfg(test)]
        impl FragmentSource for FixtureFragments {
            fn read(&self, path: &str) -> Option<String> {
                crate::content_fs::read_to_string(path).ok()
            }
            fn absence_is_final(&self) -> bool {
                true
            }
        }
    }
    pub mod config_cache {
        #[cfg(test)]
        pub use phoenix_content::overlay::active_packs;
        pub use phoenix_content::overlay::ActivePack;
    }
    pub mod loader;
}
pub mod world {
    mod activity_bindings;
    pub mod craft;
    pub mod opportunities;
    mod planning;
    pub use phoenix_content::pack_asset_validation;
    pub use phoenix_sim_world::world::{config, script, transit_destinations};
    pub mod load;
    pub mod manifest;
    pub mod mod_pack;
    pub mod transit;
    pub mod validate;
}
pub mod authoring;
pub use authoring as workshop;
pub use phoenix_sim_contracts::inspector;
#[cfg(test)]
mod repo_fixtures;
#[cfg(test)]
use repo_fixtures::fs as content_fs;
#[cfg(all(not(test), not(target_arch = "wasm32")))]
use std::fs as content_fs;

pub mod ship_slots;
