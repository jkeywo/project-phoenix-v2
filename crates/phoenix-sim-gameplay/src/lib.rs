//! Ship and entity mechanics, their controls, AI and template validation.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub use phoenix_math::{audio_config, bounded_history, composite_rng, simmath};
pub use phoenix_sim_contracts::{
    __plog_gated, authoritative, effect_queue, logging, pdebug, perror, pinfo,
    presentation_contracts, ptrace, pwarn, sim_sets, sim_tick, world_id,
};
pub mod core {
    pub mod balance;
    pub mod messages;
}
pub mod ai;
pub mod console;
pub mod entities;
pub mod modifiers;
pub mod objectives;
pub mod radar_config;
pub mod reference_grid;
pub mod regions;
pub mod ship;
pub mod weapons;
pub mod world {
    pub use phoenix_sim_contracts::flags;
    pub mod script {
        pub mod schedule {
            pub use phoenix_sim_contracts::sim_tick::seconds_to_ticks;
        }
    }
}
pub mod ship_plugin {
    pub use crate::ship::components::*;
}
pub mod civilian;
pub mod debris;
pub mod debug;
pub mod demolition;
pub mod dock;
pub mod dossier;
pub mod infrastructure;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod repo_fixtures;
pub mod science;
pub mod security;
pub mod tractor;
pub mod transporter;
pub mod umbilical;

pub mod sim_rng {
    pub use phoenix_sim_contracts::sim_rng::*;
    #[cfg(test)]
    #[allow(clippy::disallowed_methods)]
    pub fn unseeded_test_rng() -> vellum_rng::Pcg32 {
        vellum_rng::Pcg32::seeded(rand::random::<u64>(), 0)
    }
}

pub mod command_admission;
pub mod server_app;
pub mod lobby {
    pub use phoenix_sim_contracts::session_io::*;
    pub mod server {
        pub use phoenix_sim_contracts::session_io::*;
    }
}

#[cfg(test)]
mod test_fixtures;

pub mod console_ai {
    pub mod core;
}
