use crate::core::messages::{CameraView, ViewMode};
use crate::ship::viewscreen::{source_system_for_view_mode, ViewscreenArbiter, ViewscreenRequest};
use bevy::prelude::Component;

/// Per-entity physics state component for every ship entity (player and NPC).
///
/// Replaces the `x`, `z`, `yaw`, `forward_speed`, and `roll` fields that were
/// previously on the singleton `ShipState` resource. Both the player ship and
/// NPC ships carry this component; the physics tick reads/writes it uniformly.
///
/// Pure per-entity Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<ShipPhysics>`.
///
/// # Writer policy (issue #699)
///
/// `integrate_ship_physics` (`src/ship_plugin.rs`) is the **sole writer of the
/// helm path**: it is the only system that turns helm intent
/// (`ThrustInput`/`SteeringInput`/`LateralThrustInput`/`VerticalThrustInput`/
/// `ImpulseCommand`/`BoostCommand`) into motion, and the only production caller
/// of `compute_physics`. Do not add a second helm integrator — extend that one.
/// The helm-path fields it owns are `x`/`y`/`z`/`yaw`/`forward_speed`/
/// `lateral_speed`/`vertical_speed`/`roll` (the vertical pair added in #744).
///
/// It is deliberately **not** the only writer of these fields overall. Six
/// out-of-band writers are **sanctioned exceptions**. They are corrections and
/// overrides layered on top of the helm integration rather than competing
/// integrators, so they are intentionally left as direct writes and do not
/// opt into the debug helm write-tracker (`HelmPhysicsWriteGuard`):
///
/// | Writer | Location | Writes | Why it is exempt |
/// |---|---|---|---|
/// | `simulate_low_lod_ships` | `src/ai/server.rs` | `x`, `z`, `yaw` | Dead reckoning for ships demoted out of `AiHighFidelity`. Those ships have no helm intent components at all, so the helm path cannot serve them. |
/// | `handle_collisions` / `separate_ship_from_collision` | `src/server_app.rs` | `forward_speed`, `x`, `z` | Collision response: a hard stop plus a positional de-overlap. Routing it through helm intent would let the ship integrate *into* geometry for a frame before responding. |
/// | `tick_blaster_system` recoil (issue #638) | `src/console/weapons/blaster.rs` | `forward_speed` | An impulse applied by weapons fire, not a helm decision. It adds to whatever the helm integrator produced. |
/// | `handle_slow_zone_speed_clamp` | `src/regions/server.rs` | `forward_speed` | An **observer** (`trigger: On<RegionEntered>`), not a scheduled system — it can fire at any point, outside any `SimSet` ordering window, so it cannot be sequenced relative to the helm integrator. |
/// | `move_coupled_target` (issue #1156) | `src/tractor/server.rs` | `x`, `y`, `z`, `forward_speed`, `lateral_speed`, `vertical_speed` | A **tractor coupling**: while the beam holds, the coupled derelict's position is the operator's rig, not its own helm's. The same shape as the collision de-overlap — a correction applied after integration, in `SimSet::Modifiers` — and the speeds are zeroed for the collision responder's reason turned around: a craft released from the rig must not shoot off at a velocity its helm accumulated against a position it was never allowed to reach. |
/// | `tick_dock` (issue #1159) | `src/dock/server.rs` | `x`, `y`, `z`, `yaw`, `forward_speed`, `lateral_speed`, `vertical_speed` | A **dock manoeuvre**: writes only the OWN docking ship onto the pose it is placed at, the same after-integration correction shape as the tractor. Ordered `after` the tractor rig in `SimSet::Modifiers` so the own-ship placement is the deterministic last writer that tick; the speeds are zeroed on every placement for the released-from-the-rig reason. |
///
/// When adding a new writer, prefer helm intent components. Only write these
/// fields directly if the change is genuinely one of the above shapes — a
/// correction that must land outside the helm integration — and add a row here.
///
/// This table is enforced, not just documented: the two tests at the end of
/// `tests/headless_runner.rs` scan the real headless app for every scheduled
/// system with mutable `ShipPhysics` access, pin the count against the rows
/// above, and prove (through Bevy's own access prover) that the low-LOD
/// substitute and the helm integrator can never be handed the same entity.
/// Observers are in no schedule and so escape that scan — `handle_slow_zone_
/// speed_clamp` is covered by this table alone.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipPhysics {
    /// X position in world space.
    pub x: f32,
    /// Y (altitude / vertical) position in world space. Stays at the cruise
    /// plane (0) for `Planar` hulls; driven by the vertical helm axis for
    /// bounded / full-3D craft (issue #744).
    pub y: f32,
    /// Z position in world space.
    pub z: f32,
    /// Yaw angle in radians (0 = facing negative Z).
    pub yaw: f32,
    /// Current forward speed (positive = forward, negative = reverse).
    pub forward_speed: f32,
    /// Current visual banking roll angle in radians (leans into turns).
    pub roll: f32,
    /// Current lateral (sideways) speed. Positive = starboard (+X), negative = port (-X).
    pub lateral_speed: f32,
    /// Current vertical (up/down) speed. Positive = up (+Y), negative = down (issue #744).
    pub vertical_speed: f32,
}

/// Per-entity red-alert state for every ship entity (player and NPC).
///
/// Replaces the `red_alert` field that was previously on the singleton
/// `ShipState` resource. Added in issue #591.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShipRedAlert(pub bool);

