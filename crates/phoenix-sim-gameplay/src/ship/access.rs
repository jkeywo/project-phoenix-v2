//! Physical boundary state. Routing and containment read this single owner.
use super::composition::{CommandChannelValue, SystemCommandChannels, SystemComponentConfig};
use super::config::ShipConfig;
use crate::core::messages::SystemId;
pub use phoenix_model::interior::{BoundaryPhase, BoundaryStatus};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, bevy::prelude::Component)]
pub struct InteriorAccess {
    pub controls: BTreeMap<SystemId, AccessControl>,
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
#[derive(Clone, Debug)]
pub struct AccessControl {
    pub channels: SystemCommandChannels,
    pub sealed_channel: String,
    pub transition_secs: f32,
    pub seal_requested: bool,
    pub phase: BoundaryPhase,
    pub elapsed: f32,
}
pub fn bindings(channel: &str) -> crate::ai::state_tree::SupportedBindings {
    crate::ai::state_tree::SupportedBindings {
        facts: ["seal_requested".into(), "sealed".into()].into(),
        channels: [channel.into()].into(),
    }
}
impl InteriorAccess {
    pub fn from_ship(ship: &ShipConfig) -> Self {
        Self {
            controls: ship
                .systems
                .iter()
                .filter_map(|system| {
                    let [SystemComponentConfig::AccessBoundary {
                        sealed_channel,
                        transition_secs,
                        ..
                    }] = system.components.as_slice()
                    else {
                        return None;
                    };
                    Some((
                        system.id.clone(),
                        AccessControl {
                            channels: SystemCommandChannels::from_definitions(&system.channels),
                            sealed_channel: sealed_channel.clone(),
                            transition_secs: *transition_secs,
                            seal_requested: false,
                            phase: BoundaryPhase::Open,
                            elapsed: 0.0,
                        },
                    ))
                })
                .collect(),
        }
    }
    pub fn statuses(&self) -> Vec<BoundaryStatus> {
        self.controls
            .iter()
            .map(|(system, control)| BoundaryStatus {
                system: system.clone(),
                seal_requested: control.seal_requested,
                actuator_sealed: control.channels.boolean(&control.sealed_channel),
                phase: control.phase,
                elapsed: control.elapsed,
            })
            .collect()
    }
    pub fn set(&mut self, system: &SystemId, sealed: bool, request: bool) -> bool {
        let Some(control) = self.controls.get_mut(system) else {
            return false;
        };
        if request {
            control.seal_requested = sealed;
        }
        control
            .channels
            .set(
                &control.sealed_channel,
                CommandChannelValue::Boolean(sealed),
            )
            .expect("validated access channel");
        true
    }
    pub fn blocked_connections(
        &self,
        layout: &super::interior::InteriorConfig,
    ) -> BTreeSet<String> {
        layout
            .connections
            .iter()
            .filter(|link| {
                link.boundary
                    .as_ref()
                    .and_then(|id| self.controls.get(id))
                    .is_some_and(|control| {
                        control.seal_requested
                            || control.channels.boolean(&control.sealed_channel)
                            || control.phase != BoundaryPhase::Open
                    })
            })
            .map(|link| link.id.clone())
            .collect()
    }
    pub fn tick(&mut self, dt: f32, crossing: &BTreeSet<SystemId>) {
        self.tick_available(dt, crossing, &BTreeSet::new());
    }
    pub fn tick_available(
        &mut self,
        dt: f32,
        crossing: &BTreeSet<SystemId>,
        unavailable: &BTreeSet<SystemId>,
    ) {
        if !dt.is_finite() || dt < 0.0 {
            return;
        }
        for (id, control) in &mut self.controls {
            if unavailable.contains(id) {
                continue;
            }
            let desired =
                control.seal_requested || control.channels.boolean(&control.sealed_channel);
            let target = if desired {
                BoundaryPhase::Sealed
            } else {
                BoundaryPhase::Open
            };
            if control.phase == target {
                continue;
            }
            let transition = if desired {
                BoundaryPhase::Sealing
            } else {
                BoundaryPhase::Opening
            };
            if control.phase != transition {
                control.phase = transition;
                control.elapsed = 0.0;
            }
            if desired && crossing.contains(id) {
                continue;
            }
            control.elapsed = (control.elapsed + dt).min(control.transition_secs);
            if control.elapsed >= control.transition_secs {
                control.phase = target;
                control.elapsed = 0.0;
            }
        }
    }
    pub fn restore(&mut self, states: &[BoundaryStatus]) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        if states.len() != self.controls.len() {
            return Err("access boundary count mismatch".into());
        }
        for state in states {
            let control = self
                .controls
                .get(&state.system)
                .ok_or("unknown access boundary")?;
            if !seen.insert(&state.system)
                || !state.elapsed.is_finite()
                || state.elapsed < 0.0
                || state.elapsed >= control.transition_secs
                || matches!(state.phase, BoundaryPhase::Open | BoundaryPhase::Sealed)
                    && state.elapsed != 0.0
            {
                return Err("invalid access boundary continuation".into());
            }
        }
        for state in states {
            let control = self
                .controls
                .get_mut(&state.system)
                .expect("preflighted boundary");
            control
                .channels
                .set(
                    &control.sealed_channel,
                    CommandChannelValue::Boolean(state.actuator_sealed),
                )
                .expect("validated channel");
            control.seal_requested = state.seal_requested;
            control.phase = state.phase;
            control.elapsed = state.elapsed;
        }
        Ok(())
    }
}
