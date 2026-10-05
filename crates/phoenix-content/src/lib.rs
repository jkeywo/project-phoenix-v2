//! Authored content preparation. Games install the validated result separately.
#![forbid(unsafe_code)]
pub mod entity_override;

pub mod findings;
pub mod include_resolve;
pub mod ledger;
pub mod manifest;
pub mod overlay;
pub mod template_preload;

mod codec;
pub mod pack_asset_validation;
pub mod sound_cues;

pub mod archive;
pub mod string_catalogue;

pub mod rig;
