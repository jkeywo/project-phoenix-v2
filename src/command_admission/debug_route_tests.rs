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
