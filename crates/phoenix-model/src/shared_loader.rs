//! Own-tube handling report; shared hardware conveys no other member facts.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderWait {
    SharedHandling,
    TubeWork,
    MagazineUnavailable,
    Stock,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderView {
    pub group: String,
    pub work_rate: f32,
    pub priority: u16,
    pub waiting: Option<LoaderWait>,
}
