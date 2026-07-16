// Helm intent components (issue #695).
//
// These components decouple *admission* (turning human `AdmittedCommands`
// or AI decisions into a desired helm state) from *physics integration*
// (actually advancing `ShipPhysics`/`ShipImpulse`/`ShipBoost` from that
// desired state). The writers are `process_helm_inputs` (human/admission
// path) and the per-axis helm AI (`ai_helm_thrust`, `ai_helm_steering`,
// `ai_helm_lateral_thrust`, `ai_helm_impulse` — one component each, gated on
// its own system's `ControlTickPolicy`; the `operate_helm_ai` monolith was the
// AI-side writer until #704 split and deleted it). They write these components
// for whichever ship they're currently authoritative for, mutually exclusive
// per tick. A single shared system, `integrate_ship_physics`, reads them and
// performs the actual physics/impulse/boost integration for both the player
// ship and any AI-promoted NPC.
//
// Scoped to `AiHighFidelity`: these components exist only on ships running
// full-fidelity helm systems (the player's `LocalShip`, always, and NPCs
// while promoted by `lod_ai_ships`). They are inserted/removed alongside
// the `AiHighFidelity` marker.

use bevy::prelude::*;

use crate::ship::impulse::ImpulsePhase;

/// Desired forward/reverse thrust, in the same `[-1.0, 1.0]` range as
/// `SystemControlPayload::SetThrust::value`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ThrustInput(pub f32);

/// Desired yaw steering input, in the same `[-1.0, 1.0]` range as
/// `SystemControlPayload::SetSteering::value`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct SteeringInput(pub f32);

/// Desired lateral (strafe) thrust input.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct LateralThrustInput(pub f32);

/// Desired impulse-drive phase transition. Only `Idle` (cancel) and
/// `Charging` (start) are ever written as commands — `Active` is reached by
/// natural progression in `tick_impulse`, never commanded directly.
/// Applying the same desired phase repeatedly is idempotent: `start_charge`
/// only transitions from `Idle`, and `cancel_charge` resets progress that is
/// already zero once `Idle`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ImpulseCommand(pub ImpulsePhase);

/// Desired boost-drive engagement state.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct BoostCommand(pub bool);

// ── Debug-only helm-path single-writer tracker (issue #699) ─────────────────
//
// `integrate_ship_physics` is the sole writer of `ShipPhysics` *along the helm
// path*. It is deliberately NOT the only writer of `ShipPhysics` overall —
// four out-of-band writers are sanctioned exceptions, documented on
// `ShipPhysics` in `src/ship/state.rs`:
//
//   * `simulate_low_lod_ships`        (src/ai/server.rs)      — dead reckoning
//   * `handle_collisions` / `separate_ship_from_collision` (src/server_app.rs)
//   * `tick_blaster_system` recoil    (src/console/weapons/blaster.rs)
//   * `handle_slow_zone_speed_clamp`  (src/regions/server.rs) — an observer
//
// Those four do NOT opt into this tracker and must never trip it: they are
// corrections/overrides applied on top of the helm integration, not competing
// helm integrators.
//
// SCOPE — what this tracker actually catches:
//   * `integrate_ship_physics` being scheduled/registered more than once.
//   * `integrate_ship_physics` visiting the same ship twice in one frame
//     (e.g. after a refactor splits or duplicates its query).
//   * A future *helm-path* writer that opts in by calling `record_write`.
//
// What it CANNOT catch: a future writer that mutates `ShipPhysics` directly
// without calling `record_write`. Nothing in Rust's type system forces
// opt-in, so this is a regression tripwire, not a proof of single-writer.
// It is compiled out entirely in release builds.

/// Monotonic per-frame stamp shared by every helm-path `ShipPhysics` writer.
///
/// Bumped once per frame in `First` by `tick_helm_physics_frame`, so every
/// system in a given frame observes the same value. Starts at 0 and is first
/// observed as 1, which keeps a freshly-inserted (`Default`) guard from
/// looking like it was already written this frame.
#[cfg(debug_assertions)]
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct HelmPhysicsFrame(pub u64);

#[cfg(debug_assertions)]
pub fn tick_helm_physics_frame(mut frame: ResMut<HelmPhysicsFrame>) {
    frame.0 = frame.0.wrapping_add(1);
}

/// Per-ship record of who last advanced this ship's `ShipPhysics` along the
/// helm path, and on which frame. Self-healing: `integrate_ship_physics`
/// inserts it on any ship that lacks one, so there is no insertion-site
/// churn and demoted (low-LOD) ships simply stop being stamped.
#[cfg(debug_assertions)]
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct HelmPhysicsWriteGuard {
    last_frame: u64,
    last_writer: Option<&'static str>,
}

#[cfg(debug_assertions)]
impl HelmPhysicsWriteGuard {
    /// Stamp this ship as helm-integrated by `writer` on `frame`, panicking if
    /// some helm-path writer already claimed the same ship on the same frame.
    pub fn record_write(&mut self, entity: Entity, writer: &'static str, frame: u64) {
        assert!(
            self.last_frame != frame,
            "ShipPhysics helm-path single-writer violation on {entity}: frame {frame} was \
             already integrated by `{}`, and `{writer}` is now writing it again. \
             `integrate_ship_physics` must be the only helm-path writer of \
             ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll. If you are adding an \
             out-of-band correction (collision, recoil, slow-zone clamp, low-LOD dead \
             reckoning), do not call record_write — document it on `ShipPhysics` instead.",
            self.last_writer.unwrap_or("<unknown>"),
        );
        self.last_frame = frame;
        self.last_writer = Some(writer);
    }

    /// The frame this ship was last helm-integrated on, and by whom.
    pub fn last_write(&self) -> Option<(u64, &'static str)> {
        self.last_writer.map(|w| (self.last_frame, w))
    }
}
