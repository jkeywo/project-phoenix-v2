//! Own equipment facts only; no observer census or detection-success signal.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcealmentControlsView {
    pub enabled: bool,
    pub active: bool,
    pub paid_per_mille: u16,
    pub perceptibility_per_mille: u16,
    pub operating_emission: String,
    pub signature: crate::sensing::PassiveSignature,
    pub controllable: bool,
}