/// Per-entity **weapons hold** for every ship entity (player and NPC), issue
/// #1041.
///
/// The tactical restraint lever. Red Alert is binary and stays binary: this is a
/// second, independent state layered *under* it, so a captain can be at
/// stations, shields up, guns cold. It is set with the same explicit
/// [`SetWeaponsHold`](crate::core::messages::SystemControlPayload::SetWeaponsHold)
/// command on the same `red-alert` admitted-command target the alert itself uses
/// — one console control source governs the ship's whole firing posture, and an
/// NPC whose Red Alert system is AI-run can be ordered to hold through exactly
/// the same path, with no second capability to provision.
///
/// # How it suppresses fire
///
/// It adds NO doctrine vocabulary. Every armed hull in the fleet gates its guns
/// on one authored predicate — `fact(red_alert) >= param(min_alert_to_fire)` —
/// and this state is composed into the *value* of that fact at the fire hosts,
/// through [`WeaponsAlertPosture`](crate::console::weapons::WeaponsAlertPosture).
/// Held, the fact reads below every authorable floor; released, it reads exactly
/// the `1.0`/`0.0` it always did. See that type for the full argument.
///
/// # Not graduated rules of engagement
///
/// Deliberately one boolean. There are no authority tiers and no per-target
/// rules of engagement here; that design is parked (issue #834) and this is the
/// thin lever that was shipped instead.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShipWeaponsHold(pub bool);

/// The world flag mirroring the hull the crew fly: `weapons_hold.own_ship`.
///
/// Keyed by ROLE rather than by an authored reference name, because the player's
/// hull is not required to declare one — `falling_skyway.toml` gives its
/// `player-ship` an `id` and no `name`, so there is no name to key on and a
/// scenario still has to be able to ask the question.
pub const OWN_SHIP_WEAPONS_HOLD_FLAG: &str = "weapons_hold.own_ship";

/// The world flag mirroring a named ship's weapons hold: `weapons_hold.<name>`.
///
/// The naming imitates issue #1035's `workforce.<id>.on_strike` deliberately:
/// authoritative state lives in the component, the flag is a MIRROR of it, and
/// scenario script reads the mirror. `name` is the entity's authored reference
/// name — the same string `on_destroyed(...)` and `hail(...)` take.
pub fn weapons_hold_flag(name: &str) -> String {
    format!("weapons_hold.{name}")
}

/// Per-entity viewscreen mode state for every ship entity (player and NPC).
///
/// Replaces the `view_mode` field that was previously on the singleton
/// `ShipState` resource. Added in issue #591.
///
/// The embedded `ViewscreenArbiter` holds the monotonic recency `sequence`
/// backing the latest-valid-command-wins policy (issue #769). Because this is a
/// per-*entity* component owned by the ship entity — spawned once at game start,
/// never re-inserted on player reconnect (reconnect is a session/station event,
/// not a ship respawn) — the `sequence` persists across reconnects. A
/// reconnecting console therefore cannot reset the counter and clobber a newer
/// view issued by another console.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShipViewMode {
    pub view_mode: ViewMode,
    captain_view: CameraView,
    pub viewscreen: ViewscreenArbiter,
}

impl Default for ShipViewMode {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Camera(CameraView::default()),
            captain_view: CameraView::default(),
            viewscreen: ViewscreenArbiter::new(),
        }
    }
}

impl ShipViewMode {
    pub fn request_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        self.request_view_mode_from(requester, mode);
    }

    pub fn request_view_mode_from(
        &mut self,
        requester: crate::core::messages::SystemId,
        mode: ViewMode,
    ) {
        let resolution = self
            .viewscreen
            .apply_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }

    pub fn show_view_mode(&mut self, mode: ViewMode) {
        let requester = source_system_for_view_mode(&mode);
        let resolution = self
            .viewscreen
            .apply_channel_2(ViewscreenRequest { requester, mode });
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }

    pub fn restore_captain_view(&mut self) {
        let resolution = self.viewscreen.restore_captain_view();
        self.view_mode = resolution.mode;
        self.captain_view = self.viewscreen.captain_view();
    }
}

/// Per-entity phaser emitter frequency (0.0–1.0).
///
/// Replaces the `phaser_frequency` field that was previously on the singleton
/// `ShipState` resource.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShipPhaserFrequency(pub f32);

impl ShipPhaserFrequency {
    /// Read the authoritative ship-wide emitter frequency for continuation.
    pub(crate) fn continuation(&self) -> f32 {
        self.0
    }

    /// Replace the bootstrap frequency with the restored continuation.
    pub(crate) fn replace_continuation(&mut self, frequency: f32) {
        self.0 = frequency;
    }
}

impl Default for ShipPhaserFrequency {
    fn default() -> Self {
        Self(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_view_mode_defaults_to_camera() {
        let vm = ShipViewMode::default();
        assert_eq!(vm.view_mode, ViewMode::Camera(CameraView::default()));
    }

    #[test]
    fn ship_view_mode_request_toggles_correctly() {
        let mut vm = ShipViewMode::default();
        vm.request_view_mode(ViewMode::Camera(CameraView::new("camera_aft")));
        vm.request_view_mode(ViewMode::Radar);
        assert_eq!(vm.view_mode, ViewMode::Radar);
        vm.request_view_mode(ViewMode::Radar);
        assert_eq!(
            vm.view_mode,
            ViewMode::Camera(CameraView::new("camera_aft"))
        );
    }
}
