pub mod celestial_visual;
pub mod config;
pub mod config_cache;
/// Behavioural pins for the fourteen `default_*_ai_config()` synthesisers
/// (issue #885 step 1). Test-only, and expected to be deleted together with the
/// synthesisers when #885 lands.
#[cfg(test)]
mod default_ai_policy_pins;
pub mod entity_override;
pub mod glb_visual;
pub mod loader;
pub mod marker_validate;
pub mod model_rig;
pub mod planet;
pub mod spawner;
pub mod star;
pub mod tags;
pub mod target;
