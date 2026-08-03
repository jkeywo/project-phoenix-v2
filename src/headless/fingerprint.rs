//! A wide, cheap fingerprint of a whole run's end state.
//!
//! Extracted from `tests/headless_runner.rs` (issues #895/#896) so issue
//! #899's registration-order guard (`tests/registration_order_determinism.rs`)
//! can reuse it instead of carrying a second copy of the same struct and
//! extraction logic. Each integration-test file is its own process/crate, so
//! the only way to share this between two of them is through the library.

use bevy::prelude::*;

use crate::balance::BalanceEvent;
use crate::entity_spawner::EntitySystemHull;
use crate::headless::report::RunTelemetry;
use crate::ship::state::ShipPhysics;
use crate::sim_rng::{SimRng, SimStream};
use crate::sim_tick::SimTick;
use crate::simulation::Ship;

/// Deliberately not just the player ship: a slice narrow enough to miss a
/// divergence is worse than no assertion at all. It folds the logical tick
/// count, the seeded RNG's position on EVERY stream, and every ship's physics
/// and hull integrity — the three places a frame-coupled or
/// registration-order-coupled system would show up (an extra/missed step, an
/// extra/missed random draw, an extra/missed integration or damage
/// application).
#[derive(Debug, PartialEq)]
pub struct RunFingerprint {
    pub tick: u64,
    pub seed: u64,
    /// One probe draw per `SimStream`, taken after the run: two runs that made
    /// a different NUMBER of draws on any stream land at different positions,
    /// so this catches divergence in the damage/uuid paths without needing the
    /// generators' private state. The width is the generator's (`Pcg32` draws
    /// 32 bits); the comparison is generator-agnostic either way.
    pub rng_positions: Vec<u32>,
    /// `(entity index, x, z, yaw, forward_speed, hull current, hull max)` for
    /// every `Ship`, sorted by entity index — which is itself part of the
    /// comparison, so a run that spawned a different number of entities, or
    /// spawned them in a different order, fails here too.
    pub ships: Vec<(bevy::ecs::entity::EntityIndex, f32, f32, f32, f32, f32, f32)>,
    /// Every collision the run applied, as `(victim uuid, damage, shield
    /// absorbed, hull damage)` in the order the balance tracer saw them.
    ///
    /// Added by issue #896, and the part of the fingerprint that is actually
    /// about physics. The `ships` slice above records where a collision *left*
    /// a hull, but two runs can land on the same hull total having hit
    /// different rocks in a different order; this records the attribution
    /// itself.
    pub collisions: Vec<(String, f32, f32, f32)>,
}

/// Compute a [`RunFingerprint`] for `app`'s current state.
///
/// Requires `RunTelemetry` (from `HeadlessArgs`-built apps) to be present, so
/// this is a headless-app helper, not a general one.
pub fn fingerprint(app: &mut App) -> RunFingerprint {
    let mut ships: Vec<_> = app
        .world_mut()
        .query_filtered::<(Entity, &ShipPhysics, Option<&EntitySystemHull>), With<Ship>>()
        .iter(app.world())
        .map(|(e, p, hull)| {
            let (current, max) =
                hull.map_or((0.0, 0.0), |h| (h.0.total_current(), h.0.total_max()));
            (e.index(), p.x, p.z, p.yaw, p.forward_speed, current, max)
        })
        .collect();
    ships.sort_by_key(|s| s.0);

    let collisions = app
        .world()
        .resource::<RunTelemetry>()
        .balance_events
        .iter()
        .filter_map(|stamped| match &stamped.event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == crate::balance::WEAPON_KIND_COLLISION => {
                Some((victim.clone(), *amount, *shield_absorbed, *hull_damage))
            }
            _ => None,
        })
        .collect();

    let rng = app.world().resource::<SimRng>();
    let rng_positions = SimStream::ALL
        .iter()
        .map(|s| rng.stream(*s).next_u32())
        .collect();

    RunFingerprint {
        tick: app.world().resource::<SimTick>().0,
        seed: rng.seed(),
        rng_positions,
        ships,
        collisions,
    }
}
