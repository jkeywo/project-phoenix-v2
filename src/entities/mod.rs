/// Which AI-capable fine systems each hull declares, and the strict mode — ON by
/// default since #885b stage 5d — that turns a missing declaration into a load
/// error (issue #885a).
pub mod ai_declaration_manifest;
/// Which fine-system AI hosts can evaluate `flag()`/`counter()` guards, and the
/// load-time rejection for the ones that cannot (issue #891 stage 1).
pub mod ai_flag_hosts;
/// The fine-system AI policy schema — channel/verb/selector-source name tables,
/// the `FineSystemAi*Toml` types, verb decoding, and load-time policy/selector
/// validation — extracted verbatim from `config` and re-exported by it (#1196).
pub mod ai_policy_schema;
/// Behavioural pins for the AUTHORED fine-system AI declarations: the fleet
/// baseline the bespoke doctrines are measured against, the guard truth tables,
/// and the selector ordering invariants (issue #885b stage 5d). Test-only, and
/// the successor to the deleted `default_ai_policy_pins`.
#[cfg(test)]
pub(crate) mod authored_ai_pins;
pub mod billboard;
pub mod celestial_visual;
pub mod config;
pub mod config_cache;
pub mod entity_override;
pub mod glb_visual;
/// Ordered entity-template `includes`, resolved and merged into ONE final TOML
/// document — with provenance — before validation and spawning (issue #869).
pub mod include_resolve;
pub mod loader;
pub mod marker_validate;
/// Triangle and pixel counting, shared by the perf pass and the model viewer.
pub mod mesh_stats;
pub mod model_rig;
pub mod planet;
pub mod spawner;
pub mod star;
pub mod tags;
pub mod target;
/// Fading a visual in or out — the LOD cross-fade window and the mid-mission
/// arrival flourish built on it (PRD #1023, module 5).
pub mod visual_fade;
