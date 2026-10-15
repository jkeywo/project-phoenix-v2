//! Closed accepted LocalJump data. Physical work and phase ownership stay in Gameplay.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitPhaseView {
    Idle,
    Preparing,
    Committed,
    Arrived,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitWaitingView {
    Unavailable,
    Unfunded,
    OutOfRange,
    NoPlacement,
}

/// Own drive intent/progress only. The private physical placement recipe and
/// undisclosed gravity inputs never enter an ordinary publication.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitDriveView {
    pub owner: crate::identity::SystemId,
    pub phase: TransitPhaseView,
    pub preparation_fraction: f32,
    pub fuel_required: i64,
    pub can_begin: bool,
    pub can_cancel: bool,
    pub waiting: Option<TransitWaitingView>,
    pub request: Option<TransitRequestKey>,
    pub course: Option<TransitDestination>,
    pub actual_arrival: Option<[f32; 3]>,
    pub last_step_tick: Option<u64>,
}
impl TransitDriveView {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.owner.0)
            || !self.preparation_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.preparation_fraction)
            || self.fuel_required <= 0
            || self
                .actual_arrival
                .is_some_and(|point| !point.iter().all(|value| value.is_finite()))
        {
            return Err("invalid finite own Transit reading".into());
        }
        if let Some(request) = &self.request {
            request.validate()?;
        }
        if let Some(course) = &self.course {
            course.validate()?;
        }
        if self.can_cancel && self.phase != TransitPhaseView::Preparing
            || self.can_begin
                && !matches!(
                    self.phase,
                    TransitPhaseView::Idle | TransitPhaseView::Arrived
                )
            || self.actual_arrival.is_some() != (self.phase == TransitPhaseView::Arrived)
            || self.phase == TransitPhaseView::Idle
                && (self.preparation_fraction != 0.0 || self.request.is_some())
            || matches!(
                self.phase,
                TransitPhaseView::Committed | TransitPhaseView::Arrived
            ) && self.preparation_fraction != 1.0
            || self.phase != TransitPhaseView::Idle && self.course.is_none()
        {
            return Err("own Transit reading contradicts its phase".into());
        }
        Ok(())
    }
}

/// Technical retention bound, never an evicting gameplay message capacity.
pub const MAX_HANDLED_TRANSIT_OBJECTIVES: usize = 128;
pub const MAX_TRANSIT_GRAVITY_BODIES: usize = 128;

/// Explicit inherited disable; `true` never fabricates a gravity policy.
pub fn deserialize_optional_gravity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ArrivalGravityConfig>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Authored {
        Policy(ArrivalGravityConfig),
        Disabled(bool),
    }
    match Authored::deserialize(deserializer)? {
        Authored::Policy(policy) => Ok(Some(policy)),
        Authored::Disabled(false) => Ok(None),
        Authored::Disabled(true) => Err(serde::de::Error::custom(
            "arrival_gravity accepts a finite policy or false, never true",
        )),
    }
}

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitRequestKey {
    pub objective_id: String,
    pub revision: u64,
}
impl TransitRequestKey {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.objective_id) {
            return Err("transit request requires a bounded actual objective identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitFuel {
    /// Actual Logistics Local store bound to this drive's System.
    pub store: String,
    pub amount: i64,
}
impl TransitFuel {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.store) || self.amount <= 0 {
            return Err(
                "transit fuel requires an actual store and positive finite quantity".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalJumpConfig {
    pub preparation_work_secs: f32,
    pub max_distance: f32,
    pub scatter_radius: f32,
    /// Scales fixed-body arrival exclusion without changing physical collision clearance.
    #[serde(
        default = "unit_gravity_response",
        skip_serializing_if = "is_unit_gravity_response"
    )]
    pub gravity_response: f32,
    /// May bind an explicitly known authored World route; free courses stay local.
    #[serde(default, skip_serializing_if = "is_false")]
    pub world_travel: bool,
    pub fuel: TransitFuel,
    /// Existing per-System text reading; zero is immediate compatibility tuning.
    #[serde(default)]
    pub reading_ticks: u32,
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn unit_gravity_response() -> f32 {
    1.0
}
fn is_unit_gravity_response(value: &f32) -> bool {
    *value == 1.0
}
impl LocalJumpConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.preparation_work_secs.is_finite()
            || self.preparation_work_secs <= 0.0
            || !self.max_distance.is_finite()
            || self.max_distance <= 0.0
            || !self.scatter_radius.is_finite()
            || self.scatter_radius < 0.0
            || !self.gravity_response.is_finite()
            || self.gravity_response < 0.0
        {
            return Err(
                "LocalJump requires finite positive work/range and nonnegative scatter/gravity response".into(),
            );
        }
        self.fuel.validate()
    }
}

/// Actual costs frozen with the accepted profile, not a second live power owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedLocalJumpProfile {
    pub config: LocalJumpConfig,
    pub preparation_energy_per_sec: f32,
    pub commit_energy: f32,
}
impl AcceptedLocalJumpProfile {
    pub fn validate(&self) -> Result<(), String> {
        self.config.validate()?;
        if !self.preparation_energy_per_sec.is_finite()
            || self.preparation_energy_per_sec <= 0.0
            || !self.commit_energy.is_finite()
            || self.commit_energy <= 0.0
        {
            return Err(
                "accepted transit work requires actual finite positive energy costs".into(),
            );
        }
        Ok(())
    }
}

/// The accepted own course; point is read from the permitted Navigation board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitDestination {
    Local {
        point: [f32; 3],
        course_generation: u64,
    },
    World {
        id: String,
        point: [f32; 3],
        course_generation: u64,
    },
}
impl TransitDestination {
    pub fn point(&self) -> [f32; 3] {
        match self {
            Self::Local { point, .. } | Self::World { point, .. } => *point,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self, Self::World {id, ..} if !identity(id)) {
            return Err("World transit needs a bounded closed route identity".into());
        }
        if self.point().into_iter().any(|value| !value.is_finite()) {
            return Err("transit course requires finite representable geometry".into());
        }
        Ok(())
    }
}

/// Explicit physical arrival policy. Visual size and mass do not grant gravity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrivalGravityConfig {
    pub exclusion_radius: f32,
}
impl ArrivalGravityConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.exclusion_radius.is_finite() || self.exclusion_radius <= 0.0 {
            return Err("arrival gravity requires a finite positive displacement radius".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;

/// Current accepted arrival-policy context, not physical membership or lifetime.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitWorldBinding {
    #[default]
    Base,
    Layer {
        path: String,
    },
}

impl TransitWorldBinding {
    pub fn is_base(&self) -> bool {
        matches!(self, Self::Base)
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Layer { path } = self {
            if !identity(path)
                || !path.starts_with("assets/worlds/")
                || !path.ends_with(".toml")
                || path.contains(char::from(92))
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err("Transit World context needs a canonical accepted World path".into());
            }
        }
        Ok(())
    }
}

/// Latest actual successful World arrival. It owns no work, placement or payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitWorldArrival {
    pub owner: crate::identity::SystemId,
    pub route: String,
    pub tick: u64,
}

/// Authored known routes on the permitted Navigation publication. No paths or actors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitRouteView {
    pub id: String,
    pub label: String,
    pub point: [f32; 3],
}
impl TransitRouteView {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.id)
            || !identity(&self.label)
            || !self.point.iter().all(|value| value.is_finite())
        {
            return Err("invalid known authored Transit route".into());
        }
        Ok(())
    }
}
