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

/// One Test-local condition over the scenario Flag store. The boolean and
/// integer views are the runtime's existing `flag(name)` / `counter(name)`
/// vocabulary; no Rhai expression or debugger state crosses this boundary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TestBreakpointCondition {
    Flag {
        name: String,
        value: bool,
    },
    Counter {
        name: String,
        comparison: TestBreakpointComparison,
        value: i64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TestBreakpointComparison {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

impl TestBreakpointComparison {
    pub fn matches(self, current: i64, expected: i64) -> bool {
        match self {
            Self::Eq => current == expected,
            Self::Ne => current != expected,
            Self::Ge => current >= expected,
            Self::Gt => current > expected,
            Self::Le => current <= expected,
            Self::Lt => current < expected,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestBreakpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    pub condition: TestBreakpointCondition,
}

impl TestBreakpoint {
    pub fn name(&self) -> &str {
        match &self.condition {
            TestBreakpointCondition::Flag { name, .. }
            | TestBreakpointCondition::Counter { name, .. } => name,
        }
    }
    pub fn matches(&self, current: i64) -> bool {
        match self.condition {
            TestBreakpointCondition::Flag { value, .. } => (current != 0) == value,
            TestBreakpointCondition::Counter {
                comparison, value, ..
            } => comparison.matches(current, value),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = self.name().as_bytes();
        if name.is_empty()
            || name.len() > 128
            || !(name[0].is_ascii_alphabetic() || name[0] == b'_')
            || !name
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-'))
        {
            return Err("Invalid Test breakpoint Flag name");
        }
        if self.layer.as_deref().is_some_and(|path| {
            !path.starts_with("assets/worlds/")
                || !path.ends_with(".toml")
                || path.contains('\\')
                || path.contains('\0')
                || path
                    .split('/')
                    .any(|part| part.is_empty() || matches!(part, "." | ".."))
        }) {
            return Err("Invalid Test breakpoint layer");
        }
        Ok(())
    }
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

/// One ordered observation from the existing scenario-script runtime.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestTraceSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TestTraceKind {
    HostCall {
        function: String,
    },
    FlagMutation {
        name: String,
        before: i64,
        after: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
    },
    CallbackScheduled {
        function: String,
        fire_tick: u64,
    },
    CallbackFired {
        function: String,
        scheduled_tick: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TestTraceRecord {
    pub tick: u64,
    /// Zero-based runtime order within `tick`.
    pub order: u32,
    pub source: TestTraceSource,
    #[serde(flatten)]
    pub kind: TestTraceKind,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestBreakpointHit {
    pub breakpoint: TestBreakpoint,
    pub current: i64,
    /// Number of completed fixed ticks at the exact held boundary.
    pub tick: u64,
    pub source: TestTraceSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjacent_trace: Vec<TestTraceRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub selection: TestSelection,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakpoint: Option<TestBreakpoint>,
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
    /// Bounded, oldest-to-newest observations from this disposable run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<TestTraceRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakpoint: Option<TestBreakpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakpoint_hit: Option<TestBreakpointHit>,
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
            trace: Vec::new(),
            breakpoint: launch.breakpoint.clone(),
            breakpoint_hit: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattened_trace_event_round_trips_through_native_status_json() {
        let record = TestTraceRecord {
            tick: 7,
            order: 2,
            source: TestTraceSource {
                path: Some("assets/worlds/test.rhai".into()),
                line: Some(4),
            },
            kind: TestTraceKind::CallbackScheduled {
                function: "later".into(),
                fire_tick: 12,
            },
        };
        let encoded = serde_json::to_string(&record).expect("encode trace record");
        assert_eq!(
            serde_json::from_str::<TestTraceRecord>(&encoded).expect("decode trace record"),
            record
        );
    }

    #[test]
    fn typed_breakpoints_refuse_paths_and_expression_shaped_fields() {
        let breakpoint = TestBreakpoint {
            layer: Some("assets/worlds/arrival.toml".into()),
            condition: TestBreakpointCondition::Counter {
                name: "arrivals".into(),
                comparison: TestBreakpointComparison::Ge,
                value: 2,
            },
        };
        assert!(breakpoint.validate().is_ok());
        assert!(breakpoint.matches(2));
        let encoded = serde_json::to_string(&breakpoint).unwrap();
        assert_eq!(
            serde_json::from_str::<TestBreakpoint>(&encoded).unwrap(),
            breakpoint
        );
        assert!(serde_json::from_str::<TestBreakpoint>(
            r#"{"condition":{"kind":"flag","name":"ready","value":true,"expression":"debug()"}}"#,
        )
        .is_err());
        assert!(TestBreakpoint {
            layer: Some("assets/worlds/../private.toml".into()),
            condition: TestBreakpointCondition::Flag {
                name: "ready".into(),
                value: true
            },
        }
        .validate()
        .is_err());
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
