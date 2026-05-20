//! Backwards-compatibility shim — all helm panel logic now lives in
//! `crate::console::helm::client` (extracted as part of issue #328).
//!
//! Public items re-exported here so any code still importing from
//! `crate::phone_border::helm` continues to compile without change.

pub use crate::console::helm::client::{
    yaw_to_heading, HelmPanelPlugin, HelmTickTimer, PhoneHelmSpawned, HELM_PAD_SIZE,
};
