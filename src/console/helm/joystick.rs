//! Pure helm-joystick logic (Bevy-free, unit-testable).
//!
//! Mirrors the JS contract that previously lived in `client.html`:
//!   - Knob is constrained to a circle of radius `max_radius`.
//!   - `up` (negative dy) → positive thrust (forward).
//!   - `down` (positive dy) → negative thrust (reverse).
//!   - Horizontal dx → steering, right is positive.
//!   - Both outputs clamped to `[-1.0, 1.0]`.
//!   - While active, the *last* input is resent on each tick (~10Hz).
//!   - Releasing snaps to centre and emits a final `(0, 0)` `HelmInput`.

use crate::messages::ClientMessage;
use crate::impulse::{ImpulseState, ImpulsePhase};
use bevy::prelude::Resource;

/// Local-only state for the helm joystick. Lives as a `Resource` on the
/// client app; never serialized.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct HelmJoystickState {
    pub active: bool,
    /// Knob offset from pad centre in pixels, post-clamp. (0,0) when idle.
    pub knob_dx: f32,
    pub knob_dy: f32,
    pub last_thrust: f32,
    pub last_steering: f32,
}

impl Default for HelmJoystickState {
    fn default() -> Self {
        Self {
            active: false,
            knob_dx: 0.0,
            knob_dy: 0.0,
            last_thrust: 0.0,
            last_steering: 0.0,
        }
    }
}

/// Clamp a 2D offset to a disc of radius `max_radius`.
/// `max_radius` of 0 (or negative) collapses to the origin.
pub fn clamp_to_circle(dx: f32, dy: f32, max_radius: f32) -> (f32, f32) {
    if max_radius <= 0.0 {
        return (0.0, 0.0);
    }
    let dist_sq = dx * dx + dy * dy;
    let max_sq = max_radius * max_radius;
    if dist_sq <= max_sq {
        return (dx, dy);
    }
    let dist = dist_sq.sqrt();
    (dx / dist * max_radius, dy / dist * max_radius)
}

/// Convert a clamped knob offset into `(thrust, steering)` in `[-1, 1]`.
/// `up` (negative dy) → positive thrust.
pub fn compute_thrust_steering(dx: f32, dy: f32, max_radius: f32) -> (f32, f32) {
    if max_radius <= 0.0 {
        return (0.0, 0.0);
    }
    let thrust   = (-dy / max_radius).clamp(-1.0, 1.0);
    let steering = (dx  / max_radius).clamp(-1.0, 1.0);
    (thrust, steering)
}

/// Begin a drag. Returns the `HelmInput` to send.
pub fn press(state: &mut HelmJoystickState, dx: f32, dy: f32, max_radius: f32) -> ClientMessage {
    state.active = true;
    apply_drag(state, dx, dy, max_radius)
}

/// Continue a drag. If the joystick isn't active the call is a no-op and
/// returns `None`; otherwise returns the latest `HelmInput`.
pub fn drag(
    state: &mut HelmJoystickState,
    dx: f32,
    dy: f32,
    max_radius: f32,
) -> Option<ClientMessage> {
    if !state.active {
        return None;
    }
    Some(apply_drag(state, dx, dy, max_radius))
}

fn apply_drag(
    state: &mut HelmJoystickState,
    dx: f32,
    dy: f32,
    max_radius: f32,
) -> ClientMessage {
    let (cdx, cdy) = clamp_to_circle(dx, dy, max_radius);
    let (thrust, steering) = compute_thrust_steering(cdx, cdy, max_radius);
    state.knob_dx = cdx;
    state.knob_dy = cdy;
    state.last_thrust = thrust;
    state.last_steering = steering;
    ClientMessage::HelmInput { thrust, steering }
}

/// Release the joystick: snap to centre, zero outputs. Returns the final
/// zero `HelmInput` to send (always sent so the server stops the ship).
pub fn release(state: &mut HelmJoystickState) -> ClientMessage {
    state.active = false;
    state.knob_dx = 0.0;
    state.knob_dy = 0.0;
    state.last_thrust = 0.0;
    state.last_steering = 0.0;
    ClientMessage::HelmInput { thrust: 0.0, steering: 0.0 }
}

