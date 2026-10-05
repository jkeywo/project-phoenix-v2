pub use phoenix_sim_contracts::flags;
pub mod commitments;
pub mod config;
pub mod content;
pub mod deadlines;
pub mod delayed;
pub mod native_render_config;
pub mod script;
pub mod trigger_registry;
pub mod workforce;
pub mod validate {
    pub use phoenix_content::findings::*;
}

pub mod dispatch {
    pub use crate::commands::FlagMutation;
    pub type ActionCmd = crate::commands::ActionCmd<std::convert::Infallible>;
}
