//! Pure delegation allowlist.
//!
//! A per-control table that, given `(control, sender_console, tactical_is_low)`,
//! returns whether the sender is authorised to issue that control.
//!
//! This module is Bevy-free and has no side effects — it is a pure look-up.

use crate::messages::Console;

/// A named control that can be delegated between consoles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelegatedControl {
    /// Set the phaser frequency (normally Tactical's responsibility).
    SetPhaserFrequency,
}

/// Complexity context required by the allowlist.
///
/// Currently only `tactical_is_low` is needed; extend this struct when more
/// delegation rows are introduced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComplexityContext {
    /// Whether Tactical is currently at Low complexity.
    pub tactical_is_low: bool,
}

/// Returns `true` when `sender_console` is authorised to issue `control`
/// under the given complexity state.
///
/// The allowlist is defined inline as a match:
///
/// | control              | sender             | condition           |
/// |----------------------|--------------------|---------------------|
/// | SetPhaserFrequency   | Tactical           | always              |
/// | SetPhaserFrequency   | Science            | tactical_is_low     |
pub fn is_sender_authorized(
    control: DelegatedControl,
    sender: &Console,
    ctx: &ComplexityContext,
) -> bool {
    match (control, sender) {
        // Tactical may always set phaser frequency.
        (DelegatedControl::SetPhaserFrequency, Console::Tactical) => true,
        // Science may set phaser frequency only when Tactical is Low.
        (DelegatedControl::SetPhaserFrequency, Console::Science) => ctx.tactical_is_low,
        // All other combinations are denied.
        _ => false,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn low_ctx() -> ComplexityContext {
        ComplexityContext { tactical_is_low: true }
    }

    fn full_ctx() -> ComplexityContext {
        ComplexityContext { tactical_is_low: false }
    }

    // ── SetPhaserFrequency × Tactical ──────────────────────────────────────

    #[test]
    fn tactical_always_authorized_for_set_phaser_frequency_when_low() {
        assert!(is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Tactical,
            &low_ctx(),
        ));
    }

    #[test]
    fn tactical_always_authorized_for_set_phaser_frequency_when_full() {
        assert!(is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Tactical,
            &full_ctx(),
        ));
    }

    // ── SetPhaserFrequency × Science ──────────────────────────────────────

    #[test]
    fn science_authorized_for_set_phaser_frequency_when_tactical_is_low() {
        assert!(is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Science,
            &low_ctx(),
        ));
    }

    #[test]
    fn science_not_authorized_for_set_phaser_frequency_when_tactical_is_full() {
        assert!(!is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Science,
            &full_ctx(),
        ));
    }

    // ── SetPhaserFrequency × other consoles ────────────────────────────────

    #[test]
    fn helm_not_authorized_for_set_phaser_frequency() {
        assert!(!is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Helm,
            &low_ctx(),
        ));
    }

    #[test]
    fn captain_not_authorized_for_set_phaser_frequency() {
        assert!(!is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::CaptainChair,
            &low_ctx(),
        ));
    }

    #[test]
    fn repair_not_authorized_for_set_phaser_frequency() {
        assert!(!is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Repair,
            &low_ctx(),
        ));
    }

    #[test]
    fn power_not_authorized_for_set_phaser_frequency() {
        assert!(!is_sender_authorized(
            DelegatedControl::SetPhaserFrequency,
            &Console::Power,
            &low_ctx(),
        ));
    }
}
