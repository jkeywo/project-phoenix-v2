//! Typed transfer direction from the operating ship's point of view.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UmbilicalDirection {
    /// Operating ship sends stock to its docked partner.
    #[default]
    Deliver,
    /// Operating ship collects real stock from its docked partner.
    Collect,
}
