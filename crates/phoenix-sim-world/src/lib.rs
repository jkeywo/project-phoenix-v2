//! Scenario parsing, scripting, objectives, task lifecycles and narrative state.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub use phoenix_math::{audio_config, bounded_history, simmath};
pub use phoenix_sim_contracts::{
    __plog_gated, effect_queue, logging, pdebug, perror, pinfo, presentation_contracts, ptrace,
    pwarn, sim_rng, sim_tick, world_id,
};
pub mod core {
    pub use phoenix_sim_contracts::messages;
    pub mod balance {
        pub use phoenix_sim_contracts::outcome::Outcome;
    }
    pub mod computer_message;
    pub mod narrative;
    pub mod report;
    pub mod task_lifecycle;
}
pub mod entities {
    pub mod config {
        pub use phoenix_sim_contracts::{doctrine::*, scene::*};
    }
}
pub mod ship {
    pub mod config {
        pub use phoenix_sim_contracts::stance::*;
    }
}
pub mod modifiers {
    pub use phoenix_sim_contracts::modifiers::*;
    pub use phoenix_sim_contracts::power as power_system;
}
pub mod civilian {
    pub use phoenix_model::wire::CivilianOrder;
    pub use phoenix_sim_contracts::routes::*;
}
pub mod command_admission {
    pub mod log {
        pub use phoenix_model::wire::ShipKey;
    }
}
pub mod gm_attention;
pub mod gm_comms;
pub mod gm_information;
pub mod gm_npc;
pub mod gm_objective;
pub mod gm_presentation;
pub mod gm_quiet;
pub mod gm_workload;
pub mod objective_instances;
pub mod objectives;
pub mod recipients;
pub mod world;
pub mod comms {
    pub mod ai_choice;
    pub mod content;
}
pub mod dossier {
    pub mod evidence;
}
#[cfg(test)]
mod repo_fixtures;

pub mod commands;

pub use phoenix_content::ledger as content_ledger;
