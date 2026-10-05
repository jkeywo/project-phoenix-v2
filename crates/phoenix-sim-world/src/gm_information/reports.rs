use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportPolicy {
    pub delay_ticks: u32,
    pub position_step_mm: u32,
    pub hide_identity: bool,
}

impl ReportPolicy {
    pub fn changes_report(&self) -> bool {
        self.delay_ticks != 0 || self.position_step_mm != 0 || self.hide_identity
    }
}
