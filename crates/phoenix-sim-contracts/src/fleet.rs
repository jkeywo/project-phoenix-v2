use bevy::prelude::*;
use phoenix_runtime::HostSlot;

/// Which fleet slot a player ship belongs to (issue #1116).
///
/// Present on every ship the frozen roster put in the world, absent on every
/// NPC. It is what tells a host that the cruiser over there is *slot 2's ship*
/// — flown by a crew on another machine — rather than one more hull for its own
/// AI to operate, and it is the key a peer's `ShipKey`-routed command is
/// ultimately answered by.
///
/// Deliberately a component and not a lookup table: the roster is frozen, so
/// the ship and its slot are born together and cannot drift apart.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FleetSlotOf(pub HostSlot);
