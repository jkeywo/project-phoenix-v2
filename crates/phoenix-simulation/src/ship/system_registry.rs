pub use phoenix_sim_gameplay::ship::system_registry::*;

#[cfg(test)]
#[path = "system_registry_tests.rs"]
mod tests;

#[cfg(test)]
use crate::core::messages::{ConsoleFamily, SystemId};
#[cfg(test)]
/// Wire `SystemId` for the Command coarse system (issue #1107).
///
/// The admitted-command target for an auxiliary Command station's stance
/// selection (`SystemControlPayload::SetStationStance`). Like the other
/// capability systems it owns no fine actuator — it is the seat a
/// `human_seeking` + `auxiliary` Command station carries so its stance orders
/// have a station to be authorised against (the seek host, normally Captain).
pub use phoenix_model::wire::COMMAND_SYSTEM_ID;
#[cfg(test)]
/// Wire `SystemId` for the Red Alert coarse system.
///
/// Ownerless capability — multi-word kebab id. Registry kind key is `"red_alert"`
/// (snake_case legacy quirk; see module-level doc for details).
pub use phoenix_model::wire::RED_ALERT_SYSTEM_ID;
#[cfg(test)]
/// Wire `SystemId` for the Viewscreen coarse system.
///
/// Ownerless capability — single-word lowercase id.
pub use phoenix_model::wire::VIEWSCREEN_SYSTEM_ID;
#[cfg(test)]
use std::collections::HashMap;
