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

/// What a Workshop model preview was asked to show.
///
/// Exactly one of `model` or `entity`: a GLB by its captured path, or an
/// entity template whose `[star]`, `[planet]` or `[mesh]` visual the shared
/// subject renderer dispatches the way the game does.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSelection {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub entity: Option<String>,
    /// Rig markers and gizmos, ON unless the caller says otherwise.
    ///
    /// Defaulting to false would boot the renderer with them hidden while the
    /// panel's own checkbox starts checked, so the first frame would contradict
    /// its own control. Previewing a model is mostly about where its markers
    /// are, so on is also the useful default.
    #[serde(default = "gizmos_on")]
    pub gizmos: bool,
}

fn gizmos_on() -> bool {
    true
}

impl PreviewSelection {
    /// A preview shows one subject. Neither is nothing to draw and both is two
    /// pictures in one frame, so both are refused rather than ordered.
    pub fn subject(&self) -> Option<&str> {
        match (self.model.as_deref(), self.entity.as_deref()) {
            (Some(model), None) => Some(model),
            (None, Some(entity)) => Some(entity),
            _ => None,
        }
    }
}
