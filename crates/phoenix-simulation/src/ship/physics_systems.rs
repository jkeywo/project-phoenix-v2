pub use phoenix_sim_gameplay::ship::physics_systems::*;
#[cfg(test)]
#[path = "physics_systems_tests.rs"]
mod tests;

#[cfg(test)]
use crate::core::messages::ModifierSlot;
#[cfg(test)]
use crate::modifiers::ShipModifiers;
#[cfg(test)]
use crate::server_app::{ShipBoost, ShipImpulse};
#[cfg(test)]
use crate::ship::components::{
    BankConfigResource, ImpulseConfigResource, ShipPhysicsConfigResource, ShipSystemControlSources,
};
#[cfg(test)]
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
#[cfg(test)]
use crate::ship::physics::ShipPhysicsConfig;
#[cfg(test)]
use crate::ship::state::ShipPhysics;
#[cfg(test)]
use bevy::prelude::*;
