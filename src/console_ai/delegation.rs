//! Data-driven delegation allowlist.
//!
//! Whether a console may issue a given control is decided by the owning
//! console's *active complexity preset*: the `[preset.delegated]` table in
//! that console's complexity TOML (e.g. `assets/complexity/tactical.toml`)
//! lists, per receiver console, the control ids the receiver may issue while
//! the preset is active. A console is always authorised for its own controls.
//!
//! This module is Bevy-free and has no side effects — it is a pure look-up
//! over an already-parsed [`ComplexityPreset`].

use crate::complexity::ComplexityPreset;
use crate::messages::Console;

/// Control id for setting the phaser frequency (owned by Tactical).
/// Matches the `controls` entries in `[preset.delegated]` tables.
pub const CONTROL_SET_PHASER_FREQUENCY: &str = "set_phaser_frequency";

/// Stable string key for a console, as used by `[preset.delegated]` table
/// keys in complexity TOMLs (matches the `Console` enum variant names).
pub fn console_key(console: &Console) -> &'static str {
    match console {
        Console::CaptainChair => "CaptainChair",
        Console::Helm => "Helm",
        Console::Tactical => "Tactical",
        Console::Repair => "Repair",
        Console::Sensors => "Sensors",
        Console::Shields => "Shields",
        Console::Navigation => "Navigation",
        Console::Power => "Power",
        Console::Comms => "Comms",
    }
}

/// Returns `true` when `sender` is authorised to issue `control` on the
/// console `owner`, given the owner's currently-active complexity preset
/// (`None` when the owner has no preset selected or no complexity TOML).
///
/// Rules:
/// 1. The owner console is always authorised for its own controls.
/// 2. Any other console is authorised only when the owner's active preset
///    has a `[preset.delegated]` entry for it that lists `control`.
pub fn is_sender_authorized(
    control: &str,
    sender: &Console,
    owner: &Console,
    owner_active_preset: Option<&ComplexityPreset>,
) -> bool {
    if sender == owner {
        return true;
    }
    owner_active_preset
        .and_then(|preset| preset.delegated.get(console_key(sender)))
        .is_some_and(|grant| grant.controls.iter().any(|c| c == control))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complexity::parse_complexity_config;

    /// The shipped Tactical complexity TOML drives these tests, so they fail
    /// if the asset and the delegation behaviour drift apart.
    fn tactical_preset(name: &str) -> ComplexityPreset {
        let toml_str = std::fs::read_to_string("assets/complexity/tactical.toml")
            .expect("tactical.toml must be readable");
        parse_complexity_config(&toml_str)
            .expect("tactical.toml must parse")
            .get_preset(name)
            .unwrap_or_else(|| panic!("preset '{name}' must exist"))
            .clone()
    }

    // ── SetPhaserFrequency × Tactical (owner) ──────────────────────────────

    #[test]
    fn owner_always_authorized_for_own_control_when_low() {
        let low = tactical_preset("Low");
        assert!(is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &Console::Tactical,
            &Console::Tactical,
            Some(&low),
        ));
    }

    #[test]
    fn owner_always_authorized_even_without_preset() {
        assert!(is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &Console::Tactical,
            &Console::Tactical,
            None,
        ));
    }

    // ── SetPhaserFrequency × Sensors ──────────────────────────────────────

    #[test]
    fn sensors_authorized_when_tactical_low_preset_grants_it() {
        let low = tactical_preset("Low");
        assert!(is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &Console::Sensors,
            &Console::Tactical,
            Some(&low),
        ));
    }

    #[test]
    fn sensors_not_authorized_under_std_preset() {
        let std_preset = tactical_preset("Std");
        assert!(!is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &Console::Sensors,
            &Console::Tactical,
            Some(&std_preset),
        ));
    }

    #[test]
    fn sensors_not_authorized_without_active_preset() {
        assert!(!is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &Console::Sensors,
            &Console::Tactical,
            None,
        ));
    }

    // ── SetPhaserFrequency × other consoles ────────────────────────────────

    #[test]
    fn other_consoles_not_authorized_even_when_low() {
        let low = tactical_preset("Low");
        for sender in [
            Console::Helm,
            Console::CaptainChair,
            Console::Repair,
            Console::Power,
            Console::Shields,
            Console::Navigation,
            Console::Comms,
        ] {
            assert!(
                !is_sender_authorized(
                    CONTROL_SET_PHASER_FREQUENCY,
                    &sender,
                    &Console::Tactical,
                    Some(&low),
                ),
                "{sender:?} must not be authorised",
            );
        }
    }

    #[test]
    fn unknown_control_is_denied_for_non_owner() {
        let low = tactical_preset("Low");
        assert!(!is_sender_authorized(
            "no_such_control",
            &Console::Sensors,
            &Console::Tactical,
            Some(&low),
        ));
    }
}
