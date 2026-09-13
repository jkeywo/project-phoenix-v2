//! Private disposable Test vocabulary shared by the native and browser owners.
//! No world mutation, filesystem path selection or Live transport entry lives here.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestSelection {
    pub world: String,
    pub ship: String,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TestControl {
    Pause {},
    Resume {},
    Step {},
    Rate { multiplier: u8 },
    Visibility { visible: bool },
    Stop {},
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub selection: TestSelection,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestStatus {
    pub running: bool,
    pub starting: bool,
    pub paused: bool,
    pub tick: u64,
    pub multiplier: u8,
    pub acknowledged: u64,
    pub error: Option<String>,
    pub revision: String,
    pub selection: TestSelection,
}
impl TestStatus {
    pub fn starting(launch: &Launch) -> Self {
        Self {
            running: true,
            starting: true,
            paused: false,
            tick: 0,
            multiplier: 1,
            acknowledged: 0,
            error: None,
            revision: launch.revision.clone(),
            selection: launch.selection.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRecord {
    pub id: u64,
    pub control: TestControl,
}
