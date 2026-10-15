//! Own emitter controls/readout only; never a receiver census.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IlluminationControlsView {
    pub enabled: bool,
    pub target: Option<String>,
    /// The source paid and emitted its last interval; no receiver success claim.
    pub emitting: bool,
    pub controllable: bool,
    pub targets: Vec<crate::guided_orders::GuidanceTargetOption>,
}
