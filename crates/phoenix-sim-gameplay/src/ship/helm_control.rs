//! Retained requests of actual Helm axes; no controller, physics or resource copy.
use super::physics::{LongitudinalDriveMode, ShipPhysicsConfig, ShipPhysicsState};
use crate::core::messages::SystemId;
pub use phoenix_model::helm_control::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Immutable authored axis bindings, derived at accepted entity installation.
#[derive(bevy::prelude::Component, Clone, Debug)]
#[require(HelmNavigationInputs)]
pub struct CompiledHelmControl(pub HelmControlConfig);

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

#[derive(bevy::prelude::Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmControlState {
    pub axes: BTreeMap<SystemId, AxisIntent>,
}

/// Current permitted input, rebuilt at the ordinary mechanical command boundary.
/// This is derived and never captured as authoritative intent or paid work.
#[derive(bevy::prelude::Component, Clone, Debug, Default)]
pub struct HelmNavigationInputs {
    pub tick: Option<u64>,
    pub axes: BTreeMap<SystemId, NavigationMotion>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationMotion {
    pub generation: u64,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
}
impl HelmNavigationInputs {
    pub fn current(&self, tick: Option<u64>) -> Option<&Self> {
        (tick.is_some() && self.tick == tick).then_some(self)
    }
}
impl NavigationMotion {
    /// Use actual own tuning and measured World-plane motion. No leader heading
    /// or live body route is available here. The stopping-speed correction is
    /// bounded by existing acceleration/speed limits rather than a new autopilot.
    fn desired_velocity(
        self,
        actual: ShipPhysicsState,
        config: &ShipPhysicsConfig,
    ) -> Option<[f32; 2]> {
        if self.generation == 0
            || !self
                .position
                .iter()
                .chain(&self.velocity)
                .all(|value| value.is_finite())
            || !actual.x.is_finite()
            || !actual.z.is_finite()
            || !actual.yaw.is_finite()
        {
            return None;
        }
        let error = [
            f64::from(self.position[0]) - f64::from(actual.x),
            f64::from(self.position[1]) - f64::from(actual.z),
        ];
        let distance = (error[0] * error[0] + error[1] * error[1]).sqrt();
        let correction = (2.0 * f64::from(config.acceleration) * distance)
            .sqrt()
            .min(f64::from(config.max_speed));
        let mut desired = std::array::from_fn::<_, 2, _>(|axis| {
            f64::from(self.velocity[axis])
                + if distance > 0.0 {
                    error[axis] / distance * correction
                } else {
                    0.0
                }
        });
        let speed = (desired[0] * desired[0] + desired[1] * desired[1]).sqrt();
        if speed > f64::from(config.max_speed) && speed > 0.0 {
            for axis in &mut desired {
                *axis *= f64::from(config.max_speed) / speed;
            }
        }
        desired
            .iter()
            .all(|value| value.is_finite() && value.abs() <= f64::from(f32::MAX))
            .then(|| desired.map(|value| value as f32))
    }
}

/// Fixed known height through the same finite stopping-speed rule as planar
/// Navigation. This returns a demand for the ordinary paid vertical actuator;
/// no pose, speed, energy or inertia is written here.
pub fn charted_vertical_input(
    height: f32,
    actual_height: f32,
    config: &ShipPhysicsConfig,
) -> Option<f32> {
    if !height.is_finite()
        || !actual_height.is_finite()
        || !config.max_vertical_speed.is_finite()
        || config.max_vertical_speed <= 0.0
        || !config.vertical_acceleration.is_finite()
        || config.vertical_acceleration <= 0.0
    {
        return None;
    }
    let error = f64::from(height) - f64::from(actual_height);
    let speed = (2.0 * f64::from(config.vertical_acceleration) * error.abs())
        .sqrt()
        .min(f64::from(config.max_vertical_speed));
    let input = error.signum() * speed / f64::from(config.max_vertical_speed);
    input.is_finite().then_some(input as f32)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedHelmControl {
    pub thrust: f32,
    pub steering: f32,
    pub steering_direct: bool,
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
    /// Compatibility stick display, derived from retained requests. Absolute
    /// servos have their own display and do not pretend to be a held stick.
    pub fn stick_display(&self, profile: &HelmControlConfig) -> [f32; 3] {
        let thrust = match self.axes.get(&profile.thrust) {
            Some(AxisIntent::Thrust(ThrustIntent::Direct { force })) => *force,
            Some(AxisIntent::Thrust(ThrustIntent::Assisted {
                demand: SpeedDemand::Fraction { value },
            })) => *value,
            _ => 0.0,
        };
        let steering = match self.axes.get(&profile.steering) {
            Some(AxisIntent::Steering(SteeringIntent::Direct { rate })) => *rate,
            Some(AxisIntent::Steering(SteeringIntent::Assisted {
                demand: SteeringDemand::Rate { value },
            })) => *value,
            _ => 0.0,
        };
        let lateral = profile
            .lateral
            .as_ref()
            .and_then(|owner| self.axes.get(owner));
        let lateral = match lateral {
            Some(AxisIntent::Lateral(LateralIntent::Direct { force })) => *force,
            Some(AxisIntent::Lateral(LateralIntent::Assisted { value })) => *value,
            _ => 0.0,
        };
        [thrust, steering, lateral]
    }

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

    pub fn follow_navigation(
        &mut self,
        owner: &SystemId,
        generation: Option<u64>,
        actual: ShipPhysicsState,
    ) -> Result<(), String> {
        if generation == Some(0) || !actual.forward_speed.is_finite() || !actual.yaw.is_finite() {
            return Err("Navigation generation and own axis readings must be valid".into());
        }
        let intent = self.axes.get_mut(owner).ok_or("unknown Helm axis owner")?;
        match intent {
            AxisIntent::Thrust(request) => {
                *request = ThrustIntent::Assisted {
                    demand: match generation {
                        Some(generation) => SpeedDemand::Navigation { generation },
                        None => SpeedDemand::Absolute {
                            speed: actual.forward_speed,
                        },
                    },
                }
            }
            AxisIntent::Steering(request) => {
                *request = SteeringIntent::Assisted {
                    demand: match generation {
                        Some(generation) => SteeringDemand::Navigation { generation },
                        None => SteeringDemand::Heading {
                            heading_rad: actual.yaw.rem_euclid(std::f32::consts::TAU),
                        },
                    },
                }
            }
            AxisIntent::Lateral(request) => {
                *request = match generation {
                    Some(generation) => LateralIntent::Navigation { generation },
                    None => LateralIntent::Assisted { value: 0.0 },
                }
            }
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
        navigation: Option<&HelmNavigationInputs>,
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
        let navigation_velocity = |owner: &SystemId, generation: u64| {
            navigation?
                .axes
                .get(owner)
                .filter(|motion| motion.generation == generation)?
                .desired_velocity(actual, config)
        };
        let forward = [
            crate::simmath::sin(actual.yaw),
            -crate::simmath::cos(actual.yaw),
        ];
        let right = [
            crate::simmath::cos(actual.yaw),
            crate::simmath::sin(actual.yaw),
        ];
        let (thrust, drive) = match thrust {
            ThrustIntent::Direct { force } => (*force, LongitudinalDriveMode::Force),
            ThrustIntent::Assisted {
                demand: SpeedDemand::Fraction { value },
            } => (*value, LongitudinalDriveMode::VelocityFraction),
            ThrustIntent::Assisted { demand } => {
                let speed = match demand {
                    SpeedDemand::Navigation { generation } => {
                        navigation_velocity(&profile.thrust, *generation).map_or(0.0, |velocity| {
                            velocity[0] * forward[0] + velocity[1] * forward[1]
                        })
                    }
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
            SteeringIntent::Assisted { demand } => {
                let heading_rad = match demand {
                    SteeringDemand::Navigation { generation } => {
                        navigation_velocity(&profile.steering, *generation)
                            .filter(|velocity| velocity[0].abs() + velocity[1].abs() > f32::EPSILON)
                            .map_or(actual.yaw, |velocity| {
                                crate::simmath::atan2(velocity[0], -velocity[1])
                                    .rem_euclid(std::f32::consts::TAU)
                            })
                    }
                    SteeringDemand::Heading { heading_rad } => *heading_rad,
                    _ => unreachable!(),
                };
                let delta = (heading_rad as f64 - actual.yaw as f64 + std::f64::consts::PI)
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
                AxisIntent::Lateral(LateralIntent::Navigation { generation }) => Some((
                    navigation_velocity(owner, *generation).map_or(0.0, |velocity| {
                        if config.max_lateral_speed > 0.0 {
                            ((velocity[0] * right[0] + velocity[1] * right[1])
                                / config.max_lateral_speed)
                                .clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    }),
                    false,
                )),
                AxisIntent::Lateral(LateralIntent::Assisted { value }) => Some((*value, false)),
                AxisIntent::Lateral(LateralIntent::Direct { force }) => Some((*force, true)),
                _ => None,
            });
        Some(ResolvedHelmControl {
            thrust,
            steering,
            steering_direct: matches!(
                self.axes.get(&profile.steering),
                Some(AxisIntent::Steering(SteeringIntent::Direct { .. }))
            ),
            drive,
            lateral,
        })
    }
}

#[cfg(test)]
#[path = "helm_control_tests.rs"]
mod tests;
