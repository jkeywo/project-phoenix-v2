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

/// Which observer a disposable Test is drawing for.
///
/// PRESENTATION ONLY. The omniscient view publishes the ordinary GM projections
/// and the ship view draws the ordinary player viewscreen; neither adds a
/// command route, a credential or a save path, and switching between them
/// changes nothing the fixed tick reads.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "view", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TestView {
    /// The authentic viewscreen of one simulated player ship. `None` is the
    /// ship the Test was launched with, which is what Start opens on.
    Ship { entity: Option<String> },
    /// The omniscient Game Master workspace over the same running simulation.
    GameMaster,
}

impl Default for TestView {
    /// A Test opens on the viewscreen of the ship it was launched with: that is
    /// the thing an author pressed Start to look at.
    fn default() -> Self {
        Self::Ship { entity: None }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TestControl {
    Pause {},
    Resume {},
    Step {},
    Rate {
        multiplier: u8,
    },
    Visibility {
        visible: bool,
    },
    /// Observe the same run through another view. Never a restart.
    View {
        view: TestView,
    },
    Stop {},
}

/// One simulated player ship a Test can be observed from.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestShip {
    pub entity: String,
    pub name: String,
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
    /// The view this run is drawing, and the ships it could draw instead.
    #[serde(default)]
    pub view: TestView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ships: Vec<TestShip>,
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
            view: TestView::default(),
            ships: Vec::new(),
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
