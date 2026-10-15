//! Closed retained Helm intent. Actual motion, work and knowledge stay at their owners.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelmControlMode {
    #[default]
    Assisted,
    Direct,
}

/// Opt-in bindings of the first supported planar assisted/direct controls.
/// Gameplay validates these against actual distinct thrust and steering Systems.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmControlConfig {
    pub thrust: SystemId,
    pub steering: SystemId,
    #[serde(default)]
    pub lateral: Option<SystemId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmAxisView<I> {
    pub system: SystemId,
    pub intent: I,
    pub available: bool,
}

/// Ownship retained intent and derived availability, not another controller.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmMovementView {
    pub thrust: HelmAxisView<ThrustIntent>,
    pub steering: HelmAxisView<SteeringIntent>,
    pub lateral: Option<HelmAxisView<LateralIntent>>,
    /// Whether the same original contact currently supplies a permitted dated
    /// velocity. The retained intent's observed_tick still reports its age.
    pub match_observation_available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpeedDemand {
    /// Equivalent interpretation of the existing normalized throttle command.
    Fraction { value: f32 },
    /// Signed speed in the existing local forward frame, not thrust or energy.
    Absolute { speed: f32 },
    /// Last actual permitted measurement. Receipt/loss does not renew this tick.
    Match {
        contact: String,
        speed: f32,
        observed_tick: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ThrustIntent {
    Assisted { demand: SpeedDemand },
    Direct { force: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SteeringDemand {
    /// Equivalent interpretation of the existing normalized yaw-rate command.
    Rate { value: f32 },
    /// Canonical absolute heading in radians, in [0, TAU).
    Heading { heading_rad: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SteeringIntent {
    Assisted { demand: SteeringDemand },
    Direct { rate: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum LateralIntent {
    Assisted { value: f32 },
    Direct { force: f32 },
}

impl LateralIntent {
    pub fn mode(&self) -> HelmControlMode {
        match self {
            Self::Assisted { .. } => HelmControlMode::Assisted,
            Self::Direct { .. } => HelmControlMode::Direct,
        }
    }
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Assisted { value } => axis(*value),
            Self::Direct { force } => axis(*force),
        }
    }
}

fn axis(value: f32) -> bool {
    value.is_finite() && (-1.0..=1.0).contains(&value)
}

impl ThrustIntent {
    pub fn mode(&self) -> HelmControlMode {
        match self {
            Self::Assisted { .. } => HelmControlMode::Assisted,
            Self::Direct { .. } => HelmControlMode::Direct,
        }
    }
    /// Shape/time only: permission, exact owner and physical limits are joined
    /// against the actual candidate hull, never inferred from this request.
    pub fn valid_at(&self, tick: u64) -> bool {
        match self {
            Self::Direct { force } => axis(*force),
            Self::Assisted { demand } => match demand {
                SpeedDemand::Fraction { value } => axis(*value),
                SpeedDemand::Absolute { speed } => speed.is_finite(),
                SpeedDemand::Match {
                    contact,
                    speed,
                    observed_tick,
                } => {
                    !contact.trim().is_empty()
                        && contact.len() <= 1024
                        && !contact.chars().any(char::is_control)
                        && speed.is_finite()
                        && *speed >= 0.0
                        && *observed_tick <= tick
                }
            },
        }
    }
}

impl Default for ThrustIntent {
    fn default() -> Self {
        Self::Assisted {
            demand: SpeedDemand::Fraction { value: 0.0 },
        }
    }
}

impl SteeringIntent {
    pub fn mode(&self) -> HelmControlMode {
        match self {
            Self::Assisted { .. } => HelmControlMode::Assisted,
            Self::Direct { .. } => HelmControlMode::Direct,
        }
    }
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Direct { rate } => axis(*rate),
            Self::Assisted { demand } => match demand {
                SteeringDemand::Rate { value } => axis(*value),
                SteeringDemand::Heading { heading_rad } => {
                    heading_rad.is_finite() && (0.0..std::f32::consts::TAU).contains(heading_rad)
                }
            },
        }
    }
}

impl Default for SteeringIntent {
    fn default() -> Self {
        Self::Assisted {
            demand: SteeringDemand::Rate { value: 0.0 },
        }
    }
}

#[cfg(test)]
#[path = "helm_control_tests.rs"]
mod tests;
