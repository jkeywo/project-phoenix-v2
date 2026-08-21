pub mod dispatch;
/// The pure, Bevy-free external repair-dispatch config, refusal vocabulary and
/// eligibility verdict (issue #1161).
pub mod external;
/// The Bevy adapter for external repair dispatch (issue #1161): the per-ship
/// component and the fixed-tick command / maintenance / work systems.
pub mod external_server;
pub mod server;
pub mod visibility;

pub use dispatch::handle_dispatch_repair_team;
pub use external::{ExternalRepairConfig, ExternalRepairRefusal};
pub use external_server::{
    operate_external_repair_ai, ExternalRepairDispatch, ExternalRepairSaveState,
};
pub use server::*;
