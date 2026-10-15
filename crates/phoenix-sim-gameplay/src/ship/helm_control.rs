//! Retained requests of actual Helm axes; no controller, physics or resource copy.
use super::physics::{LongitudinalDriveMode, ShipPhysicsConfig, ShipPhysicsState};
use crate::core::messages::SystemId;
pub use phoenix_model::helm_control::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "axis",
    content = "intent",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AxisIntent {
    Thrust(ThrustIntent),
    Steering(SteeringIntent),
    Lateral(LateralIntent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmControlState {
    pub axes: BTreeMap<SystemId, AxisIntent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedHelmControl {
    pub thrust: f32,
    pub steering: f32,
    pub drive: LongitudinalDriveMode,
    pub lateral: Option<(f32, bool)>,
}

pub fn validate_profile(
    profile: &HelmControlConfig,
    entity: &crate::entities::config::EntityConfig,
) -> Result<(), String> {
    use crate::ship::system_registry::{HELM_STEERING_KIND, HELM_THRUST_KIND, LATERAL_THRUST_KIND};
    let ship = entity
        .ship_config
        .as_ref()
        .ok_or("Helm controls require actual System composition")?;
    let helm = entity
        .helm_console
        .as_ref()
        .ok_or("Helm controls require actual physical drive tuning")?;
    let energy = entity
        .energy
        .as_ref()
        .ok_or("Helm controls require actual local energy consumers")?;
    let interior = entity
        .interior
        .as_ref()
        .ok_or("Helm controls require actual equipment placement")?;
    if profile.thrust == profile.steering
        || profile
            .lateral
            .as_ref()
            .is_some_and(|id| id == &profile.thrust || id == &profile.steering)
    {
        return Err("Helm controls require distinct actual axis owners".into());
    }
    for (owner, kind) in [
        (&profile.thrust, HELM_THRUST_KIND),
        (&profile.steering, HELM_STEERING_KIND),
    ]
    .into_iter()
    .chain(
        profile
            .lateral
            .as_ref()
            .map(|owner| (owner, LATERAL_THRUST_KIND)),
    ) {
        let system = ship
            .system(owner)
            .ok_or("Helm axis owner is not installed")?;
        if system.kind != kind
            || system
                .console
                .as_ref()
                .is_none_or(|id| !ship.consoles.iter().any(|console| &console.id == id))
            || !energy
                .consumers
                .iter()
                .any(|consumer| &consumer.system == owner)
            || !interior.placements.contains_key(owner)
            || !system.damage_components.iter().any(|component| {
                component.binding == phoenix_model::damage::DamageBinding::Operation
            })
            || entity.damage.is_none()
        {
            return Err(format!("Helm axis {} requires its actual {kind} System, Console, reserve, room and Operation hardware", owner.0));
        }
    }
    if ![
        helm.max_speed,
        helm.max_reverse_speed,
        helm.acceleration,
        helm.deceleration,
        helm.max_yaw_rate,
        helm.low_speed_turn_boost,
    ]
    .iter()
    .all(|value| value.is_finite() && *value >= 0.0)
        || helm.max_speed <= 0.0
        || helm.acceleration <= 0.0
        || helm.max_yaw_rate <= 0.0
    {
        return Err("Helm controls require finite supported motion limits and positive speed/acceleration/yaw".into());
    }
    Ok(())
}

impl HelmControlState {
    pub fn new(profile: &HelmControlConfig) -> Self {
        let mut axes: BTreeMap<_, _> = [
            (
                profile.thrust.clone(),
                AxisIntent::Thrust(ThrustIntent::default()),
            ),
            (
                profile.steering.clone(),
                AxisIntent::Steering(SteeringIntent::default()),
            ),
        ]
        .into();
        if let Some(owner) = &profile.lateral {
            axes.insert(
                owner.clone(),
                AxisIntent::Lateral(LateralIntent::Assisted { value: 0.0 }),
            );
        }
        Self { axes }
    }

    pub fn validate(&self, profile: &HelmControlConfig, tick: u64) -> Result<(), String> {
        if profile.thrust == profile.steering
            || self.axes.len() != 2 + usize::from(profile.lateral.is_some())
            || !matches!(self.axes.get(&profile.thrust), Some(AxisIntent::Thrust(request)) if request.valid_at(tick))
            || !matches!(self.axes.get(&profile.steering), Some(AxisIntent::Steering(request)) if request.is_valid())
            || profile.lateral.as_ref().is_some_and(|owner| owner == &profile.thrust || owner == &profile.steering || !matches!(self.axes.get(owner), Some(AxisIntent::Lateral(request)) if request.is_valid()))
        {
            return Err("Helm continuation differs from actual axis owners or has invalid intent".into());
        }
        Ok(())
    }

    pub fn set_mode(
        &mut self,
        owner: &SystemId,
        mode: HelmControlMode,
        actual: ShipPhysicsState,
        config: &ShipPhysicsConfig,
    ) -> Result<(), String> {
        let intent = self.axes.get_mut(owner).ok_or("unknown Helm axis owner")?;
        match intent {
            AxisIntent::Thrust(request) if request.mode() != mode => {
                if !actual.forward_speed.is_finite() {
                    return Err("actual Helm speed is unavailable".into());
                }
                *request = match mode {
                    HelmControlMode::Assisted => ThrustIntent::Assisted {
                        demand: SpeedDemand::Absolute {
                            speed: actual.forward_speed,
                        },
                    },
                    HelmControlMode::Direct => ThrustIntent::Direct { force: 0.0 },
                };
            }
            AxisIntent::Steering(request) if request.mode() != mode => {
                if !actual.yaw.is_finite() {
                    return Err("actual Helm heading is unavailable".into());
                }
                *request = match mode {
                    HelmControlMode::Assisted => SteeringIntent::Assisted {
                        demand: SteeringDemand::Heading {
                            heading_rad: actual.yaw.rem_euclid(std::f32::consts::TAU),
                        },
                    },
                    HelmControlMode::Direct => SteeringIntent::Direct { rate: 0.0 },
                };
            }
            AxisIntent::Lateral(request) if request.mode() != mode => {
                if !actual.lateral_speed.is_finite()
                    || !config.max_lateral_speed.is_finite()
                    || config.max_lateral_speed < 0.0
                {
                    return Err("actual lateral control is unavailable".into());
                }
                *request = match mode {
                    HelmControlMode::Assisted => LateralIntent::Assisted {
                        value: if config.max_lateral_speed > 0.0 {
                            (actual.lateral_speed / config.max_lateral_speed).clamp(-1.0, 1.0)
                        } else {
                            0.0
                        },
                    },
                    HelmControlMode::Direct => LateralIntent::Direct { force: 0.0 },
                };
            }
            _ => {}
        }
        Ok(())
    }

    pub fn set_lateral(&mut self, owner: &SystemId, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err("invalid lateral input".into());
        }
        let Some(AxisIntent::Lateral(request)) = self.axes.get_mut(owner) else {
            return Err("command does not address the lateral owner".into());
        };
        *request = match request.mode() {
            HelmControlMode::Assisted => LateralIntent::Assisted { value },
            HelmControlMode::Direct => LateralIntent::Direct { force: value },
        };
        Ok(())
    }

    /// Existing scalar command: the owner mode decides its units. Any explicit
    /// throttle request ends matching; the independent steering request survives.
    pub fn set_thrust(&mut self, owner: &SystemId, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err("invalid Helm thrust input".into());
        }
        let Some(AxisIntent::Thrust(request)) = self.axes.get_mut(owner) else {
            return Err("command does not address the thrust owner".into());
        };
        *request = match request.mode() {
            HelmControlMode::Assisted => ThrustIntent::Assisted {
                demand: SpeedDemand::Fraction { value },
            },
            HelmControlMode::Direct => ThrustIntent::Direct { force: value },
        };
        Ok(())
    }

    pub fn set_steering(&mut self, owner: &SystemId, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err("invalid Helm steering input".into());
        }
        let Some(AxisIntent::Steering(request)) = self.axes.get_mut(owner) else {
            return Err("command does not address the steering owner".into());
        };
        *request = match request.mode() {
            HelmControlMode::Assisted => SteeringIntent::Assisted {
                demand: SteeringDemand::Rate { value },
            },
            HelmControlMode::Direct => SteeringIntent::Direct { rate: value },
        };
        Ok(())
    }

    pub fn set_speed(&mut self, owner: &SystemId, speed: f32) -> Result<(), String> {
        if !speed.is_finite() {
            return Err("invalid desired Helm speed".into());
        }
        let Some(AxisIntent::Thrust(request)) = self.axes.get_mut(owner) else {
            return Err("command does not address the thrust owner".into());
        };
        *request = ThrustIntent::Assisted {
            demand: SpeedDemand::Absolute { speed },
        };
        Ok(())
    }

    pub fn set_heading(&mut self, owner: &SystemId, heading_rad: f32) -> Result<(), String> {
        if !heading_rad.is_finite() {
            return Err("invalid desired Helm heading".into());
        }
        let Some(AxisIntent::Steering(request)) = self.axes.get_mut(owner) else {
            return Err("command does not address the steering owner".into());
        };
        *request = SteeringIntent::Assisted {
            demand: SteeringDemand::Heading {
                heading_rad: heading_rad.rem_euclid(std::f32::consts::TAU),
            },
        };
        Ok(())
    }

    /// Caller supplies a current permitted same-contact measurement. There is
    /// no private actor route or observation producer inside this pure owner.
    pub fn match_speed(
        &mut self,
        owner: &SystemId,
        contact: &str,
        speed: f32,
        observed_tick: u64,
        now: u64,
    ) -> Result<(), String> {
        let next = ThrustIntent::Assisted {
            demand: SpeedDemand::Match {
                contact: contact.into(),
                speed,
                observed_tick,
            },
        };
        if !next.valid_at(now) {
            return Err("invalid dated Match Speed observation".into());
        }
        let Some(AxisIntent::Thrust(request)) = self.axes.get_mut(owner) else {
            return Err("matching does not address the thrust owner".into());
        };
        *request = next;
        Ok(())
    }

    pub fn cancel_match(&mut self, owner: &SystemId) -> Result<(), String> {
        let Some(AxisIntent::Thrust(request)) = self.axes.get_mut(owner) else {
            return Err("matching does not address the thrust owner".into());
        };
        if let ThrustIntent::Assisted {
            demand: SpeedDemand::Match { speed, .. },
        } = request
        {
            *request = ThrustIntent::Assisted {
                demand: SpeedDemand::Absolute { speed: *speed },
            };
        }
        Ok(())
    }

    pub fn refresh_match(
        &mut self,
        owner: &SystemId,
        contact: &str,
        speed: f32,
        observed_tick: u64,
        now: u64,
    ) -> Result<bool, String> {
        let same = matches!(self.axes.get(owner), Some(AxisIntent::Thrust(ThrustIntent::Assisted { demand: SpeedDemand::Match { contact: held, observed_tick: taken, .. } })) if held == contact && observed_tick > *taken);
        if same {
            self.match_speed(owner, contact, speed, observed_tick, now)?;
        }
        Ok(same)
    }

    /// Resolve requests before ordinary energy/condition scaling. Absolute speed
    /// and matching use a bounded force servo, so braking needs actual actuator
    /// work too. Legacy fractional requests preserve their exact old path.
    pub fn resolve(
        &self,
        profile: &HelmControlConfig,
        actual: ShipPhysicsState,
        dt: f32,
        config: &ShipPhysicsConfig,
    ) -> Option<ResolvedHelmControl> {
        if !dt.is_finite()
            || dt <= 0.0
            || !actual.forward_speed.is_finite()
            || !actual.yaw.is_finite()
            || self.validate(profile, u64::MAX).is_err()
            || [
                config.max_speed,
                config.max_reverse_speed,
                config.acceleration,
                config.max_yaw_rate,
                config.low_speed_turn_boost,
            ]
            .iter()
            .any(|n| !n.is_finite() || *n < 0.0)
        {
            return None;
        }
        let AxisIntent::Thrust(thrust) = self.axes.get(&profile.thrust)? else {
            return None;
        };
        let AxisIntent::Steering(steering) = self.axes.get(&profile.steering)? else {
            return None;
        };
        let (thrust, drive) = match thrust {
            ThrustIntent::Direct { force } => (*force, LongitudinalDriveMode::Force),
            ThrustIntent::Assisted {
                demand: SpeedDemand::Fraction { value },
            } => (*value, LongitudinalDriveMode::VelocityFraction),
            ThrustIntent::Assisted { demand } => {
                let speed = match demand {
                    SpeedDemand::Absolute { speed } | SpeedDemand::Match { speed, .. } => *speed,
                    _ => unreachable!(),
                };
                let speed = speed.clamp(-config.max_reverse_speed, config.max_speed);
                let step = config.acceleration as f64 * dt as f64;
                let force = if step.is_finite() && step > 0.0 {
                    ((speed as f64 - actual.forward_speed as f64) / step).clamp(-1.0, 1.0) as f32
                } else {
                    0.0
                };
                (force, LongitudinalDriveMode::Force)
            }
        };
        let steering = match steering {
            SteeringIntent::Direct { rate } => *rate,
            SteeringIntent::Assisted {
                demand: SteeringDemand::Rate { value },
            } => *value,
            SteeringIntent::Assisted {
                demand: SteeringDemand::Heading { heading_rad },
            } => {
                let delta = (*heading_rad as f64 - actual.yaw as f64 + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                // Conservative maximum avoids overshooting while longitudinal
                // motion changes this step's actual low-speed turn multiplier.
                let step = config.max_yaw_rate as f64
                    * (1.0 + config.low_speed_turn_boost.max(0.0) as f64)
                    * dt as f64;
                if step.is_finite() && step > 0.0 {
                    (delta / step).clamp(-1.0, 1.0) as f32
                } else {
                    0.0
                }
            }
        };
        let lateral = profile
            .lateral
            .as_ref()
            .and_then(|owner| match self.axes.get(owner)? {
                AxisIntent::Lateral(LateralIntent::Assisted { value }) => Some((*value, false)),
                AxisIntent::Lateral(LateralIntent::Direct { force }) => Some((*force, true)),
                _ => None,
            });
        Some(ResolvedHelmControl {
            thrust,
            steering,
            drive,
            lateral,
        })
    }
}

#[cfg(test)]
#[path = "helm_control_tests.rs"]
mod tests;