/// 10Hz tick. While active, resends the last input. While idle, sends
/// zero input so the server always receives periodic updates (handles the
/// case where the release message is lost over WebRTC, preventing steering
/// drift).
pub fn tick(state: &HelmJoystickState) -> Option<ClientMessage> {
    Some(ClientMessage::HelmInput {
        thrust: state.last_thrust,
        steering: state.last_steering,
    })
}

/// View of the impulse button the UI can render without exposing `ImpulseState`
/// internals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImpulseButtonView {
    /// Drive is idle — button is ready to press.
    Ready,
    /// Drive is charging — `progress` is 0.0..=1.0.
    Charging { progress: f32 },
    /// Drive is active (engaged).
    Active,
}

/// Derive the current button view from an `ImpulseState`.
pub fn impulse_button_view(state: &ImpulseState) -> ImpulseButtonView {
    match state.phase {
        ImpulsePhase::Idle => ImpulseButtonView::Ready,
        ImpulsePhase::Charging => ImpulseButtonView::Charging { progress: state.charge_progress },
        ImpulsePhase::Active => ImpulseButtonView::Active,
    }
}

/// Called when the player presses the impulse button.
/// Returns the `ClientMessage` to send if the drive is idle (ready to charge),
/// or `None` if already charging/active.
pub fn press_impulse_button(state: &ImpulseState) -> Option<ClientMessage> {
    if state.phase == ImpulsePhase::Idle {
        Some(ClientMessage::StartImpulseCharge)
    } else {
        None
    }
}

/// Visibility derivation for the helm panel's impulse-related controls.
///
/// `progress > 0.0` means the impulse drive is either charging or fully
/// active (`charge_progress` ∈ `(0.0, 1.0]`). In both cases the player must
/// not be able to steer (the server is autopiloting during Active, and
/// during Charging the drive is committing to the manoeuvre) — so the
/// joystick hides and a Cancel button takes its place.
///
/// Returns `(joystick_visible, cancel_visible)`.
pub fn impulse_ui_visibility(progress: f32) -> (bool, bool) {
    if progress > 0.0 {
        (false, true)
    } else {
        (true, false)
    }
}

/// Returns `true` iff a `HelmInput` message should be sent given the
/// current impulse charge progress. While the drive is charging or active
/// (`progress > 0.0`) the periodic 10 Hz joystick resend must be suppressed
/// so stale knob values can't override the autopilot or stall it the
/// instant it disengages.
pub fn should_send_helm_input(impulse_charge_progress: f32) -> bool {
    impulse_charge_progress <= 0.0
}

