// Helm intent components (issue #695).
//
// These components decouple *admission* (turning human `AdmittedCommands`
// or AI decisions into a desired helm state) from *physics integration*
// (actually advancing `ShipPhysics`/`ShipImpulse`/`ShipBoost` from that
// desired state). Two writers — `process_helm_inputs` (human/admission
// path) and `operate_helm_ai` (AI decision path) — each write these
// components for whichever ship they're currently authoritative for
// (mutually exclusive per tick via `ControlTickPolicy`). A single shared
// system, `integrate_helm_physics`, reads them and performs the actual
// physics/impulse/boost integration for both the player ship and any
// AI-promoted NPC.
//
// Scoped to `AiHighFidelity`: these components exist only on ships running
// full-fidelity helm systems (the player's `LocalShip`, always, and NPCs
// while promoted by `lod_ai_ships`). They are inserted/removed alongside
// the `AiHighFidelity` marker.

use bevy::prelude::*;

use crate::ship::impulse::ImpulsePhase;

/// Desired forward/reverse thrust, in the same `[-1.0, 1.0]` range as
/// `SystemControlPayload::HelmInput::thrust`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ThrustInput(pub f32);

/// Desired yaw steering input, in the same `[-1.0, 1.0]` range as
/// `SystemControlPayload::HelmInput::steering`.
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
