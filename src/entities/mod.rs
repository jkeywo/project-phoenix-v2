/// Which AI-capable fine systems each hull declares, which ones a synthesiser
/// invents for it, and the (default-off) strict mode that turns a missing
/// declaration into a load error (issue #885a).
pub mod ai_declaration_manifest;
/// Which fine-system AI hosts can evaluate `flag()`/`counter()` guards, and the
/// load-time rejection for the ones that cannot (issue #891 stage 1).
pub mod ai_flag_hosts;
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
/// Ordered entity-template `includes`, resolved and merged into ONE final TOML
/// document — with provenance — before validation and spawning (issue #869).
pub mod include_resolve;
pub mod loader;
pub mod marker_validate;
pub mod model_rig;
pub mod planet;
pub mod spawner;
pub mod star;
pub mod tags;
pub mod target;