/// Format the impulse status readout shown next to the charging progress
/// bar on the helm panel.
///
/// * `progress` — `ShipView.impulse_charge_progress` ∈ `[0.0, 1.0]`.
/// * `charge_duration` — `ShipClientConfig.impulse_charge_duration` (s).
///
/// Returns `None` when the drive is idle (progress ≤ 0). Returns
/// `Some("ENGAGED")` once the drive reaches Active (progress ≥ 1.0). In
/// between, returns `Some("X.X / Y.Y s")` reflecting elapsed / total
/// charge time, so the player sees a live countdown.
pub fn format_impulse_status(progress: f32, charge_duration: f32) -> Option<String> {
    if progress <= 0.0 {
        return None;
    }
    if progress >= 1.0 {
        return Some("ENGAGED".to_string());
    }
    let total = charge_duration.max(0.0);
    let elapsed = (progress * total).clamp(0.0, total);
    Some(format!("{:.1} / {:.1} s", elapsed, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helm(thrust: f32, steering: f32) -> ClientMessage {
        ClientMessage::HelmInput { thrust, steering }
    }

    #[test]
    fn default_state_is_idle_at_origin() {
        let s = HelmJoystickState::default();
        assert!(!s.active);
        assert_eq!(s.knob_dx, 0.0);
        assert_eq!(s.knob_dy, 0.0);
        assert_eq!(s.last_thrust, 0.0);
        assert_eq!(s.last_steering, 0.0);
    }

    #[test]
    fn clamp_to_circle_passes_through_when_inside() {
        let (x, y) = clamp_to_circle(3.0, 4.0, 100.0);
        assert_eq!(x, 3.0);
        assert_eq!(y, 4.0);
    }

    #[test]
    fn clamp_to_circle_scales_to_boundary_when_outside() {
        // Distance 5, max radius 5 → boundary; doubling input lands on boundary too.
        let (x, y) = clamp_to_circle(6.0, 8.0, 5.0);
        let dist = (x * x + y * y).sqrt();
        assert!((dist - 5.0).abs() < 1e-5, "expected radius 5, got {dist}");
        // Direction preserved.
        assert!((x - 3.0).abs() < 1e-5);
        assert!((y - 4.0).abs() < 1e-5);
    }

    #[test]
    fn clamp_to_circle_collapses_when_radius_nonpositive() {
        assert_eq!(clamp_to_circle(7.0, -2.0, 0.0), (0.0, 0.0));
        assert_eq!(clamp_to_circle(7.0, -2.0, -3.0), (0.0, 0.0));
    }

    #[test]
    fn up_is_positive_thrust_down_is_negative() {
        let (t_up, s_up) = compute_thrust_steering(0.0, -50.0, 50.0);
        assert_eq!(t_up, 1.0);
        assert_eq!(s_up, 0.0);

        let (t_down, _) = compute_thrust_steering(0.0, 50.0, 50.0);
        assert_eq!(t_down, -1.0);
    }

    #[test]
    fn right_is_positive_steering_left_is_negative() {
        let (_, s_right) = compute_thrust_steering(50.0, 0.0, 50.0);
        assert_eq!(s_right, 1.0);

        let (_, s_left) = compute_thrust_steering(-50.0, 0.0, 50.0);
        assert_eq!(s_left, -1.0);
    }

    #[test]
    fn thrust_and_steering_clamp_outside_unit_range() {
        // Caller may pass an unclamped offset; the clamp here is defensive.
        let (t, s) = compute_thrust_steering(200.0, -300.0, 50.0);
        assert_eq!(t, 1.0);
        assert_eq!(s, 1.0);
    }

    #[test]
    fn press_activates_state_and_returns_helm_input() {
        let mut s = HelmJoystickState::default();
        let msg = press(&mut s, 0.0, -50.0, 50.0);
        assert!(s.active);
        assert_eq!(msg, helm(1.0, 0.0));
        assert_eq!(s.last_thrust, 1.0);
        assert_eq!(s.last_steering, 0.0);
        assert_eq!(s.knob_dy, -50.0);
    }

    #[test]
    fn press_clamps_offset_into_the_disc() {
        let mut s = HelmJoystickState::default();
        press(&mut s, 0.0, -200.0, 50.0);
        assert_eq!(s.knob_dy, -50.0, "knob should land on boundary");
    }

    #[test]
    fn drag_is_noop_when_not_active() {
        let mut s = HelmJoystickState::default();
        let out = drag(&mut s, 10.0, 10.0, 50.0);
        assert!(out.is_none());
        assert!(!s.active);
        assert_eq!(s.knob_dx, 0.0);
    }

    #[test]
    fn drag_updates_last_values_when_active() {
        let mut s = HelmJoystickState::default();
        press(&mut s, 0.0, -50.0, 50.0);
        let msg = drag(&mut s, 25.0, 0.0, 50.0).expect("active drag yields message");
        assert_eq!(msg, helm(0.0, 0.5));
        assert_eq!(s.last_thrust, 0.0);
        assert_eq!(s.last_steering, 0.5);
    }

    #[test]
    fn release_resets_state_and_emits_zero_input() {
        let mut s = HelmJoystickState::default();
        press(&mut s, 30.0, -40.0, 50.0);
        let msg = release(&mut s);
        assert_eq!(msg, helm(0.0, 0.0));
        assert!(!s.active);
        assert_eq!(s.knob_dx, 0.0);
        assert_eq!(s.knob_dy, 0.0);
        assert_eq!(s.last_thrust, 0.0);
        assert_eq!(s.last_steering, 0.0);
    }

    #[test]
    fn tick_resends_last_values_only_while_active() {
        let mut s = HelmJoystickState::default();
        assert_eq!(tick(&s), Some(helm(0.0, 0.0)), "idle tick sends zero");

        press(&mut s, 0.0, -25.0, 50.0); // thrust 0.5
        assert_eq!(tick(&s), Some(helm(0.5, 0.0)));
        assert_eq!(tick(&s), Some(helm(0.5, 0.0)), "tick is repeatable");

        release(&mut s);
        assert_eq!(tick(&s), Some(helm(0.0, 0.0)), "idle tick sends zero after release");
    }

    // --- impulse button ---

    #[test]
    fn impulse_button_ready_when_idle() {
        let s = ImpulseState::new();
        assert_eq!(impulse_button_view(&s), ImpulseButtonView::Ready);
    }

    #[test]
    fn impulse_button_press_sends_start_charge_when_idle() {
        let s = ImpulseState::new();
        assert_eq!(press_impulse_button(&s), Some(ClientMessage::StartImpulseCharge));
    }

    #[test]
    fn impulse_button_press_noop_when_charging() {
        let mut s = ImpulseState::new();
        s.start_charge();
        assert_eq!(press_impulse_button(&s), None);
    }

    #[test]
    fn impulse_button_press_noop_when_active() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(crate::impulse::IMPULSE_CHARGE_DURATION, crate::impulse::IMPULSE_CHARGE_DURATION);
        assert_eq!(press_impulse_button(&s), None);
    }

    #[test]
    fn impulse_button_shows_charge_progress() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(crate::impulse::IMPULSE_CHARGE_DURATION / 2.0, crate::impulse::IMPULSE_CHARGE_DURATION);
        match impulse_button_view(&s) {
            ImpulseButtonView::Charging { progress } => {
                assert!((progress - 0.5).abs() < 0.01);
            }
            other => panic!("expected Charging, got {other:?}"),
        }
    }

    #[test]
    fn impulse_button_active_when_fully_charged() {
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(crate::impulse::IMPULSE_CHARGE_DURATION, crate::impulse::IMPULSE_CHARGE_DURATION);
        assert_eq!(impulse_button_view(&s), ImpulseButtonView::Active);
    }

    // --- impulse_ui_visibility ---

    #[test]
    fn impulse_ui_visibility_idle_shows_joystick_hides_cancel() {
        let (joystick, cancel) = impulse_ui_visibility(0.0);
        assert!(joystick, "joystick must be visible when idle");
        assert!(!cancel, "cancel must be hidden when idle");
    }

    #[test]
    fn impulse_ui_visibility_charging_hides_joystick_shows_cancel() {
        let (joystick, cancel) = impulse_ui_visibility(0.5);
        assert!(!joystick, "joystick must be hidden while charging");
        assert!(cancel, "cancel must be visible while charging");
    }

    #[test]
    fn impulse_ui_visibility_active_hides_joystick_shows_cancel() {
        let (joystick, cancel) = impulse_ui_visibility(1.0);
        assert!(!joystick, "joystick must be hidden when active");
        assert!(cancel, "cancel must be visible when active");
    }

    // --- should_send_helm_input ---

    #[test]
    fn helm_input_sent_when_impulse_idle() {
        assert!(should_send_helm_input(0.0));
    }

    #[test]
    fn helm_input_suppressed_while_charging() {
        assert!(!should_send_helm_input(0.25));
        assert!(!should_send_helm_input(0.99));
    }

    #[test]
    fn helm_input_suppressed_while_active() {
        assert!(!should_send_helm_input(1.0));
    }

    // --- format_impulse_status ---

    #[test]
    fn impulse_status_none_when_idle() {
        assert_eq!(format_impulse_status(0.0, 3.0), None);
        assert_eq!(format_impulse_status(-0.1, 3.0), None);
    }

    #[test]
    fn impulse_status_engaged_when_charge_full() {
        assert_eq!(format_impulse_status(1.0, 3.0), Some("ENGAGED".to_string()));
        assert_eq!(format_impulse_status(1.5, 3.0), Some("ENGAGED".to_string()));
    }

    #[test]
    fn impulse_status_shows_elapsed_over_total_while_charging() {
        assert_eq!(format_impulse_status(0.5, 3.0), Some("1.5 / 3.0 s".to_string()));
        assert_eq!(format_impulse_status(0.25, 4.0), Some("1.0 / 4.0 s".to_string()));
    }

    #[test]
    fn impulse_status_handles_zero_duration_gracefully() {
        // No division by zero, no NaN: just shows 0.0 / 0.0.
        assert_eq!(format_impulse_status(0.5, 0.0), Some("0.0 / 0.0 s".to_string()));
    }
}
