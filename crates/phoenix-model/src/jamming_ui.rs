//! Own jammer intent/emission only. No physical receiver census or success.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JammingControlsView {
    pub enabled: bool,
    pub target: Option<String>,
    pub emitting: bool,
    pub controllable: bool,
    pub targets: Vec<crate::guided_orders::GuidanceTargetOption>,
}
