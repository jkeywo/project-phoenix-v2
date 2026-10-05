pub use phoenix_sim_gameplay::entities::ai_declaration_manifest::*;

#[cfg(test)]
#[path = "ai_declaration_manifest_tests.rs"]
mod tests;

#[cfg(test)]
use crate::entities::ai_flag_hosts::{self};
#[cfg(test)]
use crate::entities::config::EntityConfig;
