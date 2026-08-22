//! The client cheat admission route — and the switch that deletes it from a
//! demo build (issue #940).
//!
//! ## Why a route at all
//!
//! The phone client's settings menu grows a Debug/Cheat tab mirroring the host
//! page's (issue #939). The host page reaches the simulation through
//! `wasm_*` exports it can call directly; a phone cannot, and the two paths a
//! phone *does* have both refuse it:
//!
//!  - `policy::is_command_authorized` admits cheats only for
//!    [`crate::console_bridge::LOCAL_CONSOLE_TOKEN`], which no remote peer may
//!    claim — issue #939 closed exactly that spoof, in `is_reserved_token` and
//!    in `server.html`'s `isPeerTokenAllowed()`.
//!  - Ordinary station tenure does not help either: `god-mode` is declared by
//!    no ship TOML, so `station_for_system` returns `None` and every remote
//!    human token hits the unknown-system deny.
//!
//! So the phone needs a route of its own — one that is *honest about being a
//! remote client* rather than borrowing the host's identity. This module is it.
//!
//! ## Why it is `#[cfg]`-split rather than an `if`
//!
//! The Debug/Cheat tab is hidden in the public demo build. A hidden tab is a UI
//! fact, and UI facts are forgeable — the wire is not hidden just because a
//! button is. So the gate disappears with the tab: `build.rs` turns the same
//! `PHOENIX_DEMO_BUILD` that `crate::build_flags` reads into the
//! `phoenix_demo_build` cfg, and the demo build of this module contains no
//! route to reach. `build_flags::the_cfg_gate_and_the_runtime_flag_agree`
//! pins the two answers together.
//!
//! ## The three client debug routes, and where each one is gated
//!
//! Only the first is here, because only the first is an admission question:
//!
//!  - **God Mode** — an admitted `ToggleGodMode` on `god-mode`, gated by
//!    [`admits_debug_command`] below. It changes damage outcomes, so issue
//!    #900 put it on the normal command-admission path where it is
//!    tick-stamped, logged and replayable. Loosening it for a remote token is
//!    the only change here; everything downstream is untouched, so a human's
//!    God Mode and the host's are the same admitted command with the same
//!    source identity stripped. The route needs a runtime predicate because the
//!    *message* (`ControlSystem`) exists in every build — only this one
//!    `(target, payload)` pair may vanish.
//!  - **The overlay flags** — `ClientMessage::ToggleDebugFlag`, which does not
//!    reach admission at all (the flags are presentation; see the variant's
//!    doc). No predicate gates it because the **variant itself** carries
//!    `#[cfg(not(phoenix_demo_build))]`: in a demo build there is no such
//!    message to decode and no `drain_client_debug_flags` to run.
//!  - **Pause** — `ClientMessage::TogglePause`, gated identically and for a
//!    blunter reason: any one of N demo players could otherwise freeze the
//!    mission for everyone. The host's own pause is untouched in every build.
//!
//! There used to be an `admits_flag` predicate here, narrowing
//! `ToggleDebugFlag` per flag so that `Pause` survived a demo build while its
//! neighbours did not. That was the defect: the message and its drain carried
//! no cfg, so a demo binary still decoded and dispatched a phone's pause. With
//! pause moved to its own variant every remaining flag is diagnostic-only, so
//! the narrowing has nothing left to narrow and the whole message is gated
//! instead. The predicate is gone rather than left behind as a constant `true`.
//!
//! Pure and Bevy-free: the Bevy seam that applies this verdict is
//! `policy::is_command_authorized`.

use crate::core::messages::{SystemControlPayload, SystemId};

/// True when `(target, payload)` is the client Debug/Cheat tab's one
/// admission-path command.
///
/// Split from [`admits_debug_command`] so the *shape* question ("is this the
/// route?") stays testable in a demo build, where the *authority* question is
/// compiled to a constant `false`.
pub fn is_debug_command(target: &SystemId, payload: &SystemControlPayload) -> bool {
    target.0 == crate::ship::system_registry::GOD_MODE_SYSTEM_ID
        && matches!(payload, SystemControlPayload::ToggleGodMode)
}

// ── The build-gated half ────────────────────────────────────────────────────
//
// Two bodies, one signature. In a demo build the body below is the whole of
// the route: a constant `false`.

/// Whether a remote client's `(target, payload)` is admitted by the
/// Debug/Cheat route. **Absent in a demo build** — the non-demo body is not
/// compiled, so there is no route to reach whatever the UI shows.
#[cfg(not(phoenix_demo_build))]
pub fn admits_debug_command(target: &SystemId, payload: &SystemControlPayload) -> bool {
    is_debug_command(target, payload)
}

/// Demo-build body: no client may reach a cheat, whatever it sends.
#[cfg(phoenix_demo_build)]
pub fn admits_debug_command(_target: &SystemId, _payload: &SystemControlPayload) -> bool {
    false
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_flags::is_demo_cfg;

    fn god_mode() -> SystemId {
        SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into())
    }

    /// The route's shape, independent of the build: only `ToggleGodMode` on
    /// `god-mode` is the Debug/Cheat command. A phone cannot smuggle anything
    /// else through this branch.
    #[test]
    fn only_toggle_god_mode_on_god_mode_is_the_debug_command() {
        assert!(is_debug_command(
            &god_mode(),
            &SystemControlPayload::ToggleGodMode
        ));
        // Right payload, wrong target.
        assert!(!is_debug_command(
            &SystemId("repair".into()),
            &SystemControlPayload::ToggleGodMode
        ));
        // Right target, wrong payload — the branch must not become a blanket
        // "anything aimed at god-mode is fine".
        assert!(!is_debug_command(
            &god_mode(),
            &SystemControlPayload::SetThrust { value: 1.0 }
        ));
    }

    /// **The gate test.** Whichever build this is, the cheat route's answer
    /// must equal "this is not the demo". Fails if the cfg is inverted, if a
    /// body is edited to ignore the build, or if `build.rs` stops deriving the
    /// cfg from `PHOENIX_DEMO_BUILD`.
    #[test]
    fn the_cheat_route_exists_exactly_when_this_is_not_a_demo_build() {
        assert_eq!(
            admits_debug_command(&god_mode(), &SystemControlPayload::ToggleGodMode),
            !is_demo_cfg(),
            "the client cheat route must be absent from a demo build and \
             present everywhere else"
        );
    }
}
