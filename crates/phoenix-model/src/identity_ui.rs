//! Own transponder intent/capability only; no observer census or deception success.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityControlsView {
    pub enabled: bool,
    pub active: bool,
    pub callsign: String,
    pub faction: Option<String>,
    pub controllable: bool,
}
